//! GitHub REST client (github.com + GitHub Enterprise Server).
//!
//! Speaks the v3 REST API (`https://api.github.com` or
//! `https://<host>/api/v3`) and maps pull requests onto the neutral domain
//! model. CI status comes primarily from the check-runs API (GitHub Actions &
//! apps), falling back to the legacy commit-status API.

use futures_util::future::join_all;
use serde::{Deserialize, Serialize};

use crate::error::{classify_status, ProviderError, Result};
use crate::ProviderClient;
use tg_domain::{ChangeRequest, CiStatus, NewChangeRequest};

/// GET request with the headers GitHub requires.
fn get(c: &ProviderClient, url: &str) -> reqwest::RequestBuilder {
    c.http
        .get(url)
        .header("Authorization", format!("Bearer {}", c.token))
        .header("User-Agent", "terra-git")
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
}

/// Sends the request, maps non-2xx via [`classify_status`] and deserializes the
/// body as `T`.
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

/// POST request with the headers GitHub requires.
fn post(c: &ProviderClient, url: &str) -> reqwest::RequestBuilder {
    c.http
        .post(url)
        .header("Authorization", format!("Bearer {}", c.token))
        .header("User-Agent", "terra-git")
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
}

/// Sends `body` as JSON via POST, maps non-2xx via [`classify_status`] and
/// deserializes the response as `T`.
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

/// Splits a project path into `owner`/`repo` (the first two segments — GHES
/// paths may carry further segments, same as in
/// [`list_change_requests`]).
fn owner_repo(repo_path: &str) -> Result<(&str, &str)> {
    let mut segments = repo_path.split('/').filter(|s| !s.is_empty());
    match (segments.next(), segments.next()) {
        (Some(owner), Some(repo)) => Ok((owner, repo)),
        _ => Err(ProviderError::InvalidResponse(format!(
            "Project path '{repo_path}' is not in owner/repo form"
        ))),
    }
}

// ---- Response structures (only the fields we need) -------------------------

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
struct CheckRunsResponse {
    #[serde(default)]
    total_count: u64,
    #[serde(default)]
    check_runs: Vec<CheckRun>,
}

#[derive(Deserialize)]
struct CheckRun {
    status: Option<String>,
    conclusion: Option<String>,
}

#[derive(Deserialize)]
struct CommitStatusResponse {
    state: Option<String>,
    #[serde(default)]
    total_count: u64,
}

#[derive(Deserialize)]
struct RepoResponse {
    default_branch: Option<String>,
}

/// Maps a PR response onto the neutral domain model.
fn map_pull(p: PullResponse, ci: CiStatus) -> ChangeRequest {
    ChangeRequest {
        number: p.number,
        title: p.title.unwrap_or_default(),
        author: p.user.and_then(|u| u.login).unwrap_or_default(),
        source_branch: p.head.and_then(|h| h.branch).unwrap_or_default(),
        target_branch: p.base.and_then(|b| b.branch).unwrap_or_default(),
        is_draft: p.draft.unwrap_or(false),
        web_url: p.html_url.unwrap_or_default(),
        updated_at: p
            .updated_at
            .as_deref()
            .and_then(crate::time::parse_iso8601_utc)
            .unwrap_or(0),
        ci_status: ci,
    }
}

// ---- Request structures ------------------------------------------------------

/// Body for `POST /repos/{owner}/{repo}/pulls`. Kept separate as a pure mapping
/// so the serialization is unit-testable without an HTTP stub (the test server
/// does not record request bodies).
#[derive(Serialize)]
struct CreatePullBody<'a> {
    title: &'a str,
    head: &'a str,
    base: &'a str,
    /// PR description; for an empty description the field is omitted entirely
    /// instead of sending an empty string.
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<&'a str>,
    draft: bool,
}

/// Builds the request body from the neutral [`NewChangeRequest`].
fn create_pull_body(req: &NewChangeRequest) -> CreatePullBody<'_> {
    CreatePullBody {
        title: &req.title,
        head: &req.source_branch,
        base: &req.target_branch,
        body: (!req.description.is_empty()).then_some(req.description.as_str()),
        draft: req.draft,
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
    // Use only owner/repo — GHES paths may carry further segments.
    let mut segments = repo_path.split('/').filter(|s| !s.is_empty());
    let (Some(owner), Some(repo)) = (segments.next(), segments.next()) else {
        return Err(ProviderError::InvalidResponse(format!(
            "Project path '{repo_path}' is not in owner/repo form"
        )));
    };

    let url = format!(
        "{}/repos/{owner}/{repo}/pulls?state=open&per_page=50&sort=updated&direction=desc",
        c.api_base
    );
    let pulls: Vec<PullResponse> = get_json(c, &url).await?;

    // CI status in parallel, but only for the leading entries (the order is
    // preserved, join_all returns in input order).
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
        .map(|(p, ci)| ChangeRequest {
            number: p.number,
            title: p.title.unwrap_or_default(),
            author: p.user.and_then(|u| u.login).unwrap_or_default(),
            source_branch: p.head.and_then(|h| h.branch).unwrap_or_default(),
            target_branch: p.base.and_then(|b| b.branch).unwrap_or_default(),
            is_draft: p.draft.unwrap_or(false),
            web_url: p.html_url.unwrap_or_default(),
            updated_at: p
                .updated_at
                .as_deref()
                .and_then(crate::time::parse_iso8601_utc)
                .unwrap_or(0),
            ci_status: ci,
        })
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

async fn try_ci_status(c: &ProviderClient, owner: &str, repo: &str, sha: &str) -> Result<CiStatus> {
    // 1. Check-runs API (GitHub Actions & check apps). per_page=100 is the API
    //    maximum — the default (30) would treat only the first page as the whole
    //    truth when there are many runs.
    let url = format!(
        "{}/repos/{owner}/{repo}/commits/{sha}/check-runs?per_page=100",
        c.api_base
    );
    let runs: CheckRunsResponse = get_json(c, &url).await?;
    if runs.total_count > 0 {
        return Ok(classify_check_runs(&runs.check_runs, runs.total_count));
    }

    // 2. Fallback: legacy commit-status API (Jenkins & co.).
    let url = format!("{}/repos/{owner}/{repo}/commits/{sha}/status", c.api_base);
    let combined: CommitStatusResponse = get_json(c, &url).await?;
    if combined.total_count == 0 {
        return Ok(CiStatus::Unknown);
    }
    Ok(match combined.state.as_deref() {
        Some("success") => CiStatus::Success,
        Some("failure") | Some("error") => CiStatus::Failed,
        Some("pending") => CiStatus::Pending,
        _ => CiStatus::Unknown,
    })
}

/// Default branch of the repo via `GET /repos/{owner}/{repo}`.
/// If the field is missing from the response, the value falls back to `"main"`.
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
/// Non-2xx (e.g. 422 for an already existing PR or missing diffs) comes back
/// typed through [`classify_status`]; the CI status of a freshly created PR is
/// always [`CiStatus::Unknown`] (no run yet).
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

/// Aggregates check runs into an overall status. `total_count` is the total the
/// API reports: if more runs exist than were loaded (more than one page), only a
/// visible failure or a visibly running run is definitive — everything else
/// conservatively becomes Pending instead of a guessed Success/Canceled.
/// Priority: Failed > Running > Pending > Canceled > Success
/// (success/neutral/skipped count as green; action_required waits for a click —
/// like GitLab's "manual" — and stale is no reliable result: both Pending,
/// never green).
fn classify_check_runs(runs: &[CheckRun], total_count: u64) -> CiStatus {
    let any_conclusion = |v: &str| runs.iter().any(|r| r.conclusion.as_deref() == Some(v));
    let any_status = |v: &str| runs.iter().any(|r| r.status.as_deref() == Some(v));

    if any_conclusion("failure") || any_conclusion("timed_out") {
        CiStatus::Failed
    } else if any_status("queued") || any_status("in_progress") || any_status("pending") {
        CiStatus::Running
    } else if total_count > runs.len() as u64
        || any_conclusion("action_required")
        || any_conclusion("stale")
    {
        CiStatus::Pending
    } else if any_conclusion("cancelled") {
        CiStatus::Canceled
    } else {
        CiStatus::Success
    }
}

// The stub tests below live as module tests because `default_branch`/
// `create_change_request` are pub(crate) and not (yet) bound to the public API
// through `lib.rs` — they are therefore not callable from `tests/`. As soon as
// `ProviderClient` binds them, they can move to `tests/github_tests.rs`.
#[cfg(test)]
mod tests {
    use super::{
        classify_check_runs, create_change_request, create_pull_body, default_branch, CheckRun,
    };
    use crate::ProviderClient;
    use tg_domain::{CiStatus, NewChangeRequest, ProviderKind};

    fn client(base_url: &str) -> ProviderClient {
        ProviderClient::with_api_base(ProviderKind::Github, base_url, "test-token", false)
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

    // --- Minimal HTTP stub ------------------------------------------------------
    //
    // Deliberately NOT pulled in via `#[path]` from `tests/common/mod.rs` —
    // gitlab.rs already loads that file as a module, and a second load would be
    // `clippy::duplicate_mod`. Unlike the shared stub, this variant drains the
    // request body using `Content-Length` before answering: if a stub closes with
    // an unread body, Windows sends a TCP RST that can discard the already
    // written response on the client side (flaky "network" errors). The body is
    // deliberately NOT recorded — body assertions run indirectly through the
    // `create_pull_body` unit tests.

    #[derive(Clone)]
    struct RecordedRequest {
        method: String,
        /// Path including the query string.
        path: String,
        /// Header names lower-cased.
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

    /// Starts a stub on a free port that answers EVERY request with `status` +
    /// `resp_body` and records method/path/headers.
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
                // Read the request head (up to CRLFCRLF), as in the shared stub.
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
                // Drain the body so the connection closes without an RST.
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

    // --- classify_check_runs: aggregation including pagination (pure) ----------

    fn run(status: &str, conclusion: Option<&str>) -> CheckRun {
        CheckRun {
            status: Some(status.to_string()),
            conclusion: conclusion.map(str::to_string),
        }
    }

    #[test]
    fn action_required_is_not_green_but_pending() {
        // action_required waits for a user click — like GitLab "manual"
        // (map_pipeline_status -> Pending). As Success, a blocked PR would
        // wrongly show up green.
        let runs = [
            run("completed", Some("success")),
            run("completed", Some("action_required")),
        ];
        assert_eq!(classify_check_runs(&runs, 2), CiStatus::Pending);
    }

    #[test]
    fn stale_is_not_green_but_pending() {
        // stale = the result no longer holds (e.g. the check suite was
        // invalidated) — no reliable success, pending from the user's point of view.
        let runs = [run("completed", Some("stale"))];
        assert_eq!(classify_check_runs(&runs, 1), CiStatus::Pending);
    }

    #[test]
    fn failure_beats_action_required() {
        let runs = [
            run("completed", Some("action_required")),
            run("completed", Some("failure")),
        ];
        assert_eq!(classify_check_runs(&runs, 2), CiStatus::Failed);
    }

    #[test]
    fn more_runs_than_loaded_yields_pending_instead_of_success() {
        // total_count > the loaded page: the unseen runs could be red — a success
        // verdict would be a guess.
        let runs = [run("completed", Some("success"))];
        assert_eq!(classify_check_runs(&runs, 101), CiStatus::Pending);
    }

    #[test]
    fn visible_failure_is_definitive_even_with_pagination() {
        // A failure on the first page cannot turn green through unseen runs.
        let runs = [run("completed", Some("failure"))];
        assert_eq!(classify_check_runs(&runs, 101), CiStatus::Failed);
    }

    #[test]
    fn visibly_running_run_stays_running_even_with_pagination() {
        // A visibly running run is an honest "in progress" regardless of unseen
        // pages.
        let runs = [run("in_progress", None)];
        assert_eq!(classify_check_runs(&runs, 101), CiStatus::Running);
    }

    #[test]
    fn fully_green_page_stays_success() {
        let runs = [
            run("completed", Some("success")),
            run("completed", Some("neutral")),
            run("completed", Some("skipped")),
        ];
        assert_eq!(classify_check_runs(&runs, 3), CiStatus::Success);
    }

    // --- Body serialization of PR creation (pure, without HTTP) ----------------

    #[test]
    fn create_body_carries_all_fields_including_draft() {
        let req = new_cr("Description of the PR", true);
        let v = serde_json::to_value(create_pull_body(&req)).expect("serializable");

        assert_eq!(v["title"], "Feature: Verdigris");
        assert_eq!(v["head"], "feature/verdigris");
        assert_eq!(v["base"], "develop");
        assert_eq!(v["body"], "Description of the PR");
        assert_eq!(v["draft"], true);
        assert_eq!(v.as_object().map(|o| o.len()), Some(5));
    }

    #[test]
    fn empty_description_produces_no_body_field() {
        let req = new_cr("", false);
        let v = serde_json::to_value(create_pull_body(&req)).expect("serializable");

        assert!(v.get("body").is_none(), "body field may be absent: {v}");
        assert_eq!(v["draft"], false);
        // Only title/head/base/draft — no body, no extras.
        assert_eq!(v.as_object().map(|o| o.len()), Some(4));
    }

    // --- Stub tests of the new endpoints (method, path, response mapping) ------

    /// Response of `POST /pulls` (201): a single PR object.
    const PULL_CREATED: &str = r#"{
      "number": 55,
      "title": "Feature: Verdigris",
      "user": { "login": "carol" },
      "head": { "ref": "feature/verdigris", "sha": "ddd444" },
      "base": { "ref": "develop" },
      "draft": true,
      "html_url": "https://github.example/octo/hello/pull/55",
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
        assert_eq!(cr.title, "Feature: Verdigris");
        assert_eq!(cr.author, "carol");
        assert_eq!(cr.source_branch, "feature/verdigris");
        assert_eq!(cr.target_branch, "develop");
        assert!(cr.is_draft);
        assert_eq!(cr.web_url, "https://github.example/octo/hello/pull/55");
        assert_eq!(cr.updated_at, 1_782_983_700); // 2026-07-02T09:15:00Z
        assert_eq!(
            cr.ci_status,
            CiStatus::Unknown,
            "freshly created -> Unknown"
        );

        let reqs = server.requests();
        assert_eq!(reqs.len(), 1, "exactly one request, no CI lookup");
        assert_eq!(reqs[0].method, "POST");
        assert_eq!(reqs[0].path, "/repos/octo/hello/pulls");
        assert_eq!(reqs[0].header("authorization"), Some("Bearer test-token"));
        assert_eq!(
            reqs[0].header("accept"),
            Some("application/vnd.github+json")
        );
        assert_eq!(reqs[0].header("content-type"), Some("application/json"));
    }

    #[tokio::test]
    async fn create_422_pr_already_exists_yields_api_error() {
        let server = start_stub(
            422,
            r#"{"message":"A pull request already exists for octo:feature/verdigris."}"#,
        );
        let c = client(&server.base_url);

        let err = create_change_request(&c, "octo/hello", &new_cr("", false))
            .await
            .expect_err("422 must fail");

        assert_eq!(err.code(), "api_error");
        let msg = err.to_string();
        assert!(msg.contains("422"), "status missing in: {msg}");
        assert!(
            msg.contains("A pull request already exists"),
            "body excerpt missing in: {msg}"
        );
    }

    #[tokio::test]
    async fn default_branch_comes_from_the_repo_endpoint() {
        let server = start_stub(200, r#"{"name":"hello","default_branch":"develop"}"#);
        let c = client(&server.base_url);

        let branch = default_branch(&c, "octo/hello").await.expect("default");
        assert_eq!(branch, "develop");

        let reqs = server.requests();
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].method, "GET");
        assert_eq!(reqs[0].path, "/repos/octo/hello");
        assert_eq!(reqs[0].header("authorization"), Some("Bearer test-token"));
    }

    #[tokio::test]
    async fn default_branch_falls_back_to_main_without_the_field() {
        let server = start_stub(200, r#"{"name":"r"}"#);
        let c = client(&server.base_url);

        let branch = default_branch(&c, "o/r").await.expect("default");
        assert_eq!(branch, "main");
    }
}
