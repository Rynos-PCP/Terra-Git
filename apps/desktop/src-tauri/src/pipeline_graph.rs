//! Pipeline graph: pure types + parsers for the runner metadata
//! (gitlab-ci-local --list-csv, act -l). NO YAML parsing of our own —
//! include/extends is resolved by the runner (delegation principle).

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Pending,
    Running,
    Success,
    Failed,
    Skipped,
    Canceled,
    /// Reserved for a frontend fallback (e.g. before the first event); not
    /// constructed from Rust at the moment.
    #[allow(dead_code)]
    Unknown,
}

/// A discovered CI configuration file (repo-relative path, `/`-separated).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineConfig {
    pub path: String,
    /// "gitlab" | "github"
    pub provider: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineJobNode {
    pub name: String,
    pub stage: String,
    pub needs: Vec<String>,
    pub when: String,
    pub allow_failure: bool,
    /// Differing display name (act's "Job name" column): act logs lines with
    /// this name instead of the job id — needed for log attribution.
    /// None for GitLab (and when job name == job id).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineGraph {
    pub provider: String,
    pub config_file: String,
    /// Stages in execution/display order (first occurrence in the CSV — a
    /// documented approximation).
    pub stages: Vec<String>,
    pub jobs: Vec<PipelineJobNode>,
}

/// Live event of a run (Tauri channel).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum PipelineEvent {
    Line { job: Option<String>, line: String },
    Status { job: String, status: JobStatus },
}

/// gitlab-ci-local --list-csv: `name;stage;when;allowFailure;environment;needs`.
/// The needs cell is parsed tolerantly: `[a, b]` or `a,b` or empty.
pub fn parse_gitlab_csv(output: &str) -> (Vec<String>, Vec<PipelineJobNode>) {
    let mut stages: Vec<String> = Vec::new();
    let mut jobs = Vec::new();
    for (i, line) in output.lines().enumerate() {
        let line = line.trim();
        if i == 0 || line.is_empty() {
            continue; // Header
        }
        let f: Vec<&str> = line.split(';').collect();
        let name = f.first().map(|s| s.trim()).unwrap_or("");
        if name.is_empty() {
            continue;
        }
        let stage = f.get(1).map(|s| s.trim()).unwrap_or("").to_string();
        if !stage.is_empty() && !stages.iter().any(|s| s == &stage) {
            stages.push(stage.clone());
        }
        let needs = f
            .get(5)
            .map(|c| c.trim().trim_start_matches('[').trim_end_matches(']'))
            .unwrap_or("")
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        jobs.push(PipelineJobNode {
            name: name.to_string(),
            stage,
            needs,
            when: f.get(2).map(|s| s.trim()).unwrap_or("").to_string(),
            allow_failure: f.get(3).map(|s| s.trim()) == Some("true"),
            display_name: None,
        });
    }
    (stages, jobs)
}

/// act -l: whitespace table `Stage  Job ID  Job name  Workflow name
/// Workflow file  Events`; act's "stage" (topological level, a number) becomes
/// the pseudo stage. No needs edges (v1).
///
/// The columns are purely whitespace-separated, and "Job name" and "Workflow
/// name" may contain spaces themselves — only column 0 (stage) and 1 (job id)
/// are SAFELY parseable. The display name ("Job name") is captured
/// heuristically: the last token ending in `.yml`/`.yaml` counts as the
/// workflow file, the token before it as the workflow name, everything in
/// between as the job name. Documented limit: a MULTI-PART workflow name
/// distorts the display name (its leading tokens end up in the job name) — the
/// safe columns (job id/stage), and therefore graph and events, stay unaffected.
pub fn parse_act_table(output: &str) -> (Vec<String>, Vec<PipelineJobNode>) {
    let mut stages: Vec<String> = Vec::new();
    let mut jobs = Vec::new();
    for (i, line) in output.lines().enumerate() {
        let line = line.trim();
        if i == 0 || line.is_empty() {
            continue; // Header
        }
        let f: Vec<&str> = line.split_whitespace().collect();
        let (Some(stage), Some(id)) = (f.first(), f.get(1)) else {
            continue;
        };
        let stage = stage.to_string();
        if !stages.iter().any(|s| s == &stage) {
            stages.push(stage.clone());
        }
        // Heuristic for the display name (see the doc comment above).
        let rest = &f[2..];
        let display_name = rest
            .iter()
            .rposition(|t| t.ends_with(".yml") || t.ends_with(".yaml"))
            .and_then(|wf| match wf {
                0 => None,
                1 => Some(rest[0].to_string()),
                _ => Some(rest[..wf - 1].join(" ")),
            })
            .filter(|d| !d.is_empty() && d != id);
        jobs.push(PipelineJobNode {
            name: id.to_string(),
            stage,
            needs: Vec::new(),
            when: String::new(),
            allow_failure: false,
            display_name,
        });
    }
    (stages, jobs)
}

/// Removes ANSI CSI sequences (colors/cursor) from a runner line.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for n in chars.by_ref() {
                    if ('@'..='~').contains(&n) {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// Attributes an (ANSI-cleaned) runner line to a known job.
/// Two forms: gitlab-ci-local prefixes job log lines BARE with the job name;
/// act uses the bracket form `[Workflow/Job] …` (job = display name). The list
/// must be sorted LONGEST NAME FIRST (otherwise "build" matches before
/// "build-image").
pub fn attribute_line<'a>(jobs_longest_first: &'a [String], line: &str) -> Option<&'a str> {
    let l = line.trim_start();
    // act bracket form: take the content up to ']', split at the LAST '/'
    // (workflow names may contain '/' themselves) and match the trailing part
    // exactly against the known names.
    if let Some(rest) = l.strip_prefix('[') {
        if let Some(end) = rest.find(']') {
            let inner = &rest[..end];
            let cand = inner.rsplit('/').next().unwrap_or(inner).trim();
            if let Some(j) = jobs_longest_first.iter().find(|j| j.as_str() == cand) {
                return Some(j);
            }
        }
    }
    for j in jobs_longest_first {
        if let Some(rest) = l.strip_prefix(j.as_str()) {
            if rest.is_empty() || rest.starts_with([' ', '\t', '>', ':', '$', '|']) {
                return Some(j);
            }
        }
    }
    None
}

/// Deterministic status finalization after the process ends. The exit code is
/// ground truth; intermediate status from the output is an approximation.
/// `started` = jobs whose prefix appeared in the output at least once.
pub fn finalize_statuses(
    targeted: &[String],
    started: &std::collections::HashSet<String>,
    canceled: bool,
    exit: i32,
) -> Vec<(String, JobStatus)> {
    targeted
        .iter()
        .map(|j| {
            let s = if !started.contains(j) {
                JobStatus::Skipped
            } else if canceled {
                JobStatus::Canceled
            } else if exit == 0 {
                JobStatus::Success
            } else {
                JobStatus::Failed
            };
            (j.clone(), s)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gitlab_csv_with_needs_when_allowfailure_and_stage_order() {
        let csv = "name;stage;when;allowFailure;environment;needs\n\
                   build;build;on_success;false;;\n\
                   lint;check;on_success;true;;\n\
                   test;check;on_success;false;;[build]\n\
                   deploy;ship;manual;false;prod;build, test\n";
        let (stages, jobs) = parse_gitlab_csv(csv);
        assert_eq!(stages, vec!["build", "check", "ship"]);
        assert_eq!(jobs.len(), 4);
        assert_eq!(jobs[0].needs, Vec::<String>::new());
        assert!(jobs[1].allow_failure);
        assert_eq!(jobs[2].needs, vec!["build"]);
        assert_eq!(jobs[3].needs, vec!["build", "test"]);
        assert_eq!(jobs[3].when, "manual");
        assert_eq!(jobs[3].stage, "ship");
    }

    #[test]
    fn act_table_levels_as_pseudo_stages() {
        let t = "Stage  Job ID  Job name  Workflow name  Workflow file  Events\n\
                 0      build   build     CI             ci.yml         push\n\
                 1      test    test      CI             ci.yml         push\n";
        let (stages, jobs) = parse_act_table(t);
        assert_eq!(stages, vec!["0", "1"]);
        assert_eq!(jobs.len(), 2);
        assert_eq!(jobs[1].name, "test");
        assert!(jobs[1].needs.is_empty());
        // Job name == job id -> no separate display name.
        assert_eq!(jobs[0].display_name, None);
    }

    #[test]
    fn act_table_captures_differing_display_name() {
        // act logs lines with the DISPLAY NAME ("Job name"), not the job id —
        // the parser has to capture that column heuristically (spaces included).
        let t = "Stage  Job ID  Job name    Workflow name  Workflow file  Events\n\
                 0      build   My Build  CI             ci.yml         push\n\
                 1      test    Checking     My Workflow  ci.yml         push\n";
        let (_, jobs) = parse_act_table(t);
        assert_eq!(jobs[0].name, "build");
        assert_eq!(jobs[0].display_name.as_deref(), Some("My Build"));
        // Documented limit: multi-part workflow names distort the display name
        // (everything but one token before the workflow file is attributed to
        // the job name) — the safe columns 0/1 stay correct.
        assert_eq!(jobs[1].name, "test");
        assert!(jobs[1]
            .display_name
            .as_deref()
            .unwrap_or("test")
            .starts_with("Checking"));
    }

    #[test]
    fn strip_ansi_removes_color_codes() {
        assert_eq!(
            strip_ansi("\u{1b}[32mbuild\u{1b}[0m $ echo hi"),
            "build $ echo hi"
        );
        assert_eq!(strip_ansi("plain"), "plain");
    }

    #[test]
    fn line_attribution_longest_prefix_first() {
        // Longest name first so "build-image" does not match as "build".
        let jobs: Vec<String> = vec!["build-image".into(), "build".into()];
        assert_eq!(
            attribute_line(&jobs, "build-image $ docker build ."),
            Some("build-image")
        );
        assert_eq!(attribute_line(&jobs, "build > compiling"), Some("build"));
        assert_eq!(attribute_line(&jobs, "  build\tfoo"), Some("build"));
        // A name as a plain substring of another word must NOT match.
        assert_eq!(attribute_line(&jobs, "builder failed"), None);
        assert_eq!(attribute_line(&jobs, "starting pipeline"), None);
    }

    #[test]
    fn line_attribution_act_bracket_form() {
        // act prefixes log lines as "[Workflow/Job] …" — content up to ']',
        // split at the LAST '/' (workflow names may contain '/' themselves).
        let jobs: Vec<String> = vec!["build".into()];
        assert_eq!(attribute_line(&jobs, "[CI/build] | echo hi"), Some("build"));
        assert_eq!(
            attribute_line(&jobs, "[My/Workflow/build]   Job started"),
            Some("build")
        );
        // Unknown job in the brackets -> no attribution.
        assert_eq!(attribute_line(&jobs, "[CI/deploy] | ship"), None);
        // Brackets without a closing ']' -> no attribution.
        assert_eq!(attribute_line(&jobs, "[CI/build incomplete"), None);
    }

    #[test]
    fn finalize_statuses_exit_and_cancel() {
        use std::collections::HashSet;
        let targeted: Vec<String> = vec!["a".into(), "b".into(), "c".into()];
        let started: HashSet<String> = ["a", "b"].into_iter().map(String::from).collect();
        // Exit 0: started -> success, never seen -> skipped (rules/manual).
        let fin = finalize_statuses(&targeted, &started, false, 0);
        assert_eq!(fin[0].1, JobStatus::Success);
        assert_eq!(fin[2].1, JobStatus::Skipped);
        // Exit != 0: started -> failed, never seen -> skipped.
        let fin = finalize_statuses(&targeted, &started, false, 1);
        assert_eq!(fin[1].1, JobStatus::Failed);
        assert_eq!(fin[2].1, JobStatus::Skipped);
        // Cancelled: started -> canceled, never seen -> skipped.
        let fin = finalize_statuses(&targeted, &started, true, 130);
        assert_eq!(fin[0].1, JobStatus::Canceled);
        assert_eq!(fin[2].1, JobStatus::Skipped);
    }
}
