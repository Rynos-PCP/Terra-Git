//! Structured logging to a rotating file + panic hook (M4 hardening).
//!
//! Goal: reproducible diagnosis of errors/crashes without users needing a
//! console. Logs live in the OS app log directory (rotated daily), plus stderr
//! in debug builds. The level is controlled via `RUST_LOG` (default `info`).
//! Panics land in the log as an `error!` entry.

use std::fs;
use std::path::PathBuf;

use tauri::{AppHandle, Manager};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{prelude::*, EnvFilter};

/// Keeps the background writer of the file appender alive (drop = stop flushing).
/// Held for the app's lifetime via `app.manage()`.
pub struct LogGuard(#[allow(dead_code)] WorkerGuard);

/// Determines the log directory (OS app log dir) and creates it.
pub fn log_dir(app: &AppHandle) -> Option<PathBuf> {
    let dir = app.path().app_log_dir().ok()?;
    fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// Initializes global logging and returns the guard the caller must hold for
/// the app's lifetime (drop = writer stop). `None` if the log directory is
/// missing or a subscriber was already installed.
#[must_use]
pub fn init(app: &AppHandle) -> Option<LogGuard> {
    let dir = log_dir(app)?;

    let file_appender = tracing_appender::rolling::daily(&dir, "terra-git.log");
    let (writer, guard) = tracing_appender::non_blocking(file_appender);

    // RUST_LOG wins; otherwise info for our own crates.
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,tg_app=info,tg_git_engine=info"));

    let file_layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_target(true)
        .with_writer(writer);

    let registry = tracing_subscriber::registry().with(filter).with(file_layer);

    // In debug builds also to stderr (development).
    #[cfg(debug_assertions)]
    let registry = registry.with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr));

    if registry.try_init().is_err() {
        return None; // already initialized
    }

    install_panic_hook(dir.clone());
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        dir = %dir.display(),
        "terra-git started"
    );
    Some(LogGuard(guard))
}

/// Formats a standalone crash report (easy to attach to a bug report).
fn format_crash_report(version: &str, location: &str, message: &str, epoch_nanos: u128) -> String {
    format!(
        "terra-git crash report\n\
         ======================\n\
         Version : {version}\n\
         Time    : {epoch_nanos} ns since the epoch (UTC)\n\
         Location: {location}\n\
         Message : {message}\n\
         \n\
         Please attach this file together with the daily log to the bug report.\n"
    )
}

/// Writes panics to the log (location + message) AND drops a standalone
/// `crash-<nanos>.log`, then runs the default hook.
fn install_panic_hook(dir: PathBuf) {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let loc = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "?".into());
        let msg = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "<non-textual panic>".into());
        tracing::error!(location = %loc, "PANIC: {msg}");

        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let report = format_crash_report(env!("CARGO_PKG_VERSION"), &loc, &msg, nanos);
        let _ = fs::write(dir.join(format!("crash-{nanos}.log")), report);

        default(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::format_crash_report;

    #[test]
    fn crash_report_contains_core_fields() {
        let r = format_crash_report("1.0.0", "src/x.rs:42", "index out of bounds", 12345);
        assert!(r.contains("Version : 1.0.0"));
        assert!(r.contains("src/x.rs:42"));
        assert!(r.contains("index out of bounds"));
        assert!(r.contains("12345"));
    }
}
