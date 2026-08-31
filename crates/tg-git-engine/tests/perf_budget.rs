//! Performance budget gate — makes "performance is a gate, not
//! an afterthought" concretely enforceable.
//!
//! Deliberate design: the upper bounds are GENEROUS multiples of the real
//! targets (status target 200 ms → a ceiling of several seconds). A CI runner is
//! 3–5× slower than a developer machine depending on load; a tight absolute
//! budget would therefore constantly raise false alarms. In exchange this gate
//! reliably catches the CATASTROPHIC regressions that actually matter for the
//! GitHub Desktop comparison: hangs (>40 s) and O(n²) blowups. Precise budget
//! tracking (CodSpeed or a stored baseline) is follow-up work; see ROADMAP.md.
//!
//!
//! `#[ignore]`, because creating the fixture costs time. CI calls it explicitly:
//!   cargo test -p tg-git-engine --test perf_budget -- --ignored --nocapture
//! Fixture size via TG_PERF_FILES (small by default for local runs).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use tg_git_engine::prelude::*;
use tg_git_engine::{Git2Engine, GitEngine};

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .status()
        .expect("git start")
        .success();
    assert!(ok, "git {args:?} failed");
}

/// Fixture with `files` tracked files (in folders of 100): an initial commit,
/// then a second commit that changes a HANDFUL of files (the realistic "look at
/// one commit" case for the diff budget), plus a few uncommitted changes for the
/// status budget. The expensive part is creating the files — hence the gate
/// builds exactly ONE fixture.
fn build_fixture(files: usize) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().to_path_buf();
    git(&path, &["init", "-q"]);
    git(&path, &["config", "user.name", "Perf"]);
    git(&path, &["config", "user.email", "perf@test.local"]);
    git(&path, &["config", "commit.gpgsign", "false"]);

    for i in 0..files {
        let sub = path.join(format!("dir-{:03}", i / 100));
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join(format!("f-{i}.txt")), format!("content {i}\n")).unwrap();
    }
    git(&path, &["add", "-A"]);
    git(&path, &["commit", "-q", "-m", "initial"]);

    // Second commit: only a few changed files = the normal diff case.
    for i in 0..20.min(files.max(1)) {
        let sub = path.join(format!("dir-{:03}", i / 100));
        std::fs::write(sub.join(format!("f-{i}.txt")), format!("rev2 {i}\n")).unwrap();
    }
    git(&path, &["add", "-A"]);
    git(&path, &["commit", "-q", "-m", "update"]);

    // Uncommitted working state for the status budget.
    for i in 0..(files / 20).max(1) {
        let sub = path.join(format!("dir-{:03}", i / 100));
        std::fs::write(sub.join(format!("f-{i}.txt")), format!("changed {i}\n")).unwrap();
        std::fs::write(sub.join(format!("new-{i}.txt")), "untracked\n").unwrap();
    }
    (dir, path)
}

/// Warmup (load index/cache), then ONE measured run.
fn measure<T>(f: impl Fn() -> T) -> (T, Duration) {
    let _warm = f();
    let start = Instant::now();
    let out = f();
    (out, start.elapsed())
}

fn assert_budget(label: &str, dur: Duration, ceiling: Duration) {
    println!("[perf] {label}: {dur:?} (catastrophe ceiling {ceiling:?})");
    assert!(
        dur < ceiling,
        "{label} took {dur:?} — over the budget of {ceiling:?}. Hang/O(n²) regression?"
    );
}

/// Hardening report against a REAL (large) repo — measures the app's core reads
/// and PRINTS the times, without budgets (a report, not a gate). Read-only
/// operations; the repo is not modified. Invocation:
///   TG_PERF_REPO=<path> cargo test -p tg-git-engine --test perf_budget \
///     real_repo_report -- --ignored --nocapture
/// Optional: TG_PERF_BLAME=<file relative to the repo> for the blame budget
/// (default MAINTAINERS, if present) and TG_PERF_DEEP_SKIP (default 100000) for
/// the depth of the log page.
#[test]
#[ignore = "hardening report: run explicitly via TG_PERF_REPO + --ignored --nocapture"]
fn real_repo_report() {
    let Some(repo) = std::env::var_os("TG_PERF_REPO") else {
        eprintln!("TG_PERF_REPO not set — report skipped.");
        return;
    };
    let path = PathBuf::from(repo);
    assert!(
        path.join(".git").exists(),
        "TG_PERF_REPO is not a git repo: {path:?}"
    );
    let engine = Git2Engine;

    let (st, dur) = measure(|| engine.status(&path).unwrap());
    println!(
        "[report] status: {dur:?} (staged {}, unstaged {}, branch {:?})",
        st.staged.len(),
        st.unstaged.len(),
        st.branch
    );

    let (page, dur) = measure(|| engine.log(&path, 0, 200).unwrap());
    println!(
        "[report] log first page (200): {dur:?} ({} commits)",
        page.len()
    );
    assert!(!page.is_empty(), "repo without commits?");

    let deep_skip = env_usize("TG_PERF_DEEP_SKIP", 100_000);
    let (deep, dur) = measure(|| engine.log(&path, deep_skip, 200).unwrap());
    println!(
        "[report] log deep page (skip {deep_skip}): {dur:?} ({} commits)",
        deep.len()
    );

    let (files, dur) = measure(|| engine.commit_diff(&path, &page[0].id).unwrap());
    println!("[report] commit_diff HEAD: {dur:?} ({} files)", files.len());

    let (branches, dur) = measure(|| engine.branches(&path).unwrap());
    println!("[report] branches: {dur:?} ({})", branches.len());

    let (unp, dur) = measure(|| engine.unpushed_commits(&path).unwrap());
    println!("[report] unpushed_commits: {dur:?} ({})", unp.len());

    let (hits, dur) = measure(|| engine.search_log(&path, "fix", 100).unwrap());
    println!(
        "[report] search_log \"fix\" (max 100): {dur:?} ({} hits)",
        hits.len()
    );

    let blame_file = std::env::var("TG_PERF_BLAME").unwrap_or_else(|_| "MAINTAINERS".into());
    if path.join(&blame_file).exists() {
        let (lines, dur) = measure(|| engine.blame_file(&path, &blame_file).unwrap());
        println!(
            "[report] blame {blame_file}: {dur:?} ({} lines)",
            lines.len()
        );
    } else {
        println!("[report] blame: \"{blame_file}\" does not exist — skipped");
    }
}

#[test]
#[ignore = "perf gate: run explicitly via `cargo test --test perf_budget -- --ignored`"]
fn performance_budgets() {
    let n = env_usize("TG_PERF_FILES", 3_000);
    let (_g, path) = build_fixture(n);
    let engine = Git2Engine;

    // Target: status < 200 ms @ 100k. Ceiling = a generous multiple.
    let (st, dur) = measure(|| engine.status(&path).unwrap());
    assert!(!st.unstaged.is_empty(), "the fixture should show changes");
    assert_budget(&format!("status ({n} files)"), dur, Duration::from_secs(8));

    // Below FAST_PATH_MIN_INDEX_ENTRIES (30k) `status` always picks git2 — the
    // sidecar path (status_via_git + the porcelain v2 parser) that is supposed
    // to carry the 100k target would otherwise stay ungated. Hence measure it
    // directly here; the sidecar is nearly flat (~55 ms + the process spawn),
    // and the ceiling covers CI runner variance.
    let (st, dur) = measure(|| engine.status_via_sidecar(&path).unwrap());
    assert!(
        !st.unstaged.is_empty(),
        "the sidecar status should show changes"
    );
    assert_budget(
        &format!("status via sidecar ({n} files)"),
        dur,
        Duration::from_secs(8),
    );

    // Target: diff rendering < 100 ms. The realistic case: the second commit
    // (a few files), not the 4000-file initial import.
    let head = engine.log(&path, 0, 1).unwrap();
    let (files, dur) = measure(|| engine.commit_diff(&path, &head[0].id).unwrap());
    assert!(!files.is_empty(), "the update commit should contain files");
    assert_budget(
        &format!("commit_diff ({} files)", files.len()),
        dur,
        Duration::from_secs(5),
    );
}
