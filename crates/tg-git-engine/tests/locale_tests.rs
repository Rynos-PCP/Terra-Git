//! Remote errors are classified whatever the user's language.
//!
//! The classifier matches git's English wording. With a German git ("Updates
//! wurden zurückgewiesen …") every remote error used to fall through to
//! `sidecar_failed`; remote ops now run with `LC_ALL=C`. This binary sets a
//! German locale for the whole process (its own test binary, so no other test
//! sees it) — on a machine whose git ships German translations it proves the
//! fix, elsewhere it guards the classification as a regression test.

use std::path::Path;
use std::process::Command;

use tg_git_engine::prelude::*;

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .status()
        .expect("git start")
        .success();
    assert!(ok, "git {args:?} failed");
}

fn commit(dir: &Path, file: &str, msg: &str) {
    std::fs::write(dir.join(file), format!("{msg}\n")).unwrap();
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", msg]);
}

fn configure(dir: &Path) {
    git(dir, &["config", "user.name", "Locale"]);
    git(dir, &["config", "user.email", "locale@test.local"]);
    git(dir, &["config", "commit.gpgsign", "false"]);
}

#[test]
fn a_rejected_push_is_classified_under_a_german_locale() {
    for (key, value) in [
        ("LANGUAGE", "de"),
        ("LANG", "de_DE.UTF-8"),
        ("LC_ALL", "de_DE.UTF-8"),
        ("LC_MESSAGES", "de_DE.UTF-8"),
    ] {
        std::env::set_var(key, value);
    }

    let remote = tempfile::tempdir().unwrap();
    git(remote.path(), &["init", "--bare", "-q"]);
    let url = remote.path().to_string_lossy().replace('\\', "/");

    let work_dir = tempfile::tempdir().unwrap();
    let work = work_dir.path().join("work");
    git(work_dir.path(), &["clone", "-q", &url, "work"]);
    configure(&work);
    commit(&work, "a.txt", "A");
    git(&work, &["push", "-q", "origin", "HEAD"]);

    let other_dir = tempfile::tempdir().unwrap();
    let other = other_dir.path().join("other");
    git(other_dir.path(), &["clone", "-q", &url, "other"]);
    configure(&other);
    commit(&other, "b.txt", "B");
    git(&other, &["push", "-q", "origin", "HEAD"]);

    commit(&work, "c.txt", "C");
    let err = Git2Engine.push_remote(&work, "origin", false).unwrap_err();
    assert_eq!(err.code(), "non_fast_forward", "got {} / {err}", err.code());
    // git's own output is kept for the details — in English.
    let detail = err.detail().expect("raw git output");
    assert!(detail.contains("[rejected]"), "detail: {detail}");
}
