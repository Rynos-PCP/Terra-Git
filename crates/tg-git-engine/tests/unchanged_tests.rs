//! Tests for `explain_unchanged`: why does Git report a file as changed even
//! though the diff is empty?
//!
//! Background (measured 2026-07-21): in these cases `file_diff` returns NOT
//! `None` but `Some(FileDiff)` with an empty `hunks` vector. `None` means
//! exclusively "the file is clean". The diff view therefore showed a
//! meaningless "no changes".

use std::fs;
use std::path::{Path, PathBuf};

use tg_domain::{EolStyle, UnchangedReason};
use tg_git_engine::prelude::*;

fn init_repo() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = git2::Repository::init(dir.path()).expect("init");
    let mut config = repo.config().expect("config");
    config.set_str("user.name", "Terra Tester").unwrap();
    config.set_str("user.email", "terra@test.local").unwrap();
    config.set_str("commit.gpgsign", "false").unwrap();
    // A defined starting point for the line endings instead of the host's:
    // GitHub's Windows runners set core.autocrlf=true globally. The tests that
    // are about the conversion set their own value (see set_eol_config below and
    // the two autocrlf_* tests); everything else expects the neutral case.
    config.set_bool("core.autocrlf", false).unwrap();
    // A global attributes file marking files as text would convert despite
    // autocrlf=false; core.eol decides the direction, so pin it as well.
    config.set_str("core.eol", "lf").unwrap();
    let path = dir.path().to_path_buf();
    (dir, path)
}

/// Raw git for the setup. Always under the C locale: the user runs a
/// German-language git whose plain text would otherwise not be parseable.
fn git(dir: &Path, args: &[&str]) {
    let ok = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .status()
        .expect("git start")
        .success();
    assert!(ok, "git {args:?} failed");
}

/// Sets `core.eol`/`core.autocrlf` in its own block so the git2 handle drops
/// before the engine call (a lock trap on Windows otherwise).
fn set_eol_config(path: &Path, autocrlf: bool, eol: &str) {
    let repo = git2::Repository::open(path).unwrap();
    let mut c = repo.config().unwrap();
    c.set_bool("core.autocrlf", autocrlf).unwrap();
    c.set_str("core.eol", eol).unwrap();
}

/// The case actually observed: blob and working copy are byte-identical (both
/// LF), but a checkout would write CRLF because of `text=auto`. Git therefore
/// reports the file as changed while the diff stays empty.
///
/// A pure byte comparison would wrongly say "identical" here — this test guards
/// exactly that trap.
#[test]
fn working_copy_lf_while_checkout_writes_crlf_is_eol_only() {
    let (_g, path) = init_repo();
    set_eol_config(&path, false, "crlf");
    fs::write(path.join(".gitattributes"), "* text=auto\n").unwrap();
    fs::write(path.join("a.txt"), "one\r\ntwo\r\nthree\r\n").unwrap();
    git(&path, &["add", "-A"]);
    git(&path, &["commit", "-m", "start"]);
    git(&path, &["checkout", "--", "a.txt"]);

    // A tool rewrites the file with LF — the content is unchanged.
    fs::write(path.join("a.txt"), "one\ntwo\nthree\n").unwrap();

    let info = Git2Engine.explain_unchanged(&path, "a.txt", false).unwrap();
    assert_eq!(
        info.reason,
        UnchangedReason::EolOnly,
        "byte-equal but differing from the expected checkout: that is line endings"
    );
    assert_eq!(info.new_eol, Some(EolStyle::Lf), "the working copy has LF");
    assert_eq!(
        info.expected_eol,
        Some(EolStyle::Crlf),
        "core.eol=crlf plus text=auto yields CRLF in the working tree"
    );
}

/// The classic case: blob LF, working copy CRLF. Here the bytes really differ;
/// after CR normalization they are equal.
#[test]
fn working_copy_crlf_against_lf_blob_is_eol_only() {
    let (_g, path) = init_repo();
    set_eol_config(&path, false, "lf");
    fs::write(path.join(".gitattributes"), "* text=auto\n").unwrap();
    fs::write(path.join("a.txt"), "one\ntwo\nthree\n").unwrap();
    git(&path, &["add", "-A"]);
    git(&path, &["commit", "-m", "start"]);

    fs::write(path.join("a.txt"), "one\r\ntwo\r\nthree\r\n").unwrap();

    let info = Git2Engine.explain_unchanged(&path, "a.txt", false).unwrap();
    assert_eq!(info.reason, UnchangedReason::EolOnly);
    assert_eq!(info.old_eol, Some(EolStyle::Lf), "the repository has LF");
    assert_eq!(
        info.new_eol,
        Some(EolStyle::Crlf),
        "the working copy has CRLF"
    );
}

/// Only the executable bit. Per measurement this is only reproducible through
/// the staged path (tree against index) — on Windows the working tree does not
/// know the bit.
#[test]
fn mode_bit_only_is_reported_as_such() {
    let (_g, path) = init_repo();
    fs::write(path.join("s.sh"), "#!/bin/sh\necho hi\n").unwrap();
    git(&path, &["add", "-A"]);
    git(&path, &["commit", "-m", "start"]);
    git(&path, &["update-index", "--chmod=+x", "s.sh"]);

    let info = Git2Engine.explain_unchanged(&path, "s.sh", true).unwrap();
    assert_eq!(
        info.reason,
        UnchangedReason::ModeOnly,
        "identical content, only the mode bit differs"
    );
    assert_eq!(info.old_mode.as_deref(), Some("100644"));
    assert_eq!(info.new_mode.as_deref(), Some("100755"));
}

/// Counter-check: a real content change must NEVER pass as harmless. A false
/// "line endings only" would lead the user to throw away a real change.
#[test]
fn real_change_is_not_classified_as_harmless() {
    let (_g, path) = init_repo();
    fs::write(path.join("a.txt"), "one\ntwo\n").unwrap();
    git(&path, &["add", "-A"]);
    git(&path, &["commit", "-m", "start"]);

    fs::write(path.join("a.txt"), "one\nTWO\n").unwrap();

    let info = Git2Engine.explain_unchanged(&path, "a.txt", false).unwrap();
    assert_eq!(
        info.reason,
        UnchangedReason::Unknown,
        "different content: no harmless cause may be claimed"
    );
}

/// Counter-check: binary files also have zero hunks. Without this guard every
/// binary change would be mislabelled as "line endings only".
#[test]
fn binary_change_is_not_classified_as_eol_only() {
    let (_g, path) = init_repo();
    fs::write(path.join("image.bin"), [0u8, 159, 146, 150, 0, 1, 2]).unwrap();
    git(&path, &["add", "-A"]);
    git(&path, &["commit", "-m", "start"]);

    fs::write(path.join("image.bin"), [0u8, 1, 2, 3, 4, 5, 6, 7]).unwrap();

    let info = Git2Engine
        .explain_unchanged(&path, "image.bin", false)
        .unwrap();
    assert_ne!(
        info.reason,
        UnchangedReason::EolOnly,
        "binary content must never be reported as a line-ending problem"
    );
    assert_ne!(info.reason, UnchangedReason::Identical);
}

/// REGRESSION (counter-check 2026-07-21, high): a newly created file with real
/// content was reported as "only the executable bit — content identical".
/// Cause: the mode comparison ran BEFORE the existence check, and the missing
/// side has mode 0, so it always differs.
///
/// That is the most dangerous direction of error there is: presenting a real
/// change as harmless.
#[test]
fn newly_created_file_is_not_reported_as_a_mode_change() {
    let (_g, path) = init_repo();
    fs::write(path.join("start.txt"), "start\n").unwrap();
    git(&path, &["add", "-A"]);
    git(&path, &["commit", "-m", "start"]);

    // intent-to-add: the index carries the empty blob, the diff has zero hunks.
    fs::write(
        path.join("secret.txt"),
        "PASSWORD=hunter2\nvery important\n",
    )
    .unwrap();
    git(&path, &["add", "-N", "secret.txt"]);

    for staged in [true, false] {
        let info = Git2Engine
            .explain_unchanged(&path, "secret.txt", staged)
            .unwrap();
        assert_ne!(
            info.reason,
            UnchangedReason::ModeOnly,
            "a new file with content must never count as a mere mode change (staged={staged})"
        );
        assert_ne!(info.reason, UnchangedReason::Identical, "staged={staged}");
    }
}

/// REGRESSION (counter-check 2026-07-21, high): a staged deletion was presented
/// as a harmless mode change (100644 -> 000000).
#[test]
fn staged_deletion_is_not_reported_as_a_mode_change() {
    let (_g, path) = init_repo();
    fs::write(path.join("empty.txt"), "").unwrap();
    fs::write(path.join("start.txt"), "start\n").unwrap();
    git(&path, &["add", "-A"]);
    git(&path, &["commit", "-m", "start"]);
    git(&path, &["rm", "--cached", "empty.txt"]);

    let info = Git2Engine
        .explain_unchanged(&path, "empty.txt", true)
        .unwrap();
    assert_ne!(
        info.reason,
        UnchangedReason::ModeOnly,
        "a deletion is not a mode change"
    );
    assert_ne!(info.reason, UnchangedReason::Identical);
}

/// The binary guard has to bite BEFORE the CR normalization judges: binary
/// content that happens to differ only in CR bytes would otherwise be
/// mislabelled as a line-ending problem.
#[test]
fn binary_content_differing_in_cr_is_not_eol_only() {
    let (_g, path) = init_repo();
    fs::write(path.join("b.bin"), [0u8, 1, b'\r', b'\n', 2]).unwrap();
    git(&path, &["add", "-A"]);
    git(&path, &["commit", "-m", "start"]);

    // The same bytes without the CR — without the NUL guard this would be "line endings only".
    fs::write(path.join("b.bin"), [0u8, 1, b'\n', 2]).unwrap();

    let info = Git2Engine.explain_unchanged(&path, "b.bin", false).unwrap();
    assert_eq!(
        info.reason,
        UnchangedReason::Unknown,
        "NUL in the content means binary — no line-ending statement"
    );
}

/// Mixed line endings have to be named as such, not as LF or CRLF.
#[test]
fn mixed_line_endings_are_reported_as_mixed() {
    let (_g, path) = init_repo();
    set_eol_config(&path, false, "lf");
    fs::write(path.join(".gitattributes"), "* text=auto\n").unwrap();
    fs::write(path.join("a.txt"), "one\ntwo\nthree\n").unwrap();
    git(&path, &["add", "-A"]);
    git(&path, &["commit", "-m", "start"]);

    fs::write(path.join("a.txt"), "one\r\ntwo\nthree\r\n").unwrap();

    let info = Git2Engine.explain_unchanged(&path, "a.txt", false).unwrap();
    assert_eq!(info.reason, UnchangedReason::EolOnly);
    assert_eq!(info.new_eol, Some(EolStyle::Mixed), "CRLF and LF mixed");
}

/// git writes `core.autocrlf` as a boolean — "True" or "1" are valid too.
/// A raw comparison against "true" would swallow the configuration.
#[test]
fn autocrlf_is_read_boolean_and_case_insensitively() {
    let (_g, path) = init_repo();
    {
        let repo = git2::Repository::open(&path).unwrap();
        repo.config()
            .unwrap()
            .set_str("core.autocrlf", "True")
            .unwrap();
    }
    fs::write(path.join(".gitattributes"), "* text=auto\n").unwrap();
    fs::write(path.join("a.txt"), "one\r\ntwo\r\n").unwrap();
    git(&path, &["add", "-A"]);
    git(&path, &["commit", "-m", "start"]);
    git(&path, &["checkout", "--", "a.txt"]);

    fs::write(path.join("a.txt"), "one\ntwo\n").unwrap();

    let info = Git2Engine.explain_unchanged(&path, "a.txt", false).unwrap();
    assert_eq!(
        info.reason,
        UnchangedReason::EolOnly,
        "\"True\" has to act like \"true\""
    );
    assert_eq!(info.expected_eol, Some(EolStyle::Crlf));
}

/// `core.autocrlf=input` only converts on commit — in the working tree
/// everything stays as it is. No line-ending expectation may be claimed then.
#[test]
fn autocrlf_input_claims_no_expectation() {
    let (_g, path) = init_repo();
    {
        let repo = git2::Repository::open(&path).unwrap();
        repo.config()
            .unwrap()
            .set_str("core.autocrlf", "input")
            .unwrap();
    }
    fs::write(path.join(".gitattributes"), "* text=auto\n").unwrap();
    fs::write(path.join("a.txt"), "one\ntwo\n").unwrap();
    git(&path, &["add", "-A"]);
    git(&path, &["commit", "-m", "start"]);

    let info = Git2Engine.explain_unchanged(&path, "a.txt", false).unwrap();
    assert_eq!(info.reason, UnchangedReason::Identical);
    assert_eq!(
        info.expected_eol, None,
        "input enforces nothing in the working tree"
    );
}

/// Paths from outside must not run into the git2 panic (Index::get_path panics
/// on ".." and absolute paths).
#[test]
fn malicious_paths_do_not_panic() {
    let (_g, path) = init_repo();
    fs::write(path.join("a.txt"), "one\n").unwrap();
    git(&path, &["add", "-A"]);
    git(&path, &["commit", "-m", "start"]);

    for evil in ["../../etc/passwd", "..", "a/../../b.txt"] {
        let info = Git2Engine.explain_unchanged(&path, evil, false).unwrap();
        assert_eq!(
            info.reason,
            UnchangedReason::Unknown,
            "{evil} must produce neither a statement nor a panic"
        );
    }
}

/// A genuinely unchanged file: no delta, no cause to name.
#[test]
fn clean_file_reports_identical() {
    let (_g, path) = init_repo();
    fs::write(path.join("a.txt"), "one\ntwo\n").unwrap();
    git(&path, &["add", "-A"]);
    git(&path, &["commit", "-m", "start"]);

    let info = Git2Engine.explain_unchanged(&path, "a.txt", false).unwrap();
    assert_eq!(info.reason, UnchangedReason::Identical);
}
