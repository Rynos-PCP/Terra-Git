//! Multi-level undo/redo stack (a v1 differentiator).
//!
//! Session-local, per repository. The entries carry executable, inverse actions
//! (tg_domain::UndoAction); the engine performs them (`apply_undo_action`). The
//! durable safety net stays the backup refs — this stack is the convenient
//! multi-step layer on top.

use std::collections::HashMap;
use std::sync::Mutex;

use tg_domain::{UndoEntry, UndoStatus};

/// Upper bound per repo — older entries fall off the back.
const MAX_ENTRIES: usize = 50;

#[derive(Default)]
struct RepoStacks {
    undo: Vec<UndoEntry>,
    redo: Vec<UndoEntry>,
}

/// Tauri-managed state: undo/redo stacks of all open repos.
#[derive(Default)]
pub struct UndoState(Mutex<HashMap<String, RepoStacks>>);

/// Path normalization as in recents.rs (Windows: slashes + case).
fn key(path: &str) -> String {
    let s = if cfg!(windows) {
        path.replace('/', "\\").to_lowercase()
    } else {
        path.to_string()
    };
    s.trim_end_matches(['\\', '/']).to_string()
}

impl UndoState {
    /// Record a new operation: it lands on the undo stack and discards the redo
    /// stack (classic semantics).
    pub fn push(&self, path: &str, entry: UndoEntry) {
        let mut map = self.0.lock().unwrap_or_else(|p| p.into_inner());
        let stacks = map.entry(key(path)).or_default();
        stacks.undo.push(entry);
        stacks.redo.clear();
        if stacks.undo.len() > MAX_ENTRIES {
            let overflow = stacks.undo.len() - MAX_ENTRIES;
            stacks.undo.drain(..overflow);
        }
    }

    pub fn pop_undo(&self, path: &str) -> Option<UndoEntry> {
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get_mut(&key(path))?
            .undo
            .pop()
    }

    pub fn pop_redo(&self, path: &str) -> Option<UndoEntry> {
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get_mut(&key(path))?
            .redo
            .pop()
    }

    /// After a successful undo: the entry moves to the redo stack.
    pub fn push_redo(&self, path: &str, entry: UndoEntry) {
        let mut map = self.0.lock().unwrap_or_else(|p| p.into_inner());
        map.entry(key(path)).or_default().redo.push(entry);
    }

    /// Failed undo: the entry goes back onto the undo stack (WITHOUT discarding
    /// the redo stack).
    pub fn push_undo_back(&self, path: &str, entry: UndoEntry) {
        let mut map = self.0.lock().unwrap_or_else(|p| p.into_inner());
        map.entry(key(path)).or_default().undo.push(entry);
    }

    pub fn status(&self, path: &str) -> UndoStatus {
        let map = self.0.lock().unwrap_or_else(|p| p.into_inner());
        let stacks = map.get(&key(path));
        UndoStatus {
            undo: stacks.and_then(|s| s.undo.last().cloned()),
            redo: stacks.and_then(|s| s.redo.last().cloned()),
            undo_count: stacks.map_or(0, |s| s.undo.len()),
            redo_count: stacks.map_or(0, |s| s.redo.len()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tg_domain::{ResetMode, UndoAction};

    fn entry(op: &str) -> UndoEntry {
        UndoEntry {
            op: op.into(),
            detail: None,
            timestamp: 0,
            undo: UndoAction::ResetBranch {
                branch: "main".into(),
                commit: "a".into(),
                mode: ResetMode::Soft,
            },
            redo: UndoAction::ResetBranch {
                branch: "main".into(),
                commit: "b".into(),
                mode: ResetMode::Soft,
            },
        }
    }

    #[test]
    fn push_pop_and_redo_flow() {
        let s = UndoState::default();
        assert_eq!(s.status("C:/repo").undo_count, 0);

        s.push("C:/repo", entry("commit"));
        s.push("C:/repo", entry("merge"));
        assert_eq!(s.status("C:/repo").undo_count, 2);
        assert_eq!(s.status("C:/repo").undo.unwrap().op, "merge");

        // Undo: the entry moves to the redo stack.
        let e = s.pop_undo("C:/repo").unwrap();
        s.push_redo("C:/repo", e);
        let st = s.status("C:/repo");
        assert_eq!((st.undo_count, st.redo_count), (1, 1));
        assert_eq!(st.redo.unwrap().op, "merge");

        // A new operation discards the redo stack.
        s.push("C:/repo", entry("rebase"));
        let st = s.status("C:/repo");
        assert_eq!((st.undo_count, st.redo_count), (2, 0));
    }

    #[test]
    fn failed_undo_keeps_both_stacks() {
        let s = UndoState::default();
        s.push("C:/repo", entry("commit"));
        let e = s.pop_undo("C:/repo").unwrap();
        s.push_redo("C:/repo", e);
        // A failed redo pushes the entry back onto the redo stack (pop_redo +
        // push_redo on error). Same for undo: push_undo_back leaves redo alone.
        let e = s.pop_redo("C:/repo").unwrap();
        s.push_undo_back("C:/repo", e);
        let st = s.status("C:/repo");
        assert_eq!((st.undo_count, st.redo_count), (1, 0));
    }

    #[test]
    fn repos_are_separate_and_paths_normalized() {
        let s = UndoState::default();
        s.push("C:/repo", entry("commit"));
        assert_eq!(s.status("C:/repo").undo_count, 1);
        // Folding case and slashes is a Windows property of `key()`; on a
        // case-sensitive filesystem the same spelling is a different repository
        // and must stay separate.
        if cfg!(windows) {
            assert_eq!(
                s.status("c:\\repo").undo_count,
                1,
                "same path, different spelling"
            );
        }
        assert_eq!(s.status("C:/other").undo_count, 0);
    }

    #[test]
    fn capping_at_max_entries() {
        let s = UndoState::default();
        for i in 0..60 {
            s.push("C:/repo", entry(&format!("op{i}")));
        }
        let st = s.status("C:/repo");
        assert_eq!(st.undo_count, 50);
        assert_eq!(st.undo.unwrap().op, "op59", "the newest are kept");
    }
}
