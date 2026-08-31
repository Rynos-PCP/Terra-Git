//! git credential protocol: the credential-helper bridge.
//!
//! git calls the helper with `get`/`store`/`erase` and passes the request as
//! `key=value` lines on stdin (terminated by an empty line/EOF). These pure
//! functions are separated from the I/O so they stay testable.

/// A parsed credential request.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct CredentialRequest {
    pub protocol: Option<String>,
    /// Host, possibly including the port ("gitlab.example.com:8443").
    pub host: Option<String>,
}

/// Parses the `key=value` lines of a credential request.
pub fn parse_credential_input(input: &str) -> CredentialRequest {
    let mut req = CredentialRequest::default();
    for line in input.lines() {
        let line = line.trim();
        if line.is_empty() {
            break; // a blank line ends the request
        }
        match line.split_once('=') {
            Some(("protocol", v)) => req.protocol = Some(v.trim().to_string()),
            Some(("host", v)) => req.host = Some(v.trim().to_lowercase()),
            _ => {}
        }
    }
    req
}

/// Checks whether a value can safely be written into ONE line of the git
/// credential protocol. The protocol is line-based and knows no escaping: a
/// CR/LF (or NUL) would split the answer into further protocol lines chosen by
/// the attacker.
fn is_single_line(value: &str) -> bool {
    !value.bytes().any(|b| b == b'\n' || b == b'\r' || b == 0)
}

/// Answers a `get` request. `lookup` returns the pair (user name, token) per
/// host. Hosts with a port are looked up exactly first, then without the port.
/// ONLY `https` is answered: handing out the OS-keychain-protected PAT over
/// plaintext `http` (on-path interception) would undermine exactly that
/// protection; SSH goes through the system agent anyway. `None` = print
/// nothing, git asks the next helper or the user.
pub fn answer_get(
    input: &str,
    lookup: &dyn Fn(&str) -> Option<(String, String)>,
) -> Option<String> {
    let req = parse_credential_input(input);
    if req.protocol.as_deref() != Some("https") {
        return None;
    }
    let host = req.host?;
    let (username, token) = lookup(&host).or_else(|| {
        let bare = host.split(':').next()?;
        (bare != host).then(|| lookup(bare)).flatten()
    })?;
    // An empty user name would be "no value" to git — PAT auth accepts any
    // non-empty name on GitHub/GitLab.
    let username = if username.is_empty() {
        "token".to_string()
    } else {
        username
    };
    // User name/token come from the (untrusted) provider `/user` response. If
    // either contains CR/LF/NUL it could not be emitted safely as a single
    // protocol line (line injection: `quit=1`, an overriding `password=`, …) —
    // in that case stay silent (git asks the next helper or the user) instead of
    // sending a manipulable answer.
    if !is_single_line(&username) || !is_single_line(&token) {
        return None;
    }
    Some(format!("username={username}\npassword={token}\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup(host: &str) -> Option<(String, String)> {
        match host {
            "gitlab.example.com" => Some(("demo-user".into(), "glpat-123".into())),
            "github.com" => Some((String::new(), "ghp_456".into())),
            _ => None,
        }
    }

    #[test]
    fn answers_https_request() {
        let input = "protocol=https\nhost=gitlab.example.com\n\n";
        assert_eq!(
            answer_get(input, &lookup),
            Some("username=demo-user\npassword=glpat-123\n".into())
        );
    }

    #[test]
    fn host_with_port_falls_back_to_bare_host() {
        let input = "protocol=https\nhost=gitlab.example.com:8443\n";
        assert!(answer_get(input, &lookup).unwrap().contains("glpat-123"));
    }

    #[test]
    fn empty_username_becomes_token() {
        let input = "protocol=https\nhost=github.com\n";
        assert_eq!(
            answer_get(input, &lookup),
            Some("username=token\npassword=ghp_456\n".into())
        );
    }

    #[test]
    fn unknown_host_and_ssh_return_none() {
        assert_eq!(
            answer_get("protocol=https\nhost=foreign.example\n", &lookup),
            None
        );
        assert_eq!(
            answer_get("protocol=ssh\nhost=gitlab.example.com\n", &lookup),
            None
        );
        assert_eq!(answer_get("complete garbage", &lookup), None);
        assert_eq!(answer_get("", &lookup), None);
    }

    #[test]
    fn plaintext_http_is_not_answered() {
        // The PAT is stored protected by the OS keychain — handing it out over
        // plaintext http (on-path interception on the LAN) undermines exactly that.
        assert_eq!(
            answer_get("protocol=http\nhost=gitlab.example.com\n", &lookup),
            None
        );
    }

    #[test]
    fn host_is_lowercased_and_blank_line_terminates() {
        let input = "protocol=https\nhost=GitLab.Example.Com\n\nhost=other.example\n";
        assert!(answer_get(input, &lookup).unwrap().contains("demo-user"));
    }

    #[test]
    fn username_with_line_break_returns_none() {
        // Injection guard: a '\n' in the user name (from the provider `/user`
        // response) must not smuggle in an additional protocol line.
        let lookup = |host: &str| -> Option<(String, String)> {
            (host == "evil.example").then(|| ("x\nquit=1".into(), "tok".into()))
        };
        assert_eq!(
            answer_get("protocol=https\nhost=evil.example\n", &lookup),
            None
        );
    }

    #[test]
    fn token_with_carriage_return_returns_none() {
        let lookup = |host: &str| -> Option<(String, String)> {
            (host == "evil.example").then(|| ("user".into(), "tok\rpassword=hijack".into()))
        };
        assert_eq!(
            answer_get("protocol=https\nhost=evil.example\n", &lookup),
            None
        );
    }
}
