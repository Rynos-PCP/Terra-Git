//! Status fast path via the system git (`status --porcelain=v2 -z`).
//!
//! Motivation (re-prioritized 2026-07-06): on large
//! worktrees `git status` clearly beats the libgit2 status (parallel lstat via
//! core.preloadIndex, untrackedCache, fsmonitor) — and is semantically identical
//! to the CLI by definition. The parser maps the porcelain v2 output onto
//! exactly the same [`RepoStatus`] structures as the git2 path (equivalence test
//! in `tests/engine_tests.rs`).
//!
//! Format reference: git-status(1), section "Porcelain Format Version 2".
//! With `-z` records are NUL-terminated; for `2` records the original path
//! follows as its own NUL-separated field.

use std::path::Path;

use tg_domain::{ChangeKind, RepoStatus, StatusEntry};

use crate::error::Result;
use crate::sidecar;

/// From this index size on, the process spawn (~50 ms base cost) pays off
/// against the single-threaded libgit2 scan. Empirically determined crossover
/// (see docs/PERFORMANCE.md, Win11/NTFS): git2 ~28 ms @ 15k, ~87 ms @ 50k; the
/// sidecar stays nearly flat at ~55 ms. Break-even ~30k — below that git2 is
/// faster, so only use the sidecar from here on.
pub(crate) const FAST_PATH_MIN_INDEX_ENTRIES: usize = 30_000;

/// Arguments of the status read.
///
/// `--no-optional-locks` is essential: otherwise `git status` opportunistically
/// takes `.git/index.lock` and rewrites `.git/index` (fsmonitor/untrackedCache
/// token). On large worktrees (>= FAST_PATH_MIN_INDEX_ENTRIES) that index write
/// triggers a watcher self-trigger loop: status -> index write -> repo-changed
/// -> status -> …, visible as `.git/index.lock` being created/removed
/// continuously. With optional locks disabled the read is purely read-only and
/// never touches the index.
///
/// `--untracked-files=all`: individual files instead of collapsed directories
/// (equivalent to recurse_untracked_dirs on the git2 path).
///
/// As a main git option, `--no-optional-locks` has to come BEFORE the `status`
/// subcommand; run_git_raw places it after `-C <dir>`, which is valid.
fn status_args() -> [&'static str; 6] {
    [
        "--no-optional-locks",
        "status",
        "--porcelain=v2",
        "--branch",
        "--untracked-files=all",
        "-z",
    ]
}

/// Fetches the status via the system git and parses porcelain v2.
/// `op_state` is supplied by the caller (it still comes from git2 — cheap and
/// independent of the worktree scan).
pub(crate) fn status_via_git(path: &Path) -> Result<PorcelainStatus> {
    let raw = sidecar::run_git_raw(path, &status_args())?;
    Ok(parse_porcelain_v2(&raw))
}

/// Intermediate result of the parser (the caller adds op_state).
pub(crate) struct PorcelainStatus {
    pub staged: Vec<StatusEntry>,
    pub unstaged: Vec<StatusEntry>,
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: usize,
    pub behind: usize,
}

impl PorcelainStatus {
    pub fn into_repo_status(self, op_state: tg_domain::RepoOpState) -> RepoStatus {
        RepoStatus {
            staged: self.staged,
            unstaged: self.unstaged,
            branch: self.branch,
            upstream: self.upstream,
            ahead: self.ahead,
            behind: self.behind,
            op_state,
        }
    }
}

/// Index side (X) -> ChangeKind of the staged entry.
fn staged_kind(x: char) -> Option<ChangeKind> {
    match x {
        'A' => Some(ChangeKind::Added),
        'M' => Some(ChangeKind::Modified),
        'D' => Some(ChangeKind::Deleted),
        // Copies (status.renames=copies) are treated like renames:
        // new path + origin.
        'R' | 'C' => Some(ChangeKind::Renamed),
        'T' => Some(ChangeKind::Typechange),
        _ => None,
    }
}

/// Worktree side (Y) -> ChangeKind of the unstaged entry.
fn unstaged_kind(y: char) -> Option<ChangeKind> {
    match y {
        'M' => Some(ChangeKind::Modified),
        'D' => Some(ChangeKind::Deleted),
        'R' | 'C' => Some(ChangeKind::Renamed),
        'T' => Some(ChangeKind::Typechange),
        // Intent-to-add (git add -N): the content is not in the index yet —
        // for the UI that is the same as untracked (git2 reports WT_NEW).
        'A' => Some(ChangeKind::Untracked),
        _ => None,
    }
}

fn parse_porcelain_v2(raw: &str) -> PorcelainStatus {
    let mut st = PorcelainStatus {
        staged: Vec::new(),
        unstaged: Vec::new(),
        branch: None,
        upstream: None,
        ahead: 0,
        behind: 0,
    };
    let mut unborn = false;

    let mut records = raw.split('\0');
    while let Some(rec) = records.next() {
        if rec.is_empty() {
            continue;
        }
        match rec.as_bytes()[0] {
            b'#' => {
                if let Some(v) = rec.strip_prefix("# branch.oid ") {
                    unborn = v == "(initial)";
                } else if let Some(v) = rec.strip_prefix("# branch.head ") {
                    st.branch = (v != "(detached)").then(|| v.to_string());
                } else if let Some(v) = rec.strip_prefix("# branch.upstream ") {
                    st.upstream = Some(v.to_string());
                } else if let Some(v) = rec.strip_prefix("# branch.ab ") {
                    for part in v.split(' ') {
                        if let Some(a) = part.strip_prefix('+') {
                            st.ahead = a.parse().unwrap_or(0);
                        } else if let Some(b) = part.strip_prefix('-') {
                            st.behind = b.parse().unwrap_or(0);
                        }
                    }
                }
            }
            b'?' => {
                if let Some(p) = rec.strip_prefix("? ") {
                    st.unstaged.push(StatusEntry {
                        path: p.to_string(),
                        orig_path: None,
                        kind: ChangeKind::Untracked,
                    });
                }
            }
            b'1' => {
                // 1 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <path>
                let mut it = rec.splitn(9, ' ');
                let (Some(_), Some(xy), Some(sub)) = (it.next(), it.next(), it.next()) else {
                    continue;
                };
                let Some(path) = it.nth(5) else { continue };
                push_changed(&mut st, xy, sub, path, None);
            }
            b'2' => {
                // 2 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <X><score> <path> NUL <origPath>
                let mut it = rec.splitn(10, ' ');
                let (Some(_), Some(xy), Some(sub)) = (it.next(), it.next(), it.next()) else {
                    continue;
                };
                let Some(path) = it.nth(6) else { continue };
                let orig = records.next().map(str::to_string);
                push_changed(&mut st, xy, sub, path, orig);
            }
            b'u' => {
                // u <XY> <sub> <m1> <m2> <m3> <mW> <h1> <h2> <h3> <path>
                let mut it = rec.splitn(11, ' ');
                let (Some(_), _, Some(sub)) = (it.next(), it.next(), it.next()) else {
                    continue;
                };
                if sub.starts_with('S') {
                    continue; // Submodule — the equivalent of exclude_submodules
                }
                let Some(path) = it.nth(7) else { continue };
                st.unstaged.push(StatusEntry {
                    path: path.to_string(),
                    orig_path: None,
                    kind: ChangeKind::Conflicted,
                });
            }
            _ => {}
        }
    }

    // Equivalence with the git2 path: an unborn HEAD has no branch name.
    if unborn {
        st.branch = None;
    }

    st.staged.sort_by(|a, b| a.path.cmp(&b.path));
    st.unstaged.sort_by(|a, b| a.path.cmp(&b.path));
    st
}

/// Books a `1`/`2` record onto the staged and/or unstaged list.
fn push_changed(st: &mut PorcelainStatus, xy: &str, sub: &str, path: &str, orig: Option<String>) {
    if sub.starts_with('S') {
        return; // hide submodule entries (like exclude_submodules)
    }
    let mut chars = xy.chars();
    let (Some(x), Some(y)) = (chars.next(), chars.next()) else {
        return;
    };
    if let Some(kind) = staged_kind(x) {
        st.staged.push(StatusEntry {
            path: path.to_string(),
            orig_path: orig.clone().filter(|_| kind == ChangeKind::Renamed),
            kind,
        });
    }
    if let Some(kind) = unstaged_kind(y) {
        st.unstaged.push(StatusEntry {
            path: path.to_string(),
            orig_path: orig.filter(|_| kind == ChangeKind::Renamed),
            kind,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_porcelain_v2_records() {
        // Synthetic output: header, modified, renamed, untracked, conflicted
        // and a submodule (which has to be filtered out).
        let raw = concat!(
            "# branch.oid 1234567890abcdef\0",
            "# branch.head main\0",
            "# branch.upstream origin/main\0",
            "# branch.ab +2 -1\0",
            "1 M. N... 100644 100644 100644 abc def stag.txt\0",
            "1 .M N... 100644 100644 100644 abc def work.txt\0",
            "1 MM N... 100644 100644 100644 abc def both.txt\0",
            "1 .M S.M. 160000 160000 160000 abc def submodule\0",
            "2 R. N... 100644 100644 100644 abc def R100 new.txt\0old.txt\0",
            "? unknown with spaces.txt\0",
            "u UU N... 100644 100644 100644 100644 a b c conflict.txt\0",
        );
        let st = parse_porcelain_v2(raw);

        assert_eq!(st.branch.as_deref(), Some("main"));
        assert_eq!(st.upstream.as_deref(), Some("origin/main"));
        assert_eq!(st.ahead, 2);
        assert_eq!(st.behind, 1);

        let staged: Vec<_> = st.staged.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(staged, vec!["both.txt", "new.txt", "stag.txt"]);
        let renamed = st.staged.iter().find(|e| e.path == "new.txt").unwrap();
        assert_eq!(renamed.kind, ChangeKind::Renamed);
        assert_eq!(renamed.orig_path.as_deref(), Some("old.txt"));

        let unstaged: Vec<_> = st
            .unstaged
            .iter()
            .map(|e| (e.path.as_str(), e.kind))
            .collect();
        assert_eq!(
            unstaged,
            vec![
                ("both.txt", ChangeKind::Modified),
                ("conflict.txt", ChangeKind::Conflicted),
                ("unknown with spaces.txt", ChangeKind::Untracked),
                ("work.txt", ChangeKind::Modified),
            ]
        );
    }

    #[test]
    fn status_args_contain_no_optional_locks() {
        let args = status_args();
        assert!(
            args.contains(&"--no-optional-locks"),
            "the status read has to set --no-optional-locks, otherwise git takes index.lock \
             and rewrites the index (watcher self-trigger on large worktrees)"
        );
        // Has to come before the subcommand as a main option.
        let no_lock = args.iter().position(|a| *a == "--no-optional-locks");
        let status = args.iter().position(|a| *a == "status");
        assert!(
            no_lock < status,
            "--no-optional-locks has to come before `status`"
        );
    }

    #[test]
    fn unborn_and_detached_return_no_branch() {
        let unborn = parse_porcelain_v2("# branch.oid (initial)\0# branch.head main\0");
        assert_eq!(unborn.branch, None);
        let detached = parse_porcelain_v2("# branch.oid abc\0# branch.head (detached)\0");
        assert_eq!(detached.branch, None);
    }
}
