//! Extended git operations (GitHub Desktop parity + community requests).
//!
//! Split as in lib.rs: local, well-mappable operations run through git2;
//! multi-step operations with complex semantics (merge, rebase, cherry-pick,
//! revert, stash-push with paths, apply) run through the system-git sidecar —
//! exactly the semantics of the CLI, hooks and signing included.

use std::path::Path;

use base64::Engine as _;
use git2::{BranchType, ErrorCode, Oid, RepositoryState, ResetType};

use crate::discarded;
use crate::error::{GitEngineError, Result};
use crate::sidecar::{self, literal_pathspec};
use crate::{is_unborn, Git2Engine, GitEngine};
use tg_domain::{
    BackupInfo, BlameLine, CloneOptions, CommitInfo, DiscardedInfo, EolStyle, FileLineStats,
    ImageDiff, ReflogEntry, RemoteInfo, RepoInfo, RepoOpState, RepoSketch, ResetMode, SketchBranch,
    SketchCommit, SparseStatus, StashApplyOutcome, StashInfo, SubmoduleInfo, TagInfo,
    UnchangedInfo, UnchangedReason, UndoAction, UnpushedCommit, WorktreeInfo,
};

/// Detects the line-ending style of a text content.
fn eol_style(bytes: &[u8]) -> EolStyle {
    let mut crlf = 0usize;
    let mut lf = 0usize;
    for (i, b) in bytes.iter().enumerate() {
        if *b == b'\n' {
            if i > 0 && bytes[i - 1] == b'\r' {
                crlf += 1;
            } else {
                lf += 1;
            }
        }
    }
    match (crlf, lf) {
        (0, 0) => EolStyle::None,
        (_, 0) => EolStyle::Crlf,
        (0, _) => EolStyle::Lf,
        _ => EolStyle::Mixed,
    }
}

/// Removes every `\r` that sits immediately before a `\n`. Two contents that are
/// equal afterwards differ exclusively in their line endings.
fn strip_cr_before_lf(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    for (i, b) in bytes.iter().enumerate() {
        if *b == b'\r' && bytes.get(i + 1) == Some(&b'\n') {
            continue;
        }
        out.push(*b);
    }
    out
}

/// Heuristic as in git: a NUL byte makes the content binary.
fn is_binary(bytes: &[u8]) -> bool {
    bytes.contains(&0)
}

/// Which line endings would a checkout of this file write into the working
/// tree? Derived from the `eol`/`text` attribute and `core.autocrlf` or
/// `core.eol` — git2 0.21 offers no filter API that could compute it directly.
/// `None` means: no conversion active, the blob lands unchanged.
fn expected_line_endings(repo: &git2::Repository, file: &str) -> Option<EolStyle> {
    use git2::{AttrCheckFlags, AttrValue};

    let attr = |name: &str| {
        repo.get_attr(Path::new(file), name, AttrCheckFlags::empty())
            .ok()
            .map(AttrValue::from_string)
    };

    // An explicit `eol` attribute beats any configuration.
    if let Some(AttrValue::String(v)) = attr("eol") {
        return match v {
            "crlf" => Some(EolStyle::Crlf),
            "lf" => Some(EolStyle::Lf),
            _ => None,
        };
    }

    // Without text handling (`-text`) no conversion happens.
    let is_text = match attr("text") {
        Some(AttrValue::False) => return None,
        Some(AttrValue::True) | Some(AttrValue::String("auto")) => true,
        _ => false,
    };

    let cfg = repo.config().ok()?;

    // git evaluates core.autocrlf as a boolean (true/1/yes/on, in any letter
    // case) and additionally knows the special value "input". A raw comparison
    // against "true" would silently swallow valid configurations.
    if let Ok(v) = cfg.get_string("core.autocrlf") {
        let v = v.trim().to_ascii_lowercase();
        if v == "input" {
            // Converts on commit only; in the working tree everything stays as it is.
            return None;
        }
        if matches!(v.as_str(), "true" | "1" | "yes" | "on") {
            return Some(EolStyle::Crlf);
        }
    }
    if !is_text {
        return None;
    }
    match cfg
        .get_string("core.eol")
        .ok()
        .map(|v| v.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("crlf") => Some(EolStyle::Crlf),
        Some("lf") => Some(EolStyle::Lf),
        // "native" or unset: platform-dependent.
        _ => Some(if cfg!(windows) {
            EolStyle::Crlf
        } else {
            EolStyle::Lf
        }),
    }
}

/// Upper bound for the content comparison in `explain_unchanged`. During the
/// comparison the index blob, the working copy and two normalized copies are in
/// memory at the same time — for a diagnostic side function that cannot be
/// unbounded.
const MAX_UNCHANGED_BYTES: usize = 5 * 1024 * 1024;

/// Formats a git2 file mode in octal, the way git prints it (`100644`).
fn mode_text(mode: git2::FileMode) -> String {
    format!("{:06o}", u32::from(mode))
}

/// Parses the output of `git blame --porcelain` into line attributions.
///
/// Porcelain format: per line group first a header line
/// `<40-hex-sha> <orig-line> <final-line> [<count>]`, then — only on the FIRST
/// occurrence of a commit — its metadata (`author`, `author-time`, …), and
/// finally one tab-prefixed content line per source line. Author and time are
/// therefore cached per SHA.
fn parse_blame_porcelain(out: &str, max_lines: usize) -> Vec<BlameLine> {
    use std::collections::HashMap;

    let mut meta: HashMap<String, (String, i64)> = HashMap::new();
    let mut result: Vec<BlameLine> = Vec::new();
    let mut cur_sha = String::new();
    let mut cur_lineno: u32 = 0;

    for line in out.lines() {
        if let Some(content) = line.strip_prefix('\t') {
            if result.len() >= max_lines {
                break;
            }
            let (author, time) = meta.get(&cur_sha).cloned().unwrap_or_default();
            result.push(BlameLine {
                line_no: cur_lineno,
                short_id: cur_sha.chars().take(8).collect(),
                commit_id: cur_sha.clone(),
                author,
                time,
                content: content.to_string(),
            });
        } else if let Some(name) = line.strip_prefix("author ") {
            meta.entry(cur_sha.clone()).or_default().0 = name.to_string();
        } else if let Some(t) = line.strip_prefix("author-time ") {
            meta.entry(cur_sha.clone()).or_default().1 = t.trim().parse().unwrap_or(0);
        } else if let Some((sha, final_line)) = parse_blame_header(line) {
            cur_sha = sha;
            cur_lineno = final_line;
        }
        // All remaining metadata lines (author-mail, summary, filename,
        // committer…) are irrelevant to BlameLine and are ignored.
    }
    result
}

/// Detects a porcelain header line and returns (SHA, final line number).
/// Only real 40-digit hex SHAs count — that way metadata lines such as
/// `author-mail …` are never misread as a header.
fn parse_blame_header(line: &str) -> Option<(String, u32)> {
    let mut it = line.split(' ');
    let sha = it.next()?;
    if sha.len() != 40 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let _orig_line = it.next()?;
    let final_line: u32 = it.next()?.parse().ok()?;
    Some((sha.to_string(), final_line))
}

// Extended engine capabilities — cut into small traits by concern (instead of
// one ~55-method god trait). All of them are implemented by [`Git2Engine`];
// callers bring them into scope via `tg_git_engine::prelude::*` (or the
// collecting supertrait [`GitEngineExt`]).

/// Stash management (named/partial, apply/pop/drop).
pub trait StashOps {
    fn stash_list(&self, path: &Path) -> Result<Vec<StashInfo>>;
    /// Create a stash; empty `files` = everything, otherwise a partial stash (request #11531).
    fn stash_push(&self, path: &Path, message: &str, files: &[String]) -> Result<String>;
    /// Re-applies a stash and keeps it. Conflicts are reported, not raised:
    /// libgit2 writes the markers into a clean worktree and returns success.
    fn stash_apply(&self, path: &Path, index: usize) -> Result<StashApplyOutcome>;
    /// Like `stash_apply`, but drops the stash — ONLY if nothing is in
    /// conflict. On conflicts the stash stays, so nothing is lost until the
    /// user has resolved them (libgit2's own pop drops unconditionally).
    fn stash_pop(&self, path: &Path, index: usize) -> Result<StashApplyOutcome>;
    fn stash_drop(&self, path: &Path, index: usize) -> Result<()>;
}

/// Create/list/delete tags.
pub trait TagOps {
    fn tags(&self, path: &Path) -> Result<Vec<TagInfo>>;
    fn create_tag(&self, path: &Path, name: &str, message: &str, target: &str) -> Result<()>;
    fn delete_tag(&self, path: &Path, name: &str) -> Result<()>;
}

/// Branch management + merge/rebase onto a branch.
pub trait BranchOps {
    fn rename_branch(&self, path: &Path, old: &str, new: &str) -> Result<()>;
    fn delete_branch(&self, path: &Path, name: &str, force: bool) -> Result<()>;
    /// Merges `name` into the current branch (sidecar; conflicts -> merge state).
    fn merge_branch(&self, path: &Path, name: &str) -> Result<String>;
    /// Like [`merge_branch`](Self::merge_branch), joining two histories
    /// without a common commit (`--allow-unrelated-histories`).
    fn merge_unrelated(&self, path: &Path, name: &str) -> Result<String>;
    /// Rebases the current branch onto `name` (sidecar).
    fn rebase_onto(&self, path: &Path, name: &str) -> Result<String>;
}

/// Running multi-step operations + conflict resolution (merge/rebase/… state).
pub trait ConflictOps {
    fn op_state(&self, path: &Path) -> Result<RepoOpState>;
    /// Context of the running operation for the conflict workshop: names both
    /// sides understandably (branch/commit instead of ours/theirs).
    fn op_context(&self, path: &Path) -> Result<tg_domain::OpContext>;
    fn abort_operation(&self, path: &Path) -> Result<String>;
    fn continue_operation(&self, path: &Path) -> Result<String>;
    /// Resolves a conflict for `file` with "ours" or "theirs" and stages it.
    fn resolve_conflict(&self, path: &Path, file: &str, ours: bool) -> Result<()>;
    /// Splits a conflicted file into segments for the in-app editor.
    fn read_conflict(&self, path: &Path, file: &str) -> Result<tg_domain::ConflictFile>;
    /// Writes the resolved content into the file and stages it.
    fn save_resolution(&self, path: &Path, file: &str, content: &str) -> Result<()>;
    /// Gives up conflicts that belong to NO running operation (e.g. a stash
    /// applied with conflicts): the files go back to their HEAD
    /// state — or away, if HEAD does not have them — after a copy into the
    /// discard store (`limit_bytes`). Returns the snapshot name.
    fn abandon_conflicts(&self, path: &Path, files: &[String], limit_bytes: u64) -> Result<String>;
}

/// History surgery (cherry-pick/revert/squash/interactive rebase) + undo executor.
pub trait HistoryOps {
    fn cherry_pick(&self, path: &Path, commit_id: &str) -> Result<String>;
    fn revert_commit(&self, path: &Path, commit_id: &str) -> Result<String>;
    /// Undoes the last (unpushed) commit; the changes stay staged.
    fn undo_last_commit(&self, path: &Path) -> Result<()>;
    /// Squashes all commits from `oldest_id` up to HEAD into one new commit.
    /// `oldest_id` is the oldest commit to include (its base is that commit's
    /// first parent — or none, when it is the root commit: the result is then a
    /// new root commit). Transactional: on error HEAD stays unchanged.
    fn squash_from(&self, path: &Path, oldest_id: &str, message: &str) -> Result<String>;
    fn create_branch_from_commit(
        &self,
        path: &Path,
        name: &str,
        commit_id: &str,
        checkout: bool,
    ) -> Result<()>;
    fn checkout_commit(&self, path: &Path, commit_id: &str) -> Result<()>;
    fn search_log(&self, path: &Path, query: &str, limit: usize) -> Result<Vec<CommitInfo>>;
    /// The branch HEAD points at while it has no commit yet ("unborn" — a
    /// fresh repo or after undoing the first commit); `None` otherwise.
    fn unborn_branch(&self, path: &Path) -> Result<Option<String>>;
    /// How many commits `to` has that `from` has not (`git rev-list --count
    /// from..to`), capped at 100,000.
    fn count_commits(&self, path: &Path, from: &str, to: &str) -> Result<u32>;
    /// The repository's default branch: the remote's HEAD (`origin/main`), or
    /// a local `main`/`master` when there is none.
    fn default_branch(&self, path: &Path) -> Result<Option<String>>;
    /// The reflog of `reference` (`HEAD` or `refs/heads/<branch>`), newest
    /// first, at most `limit` entries.
    fn reflog(&self, path: &Path, reference: &str, limit: usize) -> Result<Vec<ReflogEntry>>;
    /// Moves the current branch to `commit`. A backup ref is
    /// written first; a hard reset copies uncommitted changes into the discard
    /// store before they are overwritten (`limit_bytes`). Returns the snapshot
    /// name if one was taken.
    fn reset_to(
        &self,
        path: &Path,
        commit: &str,
        mode: ResetMode,
        limit_bytes: u64,
    ) -> Result<Option<String>>;
    /// Commits reachable from `to` but not from `from`, newest first.
    fn log_range(&self, path: &Path, from: &str, to: &str, limit: usize)
        -> Result<Vec<CommitInfo>>;
    /// Streams the diff between two commits/refs (tree `from` → tree `to`).
    fn range_diff_stream(
        &self,
        path: &Path,
        from: &str,
        to: &str,
        max_files: usize,
        sink: &mut dyn FnMut(tg_domain::FileDiff) -> bool,
    ) -> Result<usize>;
    /// The best common ancestor of two commits/refs.
    fn merge_base(&self, path: &Path, a: &str, b: &str) -> Result<String>;
    /// Interactive rebase of the range `base_id..HEAD` according to `steps`
    /// (an EMPTY `base_id` rewrites the whole history from the root commit on,
    /// `git rebase -i --root`)
    /// (order = the new order of application, oldest first). Strictly validated
    /// against data loss: the step set has to match the range exactly, no merges,
    /// and the first non-drop action has to be `pick`. On conflicts the repo
    /// stays in the rebase state (banner + continue/abort).
    fn rebase_interactive(
        &self,
        path: &Path,
        base_id: &str,
        steps: &[tg_domain::RebaseStep],
    ) -> Result<String>;
    /// Executes a stored undo/redo action from the undo stack.
    /// Before destructive steps it checks that the repo state still matches the
    /// recorded action (right branch, no running operation, no uncommitted
    /// changes on hard resets).
    /// `expected_tip`: the expected branch tip BEFORE the reset — it is
    /// compared against the actual tip INSIDE this function (i.e. under the
    /// caller's lock); on a mismatch the guard refuses with
    /// [`GitEngineError::UndoStale`] instead of throwing away foreign commits.
    fn apply_undo_action(
        &self,
        path: &Path,
        action: &UndoAction,
        expected_tip: Option<&str>,
    ) -> Result<()>;
    /// Commits from HEAD that are on NO remote-tracking branch
    /// (`HEAD --not --remotes`). Newest first; `is_head`/`is_merge` set.
    fn unpushed_commits(&self, path: &Path) -> Result<Vec<UnpushedCommit>>;
}

/// git bisect: binary search for the faulty commit (through the sidecar,
/// because every step checks out a commit).
pub trait BisectOps {
    /// Starts bisect: `good` (hex OID) is known good, `bad` (hex OID or None =
    /// HEAD) known bad. Returns git's output.
    fn bisect_start(&self, path: &Path, good: &str, bad: Option<&str>) -> Result<String>;
    /// Marks the currently checked-out commit: action ∈ {good, bad, skip}.
    fn bisect_mark(&self, path: &Path, action: &str) -> Result<String>;
    /// Ends bisect and returns to the original branch.
    fn bisect_reset(&self, path: &Path) -> Result<()>;
}

/// Hunk/line-wise staging (byte-exact, CRLF-safe).
pub trait StagingOps {
    /// Line balance (+x/−y) per changed file: working tree + index against HEAD,
    /// untracked files fully as additions. For the changes overview.
    fn status_numstat(&self, path: &Path) -> Result<Vec<FileLineStats>>;
    /// Stages (`unstage=false`) or unstages (`unstage=true`) a single hunk.
    fn apply_hunk(&self, path: &Path, file: &str, hunk_index: usize, unstage: bool) -> Result<()>;
    /// Discards a single hunk in the workdir.
    fn discard_hunk(&self, path: &Path, file: &str, hunk_index: usize) -> Result<()>;
    /// Stages like [`GitEngine::stage`]; large sets report `staging` progress
    /// with a file counter and stop on `cancel`. The index is written only at
    /// the end, so a cancelled run stages nothing.
    fn stage_with_progress(
        &self,
        path: &Path,
        files: &[String],
        cancel: &crate::CancelToken,
        on: &mut dyn FnMut(tg_domain::GitProgress),
    ) -> Result<()>;
    /// Stages/unstages selected lines (indices into the hunk's body lines).
    fn apply_lines(
        &self,
        path: &Path,
        file: &str,
        hunk_index: usize,
        line_indices: &[usize],
        unstage: bool,
    ) -> Result<()>;
}

/// Remote management (list/add/rename/URL/remove) + push.
pub trait RemoteOps {
    fn remotes(&self, path: &Path) -> Result<Vec<RemoteInfo>>;
    fn push_remote(&self, path: &Path, remote: &str, force: bool) -> Result<String>;
    /// Creates a new remote. Errors on an already taken or invalid name.
    fn add_remote(&self, path: &Path, name: &str, url: &str) -> Result<()>;
    /// Removes a remote together with its remote-tracking refs and configuration.
    fn remove_remote(&self, path: &Path, name: &str) -> Result<()>;
    /// Renames a remote (including the default refspec and tracking refs).
    fn rename_remote(&self, path: &Path, old: &str, new: &str) -> Result<()>;
    /// Changes the fetch/push URL of an existing remote.
    fn set_remote_url(&self, path: &Path, name: &str, url: &str) -> Result<()>;
}

/// Automatic backups (backup refs written before history rewrites).
pub trait BackupOps {
    /// Lists all automatic backups (newest first).
    fn backups(&self, path: &Path) -> Result<Vec<BackupInfo>>;
    /// Hard-resets the current branch back to the backed-up state.
    /// Backs up the current HEAD first (a restore is itself undoable).
    fn restore_backup(&self, path: &Path, ref_name: &str) -> Result<String>;
    /// Removes a backup.
    fn delete_backup(&self, path: &Path, ref_name: &str) -> Result<()>;
}

/// Recoverable discard: every discard first snapshots what it
/// is about to throw away, see [`crate::discarded`].
pub trait DiscardOps {
    /// Discards like [`GitEngine::discard`], after copying ALL affected files
    /// into the discard store. If the copy does not fit next to the snapshots
    /// already kept (`limit_bytes`), nothing is discarded (`DiscardStoreFull`).
    /// Returns the snapshot name.
    fn discard_protected(&self, path: &Path, files: &[String], limit_bytes: u64) -> Result<String>;
    /// Discards one hunk like [`StagingOps::discard_hunk`], after a snapshot of
    /// the file (same limit rule).
    fn discard_hunk_protected(
        &self,
        path: &Path,
        file: &str,
        hunk_index: usize,
        limit_bytes: u64,
    ) -> Result<String>;
    /// All snapshots, newest first, with their size on disk.
    fn discarded(&self, path: &Path) -> Result<Vec<DiscardedInfo>>;
    /// Writes the state before the discard back. Files changed since are
    /// reported (`DiscardRestoreConflict`) unless `force`.
    fn restore_discarded(
        &self,
        path: &Path,
        name: &str,
        force: bool,
        limit_bytes: u64,
    ) -> Result<()>;
    /// Removes a snapshot.
    fn delete_discarded(&self, path: &Path, name: &str) -> Result<()>;
}

/// Repo lifecycle (init/ignore). Cloning runs exclusively in two phases through
/// [`RemoteProgressOps`]: `clone_prepare` + `clone_fetch` (the former
/// single-shot `clone_repo` had no callers and was removed, A-CLONE).
pub trait RepoLifecycleOps {
    fn init_repo(&self, dir: &Path) -> Result<RepoInfo>;
    fn ignore_pattern(&self, path: &Path, pattern: &str) -> Result<()>;
}

/// Read-only views (blame, image diff).
pub trait ViewOps {
    fn blame_file(&self, path: &Path, file: &str) -> Result<Vec<BlameLine>>;
    fn image_diff(&self, path: &Path, file: &str, staged: bool) -> Result<ImageDiff>;
    /// Explains why a file counts as changed even though the diff is empty.
    ///
    /// Only call this when `file_diff` returned a `FileDiff` with an empty
    /// `hunks` vector — `None` on the other hand simply means "clean".
    fn explain_unchanged(&self, path: &Path, file: &str, staged: bool) -> Result<UnchangedInfo>;

    /// Repo sketch for the welcome screen's vein: the last
    /// `window` commits of the HEAD line (merge/tag flag) plus up to
    /// `max_branches` local branches with their branch point (merge base inside
    /// the window) and ahead count, newest tips first. An unborn HEAD yields an
    /// empty sketch — the UI then shows the decorative vein.
    fn repo_sketch(&self, path: &Path, window: usize, max_branches: usize) -> Result<RepoSketch>;
}

/// Worktrees & submodules.
pub trait WorktreeOps {
    fn worktrees(&self, path: &Path) -> Result<Vec<WorktreeInfo>>;
    fn add_worktree(&self, path: &Path, dest: &Path, branch: &str) -> Result<String>;
    fn remove_worktree(&self, path: &Path, worktree_path: &str) -> Result<String>;
    fn submodules(&self, path: &Path) -> Result<Vec<SubmoduleInfo>>;
    fn update_submodules(&self, path: &Path) -> Result<String>;
}

/// Sparse checkout (cone mode) for large worktrees.
pub trait SparseOps {
    /// State of the sparse checkout: active?, cone directories and the top-level
    /// directories of the HEAD tree as the basis for the selection.
    fn sparse_status(&self, path: &Path) -> Result<SparseStatus>;
    /// Restricts the worktree to `dirs` (`git sparse-checkout set --cone`).
    fn sparse_set(&self, path: &Path, dirs: &[String]) -> Result<()>;
    /// Disables sparse checkout and restores the full worktree.
    fn sparse_disable(&self, path: &Path) -> Result<()>;
}

/// Git config + external tools (mergetool, signature check).
pub trait ConfigOps {
    fn config_get(&self, path: &Path, key: &str) -> Result<Option<String>>;
    fn config_set(&self, path: &Path, key: &str, value: &str, global: bool) -> Result<()>;
    /// Checks whether commit signing works with the current configuration.
    fn check_signing(&self, path: &Path) -> Result<String>;
    fn open_mergetool(&self, path: &Path, file: &str) -> Result<String>;
}

/// Progress-streaming, cancellable remote operations (fetch/pull/push/clone).
/// They take `&mut dyn FnMut(GitProgress)` + `&CancelToken` — deliberately not
/// fitting the lean core signatures, but they do belong in the trait abstraction
/// (they used to sit "next to" it as inherent `Git2Engine` methods).
pub trait RemoteProgressOps {
    fn fetch_with_progress(
        &self,
        path: &Path,
        cancel: &crate::CancelToken,
        on: &mut dyn FnMut(tg_domain::GitProgress),
    ) -> Result<String>;
    fn pull_with_progress(
        &self,
        path: &Path,
        prune: bool,
        cancel: &crate::CancelToken,
        on: &mut dyn FnMut(tg_domain::GitProgress),
    ) -> Result<String>;
    /// Pull with an explicit strategy (merge / rebase / fast-forward only) —
    /// unless the repository's or the user's git config sets `pull.rebase` or
    /// `pull.ff` itself: that choice wins.
    fn pull_with_strategy(
        &self,
        path: &Path,
        prune: bool,
        strategy: Option<&str>,
        from: Option<(&str, &str)>,
        cancel: &crate::CancelToken,
        on: &mut dyn FnMut(tg_domain::GitProgress),
    ) -> Result<String>;
    /// Fetch from one named remote, or from all of them (`remote = None`).
    fn fetch_from(
        &self,
        path: &Path,
        remote: Option<&str>,
        cancel: &crate::CancelToken,
        on: &mut dyn FnMut(tg_domain::GitProgress),
    ) -> Result<String>;
    /// Publishes the current branch WITHOUT its unpushed commits: pushes the
    /// newest commit that is already on a remote as the new branch and sets
    /// the upstream. The commits follow with a normal push.
    fn publish_branch_only(
        &self,
        path: &Path,
        remote: &str,
        cancel: &crate::CancelToken,
        on: &mut dyn FnMut(tg_domain::GitProgress),
    ) -> Result<String>;
    /// Clone stage 1: create the target folder, `git init` + `origin` remote (no network).
    fn clone_prepare(&self, url: &str, dest_dir: &Path) -> Result<()>;
    /// Clone stage 2: fetch the data + check out the default branch (with progress).
    fn clone_fetch(
        &self,
        path: &Path,
        options: &CloneOptions,
        cancel: &crate::CancelToken,
        on: &mut dyn FnMut(tg_domain::GitProgress),
    ) -> Result<String>;
    fn push_with_progress(
        &self,
        path: &Path,
        cancel: &crate::CancelToken,
        on: &mut dyn FnMut(tg_domain::GitProgress),
    ) -> Result<String>;
    /// Push to an explicitly chosen remote (push dropdown), with progress.
    fn push_remote_with_progress(
        &self,
        path: &Path,
        remote: &str,
        force: bool,
        cancel: &crate::CancelToken,
        on: &mut dyn FnMut(tg_domain::GitProgress),
    ) -> Result<String>;
}

/// Collecting supertrait over all extended concern traits (convenience /
/// backwards compatibility). Automatically satisfied through the blanket impl
/// for every type that implements the individual traits.
pub trait GitEngineExt:
    GitEngine
    + StashOps
    + TagOps
    + BranchOps
    + ConflictOps
    + HistoryOps
    + BisectOps
    + StagingOps
    + RemoteOps
    + BackupOps
    + DiscardOps
    + RepoLifecycleOps
    + ViewOps
    + WorktreeOps
    + SparseOps
    + ConfigOps
    + RemoteProgressOps
{
}

impl<T> GitEngineExt for T where
    T: GitEngine
        + StashOps
        + TagOps
        + BranchOps
        + ConflictOps
        + HistoryOps
        + BisectOps
        + StagingOps
        + RemoteOps
        + BackupOps
        + DiscardOps
        + RepoLifecycleOps
        + ViewOps
        + WorktreeOps
        + SparseOps
        + ConfigOps
        + RemoteProgressOps
{
}

/// Brings the core and all extended engine traits as well as [`Git2Engine`] into
/// scope: `use tg_git_engine::prelude::*;`. That keeps the call sites unchanged
/// even though the methods now live on several traits.
pub mod prelude {
    pub use super::{
        BackupOps, BisectOps, BranchOps, ConfigOps, ConflictOps, DiscardOps, GitEngineExt,
        HistoryOps, RemoteOps, RemoteProgressOps, RepoLifecycleOps, SparseOps, StagingOps,
        StashOps, TagOps, ViewOps, WorktreeOps,
    };
    pub use crate::{Git2Engine, GitEngine};
}

/// Turns a branch name into a safe reference: if it exists as a local branch,
/// `refs/heads/<name>` is used (never starts with '-', no option injection).
/// Otherwise a leading '-' is rejected.
fn safe_ref(path: &Path, name: &str) -> Result<String> {
    if let Ok(repo) = git2::Repository::discover(path) {
        if repo.find_branch(name, git2::BranchType::Local).is_ok() {
            return Ok(format!("refs/heads/{name}"));
        }
    }
    if name.starts_with('-') {
        return Err(GitEngineError::InvalidOperation(format!(
            "Invalid reference name: {name}"
        )));
    }
    Ok(name.to_string())
}

/// Namespace of the automatic backups.
const BACKUP_REF_PREFIX: &str = "refs/terra-git/backup/";

/// Process counter for unique backup ref names: two backups of the same op
/// within the same second would otherwise get the same name and the (force)
/// reference would overwrite the older backup.
static BACKUP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Safety net before history rewrites: anchors the current HEAD
/// under `refs/terra-git/backup/<op>-<unix>-<n>`. Unlike the reflog, a real
/// reference survives gc/reflog expiry — the old state stays recoverable, e.g.
/// through "create branch from here". The `<n>` suffix (a process-wide counter)
/// makes the name unique even for several rewrites within the same second, so no
/// older backup gets overwritten.
pub(crate) fn create_backup_ref(repo: &git2::Repository, op: &str) -> Result<()> {
    let head = repo.head()?.peel_to_commit()?;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    let n = BACKUP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let name = format!("{BACKUP_REF_PREFIX}{op}-{ts}-{n}");
    repo.reference(
        &name,
        head.id(),
        true,
        &format!("terra-git: backup before {op}"),
    )?;
    Ok(())
}

/// Splits the short name of a backup ref into (op, timestamp). Format:
/// `<op>-<unix>[-<n>]`. The op name may contain `-` itself (rebase-interactive)
/// but is never numeric; `<unix>` and the optional counter `<n>` are. So split
/// the numeric segments off from the right — the LEFTMOST of them is the
/// timestamp, everything before it the op. That parses both the new
/// `<op>-<unix>-<n>` and old `<op>-<unix>` refs correctly (pure & unit-tested).
fn parse_backup_ref_name(short: &str) -> (String, i64) {
    let parts: Vec<&str> = short.split('-').collect();
    // Number of contiguous numeric trailing segments.
    let mut num_tail = 0usize;
    while num_tail < parts.len() && parts[parts.len() - 1 - num_tail].parse::<i64>().is_ok() {
        num_tail += 1;
    }
    // Timestamp = leftmost numeric trailing segment (0 if there is none).
    let ts: i64 = if num_tail > 0 {
        parts[parts.len() - num_tail].parse().unwrap_or(0)
    } else {
        0
    };
    let op = parts[..parts.len().saturating_sub(num_tail)].join("-");
    (op, ts)
}

/// Looks up a backup ref and rejects everything outside the backup namespace
/// (no reset/delete of arbitrary refs through the IPC).
fn find_backup_ref<'r>(repo: &'r git2::Repository, ref_name: &str) -> Result<git2::Reference<'r>> {
    if !ref_name.starts_with(BACKUP_REF_PREFIX) {
        return Err(GitEngineError::InvalidOperation(format!(
            "Not a terra-git backup: {ref_name}"
        )));
    }
    Ok(repo.find_reference(ref_name)?)
}

/// An empty value = remove the entry. An empty config entry (`key =`) is never
/// wanted: it invisibly masks deeper levels (e.g. a local empty email masks the
/// global one — the "it is not being saved" symptom).
fn config_set_or_remove(config: &mut git2::Config, key: &str, value: &str) -> Result<()> {
    if value.is_empty() {
        config_remove(config, key)
    } else {
        config.set_str(key, value)?;
        Ok(())
    }
}

/// Removes a config entry; "does not exist" is not an error.
fn config_remove(config: &mut git2::Config, key: &str) -> Result<()> {
    match config.remove(key) {
        Ok(()) => Ok(()),
        Err(e) if e.code() == ErrorCode::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// The only keys `config_set` writes (lower case): the identity and signing
/// settings and the long-paths way out of `path_too_long`.
const SETTABLE_CONFIG_KEYS: &[&str] = &[
    "user.name",
    "user.email",
    "commit.gpgsign",
    "core.longpaths",
];

/// Refuses git config keys whose value executes a COMMAND during later git
/// operations (command-injection/RCE surface). The `config_set` command is only
/// reachable from our own webview (deny-by-default, strict CSP), but this
/// denylist is defense in depth: it prevents e.g. core.sshCommand, core.pager,
/// alias.* (`!sh`), filter.*.clean/smudge or mergetool.<t>.cmd from being set
/// through the generic settings interface (above all with `global=true`).
/// Pure & unit-tested. Internal engine calls bypass config_set_or_remove and are
/// not affected.
pub fn is_forbidden_config_key(key: &str) -> bool {
    let k = key.trim().to_ascii_lowercase();
    // Whole families whose values start a shell or a program: alias.* (`!sh`),
    // filter.*.clean|smudge|process, pager.<cmd> (per-subcommand pager).
    // include.*/includeIf.* are on the list because they pull in ANOTHER config
    // file wholesale — that file can carry every key denied below, so allowing
    // them would hand out a bypass for this entire denylist.
    const FORBIDDEN_PREFIXES: &[&str] = &["alias.", "filter.", "pager.", "include.", "includeif."];
    if FORBIDDEN_PREFIXES.iter().any(|p| k.starts_with(p)) {
        return true;
    }
    // Last segment: keys that trigger a command/program.
    let last = k.rsplit('.').next().unwrap_or(k.as_str());
    const FORBIDDEN_LEAVES: &[&str] = &[
        "sshcommand",
        "pager", // core.pager
        "editor",
        "hookspath",
        "fsmonitor",
        "askpass",
        "gitproxy",
        "helper",  // credential.helper
        "cmd",     // mergetool.<t>.cmd / difftool.<t>.cmd
        "command", // diff.<d>.command / trailer.<t>.command
        "driver",
        "program", // gpg.program
        "process",
        "clean",
        "smudge",
        "templatedir",
        "packobjectshook",
        "external",             // diff.external (= GIT_EXTERNAL_DIFF)
        "alternaterefscommand", // core.alternateRefsCommand
        "defaultkeycommand",    // gpg.ssh.defaultKeyCommand
        "uploadpack",           // remote.<n>.uploadpack (a local command with file://)
        "receivepack",          // remote.<n>.receivepack
        "tunnel",               // imap.tunnel
        // mergetool.<t>.path / difftool.<t>.path IS the program git executes
        // (git-mergetool--lib, get_merge_tool_path). open_mergetool forces this
        // key from the trusted config levels — writable through `global=true`,
        // that "trusted" value would be attacker-supplied.
        "path",
        "textconv",      // diff.<d>.textconv
        "insteadof",     // url.<base>.insteadOf (redirects any remote)
        "pushinsteadof", // url.<base>.pushInsteadOf
    ];
    FORBIDDEN_LEAVES.contains(&last)
}

/// Rejects arguments git would interpret as an option (leading '-').
/// For free-text arguments (paths, branch names) from the frontend that go
/// positionally to the git CLI without a `--` separator.
fn reject_option_like(value: &str, what: &str) -> Result<()> {
    if value.starts_with('-') {
        return Err(GitEngineError::InvalidOperation(format!(
            "Invalid {what} (starts with '-'): {value}"
        )));
    }
    Ok(())
}

/// Validates a sparse-checkout directory from the frontend against option and
/// path injection: not empty, no leading '-' (option), no ".." (escape from the
/// repo), no backslash (the Windows separator bypasses the checks and is invalid
/// in git pathspecs anyway).
fn validate_sparse_dir(dir: &str) -> Result<()> {
    let bad =
        dir.trim().is_empty() || dir.starts_with('-') || dir.contains("..") || dir.contains('\\');
    if bad {
        return Err(GitEngineError::InvalidOperation(format!(
            "Invalid directory for sparse-checkout: {dir}"
        )));
    }
    Ok(())
}

/// Validates a (possibly abbreviated) commit id for positional CLI arguments:
/// hex characters only, plausible length — it can never be interpreted as an
/// option or a command (option injection).
fn validate_commit_hex(commit: &str) -> Result<()> {
    let ok = (7..=40).contains(&commit.len()) && commit.bytes().all(|b| b.is_ascii_hexdigit());
    if ok {
        Ok(())
    } else {
        Err(GitEngineError::InvalidOperation(format!(
            "Invalid commit id: {commit}"
        )))
    }
}

/// Uncommitted work as git sees it — staged and unstaged changes (both sides
/// of a rename), untracked files left out. Through the engine's status: in a
/// repository with filter drivers (LFS) that is git's own status, where
/// libgit2 would report every filtered file as modified.
fn uncommitted_paths(engine: &Git2Engine, path: &Path) -> Result<Vec<String>> {
    let st = engine.status(path)?;
    let mut out: Vec<String> = st
        .staged
        .iter()
        .chain(st.unstaged.iter())
        .filter(|e| e.kind != tg_domain::ChangeKind::Untracked)
        .flat_map(|e| std::iter::once(e.path.clone()).chain(e.orig_path.clone()))
        .collect();
    out.sort();
    out.dedup();
    Ok(out)
}

/// What a hard reset to `target` overwrites without git saying a word:
/// untracked or ignored files at paths the target commit holds and the index
/// does not — plus files standing where the target needs a folder. `git reset
/// --hard` replaces them silently; none of them is in any commit.
fn overwritten_untracked(
    repo: &git2::Repository,
    workdir: &Path,
    target: &git2::Tree,
) -> Result<Vec<String>> {
    let index = repo.index()?;
    let diff = repo.diff_tree_to_index(Some(target), Some(&index), None)?;
    let target_only: Vec<String> = diff
        .deltas()
        .filter(|d| d.status() == git2::Delta::Deleted)
        .filter_map(|d| {
            d.old_file()
                .path()
                .and_then(|p| p.to_str())
                .map(str::to_string)
        })
        .collect();
    let mut out: Vec<String> = target_only
        .iter()
        .filter(|p| {
            workdir
                .join(p.as_str())
                .symlink_metadata()
                .is_ok_and(|md| !md.is_dir())
        })
        .cloned()
        .collect();
    out.extend(discarded::blocking_ancestors(workdir, &target_only));
    out.sort();
    out.dedup();
    Ok(out)
}

/// Refuses a hard reset that would overwrite untracked or ignored files
/// ([`overwritten_untracked`]) — for the resets that keep no copy.
fn refuse_overwritten_untracked(repo: &git2::Repository, target: &git2::Commit) -> Result<()> {
    let workdir = discard_workdir(repo)?;
    let files = overwritten_untracked(repo, &workdir, &target.tree()?)?;
    if files.is_empty() {
        return Ok(());
    }
    let shown: Vec<&str> = files.iter().take(5).map(String::as_str).collect();
    let more = files.len().saturating_sub(shown.len());
    Err(GitEngineError::InvalidOperation(format!(
        "Files that are not under version control would be overwritten: {}{} — \
         move them away first.",
        shown.join(", "),
        if more > 0 {
            format!(" and {more} more")
        } else {
            String::new()
        }
    )))
}

/// Checks whether index AND workdir (untracked excluded) match the tree of
/// `commit` exactly. Then a hard reset there discards no uncommitted changes —
/// the case "redo of a commit right after its soft undo": the staged changes ARE
/// the content of the target commit.
fn worktree_matches_commit(repo: &git2::Repository, commit: &git2::Commit) -> Result<bool> {
    let tree = commit.tree()?;
    if repo
        .diff_tree_to_index(Some(&tree), None, None)?
        .deltas()
        .len()
        > 0
    {
        return Ok(false);
    }
    let mut opts = git2::DiffOptions::new();
    opts.include_untracked(false);
    Ok(repo
        .diff_index_to_workdir(None, Some(&mut opts))?
        .deltas()
        .len()
        == 0)
}

/// Image extensions for the image diff view.
pub fn image_mime(file: &str) -> Option<&'static str> {
    let ext = file.rsplit('.').next()?.to_ascii_lowercase();
    match ext.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "bmp" => Some("image/bmp"),
        "ico" => Some("image/x-icon"),
        "svg" => Some("image/svg+xml"),
        _ => None,
    }
}

/// Paths in conflict in the index (each once, sorted). Reads the index fresh
/// from disk, so it sees what an operation just wrote.
/// `git merge` of `name` into the current branch. English messages +
/// classification: local changes in the way, conflicts and unrelated
/// histories come back as stable codes.
fn merge_with(path: &Path, name: &str, unrelated: bool) -> Result<String> {
    let target = safe_ref(path, name)?;
    let mut args = vec![CONFLICT_STYLE[0], CONFLICT_STYLE[1], "merge", "--no-edit"];
    if unrelated {
        args.push("--allow-unrelated-histories");
    }
    args.push(&target);
    sidecar::classified(sidecar::run_git_long_env(path, &args, &[("LC_ALL", "C")]))
}

/// Applies stash `index` without dropping it. In a repo with filter drivers
/// (Git LFS) through `git stash apply`, which runs the smudge filter — libgit2
/// would leave LFS pointers in place of changed files. Conflicts are no
/// error here: the caller reads them from the index. Returns a fresh handle.
fn apply_stash(mut repo: git2::Repository, path: &Path, index: usize) -> Result<git2::Repository> {
    if !crate::lfs::declares_filters(&repo) {
        repo.stash_apply(index, None)?;
        return Ok(repo);
    }
    drop(repo);
    let stash = format!("stash@{{{index}}}");
    let applied = sidecar::run_git_long(path, &["stash", "apply", &stash]);
    let repo = git2::Repository::discover(path)?;
    match applied {
        Err(_) if !conflicted_paths(&repo)?.is_empty() => Ok(repo),
        Err(e) => Err(e),
        Ok(_) => Ok(repo),
    }
}

fn conflicted_paths(repo: &git2::Repository) -> Result<Vec<String>> {
    let mut index = repo.index()?;
    index.read(true)?;
    let mut paths = std::collections::BTreeSet::new();
    for conflict in index.conflicts()? {
        let conflict = conflict?;
        // One of the three sides is always present; a deleted side has no entry.
        if let Some(entry) = conflict.our.or(conflict.their).or(conflict.ancestor) {
            paths.insert(String::from_utf8_lossy(&entry.path).into_owned());
        }
    }
    Ok(paths.into_iter().collect())
}

/// Short name of the branch HEAD points at (also while unborn); an error on a
/// detached HEAD — "back to no commit" needs a branch to empty.
fn current_branch_name(repo: &git2::Repository) -> Result<String> {
    let head = repo.find_reference("HEAD")?;
    let target = head
        .symbolic_target()
        .ok()
        .flatten()
        .and_then(|t| t.strip_prefix("refs/heads/").map(str::to_string));
    target.ok_or_else(|| GitEngineError::InvalidOperation("Not possible on a detached HEAD".into()))
}

/// Deletes the branch ref without the root-commit checks (squash to a new
/// root, which replaces a whole range).
fn uncommit_root_unchecked(repo: &git2::Repository, branch: &str) -> Result<()> {
    let mut reference = repo.find_reference(&format!("refs/heads/{branch}"))?;
    delete_keeping_reflog(repo, &mut reference, branch)
}

/// Deletes the branch ref but keeps its reflog: libgit2 removes
/// `logs/refs/heads/<branch>` together with the ref, and the Reflog view would
/// lose the branch's whole history. git appends to the kept file once the
/// branch has a commit again.
fn delete_keeping_reflog(
    repo: &git2::Repository,
    reference: &mut git2::Reference,
    branch: &str,
) -> Result<()> {
    let log = repo
        .commondir()
        .join("logs")
        .join("refs")
        .join("heads")
        .join(branch);
    let kept = std::fs::read(&log).ok();
    reference.delete()?;
    if let Some(bytes) = kept {
        if let Some(parent) = log.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&log, bytes)?;
    }
    Ok(())
}

/// Takes `branch` back to "no commit yet": deletes its ref while HEAD keeps
/// pointing at it. Index and worktree are untouched, so the commit's content
/// shows up as staged. Guarded: HEAD has to be on `branch`,
/// the branch on `expected` (the root commit) — otherwise `UndoStale`.
fn uncommit_root(repo: &git2::Repository, branch: &str, expected: &str) -> Result<()> {
    if current_branch_name(repo)? != branch {
        return Err(GitEngineError::InvalidOperation(format!(
            "Undo is only possible on the original branch: {branch}"
        )));
    }
    let mut reference = repo.find_reference(&format!("refs/heads/{branch}"))?;
    let tip = reference.peel_to_commit()?;
    if tip.id().to_string() != expected {
        return Err(GitEngineError::UndoStale);
    }
    if tip.parent_count() != 0 {
        return Err(GitEngineError::InvalidOperation(
            "Only the very first commit can be taken back to an empty branch".into(),
        ));
    }
    drop(tip);
    delete_keeping_reflog(repo, &mut reference, branch)
}

/// `-c merge.conflictStyle=diff3` for every git call that can leave conflict
/// markers: the conflict workshop then always gets the common base as a third
/// column, whatever the user configured. diff3, not zdiff3:
/// every git version knows it.
const CONFLICT_STYLE: [&str; 2] = ["-c", "merge.conflictStyle=diff3"];

/// libgit2 writes stash conflicts without the base. git can render them again
/// from the index stages, this time with the base. Best effort:
/// the plain markers stay if it fails.
fn rewrite_markers_with_base(path: &Path, files: &[String]) {
    if files.is_empty() {
        return;
    }
    let mut args: Vec<String> = vec!["checkout".into(), "--conflict=diff3".into(), "--".into()];
    args.extend(files.iter().map(|f| literal_pathspec(f)));
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    if let Err(e) = sidecar::run_git(path, &refs) {
        tracing::warn!("could not rewrite the conflict markers with the base: {e}");
    }
}

fn to_data_url(mime: &str, bytes: &[u8]) -> String {
    format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

/// Upper bound for images in the image diff view (base64 across IPC).
const MAX_IMAGE_BYTES: usize = 25 * 1024 * 1024;

/// Drops an image that exceeds the cap (no data-URL roundtrip).
fn cap_image(bytes: Option<Vec<u8>>) -> Option<Vec<u8>> {
    bytes.filter(|b| b.len() <= MAX_IMAGE_BYTES)
}

/// Canonicalizes `workdir/file` and makes sure the path lies INSIDE the workdir
/// (no `..` escape). For read/write access to existing workdir files coming from
/// frontend paths.
fn safe_workdir_path(workdir: &Path, file: &str) -> Result<std::path::PathBuf> {
    let full = std::fs::canonicalize(workdir.join(file))?;
    let wd = std::fs::canonicalize(workdir)?;
    if !full.starts_with(&wd) {
        return Err(GitEngineError::InvalidOperation(format!(
            "Path outside the repository: {file}"
        )));
    }
    Ok(full)
}

/// Reads a workdir image file, but only when its canonical path lies INSIDE the
/// workdir (no `..` escape) and the size is within limits.
fn read_workdir_image(workdir: &Path, file: &str) -> Option<Vec<u8>> {
    let full = std::fs::canonicalize(workdir.join(file)).ok()?;
    let wd = std::fs::canonicalize(workdir).ok()?;
    if !full.starts_with(&wd) {
        return None;
    }
    let meta = std::fs::metadata(&full).ok()?;
    if meta.len() as usize > MAX_IMAGE_BYTES {
        return None;
    }
    std::fs::read(&full).ok()
}

/// Splits a `git diff` patch into a head (up to the first `@@`) and hunks.
fn split_patch(patch: &str) -> (String, Vec<String>) {
    let mut header = String::new();
    let mut hunks: Vec<String> = Vec::new();
    for line in patch.split_inclusive('\n') {
        if line.starts_with("@@") {
            hunks.push(String::from(line));
        } else if let Some(current) = hunks.last_mut() {
            current.push_str(line);
        } else {
            header.push_str(line);
        }
    }
    (header, hunks)
}

/// Builds a partial hunk containing only the selected lines from a hunk.
///
/// Direction-dependent rule (like git gui / GitHub Desktop):
/// - Staging (forward onto the index): unselected `+` lines are dropped,
///   unselected `-` lines become context.
/// - Unstaging (reverse onto the index): unselected `-` lines are dropped,
///   unselected `+` lines become context.
fn build_partial_hunk(hunk: &str, line_indices: &[usize], reverse: bool) -> Result<String> {
    // split_inclusive keeps the original line endings (\n AND \r\n) — otherwise
    // `git apply` fails on CRLF files (context match is byte-exact).
    let mut parts = hunk.split_inclusive('\n');
    let head = parts
        .next()
        .ok_or_else(|| GitEngineError::InvalidOperation("Empty hunk".into()))?;

    // Take the old start from the header.
    let old_start: u32 = head
        .trim_end()
        .strip_prefix("@@ -")
        .and_then(|s| s.split([',', ' ']).next())
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| GitEngineError::InvalidOperation("Invalid hunk header".into()))?;
    // Take the new start as well: on a REVERSE application (unstage via
    // `git apply --cached --reverse`) the NEW side is the anchor. If new_start
    // differs from old_start because of preceding changes, it has to be correct
    // in the header — otherwise the reverse patch locates the hunk wrongly
    // (git-gui deliberately emits BOTH original offsets).
    let new_start: u32 = head
        .trim_end()
        .split(" +")
        .nth(1)
        .and_then(|s| s.split([',', ' ']).next())
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| GitEngineError::InvalidOperation("Invalid hunk header (new)".into()))?;

    // Collect the body lines; tie the "\ No newline" marker firmly to ITS
    // reference line (not to "out is not empty") — otherwise it drifts to the
    // wrong line when lines are dropped and corrupts the index.
    struct Item<'a> {
        text: &'a str,
        marker: Option<&'a str>,
    }
    let mut items: Vec<Item> = Vec::new();
    let mut has_no_newline = false;
    for raw in parts {
        if raw.starts_with('\\') {
            has_no_newline = true;
            if let Some(last) = items.last_mut() {
                last.marker = Some(raw);
            }
            continue;
        }
        items.push(Item {
            text: raw,
            marker: None,
        });
    }

    let selected: std::collections::HashSet<usize> = line_indices.iter().copied().collect();

    // For files without a trailing newline, a PARTIAL selection of the changed
    // lines is ambiguous (marker assignment) — instead of corrupting silently we
    // reject it. A full hunk and normal staging still work.
    if has_no_newline {
        let change_lines = items
            .iter()
            .filter(|i| i.text.starts_with('+') || i.text.starts_with('-'))
            .count();
        if selected.len() < change_lines {
            return Err(GitEngineError::InvalidOperation(
                "File without a trailing newline: please stage the whole hunk".into(),
            ));
        }
    }
    let mut out: Vec<String> = Vec::new();

    // Take the line over unchanged (including its no-newline marker).
    let keep = |out: &mut Vec<String>, item: &Item| {
        out.push(item.text.to_string());
        if let Some(m) = item.marker {
            out.push(m.to_string());
        }
    };
    // Replace the first character (+/-) with a context space, leave the rest byte-exact.
    let to_context = |out: &mut Vec<String>, item: &Item| {
        out.push(format!(" {}", &item.text[1..]));
        if let Some(m) = item.marker {
            out.push(m.to_string());
        }
    };

    for (idx, item) in items.iter().enumerate() {
        let is_selected = selected.contains(&idx);
        match (item.text.chars().next(), is_selected, reverse) {
            (Some('+'), true, _) | (Some('-'), true, _) => keep(&mut out, item),
            // Unselected lines are dropped WITH their marker:
            (Some('+'), false, false) => {} // staging: drop the unwanted addition
            (Some('-'), false, true) => {}  // unstaging: drop the unwanted deletion
            (Some('-'), false, false) => to_context(&mut out, item),
            (Some('+'), false, true) => to_context(&mut out, item),
            _ => keep(&mut out, item), // real context
        }
    }

    let count_old = out
        .iter()
        .filter(|l| l.starts_with(' ') || l.starts_with('-'))
        .count();
    let count_new = out
        .iter()
        .filter(|l| l.starts_with(' ') || l.starts_with('+'))
        .count();
    if count_old == 0 && count_new == 0 {
        return Err(GitEngineError::InvalidOperation("No lines selected".into()));
    }

    let mut result = format!("@@ -{old_start},{count_old} +{new_start},{count_new} @@\n");
    for line in &out {
        result.push_str(line);
    }
    if !result.ends_with('\n') {
        result.push('\n');
    }
    Ok(result)
}

impl Git2Engine {
    /// Squash down to the root: empty the branch (HEAD unborn, index = the
    /// tree of `head_id` stays) and commit the index as the new root commit.
    /// On a failed commit the branch goes back to `head_id`.
    fn squash_into_new_root(
        &self,
        path: &Path,
        branch: &str,
        head_id: Oid,
        message: &str,
    ) -> Result<String> {
        {
            let repo = self.open_repo_at(path)?;
            uncommit_root_unchecked(&repo, branch)?;
        }
        match self.commit(path, message, false) {
            Ok(id) => Ok(id),
            Err(e) => {
                if let Ok(repo) = self.open_repo_at(path) {
                    if let Ok(commit) = repo.find_commit(head_id) {
                        let _ = repo.branch(branch, &commit, true);
                    }
                }
                Err(e)
            }
        }
    }

    fn open_repo_at(&self, path: &Path) -> Result<git2::Repository> {
        let repo =
            git2::Repository::discover(path).map_err(|e| crate::error::open_error(path, e))?;
        // Same rejection as in the core trait (Git2Engine::open): bare repos have
        // no workdir, and all ext operations would otherwise return confusing errors.
        if repo.is_bare() {
            return Err(GitEngineError::NotARepository(format!(
                "{} (bare repositories are not supported)",
                path.display()
            )));
        }
        Ok(repo)
    }

    /// Patch text for a file: workdir↔index (`staged=false`) or index↔HEAD
    /// (`staged=true`), always with a literal pathspec.
    ///
    /// The diff MUST later be applicable with `git apply` AND have exactly the
    /// same hunk boundaries/lines as the libgit2 display (`file_diff`) —
    /// otherwise the hunk/line indices from the frontend address other content.
    /// Therefore every user configuration that would change the format or the
    /// hunk split compared to libgit2 is neutralized:
    /// - `diff.noprefix`/`mnemonicPrefix`: apply expects a/ b/ prefixes
    /// - `diff.external`/`--no-ext-diff` & `--no-textconv`: a real text diff
    /// - `diff.algorithm=myers`, `indentHeuristic=false`, `interHunkContext=0`:
    ///   mirror the libgit2 defaults (otherwise hunk boundaries drift).
    ///
    /// IMPORTANT: NO `core.autocrlf` override. libgit2 normalizes EOLs for the
    /// display diff according to the repo config; the git CLI has to use the same
    /// config, otherwise with `autocrlf=true` (the Windows default) it shows every
    /// line as changed and the patch would silently write the whole file into the
    /// index with CRLF. `git apply` (also without an override in the apply path)
    /// therefore stays consistent with the diff.
    fn file_patch(&self, path: &Path, file: &str, staged: bool) -> Result<String> {
        let literal = literal_pathspec(file);
        let mut args: Vec<&str> = vec![
            "-c",
            "diff.noprefix=false",
            "-c",
            "diff.mnemonicPrefix=false",
            "-c",
            "diff.external=",
            "-c",
            "diff.algorithm=myers",
            "-c",
            "diff.indentHeuristic=false",
            "-c",
            "diff.interHunkContext=0",
            "diff",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "-U3",
        ];
        if staged {
            args.push("--cached");
        }
        args.push("--");
        args.push(&literal);
        // Raw (untrimmed): git apply requires the trailing newline.
        sidecar::run_git_raw(path, &args)
    }

    /// Makes an untracked file visible to the diff (`git add -N`) so hunk/line
    /// staging works for new files too.
    fn ensure_diffable(&self, path: &Path, file: &str) -> Result<()> {
        let repo = self.open_repo_at(path)?;
        let st = repo
            .status_file(Path::new(file))
            .unwrap_or(git2::Status::CURRENT);
        if st.contains(git2::Status::WT_NEW) {
            drop(repo);
            let literal = literal_pathspec(file);
            sidecar::run_git(path, &["add", "--intent-to-add", "--", &literal])?;
        }
        Ok(())
    }

    fn select_hunk(&self, patch: &str, hunk_index: usize) -> Result<(String, String)> {
        let (header, hunks) = split_patch(patch);
        let hunk = hunks.get(hunk_index).cloned().ok_or_else(|| {
            GitEngineError::InvalidOperation(format!("Hunk {hunk_index} does not exist"))
        })?;
        Ok((header, hunk))
    }

    fn blob_bytes_at_head(&self, repo: &git2::Repository, file: &str) -> Option<Vec<u8>> {
        let tree = repo.head().ok()?.peel_to_tree().ok()?;
        let entry = tree.get_path(Path::new(file)).ok()?;
        let blob = repo.find_blob(entry.id()).ok()?;
        Some(blob.content().to_vec())
    }

    fn blob_bytes_in_index(&self, repo: &git2::Repository, file: &str) -> Option<Vec<u8>> {
        let index = repo.index().ok()?;
        let entry = index.get_path(Path::new(file), 0)?;
        let blob = repo.find_blob(entry.id).ok()?;
        Some(blob.content().to_vec())
    }

    /// File mode of both sides from the delta — `FileDiff` does not carry it.
    /// The diff is built the same way as in `file_diff` so the same delta is
    /// looked at.
    ///
    /// On the working-tree path, libgit2 reports both sides as equal on platforms
    /// without reliable mode bits (Windows) — so there is no false hit there,
    /// just no hit at all.
    fn delta_modes(
        &self,
        repo: &git2::Repository,
        file: &str,
        staged: bool,
    ) -> Option<(git2::FileMode, git2::FileMode)> {
        let mut opts = git2::DiffOptions::new();
        opts.pathspec(file).disable_pathspec_match(true);
        let diff = if staged {
            let head_tree = repo.head().ok()?.peel_to_tree().ok();
            repo.diff_tree_to_index(head_tree.as_ref(), None, Some(&mut opts))
                .ok()?
        } else {
            repo.diff_index_to_workdir(None, Some(&mut opts)).ok()?
        };
        let delta = diff.deltas().next()?;
        Some((delta.old_file().mode(), delta.new_file().mode()))
    }
}

impl StashOps for Git2Engine {
    // --- Stash ---

    fn stash_list(&self, path: &Path) -> Result<Vec<StashInfo>> {
        let mut repo = self.open_repo_at(path)?;
        let mut result = Vec::new();
        repo.stash_foreach(|index, message, id| {
            result.push(StashInfo {
                index,
                message: message.to_string(),
                id: id.to_string(),
            });
            true
        })?;
        Ok(result)
    }

    fn stash_push(&self, path: &Path, message: &str, files: &[String]) -> Result<String> {
        // Sidecar instead of git2: a partial stash (paths) and -u in one go.
        let mut args: Vec<String> = vec!["stash".into(), "push".into(), "-u".into()];
        if !message.trim().is_empty() {
            args.push("-m".into());
            args.push(message.to_string());
        }
        if !files.is_empty() {
            args.push("--".into());
            for f in files {
                args.push(literal_pathspec(f));
            }
        }
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        // Long timeout: stash writes objects + resets the worktree — a hard kill
        // after 120 s leaves index.lock behind on large repos.
        sidecar::run_git_long(path, &arg_refs)
    }

    fn stash_apply(&self, path: &Path, index: usize) -> Result<StashApplyOutcome> {
        let repo = apply_stash(self.open_repo_at(path)?, path, index)?;
        let conflicted = conflicted_paths(&repo)?;
        rewrite_markers_with_base(path, &conflicted);
        Ok(StashApplyOutcome {
            conflicted,
            dropped: false,
        })
    }

    fn stash_pop(&self, path: &Path, index: usize) -> Result<StashApplyOutcome> {
        // Never `repo.stash_pop`: it drops even when the apply left conflicts.
        let mut repo = apply_stash(self.open_repo_at(path)?, path, index)?;
        let conflicted = conflicted_paths(&repo)?;
        rewrite_markers_with_base(path, &conflicted);
        let dropped = conflicted.is_empty();
        if dropped {
            repo.stash_drop(index)?;
        }
        Ok(StashApplyOutcome {
            conflicted,
            dropped,
        })
    }

    fn stash_drop(&self, path: &Path, index: usize) -> Result<()> {
        let mut repo = self.open_repo_at(path)?;
        repo.stash_drop(index)?;
        Ok(())
    }
}

impl TagOps for Git2Engine {
    // --- Tags ---

    fn tags(&self, path: &Path) -> Result<Vec<TagInfo>> {
        let repo = self.open_repo_at(path)?;
        let names = repo.tag_names(None)?;
        let mut result = Vec::new();
        // A tag name that is not valid UTF-8 is skipped, as before — one
        // unreadable name must not fail the whole list.
        for name in names.iter().filter_map(|n| n.ok().flatten()) {
            let Ok(obj) = repo.revparse_single(&format!("refs/tags/{name}")) else {
                continue;
            };
            match obj.into_tag() {
                Ok(tag) => result.push(TagInfo {
                    name: name.to_string(),
                    target_id: tag.target_id().to_string(),
                    message: tag.message().ok().flatten().map(|m| m.trim().to_string()),
                    is_annotated: true,
                }),
                Err(obj) => result.push(TagInfo {
                    name: name.to_string(),
                    target_id: obj
                        .peel_to_commit()
                        .map(|c| c.id().to_string())
                        .unwrap_or_else(|_| obj.id().to_string()),
                    message: None,
                    is_annotated: false,
                }),
            }
        }
        result.sort_by(|a, b| b.name.cmp(&a.name));
        Ok(result)
    }

    fn create_tag(&self, path: &Path, name: &str, message: &str, target: &str) -> Result<()> {
        let repo = self.open_repo_at(path)?;
        let spec = if target.is_empty() { "HEAD" } else { target };
        let obj = repo.revparse_single(spec)?;
        if message.trim().is_empty() {
            repo.tag_lightweight(name, &obj, false)?;
        } else {
            let sig = repo.signature()?;
            repo.tag(name, &obj, &sig, message, false)?;
        }
        Ok(())
    }

    fn delete_tag(&self, path: &Path, name: &str) -> Result<()> {
        let repo = self.open_repo_at(path)?;
        repo.tag_delete(name)?;
        Ok(())
    }
}

impl BranchOps for Git2Engine {
    // --- Branch management ---

    fn rename_branch(&self, path: &Path, old: &str, new: &str) -> Result<()> {
        let repo = self.open_repo_at(path)?;
        if let Ok(mut branch) = repo.find_branch(old, BranchType::Local) {
            branch.rename(new, false)?;
            return Ok(());
        }
        // Unborn default branch (fresh repo): it only exists as the symbolic HEAD
        // target, not as a reference. Renaming = moving HEAD.
        let unborn = matches!(repo.head(), Err(ref e) if is_unborn(e));
        let head_target = repo
            .find_reference("HEAD")
            .ok()
            .and_then(|h| h.symbolic_target().ok().flatten().map(str::to_owned));
        if unborn && head_target.as_deref() == Some(format!("refs/heads/{old}").as_str()) {
            if !git2::Branch::name_is_valid(new).unwrap_or(false) {
                return Err(GitEngineError::InvalidOperation(format!(
                    "Invalid branch name: {new}"
                )));
            }
            repo.set_head(&format!("refs/heads/{new}"))?;
            return Ok(());
        }
        Err(GitEngineError::BranchNotFound(old.to_string()))
    }

    fn delete_branch(&self, path: &Path, name: &str, force: bool) -> Result<()> {
        let repo = self.open_repo_at(path)?;
        let mut branch = repo
            .find_branch(name, BranchType::Local)
            .map_err(|_| GitEngineError::BranchNotFound(name.to_string()))?;
        if branch.is_head() {
            return Err(GitEngineError::InvalidOperation(
                "The current branch cannot be deleted".into(),
            ));
        }
        if !force {
            // Like `git branch -d`: only delete when it is contained in HEAD.
            let head_oid = repo.head()?.target();
            let tip = branch.get().target();
            let merged = match (head_oid, tip) {
                (Some(h), Some(t)) => h == t || repo.graph_descendant_of(h, t)?,
                _ => false,
            };
            if !merged {
                return Err(GitEngineError::BranchNotMerged(name.to_string()));
            }
        }
        branch.delete()?;
        Ok(())
    }

    fn merge_branch(&self, path: &Path, name: &str) -> Result<String> {
        merge_with(path, name, false)
    }

    fn merge_unrelated(&self, path: &Path, name: &str) -> Result<String> {
        merge_with(path, name, true)
    }

    fn rebase_onto(&self, path: &Path, name: &str) -> Result<String> {
        let target = safe_ref(path, name)?;
        {
            let repo = self.open_repo_at(path)?;
            create_backup_ref(&repo, "rebase")?;
        }
        sidecar::classified(sidecar::run_git_long_env(
            path,
            &[CONFLICT_STYLE[0], CONFLICT_STYLE[1], "rebase", &target],
            &[("LC_ALL", "C")],
        ))
    }
}

/// Short id (8 hex) for side labels.
fn short_id(oid: Oid) -> String {
    oid.to_string().chars().take(8).collect()
}

/// Finds a local (otherwise remote) branch pointing at `oid` — for
/// understandable labels instead of raw ids. Falls back to the short id (e.g.
/// merging a commit without a branch).
fn branch_label(repo: &git2::Repository, oid: Oid) -> String {
    for kind in [git2::BranchType::Local, git2::BranchType::Remote] {
        if let Ok(branches) = repo.branches(Some(kind)) {
            for (branch, _) in branches.flatten() {
                if branch.get().target() == Some(oid) {
                    if let Ok(Some(name)) = branch.name() {
                        return name.to_string();
                    }
                }
            }
        }
    }
    short_id(oid)
}

/// First non-empty line of a file in the gitdir (MERGE_HEAD, rebase-merge/onto, …).
/// `repo.path()` points at the right gitdir for worktrees too.
fn gitdir_line(repo: &git2::Repository, rel: &str) -> Option<String> {
    let s = std::fs::read_to_string(repo.path().join(rel)).ok()?;
    let line = s.lines().next()?.trim().to_string();
    (!line.is_empty()).then_some(line)
}

pub(crate) fn gitdir_oid(repo: &git2::Repository, rel: &str) -> Option<Oid> {
    gitdir_line(repo, rel).and_then(|s| Oid::from_str(&s).ok())
}

fn commit_summary(repo: &git2::Repository, oid: Oid) -> Option<String> {
    repo.find_commit(oid)
        .ok()?
        .summary()
        .ok()
        .flatten()
        .map(str::to_string)
}

impl ConflictOps for Git2Engine {
    // --- Multi-step operations ---

    fn op_state(&self, path: &Path) -> Result<RepoOpState> {
        let repo = self.open_repo_at(path)?;
        Ok(map_state(repo.state()))
    }

    fn op_context(&self, path: &Path) -> Result<tg_domain::OpContext> {
        let repo = self.open_repo_at(path)?;
        let kind = map_state(repo.state());
        // HEAD label: branch name, otherwise the short id (detached — the normal
        // case during a rebase).
        let head_label = repo.head().ok().and_then(|h| {
            if h.is_branch() {
                h.shorthand().ok().map(str::to_string)
            } else {
                h.target().map(short_id)
            }
        });
        let mut ctx = tg_domain::OpContext {
            kind,
            ours_label: head_label,
            theirs_label: None,
            theirs_summary: None,
            step: None,
            total: None,
        };
        match kind {
            RepoOpState::Merge => {
                // Octopus merges have several MERGE_HEADs; the workshop labels
                // the first one (the UI actions apply to all of them).
                if let Some(oid) = gitdir_oid(&repo, "MERGE_HEAD") {
                    ctx.theirs_label = Some(branch_label(&repo, oid));
                    ctx.theirs_summary = commit_summary(&repo, oid);
                }
            }
            RepoOpState::Rebase => {
                // During a rebase, "ours" is the NEW BASE (onto) and "theirs" is
                // the commit of your own branch currently being replayed — exactly
                // the other way around from what users expect. The workshop
                // therefore names both sides explicitly.
                let dir = if repo.path().join("rebase-merge").is_dir() {
                    "rebase-merge"
                } else {
                    "rebase-apply"
                };
                if let Some(oid) = gitdir_oid(&repo, &format!("{dir}/onto")) {
                    ctx.ours_label = Some(branch_label(&repo, oid));
                }
                if let Some(name) = gitdir_line(&repo, &format!("{dir}/head-name")) {
                    ctx.theirs_label = Some(
                        name.strip_prefix("refs/heads/")
                            .unwrap_or(&name)
                            .to_string(),
                    );
                }
                if let Some(oid) = gitdir_oid(&repo, &format!("{dir}/stopped-sha")) {
                    ctx.theirs_summary = commit_summary(&repo, oid);
                }
                let (step_file, total_file) = if dir == "rebase-merge" {
                    ("msgnum", "end")
                } else {
                    ("next", "last")
                };
                ctx.step =
                    gitdir_line(&repo, &format!("{dir}/{step_file}")).and_then(|s| s.parse().ok());
                ctx.total =
                    gitdir_line(&repo, &format!("{dir}/{total_file}")).and_then(|s| s.parse().ok());
            }
            RepoOpState::Cherrypick => {
                if let Some(oid) = gitdir_oid(&repo, "CHERRY_PICK_HEAD") {
                    ctx.theirs_label = Some(short_id(oid));
                    ctx.theirs_summary = commit_summary(&repo, oid);
                }
            }
            RepoOpState::Revert => {
                if let Some(oid) = gitdir_oid(&repo, "REVERT_HEAD") {
                    ctx.theirs_label = Some(short_id(oid));
                    ctx.theirs_summary = commit_summary(&repo, oid);
                }
            }
            RepoOpState::Clean | RepoOpState::Bisect => {}
        }
        Ok(ctx)
    }

    fn abort_operation(&self, path: &Path) -> Result<String> {
        match self.op_state(path)? {
            RepoOpState::Merge => sidecar::run_git(path, &["merge", "--abort"]),
            RepoOpState::Rebase => sidecar::run_git(path, &["rebase", "--abort"]),
            RepoOpState::Cherrypick => sidecar::run_git(path, &["cherry-pick", "--abort"]),
            RepoOpState::Revert => sidecar::run_git(path, &["revert", "--abort"]),
            RepoOpState::Bisect => Err(GitEngineError::InvalidOperation(
                "Bisect is ended via bisect_reset".into(),
            )),
            RepoOpState::Clean => Err(GitEngineError::InvalidOperation(
                "No running operation to abort".into(),
            )),
        }
    }

    fn continue_operation(&self, path: &Path) -> Result<String> {
        match self.op_state(path)? {
            RepoOpState::Merge => sidecar::run_git_long(path, &["merge", "--continue"]),
            RepoOpState::Rebase => sidecar::run_git_long(
                path,
                &[CONFLICT_STYLE[0], CONFLICT_STYLE[1], "rebase", "--continue"],
            ),
            RepoOpState::Cherrypick => sidecar::run_git_long(
                path,
                &[
                    CONFLICT_STYLE[0],
                    CONFLICT_STYLE[1],
                    "cherry-pick",
                    "--continue",
                ],
            ),
            RepoOpState::Revert => sidecar::run_git_long(
                path,
                &[CONFLICT_STYLE[0], CONFLICT_STYLE[1], "revert", "--continue"],
            ),
            RepoOpState::Bisect => Err(GitEngineError::InvalidOperation(
                "Bisect has no continue — mark good/bad/skip".into(),
            )),
            RepoOpState::Clean => Err(GitEngineError::InvalidOperation(
                "No running operation to continue".into(),
            )),
        }
    }

    fn resolve_conflict(&self, path: &Path, file: &str, ours: bool) -> Result<()> {
        let literal = literal_pathspec(file);
        let side = if ours { "--ours" } else { "--theirs" };
        sidecar::run_git(path, &["checkout", side, "--", &literal])?;
        sidecar::run_git(path, &["add", "--", &literal])?;
        Ok(())
    }

    fn abandon_conflicts(&self, path: &Path, files: &[String], limit_bytes: u64) -> Result<String> {
        let repo = self.open_repo_at(path)?;
        if repo.state() != RepositoryState::Clean {
            return Err(GitEngineError::InvalidOperation(
                "A running operation is aborted with its own Abort".into(),
            ));
        }
        let workdir = discard_workdir(&repo)?;
        discarded::ensure_room(
            &repo,
            discarded::estimate(&repo, &workdir, files),
            limit_bytes,
        )?;
        let name = discarded::begin(&repo, &workdir, "overwrite", files)?;
        let head_tree = repo.head().ok().and_then(|h| h.peel_to_tree().ok());
        for file in files {
            let literal = literal_pathspec(file);
            let in_head = head_tree
                .as_ref()
                .is_some_and(|t| t.get_path(Path::new(file)).is_ok());
            if in_head {
                sidecar::run_git(path, &["checkout", "HEAD", "--", &literal])?;
            } else {
                sidecar::run_git(path, &["rm", "-f", "-q", "--", &literal])?;
            }
        }
        discarded::finish(&repo, &workdir, &name)?;
        Ok(name)
    }

    fn read_conflict(&self, path: &Path, file: &str) -> Result<tg_domain::ConflictFile> {
        let repo = self.open_repo_at(path)?;
        let workdir = repo
            .workdir()
            .ok_or_else(|| GitEngineError::NotARepository("no workdir".into()))?;
        let full = safe_workdir_path(workdir, file)?;
        let meta = std::fs::metadata(&full)?;
        if meta.len() as usize > crate::conflict::MAX_CONFLICT_BYTES {
            return Err(GitEngineError::InvalidOperation(
                "File is too large for the conflict editor".into(),
            ));
        }
        let bytes = std::fs::read(&full)?;
        // No from_utf8_lossy: save_resolution would write the U+FFFD replacement
        // characters back permanently — even into untouched context lines.
        let content = String::from_utf8(bytes).map_err(|_| {
            GitEngineError::InvalidOperation(
                "File is not UTF-8 encoded — please resolve it externally".into(),
            )
        })?;
        Ok(crate::conflict::parse(file, &content))
    }

    fn save_resolution(&self, path: &Path, file: &str, content: &str) -> Result<()> {
        let repo = self.open_repo_at(path)?;
        let workdir = repo
            .workdir()
            .ok_or_else(|| GitEngineError::NotARepository("no workdir".into()))?;
        // The file exists (it is conflicted); make sure it is contained.
        let full = safe_workdir_path(workdir, file)?;
        std::fs::write(&full, content)?;
        drop(repo);
        // Resolved -> stage it (removes the conflict marking in the index).
        let literal = literal_pathspec(file);
        sidecar::run_git(path, &["add", "--", &literal])?;
        Ok(())
    }
}

impl HistoryOps for Git2Engine {
    // --- History operations ---

    fn cherry_pick(&self, path: &Path, commit_id: &str) -> Result<String> {
        // Oid::from_str validates the hex SHA -> never interpretable as an option.
        let oid = Oid::from_str(commit_id)?.to_string();
        sidecar::run_git_long(
            path,
            &[CONFLICT_STYLE[0], CONFLICT_STYLE[1], "cherry-pick", &oid],
        )
    }

    fn revert_commit(&self, path: &Path, commit_id: &str) -> Result<String> {
        let oid = Oid::from_str(commit_id)?.to_string();
        sidecar::run_git_long(
            path,
            &[
                CONFLICT_STYLE[0],
                CONFLICT_STYLE[1],
                "revert",
                "--no-edit",
                &oid,
            ],
        )
    }

    fn undo_last_commit(&self, path: &Path) -> Result<()> {
        let repo = self.open_repo_at(path)?;
        if repo.state() != RepositoryState::Clean {
            return Err(GitEngineError::InvalidOperation(
                "Undo is not possible during a running operation".into(),
            ));
        }
        let head = repo.head()?.peel_to_commit()?;
        // A merge commit has several parents — a soft reset to parent(0) would
        // silently drop the second merge side. That is not an "uncommit" but an
        // un-merge; hence reject it.
        if head.parent_count() > 1 {
            return Err(GitEngineError::InvalidOperation(
                "A merge commit cannot be undone via uncommit".into(),
            ));
        }
        if head.parent_count() == 0 {
            // The very first commit: back to "no commit yet".
            let branch = current_branch_name(&repo)?;
            let id = head.id().to_string();
            create_backup_ref(&repo, "uncommit")?;
            return uncommit_root(&repo, &branch, &id);
        }
        let parent = head.parent(0)?;
        // Soft reset: the commit's changes stay staged (like GitHub Desktop).
        repo.reset(parent.as_object(), ResetType::Soft, None)?;
        Ok(())
    }

    fn squash_from(&self, path: &Path, oldest_id: &str, message: &str) -> Result<String> {
        // Validate first (message + state + base), only THEN reset — otherwise an
        // error leaves the history reset.
        if message.trim().is_empty() {
            return Err(GitEngineError::EmptyCommitMessage);
        }
        // Wrap all repo borrows in one block so they end before the sidecar calls
        // (only the copyable Oids survive).
        let (head_id, base_id) = {
            let repo = self.open_repo_at(path)?;
            if repo.state() != RepositoryState::Clean {
                return Err(GitEngineError::InvalidOperation(
                    "Squash is not possible during a running operation".into(),
                ));
            }
            // The squash commits the index: anything staged on top would end up
            // inside it, and an undo would then throw it away with the commit.
            let head_id = {
                let head_commit = repo.head()?.peel_to_commit()?;
                let staged = repo
                    .diff_tree_to_index(Some(&head_commit.tree()?), None, None)?
                    .deltas()
                    .len();
                if staged > 0 {
                    return Err(GitEngineError::InvalidOperation(
                        "Staged changes would end up in the squashed commit — commit or \
                         unstage them first"
                            .into(),
                    ));
                }
                head_commit.id()
            };
            let oldest = repo.find_commit(Oid::from_str(oldest_id)?)?;
            if oldest.parent_count() == 0 {
                // Down to the root commit: the result becomes the new root.
                if oldest.id() == head_id {
                    return Err(GitEngineError::InvalidOperation(
                        "There is nothing to squash".into(),
                    ));
                }
                if !repo.graph_descendant_of(head_id, oldest.id())? {
                    return Err(GitEngineError::InvalidOperation(
                        "The squash base is not an ancestor of the current commit".into(),
                    ));
                }
                let branch = current_branch_name(&repo)?;
                create_backup_ref(&repo, "squash")?;
                drop(oldest);
                drop(repo);
                return self.squash_into_new_root(path, &branch, head_id, message);
            }
            // Base = first parent of the oldest commit to be squashed.
            let base_id = oldest.parent(0)?.id();
            if base_id == head_id {
                return Err(GitEngineError::InvalidOperation(
                    "There is nothing to squash".into(),
                ));
            }
            if !repo.graph_descendant_of(head_id, base_id)? {
                return Err(GitEngineError::InvalidOperation(
                    "The squash base is not an ancestor of the current commit".into(),
                ));
            }
            // Back up only AFTER all read validations (no orphaned backup ref on
            // rejection), immediately before the first mutating step.
            create_backup_ref(&repo, "squash")?;
            (head_id, base_id)
        };

        // Soft reset to the base, then one shared commit.
        sidecar::run_git(path, &["reset", "--soft", &base_id.to_string()])?;
        match self.commit(path, message, false) {
            Ok(id) => Ok(id),
            Err(e) => {
                // Transactional: restore the history.
                let _ = sidecar::run_git(path, &["reset", "--soft", &head_id.to_string()]);
                Err(e)
            }
        }
    }

    fn create_branch_from_commit(
        &self,
        path: &Path,
        name: &str,
        commit_id: &str,
        checkout: bool,
    ) -> Result<()> {
        {
            let repo = self.open_repo_at(path)?;
            let commit = repo.find_commit(Oid::from_str(commit_id)?)?;
            repo.branch(name, &commit, false)?;
        }
        if checkout {
            self.checkout_branch(path, name)?;
        }
        Ok(())
    }

    fn checkout_commit(&self, path: &Path, commit_id: &str) -> Result<()> {
        let repo = self.open_repo_at(path)?;
        let commit = repo.find_commit(Oid::from_str(commit_id)?)?;
        let blocked = std::cell::RefCell::new(Vec::<String>::new());
        {
            let mut cb = git2::build::CheckoutBuilder::new();
            cb.safe();
            // As with the branch switch: collect the blocking paths, otherwise
            // only libgit2's "n conflicts prevent checkout" is left.
            cb.notify_on(git2::CheckoutNotificationType::CONFLICT);
            cb.notify(|_why, p, _b, _t, _w| {
                if let Some(p) = p {
                    blocked.borrow_mut().push(p.display().to_string());
                }
                true
            });
            repo.checkout_tree(commit.as_object(), Some(&mut cb))
                .map_err(|e| crate::checkout_error(e, blocked.borrow().clone()))?;
        }
        repo.set_head_detached(commit.id())?;
        crate::lfs::restore_content(&repo, None);
        Ok(())
    }

    fn count_commits(&self, path: &Path, from: &str, to: &str) -> Result<u32> {
        const CAP: u32 = 100_000;
        let repo = self.open_repo_at(path)?;
        let from = repo.revparse_single(from)?.peel_to_commit()?.id();
        let to = repo.revparse_single(to)?.peel_to_commit()?.id();
        let mut walk = repo.revwalk()?;
        walk.push(to)?;
        walk.hide(from)?;
        let mut n = 0;
        for oid in walk {
            oid?;
            n += 1;
            if n == CAP {
                break;
            }
        }
        Ok(n)
    }

    fn default_branch(&self, path: &Path) -> Result<Option<String>> {
        let repo = self.open_repo_at(path)?;
        // The remote's HEAD: what the server calls its default branch.
        if let Ok(head) = repo.find_reference("refs/remotes/origin/HEAD") {
            if let Ok(Some(target)) = head.symbolic_target() {
                if let Some(short) = target.strip_prefix("refs/remotes/") {
                    return Ok(Some(short.to_string()));
                }
            }
        }
        for name in ["main", "master"] {
            if repo.find_branch(name, BranchType::Local).is_ok() {
                return Ok(Some(name.to_string()));
            }
        }
        Ok(None)
    }

    fn reflog(&self, path: &Path, reference: &str, limit: usize) -> Result<Vec<ReflogEntry>> {
        if reference != "HEAD" && !reference.starts_with("refs/heads/") {
            return Err(GitEngineError::InvalidOperation(format!(
                "Not a reflog this view shows: {reference}"
            )));
        }
        // A valid ref name has no `..` segments: libgit2 builds `.git/logs/<name>`
        // from it.
        if reference != "HEAD" && !git2::Reference::is_valid_name(reference) {
            return Err(GitEngineError::InvalidOperation(format!(
                "Invalid branch name: {reference}"
            )));
        }
        let repo = self.open_repo_at(path)?;
        let log = repo.reflog(reference)?;
        let mut out = Vec::new();
        for (index, entry) in log.iter().enumerate().take(limit) {
            let message = entry.message().ok().flatten().unwrap_or("").to_string();
            let action = message
                .split(':')
                .next()
                .unwrap_or("")
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_string();
            let id = entry.id_new();
            let summary = repo
                .find_commit(id)
                .ok()
                .and_then(|c| c.summary().ok().flatten().map(str::to_string));
            out.push(ReflogEntry {
                index,
                id: id.to_string(),
                old_id: entry.id_old().to_string(),
                action,
                message,
                time: entry.committer().when().seconds(),
                summary,
            });
        }
        Ok(out)
    }

    fn reset_to(
        &self,
        path: &Path,
        commit: &str,
        mode: ResetMode,
        limit_bytes: u64,
    ) -> Result<Option<String>> {
        validate_commit_hex(commit)?;
        let repo = self.open_repo_at(path)?;
        if repo.state() != RepositoryState::Clean {
            return Err(GitEngineError::InvalidOperation(
                "Finish or abort the running operation first".into(),
            ));
        }
        let head = repo.head()?;
        if !head.is_branch() {
            return Err(GitEngineError::InvalidOperation(
                "Resetting needs a branch (HEAD is detached)".into(),
            ));
        }
        let target = repo.find_commit(Oid::from_str(commit)?)?;
        create_backup_ref(&repo, "reset")?;

        // A hard reset overwrites uncommitted work: copy it into the discard
        // store first, like any discard, so even that is restorable — and
        // that includes untracked or ignored files at paths the target holds,
        // which the reset replaces silently.
        let snapshot = if mode == ResetMode::Hard {
            let workdir = discard_workdir(&repo)?;
            let mut changed = uncommitted_paths(self, path)?;
            changed.extend(overwritten_untracked(&repo, &workdir, &target.tree()?)?);
            changed.sort();
            changed.dedup();
            if changed.is_empty() {
                None
            } else {
                discarded::ensure_room(
                    &repo,
                    discarded::estimate(&repo, &workdir, &changed),
                    limit_bytes,
                )?;
                Some((
                    discarded::begin(&repo, &workdir, "files", &changed)?,
                    workdir,
                ))
            }
        } else {
            None
        };

        let reset_type = match mode {
            ResetMode::Soft => ResetType::Soft,
            ResetMode::Mixed => ResetType::Mixed,
            ResetMode::Hard => ResetType::Hard,
        };
        repo.reset(target.as_object(), reset_type, None)?;
        if mode == ResetMode::Hard {
            crate::lfs::restore_content(&repo, None);
        }
        match snapshot {
            Some((name, workdir)) => {
                discarded::finish(&repo, &workdir, &name)?;
                Ok(Some(name))
            }
            None => Ok(None),
        }
    }

    fn log_range(
        &self,
        path: &Path,
        from: &str,
        to: &str,
        limit: usize,
    ) -> Result<Vec<CommitInfo>> {
        let repo = self.open_repo_at(path)?;
        let from = repo.revparse_single(from)?.peel_to_commit()?.id();
        let to = repo.revparse_single(to)?.peel_to_commit()?.id();
        let mut walk = repo.revwalk()?;
        walk.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::TIME)?;
        walk.push(to)?;
        walk.hide(from)?;
        let mut out = Vec::new();
        for oid in walk.take(limit) {
            out.push(crate::commit_to_info(&repo.find_commit(oid?)?));
        }
        Ok(out)
    }

    fn range_diff_stream(
        &self,
        path: &Path,
        from: &str,
        to: &str,
        max_files: usize,
        sink: &mut dyn FnMut(tg_domain::FileDiff) -> bool,
    ) -> Result<usize> {
        let repo = self.open_repo_at(path)?;
        let from_tree = repo.revparse_single(from)?.peel_to_tree()?;
        let to_tree = repo.revparse_single(to)?.peel_to_tree()?;
        let mut opts = git2::DiffOptions::new();
        opts.context_lines(3);
        let mut diff = repo.diff_tree_to_tree(Some(&from_tree), Some(&to_tree), Some(&mut opts))?;
        let (added, deleted) = diff
            .deltas()
            .fold((0, 0), |(a, d), delta| match delta.status() {
                git2::Delta::Added => (a + 1, d),
                git2::Delta::Deleted => (a, d + 1),
                _ => (a, d),
            });
        if crate::rename_affordable(added, deleted) {
            diff.find_similar(None)?;
        }
        let mut sent = 0usize;
        crate::stream_diff_pub(&repo, &diff, &mut |fd| {
            if sent == max_files || !sink(fd) {
                return false;
            }
            sent += 1;
            true
        })
    }

    fn merge_base(&self, path: &Path, a: &str, b: &str) -> Result<String> {
        let repo = self.open_repo_at(path)?;
        let a = repo.revparse_single(a)?.peel_to_commit()?.id();
        let b = repo.revparse_single(b)?.peel_to_commit()?.id();
        Ok(repo.merge_base(a, b)?.to_string())
    }

    fn unborn_branch(&self, path: &Path) -> Result<Option<String>> {
        let repo = self.open_repo_at(path)?;
        let unborn = match repo.head() {
            Ok(_) => false,
            Err(e) if is_unborn(&e) => true,
            Err(e) => return Err(e.into()),
        };
        if unborn {
            current_branch_name(&repo).map(Some)
        } else {
            Ok(None)
        }
    }

    fn search_log(&self, path: &Path, query: &str, limit: usize) -> Result<Vec<CommitInfo>> {
        // Safety cap: never run unbounded through huge repos.
        const SCAN_CAP: usize = 100_000;
        let repo = self.open_repo_at(path)?;
        let needle = query.to_lowercase();
        let matches = |c: &CommitInfo| {
            c.summary.to_lowercase().contains(&needle)
                || c.author_name.to_lowercase().contains(&needle)
                || c.author_email.to_lowercase().contains(&needle)
                || c.id.starts_with(&needle)
        };

        // Sidecar first: it streams and aborts early once `limit` hits or the cap
        // is reached (reasoning see GitEngine::log). The search covers the ENTIRE
        // history (all branches + tags + HEAD), matching the all-refs graph of the
        // history view (item 3).
        let mut result = Vec::new();
        let mut scanned = 0usize;
        let streamed =
            sidecar::stream_log(path, &["--branches", "--remotes", "--tags", "HEAD"], |c| {
                scanned += 1;
                if matches(&c) {
                    result.push(c);
                }
                result.len() < limit && scanned < SCAN_CAP
            });
        if streamed.is_ok() {
            return Ok(result);
        }

        // Fallback libgit2 (no git in PATH etc. — or an unborn HEAD at which the
        // sidecar call fails): ref families by glob, HEAD only when born; a fresh
        // repo simply yields no hits.
        let mut walk = repo.revwalk()?;
        for glob in ["refs/heads/*", "refs/remotes/*", "refs/tags/*"] {
            walk.push_glob(glob)?;
        }
        match repo.head() {
            Ok(_) => walk.push_head()?,
            Err(e) if is_unborn(&e) => {}
            Err(e) => return Err(e.into()),
        }
        walk.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::TIME)?;

        let mut result = Vec::new();
        for oid in walk.take(SCAN_CAP) {
            let commit = repo.find_commit(oid?)?;
            let info = crate::commit_to_info(&commit);
            if matches(&info) {
                result.push(info);
                if result.len() >= limit {
                    break;
                }
            }
        }
        Ok(result)
    }

    fn rebase_interactive(
        &self,
        path: &Path,
        base_id: &str,
        steps: &[tg_domain::RebaseStep],
    ) -> Result<String> {
        const VALID: [&str; 5] = ["pick", "reword", "squash", "fixup", "drop"];

        if steps.is_empty() {
            return Err(GitEngineError::InvalidOperation("Empty rebase plan".into()));
        }
        for s in steps {
            if !VALID.contains(&s.action.as_str()) {
                return Err(GitEngineError::InvalidOperation(format!(
                    "Unknown rebase action: {}",
                    s.action
                )));
            }
            // map_or instead of is_none_or: the latter is only stable from Rust 1.82 (MSRV 1.77.2).
            if s.action == "reword" && s.message.as_deref().is_none_or(|m| m.trim().is_empty()) {
                return Err(GitEngineError::InvalidOperation(
                    "Reword needs a new commit message.".into(),
                ));
            }
        }
        // Dropping every commit down to the very first: git cannot rebase onto
        // nothing and leaves an empty commit behind. Taking back the first
        // commit is "undo last commit" on it.
        if base_id.is_empty() && steps.iter().all(|s| s.action == "drop") {
            return Err(GitEngineError::InvalidOperation(
                "Every commit down to the very first one would be dropped — keep at least \
                 one, or take the first commit back with “Undo last commit”."
                    .into(),
            ));
        }
        // The first non-drop action has to be pick/reword (squash/fixup needs a
        // preceding commit to fall into).
        if let Some(first) = steps.iter().find(|s| s.action != "drop") {
            if first.action != "pick" && first.action != "reword" {
                return Err(GitEngineError::InvalidOperation(
                    "The first kept commit has to be 'pick' or 'reword' (squash/fixup needs a predecessor).".into(),
                ));
            }
        }

        // Finish all repo borrows before the sidecar calls.
        let (base_oid, todo) = {
            // RAII: reword temp files written while building the plan are removed
            // automatically on an early error return. On the success path they are
            // taken out (mem::take) and survive for the rebase (cleanup then
            // depends on its outcome).
            struct TempFiles(Vec<(std::path::PathBuf, String)>);
            impl Drop for TempFiles {
                fn drop(&mut self) {
                    for (f, _) in self.0.drain(..) {
                        let _ = std::fs::remove_file(f);
                    }
                }
            }
            let repo = self.open_repo_at(path)?;
            if repo.state() != RepositoryState::Clean {
                return Err(GitEngineError::InvalidOperation(
                    "Rebase is not possible during a running operation".into(),
                ));
            }
            // Empty base = from the root commit on (`--root`).
            let base_oid = if base_id.is_empty() {
                None
            } else {
                let oid = Oid::from_str(base_id)?;
                repo.find_commit(oid)?; // has to exist
                Some(oid)
            };

            // Collect the range base..HEAD (first-parent linear history only).
            let mut walk = repo.revwalk()?;
            walk.push_head()?;
            if let Some(base) = base_oid {
                walk.hide(base)?;
            }
            let mut range: std::collections::HashSet<Oid> = std::collections::HashSet::new();
            for oid in walk {
                let oid = oid?;
                let commit = repo.find_commit(oid)?;
                if commit.parent_count() > 1 {
                    return Err(GitEngineError::InvalidOperation(
                        "Interactive rebase across a merge commit is not supported.".into(),
                    ));
                }
                range.insert(oid);
            }

            // Data-loss protection: the step set MUST match the range exactly.
            let mut plan_set: std::collections::HashSet<Oid> = std::collections::HashSet::new();
            let mut lines = String::new();
            let mut msg_files = TempFiles(Vec::new());
            for s in steps {
                let oid = Oid::from_str(&s.commit_id)?;
                if !range.contains(&oid) {
                    return Err(GitEngineError::InvalidOperation(format!(
                        "Commit {} is not in the rebase range",
                        &s.commit_id[..s.commit_id.len().min(8)]
                    )));
                }
                if !plan_set.insert(oid) {
                    return Err(GitEngineError::InvalidOperation(
                        "A commit appears twice in the plan".into(),
                    ));
                }
                // An author change only makes sense for pick/reword (squash/fixup
                // fall into another commit, drop discards) — otherwise the value
                // would be silently lost.
                if s.author.is_some() && s.action != "pick" && s.action != "reword" {
                    return Err(GitEngineError::InvalidOperation(
                        "Author is only allowed with pick/reword".into(),
                    ));
                }
                // amend when the message (reword) OR the author of a pick is to be
                // changed.
                // Reword without an editor: as pick + amend with the message from a
                // temp file (GIT_EDITOR=true would swallow any editing). File
                // quoting via sh_single_quote + slashes (the exec line runs in
                // git's POSIX sh).
                let want_amend = s.action == "reword" || (s.author.is_some() && s.action == "pick");
                if want_amend {
                    // Quote the author safely (the exec line runs in POSIX sh).
                    let author_arg = match s.author.as_deref() {
                        Some(a) => {
                            if a.contains('\'')
                                || a.trim_start().starts_with('-')
                                || a.chars().any(|c| c.is_ascii_control())
                            {
                                return Err(GitEngineError::InvalidOperation(
                                    "Invalid author (quotes / leading '-' / control characters)"
                                        .into(),
                                ));
                            }
                            // Additionally demand a well-formed ident: exactly ONE
                            // "<", a trailing ">", a non-empty name part before it
                            // and a non-empty email part in between.
                            let t = a.trim();
                            let open = t.find('<');
                            let well_formed = matches!(open, Some(i)
                                if t.ends_with('>')
                                    && t.matches('<').count() == 1
                                    && t.matches('>').count() == 1
                                    && !t[..i].trim().is_empty()
                                    && !t[i + 1..t.len() - 1].trim().is_empty());
                            if !well_formed {
                                return Err(GitEngineError::InvalidOperation(
                                    "Invalid author: expected \"Name <email>\"".into(),
                                ));
                            }
                            format!(" --author='{a}'")
                        }
                        None => String::new(),
                    };
                    let msg_part = match s.message.as_deref().filter(|m| !m.trim().is_empty()) {
                        Some(msg) => {
                            // Create it safely (O_EXCL + 0600). The file is created
                            // in temp_dir but copied by the sequence editor into
                            // git's rebase-merge and read ONLY from there — that way
                            // git cleans it up itself at the end of the rebase.
                            let file = sidecar::write_secure_temp(
                                "terra-git-reword",
                                "txt",
                                msg.as_bytes(),
                            )?;
                            // A self-generated ASCII name with a counter: no quoting
                            // risk in the exec line, and no collision with git's own
                            // files in rebase-merge.
                            let name = format!("terra-msg-{}.txt", msg_files.0.len() + 1);
                            msg_files.0.push((file, name.clone()));
                            format!("-F \"$(git rev-parse --git-path rebase-merge/{name})\"")
                        }
                        None => "--no-edit".to_string(),
                    };
                    lines.push_str(&format!("pick {oid}\n"));
                    lines.push_str(&format!("exec git commit --amend {msg_part}{author_arg}\n"));
                } else {
                    lines.push_str(&format!("{} {}\n", s.action, oid));
                }
            }
            if plan_set != range {
                return Err(GitEngineError::InvalidOperation(
                    "The plan does not cover exactly all commits of the range (data-loss protection)."
                        .into(),
                ));
            }
            // Back up only AFTER all validations (range/merge/plan) — otherwise a
            // rejection would leave an orphaned backup ref behind.
            create_backup_ref(&repo, "rebase-interactive")?;
            (base_oid, (lines, std::mem::take(&mut msg_files.0)))
        };
        let (todo_lines, msg_files) = todo;

        // The message files move into git's rebase-merge and are disposed of by
        // git there — even when the rebase pauses on a conflict and the user only
        // finishes it much later (or externally).
        let base_arg = base_oid.map_or_else(|| "--root".to_string(), |o| o.to_string());
        sidecar::rebase_interactive(path, &base_arg, &todo_lines, &msg_files)
    }

    // --- Undo/redo executor ---

    fn apply_undo_action(
        &self,
        path: &Path,
        action: &UndoAction,
        expected_tip: Option<&str>,
    ) -> Result<()> {
        match action {
            UndoAction::ResetBranch {
                branch,
                commit,
                mode,
            } => {
                let repo = self.open_repo_at(path)?;
                if repo.state() != RepositoryState::Clean {
                    return Err(GitEngineError::InvalidOperation(
                        "Finish or abort the running operation first".into(),
                    ));
                }
                // The reset always hits the branch HEAD sits on — it may only run
                // as long as that is still the recorded one.
                let head = repo.head()?;
                if !head.is_branch() || head.shorthand().ok() != Some(branch.as_str()) {
                    return Err(GitEngineError::InvalidOperation(format!(
                        "Undo is only possible on the original branch: {branch}"
                    )));
                }
                // Staleness guard UNDER the caller's lock: the app's
                // pre-check runs outside the INDEX_LOCK — a second command can
                // commit in the await window in between. Only the comparison HERE
                // reliably prevents the reset from throwing away a foreign commit.
                if let Some(expected) = expected_tip {
                    let tip = head.target().map(|oid| oid.to_string());
                    if tip.as_deref() != Some(expected) {
                        return Err(GitEngineError::UndoStale);
                    }
                }
                let target = repo.find_commit(Oid::from_str(commit)?)?;
                let reset_type = match mode {
                    ResetMode::Soft => ResetType::Soft,
                    ResetMode::Mixed => ResetType::Mixed,
                    ResetMode::Hard => {
                        // A hard reset overwrites index + workdir. Allowed only
                        // when nothing unsaved is lost in the process: either the
                        // tree is clean, or index and workdir already match the
                        // target commit exactly (redo right after a soft undo).
                        // Untracked files are left alone by reset --hard — except
                        // at paths the target holds, which it replaces silently.
                        let clean = uncommitted_paths(self, path)?.is_empty();
                        if !clean && !worktree_matches_commit(&repo, &target)? {
                            return Err(GitEngineError::InvalidOperation(
                                "Uncommitted changes in the working directory — \
                                 stash or discard them first"
                                    .into(),
                            ));
                        }
                        refuse_overwritten_untracked(&repo, &target)?;
                        ResetType::Hard
                    }
                };
                repo.reset(target.as_object(), reset_type, None)?;
                if reset_type == ResetType::Hard {
                    crate::lfs::restore_content(&repo, None);
                }
                Ok(())
            }
            UndoAction::UncommitRoot { branch, commit } => {
                let repo = self.open_repo_at(path)?;
                if repo.state() != RepositoryState::Clean {
                    return Err(GitEngineError::InvalidOperation(
                        "Finish or abort the running operation first".into(),
                    ));
                }
                uncommit_root(&repo, branch, commit)
            }
            UndoAction::RecreateBranch { name, commit } => {
                let repo = self.open_repo_at(path)?;
                if repo.find_branch(name, BranchType::Local).is_ok() {
                    return Err(GitEngineError::InvalidOperation(format!(
                        "Branch \u{201c}{name}\u{201d} already exists"
                    )));
                }
                let target = repo.find_commit(Oid::from_str(commit)?)?;
                repo.branch(name, &target, false)?;
                Ok(())
            }
            // delete_branch checks itself that it is not the current branch;
            // force, because the redo of a deletion must not fail on the merged
            // check (the user has already confirmed it).
            UndoAction::DeleteBranch { name } => self.delete_branch(path, name, true),
            UndoAction::Checkout { target } => self.checkout_branch(path, target),
            UndoAction::RestoreStash { message, commit } => {
                // The commit goes positionally to the git CLI — validate strictly.
                validate_commit_hex(commit)?;
                sidecar::stash_store(path, message, commit)?;
                Ok(())
            }
            // Never forced from the undo stack, so no overwrite snapshot (and
            // no limit) comes into play here.
            UndoAction::RestoreDiscarded { reference } => {
                self.restore_discarded(path, reference, false, u64::MAX)
            }
            UndoAction::ReapplyDiscarded { reference } => {
                let repo = self.open_repo_at(path)?;
                let workdir = discard_workdir(&repo)?;
                discarded::reapply(&repo, &workdir, reference, false, u64::MAX)
            }
            UndoAction::DropStashByCommit { commit } => {
                // Stash indices shift; the commit id is stable. Prefix comparison
                // in both directions (the stored id can be shorter or longer than
                // the listed one).
                validate_commit_hex(commit)?;
                let entry = self.stash_list(path)?.into_iter().find(|s| {
                    s.id.starts_with(commit.as_str()) || commit.starts_with(s.id.as_str())
                });
                match entry {
                    Some(s) => self.stash_drop(path, s.index),
                    None => Err(GitEngineError::InvalidOperation(
                        "Stash no longer exists".into(),
                    )),
                }
            }
        }
    }

    fn unpushed_commits(&self, path: &Path) -> Result<Vec<UnpushedCommit>> {
        let repo = self.open_repo_at(path)?;
        // Fresh repo without a commit (unborn HEAD): there is nothing unpushed.
        // Mirrors the search_log handling in this file.
        match repo.head() {
            Ok(_) => {}
            Err(e) if is_unborn(&e) => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        }
        let mut walk = repo.revwalk()?;
        walk.set_sorting(git2::Sort::TOPOLOGICAL)?; // children before parents = newest first
        walk.push_head()?;
        // Hide everything that sits on a remote-tracking branch.
        for r in repo.references_glob("refs/remotes/*")?.flatten() {
            if let Some(oid) = r.target() {
                let _ = walk.hide(oid);
            }
        }
        let mut out = Vec::new();
        for oid in walk {
            let oid = oid?;
            let c = repo.find_commit(oid)?;
            let full = c.message().unwrap_or("");
            let body = full
                .split_once('\n')
                .map(|(_, rest)| rest.trim())
                .unwrap_or("")
                .to_string();
            let author = c.author();
            out.push(UnpushedCommit {
                id: oid.to_string(),
                subject: c.summary().ok().flatten().unwrap_or("").to_string(),
                body,
                author_name: author.name().unwrap_or("").to_string(),
                author_email: author.email().unwrap_or("").to_string(),
                time: author.when().seconds(),
                parent_ids: c.parent_ids().map(|p| p.to_string()).collect(),
                is_head: false,
                is_merge: c.parent_count() > 1,
            });
        }
        if let Some(first) = out.first_mut() {
            first.is_head = true;
        }
        Ok(out)
    }
}

impl BisectOps for Git2Engine {
    // --- Bisect ---

    fn bisect_start(&self, path: &Path, good: &str, bad: Option<&str>) -> Result<String> {
        validate_commit_hex(good)?;
        let bad_ref = match bad {
            Some(b) => {
                validate_commit_hex(b)?;
                b.to_string()
            }
            None => "HEAD".to_string(),
        };
        // `git bisect start <bad> <good>`: the first rev is bad, the rest good.
        // LC_ALL=C: the output ("Bisecting: …", "… is the first 'bad' commit") is
        // parsed in the frontend by English phrases — without a forced C locale
        // that fails on a non-English git.
        sidecar::run_git_long_env(
            path,
            &["bisect", "start", &bad_ref, good],
            &[("LC_ALL", "C")],
        )
    }

    fn bisect_mark(&self, path: &Path, action: &str) -> Result<String> {
        if !matches!(action, "good" | "bad" | "skip") {
            return Err(GitEngineError::InvalidOperation(format!(
                "Invalid bisect action: {action}"
            )));
        }
        // LC_ALL=C: see bisect_start — the next "Bisecting: …" line or
        // "… is the first 'bad' commit" has to stay English (locale-free parsing).
        sidecar::run_git_long_env(path, &["bisect", action], &[("LC_ALL", "C")])
    }

    fn bisect_reset(&self, path: &Path) -> Result<()> {
        sidecar::run_git_long_env(path, &["bisect", "reset"], &[("LC_ALL", "C")]).map(|_| ())
    }
}

/// From this many paths on, staging reports progress: below it, the call is
/// over before a progress bar could mean anything. The app uses the same bound
/// to decide when staging is a visible, cancellable operation.
pub const STAGE_PROGRESS_MIN: usize = 200;

impl StagingOps for Git2Engine {
    fn stage_with_progress(
        &self,
        path: &Path,
        files: &[String],
        cancel: &crate::CancelToken,
        on: &mut dyn FnMut(tg_domain::GitProgress),
    ) -> Result<()> {
        if files.len() < STAGE_PROGRESS_MIN {
            return self.stage(path, files);
        }
        let total = files.len();
        // One event per percent is plenty — 25k events would only flood IPC.
        let mut last_percent = None;
        self.stage_reporting(path, files, Some(cancel), &mut |done| {
            let percent = (done * 100 / total) as u8;
            if last_percent != Some(percent) {
                last_percent = Some(percent);
                on(tg_domain::GitProgress {
                    phase: "staging".to_string(),
                    percent,
                    done: Some(done as u64),
                    total: Some(total as u64),
                });
            }
        })
    }

    fn status_numstat(&self, path: &Path) -> Result<Vec<FileLineStats>> {
        let repo = self.open_repo_at(path)?;
        // Unborn HEAD (fresh repo): diff against the empty tree — everything
        // counts as an addition (matching the status, which already shows entries
        // there too).
        let head_tree = match repo.head() {
            Ok(h) => Some(h.peel_to_tree()?),
            Err(e) if is_unborn(&e) => None,
            Err(e) => return Err(e.into()),
        };
        let mut opts = git2::DiffOptions::new();
        opts.include_untracked(true)
            .recurse_untracked_dirs(true)
            .show_untracked_content(true)
            .include_typechange(true);
        let mut diff = repo.diff_tree_to_workdir_with_index(head_tree.as_ref(), Some(&mut opts))?;
        // Fold renames together so the balance matches the status model (a renamed
        // file = ONE entry under the new path, not a full deletion + a full
        // addition).
        // Same rename budget as the status (crate::RENAME_PAIR_BUDGET): beyond
        // it, find_similar compares every added with every deleted file.
        let (mut added, mut deleted) = (0, 0);
        for delta in diff.deltas() {
            match delta.status() {
                git2::Delta::Added | git2::Delta::Untracked => added += 1,
                git2::Delta::Deleted => deleted += 1,
                _ => {}
            }
        }
        if crate::rename_affordable(added, deleted) {
            let mut find = git2::DiffFindOptions::new();
            find.renames(true);
            diff.find_similar(Some(&mut find))?;
        }
        let mut out = Vec::with_capacity(diff.deltas().len());
        for (i, delta) in diff.deltas().enumerate() {
            let Some(p) = delta.new_file().path().or_else(|| delta.old_file().path()) else {
                continue;
            };
            let path_str = p.to_string_lossy().into_owned();
            match git2::Patch::from_diff(&diff, i)? {
                Some(patch) => {
                    // line_stats: (context, additions, deletions).
                    let (_, added, deleted) = patch.line_stats()?;
                    // The flags are only reliable AFTER the patch was produced
                    // (binary detection happens while loading the contents).
                    let binary = patch.delta().flags().is_binary();
                    out.push(FileLineStats {
                        path: path_str,
                        added: added as u32,
                        deleted: deleted as u32,
                        binary,
                    });
                }
                None => out.push(FileLineStats {
                    path: path_str,
                    added: 0,
                    deleted: 0,
                    binary: true,
                }),
            }
        }
        Ok(out)
    }

    // --- Hunk/line staging ---

    fn apply_hunk(&self, path: &Path, file: &str, hunk_index: usize, unstage: bool) -> Result<()> {
        if !unstage {
            self.ensure_diffable(path, file)?;
        }
        let patch = self.file_patch(path, file, unstage)?;
        let (header, hunk) = self.select_hunk(&patch, hunk_index)?;
        sidecar::apply_patch(path, &format!("{header}{hunk}"), true, unstage)?;
        Ok(())
    }

    fn discard_hunk(&self, path: &Path, file: &str, hunk_index: usize) -> Result<()> {
        let patch = self.file_patch(path, file, false)?;
        let (header, hunk) = self.select_hunk(&patch, hunk_index)?;
        // Reverse onto the workdir = discard the change.
        sidecar::apply_patch(path, &format!("{header}{hunk}"), false, true)?;
        Ok(())
    }

    fn apply_lines(
        &self,
        path: &Path,
        file: &str,
        hunk_index: usize,
        line_indices: &[usize],
        unstage: bool,
    ) -> Result<()> {
        if !unstage {
            self.ensure_diffable(path, file)?;
        }
        let patch = self.file_patch(path, file, unstage)?;
        let (header, hunk) = self.select_hunk(&patch, hunk_index)?;
        let partial = build_partial_hunk(&hunk, line_indices, unstage)?;
        sidecar::apply_patch(path, &format!("{header}{partial}"), true, unstage)?;
        Ok(())
    }
}

fn discard_workdir(repo: &git2::Repository) -> Result<std::path::PathBuf> {
    repo.workdir()
        .map(Path::to_path_buf)
        .ok_or_else(|| GitEngineError::NotARepository("no workdir".into()))
}

impl Git2Engine {
    /// [`DiscardOps::discard_protected`] for large sets: a file counter while
    /// the copy is written (phase `backup`, cancellable — nothing is discarded
    /// yet) and while the state after the discard is recorded (phase
    /// `discarding`). Below [`STAGE_PROGRESS_MIN`] files no events are sent.
    pub fn discard_protected_with_progress(
        &self,
        path: &Path,
        files: &[String],
        limit_bytes: u64,
        cancel: &crate::CancelToken,
        on: &mut dyn FnMut(tg_domain::GitProgress),
    ) -> Result<String> {
        let repo = self.open_repo_at(path)?;
        let workdir = discard_workdir(&repo)?;
        // A discarded rename also brings the OLD path back — it belongs into
        // the snapshot, otherwise a restore could not remove it again.
        let renames = crate::rename_pairs(&repo, false)?;
        let mut wanted: Vec<String> = files.to_vec();
        wanted.extend(files.iter().filter_map(|f| renames.get(f).cloned()));
        // A file standing where a restored path needs its folder (`d` instead
        // of `d/a`) has to go — it belongs into the copy too.
        let blocking = discarded::blocking_ancestors(&workdir, &wanted);
        wanted.extend(blocking.iter().cloned());
        let paths = discarded::expand_paths(&workdir, &wanted)?;
        discarded::remove_broken(&repo);
        discarded::ensure_room(
            &repo,
            discarded::estimate(&repo, &workdir, &paths),
            limit_bytes,
        )?;

        let total = paths.len();
        let report = total >= STAGE_PROGRESS_MIN;
        let mut last = None;
        let mut progress = |phase: &str, done: usize| {
            if !report {
                return;
            }
            // One event per percent and phase — not one per file.
            let percent = (done * 100 / total) as u8;
            if last != Some((phase.to_string(), percent)) {
                last = Some((phase.to_string(), percent));
                on(tg_domain::GitProgress {
                    phase: phase.to_string(),
                    percent,
                    done: Some(done as u64),
                    total: Some(total as u64),
                });
            }
        };
        let name =
            discarded::begin_reporting(&repo, &workdir, "files", &paths, Some(cancel), &mut |d| {
                progress("backup", d)
            })?;
        // The copy holds them now. libgit2 replaces such a file by the folder
        // only with `core.ignorecase` (Windows, macOS); a case-sensitive repo
        // (Linux) fails with "failed to make directory … directory exists" —
        // removed here, the checkout behaves the same everywhere.
        for file in &blocking {
            std::fs::remove_file(discarded::check_path(&workdir, file)?)?;
        }
        self.discard(path, files)?;
        discarded::finish_reporting(&repo, &workdir, &name, &mut |d| progress("discarding", d))?;
        Ok(name)
    }
}

impl DiscardOps for Git2Engine {
    fn discard_protected(&self, path: &Path, files: &[String], limit_bytes: u64) -> Result<String> {
        self.discard_protected_with_progress(
            path,
            files,
            limit_bytes,
            &crate::CancelToken::new(),
            &mut |_| {},
        )
    }

    fn discard_hunk_protected(
        &self,
        path: &Path,
        file: &str,
        hunk_index: usize,
        limit_bytes: u64,
    ) -> Result<String> {
        let repo = self.open_repo_at(path)?;
        let workdir = discard_workdir(&repo)?;
        let paths = vec![file.to_string()];
        discarded::remove_broken(&repo);
        // Both sides of a hunk discard are worktree contents git does not hold
        // yet: before AND after get copied, each about the file's size.
        let needed = discarded::worktree_bytes(&workdir, &paths).saturating_mul(2);
        discarded::ensure_room(&repo, needed, limit_bytes)?;
        let name = discarded::begin(&repo, &workdir, "hunk", &paths)?;
        self.discard_hunk(path, file, hunk_index)?;
        discarded::finish(&repo, &workdir, &name)?;
        Ok(name)
    }

    fn discarded(&self, path: &Path) -> Result<Vec<DiscardedInfo>> {
        discarded::list(&self.open_repo_at(path)?)
    }

    fn restore_discarded(
        &self,
        path: &Path,
        name: &str,
        force: bool,
        limit_bytes: u64,
    ) -> Result<()> {
        let repo = self.open_repo_at(path)?;
        let workdir = discard_workdir(&repo)?;
        discarded::restore(&repo, &workdir, name, force, limit_bytes)
    }

    fn delete_discarded(&self, path: &Path, name: &str) -> Result<()> {
        discarded::delete(&self.open_repo_at(path)?, name)
    }
}

impl RemoteOps for Git2Engine {
    // --- Remotes & sync ---

    fn remotes(&self, path: &Path) -> Result<Vec<RemoteInfo>> {
        let repo = self.open_repo_at(path)?;
        let names = repo.remotes()?;
        let mut result = Vec::new();
        for name in names.iter().filter_map(|n| n.ok().flatten()) {
            if let Ok(remote) = repo.find_remote(name) {
                result.push(RemoteInfo {
                    name: name.to_string(),
                    url: remote.url().unwrap_or("").to_string(),
                });
            }
        }
        Ok(result)
    }

    fn push_remote(&self, path: &Path, remote: &str, force: bool) -> Result<String> {
        let repo = self.open_repo_at(path)?;
        let branch_name = match repo.head() {
            Ok(head) if head.is_branch() => head.shorthand().ok().map(str::to_owned),
            _ => None,
        };
        drop(repo);
        // Always push explicitly to THE chosen remote (not to the upstream).
        sidecar::push_to(path, remote, branch_name.as_deref(), force)
    }

    fn add_remote(&self, path: &Path, name: &str, url: &str) -> Result<()> {
        // Do not anchor ext::/fd:: transports or option URLs in the config
        // (defense in depth on top of the ext:: guard on every remote op).
        sidecar::validate_remote_url(url)?;
        let repo = self.open_repo_at(path)?;
        if repo.find_remote(name).is_ok() {
            return Err(GitEngineError::InvalidOperation(format!(
                "Remote \u{201c}{name}\u{201d} already exists"
            )));
        }
        repo.remote(name, url)?;
        Ok(())
    }

    fn remove_remote(&self, path: &Path, name: &str) -> Result<()> {
        let repo = self.open_repo_at(path)?;
        // find_remote first: it gives a clear error message for unknown names.
        repo.find_remote(name)?;
        repo.remote_delete(name)?;
        Ok(())
    }

    fn rename_remote(&self, path: &Path, old: &str, new: &str) -> Result<()> {
        let repo = self.open_repo_at(path)?;
        if repo.find_remote(new).is_ok() {
            return Err(GitEngineError::InvalidOperation(format!(
                "Remote \u{201c}{new}\u{201d} already exists"
            )));
        }
        repo.find_remote(old)?;
        let problems = repo.remote_rename(old, new)?;
        // Non-standard refspecs that git2 could not rewrite automatically are no
        // reason to abort — but they belong in the log.
        for p in problems.iter().filter_map(|p| p.ok().flatten()) {
            tracing::warn!(refspec = p, "Refspec not adjusted during the remote rename");
        }
        Ok(())
    }

    fn set_remote_url(&self, path: &Path, name: &str, url: &str) -> Result<()> {
        sidecar::validate_remote_url(url)?;
        let repo = self.open_repo_at(path)?;
        repo.find_remote(name)?;
        repo.remote_set_url(name, url)?;
        Ok(())
    }
}

impl BackupOps for Git2Engine {
    // --- Backups (backup refs) ---

    fn backups(&self, path: &Path) -> Result<Vec<BackupInfo>> {
        let repo = self.open_repo_at(path)?;
        let mut result = Vec::new();
        for r in repo.references_glob("refs/terra-git/backup/*")?.flatten() {
            let Ok(name) = r.name() else { continue };
            let short = name.trim_start_matches(BACKUP_REF_PREFIX);
            let (op, timestamp) = parse_backup_ref_name(short);
            let Ok(commit) = r.peel_to_commit() else {
                continue;
            };
            result.push(BackupInfo {
                name: name.to_string(),
                op,
                timestamp,
                target_id: commit.id().to_string(),
                subject: commit.summary().ok().flatten().unwrap_or("").to_string(),
            });
        }
        result.sort_by(|a, b| b.timestamp.cmp(&a.timestamp).then(b.name.cmp(&a.name)));
        Ok(result)
    }

    fn restore_backup(&self, path: &Path, ref_name: &str) -> Result<String> {
        let repo = self.open_repo_at(path)?;
        let reference = find_backup_ref(&repo, ref_name)?;
        if repo.state() != RepositoryState::Clean {
            return Err(GitEngineError::InvalidOperation(
                "Finish or abort the running operation (merge/rebase) first".into(),
            ));
        }
        // The restore hard-resets the branch. create_backup_ref only backs up the
        // committed HEAD — uncommitted changes in the working directory would be
        // lost and would be in NO backup. Hence (as with the hard-reset undo, see
        // apply_undo_action) reject a dirty worktree — and untracked files the
        // reset would replace.
        if !uncommitted_paths(self, path)?.is_empty() {
            return Err(GitEngineError::InvalidOperation(
                "Uncommitted changes in the working directory — \
                 stash or discard them first"
                    .into(),
            ));
        }
        let commit = reference.peel_to_commit()?;
        refuse_overwritten_untracked(&repo, &commit)?;
        // Back up the current state — the restore itself stays undoable.
        create_backup_ref(&repo, "restore")?;
        repo.reset(commit.as_object(), ResetType::Hard, None)?;
        crate::lfs::restore_content(&repo, None);
        Ok(commit.id().to_string())
    }

    fn delete_backup(&self, path: &Path, ref_name: &str) -> Result<()> {
        let repo = self.open_repo_at(path)?;
        let mut reference = find_backup_ref(&repo, ref_name)?;
        reference.delete()?;
        Ok(())
    }
}

impl RepoLifecycleOps for Git2Engine {
    // --- Repo lifecycle ---

    fn init_repo(&self, dir: &Path) -> Result<RepoInfo> {
        std::fs::create_dir_all(dir)?;
        // Fallback "main" instead of libgit2's "master".
        // A user-set init.defaultBranch configuration still wins — our fallback
        // only applies when NONE is set (initial_head would otherwise silently
        // override the configuration).
        let configured = git2::Config::open_default()
            .ok()
            .and_then(|mut c| c.snapshot().ok())
            .and_then(|c| c.get_string("init.defaultBranch").ok())
            .filter(|s| !s.trim().is_empty());
        let mut opts = git2::RepositoryInitOptions::new();
        if configured.is_none() {
            opts.initial_head("main");
        }
        git2::Repository::init_opts(dir, &opts)?;
        self.open_repo(dir)
    }

    fn ignore_pattern(&self, path: &Path, pattern: &str) -> Result<()> {
        let repo = self.open_repo_at(path)?;
        let workdir = repo
            .workdir()
            .ok_or_else(|| GitEngineError::NotARepository("no workdir".into()))?;
        // A newline would let the caller append arbitrary further lines — and a
        // .gitignore is a plausible target for `include.path`, which would turn
        // free text into executable git config. One pattern per call, no breaks.
        if pattern.contains('\n') || pattern.contains('\r') {
            return Err(GitEngineError::InvalidOperation(format!(
                "Ignore pattern must be a single line: {pattern:?}"
            )));
        }
        let file = workdir.join(".gitignore");
        // ONLY "file does not exist" may count as empty. A real read error
        // (non-UTF-8 content, locked) must NOT become unwrap_or_default() —
        // otherwise the following write would replace the whole .gitignore.
        let mut content = match std::fs::read_to_string(&file) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e.into()),
        };
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(pattern);
        content.push('\n');
        std::fs::write(&file, content)?;
        Ok(())
    }
}

impl ViewOps for Git2Engine {
    // --- Views ---

    fn blame_file(&self, path: &Path, file: &str) -> Result<Vec<BlameLine>> {
        const MAX_BLAME_LINES: usize = 5000;

        // It has to exist in HEAD — otherwise there is nothing to blame (the same
        // statement as before, but without invoking the crashing libgit2 blame).
        let repo = self.open_repo_at(path)?;
        if self.blob_bytes_at_head(&repo, file).is_none() {
            return Err(GitEngineError::InvalidOperation(format!(
                "\u{201c}{file}\u{201d} is not committed yet — blame is not available"
            )));
        }
        drop(repo);

        // Deliberately the sidecar instead of git2: libgit2's
        // `git_blame_get_hunk_byline` segfaults (ACCESS_VIOLATION) on real
        // multi-hunk blames of this repo — the internally reported hunk table is
        // partly null. `git blame` delivers the same result correctly and
        // completely.
        //
        // Blame `HEAD` explicitly (not the worktree): a pure EOL deviation
        // (CRLF worktree vs. LF blob, see the known project trap) would otherwise
        // mark every line as "Not Committed Yet". The UI shows the HEAD state
        // anyway.
        let out = sidecar::run_git_raw(
            path,
            &["blame", "--porcelain", "--no-textconv", "HEAD", "--", file],
        )?;
        Ok(parse_blame_porcelain(&out, MAX_BLAME_LINES))
    }

    fn explain_unchanged(&self, path: &Path, file: &str, staged: bool) -> Result<UnchangedInfo> {
        let repo = self.open_repo_at(path)?;

        let empty = UnchangedInfo {
            reason: UnchangedReason::Unknown,
            old_eol: None,
            new_eol: None,
            expected_eol: None,
            old_mode: None,
            new_mode: None,
        };

        // Path guard: git2::Index::get_path panics on absolute paths and on paths
        // containing "..". The path comes in over IPC, so catch it here.
        let rel = Path::new(file);
        if rel.is_absolute()
            || rel
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Ok(empty);
        }

        // 1. Read both sides — the way file_diff compares them.
        //    If one side is missing, the file was created, deleted or is in
        //    conflict. Then it is NOT unchanged, and we claim nothing: the mode
        //    comparison further down would otherwise mark a real change as
        //    harmless (a missing side = mode 0).
        let (old, new) = if staged {
            (
                self.blob_bytes_at_head(&repo, file),
                self.blob_bytes_in_index(&repo, file),
            )
        } else {
            let workdir = Self::workdir(&repo)?;
            let full_path = workdir.join(file);
            // Size guard as in the crate's other workdir readers: up to four full
            // copies are in memory here at the same time.
            let too_large = std::fs::metadata(&full_path)
                .map(|m| m.len() as usize > MAX_UNCHANGED_BYTES)
                .unwrap_or(true);
            if too_large {
                return Ok(empty);
            }
            (
                self.blob_bytes_in_index(&repo, file),
                std::fs::read(&full_path).ok(),
            )
        };
        let (Some(old), Some(new)) = (old, new) else {
            return Ok(empty);
        };

        if old == new {
            // 2. The content is byte-equal. Only NOW is the statement "only the
            //    mode differs" permissible at all.
            if let Some((m_old, m_new)) = self.delta_modes(&repo, file, staged) {
                let (a, n) = (u32::from(m_old), u32::from(m_new));
                // Mode 0 means "the side does not exist" — no mode change.
                if a != n && a != 0 && n != 0 {
                    return Ok(UnchangedInfo {
                        reason: UnchangedReason::ModeOnly,
                        old_mode: Some(mode_text(m_old)),
                        new_mode: Some(mode_text(m_new)),
                        ..empty
                    });
                }
            }

            // 3. Despite equal bytes the cause can lie in the line endings: when a
            //    checkout would write something other than what is on disk, Git
            //    reports the file for exactly that reason. That was the case
            //    actually observed.
            //
            //    old_eol deliberately stays empty here: both sides are the same
            //    content, a side-by-side comparison would inevitably be identical
            //    and would contradict the heading.
            let actual = eol_style(&new);
            if !staged {
                if let Some(expected) = expected_line_endings(&repo, file) {
                    if actual != expected && actual != EolStyle::None {
                        return Ok(UnchangedInfo {
                            reason: UnchangedReason::EolOnly,
                            new_eol: Some(actual),
                            expected_eol: Some(expected),
                            ..empty
                        });
                    }
                }
            }
            return Ok(UnchangedInfo {
                reason: UnchangedReason::Identical,
                ..empty
            });
        }

        // 3. The bytes differ. Exclude binary content consistently here: it also
        //    has zero hunks, and a coincidental CR pattern would otherwise
        //    mislabel it as a line-ending problem.
        if is_binary(&old) || is_binary(&new) {
            return Ok(empty);
        }

        if strip_cr_before_lf(&old) == strip_cr_before_lf(&new) {
            return Ok(UnchangedInfo {
                reason: UnchangedReason::EolOnly,
                old_eol: Some(eol_style(&old)),
                new_eol: Some(eol_style(&new)),
                expected_eol: if staged {
                    None
                } else {
                    expected_line_endings(&repo, file)
                },
                ..empty
            });
        }

        Ok(empty)
    }

    fn image_diff(&self, path: &Path, file: &str, staged: bool) -> Result<ImageDiff> {
        let mime = image_mime(file).ok_or_else(|| {
            GitEngineError::InvalidOperation(format!("Not an image file: {file}"))
        })?;
        let repo = self.open_repo_at(path)?;
        let workdir = repo
            .workdir()
            .ok_or_else(|| GitEngineError::NotARepository("no workdir".into()))?;

        let (old_bytes, new_bytes) = if staged {
            (
                self.blob_bytes_at_head(&repo, file),
                self.blob_bytes_in_index(&repo, file),
            )
        } else {
            (
                self.blob_bytes_in_index(&repo, file)
                    .or_else(|| self.blob_bytes_at_head(&repo, file)),
                read_workdir_image(workdir, file),
            )
        };

        // Do not push oversized images across the IPC boundary as base64
        // (memory/freeze) — analogous to the diff/blame caps.
        Ok(ImageDiff {
            old_data_url: cap_image(old_bytes).map(|b| to_data_url(mime, &b)),
            new_data_url: cap_image(new_bytes).map(|b| to_data_url(mime, &b)),
        })
    }

    fn repo_sketch(&self, path: &Path, window: usize, max_branches: usize) -> Result<RepoSketch> {
        let repo = self.open_repo_at(path)?;

        // Unborn HEAD / empty repo: an empty sketch — the UI falls back to the
        // decorative vein, that is not an error state.
        let head = repo.head();
        if let Err(e) = &head {
            if is_unborn(e) {
                return Ok(RepoSketch::default());
            }
        }
        let head_commit = head?.peel_to_commit()?;

        // Tag targets (peeled to commits) for the ochre markers.
        let mut tag_targets = std::collections::HashSet::new();
        if let Ok(refs) = repo.references_glob("refs/tags/*") {
            for r in refs.flatten() {
                if let Ok(c) = r.peel_to_commit() {
                    tag_targets.insert(c.id());
                }
            }
        }

        // HEAD line: the last `window` commits (sorted as in log()).
        let mut walk = repo.revwalk()?;
        walk.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::TIME)?;
        walk.push(head_commit.id())?;
        let mut ids = Vec::with_capacity(window);
        let mut commits = Vec::with_capacity(window);
        for oid in walk.take(window) {
            let oid = oid?;
            let c = repo.find_commit(oid)?;
            commits.push(SketchCommit {
                time: c.time().seconds(),
                is_merge: c.parent_count() > 1,
                has_tag: tag_targets.contains(&oid),
            });
            ids.push(oid);
        }

        // Local branches other than the HEAD branch: branch point + ahead count.
        // All best effort — one broken ref does not topple the sketch.
        let mut branches = Vec::new();
        for entry in repo.branches(Some(BranchType::Local))? {
            let Ok((branch, _)) = entry else { continue };
            if branch.is_head() {
                continue;
            }
            let Ok(Some(name)) = branch.name().map(|n| n.map(str::to_owned)) else {
                continue;
            };
            let Some(tip) = branch.get().target() else {
                continue;
            };
            let Ok(tip_commit) = repo.find_commit(tip) else {
                continue;
            };
            // The merge base can be missing (a second root history) — then the
            // branch point is simply "outside the window".
            let base_index = repo
                .merge_base(tip, head_commit.id())
                .ok()
                .and_then(|base| ids.iter().position(|id| *id == base))
                .map(|i| i as u32);
            let ahead = repo
                .graph_ahead_behind(tip, head_commit.id())
                .map(|(a, _)| a.min(99) as u32)
                .unwrap_or(0);
            branches.push(SketchBranch {
                name,
                base_index,
                ahead,
                tip_time: tip_commit.time().seconds(),
            });
        }
        // Newest tips first; more strands would turn the sketch into a second
        // commit graph.
        branches.sort_by_key(|b| std::cmp::Reverse(b.tip_time));
        branches.truncate(max_branches);

        Ok(RepoSketch { commits, branches })
    }
}

impl WorktreeOps for Git2Engine {
    // --- Worktrees & submodules ---

    fn worktrees(&self, path: &Path) -> Result<Vec<WorktreeInfo>> {
        let output = sidecar::run_git(path, &["worktree", "list", "--porcelain"])?;
        let mut result: Vec<WorktreeInfo> = Vec::new();
        let mut current: Option<WorktreeInfo> = None;
        for line in output.lines() {
            if let Some(p) = line.strip_prefix("worktree ") {
                if let Some(w) = current.take() {
                    result.push(w);
                }
                current = Some(WorktreeInfo {
                    path: p.to_string(),
                    branch: None,
                    head_id: None,
                    is_main: result.is_empty(),
                });
            } else if let Some(w) = current.as_mut() {
                if let Some(h) = line.strip_prefix("HEAD ") {
                    w.head_id = Some(h.to_string());
                } else if let Some(b) = line.strip_prefix("branch ") {
                    w.branch = Some(b.trim_start_matches("refs/heads/").to_string());
                }
            }
        }
        if let Some(w) = current.take() {
            result.push(w);
        }
        Ok(result)
    }

    fn add_worktree(&self, path: &Path, dest: &Path, branch: &str) -> Result<String> {
        let dest_str = dest.to_string_lossy().into_owned();
        reject_option_like(&dest_str, "worktree path")?;
        reject_option_like(branch, "branch name")?;
        // Long timeout: the full checkout into the new worktree can legitimately
        // exceed 120 s on large repos — a kill = half a worktree.
        sidecar::run_git_long(path, &["worktree", "add", &dest_str, branch])
    }

    fn remove_worktree(&self, path: &Path, worktree_path: &str) -> Result<String> {
        reject_option_like(worktree_path, "worktree path")?;
        sidecar::run_git(path, &["worktree", "remove", worktree_path])
    }

    fn submodules(&self, path: &Path) -> Result<Vec<SubmoduleInfo>> {
        let repo = self.open_repo_at(path)?;
        let mut result = Vec::new();
        for sm in repo.submodules()? {
            result.push(SubmoduleInfo {
                name: sm.name().unwrap_or("?").to_string(),
                path: sm.path().to_string_lossy().into_owned(),
                url: sm.url().ok().flatten().map(str::to_owned),
            });
        }
        Ok(result)
    }

    fn update_submodules(&self, path: &Path) -> Result<String> {
        // Clones network content -> generous timeout (not the 120 s one).
        sidecar::run_git_in(
            path,
            &["submodule", "update", "--init", "--recursive"],
            sidecar::CLONE_TIMEOUT,
        )
    }
}

impl SparseOps for Git2Engine {
    // --- Sparse checkout ---

    fn sparse_status(&self, path: &Path) -> Result<SparseStatus> {
        let (enabled, top_dirs) = {
            let repo = self.open_repo_at(path)?;
            let enabled = repo
                .config()?
                .get_bool("core.sparsecheckout")
                .unwrap_or(false);

            // Top-level directories of the HEAD tree as the basis for the
            // selection; an empty/unborn repo is NOT an error, just an empty basis.
            let mut top_dirs: Vec<String> = Vec::new();
            match repo.head() {
                Ok(head) => {
                    let tree = head.peel_to_tree()?;
                    for entry in tree.iter() {
                        if entry.kind() == Some(git2::ObjectType::Tree) {
                            if let Ok(name) = entry.name() {
                                top_dirs.push(name.to_string());
                            }
                        }
                    }
                }
                Err(e) if is_unborn(&e) => {}
                Err(e) => return Err(e.into()),
            }
            top_dirs.sort();
            (enabled, top_dirs)
        };

        // Only list when sparse checkout is active; an error (e.g. never
        // initialized, no file) simply means "no patterns".
        let patterns = if enabled {
            sidecar::sparse_checkout_list(path)
                .map(|out| {
                    out.lines()
                        .map(|l| l.trim().to_string())
                        .filter(|l| !l.is_empty())
                        .collect()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        Ok(SparseStatus {
            enabled,
            patterns,
            top_dirs,
        })
    }

    fn sparse_set(&self, path: &Path, dirs: &[String]) -> Result<()> {
        if dirs.is_empty() {
            return Err(GitEngineError::InvalidOperation(
                "Select at least one directory — or disable sparse-checkout".into(),
            ));
        }
        for dir in dirs {
            validate_sparse_dir(dir)?;
        }
        sidecar::sparse_checkout_set(path, dirs)?;
        Ok(())
    }

    fn sparse_disable(&self, path: &Path) -> Result<()> {
        sidecar::sparse_checkout_disable(path)?;
        Ok(())
    }
}

impl ConfigOps for Git2Engine {
    // --- Configuration ---

    fn config_get(&self, path: &Path, key: &str) -> Result<Option<String>> {
        let repo = self.open_repo_at(path)?;
        let config = repo.config()?;
        match config.get_string(key) {
            Ok(v) => Ok(Some(v)),
            Err(e) if e.code() == ErrorCode::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn config_set(&self, path: &Path, key: &str, value: &str, global: bool) -> Result<()> {
        // Defense in depth: command-triggering config keys are never settable
        // through the generic settings interface (see is_forbidden_config_key)
        // — and beyond that only the keys the app actually writes are. A
        // denylist always lags behind git (submodule.<n>.update = !cmd,
        // core.worktree, safe.directory slipped through it).
        if is_forbidden_config_key(key)
            || !SETTABLE_CONFIG_KEYS.contains(&key.trim().to_ascii_lowercase().as_str())
        {
            return Err(GitEngineError::InvalidOperation(format!(
                "Config key is not allowed (can execute commands): {key}"
            )));
        }
        if global {
            let mut config = git2::Config::open_default()?.open_global()?;
            config_set_or_remove(&mut config, key, value)?;
            // Remove a local override of the same key — otherwise the global value
            // stays ineffective and "save globally" seems to fail (an empty local
            // entry `key =` even masks it invisibly).
            let repo = self.open_repo_at(path)?;
            let mut local = repo.config()?.open_level(git2::ConfigLevel::Local)?;
            config_remove(&mut local, key)?;
        } else {
            let repo = self.open_repo_at(path)?;
            let mut config = repo.config()?.open_level(git2::ConfigLevel::Local)?;
            config_set_or_remove(&mut config, key, value)?;
        }
        Ok(())
    }

    fn check_signing(&self, path: &Path) -> Result<String> {
        // Signing runs through the system git on a real commit — so does this
        // test. `commit-tree -S` produces a signed, UNREFERENCED commit object
        // (attached to no ref, cleaned up by the next gc) and fails with the real
        // gpg/ssh/ident error when the configuration does not hold.
        let repo = self.open_repo_at(path)?;
        let tree = repo
            .head()
            .ok()
            .and_then(|h| h.peel_to_tree().ok())
            .ok_or_else(|| {
                GitEngineError::InvalidOperation(
                    "The signature check needs at least one commit in the repository".into(),
                )
            })?
            .id()
            .to_string();
        sidecar::run_git(
            path,
            &["commit-tree", &tree, "-S", "-m", "terra-git signature test"],
        )?;

        let config = repo.config()?.snapshot()?;
        let format = config
            .get_string("gpg.format")
            .unwrap_or_else(|_| "openpgp".into());
        Ok(format!("Signing works (format: {format})"))
    }

    // --- External tools ---

    fn open_mergetool(&self, path: &Path, file: &str) -> Result<String> {
        let literal = literal_pathspec(file);
        let repo = self.open_repo_at(path)?;
        // The repo-local level may supply the NAME — after validation it is
        // harmless, and without a tool git would guess and fail on the forced
        // prompt with stdin=null.
        let tool = repo
            .config()?
            .snapshot()?
            .get_string("merge.tool")
            .ok()
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .ok_or_else(|| {
                GitEngineError::InvalidOperation(
                    "No mergetool configured (merge.tool). Please set it in the global \
                     git configuration."
                        .into(),
                )
            })?;
        if !valid_mergetool_name(&tool) {
            return Err(GitEngineError::InvalidOperation(format!(
                "Invalid mergetool name in merge.tool: {tool}"
            )));
        }
        let cmd_key = format!("mergetool.{tool}.cmd");
        let path_key = format!("mergetool.{tool}.path");
        let trusted_cmd = trusted_config_value(&cmd_key);
        let trusted_path = trusted_config_value(&path_key);
        // A repo-local definition without a trustworthy counterpart: abort
        // immediately and explain why. The run would fail anyway (the override
        // below empties the key), but with a cryptic git message.
        let local = repo.config()?.open_level(git2::ConfigLevel::Local).ok();
        let has_local = |k: &str| local.as_ref().is_some_and(|c| c.get_string(k).is_ok());
        if trusted_cmd.is_none() && (has_local(&cmd_key) || has_local(&path_key)) {
            return Err(GitEngineError::InvalidOperation(format!(
                "This repository ships its own mergetool definition ({cmd_key}). \
                 terra-git does not run repo-locally configured commands — add the \
                 tool to your global git configuration instead."
            )));
        }
        let args = mergetool_args(
            &tool,
            trusted_cmd.as_deref(),
            trusted_path.as_deref(),
            &literal,
        );
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        // Interactive session: the 120 s hang detection would kill the external
        // tool in the middle of editing and git's post-processing (staging,
        // cleaning up .orig) would be skipped. The generous timeout is pragmatic —
        // a real no-timeout with a CancelToken would be a bigger rebuild.
        sidecar::run_git_long(path, &refs)
    }
}

/// Allowed characters of a mergetool name. git sources
/// `$(git --exec-path)/mergetools/<name>` as an sh script (git-mergetool--lib,
/// `setup_tool`) — a name with `/` or `\` would be a path traversal into an
/// attacker-controlled file. `=` is forbidden because such a name would
/// undermine the later `-c` override: git splits at the FIRST `=`.
fn valid_mergetool_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.contains("..")
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b))
}

/// Value of a key exclusively from the trustworthy config levels (global > XDG >
/// system). The repo-local level stays out — `Config::open_default()` does not
/// contain it anyway.
fn trusted_config_value(key: &str) -> Option<String> {
    let cfg = git2::Config::open_default().ok()?;
    for level in [
        git2::ConfigLevel::Global,
        git2::ConfigLevel::XDG,
        git2::ConfigLevel::System,
    ] {
        if let Ok(lvl) = cfg.open_level(level) {
            if let Ok(v) = lvl.get_string(key) {
                return Some(v);
            }
        }
    }
    None
}

/// Argument list for `git mergetool` including hardening: the tool's two
/// command-triggering keys are forced via `-c` to the trustworthy value
/// (global/system, otherwise empty), and the name goes in explicitly as
/// `--tool=` — that bypasses git's own config resolution and with it the
/// sourcing path through `merge.tool`. An EMPTY override leaves git's built-in
/// tool definitions unchanged.
fn mergetool_args(
    tool: &str,
    trusted_cmd: Option<&str>,
    trusted_path: Option<&str>,
    literal: &str,
) -> Vec<String> {
    vec![
        "-c".into(),
        format!("mergetool.{tool}.cmd={}", trusted_cmd.unwrap_or("")),
        "-c".into(),
        format!("mergetool.{tool}.path={}", trusted_path.unwrap_or("")),
        "mergetool".into(),
        format!("--tool={tool}"),
        "--no-prompt".into(),
        "--".into(),
        literal.into(),
    ]
}

/// Maps the libgit2 repository state onto the domain state.
/// The single source of truth — also used by `GitEngine::status`.
pub(crate) fn map_state(state: RepositoryState) -> RepoOpState {
    match state {
        RepositoryState::Merge => RepoOpState::Merge,
        RepositoryState::Rebase
        | RepositoryState::RebaseInteractive
        | RepositoryState::RebaseMerge => RepoOpState::Rebase,
        RepositoryState::CherryPick | RepositoryState::CherryPickSequence => {
            RepoOpState::Cherrypick
        }
        RepositoryState::Revert | RepositoryState::RevertSequence => RepoOpState::Revert,
        RepositoryState::Bisect => RepoOpState::Bisect,
        _ => RepoOpState::Clean,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_patch_separates_head_and_hunks() {
        let patch = "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1,2 +1,2 @@\n-a\n+b\n c\n@@ -5,1 +5,2 @@\n d\n+e\n";
        let (header, hunks) = split_patch(patch);
        assert!(header.starts_with("diff --git"));
        assert_eq!(hunks.len(), 2);
        assert!(hunks[0].starts_with("@@ -1,2"));
        assert!(hunks[1].starts_with("@@ -5,1"));
    }

    #[test]
    fn build_partial_hunk_stage_selected() {
        // Hunk: -old, +new1, +new2 — only +new1 (index 1) is selected.
        let hunk = "@@ -1,2 +1,3 @@\n-old\n+new1\n+new2\n end\n";
        let partial = build_partial_hunk(hunk, &[1], false).unwrap();
        // -old unselected -> context; +new2 unselected -> dropped.
        assert!(partial.contains(" old"));
        assert!(partial.contains("+new1"));
        assert!(!partial.contains("+new2"));
        assert!(partial.starts_with("@@ -1,2 +1,3 @@"));
    }

    #[test]
    fn build_partial_hunk_unstage_selected() {
        let hunk = "@@ -1,2 +1,3 @@\n-old\n+new1\n+new2\n end\n";
        let partial = build_partial_hunk(hunk, &[1], true).unwrap();
        // Reverse: -old unselected -> dropped; +new2 unselected -> context.
        assert!(!partial.contains("-old"));
        assert!(partial.contains("+new1"));
        assert!(partial.contains(" new2"));
    }

    #[test]
    fn config_denylist_blocks_command_keys_allows_harmless() {
        // Allowed: the keys actually set + typical harmless ones.
        for ok in [
            "user.name",
            "user.email",
            "commit.gpgsign",
            "core.autocrlf",
            "pull.rebase",
            "branch.main.remote",
        ] {
            assert!(!is_forbidden_config_key(ok), "{ok} should be allowed");
        }
        // Forbidden: command/program-triggering keys (case-insensitive).
        for bad in [
            "core.sshCommand",
            "CORE.PAGER",
            "core.editor",
            "core.hooksPath",
            "core.fsmonitor",
            "core.gitProxy",
            "credential.helper",
            "alias.co",
            "filter.lfs.clean",
            "filter.lfs.smudge",
            "mergetool.mine.cmd",
            "difftool.x.cmd",
            "diff.foo.command",
            "merge.bar.driver",
            "gpg.program",
            "init.templateDir",
            "uploadpack.packObjectsHook",
            // Added after review (command-triggering):
            "diff.external",
            "core.alternateRefsCommand",
            "gpg.ssh.defaultKeyCommand",
            "pager.log",
            "pager.diff",
            // Config include: loads another file that may carry any key above.
            "include.path",
            "includeIf.gitdir:~/work/.path",
            "  diff.external  ", // surrounding whitespace is trimmed
            "remote.origin.uploadpack",
            "imap.tunnel",
            // Added after the security review: `.path` is the program git runs for
            // a mergetool/difftool, and open_mergetool trusts the global level.
            "mergetool.meld.path",
            "difftool.x.path",
            "diff.foo.textconv",
            "url.https://evil/.insteadOf",
            "url.https://evil/.pushInsteadOf",
        ] {
            assert!(is_forbidden_config_key(bad), "{bad} should be forbidden");
        }
    }

    #[test]
    fn mergetool_name_validation_blocks_traversal() {
        for ok in [
            "meld",
            "vimdiff3",
            "p4merge",
            "my-tool.v2",
            "kdiff3",
            "bc_4",
        ] {
            assert!(valid_mergetool_name(ok), "{ok} should be allowed");
        }
        // git sources <exec-path>/mergetools/<name> as an sh script: a name with
        // path separators or ".." loads a foreign file. "=" would additionally
        // undermine the later `-c` override (git splits at the first "=").
        for bad in [
            "",
            "../../evil.sh",
            "..\\evil",
            "sub/tool",
            "a b",
            "x=y",
            "tool;rm",
            "$(id)",
            &"a".repeat(65),
        ] {
            assert!(!valid_mergetool_name(bad), "{bad:?} should be forbidden");
        }
    }

    #[test]
    fn mergetool_args_force_trusted_values() {
        // Without a trustworthy value both keys are emptied so a repo-local value
        // does not apply. Empty leaves git's built-in tool definitions untouched.
        assert_eq!(
            mergetool_args("evil", None, None, ":(literal)a.txt"),
            vec![
                "-c",
                "mergetool.evil.cmd=",
                "-c",
                "mergetool.evil.path=",
                "mergetool",
                "--tool=evil",
                "--no-prompt",
                "--",
                ":(literal)a.txt",
            ]
        );
        // A "=" in the VALUE is uncritical: git only splits at the first "=", the
        // rest stays the value.
        let args = mergetool_args(
            "meld",
            Some("meld --diff a=b $LOCAL"),
            Some("/usr/bin/meld"),
            ":(literal)x",
        );
        assert!(args.contains(&"mergetool.meld.cmd=meld --diff a=b $LOCAL".to_string()));
        assert!(args.contains(&"mergetool.meld.path=/usr/bin/meld".to_string()));
        assert!(args.contains(&"--tool=meld".to_string()));
    }

    #[test]
    fn parse_backup_ref_name_old_and_new() {
        // New format <op>-<unix>-<n>.
        assert_eq!(
            parse_backup_ref_name("squash-1690000000-5"),
            ("squash".to_string(), 1690000000)
        );
        assert_eq!(
            parse_backup_ref_name("rebase-interactive-1690000000-0"),
            ("rebase-interactive".to_string(), 1690000000)
        );
        // The old format <op>-<unix> (before the `-<n>` counter suffix) MUST
        // still be parsed correctly — regression protection (review finding).
        assert_eq!(
            parse_backup_ref_name("squash-1690000000"),
            ("squash".to_string(), 1690000000)
        );
        assert_eq!(
            parse_backup_ref_name("rebase-interactive-1690000000"),
            ("rebase-interactive".to_string(), 1690000000)
        );
        assert_eq!(
            parse_backup_ref_name("restore-42"),
            ("restore".to_string(), 42)
        );
    }

    #[test]
    fn build_partial_hunk_takes_new_start_as_reverse_anchor() {
        // old_start != new_start (because of preceding changes). On a reverse
        // unstage the NEW side is the anchor -> the header has to carry +14, NOT
        // +10 (former bug: old_start recycled for both sides).
        let hunk = "@@ -10,2 +14,3 @@\n-old\n+new1\n+new2\n end\n";
        let partial = build_partial_hunk(hunk, &[1], true).unwrap();
        assert!(
            partial.starts_with("@@ -10,2 +14,3 @@"),
            "Header has to carry +new_start (14), was: {}",
            partial.lines().next().unwrap_or("")
        );
    }
}
