//! Read-path benchmarks: measure `status` (git2 vs. the
//! system-git fast path) and the first stretch of the history against a large
//! fixture repo. Purpose: hard numbers for the performance budgets and for the
//! decision whether a gix read fast path is needed.
//!
//! Run with:  cargo bench -p tg-git-engine
//! Control the size via env (small by default so `cargo bench` does not run
//! forever in CI/dev):
//!   TG_BENCH_FILES=20000 TG_BENCH_COMMITS=2000 cargo bench -p tg-git-engine

use std::path::{Path, PathBuf};
use std::process::Command;

use criterion::{criterion_group, criterion_main, Criterion};
use tg_git_engine::{Git2Engine, GitEngine};

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .status()
        .expect("git start");
    assert!(status.success(), "git {args:?} failed");
}

/// Builds a fixture repo with `files` tracked files (in folders of 100) and
/// `commits` commits; then leaves a few files changed/untracked behind (a
/// realistic status case). Cached across the bench run.
fn build_fixture(files: usize, commits: usize) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().to_path_buf();
    git(&path, &["init", "-q"]);
    git(&path, &["config", "user.name", "Bench"]);
    git(&path, &["config", "user.email", "bench@test.local"]);
    git(&path, &["config", "commit.gpgsign", "false"]);

    for i in 0..files {
        let sub = path.join(format!("dir-{:03}", i / 100));
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join(format!("f-{i}.txt")), format!("content {i}\n")).unwrap();
    }
    git(&path, &["add", "-A"]);
    git(&path, &["commit", "-q", "-m", "initial"]);

    // Further commits so the first stretch of the history has something to do.
    for c in 1..commits {
        let f = path.join(format!("dir-000/f-{}.txt", c % 100.min(files.max(1))));
        std::fs::write(&f, format!("rev {c}\n")).unwrap();
        git(&path, &["add", "-A"]);
        git(&path, &["commit", "-q", "-m", &format!("commit {c}")]);
    }
    // Write the commit graph as in the production path (after fetch/clone).
    let _ = Command::new("git")
        .arg("-C")
        .arg(&path)
        .args(["commit-graph", "write", "--reachable", "--split"])
        .status();

    // A realistic working state: a few modified + untracked files.
    for i in 0..(files / 20).max(1) {
        let sub = path.join(format!("dir-{:03}", i / 100));
        std::fs::write(sub.join(format!("f-{i}.txt")), format!("changed {i}\n")).unwrap();
        std::fs::write(sub.join(format!("new-{i}.txt")), "untracked\n").unwrap();
    }

    (dir, path)
}

fn bench_read_path(c: &mut Criterion) {
    let files = env_usize("TG_BENCH_FILES", 2_000);
    let commits = env_usize("TG_BENCH_COMMITS", 200);
    eprintln!("Fixture: {files} files, {commits} commits (via TG_BENCH_FILES/TG_BENCH_COMMITS)");

    let (_guard, path) = build_fixture(files, commits);
    let engine = Git2Engine;

    // Sanity: both status paths return the same number of entries.
    let g = engine.status_git2(&path).unwrap();
    let s = engine.status_via_sidecar(&path).unwrap();
    assert_eq!(g.staged.len(), s.staged.len());
    assert_eq!(g.unstaged.len(), s.unstaged.len());
    eprintln!(
        "Status: {} staged / {} unstaged",
        g.staged.len(),
        g.unstaged.len()
    );

    let mut group = c.benchmark_group("read_path");
    group.sample_size(20);

    group.bench_function("status_git2", |b| {
        b.iter(|| engine.status_git2(std::hint::black_box(&path)).unwrap())
    });
    group.bench_function("status_sidecar", |b| {
        b.iter(|| {
            engine
                .status_via_sidecar(std::hint::black_box(&path))
                .unwrap()
        })
    });
    group.bench_function("log_first_100", |b| {
        b.iter(|| engine.log(std::hint::black_box(&path), 0, 100).unwrap())
    });

    group.finish();
}

criterion_group!(benches, bench_read_path);
criterion_main!(benches);
