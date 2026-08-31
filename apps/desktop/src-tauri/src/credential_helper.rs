//! Headless mode: the app itself acting as a git credential helper.
//!
//! The sidecar registers `"<exe>" __credential` as an ADDITIONAL helper
//! (existing system helpers run first). git then calls
//! `tg-app __credential get` with the request on stdin; we look up the account
//! (providers.json) and the token (OS keychain). No account for the host ->
//! no output, git asks the next helper or the user.

use std::io::Read;
use std::path::PathBuf;

use tg_auth::{credential_helper_proto, KeyringStore, TokenStore};
use tg_domain::ProviderAccount;

/// Must match the "identifier" in tauri.conf.json — it determines the app
/// config directory that holds providers.json.
const IDENTIFIER: &str = "dev.terragit.desktop";

pub fn run(op: &str) {
    // store/erase are deliberately ignored: the app manages tokens itself.
    if op != "get" {
        return;
    }
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);

    let accounts = load_accounts();
    let answer = credential_helper_proto::answer_get(&input, &|host| {
        let acc = accounts
            .iter()
            .find(|a| a.host.eq_ignore_ascii_case(host))?;
        let token = KeyringStore::new().get(&acc.host).ok().flatten()?;
        Some((acc.username.clone(), token))
    });
    if let Some(a) = answer {
        print!("{a}");
    }
}

fn load_accounts() -> Vec<ProviderAccount> {
    let Some(file) = config_dir().map(|d| d.join("providers.json")) else {
        return Vec::new();
    };
    std::fs::read_to_string(file)
        .ok()
        .and_then(|s| serde_json::from_str(s.trim_start_matches('\u{feff}')).ok())
        .unwrap_or_default()
}

/// App config directory WITHOUT a running Tauri runtime — the same paths Tauri
/// uses for `app_config_dir()`.
fn config_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join(IDENTIFIER))
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME").map(|h| {
            PathBuf::from(h)
                .join("Library/Application Support")
                .join(IDENTIFIER)
        })
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .map(|d| d.join(IDENTIFIER))
    }
}
