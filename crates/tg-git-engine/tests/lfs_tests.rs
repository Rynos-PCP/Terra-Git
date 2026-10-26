//! Git LFS. These tests need a real `git-lfs`; without one
//! they print a note and pass, so a developer machine without LFS is not
//! blocked — the CI runners install it.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tg_git_engine::prelude::*;

const POINTER_HEADER: &str = "version https://git-lfs.github.com/spec/v1";

fn lfs_available() -> bool {
    Command::new("git")
        .args(["lfs", "version"])
        .output()
        .is_ok_and(|o| o.status.success())
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

/// A repo with `*.bin` tracked by LFS and `.gitattributes` committed.
fn lfs_repo() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().to_path_buf();
    git(&path, &["init", "-q", "-b", "main"]);
    for (k, v) in [
        ("user.name", "Terra Tester"),
        ("user.email", "terra@test.local"),
        ("commit.gpgsign", "false"),
        ("core.autocrlf", "false"),
        ("core.eol", "lf"),
    ] {
        git(&path, &["config", k, v]);
    }
    git(&path, &["lfs", "install", "--local"]);
    git(&path, &["lfs", "track", "*.bin"]);
    git(&path, &["add", ".gitattributes"]);
    git(&path, &["commit", "-q", "-m", "track bin with lfs"]);
    (dir, path)
}

fn blob_at_head(path: &Path, file: &str) -> String {
    git(path, &["cat-file", "-p", &format!("HEAD:{file}")])
}

#[test]
fn staging_an_lfs_file_commits_the_pointer_not_the_content() {
    if !lfs_available() {
        eprintln!("git-lfs not installed — skipped");
        return;
    }
    let (_dir, path) = lfs_repo();
    fs::write(path.join("big.bin"), vec![7u8; 4096]).unwrap();

    let engine = Git2Engine;
    engine.stage(&path, &["big.bin".to_string()]).unwrap();
    engine.commit(&path, "add big.bin", false).unwrap();

    let blob = blob_at_head(&path, "big.bin");
    assert!(
        blob.starts_with(POINTER_HEADER),
        "a pointer is committed, got {} bytes",
        blob.len()
    );
    // And the worktree is clean afterwards — no phantom change on the file.
    let st = engine.status(&path).unwrap();
    assert!(st.unstaged.is_empty() && st.staged.is_empty(), "{st:?}");
}

/// A committed LFS file `big.bin` with the content `A` (4 KiB of 0xAA).
fn lfs_repo_with_file() -> (tempfile::TempDir, PathBuf) {
    let (dir, path) = lfs_repo();
    fs::write(path.join("big.bin"), content_a()).unwrap();
    git(&path, &["add", "big.bin"]);
    git(&path, &["commit", "-q", "-m", "add big.bin"]);
    (dir, path)
}

fn content_a() -> Vec<u8> {
    vec![0xAA; 4096]
}

fn content_b() -> Vec<u8> {
    vec![0xBB; 5000]
}

fn read(path: &Path) -> Vec<u8> {
    fs::read(path.join("big.bin")).unwrap()
}

#[test]
fn switching_branches_writes_the_content_not_the_pointer() {
    if !lfs_available() {
        eprintln!("git-lfs not installed — skipped");
        return;
    }
    let (_dir, path) = lfs_repo_with_file();
    git(&path, &["switch", "-q", "-c", "other"]);
    fs::write(path.join("big.bin"), content_b()).unwrap();
    git(&path, &["commit", "-q", "-am", "change big.bin"]);

    let engine = Git2Engine;
    engine.checkout_branch(&path, "main").unwrap();
    assert_eq!(read(&path), content_a());
    engine.checkout_branch(&path, "other").unwrap();
    assert_eq!(read(&path), content_b());
    let st = engine.status(&path).unwrap();
    assert!(st.unstaged.is_empty() && st.staged.is_empty(), "{st:?}");
}

#[test]
fn discarding_an_lfs_file_brings_back_the_content() {
    if !lfs_available() {
        eprintln!("git-lfs not installed — skipped");
        return;
    }
    let (_dir, path) = lfs_repo_with_file();
    fs::write(path.join("big.bin"), content_b()).unwrap();

    let engine = Git2Engine;
    let st = engine.status(&path).unwrap();
    assert_eq!(st.unstaged.len(), 1, "{st:?}");
    engine
        .discard_protected(&path, &["big.bin".to_string()], 50 * 1024 * 1024)
        .unwrap();

    assert_eq!(read(&path), content_a());
    assert!(engine.status(&path).unwrap().unstaged.is_empty());
}

#[test]
fn stash_round_trip_keeps_the_lfs_content() {
    if !lfs_available() {
        eprintln!("git-lfs not installed — skipped");
        return;
    }
    let (_dir, path) = lfs_repo_with_file();
    fs::write(path.join("big.bin"), content_b()).unwrap();

    let engine = Git2Engine;
    engine.stash_push(&path, "wip", &[]).unwrap();
    assert_eq!(read(&path), content_a());
    let out = engine.stash_pop(&path, 0).unwrap();
    assert!(out.dropped && out.conflicted.is_empty());
    assert_eq!(read(&path), content_b());
}

#[test]
fn hard_reset_writes_the_content_of_the_target() {
    if !lfs_available() {
        eprintln!("git-lfs not installed — skipped");
        return;
    }
    let (_dir, path) = lfs_repo_with_file();
    let first = git(&path, &["rev-parse", "HEAD"]).trim().to_string();
    fs::write(path.join("big.bin"), content_b()).unwrap();
    git(&path, &["commit", "-q", "-am", "change big.bin"]);

    let engine = Git2Engine;
    engine
        .reset_to(&path, &first, tg_domain::ResetMode::Hard, 50 * 1024 * 1024)
        .unwrap();
    assert_eq!(read(&path), content_a());
}

#[test]
fn a_repo_without_lfs_is_not_detected_as_one() {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q"]);
    let repo = git2::Repository::open(dir.path()).unwrap();
    assert!(!tg_git_engine::lfs::uses_lfs(&repo));
    assert!(!tg_git_engine::lfs::declares_filters(&repo));
}

#[test]
fn diffs_of_lfs_files_show_the_content_sizes_not_the_pointer() {
    if !lfs_available() {
        eprintln!("git-lfs not installed — skipped");
        return;
    }
    let (_dir, path) = lfs_repo_with_file();
    fs::write(path.join("big.bin"), content_b()).unwrap();
    fs::write(path.join("new.bin"), vec![1u8; 3000]).unwrap();
    let engine = Git2Engine;

    // Worktree against the index.
    let d = engine.file_diff(&path, "big.bin", false).unwrap().unwrap();
    assert!(d.lfs && d.hunks.is_empty(), "{d:?}");
    assert_eq!((d.old_size, d.new_size), (Some(4096), Some(5000)));

    // A new file not staged yet.
    let d = engine.file_diff(&path, "new.bin", false).unwrap().unwrap();
    assert!(d.lfs, "{d:?}");
    assert_eq!((d.old_size, d.new_size), (None, Some(3000)));

    // Index against HEAD, then the commit itself.
    engine.stage(&path, &["big.bin".to_string()]).unwrap();
    let d = engine.file_diff(&path, "big.bin", true).unwrap().unwrap();
    assert!(d.lfs && d.hunks.is_empty(), "{d:?}");
    assert_eq!((d.old_size, d.new_size), (Some(4096), Some(5000)));
    let id = engine.commit(&path, "change big.bin", false).unwrap();
    let d = engine.commit_diff(&path, &id).unwrap();
    assert_eq!(d.len(), 1);
    assert!(d[0].lfs && d[0].hunks.is_empty(), "{:?}", d[0]);
    assert_eq!((d[0].old_size, d[0].new_size), (Some(4096), Some(5000)));
}

#[test]
fn state_counts_missing_objects_and_pull_fetches_them() {
    if !lfs_available() {
        eprintln!("git-lfs not installed — skipped");
        return;
    }
    let (_src_dir, src) = lfs_repo_with_file();
    let state = tg_git_engine::lfs::state(&src).unwrap();
    assert!(
        state.used && state.installed && state.missing == 0,
        "{state:?}"
    );

    // A clone that skipped the LFS download: the file is only a pointer.
    let dst_dir = tempfile::tempdir().unwrap();
    let dst = dst_dir.path().join("clone");
    let src_slashes = src.to_string_lossy().replace('\\', "/");
    let url = format!("file:///{}", src_slashes.trim_start_matches('/'));
    let out = Command::new("git")
        .args(["clone", "-q", &url, &dst.to_string_lossy()])
        .env("GIT_LFS_SKIP_SMUDGE", "1")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(read(&dst).starts_with(POINTER_HEADER.as_bytes()));
    let state = tg_git_engine::lfs::state(&dst).unwrap();
    assert_eq!(state.missing, 1, "{state:?}");

    let mut phases = Vec::new();
    tg_git_engine::lfs::pull(&dst, &tg_git_engine::CancelToken::new(), &mut |p| {
        phases.push(p.phase)
    })
    .unwrap();
    assert_eq!(read(&dst), content_a());
    assert_eq!(tg_git_engine::lfs::state(&dst).unwrap().missing, 0);
}

#[test]
fn tracking_a_committed_file_moves_it_to_lfs() {
    if !lfs_available() {
        eprintln!("git-lfs not installed — skipped");
        return;
    }
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
    git(&path, &["lfs", "install", "--local"]);
    fs::write(path.join("big.psd"), content_a()).unwrap();
    git(&path, &["add", "big.psd"]);
    git(&path, &["commit", "-q", "-m", "raw psd"]);

    tg_git_engine::lfs::track(&path, "*.psd", "big.psd").unwrap();

    let attrs = fs::read_to_string(path.join(".gitattributes")).unwrap();
    assert!(attrs.contains("*.psd filter=lfs"), "{attrs}");
    let staged = git(&path, &["cat-file", "-p", ":big.psd"]);
    assert!(
        staged.starts_with(POINTER_HEADER),
        "the index holds the pointer"
    );
    let names = git(&path, &["diff", "--cached", "--name-only"]);
    assert!(
        names.contains(".gitattributes") && names.contains("big.psd"),
        "{names}"
    );
    assert_eq!(fs::read(path.join("big.psd")).unwrap(), content_a());

    assert!(tg_git_engine::lfs::track(&path, "--help", "x").is_err());
}
