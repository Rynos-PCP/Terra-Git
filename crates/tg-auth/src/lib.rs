//! terra-git credential management.
//!
//! Tokens (PATs) live exclusively in the OS keychain — Windows Credential
//! Manager, macOS Keychain, Linux Secret Service — never in files or logs.
//! `TokenStore` is cut as a trait so tests can run without an OS keychain
//! (`MemoryStore`) and the backing store stays replaceable later on.

// Parser/response builder of the git-credential HELPER protocol (SEC27: it used
// to be misleadingly named `credential` — it is NOT the credential model).
pub mod credential_helper_proto;

use std::collections::HashMap;
use std::sync::Mutex;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("Keychain error: {0}")]
    Keyring(String),
}

pub type Result<T> = std::result::Result<T, AuthError>;

/// Abstraction over the token store (host -> token).
pub trait TokenStore: Send + Sync {
    fn set(&self, host: &str, token: &str) -> Result<()>;
    fn get(&self, host: &str) -> Result<Option<String>>;
    /// Idempotent: a missing entry is not an error.
    fn delete(&self, host: &str) -> Result<()>;
}

/// Production store: OS keychain via `keyring`.
pub struct KeyringStore {
    service: String,
}

impl KeyringStore {
    /// Default service name ("terra-git") — one keychain entry per host.
    pub fn new() -> Self {
        Self::with_service("terra-git")
    }

    /// Custom service name (lets tests clean up safely).
    pub fn with_service(service: &str) -> Self {
        Self {
            service: service.to_string(),
        }
    }

    fn entry(&self, host: &str) -> Result<keyring::Entry> {
        keyring::Entry::new(&self.service, host).map_err(|e| AuthError::Keyring(e.to_string()))
    }
}

impl Default for KeyringStore {
    fn default() -> Self {
        Self::new()
    }
}

impl TokenStore for KeyringStore {
    fn set(&self, host: &str, token: &str) -> Result<()> {
        self.entry(host)?
            .set_password(token)
            .map_err(|e| AuthError::Keyring(e.to_string()))
    }

    fn get(&self, host: &str) -> Result<Option<String>> {
        match self.entry(host)?.get_password() {
            Ok(token) => Ok(Some(token)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(AuthError::Keyring(e.to_string())),
        }
    }

    fn delete(&self, host: &str) -> Result<()> {
        match self.entry(host)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(AuthError::Keyring(e.to_string())),
        }
    }
}

/// In-memory store for tests (no OS access).
#[derive(Default)]
pub struct MemoryStore {
    map: Mutex<HashMap<String, String>>,
}

impl TokenStore for MemoryStore {
    fn set(&self, host: &str, token: &str) -> Result<()> {
        self.map
            .lock()
            .unwrap()
            .insert(host.to_string(), token.to_string());
        Ok(())
    }

    fn get(&self, host: &str) -> Result<Option<String>> {
        Ok(self.map.lock().unwrap().get(host).cloned())
    }

    fn delete(&self, host: &str) -> Result<()> {
        self.map.lock().unwrap().remove(host);
        Ok(())
    }
}
