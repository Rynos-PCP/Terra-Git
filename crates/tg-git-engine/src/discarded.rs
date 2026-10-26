//! Recoverable discard with a storage limit.
//!
//! Before any discard the affected files are copied into a store folder of
//! their own, `<git dir>/terra-git/discarded/<kind>-<unix>-<n>/`:
//!
//! - `manifest` — one record per affected path, three NUL-terminated fields:
//!   the path, its state before and its state after the discard. A state is
//!   `-` (the file did not exist), `?` (not recorded yet) or `<mode>:<blob id>`;
//! - `b<i>` — the content of record `i` before the discard;
//! - `a<i>` — its content after the discard, only when the object database
//!   does not already hold that blob (the "after" of a whole-file discard is the
//!   index version, which git stores anyway).
//!
//! Restoring writes the "before" state back, but only where the worktree still
//! equals the "after" state — anything changed since is reported instead of
//! being overwritten. Re-applying (redo) is the mirror image. Because both
//! states are recorded, the same mechanism covers whole files and single hunks.
//!
//! **Why a folder and not git objects:** the user wanted a storage limit that is
//! actually kept. Objects of a deleted snapshot would stay on disk until git's
//! own gc prunes them (weeks); a folder is exact to measure and deleting it
//! frees the space at once.
//!
//! **The limit is a hard cap, and nothing is deleted automatically** (user
//! decision 2026-10-13): every discard copies ALL its files; if that copy does
//! not fit next to the snapshots already kept, the discard is refused
//! (`DiscardStoreFull`) and the user frees space in the backups view or raises
//! the limit (a setting, passed in by the caller). Snapshots only ever go when
//! the user deletes them.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use git2::{FileMode, ObjectType, Oid, Repository};
use tg_domain::DiscardedInfo;

use crate::error::{GitEngineError, Result};

/// Makes snapshot names unique within one second.
static SEQ: AtomicU64 = AtomicU64::new(0);

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

/// The store folder of a repository (per worktree: `repo.path()` is the
/// worktree's own git dir).
fn store_root(repo: &Repository) -> PathBuf {
    repo.path().join("terra-git").join("discarded")
}

/// Expands the given paths to files: an (untracked) directory stands for every
/// file below it — the discard deletes it as a whole. Symlinks are taken as
/// they are, never followed. Sorted and deduplicated.
///
/// Refused before anything is touched — every refusal names the path:
/// - a nested repository (a folder holding a `.git`): its copy could never be
///   restored (no snapshot writes into a `.git`), and deleting it would take
///   its whole history; `git clean` skips them for the same reason;
/// - a name that is not UTF-8: the manifest could not name it, so it would be
///   deleted without a copy;
/// - any path [`check_path`] refuses.
pub(crate) fn expand_paths(workdir: &Path, files: &[String]) -> Result<Vec<String>> {
    fn nested(rel: &str) -> GitEngineError {
        GitEngineError::InvalidOperation(format!(
            "“{rel}” is a git repository of its own — terra-git does not discard it \
             (that would delete its whole history). Leave it out, or remove it \
             yourself if you really mean to."
        ))
    }
    fn walk(dir: &Path, rel: &str, out: &mut Vec<String>) -> Result<()> {
        if dir.join(".git").symlink_metadata().is_ok() {
            return Err(nested(rel));
        }
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let name = entry.file_name().into_string().map_err(|raw| {
                GitEngineError::InvalidOperation(format!(
                    "A file name in “{rel}” is not valid UTF-8 ({}) — terra-git cannot \
                     keep a copy of it, so it does not discard it.",
                    raw.to_string_lossy()
                ))
            })?;
            let child_rel = format!("{rel}/{name}");
            let md = entry.path().symlink_metadata()?;
            if md.is_dir() {
                walk(&entry.path(), &child_rel, out)?;
            } else {
                out.push(child_rel);
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    for f in files {
        let rel = f.trim_end_matches('/');
        check_path(workdir, rel)?;
        let abs = workdir.join(rel);
        match abs.symlink_metadata() {
            Ok(md) if md.is_dir() => walk(&abs, rel, &mut out)?,
            _ => out.push(rel.to_string()),
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

/// The ancestors of `paths` that are FILES in the worktree: restoring `d/a`
/// from the index removes a file `d` that took the folder's place, so it
/// belongs into the snapshot too.
pub(crate) fn blocking_ancestors(workdir: &Path, paths: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for p in paths {
        let mut prefix = String::new();
        let segments: Vec<&str> = p.split('/').collect();
        for seg in &segments[..segments.len().saturating_sub(1)] {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(seg);
            let file = workdir
                .join(&prefix)
                .symlink_metadata()
                .is_ok_and(|md| md.is_file());
            if file && !out.contains(&prefix) {
                out.push(prefix.clone());
            }
        }
    }
    out
}

/// Bytes of the current worktree files of `paths` (the "before" side).
pub(crate) fn worktree_bytes(workdir: &Path, paths: &[String]) -> u64 {
    paths
        .iter()
        .map(|p| {
            workdir
                .join(p.as_str())
                .symlink_metadata()
                .map(|md| if md.is_file() { md.len() } else { 0 })
                .unwrap_or(0)
        })
        .sum()
}

/// Bytes a snapshot of a whole-file discard may need: the "before" side plus,
/// for every path the index holds, the "after" side — the index version as it
/// is checked out. git stores that side itself only when nothing converts it
/// on the way out (CRLF, LFS); then the snapshot copies it. Counting it keeps
/// the limit a hard cap instead of reaching up to twice the limit.
pub(crate) fn estimate(repo: &Repository, workdir: &Path, paths: &[String]) -> u64 {
    let index = repo.index().ok();
    let after: u64 = paths
        .iter()
        .filter_map(|p| {
            index
                .as_ref()
                .and_then(|i| i.get_path(Path::new(p), 0))
                .map(|e| u64::from(e.file_size))
        })
        .sum();
    worktree_bytes(workdir, paths).saturating_add(after)
}

/// Bytes the store occupies now (all snapshots).
pub(crate) fn used(repo: &Repository) -> u64 {
    list(repo)
        .map(|l| l.iter().map(|d| d.size).sum())
        .unwrap_or(0)
}

/// Refuses a snapshot of `needed` bytes that does not fit next to the ones
/// already kept — nothing is deleted to make room.
pub(crate) fn ensure_room(repo: &Repository, needed: u64, limit_bytes: u64) -> Result<()> {
    let used = used(repo);
    if used.saturating_add(needed) > limit_bytes {
        return Err(GitEngineError::DiscardStoreFull {
            needed,
            used,
            limit: limit_bytes,
        });
    }
    Ok(())
}

fn link_bytes(abs: &Path) -> Result<Vec<u8>> {
    let target = std::fs::read_link(abs)?;
    Ok(target.to_string_lossy().replace('\\', "/").into_bytes())
}

#[cfg(unix)]
fn is_executable(md: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    md.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_md: &std::fs::Metadata) -> bool {
    false
}

/// State of one path: absent, not yet recorded, or present with git mode and
/// blob id (hashed exactly as git would store it).
#[derive(Debug, Clone, PartialEq, Eq)]
enum State {
    Absent,
    Unknown,
    Present { mode: i32, oid: Oid },
}

impl State {
    fn encode(&self) -> String {
        match self {
            State::Absent => "-".into(),
            State::Unknown => "?".into(),
            State::Present { mode, oid } => format!("{mode:o}:{oid}"),
        }
    }

    fn decode(s: &str) -> Result<Self> {
        match s {
            "-" => Ok(State::Absent),
            "?" => Ok(State::Unknown),
            _ => {
                let (mode, oid) = s.split_once(':').ok_or_else(damaged)?;
                Ok(State::Present {
                    mode: i32::from_str_radix(mode, 8).map_err(|_| damaged())?,
                    oid: Oid::from_str(oid).map_err(|_| damaged())?,
                })
            }
        }
    }

    fn oid(&self) -> Option<Oid> {
        match self {
            State::Present { oid, .. } => Some(*oid),
            _ => None,
        }
    }
}

fn damaged() -> GitEngineError {
    GitEngineError::InvalidOperation("Damaged discard snapshot".into())
}

/// Current state of a worktree path, hashed without writing anything.
fn current_state(abs: &Path) -> Result<State> {
    match abs.symlink_metadata() {
        Ok(md) if md.file_type().is_symlink() => Ok(State::Present {
            mode: FileMode::Link.into(),
            oid: Oid::hash_object(ObjectType::Blob, &link_bytes(abs)?)?,
        }),
        Ok(md) if md.is_file() => Ok(State::Present {
            mode: if is_executable(&md) {
                FileMode::BlobExecutable.into()
            } else {
                FileMode::Blob.into()
            },
            oid: Oid::hash_file(ObjectType::Blob, abs)?,
        }),
        _ => Ok(State::Absent),
    }
}

/// Copies a present worktree path into the store (`dest`).
fn copy_into(abs: &Path, dest: &Path) -> Result<()> {
    let md = abs.symlink_metadata()?;
    if md.file_type().is_symlink() {
        std::fs::write(dest, link_bytes(abs)?)?;
    } else {
        std::fs::copy(abs, dest)?;
    }
    Ok(())
}

struct Record {
    path: String,
    before: State,
    after: State,
}

fn write_manifest(dir: &Path, records: &[Record]) -> Result<()> {
    let mut out = Vec::new();
    for r in records {
        for field in [r.path.clone(), r.before.encode(), r.after.encode()] {
            out.extend_from_slice(field.as_bytes());
            out.push(0);
        }
    }
    // Atomic: a half-written manifest must never be read back.
    let tmp = dir.join("manifest.tmp");
    std::fs::write(&tmp, out)?;
    std::fs::rename(tmp, dir.join("manifest"))?;
    Ok(())
}

fn read_manifest(dir: &Path) -> Result<Vec<Record>> {
    let raw = std::fs::read(dir.join("manifest"))?;
    let fields: Vec<String> = raw
        .split(|b| *b == 0)
        .map(|f| String::from_utf8_lossy(f).into_owned())
        .collect();
    // The trailing NUL leaves one empty field at the end.
    let fields = &fields[..fields.len().saturating_sub(1)];
    if !fields.len().is_multiple_of(3) {
        return Err(damaged());
    }
    fields
        .chunks(3)
        .map(|f| {
            Ok(Record {
                path: f[0].clone(),
                before: State::decode(&f[1])?,
                after: State::decode(&f[2])?,
            })
        })
        .collect()
}

/// Validates a snapshot name (`<kind>-<unix>-<n>`) — it becomes a folder name,
/// so nothing but that shape is accepted.
fn check_name(name: &str) -> Result<()> {
    let ok = matches!(parse_name(name), Some((kind, _)) if !kind.is_empty()
        && kind.bytes().all(|b| b.is_ascii_lowercase()));
    if !ok {
        return Err(GitEngineError::InvalidOperation(format!(
            "Not a discard snapshot: {name}"
        )));
    }
    Ok(())
}

/// `<kind>-<unix>-<n>` → (kind, unix).
fn parse_name(name: &str) -> Option<(String, i64)> {
    let mut parts = name.rsplitn(3, '-');
    let n = parts.next()?;
    let ts = parts.next()?;
    let kind = parts.next()?;
    if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((kind.to_string(), ts.parse().ok()?))
}

fn snapshot_dir(repo: &Repository, name: &str) -> Result<PathBuf> {
    check_name(name)?;
    let dir = store_root(repo).join(name);
    if !dir.join("manifest").exists() {
        return Err(GitEngineError::InvalidOperation(format!(
            "Discard snapshot no longer exists: {name}"
        )));
    }
    Ok(dir)
}

fn dir_size(dir: &Path) -> u64 {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|e| e.metadata().ok())
                .filter(|md| md.is_file())
                .map(|md| md.len())
                .sum()
        })
        .unwrap_or(0)
}

/// First half of a protected discard: copies the "before" state into a new
/// snapshot BEFORE anything is touched. Returns the snapshot name.
pub(crate) fn begin(
    repo: &Repository,
    workdir: &Path,
    kind: &str,
    paths: &[String],
) -> Result<String> {
    begin_reporting(repo, workdir, kind, paths, None, &mut |_| {})
}

/// [`begin`] with a count of copied paths for `on_done` and a cancel check
/// between them. A cancelled copy is removed again — nothing was discarded
/// yet, so nothing is lost.
pub(crate) fn begin_reporting(
    repo: &Repository,
    workdir: &Path,
    kind: &str,
    paths: &[String],
    cancel: Option<&crate::CancelToken>,
    on_done: &mut dyn FnMut(usize),
) -> Result<String> {
    // Every path checked before anything is written (they come over IPC).
    for p in paths {
        check_path(workdir, p)?;
    }
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let name = format!("{kind}-{}-{n}", now_secs());
    let dir = store_root(repo).join(&name);
    std::fs::create_dir_all(&dir)?;
    let mut records = Vec::with_capacity(paths.len());
    for (i, p) in paths.iter().enumerate() {
        if cancel.is_some_and(crate::CancelToken::is_cancelled) {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(GitEngineError::Cancelled);
        }
        let abs = workdir.join(p.as_str());
        let before = current_state(&abs)?;
        if matches!(before, State::Present { .. }) {
            copy_into(&abs, &dir.join(format!("b{i}")))?;
        }
        records.push(Record {
            path: p.clone(),
            before,
            after: State::Unknown,
        });
        on_done(i + 1);
    }
    write_manifest(&dir, &records)?;
    Ok(name)
}

/// Second half: records the "after" state, copying it only where git does not
/// hold the blob already.
pub(crate) fn finish(repo: &Repository, workdir: &Path, name: &str) -> Result<()> {
    finish_reporting(repo, workdir, name, &mut |_| {})
}

/// [`finish`] with a count of recorded paths for `on_done`.
pub(crate) fn finish_reporting(
    repo: &Repository,
    workdir: &Path,
    name: &str,
    on_done: &mut dyn FnMut(usize),
) -> Result<()> {
    let dir = snapshot_dir(repo, name)?;
    let odb = repo.odb()?;
    let mut records = read_manifest(&dir)?;
    for (i, r) in records.iter_mut().enumerate() {
        let abs = check_path(workdir, &r.path)?;
        r.after = current_state(&abs)?;
        if let Some(oid) = r.after.oid() {
            if !odb.exists(oid) {
                copy_into(&abs, &dir.join(format!("a{i}")))?;
            }
        }
        on_done(i + 1);
    }
    write_manifest(&dir, &records)?;
    Ok(())
}

/// All snapshots, newest first, with their size on disk.
pub(crate) fn list(repo: &Repository) -> Result<Vec<DiscardedInfo>> {
    let root = store_root(repo);
    let Ok(entries) = std::fs::read_dir(&root) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some((kind, timestamp)) = parse_name(&name) else {
            continue;
        };
        let dir = entry.path();
        let Ok(records) = read_manifest(&dir) else {
            continue;
        };
        out.push(DiscardedInfo {
            name,
            kind,
            timestamp,
            files: records.into_iter().map(|r| r.path).collect(),
            size: dir_size(&dir),
        });
    }
    // Newest first; the sequence number breaks ties within one second.
    out.sort_by(|a, b| {
        b.timestamp.cmp(&a.timestamp).then_with(|| {
            let seq = |n: &str| {
                n.rsplit('-')
                    .next()
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(0)
            };
            seq(&b.name).cmp(&seq(&a.name))
        })
    });
    Ok(out)
}

/// Removes snapshot folders without a readable manifest — left over by a crash
/// between creating the folder and writing it. Never touches complete ones.
pub(crate) fn remove_broken(repo: &Repository) {
    if let Ok(entries) = std::fs::read_dir(store_root(repo)) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && !path.join("manifest").exists() {
                let _ = std::fs::remove_dir_all(path);
            }
        }
    }
}

pub(crate) fn delete(repo: &Repository, name: &str) -> Result<()> {
    let dir = snapshot_dir(repo, name)?;
    std::fs::remove_dir_all(dir)?;
    Ok(())
}

/// Writes one side of a snapshot back into the worktree: `to_before` writes the
/// "before" state where the worktree still matches "after" (and vice versa).
/// Paths that differ are reported unless `force` — then they are snapshotted
/// first (kind `overwrite`, refused like any snapshot if the store is full)
/// and overwritten, so even that loses nothing.
fn write_side(
    repo: &Repository,
    workdir: &Path,
    name: &str,
    to_before: bool,
    force: bool,
    limit_bytes: u64,
) -> Result<()> {
    let dir = snapshot_dir(repo, name)?;
    let records = read_manifest(&dir)?;
    fn pick(r: &Record, before: bool) -> &State {
        if before {
            &r.before
        } else {
            &r.after
        }
    }
    let side = |r| pick(r, to_before);
    let expected = |r| pick(r, !to_before);

    // Everything is checked before anything is written — every path (the
    // manifest is data) and every copy: a damaged snapshot or a lost copy must
    // not leave half a restore behind.
    let odb = repo.odb()?;
    let prefix = if to_before { "b" } else { "a" };
    let mut targets = Vec::with_capacity(records.len());
    for (i, r) in records.iter().enumerate() {
        let abs = check_path(workdir, &r.path)?;
        if let State::Present { oid, .. } = side(r) {
            if !dir.join(format!("{prefix}{i}")).exists() && !odb.exists(*oid) {
                return Err(GitEngineError::InvalidOperation(format!(
                    "The copy of “{}” is no longer available",
                    r.path
                )));
            }
        }
        targets.push(abs);
    }

    let mut changed = Vec::new();
    for (r, abs) in records.iter().zip(&targets) {
        let now = current_state(abs)?;
        // `Unknown` never matches: a snapshot whose second half was not
        // written (crash) is only restored on purpose.
        let matches = match expected(r) {
            State::Unknown => false,
            other => now.oid() == other.oid(),
        };
        if !matches {
            changed.push(r.path.clone());
        }
    }
    if !changed.is_empty() && !force {
        return Err(GitEngineError::DiscardRestoreConflict { files: changed });
    }
    let guard = if changed.is_empty() {
        None
    } else {
        ensure_room(repo, estimate(repo, workdir, &changed), limit_bytes)?;
        Some(begin(repo, workdir, "overwrite", &changed)?)
    };

    // Removals first, deepest first: where a folder `d` took the place of a
    // file `d` (or the other way round), the way must be clear before the
    // other side comes back.
    for (r, abs) in records.iter().zip(&targets).rev() {
        if matches!(side(r), State::Absent) && abs.symlink_metadata().is_ok_and(|md| !md.is_dir()) {
            std::fs::remove_file(abs)?;
        }
    }
    for (i, (r, abs)) in records.iter().zip(&targets).enumerate() {
        let State::Present { mode, oid } = side(r) else {
            continue;
        };
        let stored = dir.join(format!("{prefix}{i}"));
        let content = if stored.exists() {
            std::fs::read(&stored)?
        } else {
            odb.read(*oid)
                .map_err(|_| {
                    GitEngineError::InvalidOperation(format!(
                        "The copy of “{}” is no longer available",
                        r.path
                    ))
                })?
                .data()
                .to_vec()
        };
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent)?;
        }
        match abs.symlink_metadata() {
            // An emptied folder in the file's place goes; a full one stays an error.
            Ok(md) if md.is_dir() => std::fs::remove_dir(abs)?,
            Ok(_) => std::fs::remove_file(abs)?,
            Err(_) => {}
        }
        write_entry(abs, &content, *mode)?;
    }
    if let Some(guard) = guard {
        finish(repo, workdir, &guard)?;
    }
    Ok(())
}

/// Joins a snapshot path onto the worktree and refuses anything that would
/// leave it (`..`, absolute paths, `.git`) — the manifest is data, not trusted,
/// and the paths of a discard come over IPC. `:` and `\` only on Windows,
/// where they mean a drive and a separator (and `\\host\share` a network path);
/// elsewhere they are ordinary characters of a file name.
fn safe_join(workdir: &Path, rel: &str) -> Result<PathBuf> {
    let bad = rel.is_empty()
        || rel.starts_with('/')
        || (cfg!(windows) && (rel.contains('\\') || rel.contains(':')))
        || rel.split('/').any(|seg| {
            seg.is_empty() || seg == "." || seg == ".." || seg.eq_ignore_ascii_case(".git")
        });
    if bad {
        return Err(GitEngineError::InvalidOperation(format!(
            "Unsafe path in discard snapshot: {rel}"
        )));
    }
    Ok(workdir.join(rel))
}

/// [`safe_join`], and no folder on the way is a symbolic link (or a Windows
/// junction): copying, writing or deleting `d/a` must not land wherever `d`
/// points to.
pub(crate) fn check_path(workdir: &Path, rel: &str) -> Result<PathBuf> {
    let abs = safe_join(workdir, rel)?;
    let segments: Vec<&str> = rel.split('/').collect();
    let mut dir = workdir.to_path_buf();
    for seg in &segments[..segments.len() - 1] {
        dir.push(seg);
        if dir
            .symlink_metadata()
            .is_ok_and(|md| md.file_type().is_symlink())
        {
            return Err(GitEngineError::InvalidOperation(format!(
                "A folder on the way to “{rel}” is a symbolic link — terra-git does not \
                 copy, write or delete through it."
            )));
        }
    }
    Ok(abs)
}

fn write_entry(abs: &Path, content: &[u8], mode: i32) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if mode == i32::from(FileMode::Link) {
            let target = String::from_utf8_lossy(content).into_owned();
            std::os::unix::fs::symlink(target, abs)?;
            return Ok(());
        }
        std::fs::write(abs, content)?;
        if mode == i32::from(FileMode::BlobExecutable) {
            let mut perms = std::fs::metadata(abs)?.permissions();
            perms.set_mode(perms.mode() | 0o111);
            std::fs::set_permissions(abs, perms)?;
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        // Without symlink rights git for Windows stores a link as a plain file
        // holding the target — the same fallback here.
        let _ = mode;
        std::fs::write(abs, content)?;
        Ok(())
    }
}

/// Undo of a discard: the "before" state comes back.
pub(crate) fn restore(
    repo: &Repository,
    workdir: &Path,
    name: &str,
    force: bool,
    limit_bytes: u64,
) -> Result<()> {
    write_side(repo, workdir, name, true, force, limit_bytes)
}

/// Redo of a discard: the "after" state comes back.
pub(crate) fn reapply(
    repo: &Repository,
    workdir: &Path,
    name: &str,
    force: bool,
    limit_bytes: u64,
) -> Result<()> {
    write_side(repo, workdir, name, false, force, limit_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_name_splits_kind_and_time() {
        assert_eq!(
            parse_name("files-1700000000-3"),
            Some(("files".into(), 1_700_000_000))
        );
        assert_eq!(
            parse_name("hunk-1700000000-0"),
            Some(("hunk".into(), 1_700_000_000))
        );
        assert_eq!(parse_name("files-17x-0"), None);
        assert_eq!(parse_name("nonsense"), None);
    }

    #[test]
    fn check_name_only_accepts_the_snapshot_shape() {
        assert!(check_name("files-1-0").is_ok());
        assert!(check_name("overwrite-1700000000-12").is_ok());
        for bad in ["../x-1-0", "a/b-1-0", "files-1", "Files-1-0", "-1-0", ""] {
            assert!(check_name(bad).is_err(), "{bad} must be rejected");
        }
    }

    #[test]
    fn state_round_trips() {
        let oid = Oid::from_str("0123456789abcdef0123456789abcdef01234567").unwrap();
        for s in [
            State::Absent,
            State::Unknown,
            State::Present {
                mode: FileMode::Blob.into(),
                oid,
            },
            State::Present {
                mode: FileMode::Link.into(),
                oid,
            },
        ] {
            assert_eq!(State::decode(&s.encode()).unwrap(), s);
        }
        assert!(State::decode("100644").is_err());
    }

    #[test]
    fn safe_join_rejects_escapes() {
        let w = Path::new("/w");
        assert!(safe_join(w, "a/b.txt").is_ok());
        for bad in [
            "../x",
            "a/../../x",
            "/etc/passwd",
            "//host/share/x",
            ".git/config",
            "a/.GIT/x",
            "",
        ] {
            assert!(safe_join(w, bad).is_err(), "{bad} must be rejected");
        }
        // Drive and separator on Windows; ordinary name characters elsewhere,
        // where a file "notes 10:00.md" must stay restorable.
        for name in ["C:x", "a\\b", "notes 10:00.md"] {
            assert_eq!(safe_join(w, name).is_err(), cfg!(windows), "{name}");
        }
    }

    #[test]
    fn nested_repositories_and_symlinked_folders_are_refused_up_front() {
        let dir = tempfile::tempdir().unwrap();
        let w = dir.path();
        std::fs::create_dir_all(w.join("nested/.git")).unwrap();
        std::fs::write(w.join("nested/work.txt"), "x").unwrap();
        std::fs::create_dir_all(w.join("plain/sub")).unwrap();
        std::fs::write(w.join("plain/sub/a.txt"), "a").unwrap();

        let err = expand_paths(w, &["nested".into()]).unwrap_err();
        assert!(err.to_string().contains("nested"), "{err}");
        // Also when the nested repository sits deeper in an untracked folder.
        std::fs::create_dir_all(w.join("plain/inner/.git")).unwrap();
        assert!(expand_paths(w, &["plain".into()]).is_err());
        std::fs::remove_dir_all(w.join("plain/inner")).unwrap();
        assert_eq!(
            expand_paths(w, &["plain".into()]).unwrap(),
            vec!["plain/sub/a.txt".to_string()]
        );

        #[cfg(unix)]
        {
            let outside = tempfile::tempdir().unwrap();
            std::os::unix::fs::symlink(outside.path(), w.join("link")).unwrap();
            assert!(check_path(w, "link/file").is_err());
            assert!(
                check_path(w, "link").is_ok(),
                "the link itself is a file entry"
            );
        }
    }

    #[test]
    fn a_file_in_the_place_of_a_folder_is_found() {
        let dir = tempfile::tempdir().unwrap();
        let w = dir.path();
        std::fs::write(w.join("d"), "file now").unwrap();
        assert_eq!(
            blocking_ancestors(w, &["d/a".into(), "d/b".into(), "e/x".into()]),
            vec!["d".to_string()]
        );
    }
}
