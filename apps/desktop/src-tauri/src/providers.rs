//! Provider accounts: metadata in `providers.json` (app config dir), tokens
//! exclusively in the OS keychain (tg-auth).

use std::path::PathBuf;

use tauri::{AppHandle, Manager};
use tg_domain::ProviderAccount;

fn store_file(app: &AppHandle) -> Option<PathBuf> {
    let dir = app.path().app_config_dir().ok()?;
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join("providers.json"))
}

pub fn list(app: &AppHandle) -> Vec<ProviderAccount> {
    let Some(file) = store_file(app) else {
        return Vec::new();
    };
    std::fs::read_to_string(file)
        .ok()
        .and_then(|s| serde_json::from_str(s.trim_start_matches('\u{feff}')).ok())
        .unwrap_or_default()
}

fn write(app: &AppHandle, accounts: &[ProviderAccount]) {
    if let Some(file) = store_file(app) {
        if let Ok(json) = serde_json::to_string_pretty(accounts) {
            // Atomic (temp+rename): a truncated providers.json would silently be
            // read as "no accounts" by the tolerant reader.
            if let Err(e) = crate::jsonstore::atomic_write(&file, json.as_bytes()) {
                tracing::warn!("could not write providers.json: {e}");
            }
        }
    }
}

/// Adds an account or replaces the existing one for the same host.
pub fn upsert(app: &AppHandle, account: ProviderAccount) {
    let mut accounts = list(app);
    accounts.retain(|a| !a.host.eq_ignore_ascii_case(&account.host));
    accounts.push(account);
    accounts.sort_by(|a, b| a.host.cmp(&b.host));
    write(app, &accounts);
    sync_insecure_tls_hosts(app);
}

pub fn remove(app: &AppHandle, host: &str) {
    let mut accounts = list(app);
    accounts.retain(|a| !a.host.eq_ignore_ascii_case(host));
    write(app, &accounts);
    sync_insecure_tls_hosts(app);
}

/// Mirrors the "TLS verification off" hosts into the engine process state
/// ([`tg_git_engine::set_insecure_tls_hosts`]) so the git sidecar
/// (fetch/pull/push/clone) sets a host-bound `http.sslVerify=false` for
/// self-signed self-hosted instances. Without this mirroring an account could
/// list/create MRs (REST client) but not synchronize via git — the instance
/// would be practically unusable.
/// Call once at startup; after that it happens automatically on every account change.
/// Deliberately no `env::set_var` any more: setenv at runtime is UB on
/// Unix next to `Command::spawn` from other threads.
pub fn sync_insecure_tls_hosts(app: &AppHandle) {
    tg_git_engine::set_insecure_tls_hosts(insecure_hosts_of(&list(app)));
}

/// Hosts with TLS verification disabled. Pure & tested — security-relevant,
/// because this list controls for which hosts the git sidecar sets
/// `sslVerify=false`.
fn insecure_hosts_of(accounts: &[ProviderAccount]) -> Vec<String> {
    accounts
        .iter()
        .filter(|a| a.insecure_tls)
        .map(|a| a.host.clone())
        .filter(|h| !h.is_empty())
        .collect()
}

/// Looks up the account for a remote (host + project path) and returns the
/// EFFECTIVE project path for the provider API alongside it. Handles subpath
/// installations: an account host containing '/' (e.g.
/// "example.com/gitlab") matches when the host part is right AND the project
/// path starts with the subpath — the subpath is then stripped (the API expects
/// "group/project", not "gitlab/group/project").
pub fn find_for_remote(
    app: &AppHandle,
    host: &str,
    repo_path: &str,
) -> Option<(ProviderAccount, String)> {
    match_account(&list(app), host, repo_path)
}

/// Pure & tested: matching/stripping logic of [`find_for_remote`].
/// The most specific hit wins (longest matching subpath before the bare host
/// account — both can be configured side by side).
fn match_account(
    accounts: &[ProviderAccount],
    host: &str,
    repo_path: &str,
) -> Option<(ProviderAccount, String)> {
    let mut best: Option<(ProviderAccount, String, usize)> = None;
    for account in accounts {
        let (acc_host, sub) = match account.host.split_once('/') {
            Some((h, s)) => (h, s.trim_matches('/')),
            None => (account.host.as_str(), ""),
        };
        if !acc_host.eq_ignore_ascii_case(host) {
            continue;
        }
        let candidate = if sub.is_empty() {
            (account.clone(), repo_path.to_string(), 0)
        } else {
            match strip_subpath(repo_path, sub) {
                Some(rest) => (account.clone(), rest, sub.len()),
                None => continue,
            }
        };
        if best.as_ref().is_none_or(|(_, _, s)| candidate.2 > *s) {
            best = Some(candidate);
        }
    }
    best.map(|(account, path, _)| (account, path))
}

/// Strips `sub` from the beginning of `repo_path`, segment-exact (and
/// ASCII-case-insensitive, like the host). `None` if the path does not start
/// with the subpath or no project path remains afterwards.
fn strip_subpath(repo_path: &str, sub: &str) -> Option<String> {
    if repo_path.len() < sub.len() {
        return None;
    }
    let (head, rest) = repo_path.split_at(sub.len());
    if !head.eq_ignore_ascii_case(sub) {
        return None;
    }
    // Enforce a segment boundary: "gitlabx/…" must not match "gitlab".
    let rest = rest.strip_prefix('/')?;
    (!rest.is_empty()).then(|| rest.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tg_domain::ProviderKind;

    fn account(host: &str, insecure: bool) -> ProviderAccount {
        ProviderAccount {
            host: host.into(),
            kind: ProviderKind::Gitlab,
            username: "u".into(),
            insecure_tls: insecure,
        }
    }

    #[test]
    fn only_insecure_marked_hosts_end_up_in_the_engine_state() {
        // No insecure account -> empty list (engine state: no opt-outs).
        assert!(insecure_hosts_of(&[account("github.com", false)]).is_empty());
        assert!(insecure_hosts_of(&[]).is_empty());

        // Only the insecure-marked, non-empty hosts.
        let accounts = [
            account("git.intern.example", true),
            account("github.com", false),
            account("", true), // an empty host is filtered out
            account("gitlab.example.com", true),
        ];
        assert_eq!(
            insecure_hosts_of(&accounts),
            vec!["git.intern.example", "gitlab.example.com"]
        );
    }

    #[test]
    fn match_account_finds_subpath_account_and_strips_subpath() {
        // F27: the account of a subpath installation ("example.com/gitlab") has
        // to be found for remote https://example.com/gitlab/group/repo.git — and
        // the subpath disappears from the project path.
        let accounts = [
            account("example.com/gitlab", false),
            account("github.com", false),
        ];
        let (acc, path) = match_account(&accounts, "example.com", "gitlab/group/repo")
            .expect("the subpath account must match");
        assert_eq!(acc.host, "example.com/gitlab");
        assert_eq!(path, "group/repo", "the subpath is stripped");

        // Segment boundary: "gitlabx/…" does NOT match "…/gitlab".
        assert!(match_account(&accounts, "example.com", "gitlabx/group/repo").is_none());
        // A different path without the subpath: no hit.
        assert!(match_account(&accounts, "example.com", "other/repo").is_none());
        // Only the subpath without a project path: no hit.
        assert!(match_account(&accounts, "example.com", "gitlab").is_none());
        // Foreign host: no hit.
        assert!(match_account(&accounts, "example.org", "gitlab/group/repo").is_none());
    }

    #[test]
    fn match_account_bare_host_and_most_specific_hit() {
        // Bare host account: the project path stays unchanged.
        let plain = [account("github.com", false)];
        let (acc, path) = match_account(&plain, "github.com", "foo/bar").unwrap();
        assert_eq!(acc.host, "github.com");
        assert_eq!(path, "foo/bar");

        // Bare AND subpath account side by side: the most specific one wins.
        let both = [
            account("example.com", false),
            account("example.com/gitlab", false),
        ];
        let (acc, path) = match_account(&both, "example.com", "gitlab/g/r").unwrap();
        assert_eq!(acc.host, "example.com/gitlab");
        assert_eq!(path, "g/r");
        // Without the subpath in the path, the bare host account applies.
        let (acc, path) = match_account(&both, "example.com", "g/r").unwrap();
        assert_eq!(acc.host, "example.com");
        assert_eq!(path, "g/r");

        // Case: host AND subpath are ASCII-case-insensitive.
        let cased = [account("example.com/GitLab", false)];
        let (_, path) = match_account(&cased, "example.com", "gitlab/g/r").unwrap();
        assert_eq!(path, "g/r");
    }
}
