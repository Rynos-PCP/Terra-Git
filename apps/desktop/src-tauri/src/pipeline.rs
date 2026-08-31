//! Local pipeline testing: the pipeline cockpit.
//!
//! Runners (gitlab-ci-local, act) and Docker are DETECTED, not shipped; missing
//! prerequisites come back as stable codes. The cockpit discovers CI configs
//! (root + `.gitlab/` + `.github/workflows/`), loads the job graph through the
//! runner metadata (include/extends is resolved by the runner — delegation
//! principle) and executes runs by scope (pipeline/stage/job): ONE child process
//! per repo with line-by-line log streaming (Tauri channel, same pattern as
//! commitDiffStream), job attribution and a deterministic status finalization
//! (the exit code is ground truth).
//! Cancelling hits the whole process tree on Windows (taskkill /T).
//! Local runs are an APPROXIMATION (secrets/tags/caches differ) — a complement
//! to the remote CI status, not a replacement.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::pipeline_graph::{self, PipelineConfig, PipelineEvent, PipelineGraph};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineInfo {
    /// "gitlab" | "github" | null (no CI file found).
    pub provider: Option<String>,
    pub config_file: Option<String>,
    /// Runner availability PER provider (not just for the auto-detected one):
    /// with several CI configs in the repo (e.g. .gitlab-ci.yml AND
    /// .github/workflows/) the gate and the banner hang off the CHOSEN config,
    /// not off detect()'s heuristic.
    pub runners_installed: RunnersInstalled,
    pub docker_running: bool,
    /// Host tools the chosen runner needs but that are missing from PATH.
    /// gitlab-ci-local copies files via `rsync` inside a `bash` shell — on
    /// Windows (Git for Windows) rsync is typically missing, which makes every
    /// job start fail. Detect that early instead of failing cryptically.
    pub missing_tools: Vec<String>,
}

/// Is the respective runner (gitlab-ci-local or act) in PATH?
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunnersInstalled {
    pub gitlab: bool,
    pub github: bool,
}

/// Run slot per repo: the child PID (RESERVED_PID while the process does not
/// exist yet) plus a cancellation note for exactly that window — cancel() can
/// pre-register the cancellation, and run_scope acts on it right after the
/// spawn.
#[derive(Debug, Clone, Copy, Default)]
pub struct RunSlot {
    pid: u32,
    cancel_requested: bool,
}

/// Active runs per repo (double-start lock + cancellation bookkeeping).
#[derive(Default)]
pub struct PipelineState(pub Mutex<HashMap<String, RunSlot>>);

/// Placeholder PID: the slot is reserved, the child process does not exist yet.
/// Neither Windows nor Unix ever hands out 0 as a real process id.
const RESERVED_PID: u32 = 0;

/// Releases the reserved run slot on an early failure (before/during spawn).
/// After a successful start the guard is disarmed — from then on the entry
/// belongs to the run/cancel bookkeeping in run_scope/cancel.
struct RunSlotGuard<'a> {
    state: &'a PipelineState,
    key: String,
    armed: bool,
}

impl Drop for RunSlotGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.state
                .0
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(&self.key);
        }
    }
}

/// Command + prefix arguments of the runner. npm shims (.cmd) need the detour
/// through cmd /C on Windows.
fn runner(provider: &str) -> (&'static str, Vec<&'static str>) {
    match (provider, cfg!(windows)) {
        ("gitlab", true) => ("cmd", vec!["/C", "gitlab-ci-local"]),
        ("gitlab", false) => ("gitlab-ci-local", vec![]),
        (_, _) => ("act", vec![]),
    }
}

/// Hardening every spawn of this module shares.
///
/// 1. No console window flashing up on Windows (CREATE_NO_WINDOW).
/// 2. `NoDefaultCurrentDirectoryInExePath`: the GitLab runner is started as
///    `cmd /C gitlab-ci-local` (npm ships a `.cmd` shim) with `current_dir(repo)`,
///    and cmd.exe resolves a bare command name from the CURRENT DIRECTORY before
///    PATH. The current directory is a possibly third-party repository, so a
///    `gitlab-ci-local.bat` committed into its root would run as soon as the user
///    opens the pipeline view — no click on "Run" and no compromised renderer
///    needed. With this variable set, cmd.exe skips the current directory.
fn harden_spawn(cmd: &mut Command) {
    cmd.env("NoDefaultCurrentDirectoryInExePath", "1");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
}

fn probe(program: &str, args: &[&str]) -> bool {
    let mut c = Command::new(program);
    c.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    harden_spawn(&mut c);
    matches!(c.status(), Ok(s) if s.success())
}

/// Checks whether the runner for the given provider ("gitlab"/"github") is
/// available in PATH. Refers to the REQUESTED provider, not to the
/// heuristically auto-detected one (which can differ when several CI configs
/// are present).
pub fn runner_installed(provider: &str) -> bool {
    let (prog, mut args) = runner(provider);
    args.push("--version");
    probe(prog, &args)
}

/// Checks whether Docker is running (mandatory for act).
pub fn docker_running() -> bool {
    probe("docker", &["version", "--format", "{{.Server.Os}}"])
}

/// Host tools the provider's runner needs but that are missing from PATH.
///
/// gitlab-ci-local copies the tracked files into the build folder via `rsync`
/// inside a `bash` shell — and it does so EVEN for jobs with `image:`, i.e.
/// before any container starts. Without rsync every job start fails with a
/// cryptic `rsync: command not found` from deep inside the runner. Git for
/// Windows does not ship rsync, which makes this the normal case on Windows.
///
/// `act` needs no host tools (it works directly against the Docker daemon).
pub fn missing_host_tools(provider: &str) -> Vec<String> {
    if provider != "gitlab" {
        return Vec::new();
    }
    ["rsync", "bash"]
        .into_iter()
        .filter(|t| !probe(t, &["--version"]))
        .map(str::to_string)
        .collect()
}

pub fn detect(repo: &Path) -> PipelineInfo {
    let (provider, config_file) = if repo.join(".gitlab-ci.yml").exists() {
        (Some("gitlab"), Some(".gitlab-ci.yml".to_string()))
    } else {
        let wf = repo.join(".github").join("workflows");
        let yml = std::fs::read_dir(&wf).ok().and_then(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .find(|n| n.ends_with(".yml") || n.ends_with(".yaml"))
        });
        match yml {
            Some(n) => (Some("github"), Some(format!(".github/workflows/{n}"))),
            None => (None, None),
        }
    };

    // Probe both runners — the config choice in the frontend can hit either
    // provider (heuristic finds), not only the auto-detected one.
    let runners_installed = RunnersInstalled {
        gitlab: runner_installed("gitlab"),
        github: runner_installed("github"),
    };

    let missing_tools = if runners_installed.gitlab {
        missing_host_tools("gitlab")
    } else {
        Vec::new()
    };

    PipelineInfo {
        provider: provider.map(str::to_string),
        config_file,
        runners_installed,
        docker_running: docker_running(),
        missing_tools,
    }
}

/// Rough content heuristic: does the file look like GitLab CI?
/// (A line starts with `stages:`/`include:` or contains a `script:` key.)
pub fn looks_like_gitlab_ci(content: &str) -> bool {
    content.lines().any(|l| {
        let t = l.trim_start();
        t.starts_with("stages:") || t.starts_with("include:") || t.starts_with("script:")
    })
}

/// Reads at most `limit` bytes from the start of the file (UTF-8 lossy): the
/// heuristic only needs the beginning — huge YAMLs are not loaded completely.
/// A signal BEYOND the limit is deliberately not detected.
fn read_prefix(p: &Path, limit: u64) -> Option<String> {
    use std::io::Read;
    let f = std::fs::File::open(p).ok()?;
    let mut buf = Vec::new();
    f.take(limit).read_to_end(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// Discovers CI configuration candidates: `.gitlab-ci.yml` (always first),
/// further `*.yml|yaml` in the root + `.gitlab/` (heuristic on a 64 KiB prefix,
/// the scan is capped — the cap applies to the root as well), all
/// `.github/workflows/*.yml|yaml`. Paths are repo-relative, `/`-separated.
pub fn discover_configs(repo: &Path) -> Vec<PipelineConfig> {
    const MAX_FILES: usize = 200;
    const MAX_DEPTH: usize = 4;
    const HEURISTIC_PREFIX: u64 = 64 * 1024;
    let mut out: Vec<PipelineConfig> = Vec::new();
    let push = |path: String, provider: &str, out: &mut Vec<PipelineConfig>| {
        if !out.iter().any(|c| c.path == path) {
            out.push(PipelineConfig {
                path,
                provider: provider.into(),
            });
        }
    };

    if repo.join(".gitlab-ci.yml").is_file() {
        push(".gitlab-ci.yml".into(), "gitlab", &mut out);
    }

    let is_yml = |n: &str| n.ends_with(".yml") || n.ends_with(".yaml");
    // A shared cap for the root loop AND the .gitlab/ scan.
    let mut seen = 0usize;
    // Root candidates (other than the default itself).
    let mut root_extra: Vec<String> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(repo) {
        for e in rd.flatten() {
            seen += 1;
            if seen > MAX_FILES {
                break;
            }
            let name = e.file_name().to_string_lossy().into_owned();
            if e.path().is_file() && is_yml(&name) && name != ".gitlab-ci.yml" {
                if let Some(c) = read_prefix(&e.path(), HEURISTIC_PREFIX) {
                    if looks_like_gitlab_ci(&c) {
                        root_extra.push(name);
                    }
                }
            }
        }
    }
    // .gitlab/ recursively, depth/count capped, known ballast dirs skipped.
    let mut gitlab_extra: Vec<String> = Vec::new();
    let mut stack: Vec<(std::path::PathBuf, usize)> = vec![(repo.join(".gitlab"), 1)];
    while let Some((dir, depth)) = stack.pop() {
        if depth > MAX_DEPTH || seen > MAX_FILES {
            continue;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            seen += 1;
            if seen > MAX_FILES {
                break;
            }
            let name = e.file_name().to_string_lossy().into_owned();
            let p = e.path();
            if p.is_dir() {
                if !matches!(name.as_str(), ".git" | "node_modules" | "target" | "dist") {
                    stack.push((p, depth + 1));
                }
            } else if is_yml(&name) {
                if let Some(c) = read_prefix(&p, HEURISTIC_PREFIX) {
                    if looks_like_gitlab_ci(&c) {
                        if let Ok(rel) = p.strip_prefix(repo) {
                            gitlab_extra.push(rel.to_string_lossy().replace('\\', "/"));
                        }
                    }
                }
            }
        }
    }
    root_extra.sort();
    gitlab_extra.sort();
    for p in root_extra.into_iter().chain(gitlab_extra) {
        push(p, "gitlab", &mut out);
    }

    // GitHub workflows (no heuristic needed — the folder is unambiguous).
    let wf = repo.join(".github").join("workflows");
    let mut gh: Vec<String> = std::fs::read_dir(&wf)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().is_file())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| is_yml(n))
                .map(|n| format!(".github/workflows/{n}"))
                .collect()
        })
        .unwrap_or_default();
    gh.sort();
    for p in gh {
        push(p, "github", &mut out);
    }
    out
}

/// Run timeout (same pattern as CLONE_TIMEOUT in sidecar.rs): generous,
/// overridable via env (TERRA_GIT_PIPELINE_TIMEOUT_SECS).
fn pipeline_timeout() -> Duration {
    let secs = std::env::var("TERRA_GIT_PIPELINE_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(1800);
    Duration::from_secs(secs)
}

/// Safely resolves a repo-relative config path and returns the canonical file
/// path. Guards (in this order): character allowlist + no `..`/absolute path
/// (the path goes through `cmd /C` on Windows), extension `.yml/.yaml`,
/// existence, and — via canonicalize — containment INSIDE the repo (catches
/// symlink escapes; the runner runs with `current_dir(repo)`).
fn resolve_repo_config(repo: &Path, config: &str) -> Result<std::path::PathBuf, RunError> {
    if config.contains("..") || Path::new(config).is_absolute() || !is_safe_job_name(config) {
        return Err(RunError::rejected(
            "invalid_target",
            "Invalid configuration path",
        ));
    }
    let lower = config.to_ascii_lowercase();
    if !(lower.ends_with(".yml") || lower.ends_with(".yaml")) {
        return Err(RunError::rejected(
            "invalid_target",
            "Not a YAML configuration (.yml/.yaml)",
        ));
    }
    let repo_canon = repo
        .canonicalize()
        .map_err(|e| RunError::Failed(e.to_string()))?;
    let file_canon = repo_canon
        .join(config)
        .canonicalize()
        .map_err(|_| RunError::rejected("invalid_target", "Configuration file not found"))?;
    if !file_canon.starts_with(&repo_canon) {
        return Err(RunError::rejected(
            "invalid_target",
            "Path outside the repository",
        ));
    }
    if !file_canon.is_file() {
        return Err(RunError::rejected("invalid_target", "Not a regular file"));
    }
    Ok(file_canon)
}

/// Derives the provider of an (existing) configuration file:
/// `.github/workflows/*` -> github; otherwise by content heuristic. Default is
/// gitlab, because `act` insists on `.github/workflows` while gitlab-ci-local
/// accepts arbitrary files via `--file`.
// Both the heuristic branch and the default return "gitlab" (github only for a
// .github/workflows/ path); the content check stays as a deliberate placeholder
// for a future, finer provider distinction.
#[allow(clippy::if_same_then_else)]
fn infer_provider(config: &str, file_canon: &Path) -> &'static str {
    if config.starts_with(".github/workflows/") {
        "github"
    } else if read_prefix(file_canon, 64 * 1024)
        .map(|c| looks_like_gitlab_ci(&c))
        .unwrap_or(false)
    {
        "gitlab"
    } else {
        "gitlab"
    }
}

/// Config path guard. Security allowlist/traversal FIRST (for EVERY config,
/// auto-discovered ones included). Allowed afterwards if the config was either
/// auto-discovered (with a matching provider) OR chosen manually and exists
/// inside the repo — then the provider has to match the content
/// (provider=github with a GitLab file stays an invalid_target so no GitLab
/// file is ever fed to act).
pub fn validate_config(repo: &Path, provider: &str, config: &str) -> Result<(), RunError> {
    if provider != "gitlab" && provider != "github" {
        return Err(RunError::rejected("invalid_target", "Unknown provider"));
    }
    // Block the cmd /C injection vector (`a&b.yml`) and traversal for ALL paths.
    if config.contains("..") || Path::new(config).is_absolute() || !is_safe_job_name(config) {
        return Err(RunError::rejected(
            "invalid_target",
            "Invalid configuration path",
        ));
    }
    if discover_configs(repo)
        .iter()
        .any(|c| c.path == config && c.provider == provider)
    {
        return Ok(());
    }
    // Chosen manually (not auto-discovered): resolve safely + the provider has
    // to match the content.
    let file_canon = resolve_repo_config(repo, config)?;
    if provider != infer_provider(config, &file_canon) {
        return Err(RunError::rejected(
            "invalid_target",
            "Provider does not match the configuration",
        ));
    }
    Ok(())
}

/// Builds a [`PipelineConfig`] from an (absolute) path chosen in the file
/// picker: canonicalizes it, checks containment in the repo, forms the
/// repo-relative `/` path and derives the provider. Errors when the file lies
/// outside the repo or fails the guards (extension/allowlist).
pub fn config_from_path(repo: &Path, abs_path: &str) -> Result<PipelineConfig, RunError> {
    let repo_canon = repo
        .canonicalize()
        .map_err(|e| RunError::Failed(e.to_string()))?;
    let file_canon = Path::new(abs_path)
        .canonicalize()
        .map_err(|_| RunError::rejected("invalid_target", "File not found"))?;
    let rel = file_canon
        .strip_prefix(&repo_canon)
        .map_err(|_| RunError::rejected("invalid_target", "File lies outside the repository"))?;
    let config = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/");
    // Through the shared, strict resolution (extension/allowlist/containment).
    let resolved = resolve_repo_config(repo, &config)?;
    let provider = infer_provider(&config, &resolved);
    Ok(PipelineConfig {
        path: config,
        provider: provider.into(),
    })
}

/// Runs a command with a deadline: spawn + a try_wait loop instead of
/// `Command::output()`, which would block indefinitely (a hanging runner).
/// On expiry the whole process tree is killed and a timeout is reported.
fn output_with_timeout(
    c: &mut Command,
    timeout: Duration,
) -> Result<std::process::Output, RunError> {
    use std::io::Read;
    c.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    harden_spawn(c);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        c.process_group(0); // kill_tree signals the whole process group
    }
    let mut child = c.spawn().map_err(|e| RunError::Failed(e.to_string()))?;
    let mut out_pipe = child.stdout.take();
    let mut err_pipe = child.stderr.take();
    let out_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(p) = out_pipe.as_mut() {
            let _ = p.read_to_end(&mut buf);
        }
        buf
    });
    let err_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(p) = err_pipe.as_mut() {
            let _ = p.read_to_end(&mut buf);
        }
        buf
    });
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return Ok(std::process::Output {
                    status,
                    stdout: out_thread.join().unwrap_or_default(),
                    stderr: err_thread.join().unwrap_or_default(),
                });
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = kill_tree(child.id());
                    let _ = child.wait();
                    let _ = out_thread.join();
                    let _ = err_thread.join();
                    return Err(RunError::Timeout);
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                let _ = kill_tree(child.id());
                return Err(RunError::Failed(e.to_string()));
            }
        }
    }
}

/// Loads the graph through the runner metadata (include/extends is resolved by
/// the runner itself — delegation principle). The metadata process runs with its
/// own deadline (min(120s, run timeout)) — a hanging runner must block neither
/// the caller nor the run slot permanently.
pub fn load_graph(repo: &Path, provider: &str, config: &str) -> Result<PipelineGraph, RunError> {
    validate_config(repo, provider, config)?;
    let (prog, args) = runner(provider);
    let list_args: Vec<String> = if provider == "gitlab" {
        vec!["--list-csv".into(), "--file".into(), config.into()]
    } else {
        vec!["-l".into(), "-W".into(), config.into()]
    };
    let mut c = Command::new(prog);
    c.args(&args).args(&list_args).current_dir(repo);
    let out = output_with_timeout(&mut c, pipeline_timeout().min(Duration::from_secs(120)))?;
    if !out.status.success() {
        return Err(RunError::Failed(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let (stages, jobs) = if provider == "gitlab" {
        pipeline_graph::parse_gitlab_csv(&text)
    } else {
        pipeline_graph::parse_act_table(&text)
    };
    Ok(PipelineGraph {
        provider: provider.into(),
        config_file: config.into(),
        stages,
        jobs,
    })
}

/// Permitted act trigger events (a fixed enum choice from the frontend). Only
/// these end up as act's first positional argument; everything else is rejected
/// server-side (even though act is started via a direct `Command`, not through a
/// shell — defense in depth, the client is not trusted).
pub fn is_valid_event(ev: &str) -> bool {
    matches!(ev, "push" | "pull_request" | "workflow_dispatch" | "tag")
}

/// Permitted name of a CI variable (env-var convention): the first character is
/// a letter/`_`, the rest alphanumeric/`_`. Keys only end up in the variables
/// file (not on the command line), but a strict key keeps the file format stable
/// and rules out YAML/dotenv key tricks.
pub fn is_valid_var_key(k: &str) -> bool {
    let mut chars = k.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Running counter for unique temp file names (combined with the PID).
static VARS_SEQ: AtomicU64 = AtomicU64::new(0);

/// Deletes the temporary variables file as soon as the run ends (success,
/// failure or panic — the drop runs in run_scope when the function is left, i.e.
/// AFTER waiting for the runner).
struct VarsFile {
    /// Absolute path, only used for deletion on drop.
    path: PathBuf,
    /// Runner argument (the `--variables-file`/`--env-file` value): a
    /// repo-relative, metacharacter-free file name. Deliberately NOT the
    /// absolute path: its directory prefix (system temp/USERPROFILE, driven by
    /// the environment) could contain cmd metacharacters (`& | ^ ( ) %`) WITHOUT
    /// spaces; Rust's std `Command` only quotes on spaces, so such a prefix would
    /// make `cmd /C` split the command on Windows. The file therefore lives in
    /// the cwd (= repo), and the runner resolves the relative name against it.
    arg: String,
}

impl Drop for VarsFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Serializes a string as a dotenv value (act `--env-file`): double-quoted with
/// godotenv-compatible escaping — so even values containing a newline/`#`/quotes
/// do not corrupt the file. Values never reach the command line; this is purely
/// about file-format robustness.
fn dotenv_escape(v: &str) -> String {
    let mut s = String::with_capacity(v.len() + 2);
    s.push('"');
    for ch in v.chars() {
        match ch {
            '\\' => s.push_str("\\\\"),
            '"' => s.push_str("\\\""),
            '\n' => s.push_str("\\n"),
            '\r' => s.push_str("\\r"),
            _ => s.push(ch),
        }
    }
    s.push('"');
    s
}

/// Writes valid CI variables into an app-controlled file in the repo directory
/// (the runner's cwd) and returns the guard (None when there are no valid
/// variables). The file-based route is the injection protection: VALUES never
/// appear on the command line (`cmd /C` on Windows), and the only argument is a
/// repo-relative, metacharacter-free file name (no environment-driven absolute
/// path — see `VarsFile::arg`). Only allowlisted keys are taken over; invalid
/// ones are dropped. GitLab: JSON (= valid YAML) for `--variables-file`
/// (serde_json escapes values completely); act: dotenv for `--env-file`. The
/// file is untracked, and gitlab-ci-local's rsync (`git ls-files -o`) excludes it
/// from the build context.
fn write_vars_file(
    repo: &Path,
    provider: &str,
    vars: &[(String, String)],
) -> Result<Option<VarsFile>, RunError> {
    let valid: Vec<(&str, &str)> = vars
        .iter()
        .filter(|(k, _)| is_valid_var_key(k))
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    if valid.is_empty() {
        return Ok(None);
    }
    let (content, ext) = if provider == "gitlab" {
        // JSON is valid YAML; serde_json handles all escaping. gitlab-ci-local
        // reads the top-level keys as global variables.
        let mut map = serde_json::Map::new();
        for (k, v) in &valid {
            map.insert(
                (*k).to_string(),
                serde_json::Value::String((*v).to_string()),
            );
        }
        let json = serde_json::to_string(&serde_json::Value::Object(map))
            .map_err(|e| RunError::Failed(e.to_string()))?;
        (json, "yml")
    } else {
        let mut s = String::new();
        for (k, v) in &valid {
            s.push_str(k);
            s.push('=');
            s.push_str(&dotenv_escape(v));
            s.push('\n');
        }
        (s, "env")
    };
    let n = VARS_SEQ.fetch_add(1, Ordering::Relaxed);
    // Repo-relative name (digits + `.`, no cmd metacharacters, no prefix).
    let name = format!(".terra-git-vars-{}-{}.{}", std::process::id(), n, ext);
    // Construct the guard BEFORE writing: if fs::write fails after creating the
    // file (ENOSPC/IO), the guard's drop still deletes the already created file
    // on the early return.
    let guard = VarsFile {
        path: repo.join(&name),
        arg: name,
    };
    std::fs::write(&guard.path, content).map_err(|e| RunError::Failed(e.to_string()))?;
    Ok(Some(guard))
}

/// Runner arguments per scope (pure, unit-tested). `event` is only relevant for
/// act (GitLab has no trigger events) and, when non-empty, is placed as the
/// first positional argument before `-W` (`act <event> -W …`); empty = act's
/// default (push). `manual_jobs` is only relevant for GitLab: gitlab-ci-local
/// skips `when: manual` jobs, `--manual <job>` runs them (repeated per job so
/// yargs collects them unambiguously regardless of positional arguments).
/// `vars_file` (when Some) is the path of an app-controlled variables file —
/// GitLab `--variables-file`, act `--env-file`; that way the VALUES never appear
/// on the command line.
pub fn build_scope_args(
    provider: &str,
    config: &str,
    scope: &str,
    jobs: &[String],
    event: &str,
    manual_jobs: &[String],
    vars_file: Option<&str>,
) -> Vec<String> {
    let mut a: Vec<String> = Vec::new();
    if provider == "gitlab" {
        a.push("--file".into());
        a.push(config.into());
        match scope {
            "pipeline" => {}
            "stage" => {
                a.extend(jobs.iter().cloned());
                a.push("--needs".into());
            }
            _ => a.extend(jobs.iter().cloned()),
        }
        for j in manual_jobs {
            a.push("--manual".into());
            a.push(j.clone());
        }
        if let Some(p) = vars_file {
            a.push("--variables-file".into());
            a.push(p.into());
        }
    } else {
        if !event.is_empty() {
            a.push(event.into());
        }
        a.push("-W".into());
        a.push(config.into());
        if scope == "job" {
            a.push("-j".into());
            a.extend(jobs.iter().cloned());
        }
        if let Some(p) = vars_file {
            a.push("--env-file".into());
            a.push(p.into());
        }
    }
    a
}

/// Jobs of a stage plus transitive needs, in graph order (pure).
pub fn stage_jobs_with_needs(graph: &PipelineGraph, stage: &str) -> Vec<String> {
    let mut wanted: HashSet<&str> = graph
        .jobs
        .iter()
        .filter(|j| j.stage == stage)
        .map(|j| j.name.as_str())
        .collect();
    // Fixed point: collect needs transitively.
    loop {
        let before = wanted.len();
        for j in &graph.jobs {
            if wanted.contains(j.name.as_str()) {
                for n in &j.needs {
                    if let Some(t) = graph.jobs.iter().find(|x| &x.name == n) {
                        wanted.insert(t.name.as_str());
                    }
                }
            }
        }
        if wanted.len() == before {
            break;
        }
    }
    graph
        .jobs
        .iter()
        .filter(|j| wanted.contains(j.name.as_str()))
        .map(|j| j.name.clone())
        .collect()
}

/// Strict allowlist for job names BEFORE they go to the runner (through
/// `cmd /C` on Windows). Rust's `Command` only quotes on spaces/tabs/quotes —
/// cmd.exe metacharacters (`&`, `|`, `^`, `<`, `>`, `(`, `)`, `%`, `;`, backtick)
/// are NOT neutralized and get re-parsed by cmd. The job name comes from a
/// potentially malicious `.gitlab-ci.yml`; hence an allowlist instead of a block
/// list, to prevent command injection/RCE.
fn is_safe_job_name(job: &str) -> bool {
    !job.is_empty()
        && !job.starts_with('-')
        && job
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || " ._-:/".contains(c))
}

/// Kill the process tree (factored out of cancel() so the timeout watchdog uses
/// the same logic). On Unix the runner runs in its own process group
/// (process_group(0) in run_scope); a NEGATIVE PID signals the whole group — so
/// it also hits docker/rsync/bash children, not just the runner.
fn kill_tree(pid: u32) -> bool {
    let mut c = if cfg!(windows) {
        let mut c = Command::new("taskkill");
        c.args(["/PID", &pid.to_string(), "/T", "/F"]);
        c
    } else {
        let mut c = Command::new("kill");
        c.args(["-s", "KILL", "--", &format!("-{pid}")]);
        c
    };
    harden_spawn(&mut c);
    c.status().map(|s| s.success()).unwrap_or(false)
}

/// Error of a pipeline run or graph load. `Rejected` are app-side validation
/// errors with a STABLE code for the frontend (run_active, stage_not_found,
/// invalid_target, invalid_scope); `Failed` is real runner/spawn failure and
/// surfaces in the command layer as "runner_failed".
#[derive(Debug)]
pub enum RunError {
    Timeout,
    Rejected { code: &'static str, message: String },
    Failed(String),
}

impl RunError {
    fn rejected(code: &'static str, message: impl Into<String>) -> Self {
        RunError::Rejected {
            code,
            message: message.into(),
        }
    }
}

/// Reads lines raw (`read_until`) and forwards them UTF-8 lossy: a single
/// non-UTF-8 line (e.g. binary output of a job) must not end the reader thread —
/// `BufRead::lines()` would abort on the first InvalidData, the pipe would fill
/// up and the run would die with EPIPE.
fn stream_lines(r: impl std::io::Read, tx: std::sync::mpsc::Sender<String>) {
    let mut reader = BufReader::new(r);
    let mut buf: Vec<u8> = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => return,
            Ok(_) => {
                if buf.last() == Some(&b'\n') {
                    buf.pop();
                    if buf.last() == Some(&b'\r') {
                        buf.pop();
                    }
                }
                if tx.send(String::from_utf8_lossy(&buf).into_owned()).is_err() {
                    return;
                }
            }
        }
    }
}

/// Target jobs per scope (pure, server-side — the client is not trusted):
/// pipeline = all graph jobs, stage = the stage's jobs including transitive
/// needs (for the status finalization), job = the target.
fn targeted_jobs(
    graph: &PipelineGraph,
    scope: &str,
    target: Option<&str>,
) -> Result<Vec<String>, RunError> {
    match scope {
        "pipeline" => Ok(graph.jobs.iter().map(|j| j.name.clone()).collect()),
        "stage" => {
            let stage =
                target.ok_or_else(|| RunError::rejected("invalid_target", "Stage is missing"))?;
            if !is_safe_job_name(stage) {
                return Err(RunError::rejected("invalid_target", "Invalid stage name"));
            }
            let jobs = stage_jobs_with_needs(graph, stage);
            if jobs.is_empty() {
                return Err(RunError::rejected(
                    "stage_not_found",
                    format!("Stage not found: {stage}"),
                ));
            }
            Ok(jobs)
        }
        "job" => {
            let job =
                target.ok_or_else(|| RunError::rejected("invalid_target", "Job is missing"))?;
            Ok(vec![job.to_string()])
        }
        other => Err(RunError::rejected(
            "invalid_scope",
            format!("Unknown scope: {other}"),
        )),
    }
}

/// Jobs that go to the runner as PROCESS arguments (pure): ONLY these have to
/// pass the strict allowlist — otherwise targeted names flow exclusively into
/// JSON events. scope=pipeline passes no job names at all, scope=stage passes
/// the stage's jobs WITHOUT transitive needs (`--needs` pulls those in),
/// scope=job passes the target. A graph job like "check (fast)" therefore no
/// longer blocks a run in which it never reaches the command line.
fn arg_jobs_for_scope(
    graph: &PipelineGraph,
    scope: &str,
    target: Option<&str>,
    targeted: &[String],
) -> Result<Vec<String>, RunError> {
    let arg_jobs: Vec<String> = match scope {
        "stage" => {
            let stage = target.unwrap_or_default();
            graph
                .jobs
                .iter()
                .filter(|j| j.stage == stage)
                .map(|j| j.name.clone())
                .collect()
        }
        "job" => targeted.to_vec(),
        _ => Vec::new(),
    };
    for j in &arg_jobs {
        if !is_safe_job_name(j) {
            return Err(RunError::rejected(
                "invalid_target",
                format!("Invalid job name: {j}"),
            ));
        }
    }
    Ok(arg_jobs)
}

/// Manual jobs (GitLab `when: manual`) that run in the current scope and are
/// safely named (pure, unit-tested). Otherwise gitlab-ci-local skips manual jobs
/// — a click on a manual job would do nothing. ONLY jobs running in scope
/// (`targeted`) and ONLY allowlist-conforming names end up as `--manual <job>`
/// on the command line: unsafely named ones stay skipped (as before this
/// feature) instead of rejecting the whole run or risking a command injection.
/// act/GitHub has no `--manual` (and carries no `when` anyway), hence an empty
/// list. For a full pipeline these are all manual jobs (a local test run plays
/// through the complete YAML), for a single job exactly the one clicked.
fn manual_jobs_in_scope(graph: &PipelineGraph, provider: &str, targeted: &[String]) -> Vec<String> {
    if provider != "gitlab" {
        return Vec::new();
    }
    let in_scope: HashSet<&str> = targeted.iter().map(String::as_str).collect();
    graph
        .jobs
        .iter()
        .filter(|j| {
            j.when == "manual" && in_scope.contains(j.name.as_str()) && is_safe_job_name(&j.name)
        })
        .map(|j| j.name.clone())
        .collect()
}

/// Line processing (pure): attributes the line (display name -> job id via
/// `alias`) and decides whether a pending->running transition is due. The
/// ATTRIBUTION runs over all graph jobs (--needs can start more whose logs should
/// be grouped); STATUS events exist only for targeted jobs — a false-positive
/// match would otherwise hang on "running" in the UI forever. Returns:
/// (job of the line, job with a new running status).
fn attribute_and_status(
    by_len: &[String],
    alias: &HashMap<String, String>,
    targeted: &HashSet<String>,
    started: &mut HashSet<String>,
    clean_line: &str,
) -> (Option<String>, Option<String>) {
    let job = pipeline_graph::attribute_line(by_len, clean_line)
        .map(|m| alias.get(m).cloned().unwrap_or_else(|| m.to_string()));
    let mut now_running = None;
    if let Some(j) = &job {
        if targeted.contains(j) && started.insert(j.clone()) {
            now_running = Some(j.clone());
        }
    }
    (job, now_running)
}

/// Finalization flag (pure): cancellation OR timeout count as canceled — the
/// spec treats the time limit as a cancellation, so started jobs end yellow
/// "cancelled" instead of red "failed".
fn run_canceled(cancel_detected: bool, timed_out: bool) -> bool {
    cancel_detected || timed_out
}

/// Releases the run slot at the end — but ONLY when it still belongs to our own
/// run (`slot.pid == our_pid`). Returns cancel_detected: true when the slot is
/// missing (cancel removed it) OR already belongs to a NEW run (same repo key,
/// different PID — e.g. through OS PID reuse or a fast restart). In the foreign
/// case the slot is NOT touched, otherwise the new run would no longer be
/// cancellable. Pure & unit-tested.
fn finalize_slot(running: &mut HashMap<String, RunSlot>, key: &str, our_pid: u32) -> bool {
    match running.get(key) {
        Some(slot) if slot.pid == our_pid => {
            running.remove(key);
            false
        }
        _ => true,
    }
}

/// Additional run options supplied by the frontend. Kept as a struct so future
/// fields do not have to touch the run_scope signature (and its many test calls)
/// again.
#[derive(Debug, Default, Clone)]
pub struct RunOptions {
    /// act trigger event (push/pull_request/workflow_dispatch/tag). None/"" =>
    /// act's default (push). Meaningless for GitLab and ignored there.
    pub event: Option<String>,
    /// CI variables (key/value) for the run. Passed file-based (never inline on
    /// the command line); only allowlisted keys count.
    pub variables: Vec<(String, String)>,
}

/// Starts a pipeline/stage/job and streams events (log lines with job
/// attribution + status transitions). The exit code is ground truth.
#[allow(clippy::too_many_arguments)]
pub fn run_scope(
    state: &PipelineState,
    repo: &Path,
    provider: &str,
    config: &str,
    scope: &str,
    target: Option<&str>,
    options: &RunOptions,
    mut on_event: impl FnMut(PipelineEvent) + Send,
) -> Result<i32, RunError> {
    // Validate the scope early (cheap, before any runner start).
    if !matches!(scope, "pipeline" | "stage" | "job") {
        return Err(RunError::rejected(
            "invalid_scope",
            format!("Unknown scope: {scope}"),
        ));
    }
    // Trigger event only for act; a fixed enum choice, otherwise reject (cheap,
    // before any slot reservation => no state left behind). GitLab ignores it.
    let event = options.event.as_deref().unwrap_or("");
    if provider != "gitlab" && !event.is_empty() && !is_valid_event(event) {
        return Err(RunError::rejected(
            "invalid_event",
            format!("Invalid trigger event: {event}"),
        ));
    }
    // Design spec: scope="stage" exists ONLY for GitLab (act has no stages).
    // Reject server-side — the client is not trusted; otherwise act would run
    // the ENTIRE workflow while targeted/status finalization only cover the
    // supposed stage jobs.
    if scope == "stage" && provider != "gitlab" {
        return Err(RunError::rejected(
            "invalid_scope",
            "Stage runs are GitLab-only",
        ));
    }
    let key = repo.to_string_lossy().to_lowercase();
    // Early double-start check (cheap): the graph load below starts a runner
    // process — a visibly occupied slot rejects immediately. The BINDING check
    // is the atomic reservation after the load.
    if state
        .0
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .contains_key(&key)
    {
        return Err(RunError::rejected("run_active", "A run is already active"));
    }

    // Load the graph BEFORE reserving the slot (load_graph validates the config
    // including the provider binding): the slot protects the actual RUN — a
    // hanging metadata load would otherwise block it permanently, because
    // cancel() only reaches the real run child process. The load itself is
    // additionally capped by output_with_timeout.
    let graph = load_graph(repo, provider, config)?;
    let targeted = targeted_jobs(&graph, scope, target)?;
    // Args: for "stage" the stage's jobs go as positional arguments WITHOUT
    // transitive needs (--needs pulls those in) — targeted still contains them
    // so the status finalization covers them all. ONLY arg_jobs reach the
    // command line and are checked against the allowlist.
    let arg_jobs = arg_jobs_for_scope(&graph, scope, target, &targeted)?;
    let manual_jobs = manual_jobs_in_scope(&graph, provider, &targeted);
    // Variables file-based (never inline): the guard lives until the end of
    // run_scope (i.e. until after waiting for the runner) and deletes the file
    // afterwards. `vars_arg` is the repo-relative file name for build_scope_args.
    let vars_file = write_vars_file(repo, provider, &options.variables)?;
    let vars_arg = vars_file.as_ref().map(|f| f.arg.clone());
    let extra = build_scope_args(
        provider,
        config,
        scope,
        &arg_jobs,
        event,
        &manual_jobs,
        vars_arg.as_deref(),
    );

    // Exactly ONE active run per repo (design spec: a second start => error).
    // Reserve atomically (placeholder PID) — otherwise a second start could
    // overwrite the PID, and the end of the first run would remove the second's
    // entry (no longer cancellable, wrongly reported as canceled).
    {
        let mut running = state.0.lock().unwrap_or_else(|p| p.into_inner());
        if running.contains_key(&key) {
            return Err(RunError::rejected("run_active", "A run is already active"));
        }
        running.insert(key.clone(), RunSlot::default()); // pid == RESERVED_PID
    }
    let mut slot = RunSlotGuard {
        state,
        key: key.clone(),
        armed: true,
    };

    let (prog, args) = runner(provider);
    let mut c = Command::new(prog);
    c.args(&args)
        .args(&extra)
        .current_dir(repo)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    harden_spawn(&mut c);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        c.process_group(0);
    }
    let mut child = c.spawn().map_err(|e| RunError::Failed(e.to_string()))?;
    let pid = child.id();
    // Record the PID — unless cancel() already PRE-REGISTERED a cancellation
    // during the reservation window: then remove the entry and kill the child
    // immediately; the flow below recognizes that as canceled (the entry is
    // missing) and finalizes accordingly. From here on, removing the entry is up
    // to the cancellation detection below or cancel() — the guard must not fire
    // any more (it would delete the entry of a NEW run).
    let canceled_early = {
        let mut running = state.0.lock().unwrap_or_else(|p| p.into_inner());
        match running.get_mut(&key) {
            Some(entry) if entry.cancel_requested => {
                running.remove(&key);
                true
            }
            Some(entry) => {
                entry.pid = pid;
                false
            }
            None => true,
        }
    };
    slot.armed = false;
    if canceled_early {
        let _ = kill_tree(pid);
    }

    // Timeout watchdog (pattern CLONE_TIMEOUT): kills the tree once it expires.
    let timed_out = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicBool::new(false));
    {
        let (timed_out, done) = (timed_out.clone(), done.clone());
        let deadline = Instant::now() + pipeline_timeout();
        std::thread::spawn(move || {
            while !done.load(Ordering::Relaxed) {
                if Instant::now() >= deadline {
                    timed_out.store(true, Ordering::Relaxed);
                    let _ = kill_tree(pid);
                    return;
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        });
    }

    // All targeted jobs start out as pending.
    for j in &targeted {
        on_event(PipelineEvent::Status {
            job: j.clone(),
            status: pipeline_graph::JobStatus::Pending,
        });
    }
    // Attribution: longest names first; all graph jobs, not only targeted ones
    // (--needs can execute more whose logs should be assigned correctly). For act
    // additionally the display names — act logs "[Workflow/Job name]" — mapped
    // onto the job id (= the node name in the graph and in events).
    let mut by_len: Vec<String> = Vec::new();
    let mut alias: HashMap<String, String> = HashMap::new();
    for j in &graph.jobs {
        by_len.push(j.name.clone());
        if let Some(d) = &j.display_name {
            alias.insert(d.clone(), j.name.clone());
            by_len.push(d.clone());
        }
    }
    by_len.sort_by_key(|s| std::cmp::Reverse(s.len()));
    let targeted_set: HashSet<String> = targeted.iter().cloned().collect();

    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let out_pipe = child.stdout.take();
    let err_pipe = child.stderr.take();
    let tx_err = tx.clone();
    let out_thread = std::thread::spawn(move || {
        if let Some(p) = out_pipe {
            stream_lines(p, tx);
        }
    });
    let err_thread = std::thread::spawn(move || {
        if let Some(p) = err_pipe {
            stream_lines(p, tx_err);
        }
    });

    let mut started: HashSet<String> = HashSet::new();
    for line in rx {
        let clean = pipeline_graph::strip_ansi(&line);
        let (job, now_running) =
            attribute_and_status(&by_len, &alias, &targeted_set, &mut started, &clean);
        if let Some(j) = now_running {
            on_event(PipelineEvent::Status {
                job: j,
                status: pipeline_graph::JobStatus::Running,
            });
        }
        on_event(PipelineEvent::Line { job, line: clean });
    }
    let _ = out_thread.join();
    let _ = err_thread.join();

    let status = child.wait().map_err(|e| RunError::Failed(e.to_string()))?;
    done.store(true, Ordering::Relaxed);
    // Cancellation detection + slot release: release it ONLY when the slot still
    // belongs to US (pid == our child PID). Otherwise cancel() removed it and a
    // NEW run (same key) may have installed a fresh slot — we must not delete
    // that one (the new run would no longer be cancellable). A timeout also
    // counts as a cancellation for the status finalization.
    let cancel_detected = finalize_slot(
        &mut state.0.lock().unwrap_or_else(|p| p.into_inner()),
        &key,
        pid,
    );
    let timed = timed_out.load(Ordering::Relaxed);
    let exit = status.code().unwrap_or(-1);
    for (job, st) in pipeline_graph::finalize_statuses(
        &targeted,
        &started,
        run_canceled(cancel_detected, timed),
        exit,
    ) {
        on_event(PipelineEvent::Status { job, status: st });
    }
    if timed {
        return Err(RunError::Timeout);
    }
    Ok(exit)
}

/// Cancels the running run — including the process tree. When the slot is only
/// reserved (the run child process does not exist yet), the cancellation is
/// PRE-REGISTERED: run_scope checks the flag right after the spawn and then ends
/// immediately with a canceled finalization.
pub fn cancel(state: &PipelineState, repo: &Path) -> bool {
    let key = repo.to_string_lossy().to_lowercase();
    let pid = {
        let mut running = state.0.lock().unwrap_or_else(|p| p.into_inner());
        match running.get_mut(&key) {
            None => return false,
            Some(slot) if slot.pid == RESERVED_PID => {
                slot.cancel_requested = true;
                return true;
            }
            Some(slot) => {
                let pid = slot.pid;
                running.remove(&key);
                pid
            }
        }
    };
    kill_tree(pid)
}

#[cfg(test)]
mod tests {
    use super::is_safe_job_name;
    use super::missing_host_tools;

    /// Stable error code of a RunError (for compact assertions).
    fn code_of(e: &super::RunError) -> &'static str {
        match e {
            super::RunError::Timeout => "timeout",
            super::RunError::Rejected { code, .. } => code,
            super::RunError::Failed(_) => "runner_failed",
        }
    }

    fn node(n: &str, st: &str, needs: &[&str]) -> crate::pipeline_graph::PipelineJobNode {
        crate::pipeline_graph::PipelineJobNode {
            name: n.into(),
            stage: st.into(),
            needs: needs.iter().map(|s| s.to_string()).collect(),
            when: String::new(),
            allow_failure: false,
            display_name: None,
        }
    }

    /// Graph with a legal but non-allowlist-conforming job name
    /// ("check (nightly)" — parentheses are allowed in GitLab job names).
    fn testgraph() -> crate::pipeline_graph::PipelineGraph {
        crate::pipeline_graph::PipelineGraph {
            provider: "gitlab".into(),
            config_file: ".gitlab-ci.yml".into(),
            stages: vec!["build".into(), "check".into(), "ship".into()],
            jobs: vec![
                node("build", "build", &[]),
                node("check (nightly)", "check", &["build"]),
                node("deploy", "ship", &["check (nightly)"]),
            ],
        }
    }

    #[test]
    fn job_name_allowlist_blocks_cmd_metacharacters() {
        // Realistic job names (GitLab/GitHub): letters, digits, . _ - : / space.
        assert!(is_safe_job_name("build"));
        assert!(is_safe_job_name("deploy:prod"));
        assert!(is_safe_job_name("test unit-1.2/foo_bar"));
        // cmd metacharacters from a malicious .gitlab-ci.yml → injection, must be rejected.
        for bad in [
            "deploy&calc",
            "a|b",
            "a^b",
            "a>b",
            "a<b",
            "a(b)",
            "%PATH%",
            "a;b",
            "a\"b",
        ] {
            assert!(!is_safe_job_name(bad), "must be rejected: {bad}");
        }
        // Empty / option injection (leading '-') stays forbidden.
        assert!(!is_safe_job_name(""));
        assert!(!is_safe_job_name("-x"));
    }

    #[test]
    fn discovers_gitlab_github_and_heuristic_candidates() {
        let dir = tempfile::tempdir().unwrap();
        let w = |rel: &str, content: &str| {
            let p = dir.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, content).unwrap();
        };
        w(
            ".gitlab-ci.yml",
            "stages: [build]\njob:\n  script: echo hi\n",
        );
        w("extra-ci.yml", "include:\n  - local: a.yml\n");
        w("not-ci.yml", "just:\n  data: 1\n");
        w(".gitlab/ci/deploy.yml", "deploy:\n  script: ship\n");
        w(".github/workflows/ci.yml", "on: push\n");
        w("node_modules/x/ci.yml", "stages: [x]\n"); // skipped (no scan of root subfolders)
                                                     // Ballast folder INSIDE .gitlab/ (the scan's exclude branch):
                                                     // despite GitLab CI content the file must not be discovered.
        w(
            ".gitlab/node_modules/x/ci.yml",
            "stages: [x]\njob:\n  script: echo\n",
        );

        let cfgs = super::discover_configs(dir.path());
        let paths: Vec<&str> = cfgs.iter().map(|c| c.path.as_str()).collect();
        assert_eq!(paths[0], ".gitlab-ci.yml", "default first");
        assert!(paths.contains(&"extra-ci.yml"));
        assert!(paths.contains(&".gitlab/ci/deploy.yml"));
        assert!(paths.contains(&".github/workflows/ci.yml"));
        assert!(!paths.iter().any(|p| p.contains("not-ci")));
        assert!(!paths.iter().any(|p| p.contains("node_modules")));
        let gh = cfgs
            .iter()
            .find(|c| c.path.ends_with("workflows/ci.yml"))
            .unwrap();
        assert_eq!(gh.provider, "github");
    }

    #[test]
    fn runner_maps_provider_independently_of_detect() {
        // Only the pure mapping runner() is checked: provider -> (program,
        // prefix args), gitlab != github, correct programs — without a process
        // spawn and without calling detect()/probe(). That runner_installed()
        // probes the REQUESTED provider (and not the heuristically detected one)
        // follows purely from it building on this mapping — runner_installed()
        // itself spawns processes and is not executed here.
        let (gitlab_prog, gitlab_args) = super::runner("gitlab");
        let (github_prog, github_args) = super::runner("github");
        assert_ne!(
            (gitlab_prog, gitlab_args.clone()),
            (github_prog, github_args.clone())
        );
        if cfg!(windows) {
            assert_eq!(gitlab_prog, "cmd");
            assert_eq!(gitlab_args, vec!["/C", "gitlab-ci-local"]);
        } else {
            assert_eq!(gitlab_prog, "gitlab-ci-local");
            assert!(gitlab_args.is_empty());
        }
        assert_eq!(github_prog, "act");
        assert!(github_args.is_empty());
    }

    #[test]
    fn gitlab_ci_heuristic() {
        assert!(super::looks_like_gitlab_ci("stages:\n  - build\n"));
        assert!(super::looks_like_gitlab_ci("include:\n  - local: x\n"));
        assert!(super::looks_like_gitlab_ci("job:\n  script: echo\n"));
        assert!(!super::looks_like_gitlab_ci("just:\n  data: 1\n"));
    }

    #[test]
    fn scope_args_gitlab_and_github() {
        let a = super::build_scope_args("gitlab", ".gitlab-ci.yml", "pipeline", &[], "", &[], None);
        assert_eq!(a, vec!["--file", ".gitlab-ci.yml"]);
        let jobs: Vec<String> = vec!["lint".into(), "test".into()];
        let a = super::build_scope_args("gitlab", "ci/x.yml", "stage", &jobs, "", &[], None);
        assert_eq!(a, vec!["--file", "ci/x.yml", "lint", "test", "--needs"]);
        let a =
            super::build_scope_args("gitlab", ".gitlab-ci.yml", "job", &jobs[..1], "", &[], None);
        assert_eq!(a, vec!["--file", ".gitlab-ci.yml", "lint"]);
        // GitLab ignores the event (it has no trigger events).
        let a = super::build_scope_args(
            "gitlab",
            ".gitlab-ci.yml",
            "pipeline",
            &[],
            "workflow_dispatch",
            &[],
            None,
        );
        assert_eq!(a, vec!["--file", ".gitlab-ci.yml"]);
        let a = super::build_scope_args(
            "github",
            ".github/workflows/ci.yml",
            "pipeline",
            &[],
            "",
            &[],
            None,
        );
        assert_eq!(a, vec!["-W", ".github/workflows/ci.yml"]);
        let a = super::build_scope_args(
            "github",
            ".github/workflows/ci.yml",
            "job",
            &jobs[..1],
            "",
            &[],
            None,
        );
        assert_eq!(a, vec!["-W", ".github/workflows/ci.yml", "-j", "lint"]);
    }

    #[test]
    fn act_event_is_first_positional_argument() {
        // A non-empty event for act goes in as the first positional argument before -W.
        let a = super::build_scope_args(
            "github",
            ".github/workflows/ci.yml",
            "pipeline",
            &[],
            "workflow_dispatch",
            &[],
            None,
        );
        assert_eq!(
            a,
            vec!["workflow_dispatch", "-W", ".github/workflows/ci.yml"]
        );
        let jobs: Vec<String> = vec!["build".into()];
        let a = super::build_scope_args(
            "github",
            ".github/workflows/ci.yml",
            "job",
            &jobs,
            "tag",
            &[],
            None,
        );
        assert_eq!(
            a,
            vec!["tag", "-W", ".github/workflows/ci.yml", "-j", "build"]
        );
    }

    #[test]
    fn manual_jobs_produce_manual_flags() {
        // GitLab: one repeated `--manual <job>` per manual job at the end.
        let manual: Vec<String> = vec!["deploy".into(), "release".into()];
        let a = super::build_scope_args(
            "gitlab",
            ".gitlab-ci.yml",
            "pipeline",
            &[],
            "",
            &manual,
            None,
        );
        assert_eq!(
            a,
            vec![
                "--file",
                ".gitlab-ci.yml",
                "--manual",
                "deploy",
                "--manual",
                "release"
            ]
        );
        // Single job: positional argument PLUS --manual for the same job.
        let one: Vec<String> = vec!["deploy".into()];
        let a = super::build_scope_args("gitlab", ".gitlab-ci.yml", "job", &one, "", &one, None);
        assert_eq!(
            a,
            vec!["--file", ".gitlab-ci.yml", "deploy", "--manual", "deploy"]
        );
        // act/GitHub has no --manual: manual_jobs are ignored.
        let a = super::build_scope_args(
            "github",
            ".github/workflows/ci.yml",
            "pipeline",
            &[],
            "",
            &manual,
            None,
        );
        assert_eq!(a, vec!["-W", ".github/workflows/ci.yml"]);
    }

    #[test]
    fn vars_file_arg_per_provider() {
        // GitLab: --variables-file <path> at the end.
        let a = super::build_scope_args(
            "gitlab",
            ".gitlab-ci.yml",
            "pipeline",
            &[],
            "",
            &[],
            Some("/tmp/vars.yml"),
        );
        assert_eq!(
            a,
            vec![
                "--file",
                ".gitlab-ci.yml",
                "--variables-file",
                "/tmp/vars.yml"
            ]
        );
        // act: --env-file <path>.
        let a = super::build_scope_args(
            "github",
            ".github/workflows/ci.yml",
            "pipeline",
            &[],
            "",
            &[],
            Some("/tmp/vars.env"),
        );
        assert_eq!(
            a,
            vec![
                "-W",
                ".github/workflows/ci.yml",
                "--env-file",
                "/tmp/vars.env"
            ]
        );
    }

    #[test]
    fn var_key_allowlist() {
        for k in ["FOO", "_x", "A1_B2", "n"] {
            assert!(super::is_valid_var_key(k), "{k} should be valid");
        }
        for k in ["", "1FOO", "FOO BAR", "FOO-BAR", "FOO=BAR", "a.b", "$X"] {
            assert!(!super::is_valid_var_key(k), "{k} should be invalid");
        }
    }

    #[test]
    fn write_vars_file_gitlab_json_and_cleanup() {
        // Invalid keys are dropped; the file is valid JSON (= YAML), lives in the
        // repo directory and is deleted when the guard drops.
        let repo = tempfile::tempdir().unwrap();
        let vars = vec![
            ("FOO".to_string(), "bar \"baz\"\nqux".to_string()),
            ("bad key".to_string(), "x".to_string()), // dropped
        ];
        let path = {
            let f = super::write_vars_file(repo.path(), "gitlab", &vars)
                .unwrap()
                .unwrap();
            let content = std::fs::read_to_string(&f.path).unwrap();
            let v: serde_json::Value = serde_json::from_str(&content).unwrap();
            assert_eq!(v["FOO"], "bar \"baz\"\nqux");
            assert!(v.get("bad key").is_none(), "an invalid key must not get in");
            assert_eq!(f.path.extension().and_then(|e| e.to_str()), Some("yml"));
            // The file lives IN the repo (the runner's cwd).
            assert_eq!(f.path.parent(), Some(repo.path()));
            // The runner argument is a plain file name without a path prefix and
            // without cmd metacharacters (no command splitting under `cmd /C`).
            assert!(
                !f.arg.contains(['/', '\\']),
                "arg must not contain a path separator: {}",
                f.arg
            );
            assert!(
                f.arg
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-'),
                "arg may only contain digits/letters/./-: {}",
                f.arg
            );
            f.path.clone()
        }; // guard dropped here
        assert!(!path.exists(), "temp file must be deleted after drop");
    }

    #[test]
    fn write_vars_file_act_dotenv() {
        let repo = tempfile::tempdir().unwrap();
        let vars = vec![("TOKEN".to_string(), "a\"b\\c\nd".to_string())];
        let f = super::write_vars_file(repo.path(), "github", &vars)
            .unwrap()
            .unwrap();
        let content = std::fs::read_to_string(&f.path).unwrap();
        // dotenv: KEY="…escaped…" — newline/quote/backslash escaped.
        assert_eq!(content, "TOKEN=\"a\\\"b\\\\c\\nd\"\n");
        assert_eq!(f.path.extension().and_then(|e| e.to_str()), Some("env"));
    }

    #[test]
    fn write_vars_file_empty_without_valid_keys() {
        let repo = tempfile::tempdir().unwrap();
        let vars = vec![("1bad".to_string(), "x".to_string())];
        assert!(super::write_vars_file(repo.path(), "gitlab", &vars)
            .unwrap()
            .is_none());
        assert!(super::write_vars_file(repo.path(), "gitlab", &[])
            .unwrap()
            .is_none());
    }

    #[test]
    fn manual_jobs_in_scope_rules() {
        use crate::pipeline_graph::{PipelineGraph, PipelineJobNode};
        let mnode = |n: &str, stage: &str| PipelineJobNode {
            name: n.into(),
            stage: stage.into(),
            needs: vec![],
            when: "manual".into(),
            allow_failure: false,
            display_name: None,
        };
        let graph = PipelineGraph {
            provider: "gitlab".into(),
            config_file: ".gitlab-ci.yml".into(),
            stages: vec!["build".into(), "ship".into()],
            jobs: vec![
                node("build", "build", &[]),  // when="" (auto)
                mnode("deploy", "ship"),      // manual, safe
                mnode("ship (prod)", "ship"), // manual, UNSAFE (parentheses)
            ],
        };
        let all: Vec<String> = graph.jobs.iter().map(|j| j.name.clone()).collect();
        // Full pipeline: only the safely named manual job; the unsafe one stays out.
        assert_eq!(
            super::manual_jobs_in_scope(&graph, "gitlab", &all),
            vec!["deploy".to_string()]
        );
        // Single job = the manual job that was clicked.
        assert_eq!(
            super::manual_jobs_in_scope(&graph, "gitlab", &["deploy".into()]),
            vec!["deploy".to_string()]
        );
        // A non-manual job in scope produces no --manual.
        assert!(super::manual_jobs_in_scope(&graph, "gitlab", &["build".into()]).is_empty());
        // GitHub/act: never --manual (even with when=manual).
        assert!(super::manual_jobs_in_scope(&graph, "github", &all).is_empty());
    }

    #[test]
    fn event_allowlist() {
        for ev in ["push", "pull_request", "workflow_dispatch", "tag"] {
            assert!(super::is_valid_event(ev), "{ev} should be valid");
        }
        for ev in ["", "deploy; rm -rf", "PUSH", "release"] {
            assert!(!super::is_valid_event(ev), "{ev} should be invalid");
        }
    }

    #[test]
    fn invalid_event_for_act_is_rejected_without_a_slot() {
        let dir = tempfile::tempdir().unwrap();
        let state = super::PipelineState::default();
        // Event validation comes BEFORE slot reservation and graph load: the error
        // arrives even without a workflow file/runner and leaves nothing behind.
        let err = super::run_scope(
            &state,
            dir.path(),
            "github",
            ".github/workflows/ci.yml",
            "pipeline",
            None,
            &super::RunOptions {
                event: Some("evil; drop".into()),
                ..Default::default()
            },
            |_| {},
        )
        .unwrap_err();
        assert_eq!(code_of(&err), "invalid_event");
        assert!(state.0.lock().unwrap().is_empty());
    }

    #[test]
    fn config_validation_blocks_foreign_and_malicious_paths() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".gitlab-ci.yml"), "stages: [a]\n").unwrap();
        assert!(super::validate_config(dir.path(), "gitlab", ".gitlab-ci.yml").is_ok());
        // Not discovered / outside / wrong extension / traversal -> error.
        for bad in ["../evil.yml", "not-there.yml", ".gitlab-ci.txt", "-x.yml"] {
            let err = super::validate_config(dir.path(), "gitlab", bad).unwrap_err();
            assert_eq!(code_of(&err), "invalid_target", "must be rejected: {bad}");
        }
        // cmd metacharacters in the file name: even if the file EXISTS and would
        // be discovered, it must not pass (cmd /C injection vector).
        std::fs::write(dir.path().join("a&b.yml"), "stages: [a]\n").unwrap();
        assert!(super::validate_config(dir.path(), "gitlab", "a&b.yml").is_err());
        // Provider binding: the config has to have been discovered with EXACTLY
        // the requested provider — provider=github would otherwise feed the
        // GitLab config to act.
        let err = super::validate_config(dir.path(), "github", ".gitlab-ci.yml").unwrap_err();
        assert_eq!(code_of(&err), "invalid_target");

        // Relaxation (file picker): a manually chosen, NOT auto-discovered file in
        // a subfolder (discover only scans the root + .gitlab/) is allowed now —
        // as long as it is inside the repo, exists and the provider matches.
        std::fs::create_dir_all(dir.path().join("ci")).unwrap();
        std::fs::write(dir.path().join("ci/build.yml"), "stages: [build]\n").unwrap();
        assert!(super::validate_config(dir.path(), "gitlab", "ci/build.yml").is_ok());
        // The provider has to match the content (a GitLab file never goes to act).
        assert!(super::validate_config(dir.path(), "github", "ci/build.yml").is_err());
        // config_from_path derives the repo-relative path + provider from the absolute path.
        let abs = dir.path().join("ci").join("build.yml");
        let cfg = super::config_from_path(dir.path(), &abs.to_string_lossy()).unwrap();
        assert_eq!(cfg.path, "ci/build.yml");
        assert_eq!(cfg.provider, "gitlab");
        // File outside the repo -> rejected.
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("x.yml"), "stages: [a]\n").unwrap();
        assert!(super::config_from_path(
            dir.path(),
            &outside.path().join("x.yml").to_string_lossy()
        )
        .is_err());
    }

    #[test]
    fn heuristic_reads_only_the_prefix() {
        // The signal only appears AFTER the 64 KiB limit -> the file must not be
        // recognized as a CI config (proves that only the prefix is read).
        let dir = tempfile::tempdir().unwrap();
        let mut big = String::new();
        while big.len() <= 64 * 1024 {
            big.push_str("# filler\n");
        }
        big.push_str("stages: [x]\n");
        std::fs::write(dir.path().join("big.yml"), &big).unwrap();
        // Control: same size, signal at the START -> recognized.
        let good = format!("stages: [x]\n{big}");
        std::fs::write(dir.path().join("good.yml"), &good).unwrap();
        let paths: Vec<String> = super::discover_configs(dir.path())
            .into_iter()
            .map(|c| c.path)
            .collect();
        assert!(!paths.contains(&"big.yml".to_string()));
        assert!(paths.contains(&"good.yml".to_string()));
    }

    #[test]
    fn second_run_per_repo_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let state = super::PipelineState::default();
        let key = dir.path().to_string_lossy().to_lowercase();
        // Run A is active (a real PID in the slot) — run B has to fail with a
        // stable code WITHOUT touching A's entry (otherwise A would no longer be
        // cancellable). The early check bites BEFORE the graph load, so the test
        // needs neither a config nor a runner.
        state.0.lock().unwrap().insert(
            key.clone(),
            super::RunSlot {
                pid: 12345,
                cancel_requested: false,
            },
        );
        let err = super::run_scope(
            &state,
            dir.path(),
            "gitlab",
            ".gitlab-ci.yml",
            "pipeline",
            None,
            &super::RunOptions::default(),
            |_| {},
        )
        .unwrap_err();
        assert_eq!(code_of(&err), "run_active");
        assert_eq!(
            state.0.lock().unwrap().get(&key).map(|s| s.pid),
            Some(12345)
        );
    }

    #[test]
    fn stage_scope_for_github_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let state = super::PipelineState::default();
        // The guard comes BEFORE slot reservation and graph load — so the error
        // has to arrive even without an existing workflow file, and it must not
        // start a runner or leave a slot behind.
        let err = super::run_scope(
            &state,
            dir.path(),
            "github",
            ".github/workflows/ci.yml",
            "stage",
            Some("1"),
            &super::RunOptions::default(),
            |_| {},
        )
        .unwrap_err();
        match err {
            super::RunError::Rejected { code, message } => {
                assert_eq!(code, "invalid_scope");
                assert_eq!(message, "Stage runs are GitLab-only");
            }
            other => panic!("expected Rejected, got: {other:?}"),
        }
        assert!(state.0.lock().unwrap().is_empty());
    }

    #[test]
    fn unknown_scope_stable_code_without_slot_residue() {
        let dir = tempfile::tempdir().unwrap();
        let state = super::PipelineState::default();
        let err = super::run_scope(
            &state,
            dir.path(),
            "gitlab",
            ".gitlab-ci.yml",
            "everything",
            None,
            &super::RunOptions::default(),
            |_| {},
        )
        .unwrap_err();
        assert_eq!(code_of(&err), "invalid_scope");
        assert!(
            state.0.lock().unwrap().is_empty(),
            "the error path must not leave a state entry behind"
        );
    }

    #[test]
    fn failed_start_leaves_no_slot_behind() {
        let dir = tempfile::tempdir().unwrap();
        let state = super::PipelineState::default();
        // The config does not exist -> load_graph/validate_config fails. That now
        // happens BEFORE the slot reservation (the slot only protects the actual
        // run) — the state has to stay empty.
        let err = super::run_scope(
            &state,
            dir.path(),
            "gitlab",
            "not-there.yml",
            "pipeline",
            None,
            &super::RunOptions::default(),
            |_| {},
        )
        .unwrap_err();
        assert_eq!(code_of(&err), "invalid_target");
        assert!(
            state.0.lock().unwrap().is_empty(),
            "no state entry after a failed start"
        );
    }

    #[test]
    fn finalize_slot_releases_only_our_own_slot() {
        use std::collections::HashMap;
        let key = "repo".to_string();

        // Own slot (pid matches) -> release it, cancel_detected=false.
        let mut m: HashMap<String, super::RunSlot> = HashMap::new();
        m.insert(
            key.clone(),
            super::RunSlot {
                pid: 4242,
                cancel_requested: false,
            },
        );
        assert!(!super::finalize_slot(&mut m, &key, 4242));
        assert!(m.is_empty(), "our own slot is released");

        // Slot missing (cancel removed it) -> cancel_detected=true.
        let mut m: HashMap<String, super::RunSlot> = HashMap::new();
        assert!(super::finalize_slot(&mut m, &key, 4242));

        // Foreign slot (a NEW run, different pid) -> cancel_detected=true AND the
        // foreign slot stays untouched (otherwise the new run would no longer be
        // cancellable). That is the actual P9 race.
        let mut m: HashMap<String, super::RunSlot> = HashMap::new();
        m.insert(
            key.clone(),
            super::RunSlot {
                pid: 9999, // slot of a new run B
                cancel_requested: false,
            },
        );
        assert!(super::finalize_slot(&mut m, &key, 4242));
        assert_eq!(
            m.get(&key).map(|s| s.pid),
            Some(9999),
            "a foreign (new) slot must not be removed"
        );
    }

    #[test]
    fn cancel_on_reserved_slot_marks_cancel_ahead() {
        let dir = tempfile::tempdir().unwrap();
        let state = super::PipelineState::default();
        let key = dir.path().to_string_lossy().to_lowercase();
        // Slot only reserved (the child process does not exist yet): cancel must
        // not trigger kill_tree(0) and must not remove the entry, but pre-register
        // the cancellation — run_scope checks the flag after the spawn and then
        // ends immediately (no permanently blocked slot any more).
        state
            .0
            .lock()
            .unwrap()
            .insert(key.clone(), super::RunSlot::default());
        assert!(super::cancel(&state, dir.path()));
        let slot = state.0.lock().unwrap().get(&key).copied().unwrap();
        assert_eq!(slot.pid, super::RESERVED_PID);
        assert!(slot.cancel_requested);
    }

    #[test]
    fn output_with_timeout_kills_hanging_processes() {
        use std::process::Command;
        use std::time::Duration;
        // A blocking process + a tiny deadline -> a timeout error (instead of
        // hanging indefinitely like Command::output()).
        let mut c = if cfg!(windows) {
            let mut c = Command::new("cmd");
            c.args(["/C", "ping", "-n", "30", "127.0.0.1"]);
            c
        } else {
            let mut c = Command::new("sleep");
            c.arg("30");
            c
        };
        let start = std::time::Instant::now();
        let err = super::output_with_timeout(&mut c, Duration::from_millis(300)).unwrap_err();
        assert!(matches!(err, super::RunError::Timeout), "was: {err:?}");
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "must kill the process"
        );
        // A fast process delivers output as usual.
        let mut c = if cfg!(windows) {
            let mut c = Command::new("cmd");
            c.args(["/C", "echo", "hi"]);
            c
        } else {
            let mut c = Command::new("sh");
            c.args(["-c", "echo hi"]);
            c
        };
        let out = super::output_with_timeout(&mut c, Duration::from_secs(30)).unwrap();
        assert!(out.status.success());
        assert!(String::from_utf8_lossy(&out.stdout).contains("hi"));
    }

    #[test]
    fn stream_lines_survives_non_utf8_lines() {
        // BufRead::lines() would abort on the first InvalidData — stream_lines has
        // to deliver ALL lines (lossy, U+FFFD).
        let (tx, rx) = std::sync::mpsc::channel();
        super::stream_lines(&b"ok1\n\xFCbroken\nok2\n"[..], tx);
        let got: Vec<String> = rx.try_iter().collect();
        assert_eq!(got, vec!["ok1", "\u{FFFD}broken", "ok2"]);
    }

    #[test]
    fn stage_not_found_stable_code() {
        let g = testgraph();
        let err = super::targeted_jobs(&g, "stage", Some("does-not-exist")).unwrap_err();
        assert_eq!(code_of(&err), "stage_not_found");
        // Missing target -> invalid_target.
        let err = super::targeted_jobs(&g, "stage", None).unwrap_err();
        assert_eq!(code_of(&err), "invalid_target");
        let err = super::targeted_jobs(&g, "job", None).unwrap_err();
        assert_eq!(code_of(&err), "invalid_target");
    }

    #[test]
    fn arg_validation_only_for_process_arguments() {
        let g = testgraph();
        // scope=pipeline: NO job names go in as process arguments — the legal
        // graph job "check (nightly)" must not block the whole run.
        let targeted = super::targeted_jobs(&g, "pipeline", None).unwrap();
        assert!(targeted.contains(&"check (nightly)".to_string()));
        let args = super::arg_jobs_for_scope(&g, "pipeline", None, &targeted).unwrap();
        assert!(args.is_empty());
        // scope=job: the target reaches the command line -> the allowlist applies.
        let targeted = super::targeted_jobs(&g, "job", Some("check (nightly)")).unwrap();
        let err =
            super::arg_jobs_for_scope(&g, "job", Some("check (nightly)"), &targeted).unwrap_err();
        assert_eq!(code_of(&err), "invalid_target");
        // scope=stage: the stage's jobs are checked, transitive needs are NOT
        // (--needs pulls those in, they never reach the command line).
        let targeted = super::targeted_jobs(&g, "stage", Some("ship")).unwrap();
        assert!(targeted.contains(&"check (nightly)".to_string()));
        let args = super::arg_jobs_for_scope(&g, "stage", Some("ship"), &targeted).unwrap();
        assert_eq!(args, vec!["deploy"]);
        // … but a stage WITH an unsafe job is rejected.
        let targeted = super::targeted_jobs(&g, "stage", Some("check")).unwrap();
        let err = super::arg_jobs_for_scope(&g, "stage", Some("check"), &targeted).unwrap_err();
        assert_eq!(code_of(&err), "invalid_target");
    }

    #[test]
    fn status_events_only_for_targeted_attribution_for_all() {
        use std::collections::{HashMap, HashSet};
        let by_len: Vec<String> = vec!["My Build".into(), "extra".into(), "build".into()];
        let alias: HashMap<String, String> =
            HashMap::from([("My Build".to_string(), "build".to_string())]);
        let targeted: HashSet<String> = HashSet::from(["build".to_string()]);
        let mut started: HashSet<String> = HashSet::new();
        // An act bracket with a DISPLAY NAME is mapped onto the job id (events go
        // through the node name); targeted -> running transition.
        let (job, run) = super::attribute_and_status(
            &by_len,
            &alias,
            &targeted,
            &mut started,
            "[CI/My Build] | echo hi",
        );
        assert_eq!(job.as_deref(), Some("build"));
        assert_eq!(run.as_deref(), Some("build"));
        // Repetition: no second running transition.
        let (_, run) = super::attribute_and_status(
            &by_len,
            &alias,
            &targeted,
            &mut started,
            "[CI/My Build] | continuing",
        );
        assert!(run.is_none());
        // A non-targeted job (e.g. pulled in via --needs): the line is attributed
        // for log grouping, but there is NO status event — otherwise a
        // false-positive match would hang on "running" in the UI forever.
        let (job, run) = super::attribute_and_status(
            &by_len,
            &alias,
            &targeted,
            &mut started,
            "extra > gets dragged along",
        );
        assert_eq!(job.as_deref(), Some("extra"));
        assert!(run.is_none());
        assert!(!started.contains("extra"));
    }

    #[test]
    fn timeout_counts_as_cancel_on_finalize() {
        use crate::pipeline_graph::{finalize_statuses, JobStatus};
        use std::collections::HashSet;
        // Spec: cancellation => canceled — including on the time limit (timed_out=true).
        assert!(super::run_canceled(false, true));
        assert!(super::run_canceled(true, false));
        assert!(!super::run_canceled(false, false));
        let targeted: Vec<String> = vec!["a".into(), "b".into()];
        let started: HashSet<String> = HashSet::from(["a".to_string()]);
        let fin = finalize_statuses(&targeted, &started, super::run_canceled(false, true), 1);
        assert_eq!(
            fin[0].1,
            JobStatus::Canceled,
            "started -> canceled, not failed"
        );
        assert_eq!(fin[1].1, JobStatus::Skipped, "never started -> skipped");
    }

    #[test]
    fn stage_jobs_with_transitive_needs() {
        use crate::pipeline_graph::PipelineGraph;
        let g = PipelineGraph {
            provider: "gitlab".into(),
            config_file: ".gitlab-ci.yml".into(),
            stages: vec!["build".into(), "test".into(), "ship".into()],
            jobs: vec![
                node("build", "build", &[]),
                node("test", "test", &["build"]),
                node("deploy", "ship", &["test"]),
            ],
        };
        // Stage "ship": deploy + transitive needs (test, build); order = graph order.
        let jobs = super::stage_jobs_with_needs(&g, "ship");
        assert_eq!(jobs, vec!["build", "test", "deploy"]);
        let jobs = super::stage_jobs_with_needs(&g, "test");
        assert_eq!(jobs, vec!["build", "test"]);
    }
    /// act works directly against the Docker daemon and needs no host tools —
    /// only gitlab-ci-local rsyncs the files itself.
    #[test]
    fn host_tools_only_required_for_gitlab() {
        assert!(missing_host_tools("github").is_empty());
        assert!(missing_host_tools("").is_empty());
        // For gitlab it is actually checked: the result may only consist of the
        // known tools (on CI machines they are often present, on Windows rsync is
        // typically missing).
        for t in missing_host_tools("gitlab") {
            assert!(
                t == "rsync" || t == "bash",
                "unexpected host tool reported: {t}"
            );
        }
    }
}
