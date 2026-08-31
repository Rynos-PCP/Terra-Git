//! Error types of the git engine.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum GitEngineError {
    #[error("Not a Git repository: {0}")]
    NotARepository(String),

    #[error("Branch not found: {0}")]
    BranchNotFound(String),

    #[error("Branch “{0}” is not merged — deleting it has to be forced")]
    BranchNotMerged(String),

    #[error("The commit message must not be empty")]
    EmptyCommitMessage,

    #[error("There is no commit to amend")]
    NothingToAmend,

    #[error("git command failed: {message}")]
    Sidecar { message: String },

    #[error("Operation cancelled")]
    Cancelled,

    /// Classified remote error (fetch/pull/push/clone) with a stable,
    /// action-oriented code for the frontend (e.g. `non_fast_forward`).
    #[error("{message}")]
    Remote { code: &'static str, message: String },

    /// SSH error with a stable frontend code (ssh_no_home, ssh_key_exists, …).
    #[error("{message}")]
    Ssh { code: &'static str, message: String },

    /// Staleness guard of the undo executor: the branch no longer sits on
    /// the recorded tip — a reset would throw away someone else's commits.
    /// Stable frontend code `undo_stale` (same as the app's pre-check guard).
    #[error("The branch has changed in the meantime — undo/redo of this step is no longer safe")]
    UndoStale,

    #[error("{0}")]
    InvalidOperation(String),

    /// Checkout refused because it would overwrite uncommitted changes. libgit2
    /// reports this as "n conflicts prevent checkout" — it has NOTHING to do
    /// with merge conflicts (no index conflict, no operation in progress), and
    /// the message names neither the files nor the way out. This error carries
    /// both: the paths come from the checkout notify callback, the text is built
    /// by the frontend from the error code.
    #[error("{}", .files.join(", "))]
    CheckoutWouldOverwrite { files: Vec<String> },

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Git(#[from] git2::Error),
}

pub type Result<T> = std::result::Result<T, GitEngineError>;

impl GitEngineError {
    /// Stable error code for the frontend (independent of the message text).
    pub fn code(&self) -> &'static str {
        match self {
            GitEngineError::NotARepository(_) => "not_a_repository",
            GitEngineError::BranchNotFound(_) => "branch_not_found",
            GitEngineError::BranchNotMerged(_) => "branch_not_merged",
            GitEngineError::EmptyCommitMessage => "empty_commit_message",
            GitEngineError::NothingToAmend => "nothing_to_amend",
            GitEngineError::Sidecar { .. } => "sidecar_failed",
            GitEngineError::Cancelled => "cancelled",
            GitEngineError::Remote { code, .. } => code,
            GitEngineError::Ssh { code, .. } => code,
            GitEngineError::UndoStale => "undo_stale",
            GitEngineError::InvalidOperation(_) => "invalid_operation",
            GitEngineError::CheckoutWouldOverwrite { .. } => "checkout_would_overwrite",
            GitEngineError::Io(_) => "io_error",
            GitEngineError::Git(_) => "git_error",
        }
    }
}
