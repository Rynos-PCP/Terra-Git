//! Typed provider errors with stable codes for the IPC boundary.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("Token is invalid or has expired (401)")]
    Unauthorized,

    #[error("Access denied — the token lacks the required scope (403)")]
    Forbidden,

    #[error("Not found — check the project path or API URL (404)")]
    NotFound,

    #[error("Provider rate limit reached (429)")]
    RateLimited,

    #[error("Network error: {0}")]
    Network(String),

    #[error("API error {status}: {message}")]
    Api { status: u16, message: String },

    #[error("Unexpected API response: {0}")]
    InvalidResponse(String),
}

impl ProviderError {
    /// Stable error code for the frontend.
    pub fn code(&self) -> &'static str {
        match self {
            ProviderError::Unauthorized => "auth_failed",
            ProviderError::Forbidden => "forbidden",
            ProviderError::NotFound => "not_found",
            ProviderError::RateLimited => "rate_limited",
            ProviderError::Network(_) => "network",
            ProviderError::Api { .. } => "api_error",
            ProviderError::InvalidResponse(_) => "invalid_response",
        }
    }
}

impl From<reqwest::Error> for ProviderError {
    fn from(e: reqwest::Error) -> Self {
        ProviderError::Network(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, ProviderError>;

/// Maps a non-2xx status onto the matching typed error.
pub(crate) fn classify_status(status: u16, body_snippet: &str) -> ProviderError {
    match status {
        401 => ProviderError::Unauthorized,
        403 => ProviderError::Forbidden,
        404 => ProviderError::NotFound,
        429 => ProviderError::RateLimited,
        s => ProviderError::Api {
            status: s,
            message: body_snippet.chars().take(200).collect(),
        },
    }
}
