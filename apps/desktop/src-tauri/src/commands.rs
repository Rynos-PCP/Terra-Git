//! Tauri commands: a thin edge over the git engine.
//!
//! Every command is `async` and moves the blocking engine work off the main
//! thread via `spawn_blocking` — the UI never freezes.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;

use tauri::AppHandle;
use tauri::Emitter;
use tauri::Manager;
use tg_auth::{KeyringStore, TokenStore};
use tg_domain::{
    BackupInfo, BlameLine, BranchInfo, ChangeRequest, ChangeRequestList, CloneOptions, CommitInfo,
    FileDiff, GitProgress, ImageDiff, NewChangeRequest, OpContext, ProviderAccount, ProviderKind,
    RemoteInfo, RepoInfo, RepoOpState, RepoStatus, ResetMode, StashInfo, SubmoduleInfo, TagInfo,
    UnchangedInfo, UndoAction, UndoEntry, UndoStatus, UnpushedCommit, WorktreeInfo,
};
use tg_git_engine::{error::GitEngineError, prelude::*};
use tg_providers::{parse_remote_url, ProviderClient};

use crate::error::CommandError;
use crate::op_registry::OpRegistry;
use crate::providers;
use crate::recents;
use crate::undo::UndoState;
use crate::watcher;

pub(crate) type CmdResult<T> = Result<T, CommandError>;

/// Serializes all index-mutating operations (defense in depth against parallel
/// writes -> libgit2 lock errors, e.g. on a double click).
static INDEX_LOCK: Mutex<()> = Mutex::new(());

pub(crate) fn with_index_lock<T>(
    op: impl FnOnce() -> Result<T, GitEngineError>,
) -> Result<T, GitEngineError> {
    let _guard = INDEX_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    op()
}

/// Runs a blocking engine operation on the blocking thread pool.
pub(crate) async fn blocking<T, F>(op: F) -> CmdResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, GitEngineError> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(op)
        .await
        .map_err(|e| CommandError::internal(format!("task error: {e}")))?
        .map_err(Into::into)
}

#[tauri::command]
pub async fn open_repository(app: AppHandle, path: String) -> CmdResult<RepoInfo> {
    let info = blocking(move || {
        let p = PathBuf::from(path);
        let info = Git2Engine.open_repo(&p)?;
        // Enable the status accelerators for large worktrees once when opening;
        // best effort, does not block opening on an older git
        // (the fallback to the git2 status applies anyway).
        Git2Engine.enable_status_accelerators(&p);
        Ok(info)
    })
    .await?;
    recents::add(&app, &info.path);
    // Maintain the commit graph in the background (the basis for streaming the
    // history, see docs/perf-stress-test.md) and report when it is done: for a
    // freshly cloned huge repo the frontend shows the "preparing history" hint
    // until then (info.history_prepared == false).
    {
        let app = app.clone();
        let repo_path = info.path.clone();
        let p = PathBuf::from(&repo_path);
        tauri::async_runtime::spawn_blocking(move || {
            let _ = Git2Engine.write_commit_graph(&p);
            let _ = app.emit("history-prepared", repo_path);
        });
    }
    Ok(info)
}

#[tauri::command]
pub async fn get_recent_repos(app: AppHandle) -> Vec<recents::RecentEntry> {
    // recents::list() stats up to 15 paths (is_dir) — with disconnected network
    // drives that blocks for seconds, so never on the main thread.
    tauri::async_runtime::spawn_blocking(move || recents::list(&app))
        .await
        .unwrap_or_default()
}

#[tauri::command]
pub async fn remove_recent_repo(app: AppHandle, path: String) -> CmdResult<()> {
    crate::recents::remove(&app, &path);
    Ok(())
}

#[tauri::command]
pub async fn set_recent_pinned(app: AppHandle, path: String, pinned: bool) -> CmdResult<()> {
    crate::recents::set_pinned(&app, &path, pinned);
    Ok(())
}

/// Number of commits for the welcome screen's vein sketch.
const PEEK_COMMITS: usize = 12;
/// Maximum sketched branches — more strands turn the vein into a graph.
const PEEK_BRANCHES: usize = 5;

/// Short portrait of a repo for the welcome screen: branch chip, dirty dot and
/// the vein sketch (HEAD line + local branch strands).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoPeek {
    /// Current branch, `None` on detached HEAD or in an empty repo.
    pub branch: Option<String>,
    /// Uncommitted changes present (staged OR unstaged).
    pub dirty: bool,
    /// The most recent commits on the HEAD line (newest first).
    pub commits: Vec<tg_domain::SketchCommit>,
    /// Local branches other than HEAD (newest first, capped).
    pub branches: Vec<tg_domain::SketchBranch>,
}

/// Short portrait for a recents entry, without "opening" the repo (no
/// recents::add, no watcher, no commit-graph write). Status and sketch are best
/// effort: a partial failure does not topple the peek — the UI then simply
/// shows no dot or the decorative vein.
#[tauri::command]
pub async fn peek_repo(path: String) -> CmdResult<RepoPeek> {
    blocking(move || {
        let p = PathBuf::from(&path);
        let info = Git2Engine.open_repo(&p)?;
        let dirty = Git2Engine
            .status(&p)
            .map(|s| !s.staged.is_empty() || !s.unstaged.is_empty())
            .unwrap_or(false);
        let sketch = Git2Engine
            .repo_sketch(&p, PEEK_COMMITS, PEEK_BRANCHES)
            .unwrap_or_default();
        Ok(RepoPeek {
            branch: info.current_branch,
            dirty,
            commits: sketch.commits,
            branches: sketch.branches,
        })
    })
    .await
}

#[tauri::command]
pub async fn delete_repo(app: AppHandle, path: String) -> CmdResult<()> {
    let p = PathBuf::from(&path);
    if !p.is_dir() {
        return Err(CommandError {
            code: "not_a_directory".into(),
            message: "Not a directory".into(),
        });
    }
    // Guard 1: never trash reparse points (symlink/junction) — is_dir() follows
    // the target, and a deleted link could hit someone else's data.
    if std::fs::symlink_metadata(&p)
        .map_err(|e| CommandError::internal(e.to_string()))?
        .file_type()
        .is_symlink()
    {
        return Err(CommandError {
            code: "invalid_target".into(),
            message: "Not a regular directory (reparse point)".into(),
        });
    }
    // Guard 2: the recents list as a sanity filter. IMPORTANT: it is NOT an
    // authorization boundary — the same untrusted webview fills it itself
    // (init_repository/open_repository) and could make an arbitrary path
    // "known" and then trash it. The actual approval comes from guard 3.
    if !crate::recents::is_known(&app, &path) {
        return Err(CommandError {
            code: "not_a_recent".into(),
            message: "Path is not a known repository".into(),
        });
    }
    // Guard 3: unforgeable confirmation through a NATIVE OS dialog. Every
    // #[tauri::command] is callable directly from the webview, so every frontend
    // modal can be bypassed — but a compromised renderer cannot click a native
    // dialog. Only a real "Move to trash" actually trashes.
    let confirmed = {
        let app = app.clone();
        let target = path.clone();
        tauri::async_runtime::spawn_blocking(move || {
            use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
            app.dialog()
                .message(format!(
                    "This repository will be moved to the trash:\n{target}"
                ))
                .title("Delete repository")
                .buttons(MessageDialogButtons::OkCancelCustom(
                    "Move to trash".into(),
                    "Cancel".into(),
                ))
                .kind(MessageDialogKind::Warning)
                .blocking_show()
        })
        .await
        .map_err(|e| CommandError::internal(e.to_string()))?
    };
    if !confirmed {
        return Err(CommandError {
            code: "cancelled".into(),
            message: "Deletion cancelled".into(),
        });
    }
    let moved = tauri::async_runtime::spawn_blocking(move || trash::delete(&p))
        .await
        .map_err(|e| CommandError::internal(e.to_string()))?;
    moved.map_err(|e| CommandError {
        code: "trash_failed".into(),
        message: e.to_string(),
    })?;
    crate::recents::remove(&app, &path);
    Ok(())
}

#[tauri::command]
pub async fn get_status(path: String) -> CmdResult<RepoStatus> {
    blocking(move || Git2Engine.status(&PathBuf::from(path))).await
}

#[tauri::command]
pub async fn status_numstat(path: String) -> CmdResult<Vec<tg_domain::FileLineStats>> {
    blocking(move || Git2Engine.status_numstat(&PathBuf::from(path))).await
}

#[tauri::command]
pub async fn get_log(path: String, skip: usize, limit: usize) -> CmdResult<Vec<CommitInfo>> {
    blocking(move || Git2Engine.log(&PathBuf::from(path), skip, limit)).await
}

#[tauri::command]
pub async fn get_log_all(path: String, skip: usize, limit: usize) -> CmdResult<Vec<CommitInfo>> {
    blocking(move || Git2Engine.log_all(&PathBuf::from(path), skip, limit)).await
}

#[tauri::command]
pub async fn unpushed_commits(path: String) -> CmdResult<Vec<UnpushedCommit>> {
    blocking(move || Git2Engine.unpushed_commits(&PathBuf::from(path))).await
}

#[tauri::command]
pub async fn get_file_diff(
    path: String,
    file: String,
    staged: bool,
) -> CmdResult<Option<FileDiff>> {
    blocking(move || Git2Engine.file_diff(&PathBuf::from(path), &file, staged)).await
}

/// Explains a file reported as changed without a content diff.
/// Only called when `get_file_diff` returned empty hunks.
#[tauri::command]
pub async fn explain_unchanged(
    path: String,
    file: String,
    staged: bool,
) -> CmdResult<UnchangedInfo> {
    blocking(move || Git2Engine.explain_unchanged(&PathBuf::from(path), &file, staged)).await
}

#[tauri::command]
pub async fn get_commit_diff(path: String, commit_id: String) -> CmdResult<Vec<FileDiff>> {
    blocking(move || Git2Engine.commit_diff(&PathBuf::from(path), &commit_id)).await
}

/// Streams the commit diff file by file over an IPC channel: large commits
/// appear progressively instead of as one huge package. Returns the total number
/// of files (for the truncation hint beyond `max_files`).
#[tauri::command]
pub async fn get_commit_diff_stream(
    path: String,
    commit_id: String,
    max_files: usize,
    on_file: tauri::ipc::Channel<FileDiff>,
) -> CmdResult<usize> {
    blocking(move || {
        Git2Engine.commit_diff_stream(&PathBuf::from(path), &commit_id, max_files, &mut |fd| {
            // A send error (window closed) ends the streaming cleanly.
            on_file.send(fd).is_ok()
        })
    })
    .await
}

// Undo recording + consistency guards (use-case orchestration) live in
// `crate::orchestration` — commands.rs stays the thin adapter layer.
use crate::orchestration::{
    continue_undo_label, head_matches, head_snapshot, now_ts, with_reset_undo,
};

#[tauri::command]
pub async fn undo_status(app: AppHandle, path: String) -> CmdResult<UndoStatus> {
    Ok(app.state::<UndoState>().status(&path))
}

#[tauri::command]
pub async fn undo_last(app: AppHandle, path: String) -> CmdResult<UndoEntry> {
    let state = app.state::<UndoState>();
    let Some(entry) = state.pop_undo(&path) else {
        return Err(CommandError {
            code: "nothing_to_undo".into(),
            message: "Nothing to undo".into(),
        });
    };
    // Guard: the branch must still sit where the operation left it — otherwise
    // other changes arrived in the meantime.
    if let (
        UndoAction::ResetBranch { branch, .. },
        UndoAction::ResetBranch {
            commit: expected, ..
        },
    ) = (&entry.undo, &entry.redo)
    {
        if !head_matches(&path, branch, expected).await {
            state.push_undo_back(&path, entry);
            return Err(CommandError {
                code: "undo_stale".into(),
                message:
                    "The branch has changed in the meantime — undoing this step is no longer safe"
                        .into(),
            });
        }
    }
    let action = entry.undo.clone();
    // F15: pass the expected branch tip (the state AFTER the original operation)
    // down to the engine — the reliable staleness comparison happens there UNDER
    // the index lock; the guard above is only the fast pre-check with a nice
    // message (between it and the apply there is an await window in which a
    // second command can commit).
    let expected_tip = match (&entry.undo, &entry.redo) {
        (UndoAction::ResetBranch { .. }, UndoAction::ResetBranch { commit, .. }) => {
            Some(commit.clone())
        }
        _ => None,
    };
    let p = PathBuf::from(&path);
    match blocking(move || {
        with_index_lock(|| Git2Engine.apply_undo_action(&p, &action, expected_tip.as_deref()))
    })
    .await
    {
        Ok(()) => {
            state.push_redo(&path, entry.clone());
            Ok(entry)
        }
        Err(e) => {
            state.push_undo_back(&path, entry);
            Err(e)
        }
    }
}

#[tauri::command]
pub async fn redo_last(app: AppHandle, path: String) -> CmdResult<UndoEntry> {
    let state = app.state::<UndoState>();
    let Some(entry) = state.pop_redo(&path) else {
        return Err(CommandError {
            code: "nothing_to_redo".into(),
            message: "Nothing to redo".into(),
        });
    };
    if let (
        UndoAction::ResetBranch {
            commit: expected, ..
        },
        UndoAction::ResetBranch { branch, .. },
    ) = (&entry.undo, &entry.redo)
    {
        if !head_matches(&path, branch, expected).await {
            state.push_redo(&path, entry);
            return Err(CommandError {
                code: "undo_stale".into(),
                message: "The branch has changed in the meantime — redo is no longer safe".into(),
            });
        }
    }
    let action = entry.redo.clone();
    // F15 (as in undo_last): expected tip for the engine guard under the lock —
    // for redo that is the state the undo left behind.
    let expected_tip = match (&entry.undo, &entry.redo) {
        (UndoAction::ResetBranch { commit, .. }, UndoAction::ResetBranch { .. }) => {
            Some(commit.clone())
        }
        _ => None,
    };
    let p = PathBuf::from(&path);
    match blocking(move || {
        with_index_lock(|| Git2Engine.apply_undo_action(&p, &action, expected_tip.as_deref()))
    })
    .await
    {
        Ok(()) => {
            state.push_undo_back(&path, entry.clone());
            Ok(entry)
        }
        Err(e) => {
            state.push_redo(&path, entry);
            Err(e)
        }
    }
}

#[tauri::command]
pub async fn stage_files(path: String, files: Vec<String>) -> CmdResult<()> {
    blocking(move || with_index_lock(|| Git2Engine.stage(&PathBuf::from(path), &files))).await
}

#[tauri::command]
pub async fn unstage_files(path: String, files: Vec<String>) -> CmdResult<()> {
    blocking(move || with_index_lock(|| Git2Engine.unstage(&PathBuf::from(path), &files))).await
}

#[tauri::command]
pub async fn discard_files(path: String, files: Vec<String>) -> CmdResult<()> {
    blocking(move || with_index_lock(|| Git2Engine.discard(&PathBuf::from(path), &files))).await
}

#[tauri::command]
pub async fn create_commit(
    app: AppHandle,
    path: String,
    message: String,
    amend: bool,
) -> CmdResult<String> {
    let p = path.clone();
    with_reset_undo(
        &app,
        &path,
        if amend { "amend" } else { "commit" },
        None,
        // Soft: undoing a commit leaves the changes staged.
        // No lock of our own: with_reset_undo takes the index lock around
        // snapshots AND mutation together.
        ResetMode::Soft,
        move || Git2Engine.commit(&PathBuf::from(p), &message, amend),
    )
    .await
}

#[tauri::command]
pub async fn list_branches(path: String) -> CmdResult<Vec<BranchInfo>> {
    blocking(move || Git2Engine.branches(&PathBuf::from(path))).await
}

#[tauri::command]
pub async fn create_branch(path: String, name: String, checkout: bool) -> CmdResult<()> {
    // checkout=true writes index/workdir -> under the lock (like stage/commit).
    blocking(move || {
        with_index_lock(|| Git2Engine.create_branch(&PathBuf::from(path), &name, checkout))
    })
    .await
}

#[tauri::command]
pub async fn checkout_branch(
    app: AppHandle,
    path: String,
    name: String,
    on_progress: tauri::ipc::Channel<GitProgress>,
) -> CmdResult<()> {
    let before = head_snapshot(&path).await;
    let p = path.clone();
    let n = name.clone();
    blocking(move || {
        with_index_lock(|| {
            Git2Engine.checkout_branch_with_progress(&PathBuf::from(p), &n, &mut |pr| {
                let _ = on_progress.send(pr);
            })
        })
    })
    .await?;
    if let Some((old_branch, _)) = before {
        if old_branch != name {
            app.state::<UndoState>().push(
                &path,
                UndoEntry {
                    op: "switchBranch".into(),
                    detail: Some(name.clone()),
                    timestamp: now_ts(),
                    undo: UndoAction::Checkout { target: old_branch },
                    redo: UndoAction::Checkout { target: name },
                },
            );
        }
    }
    Ok(())
}

// ============================ Stash ============================

#[tauri::command]
pub async fn stash_list(path: String) -> CmdResult<Vec<StashInfo>> {
    blocking(move || Git2Engine.stash_list(&PathBuf::from(path))).await
}

#[tauri::command]
pub async fn stash_push(path: String, message: String, files: Vec<String>) -> CmdResult<String> {
    blocking(move || {
        with_index_lock(|| Git2Engine.stash_push(&PathBuf::from(path), &message, &files))
    })
    .await
}

#[tauri::command]
pub async fn stash_apply(path: String, index: usize) -> CmdResult<()> {
    blocking(move || with_index_lock(|| Git2Engine.stash_apply(&PathBuf::from(path), index))).await
}

#[tauri::command]
pub async fn stash_pop(path: String, index: usize) -> CmdResult<()> {
    blocking(move || with_index_lock(|| Git2Engine.stash_pop(&PathBuf::from(path), index))).await
}

#[tauri::command]
pub async fn stash_drop(app: AppHandle, path: String, index: usize) -> CmdResult<()> {
    // Remember the entry BEFORE discarding it — the stash commit survives until gc.
    let p = path.clone();
    let dropped = blocking(move || Git2Engine.stash_list(&PathBuf::from(p)))
        .await?
        .into_iter()
        .find(|s| s.index == index);
    let p = path.clone();
    blocking(move || with_index_lock(|| Git2Engine.stash_drop(&PathBuf::from(p), index))).await?;
    if let Some(s) = dropped {
        app.state::<UndoState>().push(
            &path,
            UndoEntry {
                op: "stashDrop".into(),
                detail: Some(s.message.clone()),
                timestamp: now_ts(),
                undo: UndoAction::RestoreStash {
                    message: s.message,
                    commit: s.id.clone(),
                },
                redo: UndoAction::DropStashByCommit { commit: s.id },
            },
        );
    }
    Ok(())
}

// ============================ Tags ============================

#[tauri::command]
pub async fn list_tags(path: String) -> CmdResult<Vec<TagInfo>> {
    blocking(move || Git2Engine.tags(&PathBuf::from(path))).await
}

#[tauri::command]
pub async fn create_tag(
    path: String,
    name: String,
    message: String,
    target: String,
) -> CmdResult<()> {
    blocking(move || Git2Engine.create_tag(&PathBuf::from(path), &name, &message, &target)).await
}

#[tauri::command]
pub async fn delete_tag(path: String, name: String) -> CmdResult<()> {
    blocking(move || Git2Engine.delete_tag(&PathBuf::from(path), &name)).await
}

// ======================= Branch management =======================

#[tauri::command]
pub async fn rename_branch(path: String, old: String, new: String) -> CmdResult<()> {
    blocking(move || Git2Engine.rename_branch(&PathBuf::from(path), &old, &new)).await
}

#[tauri::command]
pub async fn delete_branch(
    app: AppHandle,
    path: String,
    name: String,
    force: bool,
) -> CmdResult<()> {
    // Remember the tip BEFORE deleting (for RecreateBranch).
    let p = path.clone();
    let n = name.clone();
    let tip = blocking(move || Git2Engine.branches(&PathBuf::from(p)))
        .await?
        .into_iter()
        .find(|b| !b.is_remote && b.name == n)
        .and_then(|b| b.target_id);
    let p = path.clone();
    let n = name.clone();
    blocking(move || Git2Engine.delete_branch(&PathBuf::from(p), &n, force)).await?;
    if let Some(commit) = tip {
        app.state::<UndoState>().push(
            &path,
            UndoEntry {
                op: "deleteBranch".into(),
                detail: Some(name.clone()),
                timestamp: now_ts(),
                undo: UndoAction::RecreateBranch {
                    name: name.clone(),
                    commit,
                },
                redo: UndoAction::DeleteBranch { name },
            },
        );
    }
    Ok(())
}

#[tauri::command]
pub async fn merge_branch(app: AppHandle, path: String, name: String) -> CmdResult<String> {
    let p = path.clone();
    let n = name.clone();
    with_reset_undo(
        &app,
        &path,
        "merge",
        Some(name),
        ResetMode::Hard,
        move || Git2Engine.merge_branch(&PathBuf::from(p), &n),
    )
    .await
}

#[tauri::command]
pub async fn rebase_onto(app: AppHandle, path: String, name: String) -> CmdResult<String> {
    let p = path.clone();
    let n = name.clone();
    with_reset_undo(
        &app,
        &path,
        "rebase",
        Some(name),
        ResetMode::Hard,
        move || Git2Engine.rebase_onto(&PathBuf::from(p), &n),
    )
    .await
}

// ==================== Multi-step operations ====================
//
// No dedicated `get_op_state` command: the operation state travels along in
// every `RepoStatus` (field `opState`), and that is exactly where the frontend
// reads it. A second route to the same information was never wired up and could
// only drift apart.

/// Context of the running operation for the conflict workshop: names both sides
/// understandably (branch/commit instead of ours/theirs).
#[tauri::command]
pub async fn get_op_context(path: String) -> CmdResult<OpContext> {
    blocking(move || Git2Engine.op_context(&PathBuf::from(path))).await
}

#[tauri::command]
pub async fn abort_operation(path: String) -> CmdResult<String> {
    blocking(move || with_index_lock(|| Git2Engine.abort_operation(&PathBuf::from(path)))).await
}

#[tauri::command]
pub async fn continue_operation(app: AppHandle, path: String) -> CmdResult<String> {
    // Read the running op state before continuing — afterwards it is "clean".
    // The conflicted operations are exactly the ones most likely to need an undo;
    // without this they would (unlike the clean path) not be undoable.
    let op = {
        let p = path.clone();
        blocking(move || Git2Engine.op_state(&PathBuf::from(p)))
            .await
            .unwrap_or(RepoOpState::Clean)
    };
    let p = path.clone();
    // Raw (no lock): with_reset_undo takes the index lock itself around snapshots
    // + mutation; the non-undo path locks explicitly.
    let run = move || Git2Engine.continue_operation(&PathBuf::from(p));
    match continue_undo_label(op) {
        Some((label, mode)) => with_reset_undo(&app, &path, label, None, mode, run).await,
        None => blocking(move || with_index_lock(run)).await,
    }
}

#[tauri::command]
pub async fn resolve_conflict(path: String, file: String, ours: bool) -> CmdResult<()> {
    blocking(move || {
        with_index_lock(|| Git2Engine.resolve_conflict(&PathBuf::from(path), &file, ours))
    })
    .await
}

#[tauri::command]
pub async fn open_mergetool(path: String, file: String) -> CmdResult<String> {
    blocking(move || Git2Engine.open_mergetool(&PathBuf::from(path), &file)).await
}

#[tauri::command]
pub async fn read_conflict(path: String, file: String) -> CmdResult<tg_domain::ConflictFile> {
    blocking(move || Git2Engine.read_conflict(&PathBuf::from(path), &file)).await
}

#[tauri::command]
pub async fn save_resolution(path: String, file: String, content: String) -> CmdResult<()> {
    blocking(move || {
        with_index_lock(|| Git2Engine.save_resolution(&PathBuf::from(path), &file, &content))
    })
    .await
}

// ===================== History operations =====================

#[tauri::command]
pub async fn cherry_pick(app: AppHandle, path: String, commit_id: String) -> CmdResult<String> {
    let p = path.clone();
    let detail = Some(commit_id.chars().take(8).collect());
    with_reset_undo(
        &app,
        &path,
        "cherryPick",
        detail,
        ResetMode::Hard,
        move || Git2Engine.cherry_pick(&PathBuf::from(p), &commit_id),
    )
    .await
}

#[tauri::command]
pub async fn revert_commit(app: AppHandle, path: String, commit_id: String) -> CmdResult<String> {
    let p = path.clone();
    let detail = Some(commit_id.chars().take(8).collect());
    with_reset_undo(&app, &path, "revert", detail, ResetMode::Hard, move || {
        Git2Engine.revert_commit(&PathBuf::from(p), &commit_id)
    })
    .await
}

#[tauri::command]
pub async fn undo_last_commit(app: AppHandle, path: String) -> CmdResult<()> {
    let p = path.clone();
    with_reset_undo(
        &app,
        &path,
        "undoCommit",
        None,
        ResetMode::Soft,
        move || {
            Git2Engine
                .undo_last_commit(&PathBuf::from(p))
                .map(|()| String::new())
        },
    )
    .await?;
    Ok(())
}

#[tauri::command]
pub async fn squash_from(
    app: AppHandle,
    path: String,
    oldest_id: String,
    message: String,
) -> CmdResult<String> {
    let p = path.clone();
    with_reset_undo(&app, &path, "squash", None, ResetMode::Hard, move || {
        Git2Engine.squash_from(&PathBuf::from(p), &oldest_id, &message)
    })
    .await
}

#[tauri::command]
pub async fn create_branch_from_commit(
    path: String,
    name: String,
    commit_id: String,
    checkout: bool,
) -> CmdResult<()> {
    blocking(move || {
        with_index_lock(|| {
            Git2Engine.create_branch_from_commit(&PathBuf::from(path), &name, &commit_id, checkout)
        })
    })
    .await
}

#[tauri::command]
pub async fn checkout_commit(path: String, commit_id: String) -> CmdResult<()> {
    blocking(move || {
        with_index_lock(|| Git2Engine.checkout_commit(&PathBuf::from(path), &commit_id))
    })
    .await
}

#[tauri::command]
pub async fn search_log(path: String, query: String, limit: usize) -> CmdResult<Vec<CommitInfo>> {
    blocking(move || Git2Engine.search_log(&PathBuf::from(path), &query, limit)).await
}

// ========================= Bisect =========================

#[tauri::command]
pub async fn bisect_start(path: String, good: String, bad: Option<String>) -> CmdResult<String> {
    blocking(move || {
        with_index_lock(|| Git2Engine.bisect_start(&PathBuf::from(path), &good, bad.as_deref()))
    })
    .await
}

#[tauri::command]
pub async fn bisect_mark(path: String, action: String) -> CmdResult<String> {
    blocking(move || with_index_lock(|| Git2Engine.bisect_mark(&PathBuf::from(path), &action)))
        .await
}

#[tauri::command]
pub async fn bisect_reset(path: String) -> CmdResult<()> {
    blocking(move || with_index_lock(|| Git2Engine.bisect_reset(&PathBuf::from(path)))).await
}

#[tauri::command]
pub async fn rebase_interactive(
    app: AppHandle,
    path: String,
    base_id: String,
    steps: Vec<tg_domain::RebaseStep>,
) -> CmdResult<String> {
    let p = path.clone();
    with_reset_undo(
        &app,
        &path,
        "rebaseInteractive",
        None,
        ResetMode::Hard,
        move || Git2Engine.rebase_interactive(&PathBuf::from(p), &base_id, &steps),
    )
    .await
}

// ==================== Hunk/line staging ====================

#[tauri::command]
pub async fn apply_hunk(
    path: String,
    file: String,
    hunk_index: usize,
    unstage: bool,
) -> CmdResult<()> {
    blocking(move || {
        with_index_lock(|| Git2Engine.apply_hunk(&PathBuf::from(path), &file, hunk_index, unstage))
    })
    .await
}

#[tauri::command]
pub async fn discard_hunk(path: String, file: String, hunk_index: usize) -> CmdResult<()> {
    blocking(move || {
        with_index_lock(|| Git2Engine.discard_hunk(&PathBuf::from(path), &file, hunk_index))
    })
    .await
}

#[tauri::command]
pub async fn apply_lines(
    path: String,
    file: String,
    hunk_index: usize,
    line_indices: Vec<usize>,
    unstage: bool,
) -> CmdResult<()> {
    blocking(move || {
        with_index_lock(|| {
            Git2Engine.apply_lines(
                &PathBuf::from(path),
                &file,
                hunk_index,
                &line_indices,
                unstage,
            )
        })
    })
    .await
}

// ======================= Remotes & sync =======================

#[tauri::command]
pub async fn list_remotes(path: String) -> CmdResult<Vec<RemoteInfo>> {
    blocking(move || Git2Engine.remotes(&PathBuf::from(path))).await
}

#[tauri::command]
pub async fn push_remote(
    app: AppHandle,
    path: String,
    remote: String,
    force: bool,
    on_progress: tauri::ipc::Channel<GitProgress>,
) -> CmdResult<String> {
    let token = app.state::<OpRegistry>().register(&path);
    let p = path.clone();
    let result = blocking(move || {
        Git2Engine.push_remote_with_progress(&PathBuf::from(p), &remote, force, &token, &mut |pr| {
            let _ = on_progress.send(pr);
        })
    })
    .await;
    app.state::<OpRegistry>().unregister(&path);
    result
}

#[tauri::command]
pub async fn add_remote(path: String, name: String, url: String) -> CmdResult<()> {
    blocking(move || Git2Engine.add_remote(&PathBuf::from(path), &name, &url)).await
}

#[tauri::command]
pub async fn remove_remote(path: String, name: String) -> CmdResult<()> {
    blocking(move || Git2Engine.remove_remote(&PathBuf::from(path), &name)).await
}

#[tauri::command]
pub async fn rename_remote(path: String, old_name: String, new_name: String) -> CmdResult<()> {
    blocking(move || Git2Engine.rename_remote(&PathBuf::from(path), &old_name, &new_name)).await
}

#[tauri::command]
pub async fn set_remote_url(path: String, name: String, url: String) -> CmdResult<()> {
    blocking(move || Git2Engine.set_remote_url(&PathBuf::from(path), &name, &url)).await
}

// ============ Provider accounts & change requests ============

/// Normalizes user input like "https://gitlab.example.com/" down to the host —
/// optionally with a path part for subpath installations ("example.com/gitlab").
/// Only the host part is lower-cased, the path part is preserved (without a
/// trailing slash). An "http://" prefix is deliberately kept:
/// `ProviderClient::new` rejects http hosts with a clear message (provider API
/// over https only) instead of silently running against https.
fn normalize_host(input: &str) -> String {
    let s = input.trim();
    let (http_prefix, s) = if s
        .get(..8)
        .is_some_and(|p| p.eq_ignore_ascii_case("https://"))
    {
        ("", &s[8..])
    } else if s
        .get(..7)
        .is_some_and(|p| p.eq_ignore_ascii_case("http://"))
    {
        ("http://", &s[7..])
    } else {
        ("", s)
    };
    let s = s.trim_matches('/');
    let (host, path) = match s.split_once('/') {
        Some((h, p)) => (h, p.trim_matches('/')),
        None => (s, ""),
    };
    if host.is_empty() {
        return String::new();
    }
    let mut out = format!("{http_prefix}{}", host.to_lowercase());
    if !path.is_empty() {
        out.push('/');
        out.push_str(path);
    }
    out
}

#[cfg(test)]
mod normalize_host_tests {
    use super::normalize_host;

    #[test]
    fn host_table_scheme_subpath_and_letter_case() {
        // (input, expected normal form)
        let cases = [
            ("github.com", "github.com"),
            ("  GitHub.COM  ", "github.com"),
            ("https://gitlab.example.com/", "gitlab.example.com"),
            // Subpath installation: the path part is preserved …
            ("example.com/gitlab", "example.com/gitlab"),
            ("https://example.com/gitlab/", "example.com/gitlab"),
            // … and only the HOST part gets lower-cased.
            ("HTTPS://Example.COM/GitLab/", "example.com/GitLab"),
            // http:// is deliberately kept: ProviderClient::new rejects http with
            // a clear message (instead of silently running against https).
            ("http://gitlab.local", "http://gitlab.local"),
            // Degenerate input -> empty (the command reports invalid_host).
            ("", ""),
            ("https://", ""),
            ("http://", ""),
        ];
        for (input, expected) in cases {
            assert_eq!(normalize_host(input), expected, "input was: {input:?}");
        }
    }
}

#[cfg(test)]
mod validate_editor_tests {
    use super::validate_editor_name;

    #[test]
    fn interpreters_and_paths_are_rejected() {
        // Interpreters (also upper-cased and with a suffix) — F17.
        for bad in [
            "cmd",
            "CMD",
            "cmd.exe",
            "powershell",
            "PowerShell.EXE",
            "pwsh",
            "wscript",
            "cscript.exe",
            "mshta",
            "rundll32",
            "sh",
            "bash",
            "python",
            "python3.exe",
            "wsl",
        ] {
            assert!(
                validate_editor_name(bad).is_err(),
                "interpreter must be rejected: {bad:?}"
            );
        }
        // Paths/separators/spaces/control characters/empty.
        for bad in [
            "",
            "C:\\tools\\evil",
            "..\\evil",
            "sub/dir",
            "a b",
            "code\t",
            "code\n",
            "c:evil",
        ] {
            assert!(
                validate_editor_name(bad).is_err(),
                "invalid name must be rejected: {bad:?}"
            );
        }
    }

    #[test]
    fn normal_editor_names_stay_allowed() {
        for ok in ["code", "code.cmd", "subl", "notepad", "notepad++", "zed"] {
            assert!(
                validate_editor_name(ok).is_ok(),
                "editor must stay allowed: {ok:?}"
            );
        }
    }
}

/// Keychain access blocks → always onto the blocking pool.
async fn keyring_blocking<T, F>(op: F) -> CmdResult<T>
where
    T: Send + 'static,
    F: FnOnce(KeyringStore) -> Result<T, tg_auth::AuthError> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || op(KeyringStore::new()))
        .await
        .map_err(|e| CommandError::internal(e.to_string()))?
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn provider_accounts(app: AppHandle) -> CmdResult<Vec<ProviderAccount>> {
    Ok(providers::list(&app))
}

#[tauri::command]
pub async fn provider_add_account(
    app: AppHandle,
    host: String,
    kind: ProviderKind,
    token: String,
    insecure_tls: bool,
) -> CmdResult<ProviderAccount> {
    let host = normalize_host(&host);
    if host.is_empty() {
        return Err(CommandError {
            code: "invalid_host".into(),
            message: "Please enter a host name (e.g. github.com)".into(),
        });
    }
    let token = token.trim().to_string();
    if token.is_empty() {
        return Err(CommandError {
            code: "invalid_token".into(),
            message: "Please enter a token".into(),
        });
    }

    // Validate first (yields the user name), then store.
    let client = ProviderClient::new(kind, &host, &token, insecure_tls)?;
    let username = client.validate().await?;

    let (h, tok) = (host.clone(), token.clone());
    keyring_blocking(move |store| store.set(&h, &tok)).await?;

    let account = ProviderAccount {
        host,
        kind,
        username,
        insecure_tls,
    };
    providers::upsert(&app, account.clone());
    Ok(account)
}

#[tauri::command]
pub async fn provider_remove_account(app: AppHandle, host: String) -> CmdResult<()> {
    providers::remove(&app, &host);
    keyring_blocking(move |store| store.delete(&host)).await
}

/// Resolved provider context of a repo: account + token + remote target.
struct ProviderContext {
    account: ProviderAccount,
    client: ProviderClient,
    repo_path: String,
}

/// Determines the remote (origin preferred), account and token for a repo and
/// builds the authenticated client. Stable error codes: `no_remote`,
/// `no_account` (the frontend shows targeted hints for these).
async fn provider_context(app: &AppHandle, path: &str) -> CmdResult<ProviderContext> {
    let repo_path = PathBuf::from(path);
    let remotes = blocking(move || Git2Engine.remotes(&repo_path)).await?;
    let remote = remotes
        .iter()
        .find(|r| r.name == "origin")
        .or_else(|| remotes.first())
        .ok_or_else(|| CommandError {
            code: "no_remote".into(),
            message: "This repository has no remote".into(),
        })?;
    let target = parse_remote_url(&remote.url).ok_or_else(|| CommandError {
        code: "no_remote".into(),
        message: format!(
            "Remote URL is not recognizable as a hosting provider: {}",
            remote.url
        ),
    })?;

    // F27: subpath accounts ("example.com/gitlab") match too — the subpath is
    // then stripped from the project path before it goes to the provider API.
    let (account, repo_path) = providers::find_for_remote(app, &target.host, &target.repo_path)
        .ok_or_else(|| CommandError {
            code: "no_account".into(),
            message: format!("No account stored for {}", target.host),
        })?;
    let host_for_token = account.host.clone();
    let token = keyring_blocking(move |store| store.get(&host_for_token))
        .await?
        .ok_or_else(|| CommandError {
            code: "no_account".into(),
            message: format!("No token for {} found in the keychain", account.host),
        })?;

    let client = ProviderClient::new(account.kind, &account.host, &token, account.insecure_tls)?;
    Ok(ProviderContext {
        account,
        client,
        repo_path,
    })
}

#[tauri::command]
pub async fn list_change_requests(app: AppHandle, path: String) -> CmdResult<ChangeRequestList> {
    let ctx = provider_context(&app, &path).await?;
    let items = ctx.client.list_change_requests(&ctx.repo_path).await?;
    Ok(ChangeRequestList {
        host: ctx.account.host,
        repo_path: ctx.repo_path,
        kind: ctx.account.kind,
        items,
    })
}

#[tauri::command]
pub async fn provider_default_branch(app: AppHandle, path: String) -> CmdResult<String> {
    let ctx = provider_context(&app, &path).await?;
    Ok(ctx.client.default_branch(&ctx.repo_path).await?)
}

#[tauri::command]
pub async fn create_change_request(
    app: AppHandle,
    path: String,
    request: NewChangeRequest,
) -> CmdResult<ChangeRequest> {
    if request.title.trim().is_empty() {
        return Err(CommandError {
            code: "invalid_title".into(),
            message: "Please enter a title".into(),
        });
    }
    let ctx = provider_context(&app, &path).await?;
    Ok(ctx
        .client
        .create_change_request(&ctx.repo_path, &request)
        .await?)
}

// ========== Pipeline cockpit: local CI testing ==========
// Detection/discovery of the configs, graph load through runner metadata,
// scope runs (pipeline/stage/job) with event streaming and cancellation.

/// Maps pipeline errors onto stable command codes: app validation carries its
/// own code (run_active, stage_not_found, invalid_target, invalid_scope), real
/// runner failure stays "runner_failed".
fn pipeline_error(e: crate::pipeline::RunError) -> CommandError {
    match e {
        crate::pipeline::RunError::Timeout => CommandError {
            code: "timeout".into(),
            message: "Pipeline run aborted: time limit reached".into(),
        },
        crate::pipeline::RunError::Rejected { code, message } => CommandError {
            code: code.into(),
            message,
        },
        crate::pipeline::RunError::Failed(message) => CommandError {
            code: "runner_failed".into(),
            message,
        },
    }
}

#[tauri::command]
pub async fn pipeline_detect(path: String) -> CmdResult<crate::pipeline::PipelineInfo> {
    tauri::async_runtime::spawn_blocking(move || crate::pipeline::detect(&PathBuf::from(path)))
        .await
        .map_err(|e| CommandError::internal(e.to_string()))
}

#[tauri::command]
pub async fn pipeline_cancel(app: AppHandle, path: String) -> CmdResult<bool> {
    let state = app.state::<crate::pipeline::PipelineState>();
    Ok(crate::pipeline::cancel(&state, &PathBuf::from(path)))
}

#[tauri::command]
pub async fn pipeline_configs(
    path: String,
) -> CmdResult<Vec<crate::pipeline_graph::PipelineConfig>> {
    tauri::async_runtime::spawn_blocking(move || {
        crate::pipeline::discover_configs(&PathBuf::from(path))
    })
    .await
    .map_err(|e| CommandError::internal(e.to_string()))
}

/// Adds a CI file chosen manually (file picker) as a configuration: derives the
/// repo-relative path + provider and checks the security guards.
#[tauri::command]
pub async fn pipeline_add_config(
    path: String,
    file_path: String,
) -> CmdResult<crate::pipeline_graph::PipelineConfig> {
    tauri::async_runtime::spawn_blocking(move || {
        crate::pipeline::config_from_path(&PathBuf::from(path), &file_path).map_err(pipeline_error)
    })
    .await
    .map_err(|e| CommandError::internal(e.to_string()))?
}

#[tauri::command]
pub async fn pipeline_graph(
    path: String,
    provider: String,
    config: String,
) -> CmdResult<crate::pipeline_graph::PipelineGraph> {
    tauri::async_runtime::spawn_blocking(move || {
        // Stable pre-check against the REQUESTED provider (same pattern as
        // pipeline_run_scope): without a runner the graph load would otherwise
        // end as a cryptic spawn error — the frontend needs the stable code to
        // show a final error state (with retry).
        if !crate::pipeline::runner_installed(&provider) {
            return Err(CommandError {
                code: "runner_not_installed".into(),
                message: "Pipeline runner is not installed".into(),
            });
        }
        crate::pipeline::load_graph(&PathBuf::from(path), &provider, &config)
            .map_err(pipeline_error)
    })
    .await
    .map_err(|e| CommandError::internal(e.to_string()))?
}

#[tauri::command]
#[allow(clippy::too_many_arguments)] // Tauri command with a grown run context (event/variables)
pub async fn pipeline_run_scope(
    app: AppHandle,
    path: String,
    provider: String,
    config: String,
    scope: String,
    target: Option<String>,
    event: Option<String>,
    variables: Option<Vec<(String, String)>>,
    on_event: tauri::ipc::Channel<crate::pipeline_graph::PipelineEvent>,
) -> CmdResult<i32> {
    tauri::async_runtime::spawn_blocking(move || {
        let repo = PathBuf::from(path);
        // Stable pre-checks (instead of a cryptic spawn error): check against the
        // REQUESTED provider, not against the heuristically auto-detected one
        // (detect() prefers gitlab over github and may not know configs found by
        // heuristics at all).
        if !crate::pipeline::runner_installed(&provider) {
            return Err(CommandError {
                code: "runner_not_installed".into(),
                message: "Pipeline runner is not installed".into(),
            });
        }
        // act ALWAYS needs Docker; gitlab-ci-local only for image: jobs (shell
        // jobs run without it) — there it stays a UI hint.
        // runner() treats every non-gitlab provider as act, so check the same way
        // here (not only for exactly "github").
        if provider != "gitlab" && !crate::pipeline::docker_running() {
            return Err(CommandError {
                code: "docker_not_running".into(),
                message: "Docker is not running".into(),
            });
        }
        // Host tools of the runner: gitlab-ci-local rsyncs the tracked files into
        // the build folder inside a bash shell — even for jobs with `image:`, so
        // before a container even starts. Without a pre-check the run only fails
        // deep inside the runner with "rsync: command not found".
        let missing = crate::pipeline::missing_host_tools(&provider);
        if !missing.is_empty() {
            return Err(CommandError {
                code: "tools_missing".into(),
                message: missing.join(", "),
            });
        }
        let state = app.state::<crate::pipeline::PipelineState>();
        let options = crate::pipeline::RunOptions {
            event,
            variables: variables.unwrap_or_default(),
        };
        crate::pipeline::run_scope(
            &state,
            &repo,
            &provider,
            &config,
            &scope,
            target.as_deref(),
            &options,
            |ev| {
                let _ = on_event.send(ev);
            },
        )
        .map_err(pipeline_error)
    })
    .await
    .map_err(|e| CommandError::internal(e.to_string()))?
}

// ==================== Sparse checkout ====================

#[tauri::command]
pub async fn sparse_status(path: String) -> CmdResult<tg_domain::SparseStatus> {
    blocking(move || Git2Engine.sparse_status(&PathBuf::from(path))).await
}

#[tauri::command]
pub async fn sparse_set(path: String, dirs: Vec<String>) -> CmdResult<()> {
    blocking(move || with_index_lock(|| Git2Engine.sparse_set(&PathBuf::from(path), &dirs))).await
}

#[tauri::command]
pub async fn sparse_disable(path: String) -> CmdResult<()> {
    blocking(move || with_index_lock(|| Git2Engine.sparse_disable(&PathBuf::from(path)))).await
}

// ================= Backups (backup refs) =================

#[tauri::command]
pub async fn list_backups(path: String) -> CmdResult<Vec<BackupInfo>> {
    blocking(move || Git2Engine.backups(&PathBuf::from(path))).await
}

#[tauri::command]
pub async fn restore_backup(app: AppHandle, path: String, ref_name: String) -> CmdResult<String> {
    let p = path.clone();
    with_reset_undo(
        &app,
        &path,
        "restoreBackup",
        None,
        ResetMode::Hard,
        move || Git2Engine.restore_backup(&PathBuf::from(p), &ref_name),
    )
    .await
}

#[tauri::command]
pub async fn delete_backup(path: String, ref_name: String) -> CmdResult<()> {
    blocking(move || Git2Engine.delete_backup(&PathBuf::from(path), &ref_name)).await
}

// ===================== Repo lifecycle =====================
// Cloning runs EXCLUSIVELY in two phases (clone_prepare + clone_fetch); the
// former single-shot path (clone_repository) had no callers and was removed
// (backlog A-CLONE).

/// Clone stage 1: create + open the repo (fast, no network). The frontend opens
/// the (empty) repo immediately and then calls [`clone_fetch`].
#[tauri::command]
pub async fn clone_prepare(app: AppHandle, url: String, dest_dir: String) -> CmdResult<RepoInfo> {
    let dest = PathBuf::from(dest_dir);
    let info = blocking(move || {
        Git2Engine.clone_prepare(&url, &dest)?;
        Git2Engine.open_repo(&dest)
    })
    .await?;
    recents::add(&app, &info.path);
    Ok(info)
}

/// Clone stage 2: fetch the data + check out the default branch, with progress.
#[tauri::command]
pub async fn clone_fetch(
    app: AppHandle,
    path: String,
    options: CloneOptions,
    on_progress: tauri::ipc::Channel<GitProgress>,
) -> CmdResult<String> {
    let token = app.state::<OpRegistry>().register(&path);
    let p = path.clone();
    let result = blocking(move || {
        Git2Engine.clone_fetch(&PathBuf::from(p), &options, &token, &mut |pr| {
            let _ = on_progress.send(pr);
        })
    })
    .await;
    app.state::<OpRegistry>().unregister(&path);
    result
}

#[tauri::command]
pub async fn init_repository(app: AppHandle, dir: String) -> CmdResult<RepoInfo> {
    let info = blocking(move || Git2Engine.init_repo(&PathBuf::from(dir))).await?;
    recents::add(&app, &info.path);
    Ok(info)
}

#[tauri::command]
pub async fn ignore_pattern(path: String, pattern: String) -> CmdResult<()> {
    blocking(move || Git2Engine.ignore_pattern(&PathBuf::from(path), &pattern)).await
}

// ========================= Views =========================

#[tauri::command]
pub async fn blame_file(path: String, file: String) -> CmdResult<Vec<BlameLine>> {
    blocking(move || Git2Engine.blame_file(&PathBuf::from(path), &file)).await
}

#[tauri::command]
pub async fn get_image_diff(path: String, file: String, staged: bool) -> CmdResult<ImageDiff> {
    blocking(move || Git2Engine.image_diff(&PathBuf::from(path), &file, staged)).await
}

// =================== Worktrees & submodules ===================

#[tauri::command]
pub async fn list_worktrees(path: String) -> CmdResult<Vec<WorktreeInfo>> {
    blocking(move || Git2Engine.worktrees(&PathBuf::from(path))).await
}

#[tauri::command]
pub async fn add_worktree(path: String, dest: String, branch: String) -> CmdResult<String> {
    blocking(move || Git2Engine.add_worktree(&PathBuf::from(path), &PathBuf::from(dest), &branch))
        .await
}

#[tauri::command]
pub async fn remove_worktree(path: String, worktree_path: String) -> CmdResult<String> {
    blocking(move || Git2Engine.remove_worktree(&PathBuf::from(path), &worktree_path)).await
}

#[tauri::command]
pub async fn list_submodules(path: String) -> CmdResult<Vec<SubmoduleInfo>> {
    blocking(move || Git2Engine.submodules(&PathBuf::from(path))).await
}

#[tauri::command]
pub async fn update_submodules(path: String) -> CmdResult<String> {
    blocking(move || Git2Engine.update_submodules(&PathBuf::from(path))).await
}

// ======================= Configuration =======================

#[tauri::command]
pub async fn config_get(path: String, key: String) -> CmdResult<Option<String>> {
    blocking(move || Git2Engine.config_get(&PathBuf::from(path), &key)).await
}

#[tauri::command]
pub async fn config_set(path: String, key: String, value: String, global: bool) -> CmdResult<()> {
    blocking(move || Git2Engine.config_set(&PathBuf::from(path), &key, &value, global)).await
}

#[tauri::command]
pub async fn check_signing(path: String) -> CmdResult<String> {
    blocking(move || Git2Engine.check_signing(&PathBuf::from(path))).await
}

// ================== System integrations ==================

fn spawn_detached(mut cmd: Command) -> CmdResult<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0000_0008); // DETACHED_PROCESS
    }
    cmd.spawn()
        .map(|_| ())
        .map_err(|e| CommandError::internal(format!("Failed to start: {e}")))
}

/// Opens the file explorer in the repo directory.
///
/// Only DIRECTORIES: `explorer`/`open`/`xdg-open` perform a shell-association
/// launch, so handing them a file path would execute an .exe/.bat/.lnk. The
/// command is reachable directly from the (untrusted) webview, and the clone
/// commands let it choose where a payload lands — without this check it is an
/// "open anything" primitive equivalent to the interpreters in EDITOR_DENYLIST.
#[tauri::command]
pub fn open_in_explorer(path: String) -> CmdResult<()> {
    if !std::path::Path::new(&path).is_dir() {
        return Err(CommandError {
            code: "invalid_path".into(),
            message: format!("Not a directory: {path}"),
        });
    }
    #[cfg(target_os = "windows")]
    {
        let mut c = Command::new("explorer");
        c.arg(&path);
        spawn_detached(c)
    }
    #[cfg(target_os = "macos")]
    {
        let mut c = Command::new("open");
        c.arg(&path);
        spawn_detached(c)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let mut c = Command::new("xdg-open");
        c.arg(&path);
        spawn_detached(c)
    }
}

/// Interpreters that a compromised webview could abuse through the `editor`
/// parameter to run arbitrary commands. Compared case-insensitively and
/// without the .exe/.cmd/.bat/.com suffix.
const EDITOR_DENYLIST: &[&str] = &[
    "cmd",
    "command",
    "powershell",
    "powershell_ise",
    "pwsh",
    "wscript",
    "cscript",
    "mshta",
    "rundll32",
    "regsvr32",
    "wsl",
    "sh",
    "bash",
    "zsh",
    "dash",
    "ksh",
    "fish",
    "python",
    "python3",
    "pythonw",
    "py",
    "perl",
    "ruby",
    "node",
    // Shell-association launchers: they hand the path to the OS handler, which
    // EXECUTES it when it is an .exe/.bat/.lnk — the same primitive the list
    // above exists to prevent.
    "explorer",
    "open",
    "xdg-open",
    "start",
    "wt",
    "conhost",
    "rundll32",
];

/// Validates the editor name (controllable from the untrusted webview): a bare
/// program NAME only — no path separators (`\ / :`), no `..`, no
/// whitespace/control characters and no interpreter from the denylist.
/// Otherwise a compromised webview could start arbitrary programs with a
/// controlled argument via `editor="powershell"`, `path="calc"`.
fn validate_editor_name(editor: &str) -> CmdResult<()> {
    let malformed = editor.is_empty()
        || editor.contains(['\\', '/', ':'])
        || editor.contains("..")
        || editor
            .chars()
            .any(|c| c.is_whitespace() || (c as u32) < 0x20 || c == '\u{7f}');
    if malformed {
        return Err(CommandError {
            code: "invalid_editor".into(),
            message: format!("Invalid editor name: {editor}"),
        });
    }
    let lower = editor.to_ascii_lowercase();
    let base = lower
        .strip_suffix(".exe")
        .or_else(|| lower.strip_suffix(".cmd"))
        .or_else(|| lower.strip_suffix(".bat"))
        .or_else(|| lower.strip_suffix(".com"))
        .unwrap_or(&lower);
    if EDITOR_DENYLIST.contains(&base) {
        return Err(CommandError {
            code: "invalid_editor".into(),
            message: format!("“{editor}” is not allowed as an editor (interpreter)"),
        });
    }
    Ok(())
}

/// Opens the repo in the editor (configurable, default: VS Code).
///
/// Deliberately NO `cmd /C` (metacharacter injection through path/setting): the
/// editor is started directly; on Windows the `.cmd` shim is tried in addition
/// (VS Code installs `code.cmd`). The editor name is validated and `path`
/// has to exist — a renderer therefore cannot start arbitrary programs with
/// made-up arguments.
#[tauri::command]
pub fn open_in_editor(path: String, editor: Option<String>) -> CmdResult<()> {
    let editor = editor.unwrap_or_else(|| "code".into());
    validate_editor_name(&editor)?;
    if !std::path::Path::new(&path).exists() {
        return Err(CommandError {
            code: "invalid_path".into(),
            message: format!("Path does not exist: {path}"),
        });
    }
    let mut direct = Command::new(&editor);
    direct.arg(&path);
    if spawn_detached(direct).is_ok() {
        return Ok(());
    }
    #[cfg(target_os = "windows")]
    {
        let mut shim = Command::new(format!("{editor}.cmd"));
        shim.arg(&path);
        spawn_detached(shim)
    }
    #[cfg(not(target_os = "windows"))]
    {
        Err(CommandError::internal(format!(
            "Editor “{editor}” could not be started"
        )))
    }
}

/// Opens a terminal in the repo directory.
#[tauri::command]
pub fn open_terminal(path: String) -> CmdResult<()> {
    #[cfg(target_os = "windows")]
    {
        // Windows Terminal preferred, otherwise the classic console. The path
        // NEVER goes through cmd parsing (injection): wt receives it as a real
        // argument, the cmd console only as its working directory.
        let mut wt = Command::new("wt");
        wt.args(["-d", &path]);
        if spawn_detached(wt).is_ok() {
            return Ok(());
        }
        use std::os::windows::process::CommandExt;
        let mut c = Command::new("cmd");
        c.current_dir(&path);
        c.creation_flags(0x0000_0010); // CREATE_NEW_CONSOLE
        c.spawn()
            .map(|_| ())
            .map_err(|e| CommandError::internal(format!("Failed to start the terminal: {e}")))
    }
    #[cfg(target_os = "macos")]
    {
        let mut c = Command::new("open");
        c.args(["-a", "Terminal", &path]);
        spawn_detached(c)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let mut c = Command::new("x-terminal-emulator");
        c.current_dir(&path);
        spawn_detached(c)
    }
}

/// Opens a URL in the default browser (http/https only).
#[tauri::command]
pub fn open_external(url: String) -> CmdResult<()> {
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return Err(CommandError::internal("Only http(s) URLs are allowed"));
    }
    #[cfg(target_os = "windows")]
    {
        // rundll32 instead of `cmd /C start`: no cmd metacharacter parsing.
        let mut c = Command::new("rundll32");
        c.args(["url.dll,FileProtocolHandler", &url]);
        spawn_detached(c)
    }
    #[cfg(target_os = "macos")]
    {
        let mut c = Command::new("open");
        c.arg(&url);
        spawn_detached(c)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let mut c = Command::new("xdg-open");
        c.arg(&url);
        spawn_detached(c)
    }
}

/// Starts another terra-git window (community request #3606).
#[tauri::command]
pub fn new_window() -> CmdResult<()> {
    let exe = std::env::current_exe()
        .map_err(|e| CommandError::internal(format!("Own executable not found: {e}")))?;
    spawn_detached(Command::new(exe))
}

/// Opens the log directory in the file explorer (for bug reports/diagnosis).
#[tauri::command]
pub fn open_logs(app: AppHandle) -> CmdResult<()> {
    let dir = crate::logging::log_dir(&app)
        .ok_or_else(|| CommandError::internal("Log directory unavailable"))?;
    let dir = dir.to_string_lossy().into_owned();
    open_in_explorer(dir)
}

// ======================= File watcher =======================

/// Watches the repo directory; changes arrive as a `repo-changed` event.
///
/// Async + blocking pool: the recursive watcher registration (inotify) or
/// the initial PollWatcher scan takes seconds on large repos — on the main
/// thread that would freeze the UI. `WatchState` is a clonable Arc because
/// `tauri::State` itself is not `'static`.
#[tauri::command]
pub async fn watch_repository(
    app: AppHandle,
    state: tauri::State<'_, watcher::WatchState>,
    path: String,
) -> CmdResult<()> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || watcher::watch(&app, &state, path))
        .await
        .map_err(|e| CommandError::internal(format!("task error: {e}")))?
        .map_err(CommandError::internal)
}

/// Stops watching. Also on the blocking pool: dropping the old watcher cleans up
/// the OS registration (large repo: many watches).
#[tauri::command]
pub async fn unwatch_repository(state: tauri::State<'_, watcher::WatchState>) -> CmdResult<()> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || watcher::unwatch(&state))
        .await
        .map_err(|e| CommandError::internal(format!("task error: {e}")))
}

#[tauri::command]
pub async fn git_fetch(
    app: AppHandle,
    path: String,
    on_progress: tauri::ipc::Channel<GitProgress>,
) -> CmdResult<String> {
    let token = app.state::<OpRegistry>().register(&path);
    let p = path.clone();
    let result = blocking(move || {
        Git2Engine.fetch_with_progress(&PathBuf::from(p), &token, &mut |pr| {
            let _ = on_progress.send(pr);
        })
    })
    .await;
    app.state::<OpRegistry>().unregister(&path);
    result
}

#[tauri::command]
pub async fn git_pull(
    app: AppHandle,
    path: String,
    // Optional so a missing key (frontend without prune) arrives as `None` ->
    // false instead of failing argument deserialization: default-off stays
    // behavior-neutral that way (Tauri v2 defaults missing Option args to None).
    prune: Option<bool>,
    on_progress: tauri::ipc::Channel<GitProgress>,
) -> CmdResult<String> {
    // pull = fetch + merge/rebase -> writes index/workdir, hence under the lock.
    let prune = prune.unwrap_or(false);
    let token = app.state::<OpRegistry>().register(&path);
    let p = path.clone();
    let result = blocking(move || {
        with_index_lock(|| {
            Git2Engine.pull_with_progress(&PathBuf::from(p), prune, &token, &mut |pr| {
                let _ = on_progress.send(pr);
            })
        })
    })
    .await;
    app.state::<OpRegistry>().unregister(&path);
    result
}

#[tauri::command]
pub async fn git_push(
    app: AppHandle,
    path: String,
    on_progress: tauri::ipc::Channel<GitProgress>,
) -> CmdResult<String> {
    let token = app.state::<OpRegistry>().register(&path);
    let p = path.clone();
    let result = blocking(move || {
        Git2Engine.push_with_progress(&PathBuf::from(p), &token, &mut |pr| {
            let _ = on_progress.send(pr);
        })
    })
    .await;
    app.state::<OpRegistry>().unregister(&path);
    result
}

/// Cancels the running remote operation (fetch/pull/push/clone_fetch) for
/// `path`. Returns `true` if one was running. The git child process is killed
/// immediately: cancel allows an immediate abort from the UI.
#[tauri::command]
pub async fn cancel_operation(app: AppHandle, path: String) -> bool {
    app.state::<OpRegistry>().cancel(&path)
}

// ── SSH key manager ─────────────────────────────────────────────────────────
// Creating/managing local SSH keys + known_hosts TOFU. Pure process/file work
// through the OpenSSH CLIs — no repo/index needed, therefore directly via
// `blocking` (no index lock).

#[tauri::command]
pub async fn ssh_list_keys() -> CmdResult<Vec<tg_domain::SshKey>> {
    blocking(tg_git_engine::ssh::list_keys).await
}

#[tauri::command]
pub async fn ssh_generate_key(
    name: String,
    comment: String,
    passphrase: String,
) -> CmdResult<tg_domain::SshKey> {
    blocking(move || tg_git_engine::ssh::generate_key(&name, &comment, &passphrase)).await
}

#[tauri::command]
pub async fn ssh_scan_host(host: String, port: Option<u16>) -> CmdResult<tg_domain::ScannedHost> {
    blocking(move || tg_git_engine::ssh::scan_host(&host, port)).await
}

#[tauri::command]
pub async fn ssh_trust_host(
    host: String,
    port: Option<u16>,
    lines: String,
    replace: bool,
) -> CmdResult<()> {
    blocking(move || tg_git_engine::ssh::trust_host(&host, port, &lines, replace)).await
}

#[tauri::command]
pub async fn ssh_remove_key(app: AppHandle, name: String) -> CmdResult<()> {
    // Security guard as in delete_repo (guard 3): removing a private SSH key is
    // callable directly from the (untrusted) webview, so every frontend modal can
    // be bypassed. Only a NATIVE OS dialog cannot be "clicked" by a compromised
    // renderer. Only after that does the engine trash the key pair (trash ->
    // recoverable).
    let confirmed = {
        let app = app.clone();
        let key = name.clone();
        tauri::async_runtime::spawn_blocking(move || {
            use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
            app.dialog()
                .message(format!("This SSH key will be moved to the trash:\n{key}"))
                .title("Delete SSH key")
                .buttons(MessageDialogButtons::OkCancelCustom(
                    "Move to trash".into(),
                    "Cancel".into(),
                ))
                .kind(MessageDialogKind::Warning)
                .blocking_show()
        })
        .await
        .map_err(|e| CommandError::internal(e.to_string()))?
    };
    if !confirmed {
        return Err(CommandError {
            code: "cancelled".into(),
            message: "Deletion cancelled".into(),
        });
    }
    blocking(move || tg_git_engine::ssh::remove_key(&name)).await
}
