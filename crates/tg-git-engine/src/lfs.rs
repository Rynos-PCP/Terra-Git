//! Git LFS and other filter drivers.
//!
//! libgit2 runs no external filter drivers: staging a path with
//! `filter=lfs` through `index.add_path` stores the whole file instead of the
//! LFS pointer (confirmed by `lfs_tests`), and every libgit2 checkout writes
//! the pointer text instead of the content. So:
//! - paths with a filter attribute are staged through the system git
//!   ([`stage_filtered`]), which runs the clean filter;
//! - repos that declare filters read their status through the system git
//!   ([`declares_filters`]), so the clean filter decides what changed;
//! - after a libgit2 operation wrote the worktree, `git lfs checkout` puts
//!   the content back in place of the pointers ([`restore_content`]).

use std::io::Read as _;
use std::path::Path;

use git2::{AttrCheckFlags, AttrValue, Repository};
use tg_domain::{FileDiff, GitProgress, LfsState};

use crate::cancel::CancelToken;
use crate::error::{GitEngineError, Result};
use crate::sidecar;

/// Upper bound for the paths of one `git add` call: Windows caps a command
/// line at 32 767 characters.
const ARG_BUDGET: usize = 24_000;

/// Whether the repository declares a filter driver at its root
/// (`.gitattributes` or `.git/info/attributes`). Nested attribute files are
/// not read here — staging checks every path on its own ([`has_filter`]).
pub fn declares_filters(repo: &Repository) -> bool {
    attribute_files(repo).any(|text| text.contains("filter="))
}

/// Whether the repository uses Git LFS (`filter=lfs` at its root).
pub fn uses_lfs(repo: &Repository) -> bool {
    attribute_files(repo).any(|text| text.contains("filter=lfs"))
}

fn attribute_files(repo: &Repository) -> impl Iterator<Item = String> + '_ {
    let root = repo.workdir().map(|w| w.join(".gitattributes"));
    let info = Some(repo.path().join("info").join("attributes"));
    [root, info]
        .into_iter()
        .flatten()
        .filter_map(|p| std::fs::read_to_string(p).ok())
}

/// Whether `rel` passes through a filter driver (`filter=<name>` set).
pub(crate) fn has_filter(repo: &Repository, rel: &Path) -> bool {
    match repo.get_attr(rel, "filter", AttrCheckFlags::FILE_THEN_INDEX) {
        Ok(v) => matches!(AttrValue::from_string(v), AttrValue::String(_)),
        Err(_) => false,
    }
}

/// Pointers are small; anything larger is content.
const POINTER_MAX: usize = 1024;

/// The content size from an LFS pointer text; `None` when `data` is not one.
pub fn pointer_size(data: &[u8]) -> Option<u64> {
    if data.len() > POINTER_MAX {
        return None;
    }
    let text = std::str::from_utf8(data).ok()?;
    let mut lines = text.lines();
    if !lines
        .next()?
        .starts_with("version https://git-lfs.github.com/spec/")
    {
        return None;
    }
    lines.find_map(|l| l.strip_prefix("size ")?.trim().parse().ok())
}

/// One side of a diff: absent, an LFS pointer (with the content size), or
/// plain content (its size).
enum Side {
    Absent,
    Pointer(u64),
    Content(u64),
}

fn side(repo: &Repository, file: &git2::DiffFile<'_>) -> Side {
    if !file.exists() {
        return Side::Absent;
    }
    let small = |n: u64| n <= POINTER_MAX as u64;
    let id = file.id();
    let stored = !id.is_zero() && repo.odb().is_ok_and(|odb| odb.exists(id));
    if stored {
        // Only small blobs are read — a large one cannot be a pointer.
        if small(file.size()) {
            if let Some(size) = repo
                .find_blob(id)
                .ok()
                .and_then(|b| pointer_size(b.content()))
            {
                return Side::Pointer(size);
            }
        }
        return Side::Content(file.size());
    }
    // Not in the object store: a worktree file — one that is itself still a
    // pointer when its object was never downloaded.
    if let (Some(workdir), Some(path)) = (repo.workdir(), file.path()) {
        let abs = workdir.join(path);
        // A regular file only, and never more read than a pointer can be: a
        // symlink to /dev/zero or a FIFO reports size 0 and would never end.
        let regular = std::fs::symlink_metadata(&abs).is_ok_and(|m| m.is_file() && small(m.len()));
        if regular {
            let mut head = Vec::new();
            let read = std::fs::File::open(&abs).and_then(|f| {
                std::io::Read::take(f, POINTER_MAX as u64 + 1).read_to_end(&mut head)
            });
            if let Some(size) = read.ok().and_then(|_| pointer_size(&head)) {
                return Side::Pointer(size);
            }
        }
    }
    Side::Content(file.size())
}

/// Marks `fd` as an LFS file when one side is a pointer, or when the path is
/// tracked by LFS (a new file not staged yet). The sizes then describe the
/// real content.
pub(crate) fn describe(repo: &Repository, delta: &git2::DiffDelta<'_>, fd: &mut FileDiff) {
    let old = side(repo, &delta.old_file());
    let new = side(repo, &delta.new_file());
    let pointer = matches!(old, Side::Pointer(_)) || matches!(new, Side::Pointer(_));
    if !pointer && !delta.new_file().path().is_some_and(|p| lfs_filter(repo, p)) {
        return;
    }
    let size = |s: Side| match s {
        Side::Absent => None,
        Side::Pointer(n) | Side::Content(n) => Some(n),
    };
    fd.lfs = true;
    fd.is_binary = false;
    fd.hunks.clear();
    fd.old_size = size(old);
    fd.new_size = size(new);
}

fn lfs_filter(repo: &Repository, rel: &Path) -> bool {
    matches!(
        repo.get_attr(rel, "filter", AttrCheckFlags::FILE_THEN_INDEX),
        Ok(Some("lfs"))
    )
}

/// The LFS state of a repository. Cheap for repos
/// without LFS: one attributes read, no process.
pub fn state(path: &Path) -> Result<LfsState> {
    let repo = Repository::discover(path).map_err(|e| crate::error::open_error(path, e))?;
    if !uses_lfs(&repo) {
        return Ok(LfsState::default());
    }
    drop(repo);
    if !lfs_installed(path) {
        return Ok(LfsState {
            used: true,
            installed: false,
            missing: 0,
        });
    }
    let listing = sidecar::run_git(path, &["lfs", "ls-files"])?;
    Ok(LfsState {
        used: true,
        installed: true,
        missing: count_missing(&listing),
    })
}

/// `git lfs ls-files` marks a file whose content is in place with `*` and
/// one that is still a pointer with `-` (`<oid> <*|-> <path>`).
fn count_missing(listing: &str) -> u32 {
    listing
        .lines()
        .filter(|l| l.split_whitespace().nth(1) == Some("-"))
        .count() as u32
}

/// Downloads the missing LFS objects and puts their content in place
/// (`git lfs pull`), with progress.
pub fn pull(path: &Path, cancel: &CancelToken, on: &mut dyn FnMut(GitProgress)) -> Result<String> {
    sidecar::lfs_pull_streaming(path, cancel, on)
}

/// "Track with LFS": `git lfs track <pattern>` writes
/// `.gitattributes`, which is staged; `file`, when git already tracks it, is
/// staged again through the clean filter, so its pointer replaces the content
/// in the index (the next commit moves it to LFS). Staging it again takes the
/// whole worktree file — a file staged only in parts is refused up front, so
/// that choice of hunks is not lost.
pub fn track(path: &Path, pattern: &str, file: &str) -> Result<()> {
    if pattern.trim().is_empty() || pattern.starts_with('-') {
        return Err(GitEngineError::InvalidOperation(format!(
            "Not a valid LFS pattern: {pattern}"
        )));
    }
    if let Ok(repo) = Repository::discover(path) {
        let st = repo
            .status_file(Path::new(file))
            .unwrap_or(git2::Status::CURRENT);
        let staged = st.intersects(
            git2::Status::INDEX_MODIFIED | git2::Status::INDEX_NEW | git2::Status::INDEX_RENAMED,
        );
        let unstaged = st.intersects(git2::Status::WT_MODIFIED | git2::Status::WT_RENAMED);
        if staged && unstaged {
            return Err(GitEngineError::InvalidOperation(format!(
                "“{file}” is staged only in parts. Tracking it with LFS stages the whole \
                 file — stage or unstage the rest first."
            )));
        }
    }
    sidecar::classified(sidecar::run_git(path, &["lfs", "track", pattern]))?;
    let literal = [("GIT_LITERAL_PATHSPECS", "1")];
    sidecar::run_git_long_env(path, &["add", "--", ".gitattributes"], &literal)?;
    let tracked = Repository::discover(path)
        .ok()
        .and_then(|r| r.index().ok())
        .is_some_and(|i| i.get_path(Path::new(file), 0).is_some());
    if tracked {
        sidecar::run_git_long_env(path, &["add", "--renormalize", "--", file], &literal)?;
    }
    Ok(())
}

/// Whether `git-lfs` is installed.
pub fn lfs_installed(repo_path: &Path) -> bool {
    sidecar::run_git(repo_path, &["lfs", "version"]).is_ok()
}

/// Stages `files` through `git add`, in chunks that fit a command line,
/// reporting the count after each chunk and checking `cancel` between them.
pub(crate) fn stage_filtered(
    repo_path: &Path,
    files: &[String],
    cancel: Option<&CancelToken>,
    on_done: &mut dyn FnMut(usize),
) -> Result<()> {
    let mut done = 0;
    for chunk in chunks(files) {
        if cancel.is_some_and(CancelToken::is_cancelled) {
            return Err(GitEngineError::Cancelled);
        }
        let mut args = vec!["add", "--"];
        args.extend(chunk.iter().map(String::as_str));
        // Literal pathspecs: `a[b].bin` must not act as a glob.
        sidecar::run_git_long_env(repo_path, &args, &[("GIT_LITERAL_PATHSPECS", "1")])?;
        done += chunk.len();
        on_done(done);
    }
    Ok(())
}

fn chunks(files: &[String]) -> Vec<&[String]> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut len = 0;
    for (i, f) in files.iter().enumerate() {
        if len + f.len() + 3 > ARG_BUDGET && i > start {
            out.push(&files[start..i]);
            start = i;
            len = 0;
        }
        len += f.len() + 3;
    }
    if start < files.len() {
        out.push(&files[start..]);
    }
    out
}

/// After libgit2 wrote the worktree: replace the LFS pointers it left with
/// the content (`git lfs checkout`, from the local LFS store — objects that
/// were never downloaded stay pointers and show up as missing). Only in
/// repos that use LFS, and best effort: without `git-lfs` the pointers stay,
/// which the repo header reports separately.
pub(crate) fn restore_content(repo: &Repository, paths: Option<&[String]>) {
    if !uses_lfs(repo) {
        return;
    }
    let Some(workdir) = repo.workdir() else {
        return;
    };
    let mut args = vec!["lfs", "checkout"];
    match paths {
        // A long list would not fit a command line — then all of them.
        Some([]) => return,
        Some(p) if p.iter().map(|s| s.len() + 3).sum::<usize>() < ARG_BUDGET => {
            args.push("--");
            args.extend(p.iter().map(String::as_str));
        }
        _ => {}
    }
    // No GIT_LITERAL_PATHSPECS here: git-lfs then matches nothing at all. Its
    // paths are patterns, so a glob character widens the set at most — which
    // only replaces more pointers by their content.
    if let Err(e) = sidecar::run_git_long(workdir, &args) {
        tracing::warn!("git lfs checkout after a worktree write failed: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_respect_the_budget_and_keep_every_path() {
        let files: Vec<String> = (0..5000).map(|i| format!("dir/file-{i:05}.bin")).collect();
        let parts = chunks(&files);
        assert!(parts.len() > 1);
        assert_eq!(parts.iter().map(|c| c.len()).sum::<usize>(), files.len());
        for c in &parts {
            assert!(c.iter().map(|f| f.len() + 3).sum::<usize>() <= ARG_BUDGET);
        }
    }

    #[test]
    fn pointer_size_reads_a_pointer_and_rejects_content() {
        let pointer = b"version https://git-lfs.github.com/spec/v1\noid sha256:abc\nsize 4096\n";
        assert_eq!(pointer_size(pointer), Some(4096));
        assert_eq!(pointer_size(b"hello world"), None);
        assert_eq!(pointer_size(&[0u8; 2000]), None);
    }

    #[test]
    fn ls_files_listing_counts_the_pointers() {
        let listing =
            "4d7a214614 * assets/logo.png\n9b1c3e5f77 - video/intro.mp4\n0aa3 - a b.bin\n";
        assert_eq!(count_missing(listing), 2);
        assert_eq!(count_missing(""), 0);
    }

    #[test]
    fn chunks_of_an_empty_list_are_empty() {
        assert!(chunks(&[]).is_empty());
    }
}
