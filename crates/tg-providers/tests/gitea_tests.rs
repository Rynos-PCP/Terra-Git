//! Integration tests for the Gitea/Forgejo client against the local test server.

mod common;

use tg_domain::{CiStatus, ProviderKind};
use tg_providers::ProviderClient;

fn client(base_url: &str) -> ProviderClient {
    ProviderClient::with_api_base(ProviderKind::Gitea, base_url, "test-token", false)
        .expect("client")
}

// ---- Fixtures ---------------------------------------------------------------

/// Two open PRs: #101 regular, #100 without "user" and with a WIP title (draft
/// through the title convention, no native draft field).
const PULLS_TWO: &str = r#"[
  {
    "number": 101,
    "title": "Feature: Palette",
    "user": { "login": "alice" },
    "head": { "ref": "feature/palette", "sha": "aaa111" },
    "base": { "ref": "main" },
    "html_url": "https://gitea.example/octo/hello/pulls/101",
    "updated_at": "2026-07-01T12:00:00Z"
  },
  {
    "number": 100,
    "title": "WIP: Fix EOL",
    "head": { "ref": "fix/eol", "sha": "bbb222" },
    "base": { "ref": "develop" },
    "html_url": "https://gitea.example/octo/hello/pulls/100",
    "updated_at": "2026-06-30T08:30:00Z"
  }
]"#;

// ---- Tests ------------------------------------------------------------------

#[tokio::test]
async fn validate_returns_login_and_sets_token_header() {
    let server = common::start(vec![("/user", 200, r#"{"login":"forgejo-user"}"#)]);
    let c = client(&server.base_url);

    let login = c.validate().await.expect("validate");
    assert_eq!(login, "forgejo-user");

    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    let req = &requests[0];
    assert_eq!(req.method, "GET");
    assert_eq!(req.path, "/user");
    // Gitea auth scheme: "token <T>" (not Bearer / PRIVATE-TOKEN).
    assert_eq!(req.header("authorization"), Some("token test-token"));
    assert_eq!(req.header("user-agent"), Some("terra-git"));
    assert_eq!(req.header("accept"), Some("application/json"));
    assert_eq!(req.header("x-github-api-version"), None);
}

#[tokio::test]
async fn list_maps_fields_ci_and_wip_draft() {
    let server = common::start(vec![
        (
            "/repos/octo/hello/commits/aaa111/status",
            200,
            r#"{"state":"success"}"#,
        ),
        (
            "/repos/octo/hello/commits/bbb222/status",
            200,
            r#"{"state":"failure"}"#,
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
    assert_eq!(first.web_url, "https://gitea.example/octo/hello/pulls/101");
    assert_eq!(first.updated_at, 1_782_907_200); // 2026-07-01T12:00:00Z
    assert_eq!(first.ci_status, CiStatus::Success);

    let second = &crs[1];
    assert_eq!(second.number, 100);
    assert_eq!(second.author, ""); // "user" missing -> empty
    assert!(second.is_draft, "WIP title -> is_draft");
    assert_eq!(second.ci_status, CiStatus::Failed);

    // Query parameters in the Gitea style.
    let requests = server.requests();
    let pulls_req = requests
        .iter()
        .find(|r| r.path.starts_with("/repos/octo/hello/pulls"))
        .expect("pulls request recorded");
    assert_eq!(
        pulls_req.path,
        "/repos/octo/hello/pulls?state=open&sort=recentupdate&limit=50"
    );
}

#[tokio::test]
async fn unauthorized_on_pulls_returns_auth_failed() {
    let server = common::start(vec![(
        "/repos/o/r/pulls",
        401,
        r#"{"message":"unauthorized"}"#,
    )]);
    let c = client(&server.base_url);

    let err = c.list_change_requests("o/r").await.expect_err("401");
    assert_eq!(err.code(), "auth_failed");
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
    assert!(requests[0].path.starts_with("/repos/owner/repo/pulls?"));
    assert!(!requests[0].path.contains("extra"));
}

#[tokio::test]
async fn repo_path_with_one_segment_is_invalid_response() {
    let server = common::start(vec![]);
    let c = client(&server.base_url);

    let err = c.list_change_requests("solo").await.expect_err("path");
    assert_eq!(err.code(), "invalid_response");
    assert!(server.requests().is_empty());
}
