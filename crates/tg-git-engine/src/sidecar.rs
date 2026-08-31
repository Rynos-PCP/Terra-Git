//! System-git sidecar for remote and multi-step operations.
//!
//! A deliberate architectural decision: network and
//! auth paths run through the installed system git. That way its credential
//! helpers (Windows Credential Manager, SSH agent, self-hosted GitLab access)
//! apply without terra-git needing auth callbacks of its own.
//! Delicate multi-step operations (merge/rebase/cherry-pick/apply) run through
//! the system git too — exactly the semantics of the CLI.

use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::cancel::CancelToken;
use crate::error::{GitEngineError, Result};
use tg_domain::{CloneOptions, CommitInfo, GitProgress};

/// Hard timeout for normal operations: a hanging git process (dead network, a
/// waiting credential dialog) must never block the app.
const SIDECAR_TIMEOUT: Duration = Duration::from_secs(120);

/// Generous timeout for clones of large repositories.
pub const CLONE_TIMEOUT: Duration = Duration::from_secs(3600);

/// Runs `git -C <repo> <args…>` with the standard timeout (output trimmed).
pub fn run_git(repo_path: &Path, args: &[&str]) -> Result<String> {
    run_git_in(repo_path, args, SIDECAR_TIMEOUT)
}

/// Like [`run_git`], but without trimming — needed for patch output where the
/// trailing newline is significant (`git apply` requires it).
pub fn run_git_raw(repo_path: &Path, args: &[&str]) -> Result<String> {
    run_git_impl(repo_path, args, SIDECAR_TIMEOUT, false)
}

/// Runs `git -C <dir> <args…>` and returns the combined output.
///
/// `GIT_TERMINAL_PROMPT=0` prevents hanging credential prompts, `GIT_EDITOR=true`
/// prevents git from opening an interactive editor on merge/rebase/revert (one
/// that would never appear in a GUI context).
pub fn run_git_in(dir: &Path, args: &[&str], timeout: Duration) -> Result<String> {
    run_git_impl(dir, args, timeout, true)
}

/// Like [`run_git`], but with the generous large-repo timeout ([`CLONE_TIMEOUT`])
/// instead of the 120 s hang detection. For local multi-step ops
/// (merge/rebase/cherry-pick/revert/continue) whose worktree checkout can
/// legitimately exceed the standard limit on very large repos — a SIGKILL in the
/// middle of the index/merge phase could leave `.git/index.lock` or half a
/// merge/rebase behind.
pub fn run_git_long(repo_path: &Path, args: &[&str]) -> Result<String> {
    run_git_in(repo_path, args, CLONE_TIMEOUT)
}

/// Like [`run_git_long`] (generous [`CLONE_TIMEOUT`]), but with additional
/// environment variables — e.g. `GIT_SEQUENCE_EDITOR` for the non-interactive
/// interactive rebase, or `LC_ALL=C` for local multi-step ops whose plain-text
/// output has to be parsed locale-independently in English (git/gettext then
/// ignore `LANGUAGE` too).
/// The long timeout (not the 120 s hang detection) is intentional: a kill in the
/// middle of the worktree/index rebuild would leave half a state behind.
pub fn run_git_long_env(repo_path: &Path, args: &[&str], envs: &[(&str, &str)]) -> Result<String> {
    run_git_impl_env(repo_path, args, CLONE_TIMEOUT, true, envs)
}

/// Enables the status accelerators for large worktrees (the basis of the
/// `status` <200 ms budget): the builtin fsmonitor daemon and the
/// untracked cache. Best effort — errors (an older git without builtin
/// fsmonitor) are harmless because `status()` falls back to the libgit2 path on
/// every sidecar error. Call once when OPENING a repo (it writes to
/// `.git/config`), not on every internal open.
pub fn enable_status_accelerators(repo_path: &Path) {
    let _ = run_git(repo_path, &["config", "core.fsmonitor", "true"]);
    let _ = run_git(repo_path, &["config", "core.untrackedCache", "true"]);
    // The config entry alone is not enough — the untracked cache has to be
    // created in the index.
    let _ = run_git(repo_path, &["update-index", "--untracked-cache"]);
}

/// Does a commit graph exist? It carries the generation numbers that let
/// `git log --topo-order` STREAM — without it the first history page falls back
/// to a full walk (Linux kernel: ~15 s instead of 53 ms;
/// docs/perf-stress-test.md). `rev-parse --git-path` also resolves
/// worktree/common-dir cases correctly.
pub(crate) fn commit_graph_ready(repo_path: &Path) -> bool {
    let Ok(objects) = run_git(repo_path, &["rev-parse", "--git-path", "objects"]) else {
        return false;
    };
    let obj = if Path::new(&objects).is_absolute() {
        std::path::PathBuf::from(&objects)
    } else {
        repo_path.join(&objects)
    };
    obj.join("info/commit-graph").exists()
        || obj.join("info/commit-graphs/commit-graph-chain").exists()
}

/// Writes/updates the commit graph (blocking — the caller puts it on a
/// background task). `--split` keeps follow-up runs incrementally cheap; a
/// generous timeout because the first run on huge repos may take minutes.
/// Errors (a very old git) are harmless, git log then simply runs without the
/// graph.
pub(crate) fn write_commit_graph(repo_path: &Path) -> Result<()> {
    run_git_in(
        repo_path,
        &[
            "commit-graph",
            "write",
            "--reachable",
            "--split",
            "--no-progress",
        ],
        CLONE_TIMEOUT,
    )
    .map(|_| ())
}

/// Stable machine-readable `git log` format: records NUL-separated (`-z`),
/// fields separated by \x1f (unit separator). The subject is the LAST field —
/// `splitn` simply leaves any theoretically contained \x1f in there.
const LOG_FORMAT: &str = "--format=%H%x1f%an%x1f%ae%x1f%at%x1f%P%x1f%s";

/// Parses a [`LOG_FORMAT`] record. `short_id` is built from the first 8
/// characters of the full id, as in `commit_to_info`.
fn parse_log_record(rec: &str) -> Option<CommitInfo> {
    let mut f = rec.splitn(6, '\u{1f}');
    let id = f.next()?.to_string();
    let author_name = f.next()?.to_string();
    let author_email = f.next()?.to_string();
    let time = f.next()?.parse::<i64>().ok()?;
    let parents = f.next()?;
    let summary = f.next().unwrap_or("").to_string();
    if id.len() < 7 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some(CommitInfo {
        short_id: id.chars().take(8).collect(),
        id,
        summary,
        author_name,
        author_email,
        time,
        parent_ids: parents.split_whitespace().map(str::to_string).collect(),
    })
}

/// Streams `git log --topo-order` (children before parents — the order the
/// history graph expects) and calls `take` per commit; if `take` returns
/// `false`, the child process is terminated (early aborts cost nothing).
///
/// Why the sidecar instead of libgit2: the libgit2 revwalk buffers the COMPLETE
/// graph first, for TOPOLOGICAL as well as TIME (Linux kernel: 68 s), whereas
/// git streams the topo order thanks to commit-graph generation numbers (53 ms).
/// See docs/perf-stress-test.md.
pub(crate) fn stream_log(
    repo_path: &Path,
    extra_args: &[&str],
    mut take: impl FnMut(CommitInfo) -> bool,
) -> Result<()> {
    let mut cmd = Command::new("git");
    cmd.args(["-c", "protocol.ext.allow=never"])
        .arg("-C")
        .arg(repo_path)
        .args(["log", "--topo-order", "-z", LOG_FORMAT])
        .args(extra_args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    tracing::debug!(?extra_args, dir = %repo_path.display(), "sidecar: git log");
    let mut child = cmd.spawn()?;
    let stdout = child.stdout.take().expect("stdout is piped");

    // A reader thread parses and pushes commits through a bounded channel; the
    // bound acts as backpressure, and a closed receiver (an early end) stops the
    // thread through the send error.
    let (tx, rx) = mpsc::sync_channel::<CommitInfo>(256);
    let reader = std::thread::spawn(move || {
        let mut r = BufReader::new(stdout);
        let mut buf: Vec<u8> = Vec::new();
        loop {
            buf.clear();
            match r.read_until(0, &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            if buf.last() == Some(&0) {
                buf.pop();
            }
            let rec = String::from_utf8_lossy(&buf);
            let rec = rec.trim_matches(|c| c == '\n' || c == '\r');
            if rec.is_empty() {
                continue;
            }
            if let Some(info) = parse_log_record(rec) {
                if tx.send(info).is_err() {
                    break;
                }
            }
        }
    });

    // Hang protection as in run_git: the deadline applies to WAITING without
    // data — a flowing stream may run arbitrarily long.
    let deadline = Instant::now() + SIDECAR_TIMEOUT;
    let mut stopped = false;
    let mut timed_out = false;
    loop {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(info) => {
                if !take(info) {
                    stopped = true;
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if Instant::now() >= deadline {
                    stopped = true;
                    timed_out = true;
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    drop(rx);
    if stopped {
        let _ = child.kill();
    }
    let _ = reader.join();
    let status = child.wait()?;

    if timed_out {
        return Err(GitEngineError::Sidecar {
            message: format!(
                "git log aborted after {} s without data (hung?)",
                SIDECAR_TIMEOUT.as_secs()
            ),
        });
    }
    if stopped || status.success() {
        Ok(())
    } else {
        Err(GitEngineError::Sidecar {
            message: format!("git log failed (exit code {:?})", status.code()),
        })
    }
}

/// One history page (skip/limit) in topo order through the sidecar.
pub(crate) fn log_page(repo_path: &Path, skip: usize, limit: usize) -> Result<Vec<CommitInfo>> {
    let skip_arg = format!("--skip={skip}");
    let count_arg = format!("--max-count={limit}");
    let mut out = Vec::with_capacity(limit.min(1024));
    stream_log(repo_path, &[skip_arg.as_str(), count_arg.as_str()], |c| {
        out.push(c);
        true
    })?;
    Ok(out)
}

/// Like [`log_page`], but across ALL branches (local + remote), tags and HEAD —
/// the data basis of the whole-repository graph.
/// Deliberately explicit ref families instead of `--all`: refs/stash would
/// otherwise linger as noise nodes in the graph.
pub(crate) fn log_page_all(repo_path: &Path, skip: usize, limit: usize) -> Result<Vec<CommitInfo>> {
    let skip_arg = format!("--skip={skip}");
    let count_arg = format!("--max-count={limit}");
    let mut out = Vec::with_capacity(limit.min(1024));
    stream_log(
        repo_path,
        &[
            "--branches",
            "--remotes",
            "--tags",
            "HEAD",
            skip_arg.as_str(),
            count_arg.as_str(),
        ],
        |c| {
            out.push(c);
            true
        },
    )?;
    Ok(out)
}

fn run_git_impl(dir: &Path, args: &[&str], timeout: Duration, trim: bool) -> Result<String> {
    run_git_impl_env(dir, args, timeout, trim, &[])
}

fn run_git_impl_env(
    dir: &Path,
    args: &[&str],
    timeout: Duration,
    trim: bool,
    envs: &[(&str, &str)],
) -> Result<String> {
    let mut cmd = Command::new("git");
    // A hard, platform-wide guard against the ext:: code-execution transport: a
    // `-c` on the command line also overrides a malicious repo-local
    // `protocol.ext.allow=always` in a shipped .git/config. Applies to EVERY
    // sidecar call (fetch/pull/push/status/…).
    cmd.args(["-c", "protocol.ext.allow=never"])
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in envs {
        cmd.env(k, v);
    }

    // No console window flashing up on Windows (CREATE_NO_WINDOW).
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }

    tracing::debug!(?args, dir = %dir.display(), "sidecar: git");
    let mut child = cmd.spawn()?;

    // Drain the pipes in threads so the child process never blocks on a full
    // pipe buffer while we wait for it.
    let mut stdout_pipe = child.stdout.take();
    let mut stderr_pipe = child.stderr.take();
    let out_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(p) = stdout_pipe.as_mut() {
            let _ = p.read_to_end(&mut buf);
        }
        buf
    });
    let err_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(p) = stderr_pipe.as_mut() {
            let _ = p.read_to_end(&mut buf);
        }
        buf
    });

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait()? {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = out_reader.join();
                let _ = err_reader.join();
                return Err(GitEngineError::Sidecar {
                    message: format!(
                        "git {} aborted after {} s (timeout) — check the network/remote",
                        args.join(" "),
                        timeout.as_secs()
                    ),
                });
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    };

    let stdout_buf = out_reader.join().unwrap_or_default();
    let stderr_buf = err_reader.join().unwrap_or_default();
    let stdout = String::from_utf8_lossy(&stdout_buf);
    let stderr = String::from_utf8_lossy(&stderr_buf);

    if status.success() {
        if trim {
            Ok(format!("{stdout}{stderr}").trim().to_string())
        } else {
            // Raw: stdout only (patch data), stderr is diagnostics.
            Ok(stdout.into_owned())
        }
    } else {
        let combined = format!("{stdout}{stderr}").trim().to_string();
        Err(GitEngineError::Sidecar {
            message: if combined.is_empty() {
                format!(
                    "git {} failed (exit code {:?})",
                    args.join(" "),
                    status.code()
                )
            } else {
                combined
            },
        })
    }
}

/// Parses a `git --progress` line into phase + percent. `None` when the line
/// carries no progress (e.g. plain "done." or status lines).
/// Git writes progress to stderr, updated with `\r` separators:
/// `Receiving objects:  45% (450/1000), 1.2 MiB | 2.0 MiB/s`.
pub(crate) fn parse_progress(line: &str) -> Option<GitProgress> {
    // Strip the optional "remote: " prefix (server-side phases).
    let line = line.trim();
    let line = line.strip_prefix("remote:").map(str::trim).unwrap_or(line);
    let (phase_raw, rest) = line.split_once(':')?;
    let pct = rest.find('%')?;
    // Collect the digits directly before the '%'.
    let digits: String = rest[..pct]
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    let percent: u8 = digits.parse().ok()?;
    Some(GitProgress {
        phase: normalize_phase(phase_raw),
        percent: percent.min(100),
    })
}

fn normalize_phase(raw: &str) -> String {
    let low = raw.to_ascii_lowercase();
    for key in [
        "receiving",
        "resolving",
        "compressing",
        "counting",
        "writing",
        "enumerating",
    ] {
        if low.contains(key) {
            return key.to_string();
        }
    }
    "other".to_string()
}

/// Like [`run_git_in`], but streams the stderr progress lines live to
/// `on_progress`. stdout is collected as usual; the timeout still applies.
pub fn run_git_streaming(
    dir: &Path,
    args: &[&str],
    timeout: Duration,
    cancel: &CancelToken,
    on_progress: &mut dyn FnMut(GitProgress),
) -> Result<String> {
    let mut cmd = Command::new("git");
    // ext:: guard as in run_git_impl_env (see there) — also for the streaming
    // remote ops (fetch/pull/push/clone_fetch).
    cmd.args(["-c", "protocol.ext.allow=never"])
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    tracing::debug!(?args, dir = %dir.display(), "sidecar: git (streaming)");
    let mut child = cmd.spawn()?;

    let mut stdout_pipe = child.stdout.take();
    let out_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(p) = stdout_pipe.as_mut() {
            let _ = p.read_to_end(&mut buf);
        }
        buf
    });

    // Read stderr byte by byte: `\r` AND `\n` separate progress segments. The
    // reader thread parses and sends progress over a channel so the main thread
    // can supervise the timeout.
    let mut stderr_pipe = child.stderr.take();
    let (tx, rx) = std::sync::mpsc::channel::<GitProgress>();
    let err_reader = std::thread::spawn(move || {
        let mut raw = Vec::new();
        let mut seg = Vec::new();
        if let Some(p) = stderr_pipe.as_mut() {
            let mut byte = [0u8; 1];
            while let Ok(1) = p.read(&mut byte) {
                raw.push(byte[0]);
                if byte[0] == b'\r' || byte[0] == b'\n' {
                    flush_segment(&mut seg, &tx);
                } else {
                    seg.push(byte[0]);
                }
            }
        }
        flush_segment(&mut seg, &tx);
        String::from_utf8_lossy(&raw).into_owned()
    });

    let deadline = Instant::now() + timeout;
    let status = loop {
        while let Ok(p) = rx.try_recv() {
            on_progress(p);
        }
        match child.try_wait()? {
            Some(status) => break status,
            // Cancel from the UI: kill the child process immediately.
            None if cancel.is_cancelled() => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = out_reader.join();
                let _ = err_reader.join();
                return Err(GitEngineError::Cancelled);
            }
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = out_reader.join();
                let _ = err_reader.join();
                return Err(GitEngineError::Sidecar {
                    message: format!(
                        "git {} aborted after {} s (timeout) — check the network/remote",
                        args.join(" "),
                        timeout.as_secs()
                    ),
                });
            }
            None => std::thread::sleep(Duration::from_millis(40)),
        }
    };

    let stderr = err_reader.join().unwrap_or_default();
    // Deliver progress messages still buffered after the process ended.
    while let Ok(p) = rx.try_recv() {
        on_progress(p);
    }
    let stdout_buf = out_reader.join().unwrap_or_default();
    let stdout = String::from_utf8_lossy(&stdout_buf);

    if status.success() {
        Ok(format!("{stdout}{stderr}").trim().to_string())
    } else {
        let combined = format!("{stdout}{stderr}").trim().to_string();
        Err(GitEngineError::Sidecar {
            message: if combined.is_empty() {
                format!(
                    "git {} failed (exit code {:?})",
                    args.join(" "),
                    status.code()
                )
            } else {
                combined
            },
        })
    }
}

fn flush_segment(seg: &mut Vec<u8>, tx: &std::sync::mpsc::Sender<GitProgress>) {
    if seg.is_empty() {
        return;
    }
    let line = String::from_utf8_lossy(seg).into_owned();
    seg.clear();
    if let Some(p) = parse_progress(&line) {
        let _ = tx.send(p);
    }
}

/// Process counter for unique temp file names (Windows SystemTime is coarse;
/// nanos alone would collide on fast calls and make `create_new` fail).
static TEMP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Creates a temp file in `std::env::temp_dir()` and writes `content` SAFELY:
/// `create_new` (O_EXCL) follows no pre-placed symlink and overwrites no existing
/// file — an attacker in a shared /tmp cannot redirect a write onto a foreign
/// target; on Unix additionally 0600 (no reading by other local users). The name
/// carries the PID, nanos and a process counter.
pub(crate) fn write_secure_temp(
    prefix: &str,
    ext: &str,
    content: &[u8],
) -> std::io::Result<std::path::PathBuf> {
    use std::io::Write;
    let n = TEMP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let path =
        std::env::temp_dir().join(format!("{prefix}-{}-{nanos}-{n}.{ext}", std::process::id()));
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(&path)?;
    f.write_all(content)?;
    Ok(path)
}

/// Applies a patch text via `git apply` (index and/or workdir).
///
/// The patch travels through a temp file because `run_git_in` deliberately keeps
/// stdin closed.
pub fn apply_patch(repo_path: &Path, patch: &str, cached: bool, reverse: bool) -> Result<String> {
    let file = write_secure_temp("terra-git-patch", "diff", patch.as_bytes())?;
    let file_str = file.to_string_lossy().into_owned();

    let mut args: Vec<&str> = vec!["apply", "--whitespace=nowarn"];
    if cached {
        args.push("--cached");
    }
    if reverse {
        args.push("--reverse");
    }
    args.push(&file_str);

    let result = run_git(repo_path, &args);
    let _ = std::fs::remove_file(&file);
    result
}

/// Literal pathspec (prevents fnmatch globbing in git commands).
pub fn literal_pathspec(file: &str) -> String {
    format!(":(literal){file}")
}

/// Quotes a string for a POSIX sh single-quote context: every `'` is replaced by
/// the sequence `'\''` and the whole thing is wrapped in `'…'`. For file paths
/// embedded into an `sh -c` line (e.g. the todo/message files of the interactive
/// rebase). This way a path with an apostrophe (`O'Brien`) cannot escape the sh
/// line.
pub(crate) fn sh_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Registers an existing stash commit in the stash stack again
/// (`git stash store -m <message> <commit>`) — the counterpart to a stash drop
/// whose commit object lives on in the object database.
///
/// SECURITY: the caller MUST have validated `commit` as a pure hex SHA — the
/// argument goes positionally to the git CLI (no option injection).
/// `message` is uncritical: it is consumed as its own argv value of `-m`.
pub fn stash_store(repo_path: &Path, message: &str, commit: &str) -> Result<String> {
    run_git(repo_path, &["stash", "store", "-m", message, commit])
}

/// Runs an interactive rebase onto `base` with a given todo list, WITHOUT an
/// interactive editor: `GIT_SEQUENCE_EDITOR` is set to a `cp` command that
/// overwrites the todo file generated by git with ours. Cross-platform, because
/// git (git-for-windows included) starts the sequence editor through the `sh` +
/// coreutils it ships.
///
/// `msgs` are reword message files as `(source in temp_dir, target name)`. The
/// sequence editor copies them NEXT TO the todo file into git's own
/// `rebase-merge` directory; the exec lines reference them from there via
/// `git rev-parse --git-path`. Reason: if the rebase pauses on a conflict, the
/// messages have to survive until `--continue` — in temp_dir nobody would clean
/// them up afterwards, whereas git removes its `rebase-merge`
/// recursively and reliably on `--continue`-to-the-end, `--abort` and `--skip`,
/// even when the user ends the rebase externally. The temp_dir sources are
/// deleted here unconditionally.
///
/// On conflicts git returns with exit != 0 and the repo is in the rebase state —
/// the caller/frontend handles that like merge/rebase (conflict banner,
/// continue/abort).
pub fn rebase_interactive(
    repo_path: &Path,
    base: &str,
    todo: &str,
    msgs: &[(std::path::PathBuf, String)],
) -> Result<String> {
    let plan = write_secure_temp("terra-git-rebase", "txt", todo.as_bytes())?;
    // Forward slashes for git's POSIX sh; safe single-quote escaping (spaces AND
    // apostrophes in the path).
    let plan_posix = plan.to_string_lossy().replace('\\', "/");
    let seq_editor = if msgs.is_empty() {
        format!("cp {}", sh_single_quote(&plan_posix))
    } else {
        // git calls the editor as `sh -c "<editor> \"$@\"" <editor> <todo>`, and
        // the todo path arrives absolute. `${1%/*}` yields its directory without
        // an external `dirname`, `cp -p` preserves the 0600 mode.
        //
        // Chained with `&&`, NOT `;`: with `;` the return value would be that of
        // the last copy — if only the plan copy failed, git would carry on with
        // the unchanged default todo and silently discard the user's plan
        // (reorder/squash/drop/reword).
        let mut cmd = format!("cp -p {} \"$1\"", sh_single_quote(&plan_posix));
        for (src, name) in msgs {
            let src_posix = src.to_string_lossy().replace('\\', "/");
            cmd.push_str(&format!(
                " && cp -p {} \"${{1%/*}}/{name}\"",
                sh_single_quote(&src_posix)
            ));
        }
        format!("f() {{ {cmd}; }}; f")
    };
    // Long timeout: replaying many commits on large repos legitimately exceeds
    // the 120 s hang detection (a kill would leave half a rebase behind).
    let result = run_git_long_env(
        repo_path,
        // autostash: safely stash local changes around the rebase;
        // autosquash=false: we control the todo ourselves.
        &[
            "-c",
            "rebase.autosquash=false",
            "rebase",
            "-i",
            "--autostash",
            base,
        ],
        &[("GIT_SEQUENCE_EDITOR", &seq_editor)],
    );
    let _ = std::fs::remove_file(&plan);
    // Unconditionally: from here on the message lives in git's rebase-merge (or
    // the rebase never came about) — the source is never needed again.
    for (src, _) in msgs {
        let _ = std::fs::remove_file(src);
    }
    result
}

/// Lists the active sparse-checkout entries (one directory per line in cone
/// mode). Errors when sparse checkout is not initialized.
pub fn sparse_checkout_list(repo_path: &Path) -> Result<String> {
    run_git(repo_path, &["sparse-checkout", "list"])
}

/// Sets the sparse-checkout selection in cone mode to exactly `dirs`.
///
/// SECURITY: the caller MUST have validated the entries (not empty, no leading
/// '-', no "..", no backslash) — the `--` separator additionally stops option
/// injection.
///
/// Long timeout: set/disable rebuild tens of thousands of files in the worktree
/// on large repos — the 120 s hang detection would kill halfway through.
pub fn sparse_checkout_set(repo_path: &Path, dirs: &[String]) -> Result<String> {
    let mut args: Vec<&str> = vec!["sparse-checkout", "set", "--cone", "--"];
    args.extend(dirs.iter().map(String::as_str));
    run_git_long(repo_path, &args)
}

/// Disables sparse checkout and restores the full worktree (long timeout, see
/// [`sparse_checkout_set`]).
pub fn sparse_checkout_disable(repo_path: &Path) -> Result<String> {
    run_git_long(repo_path, &["sparse-checkout", "disable"])
}

/// Classifies a failed remote operation from the git output into a stable code +
/// an action-oriented message. Order: from specific to general. `None` = no
/// known signature.
pub(crate) fn classify_remote_error(text: &str) -> Option<(&'static str, String)> {
    let t = text.to_ascii_lowercase();
    let has = |n: &str| t.contains(n);

    if has("host key verification failed") || has("remote host identification has changed") {
        return Some((
            "host_key",
            "SSH host key not verified or changed. Check the system's known_hosts \
             entries (remove the old entry if the server key changed)."
                .into(),
        ));
    }
    if has("permission denied (publickey") || has("no matching host key") {
        return Some((
            "ssh_auth",
            "SSH authentication failed (no matching key). Is the right key loaded in \
             the SSH agent and registered with the remote?"
                .into(),
        ));
    }
    // Force push rejected by --force-with-lease (the remote has moved).
    if has("stale info") || (has("force-with-lease") && has("reject")) {
        return Some((
            "force_lease_stale",
            "Force push rejected: the remote branch has changed since the last fetch. \
             Fetch first and review the new commits before overwriting."
                .into(),
        ));
    }
    if has("non-fast-forward")
        || has("fetch first")
        || (has("[rejected]") && has("behind"))
        || has("tip of your current branch is behind")
    {
        return Some((
            "non_fast_forward",
            "Push rejected: the remote branch has commits you do not have locally. Pull \
             first (merge/rebase) — or deliberately overwrite with a force push."
                .into(),
        ));
    }
    // Treat "403" as an HTTP status only in context: the search runs over the
    // COMPLETE output including progress lines and object names — a counter
    // "(1403/44000)" or a SHA "403f9ab" is NOT a forbidden.
    // Hence: a known prefix in front AND no digit/hex position after it.
    let http_403 = ["http 403", "error: 403", "status code: 403"]
        .iter()
        .any(|p| {
            t.match_indices(p).any(|(i, m)| {
                t.as_bytes()
                    .get(i + m.len())
                    .is_none_or(|b| !b.is_ascii_alphanumeric())
            })
        })
        || has("403 forbidden");
    if has("pre-receive hook declined")
        || has("protected branch")
        || has("you are not allowed")
        || http_403
        || has("access denied")
    {
        return Some((
            "forbidden",
            "Rejected by the server (missing permission or a protected branch). Check \
             your repository rights and the branch protection rules."
                .into(),
        ));
    }
    if has("authentication failed")
        || has("could not read username")
        || has("invalid username or password")
        || has("terminal prompts disabled")
        || has("http basic: access denied")
    {
        return Some((
            "auth_failed",
            "Authentication failed. Check the credentials (credential manager / is the \
             token valid?) — terra-git uses the system credentials."
                .into(),
        ));
    }
    if (has("repository") && has("not found")) || has("does not appear to be a git repository") {
        return Some((
            "repo_not_found",
            "Repository not found on the remote. Check the URL (and whether you have access)."
                .into(),
        ));
    }
    if has("no tracking information") || has("no upstream") || has("has no upstream branch") {
        return Some((
            "no_upstream",
            "No upstream set for this branch. The first push creates one automatically; \
             otherwise pick a remote target."
                .into(),
        ));
    }
    if has("need to specify how to reconcile") || has("divergent branches") {
        return Some((
            "divergent",
            "The local and remote branch have diverged. Reconcile them with a pull using \
             merge or rebase."
                .into(),
        ));
    }
    if has("conflict") || has("automatic merge failed") {
        return Some((
            "merge_conflict",
            "The pull created conflicts. Resolve them in the conflict view and continue \
             the operation afterwards."
                .into(),
        ));
    }
    if has("could not resolve host")
        || has("failed to connect")
        || has("unable to access")
        || has("connection timed out")
        || has("could not read from remote repository")
        || has("operation timed out")
    {
        return Some((
            "network",
            "Network error: the remote is unreachable. Check the connection, the URL and \
             any proxy/VPN."
                .into(),
        ));
    }
    None
}

/// Runs a remote operation and classifies errors into a stable,
/// action-oriented [`GitEngineError::Remote`].
fn remote_op(res: Result<String>) -> Result<String> {
    match res {
        Ok(v) => Ok(v),
        Err(GitEngineError::Sidecar { message }) => match classify_remote_error(&message) {
            Some((code, msg)) => Err(GitEngineError::Remote { code, message: msg }),
            None => Err(GitEngineError::Sidecar { message }),
        },
        Err(e) => Err(e),
    }
}

/// Config value that registers the app as an ADDITIONAL git credential helper
/// (reads the provider accounts' tokens from the OS keychain).
/// git appends `-c credential.helper=…` to the helper list — existing system
/// helpers run first and keep priority. Only active when the app has set the env
/// variable (test binaries never inject it).
fn credential_helper_config() -> Option<String> {
    let exe = std::env::var("TERRA_GIT_CREDENTIAL_EXE").ok()?;
    // The leading `!` is mandatory: only then does git execute the value as a
    // shell command (git for Windows ships the sh). Without `!`, git would look
    // for a subcommand `git credential-'<exe>'` — the helper would NEVER run.
    // Forward-slash the path and single-quote it (spaces!). If it contains a
    // quote itself, rather do not inject at all than quote it broken (known
    // limit: an apostrophe in the installation path).
    let exe = exe.replace('\\', "/");
    if exe.is_empty() || exe.contains('\'') {
        return None;
    }
    Some(format!("credential.helper=!'{exe}' __credential"))
}

/// Host of an HTTPS (or HTTP) remote URL, lower-cased, without userinfo and
/// port. `None` for anything without an http(s) scheme (above all SSH — that
/// uses no TLS, `sslVerify` is meaningless there).
fn https_host(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    // Authority = everything up to the first path/query/fragment separator.
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    // Split off the userinfo (user[:pw]@), then the port.
    let hostport = authority.rsplit('@').next().unwrap_or(authority);
    let host = hostport.split(':').next().unwrap_or(hostport);
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// Process-wide state of the "TLS verification off" hosts. Replaces the
/// former runtime channel through `std::env::set_var`: setenv at runtime is UB on
/// Unix next to `Command::spawn` from other threads (glibc setenv vs. the
/// environ copy; Rust marks `set_var` unsafe from edition 2024 on).
/// `None` = the setter was never called -> env fallback (startup path/tests).
static INSECURE_TLS_HOSTS: std::sync::RwLock<Option<Vec<String>>> = std::sync::RwLock::new(None);

/// Sets the opt-in hosts with TLS verification disabled (normalized: trimmed,
/// lower-cased, without empties). An empty list explicitly means "no insecure
/// hosts" — the env fallback no longer applies after that. The app calls this at
/// startup and on every account change.
pub fn set_insecure_tls_hosts(hosts: Vec<String>) {
    let normalized: Vec<String> = hosts
        .iter()
        .map(|h| h.trim().to_ascii_lowercase())
        .filter(|h| !h.is_empty())
        .collect();
    *INSECURE_TLS_HOSTS
        .write()
        .unwrap_or_else(|p| p.into_inner()) = Some(normalized);
}

/// The hosts the user marked as "TLS verification off" (self-signed self-hosted
/// instances). The process state (setter above) counts first; if it was never
/// set, the comma-separated env variable `TERRA_GIT_INSECURE_TLS_HOSTS` applies
/// as a fallback (startup path; test binaries never set it).
fn insecure_hosts() -> Vec<String> {
    if let Some(hosts) = INSECURE_TLS_HOSTS
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
    {
        return hosts;
    }
    std::env::var("TERRA_GIT_INSECURE_TLS_HOSTS")
        .ok()
        .map(|s| {
            s.split(',')
                .map(|h| h.trim().to_ascii_lowercase())
                .filter(|h| !h.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Config keys whose repo-local value makes git run a COMMAND during a remote
/// operation. `credential.helper` is multi-valued: the app's own `-c` is
/// APPENDED to the list, so a repo-local `helper = "!<command>"` still runs (and
/// runs first). `core.askPass` is a program git starts when it needs a password;
/// `GIT_TERMINAL_PROMPT=0` only suppresses the TERMINAL fallback, not these two.
const LOCAL_CREDENTIAL_HOOKS: &[&str] = &["credential.helper", "core.askpass"];

/// Refuses a remote operation when the repository's OWN `.git/config` defines a
/// credential hook.
///
/// A `.git` directory someone else supplied (an unpacked archive, a network
/// share, a USB stick) is executable code as far as git is concerned. All it
/// takes is a remote whose server answers 401 and a repo-local
/// `[credential] helper = "!<command>"`, and a single click on Fetch runs that
/// command — without any dialog and without a compromised renderer.
///
/// Same stance as `open_mergetool`: repo-local command definitions are not
/// executed, and the user is told why instead of being handed a cryptic git
/// error. Global/system helpers (Windows Credential Manager, osxkeychain,
/// libsecret) are untouched — they are not attacker-controlled.
pub(crate) fn reject_local_credential_hooks(repo_path: &Path) -> Result<()> {
    let Ok(repo) = git2::Repository::open(repo_path) else {
        return Ok(());
    };
    let Ok(config) = repo.config() else {
        return Ok(());
    };
    let Ok(local) = config.open_level(git2::ConfigLevel::Local) else {
        return Ok(());
    };
    for key in LOCAL_CREDENTIAL_HOOKS {
        // Multi-valued: a single entry is enough, so check the whole multivar.
        let found = local
            .multivar(key, None)
            .map(|mut e| e.next().is_some())
            .unwrap_or(false);
        if found {
            return Err(GitEngineError::Remote {
                code: "local_credential_hook",
                message: format!(
                    "This repository carries its own `{key}` in .git/config. git would run \
                     that as a command during the transfer, so terra-git refuses the remote \
                     operation. Remove the entry (git config --unset {key}) or set the \
                     credential helper in your global git configuration."
                ),
            });
        }
    }
    Ok(())
}

/// HTTPS remote URLs of a local repo (empty when it cannot be opened).
fn repo_remote_urls(repo_path: &Path) -> Vec<String> {
    let Ok(repo) = git2::Repository::open(repo_path) else {
        return Vec::new();
    };
    let Ok(remotes) = repo.remotes() else {
        return Vec::new();
    };
    remotes
        .iter()
        // A remote whose name or URL is not valid UTF-8 is skipped, not reported.
        .filter_map(|name| name.ok().flatten())
        .filter_map(|name| {
            repo.find_remote(name)
                .ok()
                .and_then(|r| r.url().ok().map(String::from))
        })
        .collect()
}

/// Host-BOUND `http.https://<host>/.sslVerify=false` configs for every remote
/// URL whose host is insecure by opt-in. Deliberately host-bound instead of a
/// global `http.sslVerify=false`: a second remote to a different host in the same
/// repo stays verified. Deduplicated.
fn select_insecure_configs(remote_urls: &[String], insecure_hosts: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for url in remote_urls {
        if let Some(host) = https_host(url) {
            if insecure_hosts.iter().any(|h| h == &host) {
                let cfg = format!("http.https://{host}/.sslVerify=false");
                if !out.contains(&cfg) {
                    out.push(cfg);
                }
            }
        }
    }
    out
}

/// sslVerify opt-out configs for THIS repo's remotes (state/env + repo remotes).
fn insecure_tls_configs(repo_path: &Path) -> Vec<String> {
    let hosts = insecure_hosts();
    if hosts.is_empty() {
        return Vec::new();
    }
    select_insecure_configs(&repo_remote_urls(repo_path), &hosts)
}

/// Leading `-c key=value` pairs for a remote operation: first the credential
/// helper (system helpers keep priority), then the host-bound sslVerify opt-outs.
/// As owned strings, because the sslVerify configs are built from host names at
/// runtime.
fn leading_config(cred: &Option<String>, insecure: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Some(c) = cred {
        out.push("-c".to_string());
        out.push(c.clone());
    }
    for cfg in insecure {
        out.push("-c".to_string());
        out.push(cfg.clone());
    }
    out
}

pub fn fetch(repo_path: &Path) -> Result<String> {
    fetch_streaming(repo_path, &CancelToken::new(), &mut |_| {})
}

/// Like [`fetch`], but streams the progress to `on` and is cancellable through
/// `cancel`.
pub fn fetch_streaming(
    repo_path: &Path,
    cancel: &CancelToken,
    on: &mut dyn FnMut(GitProgress),
) -> Result<String> {
    reject_local_credential_hooks(repo_path)?;
    let lead = leading_config(
        &credential_helper_config(),
        &insecure_tls_configs(repo_path),
    );
    let mut args: Vec<&str> = lead.iter().map(String::as_str).collect();
    args.extend(["fetch", "--prune", "--progress"]);
    let out = remote_op(run_git_streaming(
        repo_path,
        &args,
        CLONE_TIMEOUT,
        cancel,
        on,
    ))?;
    maintain_commit_graph(repo_path);
    Ok(out)
}

pub fn pull(repo_path: &Path) -> Result<String> {
    pull_streaming(repo_path, false, &CancelToken::new(), &mut |_| {})
}

/// Like [`pull`], but streams the progress to `on` and is cancellable.
/// `prune = true` appends `--prune` (removes dead remote-tracking refs on pull,
/// analogous to `git fetch --prune`).
pub fn pull_streaming(
    repo_path: &Path,
    prune: bool,
    cancel: &CancelToken,
    on: &mut dyn FnMut(GitProgress),
) -> Result<String> {
    reject_local_credential_hooks(repo_path)?;
    let lead = leading_config(
        &credential_helper_config(),
        &insecure_tls_configs(repo_path),
    );
    let mut args: Vec<&str> = lead.iter().map(String::as_str).collect();
    args.push("pull");
    if prune {
        args.push("--prune");
    }
    args.push("--progress");
    let out = remote_op(run_git_streaming(
        repo_path,
        &args,
        CLONE_TIMEOUT,
        cancel,
        on,
    ))?;
    maintain_commit_graph(repo_path);
    Ok(out)
}

/// Writes/updates the commit-graph file (best effort — errors abort nothing but
/// are logged once per process).
///
/// The commit-graph file speeds up history walks and ahead/behind computations
/// massively — libgit2 AND the system git use it automatically as soon as it
/// exists. `--split` appends incrementally instead of rewriting everything every
/// time; `--reachable` covers all refs. Called after network operations that
/// bring in new objects. Delegates to [`write_commit_graph`]: its generous
/// timeout is mandatory — the first run after a huge clone takes minutes, and a
/// kill would permanently leave `commit-graph-chain.lock` behind (the graph would
/// then never come into existence).
pub fn maintain_commit_graph(repo_path: &Path) {
    if let Err(e) = write_commit_graph(repo_path) {
        // Best effort, but not silent: a permanently failing run (e.g. an orphaned
        // lock file) would otherwise stay invisible forever. Once per process is
        // enough — the error would repeat on every fetch/pull.
        static WARNED: std::sync::Once = std::sync::Once::new();
        WARNED.call_once(|| {
            tracing::warn!(
                repo = %repo_path.display(),
                "Commit-graph maintenance failed: {e}"
            );
        });
    }
}

/// Push; automatically creates `<remote>/<branch>` when the upstream is missing.
/// `force` uses `--force-with-lease` (safer than a bare `--force`).
/// Streams the progress to `on` (the app uses push through
/// [`Git2Engine::push_with_progress`](crate::Git2Engine::push_with_progress)).
pub fn push_streaming(
    repo_path: &Path,
    remote: &str,
    branch: Option<&str>,
    has_upstream: bool,
    force: bool,
    cancel: &CancelToken,
    on: &mut dyn FnMut(GitProgress),
) -> Result<String> {
    reject_local_credential_hooks(repo_path)?;
    let lead = leading_config(
        &credential_helper_config(),
        &insecure_tls_configs(repo_path),
    );
    let mut args: Vec<&str> = lead.iter().map(String::as_str).collect();
    args.push("push");
    args.push("--progress");
    if force {
        args.push("--force-with-lease");
    }
    if let (false, Some(name)) = (has_upstream, branch) {
        // remote lands here positionally — same protection as push_to_streaming.
        reject_remote_option(remote)?;
        args.push("--set-upstream");
        args.push("--");
        args.push(remote);
        args.push(name);
    }
    remote_op(run_git_streaming(
        repo_path,
        &args,
        CLONE_TIMEOUT,
        cancel,
        on,
    ))
}

/// Push explicitly to `remote` (not to the upstream), e.g. from the push
/// dropdown. Without a known branch, `HEAD` is pushed.
pub fn push_to(
    repo_path: &Path,
    remote: &str,
    branch: Option<&str>,
    force: bool,
) -> Result<String> {
    push_to_streaming(
        repo_path,
        remote,
        branch,
        force,
        &CancelToken::new(),
        &mut |_| {},
    )
}

/// Like [`push_to`], but streams the progress to `on` and is cancellable.
pub fn push_to_streaming(
    repo_path: &Path,
    remote: &str,
    branch: Option<&str>,
    force: bool,
    cancel: &CancelToken,
    on: &mut dyn FnMut(GitProgress),
) -> Result<String> {
    reject_local_credential_hooks(repo_path)?;
    let lead = leading_config(
        &credential_helper_config(),
        &insecure_tls_configs(repo_path),
    );
    let mut args: Vec<&str> = lead.iter().map(String::as_str).collect();
    args.push("push");
    args.push("--progress");
    if force {
        args.push("--force-with-lease");
    }
    // remote comes from the frontend (`push_remote`) and is free user input.
    // Protected twice against option/argument injection: a leading '-' is
    // rejected AND `--` strictly separates options from the positional arguments,
    // so nothing (e.g. `--receive-pack=…`) gets reinterpreted as a git option.
    reject_remote_option(remote)?;
    args.push("--");
    args.push(remote);
    args.push(branch.unwrap_or("HEAD"));
    remote_op(run_git_streaming(
        repo_path,
        &args,
        CLONE_TIMEOUT,
        cancel,
        on,
    ))
}

/// Validates a clone URL against option and transport injection.
///
/// A URL from the frontend (a paste, a website's clone button) must never be
/// interpreted as a git option (`--upload-pack=…`) and must not select a
/// code-execution transport (`ext::sh -c …`, `fd::`).
pub(crate) fn validate_remote_url(url: &str) -> Result<()> {
    let u = url.trim();
    if u.is_empty() {
        return Err(GitEngineError::Sidecar {
            message: "Empty repository URL".into(),
        });
    }
    // A leading '-' => would be parsed as an option (the `--` separator catches
    // it too, but a clear error beats silent reinterpretation).
    if u.starts_with('-') {
        return Err(GitEngineError::Sidecar {
            message: format!("Invalid URL (starts with '-'): {u}"),
        });
    }
    // Transports that can start arbitrary programs.
    let lower = u.to_ascii_lowercase();
    for bad in ["ext::", "fd::"] {
        if lower.starts_with(bad) {
            return Err(GitEngineError::Sidecar {
                message: "Unsupported transport scheme (possible attack vector)".into(),
            });
        }
    }
    Ok(())
}

/// Rejects a remote coming from the frontend that could be reinterpreted as a git
/// option (leading '-'). `push_remote` passes this value positionally to
/// `git push`; without this check (and the `--` separator) e.g.
/// `--receive-pack=…` would be parsed as an option instead of a remote name.
fn reject_remote_option(remote: &str) -> Result<()> {
    if remote.starts_with('-') {
        return Err(GitEngineError::Sidecar {
            message: format!("Invalid remote (starts with '-'): {remote}"),
        });
    }
    Ok(())
}

/// Validates a branch name coming from the frontend for cloning.
/// Conservative: not empty, no leading '-' (option injection), no `..` and no
/// characters that would break a refspec or act as a glob/special character.
/// `git check-ref-format` is stricter; this allowlist covers the practically
/// relevant, safe cases.
fn validate_branch_name(branch: &str) -> Result<()> {
    let invalid = branch.is_empty()
        || branch.starts_with('-')
        || branch.contains("..")
        || branch.chars().any(|c| {
            c.is_whitespace()
                || (c as u32) < 0x20
                || matches!(c, ':' | '~' | '^' | '?' | '*' | '[' | '\\' | '\u{7f}')
        });
    if invalid {
        return Err(GitEngineError::Sidecar {
            message: format!("Invalid branch name: {branch}"),
        });
    }
    Ok(())
}

// The former single-shot `clone` was removed: cloning runs
// exclusively in two phases through `clone_prepare` + `clone_fetch`.

/// First clone stage ("create and open immediately"): creates the
/// target folder, runs `git init` and sets the `origin` remote — fast and WITHOUT
/// network. The repo can be opened right after; [`clone_fetch`] fetches the data
/// with progress.
pub fn clone_prepare(url: &str, dest_dir: &Path) -> Result<()> {
    validate_remote_url(url)?;
    let nonempty = dest_dir
        .read_dir()
        .map(|mut d| d.next().is_some())
        .unwrap_or(false);
    if nonempty {
        return Err(GitEngineError::Sidecar {
            message: format!(
                "Target folder already exists and is not empty: {}",
                dest_dir.display()
            ),
        });
    }
    std::fs::create_dir_all(dest_dir)?;
    run_git(dest_dir, &["init"])?;
    // The URL is validated (no leading '-') — safe as a positional argument.
    run_git(dest_dir, &["remote", "add", "origin", url])?;
    Ok(())
}

/// Second clone stage: `fetch origin` (with progress, credential helper and the
/// large-repo options), then check out the default branch. Fetch errors leave the
/// repo as a valid, empty local repo (no half state); a failed CHECKOUT is
/// reported as an error — the fetched objects are kept, the worktree may be
/// incomplete.
pub fn clone_fetch(
    path: &Path,
    options: &CloneOptions,
    cancel: &CancelToken,
    on: &mut dyn FnMut(GitProgress),
) -> Result<String> {
    reject_local_credential_hooks(path)?;
    let cred = credential_helper_config();
    let lead = leading_config(&cred, &insecure_tls_configs(path));
    let mut args: Vec<&str> = lead.iter().map(String::as_str).collect();
    args.extend(["fetch", "origin", "--progress"]);
    let depth_arg = options.depth.map(|d| d.max(1).to_string());
    if let Some(d) = depth_arg.as_deref() {
        args.extend(["--depth", d]);
    }
    if options.blobless {
        args.push("--filter=blob:none");
    }
    // Single-branch clone: fetch only that branch, otherwise the full default
    // refspec of the origin remote.
    let branch = options.branch.as_deref().filter(|b| !b.is_empty());
    if let Some(b) = branch {
        validate_branch_name(b)?;
    }
    let refspec = branch.map(|b| format!("+refs/heads/{b}:refs/remotes/origin/{b}"));
    if let Some(rs) = refspec.as_deref() {
        args.push(rs);
    }
    remote_op(run_git_streaming(path, &args, CLONE_TIMEOUT, cancel, on))?;
    maintain_commit_graph(path);

    // For blobless clones (--filter=blob:none) the checkout fetches ALL blobs over
    // the network: the same generous timeout and the same credential/TLS configs
    // as the fetch. Errors are reported instead of swallowed — a strangled
    // checkout would otherwise silently leave `index.lock` and half a worktree
    // behind.
    let checkout = |target: &str| -> Result<String> {
        let mut co: Vec<&str> = lead.iter().map(String::as_str).collect();
        co.extend(["checkout", target]);
        remote_op(run_git_in(path, &co, CLONE_TIMEOUT))
    };
    match branch {
        // Check out exactly the chosen branch (with a single remote, DWIM creates
        // a local tracking branch automatically).
        Some(b) => {
            checkout(b)?;
        }
        // Determine the remote's default branch and check it out (best effort — an
        // empty remote has none and stays a valid empty repo).
        None => {
            if let Some(def) = detect_default_branch(path, &cred) {
                checkout(&def)?;
            }
        }
    }
    Ok(String::new())
}

/// Default branch of the `origin` remote (`refs/remotes/origin/HEAD`). Sets the
/// symbolic ref via `remote set-head --auto` when needed (contacts the remote —
/// hence with the credential helper).
fn detect_default_branch(path: &Path, cred: &Option<String>) -> Option<String> {
    let lead = leading_config(cred, &insecure_tls_configs(path));
    let mut args: Vec<&str> = lead.iter().map(String::as_str).collect();
    args.extend(["remote", "set-head", "origin", "--auto"]);
    let _ = run_git(path, &args);
    let head = run_git(
        path,
        &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
    )
    .ok()?;
    let name = head.trim().strip_prefix("origin/")?.to_string();
    (!name.is_empty() && !name.starts_with('-')).then_some(name)
}

#[cfg(test)]
mod tests {
    use super::{
        classify_remote_error, credential_helper_config, https_host, insecure_hosts,
        leading_config, parse_progress, reject_remote_option, run_git, select_insecure_configs,
        set_insecure_tls_hosts, sh_single_quote, validate_remote_url,
    };

    #[test]
    fn sh_single_quote_escaped_apostrophe() {
        // An apostrophe becomes '\'' and the rest stays inside '…' — the sh line
        // cannot be escaped this way.
        assert_eq!(
            sh_single_quote("C:/Users/O'Brien/x"),
            r"'C:/Users/O'\''Brien/x'"
        );
        // Without an apostrophe, just wrap it.
        assert_eq!(sh_single_quote("/tmp/plan.txt"), "'/tmp/plan.txt'");
    }

    #[test]
    fn reject_remote_option_rejects_leading_minus() {
        // Option injection: a remote coming from the frontend like
        // "--receive-pack=…" must not pass as a git option.
        assert!(reject_remote_option("--force").is_err());
        assert!(reject_remote_option("-x").is_err());
        // Normal remote names and URLs stay allowed.
        assert!(reject_remote_option("origin").is_ok());
        assert!(reject_remote_option("https://example.com/x.git").is_ok());
    }

    #[test]
    fn run_git_smoke_and_error_code() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();
        for args in [
            &["init", "-q"][..],
            &["config", "user.name", "T"],
            &["config", "user.email", "t@t.local"],
        ] {
            assert!(std::process::Command::new("git")
                .arg("-C")
                .arg(path)
                .args(args)
                .status()
                .unwrap()
                .success());
        }
        // status runs cleanly (empty repo → empty output).
        assert!(run_git(path, &["status", "--porcelain"])
            .unwrap()
            .is_empty());
        // Broken remote → a clean sidecar error (no panic), code "sidecar_failed".
        let err = run_git(path, &["fetch", "definitely-no-remote"]).unwrap_err();
        assert_eq!(err.code(), "sidecar_failed");
    }

    #[test]
    fn https_host_extracts_host_without_userinfo_and_port() {
        assert_eq!(
            https_host("https://git.example.com/group/repo.git").as_deref(),
            Some("git.example.com")
        );
        assert_eq!(
            https_host("https://user@git.example.com:8443/x.git").as_deref(),
            Some("git.example.com")
        );
        assert_eq!(
            https_host("http://192.0.2.10/x.git").as_deref(),
            Some("192.0.2.10")
        );
        // Hosts are case-insensitive → return them lower-cased.
        assert_eq!(
            https_host("https://Git.Example.COM/x").as_deref(),
            Some("git.example.com")
        );
        // SSH uses no TLS → no host for sslVerify.
        assert_eq!(https_host("git@git.example.com:group/repo.git"), None);
        assert_eq!(https_host("ssh://git@git.example.com/x.git"), None);
    }

    #[test]
    fn insecure_configs_only_for_opt_in_hosts_and_host_bound() {
        let urls = vec![
            "https://git.intern.example/group/a.git".to_string(),
            // NOT in the opt-in → stays verified (no config).
            "https://github.com/foo/b.git".to_string(),
            // SSH → no TLS, ignored.
            "git@git.intern.example:group/c.git".to_string(),
            // A second remote to the same host → not duplicated.
            "https://git.intern.example/group/d.git".to_string(),
        ];
        let insecure_hosts = vec!["git.intern.example".to_string()];
        assert_eq!(
            select_insecure_configs(&urls, &insecure_hosts),
            vec!["http.https://git.intern.example/.sslVerify=false".to_string()]
        );
        // Without an opt-in host: nothing disabled.
        assert!(select_insecure_configs(&urls, &[]).is_empty());
    }

    #[test]
    fn insecure_hosts_state_beats_env_fallback() {
        // Process-wide state + env: the whole flow deliberately in ONE test
        // (tests run in parallel; no other test touches this state).
        std::env::set_var("TERRA_GIT_INSECURE_TLS_HOSTS", "env.example, ,Two.Example");
        // The setter was never called -> env fallback (trimmed, lower-cased, no empties).
        assert_eq!(insecure_hosts(), vec!["env.example", "two.example"]);

        // The set state wins — normalized as well.
        set_insecure_tls_hosts(vec![" Git.Intern.Example ".into(), String::new()]);
        assert_eq!(insecure_hosts(), vec!["git.intern.example"]);

        // An empty list means "no insecure hosts" — NO env fallback any more.
        set_insecure_tls_hosts(Vec::new());
        assert!(insecure_hosts().is_empty());
        std::env::remove_var("TERRA_GIT_INSECURE_TLS_HOSTS");
    }

    #[test]
    fn leading_config_puts_credential_before_insecure() {
        let none: Option<String> = None;
        assert!(leading_config(&none, &[]).is_empty());

        let cred = Some("credential.helper=!'x' __credential".to_string());
        let insecure = vec!["http.https://h/.sslVerify=false".to_string()];
        assert_eq!(
            leading_config(&cred, &insecure),
            vec![
                "-c".to_string(),
                "credential.helper=!'x' __credential".to_string(),
                "-c".to_string(),
                "http.https://h/.sslVerify=false".to_string(),
            ]
        );
    }

    #[test]
    fn parses_git_progress_lines() {
        let p = parse_progress("Receiving objects:  45% (450/1000), 1.2 MiB | 2.0 MiB/s").unwrap();
        assert_eq!((p.phase.as_str(), p.percent), ("receiving", 45));

        let p = parse_progress("Resolving deltas: 100% (2/2), done.").unwrap();
        assert_eq!((p.phase.as_str(), p.percent), ("resolving", 100));

        // The "remote: " prefix is stripped.
        let p = parse_progress("remote: Compressing objects:   7% (1/14)").unwrap();
        assert_eq!((p.phase.as_str(), p.percent), ("compressing", 7));

        let p = parse_progress("Writing objects:   3% (1/33)").unwrap();
        assert_eq!((p.phase.as_str(), p.percent), ("writing", 3));

        // An unknown phase with a percentage -> "other".
        assert_eq!(parse_progress("Foo bar: 50% (x)").unwrap().phase, "other");
    }

    #[test]
    fn ignores_lines_without_progress() {
        assert_eq!(parse_progress("done."), None);
        assert_eq!(parse_progress("remote: Total 33 (delta 2), reused 0"), None);
        assert_eq!(parse_progress(""), None);
        assert_eq!(parse_progress("Cloning into 'x'..."), None);
    }

    #[test]
    fn credential_helper_only_with_env_and_properly_quoted() {
        // Env variables are process-wide — order matters inside ONE test.
        std::env::remove_var("TERRA_GIT_CREDENTIAL_EXE");
        assert_eq!(
            credential_helper_config(),
            None,
            "no injection without the env var"
        );

        std::env::set_var(
            "TERRA_GIT_CREDENTIAL_EXE",
            r"C:\Program Files\terra-git\tg-app.exe",
        );
        assert_eq!(
            credential_helper_config().as_deref(),
            Some("credential.helper=!'C:/Program Files/terra-git/tg-app.exe' __credential"),
            "leading '!' (otherwise git never starts the helper) + forward slashes + single quotes"
        );

        // Single quote in the path: rather no injection than broken quoting.
        std::env::set_var("TERRA_GIT_CREDENTIAL_EXE", "C:/it's/tg-app.exe");
        assert_eq!(credential_helper_config(), None);
        std::env::remove_var("TERRA_GIT_CREDENTIAL_EXE");
    }

    fn code_of(text: &str) -> Option<&'static str> {
        classify_remote_error(text).map(|(c, _)| c)
    }

    #[test]
    fn classifies_remote_errors() {
        assert_eq!(
            code_of("! [rejected] main -> main (non-fast-forward)"),
            Some("non_fast_forward")
        );
        assert_eq!(
            code_of("! [rejected] main -> main (stale info)"),
            Some("force_lease_stale")
        );
        assert_eq!(
            code_of("fatal: Authentication failed for 'https://host/x.git'"),
            Some("auth_failed")
        );
        assert_eq!(
            code_of("remote: HTTP Basic: Access denied\nfatal: unable to access"),
            Some("forbidden")
        );
        assert_eq!(
            code_of("git@host: Permission denied (publickey)."),
            Some("ssh_auth")
        );
        assert_eq!(code_of("Host key verification failed."), Some("host_key"));
        assert_eq!(
            code_of("fatal: repository 'https://host/x.git' not found"),
            Some("repo_not_found")
        );
        assert_eq!(
            code_of("fatal: The current branch main has no upstream branch."),
            Some("no_upstream")
        );
        assert_eq!(
            code_of("fatal: Could not resolve host: gitlab.invalid"),
            Some("network")
        );
        assert_eq!(code_of("Everything up to date, no error"), None);
    }

    #[test]
    fn prioritizes_forbidden_over_generic_403() {
        // Contains both "403" and "pre-receive hook declined" -> forbidden.
        assert_eq!(
            code_of("remote: error: GH006: pre-receive hook declined\nHTTP 403"),
            Some("forbidden")
        );
    }

    #[test]
    fn incidental_403_in_progress_or_sha_is_not_forbidden() {
        // A substring "403" in progress counters or object names must not
        // classify a transfer abort as "forbidden".
        assert_eq!(code_of("Receiving objects:  3% (1403/44000)"), None);
        assert_eq!(
            code_of("Receiving objects:  3% (1403/44000)\nfatal: early EOF"),
            None
        );
        // A SHA prefix directly after "error: " — not an HTTP status line.
        assert_eq!(code_of("error: 403f9ab is not a valid object name"), None);
    }

    #[test]
    fn real_403_lines_stay_forbidden() {
        assert_eq!(
            code_of(
                "fatal: unable to access 'https://host/x.git/': \
                 The requested URL returned error: 403"
            ),
            Some("forbidden")
        );
        assert_eq!(
            code_of("error: RPC failed; HTTP 403 curl 22 The requested URL returned error: 403"),
            Some("forbidden")
        );
        assert_eq!(code_of("remote: 403 Forbidden"), Some("forbidden"));
    }

    #[test]
    fn clone_url_rejects_injection() {
        // Option injection
        assert!(validate_remote_url("--upload-pack=calc.exe").is_err());
        assert!(validate_remote_url("-x").is_err());
        // Code-execution transports
        assert!(validate_remote_url("ext::sh -c whoami").is_err());
        assert!(validate_remote_url("EXT::sh -c whoami").is_err());
        assert!(validate_remote_url("fd::17").is_err());
        // Empty
        assert!(validate_remote_url("   ").is_err());
    }

    #[test]
    fn clone_url_accepts_valid_ones() {
        assert!(validate_remote_url("https://github.com/x/y.git").is_ok());
        assert!(validate_remote_url("http://192.0.2.10/a/b.git").is_ok());
        assert!(validate_remote_url("git@192.0.2.10:acme/terra-git.git").is_ok());
        assert!(validate_remote_url("ssh://git@host:22/x/y.git").is_ok());
        assert!(validate_remote_url("file:///srv/repos/x.git").is_ok());
    }
}
