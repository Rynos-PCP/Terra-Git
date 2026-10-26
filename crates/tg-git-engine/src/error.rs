//! Error types of the git engine.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum GitEngineError {
    #[error("Not a Git repository: {0}")]
    NotARepository(String),

    /// git refuses the repository because it belongs to another user
    /// ("dubious ownership"); the path is what `safe.directory` needs.
    #[error("git does not trust this folder (it belongs to another user): {0}")]
    UnsafeRepository(String),

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
    /// action-oriented code for the frontend (e.g. `non_fast_forward`). `raw`
    /// keeps git's own output for the error details (empty if there is none).
    #[error("{message}")]
    Remote {
        code: &'static str,
        message: String,
        raw: String,
    },

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

    /// The copy a discard needs does not fit into the storage limit of the
    /// discard store. Nothing is discarded: the user frees space or raises the
    /// limit first. The message carries the three byte counts, space-separated
    /// (`<needed> <used> <limit>`), for the frontend to format.
    #[error("{needed} {used} {limit}")]
    DiscardStoreFull { needed: u64, used: u64, limit: u64 },

    /// Restoring (or re-applying) a discard would overwrite files that changed
    /// since — they are named instead of overwritten.
    #[error("{}", .files.join(", "))]
    DiscardRestoreConflict { files: Vec<String> },

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Git(#[from] git2::Error),
}

pub type Result<T> = std::result::Result<T, GitEngineError>;

/// Stable code for a libgit2 error: the common causes by
/// libgit2's own error code, the rest stays `git_error`.
fn git2_code(e: &git2::Error) -> &'static str {
    let msg = e.message().to_ascii_lowercase();
    match e.code() {
        // Locked also covers ref locks (`refs/heads/main.lock`): only the index
        // lock is what the "remove lock" way out removes.
        git2::ErrorCode::Locked if msg.contains("index.lock") => "index_locked",
        git2::ErrorCode::Owner => "unsafe_repository",
        _ if e.class() == git2::ErrorClass::Config
            && (msg.contains("user.name") || msg.contains("user.email")) =>
        {
            "identity_missing"
        }
        _ if msg.contains("path too long") || msg.contains("filename too long") => "path_too_long",
        _ => "git_error",
    }
}

/// Maps a failed `Repository::discover`/`open`: a repository git refuses for
/// its owner is not "not a repository" — it gets its own way out.
pub fn open_error(path: &std::path::Path, e: git2::Error) -> GitEngineError {
    if e.code() == git2::ErrorCode::Owner {
        GitEngineError::UnsafeRepository(path.display().to_string())
    } else {
        GitEngineError::NotARepository(path.display().to_string())
    }
}

impl GitEngineError {
    /// git's raw output behind the error, for the details view — only for
    /// errors that come from a git process. The message stays the short,
    /// human-readable part.
    pub fn detail(&self) -> Option<&str> {
        match self {
            GitEngineError::Sidecar { message } => Some(message),
            GitEngineError::Remote { raw, .. } if !raw.trim().is_empty() => Some(raw),
            _ => None,
        }
    }

    /// Stable error code for the frontend (independent of the message text).
    pub fn code(&self) -> &'static str {
        match self {
            GitEngineError::NotARepository(_) => "not_a_repository",
            GitEngineError::UnsafeRepository(_) => "unsafe_repository",
            GitEngineError::BranchNotFound(_) => "branch_not_found",
            GitEngineError::BranchNotMerged(_) => "branch_not_merged",
            GitEngineError::EmptyCommitMessage => "empty_commit_message",
            GitEngineError::NothingToAmend => "nothing_to_amend",
            // Unclassified git output: still name the common local causes.
            GitEngineError::Sidecar { message } => {
                match crate::sidecar::classify_local_error(message) {
                    Some((code, _)) => code,
                    None => "sidecar_failed",
                }
            }
            GitEngineError::Cancelled => "cancelled",
            GitEngineError::Remote { code, .. } => code,
            GitEngineError::Ssh { code, .. } => code,
            GitEngineError::UndoStale => "undo_stale",
            GitEngineError::InvalidOperation(_) => "invalid_operation",
            GitEngineError::CheckoutWouldOverwrite { .. } => "checkout_would_overwrite",
            GitEngineError::DiscardStoreFull { .. } => "discard_store_full",
            GitEngineError::DiscardRestoreConflict { .. } => "discard_restore_conflict",
            GitEngineError::Io(_) => "io_error",
            GitEngineError::Git(e) => git2_code(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err(code: git2::ErrorCode, class: git2::ErrorClass, msg: &str) -> GitEngineError {
        GitEngineError::Git(git2::Error::new(code, class, msg))
    }

    #[test]
    fn libgit2_errors_get_their_codes() {
        use git2::{ErrorClass as C, ErrorCode as E};
        let locked = err(
            E::Locked,
            C::Index,
            "failed to create locked file 'x/.git/index.lock'",
        );
        assert_eq!(locked.code(), "index_locked");
        assert_eq!(
            err(E::Owner, C::Config, "not owned by current user").code(),
            "unsafe_repository"
        );
        let ident = err(
            E::NotFound,
            C::Config,
            "config value 'user.name' was not found",
        );
        assert_eq!(ident.code(), "identity_missing");
        assert_eq!(
            err(E::GenericError, C::Os, "path too long").code(),
            "path_too_long"
        );
        assert_eq!(
            err(E::GenericError, C::Odb, "object not found").code(),
            "git_error"
        );
    }

    #[test]
    fn unclassified_git_output_names_local_causes() {
        let e = GitEngineError::Sidecar {
            message: "fatal: Unable to create '/r/.git/index.lock': File exists.".into(),
        };
        assert_eq!(e.code(), "index_locked");
        let e = GitEngineError::Sidecar {
            message: "fatal: something unexpected".into(),
        };
        assert_eq!(e.code(), "sidecar_failed");
    }

    #[test]
    fn an_owner_refusal_is_not_not_a_repository() {
        let path = std::path::Path::new("/r");
        let owner = git2::Error::new(git2::ErrorCode::Owner, git2::ErrorClass::Config, "x");
        assert_eq!(open_error(path, owner).code(), "unsafe_repository");
        let other = git2::Error::new(git2::ErrorCode::NotFound, git2::ErrorClass::Repository, "x");
        assert_eq!(open_error(path, other).code(), "not_a_repository");
    }
}
