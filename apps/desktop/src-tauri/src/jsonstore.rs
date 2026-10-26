//! Small persistence helper: atomic writes for JSON stores.
//!
//! `std::fs::write` truncates the target file in place — a crash or power cut
//! mid-write leaves a truncated/corrupt file, which the tolerant reader
//! (`serde_json::from_str(...).unwrap_or_default()`) silently reads as an empty
//! list: every recent repo or provider account would be gone. `atomic_write`
//! therefore writes to a temp file in the SAME directory first and replaces the
//! target via rename (on Windows through MoveFileEx+REPLACE_EXISTING, atomic on
//! Unix) — a reader never sees a half-written file, and a failure leaves the
//! old file intact.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Unique suffix counter so concurrent writes (even to the same file) never
/// collide on the same temp name.
static SEQ: AtomicU64 = AtomicU64::new(0);

/// Overrides the app config directory (recents, provider accounts). Set by
/// the E2E tests, so they never touch the real profile; unset in normal use.
pub const CONFIG_DIR_ENV: &str = "TERRA_GIT_CONFIG_DIR";

/// The override of [`CONFIG_DIR_ENV`], if set. Debug builds only (the E2E
/// suite drives `target/debug`): a release build always uses the real profile,
/// so a stray variable cannot point the provider accounts somewhere else.
pub fn config_dir_override() -> Option<PathBuf> {
    if !cfg!(debug_assertions) {
        return None;
    }
    std::env::var_os(CONFIG_DIR_ENV)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// The directory the JSON stores live in: the override, else Tauri's app
/// config directory.
pub fn config_dir(app: &tauri::AppHandle) -> Option<PathBuf> {
    use tauri::Manager;
    config_dir_override().or_else(|| app.path().app_config_dir().ok())
}

/// Writes `bytes` atomically to `path` (temp file in the same directory +
/// rename). The temp name carries the PID and a process counter.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("store.json");
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = dir.join(format!(".{file_name}.tmp-{}-{}", std::process::id(), n));

    // Write and flush before renaming (otherwise the rename could become
    // visible before the bytes have been written through).
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.flush()?;
    }
    // On failure, clean up the temp file best-effort so no garbage is left
    // behind.
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{atomic_write, config_dir_override, CONFIG_DIR_ENV};

    #[test]
    fn an_empty_override_counts_as_unset() {
        // The only test touching this variable — no race with others.
        std::env::set_var(CONFIG_DIR_ENV, "");
        assert_eq!(config_dir_override(), None);
        std::env::set_var(CONFIG_DIR_ENV, "/tmp/terra-e2e-config");
        assert_eq!(
            config_dir_override(),
            Some(std::path::PathBuf::from("/tmp/terra-e2e-config"))
        );
        std::env::remove_var(CONFIG_DIR_ENV);
    }

    #[test]
    fn atomic_write_replaces_and_leaves_no_temp() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("store.json");
        std::fs::write(&target, b"OLD").unwrap();
        atomic_write(&target, b"NEW").unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "NEW");
        // No leftover .tmp files in the directory.
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp-"))
            .collect();
        assert!(leftovers.is_empty(), "temp file was left behind");
    }
}
