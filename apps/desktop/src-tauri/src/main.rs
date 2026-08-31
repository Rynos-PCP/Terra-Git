//! terra-git desktop app: Tauri 2 entry point.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod credential_helper;
mod error;
mod jsonstore;
mod logging;
mod op_registry;
mod orchestration;
mod pipeline;
mod pipeline_graph;
mod providers;
mod recents;
mod undo;
mod watcher;

use tauri::Manager;

fn main() {
    // Headless invocation as a git credential helper (sidecar bridge)? Then
    // answer and exit — no Tauri runtime, no window.
    let mut cli_args = std::env::args().skip(1);
    if cli_args.next().as_deref() == Some("__credential") {
        credential_helper::run(&cli_args.next().unwrap_or_default());
        return;
    }
    // The engine only injects the helper when this variable is set — that way
    // test binaries never end up acting as the credential helper.
    if let Ok(exe) = std::env::current_exe() {
        std::env::set_var("TERRA_GIT_CREDENTIAL_EXE", exe);
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(watcher::WatchState::default())
        .manage(undo::UndoState::default())
        .manage(pipeline::PipelineState::default())
        .manage(op_registry::OpRegistry::default())
        .setup(|app| {
            // Logging into the app log directory (rotating) + panic hook.
            // Keep the guard as managed state for the app's lifetime.
            if let Some(guard) = logging::init(app.handle()) {
                app.manage(guard);
            }
            // Mirror the already configured "TLS off" hosts into the engine's
            // process state (providers::upsert/remove keep it current after that).
            // No env::set_var: at runtime that is UB next to spawn on Unix.
            providers::sync_insecure_tls_hosts(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::open_repository,
            commands::get_status,
            commands::status_numstat,
            commands::get_log,
            commands::get_log_all,
            commands::unpushed_commits,
            commands::get_file_diff,
            commands::explain_unchanged,
            commands::get_commit_diff,
            commands::stage_files,
            commands::unstage_files,
            commands::discard_files,
            commands::create_commit,
            commands::list_branches,
            commands::create_branch,
            commands::checkout_branch,
            commands::git_fetch,
            commands::git_pull,
            commands::git_push,
            commands::cancel_operation,
            commands::get_recent_repos,
            commands::remove_recent_repo,
            commands::set_recent_pinned,
            commands::peek_repo,
            commands::delete_repo,
            // Stash
            commands::stash_list,
            commands::stash_push,
            commands::stash_apply,
            commands::stash_pop,
            commands::stash_drop,
            // Tags
            commands::list_tags,
            commands::create_tag,
            commands::delete_tag,
            // Branch management
            commands::rename_branch,
            commands::delete_branch,
            commands::merge_branch,
            commands::rebase_onto,
            // Multi-step operations
            commands::get_op_context,
            commands::abort_operation,
            commands::continue_operation,
            commands::resolve_conflict,
            commands::open_mergetool,
            commands::read_conflict,
            commands::save_resolution,
            // History operations
            commands::cherry_pick,
            commands::revert_commit,
            commands::undo_last_commit,
            commands::squash_from,
            commands::create_branch_from_commit,
            commands::checkout_commit,
            commands::search_log,
            commands::rebase_interactive,
            // Bisect
            commands::bisect_start,
            commands::bisect_mark,
            commands::bisect_reset,
            // Hunk/line staging
            commands::apply_hunk,
            commands::discard_hunk,
            commands::apply_lines,
            // Remotes & repo lifecycle
            commands::list_remotes,
            commands::push_remote,
            commands::add_remote,
            commands::remove_remote,
            commands::rename_remote,
            commands::set_remote_url,
            // Backups (backup refs)
            commands::list_backups,
            commands::restore_backup,
            commands::delete_backup,
            // Local pipeline testing
            commands::pipeline_detect,
            commands::pipeline_cancel,
            commands::pipeline_configs,
            commands::pipeline_add_config,
            commands::pipeline_graph,
            commands::pipeline_run_scope,
            // Sparse checkout
            commands::sparse_status,
            commands::sparse_set,
            commands::sparse_disable,
            // Multi-level undo/redo
            commands::undo_status,
            commands::undo_last,
            commands::redo_last,
            // Provider accounts & change requests
            commands::provider_accounts,
            commands::provider_add_account,
            commands::provider_remove_account,
            commands::list_change_requests,
            commands::provider_default_branch,
            commands::create_change_request,
            commands::clone_prepare,
            commands::clone_fetch,
            commands::init_repository,
            commands::ignore_pattern,
            // Views
            commands::blame_file,
            commands::get_image_diff,
            // Worktrees & submodules
            commands::list_worktrees,
            commands::add_worktree,
            commands::remove_worktree,
            commands::list_submodules,
            commands::update_submodules,
            // Configuration & system
            commands::config_get,
            commands::config_set,
            commands::check_signing,
            commands::open_in_explorer,
            commands::open_in_editor,
            commands::open_terminal,
            commands::open_external,
            commands::new_window,
            commands::open_logs,
            // File watcher & streaming
            commands::watch_repository,
            commands::unwatch_repository,
            commands::get_commit_diff_stream,
            // SSH key manager
            commands::ssh_list_keys,
            commands::ssh_generate_key,
            commands::ssh_scan_host,
            commands::ssh_trust_host,
            commands::ssh_remove_key,
        ])
        .run(tauri::generate_context!())
        .expect("terra-git failed to start");
}
