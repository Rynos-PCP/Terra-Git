//! GitLab REST client (gitlab.com + self-hosted, API v4).
//!
//! Endpoints: `GET /user` (token check), `GET /projects/{id}` (default branch),
//! `GET /projects/{id}/merge_requests` (open MRs, most recently updated first),
//! `GET /projects/{id}/merge_requests/{iid}/pipelines` (CI status) and
//! `POST /projects/{id}/merge_requests` (create an MR).
//! Authentication goes through the `PRIVATE-TOKEN` header.

use futures_util::future::join_all;
use serde::{Deserialize, Serialize};

use crate::error::{classify_status, ProviderError, Result};
use crate::ProviderClient;
use tg_domain::{ChangeRequest, CiStatus, NewChangeRequest};

// --- Response structures (only the required fields, tolerant) ---------------

#[derive(Deserialize)]
struct User {
    username: String,
}

#[derive(Deserialize)]
struct Author {
    username: Option<String>,
}

#[derive(Deserialize)]
struct MergeRequest {
    iid: u64,
    title: Option<String>,
    author: Option<Author>,
    source_branch: Option<String>,
    target_branch: Option<String>,
    draft: Option<bool>,
    work_in_progress: Option<bool>,
    web_url: Option<String>,
    updated_at: Option<String>,
}

#[derive(Deserialize)]
struct Pipeline {
    status: Option<String>,
}

#[derive(Deserialize)]
struct Project {
    default_branch: Option<String>,
}

// --- Endpoints ---------------------------------------------------------------

/// Validates the token via `GET /user` and returns the user name.
pub(crate) async fn validate(c: &ProviderClient) -> Result<String> {
    let user: User = get_json(c, &format!("{}/user", c.api_base)).await?;
    Ok(user.username)
}

/// Open merge requests of the project including CI status (for the first
/// [`crate::MAX_CI_LOOKUPS`] entries; order = API order, i.e. most recently
/// updated first).
pub(crate) async fn list_change_requests(
    c: &ProviderClient,
    repo_path: &str,
) -> Result<Vec<ChangeRequest>> {
    let project = encode_project_path(repo_path);
    let url = format!(
        "{}/projects/{project}/merge_requests?state=opened&per_page=50&order_by=updated_at&sort=desc",
        c.api_base
    );
    let mrs: Vec<MergeRequest> = get_json(c, &url).await?;

    // CI status only for the leading entries (request budget), concurrently;
    // join_all preserves the order.
    let ci = join_all(
        mrs.iter()
            .take(crate::MAX_CI_LOOKUPS)
            .map(|mr| ci_status(c, &project, mr.iid)),
    )
    .await;

    Ok(mrs
        .into_iter()
        .enumerate()
        .map(|(i, mr)| {
            let title = mr.title.unwrap_or_default();
            let is_draft = match (mr.draft, mr.work_in_progress) {
                // Older GitLab versions return neither field: detect the draft
                // convention through the title prefix.
                (None, None) => title
                    .get(..6)
                    .is_some_and(|p| p.eq_ignore_ascii_case("draft:")),
                (draft, wip) => draft.unwrap_or(false) || wip.unwrap_or(false),
            };
            ChangeRequest {
                number: mr.iid,
                author: mr.author.and_then(|a| a.username).unwrap_or_default(),
                source_branch: mr.source_branch.unwrap_or_default(),
                target_branch: mr.target_branch.unwrap_or_default(),
                is_draft,
                web_url: mr.web_url.unwrap_or_default(),
                updated_at: mr
                    .updated_at
                    .as_deref()
                    .and_then(crate::time::parse_iso8601_utc)
                    .unwrap_or(0),
                ci_status: ci.get(i).copied().unwrap_or(CiStatus::Unknown),
                title,
            }
        })
        .collect())
}

/// Default branch of the project via `GET /projects/{id}`. If the field is
/// missing or `null` (e.g. an empty project), "main" is assumed.
pub(crate) async fn default_branch(c: &ProviderClient, repo_path: &str) -> Result<String> {
    let project = encode_project_path(repo_path);
    let p: Project = get_json(c, &format!("{}/projects/{project}", c.api_base)).await?;
    Ok(p.default_branch.unwrap_or_else(|| "main".to_string()))
}

/// Creates a merge request via `POST /projects/{id}/merge_requests`.
/// Draft works through the title prefix "Draft: " (a GitLab convention; the API
/// has no dedicated flag). The CI status of a freshly created MR cannot be
/// determined yet -> [`CiStatus::Unknown`]. An already existing MR arrives as a
/// 409 and therefore as a typed Api error.
pub(crate) async fn create_change_request(
    c: &ProviderClient,
    repo_path: &str,
    req: &NewChangeRequest,
) -> Result<ChangeRequest> {
    let project = encode_project_path(repo_path);
    let url = format!("{}/projects/{project}/merge_requests", c.api_base);
    let mr: MergeRequest = post_json(c, &url, &create_mr_body(req)).await?;
    tracing::info!("GitLab MR !{} created in {repo_path}", mr.iid);

    // Response mapping as in the MR list (inline in the closure there).
    let title = mr.title.unwrap_or_default();
    let is_draft = match (mr.draft, mr.work_in_progress) {
        (None, None) => has_draft_prefix(&title),
        (draft, wip) => draft.unwrap_or(false) || wip.unwrap_or(false),
    };
    Ok(ChangeRequest {
        number: mr.iid,
        author: mr.author.and_then(|a| a.username).unwrap_or_default(),
        source_branch: mr.source_branch.unwrap_or_default(),
        target_branch: mr.target_branch.unwrap_or_default(),
        is_draft,
        web_url: mr.web_url.unwrap_or_default(),
        updated_at: mr
            .updated_at
            .as_deref()
            .and_then(crate::time::parse_iso8601_utc)
            .unwrap_or(0),
        ci_status: CiStatus::Unknown,
        title,
    })
}

/// Request body for MR creation (only the fields GitLab expects).
#[derive(Serialize)]
struct CreateMergeRequestBody<'a> {
    source_branch: &'a str,
    target_branch: &'a str,
    title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<&'a str>,
}

/// Builds the JSON body for `POST …/merge_requests` — as a pure function so the
/// draft/description logic is testable without HTTP. An empty description is
/// omitted; with `draft` the title prefix "Draft: " is added unless it is
/// already there (case-insensitively).
fn create_mr_body(req: &NewChangeRequest) -> serde_json::Value {
    let title = if req.draft && !has_draft_prefix(&req.title) {
        format!("Draft: {}", req.title)
    } else {
        req.title.clone()
    };
    serde_json::to_value(CreateMergeRequestBody {
        source_branch: &req.source_branch,
        target_branch: &req.target_branch,
        title,
        description: (!req.description.is_empty()).then_some(req.description.as_str()),
    })
    .expect("the body struct is statically serializable")
}

/// Does the title start (case-insensitively) with "draft:"?
fn has_draft_prefix(title: &str) -> bool {
    title
        .get(..6)
        .is_some_and(|p| p.eq_ignore_ascii_case("draft:"))
}

/// CI status of an MR through its newest pipeline. Errors are swallowed
/// (-> Unknown) so a single broken lookup does not fail the whole list.
async fn ci_status(c: &ProviderClient, project: &str, iid: u64) -> CiStatus {
    let url = format!(
        "{}/projects/{project}/merge_requests/{iid}/pipelines?per_page=1",
        c.api_base
    );
    match get_json::<Vec<Pipeline>>(c, &url).await {
        Ok(pipelines) => pipelines.first().map_or(CiStatus::Unknown, |p| {
            map_pipeline_status(p.status.as_deref().unwrap_or(""))
        }),
        Err(e) => {
            tracing::warn!("GitLab CI status for MR !{iid} could not be determined: {e}");
            CiStatus::Unknown
        }
    }
}

/// Maps the GitLab pipeline status onto the neutral model.
fn map_pipeline_status(status: &str) -> CiStatus {
    match status {
        "success" => CiStatus::Success,
        "failed" => CiStatus::Failed,
        "running" => CiStatus::Running,
        // "manual" waits for a click — pending from the user's point of view.
        "created" | "pending" | "preparing" | "waiting_for_resource" | "scheduled" | "manual" => {
            CiStatus::Pending
        }
        "canceled" | "canceling" => CiStatus::Canceled,
        // including "skipped" and future/unknown states
        _ => CiStatus::Unknown,
    }
}

// --- Helpers ------------------------------------------------------------------

/// GET with the `PRIVATE-TOKEN` header; non-2xx is typed through
/// [`classify_status`], decode errors become `InvalidResponse`.
async fn get_json<T: serde::de::DeserializeOwned>(c: &ProviderClient, url: &str) -> Result<T> {
    let resp = c
        .http
        .get(url)
        .header("PRIVATE-TOKEN", &c.token)
        .send()
        .await?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(classify_status(status.as_u16(), &body));
    }
    resp.json::<T>()
        .await
        .map_err(|e| ProviderError::InvalidResponse(e.to_string()))
}

/// POST with a JSON body and the `PRIVATE-TOKEN` header; error handling as in
/// [`get_json`].
async fn post_json<T: serde::de::DeserializeOwned>(
    c: &ProviderClient,
    url: &str,
    body: &serde_json::Value,
) -> Result<T> {
    let resp = c
        .http
        .post(url)
        .header("PRIVATE-TOKEN", &c.token)
        .json(body)
        .send()
        .await?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(classify_status(status.as_u16(), &body));
    }
    resp.json::<T>()
        .await
        .map_err(|e| ProviderError::InvalidResponse(e.to_string()))
}

const HEX: &[u8; 16] = b"0123456789ABCDEF";

/// Percent-encodes the project path as ONE URL segment (GitLab convention:
/// `group/sub/repo` -> `group%2Fsub%2Frepo`). Unreserved characters per
/// RFC 3986 (`[A-Za-z0-9._~-]`) stay, all other bytes become `%XX` with
/// upper-case hex.
fn encode_project_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for b in path.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'~' | b'-' => {
                out.push(char::from(b));
            }
            _ => {
                out.push('%');
                out.push(char::from(HEX[usize::from(b >> 4)]));
                out.push(char::from(HEX[usize::from(b & 0x0F)]));
            }
        }
    }
    out
}

// The HTTP stub of the integration tests is reused here via `#[path]`:
// `default_branch`/`create_change_request` are pub(crate) and not (yet) bound to
// the public API through `lib.rs` — they are therefore not callable from
// `tests/`. As soon as `ProviderClient` binds them, the stub tests below can
// move to `tests/gitlab_tests.rs`.
#[cfg(test)]
#[path = "../tests/common/mod.rs"]
mod test_server;

#[cfg(test)]
mod tests {
    use super::{
        create_change_request, create_mr_body, default_branch, encode_project_path,
        map_pipeline_status, test_server,
    };
    use crate::ProviderClient;
    use tg_domain::{CiStatus, NewChangeRequest, ProviderKind};

    fn client(base_url: &str) -> ProviderClient {
        ProviderClient::with_api_base(ProviderKind::Gitlab, base_url, "test-token", false)
            .expect("client")
    }

    fn new_mr(title: &str, description: &str, draft: bool) -> NewChangeRequest {
        NewChangeRequest {
            title: title.to_string(),
            description: description.to_string(),
            source_branch: "feature/x".to_string(),
            target_branch: "main".to_string(),
            draft,
        }
    }

    // --- Stub for POST tests ----------------------------------------------------
    //
    // Like `test_server::start`, but drains the request body using
    // `Content-Length` before answering: if a stub closes with an unread body,
    // Windows sends a TCP RST that can discard the already written response on
    // the client side (flaky "network" errors). The body is deliberately NOT
    // recorded — body assertions run indirectly through the `create_mr_body`
    // unit tests.

    struct PostStub {
        base_url: String,
        requests: std::sync::Arc<std::sync::Mutex<Vec<test_server::RecordedRequest>>>,
    }

    impl PostStub {
        fn requests(&self) -> Vec<test_server::RecordedRequest> {
            self.requests.lock().unwrap().clone()
        }
    }

    fn start_post_stub(status: u16, resp_body: &'static str) -> PostStub {
        use std::io::{Read, Write};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let requests: std::sync::Arc<std::sync::Mutex<Vec<test_server::RecordedRequest>>> =
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
                let req = test_server::RecordedRequest {
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

        PostStub { base_url, requests }
    }

    #[test]
    fn project_path_with_subgroups_becomes_one_segment() {
        assert_eq!(encode_project_path("group/sub/repo"), "group%2Fsub%2Frepo");
    }

    #[test]
    fn unreserved_characters_stay_unchanged() {
        assert_eq!(encode_project_path("A-z0.9_~"), "A-z0.9_~");
    }

    #[test]
    fn special_characters_as_uppercase_hex() {
        // Space, plus and multi-byte UTF-8 (ae umlaut = C3 A4)
        assert_eq!(encode_project_path("a b+\u{e4}"), "a%20b%2B%C3%A4");
    }

    #[test]
    fn pipeline_status_mapping() {
        assert_eq!(map_pipeline_status("success"), CiStatus::Success);
        assert_eq!(map_pipeline_status("failed"), CiStatus::Failed);
        assert_eq!(map_pipeline_status("running"), CiStatus::Running);
        assert_eq!(
            map_pipeline_status("waiting_for_resource"),
            CiStatus::Pending
        );
        assert_eq!(map_pipeline_status("manual"), CiStatus::Pending);
        assert_eq!(map_pipeline_status("canceling"), CiStatus::Canceled);
        assert_eq!(map_pipeline_status("skipped"), CiStatus::Unknown);
        assert_eq!(map_pipeline_status("completely-new"), CiStatus::Unknown);
    }

    // --- Body serialization of MR creation (pure, without HTTP) ---------------

    #[test]
    fn mr_body_adds_draft_prefix() {
        let body = create_mr_body(&new_mr("Feature X", "Description", true));

        assert_eq!(body["title"], "Draft: Feature X");
        assert_eq!(body["source_branch"], "feature/x");
        assert_eq!(body["target_branch"], "main");
        assert_eq!(body["description"], "Description");
    }

    #[test]
    fn mr_body_does_not_double_the_draft_prefix() {
        // An existing prefix stays untouched — case-insensitively too.
        let body = create_mr_body(&new_mr("Draft: Feature X", "", true));
        assert_eq!(body["title"], "Draft: Feature X");

        let body = create_mr_body(&new_mr("dRaFt: Feature X", "", true));
        assert_eq!(body["title"], "dRaFt: Feature X");
    }

    #[test]
    fn mr_body_without_draft_flag_leaves_title_unchanged() {
        let body = create_mr_body(&new_mr("Feature X", "", false));
        assert_eq!(body["title"], "Feature X");
    }

    #[test]
    fn mr_body_omits_an_empty_description() {
        let body = create_mr_body(&new_mr("Feature X", "", true));

        assert!(body.get("description").is_none(), "empty -> field missing");
        let obj = body.as_object().expect("object");
        assert_eq!(obj.len(), 3, "only source/target/title");
    }

    // --- Stub tests of the new endpoints (method, path, response mapping) ------

    const MR_CREATED: &str = r#"{
      "iid": 42,
      "title": "Draft: Feature X",
      "author": {"username": "ana"},
      "source_branch": "feature/x",
      "target_branch": "main",
      "draft": true,
      "work_in_progress": false,
      "web_url": "https://gitlab.example.com/group/sub/repo/-/merge_requests/42",
      "updated_at": "2009-02-13T23:31:30Z"
    }"#;

    #[tokio::test]
    async fn create_posts_to_encoded_path_and_maps_response() {
        let server = start_post_stub(201, MR_CREATED);
        let c = client(&server.base_url);

        let cr = create_change_request(&c, "group/sub/repo", &new_mr("Feature X", "Text", true))
            .await
            .expect("create");

        assert_eq!(cr.number, 42);
        assert_eq!(cr.title, "Draft: Feature X");
        assert!(cr.is_draft);
        assert_eq!(cr.author, "ana");
        assert_eq!(cr.source_branch, "feature/x");
        assert_eq!(cr.target_branch, "main");
        assert_eq!(
            cr.web_url,
            "https://gitlab.example.com/group/sub/repo/-/merge_requests/42"
        );
        assert_eq!(cr.updated_at, 1_234_567_890);
        assert_eq!(
            cr.ci_status,
            CiStatus::Unknown,
            "freshly created -> Unknown"
        );

        let reqs = server.requests();
        assert_eq!(reqs.len(), 1, "exactly one request, no CI lookup");
        assert_eq!(reqs[0].method, "POST");
        assert_eq!(reqs[0].path, "/projects/group%2Fsub%2Frepo/merge_requests");
        assert_eq!(reqs[0].header("PRIVATE-TOKEN"), Some("test-token"));
        assert_eq!(reqs[0].header("content-type"), Some("application/json"));
    }

    #[tokio::test]
    async fn create_409_mr_already_exists_yields_api_error() {
        let server = start_post_stub(
            409,
            r#"{"message": ["Another open merge request already exists"]}"#,
        );
        let c = client(&server.base_url);

        let err = create_change_request(&c, "g/r", &new_mr("T", "", false))
            .await
            .expect_err("409 must fail");

        assert_eq!(err.code(), "api_error");
    }

    #[tokio::test]
    async fn default_branch_reads_project_field() {
        let server = test_server::start(vec![(
            "/projects/group%2Fsub%2Frepo",
            200,
            r#"{"id": 1, "default_branch": "trunk"}"#,
        )]);
        let c = client(&server.base_url);

        let branch = default_branch(&c, "group/sub/repo")
            .await
            .expect("default_branch");

        assert_eq!(branch, "trunk");
        let reqs = server.requests();
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].method, "GET");
        assert_eq!(reqs[0].path, "/projects/group%2Fsub%2Frepo");
        assert_eq!(reqs[0].header("PRIVATE-TOKEN"), Some("test-token"));
    }

    #[tokio::test]
    async fn default_branch_fallback_main() {
        // Field null (empty project; a missing field covers Option as well).
        let server = test_server::start(vec![(
            "/projects/g%2Fr",
            200,
            r#"{"id": 1, "default_branch": null}"#,
        )]);
        let c = client(&server.base_url);

        assert_eq!(default_branch(&c, "g/r").await.expect("ok"), "main");
    }
}
