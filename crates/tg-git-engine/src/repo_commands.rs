//! Commands a repository's OWN configuration makes git run.
//!
//! As far as git is concerned, a repo-local `.git/config` (and `.git/hooks`) is
//! executable code: `core.fsmonitor` names a program git starts on every index
//! refresh, filter drivers run on status and checkout, hooks run on commits,
//! ref updates and index writes, `core.sshCommand` and credential helpers run
//! during transfers. A clone never carries any of that — the risk is a `.git`
//! directory someone else supplied (an unpacked archive, a network share, a USB
//! stick). The rules (ARCHITECTURE.md, "Foreign repositories"):
//!
//! - A repo-local `core.fsmonitor` COMMAND never runs. A boolean — git's builtin
//!   daemon, which terra-git switches on itself — is left alone. Nor does a
//!   repo-local `submodule.<n>.update = !command`: it becomes `checkout`.
//! - Calls the app makes on its own ([`on_its_own`]: status, history, commit
//!   graph, the automatic fetch) run no hooks and no repo-local filter command
//!   other than git-lfs. Opening a repository, or leaving it open, runs nothing
//!   it defines.
//! - Transfers refuse repo-local credential helpers, askpass, proxy commands,
//!   upload/receive-pack programs of local remotes and any `core.sshCommand`
//!   that is more than a plain `ssh` call ([`transfer_command`]).
//!
//! What the user starts — a commit, a merge, a checkout — behaves exactly as
//! with git itself, hooks and filters included.
//!
//! The overrides travel as `GIT_CONFIG_COUNT`/`GIT_CONFIG_KEY_n`/
//! `GIT_CONFIG_VALUE_n` (command-line scope, so they beat every config file):
//! unlike `-c key=value`, a driver name containing `=` cannot shift where the key
//! ends and the value begins.

use std::cell::Cell;
use std::path::Path;
use std::process::Command;

use git2::ConfigLevel;

thread_local! {
    static ON_ITS_OWN: Cell<bool> = const { Cell::new(false) };
}

/// Runs `f` as a call the app makes on its own (see the module docs): every git
/// process started on this thread meanwhile gets the stricter rules.
pub fn on_its_own<T>(f: impl FnOnce() -> T) -> T {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            ON_ITS_OWN.with(|c| c.set(self.0));
        }
    }
    let _restore = Restore(ON_ITS_OWN.with(|c| c.replace(true)));
    f()
}

/// The values `git lfs install --local` writes — the one repo-local filter that
/// still runs on the app's own calls (an LFS status needs its clean filter).
const GIT_LFS_FILTERS: &[&str] = &[
    "git-lfs clean -- %f",
    "git-lfs smudge -- %f",
    "git-lfs smudge --skip -- %f",
    "git-lfs filter-process",
    "git-lfs filter-process --skip",
];

fn is_repo_level(level: ConfigLevel) -> bool {
    matches!(level, ConfigLevel::Local | ConfigLevel::Worktree)
}

/// Every repo-level (`.git/config`, `config.worktree`, files they include)
/// entry as (lower-cased name with the subsection kept, value).
fn repo_level_entries(dir: &Path) -> Vec<(String, String)> {
    let Ok(repo) = git2::Repository::discover(dir) else {
        return Vec::new();
    };
    let Ok(config) = repo.config() else {
        return Vec::new();
    };
    let Ok(mut entries) = config.entries(None) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    while let Some(entry) = entries.next() {
        let Ok(entry) = entry else { continue };
        if !is_repo_level(entry.level()) {
            continue;
        }
        let name = String::from_utf8_lossy(entry.name_bytes()).into_owned();
        let value = String::from_utf8_lossy(entry.value_bytes()).into_owned();
        out.push((name, value));
    }
    out
}

/// Where hooks are looked up during the app's own calls: this program's own
/// file. It is a FILE, so `<it>/pre-commit` can never exist — unlike an empty
/// directory, nothing can be placed into it.
fn no_hooks_path() -> String {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "/dev/null".to_string())
}

/// The config overrides for one git call in `dir`, as (key, value) pairs.
pub(crate) fn overrides(dir: &Path) -> Vec<(String, String)> {
    let on_its_own = ON_ITS_OWN.with(Cell::get);
    let mut fsmonitor_command = false;
    let mut drivers: Vec<String> = Vec::new();
    let mut submodule_commands: Vec<String> = Vec::new();
    for (name, value) in repo_level_entries(dir) {
        if name == "core.fsmonitor" {
            fsmonitor_command |= git2::Config::parse_bool(value.as_str()).is_err();
        } else if name.starts_with("submodule.") && name.ends_with(".update") {
            // `update = !command`: run by *Update submodules*. git ignores it in
            // .gitmodules for that reason; in .git/config it still counts.
            if value.trim_start().starts_with('!') {
                submodule_commands.push(name);
            }
        } else if let Some(rest) = name.strip_prefix("filter.") {
            if let Some((driver, key)) = rest.rsplit_once('.') {
                let command = matches!(key, "clean" | "smudge" | "process");
                if command
                    && !GIT_LFS_FILTERS.contains(&value.trim())
                    && !drivers.iter().any(|d| d == driver)
                {
                    drivers.push(driver.to_string());
                }
            }
        }
    }

    let mut out = Vec::new();
    if fsmonitor_command {
        out.push(("core.fsmonitor".to_string(), "false".to_string()));
    }
    for key in submodule_commands {
        out.push((key, "checkout".to_string()));
    }
    if on_its_own {
        out.push(("core.hooksPath".to_string(), no_hooks_path()));
        for d in drivers {
            // An empty command means "no filter" to git; `required` would turn
            // that into a hard error, so it goes too.
            for key in ["clean", "smudge", "process"] {
                out.push((format!("filter.{d}.{key}"), String::new()));
            }
            out.push((format!("filter.{d}.required"), "false".to_string()));
        }
    }
    out
}

/// Puts [`overrides`] for `dir` onto `cmd`. Entries the user's own environment
/// already carries in `GIT_CONFIG_COUNT` stay first.
pub(crate) fn apply(cmd: &mut Command, dir: &Path) {
    let pairs = overrides(dir);
    if pairs.is_empty() {
        return;
    }
    let base: usize = std::env::var("GIT_CONFIG_COUNT")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0);
    for (i, (key, value)) in pairs.iter().enumerate() {
        cmd.env(format!("GIT_CONFIG_KEY_{}", base + i), key);
        cmd.env(format!("GIT_CONFIG_VALUE_{}", base + i), value);
    }
    cmd.env("GIT_CONFIG_COUNT", (base + pairs.len()).to_string());
}

/// The first repo-level entry that would make a transfer run a program the
/// repository itself defines — `None` when there is none. See the module docs.
pub(crate) fn transfer_command(repo_path: &Path) -> Option<String> {
    let entries = repo_level_entries(repo_path);
    let url_of = |remote: &str| {
        let key = format!("remote.{remote}.url");
        git2::Repository::discover(repo_path)
            .and_then(|r| r.config())
            .and_then(|c| c.get_string(&key))
            .ok()
    };
    for (name, value) in &entries {
        // `credential.helper` and the URL-scoped `credential.<url>.helper`.
        let credential = name.starts_with("credential.") && name.ends_with(".helper");
        let program = matches!(name.as_str(), "core.askpass" | "core.gitproxy");
        let ssh = name == "core.sshcommand" && !is_plain_ssh(value);
        if credential || program || ssh {
            return Some(name.clone());
        }
        if let Some(rest) = name.strip_prefix("remote.") {
            if let Some((remote, key)) = rest.rsplit_once('.') {
                // On ssh/https these name a program on the SERVER; only a local
                // remote runs them on this machine.
                if matches!(key, "uploadpack" | "receivepack")
                    && url_of(remote).is_some_and(|u| is_local_transport(&u))
                {
                    return Some(name.clone());
                }
            }
        }
    }
    None
}

/// Whether git reaches `url` without a network transport (a path or `file://`),
/// i.e. runs upload-pack/receive-pack on this machine. git's rule: scp-like
/// syntax (`host:path`) needs the colon before the first slash; a Windows drive
/// letter is a path.
pub(crate) fn is_local_transport(url: &str) -> bool {
    let url = url.trim();
    if let Some((scheme, _)) = url.split_once("://") {
        return scheme.eq_ignore_ascii_case("file");
    }
    match url.find(':') {
        None => true,
        Some(i) => {
            let before = &url[..i];
            let drive = i == 1 && before.chars().all(|c| c.is_ascii_alphabetic());
            drive || before.contains('/') || before.contains('\\')
        }
    }
}

/// Splits a command line the way a shell would for the plain cases: words,
/// single and double quotes. `None` when it uses anything else a shell
/// interprets (variables, substitution, redirection, chaining) — such a command
/// is never "plain".
fn split_command(command: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    for c in command.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => word.push(c),
            None => match c {
                '\'' | '"' => {
                    quote = Some(c);
                    in_word = true;
                }
                c if c.is_whitespace() => {
                    if in_word {
                        words.push(std::mem::take(&mut word));
                        in_word = false;
                    }
                }
                '$' | '`' | ';' | '|' | '&' | '<' | '>' | '(' | ')' | '{' | '}' | '\n' | '*'
                | '?' => return None,
                c => {
                    word.push(c);
                    in_word = true;
                }
            },
        }
        if quote.is_some() && matches!(c, '$' | '`' | '\n') {
            return None;
        }
    }
    if quote.is_some() {
        return None;
    }
    if in_word {
        words.push(word);
    }
    Some(words)
}

/// Whether `program` is OpenSSH's `ssh` (any directory, `.exe` or not).
fn is_ssh_program(program: &str) -> bool {
    let base = program
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(program)
        .to_ascii_lowercase();
    base == "ssh" || base == "ssh.exe"
}

/// Whether a command line starts OpenSSH's `ssh` — the user's own
/// `core.sshCommand` then gets BatchMode appended.
pub(crate) fn starts_ssh(command: &str) -> bool {
    split_command(command)
        .and_then(|w| w.first().cloned())
        .is_some_and(|p| is_ssh_program(&p))
}

/// `-o` options of a plain ssh call: identities, ports, users, host-key and
/// algorithm choices. Missing on purpose: everything that starts a program or
/// loads code (ProxyCommand, LocalCommand, KnownHostsCommand, Match, Include,
/// PKCS11Provider, SecurityKeyProvider) and ForwardAgent.
const PLAIN_SSH_OPTIONS: &[&str] = &[
    "addkeystoagent",
    "batchmode",
    "certificatefile",
    "checkhostip",
    "ciphers",
    "compression",
    "connecttimeout",
    "hostkeyalgorithms",
    "hostkeyalias",
    "hostname",
    "identitiesonly",
    "identityagent",
    "identityfile",
    "kexalgorithms",
    "loglevel",
    "macs",
    "port",
    "preferredauthentications",
    "pubkeyacceptedalgorithms",
    "pubkeyacceptedkeytypes",
    "serveralivecountmax",
    "serveraliveinterval",
    "stricthostkeychecking",
    "updatehostkeys",
    "user",
    "userknownhostsfile",
];

/// Whether a repo-level `core.sshCommand` is a plain `ssh` call — the program
/// found through PATH (a path could point into the supplied folder) with
/// identity, port, user and [`PLAIN_SSH_OPTIONS`] only. The common per-repo
/// setup `ssh -i ~/.ssh/work_key -o IdentitiesOnly=yes` qualifies.
pub(crate) fn is_plain_ssh(command: &str) -> bool {
    let Some(words) = split_command(command) else {
        return false;
    };
    let Some((program, args)) = words.split_first() else {
        return false;
    };
    if !matches!(program.to_ascii_lowercase().as_str(), "ssh" | "ssh.exe") {
        return false;
    }
    let option_ok = |opt: &str| {
        let key = opt
            .split(['=', ' ', '\t'])
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        PLAIN_SSH_OPTIONS.contains(&key.as_str())
    };
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "-4" | "-6" | "-C" | "-q" | "-T" | "-v" | "-vv" | "-vvv" | "-x" | "-a" => {}
            "-i" | "-p" | "-l" => {
                if i + 1 >= args.len() {
                    return false;
                }
                i += 1;
            }
            "-o" => {
                let Some(opt) = args.get(i + 1) else {
                    return false;
                };
                if !option_ok(opt) {
                    return false;
                }
                i += 1;
            }
            _ => {
                let joined = a.strip_prefix("-o").filter(|o| !o.is_empty());
                let attached = ["-i", "-p", "-l"]
                    .iter()
                    .any(|f| a.starts_with(f) && a.len() > 2);
                match joined {
                    Some(opt) if option_ok(opt) => {}
                    Some(_) => return false,
                    None if attached => {}
                    None => return false,
                }
            }
        }
        i += 1;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_with(entries: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        let mut cfg = repo
            .config()
            .unwrap()
            .open_level(ConfigLevel::Local)
            .unwrap();
        for (k, v) in entries {
            cfg.set_str(k, v).unwrap();
        }
        dir
    }

    fn keys(pairs: &[(String, String)]) -> Vec<&str> {
        pairs.iter().map(|(k, _)| k.as_str()).collect()
    }

    #[test]
    fn a_repo_local_fsmonitor_command_is_switched_off_always() {
        let dir = repo_with(&[("core.fsmonitor", "sh -c 'touch pwned'")]);
        let o = overrides(dir.path());
        assert_eq!(o, vec![("core.fsmonitor".to_string(), "false".to_string())]);
        // The builtin daemon (a boolean, set by terra-git itself) stays.
        let dir = repo_with(&[("core.fsmonitor", "true")]);
        assert!(overrides(dir.path()).is_empty());
    }

    #[test]
    fn a_submodule_update_command_becomes_checkout() {
        let dir = repo_with(&[
            ("submodule.lib.update", "!sh -c 'touch pwned'"),
            ("submodule.other.update", "rebase"),
        ]);
        assert_eq!(
            overrides(dir.path()),
            vec![("submodule.lib.update".to_string(), "checkout".to_string())]
        );
    }

    #[test]
    fn the_apps_own_calls_run_no_hooks_and_no_foreign_filter() {
        let dir = repo_with(&[
            ("filter.evil.clean", "sh -c 'touch pwned'"),
            ("filter.lfs.clean", "git-lfs clean -- %f"),
            ("filter.lfs.process", "git-lfs filter-process"),
        ]);
        // What the user starts behaves as with git: nothing overridden.
        assert!(overrides(dir.path()).is_empty());
        let o = on_its_own(|| overrides(dir.path()));
        let k = keys(&o);
        assert!(k.contains(&"core.hooksPath"), "{k:?}");
        for key in ["clean", "smudge", "process", "required"] {
            assert!(k.contains(&format!("filter.evil.{key}").as_str()), "{k:?}");
        }
        assert!(!k.iter().any(|k| k.starts_with("filter.lfs.")), "{k:?}");
        // The mark ends with the call.
        assert!(overrides(dir.path()).is_empty());
    }

    #[test]
    fn included_files_count_as_the_repository_level() {
        let dir = repo_with(&[]);
        let inc = dir.path().join("evil.cfg");
        std::fs::write(&inc, "[core]\n\tfsmonitor = sh -c 'touch pwned'\n").unwrap();
        let repo = git2::Repository::open(dir.path()).unwrap();
        repo.config()
            .unwrap()
            .open_level(ConfigLevel::Local)
            .unwrap()
            .set_str("include.path", inc.to_str().unwrap())
            .unwrap();
        assert_eq!(keys(&overrides(dir.path())), vec!["core.fsmonitor"]);
    }

    #[test]
    fn a_hook_and_a_filter_never_run_on_the_apps_own_status() {
        use std::process::Stdio;
        let dir = repo_with(&[("filter.evil.clean", "sh -c 'echo x > ran-filter'")]);
        let p = dir.path();
        std::fs::write(p.join(".gitattributes"), "*.txt filter=evil\n").unwrap();
        std::fs::write(p.join("a.txt"), "a\n").unwrap();
        let hooks = p.join(".git").join("hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        std::fs::write(
            hooks.join("post-index-change"),
            "#!/bin/sh\necho x > ran-hook\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                hooks.join("post-index-change"),
                std::fs::Permissions::from_mode(0o755),
            )
            .unwrap();
        }
        let run = |args: &[&str]| {
            let mut cmd = Command::new("git");
            cmd.arg("-C")
                .arg(p)
                .args(args)
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            apply(&mut cmd, p);
            cmd.status().unwrap()
        };
        on_its_own(|| {
            run(&["add", "a.txt"]);
            run(&["status", "--porcelain"]);
        });
        assert!(!p.join("ran-filter").exists(), "the foreign filter ran");
        assert!(!p.join("ran-hook").exists(), "the hook ran");
        // Control: started by the user, git runs both — so the test can see it.
        std::fs::write(p.join("a.txt"), "b\n").unwrap();
        run(&["add", "a.txt"]);
        assert!(p.join("ran-filter").exists(), "control: filter did not run");
        assert!(p.join("ran-hook").exists(), "control: hook did not run");
    }

    #[test]
    fn transfers_refuse_programs_the_repository_defines() {
        for (k, v) in [
            ("credential.helper", "!sh -c 'touch pwned'"),
            (
                "credential.https://example.com.helper",
                "!sh -c 'touch pwned'",
            ),
            ("core.askPass", "/tmp/x"),
            ("core.gitProxy", "/tmp/x"),
            ("core.sshCommand", "sh -c 'touch pwned'"),
            ("core.sshCommand", "ssh -o ProxyCommand=evil"),
            ("core.sshCommand", "./ssh"),
        ] {
            let dir = repo_with(&[(k, v)]);
            assert!(transfer_command(dir.path()).is_some(), "{k} = {v}");
        }
        // Local remote: upload-pack runs here. Network remote: on the server.
        let dir = repo_with(&[
            ("remote.origin.url", "../other"),
            ("remote.origin.uploadpack", "sh -c 'touch pwned'"),
        ]);
        assert_eq!(
            transfer_command(dir.path()).as_deref(),
            Some("remote.origin.uploadpack")
        );
        let dir = repo_with(&[
            ("remote.origin.url", "git@example.com:a/b.git"),
            ("remote.origin.uploadpack", "/opt/git/bin/git-upload-pack"),
        ]);
        assert_eq!(transfer_command(dir.path()), None);
        // The common per-repo identity stays allowed.
        let dir = repo_with(&[(
            "core.sshCommand",
            "ssh -i ~/.ssh/work_key -o IdentitiesOnly=yes",
        )]);
        assert_eq!(transfer_command(dir.path()), None);
    }

    #[test]
    fn plain_ssh_calls() {
        for ok in [
            "ssh",
            "ssh -i ~/.ssh/id_work -o IdentitiesOnly=yes",
            "ssh -i \"C:/Users/A B/.ssh/key\" -p 2222",
            "ssh.exe -oStrictHostKeyChecking=yes -l git",
            "ssh -o \"UserKnownHostsFile /dev/null\"",
        ] {
            assert!(is_plain_ssh(ok), "{ok}");
        }
        for bad in [
            "ssh -o ProxyCommand=evil",
            "ssh -oLocalCommand=evil -o PermitLocalCommand=yes",
            "ssh -F evil.cfg",
            "/tmp/x/ssh",
            "ssh $(evil)",
            "ssh `evil`",
            "ssh; evil",
            "ssh -i",
            "plink -batch",
        ] {
            assert!(!is_plain_ssh(bad), "{bad}");
        }
        assert!(starts_ssh("\"C:/Program Files/Git/usr/bin/ssh.exe\" -i k"));
        assert!(!starts_ssh("plink -batch"));
    }

    #[test]
    fn local_transports() {
        for local in [
            "../other",
            "/srv/repo.git",
            "file:///srv/repo",
            "C:/repos/x",
            "C:\\repos\\x",
            "./a:b",
        ] {
            assert!(is_local_transport(local), "{local}");
        }
        for remote in [
            "git@github.com:a/b.git",
            "https://x/y",
            "ssh://git@x/y",
            "host:path",
        ] {
            assert!(!is_local_transport(remote), "{remote}");
        }
    }
}
