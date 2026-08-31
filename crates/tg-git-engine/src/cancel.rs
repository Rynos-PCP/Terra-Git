//! Cooperative cancellation for long-running operations.
//!
//! The app keeps one clone of the token per running operation (in the
//! `OpRegistry` state); the operation itself checks the flag in its wait loop
//! and kills the git child process on cancel. Cheaply clonable (`Arc`) so one
//! clone can move into the `spawn_blocking` closure while a second stays in
//! the registry state for the cancel command.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Clone, Debug, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    /// Requests cancellation (idempotent; callable from another thread).
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}
