//! File watcher: reports workdir/ref changes to the frontend immediately
//! (event `repo-changed`) instead of waiting for the next status poll.
//!
//! Windows: ReadDirectoryChangesW, macOS: FSEvents, Linux: inotify (via
//! `notify`). Events are debounced (400 ms trailing edge) so a build with
//! thousands of file writes only triggers a handful of refreshes.

use std::path::Path;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use notify::{Config, PollWatcher, RecursiveMode, Watcher};
use tauri::{AppHandle, Emitter};

/// At most ONE repo is watched at a time (the currently opened one). The old
/// watcher is replaced on switch; its debounce thread ends as soon as the mpsc
/// sender (inside the watcher callback) has been dropped.
///
/// `Arc` so the async commands can take a `'static` clone onto the
/// blocking pool — `tauri::State` itself is not `'static`.
#[derive(Default, Clone)]
pub struct WatchState(Arc<Mutex<Option<ActiveWatch>>>);

struct ActiveWatch {
    // Only kept alive; dropping it stops the OS-level watch. Boxed because
    // either a native or a polling watcher is active depending on availability.
    _watcher: Box<dyn Watcher + Send>,
}

/// Relevance filter: workdir changes yes; from inside `.git` only what visibly
/// changes status/branches/history. In particular NOT `index.lock` & co.,
/// otherwise every operation of our own would trigger double refreshes.
fn is_relevant(path: &Path) -> bool {
    let s = path.to_string_lossy().replace('\\', "/");
    if let Some(idx) = s.find("/.git/") {
        let inner = &s[idx + 6..];
        return inner == "HEAD"
            || inner == "index"
            || inner == "packed-refs"
            || inner == "MERGE_HEAD"
            || inner == "FETCH_HEAD"
            || inner == "ORIG_HEAD"
            || inner.starts_with("refs/");
    }
    true
}

/// Starts watching `repo_path` (replacing a running watch).
pub fn watch(app: &AppHandle, state: &WatchState, repo_path: String) -> Result<(), String> {
    let (tx, rx) = mpsc::channel::<()>();

    // Event handler factory: clones the sender so a fallback attempt gets a
    // fresh handler (the first one has already consumed its sender).
    let handler = |tx: mpsc::Sender<()>| {
        move |res: Result<notify::Event, notify::Error>| {
            if let Ok(event) = res {
                if event.paths.iter().any(|p| is_relevant(p)) {
                    // A full channel/closed receiver is fine — a refresh is
                    // already running, or the watch has been replaced.
                    let _ = tx.send(());
                }
            }
        }
    };

    let path = Path::new(&repo_path);

    // 1) Native watcher (inotify/FSEvents/ReadDirectoryChangesW).
    let native: Option<Box<dyn Watcher + Send>> = notify::recommended_watcher(handler(tx.clone()))
        .ok()
        .and_then(|mut w| {
            w.watch(path, RecursiveMode::Recursive).ok()?;
            Some(Box::new(w) as Box<dyn Watcher + Send>)
        });

    // 2) Fallback: PollWatcher — with an exhausted inotify limit (realistic for
    //    the large-repo target) or on network drives the native watcher
    //    delivers no events. Slower 4 s polling beats NO auto refresh at all
    //    (the user would otherwise only have the 30 s poll).
    let watcher: Box<dyn Watcher + Send> = match native {
        Some(w) => w,
        None => {
            let cfg = Config::default().with_poll_interval(Duration::from_secs(4));
            let mut w = PollWatcher::new(handler(tx.clone()), cfg)
                .map_err(|e| format!("could not create watcher: {e}"))?;
            w.watch(path, RecursiveMode::Recursive)
                .map_err(|e| format!("could not watch directory: {e}"))?;
            tracing::warn!(path = %repo_path, "native file watcher unavailable — falling back to polling (4 s)");
            Box::new(w)
        }
    };

    // Debounce thread: wait for the first signal, swallow a 400 ms trailing
    // edge, then send exactly ONE event to the frontend.
    let app = app.clone();
    let emitted_path = repo_path.clone();
    std::thread::spawn(move || {
        while rx.recv().is_ok() {
            let deadline = Instant::now() + Duration::from_millis(400);
            loop {
                let rest = deadline.saturating_duration_since(Instant::now());
                if rest.is_zero() || rx.recv_timeout(rest).is_err() {
                    break;
                }
            }
            if app.emit("repo-changed", &emitted_path).is_err() {
                break;
            }
        }
        // Channel closed -> watcher was replaced/removed -> the thread ends.
    });

    *state.0.lock().unwrap_or_else(|p| p.into_inner()) = Some(ActiveWatch { _watcher: watcher });
    tracing::info!(path = %repo_path, "file watcher active");
    Ok(())
}

/// Stops the running watch (e.g. when closing the repo).
pub fn unwatch(state: &WatchState) {
    *state.0.lock().unwrap_or_else(|p| p.into_inner()) = None;
}

#[cfg(test)]
mod tests {
    use super::is_relevant;
    use std::path::Path;

    #[test]
    fn git_internals_are_filtered() {
        assert!(!is_relevant(Path::new(r"C:\repo\.git\index.lock")));
        assert!(!is_relevant(Path::new(r"C:\repo\.git\objects\ab\cdef")));
        assert!(!is_relevant(Path::new("/repo/.git/COMMIT_EDITMSG")));
        assert!(is_relevant(Path::new(r"C:\repo\.git\HEAD")));
        assert!(is_relevant(Path::new(r"C:\repo\.git\index")));
        assert!(is_relevant(Path::new(r"C:\repo\.git\refs\heads\main")));
        assert!(is_relevant(Path::new("/repo/.git/packed-refs")));
        assert!(is_relevant(Path::new(r"C:\repo\src\main.rs")));
        // ".gitignore" contains ".git" but must not count as internals
        assert!(is_relevant(Path::new(r"C:\repo\.gitignore")));
    }
}
