//! Use-case orchestration: undo recording + consistency
//! guards for history-changing operations. Deliberately separate from the thin
//! `#[tauri::command]` adapter layer (commands.rs) — this module knows only
//! domain types, the engine and the undo state, no Tauri IPC.

use tauri::{AppHandle, Manager};

use tg_domain::{RepoOpState, ResetMode, UndoAction, UndoEntry};
use tg_git_engine::{error::GitEngineError, Git2Engine, GitEngine};

use crate::commands::{blocking, CmdResult};
use crate::undo::UndoState;

pub(crate) fn now_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

/// Current branch + HEAD id; `None` on detached HEAD or in an empty repo.
/// Synchronous/blocking — for calls INSIDE an already running `spawn_blocking`
/// section (e.g. under the index lock, see C11).
pub(crate) fn head_snapshot_sync(path: &str) -> Option<(String, String)> {
    let p = std::path::PathBuf::from(path);
    let info = Git2Engine.open_repo(&p).ok()?;
    let branch = info.current_branch?;
    Git2Engine
        .log(&p, 0, 1)
        .ok()?
        .first()
        .map(|c| (branch, c.id.clone()))
}

/// Async variant for commands (checkout undo, staleness pre-check).
pub(crate) async fn head_snapshot(path: &str) -> Option<(String, String)> {
    let p = path.to_string();
    blocking(move || Ok::<_, GitEngineError>(head_snapshot_sync(&p)))
        .await
        .ok()
        .flatten()
}

/// Records a move of the branch tip as an undo entry.
/// Nothing is recorded if the branch changed (that is a checkout) or if nothing
/// moved (e.g. a merge that stopped in the conflict state).
fn record_reset(
    app: &AppHandle,
    path: &str,
    op: &str,
    detail: Option<String>,
    before: (String, String),
    after: (String, String),
    mode: ResetMode,
) {
    if before.0 != after.0 || before.1 == after.1 {
        return;
    }
    app.state::<UndoState>().push(
        path,
        UndoEntry {
            op: op.into(),
            detail,
            timestamp: now_ts(),
            undo: UndoAction::ResetBranch {
                branch: before.0,
                commit: before.1,
                mode,
            },
            redo: UndoAction::ResetBranch {
                branch: after.0,
                commit: after.1,
                mode,
            },
        },
    );
}

/// Core of the undo recording: before snapshot, mutation `f` and after
/// snapshot run in ONE contiguous section. The caller holds the index lock
/// while doing so — only then can no foreign mutation slip into an await window
/// and falsify the before/after comparison. Pure and blocking, so the logic
/// stays testable without a Tauri runtime.
type Snapshot = Option<(String, String)>;
fn run_with_snapshots<F>(path: &str, f: F) -> Result<(String, Snapshot, Snapshot), GitEngineError>
where
    F: FnOnce() -> Result<String, GitEngineError>,
{
    let before = head_snapshot_sync(path);
    let result = f()?;
    let after = head_snapshot_sync(path);
    Ok((result, before, after))
}

/// Runs an engine operation and records the HEAD movement as an undo entry
/// (the pattern for merge/rebase/squash/cherry-pick/…).
/// `f` is the RAW engine operation WITHOUT its own lock: the index lock is
/// taken here around snapshots AND mutation together (C11 — the snapshots used
/// to run in their own spawn_blockings outside the lock).
pub(crate) async fn with_reset_undo<F>(
    app: &AppHandle,
    path: &str,
    op: &str,
    detail: Option<String>,
    mode: ResetMode,
    f: F,
) -> CmdResult<String>
where
    F: FnOnce() -> Result<String, GitEngineError> + Send + 'static,
{
    let p = path.to_string();
    let (result, before, after) =
        blocking(move || crate::commands::with_index_lock(|| run_with_snapshots(&p, f))).await?;
    if let (Some(b), Some(a)) = (before, after) {
        record_reset(app, path, op, detail, b, a, mode);
    }
    Ok(result)
}

/// Is the tip of `branch` still at `expected`? (consistency guard)
pub(crate) async fn head_matches(path: &str, branch: &str, expected: &str) -> bool {
    matches!(head_snapshot(path).await, Some((b, id)) if b == branch && id == expected)
}

/// Undo recording for an operation CONTINUED after conflict resolution.
/// For merge/cherry-pick/revert, undoing means a hard reset to the state before
/// the operation (the conflict continue only creates the commit now). Rebase has
/// its own backup-ref protection and a HEAD detached during the pause, whose
/// intermediate state is no sensible reset target → deliberately NOT recorded
/// here. Clean = nothing to continue.
pub(crate) fn continue_undo_label(op: RepoOpState) -> Option<(&'static str, ResetMode)> {
    match op {
        RepoOpState::Merge => Some(("merge", ResetMode::Hard)),
        RepoOpState::Cherrypick => Some(("cherryPick", ResetMode::Hard)),
        RepoOpState::Revert => Some(("revert", ResetMode::Hard)),
        // Bisect has no continue (good/bad/skip); nothing to continue.
        RepoOpState::Rebase | RepoOpState::Bisect | RepoOpState::Clean => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continue_records_conflict_resolution_as_undo() {
        // merge/cherry-pick/revert after a conflict continue can be undone
        // (hard reset to the state before the operation) …
        assert_eq!(
            continue_undo_label(RepoOpState::Merge),
            Some(("merge", ResetMode::Hard))
        );
        assert_eq!(
            continue_undo_label(RepoOpState::Cherrypick),
            Some(("cherryPick", ResetMode::Hard))
        );
        assert_eq!(
            continue_undo_label(RepoOpState::Revert),
            Some(("revert", ResetMode::Hard))
        );
        // … rebase has its own backup-ref protection, clean has nothing to continue.
        assert_eq!(continue_undo_label(RepoOpState::Rebase), None);
        assert_eq!(continue_undo_label(RepoOpState::Clean), None);
    }

    #[test]
    fn snapshots_enclose_the_mutation() {
        // C11: run_with_snapshots must deliver the state IMMEDIATELY before and
        // after the mutation (no await window in between) — before points at the
        // old HEAD, after exactly at the result of the mutation.
        use tg_git_engine::prelude::*;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();
        Git2Engine.init_repo(path).unwrap();
        for (k, v) in [
            ("user.name", "T"),
            ("user.email", "t@t.local"),
            // Hermetic against host configuration (global gpgsign).
            ("commit.gpgsign", "false"),
        ] {
            Git2Engine.config_set(path, k, v, false).unwrap();
        }
        std::fs::write(path.join("a.txt"), "one\n").unwrap();
        Git2Engine.stage(path, &["a.txt".to_string()]).unwrap();
        let first = Git2Engine.commit(path, "First", false).unwrap();

        let p = path.to_string_lossy().into_owned();
        let (result, before, after) = run_with_snapshots(&p, || {
            std::fs::write(path.join("a.txt"), "two\n")?;
            Git2Engine.stage(path, &["a.txt".to_string()])?;
            Git2Engine.commit(path, "Second", false)
        })
        .unwrap();

        let (b_branch, b_id) = before.expect("before snapshot present");
        let (a_branch, a_id) = after.expect("after snapshot present");
        assert_eq!(b_id, first, "before = state BEFORE the mutation");
        assert_eq!(a_id, result, "after = new HEAD AFTER the mutation");
        assert_eq!(b_branch, a_branch, "same branch before/after");
    }
}
