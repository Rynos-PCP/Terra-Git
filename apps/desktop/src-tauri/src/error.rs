//! Error mapping at the command boundary: engine errors are translated into a
//! serializable, stable format for the frontend.

use serde::Serialize;
use tg_git_engine::error::GitEngineError;

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandError {
    /// Stable, machine-readable code (e.g. `not_a_repository`).
    pub code: String,
    /// Short human-readable message (English; the frontend shows its own
    /// translation for known codes).
    pub message: String,
    /// git's raw output behind the error, cleaned up and capped for display.
    /// The full text goes to the log.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl CommandError {
    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            code: "internal".into(),
            message: message.into(),
            detail: None,
        }
    }
}

/// At most this many lines of git output travel to the frontend …
const DETAIL_MAX_LINES: usize = 200;
/// … and at most this many bytes.
const DETAIL_MAX_BYTES: usize = 16 * 1024;
/// Of a long output, this many first lines stay — the heading of a list git
/// prints ("… would be overwritten by merge:") is there.
const DETAIL_HEAD_LINES: usize = 20;
/// The log keeps at most this much of git's raw output (its end).
const LOG_MAX_BYTES: usize = 256 * 1024;

/// Makes git's raw output readable: `\r`-separated progress updates collapse
/// to their last state, progress-only lines ("Receiving objects: 45% …") go,
/// empty lines at the edges go. Long output keeps its first lines and its END
/// — that is where git puts the actual error — with a note about what was left
/// out in between.
pub fn clean_detail(raw: &str) -> String {
    let lines: Vec<&str> = raw
        .split('\n')
        .map(|l| {
            l.rsplit('\r')
                .find(|seg| !seg.trim().is_empty())
                .unwrap_or("")
        })
        .map(str::trim_end)
        .filter(|l| !is_progress_line(l))
        .collect();
    let start = lines
        .iter()
        .position(|l| !l.trim().is_empty())
        .unwrap_or(lines.len());
    let end = lines
        .iter()
        .rposition(|l| !l.trim().is_empty())
        .map_or(start, |i| i + 1);
    let lines = &lines[start..end];

    let total: usize = lines.iter().map(|l| l.len() + 1).sum();
    if lines.len() <= DETAIL_MAX_LINES && total <= DETAIL_MAX_BYTES {
        return lines.join("\n");
    }
    let mut bytes = 0;
    let mut head: Vec<&str> = Vec::new();
    for line in lines.iter().take(DETAIL_HEAD_LINES) {
        if bytes + line.len() + 1 > DETAIL_MAX_BYTES / 4 {
            break;
        }
        bytes += line.len() + 1;
        head.push(line);
    }
    let mut tail: Vec<&str> = Vec::new();
    for line in lines[head.len()..].iter().rev() {
        if head.len() + tail.len() == DETAIL_MAX_LINES || bytes + line.len() + 1 > DETAIL_MAX_BYTES
        {
            break;
        }
        bytes += line.len() + 1;
        tail.push(line);
    }
    tail.reverse();
    let omitted = lines.len() - head.len() - tail.len();
    let mut out = head.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(&format!(
        "[… {omitted} lines omitted — the full output is in the log]\n"
    ));
    out.push_str(&tail.join("\n"));
    out
}

/// git's raw output for the log: at most [`LOG_MAX_BYTES`] (its end), every
/// line marked as part of this one entry — a server's sideband text can
/// neither fill the log nor pose as log entries of its own.
fn log_quote(raw: &str) -> String {
    let mut start = raw.len().saturating_sub(LOG_MAX_BYTES);
    while !raw.is_char_boundary(start) {
        start += 1;
    }
    raw[start..]
        .split(['\n', '\r'])
        .filter(|l| !l.trim().is_empty())
        .map(|l| format!("\n    | {l}"))
        .collect()
}

/// git's transfer progress ("Receiving objects:  45% (450/1000)") — noise in
/// an error report.
fn is_progress_line(line: &str) -> bool {
    let l = line
        .trim_start()
        .trim_start_matches("remote: ")
        .to_ascii_lowercase();
    [
        "receiving objects:",
        "resolving deltas:",
        "compressing objects:",
        "counting objects:",
        "writing objects:",
        "enumerating objects:",
        "updating files:",
    ]
    .iter()
    .any(|p| l.starts_with(p))
}

impl From<GitEngineError> for CommandError {
    fn from(err: GitEngineError) -> Self {
        let detail = err.detail().map(clean_detail).filter(|d| !d.is_empty());
        if let Some(raw) = err.detail() {
            tracing::warn!(code = err.code(), "git failed:{}", log_quote(raw));
        }
        let message = match &err {
            // The raw output is the detail now; the message stays short.
            GitEngineError::Sidecar { .. } => "git command failed".to_string(),
            other => other.to_string(),
        };
        Self {
            code: err.code().into(),
            message,
            detail,
        }
    }
}

impl From<tg_providers::ProviderError> for CommandError {
    fn from(err: tg_providers::ProviderError) -> Self {
        Self {
            code: err.code().into(),
            message: err.to_string(),
            detail: None,
        }
    }
}

impl From<tg_auth::AuthError> for CommandError {
    fn from(err: tg_auth::AuthError) -> Self {
        Self {
            code: "keychain".into(),
            message: err.to_string(),
            detail: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_detail_collapses_progress_and_keeps_the_error() {
        let raw = "remote: Enumerating objects: 5, done.\nReceiving objects:  10% (1/10)\rReceiving objects: 100% (10/10), done.\n\nTo origin\n ! [rejected]        main -> main (fetch first)\nerror: failed to push some refs\n";
        assert_eq!(
            clean_detail(raw),
            "To origin\n ! [rejected]        main -> main (fetch first)\nerror: failed to push some refs"
        );
    }

    #[test]
    fn clean_detail_keeps_the_start_and_the_end_of_long_output() {
        let raw: String = (0..500).map(|i| format!("line {i}\n")).collect();
        let d = clean_detail(&raw);
        assert!(d.starts_with("line 0\n"), "{d}");
        assert!(d.contains("line 19\n[… 300 lines omitted"), "{d}");
        assert!(d.ends_with("line 499"));
        assert_eq!(d.lines().count(), DETAIL_MAX_LINES + 1);
    }

    #[test]
    fn the_log_entry_is_bounded_and_cannot_forge_entries() {
        let forged = "2026-10-25T10:00:00Z  INFO tg_app: all good\n";
        let q = log_quote(forged);
        assert_eq!(q, "\n    | 2026-10-25T10:00:00Z  INFO tg_app: all good");
        let huge = "y".repeat(LOG_MAX_BYTES * 3);
        assert!(log_quote(&huge).len() <= LOG_MAX_BYTES + 16);
    }

    #[test]
    fn clean_detail_caps_the_bytes() {
        let raw: String = (0..50).map(|_| format!("{}\n", "x".repeat(1000))).collect();
        assert!(clean_detail(&raw).len() <= DETAIL_MAX_BYTES + 100);
    }

    #[test]
    fn sidecar_errors_carry_the_output_as_detail() {
        let e: CommandError = GitEngineError::Sidecar {
            message: "fatal: Unable to create '.git/index.lock': File exists.".into(),
        }
        .into();
        // A known local cause gets its code; the output stays.
        assert_eq!(e.code, "index_locked");
        assert_eq!(e.message, "git command failed");
        assert_eq!(
            e.detail.as_deref(),
            Some("fatal: Unable to create '.git/index.lock': File exists.")
        );
    }

    #[test]
    fn classified_remote_errors_keep_the_raw_output() {
        let e: CommandError = GitEngineError::Remote {
            code: "non_fast_forward",
            message: "Push rejected".into(),
            raw: " ! [rejected] main -> main (non-fast-forward)".into(),
        }
        .into();
        assert_eq!(e.message, "Push rejected");
        assert_eq!(
            e.detail.as_deref(),
            Some(" ! [rejected] main -> main (non-fast-forward)")
        );
    }

    /// The quoted snake_case literal right after `marker` (whitespace allowed),
    /// for every occurrence in `src`.
    fn codes_after(src: &str, marker: &str) -> Vec<String> {
        src.match_indices(marker)
            .filter_map(|(i, _)| {
                let rest = src[i + marker.len()..].trim_start();
                let rest = rest.strip_prefix('"')?;
                let end = rest.find('"')?;
                let code = &rest[..end];
                let snake = !code.is_empty()
                    && code.starts_with(|c: char| c.is_ascii_lowercase())
                    && code.chars().all(|c| c.is_ascii_lowercase() || c == '_');
                snake.then(|| code.to_string())
            })
            .collect()
    }

    /// Every error code the backend can emit is listed in the frontend's
    /// `errorCodes.ts` — that list drives the catalog and registry checks
    /// there. Scans the sources for the ways codes are written:
    /// `code: "…"` anywhere, the remote classifier's `Some(("…"`, `ssh_err("…"`,
    /// the pipeline's `rejected("…"` and the `=> "…"` arms of the `code()`
    /// functions.
    #[test]
    fn every_backend_error_code_is_known_to_the_frontend() {
        use std::collections::BTreeSet;
        use std::path::Path;

        let app = Path::new(env!("CARGO_MANIFEST_DIR"));
        let list = std::fs::read_to_string(app.join("../src/lib/errorCodes.ts")).unwrap();
        let known: BTreeSet<String> = codes_after(&list, "\n  ").into_iter().collect();
        assert!(known.len() > 50, "errorCodes.ts not parsed: {known:?}");

        let crates = app.join("../../../crates");
        let sources = [
            app.join("src"),
            crates.join("tg-git-engine/src"),
            crates.join("tg-providers/src"),
        ];
        let mut emitted = BTreeSet::new();
        for dir in &sources {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                    continue;
                }
                let src = std::fs::read_to_string(&path).unwrap();
                let name = path.file_name().unwrap().to_string_lossy().into_owned();
                let mut markers = vec!["code: "];
                match name.as_str() {
                    "sidecar.rs" => markers.push("Some(("),
                    "ssh.rs" => markers.push("ssh_err("),
                    "pipeline.rs" => markers.push("rejected("),
                    "error.rs" => markers.push("=> "),
                    _ => {}
                }
                for marker in markers {
                    emitted.extend(codes_after(&src, marker));
                }
            }
        }
        // The scan itself must work: one known code per marker kind.
        for probe in [
            "not_a_directory", // code: (commands.rs)
            "host_key",        // Some(( (sidecar.rs)
            "ssh_no_home",     // ssh_err( (ssh.rs)
            "run_active",      // rejected( (pipeline.rs)
            "sidecar_failed",  // => (engine error.rs)
            "rate_limited",    // => (providers error.rs)
            "discard_store_full",
        ] {
            assert!(emitted.contains(probe), "scan missed {probe}: {emitted:?}");
        }
        let missing: Vec<&String> = emitted.difference(&known).collect();
        assert!(
            missing.is_empty(),
            "error codes emitted by the backend but missing in errorCodes.ts: {missing:?}"
        );
    }

    #[test]
    fn other_errors_have_no_detail() {
        let e: CommandError = GitEngineError::NothingToAmend.into();
        assert!(e.detail.is_none());
    }
}
