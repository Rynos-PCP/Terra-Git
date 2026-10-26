//! SSH key management + known_hosts TOFU through the OpenSSH CLIs
//! (ssh-keygen, ssh-keyscan). No crypto code of our own.

use std::io::Read;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use tg_domain::{ScannedHost, SshHostFingerprint, SshKey};

use crate::error::{GitEngineError, Result};

/// Short constructor for an SSH error with a stable frontend code.
fn ssh_err(code: &'static str, message: impl Into<String>) -> GitEngineError {
    GitEngineError::Ssh {
        code,
        message: message.into(),
    }
}

/// ~/.ssh (Windows: %USERPROFILE%\.ssh, otherwise $HOME/.ssh).
fn home_ssh_dir() -> Result<PathBuf> {
    let home = if cfg!(windows) {
        std::env::var_os("USERPROFILE")
    } else {
        std::env::var_os("HOME")
    };
    home.map(|h| PathBuf::from(h).join(".ssh"))
        .ok_or_else(|| ssh_err("ssh_no_home", "No home directory found"))
}

/// File name of a key to create/delete: file name only, no traversal, no
/// option injection.
///
/// IMPORTANT: reject pure dot names (`.`, `..`, …) — `.` would otherwise point
/// at `~/.ssh` ITSELF, and `remove_key(".")` would move the ENTIRE `~/.ssh`
/// (all keys, known_hosts, config) to the trash.
fn valid_key_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('-')
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains("..")
        && !name.chars().all(|c| c == '.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// Host name/IP(/port) for ssh-keyscan: no leading '-' (option injection), no
/// whitespace, tight allowlist.
fn valid_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 255
        && !host.starts_with('-')
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '_' | '[' | ']'))
}

/// known_hosts host form: `[host]:port` for a custom port, otherwise `host`.
/// ssh-keyscan/ssh-keygen -F/-R expect exactly this form for non-default ports.
fn expected_host_form(host: &str, port: Option<u16>) -> String {
    match port {
        Some(p) => format!("[{host}]:{p}"),
        None => host.to_string(),
    }
}

/// The ssh tools git itself uses. Git for Windows starts its own ssh (in
/// `usr/bin` of the git installation), not the Windows one in System32 — a
/// different version with different key exchanges, so the trust dialog has to
/// ask the same program the fetch will run. Elsewhere: the one on PATH.
fn ssh_tool(name: &'static str) -> String {
    #[cfg(windows)]
    {
        static GIT_USR_BIN: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
        let dir = GIT_USR_BIN.get_or_init(|| {
            let out = Command::new("git").arg("--exec-path").output().ok()?;
            let exec = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
            // <git>/mingw64/libexec/git-core → <git>/usr/bin
            exec.ancestors()
                .nth(3)
                .map(|git| git.join("usr").join("bin"))
                .filter(|d| d.join("ssh.exe").is_file())
        });
        if let Some(tool) = dir.as_ref().map(|d| d.join(format!("{name}.exe"))) {
            if tool.is_file() {
                return tool.to_string_lossy().into_owned();
            }
        }
    }
    name.to_string()
}

/// `ssh -G` output → (hostname, port, hostkeyalias).
fn parse_ssh_config_dump(out: &str) -> (Option<String>, Option<u16>, Option<String>) {
    let (mut hostname, mut port, mut alias) = (None, None, None);
    for line in out.lines() {
        let Some((key, value)) = line.trim().split_once(' ') else {
            continue;
        };
        let value = value.trim();
        match key {
            "hostname" => hostname = Some(value.to_string()),
            "port" => port = value.parse().ok(),
            "hostkeyalias" if !value.eq_ignore_ascii_case("none") => {
                alias = Some(value.to_string());
            }
            _ => {}
        }
    }
    (hostname, port, alias)
}

/// The name ssh records `host` under in known_hosts: `HostKeyAlias` if set
/// (without a port, as ssh does), else the `HostName` with `[…]:port` for a
/// port other than 22.
fn recorded_name(hostname: &str, port: u16, alias: Option<&str>) -> String {
    match alias {
        Some(a) => a.to_string(),
        None if port == 22 => hostname.to_string(),
        None => format!("[{hostname}]:{port}"),
    }
}

/// Where ssh connects for `host` and the name it records the key under, after
/// the user's ssh_config (`ssh -G`).
struct Resolved {
    /// The known_hosts name (see [`recorded_name`]).
    name: String,
    hostname: String,
    /// `None` for ssh's default port.
    port: Option<u16>,
}

/// How ssh itself names `host` in known_hosts, after the user's ssh_config
/// (`ssh -G`). For `Host gh-work` → `HostName github.com` ssh records and looks
/// up `github.com`; asking for `gh-work` never matched what it wrote, and the
/// trust dialog could never save the key. Falls back to the plain form when
/// `ssh -G` is unavailable.
fn resolve_host(host: &str, port: Option<u16>) -> Resolved {
    let plain = || Resolved {
        name: expected_host_form(host, port),
        hostname: host.to_string(),
        port,
    };
    let mut args = vec!["-G".to_string()];
    if let Some(p) = port {
        args.extend(["-p".to_string(), p.to_string()]);
    }
    args.extend(["--".to_string(), host.to_string()]);
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let dump = run_capture(&ssh_tool("ssh"), &refs, None, Duration::from_secs(10))
        .ok()
        .filter(|(ok, _, _)| *ok)
        .map(|(_, out, _)| parse_ssh_config_dump(&out));
    let Some((Some(hostname), Some(p), alias)) = dump else {
        return plain();
    };
    let name = recorded_name(&hostname, p, alias.as_deref());
    if !valid_host(&hostname) || !valid_host(&name) {
        return plain();
    }
    Resolved {
        name,
        hostname,
        port: (p != 22).then_some(p),
    }
}

/// Extracts the bare host from a known_hosts host form: `[host]:port` -> `host`,
/// otherwise unchanged.
fn bare_host_of(expected: &str) -> &str {
    if let Some(rest) = expected.strip_prefix('[') {
        if let Some(idx) = rest.rfind("]:") {
            return &rest[..idx];
        }
    }
    expected
}

/// Builds the `ssh-keyscan` arguments: `-T 5 [-p <port>] <host>`. The port goes
/// in as its own u16 argv value (no injection possible).
fn keyscan_args(host: &str, port: Option<u16>) -> Vec<String> {
    let mut args = vec!["-T".to_string(), "5".to_string()];
    if let Some(p) = port {
        args.push("-p".to_string());
        args.push(p.to_string());
    }
    args.push(host.to_string());
    args
}

/// known_hosts lines with their host field replaced by `name`; comments and
/// empty lines dropped.
fn rename_known_hosts_lines(lines: &str, name: &str) -> String {
    lines
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_once(char::is_whitespace))
        .map(|(_, rest)| format!("{name} {}", rest.trim_start()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Binds the known_hosts lines supplied by the frontend to `expected_host` (the
/// known_hosts host form). Only this prevents a faulty/compromised renderer from
/// smuggling a wildcard or foreign-host entry (a persistent MITM primitive) into
/// `known_hosts`.
///
/// Rules per line: trimmed; empty/`#` comment lines are skipped. `@` markers
/// (@cert-authority/@revoked) and hashed `|1|` entries cannot be bound to a
/// concrete host -> error. The host field (first token) must not contain `*`/`?`
/// (wildcard); every comma-separated part has to equal (case-insensitively)
/// `expected_host` or the bare host. If no valid line remains at the end ->
/// error.
fn validate_known_hosts_lines(lines: &str, expected_host: &str) -> Result<Vec<String>> {
    let bare = bare_host_of(expected_host);
    let mut out = Vec::new();
    for raw in lines.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('@') || line.starts_with('|') {
            return Err(ssh_err(
                "ssh_untrusted_line",
                "known_hosts line cannot be bound to the host (marker/hashed)",
            ));
        }
        let host_field = line.split_whitespace().next().unwrap_or("");
        if host_field.contains('*') || host_field.contains('?') {
            return Err(ssh_err(
                "ssh_untrusted_line",
                "known_hosts line contains a host pattern (wildcard)",
            ));
        }
        for part in host_field.split(',') {
            if !part.eq_ignore_ascii_case(expected_host) && !part.eq_ignore_ascii_case(bare) {
                return Err(ssh_err(
                    "ssh_untrusted_line",
                    "known_hosts line does not belong to the requested host",
                ));
            }
        }
        out.push(line.to_string());
    }
    if out.is_empty() {
        return Err(ssh_err(
            "ssh_untrusted_line",
            "No known_hosts line matching the host",
        ));
    }
    Ok(out)
}

/// Splits a `.pub` line into (type, comment). Format: `<type> <base64> [comment]`.
fn parse_pub_line(line: &str) -> Option<(String, String)> {
    let mut it = line.split_whitespace();
    let key_type = it.next()?.to_string();
    let _b64 = it.next()?;
    let comment = it.collect::<Vec<_>>().join(" ");
    Some((key_type, comment))
}

/// Splits an `ssh-keygen -lf` line (`256 SHA256:xxxx comment (ED25519)`) into
/// (sha256, key_type). key_type comes from the parentheses at the end.
fn parse_fingerprint_line(line: &str) -> Option<(String, String)> {
    let sha = line.split_whitespace().find(|t| t.starts_with("SHA256:"))?;
    let key_type = line
        .rsplit('(')
        .next()
        .and_then(|s| s.strip_suffix(')'))
        .unwrap_or("")
        .to_string();
    Some((sha.to_string(), key_type))
}

/// Copies `args` for the debug log and replaces sensitive values with `***`:
/// the value AFTER `-N` (new passphrase) or `-P` (current passphrase,
/// `ssh-keygen -p`) must never end up in the persistent log file — that file is
/// attached to bug reports as the crash hint says. The real argv stays unchanged.
fn redact_args(args: &[&str]) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len());
    let mut redact_next = false;
    for a in args {
        if redact_next {
            out.push("***".to_string());
            redact_next = false;
        } else {
            redact_next = matches!(*a, "-N" | "-P");
            out.push((*a).to_string());
        }
    }
    out
}

/// Runs an OpenSSH CLI with output capture (pattern `sidecar.rs::run_git_impl_env`):
/// no console window flashing up on Windows, a hard timeout, stdout/stderr are
/// drained in threads, and `stdin` is optionally written into the process.
/// Returns `(success, stdout, stderr)`. A missing program (OpenSSH not installed)
/// is reported as `ssh_tool_missing`.
fn run_capture(
    program: &str,
    args: &[&str],
    stdin: Option<&str>,
    timeout: Duration,
) -> Result<(bool, String, String)> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // No console window flashing up on Windows (CREATE_NO_WINDOW).
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }

    // Only log the redacted copy (passphrases after -N/-P never reach the log file).
    tracing::debug!(program, args = ?redact_args(args), "ssh: capture");
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(ssh_err(
                "ssh_tool_missing",
                "OpenSSH (ssh-keygen/ssh-keyscan) not found",
            ));
        }
        Err(e) => return Err(e.into()),
    };

    // Write stdin (if present) and close it so the process does not wait for
    // further input.
    if let Some(data) = stdin {
        if let Some(mut pipe) = child.stdin.take() {
            let _ = pipe.write_all(data.as_bytes());
            // pipe drops out of scope here -> gets closed (EOF).
        }
    }

    // Drain the pipes in threads so the child process never blocks on a full
    // pipe buffer while we wait for it. Bounded: before any trust, a server
    // decides what ssh prints (its banner, for one).
    let mut stdout_pipe = child.stdout.take();
    let mut stderr_pipe = child.stderr.take();
    let out_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(p) = stdout_pipe.as_mut() {
            buf = crate::sidecar::read_tail(p, 1024 * 1024);
        }
        buf
    });
    let err_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(p) = stderr_pipe.as_mut() {
            buf = crate::sidecar::read_tail(p, crate::sidecar::STDERR_KEEP);
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
                return Err(ssh_err(
                    "ssh_timeout",
                    format!("{program} aborted after {} s (timeout)", timeout.as_secs()),
                ));
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    };

    let stdout_buf = out_reader.join().unwrap_or_default();
    let stderr_buf = err_reader.join().unwrap_or_default();
    let stdout = String::from_utf8_lossy(&stdout_buf).into_owned();
    let stderr = String::from_utf8_lossy(&stderr_buf).into_owned();
    Ok((status.success(), stdout, stderr))
}

/// Determines the fingerprint (SHA256) and key type of a .pub file via
/// `ssh-keygen -lf`.
fn fingerprint_of(pub_path: &Path) -> Result<(String, String)> {
    let path = pub_path.to_string_lossy();
    let (ok, stdout, _stderr) =
        run_capture("ssh-keygen", &["-lf", &path], None, Duration::from_secs(10))?;
    if !ok {
        return Ok((String::new(), String::new()));
    }
    let first = stdout.lines().next().unwrap_or("");
    Ok(parse_fingerprint_line(first).unwrap_or_default())
}

/// Lists the local SSH keys (all `~/.ssh/*.pub`). If `~/.ssh` is missing, an
/// empty list is returned (not an error).
pub fn list_keys() -> Result<Vec<SshKey>> {
    let dir = home_ssh_dir()?;
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };

    let mut keys = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("pub") {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let line = raw.trim();
        let Some((key_type, comment)) = parse_pub_line(line) else {
            continue;
        };
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let (fingerprint, _) = fingerprint_of(&path).unwrap_or_default();
        keys.push(SshKey {
            name,
            key_type,
            comment,
            public_key: line.to_string(),
            fingerprint,
        });
    }
    keys.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(keys)
}

/// Ensures `~/.ssh` exists (with mode 0700 on Unix).
fn ensure_ssh_dir(dir: &Path) -> Result<()> {
    if dir.exists() {
        return Ok(());
    }
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)?;
    Ok(())
}

/// Creates a new ed25519 key `~/.ssh/<name>` and returns it as an [`SshKey`].
/// Existing keys are never overwritten.
pub fn generate_key(name: &str, comment: &str, passphrase: &str) -> Result<SshKey> {
    if !valid_key_name(name) {
        return Err(ssh_err("invalid_key_name", "Invalid key name"));
    }
    let dir = home_ssh_dir()?;
    let key_path = dir.join(name);
    let pub_path = dir.join(format!("{name}.pub"));
    if key_path.exists() || pub_path.exists() {
        return Err(ssh_err(
            "ssh_key_exists",
            "A key with this name already exists",
        ));
    }
    ensure_ssh_dir(&dir)?;

    let key_str = key_path.to_string_lossy().into_owned();
    // name/comment/passphrase go to the CLI as their own argv values (no shell)
    // — `-f`/`-C`/`-N` consume them positionally, no option injection.
    let (ok, _stdout, stderr) = run_capture(
        "ssh-keygen",
        &[
            "-t", "ed25519", "-f", &key_str, "-C", comment, "-N", passphrase,
        ],
        None,
        Duration::from_secs(20),
    )?;
    if !ok {
        return Err(ssh_err(
            "ssh_keygen_failed",
            format!("Key generation failed: {}", stderr.trim()),
        ));
    }

    let raw = std::fs::read_to_string(&pub_path)?;
    let line = raw.trim();
    let (key_type, kcomment) = parse_pub_line(line).unwrap_or_default();
    let (fingerprint, _) = fingerprint_of(&pub_path).unwrap_or_default();
    Ok(SshKey {
        name: name.to_string(),
        key_type,
        comment: kcomment,
        public_key: line.to_string(),
        fingerprint,
    })
}

/// Determines a key's two paths (`~/.ssh/<name>` private + `<name>.pub`).
/// Validates the name with the same strict allowlist as on creation — essential
/// here because files get deleted afterwards: no traversal (`..`, `/`, `\`), no
/// leading `-` (option injection).
fn key_paths(name: &str) -> Result<(PathBuf, PathBuf)> {
    if !valid_key_name(name) {
        return Err(ssh_err("invalid_key_name", "Invalid key name"));
    }
    let dir = home_ssh_dir()?;
    Ok((dir.join(name), dir.join(format!("{name}.pub"))))
}

/// Moves an SSH key (private + `.pub` part) to the trash and best-effort removes
/// it from the running ssh-agent. A missing half is tolerated. Trash instead of
/// a hard delete -> recoverable.
pub fn remove_key(name: &str) -> Result<()> {
    let (key_path, pub_path) = key_paths(name)?;

    // Best effort: take it out of the agent before the file disappears. Errors
    // (no agent running, key not loaded at all) do not matter here.
    let _ = run_capture(
        "ssh-add",
        &["-d", &pub_path.to_string_lossy()],
        None,
        Duration::from_secs(5),
    );

    for p in [&key_path, &pub_path] {
        if p.exists() {
            trash::delete(p).map_err(|e| {
                ssh_err("ssh_remove_failed", format!("Moving to trash failed: {e}"))
            })?;
        }
    }
    Ok(())
}

/// Scans the host keys of `host` (`ssh-keyscan`) and returns the fingerprints
/// plus the exact known_hosts lines (the TOFU basis). `changed` = a (possibly
/// differing) known_hosts entry already exists.
pub fn scan_host(host: &str, port: Option<u16>) -> Result<ScannedHost> {
    if !valid_host(host) {
        return Err(ssh_err("invalid_host", "Invalid host name"));
    }
    let resolved = resolve_host(host, port);
    let expected = resolved.name.clone();
    // ssh-keyscan reads no ssh_config: it gets the host and port ssh resolved.
    let args = keyscan_args(&resolved.hostname, resolved.port);
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    // First ssh itself — the program git uses, so the key comes through the
    // same key exchange the fetch will make. ssh-keyscan only as the fallback:
    // the Windows build dies on "choose_kex: unsupported KEX method
    // sntrup761x25519-sha512" against an OpenSSH 10 server and still takes
    // its full timeout (user report).
    let known_hosts_lines = match host_key_via_ssh(host, port) {
        Ok(lines) => lines,
        Err(_) => run_capture(
            &ssh_tool("ssh-keyscan"),
            &arg_refs,
            None,
            Duration::from_secs(15),
        )
        .ok()
        .filter(|(ok, out, _)| *ok && !out.trim().is_empty())
        // Named the way ssh records them (a HostKeyAlias, say).
        .map(|(_, out, _)| rename_known_hosts_lines(out.trim(), &expected))
        .ok_or_else(|| ssh_err("ssh_keyscan_failed", "Could not retrieve host keys"))?,
    };

    // Fingerprints from the known_hosts lines (ssh-keygen -lf - reads stdin).
    let mut fingerprints = Vec::new();
    if let Ok((fp_ok, fp_out, _)) = run_capture(
        &ssh_tool("ssh-keygen"),
        &["-lf", "-"],
        Some(&known_hosts_lines),
        Duration::from_secs(10),
    ) {
        if fp_ok {
            for line in fp_out.lines() {
                if let Some((sha256, key_type)) = parse_fingerprint_line(line) {
                    fingerprints.push(SshHostFingerprint { key_type, sha256 });
                }
            }
        }
    }

    // Does a known_hosts entry already exist for the host (known_hosts form)?
    let changed = run_capture(
        &ssh_tool("ssh-keygen"),
        &["-F", &expected],
        None,
        Duration::from_secs(10),
    )
    .map(|(_, out, _)| !out.trim().is_empty())
    .unwrap_or(false);

    Ok(ScannedHost {
        host: expected,
        changed,
        fingerprints,
        known_hosts_lines,
    })
}

/// Options for [`host_key_via_ssh`]: every authentication method off (the
/// server is not trusted yet — it gets offered nothing), no prompt, the key
/// recorded in plain text into `record` only, nothing read from the user's own
/// known_hosts (`none` is an empty, never written file).
fn probe_ssh_args(host: &str, port: Option<u16>, record: &Path, none: &Path) -> Vec<String> {
    // Quoted: a user name with a space in the temp path would otherwise split
    // the option into two file names.
    let file = |p: &Path| format!("\"{}\"", p.to_string_lossy().replace('\\', "/"));
    let mut args: Vec<String> = [
        "BatchMode=yes",
        "StrictHostKeyChecking=accept-new",
        "HashKnownHosts=no",
        "UpdateHostKeys=no",
        "PubkeyAuthentication=no",
        "PasswordAuthentication=no",
        "KbdInteractiveAuthentication=no",
        "GSSAPIAuthentication=no",
        "HostbasedAuthentication=no",
        // Record the key under the host name only: a `host,ip` line would not
        // pass the host binding of trust_host.
        "CheckHostIP=no",
        // An untrusted server gets nothing forwarded.
        "ForwardAgent=no",
        "ClearAllForwardings=yes",
        "ConnectTimeout=10",
    ]
    .iter()
    .flat_map(|o| ["-o".to_string(), (*o).to_string()])
    .collect();
    args.extend([
        "-o".to_string(),
        format!("UserKnownHostsFile={}", file(record)),
        "-o".to_string(),
        format!("GlobalKnownHostsFile={}", file(none)),
    ]);
    if let Some(p) = port {
        args.extend(["-p".to_string(), p.to_string()]);
    }
    args.extend(["--".to_string(), host.to_string(), "exit".to_string()]);
    args
}

/// A fresh directory only this process owns. `create_dir` fails on anything
/// that already exists — a directory planted in a shared `/tmp`, or a symlink
/// to `~/.ssh` — and then the next name is tried; 0700 on Unix. The probe thus
/// reads back only what its own ssh wrote.
fn private_temp_dir(prefix: &str) -> std::io::Result<PathBuf> {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    for _ in 0..16 {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("{prefix}-{}-{nanos:x}-{n}", std::process::id()));
        #[cfg(unix)]
        let builder = {
            use std::os::unix::fs::DirBuilderExt;
            let mut b = std::fs::DirBuilder::new();
            b.mode(0o700);
            b
        };
        #[cfg(not(unix))]
        let builder = std::fs::DirBuilder::new();
        match builder.create(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "no free temporary directory",
    ))
}

/// Reads at most `limit` bytes of a text file (lossy).
fn read_bounded(path: &Path, limit: u64) -> std::io::Result<String> {
    let mut buf = Vec::new();
    std::fs::File::open(path)?
        .take(limit)
        .read_to_end(&mut buf)?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// The host's key as ssh — the program git uses — sees it: a connection with
/// all authentication off that is refused after the key exchange, the key
/// recorded in a throw-away known_hosts file. Neither the user's known_hosts
/// nor their keys are touched.
fn host_key_via_ssh(host: &str, port: Option<u16>) -> Result<String> {
    let dir = private_temp_dir("terra-git-hostkey")?;
    let record = dir.join("known_hosts");
    let args = probe_ssh_args(host, port, &record, &dir.join("none"));
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    // The refusal ("Permission denied") is expected — only the record counts.
    let _ = run_capture(&ssh_tool("ssh"), &arg_refs, None, Duration::from_secs(20));
    let lines = read_bounded(&record, 64 * 1024).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&dir);
    let lines: Vec<&str> = lines
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    if lines.is_empty() {
        return Err(ssh_err(
            "ssh_keyscan_failed",
            "Could not retrieve host keys",
        ));
    }
    Ok(lines.join("\n"))
}

/// Adds `lines` to `~/.ssh/known_hosts` (TOFU). With `replace`, an existing
/// entry for `host` is removed first (`ssh-keygen -R`).
pub fn trust_host(host: &str, port: Option<u16>, lines: &str, replace: bool) -> Result<()> {
    if !valid_host(host) {
        return Err(ssh_err("invalid_host", "Invalid host name"));
    }
    let expected = resolve_host(host, port).name;
    // Bind the lines to the host BEFORE any file/process effect: if validation
    // fails, known_hosts is neither created nor changed.
    let validated = validate_known_hosts_lines(lines, &expected)?;

    if replace {
        // Remove the old entry; tolerate errors (there may be no entry).
        let _ = run_capture(
            &ssh_tool("ssh-keygen"),
            &["-R", &expected],
            None,
            Duration::from_secs(10),
        );
    }
    let dir = home_ssh_dir()?;
    ensure_ssh_dir(&dir)?;
    let known_hosts = dir.join("known_hosts");

    // Make sure existing content ends with '\n' before appending.
    let needs_nl = match std::fs::read(&known_hosts) {
        Ok(content) => !content.is_empty() && content.last() != Some(&b'\n'),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => return Err(e.into()),
    };

    let mut opts = std::fs::OpenOptions::new();
    opts.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts.open(&known_hosts)?;
    if needs_nl {
        file.write_all(b"\n")?;
    }
    // Only write the validated lines bound to the host.
    file.write_all(validated.join("\n").as_bytes())?;
    file.write_all(b"\n")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ssh_probe_offers_nothing_and_writes_only_its_own_file() {
        let record = Path::new("C:/Users/Max Mustermann/AppData/Local/Temp/x/known_hosts");
        let none = Path::new("C:/Users/Max Mustermann/AppData/Local/Temp/x/none");
        let args = probe_ssh_args("192.0.2.12", Some(2222), record, none).join(" ");
        for off in [
            "PubkeyAuthentication=no",
            "PasswordAuthentication=no",
            "BatchMode=yes",
        ] {
            assert!(args.contains(off), "{args}");
        }
        // Quoted, so the space in the user name does not split the path.
        assert!(args.contains(
            "UserKnownHostsFile=\"C:/Users/Max Mustermann/AppData/Local/Temp/x/known_hosts\""
        ));
        assert!(args.ends_with("-p 2222 -- 192.0.2.12 exit"), "{args}");
    }

    #[test]
    fn key_name_validation_blocks_traversal_and_options() {
        assert!(valid_key_name("id_ed25519"));
        assert!(valid_key_name("work.key-1"));
        assert!(!valid_key_name(""));
        assert!(!valid_key_name("-x")); // option injection
        assert!(!valid_key_name("../evil")); // Traversal
        assert!(!valid_key_name("a/b")); // Slash
        assert!(!valid_key_name("a\\b")); // Backslash
        assert!(!valid_key_name("bad name")); // Space
    }

    #[test]
    fn key_paths_rejects_invalid_names() {
        // The guard has to bite BEFORE any path resolution/deletion (traversal /
        // option injection / slash), otherwise remove_key could trash foreign
        // files. These cases fail in valid_key_name before home_ssh_dir() is even
        // called (hence no HOME setup needed).
        assert_eq!(key_paths("-x").unwrap_err().code(), "invalid_key_name");
        assert_eq!(key_paths("../evil").unwrap_err().code(), "invalid_key_name");
        assert_eq!(key_paths("a/b").unwrap_err().code(), "invalid_key_name");
        assert_eq!(key_paths("a\\b").unwrap_err().code(), "invalid_key_name");
        assert_eq!(key_paths("").unwrap_err().code(), "invalid_key_name");
        // Pure dot names: "." would otherwise point at ~/.ssh itself -> deleting
        // would trash the WHOLE directory.
        assert_eq!(key_paths(".").unwrap_err().code(), "invalid_key_name");
        assert_eq!(key_paths("..").unwrap_err().code(), "invalid_key_name");
        assert_eq!(key_paths("...").unwrap_err().code(), "invalid_key_name");
    }

    #[test]
    fn key_paths_builds_private_and_pub_path() {
        // Valid name -> both paths end in <name> and <name>.pub in the same
        // directory. (Directory = ~/.ssh; only the endings are checked here so
        // HOME does not have to be redirected process-globally.)
        if let Ok((priv_p, pub_p)) = key_paths("work-1") {
            assert_eq!(priv_p.file_name().unwrap(), "work-1");
            assert_eq!(pub_p.file_name().unwrap(), "work-1.pub");
            assert_eq!(priv_p.parent(), pub_p.parent());
        }
        // No HOME -> an error is fine too (CI without a home); the guard test
        // above covers the security-critical path.
    }

    #[test]
    fn host_validation_blocks_options_and_metacharacters() {
        assert!(valid_host("github.com"));
        assert!(valid_host("192.0.2.10"));
        assert!(valid_host("[2001:db8::1]"));
        assert!(!valid_host(""));
        assert!(!valid_host("-oProxyCommand=calc")); // option injection
        assert!(!valid_host("a b")); // Space
        assert!(!valid_host("a;b")); // shell metacharacters
    }

    #[test]
    fn pub_line_is_split() {
        let (t, c) = parse_pub_line("ssh-ed25519 AAAAC3Nz user@host").unwrap();
        assert_eq!(t, "ssh-ed25519");
        assert_eq!(c, "user@host");
        let (t2, c2) = parse_pub_line("ssh-rsa AAAAB3").unwrap(); // without a comment
        assert_eq!(t2, "ssh-rsa");
        assert_eq!(c2, "");
    }

    #[test]
    fn redact_hides_passphrases_after_n_and_p() {
        // generate_key pattern: the value after -N is the passphrase.
        let red = redact_args(&["-t", "ed25519", "-f", "/k", "-C", "comment", "-N", "secret"]);
        assert_eq!(
            red,
            vec!["-t", "ed25519", "-f", "/k", "-C", "comment", "-N", "***"]
        );
        // ssh-keygen -p (change passphrase): -P carries the current one.
        let red = redact_args(&["-p", "-P", "old", "-N", "new", "-f", "/k"]);
        assert_eq!(red, vec!["-p", "-P", "***", "-N", "***", "-f", "/k"]);
    }

    #[test]
    fn redact_leaves_harmless_args_and_edges_intact() {
        // No sensitive flag -> unchanged.
        assert_eq!(redact_args(&["-lf", "/x.pub"]), vec!["-lf", "/x.pub"]);
        // -N as the last argument (no value) -> no panic, nothing swallowed.
        assert_eq!(redact_args(&["-N"]), vec!["-N"]);
        // A value that itself looks like a flag is redacted as a value and NOT
        // interpreted as a new flag (otherwise "x" would stay hidden).
        assert_eq!(redact_args(&["-N", "-P", "x"]), vec!["-N", "***", "x"]);
    }

    #[test]
    fn missing_cli_returns_stable_code_ssh_tool_missing() {
        // A program guaranteed not to exist -> the NotFound branch.
        let err = run_capture(
            "tg-definitely-no-ssh-binary-xyz",
            &["--version"],
            None,
            Duration::from_secs(5),
        )
        .expect_err("a missing CLI has to yield an error");
        // The stable frontend code has to be "ssh_tool_missing", not
        // "invalid_operation" (otherwise err.ssh_tool_missing never applies).
        assert_eq!(err.code(), "ssh_tool_missing");
    }

    #[test]
    fn fingerprint_line_is_split() {
        let (sha, t) = parse_fingerprint_line("256 SHA256:abc123 user@host (ED25519)").unwrap();
        assert_eq!(sha, "SHA256:abc123");
        assert_eq!(t, "ED25519");
    }

    // ---- B2: known_hosts host form + ssh-keyscan argument building ----

    #[test]
    fn expected_host_form_with_and_without_port() {
        assert_eq!(expected_host_form("github.com", None), "github.com");
        assert_eq!(
            expected_host_form("github.com", Some(2222)),
            "[github.com]:2222"
        );
        assert_eq!(
            expected_host_form("2001:db8::1", Some(22)),
            "[2001:db8::1]:22"
        );
    }

    #[test]
    fn the_known_hosts_name_follows_ssh_config() {
        let dump = "user git\nhostname github.com\nport 2222\nhostkeyalias none\n";
        let (h, p, a) = parse_ssh_config_dump(dump);
        assert_eq!(
            (h.as_deref(), p, a.as_deref()),
            (Some("github.com"), Some(2222), None)
        );
        assert_eq!(recorded_name("github.com", 22, None), "github.com");
        assert_eq!(recorded_name("github.com", 2222, None), "[github.com]:2222");
        // HostKeyAlias: the alias, without a port — as ssh records it.
        assert_eq!(recorded_name("10.1.2.3", 2222, Some("work")), "work");
        assert_eq!(
            rename_known_hosts_lines("# c\n10.1.2.3 ssh-ed25519 AAAA\n\n", "work"),
            "work ssh-ed25519 AAAA"
        );
    }

    #[test]
    fn an_unconfigured_host_resolves_to_the_plain_form() {
        // The explicit default port is no part of the name ssh records.
        let r = resolve_host("terra-git-test.invalid", Some(22));
        assert_eq!(r.name, "terra-git-test.invalid");
        assert_eq!(r.port, None);
        let r = resolve_host("terra-git-test.invalid", Some(2222));
        assert_eq!(r.name, "[terra-git-test.invalid]:2222");
    }

    #[test]
    fn bare_host_from_known_hosts_form() {
        assert_eq!(bare_host_of("github.com"), "github.com");
        assert_eq!(bare_host_of("[github.com]:2222"), "github.com");
        assert_eq!(bare_host_of("[2001:db8::1]:22"), "2001:db8::1");
    }

    #[test]
    fn keyscan_args_built_with_port() {
        let args = keyscan_args("example.org", Some(2222));
        assert_eq!(args, vec!["-T", "5", "-p", "2222", "example.org"]);
        assert!(args.iter().any(|a| a == "-p"));
        assert!(args.iter().any(|a| a == "2222"));
        // expected_host = "[host]:port"
        assert_eq!(
            expected_host_form("example.org", Some(2222)),
            "[example.org]:2222"
        );
    }

    #[test]
    fn keyscan_args_without_port_have_no_p() {
        let args = keyscan_args("example.org", None);
        assert_eq!(args, vec!["-T", "5", "example.org"]);
        assert!(!args.iter().any(|a| a == "-p"));
    }

    // ---- B1: bind known_hosts lines to the host ----

    #[test]
    fn wildcard_line_is_rejected() {
        let err = validate_known_hosts_lines("* ssh-ed25519 AAAAC3Nz", "github.com")
            .expect_err("a wildcard line has to be rejected");
        assert_eq!(err.code(), "ssh_untrusted_line");
    }

    #[test]
    fn matching_host_line_is_accepted() {
        let out =
            validate_known_hosts_lines("github.com ssh-ed25519 AAAAC3Nz", "github.com").unwrap();
        assert_eq!(out, vec!["github.com ssh-ed25519 AAAAC3Nz"]);
    }

    #[test]
    fn foreign_host_is_rejected() {
        let err = validate_known_hosts_lines("evil.com ssh-ed25519 AAAAC3Nz", "github.com")
            .expect_err("a line for a foreign host has to be rejected");
        assert_eq!(err.code(), "ssh_untrusted_line");
    }

    #[test]
    fn cert_authority_marker_is_rejected() {
        let err = validate_known_hosts_lines(
            "@cert-authority github.com ssh-ed25519 AAAAC3Nz",
            "github.com",
        )
        .expect_err("@cert-authority has to be rejected");
        assert_eq!(err.code(), "ssh_untrusted_line");
    }

    #[test]
    fn hashed_line_is_rejected() {
        let err = validate_known_hosts_lines("|1|abcd=|efgh= ssh-ed25519 AAAAC3Nz", "github.com")
            .expect_err("a hashed |1| line has to be rejected");
        assert_eq!(err.code(), "ssh_untrusted_line");
    }

    #[test]
    fn question_mark_pattern_is_rejected() {
        let err = validate_known_hosts_lines("git?ub.com ssh-ed25519 AAAAC3Nz", "github.com")
            .expect_err("a host pattern (?) has to be rejected");
        assert_eq!(err.code(), "ssh_untrusted_line");
    }

    #[test]
    fn comments_only_yield_an_error() {
        let err = validate_known_hosts_lines("# just a comment\n\n", "github.com")
            .expect_err("no valid line -> error");
        assert_eq!(err.code(), "ssh_untrusted_line");
    }

    #[test]
    fn port_form_binds_bracket_and_bare_host() {
        // ssh-keyscan -p prints "[host]:port …"; a bare host is allowed too.
        let expected = "[example.org]:2222";
        let out = validate_known_hosts_lines(
            "[example.org]:2222 ssh-ed25519 AAAAC3Nz\nexample.org ssh-rsa AAAAB3",
            expected,
        )
        .unwrap();
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn comma_host_list_every_part_must_match() {
        // One matching + one foreign part -> reject.
        let err =
            validate_known_hosts_lines("github.com,evil.com ssh-ed25519 AAAAC3Nz", "github.com")
                .expect_err("a mixed host list has to be rejected");
        assert_eq!(err.code(), "ssh_untrusted_line");
    }

    #[test]
    fn trust_host_writes_no_known_hosts_for_a_wildcard() {
        // Redirect HOME/USERPROFILE to an empty temp directory and check that a
        // wildcard line creates neither a file nor a directory.
        let tmp = std::env::temp_dir().join(format!(
            "tg-ssh-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();

        // Careful: env::set_var is process-global — this test must not change
        // HOME permanently; restore it afterwards. (Edition 2021: set_var is
        // still a safe API here.)
        let prev_home = std::env::var_os("HOME");
        let prev_userprofile = std::env::var_os("USERPROFILE");
        std::env::set_var("HOME", &tmp);
        std::env::set_var("USERPROFILE", &tmp);

        let res = trust_host("github.com", None, "* ssh-ed25519 AAAAC3Nz", false);

        // Neither .ssh nor known_hosts may have been created (validation fails
        // before any file effect).
        let ssh_dir_existed = tmp.join(".ssh").exists();
        let known_hosts_existed = tmp.join(".ssh").join("known_hosts").exists();

        // Restore the environment + clean up.
        match prev_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        match prev_userprofile {
            Some(v) => std::env::set_var("USERPROFILE", v),
            None => std::env::remove_var("USERPROFILE"),
        }
        let _ = std::fs::remove_dir_all(&tmp);

        let err = res.expect_err("a wildcard line has to yield an error");
        assert_eq!(err.code(), "ssh_untrusted_line");
        assert!(!ssh_dir_existed, ".ssh must not be created on rejection");
        assert!(
            !known_hosts_existed,
            "known_hosts must not be created on rejection"
        );
    }
}
