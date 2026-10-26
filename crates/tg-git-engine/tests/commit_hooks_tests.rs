//! Commits run hooks and report signing failures.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tg_git_engine::prelude::*;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn repo_with_staged_file() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_path_buf();
    git(&path, &["init", "-q", "-b", "main"]);
    for (k, v) in [
        ("user.name", "Terra Tester"),
        ("user.email", "terra@test.local"),
        ("commit.gpgsign", "false"),
        ("core.autocrlf", "false"),
    ] {
        git(&path, &["config", k, v]);
    }
    fs::write(path.join("a.txt"), "a\n").unwrap();
    git(&path, &["add", "a.txt"]);
    (dir, path)
}

fn write_hook(repo: &Path, name: &str, script: &str) {
    let hook = repo.join(".git").join("hooks").join(name);
    fs::write(&hook, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

fn head_count(path: &Path) -> usize {
    Command::new("git")
        .args(["rev-list", "--count", "HEAD"])
        .current_dir(path)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map_or(0, |o| {
            String::from_utf8_lossy(&o.stdout)
                .trim()
                .parse()
                .unwrap_or(0)
        })
}

#[test]
fn a_failing_pre_commit_hook_stops_the_commit_with_its_output() {
    let (_dir, path) = repo_with_staged_file();
    write_hook(
        &path,
        "pre-commit",
        "#!/bin/sh\necho 'lint: 3 problems' >&2\nexit 1\n",
    );

    let err = Git2Engine.commit(&path, "first", false).unwrap_err();

    assert_eq!(err.code(), "hook_failed");
    assert!(
        err.detail().unwrap_or("").contains("lint: 3 problems"),
        "{err:?}"
    );
    assert_eq!(head_count(&path), 0, "nothing was committed");
}

#[test]
fn a_passing_hook_runs_and_the_commit_lands() {
    let (_dir, path) = repo_with_staged_file();
    write_hook(
        &path,
        "commit-msg",
        "#!/bin/sh\necho ran > .git/hook-ran\nexit 0\n",
    );

    Git2Engine.commit(&path, "first", false).unwrap();

    assert!(
        path.join(".git").join("hook-ran").exists(),
        "libgit2 would skip it"
    );
    assert_eq!(head_count(&path), 1);
}

#[test]
fn a_failed_signature_is_named_and_can_be_skipped_once() {
    let (_dir, path) = repo_with_staged_file();
    git(&path, &["config", "commit.gpgsign", "true"]);
    git(&path, &["config", "gpg.program", "terra-git-no-such-gpg"]);

    let err = Git2Engine.commit(&path, "first", false).unwrap_err();
    assert_eq!(err.code(), "signing_failed");

    Git2Engine.commit_with(&path, "first", false, true).unwrap();
    assert_eq!(head_count(&path), 1);
    assert_eq!(
        git(&path, &["config", "commit.gpgsign"]).trim(),
        "true",
        "the setting itself stays"
    );
}

/// A clone of a bare origin with one pushed commit and one local commit.
fn clone_with_local_commit() -> (tempfile::TempDir, PathBuf) {
    let (dir, seed) = repo_with_staged_file();
    git(&seed, &["commit", "-q", "-m", "base"]);
    let origin = dir.path().join("origin.git");
    git(
        dir.path(),
        &[
            "clone",
            "-q",
            "--bare",
            &seed.to_string_lossy(),
            &origin.to_string_lossy(),
        ],
    );
    let clone = dir.path().join("clone");
    git(
        dir.path(),
        &[
            "clone",
            "-q",
            &origin.to_string_lossy(),
            &clone.to_string_lossy(),
        ],
    );
    for (k, v) in [
        ("user.name", "Terra Tester"),
        ("user.email", "terra@test.local"),
        ("commit.gpgsign", "false"),
    ] {
        git(&clone, &["config", k, v]);
    }
    fs::write(clone.join("b.txt"), "b\n").unwrap();
    git(&clone, &["add", "b.txt"]);
    git(&clone, &["commit", "-q", "-m", "local"]);
    (dir, clone)
}

#[test]
fn a_refusing_pre_push_hook_is_named_with_its_output() {
    let (_dir, clone) = clone_with_local_commit();
    write_hook(
        &clone,
        "pre-push",
        "#!/bin/sh\necho 'tests failed: 2' >&2\nexit 1\n",
    );

    let err = Git2Engine
        .push_with_progress(&clone, &tg_git_engine::CancelToken::new(), &mut |_| {})
        .unwrap_err();

    assert_eq!(err.code(), "hook_failed");
    assert!(
        err.detail().unwrap_or("").contains("tests failed: 2"),
        "{err:?}"
    );
}

#[test]
fn a_passing_pre_push_hook_lets_the_push_through() {
    let (_dir, clone) = clone_with_local_commit();
    write_hook(&clone, "pre-push", "#!/bin/sh\nexit 0\n");

    Git2Engine
        .push_with_progress(&clone, &tg_git_engine::CancelToken::new(), &mut |_| {})
        .unwrap();
    assert_eq!(
        git(&clone, &["status", "-sb"]).lines().next().unwrap_or(""),
        "## main...origin/main"
    );
}
