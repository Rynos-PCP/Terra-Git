//! Integration tests for the GitHub client against the local test server.

mod common;

use tg_domain::{CiStatus, ProviderKind};
use tg_providers::ProviderClient;

fn client(base_url: &str) -> ProviderClient {
    ProviderClient::with_api_base(ProviderKind::Github, base_url, "test-token", false)
        .expect("client")
}

// ---- Fixtures ---------------------------------------------------------------

/// Two open PRs, most recently updated first; #100 without a "user" field (a
/// deleted account) and as a draft.
const PULLS_TWO: &str = r#"[
  {
    "number": 101,
    "title": "Feature: Palette",
    "user": { "login": "alice" },
    "head": { "ref": "feature/palette", "sha": "aaa111" },
    "base": { "ref": "main" },
    "draft": false,
    "html_url": "https://github.example/octo/hello/pull/101",
    "updated_at": "2026-07-01T12:00:00Z"
  },
  {
    "number": 100,
    "title": "Fix: EOL",
    "head": { "ref": "fix/eol", "sha": "bbb222" },
    "base": { "ref": "develop" },
    "draft": true,
    "html_url": "https://github.example/octo/hello/pull/100",
    "updated_at": "2026-06-30T08:30:00Z"
  }
]"#;

const CHECK_RUNS_GREEN: &str = r#"{
  "total_count": 2,
  "check_runs": [
    { "status": "completed", "conclusion": "success" },
    { "status": "completed", "conclusion": "skipped" }
  ]
}"#;

const CHECK_RUNS_FAILED: &str = r#"{
  "total_count": 2,
  "check_runs": [
    { "status": "completed", "conclusion": "success" },
    { "status": "completed", "conclusion": "failure" }
  ]
}"#;

/// The first page is entirely green, but total_count > the loaded runs
/// (pagination, the default would be 30): success would be a guess -> pending.
const CHECK_RUNS_PAGED: &str = r#"{
  "total_count": 31,
  "check_runs": [
    { "status": "completed", "conclusion": "success" },
    { "status": "completed", "conclusion": "success" }
  ]
}"#;

/// One run waits for a user action (e.g. workflow approval).
const CHECK_RUNS_ACTION_REQUIRED: &str = r#"{
  "total_count": 1,
  "check_runs": [
    { "status": "completed", "conclusion": "action_required" }
  ]
}"#;

const PULLS_ONE: &str = r#"[
  {
    "number": 7,
    "title": "Status API only",
    "user": { "login": "bob" },
    "head": { "ref": "topic", "sha": "ccc333" },
    "base": { "ref": "main" },
    "draft": false,
    "html_url": "https://github.example/o/r/pull/7",
    "updated_at": "2026-07-01T12:00:00Z"
  }
]"#;

// ---- Tests ------------------------------------------------------------------

#[tokio::test]
async fn validate_returns_login_and_sets_required_headers() {
    let server = common::start(vec![("/user", 200, r#"{"login":"octocat"}"#)]);
    let c = client(&server.base_url);

    let login = c.validate().await.expect("validate");
    assert_eq!(login, "octocat");

    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    let req = &requests[0];
    assert_eq!(req.method, "GET");
    assert_eq!(req.path, "/user");
    assert_eq!(req.header("authorization"), Some("Bearer test-token"));
    assert_eq!(req.header("user-agent"), Some("terra-git"));
    assert_eq!(req.header("accept"), Some("application/vnd.github+json"));
    assert_eq!(req.header("x-github-api-version"), Some("2022-11-28"));
}

#[tokio::test]
async fn list_maps_fields_and_ci_status_from_check_runs() {
    let server = common::start(vec![
        (
            "/repos/octo/hello/commits/aaa111/check-runs",
            200,
            CHECK_RUNS_GREEN,
        ),
        (
            "/repos/octo/hello/commits/bbb222/check-runs",
            200,
            CHECK_RUNS_FAILED,
        ),
        ("/repos/octo/hello/pulls", 200, PULLS_TWO),
    ]);
    let c = client(&server.base_url);

    let crs = c.list_change_requests("octo/hello").await.expect("list");
    assert_eq!(crs.len(), 2);

    let first = &crs[0];
    assert_eq!(first.number, 101);
    assert_eq!(first.title, "Feature: Palette");
    assert_eq!(first.author, "alice");
    assert_eq!(first.source_branch, "feature/palette");
    assert_eq!(first.target_branch, "main");
    assert!(!first.is_draft);
    assert_eq!(first.web_url, "https://github.example/octo/hello/pull/101");
    assert_eq!(first.updated_at, 1_782_907_200); // 2026-07-01T12:00:00Z
    assert_eq!(first.ci_status, CiStatus::Success);

    let second = &crs[1];
    assert_eq!(second.number, 100);
    assert_eq!(second.title, "Fix: EOL");
    assert_eq!(second.author, ""); // "user" missing -> empty
    assert_eq!(second.source_branch, "fix/eol");
    assert_eq!(second.target_branch, "develop");
    assert!(second.is_draft);
    assert_eq!(second.web_url, "https://github.example/octo/hello/pull/100");
    assert_eq!(second.updated_at, 1_782_808_200); // 2026-06-30T08:30:00Z
    assert_eq!(second.ci_status, CiStatus::Failed);

    // The pulls call carries the specified query parameters.
    let requests = server.requests();
    let pulls_req = requests
        .iter()
        .find(|r| r.path.starts_with("/repos/octo/hello/pulls"))
        .expect("pulls request recorded");
    assert_eq!(
        pulls_req.path,
        "/repos/octo/hello/pulls?state=open&per_page=50&sort=updated&direction=desc"
    );

    // Check runs are queried with the API maximum (the default of 30 would treat
    // only the first page as the whole truth when there are many runs).
    let check_req = requests
        .iter()
        .find(|r| r.path.contains("/check-runs"))
        .expect("check-runs request recorded");
    assert!(
        check_req.path.ends_with("/check-runs?per_page=100"),
        "per_page=100 missing: {}",
        check_req.path
    );
}

#[tokio::test]
async fn more_check_runs_than_one_page_yields_pending() {
    // 31 runs reported, only 2 (all green) loaded: the unseen runs could be red —
    // conservatively pending instead of a guessed success.
    let server = common::start(vec![
        (
            "/repos/o/r/commits/ccc333/check-runs",
            200,
            CHECK_RUNS_PAGED,
        ),
        ("/repos/o/r/pulls", 200, PULLS_ONE),
    ]);
    let c = client(&server.base_url);

    let crs = c.list_change_requests("o/r").await.expect("list");
    assert_eq!(crs.len(), 1);
    assert_eq!(crs[0].ci_status, CiStatus::Pending);
}

#[tokio::test]
async fn action_required_is_reported_as_pending() {
    // action_required waits for a user click (like GitLab's "manual"): as success
    // a blocked PR would wrongly be shown green.
    let server = common::start(vec![
        (
            "/repos/o/r/commits/ccc333/check-runs",
            200,
            CHECK_RUNS_ACTION_REQUIRED,
        ),
        ("/repos/o/r/pulls", 200, PULLS_ONE),
    ]);
    let c = client(&server.base_url);

    let crs = c.list_change_requests("o/r").await.expect("list");
    assert_eq!(crs.len(), 1);
    assert_eq!(crs[0].ci_status, CiStatus::Pending);
}

#[tokio::test]
async fn unauthorized_on_pulls_returns_auth_failed() {
    let server = common::start(vec![(
        "/repos/o/r/pulls",
        401,
        r#"{"message":"Bad credentials"}"#,
    )]);
    let c = client(&server.base_url);

    let err = c.list_change_requests("o/r").await.expect_err("401");
    assert_eq!(err.code(), "auth_failed");
}

#[tokio::test]
async fn empty_check_runs_fall_back_to_commit_status() {
    let server = common::start(vec![
        (
            "/repos/o/r/commits/ccc333/check-runs",
            200,
            r#"{"total_count":0,"check_runs":[]}"#,
        ),
        (
            "/repos/o/r/commits/ccc333/status",
            200,
            r#"{"state":"success","total_count":3}"#,
        ),
        ("/repos/o/r/pulls", 200, PULLS_ONE),
    ]);
    let c = client(&server.base_url);

    let crs = c.list_change_requests("o/r").await.expect("list");
    assert_eq!(crs.len(), 1);
    assert_eq!(crs[0].ci_status, CiStatus::Success);

    // Both CI endpoints were actually queried.
    let requests = server.requests();
    assert!(requests
        .iter()
        .any(|r| r.path == "/repos/o/r/commits/ccc333/check-runs?per_page=100"));
    assert!(requests
        .iter()
        .any(|r| r.path == "/repos/o/r/commits/ccc333/status"));
}

#[tokio::test]
async fn repo_path_uses_only_the_first_two_segments() {
    let server = common::start(vec![("/repos/owner/repo/pulls", 200, "[]")]);
    let c = client(&server.base_url);

    let crs = c
        .list_change_requests("owner/repo/extra")
        .await
        .expect("list");
    assert!(crs.is_empty());

    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0].path.starts_with("/repos/owner/repo/pulls?"),
        "path was: {}",
        requests[0].path
    );
    assert!(!requests[0].path.contains("extra"));
}

#[tokio::test]
async fn repo_path_with_one_segment_is_invalid_response() {
    let server = common::start(vec![]);
    let c = client(&server.base_url);

    let err = c.list_change_requests("solo").await.expect_err("path");
    assert_eq!(err.code(), "invalid_response");
    // No request may have gone out at all.
    assert!(server.requests().is_empty());
}

#[tokio::test]
async fn status_422_yields_api_error_with_body_excerpt() {
    // classify_status fallthrough (GitHub reports e.g. "the PR already exists" or
    // "no diffs" on PR creation as a 422). The stub tests for
    // create_change_request/default_branch live as module tests in src/github.rs
    // until lib.rs binds the new pub(crate) functions publicly — then they can
    // move here.
    let server = common::start(vec![(
        "/repos/octo/hello/pulls",
        422,
        r#"{"message":"Validation Failed"}"#,
    )]);
    let c = client(&server.base_url);

    let err = c
        .list_change_requests("octo/hello")
        .await
        .expect_err("422 must fail");

    assert_eq!(err.code(), "api_error");
    let msg = err.to_string();
    assert!(msg.contains("422"), "status missing in: {msg}");
    assert!(
        msg.contains("Validation Failed"),
        "body excerpt missing in: {msg}"
    );
}
