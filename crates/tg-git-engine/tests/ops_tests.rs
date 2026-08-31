//! Integration tests of the extended operations (GitEngineExt).

use std::fs;
use std::path::{Path, PathBuf};

use tg_domain::{ChangeKind, RebaseStep, RepoOpState, ResetMode, UndoAction};
use tg_git_engine::prelude::*;

fn init_repo() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = git2::Repository::init(dir.path()).expect("init");
    let mut config = repo.config().expect("config");
    config.set_str("user.name", "Terra Tester").unwrap();
    config.set_str("user.email", "terra@test.local").unwrap();
    // Hermetic against host configuration: a global commit.gpgsign=true (without
    // a key) would otherwise make all sidecar commits of the fixtures fail.
    config.set_str("commit.gpgsign", "false").unwrap();
    // Same for line endings: GitHub's Windows runners set core.autocrlf=true
    // globally, which would rewrite the LF fixtures to CRLF and break every
    // content assertion. The two tests that examine the conversion itself set
    // their own value after this call.
    config.set_bool("core.autocrlf", false).unwrap();
    // A global attributes file marking files as text would convert despite
    // autocrlf=false; core.eol decides the direction, so pin it as well.
    config.set_str("core.eol", "lf").unwrap();
    let path = dir.path().to_path_buf();
    (dir, path)
}

/// A `file://` URL for a local path, on every platform. On Windows the path
/// starts with a drive letter and needs the third slash (`file:///C:/tmp/x`);
/// on Linux and macOS it already starts with one, and blindly adding a third
/// would produce `file:////tmp/x`.
fn file_url(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    if s.starts_with('/') {
        format!("file://{s}")
    } else {
        format!("file:///{s}")
    }
}

/// Pins the line endings of a repository that was NOT created by `init_repo` —
/// a clone destination, for instance, which the engine creates itself. Without
/// it the repo inherits the host's `core.autocrlf`, and on the Windows runners
/// (where it is globally true) the checkout turns the LF fixtures into CRLF.
fn pin_eol(repo_path: &Path) {
    let repo = git2::Repository::open(repo_path).expect("open");
    let mut config = repo.config().expect("config");
    config.set_bool("core.autocrlf", false).unwrap();
    // A global attributes file marking files as text would convert despite
    // autocrlf=false; core.eol decides the direction, so pin it as well.
    config.set_str("core.eol", "lf").unwrap();
}

fn write(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(p, content).unwrap();
}

fn commit_all(path: &Path, msg: &str) -> String {
    let engine = Git2Engine;
    let status = engine.status(path).unwrap();
    let files: Vec<String> = status.unstaged.iter().map(|e| e.path.clone()).collect();
    if !files.is_empty() {
        engine.stage(path, &files).unwrap();
    }
    engine.commit(path, msg, false).unwrap()
}

/// Runs a raw git command (for test setup beyond the engine API).
fn git(dir: &Path, args: &[&str]) {
    let ok = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .status()
        .expect("git start")
        .success();
    assert!(ok, "git {args:?} failed");
}

#[test]
fn unpushed_commits_without_and_against_remote() {
    // Bare remote + working repo with 2 pushed + 2 local commits.
    let remote_dir = tempfile::tempdir().unwrap();
    git(remote_dir.path(), &["init", "--bare", "-q"]);
    let url = remote_dir.path().to_string_lossy().replace('\\', "/");

    let (_g, work) = init_repo();
    let engine = Git2Engine;
    write(&work, "a.txt", "A\n");
    commit_all(&work, "A");
    write(&work, "b.txt", "B\n");
    commit_all(&work, "B");
    // Without a remote: both are unpushed (newest first).
    let all = engine.unpushed_commits(&work).unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].subject, "B");
    assert!(all[0].is_head);
    assert_eq!(all[1].subject, "A");
    assert!(!all[0].is_merge);
    // Author timestamp set (Unix seconds, not 0).
    assert!(all[0].time > 0);

    // After the push, A and B are on origin -> only the NEW ones are unpushed.
    git(&work, &["remote", "add", "origin", &url]);
    git(&work, &["push", "-u", "-q", "origin", "HEAD"]);
    write(&work, "c.txt", "C\n");
    commit_all(&work, "C");
    let ahead = engine.unpushed_commits(&work).unwrap();
    assert_eq!(ahead.len(), 1);
    assert_eq!(ahead[0].subject, "C");
    assert!(ahead[0].is_head);
    assert!(!ahead[0].parent_ids.is_empty());
}

#[test]
fn status_ahead_without_upstream_counts_unpushed_commits() {
    // Regression: without an upstream the toolbar showed no push count, because
    // ahead was only computed against an upstream. Without an upstream, ahead has
    // to mean commits that are on no remote (HEAD --not --remotes).
    let remote_dir = tempfile::tempdir().unwrap();
    git(remote_dir.path(), &["init", "--bare", "-q"]);
    let url = remote_dir.path().to_string_lossy().replace('\\', "/");

    let (_g, work) = init_repo();
    let engine = Git2Engine;
    write(&work, "a.txt", "A\n");
    commit_all(&work, "A");
    git(&work, &["remote", "add", "origin", &url]);
    // Push WITHOUT -u: origin/main exists as a remote-tracking ref, but NO
    // upstream is set for the local branch.
    git(&work, &["push", "-q", "origin", "HEAD:refs/heads/main"]);
    git(&work, &["fetch", "-q", "origin"]);
    write(&work, "b.txt", "B\n");
    commit_all(&work, "B");
    write(&work, "c.txt", "C\n");
    commit_all(&work, "C");

    let st = engine.status(&work).unwrap();
    assert!(st.upstream.is_none(), "no upstream set");
    assert_eq!(
        st.ahead, 2,
        "B and C are unpushed (despite the missing upstream)"
    );
    assert_eq!(st.behind, 0);
}

#[test]
fn stash_push_list_apply_drop() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    write(&path, "a.txt", "base\n");
    commit_all(&path, "Base");

    write(&path, "a.txt", "changed\n");
    write(&path, "new.txt", "untracked\n");
    engine.stash_push(&path, "WIP Test", &[]).unwrap();

    // The workdir is clean again
    assert!(engine.status(&path).unwrap().unstaged.is_empty());
    let stashes = engine.stash_list(&path).unwrap();
    assert_eq!(stashes.len(), 1);
    assert!(stashes[0].message.contains("WIP Test"));

    engine.stash_pop(&path, 0).unwrap();
    assert_eq!(fs::read_to_string(path.join("a.txt")).unwrap(), "changed\n");
    assert!(path.join("new.txt").exists());
    assert!(engine.stash_list(&path).unwrap().is_empty());
}

#[test]
fn stash_partial_only_selected_file() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    write(&path, "a.txt", "a\n");
    write(&path, "b.txt", "b\n");
    commit_all(&path, "Base");

    write(&path, "a.txt", "a changed\n");
    write(&path, "b.txt", "b changed\n");
    engine
        .stash_push(&path, "only a", &["a.txt".into()])
        .unwrap();

    let status = engine.status(&path).unwrap();
    let unstaged: Vec<&str> = status.unstaged.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(unstaged, vec!["b.txt"], "b.txt has to stay untouched");
    assert_eq!(fs::read_to_string(path.join("a.txt")).unwrap(), "a\n");
}

#[test]
fn tags_create_list_delete() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    write(&path, "a.txt", "x\n");
    let head = commit_all(&path, "Base");

    engine
        .create_tag(&path, "v1.0.0", "Release one", "")
        .unwrap();
    engine.create_tag(&path, "light", "", &head).unwrap();

    let tags = engine.tags(&path).unwrap();
    assert_eq!(tags.len(), 2);
    let annotated = tags.iter().find(|t| t.name == "v1.0.0").unwrap();
    assert!(annotated.is_annotated);
    assert_eq!(annotated.message.as_deref(), Some("Release one"));
    assert_eq!(annotated.target_id, head);
    let light = tags.iter().find(|t| t.name == "light").unwrap();
    assert!(!light.is_annotated);

    engine.delete_tag(&path, "light").unwrap();
    assert_eq!(engine.tags(&path).unwrap().len(), 1);
}

#[test]
fn branch_rename_and_delete_with_merged_check() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    write(&path, "a.txt", "x\n");
    commit_all(&path, "Base");

    engine.create_branch(&path, "feature", false).unwrap();
    engine
        .rename_branch(&path, "feature", "feature-new")
        .unwrap();
    let names: Vec<String> = engine
        .branches(&path)
        .unwrap()
        .into_iter()
        .map(|b| b.name)
        .collect();
    assert!(names.contains(&"feature-new".to_string()));
    assert!(!names.contains(&"feature".to_string()));

    // Merged (points at HEAD) -> deleting without force is fine
    engine.delete_branch(&path, "feature-new", false).unwrap();

    // An unmerged branch -> force required
    engine.create_branch(&path, "wild", true).unwrap();
    write(&path, "wild.txt", "w\n");
    commit_all(&path, "Wild commit");
    let branches = engine.branches(&path).unwrap();
    let main_name = branches
        .iter()
        .find(|b| !b.is_remote && b.name != "wild")
        .unwrap()
        .name
        .clone();
    engine.checkout_branch(&path, &main_name).unwrap();

    let err = engine.delete_branch(&path, "wild", false).unwrap_err();
    assert_eq!(err.code(), "branch_not_merged");
    engine.delete_branch(&path, "wild", true).unwrap();
}

#[test]
fn merge_branch_fast_forward_and_conflict_abort() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    write(&path, "a.txt", "base\n");
    commit_all(&path, "Base");
    let branches = engine.branches(&path).unwrap();
    let main_name = branches.iter().find(|b| !b.is_remote).unwrap().name.clone();

    // Fast-forward merge
    engine.create_branch(&path, "ff", true).unwrap();
    write(&path, "ff.txt", "ff\n");
    commit_all(&path, "FF commit");
    engine.checkout_branch(&path, &main_name).unwrap();
    engine.merge_branch(&path, "ff").unwrap();
    assert!(path.join("ff.txt").exists());

    // Conflict merge -> op_state Merge -> abort
    engine.create_branch(&path, "conflict", true).unwrap();
    write(&path, "a.txt", "from conflict\n");
    commit_all(&path, "Conflict side");
    engine.checkout_branch(&path, &main_name).unwrap();
    write(&path, "a.txt", "from main\n");
    commit_all(&path, "Main side");

    assert!(engine.merge_branch(&path, "conflict").is_err());
    assert_eq!(engine.op_state(&path).unwrap(), RepoOpState::Merge);
    let status = engine.status(&path).unwrap();
    assert_eq!(status.op_state, RepoOpState::Merge);
    assert!(status
        .unstaged
        .iter()
        .any(|e| e.kind == ChangeKind::Conflicted));

    engine.abort_operation(&path).unwrap();
    assert_eq!(engine.op_state(&path).unwrap(), RepoOpState::Clean);
    assert_eq!(
        fs::read_to_string(path.join("a.txt")).unwrap(),
        "from main\n"
    );
}

#[test]
fn op_context_names_merge_sides() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    write(&path, "a.txt", "base\n");
    commit_all(&path, "Base");
    let main_name = engine.branches(&path).unwrap()[0].name.clone();

    engine.create_branch(&path, "side", true).unwrap();
    write(&path, "a.txt", "from side\n");
    commit_all(&path, "Side commit");
    engine.checkout_branch(&path, &main_name).unwrap();
    write(&path, "a.txt", "from main\n");
    commit_all(&path, "Main commit");

    assert!(engine.merge_branch(&path, "side").is_err());
    let ctx = engine.op_context(&path).unwrap();
    assert_eq!(ctx.kind, RepoOpState::Merge);
    assert_eq!(ctx.ours_label.as_deref(), Some(main_name.as_str()));
    assert_eq!(ctx.theirs_label.as_deref(), Some("side"));
    assert_eq!(ctx.theirs_summary.as_deref(), Some("Side commit"));

    engine.abort_operation(&path).unwrap();
    let clean = engine.op_context(&path).unwrap();
    assert_eq!(clean.kind, RepoOpState::Clean);
    assert!(clean.theirs_label.is_none(), "Clean: no theirs side");
}

#[test]
fn resolve_conflict_with_ours_theirs() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    write(&path, "a.txt", "base\n");
    commit_all(&path, "Base");
    let main_name = engine.branches(&path).unwrap()[0].name.clone();

    engine.create_branch(&path, "side", true).unwrap();
    write(&path, "a.txt", "from side\n");
    commit_all(&path, "Side");
    engine.checkout_branch(&path, &main_name).unwrap();
    write(&path, "a.txt", "from main\n");
    commit_all(&path, "Main");

    assert!(engine.merge_branch(&path, "side").is_err());
    engine.resolve_conflict(&path, "a.txt", false).unwrap(); // theirs
    assert_eq!(
        fs::read_to_string(path.join("a.txt")).unwrap(),
        "from side\n"
    );
    let status = engine.status(&path).unwrap();
    assert!(!status
        .unstaged
        .iter()
        .any(|e| e.kind == ChangeKind::Conflicted));

    engine.continue_operation(&path).unwrap();
    assert_eq!(engine.op_state(&path).unwrap(), RepoOpState::Clean);
    let log = engine.log(&path, 0, 1).unwrap();
    assert_eq!(log[0].parent_ids.len(), 2, "merge commit expected");
}

#[test]
fn cherry_pick_and_revert() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    write(&path, "a.txt", "base\n");
    commit_all(&path, "Base");
    let main_name = engine.branches(&path).unwrap()[0].name.clone();

    engine.create_branch(&path, "source", true).unwrap();
    write(&path, "extra.txt", "extra\n");
    let pick_id = commit_all(&path, "Extra file");
    engine.checkout_branch(&path, &main_name).unwrap();

    engine.cherry_pick(&path, &pick_id).unwrap();
    assert!(path.join("extra.txt").exists());
    assert_eq!(engine.log(&path, 0, 1).unwrap()[0].summary, "Extra file");

    let head = engine.log(&path, 0, 1).unwrap()[0].id.clone();
    engine.revert_commit(&path, &head).unwrap();
    assert!(!path.join("extra.txt").exists());
}

#[test]
fn undo_and_squash() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    write(&path, "a.txt", "1\n");
    commit_all(&path, "One");
    write(&path, "a.txt", "2\n");
    commit_all(&path, "Two");

    // Undo: the changes stay staged
    engine.undo_last_commit(&path).unwrap();
    let status = engine.status(&path).unwrap();
    assert_eq!(engine.log(&path, 0, 10).unwrap().len(), 1);
    assert_eq!(status.staged.len(), 1);

    // Commit again, then once more — and squash
    engine.commit(&path, "Two new", false).unwrap();
    write(&path, "a.txt", "3\n");
    commit_all(&path, "Three");
    // Take the oldest commit to be squashed ("Two new") as the base anchor.
    let oldest = engine.log(&path, 0, 10).unwrap()[1].id.clone();
    engine
        .squash_from(&path, &oldest, "Two+Three together")
        .unwrap();
    let log = engine.log(&path, 0, 10).unwrap();
    assert_eq!(log.len(), 2);
    assert_eq!(log[0].summary, "Two+Three together");
    assert_eq!(fs::read_to_string(path.join("a.txt")).unwrap(), "3\n");
}

/// Regression tests for the staging bugs confirmed in the 2nd review.
#[test]
fn line_staging_preserves_crlf() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    // autocrlf=false: CRLF lands unchanged in the index (typical on Windows)
    git2::Repository::open(&path)
        .unwrap()
        .config()
        .unwrap()
        .set_bool("core.autocrlf", false)
        .unwrap();

    write(&path, "f.txt", "one\r\ntwo\r\nthree\r\n");
    commit_all(&path, "Base CRLF");
    write(&path, "f.txt", "one\r\nTWO\r\nthree\r\nfour\r\n");

    let diff = engine.file_diff(&path, "f.txt", false).unwrap().unwrap();
    let lines = &diff.hunks[0].lines;
    let sel: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| {
            matches!(
                l.kind,
                tg_domain::LineKind::Addition | tg_domain::LineKind::Deletion
            )
        })
        .map(|(i, _)| i)
        .collect();

    // Must NOT fail with "patch does not apply".
    engine.apply_lines(&path, "f.txt", 0, &sel, false).unwrap();

    // The staged index content has to have kept CRLF.
    let repo = git2::Repository::open(&path).unwrap();
    let idx = repo.index().unwrap();
    let entry = idx.get_path(Path::new("f.txt"), 0).unwrap();
    let blob = repo.find_blob(entry.id).unwrap();
    let content = String::from_utf8_lossy(blob.content());
    assert!(
        content.contains("\r\n"),
        "CRLF has to be preserved, was: {content:?}"
    );
    assert!(content.contains("TWO"));
}

#[test]
fn line_staging_without_trailing_newline() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    // Commit a file WITHOUT a trailing newline
    fs::write(path.join("f.txt"), "a\nb").unwrap();
    engine.stage(&path, &["f.txt".into()]).unwrap();
    engine.commit(&path, "Without newline", false).unwrap();
    // Change the last line (still without a trailing newline)
    fs::write(path.join("f.txt"), "a\nc").unwrap();

    let diff = engine.file_diff(&path, "f.txt", false).unwrap().unwrap();
    let lines = &diff.hunks[0].lines;

    // A partial selection (only the addition) on a no-newline file -> cleanly
    // rejected, NO silent corruption.
    let add = lines.iter().position(|l| l.content == "c").unwrap();
    let err = engine
        .apply_lines(&path, "f.txt", 0, &[add], false)
        .unwrap_err();
    assert_eq!(err.code(), "invalid_operation");

    // A full selection (both changed lines) -> the correct result "a\nc".
    let all: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| !matches!(l.kind, tg_domain::LineKind::Context))
        .map(|(i, _)| i)
        .collect();
    engine.apply_lines(&path, "f.txt", 0, &all, false).unwrap();

    let repo = git2::Repository::open(&path).unwrap();
    let idx = repo.index().unwrap();
    let entry = idx.get_path(Path::new("f.txt"), 0).unwrap();
    let blob = repo.find_blob(entry.id).unwrap();
    let content = String::from_utf8_lossy(blob.content()).into_owned();
    assert_eq!(
        content, "a\nc",
        "a no-newline file has to be staged correctly"
    );
}

#[test]
fn hunk_staging_on_untracked_file() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    write(&path, "base.txt", "x\n");
    commit_all(&path, "Base");

    // A new, untracked file with two blocks
    let content: String = (1..=40).map(|i| format!("new {i}\n")).collect();
    write(&path, "new.txt", &content);

    // Hunk staging has to work (git add -N internally)
    engine.apply_hunk(&path, "new.txt", 0, false).unwrap();
    let staged = engine.file_diff(&path, "new.txt", true).unwrap().unwrap();
    assert!(
        !staged.hunks.is_empty(),
        "part of the new file has to be staged"
    );
}

#[test]
fn squash_from_is_transactional_on_empty_message() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    write(&path, "a.txt", "1\n");
    commit_all(&path, "One");
    write(&path, "a.txt", "2\n");
    commit_all(&path, "Two");
    let before = engine.log(&path, 0, 10).unwrap();
    let oldest = before[0].id.clone();

    // An empty message -> an error, but the history has to stay UNCHANGED
    let err = engine.squash_from(&path, &oldest, "   ").unwrap_err();
    assert_eq!(err.code(), "empty_commit_message");
    let after = engine.log(&path, 0, 10).unwrap();
    assert_eq!(after.len(), before.len(), "the history must not be touched");
    assert_eq!(after[0].id, before[0].id);
}

#[test]
fn branch_from_commit_and_detached_checkout() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    write(&path, "a.txt", "1\n");
    let first = commit_all(&path, "One");
    write(&path, "a.txt", "2\n");
    commit_all(&path, "Two");

    engine
        .create_branch_from_commit(&path, "from-one", &first, true)
        .unwrap();
    assert_eq!(fs::read_to_string(path.join("a.txt")).unwrap(), "1\n");
    let info = engine.open_repo(&path).unwrap();
    assert_eq!(info.current_branch.as_deref(), Some("from-one"));

    engine.checkout_commit(&path, &first).unwrap();
    let info = engine.open_repo(&path).unwrap();
    assert!(info.head_detached);
    assert_eq!(info.current_branch, None);
}

#[test]
fn history_search_finds_message_author_and_id() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    write(&path, "a.txt", "1\n");
    commit_all(&path, "Fix: NullPointer in login");
    write(&path, "a.txt", "2\n");
    let id = commit_all(&path, "Feature: search function");

    assert_eq!(
        engine.search_log(&path, "nullpointer", 10).unwrap().len(),
        1
    );
    assert_eq!(
        engine.search_log(&path, "terra tester", 10).unwrap().len(),
        2
    );
    assert_eq!(engine.search_log(&path, &id[..10], 10).unwrap().len(), 1);
    assert!(engine
        .search_log(&path, "does-not-exist", 10)
        .unwrap()
        .is_empty());
}

#[test]
fn hunk_stage_unstage_and_discard() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    // Two blocks far apart -> two hunks
    let base_content: String = (1..=40).map(|i| format!("line {i}\n")).collect();
    write(&path, "a.txt", &base_content);
    commit_all(&path, "Base");

    let mut changed: Vec<String> = base_content.lines().map(String::from).collect();
    changed[2] = "LINE 3 NEW".into();
    changed[35] = "LINE 36 NEW".into();
    write(&path, "a.txt", &(changed.join("\n") + "\n"));

    let diff = engine.file_diff(&path, "a.txt", false).unwrap().unwrap();
    assert_eq!(diff.hunks.len(), 2);

    // Stage only hunk 0
    engine.apply_hunk(&path, "a.txt", 0, false).unwrap();
    let staged = engine.file_diff(&path, "a.txt", true).unwrap().unwrap();
    assert_eq!(staged.hunks.len(), 1);
    assert!(staged.hunks[0]
        .lines
        .iter()
        .any(|l| l.content.contains("LINE 3")));
    let unstaged = engine.file_diff(&path, "a.txt", false).unwrap().unwrap();
    assert_eq!(unstaged.hunks.len(), 1);
    assert!(unstaged.hunks[0]
        .lines
        .iter()
        .any(|l| l.content.contains("LINE 36")));

    // Unstage the hunk again
    engine.apply_hunk(&path, "a.txt", 0, true).unwrap();
    let staged = engine.file_diff(&path, "a.txt", true).unwrap();
    assert!(staged.is_none() || staged.unwrap().hunks.is_empty());

    // Discard hunk 1: line 36 back to the original
    let diff = engine.file_diff(&path, "a.txt", false).unwrap().unwrap();
    assert_eq!(diff.hunks.len(), 2);
    engine.discard_hunk(&path, "a.txt", 1).unwrap();
    let content = fs::read_to_string(path.join("a.txt")).unwrap();
    assert!(content.contains("line 36"));
    assert!(content.contains("LINE 3 NEW"));
}

#[test]
fn stage_line_by_line() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    write(&path, "a.txt", "one\ntwo\nthree\n");
    commit_all(&path, "Base");
    write(&path, "a.txt", "one\nTWO\nTHREE\n");

    let diff = engine.file_diff(&path, "a.txt", false).unwrap().unwrap();
    let lines = &diff.hunks[0].lines;
    // Find the index of the addition "TWO" in the hunk body
    let two_idx = lines
        .iter()
        .position(|l| l.content == "TWO")
        .expect("TWO in the diff");
    // The corresponding deletion "two"
    let two_del = lines
        .iter()
        .position(|l| l.content == "two")
        .expect("two in the diff");

    engine
        .apply_lines(&path, "a.txt", 0, &[two_del, two_idx], false)
        .unwrap();

    let staged = engine.file_diff(&path, "a.txt", true).unwrap().unwrap();
    let staged_adds: Vec<&str> = staged.hunks[0]
        .lines
        .iter()
        .filter(|l| matches!(l.kind, tg_domain::LineKind::Addition))
        .map(|l| l.content.as_str())
        .collect();
    assert_eq!(staged_adds, vec!["TWO"], "only TWO may be staged");
}

#[test]
fn init_ignore_remotes_and_config() {
    let dir = tempfile::tempdir().unwrap();
    let engine = Git2Engine;
    let repo_dir = dir.path().join("new");
    let info = engine.init_repo(&repo_dir).unwrap();
    assert!(info.is_empty);

    // Set & read the config locally
    engine
        .config_set(&repo_dir, "user.name", "Ini Test", false)
        .unwrap();
    assert_eq!(
        engine
            .config_get(&repo_dir, "user.name")
            .unwrap()
            .as_deref(),
        Some("Ini Test")
    );

    // Append ignore patterns
    engine.ignore_pattern(&repo_dir, "*.log").unwrap();
    engine.ignore_pattern(&repo_dir, "build/").unwrap();
    let gitignore = fs::read_to_string(repo_dir.join(".gitignore")).unwrap();
    assert!(gitignore.contains("*.log\n"));
    assert!(gitignore.contains("build/\n"));

    // A multi-line pattern is rejected: combined with `include.path` it would
    // turn .gitignore into executable git config, so one line per call only.
    for bad in ["a\n[core]\n\tsshCommand = calc", "a\rb"] {
        assert!(
            engine.ignore_pattern(&repo_dir, bad).is_err(),
            "multi-line ignore pattern must be rejected: {bad:?}"
        );
    }
    let after = fs::read_to_string(repo_dir.join(".gitignore")).unwrap();
    assert_eq!(
        after, gitignore,
        "a rejected pattern must not touch the file"
    );

    // No remotes in a fresh repo
    assert!(engine.remotes(&repo_dir).unwrap().is_empty());
}

/// Regression test for "the email is not being saved": empty values have to
/// REMOVE the config entry. An empty local entry (`email =`) otherwise masks any
/// global value invisibly — which is exactly what happened to the user.
#[test]
fn config_set_empty_value_removes_entry() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;

    engine
        .config_set(&path, "user.email", "a@b.c", false)
        .unwrap();
    assert_eq!(
        engine.config_get(&path, "user.email").unwrap().as_deref(),
        Some("a@b.c")
    );

    engine.config_set(&path, "user.email", "", false).unwrap();
    // Check raw at the LOCAL level — config_get reads the merged value and could
    // return the test machine's global one.
    let repo = git2::Repository::open(&path).unwrap();
    let local = repo
        .config()
        .unwrap()
        .open_level(git2::ConfigLevel::Local)
        .unwrap()
        .snapshot()
        .unwrap();
    assert!(
        local.get_string("user.email").is_err(),
        "an empty value has to remove the local entry"
    );
    // Emptying it again is idempotent (NotFound tolerated).
    engine.config_set(&path, "user.email", "", false).unwrap();
}

/// Signing preflight: without a commit a clear error message; with a commit the
/// result depends on the machine configuration (signing set up or not) — both
/// are valid, only a crash/hang is not.
#[test]
fn check_signing_without_head_reports_error() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;

    let err = engine.check_signing(&path).unwrap_err();
    assert!(
        err.to_string().contains("at least one commit"),
        "unexpected message: {err}"
    );

    write(&path, "a.txt", "x\n");
    commit_all(&path, "One");
    let _ = engine.check_signing(&path); // Ok or a classified error — no panic
}

#[test]
fn blame_returns_author_per_line() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    write(&path, "a.txt", "first\n");
    let c1 = commit_all(&path, "One");
    write(&path, "a.txt", "first\nsecond\n");
    let c2 = commit_all(&path, "Two");

    let blame = engine.blame_file(&path, "a.txt").unwrap();
    assert_eq!(blame.len(), 2);
    assert_eq!(blame[0].commit_id, c1);
    assert_eq!(blame[1].commit_id, c2);
    assert_eq!(blame[0].author, "Terra Tester");
    assert_eq!(blame[1].content, "second");
}

/// Regression test for the ACCESS_VIOLATION crash in blame.
///
/// The cause was libgit2's `git_blame_get_hunk_byline`, which on real blames
/// with SEVERAL hunks (several contributing commits) ran into a null-hunk region
/// of the table libgit2 itself reported and segfaulted. The engine therefore
/// blames through the system-git sidecar (`git blame --porcelain HEAD`).
/// The test builds a multi-hunk history and additionally lets the worktree
/// deviate from the LF blob via CRLF (the known project trap) — blame still has
/// to deliver the complete HEAD state correctly and without a crash.
#[test]
fn blame_with_several_hunks_does_not_crash() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;

    // Multi-hunk history: every change touches a different line range so the
    // blame result has several hunks from different commits.
    write(&path, "a.txt", "one\ntwo\nthree\n");
    let c1 = commit_all(&path, "c1");
    write(&path, "a.txt", "one\nTWO-new\nthree\n");
    let c2 = commit_all(&path, "c2");
    write(&path, "a.txt", "one\nTWO-new\nthree\nfour\nfive\n");
    let c3 = commit_all(&path, "c3");

    // The worktree deviates from the (LF) blob via CRLF: an earlier version
    // wrongly blamed everything as "Not Committed Yet". Blame has to show HEAD.
    fs::write(
        path.join("a.txt"),
        "one\r\nTWO-new\r\nthree\r\nfour\r\nfive\r\n",
    )
    .unwrap();

    let blame = engine.blame_file(&path, "a.txt").unwrap();
    assert_eq!(blame.len(), 5, "all 5 HEAD lines expected");
    assert_eq!(blame[0].commit_id, c1);
    assert_eq!(blame[1].commit_id, c2);
    assert_eq!(blame[2].commit_id, c1);
    assert_eq!(blame[3].commit_id, c3);
    assert_eq!(blame[4].commit_id, c3);
    assert_eq!(blame[1].content, "TWO-new");
    // The sidecar porcelain parser has to fill in author and time.
    assert_eq!(blame[0].author, "Terra Tester");
    assert!(blame[0].time > 0, "the author time has to be set");
    assert_eq!(blame[0].short_id, blame[0].commit_id[..8]);
}

#[test]
fn image_diff_returns_data_urls() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    // A minimal "PNG" (the header is enough for the test)
    let png: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3];
    fs::write(path.join("image.png"), png).unwrap();
    engine.stage(&path, &["image.png".into()]).unwrap();
    engine.commit(&path, "Image", false).unwrap();

    fs::write(path.join("image.png"), [png, &[9, 9, 9]].concat()).unwrap();
    let diff = engine.image_diff(&path, "image.png", false).unwrap();
    let old = diff.old_data_url.expect("old version");
    let new = diff.new_data_url.expect("new version");
    assert!(old.starts_with("data:image/png;base64,"));
    assert!(new.starts_with("data:image/png;base64,"));
    assert_ne!(old, new);
}

#[test]
fn worktree_list_contains_main_worktree() {
    let (_g, path) = init_repo();
    write(&path, "a.txt", "x\n");
    commit_all(&path, "Base");
    let wts = Git2Engine.worktrees(&path).unwrap();
    assert_eq!(wts.len(), 1);
    assert!(wts[0].is_main);
}

#[test]
fn push_non_fast_forward_is_classified() {
    // Create a bare "remote".
    let remote_dir = tempfile::tempdir().unwrap();
    git(remote_dir.path(), &["init", "--bare", "-q"]);

    // Working repo: commit A, link the remote, first push.
    let (_g, work) = init_repo();
    write(&work, "a.txt", "A\n");
    commit_all(&work, "A");
    let remote_url = remote_dir.path().to_string_lossy().replace('\\', "/");
    git(&work, &["remote", "add", "origin", &remote_url]);
    Git2Engine.push_remote(&work, "origin", false).unwrap();

    // A second clone moves the remote forward (commit B).
    let other_dir = tempfile::tempdir().unwrap();
    let other = other_dir.path().join("clone");
    git(other_dir.path(), &["clone", "-q", &remote_url, "clone"]);
    git(&other, &["config", "user.name", "Other"]);
    git(&other, &["config", "user.email", "other@test.local"]);
    git(&other, &["config", "commit.gpgsign", "false"]);
    std::fs::write(other.join("b.txt"), "B\n").unwrap();
    git(&other, &["add", "-A"]);
    git(&other, &["commit", "-q", "-m", "B"]);
    git(&other, &["push", "-q", "origin", "HEAD"]);

    // The working repo commits divergently (C) and pushes -> non-fast-forward.
    write(&work, "c.txt", "C\n");
    commit_all(&work, "C");
    let err = Git2Engine.push_remote(&work, "origin", false).unwrap_err();
    assert_eq!(
        err.code(),
        "non_fast_forward",
        "expected non_fast_forward, got: {} / {err}",
        err.code()
    );

    // A force push with --force-with-lease MUST refuse here: the remote was moved
    // from outside (commit B), which `work` has never seen. That is exactly what
    // the lease protects.
    let err = Git2Engine.push_remote(&work, "origin", true).unwrap_err();
    assert_eq!(err.code(), "force_lease_stale", "got: {err}");

    // After a fetch (the lease now knows B) the force push goes through.
    Git2Engine.fetch(&work).unwrap();
    Git2Engine.push_remote(&work, "origin", true).unwrap();
}

fn step(action: &str, id: &str) -> RebaseStep {
    RebaseStep {
        action: action.into(),
        commit_id: id.into(),
        message: None,
        author: None,
    }
}

fn step_full(action: &str, id: &str, message: Option<&str>, author: Option<&str>) -> RebaseStep {
    RebaseStep {
        action: action.into(),
        commit_id: id.into(),
        message: message.map(str::to_string),
        author: author.map(str::to_string),
    }
}

#[test]
fn rebase_changes_author_without_and_with_message() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    write(&path, "a.txt", "one\n");
    let base = commit_all(&path, "Base");
    write(&path, "a.txt", "two\n");
    let top = commit_all(&path, "Tpyo");

    // Only change the author (the message stays); a message reword on top would not exist here.
    engine
        .rebase_interactive(
            &path,
            &base,
            &[step_full("pick", &top, None, Some("New Author <new@x.de>"))],
        )
        .unwrap();
    let head = engine.log(&path, 0, 1).unwrap();
    assert_eq!(head[0].summary, "Tpyo", "message unchanged");
    assert_eq!(head[0].author_name, "New Author");
    assert_eq!(head[0].author_email, "new@x.de");

    // Message + author together.
    let top2 = head[0].id.clone();
    engine
        .rebase_interactive(
            &path,
            &base,
            &[step_full(
                "reword",
                &top2,
                Some("Typo"),
                Some("Two <two@x.de>"),
            )],
        )
        .unwrap();
    let head = engine.log(&path, 0, 1).unwrap();
    assert_eq!(head[0].summary, "Typo");
    assert_eq!(head[0].author_email, "two@x.de");
}

#[test]
fn rebase_rejects_unsafe_author() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    write(&path, "a.txt", "one\n");
    let base = commit_all(&path, "Base");
    write(&path, "a.txt", "two\n");
    let top = commit_all(&path, "Two");
    // An embedded single quote (an injection attempt) is rejected.
    assert!(engine
        .rebase_interactive(
            &path,
            &base,
            &[step_full("pick", &top, None, Some("x' && calc; '<a@b>"))]
        )
        .is_err());
    // A leading '-' is rejected.
    assert!(engine
        .rebase_interactive(
            &path,
            &base,
            &[step_full("pick", &top, None, Some("-x <a@b>"))]
        )
        .is_err());
    // A line break (todo-line injection) is rejected.
    assert!(engine
        .rebase_interactive(
            &path,
            &base,
            &[step_full(
                "pick",
                &top,
                None,
                Some("New\nexec touch PWNED\n#x <a@b>")
            )]
        )
        .is_err());
}

/// Safety net: before a history rewrite the old HEAD is anchored
/// under refs/terra-git/backup/ and therefore stays recoverable.
#[test]
fn history_rewrite_creates_backup_ref() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    write(&path, "a.txt", "1\n");
    write(&path, "b.txt", "x\n");
    commit_all(&path, "One");
    write(&path, "a.txt", "1\n2\n");
    let c2 = commit_all(&path, "Two");

    engine.squash_from(&path, &c2, "Squashed").unwrap();

    let repo = git2::Repository::open(&path).unwrap();
    let backups: Vec<_> = repo
        .references_glob("refs/terra-git/backup/*")
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();
    assert_eq!(backups.len(), 1, "exactly one backup expected");
    assert_eq!(
        backups[0].target().unwrap().to_string(),
        c2,
        "the backup has to point at the old HEAD"
    );
}

/// Reword in the interactive rebase: a new message without an editor (via exec+amend).
#[test]
fn rebase_interactive_reword_changes_message() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    write(&path, "a.txt", "1\n");
    let base = commit_all(&path, "Base");
    write(&path, "a.txt", "1\n2\n");
    let c2 = commit_all(&path, "old message");

    let steps = vec![RebaseStep {
        action: "reword".into(),
        commit_id: c2,
        message: Some("new message".into()),
        author: None,
    }];
    engine.rebase_interactive(&path, &base, &steps).unwrap();

    let log = engine.log(&path, 0, 10).unwrap();
    assert_eq!(log[0].summary, "new message");
    assert_eq!(log[1].summary, "Base");
    // A reword without a message is rejected.
    let head = log[0].id.clone();
    let err = engine
        .rebase_interactive(
            &path,
            &log[1].id,
            &[RebaseStep {
                action: "reword".into(),
                commit_id: head,
                message: None,
                author: None,
            }],
        )
        .unwrap_err();
    assert!(err.to_string().contains("message"), "unexpected: {err}");
}

/// Looks for a reword message file we created that contains `marker` — in the
/// shared temp_dir and in the repo's .git directory.
///
/// A content marker instead of counting files, because cargo runs the tests
/// multi-threaded in the SAME process: parallel rebase tests write
/// `terra-git-reword-<same PID>-…` files, so a name or count assertion would be flaky.
fn reword_leftovers(repo: &Path, marker: &str) -> Option<PathBuf> {
    fn contains_marker(p: &Path, marker: &str) -> bool {
        fs::read_to_string(p)
            .map(|c| c.contains(marker))
            .unwrap_or(false)
    }
    fn collect_files(dir: &Path, prefix: &str, recursive: bool, out: &mut Vec<PathBuf>) {
        let Ok(rd) = fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                if recursive {
                    collect_files(&p, prefix, true, out);
                }
            } else if p
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(prefix))
            {
                out.push(p);
            }
        }
    }
    let mut candidates = Vec::new();
    collect_files(
        &std::env::temp_dir(),
        "terra-git-reword",
        false,
        &mut candidates,
    );
    // The prefix filter prevents a false hit through .git/COMMIT_EDITMSG, which
    // legitimately contains the message during a running rebase.
    collect_files(&repo.join(".git"), "terra-", true, &mut candidates);
    candidates.into_iter().find(|p| contains_marker(p, marker))
}

/// A reword rebase paused on a conflict must not leave a message file behind
/// after the abort: the cleanup used to run only on `result.is_ok()`,
/// and neither `--abort` nor `--continue` knew about the files.
#[test]
fn rebase_reword_leaves_no_message_file_on_conflict() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    write(&path, "a.txt", "x\n");
    let base = commit_all(&path, "Base");
    write(&path, "a.txt", "A\n");
    let c1 = commit_all(&path, "C1");
    write(&path, "a.txt", "B\n");
    let c2 = commit_all(&path, "C2");

    let marker = format!(
        "REWORD-LEAK-{}-{:?}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
    );

    // Reorder: C2 first -> `pick C2` onto the base. The merge base is C1, both
    // sides change the same line -> a reliable conflict.
    let steps = vec![
        RebaseStep {
            action: "reword".into(),
            commit_id: c2,
            message: Some(marker.clone()),
            author: None,
        },
        RebaseStep {
            action: "pick".into(),
            commit_id: c1,
            message: None,
            author: None,
        },
    ];
    assert!(
        engine.rebase_interactive(&path, &base, &steps).is_err(),
        "conflict expected"
    );
    assert_eq!(engine.op_state(&path).unwrap(), RepoOpState::Rebase);

    engine.abort_operation(&path).unwrap();
    assert_eq!(engine.op_state(&path).unwrap(), RepoOpState::Clean);

    assert!(
        reword_leftovers(&path, &marker).is_none(),
        "reword message file not cleaned up after the abort: {:?}",
        reword_leftovers(&path, &marker)
    );
}

fn summaries(path: &Path) -> Vec<String> {
    Git2Engine
        .log(path, 0, 50)
        .unwrap()
        .into_iter()
        .map(|c| c.summary)
        .collect()
}

/// A repo-local `.git/config` is executable code as far as git is concerned:
/// `git mergetool` starts the command configured through `mergetool.<tool>.cmd`
/// via sh. For a foreign repo (an unpacked archive, a network drive) terra-git
/// must NOT execute that.
#[test]
fn open_mergetool_does_not_run_repo_local_command() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    write(&path, "a.txt", "one\ntwo\nthree\n");
    commit_all(&path, "Base");
    let main = engine.status(&path).unwrap().branch.unwrap();

    engine.create_branch(&path, "side", true).unwrap();
    write(&path, "a.txt", "one\nSIDE\nthree\n");
    commit_all(&path, "Side");
    engine.checkout_branch(&path, &main).unwrap();
    write(&path, "a.txt", "one\nMAIN\nthree\n");
    commit_all(&path, "Main");
    assert!(
        engine.merge_branch(&path, "side").is_err(),
        "conflict expected"
    );

    // Set the attacker config directly through git: config_set would reject
    // `mergetool.*.cmd` through the denylist (is_forbidden_config_key).
    let marker = path.join("pwned.txt");
    let m = marker.to_string_lossy().replace('\\', "/");
    git(&path, &["config", "--local", "merge.tool", "evil"]);
    git(
        &path,
        &[
            "config",
            "--local",
            "mergetool.evil.cmd",
            &format!("printf PWNED > \"{m}\""),
        ],
    );

    let res = engine.open_mergetool(&path, "a.txt");

    assert!(
        !marker.exists(),
        "the repo-local mergetool.evil.cmd was executed"
    );
    let err = res.unwrap_err().to_string();
    assert!(
        err.contains("repo-local"),
        "an explanatory error message expected, was: {err}"
    );
}

#[test]
fn conflict_editor_roundtrip() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    write(&path, "a.txt", "one\ntwo\nthree\n");
    commit_all(&path, "Base");
    let main = engine.status(&path).unwrap().branch.unwrap();

    engine.create_branch(&path, "side", true).unwrap();
    write(&path, "a.txt", "one\nSIDE\nthree\n");
    commit_all(&path, "Side");
    engine.checkout_branch(&path, &main).unwrap();
    write(&path, "a.txt", "one\nMAIN\nthree\n");
    commit_all(&path, "Main");

    assert!(
        engine.merge_branch(&path, "side").is_err(),
        "conflict expected"
    );

    // Read: exactly one conflict with our (MAIN) and their (SIDE) line.
    let cf = engine.read_conflict(&path, "a.txt").unwrap();
    assert!(cf.has_conflicts);
    let block = cf.segments.iter().find(|s| s.kind == "conflict").unwrap();
    assert_eq!(block.ours, vec!["MAIN"]);
    assert_eq!(block.theirs, vec!["SIDE"]);

    // Resolve: keep both (ours first), save + stage.
    let sep = if cf.eol == "crlf" { "\r\n" } else { "\n" };
    let resolved = format!("one{sep}MAIN{sep}SIDE{sep}three{sep}");
    engine.save_resolution(&path, "a.txt", &resolved).unwrap();

    let on_disk = fs::read_to_string(path.join("a.txt")).unwrap();
    assert!(!on_disk.contains("<<<<<<<"), "no markers left");
    assert!(on_disk.contains("MAIN") && on_disk.contains("SIDE"));
    let st = engine.status(&path).unwrap();
    assert!(!st.unstaged.iter().any(|e| e.kind == ChangeKind::Conflicted));

    engine.continue_operation(&path).unwrap();
    assert_eq!(engine.op_state(&path).unwrap(), RepoOpState::Clean);
}

#[test]
fn read_conflict_rejects_non_utf8() {
    // from_utf8_lossy would silently turn non-UTF-8 bytes into U+FFFD and
    // save_resolution would write the replacement characters back permanently —
    // even into untouched context lines. Hence: a clear rejection instead of lossy.
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    // ISO-8859-1: 0xE9 = “é” — as a single byte not valid UTF-8.
    fs::write(
        path.join("latin1.txt"),
        b"caf\xE9\n<<<<<<< HEAD\nmine\n=======\ntheirs\n>>>>>>> b\n",
    )
    .unwrap();
    let err = engine.read_conflict(&path, "latin1.txt").unwrap_err();
    assert!(err.to_string().contains("UTF-8"), "unexpected: {err}");
}

#[test]
fn rebase_interactive_reorder_squash_drop() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    write(&path, "a.txt", "a\n");
    let base = commit_all(&path, "Base");
    write(&path, "b.txt", "b\n");
    let c1 = commit_all(&path, "C1");
    write(&path, "c.txt", "c\n");
    let c2 = commit_all(&path, "C2");
    write(&path, "d.txt", "d\n");
    let c3 = commit_all(&path, "C3");

    // Range base..HEAD = [C1, C2, C3]. Plan: C3 first (reorder), C1 as pick,
    // C2 fixup into C1, C3 stays — and the base commit as base_id.
    // New order (oldest first): pick C3, pick C1, fixup C2.
    let steps = [step("pick", &c3), step("pick", &c1), step("fixup", &c2)];
    let out = engine.rebase_interactive(&path, &base, &steps);
    assert!(out.is_ok(), "rebase failed: {out:?}");
    assert_eq!(engine.op_state(&path).unwrap(), RepoOpState::Clean);

    // Expected history (new->old): C1(+C2 fixed up), C3, base.
    let s = summaries(&path);
    assert_eq!(s, vec!["C1", "C3", "Base"], "unexpected history: {s:?}");
    // All files of the fixed-up/reordered commits are present.
    for f in ["a.txt", "b.txt", "c.txt", "d.txt"] {
        assert!(path.join(f).exists(), "{f} missing after the rebase");
    }
}

#[test]
fn rebase_interactive_rejects_gaps() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    write(&path, "a.txt", "a\n");
    let base = commit_all(&path, "Base");
    write(&path, "b.txt", "b\n");
    let c1 = commit_all(&path, "C1");
    write(&path, "c.txt", "c\n");
    let _c2 = commit_all(&path, "C2");

    // The plan leaves C2 out -> has to be rejected (data-loss protection).
    let err = engine
        .rebase_interactive(&path, &base, &[step("pick", &c1)])
        .unwrap_err();
    assert_eq!(err.code(), "invalid_operation");
    // The history is unchanged.
    assert_eq!(summaries(&path), vec!["C2", "C1", "Base"]);
}

/// E1: an author that is not a well-formed ident "Name <email>" is rejected
/// BEFORE the rebase starts — the repo must not be left hanging in the rebase
/// state afterwards.
#[test]
fn rebase_rejects_incomplete_author_and_stays_clean() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    write(&path, "a.txt", "one\n");
    let base = commit_all(&path, "Base");
    write(&path, "a.txt", "two\n");
    let top = commit_all(&path, "Two");

    // An empty name AND an empty email.
    let err = engine
        .rebase_interactive(&path, &base, &[step_full("pick", &top, None, Some(" <>"))])
        .unwrap_err();
    assert_eq!(err.code(), "invalid_operation");
    assert_eq!(
        engine.op_state(&path).unwrap(),
        RepoOpState::Clean,
        "no rebase may have started"
    );

    // No angle brackets at all.
    let err = engine
        .rebase_interactive(&path, &base, &[step_full("pick", &top, None, Some("Name"))])
        .unwrap_err();
    assert_eq!(err.code(), "invalid_operation");
    assert_eq!(engine.op_state(&path).unwrap(), RepoOpState::Clean);
    // The history is unchanged.
    assert_eq!(summaries(&path), vec!["Two", "Base"]);
}

/// E2: an author may only be set with pick/reword — squash with an author is
/// rejected (the value would otherwise be silently lost).
#[test]
fn rebase_rejects_author_on_squash() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    write(&path, "a.txt", "a\n");
    let base = commit_all(&path, "Base");
    write(&path, "b.txt", "b\n");
    let a = commit_all(&path, "A");
    write(&path, "c.txt", "c\n");
    let b = commit_all(&path, "B");

    let err = engine
        .rebase_interactive(
            &path,
            &base,
            &[
                step("pick", &a),
                step_full("squash", &b, None, Some("New Author <new@x.de>")),
            ],
        )
        .unwrap_err();
    assert_eq!(err.code(), "invalid_operation");
    assert_eq!(engine.op_state(&path).unwrap(), RepoOpState::Clean);
}

/// Builds a real `--no-ff` merge commit on HEAD (two diverging branches) and
/// returns the HEAD commit id.
fn build_merge_head(engine: &Git2Engine, path: &Path) -> String {
    write(path, "base.txt", "base\n");
    commit_all(path, "Base");
    let main = engine.status(path).unwrap().branch.unwrap();
    engine.create_branch(path, "side", true).unwrap();
    write(path, "s.txt", "s\n");
    commit_all(path, "Side");
    engine.checkout_branch(path, &main).unwrap();
    write(path, "m.txt", "m\n");
    commit_all(path, "Main");
    git(path, &["merge", "--no-ff", "--no-edit", "side"]);
    engine.log(path, 0, 1).unwrap()[0].id.clone()
}

/// E4: undo_last_commit rejects a merge commit as HEAD (instead of silently
/// losing the second merge side through a soft reset to parent(0)).
#[test]
fn undo_rejects_merge_commit() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    build_merge_head(&engine, &path);

    let err = engine.undo_last_commit(&path).unwrap_err();
    assert_eq!(err.code(), "invalid_operation");
}

/// E5: unpushed_commits returns an empty list on an unborn HEAD (a fresh repo
/// without a commit) instead of an error.
#[test]
fn unpushed_commits_empty_on_unborn_head() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    let out = engine.unpushed_commits(&path).unwrap();
    assert!(out.is_empty(), "unborn HEAD -> empty, was: {out:?}");
}

/// E6a: unpushed_commits marks a merge commit at HEAD correctly.
#[test]
fn unpushed_commits_detects_merge_head() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    build_merge_head(&engine, &path);

    let all = engine.unpushed_commits(&path).unwrap();
    assert!(all[0].is_head);
    assert!(all[0].is_merge, "HEAD is a merge commit");
    assert_eq!(all[0].parent_ids.len(), 2);
}

/// E6b: rebase_interactive over a range containing a merge commit is rejected
/// (the existing merge reject).
#[test]
fn rebase_rejects_range_with_merge() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    let merge = build_merge_head(&engine, &path);
    // Base = the first commit; the range base..HEAD contains the merge commit.
    let base = engine.log(&path, 0, 50).unwrap().pop().unwrap().id;

    let err = engine
        .rebase_interactive(&path, &base, &[step("pick", &merge)])
        .unwrap_err();
    assert_eq!(err.code(), "invalid_operation");
    assert_eq!(engine.op_state(&path).unwrap(), RepoOpState::Clean);
}

#[test]
fn worktree_ops_reject_option_injection() {
    let (_g, path) = init_repo();
    write(&path, "a.txt", "x\n");
    commit_all(&path, "Base");
    // A leading '-' in a path/branch must never pass as a git option.
    assert!(Git2Engine
        .add_worktree(&path, Path::new("--force"), "main")
        .is_err());
    assert!(Git2Engine.remove_worktree(&path, "--force").is_err());
}

/// Regression test for the CRLF data loss in hunk staging (autocrlf=true is the
/// Git-for-Windows default). Before the fix `file_patch` forced
/// `core.autocrlf=false`, which made the staging patch write the whole file into
/// the index with CRLF. The index MUST stay LF-normalized after staging.
#[test]
fn hunk_staging_does_not_pollute_index_with_autocrlf() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    {
        let repo = git2::Repository::open(&path).unwrap();
        repo.config()
            .unwrap()
            .set_bool("core.autocrlf", true)
            .unwrap();
    }
    // Committed with LF (the ODB stores LF).
    write(&path, "a.txt", "one\ntwo\nthree\n");
    commit_all(&path, "Base");
    // Add a line; the workdir uses CRLF (the Windows reality).
    fs::write(
        path.join("a.txt"),
        "one\r\ntwo\r\ntwo-and-a-half\r\nthree\r\n",
    )
    .unwrap();

    // The display diff (libgit2, autocrlf-normalized): exactly ONE addition.
    let display = engine.file_diff(&path, "a.txt", false).unwrap().unwrap();
    let additions = display
        .hunks
        .iter()
        .flat_map(|h| &h.lines)
        .filter(|l| matches!(l.kind, tg_domain::LineKind::Addition))
        .count();
    assert_eq!(additions, 1, "autocrlf: only the new line is an addition");

    // Stage hunk 0.
    engine.apply_hunk(&path, "a.txt", 0, false).unwrap();

    // The index blob must contain NO CR (otherwise CRLF pollution).
    let repo = git2::Repository::open(&path).unwrap();
    let idx = repo.index().unwrap();
    let entry = idx.get_path(Path::new("a.txt"), 0).expect("index entry");
    let blob = repo.find_blob(entry.id).unwrap();
    let content = std::str::from_utf8(blob.content()).unwrap();
    assert!(
        !content.contains('\r'),
        "the index must not contain CRLF after hunk staging: {content:?}"
    );
    assert!(
        content.contains("two-and-a-half"),
        "the staged line has to be in the index"
    );
}

#[test]
fn remotes_manage_add_rename_seturl_remove() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    write(&path, "a.txt", "base\n");
    commit_all(&path, "Base");

    engine
        .add_remote(&path, "origin", "https://example.com/repo.git")
        .unwrap();
    let remotes = engine.remotes(&path).unwrap();
    assert_eq!(remotes.len(), 1);
    assert_eq!(remotes[0].name, "origin");
    assert_eq!(remotes[0].url, "https://example.com/repo.git");

    engine.rename_remote(&path, "origin", "upstream").unwrap();
    let remotes = engine.remotes(&path).unwrap();
    assert_eq!(remotes.len(), 1);
    assert_eq!(remotes[0].name, "upstream");

    engine
        .set_remote_url(&path, "upstream", "https://example.com/other.git")
        .unwrap();
    assert_eq!(
        engine.remotes(&path).unwrap()[0].url,
        "https://example.com/other.git"
    );

    engine.remove_remote(&path, "upstream").unwrap();
    assert!(engine.remotes(&path).unwrap().is_empty());
}

#[test]
fn add_remote_rejects_invalid_and_duplicate_names() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    write(&path, "a.txt", "base\n");
    commit_all(&path, "Base");

    // A duplicate name has to be an error.
    engine
        .add_remote(&path, "origin", "https://example.com/repo.git")
        .unwrap();
    assert!(engine
        .add_remote(&path, "origin", "https://example.com/two.git")
        .is_err());

    // An invalid remote name (git2 validates the format).
    assert!(engine.add_remote(&path, "in valid", "https://x").is_err());

    // Removing/renaming an unknown remote is an error.
    assert!(engine.remove_remote(&path, "does-not-exist").is_err());
    assert!(engine
        .rename_remote(&path, "does-not-exist", "new")
        .is_err());
}

#[test]
fn backup_refs_list_and_restore() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    write(&path, "a.txt", "one\n");
    commit_all(&path, "Base");
    write(&path, "a.txt", "two\n");
    let second = commit_all(&path, "Second");
    write(&path, "a.txt", "three\n");
    let third = commit_all(&path, "Third");

    // Squashing the last two commits creates a backup of the old HEAD.
    engine.squash_from(&path, &second, "Squashed").unwrap();

    let backups = engine.backups(&path).unwrap();
    assert_eq!(backups.len(), 1);
    assert_eq!(backups[0].op, "squash");
    assert_eq!(
        backups[0].target_id, third,
        "the backup points at the old HEAD"
    );
    assert_eq!(backups[0].subject, "Third");
    assert!(backups[0].name.starts_with("refs/terra-git/backup/"));

    // Restoring hard-resets the branch back to the backed-up state …
    engine.restore_backup(&path, &backups[0].name).unwrap();
    let head = engine.log(&path, 0, 1).unwrap();
    assert_eq!(head[0].id, third);
    assert_eq!(fs::read_to_string(path.join("a.txt")).unwrap(), "three\n");

    // … and backs up the previous state itself in the process (restore is undoable).
    let backups = engine.backups(&path).unwrap();
    assert_eq!(backups.len(), 2);
    assert!(backups.iter().any(|b| b.op == "restore"));

    // Clean up: delete the backup.
    let name = backups[0].name.clone();
    engine.delete_backup(&path, &name).unwrap();
    assert_eq!(engine.backups(&path).unwrap().len(), 1);
}

#[test]
fn restore_backup_validates_ref_names() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    write(&path, "a.txt", "one\n");
    commit_all(&path, "Base");

    // Only refs/terra-git/backup/* are permitted — no arbitrary refs.
    assert!(engine.restore_backup(&path, "refs/heads/master").is_err());
    assert!(engine.delete_backup(&path, "refs/heads/master").is_err());
    // An unknown backup is an error.
    assert!(engine
        .restore_backup(&path, "refs/terra-git/backup/squash-999")
        .is_err());
}

#[test]
fn restore_backup_refuses_on_dirty_worktree() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    write(&path, "a.txt", "one\n");
    commit_all(&path, "Base");
    write(&path, "a.txt", "two\n");
    let second = commit_all(&path, "Second");
    write(&path, "a.txt", "three\n");
    commit_all(&path, "Third");

    // Create a backup of the old HEAD (a squash creates one).
    engine.squash_from(&path, &second, "Squashed").unwrap();
    let backup = engine.backups(&path).unwrap()[0].name.clone();

    // An uncommitted change to a tracked file.
    write(&path, "a.txt", "unsaved\n");

    // The hard reset of the restore would destroy that change — but the
    // confirmation dialog promises "the state is backed up first".
    // create_backup_ref only backs up the committed HEAD, not the workdir; a
    // restore MUST therefore be rejected on a dirty worktree (like apply_undo_action).
    assert!(
        engine.restore_backup(&path, &backup).is_err(),
        "restore must not run with uncommitted changes"
    );
    // Nothing lost: the change is still in the workdir.
    assert_eq!(fs::read_to_string(path.join("a.txt")).unwrap(), "unsaved\n");
    // And NO transient "restore" backup was created.
    assert!(!engine
        .backups(&path)
        .unwrap()
        .iter()
        .any(|b| b.op == "restore"));
}

#[test]
fn amend_creates_backup_ref() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    write(&path, "a.txt", "one\n");
    commit_all(&path, "Base");
    write(&path, "a.txt", "two\n");
    let before = commit_all(&path, "Tpyo");

    // An amend is a history rewrite: the old HEAD has to be anchored as a
    // durable backup first — the volatile undo stack alone does not survive an
    // app restart.
    engine.commit(&path, "Typo", true).unwrap();

    let backups = engine.backups(&path).unwrap();
    assert_eq!(backups.len(), 1, "amend creates exactly one backup");
    assert_eq!(backups[0].op, "amend");
    assert_eq!(
        backups[0].target_id, before,
        "the backup points at the old (unamended) HEAD"
    );
}

/// Name of the branch HEAD currently sits on.
fn current_branch(engine: &Git2Engine, path: &Path) -> String {
    engine
        .branches(path)
        .unwrap()
        .into_iter()
        .find(|b| b.is_head)
        .expect("HEAD sits on a branch")
        .name
}

#[test]
fn undo_reset_branch_soft_and_hard_roundtrip() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    write(&path, "a.txt", "one\n");
    let first = commit_all(&path, "First");
    write(&path, "a.txt", "two\n");
    let second = commit_all(&path, "Second");
    let branch = current_branch(&engine, &path);

    // Undo of the second commit: a soft reset to the first … (with the correct
    // expected_tip — the staleness guard lets the reset through).
    engine
        .apply_undo_action(
            &path,
            &UndoAction::ResetBranch {
                branch: branch.clone(),
                commit: first.clone(),
                mode: ResetMode::Soft,
            },
            Some(&second),
        )
        .unwrap();
    assert_eq!(engine.log(&path, 0, 1).unwrap()[0].id, first);
    // … the commit's changes stay staged.
    let status = engine.status(&path).unwrap();
    assert_eq!(status.staged.len(), 1);
    assert_eq!(status.staged[0].path, "a.txt");

    // Redo: a hard reset back to the second commit — allowed because the staged
    // changes match the target commit exactly.
    engine
        .apply_undo_action(
            &path,
            &UndoAction::ResetBranch {
                branch,
                commit: second.clone(),
                mode: ResetMode::Hard,
            },
            Some(&first),
        )
        .unwrap();
    assert_eq!(engine.log(&path, 0, 1).unwrap()[0].id, second);
    assert_eq!(fs::read_to_string(path.join("a.txt")).unwrap(), "two\n");
    assert!(engine.status(&path).unwrap().staged.is_empty());

    // An invalid commit id is an error.
    assert!(engine
        .apply_undo_action(
            &path,
            &UndoAction::ResetBranch {
                branch: current_branch(&engine, &path),
                commit: "nothex".into(),
                mode: ResetMode::Soft,
            },
            None,
        )
        .is_err());
}

#[test]
fn undo_hard_reset_refuses_on_dirty_worktree() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    write(&path, "a.txt", "one\n");
    let first = commit_all(&path, "First");
    write(&path, "a.txt", "two\n");
    commit_all(&path, "Second");
    let branch = current_branch(&engine, &path);

    // A changed tracked file -> the hard reset is refused …
    write(&path, "a.txt", "not committed\n");
    let err = engine
        .apply_undo_action(
            &path,
            &UndoAction::ResetBranch {
                branch: branch.clone(),
                commit: first.clone(),
                mode: ResetMode::Hard,
            },
            None,
        )
        .unwrap_err();
    assert_eq!(err.code(), "invalid_operation");
    // … and the change stays untouched.
    assert_eq!(
        fs::read_to_string(path.join("a.txt")).unwrap(),
        "not committed\n"
    );

    // An untracked file alone must NOT block (reset --hard leaves it standing —
    // nothing is lost).
    write(&path, "a.txt", "two\n");
    write(&path, "new.txt", "untracked\n");
    engine
        .apply_undo_action(
            &path,
            &UndoAction::ResetBranch {
                branch,
                commit: first,
                mode: ResetMode::Hard,
            },
            None,
        )
        .unwrap();
    assert_eq!(fs::read_to_string(path.join("a.txt")).unwrap(), "one\n");
    assert!(
        path.join("new.txt").exists(),
        "an untracked file survives the hard reset"
    );
}

#[test]
fn undo_reset_refuses_on_stale_tip() {
    // F15: the app's pre-check guard runs OUTSIDE the index lock — a second
    // command can commit in the await window. The engine guard (expected_tip)
    // then has to refuse the reset under the lock, otherwise the hard reset would
    // throw away the foreign commit.
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    write(&path, "a.txt", "one\n");
    let first = commit_all(&path, "First");
    write(&path, "a.txt", "two\n");
    let second = commit_all(&path, "Second");
    let branch = current_branch(&engine, &path);

    // A "foreign" commit after the recording: the tip is no longer `second`.
    write(&path, "b.txt", "foreign\n");
    let foreign = commit_all(&path, "Foreign commit");

    let err = engine
        .apply_undo_action(
            &path,
            &UndoAction::ResetBranch {
                branch,
                commit: first,
                mode: ResetMode::Hard,
            },
            Some(&second),
        )
        .unwrap_err();
    assert_eq!(err.code(), "undo_stale");
    // Nothing lost: the foreign commit is still the branch tip.
    assert_eq!(engine.log(&path, 0, 1).unwrap()[0].id, foreign);
    assert_eq!(fs::read_to_string(path.join("b.txt")).unwrap(), "foreign\n");
}

#[test]
fn undo_reset_refuses_on_foreign_branch() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    write(&path, "a.txt", "one\n");
    let first = commit_all(&path, "First");
    let main_name = current_branch(&engine, &path);

    // HEAD now sits on a branch other than the recorded one.
    engine.create_branch(&path, "other", true).unwrap();
    let err = engine
        .apply_undo_action(
            &path,
            &UndoAction::ResetBranch {
                branch: main_name,
                commit: first,
                mode: ResetMode::Soft,
            },
            None,
        )
        .unwrap_err();
    assert_eq!(err.code(), "invalid_operation");
    assert!(err.to_string().contains("original branch"));
}

#[test]
fn undo_branch_recreate_and_delete_again() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    write(&path, "a.txt", "one\n");
    let tip = commit_all(&path, "Base");

    engine.create_branch(&path, "feature", false).unwrap();
    engine.delete_branch(&path, "feature", true).unwrap();

    // Undo of the deletion: recreate the branch with the same tip.
    engine
        .apply_undo_action(
            &path,
            &UndoAction::RecreateBranch {
                name: "feature".into(),
                commit: tip.clone(),
            },
            None,
        )
        .unwrap();
    let branches = engine.branches(&path).unwrap();
    let feature = branches
        .iter()
        .find(|b| b.name == "feature")
        .expect("feature exists again");
    assert_eq!(feature.target_id.as_deref(), Some(tip.as_str()));

    // RecreateBranch onto an existing branch is an error.
    let err = engine
        .apply_undo_action(
            &path,
            &UndoAction::RecreateBranch {
                name: "feature".into(),
                commit: tip,
            },
            None,
        )
        .unwrap_err();
    assert_eq!(err.code(), "invalid_operation");

    // Redo of the deletion: the branch is gone again.
    engine
        .apply_undo_action(
            &path,
            &UndoAction::DeleteBranch {
                name: "feature".into(),
            },
            None,
        )
        .unwrap();
    assert!(!engine
        .branches(&path)
        .unwrap()
        .iter()
        .any(|b| b.name == "feature"));
}

#[test]
fn undo_checkout_switches_back() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    write(&path, "a.txt", "one\n");
    commit_all(&path, "Base");
    let main_name = current_branch(&engine, &path);

    engine.create_branch(&path, "b", true).unwrap();
    assert_eq!(current_branch(&engine, &path), "b");

    engine
        .apply_undo_action(
            &path,
            &UndoAction::Checkout {
                target: main_name.clone(),
            },
            None,
        )
        .unwrap();
    assert_eq!(current_branch(&engine, &path), main_name);
}

#[test]
fn undo_stash_restore_and_drop() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    write(&path, "a.txt", "one\n");
    commit_all(&path, "Base");

    write(&path, "a.txt", "changed\n");
    engine.stash_push(&path, "WIP Undo", &[]).unwrap();
    let stashes = engine.stash_list(&path).unwrap();
    assert_eq!(stashes.len(), 1);
    let id = stashes[0].id.clone();
    let message = stashes[0].message.clone();
    engine.stash_drop(&path, 0).unwrap();
    assert!(engine.stash_list(&path).unwrap().is_empty());

    // Undo of the drop: register the stash commit in the stack again.
    engine
        .apply_undo_action(
            &path,
            &UndoAction::RestoreStash {
                message,
                commit: id.clone(),
            },
            None,
        )
        .unwrap();
    let stashes = engine.stash_list(&path).unwrap();
    assert_eq!(stashes.len(), 1);
    assert_eq!(stashes[0].id, id);

    // Redo: drop by commit id (the index may have shifted).
    engine
        .apply_undo_action(
            &path,
            &UndoAction::DropStashByCommit { commit: id.clone() },
            None,
        )
        .unwrap();
    assert!(engine.stash_list(&path).unwrap().is_empty());

    // An unknown commit id -> "the stash no longer exists".
    let err = engine
        .apply_undo_action(&path, &UndoAction::DropStashByCommit { commit: id }, None)
        .unwrap_err();
    assert_eq!(err.code(), "invalid_operation");

    // Option/command injection in RestoreStash is rejected.
    assert!(engine
        .apply_undo_action(
            &path,
            &UndoAction::RestoreStash {
                message: "x".into(),
                commit: "abc; rm -rf".into(),
            },
            None,
        )
        .is_err());
}

#[test]
fn checkout_branch_reports_progress() {
    // Two branches whose checkout changes files in the worktree -> git2 reports
    // progress; at the end there has to be a 100% completion.
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    write(&path, "a.txt", "one\n");
    commit_all(&path, "Base");
    git(&path, &["checkout", "-b", "feature"]);
    write(&path, "a.txt", "two\nlines\n");
    write(&path, "b.txt", "new\n");
    commit_all(&path, "Feature");
    git(&path, &["checkout", "-"]); // back to the starting branch

    let mut percents: Vec<u8> = Vec::new();
    engine
        .checkout_branch_with_progress(&path, "feature", &mut |p| percents.push(p.percent))
        .unwrap();

    assert!(!percents.is_empty(), "the checkout has to report progress");
    assert_eq!(
        *percents.last().unwrap(),
        100,
        "the last event has to be 100 %"
    );
    let repo = git2::Repository::open(&path).unwrap();
    assert_eq!(repo.head().unwrap().shorthand().unwrap(), "feature");
}

#[test]
fn checkout_names_blocking_files_instead_of_only_their_count() {
    // User finding 2026-08-15: "cannot switch branches because of conflicts, but
    // no conflicts are shown". libgit2 reports uncommitted changes as "n conflicts
    // prevent checkout" — but there are neither conflicted files nor a running
    // operation. The error therefore has to carry its own code and the affected
    // paths.
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    write(&path, "a.txt", "one\n");
    write(&path, "b.txt", "one\n");
    commit_all(&path, "Base");
    git(&path, &["checkout", "-b", "feature"]);
    write(&path, "a.txt", "changed on feature\n");
    write(&path, "b.txt", "changed on feature\n");
    commit_all(&path, "Feature");
    git(&path, &["checkout", "-"]);
    // An uncommitted change to the same files: the switch would overwrite them.
    write(&path, "a.txt", "local, unsaved\n");
    write(&path, "b.txt", "local, unsaved\n");

    let err = engine.checkout_branch(&path, "feature").unwrap_err();

    assert_eq!(err.code(), "checkout_would_overwrite");
    match &err {
        tg_git_engine::error::GitEngineError::CheckoutWouldOverwrite { files } => {
            assert_eq!(files, &vec!["a.txt".to_string(), "b.txt".to_string()]);
        }
        other => panic!("wrong error type: {other:?}"),
    }
    // The working tree stays untouched, and so does the branch.
    assert_eq!(
        fs::read_to_string(path.join("a.txt")).unwrap(),
        "local, unsaved\n"
    );
    let repo = git2::Repository::open(&path).unwrap();
    assert_ne!(repo.head().unwrap().shorthand().unwrap(), "feature");
}

#[test]
fn checkout_without_blockers_stays_error_free() {
    // Counter-check: an uncommitted change to a file the switch does NOT touch
    // must not block it.
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    write(&path, "a.txt", "one\n");
    write(&path, "free.txt", "untouched\n");
    commit_all(&path, "Base");
    git(&path, &["checkout", "-b", "feature"]);
    write(&path, "a.txt", "changed on feature\n");
    commit_all(&path, "Feature");
    git(&path, &["checkout", "-"]);
    write(&path, "free.txt", "changed locally\n");

    engine.checkout_branch(&path, "feature").unwrap();

    let repo = git2::Repository::open(&path).unwrap();
    assert_eq!(repo.head().unwrap().shorthand().unwrap(), "feature");
    assert_eq!(
        fs::read_to_string(path.join("free.txt")).unwrap(),
        "changed locally\n",
        "the unsaved change survives the switch"
    );
}

#[test]
fn sparse_checkout_set_list_disable() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    write(&path, "a/x.txt", "a\n");
    write(&path, "b/y.txt", "b\n");
    write(&path, "c/z.txt", "c\n");
    write(&path, "root.txt", "root\n");
    commit_all(&path, "Base");

    // Initial state: disabled, all top-level directories as the selection base.
    let status = engine.sparse_status(&path).unwrap();
    assert!(!status.enabled);
    assert_eq!(status.top_dirs, vec!["a", "b", "c"]);
    assert!(status.patterns.is_empty());

    // Keep only "a": b and c disappear from the worktree, root files stay
    // (cone mode).
    engine.sparse_set(&path, &["a".into()]).unwrap();
    assert!(path.join("a").join("x.txt").exists());
    assert!(
        !path.join("b").exists(),
        "b has to disappear from the worktree"
    );
    assert!(path.join("root.txt").exists(), "cone mode keeps root files");
    let status = engine.sparse_status(&path).unwrap();
    assert!(status.enabled);
    assert!(
        status.patterns.contains(&"a".to_string()),
        "patterns: {:?}",
        status.patterns
    );

    // Extend the selection: c comes back, b stays away.
    engine.sparse_set(&path, &["a".into(), "c".into()]).unwrap();
    assert!(path.join("c").join("z.txt").exists(), "c is back");
    assert!(!path.join("b").exists(), "b stays hidden");

    // Disabling restores the full worktree.
    engine.sparse_disable(&path).unwrap();
    assert!(
        path.join("b").join("y.txt").exists(),
        "b is back after disable"
    );
    let status = engine.sparse_status(&path).unwrap();
    assert!(!status.enabled);
    assert!(status.patterns.is_empty());
}

#[test]
fn sparse_set_validates_input() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    write(&path, "a/x.txt", "a\n");
    commit_all(&path, "Base");

    // An empty list: a clear rejection instead of "hide everything".
    let err = engine.sparse_set(&path, &[]).unwrap_err();
    assert_eq!(err.code(), "invalid_operation");

    // Option/path injection and Windows separators are rejected.
    for bad in ["-evil", "a/../b", "a\\b"] {
        let err = engine.sparse_set(&path, &[bad.to_string()]).unwrap_err();
        assert_eq!(
            err.code(),
            "invalid_operation",
            "expected rejection for {bad:?}"
        );
    }

    // None of the rejected inputs may have enabled sparse checkout.
    assert!(!engine.sparse_status(&path).unwrap().enabled);
}

#[test]
fn sparse_status_in_empty_repo() {
    let (_g, path) = init_repo();
    let status = Git2Engine.sparse_status(&path).unwrap();
    assert!(!status.enabled);
    assert!(status.top_dirs.is_empty(), "unborn HEAD -> empty base");
    assert!(status.patterns.is_empty());
}

#[test]
fn clone_splits_into_prepare_then_fetch() {
    let (_g, src) = init_repo();
    let engine = Git2Engine;
    write(&src, "a.txt", "one\n");
    commit_all(&src, "First");
    write(&src, "a.txt", "two\n");
    commit_all(&src, "Second");

    let url = file_url(&src);
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("copy");

    // Stage 1: prepare (init + remote) — immediate, no data/files yet.
    engine.clone_prepare(&url, &dest).unwrap();
    pin_eol(&dest);
    assert!(dest.join(".git").exists());
    assert!(
        !dest.join("a.txt").exists(),
        "no files exist before the fetch"
    );

    // Stage 2: fetch the data + check out the default branch.
    engine
        .clone_fetch(
            &dest,
            &tg_domain::CloneOptions::default(),
            &tg_git_engine::CancelToken::new(),
            &mut |_| {},
        )
        .unwrap();
    let log = engine.log(&dest, 0, 10).unwrap();
    assert_eq!(log.len(), 2, "full history after the fetch");
    assert_eq!(log[0].summary, "Second");
    assert_eq!(
        fs::read_to_string(dest.join("a.txt")).unwrap(),
        "two\n",
        "the default branch is checked out"
    );

    // Preparing again into the now non-empty folder fails.
    assert!(engine.clone_prepare(&url, &dest).is_err());
}

#[test]
fn clone_fetch_honors_depth_and_blobless_filter() {
    let (_g, src) = init_repo();
    let engine = Git2Engine;
    write(&src, "a.txt", "one\n");
    commit_all(&src, "First");
    write(&src, "a.txt", "two\n");
    commit_all(&src, "Second");
    write(&src, "a.txt", "three\n");
    commit_all(&src, "Third");
    // The partial clone filter has to be allowed explicitly by the source (like a server).
    git(&src, &["config", "uploadpack.allowFilter", "true"]);

    // A file:// URL forces the real transport (a path clone ignores --depth).
    let url = file_url(&src);
    let dir = tempfile::tempdir().unwrap();

    // Shallow: only the newest commit arrives.
    let dest = dir.path().join("shallow");
    engine.clone_prepare(&url, &dest).unwrap();
    pin_eol(&dest);
    engine
        .clone_fetch(
            &dest,
            &tg_domain::CloneOptions {
                depth: Some(1),
                blobless: false,
                branch: None,
            },
            &tg_git_engine::CancelToken::new(),
            &mut |_| {},
        )
        .unwrap();
    let log = engine.log(&dest, 0, 10).unwrap();
    assert_eq!(log.len(), 1, "shallow clone: only 1 commit");
    assert_eq!(log[0].summary, "Third");
    assert!(dest.join(".git").join("shallow").exists());

    // Blobless: full history, but a promisor remote with a blob:none filter.
    let dest = dir.path().join("blobless");
    engine.clone_prepare(&url, &dest).unwrap();
    pin_eol(&dest);
    engine
        .clone_fetch(
            &dest,
            &tg_domain::CloneOptions {
                depth: None,
                blobless: true,
                branch: None,
            },
            &tg_git_engine::CancelToken::new(),
            &mut |_| {},
        )
        .unwrap();
    assert_eq!(
        engine.log(&dest, 0, 10).unwrap().len(),
        3,
        "blobless clone: full history"
    );
    let repo = git2::Repository::open(&dest).unwrap();
    let filter = repo
        .config()
        .unwrap()
        .get_string("remote.origin.partialclonefilter")
        .unwrap_or_default();
    assert_eq!(filter, "blob:none");

    // Default: full history, no filter, not shallow.
    let dest = dir.path().join("full");
    engine.clone_prepare(&url, &dest).unwrap();
    pin_eol(&dest);
    engine
        .clone_fetch(
            &dest,
            &tg_domain::CloneOptions::default(),
            &tg_git_engine::CancelToken::new(),
            &mut |_| {},
        )
        .unwrap();
    assert_eq!(engine.log(&dest, 0, 10).unwrap().len(), 3);
    assert!(!dest.join(".git").join("shallow").exists());
}

#[test]
fn clone_fetch_honors_chosen_branch() {
    // A source with two branches: the starting branch (default) and "feature"
    // with a file of its own. HEAD ends up back on the default so we can check
    // that --branch OVERRIDES the remote default.
    let (_g, src) = init_repo();
    let engine = Git2Engine;
    write(&src, "base.txt", "base\n");
    commit_all(&src, "Base");
    git(&src, &["checkout", "-b", "feature"]);
    write(&src, "feat.txt", "feature-only\n");
    commit_all(&src, "Feature commit");
    git(&src, &["checkout", "-"]); // back to the starting branch

    let url = file_url(&src);
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("from-branch");
    engine.clone_prepare(&url, &dest).unwrap();
    pin_eol(&dest);
    engine
        .clone_fetch(
            &dest,
            &tg_domain::CloneOptions {
                depth: None,
                blobless: false,
                branch: Some("feature".into()),
            },
            &tg_git_engine::CancelToken::new(),
            &mut |_| {},
        )
        .unwrap();

    // Exactly the chosen branch is checked out and its file is there.
    let repo = git2::Repository::open(&dest).unwrap();
    assert_eq!(repo.head().unwrap().shorthand().unwrap(), "feature");
    assert!(
        dest.join("feat.txt").exists(),
        "the feature-specific file has to be present"
    );
}

#[test]
fn pull_fetches_remote_commits() {
    // Create a bare "remote".
    let remote_dir = tempfile::tempdir().unwrap();
    git(remote_dir.path(), &["init", "--bare", "-q"]);
    let remote_url = remote_dir.path().to_string_lossy().replace('\\', "/");

    // Working repo: commit A, push with upstream tracking (so `pull` knows the
    // source).
    let (_g, work) = init_repo();
    write(&work, "a.txt", "A\n");
    commit_all(&work, "A");
    git(&work, &["remote", "add", "origin", &remote_url]);
    git(&work, &["push", "-u", "-q", "origin", "HEAD"]);

    // A second clone moves the remote forward (commit B).
    let other_dir = tempfile::tempdir().unwrap();
    let other = other_dir.path().join("clone");
    git(other_dir.path(), &["clone", "-q", &remote_url, "clone"]);
    git(&other, &["config", "user.name", "Other"]);
    git(&other, &["config", "user.email", "other@test.local"]);
    git(&other, &["config", "commit.gpgsign", "false"]);
    std::fs::write(other.join("b.txt"), "B\n").unwrap();
    git(&other, &["add", "-A"]);
    git(&other, &["commit", "-q", "-m", "B"]);
    git(&other, &["push", "-q", "origin", "HEAD"]);

    // The engine pull fetches B (fast-forward) including the working copy.
    Git2Engine.pull(&work).unwrap();
    assert!(
        summaries(&work).iter().any(|s| s == "B"),
        "pull did not fetch commit B: {:?}",
        summaries(&work)
    );
    assert!(work.join("b.txt").exists(), "pull did not check out b.txt");
}

#[test]
fn rebase_onto_linearizes_diverged_branches() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    write(&path, "base.txt", "base\n");
    commit_all(&path, "Base");
    let base_branch = current_branch(&engine, &path);

    // A feature branch with its own commit.
    engine.create_branch(&path, "feature", true).unwrap();
    write(&path, "feat.txt", "feat\n");
    commit_all(&path, "Feature");

    // The base branch diverges (its own commit).
    engine.checkout_branch(&path, &base_branch).unwrap();
    write(&path, "main2.txt", "main2\n");
    commit_all(&path, "Main2");

    // Back on feature, rebase onto base → a linear history base→Main2→Feature.
    engine.checkout_branch(&path, "feature").unwrap();
    engine.rebase_onto(&path, &base_branch).unwrap();

    let s = summaries(&path);
    assert_eq!(
        s.first().map(String::as_str),
        Some("Feature"),
        "History: {s:?}"
    );
    assert!(
        s.iter().any(|x| x == "Main2"),
        "Main2 not in the base: {s:?}"
    );
    assert_eq!(engine.op_state(&path).unwrap(), RepoOpState::Clean);
    assert!(path.join("main2.txt").exists() && path.join("feat.txt").exists());
}

#[test]
fn branches_marks_orphaned_upstream() {
    let (_g, path) = init_repo();
    write(&path, "f.txt", "x\n");
    commit_all(&path, "c1");
    // Branch "feature" with a configured but missing remote upstream (simulated:
    // origin/feature was deleted + pruned).
    let repo = git2::Repository::open(&path).unwrap();
    let head = repo.head().unwrap().peel_to_commit().unwrap();
    repo.branch("feature", &head, false).unwrap();
    {
        let mut cfg = repo.config().unwrap();
        cfg.set_str("branch.feature.remote", "origin").unwrap();
        cfg.set_str("branch.feature.merge", "refs/heads/feature")
            .unwrap();
    }
    // A purely local branch without any upstream config -> not orphaned.
    repo.branch("local", &head, false).unwrap();
    // A local upstream (remote=".") pointing at a non-existent ref: upstream()
    // fails, but it must NOT count as orphaned (it is no remote branch).
    repo.branch("rebase-base", &head, false).unwrap();
    {
        let mut cfg = repo.config().unwrap();
        cfg.set_str("branch.rebase-base.remote", ".").unwrap();
        cfg.set_str("branch.rebase-base.merge", "refs/heads/does-not-exist")
            .unwrap();
    }

    let branches = Git2Engine.branches(&path).unwrap();
    let feature = branches.iter().find(|b| b.name == "feature").unwrap();
    assert!(feature.upstream_gone, "feature has to count as orphaned");
    // The current branch (no upstream configured) is not orphaned.
    let cur = branches.iter().find(|b| b.is_head).unwrap();
    assert!(!cur.upstream_gone);
    // Negative cases: neither without a config nor with a local upstream (remote=".").
    let local = branches.iter().find(|b| b.name == "local").unwrap();
    assert!(
        !local.upstream_gone,
        "a local branch without an upstream is not orphaned"
    );
    let rebase = branches.iter().find(|b| b.name == "rebase-base").unwrap();
    assert!(
        !rebase.upstream_gone,
        "a local upstream (remote=.) is not orphaned"
    );
}

#[test]
fn bisect_finds_first_bad_commit_and_resets() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    write(&path, "f.txt", "v1\n");
    let good = commit_all(&path, "good");
    write(&path, "f.txt", "v2\n");
    commit_all(&path, "c2");
    write(&path, "f.txt", "v2\nBUG\n");
    let bug = commit_all(&path, "introduces bug");
    write(&path, "f.txt", "v2\nBUG\nv4\n");
    commit_all(&path, "c4"); // BUG is kept -> bad iff the file contains "BUG"

    let mut out = engine.bisect_start(&path, &good, None).unwrap();
    assert_eq!(engine.status(&path).unwrap().op_state, RepoOpState::Bisect);

    // Simulate the user: test and mark the currently checked-out commit.
    let mut found = None;
    for _ in 0..12 {
        // git 2.55 quotes the term: "… is the first 'bad' commit". Match both
        // wordings — the same tolerance the frontend parser has.
        if let Some(line) = out.lines().find(|l| {
            l.contains("is the first bad commit") || l.contains("is the first 'bad' commit")
        }) {
            found = line.split_whitespace().next().map(str::to_string);
            break;
        }
        let content = std::fs::read_to_string(path.join("f.txt")).unwrap();
        let action = if content.contains("BUG") {
            "bad"
        } else {
            "good"
        };
        out = engine.bisect_mark(&path, action).unwrap();
    }
    assert_eq!(found.expect("first bad commit"), bug);

    engine.bisect_reset(&path).unwrap();
    assert_eq!(engine.status(&path).unwrap().op_state, RepoOpState::Clean);
}

#[test]
fn bisect_validates_ref_and_action() {
    let (_g, path) = init_repo();
    write(&path, "f.txt", "x\n");
    commit_all(&path, "c");
    let engine = Git2Engine;
    assert!(engine.bisect_start(&path, "not-hex", None).is_err());
    assert!(engine.bisect_mark(&path, "wat").is_err()); // invalid action
}

#[test]
fn status_numstat_counts_lines_and_binary() {
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    write(
        &path, "a.txt", "one
two
",
    );
    commit_all(&path, "Base");

    // Modification: "two" out, "three"+"four" in -> +2/-1.
    write(
        &path,
        "a.txt",
        "one
three
four
",
    );
    // An untracked text file -> fully as additions.
    write(
        &path, "new.txt", "x
y
",
    );
    // A binary file (NUL byte) -> the binary flag, lines 0/0.
    fs::write(path.join("image.bin"), [0u8, 159, 146, 150]).unwrap();

    let stats = engine.status_numstat(&path).unwrap();
    let by_path = |p: &str| {
        stats
            .iter()
            .find(|s| s.path == p)
            .unwrap_or_else(|| panic!("{p} missing in {stats:?}"))
    };
    let a = by_path("a.txt");
    assert_eq!((a.added, a.deleted, a.binary), (2, 1, false));
    let new = by_path("new.txt");
    assert_eq!((new.added, new.deleted, new.binary), (2, 0, false));
    let bin = by_path("image.bin");
    assert!(bin.binary, "a NUL file has to be recognized as binary");
    assert_eq!((bin.added, bin.deleted), (0, 0));
}

#[test]
fn status_numstat_unborn_head_and_rename() {
    // Unborn HEAD (a fresh repo without a commit): everything counts as an addition.
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    write(
        &path,
        "first.txt",
        "a
b
c
",
    );
    let stats = engine.status_numstat(&path).unwrap();
    assert_eq!(stats.len(), 1, "{stats:?}");
    assert_eq!((stats[0].added, stats[0].deleted), (3, 0));

    // Rename: ONE entry under the new path (matching the status model), not a
    // full deletion + a full addition.
    commit_all(&path, "Base");
    git(&path, &["mv", "first.txt", "second.txt"]);
    let stats = engine.status_numstat(&path).unwrap();
    let r = stats
        .iter()
        .find(|s| s.path == "second.txt")
        .expect("entry under the new path");
    assert_eq!((r.added, r.deleted), (0, 0));
    assert!(
        !stats.iter().any(|s| s.path == "first.txt"),
        "the old path must not appear additionally: {stats:?}"
    );
}

#[test]
fn init_repo_falls_back_to_main_and_branch_on_unborn_head() {
    // (5a) init_repo: fallback head name "main" — as long as the host sets no
    // init.defaultBranch configuration (THAT one applies then, which the
    // expected value takes into account).
    let dir = tempfile::tempdir().unwrap();
    let engine = Git2Engine;
    let repo_dir = dir.path().join("fresh");
    engine.init_repo(&repo_dir).unwrap();
    let expected = git2::Config::open_default()
        .ok()
        .and_then(|mut c| c.snapshot().ok())
        .and_then(|c| c.get_string("init.defaultBranch").ok())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "main".into());
    let repo = git2::Repository::open(&repo_dir).unwrap();
    assert_eq!(
        repo.find_reference("HEAD")
            .unwrap()
            .symbolic_target()
            .unwrap(),
        Some(format!("refs/heads/{expected}").as_str())
    );

    // (5b) Creating a branch on an unborn HEAD: the symbolic HEAD moves; the
    // branch comes into existence with the first commit.
    engine
        .create_branch(&repo_dir, "feature/start", true)
        .unwrap();
    {
        let repo = git2::Repository::open(&repo_dir).unwrap();
        let mut cfg = repo.config().unwrap();
        cfg.set_str("user.name", "Terra Tester").unwrap();
        cfg.set_str("user.email", "terra@test.local").unwrap();
        cfg.set_str("commit.gpgsign", "false").unwrap();
    }
    write(&repo_dir, "a.txt", "x");
    commit_all(&repo_dir, "First");
    let repo = git2::Repository::open(&repo_dir).unwrap();
    assert_eq!(repo.head().unwrap().shorthand().unwrap(), "feature/start");

    // An invalid name stays a clean error (no panic/raw error).
    let empty = dir.path().join("empty");
    engine.init_repo(&empty).unwrap();
    assert!(engine.create_branch(&empty, "broken..name", false).is_err());
}

#[test]
fn rename_unborn_default_branch() {
    let dir = tempfile::tempdir().unwrap();
    let engine = Git2Engine;
    let repo_dir = dir.path().join("fresh");
    engine.init_repo(&repo_dir).unwrap();
    let old = {
        let repo = git2::Repository::open(&repo_dir).unwrap();
        let head = repo.find_reference("HEAD").unwrap();
        head.symbolic_target()
            .unwrap()
            .unwrap()
            .strip_prefix("refs/heads/")
            .unwrap()
            .to_string()
    };
    // (5b) Renaming the unborn default branch = moving HEAD.
    engine.rename_branch(&repo_dir, &old, "trunk").unwrap();
    let repo = git2::Repository::open(&repo_dir).unwrap();
    assert_eq!(
        repo.find_reference("HEAD")
            .unwrap()
            .symbolic_target()
            .unwrap(),
        Some("refs/heads/trunk")
    );
    // Non-existent names stay BranchNotFound.
    assert!(engine
        .rename_branch(&repo_dir, "does-not-exist", "x")
        .is_err());
}

#[test]
fn search_finds_commits_on_foreign_branches() {
    // The history search covers the whole graph — not only the HEAD line.
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    write(&path, "a.txt", "x");
    commit_all(&path, "Base");
    git(&path, &["checkout", "-b", "side"]);
    write(&path, "b.txt", "y");
    let tip = commit_all(&path, "Needle in the haystack");
    git(&path, &["checkout", "-"]);

    let hits = engine.search_log(&path, "needle", 10).unwrap();
    assert!(hits.iter().any(|c| c.id == tip), "{hits:?}");
}

#[test]
fn repo_sketch_returns_line_branches_and_tags() {
    // Welcome screen vein: the HEAD line with tag markers,
    // branches with a branch-point index (merge base) and an ahead count.
    let (_g, path) = init_repo();
    let engine = Git2Engine;
    write(&path, "a.txt", "1");
    let c1 = commit_all(&path, "one");
    write(&path, "a.txt", "2");
    let c2 = commit_all(&path, "two");
    let main_branch = engine.open_repo(&path).unwrap().current_branch.unwrap();

    // A branch from c1 with its own commit: ahead 1, branch point at c1.
    engine
        .create_branch_from_commit(&path, "feature", &c1, true)
        .unwrap();
    write(&path, "f.txt", "f");
    commit_all(&path, "feature work");
    git(&path, &["checkout", &main_branch]);
    write(&path, "a.txt", "3");
    commit_all(&path, "three");
    git(&path, &["tag", "v1", &c2]);

    let sketch = engine.repo_sketch(&path, 12, 5).unwrap();
    // HEAD line: three, two, one — only "two" carries the tag.
    assert_eq!(sketch.commits.len(), 3);
    assert!(!sketch.commits[0].has_tag);
    assert!(sketch.commits[1].has_tag);
    assert!(!sketch.commits[2].has_tag);
    // Exactly one branch (the HEAD branch itself is missing), branch point at index 2 (c1).
    assert_eq!(sketch.branches.len(), 1);
    let b = &sketch.branches[0];
    assert_eq!(b.name, "feature");
    assert_eq!(b.base_index, Some(2));
    assert_eq!(b.ahead, 1);
    assert!(b.tip_time > 0);
}

#[test]
fn repo_sketch_unborn_head_is_empty_and_window_caps() {
    let engine = Git2Engine;

    // Unborn HEAD: an empty sketch instead of an error (the decorative vein).
    let (_g, empty) = init_repo();
    let sketch = engine.repo_sketch(&empty, 12, 5).unwrap();
    assert!(sketch.commits.is_empty());
    assert!(sketch.branches.is_empty());

    // A branch point outside the window: base_index None, the branch stays listed.
    let (_g2, path) = init_repo();
    write(&path, "a.txt", "0");
    let c1 = commit_all(&path, "old");
    engine
        .create_branch_from_commit(&path, "ancient", &c1, false)
        .unwrap();
    for i in 0..4 {
        write(&path, "a.txt", &i.to_string());
        commit_all(&path, &format!("new {i}"));
    }
    // Window 2: c1 (the merge base of "ancient") is no longer inside the window.
    let sketch = engine.repo_sketch(&path, 2, 5).unwrap();
    assert_eq!(sketch.commits.len(), 2);
    assert_eq!(sketch.branches.len(), 1);
    assert_eq!(sketch.branches[0].base_index, None);
    assert_eq!(sketch.branches[0].ahead, 0);
}
