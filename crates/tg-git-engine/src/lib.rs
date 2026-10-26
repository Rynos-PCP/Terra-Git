//! terra-git's git engine.
//!
//! Architecture: a hybrid strategy behind the
//! [`GitEngine`] trait. The current implementation [`Git2Engine`] uses git2
//! (libgit2, vendored) for all local operations and the system-git sidecar for
//! remote operations (fetch/pull/push) so the system's credential helpers and
//! SSH configuration apply. The trait keeps the door open to move read paths to
//! gitoxide (gix) later without touching app code.

pub mod cancel;
mod conflict;
pub mod discarded;
pub mod error;
pub mod lfs;
pub mod ops;
pub mod remedies;
pub mod repo_commands;
pub mod ssh;
// The sidecar (system git) is engine-INTERNAL: the app/tests never see it
// directly, only through the Git2Engine surface (clone_prepare/clone_fetch/fetch_with_progress …).
pub(crate) mod sidecar;
mod status_fast;

pub use cancel::CancelToken;
pub use ops::{prelude, GitEngineExt};
// Process state of the "TLS verification off" hosts: the app calls the
// setter at startup and on every account change — instead of env::set_var at
// runtime (UB on Unix next to Command::spawn from other threads).
pub use sidecar::set_insecure_tls_hosts;

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use git2::{
    build::CheckoutBuilder, BranchType, Delta, DiffOptions, ErrorCode, Oid, Repository, Sort,
    Status, StatusOptions, StatusShow,
};

use error::{GitEngineError, Result};
use ops::RemoteProgressOps;
use tg_domain::{
    BranchInfo, ChangeKind, CommitInfo, DiffHunk, DiffLine, FileDiff, LineKind, RepoInfo,
    RepoStatus, StatusEntry,
};

/// Abstraction of the git engine. All methods block and are kept away from the
/// UI by the app layer via `spawn_blocking`.
pub trait GitEngine: Send + Sync {
    fn open_repo(&self, path: &Path) -> Result<RepoInfo>;
    fn status(&self, path: &Path) -> Result<RepoStatus>;
    fn log(&self, path: &Path, skip: usize, limit: usize) -> Result<Vec<CommitInfo>>;
    /// Like [`log`](Self::log), but across all branches (local + remote), tags
    /// and HEAD — the data basis of the whole-repository graph in the history.
    fn log_all(&self, path: &Path, skip: usize, limit: usize) -> Result<Vec<CommitInfo>>;
    /// Diff of a single file: `staged=false` workdir↔index, `staged=true` index↔HEAD.
    fn file_diff(&self, path: &Path, file: &str, staged: bool) -> Result<Option<FileDiff>>;
    /// Diff of a commit against its first parent.
    fn commit_diff(&self, path: &Path, commit_id: &str) -> Result<Vec<FileDiff>>;
    /// Like [`commit_diff`](Self::commit_diff), but streamed file by file:
    /// `sink` receives every finished file (abort with `false`), at most
    /// `max_files` of them. Returns the TOTAL number of files in the diff so the
    /// caller can detect and report truncation.
    fn commit_diff_stream(
        &self,
        path: &Path,
        commit_id: &str,
        max_files: usize,
        sink: &mut dyn FnMut(FileDiff) -> bool,
    ) -> Result<usize>;
    fn stage(&self, path: &Path, files: &[String]) -> Result<()>;
    fn unstage(&self, path: &Path, files: &[String]) -> Result<()>;
    /// Discards workdir changes (like `git restore`); deletes untracked files.
    fn discard(&self, path: &Path, files: &[String]) -> Result<()>;
    /// Creates a commit from the index; returns the new commit id.
    fn commit(&self, path: &Path, message: &str, amend: bool) -> Result<String>;
    fn branches(&self, path: &Path) -> Result<Vec<BranchInfo>>;
    fn create_branch(&self, path: &Path, name: &str, checkout: bool) -> Result<()>;
    fn checkout_branch(&self, path: &Path, name: &str) -> Result<()>;
    fn fetch(&self, path: &Path) -> Result<String>;
    fn pull(&self, path: &Path) -> Result<String>;
    fn push(&self, path: &Path) -> Result<String>;
}

/// git2-based default engine.
#[derive(Debug, Default, Clone, Copy)]
pub struct Git2Engine;

impl Git2Engine {
    /// The staging loop behind [`GitEngine::stage`], with a running count of
    /// processed paths for `on_done` and a cancel check between paths. The
    /// index is written only at the very end, so a cancelled run stages
    /// nothing. (libgit2 stays the engine here on purpose: measured on fresh
    /// files it stages twice as fast as `git add`, which also has to write
    /// every new object — and parallel pre-writing did not help either.)
    pub(crate) fn stage_reporting(
        &self,
        path: &Path,
        files: &[String],
        cancel: Option<&CancelToken>,
        on_done: &mut dyn FnMut(usize),
    ) -> Result<()> {
        let repo = self.open(path)?;
        let workdir = Self::workdir(&repo)?;
        let mut index = repo.index()?;
        // Rename detection only when a path is not in the index — only then can
        // it be the new side of a workdir rename.
        let renames = if files
            .iter()
            .any(|f| index.get_path(Path::new(f), 0).is_none())
        {
            rename_pairs(&repo, false)?
        } else {
            HashMap::new()
        };
        // Paths behind a filter driver (Git LFS) go through `git add` after
        // the rest: libgit2 would store the content instead of the pointer.
        let mut filtered: Vec<String> = Vec::new();
        let mut done = 0;
        for file in files {
            if cancel.is_some_and(CancelToken::is_cancelled) {
                return Err(GitEngineError::Cancelled);
            }
            let rel = Path::new(file);
            let abs = workdir.join(rel);
            // Deleted files are staged by removing them from the index.
            if abs.symlink_metadata().is_ok() {
                if abs.is_file() && lfs::has_filter(&repo, rel) {
                    filtered.push(file.clone());
                } else {
                    index.add_path(rel)?;
                }
            } else {
                index.remove_path(rel)?;
            }
            // Rename: removing the old path belongs to the same status entry and
            // is staged along with it — otherwise it stays behind as an unstaged
            // deletion.
            if let Some(old) = renames.get(file) {
                index.remove_path(Path::new(old))?;
            }
            done += 1;
            on_done(done);
        }
        index.write()?;
        if !filtered.is_empty() {
            drop(index);
            drop(repo);
            lfs::stage_filtered(path, &filtered, cancel, &mut |n| on_done(done + n))?;
        }
        Ok(())
    }

    fn open(&self, path: &Path) -> Result<Repository> {
        let repo = Repository::discover(path).map_err(|e| error::open_error(path, e))?;
        if repo.is_bare() {
            return Err(GitEngineError::NotARepository(format!(
                "{} (bare repositories are not supported)",
                path.display()
            )));
        }
        Ok(repo)
    }

    fn workdir(repo: &Repository) -> Result<PathBuf> {
        repo.workdir()
            .map(Path::to_path_buf)
            .ok_or_else(|| GitEngineError::NotARepository("no workdir".into()))
    }

    /// Enables the status accelerators (fsmonitor/untrackedCache) once for large
    /// worktrees — call it when OPENING a repo, not on every internal open.
    /// Best effort (see [`sidecar::enable_status_accelerators`]).
    /// Commit-graph maintenance (the basis for streaming `log`) is orchestrated
    /// by the app layer via [`write_commit_graph`](Self::write_commit_graph) so
    /// it can report completion as an event.
    pub fn enable_status_accelerators(&self, path: &Path) {
        sidecar::enable_status_accelerators(path);
    }

    /// Writes/updates the commit graph (blocking; put it on a background task).
    /// Details in `sidecar::write_commit_graph` and docs/perf-stress-test.md.
    pub fn write_commit_graph(&self, path: &Path) -> Result<()> {
        sidecar::write_commit_graph(path)
    }

    /// Status through the system git (fast path for large worktrees).
    /// `op_state` still comes from git2 — cheap and independent of the scan.
    /// `#[doc(hidden)] pub`: directly callable for benchmarks only; the choice is
    /// normally made by [`status`](GitEngine::status).
    #[doc(hidden)]
    pub fn status_via_sidecar(&self, path: &Path) -> Result<RepoStatus> {
        let repo = self.open(path)?;
        self.status_fast_path(repo, path)
    }

    /// Core of the fast path: consumes the already open git2 handle (op_state,
    /// ahead fallback) and releases it BEFORE the system-git scan (no double
    /// index access). Without an upstream, `git status` reports no ahead count;
    /// we then count the not-yet-published commits through the git2 handle (no
    /// extra process) — mandatory before the drop, which is why the git2 look at
    /// the upstream decides.
    fn status_fast_path(&self, repo: Repository, path: &Path) -> Result<RepoStatus> {
        let op_state = crate::ops::map_state(repo.state());
        let unpushed = head_branch_without_upstream(&repo).then(|| count_unpushed(&repo));
        drop(repo); // release the git2 handle before the system-git scan runs
        let mut st = status_fast::status_via_git(path)?.into_repo_status(op_state);
        if st.branch.is_some() && st.upstream.is_none() {
            if let Some(ahead) = unpushed {
                st.ahead = ahead;
            }
        }
        Ok(st)
    }

    /// Status through libgit2 (the standard path for small/medium worktrees).
    #[doc(hidden)]
    pub fn status_git2(&self, path: &Path) -> Result<RepoStatus> {
        let repo = self.open(path)?;

        let mut opts = StatusOptions::new();
        opts.include_untracked(true)
            .recurse_untracked_dirs(true)
            .include_ignored(false)
            .exclude_submodules(true);

        // First without rename detection (cheap), then — only where affordable
        // — again with it. See RENAME_PAIR_BUDGET.
        let plain = repo.statuses(Some(&mut opts))?;
        let sides = RenameSides::affordable(&plain);
        let statuses = if sides.any() {
            drop(plain);
            opts.renames_head_to_index(sides.head_to_index)
                .renames_index_to_workdir(sides.index_to_workdir);
            repo.statuses(Some(&mut opts))?
        } else {
            plain
        };
        let mut staged = Vec::new();
        let mut unstaged = Vec::new();

        for entry in statuses.iter() {
            let st = entry.status();

            if st.is_conflicted() {
                unstaged.push(StatusEntry {
                    path: entry_path(&entry),
                    orig_path: None,
                    kind: ChangeKind::Conflicted,
                });
                continue;
            }

            if st.intersects(
                Status::INDEX_NEW
                    | Status::INDEX_MODIFIED
                    | Status::INDEX_DELETED
                    | Status::INDEX_RENAMED
                    | Status::INDEX_TYPECHANGE,
            ) {
                let (path, orig_path) = entry
                    .head_to_index()
                    .map(|d| delta_paths(&d))
                    .unwrap_or_else(|| (entry_path(&entry), None));
                let kind = if st.contains(Status::INDEX_NEW) {
                    ChangeKind::Added
                } else if st.contains(Status::INDEX_RENAMED) {
                    ChangeKind::Renamed
                } else if st.contains(Status::INDEX_DELETED) {
                    ChangeKind::Deleted
                } else if st.contains(Status::INDEX_TYPECHANGE) {
                    ChangeKind::Typechange
                } else {
                    ChangeKind::Modified
                };
                staged.push(StatusEntry {
                    path,
                    orig_path,
                    kind,
                });
            }

            if st.intersects(
                Status::WT_NEW
                    | Status::WT_MODIFIED
                    | Status::WT_DELETED
                    | Status::WT_RENAMED
                    | Status::WT_TYPECHANGE,
            ) {
                let (path, orig_path) = entry
                    .index_to_workdir()
                    .map(|d| delta_paths(&d))
                    .unwrap_or_else(|| (entry_path(&entry), None));
                let kind = if st.contains(Status::WT_NEW) {
                    ChangeKind::Untracked
                } else if st.contains(Status::WT_RENAMED) {
                    ChangeKind::Renamed
                } else if st.contains(Status::WT_DELETED) {
                    ChangeKind::Deleted
                } else if st.contains(Status::WT_TYPECHANGE) {
                    ChangeKind::Typechange
                } else {
                    ChangeKind::Modified
                };
                unstaged.push(StatusEntry {
                    path,
                    orig_path,
                    kind,
                });
            }
        }

        staged.sort_by(|a, b| a.path.cmp(&b.path));
        unstaged.sort_by(|a, b| a.path.cmp(&b.path));

        // Determine branch, upstream and ahead/behind.
        let (mut branch, mut upstream, mut ahead, mut behind) = (None, None, 0usize, 0usize);
        match repo.head() {
            Ok(head) => {
                // Detached HEAD: no branch name (shorthand would be "HEAD").
                branch = if head.is_branch() {
                    head.shorthand().ok().map(str::to_owned)
                } else {
                    None
                };
                if head.is_branch() {
                    if let (Some(name), Some(local_oid)) = (head.shorthand().ok(), head.target()) {
                        if let Ok(local) = repo.find_branch(name, BranchType::Local) {
                            if let Ok(up) = local.upstream() {
                                upstream = up.name().ok().flatten().map(str::to_owned);
                                if let Some(up_oid) = up.get().target() {
                                    let (a, b) = repo.graph_ahead_behind(local_oid, up_oid)?;
                                    ahead = a;
                                    behind = b;
                                }
                            }
                        }
                    }
                }
            }
            Err(e) if is_unborn(&e) => {}
            Err(e) => return Err(e.into()),
        }

        // On a branch without an upstream, ahead-vs-upstream is undefined;
        // count the not-yet-published commits instead (HEAD --not --remotes) —
        // the set a first push would publish.
        if branch.is_some() && upstream.is_none() {
            ahead = count_unpushed(&repo);
        }

        let op_state = crate::ops::map_state(repo.state());

        Ok(RepoStatus {
            staged,
            unstaged,
            branch,
            upstream,
            ahead,
            behind,
            op_state,
        })
    }
}

/// `true` when the error means "there is no commit yet" (unborn HEAD).
pub(crate) fn is_unborn(err: &git2::Error) -> bool {
    matches!(err.code(), ErrorCode::UnbornBranch | ErrorCode::NotFound)
}

/// Counts commits that are on NO remote-tracking branch (`HEAD --not --remotes`)
/// — the set a first push would publish. Used for the "ahead" display when no
/// upstream is set; on an unborn/empty HEAD push_head fails -> 0.
fn count_unpushed(repo: &git2::Repository) -> usize {
    let Ok(mut walk) = repo.revwalk() else {
        return 0;
    };
    if walk.push_head().is_err() {
        return 0;
    }
    if let Ok(refs) = repo.references_glob("refs/remotes/*") {
        for r in refs.flatten() {
            if let Some(oid) = r.target() {
                let _ = walk.hide(oid);
            }
        }
    }
    walk.filter_map(|oid| oid.ok()).count()
}

/// `true` when HEAD sits on a local branch without a (resolvable) upstream —
/// then `git status` has no ahead count and the count_unpushed fallback
/// applies.
fn head_branch_without_upstream(repo: &Repository) -> bool {
    match repo.head() {
        Ok(head) if head.is_branch() => head
            .shorthand()
            .ok()
            .and_then(|name| repo.find_branch(name, BranchType::Local).ok())
            .is_some_and(|branch| branch.upstream().is_err()),
        _ => false,
    }
}

fn entry_path(entry: &git2::StatusEntry<'_>) -> String {
    entry
        .path()
        .ok()
        .map(str::to_owned)
        .unwrap_or_else(|| String::from_utf8_lossy(entry.path_bytes()).into_owned())
}

fn delta_paths(delta: &git2::DiffDelta<'_>) -> (String, Option<String>) {
    let new = delta
        .new_file()
        .path()
        .or_else(|| delta.old_file().path())
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let old = if delta.status() == Delta::Renamed {
        delta
            .old_file()
            .path()
            .map(|p| p.to_string_lossy().into_owned())
    } else {
        None
    };
    (new, old)
}

/// Upper bound for inexact rename detection, in candidate pairs (added ×
/// deleted files) per status side. libgit2 compares every candidate pair —
/// unlike git, which gives up beyond `diff.renameLimit²` — so a change set of
/// 6,800 new and 1,000 deleted files took ~17 s per status instead of ~1 s
/// (user report: staging ~25k files appeared frozen). Measured cost ≈ 2.5 µs per
/// pair when nothing is similar, so the budget costs at most ~0.6 s. Beyond it
/// additions and deletions are shown as such — as git does when it skips rename
/// detection.
pub(crate) const RENAME_PAIR_BUDGET: usize = 250_000;

/// Which status sides can afford rename detection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct RenameSides {
    pub head_to_index: bool,
    pub index_to_workdir: bool,
}

impl RenameSides {
    /// Decided from a status taken WITHOUT rename detection: a side needs both
    /// additions and deletions to have renames at all, and their product has to
    /// fit the budget.
    pub(crate) fn affordable(statuses: &git2::Statuses<'_>) -> Self {
        let (mut index_new, mut index_deleted, mut wt_new, mut wt_deleted) = (0, 0, 0, 0);
        for entry in statuses.iter() {
            let st = entry.status();
            index_new += usize::from(st.contains(Status::INDEX_NEW));
            index_deleted += usize::from(st.contains(Status::INDEX_DELETED));
            wt_new += usize::from(st.contains(Status::WT_NEW));
            wt_deleted += usize::from(st.contains(Status::WT_DELETED));
        }
        Self {
            head_to_index: rename_affordable(index_new, index_deleted),
            index_to_workdir: rename_affordable(wt_new, wt_deleted),
        }
    }

    pub(crate) fn any(self) -> bool {
        self.head_to_index || self.index_to_workdir
    }
}

/// Rename detection is worth running for `added` × `deleted` candidates.
pub(crate) fn rename_affordable(added: usize, deleted: usize) -> bool {
    added > 0 && deleted > 0 && added.saturating_mul(deleted) <= RENAME_PAIR_BUDGET
}

/// Rename pairs (new path -> old path) of one status side: `staged` is
/// index↔HEAD, otherwise workdir↔index. The status reports renames as ONE entry
/// with the new path — but stage/unstage/discard have to handle BOTH sides and
/// therefore call the detection ONCE per call (not per file), and only when a
/// given path can be the new side of a rename at all.
pub(crate) fn rename_pairs(repo: &Repository, staged: bool) -> Result<HashMap<String, String>> {
    let mut opts = StatusOptions::new();
    opts.include_ignored(false).exclude_submodules(true);
    if staged {
        opts.show(StatusShow::Index);
    } else {
        // Untracked files are the possible rename targets in the worktree.
        opts.show(StatusShow::Workdir)
            .include_untracked(true)
            .recurse_untracked_dirs(true);
    }
    // Same budget as the status: where the status shows no renames, there are
    // none to pair up here either.
    let sides = RenameSides::affordable(&repo.statuses(Some(&mut opts))?);
    if !(if staged {
        sides.head_to_index
    } else {
        sides.index_to_workdir
    }) {
        return Ok(HashMap::new());
    }
    if staged {
        opts.renames_head_to_index(true);
    } else {
        opts.renames_index_to_workdir(true);
    }
    let statuses = repo.statuses(Some(&mut opts))?;
    let mut pairs = HashMap::new();
    for entry in statuses.iter() {
        let delta = if staged {
            entry
                .status()
                .contains(Status::INDEX_RENAMED)
                .then(|| entry.head_to_index())
                .flatten()
        } else {
            entry
                .status()
                .contains(Status::WT_RENAMED)
                .then(|| entry.index_to_workdir())
                .flatten()
        };
        if let Some((new, Some(old))) = delta.as_ref().map(delta_paths) {
            pairs.insert(new, old);
        }
    }
    Ok(pairs)
}

/// Restores `file` in the index literally from a HEAD tree entry (no fnmatch;
/// metadata 0 — git identifies the entry by its object id and stats it on the
/// next status).
fn index_add_from_head(
    index: &mut git2::Index,
    entry: &git2::TreeEntry<'_>,
    file: &str,
) -> Result<()> {
    index.add(&git2::IndexEntry {
        ctime: git2::IndexTime::new(0, 0),
        mtime: git2::IndexTime::new(0, 0),
        dev: 0,
        ino: 0,
        mode: entry.filemode() as u32,
        uid: 0,
        gid: 0,
        file_size: 0,
        id: entry.id(),
        flags: 0,
        flags_extended: 0,
        path: file.as_bytes().to_vec(),
    })?;
    Ok(())
}

/// Upper bound of diff lines per file that travel across the IPC boundary.
/// Larger diffs are truncated and marked with `truncated: true`.
const MAX_DIFF_LINES_PER_FILE: usize = 5000;

/// Streams a git2 diff file by file: `sink` receives every finished [`FileDiff`]
/// (line-truncated per file) and can end the iteration with `false` (e.g. the
/// file-count cap of the IPC streaming). Returns the TOTAL number of files in
/// the diff — regardless of how many the sink accepted.
fn stream_diff(
    repo: &Repository,
    diff: &git2::Diff<'_>,
    sink: &mut dyn FnMut(FileDiff) -> bool,
) -> Result<usize> {
    let total = diff.deltas().len();
    // LFS files show sizes instead of the pointer text.
    let lfs_repo = lfs::uses_lfs(repo);
    // RefCell/Cell because the three callbacks have to mutate at the same time.
    let current: RefCell<Option<FileDiff>> = RefCell::new(None);
    let line_count: RefCell<usize> = RefCell::new(0);
    let sink = RefCell::new(sink);
    let stopped = std::cell::Cell::new(false);

    let result = diff.foreach(
        &mut |delta, _| {
            // The previous file is complete -> deliver it.
            if let Some(done) = current.borrow_mut().take() {
                if !(sink.borrow_mut())(done) {
                    stopped.set(true);
                    return false; // aborts the foreach (git2 reports GIT_EUSER)
                }
            }
            let (path, old_path) = delta_paths(&delta);
            let mut fd = FileDiff {
                path,
                old_path,
                is_binary: delta.flags().is_binary(),
                hunks: Vec::new(),
                truncated: false,
                old_size: delta.old_file().exists().then(|| delta.old_file().size()),
                new_size: delta.new_file().exists().then(|| delta.new_file().size()),
                lfs: false,
            };
            if lfs_repo {
                lfs::describe(repo, &delta, &mut fd);
            }
            *current.borrow_mut() = Some(fd);
            *line_count.borrow_mut() = 0;
            true
        },
        None,
        Some(&mut |_delta, hunk| {
            let mut current = current.borrow_mut();
            let Some(file) = current.as_mut() else {
                return true;
            };
            if file.lfs {
                return true;
            }
            if *line_count.borrow() >= MAX_DIFF_LINES_PER_FILE {
                file.truncated = true;
                return true;
            }
            file.hunks.push(DiffHunk {
                header: String::from_utf8_lossy(hunk.header())
                    .trim_end()
                    .to_string(),
                lines: Vec::new(),
            });
            true
        }),
        Some(&mut |_delta, _hunk, line| {
            let mut current = current.borrow_mut();
            let Some(file) = current.as_mut() else {
                return true;
            };
            if file.lfs {
                return true;
            }
            let kind = match line.origin() {
                '+' => LineKind::Addition,
                '-' => LineKind::Deletion,
                ' ' => LineKind::Context,
                'B' => {
                    file.is_binary = true;
                    return true;
                }
                _ => return true,
            };
            let mut count = line_count.borrow_mut();
            if *count >= MAX_DIFF_LINES_PER_FILE {
                file.truncated = true;
                return true;
            }
            if let Some(hunk) = file.hunks.last_mut() {
                hunk.lines.push(DiffLine {
                    kind,
                    old_lineno: line.old_lineno(),
                    new_lineno: line.new_lineno(),
                    content: String::from_utf8_lossy(line.content())
                        .trim_end_matches(['\n', '\r'])
                        .to_string(),
                });
                *count += 1;
            }
            true
        }),
    );

    match result {
        Ok(()) => {}
        // A deliberate sink abort (truncation): not an error.
        Err(_) if stopped.get() => {}
        Err(e) => return Err(e.into()),
    }
    // Deliver the last file (foreach never reports "file finished").
    if !stopped.get() {
        if let Some(done) = current.borrow_mut().take() {
            let _ = (sink.borrow_mut())(done);
        }
    }
    Ok(total)
}

/// Turns a git2 diff into domain [`FileDiff`]s (truncated per file).
/// [`stream_diff`] for the other engine modules (range diffs).
pub(crate) fn stream_diff_pub(
    repo: &Repository,
    diff: &git2::Diff<'_>,
    sink: &mut dyn FnMut(FileDiff) -> bool,
) -> Result<usize> {
    stream_diff(repo, diff, sink)
}

fn collect_diff(repo: &Repository, diff: &git2::Diff<'_>) -> Result<Vec<FileDiff>> {
    let mut files = Vec::new();
    stream_diff(repo, diff, &mut |fd| {
        files.push(fd);
        true
    })?;
    Ok(files)
}

/// Builds the diff of a commit against its first parent (root: the empty tree),
/// including rename detection — the shared basis for the collecting and the
/// streaming path.
fn build_commit_diff<'r>(repo: &'r Repository, commit_id: &str) -> Result<git2::Diff<'r>> {
    let commit = repo.find_commit(Oid::from_str(commit_id)?)?;
    let tree = commit.tree()?;
    let parent_tree = match commit.parent(0) {
        Ok(parent) => Some(parent.tree()?),
        Err(_) => None, // root commit: diff against the empty tree
    };

    let mut opts = DiffOptions::new();
    opts.context_lines(3);
    let mut diff = repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), Some(&mut opts))?;
    diff.find_similar(None)?; // Detect renames
    Ok(diff)
}

/// Picks the push target remote: preferably the branch's upstream remote,
/// otherwise the only configured remote, otherwise the `origin` convention.
/// That way the first push also works in repos whose remote is not named `origin`.
fn pick_push_remote(repo: &Repository, branch: Option<&str>) -> String {
    if let Some(name) = branch {
        if let Ok(buf) = repo.branch_upstream_remote(&format!("refs/heads/{name}")) {
            if let Ok(s) = buf.as_str() {
                if !s.is_empty() {
                    return s.to_string();
                }
            }
        }
    }
    if let Ok(remotes) = repo.remotes() {
        if remotes.len() == 1 {
            if let Ok(Some(only)) = remotes.get(0) {
                return only.to_string();
            }
        }
    }
    "origin".to_string()
}

pub(crate) fn commit_to_info(commit: &git2::Commit<'_>) -> CommitInfo {
    let id = commit.id().to_string();
    CommitInfo {
        short_id: id.chars().take(8).collect(),
        id,
        summary: commit.summary().ok().flatten().unwrap_or("").to_string(),
        author_name: commit.author().name().unwrap_or("?").to_string(),
        author_email: commit.author().email().unwrap_or("").to_string(),
        time: commit.author().when().seconds(),
        parent_ids: commit.parent_ids().map(|p| p.to_string()).collect(),
    }
}

impl GitEngine for Git2Engine {
    fn open_repo(&self, path: &Path) -> Result<RepoInfo> {
        let repo = self.open(path)?;
        let workdir = Self::workdir(&repo)?;
        let name = workdir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| workdir.display().to_string());

        // On a detached HEAD, shorthand() returns the string "HEAD" — per the
        // domain contract current_branch is None then.
        let detached = repo.head_detached().unwrap_or(false);
        let head = match repo.head() {
            Ok(h) => Some(h),
            Err(e) if is_unborn(&e) => None,
            Err(e) => return Err(e.into()),
        };
        let current_branch = match &head {
            Some(h) if !detached => h.shorthand().ok().map(str::to_owned),
            _ => None,
        };
        // "Empty" is what RepoInfo documents: no commit yet, i.e. an unborn HEAD.
        // Deliberately NOT git2's `Repository::is_empty()`, which additionally
        // demands that HEAD point at the branch `init.defaultBranch` names. We
        // create new repos with `main` when the host configures nothing, so on a
        // machine without that setting (the default on Linux and macOS) libgit2
        // compares "main" against its built-in "master" and calls a fresh, empty
        // repository non-empty.
        let is_empty = head.is_none();

        // git2 returns the workdir with a trailing separator — normalize it,
        // otherwise duplicate recents entries and empty display names appear.
        let path_str = workdir
            .display()
            .to_string()
            .trim_end_matches(['/', '\\'])
            .to_string();

        Ok(RepoInfo {
            path: path_str,
            name,
            current_branch,
            head_detached: detached,
            is_empty,
            // Empty repos count as "prepared" — there is no history a commit
            // graph could be missing for.
            history_prepared: is_empty || sidecar::commit_graph_ready(path),
        })
    }

    fn status(&self, path: &Path) -> Result<RepoStatus> {
        // Large worktrees: the system-git status is clearly faster than the
        // single-threaded libgit2 scan thanks to parallel lstat
        // (core.preloadIndex) and an optional untrackedCache/fsmonitor — and
        // semantically identical to the CLI (equivalence test
        // fast_path_matches_git2). On any error: back to the git2 path.
        //
        // ONE git2 open delivers both signals (index size + op_state) — the
        // second open per status refresh that used to be needed
        // (status_via_sidecar) is gone in the fast path (no double index access).
        let repo = self.open(path)?;
        let index_entries = repo.index().map(|i| i.len()).unwrap_or(0);
        // Filter drivers (Git LFS): only the system git runs the clean filter,
        // libgit2 would compare the content against the pointer.
        if index_entries >= status_fast::FAST_PATH_MIN_INDEX_ENTRIES || lfs::declares_filters(&repo)
        {
            match self.status_fast_path(repo, path) {
                Ok(fast) => return Ok(fast),
                Err(e) => tracing::warn!("status fast path failed, falling back to git2: {e}"),
            }
        }
        self.status_git2(path)
    }

    fn log(&self, path: &Path, skip: usize, limit: usize) -> Result<Vec<CommitInfo>> {
        let repo = self.open(path)?;
        // Check for an unborn HEAD (empty repo) explicitly: push_head() only
        // returns a generic reference error for it.
        match repo.head() {
            Ok(_) => {}
            Err(e) if is_unborn(&e) => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        }
        // Sidecar first: `git log --topo-order` STREAMS the page thanks to
        // commit-graph generation numbers (Linux kernel: 53 ms), whereas the
        // libgit2 revwalk buffers the COMPLETE graph for TOPOLOGICAL as well as
        // TIME (68 s). See docs/perf-stress-test.md.
        if let Ok(commits) = sidecar::log_page(path, skip, limit) {
            return Ok(commits);
        }
        // Fallback libgit2 (no git in PATH etc.) — correct, but slow on huge
        // repos.
        let mut walk = repo.revwalk()?;
        walk.push_head()?;
        walk.set_sorting(Sort::TOPOLOGICAL | Sort::TIME)?;

        let mut commits = Vec::with_capacity(limit);
        for oid in walk.skip(skip).take(limit) {
            let commit = repo.find_commit(oid?)?;
            commits.push(commit_to_info(&commit));
        }
        Ok(commits)
    }

    fn log_all(&self, path: &Path, skip: usize, limit: usize) -> Result<Vec<CommitInfo>> {
        let repo = self.open(path)?;
        // Sidecar first (streams thanks to the commit graph, see log()); ref
        // families explicit instead of `--all` so refs/stash stays out.
        if let Ok(commits) = sidecar::log_page_all(path, skip, limit) {
            return Ok(commits);
        }
        // Fallback libgit2: all ref families by glob, HEAD only when it is born
        // (a fresh repo without commits simply yields an empty page).
        let mut walk = repo.revwalk()?;
        for glob in ["refs/heads/*", "refs/remotes/*", "refs/tags/*"] {
            walk.push_glob(glob)?;
        }
        match repo.head() {
            Ok(_) => walk.push_head()?,
            Err(e) if is_unborn(&e) => {}
            Err(e) => return Err(e.into()),
        }
        walk.set_sorting(Sort::TOPOLOGICAL | Sort::TIME)?;

        let mut commits = Vec::with_capacity(limit);
        for oid in walk.skip(skip).take(limit) {
            let commit = repo.find_commit(oid?)?;
            commits.push(commit_to_info(&commit));
        }
        Ok(commits)
    }

    fn file_diff(&self, path: &Path, file: &str, staged: bool) -> Result<Option<FileDiff>> {
        let repo = self.open(path)?;
        let mut opts = DiffOptions::new();
        opts.pathspec(file).context_lines(3);
        // Literal pathspec instead of fnmatch (file names with [ ] * ? would
        // otherwise act as globs).

        let diff = if staged {
            let head_tree = match repo.head() {
                Ok(head) => Some(head.peel_to_tree()?),
                Err(e) if is_unborn(&e) => None,
                Err(e) => return Err(e.into()),
            };
            repo.diff_tree_to_index(head_tree.as_ref(), None, Some(&mut opts))?
        } else {
            opts.include_untracked(true)
                .recurse_untracked_dirs(true)
                .show_untracked_content(true);
            repo.diff_index_to_workdir(None, Some(&mut opts))?
        };

        Ok(collect_diff(&repo, &diff)?.into_iter().next())
    }

    fn commit_diff(&self, path: &Path, commit_id: &str) -> Result<Vec<FileDiff>> {
        let repo = self.open(path)?;
        let diff = build_commit_diff(&repo, commit_id)?;
        collect_diff(&repo, &diff)
    }

    fn commit_diff_stream(
        &self,
        path: &Path,
        commit_id: &str,
        max_files: usize,
        sink: &mut dyn FnMut(FileDiff) -> bool,
    ) -> Result<usize> {
        let repo = self.open(path)?;
        let diff = build_commit_diff(&repo, commit_id)?;
        let mut sent = 0usize;
        stream_diff(&repo, &diff, &mut |fd| {
            if sent == max_files {
                return false;
            }
            if !sink(fd) {
                return false;
            }
            sent += 1;
            true
        })
    }

    fn stage(&self, path: &Path, files: &[String]) -> Result<()> {
        self.stage_reporting(path, files, None, &mut |_| {})
    }

    fn unstage(&self, path: &Path, files: &[String]) -> Result<()> {
        let repo = self.open(path)?;
        match repo.head() {
            // IMPORTANT: no `reset_default` — that interprets paths as fnmatch
            // pathspecs (a file `a[b].txt` would hit `ab.txt`).
            // Instead restore index entries literally from the HEAD tree.
            Ok(head) => {
                let head_tree = head.peel_to_tree()?;
                let mut index = repo.index()?;
                // Staged renames are ONE status entry (the new path); detection
                // is only needed when a path is not in HEAD — only then can it be
                // the new side of a rename.
                let renames = if files
                    .iter()
                    .any(|f| head_tree.get_path(Path::new(f)).is_err())
                {
                    rename_pairs(&repo, true)?
                } else {
                    HashMap::new()
                };
                for file in files {
                    let rel = Path::new(file);
                    match head_tree.get_path(rel) {
                        Ok(entry) => index_add_from_head(&mut index, &entry, file)?,
                        // Unknown in HEAD (newly staged): remove it from the index.
                        Err(_) => index.remove_path(rel)?,
                    }
                    // Rename: take back the old side too, i.e. restore it in the
                    // index from HEAD — otherwise its deletion would stay staged
                    // (a commit would delete the old file without adding the new
                    // one).
                    if let Some(old) = renames.get(file) {
                        if let Ok(entry) = head_tree.get_path(Path::new(old)) {
                            index_add_from_head(&mut index, &entry, old)?;
                        }
                    }
                }
                index.write()?;
            }
            // Without a commit there is no HEAD: unstage = remove from the index.
            Err(e) if is_unborn(&e) => {
                let mut index = repo.index()?;
                for file in files {
                    index.remove_path(Path::new(file))?;
                }
                index.write()?;
            }
            Err(e) => return Err(e.into()),
        }
        Ok(())
    }

    fn discard(&self, path: &Path, files: &[String]) -> Result<()> {
        let repo = self.open(path)?;
        let workdir = Self::workdir(&repo)?;

        // File status once up front: WT_NEW decides delete vs. restore — and
        // whether rename detection is needed at all (only an untracked file can
        // be the new side of a workdir rename).
        let stats: Vec<(&String, Status)> = files
            .iter()
            .map(|file| {
                let st = repo.status_file(Path::new(file)).unwrap_or(Status::CURRENT);
                (file, st)
            })
            .collect();
        let renames = if stats.iter().any(|(_, st)| st.contains(Status::WT_NEW)) {
            rename_pairs(&repo, false)?
        } else {
            HashMap::new()
        };

        let mut tracked: Vec<&String> = Vec::new();
        for (file, st) in stats {
            if st.contains(Status::WT_NEW) {
                // Untracked: delete the file (the frontend has confirmed).
                let abs = workdir.join(file);
                if abs.is_dir() {
                    std::fs::remove_dir_all(&abs)?;
                } else if abs.symlink_metadata().is_ok() {
                    std::fs::remove_file(&abs)?;
                }
                // Rename: the status entry carries only the new path — restore
                // the old path from the index as well, otherwise the file would
                // be gone entirely after the discard.
                if let Some(old) = renames.get(file) {
                    tracked.push(old);
                }
            } else {
                tracked.push(file);
            }
        }

        if !tracked.is_empty() {
            // Like `git restore`: restore the workdir from the index.
            let mut cb = CheckoutBuilder::new();
            cb.force();
            // Treat paths literally — otherwise `a[b].txt` acts as a glob on `ab.txt`.
            cb.disable_pathspec_match(true);
            for file in &tracked {
                cb.path(file.as_str());
            }
            repo.checkout_index(None, Some(&mut cb))?;
            let tracked: Vec<String> = tracked.into_iter().cloned().collect();
            lfs::restore_content(&repo, Some(&tracked));
        }
        Ok(())
    }

    fn commit(&self, path: &Path, message: &str, amend: bool) -> Result<String> {
        self.commit_with(path, message, amend, false)
    }

    fn branches(&self, path: &Path) -> Result<Vec<BranchInfo>> {
        let repo = self.open(path)?;
        let mut result = Vec::new();
        let cfg = repo.config().ok();
        for item in repo.branches(None)? {
            let (branch, btype) = item?;
            let Some(name) = branch.name()?.map(str::to_owned) else {
                continue;
            };
            // The symbolic origin/HEAD reference is worthless to the UI.
            if name.ends_with("/HEAD") {
                continue;
            }
            let is_remote = btype == BranchType::Remote;
            let upstream = if is_remote {
                None
            } else {
                branch
                    .upstream()
                    .ok()
                    .and_then(|u| u.name().ok().flatten().map(str::to_owned))
            };
            // Remote short name without guessing the prefix: git knows the remote
            // part of the reference (works for remotes other than "origin" too).
            let short_name = if is_remote {
                branch
                    .get()
                    .name()
                    .ok()
                    .and_then(|refname| repo.branch_remote_name(refname).ok())
                    .and_then(|remote| {
                        remote
                            .as_str()
                            .ok()
                            .and_then(|r| name.strip_prefix(&format!("{r}/")))
                            .map(str::to_owned)
                    })
            } else {
                None
            };
            let target_id = branch.get().target().map(|oid| oid.to_string());
            // Orphaned: an upstream was configured (remote != "." and merge set)
            // but no longer resolves (the remote ref was pruned).
            let upstream_gone = !is_remote
                && upstream.is_none()
                && cfg.as_ref().is_some_and(|c| {
                    let remote = c.get_string(&format!("branch.{name}.remote")).ok();
                    let has_merge = c.get_string(&format!("branch.{name}.merge")).is_ok();
                    matches!(remote.as_deref(), Some(r) if r != ".") && has_merge
                });
            result.push(BranchInfo {
                is_head: branch.is_head(),
                name,
                is_remote,
                upstream,
                short_name,
                target_id,
                upstream_gone,
            });
        }
        result.sort_by(|a, b| (a.is_remote, &a.name).cmp(&(b.is_remote, &b.name)));
        Ok(result)
    }

    fn create_branch(&self, path: &Path, name: &str, checkout: bool) -> Result<()> {
        {
            let repo = self.open(path)?;
            let head = repo.head();
            if let Err(e) = &head {
                // Unborn HEAD (fresh repo): there is no commit yet a branch could
                // point at. Move the symbolic HEAD instead — the branch comes into
                // existence with the first commit. That is the
                // only representable form, independent of the checkout flag.
                if is_unborn(e) {
                    if !git2::Branch::name_is_valid(name).unwrap_or(false) {
                        return Err(GitEngineError::InvalidOperation(format!(
                            "Invalid branch name: {name}"
                        )));
                    }
                    repo.set_head(&format!("refs/heads/{name}"))?;
                    return Ok(());
                }
            }
            let head_commit = head?.peel_to_commit()?;
            repo.branch(name, &head_commit, false)?;
        } // Close the repo before the checkout reopens it.
        if checkout {
            self.checkout_branch(path, name)?;
        }
        Ok(())
    }

    fn checkout_branch(&self, path: &Path, name: &str) -> Result<()> {
        // Without progress: delegate to the progress variant with a no-op callback
        // (same pattern as push -> push_with_progress).
        self.checkout_branch_with_progress(path, name, &mut |_| {})
    }

    fn fetch(&self, path: &Path) -> Result<String> {
        sidecar::fetch(path)
    }

    fn pull(&self, path: &Path) -> Result<String> {
        sidecar::pull(path)
    }

    fn push(&self, path: &Path) -> Result<String> {
        self.push_with_progress(path, &CancelToken::new(), &mut |_| {})
    }
}

impl Git2Engine {
    /// Switches to `name` (a local branch or the DWIM tracking of a remote branch
    /// with the same short name) and reports the checkout progress (git2
    /// `CheckoutBuilder::progress`) as [`tg_domain::GitProgress`] through `on`.
    /// After a successful switch a 100% completion is reported deliberately so the
    /// UI reliably shows "done" (for small/empty checkouts the progress callback
    /// may otherwise never reach 100).
    pub fn checkout_branch_with_progress(
        &self,
        path: &Path,
        name: &str,
        on: &mut dyn FnMut(tg_domain::GitProgress),
    ) -> Result<()> {
        let repo = self.open(path)?;

        // Find the local branch — or create a local tracking branch from a remote
        // branch of the same short name (like "git switch <name>"; works for any
        // remote name).
        let refname = match repo.find_branch(name, BranchType::Local) {
            Ok(branch) => branch.get().name().ok().map(str::to_owned),
            Err(_) => {
                let mut found: Option<String> = None;
                for item in repo.branches(Some(BranchType::Remote))? {
                    let (remote_branch, _) = item?;
                    let Some(full) = remote_branch.name()?.map(str::to_owned) else {
                        continue;
                    };
                    let is_match = remote_branch
                        .get()
                        .name()
                        .ok()
                        .and_then(|refname| repo.branch_remote_name(refname).ok())
                        .and_then(|r| r.as_str().ok().map(|r| format!("{r}/{name}")))
                        .is_some_and(|expected| expected == full);
                    if is_match {
                        found = Some(full);
                        break;
                    }
                }
                let remote_name =
                    found.ok_or_else(|| GitEngineError::BranchNotFound(name.to_string()))?;
                let remote = repo.find_branch(&remote_name, BranchType::Remote)?;
                let commit = remote.get().peel_to_commit()?;
                let mut local = repo.branch(name, &commit, false)?;
                local.set_upstream(Some(&remote_name))?;
                local.get().name().ok().map(str::to_owned)
            }
        }
        .ok_or_else(|| GitEngineError::BranchNotFound(name.to_string()))?;

        let object = repo.revparse_single(&refname)?;
        // cb borrows `on` mutably; hence its own scope so the borrow is released
        // before the 100% completion below (E0501).
        let blocked = std::cell::RefCell::new(Vec::<String>::new());
        {
            let mut cb = CheckoutBuilder::new();
            // safe(): fails on local changes instead of losing them.
            // Collecting the blocking paths ONLY here is the only way — the
            // libgit2 error message names just their count.
            cb.safe();
            cb.notify_on(git2::CheckoutNotificationType::CONFLICT);
            cb.notify(|_why, path, _baseline, _target, _workdir| {
                if let Some(p) = path {
                    blocked.borrow_mut().push(p.display().to_string());
                }
                true // keep counting: we want ALL blocking files
            });
            cb.progress(|_path, completed, total| {
                let percent = (completed * 100).checked_div(total).unwrap_or(0).min(100) as u8;
                on(tg_domain::GitProgress {
                    phase: "checkout".to_string(),
                    percent,
                    done: None,
                    total: None,
                });
            });
            repo.checkout_tree(&object, Some(&mut cb))
                .map_err(|e| checkout_error(e, blocked.borrow().clone()))?;
        }
        repo.set_head(&refname)?;
        lfs::restore_content(&repo, None);
        on(tg_domain::GitProgress {
            phase: "checkout".to_string(),
            percent: 100,
            done: None,
            total: None,
        });
        Ok(())
    }
}

/// Translates a failed checkout into a meaningful error.
///
/// On uncommitted changes libgit2 only says "n conflicts prevent checkout" — the
/// word "conflict" here does NOT mean a merge conflict but a locally modified
/// file the checkout would overwrite. Exactly that confusion sent users into the
/// (empty) conflict view. Detected through the error class + code, not through
/// the text: the message is a libgit2 internal, the class is stable.
pub(crate) fn checkout_error(e: git2::Error, files: Vec<String>) -> GitEngineError {
    if e.class() == git2::ErrorClass::Checkout && e.code() == git2::ErrorCode::Conflict {
        /// Nobody reads more paths in a toast; the ellipsis says there is more.
        /// Deliberately a plain character instead of text — the frontend picks
        /// the language, not the engine.
        const MAX_LISTED: usize = 8;
        let mut files = files;
        files.sort();
        files.dedup();
        if files.len() > MAX_LISTED {
            files.truncate(MAX_LISTED);
            files.push("…".into());
        }
        return GitEngineError::CheckoutWouldOverwrite { files };
    }
    GitEngineError::Git(e)
}

/// Progress-streaming, cancellable remote operations — now behind the
/// [`RemoteProgressOps`](crate::ops::RemoteProgressOps) trait (formerly inherent
/// "next to" the abstraction). The bodies stay here because they use `self.open`/
/// `pick_push_remote`.
impl crate::ops::RemoteProgressOps for Git2Engine {
    fn fetch_with_progress(
        &self,
        path: &Path,
        cancel: &CancelToken,
        on: &mut dyn FnMut(tg_domain::GitProgress),
    ) -> Result<String> {
        sidecar::fetch_streaming(path, sidecar::FetchTarget::Default, cancel, on)
    }

    fn fetch_from(
        &self,
        path: &Path,
        remote: Option<&str>,
        cancel: &CancelToken,
        on: &mut dyn FnMut(tg_domain::GitProgress),
    ) -> Result<String> {
        let target = remote.map_or(sidecar::FetchTarget::All, sidecar::FetchTarget::Remote);
        sidecar::fetch_streaming(path, target, cancel, on)
    }

    fn publish_branch_only(
        &self,
        path: &Path,
        remote: &str,
        cancel: &CancelToken,
        on: &mut dyn FnMut(tg_domain::GitProgress),
    ) -> Result<String> {
        let branch = {
            let repo = self.open(path)?;
            let head = repo.head()?;
            if !head.is_branch() {
                return Err(GitEngineError::InvalidOperation(
                    "Publishing needs a branch (HEAD is detached)".into(),
                ));
            }
            head.shorthand()?.to_string()
        };
        // The fork point: the parent of the oldest commit that is on no
        // remote yet — or HEAD itself when everything is pushed already.
        let unpushed = ops::HistoryOps::unpushed_commits(self, path)?;
        let base = match unpushed.last() {
            None => self.log(path, 0, 1)?.first().map(|c| c.id.clone()),
            Some(oldest) => oldest.parent_ids.first().cloned(),
        }
        .ok_or_else(|| {
            GitEngineError::InvalidOperation(
                "No commit of this branch is on a remote yet — publish it with its commits".into(),
            )
        })?;
        let out = sidecar::push_commit_as_branch(path, remote, &base, &branch, cancel, on)?;
        // A push of a commit id sets no tracking: do it explicitly.
        sidecar::run_git(
            path,
            &[
                "branch",
                &format!("--set-upstream-to={remote}/{branch}"),
                "--",
                &branch,
            ],
        )?;
        Ok(out)
    }

    fn pull_with_progress(
        &self,
        path: &Path,
        prune: bool,
        cancel: &CancelToken,
        on: &mut dyn FnMut(tg_domain::GitProgress),
    ) -> Result<String> {
        self.pull_with_strategy(path, prune, None, None, cancel, on)
    }

    fn pull_with_strategy(
        &self,
        path: &Path,
        prune: bool,
        strategy: Option<&str>,
        from: Option<(&str, &str)>,
        cancel: &CancelToken,
        on: &mut dyn FnMut(tg_domain::GitProgress),
    ) -> Result<String> {
        let strategy = strategy.map(sidecar::PullStrategy::parse).transpose()?;
        // A pull.rebase / pull.ff the user set in git wins over the app's
        // choice — but only in the repo's own or the user's personal (global)
        // configuration. The SYSTEM level is skipped on purpose: the Git for
        // Windows installer writes pull.rebase=false there for everyone, which
        // would silently switch the app's setting off on every Windows machine.
        let configured = {
            let repo = self.open(path)?;
            let config = repo.config()?;
            [
                git2::ConfigLevel::Local,
                git2::ConfigLevel::Global,
                git2::ConfigLevel::XDG,
                git2::ConfigLevel::Worktree,
            ]
            .into_iter()
            .filter_map(|level| config.open_level(level).ok())
            .any(|c| c.get_entry("pull.rebase").is_ok() || c.get_entry("pull.ff").is_ok())
        };
        let strategy = if configured && !strategy.is_some_and(sidecar::PullStrategy::is_explicit) {
            None
        } else {
            strategy
        };
        sidecar::pull_streaming(path, prune, strategy, from, cancel, on)
    }

    fn clone_prepare(&self, url: &str, dest_dir: &Path) -> Result<()> {
        sidecar::clone_prepare(url, dest_dir)
    }

    fn clone_fetch(
        &self,
        path: &Path,
        options: &tg_domain::CloneOptions,
        cancel: &CancelToken,
        on: &mut dyn FnMut(tg_domain::GitProgress),
    ) -> Result<String> {
        sidecar::clone_fetch(path, options, cancel, on)
    }

    fn push_with_progress(
        &self,
        path: &Path,
        cancel: &CancelToken,
        on: &mut dyn FnMut(tg_domain::GitProgress),
    ) -> Result<String> {
        let repo = self.open(path)?;
        let (branch_name, has_upstream) = match repo.head() {
            Ok(head) if head.is_branch() => {
                let name = head.shorthand().ok().map(str::to_owned);
                let has_up = name
                    .as_deref()
                    .and_then(|n| repo.find_branch(n, BranchType::Local).ok())
                    .map(|b| b.upstream().is_ok())
                    .unwrap_or(false);
                (name, has_up)
            }
            _ => (None, false),
        };
        let remote = pick_push_remote(&repo, branch_name.as_deref());
        drop(repo);
        sidecar::push_streaming(
            path,
            &remote,
            branch_name.as_deref(),
            has_upstream,
            false,
            cancel,
            on,
        )
    }

    fn push_remote_with_progress(
        &self,
        path: &Path,
        remote: &str,
        force: bool,
        cancel: &CancelToken,
        on: &mut dyn FnMut(tg_domain::GitProgress),
    ) -> Result<String> {
        let repo = self.open(path)?;
        let branch_name = match repo.head() {
            Ok(head) if head.is_branch() => head.shorthand().ok().map(str::to_owned),
            _ => None,
        };
        drop(repo);
        sidecar::push_to_streaming(path, remote, branch_name.as_deref(), force, cancel, on)
    }
}

/// The hooks a commit runs. libgit2 runs none of them, so a repo with any
/// of them — or with `core.hooksPath` set (husky & co.) — commits through
/// git.
const COMMIT_HOOKS: [&str; 4] = [
    "pre-commit",
    "prepare-commit-msg",
    "commit-msg",
    "post-commit",
];

/// Where git looks for the hook `name`: in `core.hooksPath` when that is set
/// (relative to the worktree, as git reads it), else in the common dir's
/// `hooks`.
fn hook_path(repo: &Repository, name: &str) -> PathBuf {
    let configured = repo
        .config()
        .and_then(|c| c.get_string("core.hooksPath"))
        .ok()
        .filter(|p| !p.trim().is_empty());
    let dir = match configured {
        Some(p) => {
            let p = PathBuf::from(p);
            match (p.is_absolute(), repo.workdir()) {
                (false, Some(workdir)) => workdir.join(p),
                _ => p,
            }
        }
        None => repo.commondir().join("hooks"),
    };
    dir.join(name)
}

/// Whether the hook `name` is in place (see [`hook_path`]).
pub(crate) fn hook_exists(repo: &Repository, name: &str) -> bool {
    hook_path(repo, name).is_file()
}

/// Whether the `pre-push` hook is the one `git lfs install` writes — nothing
/// but the git-lfs call (plus its "git-lfs missing" guard). Its failures are
/// LFS upload errors, not a hook of the user's stopping the push.
pub(crate) fn pre_push_is_git_lfs(repo: &Repository) -> bool {
    let Ok(text) = std::fs::read_to_string(hook_path(repo, "pre-push")) else {
        return false;
    };
    let code: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    code.iter().any(|l| l.starts_with("git lfs pre-push"))
        && code
            .iter()
            .all(|l| l.starts_with("git lfs pre-push") || l.starts_with("command -v git-lfs"))
}

fn commit_hooks(repo: &Repository) -> bool {
    let hooks_path = repo
        .config()
        .and_then(|c| c.get_string("core.hooksPath"))
        .is_ok_and(|p| !p.trim().is_empty());
    if hooks_path {
        return true;
    }
    let dir = repo.commondir().join("hooks");
    COMMIT_HOOKS.iter().any(|h| dir.join(h).is_file())
}

/// A failed `git commit`: the known causes by name; otherwise, with hooks in
/// place, the hook that stopped it (its output is the detail), and with
/// signing on, the signature.
fn commit_error(e: GitEngineError, hooks: bool, signing: bool) -> GitEngineError {
    let GitEngineError::Sidecar { message } = e else {
        return e;
    };
    let lower = message.to_ascii_lowercase();
    let (code, text) = if let Some(known) = sidecar::classify_local_error(&message) {
        known
    } else if lower.contains("nothing to commit") || lower.contains("no changes added") {
        return GitEngineError::Sidecar { message };
    } else if hooks {
        (
            "hook_failed",
            "A git hook stopped the commit — its output is below.".to_string(),
        )
    } else if signing {
        (
            "signing_failed",
            "Signing the commit failed — see git's output below.".to_string(),
        )
    } else {
        return GitEngineError::Sidecar { message };
    };
    GitEngineError::Remote {
        code,
        message: text,
        raw: message,
    }
}

impl Git2Engine {
    /// [`GitEngine::commit`], optionally without a signature for this one
    /// commit (`unsigned`, the way out of `signing_failed`).
    pub fn commit_with(
        &self,
        path: &Path,
        message: &str,
        amend: bool,
        unsigned: bool,
    ) -> Result<String> {
        if message.trim().is_empty() {
            return Err(GitEngineError::EmptyCommitMessage);
        }
        let mut repo = self.open(path)?;

        // Safety net: an amend is a history rewrite → anchor the old HEAD
        // beforehand as a durable backup (refs/terra-git/backup/). The volatile
        // undo stack survives no app restart/gc, a real reference does. Only when
        // HEAD exists and no merge is running (an amend during a merge is
        // rejected below anyway) — otherwise a backup would be created that only
        // masks the actual error.
        if amend
            && repo.state() != git2::RepositoryState::Merge
            && repo.head().and_then(|h| h.peel_to_commit()).is_ok()
        {
            crate::ops::create_backup_ref(&repo, "amend")?;
        }

        // Signed commits (commit.gpgsign=true) go through the system git, which
        // handles GPG/SSH signing (community request #78) — and so do commits
        // in a repo with commit hooks, which libgit2 would skip.
        let signing = !unsigned
            && repo
                .config()
                .and_then(|c| c.get_bool("commit.gpgsign"))
                .unwrap_or(false);
        let hooks = commit_hooks(&repo);
        if signing || hooks {
            drop(repo);
            // Exclusive create, 0600 on Unix: a hook may run for minutes, and
            // other users of a shared /tmp must neither read the message nor
            // redirect the write.
            let msg_file = sidecar::write_secure_temp("terra-git-msg", "txt", message.as_bytes())?;
            let msg_path = msg_file.to_string_lossy().into_owned();
            let mut args: Vec<&str> = Vec::new();
            if unsigned {
                // "Commit once without a signature" after `signing_failed`.
                args.extend(["-c", "commit.gpgsign=false"]);
            }
            args.extend(["commit", "-F", &msg_path]);
            if amend {
                args.push("--amend");
            }
            // Long timeout: a pre-commit hook may run linters or tests.
            // English messages whatever the user's language: commit_error reads
            // git's wording (with a German git a missing identity became
            // `hook_failed`). Only the message language — the hooks keep the
            // user's character encoding (LC_CTYPE), unlike with LC_ALL=C.
            let result =
                sidecar::run_git_long_env(path, &args, &[("LANGUAGE", "C"), ("LC_MESSAGES", "C")]);
            let _ = std::fs::remove_file(&msg_file);
            result.map_err(|e| commit_error(e, hooks, signing))?;
            let repo = self.open(path)?;
            let id = repo.head()?.peel_to_commit()?.id().to_string();
            return Ok(id);
        }

        let state = repo.state();
        let merging = state == git2::RepositoryState::Merge;
        // Cherry-pick/revert (also as a sequence) leave CHERRY_PICK_HEAD/
        // REVERT_HEAD/.git/sequencer behind — after a successful commit, clean
        // them up just like the merge state, otherwise "continue" fails afterwards
        // and "abort" is refused. The commit itself stays a single-parent commit
        // as before (no MERGE_HEADs in these states).
        let sequencing = matches!(
            state,
            git2::RepositoryState::CherryPick
                | git2::RepositoryState::CherryPickSequence
                | git2::RepositoryState::Revert
                | git2::RepositoryState::RevertSequence
        );
        // Collect the MERGE_HEADs right away (needs &mut before borrows appear).
        let mut merge_oids: Vec<Oid> = Vec::new();
        if merging {
            repo.mergehead_foreach(|oid| {
                merge_oids.push(*oid);
                true
            })?;
        }
        let sig = repo.signature()?;
        let mut index = repo.index()?;
        let tree_id = index.write_tree()?;
        let tree = repo.find_tree(tree_id)?;

        if amend {
            // Reject an amend during ANY running multi-step operation, not only
            // during a merge: otherwise amend() replaces our OWN
            // predecessor commit with the result of the running operation — the
            // foreign change lands in our commit, ours is gone, and the operation
            // state remains. Real git refuses this explicitly ("You are in the
            // middle of a cherry-pick -- cannot amend"), and the sidecar path
            // passes exactly that error through; only the git2 path did it
            // silently. On a merge it would additionally produce a falsified
            // merge parent.
            if state != git2::RepositoryState::Clean {
                return Err(GitEngineError::Sidecar {
                    message: "Cannot amend while an operation is in progress — continue or abort it first"
                        .into(),
                });
            }
            // Only "no commit present" is NothingToAmend; real errors (corrupt
            // refs, I/O) are passed through with their cause.
            let head_commit = match repo.head().and_then(|h| h.peel_to_commit()) {
                Ok(c) => c,
                Err(e) if is_unborn(&e) => return Err(GitEngineError::NothingToAmend),
                Err(e) => return Err(e.into()),
            };
            let new_id =
                head_commit.amend(Some("HEAD"), None, None, None, Some(message), Some(&tree))?;
            return Ok(new_id.to_string());
        }

        // Parents: HEAD + all MERGE_HEADs during a running merge (merge commit!).
        // Without that, a commit after a conflicted pull would not record the
        // remote history as merged.
        let mut parents_owned: Vec<git2::Commit<'_>> = Vec::new();
        match repo.head() {
            Ok(head) => parents_owned.push(head.peel_to_commit()?),
            Err(e) if is_unborn(&e) => {}
            Err(e) => return Err(e.into()),
        }
        for oid in merge_oids {
            parents_owned.push(repo.find_commit(oid)?);
        }
        // Author vs. committer: on a cherry-pick a foreign commit TRAVELS — its
        // authorship still belongs to the original, only the committer is whoever
        // finishes the pick. That is exactly how `git commit` behaves when
        // finishing a conflicted cherry-pick (verified empirically), and so does
        // the sidecar path (commit.gpgsign) and the banner button
        // (`cherry-pick --continue`) — the git2 path was the only one that
        // silently replaced the foreign authorship with our own and deleted the
        // source (CHERRY_PICK_HEAD) with cleanup_state() on top.
        //
        // Not so for a REVERT: there a new commit is created that takes the change
        // back — its author is whoever reverts. Also just like git.
        let author = if matches!(
            state,
            git2::RepositoryState::CherryPick | git2::RepositoryState::CherryPickSequence
        ) {
            crate::ops::gitdir_oid(&repo, "CHERRY_PICK_HEAD")
                .and_then(|oid| repo.find_commit(oid).ok())
                .map(|c| c.author().to_owned())
                .unwrap_or_else(|| sig.to_owned())
        } else {
            sig.to_owned()
        };
        let id = {
            let parents: Vec<&git2::Commit<'_>> = parents_owned.iter().collect();
            repo.commit(Some("HEAD"), &author, &sig, message, &tree, &parents)?
        };
        if merging || sequencing {
            // Release the borrows, then clean up MERGE_HEAD/MERGE_MSG or
            // CHERRY_PICK_HEAD/REVERT_HEAD/sequencer — otherwise the repo stays
            // "in a merge"/"in a cherry-pick" as far as git is concerned.
            drop(parents_owned);
            drop(tree);
            repo.cleanup_state()?;
        }
        Ok(id.to_string())
    }
}

#[cfg(test)]
mod fast_path_tests {
    use super::*;
    use std::fs;

    fn sh(path: &Path, args: &[&str]) {
        sidecar::run_git(path, args).unwrap();
    }

    /// CRITICAL: the system-git fast path has to deliver exactly the same status
    /// as the libgit2 path — otherwise the display (fast) shows something other
    /// than what a mutation (git2) expects. Covers all kinds of change.
    #[test]
    fn fast_path_matches_git2() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();
        git2::Repository::init(path).unwrap();
        sh(path, &["config", "user.name", "T"]);
        sh(path, &["config", "user.email", "t@t.local"]);
        // Hermetic against host configuration: a global commit.gpgsign=true
        // (without a key) would otherwise make every fixture commit fail.
        sh(path, &["config", "commit.gpgsign", "false"]);

        // Base commit with several files.
        for f in ["mod.txt", "del_staged.txt", "ren_old.txt", "del_wt.txt"] {
            fs::write(path.join(f), "base\n").unwrap();
        }
        sh(path, &["add", "-A"]);
        sh(path, &["commit", "-m", "Base"]);

        // Staged: modify, add, delete, rename (identical content -> 100%).
        fs::write(path.join("mod.txt"), "staged change\n").unwrap();
        fs::write(path.join("new_staged.txt"), "new\n").unwrap();
        sh(path, &["rm", "del_staged.txt"]);
        sh(path, &["mv", "ren_old.txt", "ren_new.txt"]);
        sh(path, &["add", "-A"]);

        // Unstaged after staging: workdir modify, delete, untracked.
        fs::write(path.join("mod.txt"), "staged change\nand workdir\n").unwrap();
        fs::remove_file(path.join("del_wt.txt")).unwrap();
        fs::write(path.join("untracked.txt"), "fresh\n").unwrap();
        fs::create_dir(path.join("new_folder")).unwrap();
        fs::write(path.join("new_folder/deep.txt"), "deep\n").unwrap();

        let via_git2 = Git2Engine.status_git2(path).unwrap();
        let via_sidecar = Git2Engine.status_via_sidecar(path).unwrap();

        assert_eq!(
            via_sidecar.staged, via_git2.staged,
            "staged differs:\nsidecar={:#?}\ngit2={:#?}",
            via_sidecar.staged, via_git2.staged
        );
        assert_eq!(
            via_sidecar.unstaged, via_git2.unstaged,
            "unstaged differs:\nsidecar={:#?}\ngit2={:#?}",
            via_sidecar.unstaged, via_git2.unstaged
        );
        assert_eq!(via_sidecar.branch, via_git2.branch);
        // The ahead fallback without an upstream has to apply identically in both paths.
        assert_eq!(via_sidecar.upstream, via_git2.upstream);
        assert_eq!(via_sidecar.ahead, via_git2.ahead);
    }

    /// Regression test: the status() fast path also has to report the
    /// unpublished commits as "ahead" on a branch without an upstream
    /// (count_unpushed fallback) — otherwise the push count is missing from the
    /// toolbar on large repos. The 30k threshold is bypassed by testing the fast
    /// path core (status_fast_path) directly.
    #[test]
    fn fast_path_counts_ahead_without_upstream() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();
        git2::Repository::init(path).unwrap();
        sh(path, &["config", "user.name", "T"]);
        sh(path, &["config", "user.email", "t@t.local"]);
        sh(path, &["config", "commit.gpgsign", "false"]);

        fs::write(path.join("a.txt"), "one\n").unwrap();
        sh(path, &["add", "-A"]);
        sh(path, &["commit", "-m", "One"]);
        fs::write(path.join("a.txt"), "two\n").unwrap();
        sh(path, &["add", "-A"]);
        sh(path, &["commit", "-m", "Two"]);

        let repo = Git2Engine.open(path).unwrap();
        let st = Git2Engine.status_fast_path(repo, path).unwrap();
        assert!(st.branch.is_some());
        assert_eq!(st.upstream, None);
        assert_eq!(st.ahead, 2, "ahead has to count the unpublished commits");
    }
}

#[cfg(test)]
mod hook_tests {
    use super::pre_push_is_git_lfs;

    #[test]
    fn the_hook_git_lfs_installs_is_told_apart_from_the_users_own() {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        let hooks = repo.path().join("hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        let hook = hooks.join("pre-push");
        // What `git lfs install` writes.
        std::fs::write(
            &hook,
            "#!/bin/sh\ncommand -v git-lfs >/dev/null 2>&1 || { printf >&2 \"\\n%s\\n\\n\" \
             \"git-lfs missing\"; exit 2; }\ngit lfs pre-push \"$@\"\n",
        )
        .unwrap();
        assert!(pre_push_is_git_lfs(&repo));
        // A hook of the user's own — also one that calls git-lfs — is named.
        std::fs::write(&hook, "#!/bin/sh\nnpm test\ngit lfs pre-push \"$@\"\n").unwrap();
        assert!(!pre_push_is_git_lfs(&repo));
        std::fs::remove_file(&hook).unwrap();
        assert!(!pre_push_is_git_lfs(&repo));
    }
}
