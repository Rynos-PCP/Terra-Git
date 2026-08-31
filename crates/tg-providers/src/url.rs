//! Splits git remote URLs into host (API origin) + project path.

/// Target of a remote from the provider's point of view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteTarget {
    /// Host name, lower-cased; for http(s) including a non-standard port.
    pub host: String,
    /// Project path without a leading/trailing slash and without `.git`
    /// (GitLab subgroups are preserved, so is the letter case).
    pub repo_path: String,
}

/// Parses https, http, ssh:// and scp-style remote URLs.
/// Local paths and paths with fewer than two segments yield `None`.
pub fn parse_remote_url(url: &str) -> Option<RemoteTarget> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }

    if let Some(rest) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
    {
        // authority[/path] — discard credentials (user[:pass]@).
        let (authority, path) = rest.split_once('/')?;
        let authority = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
        return build(authority.to_lowercase(), path);
    }

    if let Some(rest) = url.strip_prefix("ssh://") {
        // ssh://[user@]host[:port]/path — the SSH port is irrelevant for the
        // HTTPS API and gets discarded.
        let (authority, path) = rest.split_once('/')?;
        let authority = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
        let host = authority.split(':').next()?;
        return build(host.to_lowercase(), path);
    }

    // scp syntax: [user@]host:path — but no Windows drive paths (C:\…) and no
    // local paths.
    if !url.contains("://") && !url.starts_with('/') && !url.starts_with('.') {
        if let Some((left, path)) = url.split_once(':') {
            if left.len() > 1 && !path.starts_with('\\') && !path.starts_with('/') {
                let host = left.rsplit_once('@').map_or(left, |(_, h)| h);
                if host.contains('.') || host.contains('-') {
                    return build(host.to_lowercase(), path);
                }
            }
        }
    }

    None
}

fn build(host: String, path: &str) -> Option<RemoteTarget> {
    let repo_path = path
        .trim_matches('/')
        .trim_end_matches(".git")
        .trim_end_matches('/');
    if host.is_empty() || repo_path.split('/').filter(|s| !s.is_empty()).count() < 2 {
        return None;
    }
    Some(RemoteTarget {
        host,
        repo_path: repo_path.to_string(),
    })
}
