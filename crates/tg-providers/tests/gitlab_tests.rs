//! Integration tests of the GitLab client against the local test HTTP server.

mod common;

use tg_domain::{CiStatus, ProviderKind};
use tg_providers::ProviderClient;

fn client(base_url: &str) -> ProviderClient {
    ProviderClient::with_api_base(ProviderKind::Gitlab, base_url, "test-token", false)
        .expect("client")
}

#[tokio::test]
async fn validate_returns_username_and_sends_private_token() {
    let server = common::start(vec![(
        "/user",
        200,
        r#"{"id": 1, "username": "ana", "name": "Ana Example"}"#,
    )]);
    let c = client(&server.base_url);

    let username = c.validate().await.expect("validate");

    assert_eq!(username, "ana");
    let reqs = server.requests();
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].method, "GET");
    assert_eq!(reqs[0].path, "/user");
    assert_eq!(reqs[0].header("PRIVATE-TOKEN"), Some("test-token"));
}

const MR_LIST: &str = r#"[
  {
    "iid": 7,
    "title": "Feature X",
    "author": {"username": "ana"},
    "source_branch": "feature/x",
    "target_branch": "main",
    "draft": false,
    "work_in_progress": false,
    "web_url": "https://gitlab.example.com/group/sub/repo/-/merge_requests/7",
    "updated_at": "2009-02-13T23:31:30Z"
  },
  {
    "iid": 3,
    "title": "Bugfix Y",
    "author": {"username": "bo"},
    "source_branch": "fix/y",
    "target_branch": "develop",
    "draft": false,
    "work_in_progress": false,
    "web_url": "https://gitlab.example.com/group/sub/repo/-/merge_requests/3",
    "updated_at": "2024-02-29T00:00:00Z"
  }
]"#;

#[tokio::test]
async fn list_with_subgroup_path_mapping_and_ci_status() {
    // More specific pipeline routes BEFORE the list route (prefix matching).
    let server = common::start(vec![
        (
            "/projects/group%2Fsub%2Frepo/merge_requests/7/pipelines",
            200,
            r#"[{"id": 100, "status": "success"}]"#,
        ),
        (
            "/projects/group%2Fsub%2Frepo/merge_requests/3/pipelines",
            200,
            r#"[{"id": 101, "status": "failed"}]"#,
        ),
        ("/projects/group%2Fsub%2Frepo/merge_requests", 200, MR_LIST),
    ]);
    let c = client(&server.base_url);

    let crs = c
        .list_change_requests("group/sub/repo")
        .await
        .expect("list");

    assert_eq!(crs.len(), 2);
    let first = &crs[0];
    assert_eq!(first.number, 7);
    assert_eq!(first.title, "Feature X");
    assert_eq!(first.author, "ana");
    assert_eq!(first.source_branch, "feature/x");
    assert_eq!(first.target_branch, "main");
    assert!(!first.is_draft);
    assert_eq!(
        first.web_url,
        "https://gitlab.example.com/group/sub/repo/-/merge_requests/7"
    );
    assert_eq!(first.updated_at, 1_234_567_890);
    assert_eq!(first.ci_status, CiStatus::Success);

    let second = &crs[1];
    assert_eq!(second.number, 3);
    assert_eq!(second.title, "Bugfix Y");
    assert_eq!(second.author, "bo");
    assert_eq!(second.source_branch, "fix/y");
    assert_eq!(second.target_branch, "develop");
    assert!(!second.is_draft);
    assert_eq!(
        second.web_url,
        "https://gitlab.example.com/group/sub/repo/-/merge_requests/3"
    );
    assert_eq!(second.updated_at, 1_709_164_800);
    assert_eq!(second.ci_status, CiStatus::Failed);

    // The project path encoded as ONE segment (subgroups: '/' -> %2F), the query
    // parameters of the list request set, the token header everywhere.
    let reqs = server.requests();
    assert_eq!(reqs.len(), 3, "list + 2 pipeline lookups");
    let list_req = reqs
        .iter()
        .find(|r| r.path.contains("merge_requests?"))
        .expect("list request");
    assert!(list_req.path.contains("group%2Fsub%2Frepo"));
    assert!(list_req.path.contains("state=opened"));
    assert!(list_req.path.contains("order_by=updated_at"));
    for r in &reqs {
        assert!(r.path.contains("group%2Fsub%2Frepo"), "path: {}", r.path);
        assert_eq!(r.header("PRIVATE-TOKEN"), Some("test-token"));
    }
}

#[tokio::test]
async fn status_401_yields_auth_failed() {
    let server = common::start(vec![("/", 401, r#"{"message": "401 Unauthorized"}"#)]);
    let c = client(&server.base_url);

    let err = c.validate().await.expect_err("validate must fail");
    assert_eq!(err.code(), "auth_failed");

    let err = c
        .list_change_requests("g/r")
        .await
        .expect_err("list must fail");
    assert_eq!(err.code(), "auth_failed");
}

#[tokio::test]
async fn status_409_yields_api_error() {
    // classify_status fallthrough (GitLab reports e.g. "the MR already exists"
    // on MR creation as a 409). The stub tests for
    // create_change_request/default_branch live as module tests in src/gitlab.rs
    // until lib.rs binds the new pub(crate) functions publicly — then they can
    // move here.
    let server = common::start(vec![("/", 409, r#"{"message": "409 Conflict"}"#)]);
    let c = client(&server.base_url);

    let err = c
        .list_change_requests("g/r")
        .await
        .expect_err("409 must fail");
    assert_eq!(err.code(), "api_error");
}

#[tokio::test]
async fn empty_pipelines_array_yields_unknown() {
    let server = common::start(vec![
        ("/projects/g%2Fr/merge_requests/5/pipelines", 200, "[]"),
        (
            "/projects/g%2Fr/merge_requests",
            200,
            r#"[{"iid": 5, "title": "T", "author": {"username": "u"}, "source_branch": "s", "target_branch": "t", "draft": false, "web_url": "https://x/5", "updated_at": "2024-01-15T10:30:00Z"}]"#,
        ),
    ]);
    let c = client(&server.base_url);

    let crs = c.list_change_requests("g/r").await.expect("list");

    assert_eq!(crs.len(), 1);
    assert_eq!(crs[0].ci_status, CiStatus::Unknown);
}

#[tokio::test]
async fn draft_detection_via_title_prefix_when_fields_are_missing() {
    // Neither "draft" nor "work_in_progress" in the payload (older instances);
    // "author" is missing too -> an empty author.
    let server = common::start(vec![
        ("/projects/g%2Fr/merge_requests/1/pipelines", 200, "[]"),
        ("/projects/g%2Fr/merge_requests/2/pipelines", 200, "[]"),
        (
            "/projects/g%2Fr/merge_requests",
            200,
            r#"[
              {"iid": 1, "title": "DRAFT: WIP-Feature", "source_branch": "a", "target_branch": "main", "web_url": "https://x/1", "updated_at": "2024-01-15T10:30:00Z"},
              {"iid": 2, "title": "Done", "source_branch": "b", "target_branch": "main", "web_url": "https://x/2", "updated_at": "2024-01-15T10:30:00Z"}
            ]"#,
        ),
    ]);
    let c = client(&server.base_url);

    let crs = c.list_change_requests("g/r").await.expect("list");

    assert_eq!(crs.len(), 2);
    assert!(crs[0].is_draft, "case-insensitive title prefix");
    assert!(!crs[1].is_draft);
    assert_eq!(crs[0].author, "", "missing author -> empty string");
}

#[tokio::test]
async fn ci_error_does_not_fail_the_list() {
    let server = common::start(vec![
        (
            "/projects/g%2Fr/merge_requests/9/pipelines",
            500,
            r#"{"message": "broken"}"#,
        ),
        (
            "/projects/g%2Fr/merge_requests",
            200,
            r#"[{"iid": 9, "title": "T", "source_branch": "s", "target_branch": "t", "web_url": "https://x/9", "updated_at": "2024-01-15T10:30:00Z"}]"#,
        ),
    ]);
    let c = client(&server.base_url);

    let crs = c
        .list_change_requests("g/r")
        .await
        .expect("list despite CI error");

    assert_eq!(crs.len(), 1);
    assert_eq!(crs[0].ci_status, CiStatus::Unknown);
}
