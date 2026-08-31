//! Error mapping at the command boundary: engine errors are translated into a
//! serializable, stable format for the frontend.

use serde::Serialize;
use tg_git_engine::error::GitEngineError;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandError {
    /// Stable, machine-readable code (e.g. `not_a_repository`).
    pub code: String,
    /// Human-readable message (English).
    pub message: String,
}

impl CommandError {
    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            code: "internal".into(),
            message: message.into(),
        }
    }
}

impl From<GitEngineError> for CommandError {
    fn from(err: GitEngineError) -> Self {
        Self {
            code: err.code().into(),
            message: err.to_string(),
        }
    }
}

impl From<tg_providers::ProviderError> for CommandError {
    fn from(err: tg_providers::ProviderError) -> Self {
        Self {
            code: err.code().into(),
            message: err.to_string(),
        }
    }
}

impl From<tg_auth::AuthError> for CommandError {
    fn from(err: tg_auth::AuthError) -> Self {
        Self {
            code: "keychain".into(),
            message: err.to_string(),
        }
    }
}
