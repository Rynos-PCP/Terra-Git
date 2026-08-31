//! Gitea/Forgejo REST client (`https://<host>/api/v1`).
//!
//! Gitea and Forgejo share a GitHub-like v1 REST API: pull requests under
//! `/repos/{owner}/{repo}/pulls`, `owner/repo` as two path segments, login field
//! `login`. Three differences from GitHub: the auth header
//! `Authorization: token <T>`, no native draft flag in the create body (draft =
//! title prefix `WIP:`) and CI status exclusively through the commit combined
//! status API (no check runs).

use futures_util::future::join_all;
use serde::{Deserialize, Serialize};

use crate::error::{classify_status, ProviderError, Result};
use crate::ProviderClient;
use tg_domain::{ChangeRequest, CiStatus, NewChangeRequest};

/// GET request with the headers Gitea requires (auth scheme `token`).
fn get(c: &ProviderClient, url: &str) -> reqwest::RequestBuilder {
    c.http
        .get(url)
        .header("Authorization", format!("token {}", c.token))
        .header("User-Agent", "terra-git")
        .header("Accept", "application/json")
}

async fn get_json<T: serde::de::DeserializeOwned>(c: &ProviderClient, url: &str) -> Result<T> {
    let resp = get(c, url).send().await?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(classify_status(status.as_u16(), &body));
    }
    resp.json::<T>()
        .await
        .map_err(|e| ProviderError::InvalidResponse(e.to_string()))
}

/// POST request with the headers Gitea requires.
fn post(c: &ProviderClient, url: &str) -> reqwest::RequestBuilder {
    c.http
        .post(url)
        .header("Authorization", format!("token {}", c.token))
        .header("User-Agent", "terra-git")
        .header("Accept", "application/json")
}

async fn post_json<B: Serialize, T: serde::de::DeserializeOwned>(
    c: &ProviderClient,
    url: &str,
    body: &B,
) -> Result<T> {
    let resp = post(c, url).json(body).send().await?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(classify_status(status.as_u16(), &body));
    }
    resp.json::<T>()
        .await
        .map_err(|e| ProviderError::InvalidResponse(e.to_string()))
}

/// Splits a project path into `owner`/`repo` (the first two segments).
fn owner_repo(repo_path: &str) -> Result<(&str, &str)> {
    let mut segments = repo_path.split('/').filter(|s| !s.is_empty());
    match (segments.next(), segments.next()) {
        (Some(owner), Some(repo)) => Ok((owner, repo)),
        _ => Err(ProviderError::InvalidResponse(format!(
            "Project path '{repo_path}' does not have the form owner/repo"
        ))),
    }
}

/// By default Gitea/Forgejo recognize a PR as work-in-progress/draft ONLY
/// through the title prefixes `WIP:` and `[WIP]` (config
/// `WORK_IN_PROGRESS_PREFIXES`, case-insensitive). `Draft:` deliberately does
/// NOT belong there — otherwise a "Draft: …" title would wrongly count as
/// already-WIP and the automatic `WIP:` prefixing would be skipped.
fn has_gitea_wip_prefix(title: &str) -> bool {
    let t = title.trim_start().to_ascii_lowercase();
    t.starts_with("wip:") || t.starts_with("[wip]")
}

// ---- Response structures (only the required fields) ------------------------

#[derive(Deserialize)]
struct UserResponse {
    login: String,
}

#[derive(Deserialize)]
struct PullResponse {
    number: u64,
    title: Option<String>,
    user: Option<PullUser>,
    head: Option<PullRef>,
    base: Option<PullRef>,
    /// Newer Gitea/Forgejo versions return a native draft flag.
    #[serde(default)]
    draft: Option<bool>,
    html_url: Option<String>,
    updated_at: Option<String>,
}

#[derive(Deserialize)]
struct PullUser {
    login: Option<String>,
}

#[derive(Deserialize)]
struct PullRef {
    #[serde(rename = "ref")]
    branch: Option<String>,
    sha: Option<String>,
}

#[derive(Deserialize)]
struct CommitStatusResponse {
    /// Combined status: success | pending | failure | error | warning | "".
    state: Option<String>,
}

#[derive(Deserialize)]
struct RepoResponse {
    default_branch: Option<String>,
}

/// Maps a PR response onto the neutral domain model. `is_draft` honors both the
/// native draft flag and the WIP title convention.
fn map_pull(p: PullResponse, ci: CiStatus) -> ChangeRequest {
    let title = p.title.unwrap_or_default();
    let is_draft = p.draft.unwrap_or(false) || has_gitea_wip_prefix(&title);
    ChangeRequest {
        number: p.number,
        title,
        author: p.user.and_then(|u| u.login).unwrap_or_default(),
        source_branch: p.head.and_then(|h| h.branch).unwrap_or_default(),
        target_branch: p.base.and_then(|b| b.branch).unwrap_or_default(),
        is_draft,
        web_url: p.html_url.unwrap_or_default(),
        updated_at: p
            .updated_at
            .as_deref()
            .and_then(crate::time::parse_iso8601_utc)
            .unwrap_or(0),
        ci_status: ci,
    }
}

// ---- Request structures ----------------------------------------------------

/// Body for `POST /repos/{owner}/{repo}/pulls`. Gitea has NO draft field in the
/// create body — the draft state sits in the (possibly prefixed) title.
#[derive(Serialize)]
struct CreatePullBody<'a> {
    title: String,
    head: &'a str,
    base: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<&'a str>,
}

/// Title for PR creation: with `draft` a `WIP: ` prefix — but only if a draft
/// prefix is not already present (no doubling).
fn gitea_pr_title(title: &str, draft: bool) -> String {
    if draft && !has_gitea_wip_prefix(title) {
        format!("WIP: {title}")
    } else {
        title.to_string()
    }
}

fn create_pull_body(req: &NewChangeRequest) -> CreatePullBody<'_> {
    CreatePullBody {
        title: gitea_pr_title(&req.title, req.draft),
        head: &req.source_branch,
        base: &req.target_branch,
        body: (!req.description.is_empty()).then_some(req.description.as_str()),
    }
}

// ---- Endpoints -------------------------------------------------------------

/// Validates the token via `GET /user` and returns the login name.
pub(crate) async fn validate(c: &ProviderClient) -> Result<String> {
    let user: UserResponse = get_json(c, &format!("{}/user", c.api_base)).await?;
    Ok(user.login)
}

/// Open PRs, most recently updated first, including CI status for the first
/// [`crate::MAX_CI_LOOKUPS`] entries.
pub(crate) async fn list_change_requests(
    c: &ProviderClient,
    repo_path: &str,
) -> Result<Vec<ChangeRequest>> {
    let (owner, repo) = owner_repo(repo_path)?;

    // Gitea: `limit`/`sort=recentupdate` (not per_page/updated like GitHub).
    let url = format!(
        "{}/repos/{owner}/{repo}/pulls?state=open&sort=recentupdate&limit=50",
        c.api_base
    );
    let pulls: Vec<PullResponse> = get_json(c, &url).await?;

    let lookups = pulls
        .iter()
        .take(crate::MAX_CI_LOOKUPS)
        .map(|p| {
            let sha = p
                .head
                .as_ref()
                .and_then(|h| h.sha.as_deref())
                .unwrap_or_default();
            ci_status(c, owner, repo, sha)
        })
        .collect::<Vec<_>>();
    let mut statuses = join_all(lookups).await;
    statuses.resize(pulls.len(), CiStatus::Unknown);

    Ok(pulls
        .into_iter()
        .zip(statuses)
        .map(|(p, ci)| map_pull(p, ci))
        .collect())
}

/// CI status of a commit; errors are swallowed (Unknown + warn) so ONE broken
/// lookup does not fail the whole PR list.
async fn ci_status(c: &ProviderClient, owner: &str, repo: &str, sha: &str) -> CiStatus {
    if sha.is_empty() {
        tracing::warn!("CI status skipped: PR without head.sha");
        return CiStatus::Unknown;
    }
    match try_ci_status(c, owner, repo, sha).await {
        Ok(status) => status,
        Err(e) => {
            tracing::warn!("CI status for {sha} could not be determined: {e}");
            CiStatus::Unknown
        }
    }
}

/// Gitea combined status: `GET /repos/{owner}/{repo}/commits/{sha}/status`.
async fn try_ci_status(c: &ProviderClient, owner: &str, repo: &str, sha: &str) -> Result<CiStatus> {
    let url = format!("{}/repos/{owner}/{repo}/commits/{sha}/status", c.api_base);
    let combined: CommitStatusResponse = get_json(c, &url).await?;
    Ok(map_commit_status(combined.state.as_deref()))
}

/// Maps the Gitea combined state onto the neutral [`CiStatus`].
/// An empty/absent state (no CI yet) -> Unknown.
fn map_commit_status(state: Option<&str>) -> CiStatus {
    // Gitea CommitStatusState: pending | success | error | failure | warning.
    match state {
        Some("success") => CiStatus::Success,
        Some("pending") => CiStatus::Pending,
        Some("failure") | Some("error") => CiStatus::Failed,
        // "warning" (non-blocking) and an empty/absent state -> neutral.
        _ => CiStatus::Unknown,
    }
}

/// Default branch of the repo via `GET /repos/{owner}/{repo}`.
pub(crate) async fn default_branch(c: &ProviderClient, repo_path: &str) -> Result<String> {
    let (owner, repo) = owner_repo(repo_path)?;
    let url = format!("{}/repos/{owner}/{repo}", c.api_base);
    let info: RepoResponse = get_json(c, &url).await?;
    Ok(info.default_branch.unwrap_or_else(|| {
        tracing::warn!("Repo response without default_branch — falling back to 'main'");
        "main".to_string()
    }))
}

/// Creates a pull request via `POST /repos/{owner}/{repo}/pulls`.
pub(crate) async fn create_change_request(
    c: &ProviderClient,
    repo_path: &str,
    req: &NewChangeRequest,
) -> Result<ChangeRequest> {
    let (owner, repo) = owner_repo(repo_path)?;
    let url = format!("{}/repos/{owner}/{repo}/pulls", c.api_base);
    let created: PullResponse = post_json(c, &url, &create_pull_body(req)).await?;
    tracing::info!("PR #{} created in {owner}/{repo}", created.number);
    Ok(map_pull(created, CiStatus::Unknown))
}

// As in github.rs: POST tests live as module tests because the endpoints are
// pub(crate). The stub drains the body via Content-Length (Windows RST).
#[cfg(test)]
mod tests {
    use super::{
        create_change_request, create_pull_body, default_branch, gitea_pr_title,
        has_gitea_wip_prefix, map_commit_status,
    };
    use crate::ProviderClient;
    use tg_domain::{CiStatus, NewChangeRequest, ProviderKind};

    fn client(base_url: &str) -> ProviderClient {
        ProviderClient::with_api_base(ProviderKind::Gitea, base_url, "test-token", false)
            .expect("client")
    }

    fn new_cr(description: &str, draft: bool) -> NewChangeRequest {
        NewChangeRequest {
            title: "Feature: Verdigris".to_string(),
            description: description.to_string(),
            source_branch: "feature/verdigris".to_string(),
            target_branch: "develop".to_string(),
            draft,
        }
    }

    // --- Pure logic (without HTTP) ----------------------------------------------

    #[test]
    fn wip_prefix_detection_only_gitea_defaults() {
        assert!(has_gitea_wip_prefix("WIP: x"));
        assert!(has_gitea_wip_prefix("wip: x"));
        assert!(has_gitea_wip_prefix("[WIP] x"));
        // "Draft:" is NOT a Gitea default prefix.
        assert!(!has_gitea_wip_prefix("Draft: x"));
        assert!(!has_gitea_wip_prefix("Fix: no WIP"));
    }

    #[test]
    fn draft_title_without_wip_prefix_gets_wip_prepended() {
        // Regression: "Draft:" is no WIP prefix -> the draft intent is preserved.
        assert_eq!(gitea_pr_title("Draft: X", true), "WIP: Draft: X");
    }

    #[test]
    fn pr_title_prefixes_wip_only_for_draft_and_without_doubling() {
        assert_eq!(gitea_pr_title("Feature X", true), "WIP: Feature X");
        assert_eq!(gitea_pr_title("Feature X", false), "Feature X");
        // already a draft prefix -> do not double it
        assert_eq!(gitea_pr_title("WIP: Feature X", true), "WIP: Feature X");
    }

    #[test]
    fn commit_status_mapping() {
        assert_eq!(map_commit_status(Some("success")), CiStatus::Success);
        assert_eq!(map_commit_status(Some("pending")), CiStatus::Pending);
        assert_eq!(map_commit_status(Some("failure")), CiStatus::Failed);
        assert_eq!(map_commit_status(Some("error")), CiStatus::Failed);
        assert_eq!(map_commit_status(Some("warning")), CiStatus::Unknown);
        assert_eq!(map_commit_status(Some("")), CiStatus::Unknown);
        assert_eq!(map_commit_status(None), CiStatus::Unknown);
    }

    #[test]
    fn create_body_without_draft_field_uses_wip_title() {
        let v = serde_json::to_value(create_pull_body(&new_cr("Description", true)))
            .expect("serializable");
        assert_eq!(v["title"], "WIP: Feature: Verdigris");
        assert_eq!(v["head"], "feature/verdigris");
        assert_eq!(v["base"], "develop");
        assert_eq!(v["body"], "Description");
        // NO draft field (Gitea does not know it in the create body): title/head/base/body.
        assert!(
            v.get("draft").is_none(),
            "the draft field must not exist: {v}"
        );
        assert_eq!(v.as_object().map(|o| o.len()), Some(4));
    }

    #[test]
    fn empty_description_produces_no_body_field() {
        let v = serde_json::to_value(create_pull_body(&new_cr("", false))).expect("serializable");
        assert!(v.get("body").is_none(), "body field may be absent: {v}");
        assert_eq!(v.as_object().map(|o| o.len()), Some(3)); // title/head/base
    }

    // --- Minimal HTTP stub (body-draining, like github.rs) ---------------------

    #[derive(Clone)]
    struct RecordedRequest {
        method: String,
        path: String,
        headers: Vec<(String, String)>,
    }

    impl RecordedRequest {
        fn header(&self, name: &str) -> Option<&str> {
            let name = name.to_lowercase();
            self.headers
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| v.as_str())
        }
    }

    struct Stub {
        base_url: String,
        requests: std::sync::Arc<std::sync::Mutex<Vec<RecordedRequest>>>,
    }

    impl Stub {
        fn requests(&self) -> Vec<RecordedRequest> {
            self.requests.lock().unwrap().clone()
        }
    }

    fn start_stub(status: u16, resp_body: &'static str) -> Stub {
        use std::io::{Read, Write};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let requests: std::sync::Arc<std::sync::Mutex<Vec<RecordedRequest>>> =
            std::sync::Arc::default();
        let recorded = requests.clone();

        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut buf = Vec::new();
                let mut byte = [0u8; 1];
                while !buf.ends_with(b"\r\n\r\n") {
                    match stream.read(&mut byte) {
                        Ok(1) => buf.push(byte[0]),
                        _ => break,
                    }
                }
                let head = String::from_utf8_lossy(&buf);
                let mut lines = head.lines();
                let request_line = lines.next().unwrap_or_default();
                let mut parts = request_line.split_whitespace();
                let req = RecordedRequest {
                    method: parts.next().unwrap_or_default().to_string(),
                    path: parts.next().unwrap_or_default().to_string(),
                    headers: lines
                        .filter_map(|l| l.split_once(':'))
                        .map(|(n, v)| (n.trim().to_lowercase(), v.trim().to_string()))
                        .collect(),
                };
                let len: usize = req
                    .header("content-length")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                let mut sink = vec![0u8; len];
                let _ = stream.read_exact(&mut sink);
                recorded.lock().unwrap().push(req);

                let response = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{resp_body}",
                    resp_body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });

        Stub { base_url, requests }
    }

    const PULL_CREATED: &str = r#"{
      "number": 55,
      "title": "WIP: Feature: Verdigris",
      "user": { "login": "carol" },
      "head": { "ref": "feature/verdigris", "sha": "ddd444" },
      "base": { "ref": "develop" },
      "html_url": "https://gitea.example/octo/hello/pulls/55",
      "updated_at": "2026-07-02T09:15:00Z"
    }"#;

    #[tokio::test]
    async fn create_posts_to_pulls_and_maps_the_response() {
        let server = start_stub(201, PULL_CREATED);
        let c = client(&server.base_url);

        let cr = create_change_request(&c, "octo/hello", &new_cr("Text", true))
            .await
            .expect("create");

        assert_eq!(cr.number, 55);
        assert_eq!(cr.author, "carol");
        assert_eq!(cr.source_branch, "feature/verdigris");
        assert_eq!(cr.target_branch, "develop");
        assert!(cr.is_draft, "WIP title -> is_draft");
        assert_eq!(cr.web_url, "https://gitea.example/octo/hello/pulls/55");
        assert_eq!(cr.ci_status, CiStatus::Unknown);

        let reqs = server.requests();
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].method, "POST");
        assert_eq!(reqs[0].path, "/repos/octo/hello/pulls");
        assert_eq!(reqs[0].header("authorization"), Some("token test-token"));
        assert_eq!(reqs[0].header("content-type"), Some("application/json"));
    }

    #[tokio::test]
    async fn default_branch_comes_from_the_repo_endpoint() {
        let server = start_stub(200, r#"{"name":"hello","default_branch":"develop"}"#);
        let c = client(&server.base_url);

        let branch = default_branch(&c, "octo/hello").await.expect("default");
        assert_eq!(branch, "develop");

        let reqs = server.requests();
        assert_eq!(reqs[0].path, "/repos/octo/hello");
        assert_eq!(reqs[0].header("authorization"), Some("token test-token"));
    }
}
