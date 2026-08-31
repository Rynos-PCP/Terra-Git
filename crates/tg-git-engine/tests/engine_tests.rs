//! Integration tests of Git2Engine against real fixture repos (tempfile).

use std::fs;
use std::path::{Path, PathBuf};

use tg_domain::{ChangeKind, LineKind};
use tg_git_engine::ops::ConflictOps;
use tg_git_engine::{error::GitEngineError, Git2Engine, GitEngine};

/// Creates a fresh repo with the user config set.
fn init_repo() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = git2::Repository::init(dir.path()).expect("init");
    let mut config = repo.config().expect("config");
    config.set_str("user.name", "Terra Tester").unwrap();
    config.set_str("user.email", "terra@test.local").unwrap();
    // Hermetic against host configuration: a global commit.gpgsign=true (without
    // a key) would otherwise make all sidecar commits of the fixtures fail.
    config.set_str("commit.gpgsign", "false").unwrap();
    // Likewise for line endings. GitHub's Windows runners set core.autocrlf=true
    // globally, which rewrites the fixtures' LF content to CRLF on checkout and
    // makes every content assertion in this file fail. The fixtures are written
    // with LF and compared as LF, so the conversion is switched off here rather
    // than worked around in each assertion. Tests that examine the conversion
    // itself set their own value after this (see ops_tests/unchanged_tests).
    config.set_bool("core.autocrlf", false).unwrap();
    // A global attributes file marking files as text would convert despite
    // autocrlf=false; core.eol decides the direction, so pin it as well.
    config.set_str("core.eol", "lf").unwrap();
    let path = dir.path().to_path_buf();
    (dir, path)
}

fn write(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(p, content).unwrap();
}

fn stage_all_and_commit(path: &Path, msg: &str) -> String {
    let engine = Git2Engine;
    let status = engine.status(path).unwrap();
    let files: Vec<String> = status.unstaged.iter().map(|e| e.path.clone()).collect();
    if !files.is_empty() {
        engine.stage(path, &files).unwrap();
    }
    engine.commit(path, msg, false).unwrap()
}

/// Raw git for test setup beyond the engine API (trimmed stdout on success, Err
/// on a failing exit). Replaces the former `sidecar::run_git` in tests.
fn git_raw(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .expect("git start");
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

#[test]
fn open_repo_returns_info() {
    let (_guard, path) = init_repo();
    let info = Git2Engine.open_repo(&path).unwrap();
    assert!(info.is_empty, "a fresh repo is empty");
    assert!(!info.head_detached);
    // Regression test: git2 returns the workdir with a trailing separator,
    // RepoInfo.path has to be normalized (otherwise duplicate recents).
    assert!(
        !info.path.ends_with('/') && !info.path.ends_with('\\'),
        "path must not end with a separator: {}",
        info.path
    );
    assert_eq!(
        Path::new(&info.path).canonicalize().unwrap(),
        path.canonicalize().unwrap()
    );
}

/// Regression: a repository without a commit is empty no matter what its HEAD
/// branch is called. git2's `Repository::is_empty()` additionally compares HEAD
/// against the branch `init.defaultBranch` names — so on a machine that
/// configures nothing (the default on Linux and macOS), a repo created with
/// `main` was compared against the built-in `master` and reported as NOT empty.
/// `init_repo()` creates exactly such repos, which made every freshly created
/// repository look non-empty there.
#[test]
fn a_fresh_repo_is_empty_whatever_head_is_called() {
    let dir = tempfile::tempdir().unwrap();
    let mut opts = git2::RepositoryInitOptions::new();
    // Deliberately neither "main" nor "master": this mismatches init.defaultBranch
    // on every host, so the test reproduces the case everywhere.
    opts.initial_head("trunk");
    git2::Repository::init_opts(dir.path(), &opts).unwrap();

    let info = Git2Engine.open_repo(dir.path()).unwrap();
    assert!(info.is_empty, "no commit yet, so the repo is empty");
    assert_eq!(info.current_branch, None, "an unborn HEAD has no branch");
    assert!(info.history_prepared, "no history, nothing to prepare");
}

#[test]
fn open_repo_fails_on_non_repo() {
    let dir = tempfile::tempdir().unwrap();
    let err = Git2Engine.open_repo(dir.path()).unwrap_err();
    assert!(matches!(err, GitEngineError::NotARepository(_)));
    assert_eq!(err.code(), "not_a_repository");
}

#[test]
fn status_detects_untracked_and_staged() {
    let (_guard, path) = init_repo();
    let engine = Git2Engine;

    write(&path, "a.txt", "hello\n");
    write(&path, "sub/b.txt", "world\n");

    let status = engine.status(&path).unwrap();
    assert_eq!(status.staged.len(), 0);
    assert_eq!(status.unstaged.len(), 2);
    assert!(status
        .unstaged
        .iter()
        .all(|e| e.kind == ChangeKind::Untracked));

    engine.stage(&path, &["a.txt".into()]).unwrap();
    let status = engine.status(&path).unwrap();
    assert_eq!(status.staged.len(), 1);
    assert_eq!(status.staged[0].kind, ChangeKind::Added);
    assert_eq!(status.unstaged.len(), 1);
    assert_eq!(status.unstaged[0].path, "sub/b.txt");
}

#[test]
fn commit_and_log_work() {
    let (_guard, path) = init_repo();
    let engine = Git2Engine;

    write(&path, "a.txt", "v1\n");
    let id1 = stage_all_and_commit(&path, "First commit");
    write(&path, "a.txt", "v2\n");
    engine.stage(&path, &["a.txt".into()]).unwrap();
    let id2 = engine.commit(&path, "Second commit", false).unwrap();

    let log = engine.log(&path, 0, 10).unwrap();
    assert_eq!(log.len(), 2);
    assert_eq!(log[0].id, id2);
    assert_eq!(log[0].summary, "Second commit");
    assert_eq!(log[0].author_name, "Terra Tester");
    assert_eq!(log[0].parent_ids, vec![id1.clone()]);
    assert_eq!(log[1].id, id1);

    // Paging
    let page2 = engine.log(&path, 1, 10).unwrap();
    assert_eq!(page2.len(), 1);
    assert_eq!(page2[0].id, id1);
}

#[test]
fn log_survives_special_characters_and_merge_parents() {
    let (_guard, path) = init_repo();
    let engine = Git2Engine;

    // A subject with umlauts, emoji, a tab and format-like sequences — it has to
    // pass the sidecar parser (NUL/\x1f-separated records) unharmed.
    write(&path, "a.txt", "v1\n");
    let subject = "Ümläute 🎉 with\ttab, %H%x1f and \"quotes\"";
    let id1 = stage_all_and_commit(&path, subject);

    // Merge commit: a side branch from id1, commit on both sides, then merge.
    git_raw(&path, &["checkout", "-q", "-b", "side"]).unwrap();
    write(&path, "b.txt", "side\n");
    let id_side = stage_all_and_commit(&path, "Side branch");
    git_raw(&path, &["checkout", "-q", "-"]).unwrap();
    write(&path, "c.txt", "main\n");
    let id_main = stage_all_and_commit(&path, "Main branch");
    git_raw(
        &path,
        &["merge", "-q", "--no-ff", "-m", "Merge side", "side"],
    )
    .unwrap();

    let log = engine.log(&path, 0, 10).unwrap();
    assert_eq!(log.len(), 4);
    // Merge at the tip with BOTH parents (the first parent = the main branch).
    assert_eq!(log[0].summary, "Merge side");
    assert_eq!(log[0].parent_ids, vec![id_main.clone(), id_side.clone()]);
    assert_eq!(log[0].author_name, "Terra Tester");
    assert!(log[0].time > 0);
    // Topo order: children before parents — the root comes last, intact.
    assert_eq!(log[3].id, id1);
    assert_eq!(log[3].summary, subject);
    assert_eq!(log[3].short_id, id1.chars().take(8).collect::<String>());

    // Paging stays consistent with the undivided list.
    let mut paged = engine.log(&path, 0, 2).unwrap();
    paged.extend(engine.log(&path, 2, 2).unwrap());
    assert_eq!(
        paged.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
        log.iter().map(|c| c.id.as_str()).collect::<Vec<_>>()
    );
}

#[test]
fn history_prepared_follows_the_commit_graph() {
    let (_guard, path) = init_repo();
    let engine = Git2Engine;
    // An empty repo counts as prepared — there is no history.
    assert!(engine.open_repo(&path).unwrap().history_prepared);

    write(&path, "a.txt", "v1\n");
    stage_all_and_commit(&path, "First");
    // With commits but without a commit graph: not prepared.
    assert!(!engine.open_repo(&path).unwrap().history_prepared);

    engine.write_commit_graph(&path).unwrap();
    assert!(engine.open_repo(&path).unwrap().history_prepared);
}

#[test]
fn empty_repo_log_is_empty_and_commit_without_message_forbidden() {
    let (_guard, path) = init_repo();
    let engine = Git2Engine;
    assert!(engine.log(&path, 0, 10).unwrap().is_empty());
    let err = engine.commit(&path, "   ", false).unwrap_err();
    assert_eq!(err.code(), "empty_commit_message");
}

#[test]
fn amend_replaces_last_commit() {
    let (_guard, path) = init_repo();
    let engine = Git2Engine;

    write(&path, "a.txt", "one\n");
    stage_all_and_commit(&path, "Tpyo");

    write(&path, "b.txt", "two\n");
    engine.stage(&path, &["b.txt".into()]).unwrap();
    engine.commit(&path, "Typo fixed", true).unwrap();

    let log = engine.log(&path, 0, 10).unwrap();
    assert_eq!(log.len(), 1, "amend must not create a second commit");
    assert_eq!(log[0].summary, "Typo fixed");

    let status = engine.status(&path).unwrap();
    assert!(status.staged.is_empty() && status.unstaged.is_empty());
}

#[test]
fn amend_without_commit_fails() {
    let (_guard, path) = init_repo();
    let err = Git2Engine.commit(&path, "whatever", true).unwrap_err();
    assert_eq!(err.code(), "nothing_to_amend");
}

#[test]
fn unstage_and_discard() {
    let (_guard, path) = init_repo();
    let engine = Git2Engine;

    write(&path, "a.txt", "base\n");
    stage_all_and_commit(&path, "Base");

    // Stage the modification, then unstage it
    write(&path, "a.txt", "changed\n");
    engine.stage(&path, &["a.txt".into()]).unwrap();
    assert_eq!(engine.status(&path).unwrap().staged.len(), 1);
    engine.unstage(&path, &["a.txt".into()]).unwrap();
    let status = engine.status(&path).unwrap();
    assert!(status.staged.is_empty());
    assert_eq!(status.unstaged.len(), 1);
    assert_eq!(status.unstaged[0].kind, ChangeKind::Modified);

    // Discard restores the file from the index
    engine.discard(&path, &["a.txt".into()]).unwrap();
    assert_eq!(fs::read_to_string(path.join("a.txt")).unwrap(), "base\n");
    assert!(engine.status(&path).unwrap().unstaged.is_empty());

    // Discard deletes untracked files
    write(&path, "new.txt", "throw away\n");
    engine.discard(&path, &["new.txt".into()]).unwrap();
    assert!(!path.join("new.txt").exists());
}

#[test]
fn stage_deleted_file() {
    let (_guard, path) = init_repo();
    let engine = Git2Engine;

    write(&path, "gone.txt", "soon gone\n");
    stage_all_and_commit(&path, "File created");

    fs::remove_file(path.join("gone.txt")).unwrap();
    let status = engine.status(&path).unwrap();
    assert_eq!(status.unstaged[0].kind, ChangeKind::Deleted);

    engine.stage(&path, &["gone.txt".into()]).unwrap();
    let status = engine.status(&path).unwrap();
    assert_eq!(status.staged[0].kind, ChangeKind::Deleted);
    assert!(status.unstaged.is_empty());

    engine.commit(&path, "File removed", false).unwrap();
    assert!(engine.status(&path).unwrap().staged.is_empty());
}

#[test]
fn file_diff_workdir_and_staged() {
    let (_guard, path) = init_repo();
    let engine = Git2Engine;

    write(&path, "a.txt", "line1\nline2\nline3\n");
    stage_all_and_commit(&path, "Base");

    write(&path, "a.txt", "line1\nLINE TWO\nline3\n");

    // Unstaged diff: workdir vs index
    let diff = engine.file_diff(&path, "a.txt", false).unwrap().unwrap();
    assert_eq!(diff.path, "a.txt");
    assert!(!diff.is_binary);
    assert_eq!(diff.hunks.len(), 1);
    let lines = &diff.hunks[0].lines;
    assert!(lines
        .iter()
        .any(|l| l.kind == LineKind::Deletion && l.content == "line2"));
    assert!(lines
        .iter()
        .any(|l| l.kind == LineKind::Addition && l.content == "LINE TWO"));

    // Staged diff: index vs HEAD (still empty)
    let staged = engine.file_diff(&path, "a.txt", true).unwrap();
    assert!(staged.is_none() || staged.unwrap().hunks.is_empty());

    engine.stage(&path, &["a.txt".into()]).unwrap();
    let staged = engine.file_diff(&path, "a.txt", true).unwrap().unwrap();
    assert_eq!(staged.hunks.len(), 1);
}

#[test]
fn file_diff_binary_carries_sizes() {
    let (_guard, path) = init_repo();
    let engine = Git2Engine;
    fs::write(path.join("a.bin"), [0u8, 1, 2, 3]).unwrap();
    stage_all_and_commit(&path, "bin");
    fs::write(path.join("a.bin"), [0u8, 1, 2, 3, 4, 5]).unwrap();
    let d = engine.file_diff(&path, "a.bin", false).unwrap().unwrap();
    assert!(d.is_binary);
    assert_eq!(d.old_size, Some(4));
    assert_eq!(d.new_size, Some(6));
}

#[test]
fn untracked_file_has_diff_with_content() {
    let (_guard, path) = init_repo();
    write(&path, "new.txt", "content A\ncontent B\n");
    let diff = Git2Engine
        .file_diff(&path, "new.txt", false)
        .unwrap()
        .unwrap();
    let adds = diff.hunks[0]
        .lines
        .iter()
        .filter(|l| l.kind == LineKind::Addition)
        .count();
    assert_eq!(adds, 2, "untracked content has to appear as additions");
}

#[test]
fn commit_diff_against_parent_and_root() {
    let (_guard, path) = init_repo();
    let engine = Git2Engine;

    write(&path, "a.txt", "one\n");
    let root = stage_all_and_commit(&path, "Root");
    write(&path, "a.txt", "one\ntwo\n");
    engine.stage(&path, &["a.txt".into()]).unwrap();
    let second = engine.commit(&path, "Extended", false).unwrap();

    // Normal commit: diff against the parent
    let diffs = engine.commit_diff(&path, &second).unwrap();
    assert_eq!(diffs.len(), 1);
    assert!(diffs[0].hunks[0]
        .lines
        .iter()
        .any(|l| l.kind == LineKind::Addition && l.content == "two"));

    // Root commit: diff against the empty tree
    let diffs = engine.commit_diff(&path, &root).unwrap();
    assert_eq!(diffs.len(), 1);
    assert!(diffs[0].hunks[0]
        .lines
        .iter()
        .all(|l| l.kind == LineKind::Addition));
}

#[test]
fn commit_diff_stream_delivers_per_file_and_truncates() {
    let (_guard, path) = init_repo();
    let engine = Git2Engine;

    // One commit with 5 files.
    for i in 0..5 {
        write(&path, &format!("d{i}.txt"), &format!("content {i}\n"));
    }
    let id = stage_all_and_commit(&path, "Five files");

    // Untruncated: stream == the collecting variant (same files, same hunks).
    let collected = engine.commit_diff(&path, &id).unwrap();
    let mut streamed = Vec::new();
    let total = engine
        .commit_diff_stream(&path, &id, usize::MAX, &mut |fd| {
            streamed.push(fd);
            true
        })
        .unwrap();
    assert_eq!(total, 5);
    assert_eq!(streamed.len(), collected.len());
    for (s, c) in streamed.iter().zip(&collected) {
        assert_eq!(s.path, c.path);
        assert_eq!(s.hunks.len(), c.hunks.len());
        assert_eq!(
            s.hunks.iter().map(|h| h.lines.len()).sum::<usize>(),
            c.hunks.iter().map(|h| h.lines.len()).sum::<usize>()
        );
    }

    // Truncation: max_files=2 -> exactly 2 delivered, the total stays 5.
    let mut capped = Vec::new();
    let total = engine
        .commit_diff_stream(&path, &id, 2, &mut |fd| {
            capped.push(fd.path);
            true
        })
        .unwrap();
    assert_eq!(total, 5);
    assert_eq!(capped.len(), 2);

    // Sink abort: false after the first file -> exactly 1 delivered.
    let mut got = 0;
    engine
        .commit_diff_stream(&path, &id, usize::MAX, &mut |_| {
            got += 1;
            false
        })
        .unwrap();
    assert_eq!(got, 1);
}

#[test]
fn branches_create_switch_and_list() {
    let (_guard, path) = init_repo();
    let engine = Git2Engine;

    write(&path, "a.txt", "base\n");
    stage_all_and_commit(&path, "Base");

    engine.create_branch(&path, "feature/x", true).unwrap();
    let info = engine.open_repo(&path).unwrap();
    assert_eq!(info.current_branch.as_deref(), Some("feature/x"));

    // Commit a change on feature/x
    write(&path, "feat.txt", "feature\n");
    stage_all_and_commit(&path, "Feature commit");

    // Switch back: feat.txt disappears from the workdir
    let default_branch = engine
        .branches(&path)
        .unwrap()
        .into_iter()
        .find(|b| !b.is_remote && b.name != "feature/x")
        .expect("default branch");
    engine.checkout_branch(&path, &default_branch.name).unwrap();
    assert!(!path.join("feat.txt").exists());

    let branches = engine.branches(&path).unwrap();
    let names: Vec<_> = branches.iter().map(|b| b.name.as_str()).collect();
    assert!(names.contains(&"feature/x"));
    let head = branches.iter().find(|b| b.is_head).unwrap();
    assert_eq!(head.name, default_branch.name);
}

#[test]
fn checkout_unknown_branch_reports_error() {
    let (_guard, path) = init_repo();
    write(&path, "a.txt", "x\n");
    stage_all_and_commit(&path, "Base");
    let err = Git2Engine
        .checkout_branch(&path, "does-not-exist")
        .unwrap_err();
    assert_eq!(err.code(), "branch_not_found");
}

#[test]
fn fetch_without_remote_is_noop() {
    let (_guard, path) = init_repo();
    // A fetch without a configured remote is a no-op (like `git fetch` itself).
    // The sidecar error code mapping (run_git → "sidecar_failed") is checked
    // crate-internally as a unit test in sidecar.rs — the app/tests no longer see
    // `sidecar` directly (only the Git2Engine surface).
    assert!(Git2Engine.fetch(&path).is_ok());
}

/// A `.git` directory somebody else supplied is executable code as far as git is
/// concerned: `credential.helper` is multi-valued, so the helper terra-git adds
/// with `-c` is APPENDED — a repo-local `helper = "!<command>"` still runs, and
/// runs first. `GIT_TERMINAL_PROMPT=0` does not stop it (it only suppresses the
/// terminal fallback), and `core.askPass` is a second program git starts. Neither
/// needs a compromised renderer: opening the repo and clicking Fetch is enough.
///
/// Every remote operation therefore has to refuse BEFORE it starts git.
#[test]
fn repo_local_credential_hooks_block_remote_ops() {
    for key in ["credential.helper", "core.askPass"] {
        let (_guard, path) = init_repo();
        git_raw(
            &path,
            &["remote", "add", "origin", "https://example.invalid/r.git"],
        )
        .unwrap();
        git_raw(&path, &["config", "--local", key, "!echo pwned"]).unwrap();

        for (what, err) in [
            ("fetch", Git2Engine.fetch(&path).unwrap_err()),
            ("pull", Git2Engine.pull(&path).unwrap_err()),
            ("push", Git2Engine.push(&path).unwrap_err()),
        ] {
            assert_eq!(
                err.code(),
                "local_credential_hook",
                "{what} must refuse a repo-local {key}, got: {err}"
            );
        }

        // Without the repo-local entry the same operations run again (a fetch
        // against an unreachable remote fails for a NETWORK reason, never with
        // this guard).
        git_raw(&path, &["config", "--local", "--unset", key]).unwrap();
        let err = Git2Engine.fetch(&path).unwrap_err();
        assert_ne!(err.code(), "local_credential_hook");
    }
}

/// Regression test for the merge-state bug (found in review): a commit during a
/// running merge has to have TWO parents (HEAD + MERGE_HEAD) and clean up the
/// merge state — otherwise the remote history is not recorded as merged and the
/// repo stays "in a merge" as far as git is concerned.
#[test]
fn commit_in_merge_state_creates_merge_commit() {
    let (_guard, path) = init_repo();
    let engine = Git2Engine;

    // Base on main
    write(&path, "file.txt", "base\n");
    stage_all_and_commit(&path, "Base");

    // Branch "side" changes the same line as main -> a conflict is guaranteed
    engine.create_branch(&path, "side", true).unwrap();
    write(&path, "file.txt", "from side\n");
    let side_id = stage_all_and_commit(&path, "Change on side");

    // Back on the default branch, a competing change there
    let default_branch = engine
        .branches(&path)
        .unwrap()
        .into_iter()
        .find(|b| !b.is_remote && b.name != "side")
        .unwrap();
    engine.checkout_branch(&path, &default_branch.name).unwrap();
    write(&path, "file.txt", "from main\n");
    stage_all_and_commit(&path, "Change on main");

    // Produce a merge with a conflict (sets MERGE_HEAD)
    let merge_result = git_raw(&path, &["merge", "side"]);
    assert!(merge_result.is_err(), "the merge has to conflict");

    // "Resolve" the conflict and stage it
    write(&path, "file.txt", "resolved\n");
    engine.stage(&path, &["file.txt".into()]).unwrap();

    // The commit has to be a merge commit with 2 parents
    let id = engine.commit(&path, "Merge side", false).unwrap();
    let log = engine.log(&path, 0, 1).unwrap();
    assert_eq!(log[0].id, id);
    assert_eq!(log[0].parent_ids.len(), 2, "a merge commit needs 2 parents");
    assert!(log[0].parent_ids.contains(&side_id));

    // The merge state has to be cleaned up
    let state = git_raw(&path, &["status", "--porcelain"]).unwrap();
    assert!(
        !path.join(".git/MERGE_HEAD").exists(),
        "MERGE_HEAD has to be gone"
    );
    assert!(
        state.is_empty(),
        "the workdir has to be clean, was: {state}"
    );
}

/// Regression tests for the fnmatch pathspec bug (found in review):
/// a file named `a[b].txt` must NEVER be interpreted as a glob pattern on
/// discard/unstage/diff and hit `ab.txt` (data loss!).
#[test]
fn special_character_file_names_are_not_globs() {
    let (_guard, path) = init_repo();
    let engine = Git2Engine;

    write(&path, "ab.txt", "ab original\n");
    write(&path, "a[b].txt", "bracket original\n");
    stage_all_and_commit(&path, "Base");

    // --- discard: discard only a[b].txt, ab.txt has to stay untouched ---
    write(&path, "ab.txt", "ab CHANGED\n");
    write(&path, "a[b].txt", "bracket CHANGED\n");
    engine.discard(&path, &["a[b].txt".into()]).unwrap();
    assert_eq!(
        fs::read_to_string(path.join("ab.txt")).unwrap(),
        "ab CHANGED\n",
        "discarding a[b].txt must not reset ab.txt"
    );
    assert_eq!(
        fs::read_to_string(path.join("a[b].txt")).unwrap(),
        "bracket original\n"
    );

    // --- unstage: unstage only a[b].txt, ab.txt stays staged ---
    engine
        .stage(&path, &["ab.txt".into(), "a[b].txt".into()])
        .unwrap();
    write(&path, "a[b].txt", "bracket CHANGED\n");
    engine.stage(&path, &["a[b].txt".into()]).unwrap();
    engine.unstage(&path, &["a[b].txt".into()]).unwrap();
    let status = engine.status(&path).unwrap();
    let staged: Vec<&str> = status.staged.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(
        staged,
        vec!["ab.txt"],
        "only ab.txt may stay staged, a[b].txt has to be unstaged"
    );

    // --- diff: the diff of ab.txt must not be falsified by the a[b].txt pathspec ---
    let diff = engine.file_diff(&path, "a[b].txt", false).unwrap().unwrap();
    assert_eq!(diff.path, "a[b].txt");
}

#[test]
fn unicode_and_whitespace_paths() {
    let (_guard, path) = init_repo();
    let engine = Git2Engine;

    // Umlaut, CJK, an emoji directory, spaces — all of them have to pass cleanly
    // through status/stage/commit/diff/discard (path roundtrip as UTF-8).
    let files = [
        "änderung.txt",
        "日本語.txt",
        "emoji 📁/file.txt",
        "with spaces.txt",
    ];
    for f in files {
        write(&path, f, "one\n");
    }
    // status lists them all as untracked (paths unchanged).
    let st = engine.status(&path).unwrap();
    let untracked: std::collections::HashSet<&str> =
        st.unstaged.iter().map(|e| e.path.as_str()).collect();
    for f in files {
        assert!(
            untracked.contains(f),
            "{f} missing in the status: {untracked:?}"
        );
    }

    stage_all_and_commit(&path, "Unicode base");
    assert!(engine.status(&path).unwrap().unstaged.is_empty());

    // Modify + diff + a targeted discard on a Unicode path.
    write(&path, "änderung.txt", "one\ntwo\n");
    write(&path, "日本語.txt", "one\nCHANGED\n");
    let diff = engine
        .file_diff(&path, "änderung.txt", false)
        .unwrap()
        .unwrap();
    assert_eq!(diff.path, "änderung.txt");
    assert!(diff.hunks[0]
        .lines
        .iter()
        .any(|l| l.content == "two" && l.kind == LineKind::Addition));

    engine.discard(&path, &["änderung.txt".into()]).unwrap();
    assert_eq!(
        fs::read_to_string(path.join("änderung.txt")).unwrap(),
        "one\n",
        "discarding the umlaut path has to reset it"
    );
    // The other Unicode path stays untouched.
    assert_eq!(
        fs::read_to_string(path.join("日本語.txt")).unwrap(),
        "one\nCHANGED\n"
    );
}

#[cfg(unix)]
#[test]
fn exec_bit_survives_staging() {
    use std::os::unix::fs::PermissionsExt;
    let (_guard, path) = init_repo();

    write(&path, "script.sh", "#!/bin/sh\necho hi\n");
    let script = path.join("script.sh");
    let mut perm = fs::metadata(&script).unwrap().permissions();
    perm.set_mode(0o755);
    fs::set_permissions(&script, perm).unwrap();
    stage_all_and_commit(&path, "script");

    let read_mode = || {
        let repo = git2::Repository::open(&path).unwrap();
        let tree = repo.head().unwrap().peel_to_tree().unwrap();
        // The entry is bound on purpose: as part of the tail expression the
        // TreeEntry temporary would outlive `tree` and `repo` and still borrow
        // them when its Drop runs, which does not compile.
        let entry = tree.get_name("script.sh").unwrap();
        entry.filemode()
    };
    assert_eq!(read_mode(), 0o100755, "the exec bit has to be committed");

    // Change the content (fs::write keeps the permissions) and commit again.
    write(&path, "script.sh", "#!/bin/sh\necho hello\n");
    stage_all_and_commit(&path, "script v2");
    assert_eq!(
        read_mode(),
        0o100755,
        "the exec bit must not get lost on staging"
    );
}

#[test]
fn rename_is_detected() {
    let (_guard, path) = init_repo();
    let engine = Git2Engine;

    write(
        &path,
        "old.txt",
        "same content line 1\nsame content line 2\n",
    );
    stage_all_and_commit(&path, "Base");

    fs::rename(path.join("old.txt"), path.join("new.txt")).unwrap();
    engine
        .stage(&path, &["old.txt".into(), "new.txt".into()])
        .unwrap();

    let status = engine.status(&path).unwrap();
    assert_eq!(
        status.staged.len(),
        1,
        "the rename should appear as ONE entry"
    );
    assert_eq!(status.staged[0].kind, ChangeKind::Renamed);
    assert_eq!(status.staged[0].path, "new.txt");
    assert_eq!(status.staged[0].orig_path.as_deref(), Some("old.txt"));
}

/// Content of the rename fixtures: identical before/after the rename so the
/// rename detection (100% similarity) fires reliably.
const RENAME_CONTENT: &str = "same content line 1\nsame content line 2\n";

/// Fixture for the rename flows: old.txt committed, then renamed to new.txt in
/// the worktree (nothing staged yet).
fn rename_fixture() -> (tempfile::TempDir, PathBuf) {
    let (guard, path) = init_repo();
    write(&path, "old.txt", RENAME_CONTENT);
    stage_all_and_commit(&path, "Base");
    fs::rename(path.join("old.txt"), path.join("new.txt")).unwrap();
    (guard, path)
}

/// Regression test F5 (stage): the status returns a workdir rename as ONE entry
/// (new path + orig_path), and the frontend only passes the new path.
/// stage(new) has to stage the removal of the old path along with it —
/// otherwise it stays behind as an unstaged deletion.
#[test]
fn stage_workdir_rename_stages_both_sides() {
    let (_guard, path) = rename_fixture();
    let engine = Git2Engine;

    // Precondition: the workdir rename is ONE entry with an orig_path.
    let st = engine.status(&path).unwrap();
    assert_eq!(st.unstaged.len(), 1);
    assert_eq!(st.unstaged[0].kind, ChangeKind::Renamed);
    assert_eq!(st.unstaged[0].path, "new.txt");
    assert_eq!(st.unstaged[0].orig_path.as_deref(), Some("old.txt"));

    engine.stage(&path, &["new.txt".into()]).unwrap();
    let st = engine.status(&path).unwrap();
    assert!(
        st.unstaged.is_empty(),
        "the old side must not stay behind unstaged: {:?}",
        st.unstaged
    );
    assert_eq!(st.staged.len(), 1, "staged: {:?}", st.staged);
    assert_eq!(st.staged[0].kind, ChangeKind::Renamed);
    assert_eq!(st.staged[0].orig_path.as_deref(), Some("old.txt"));
}

/// Regression test F5 (unstage): unstage(new) of a staged rename also has to
/// take back the staged deletion of the old path — otherwise a commit would
/// delete old.txt without adding new.txt.
#[test]
fn unstage_rename_restores_old_path_in_index() {
    let (_guard, path) = rename_fixture();
    let engine = Git2Engine;
    engine
        .stage(&path, &["old.txt".into(), "new.txt".into()])
        .unwrap();
    // Precondition as in rename_is_detected: ONE staged rename entry.
    assert_eq!(
        engine.status(&path).unwrap().staged[0].kind,
        ChangeKind::Renamed
    );

    engine.unstage(&path, &["new.txt".into()]).unwrap();
    let st = engine.status(&path).unwrap();
    assert!(
        st.staged.is_empty(),
        "no half rename may stay staged: {:?}",
        st.staged
    );
    // The worktree is unchanged — the rename now sits as ONE workdir rename entry
    // in the unstaged area.
    assert_eq!(st.unstaged.len(), 1, "unstaged: {:?}", st.unstaged);
    assert_eq!(st.unstaged[0].kind, ChangeKind::Renamed);
    assert_eq!(st.unstaged[0].path, "new.txt");
    assert_eq!(st.unstaged[0].orig_path.as_deref(), Some("old.txt"));
    assert_eq!(
        fs::read_to_string(path.join("new.txt")).unwrap(),
        RENAME_CONTENT
    );
}

/// Regression test F5 (discard): discard(new) of a workdir rename deletes the
/// new file AND restores the old path — before, the content disappeared from the
/// worktree entirely (data loss).
#[test]
fn discard_workdir_rename_restores_old_file() {
    let (_guard, path) = rename_fixture();
    let engine = Git2Engine;

    engine.discard(&path, &["new.txt".into()]).unwrap();
    assert!(
        !path.join("new.txt").exists(),
        "the new file has to be deleted"
    );
    assert_eq!(
        fs::read_to_string(path.join("old.txt")).unwrap(),
        RENAME_CONTENT,
        "the old path has to be restored"
    );
    let st = engine.status(&path).unwrap();
    assert!(st.staged.is_empty() && st.unstaged.is_empty());
}

/// Regression test F6: a commit during a cherry-pick has to clean up the
/// cherry-pick state (CHERRY_PICK_HEAD/.git/sequencer) — otherwise "continue"
/// fails afterwards and "abort" is refused.
#[test]
fn commit_after_cherry_pick_conflict_cleans_up_state() {
    let (_guard, path) = init_repo();
    let engine = Git2Engine;

    write(&path, "file.txt", "base\n");
    stage_all_and_commit(&path, "Base");

    engine.create_branch(&path, "side", true).unwrap();
    write(&path, "file.txt", "from side\n");
    let side_id = stage_all_and_commit(&path, "Change on side");

    let default_branch = engine
        .branches(&path)
        .unwrap()
        .into_iter()
        .find(|b| !b.is_remote && b.name != "side")
        .unwrap();
    engine.checkout_branch(&path, &default_branch.name).unwrap();
    write(&path, "file.txt", "from main\n");
    stage_all_and_commit(&path, "Change on main");

    // Produce a cherry-pick with a conflict (sets CHERRY_PICK_HEAD).
    let result = git_raw(&path, &["cherry-pick", side_id.as_str()]);
    assert!(result.is_err(), "the cherry-pick has to conflict");
    assert!(path.join(".git/CHERRY_PICK_HEAD").exists());

    // Resolve the conflict, stage it, commit.
    write(&path, "file.txt", "resolved\n");
    engine.stage(&path, &["file.txt".into()]).unwrap();
    let id = engine.commit(&path, "Cherry-pick resolved", false).unwrap();

    // As before, a single-parent commit (not a merge commit).
    let log = engine.log(&path, 0, 1).unwrap();
    assert_eq!(log[0].id, id);
    assert_eq!(
        log[0].parent_ids.len(),
        1,
        "the cherry-pick commit has ONE parent"
    );

    // The state has to be cleaned up.
    let repo = git2::Repository::open(&path).unwrap();
    assert_eq!(
        repo.state(),
        git2::RepositoryState::Clean,
        "the repo state has to be Clean"
    );
    assert!(
        !path.join(".git/CHERRY_PICK_HEAD").exists(),
        "CHERRY_PICK_HEAD has to be gone"
    );
    assert!(
        !path.join(".git/sequencer").exists(),
        ".git/sequencer has to be gone"
    );
}

/// Regression test F6 (the revert branch of the same fix): a commit during a
/// revert has to clean up REVERT_HEAD/.git/sequencer.
#[test]
fn commit_after_revert_conflict_cleans_up_state() {
    let (_guard, path) = init_repo();
    let engine = Git2Engine;

    write(&path, "a.txt", "one\n");
    stage_all_and_commit(&path, "One");
    write(&path, "a.txt", "two\n");
    let two_id = stage_all_and_commit(&path, "Two");
    write(&path, "a.txt", "three\n");
    stage_all_and_commit(&path, "Three");

    // Reverting "Two" conflicts (the file is at "three" by now).
    let result = git_raw(&path, &["revert", "--no-edit", two_id.as_str()]);
    assert!(result.is_err(), "the revert has to conflict");
    assert!(path.join(".git/REVERT_HEAD").exists());

    write(&path, "a.txt", "resolved\n");
    engine.stage(&path, &["a.txt".into()]).unwrap();
    engine.commit(&path, "Revert resolved", false).unwrap();

    let repo = git2::Repository::open(&path).unwrap();
    assert_eq!(repo.state(), git2::RepositoryState::Clean);
    assert!(!path.join(".git/REVERT_HEAD").exists());
    assert!(!path.join(".git/sequencer").exists());
}

#[test]
fn log_all_covers_all_branches_and_tags() {
    let (_guard, path) = init_repo();
    let engine = Git2Engine;
    write(
        &path, "base.txt", "base
",
    );
    stage_all_and_commit(&path, "Base");

    // A feature branch with its own commit; HEAD back on the default branch.
    git_raw(&path, &["checkout", "-b", "feature"]).unwrap();
    write(
        &path, "feat.txt", "feature
",
    );
    let feature_tip = stage_all_and_commit(&path, "Feature commit");
    git_raw(&path, &["checkout", "-"]).unwrap();
    // There is no tag on an orphan history here — a normal tag is enough to cover
    // the ref family in the log call.
    git_raw(&path, &["tag", "v-test"]).unwrap();

    // The HEAD log does NOT know the feature commit …
    let head_log = engine.log(&path, 0, 10).unwrap();
    assert!(
        !head_log.iter().any(|c| c.id == feature_tip),
        "the HEAD log must not contain foreign branch tips"
    );
    // … the whole graph does.
    let all = engine.log_all(&path, 0, 10).unwrap();
    assert!(
        all.iter().any(|c| c.id == feature_tip),
        "log_all has to contain the feature tip: {all:?}"
    );
    assert_eq!(all.len(), 2, "Base + Feature, no duplicates: {all:?}");

    // Paging stays deterministic: page 2 starts after page 1.
    let page1 = engine.log_all(&path, 0, 1).unwrap();
    let page2 = engine.log_all(&path, 1, 1).unwrap();
    assert_ne!(page1[0].id, page2[0].id);

    // A fresh repo without commits: an empty page instead of an error.
    let (_g2, empty) = init_repo();
    assert!(engine.log_all(&empty, 0, 10).unwrap().is_empty());
}

/// Regression test: if `.gitattributes` sets a
/// `conflict-marker-size`, git writes its markers at EXACTLY that length.
/// The parser had 7 hard-wired and considered such files marker-less — the
/// workshop reported them as probably already resolved while git listed them as
/// conflicted. Whoever followed that committed the markers along.
///
/// The length comes from the opening line, NOT from the attribute — why, is
/// shown by the test below.
#[test]
fn read_conflict_detects_differing_marker_length() {
    let (_guard, path) = init_repo();
    let engine = Git2Engine;

    write(&path, ".gitattributes", "*.md conflict-marker-size=12\n");
    write(&path, "docs/a.md", "base\n");
    stage_all_and_commit(&path, "Base");

    engine.create_branch(&path, "side", true).unwrap();
    write(&path, "docs/a.md", "from side\n");
    stage_all_and_commit(&path, "side");

    let default_branch = engine
        .branches(&path)
        .unwrap()
        .into_iter()
        .find(|b| !b.is_remote && b.name != "side")
        .unwrap();
    engine.checkout_branch(&path, &default_branch.name).unwrap();
    write(&path, "docs/a.md", "from main\n");
    stage_all_and_commit(&path, "main");

    // Produce the conflict — git now writes 12-character markers.
    let _ = git_raw(&path, &["merge", "side"]);
    let raw = fs::read_to_string(path.join("docs/a.md")).unwrap();
    assert!(
        raw.contains("<<<<<<<<<<<<"),
        "git has to write 12-character markers, wrote: {raw}"
    );

    let f = engine.read_conflict(&path, "docs/a.md").unwrap();
    assert!(
        f.has_conflicts,
        "the conflict has to be detected, segments: {:?}",
        f.segments
    );
    let conflict = f
        .segments
        .iter()
        .find(|s| s.kind == "conflict")
        .expect("a conflict segment");
    assert_eq!(conflict.ours, vec!["from main"]);
    assert_eq!(conflict.theirs, vec!["from side"]);
}

/// Regression test: if a conflicted cherry-pick
/// is finished with the normal commit button, the ORIGINAL AUTHOR has to be
/// preserved — exactly as `git commit` does it. Before, commit() set our own
/// signature as author AND committer and then cleared CHERRY_PICK_HEAD: the
/// foreign authorship was gone and its source deleted.
#[test]
fn commit_after_cherry_pick_conflict_keeps_original_author() {
    let (_guard, path) = init_repo();
    let engine = Git2Engine;

    write(&path, "file.txt", "base\n");
    stage_all_and_commit(&path, "Base");

    // A commit by SOMEONE ELSE on a side branch.
    engine.create_branch(&path, "side", true).unwrap();
    write(&path, "file.txt", "from the colleague\n");
    engine.stage(&path, &["file.txt".into()]).unwrap();
    git_raw(
        &path,
        &[
            "-c",
            "user.name=Colleague",
            "-c",
            "user.email=colleague@foreign.example",
            "commit",
            "-m",
            "Change by the colleague",
        ],
    )
    .unwrap();
    let foreign_id = engine.log(&path, 0, 1).unwrap()[0].id.clone();

    let default_branch = engine
        .branches(&path)
        .unwrap()
        .into_iter()
        .find(|b| !b.is_remote && b.name != "side")
        .unwrap();
    engine.checkout_branch(&path, &default_branch.name).unwrap();
    write(&path, "file.txt", "from main\n");
    stage_all_and_commit(&path, "Change on main");

    // Cherry-pick with a conflict (sets CHERRY_PICK_HEAD).
    assert!(git_raw(&path, &["cherry-pick", foreign_id.as_str()]).is_err());

    // Resolve the conflict, stage it, finish through the normal commit path.
    write(&path, "file.txt", "resolved\n");
    engine.stage(&path, &["file.txt".into()]).unwrap();
    let id = engine.commit(&path, "Cherry-pick resolved", false).unwrap();

    let repo = git2::Repository::open(&path).unwrap();
    let commit = repo.find_commit(git2::Oid::from_str(&id).unwrap()).unwrap();
    assert_eq!(
        commit.author().email().unwrap(),
        "colleague@foreign.example",
        "the author has to stay the original author"
    );
    assert_eq!(commit.author().name().unwrap(), "Colleague");
    // On a cherry-pick git also carries over the author DATE (verified
    // empirically) — only the commit time is new.
    let original = repo
        .find_commit(git2::Oid::from_str(&foreign_id).unwrap())
        .unwrap();
    assert_eq!(
        commit.author().when().seconds(),
        original.author().when().seconds(),
        "the author date has to carry over"
    );
    assert_eq!(
        commit.committer().email().unwrap(),
        "terra@test.local",
        "the committer is whoever finishes the cherry-pick"
    );
    // The cleanup from F6 stays.
    assert_eq!(repo.state(), git2::RepositoryState::Clean);
}

/// Counter-check to D11: a revert creates a NEW commit — its author is whoever
/// reverts, not the author of the reverted commit. Exactly like git.
#[test]
fn commit_after_revert_conflict_keeps_own_author() {
    let (_guard, path) = init_repo();
    let engine = Git2Engine;

    write(&path, "file.txt", "base\n");
    stage_all_and_commit(&path, "Base");
    write(&path, "file.txt", "from the colleague\n");
    engine.stage(&path, &["file.txt".into()]).unwrap();
    git_raw(
        &path,
        &[
            "-c",
            "user.name=Colleague",
            "-c",
            "user.email=colleague@foreign.example",
            "commit",
            "-m",
            "Change by the colleague",
        ],
    )
    .unwrap();
    let foreign_id = engine.log(&path, 0, 1).unwrap()[0].id.clone();

    // Then something that makes the revert collide.
    write(&path, "file.txt", "changed later\n");
    stage_all_and_commit(&path, "later");

    assert!(git_raw(&path, &["revert", "--no-edit", foreign_id.as_str()]).is_err());

    write(&path, "file.txt", "resolved\n");
    engine.stage(&path, &["file.txt".into()]).unwrap();
    let id = engine.commit(&path, "Revert resolved", false).unwrap();

    let repo = git2::Repository::open(&path).unwrap();
    let commit = repo.find_commit(git2::Oid::from_str(&id).unwrap()).unwrap();
    assert_eq!(
        commit.author().email().unwrap(),
        "terra@test.local",
        "the revert is attributed to whoever makes it"
    );
    assert_eq!(repo.state(), git2::RepositoryState::Clean);
}

/// Regression test B1 (adversarial counter-check 2026-08-17): the proof that the
/// marker length must NOT come from `conflict-marker-size`.
///
/// The classic way the attribute enters a repo: a branch BRINGS it along. git
/// has then merged the file with the old length (7) — the attribute did not
/// apply at merge time — while `check-attr` says 12 afterwards. A parser that
/// believes the attribute is blind here; one that reads the opening line is not.
#[test]
fn read_conflict_reads_length_from_file_not_from_attribute() {
    let (_guard, path) = init_repo();
    let engine = Git2Engine;

    // main does NOT know the attribute.
    write(&path, "docs/a.md", "base\n");
    stage_all_and_commit(&path, "Base");

    // The side branch introduces it AND changes the same file.
    engine.create_branch(&path, "side", true).unwrap();
    write(&path, ".gitattributes", "*.md conflict-marker-size=12\n");
    write(&path, "docs/a.md", "from side\n");
    stage_all_and_commit(&path, "side with attribute");

    let default_branch = engine
        .branches(&path)
        .unwrap()
        .into_iter()
        .find(|b| !b.is_remote && b.name != "side")
        .unwrap();
    engine.checkout_branch(&path, &default_branch.name).unwrap();
    write(&path, "docs/a.md", "from main\n");
    stage_all_and_commit(&path, "main");

    let _ = git_raw(&path, &["merge", "side"]);

    // git merged with SEVEN (the attribute did not apply then) …
    let raw = fs::read_to_string(path.join("docs/a.md")).unwrap();
    assert!(
        raw.contains("<<<<<<< HEAD"),
        "git has to write 7-character markers here, wrote: {raw}"
    );
    // … while the file now prescribes twelve.
    let attr = git_raw(
        &path,
        &["check-attr", "conflict-marker-size", "--", "docs/a.md"],
    )
    .unwrap();
    assert!(attr.contains("12"), "check-attr says: {attr}");

    // The parser must not let that mislead it.
    let f = engine.read_conflict(&path, "docs/a.md").unwrap();
    assert!(
        f.has_conflicts,
        "the conflict has to be detected, segments: {:?}",
        f.segments
    );
    let conflict = f
        .segments
        .iter()
        .find(|s| s.kind == "conflict")
        .expect("a conflict segment");
    assert_eq!(conflict.ours, vec!["from main"]);
    assert_eq!(conflict.theirs, vec!["from side"]);
}

/// Regression test B2 (adversarial counter-check 2026-08-17): an amend during a
/// running multi-step operation has to be rejected.
///
/// The amend guard only knew about merge. During a cherry-pick it let things
/// through — and amend() then replaces OUR OWN predecessor commit with the
/// cherry-pick result: a foreign change in our own commit, our own commit gone,
/// and the cherry-pick state left standing. Real git refuses this explicitly
/// (fatal: You are in the middle of a cherry-pick -- cannot amend), and the
/// sidecar path passes exactly that error through — only the git2 path did it
/// silently.
#[test]
fn amend_during_cherry_pick_is_rejected() {
    let (_guard, path) = init_repo();
    let engine = Git2Engine;

    write(&path, "file.txt", "base\n");
    stage_all_and_commit(&path, "Base");

    engine.create_branch(&path, "side", true).unwrap();
    write(&path, "file.txt", "from side\n");
    let side_id = stage_all_and_commit(&path, "Change on side");

    let default_branch = engine
        .branches(&path)
        .unwrap()
        .into_iter()
        .find(|b| !b.is_remote && b.name != "side")
        .unwrap();
    engine.checkout_branch(&path, &default_branch.name).unwrap();
    write(&path, "file.txt", "from main\n");
    let own_id = stage_all_and_commit(&path, "My own commit");

    assert!(git_raw(&path, &["cherry-pick", side_id.as_str()]).is_err());

    write(&path, "file.txt", "resolved\n");
    engine.stage(&path, &["file.txt".into()]).unwrap();

    let err = engine
        .commit(&path, "accidental amend", true)
        .expect_err("an amend during a cherry-pick has to be rejected");
    assert!(
        format!("{err}").to_lowercase().contains("amend"),
        "the message should name the reason: {err}"
    );

    // Our own commit has to stand untouched at HEAD.
    let log = engine.log(&path, 0, 1).unwrap();
    assert_eq!(log[0].id, own_id, "HEAD must not be rewritten");

    // And the cherry-pick can still be finished regularly.
    let id = engine.commit(&path, "Cherry-pick resolved", false).unwrap();
    assert_ne!(id, own_id);
    let repo = git2::Repository::open(&path).unwrap();
    assert_eq!(repo.state(), git2::RepositoryState::Clean);
}

/// Counter-check to B2: without a running operation an amend obviously stays
/// allowed — the guard must not take the normal case with it.
#[test]
fn amend_without_running_operation_stays_allowed() {
    let (_guard, path) = init_repo();
    let engine = Git2Engine;

    write(&path, "file.txt", "base\n");
    stage_all_and_commit(&path, "Base");
    write(&path, "file.txt", "addendum\n");
    engine.stage(&path, &["file.txt".into()]).unwrap();

    let id = engine.commit(&path, "Base, corrected", true).unwrap();
    let log = engine.log(&path, 0, 2).unwrap();
    assert_eq!(log[0].id, id);
    assert_eq!(log.len(), 1, "amend replaces instead of appending");
}
