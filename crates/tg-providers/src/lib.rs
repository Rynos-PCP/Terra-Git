//! Hosting provider layer: the two-layer principle.
//!
//! Git transport still goes through the system-git sidecar; this crate ONLY
//! speaks the hosting REST APIs (GitHub incl. GHES, GitLab incl. self-hosted)
//! and returns the neutral domain model (`ChangeRequest`, `CiStatus`).
//! Deliberate deviation from the original design: a lean reqwest client
//! instead of the octocrab/gitlab crates — we only need a handful of endpoints.

mod error;
mod gitea;
mod github;
mod gitlab;
mod time;
mod url;

use std::time::Duration;

pub use error::{ProviderError, Result};
pub use url::{parse_remote_url, RemoteTarget};

use tg_domain::{ChangeRequest, NewChangeRequest, ProviderKind};

/// For how many change requests the CI status is looked up (bounds the number
/// of requests; order = most recently updated first).
pub(crate) const MAX_CI_LOOKUPS: usize = 25;

/// Authenticated client for ONE provider instance (host + token).
pub struct ProviderClient {
    pub(crate) http: reqwest::Client,
    pub(crate) kind: ProviderKind,
    /// REST base including the scheme, without a trailing slash
    /// (e.g. `https://api.github.com`, `https://gitlab.example.com/api/v4`).
    pub(crate) api_base: String,
    pub(crate) token: String,
}

impl ProviderClient {
    /// Client for one host. `host` may carry a path part (subpath installation,
    /// e.g. `example.com/gitlab` -> `https://example.com/gitlab/api/v4`).
    /// `insecure_tls` disables certificate verification (only for self-hosted
    /// instances with a self-signed certificate; the UI demands an explicit
    /// confirmation).
    pub fn new(kind: ProviderKind, host: &str, token: &str, insecure_tls: bool) -> Result<Self> {
        // Reject http-only instances clearly (normalize_host in the frontend
        // command deliberately keeps the "http://" prefix): otherwise the call
        // would run against the wrong https endpoint and end in a generic
        // network error.
        let host = host.trim();
        if host
            .get(..7)
            .is_some_and(|p| p.eq_ignore_ascii_case("http://"))
        {
            return Err(ProviderError::Network(
                "The provider API is only supported over https — please configure an \
                 https host"
                    .to_string(),
            ));
        }
        // An https:// prefix is redundant (we build the base ourselves) and is
        // tolerated; trailing slashes do not disturb building api_base.
        let host = if host
            .get(..8)
            .is_some_and(|p| p.eq_ignore_ascii_case("https://"))
        {
            &host[8..]
        } else {
            host
        };
        let host = host.trim_matches('/');
        let api_base = match kind {
            ProviderKind::Github if host.eq_ignore_ascii_case("github.com") => {
                "https://api.github.com".to_string()
            }
            // GitHub Enterprise Server
            ProviderKind::Github => format!("https://{host}/api/v3"),
            ProviderKind::Gitlab => format!("https://{host}/api/v4"),
            // Gitea/Forgejo (including Codeberg): always {host}/api/v1, no special case.
            ProviderKind::Gitea => format!("https://{host}/api/v1"),
        };
        Self::with_api_base(kind, &api_base, token, insecure_tls)
    }

    /// Direct API base URL (tests, exotic setups).
    pub fn with_api_base(
        kind: ProviderKind,
        api_base: &str,
        token: &str,
        insecure_tls: bool,
    ) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .danger_accept_invalid_certs(insecure_tls)
            .build()?;
        Ok(Self {
            http,
            kind,
            api_base: api_base.trim_end_matches('/').to_string(),
            token: token.to_string(),
        })
    }

    /// Validates the token and returns the account's user name.
    pub async fn validate(&self) -> Result<String> {
        match self.kind {
            ProviderKind::Github => github::validate(self).await,
            ProviderKind::Gitlab => gitlab::validate(self).await,
            ProviderKind::Gitea => gitea::validate(self).await,
        }
    }

    /// Open change requests (PRs/MRs) of the project, most recently updated
    /// first, including CI status (for the first [`MAX_CI_LOOKUPS`]).
    pub async fn list_change_requests(&self, repo_path: &str) -> Result<Vec<ChangeRequest>> {
        match self.kind {
            ProviderKind::Github => github::list_change_requests(self, repo_path).await,
            ProviderKind::Gitlab => gitlab::list_change_requests(self, repo_path).await,
            ProviderKind::Gitea => gitea::list_change_requests(self, repo_path).await,
        }
    }

    /// Default branch of the project (pre-fills the target branch).
    pub async fn default_branch(&self, repo_path: &str) -> Result<String> {
        match self.kind {
            ProviderKind::Github => github::default_branch(self, repo_path).await,
            ProviderKind::Gitlab => gitlab::default_branch(self, repo_path).await,
            ProviderKind::Gitea => gitea::default_branch(self, repo_path).await,
        }
    }

    /// Creates a change request (PR/MR) and returns it.
    pub async fn create_change_request(
        &self,
        repo_path: &str,
        request: &NewChangeRequest,
    ) -> Result<ChangeRequest> {
        match self.kind {
            ProviderKind::Github => github::create_change_request(self, repo_path, request).await,
            ProviderKind::Gitlab => gitlab::create_change_request(self, repo_path, request).await,
            ProviderKind::Gitea => gitea::create_change_request(self, repo_path, request).await,
        }
    }
}

// api_base is pub(crate) — building it (subpath, the github.com special case,
// rejecting http) is therefore covered here as a module test.
#[cfg(test)]
mod tests {
    use super::ProviderClient;
    use tg_domain::ProviderKind;

    fn base(kind: ProviderKind, host: &str) -> String {
        ProviderClient::new(kind, host, "tok", false)
            .expect("client")
            .api_base
    }

    #[test]
    fn api_base_github_com_uses_api_subdomain() {
        assert_eq!(
            base(ProviderKind::Github, "github.com"),
            "https://api.github.com"
        );
        assert_eq!(
            base(ProviderKind::Github, "GitHub.com"),
            "https://api.github.com"
        );
    }

    #[test]
    fn api_base_selfhosted_per_provider() {
        assert_eq!(
            base(ProviderKind::Github, "ghe.example.com"),
            "https://ghe.example.com/api/v3"
        );
        assert_eq!(
            base(ProviderKind::Gitlab, "gitlab.example.com"),
            "https://gitlab.example.com/api/v4"
        );
        assert_eq!(
            base(ProviderKind::Gitea, "codeberg.org"),
            "https://codeberg.org/api/v1"
        );
    }

    #[test]
    fn api_base_takes_over_subpath_installations() {
        // Subpath installation: the host's path part moves BEFORE the API
        // path — otherwise the instance would not be reachable at all.
        assert_eq!(
            base(ProviderKind::Gitlab, "example.com/gitlab"),
            "https://example.com/gitlab/api/v4"
        );
        assert_eq!(
            base(ProviderKind::Github, "ghe.example.com/github"),
            "https://ghe.example.com/github/api/v3"
        );
        // Trailing slashes / a redundant https:// do not disturb anything.
        assert_eq!(
            base(ProviderKind::Gitlab, "https://example.com/gitlab/"),
            "https://example.com/gitlab/api/v4"
        );
    }

    #[test]
    fn http_host_is_rejected_with_a_clear_message() {
        // http-only instance: a clear message instead of a later generic network
        // error against the wrong https endpoint. (No expect_err: ProviderClient
        // carries the token and deliberately implements no Debug.)
        let err =
            match ProviderClient::new(ProviderKind::Gitlab, "http://gitlab.local", "tok", false) {
                Ok(_) => panic!("http must be rejected"),
                Err(e) => e,
            };
        let msg = err.to_string();
        assert!(msg.contains("https"), "message must mention https: {msg}");
        assert!(
            msg.contains("only supported over https"),
            "message must explain the cause: {msg}"
        );
    }
}
