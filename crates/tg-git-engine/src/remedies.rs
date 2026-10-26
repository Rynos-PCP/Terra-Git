//! Ways out of common errors that need the backend: the
//! actions behind the buttons of the error toast.

use std::path::Path;
use std::time::{Duration, SystemTime};

use git2::Repository;
use tg_domain::GitProgress;

use crate::cancel::CancelToken;
use crate::error::{open_error, GitEngineError, Result};
use crate::sidecar;

/// A lock younger than this may belong to a git process that is still
/// working — it is not removed.
const LOCK_MIN_AGE: Duration = Duration::from_secs(5);

/// The lock to remove: the one git named in its message (`named`) — a
/// submodule's or a linked worktree's — when it is an `index.lock` inside this
/// repository's git directory; else `.git/index.lock`.
fn lock_to_remove(repo: &Repository, named: Option<&Path>) -> Result<std::path::PathBuf> {
    let Some(named) = named else {
        return Ok(repo.path().join("index.lock"));
    };
    let inside = match (
        named.parent().map(std::fs::canonicalize),
        std::fs::canonicalize(repo.commondir()),
    ) {
        (Some(Ok(dir)), Ok(git_dir)) => dir.starts_with(git_dir),
        _ => false,
    };
    if named.file_name().is_some_and(|n| n == "index.lock") && inside {
        Ok(named.to_path_buf())
    } else {
        Err(GitEngineError::InvalidOperation(format!(
            "Not a lock file of this repository: {}",
            named.display()
        )))
    }
}

/// Removes a leftover `index.lock` (`index_locked`) — `named` is the one git's
/// message named, if any (see [`lock_to_remove`]). Refuses a lock written
/// moments ago; the app layer additionally refuses while any git program runs.
/// `Ok(false)` when there is no lock (any more).
pub fn remove_index_lock(path: &Path, named: Option<&Path>) -> Result<bool> {
    let repo = Repository::discover(path).map_err(|e| open_error(path, e))?;
    let lock = lock_to_remove(&repo, named)?;
    let Ok(meta) = std::fs::metadata(&lock) else {
        return Ok(false);
    };
    let age = meta
        .modified()
        .ok()
        .and_then(|m| SystemTime::now().duration_since(m).ok())
        .unwrap_or(LOCK_MIN_AGE);
    if age < LOCK_MIN_AGE {
        return Err(GitEngineError::InvalidOperation(
            "The lock file was written just now — a git process is still working. \
             Wait a moment and try again."
                .into(),
        ));
    }
    std::fs::remove_file(&lock)?;
    Ok(true)
}

/// The folder git checks the owner of: the nearest one at or above `path` that
/// holds a `.git`. Trusting just an opened subfolder would never match, and
/// the retry would end in the same error again.
fn repository_root(path: &Path) -> Option<&Path> {
    path.ancestors().find(|dir| dir.join(".git").exists())
}

/// Marks exactly the repository at `path` as safe for git (`unsafe_repository`):
/// `git config --global --add safe.directory <root>`, unless it is listed
/// already. Only an absolute, existing folder inside a repository — never a
/// wildcard (`*` would switch git's ownership check off everywhere). Runs
/// outside the repository — git would refuse it there.
pub fn trust_directory(path: &Path) -> Result<()> {
    if !path.is_absolute() || !path.is_dir() {
        return Err(GitEngineError::InvalidOperation(format!(
            "Not a folder: {}",
            path.display()
        )));
    }
    let root = repository_root(path)
        .ok_or_else(|| GitEngineError::NotARepository(path.display().to_string()))?;
    let entry = root.to_string_lossy().replace('\\', "/");
    let outside = std::env::temp_dir();
    let listed = sidecar::run_git(
        &outside,
        &["config", "--global", "--get-all", "safe.directory"],
    )
    .unwrap_or_default();
    if listed.lines().any(|l| l.trim() == entry) {
        return Ok(());
    }
    sidecar::run_git(
        &outside,
        &["config", "--global", "--add", "safe.directory", &entry],
    )?;
    Ok(())
}

/// Removes the upstream of the current branch (`upstream_gone`): the remote
/// branch it followed is gone.
pub fn unset_upstream(path: &Path) -> Result<()> {
    sidecar::run_git(path, &["branch", "--unset-upstream"])?;
    Ok(())
}

/// Fetches the full history of a shallow clone (`shallow_rejected`), with
/// progress — the push that was rejected can follow.
pub fn fetch_unshallow(
    path: &Path,
    cancel: &CancelToken,
    on: &mut dyn FnMut(GitProgress),
) -> Result<String> {
    sidecar::fetch_unshallow_streaming(path, cancel, on)
}
