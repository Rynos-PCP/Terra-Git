//! Tests of the remote URL parsing (host + project path for the provider API).

use tg_providers::parse_remote_url;

fn parts(url: &str) -> Option<(String, String)> {
    parse_remote_url(url).map(|t| (t.host, t.repo_path))
}

#[test]
fn https_urls() {
    assert_eq!(
        parts("https://github.com/foo/bar.git"),
        Some(("github.com".into(), "foo/bar".into()))
    );
    // Without .git, with a trailing slash
    assert_eq!(
        parts("https://github.com/foo/bar/"),
        Some(("github.com".into(), "foo/bar".into()))
    );
    // GitLab subgroups are preserved in full
    assert_eq!(
        parts("https://gitlab.example.com/group/sub/repo.git"),
        Some(("gitlab.example.com".into(), "group/sub/repo".into()))
    );
    // Self-hosted by IP (also without .git)
    assert_eq!(
        parts("https://192.0.2.10/acme/terra-git"),
        Some(("192.0.2.10".into(), "acme/terra-git".into()))
    );
    // A non-standard port belongs to the host (the API origin)
    assert_eq!(
        parts("https://gitlab.example.com:8443/g/r.git"),
        Some(("gitlab.example.com:8443".into(), "g/r".into()))
    );
    // Credentials in the URL are discarded
    assert_eq!(
        parts("https://user:pass@github.com/foo/bar.git"),
        Some(("github.com".into(), "foo/bar".into()))
    );
    // http (e.g. a LAN instance) is accepted
    assert_eq!(
        parts("http://gitlab.local/g/r.git"),
        Some(("gitlab.local".into(), "g/r".into()))
    );
}

#[test]
fn ssh_urls() {
    // scp syntax
    assert_eq!(
        parts("git@github.com:foo/bar.git"),
        Some(("github.com".into(), "foo/bar".into()))
    );
    assert_eq!(
        parts("git@gitlab.example.com:group/sub/repo.git"),
        Some(("gitlab.example.com".into(), "group/sub/repo".into()))
    );
    // ssh:// syntax; the SSH port is irrelevant for the HTTPS API and is dropped
    assert_eq!(
        parts("ssh://git@gitlab.example.com:2222/group/repo.git"),
        Some(("gitlab.example.com".into(), "group/repo".into()))
    );
}

#[test]
fn host_is_lowercased_path_is_not() {
    assert_eq!(
        parts("https://GitHub.COM/Foo/Bar.git"),
        Some(("github.com".into(), "Foo/Bar".into()))
    );
}

#[test]
fn invalid_remotes_yield_none() {
    // Local paths (not a hosting provider)
    assert_eq!(parts(r"C:\repos\demo"), None);
    assert_eq!(parts("/home/user/repo"), None);
    assert_eq!(parts("../relative"), None);
    // Too few path segments (owner/repo required)
    assert_eq!(parts("https://github.com/owneronly"), None);
    assert_eq!(parts("git@github.com:owneronly.git"), None);
    assert_eq!(parts(""), None);
}
