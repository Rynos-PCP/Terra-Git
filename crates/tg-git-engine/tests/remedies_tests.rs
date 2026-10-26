//! Ways out of common errors, against real repositories.
//!
//! This binary points `GIT_CONFIG_GLOBAL` at a temporary file once, so the
//! `safe.directory` test never touches the developer's own configuration.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

use tg_git_engine::prelude::*;
use tg_git_engine::remedies;

fn isolated_global_config() {
    // Set exactly once, inside the initialisation: every test calls this
    // first and waits there, so no git runs in this binary before it is set.
    static GLOBAL: OnceLock<tempfile::TempDir> = OnceLock::new();
    GLOBAL.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("gitconfig");
        fs::write(&file, "").unwrap();
        std::env::set_var("GIT_CONFIG_GLOBAL", &file);
        dir
    });
}

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

fn repo_with_commit() -> (tempfile::TempDir, PathBuf) {
    isolated_global_config();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_path_buf();
    git(&path, &["init", "-q", "-b", "main"]);
    for (k, v) in [
        ("user.name", "Terra Tester"),
        ("user.email", "terra@test.local"),
        ("commit.gpgsign", "false"),
    ] {
        git(&path, &["config", k, v]);
    }
    fs::write(path.join("a.txt"), "a\n").unwrap();
    git(&path, &["add", "a.txt"]);
    git(&path, &["commit", "-q", "-m", "first"]);
    (dir, path)
}

#[test]
fn a_leftover_lock_is_removed_a_fresh_one_is_not() {
    let (_dir, path) = repo_with_commit();
    assert!(
        !remedies::remove_index_lock(&path, None).unwrap(),
        "no lock, nothing to do"
    );

    let lock = path.join(".git").join("index.lock");
    fs::write(&lock, "").unwrap();
    let err = remedies::remove_index_lock(&path, None).unwrap_err();
    assert_eq!(
        err.code(),
        "invalid_operation",
        "a fresh lock may be in use"
    );
    assert!(lock.exists());

    let old = SystemTime::now() - Duration::from_secs(60);
    fs::File::options()
        .write(true)
        .open(&lock)
        .unwrap()
        .set_modified(old)
        .unwrap();
    assert!(remedies::remove_index_lock(&path, None).unwrap());
    assert!(!lock.exists());
}

#[test]
fn the_lock_git_named_is_removed_but_only_inside_the_git_directory() {
    let (_dir, path) = repo_with_commit();
    let old = SystemTime::now() - Duration::from_secs(60);
    let make = |p: &Path| {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, "").unwrap();
        fs::File::options()
            .write(true)
            .open(p)
            .unwrap()
            .set_modified(old)
            .unwrap();
    };
    // A submodule's lock lives in .git/modules/<name>.
    let sub_lock = path
        .join(".git")
        .join("modules")
        .join("lib")
        .join("index.lock");
    make(&sub_lock);
    assert!(remedies::remove_index_lock(&path, Some(&sub_lock)).unwrap());
    assert!(!sub_lock.exists());

    // Anything else is refused: another name, or a file outside .git.
    let outside = path.join("index.lock");
    make(&outside);
    assert!(remedies::remove_index_lock(&path, Some(&outside)).is_err());
    let other = path.join(".git").join("config");
    assert!(remedies::remove_index_lock(&path, Some(&other)).is_err());
    assert!(outside.exists() && other.exists());
}

/// The trust tests write the one shared global config file; two `git config
/// --global --add` at the same moment fail on git's config lock.
static GLOBAL_WRITERS: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn trusting_a_folder_adds_it_once() {
    let _serial = GLOBAL_WRITERS.lock().unwrap_or_else(|p| p.into_inner());
    let (_dir, path) = repo_with_commit();
    remedies::trust_directory(&path).unwrap();
    remedies::trust_directory(&path).unwrap();
    let outside = std::env::temp_dir();
    let listed = git(
        &outside,
        &["config", "--global", "--get-all", "safe.directory"],
    );
    let entry = path.to_string_lossy().replace('\\', "/");
    assert_eq!(
        listed.lines().filter(|l| l.trim() == entry).count(),
        1,
        "{listed}"
    );
}

#[test]
fn trusting_takes_the_repository_root_and_never_a_wildcard() {
    let _serial = GLOBAL_WRITERS.lock().unwrap_or_else(|p| p.into_inner());
    let (_dir, path) = repo_with_commit();
    let sub = path.join("sub").join("deeper");
    fs::create_dir_all(&sub).unwrap();
    remedies::trust_directory(&sub).unwrap();
    let outside = std::env::temp_dir();
    let listed = git(
        &outside,
        &["config", "--global", "--get-all", "safe.directory"],
    );
    let root = path.to_string_lossy().replace('\\', "/");
    assert!(listed.lines().any(|l| l.trim() == root), "{listed}");
    assert!(!listed.contains("sub/deeper"), "{listed}");

    for bad in ["*", "relative/dir"] {
        assert!(remedies::trust_directory(Path::new(bad)).is_err(), "{bad}");
    }
    let plain = tempfile::tempdir().unwrap();
    assert!(
        remedies::trust_directory(plain.path()).is_err(),
        "no repository"
    );
}

#[test]
fn unset_upstream_forgets_the_remote_branch() {
    let (_dir, path) = repo_with_commit();
    git(&path, &["config", "branch.main.remote", "origin"]);
    git(&path, &["config", "branch.main.merge", "refs/heads/main"]);
    remedies::unset_upstream(&path).unwrap();
    let out = Command::new("git")
        .args(["config", "branch.main.merge"])
        .current_dir(&path)
        .output()
        .unwrap();
    assert!(!out.status.success(), "the upstream is gone");
}

#[test]
fn unrelated_histories_are_refused_then_merged_on_request() {
    let (_dir, path) = repo_with_commit();
    git(&path, &["switch", "-q", "--orphan", "other"]);
    fs::write(path.join("b.txt"), "b\n").unwrap();
    git(&path, &["add", "b.txt"]);
    git(&path, &["commit", "-q", "-m", "other root"]);
    git(&path, &["switch", "-q", "main"]);

    let engine = Git2Engine;
    let err = engine.merge_branch(&path, "other").unwrap_err();
    assert_eq!(err.code(), "unrelated_histories");

    engine.merge_unrelated(&path, "other").unwrap();
    assert!(path.join("b.txt").exists());
    let parents = git(&path, &["rev-list", "--parents", "-n", "1", "HEAD"]);
    assert_eq!(
        parents.split_whitespace().count(),
        3,
        "a merge commit: {parents}"
    );
}

#[test]
fn a_shallow_clone_gets_its_full_history() {
    let (_src_dir, src) = repo_with_commit();
    for i in 0..3 {
        fs::write(src.join("a.txt"), format!("{i}\n")).unwrap();
        git(&src, &["commit", "-q", "-am", &format!("c{i}")]);
    }
    let dst_dir = tempfile::tempdir().unwrap();
    let dst = dst_dir.path().join("shallow");
    let src_slashes = src.to_string_lossy().replace('\\', "/");
    let url = format!("file:///{}", src_slashes.trim_start_matches('/'));
    git(
        dst_dir.path(),
        &["clone", "-q", "--depth", "1", &url, &dst.to_string_lossy()],
    );
    assert_eq!(
        git(&dst, &["rev-parse", "--is-shallow-repository"]).trim(),
        "true"
    );

    remedies::fetch_unshallow(&dst, &tg_git_engine::CancelToken::new(), &mut |_| {}).unwrap();

    assert_eq!(
        git(&dst, &["rev-parse", "--is-shallow-repository"]).trim(),
        "false"
    );
    assert_eq!(git(&dst, &["rev-list", "--count", "HEAD"]).trim(), "4");
}
