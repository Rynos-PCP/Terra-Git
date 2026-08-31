//! Registry of running, cancellable remote operations per repo.
//!
//! Every long-running op (fetch/pull/push/clone_fetch) registers its
//! [`CancelToken`] under the repo path when it starts. The `cancel_operation`
//! command sets the token; the op checks it in its wait loop and kills the git
//! child process. At most one sync op runs per repo — a second start overwrites
//! the entry (the old op finishes with its own token clone).

use std::collections::HashMap;
use std::sync::Mutex;

use tg_git_engine::CancelToken;

#[derive(Default)]
pub struct OpRegistry(Mutex<HashMap<String, CancelToken>>);

impl OpRegistry {
    fn key(path: &str) -> String {
        path.to_lowercase()
    }

    /// Registers a newly started operation and returns its cancel token
    /// (a clone stays in the registry state for the cancel command).
    pub fn register(&self, path: &str) -> CancelToken {
        let token = CancelToken::new();
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(Self::key(path), token.clone());
        token
    }

    pub fn unregister(&self, path: &str) {
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&Self::key(path));
    }

    /// Cancels the running operation for `path`. `true` if one was running.
    pub fn cancel(&self, path: &str) -> bool {
        match self
            .0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&Self::key(path))
        {
            Some(token) => {
                token.cancel();
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_cancel_and_unregister() {
        let reg = OpRegistry::default();
        // Without a running op, cancelling is a no-op.
        assert!(!reg.cancel("C:/repo"));

        // The path is case-insensitive (Windows) — the op's token clone sees the cancel.
        let token = reg.register("C:/Repo");
        assert!(!token.is_cancelled());
        assert!(reg.cancel("c:/repo"));
        assert!(token.is_cancelled());

        // After unregister, cancelling has no effect any more.
        reg.unregister("C:/repo");
        assert!(!reg.cancel("C:/repo"));
    }
}
