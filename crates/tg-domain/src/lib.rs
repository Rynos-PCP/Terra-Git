//! terra-git domain types.
//!
//! These types are the shared language between the git engine, the providers
//! and the frontend. They deliberately carry no git2/gix types so the engine
//! stays replaceable (hybrid strategy: gix for reads, git2 for writes).
//! Serialization is camelCase to match the TypeScript frontend.

use serde::{Deserialize, Serialize};

/// Basic information about an opened repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoInfo {
    /// Absolute path to the repository workdir.
    pub path: String,
    /// Display name (directory name).
    pub name: String,
    /// Current branch name, `None` on detached HEAD or in an empty repo.
    pub current_branch: Option<String>,
    pub head_detached: bool,
    /// `true` if the repo has no commit yet (unborn HEAD).
    pub is_empty: bool,
    /// `true` if a commit graph exists (the basis for streaming the history).
    /// `false` for freshly cloned huge repos — the UI then shows a
    /// "preparing history" hint until the background write has finished.
    pub history_prepared: bool,
}

/// Kind of change to a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    Typechange,
    Conflicted,
    Untracked,
}

/// Line-ending style of a text content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EolStyle {
    /// `\n` only.
    Lf,
    /// `\r\n` only.
    Crlf,
    /// Both mixed — usually the result of a half-finished conversion.
    Mixed,
    /// No line breaks at all.
    None,
}

/// Why does Git report a file as changed even though the diff is empty?
///
/// Measured on 2026-07-21: in these cases `file_diff` returns `Some(FileDiff)`
/// with an empty `hunks` vector (not `None` — that means "clean").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UnchangedReason {
    /// Only the executable bit differs.
    ModeOnly,
    /// The content is byte-identical and matches what a checkout would write —
    /// the report comes from stale index information.
    Identical,
    /// The difference is purely in the line endings.
    EolOnly,
    /// No harmless cause found. Deliberately honest instead of guessing.
    Unknown,
}

/// Reason for a file reported as changed without a content diff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnchangedInfo {
    pub reason: UnchangedReason,
    /// Line endings in the repository (left diff side) — only for `EolOnly`.
    #[serde(default)]
    pub old_eol: Option<EolStyle>,
    /// Line endings in the working copy (right diff side) — only for `EolOnly`.
    #[serde(default)]
    pub new_eol: Option<EolStyle>,
    /// Line endings a checkout would write. If this differs from `new_eol`,
    /// Git reports the file even though the bytes are equal.
    #[serde(default)]
    pub expected_eol: Option<EolStyle>,
    /// Octal file mode before/after — only for `ModeOnly`.
    #[serde(default)]
    pub old_mode: Option<String>,
    #[serde(default)]
    pub new_mode: Option<String>,
}

/// A changed file in the status (staged or unstaged).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusEntry {
    /// Path relative to the repo root (on rename: the new path).
    pub path: String,
    /// Old path on rename, otherwise `None`.
    pub orig_path: Option<String>,
    pub kind: ChangeKind,
}

/// Line balance of a changed file (working tree + index against HEAD),
/// for the changes overview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileLineStats {
    /// Path relative to the repo root (on rename: the new path).
    pub path: String,
    pub added: u32,
    pub deleted: u32,
    /// Binary file: line counts are not meaningful (both 0).
    pub binary: bool,
}

/// A point on the welcome-screen vein: a commit on the HEAD line (peek_repo).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SketchCommit {
    /// Author timestamp as Unix seconds.
    pub time: i64,
    pub is_merge: bool,
    /// At least one tag points at this commit.
    pub has_tag: bool,
}

/// A local branch in the welcome-screen sketch: where it branches off the HEAD
/// line and how far ahead of it that branch is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SketchBranch {
    pub name: String,
    /// Index of the merge base inside the commit window (0 = newest commit);
    /// `None` if the branch point is older than the window.
    pub base_index: Option<u32>,
    /// Commits this branch is ahead of the HEAD line (capped).
    pub ahead: u32,
    /// Commit time of the branch tip (Unix seconds).
    pub tip_time: i64,
}

/// Repo sketch for the welcome-screen vein: HEAD line + local branches.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoSketch {
    /// The most recent commits on the HEAD line, newest first.
    pub commits: Vec<SketchCommit>,
    /// Local branches other than the HEAD branch, newest first (capped).
    pub branches: Vec<SketchBranch>,
}

/// Overall status of a repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoStatus {
    pub staged: Vec<StatusEntry>,
    pub unstaged: Vec<StatusEntry>,
    pub branch: Option<String>,
    /// Upstream reference (e.g. `origin/main`), if configured.
    pub upstream: Option<String>,
    /// Commits the local branch is ahead of its upstream.
    pub ahead: usize,
    /// Commits the local branch is behind its upstream.
    pub behind: usize,
    /// Multi-step operation in progress (merge/rebase/cherry-pick/revert).
    #[serde(default = "RepoOpState::clean")]
    pub op_state: RepoOpState,
}

impl RepoOpState {
    pub fn clean() -> Self {
        RepoOpState::Clean
    }
}

/// A commit in the history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitInfo {
    /// Full object id (hex).
    pub id: String,
    /// Abbreviated object id.
    pub short_id: String,
    pub summary: String,
    pub author_name: String,
    pub author_email: String,
    /// Author timestamp as Unix seconds.
    pub time: i64,
    pub parent_ids: Vec<String>,
}

/// A commit that has not been pushed yet (on no remote-tracking branch), for
/// the commit workshop. `body` = full message without the subject line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnpushedCommit {
    pub id: String,
    pub subject: String,
    pub body: String,
    pub author_name: String,
    pub author_email: String,
    /// Author timestamp as Unix seconds (like `CommitInfo::time`).
    pub time: i64,
    pub parent_ids: Vec<String>,
    pub is_head: bool,
    pub is_merge: bool,
}

/// A local or remote branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchInfo {
    pub name: String,
    pub is_head: bool,
    pub is_remote: bool,
    pub upstream: Option<String>,
    /// For remote branches the name without the remote prefix (e.g. `feature/x`
    /// instead of `origin/feature/x`); `None` for local branches.
    pub short_name: Option<String>,
    /// Commit OID of the branch tip (for labels in the history graph).
    #[serde(default)]
    pub target_id: Option<String>,
    /// A local branch whose configured upstream no longer exists on the remote
    /// (e.g. after a merge with "delete source branch"). Always `false` for
    /// remote branches. Only detectable after a pruning fetch.
    #[serde(default)]
    pub upstream_gone: bool,
}

/// Kind of a diff line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LineKind {
    Context,
    Addition,
    Deletion,
}

/// A line inside a diff hunk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffLine {
    pub kind: LineKind,
    pub old_lineno: Option<u32>,
    pub new_lineno: Option<u32>,
    pub content: String,
}

/// A hunk (contiguous block of changes) of a file diff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffHunk {
    /// The `@@ -a,b +c,d @@` header.
    pub header: String,
    pub lines: Vec<DiffLine>,
}

/// A stash entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StashInfo {
    /// Position in the stash stack (0 = newest).
    pub index: usize,
    pub message: String,
    pub id: String,
}

/// A tag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TagInfo {
    pub name: String,
    /// Object id of the target commit.
    pub target_id: String,
    /// Message for annotated tags.
    pub message: Option<String>,
    pub is_annotated: bool,
}

/// A configured remote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteInfo {
    pub name: String,
    pub url: String,
}

/// Progress of a remote operation (parsed from `git --progress` output).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitProgress {
    /// Normalized phase: receiving | resolving | compressing | counting
    /// | writing | enumerating | other (the frontend translates it).
    pub phase: String,
    /// Progress 0..=100 within the current phase.
    pub percent: u8,
}

/// Options for cloning (large-repo levers).
// No longer `Copy` since `branch` (String) was added — every caller takes the
// options by reference or moves them anyway.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloneOptions {
    /// Shallow clone: only the last N commits (`--depth N`).
    pub depth: Option<u32>,
    /// Partial clone without blobs (`--filter=blob:none`): full history, file
    /// contents are fetched on demand (needs server support).
    pub blobless: bool,
    /// Clone only this branch (single-branch); `None` = the remote default.
    /// `#[serde(default)]`: older frontend callers without the field stay valid.
    #[serde(default)]
    pub branch: Option<String>,
}

/// Hosting provider kind (two-layer principle, neutral model).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    Github,
    Gitlab,
    /// Gitea AND Forgejo (API-compatible; Forgejo is a Gitea fork).
    Gitea,
}

/// An account stored for a provider (the token lives in the OS keychain, NOT here).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAccount {
    /// Host name of the instance (e.g. "github.com", "gitlab.example.com").
    pub host: String,
    pub kind: ProviderKind,
    /// User name determined during validation.
    pub username: String,
    /// TLS certificate verification disabled (only for self-hosted instances
    /// with a self-signed certificate; the user confirms this explicitly).
    #[serde(default)]
    pub insecure_tls: bool,
}

/// A local SSH key (from ~/.ssh/*.pub).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshKey {
    /// File stem (e.g. "id_ed25519").
    pub name: String,
    /// "ssh-ed25519" | "ssh-rsa" | …
    pub key_type: String,
    pub comment: String,
    /// Full content of the .pub line (for copying/uploading).
    pub public_key: String,
    /// "SHA256:…".
    pub fingerprint: String,
}

/// Fingerprint of a host key (for the TOFU dialog).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshHostFingerprint {
    pub key_type: String,
    pub sha256: String,
}

/// Result of an ssh-keyscan for a host (TOFU).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannedHost {
    pub host: String,
    /// true = a (differing) known_hosts entry already exists -> MITM warning.
    pub changed: bool,
    pub fingerprints: Vec<SshHostFingerprint>,
    /// Raw ssh-keyscan output (exactly the known_hosts lines being confirmed).
    pub known_hosts_lines: String,
}

/// CI/pipeline status of a change request (provider-neutral).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CiStatus {
    Success,
    Failed,
    Running,
    Pending,
    Canceled,
    /// No CI configured, or the status could not be determined.
    Unknown,
}

/// Pull request (GitHub) or merge request (GitLab) — neutrally named.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeRequest {
    /// PR number or MR IID (instance-local, as shown in the web UI).
    pub number: u64,
    pub title: String,
    pub author: String,
    pub source_branch: String,
    pub target_branch: String,
    pub is_draft: bool,
    pub web_url: String,
    /// Unix seconds of the last update.
    pub updated_at: i64,
    /// CI status of the newest commit (Unknown = no CI / not determinable).
    pub ci_status: CiStatus,
}

/// Input data for creating a change request (PR/MR).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewChangeRequest {
    pub title: String,
    /// Description/body (empty = none).
    pub description: String,
    pub source_branch: String,
    pub target_branch: String,
    pub draft: bool,
}

/// Result of the change-request query for a repo (incl. context for the UI).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeRequestList {
    pub host: String,
    pub repo_path: String,
    pub kind: ProviderKind,
    pub items: Vec<ChangeRequest>,
}

/// Reset mode for undo actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResetMode {
    /// Keeps workdir + index (commit undo: changes stay staged).
    Soft,
    /// Also resets workdir/index (history rewrites; requires a clean tree).
    Hard,
}

/// An executable undo/redo action (stored as inverses of each other).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum UndoAction {
    /// Resets the tip of the CURRENT branch to `commit`.
    #[serde(rename_all = "camelCase")]
    ResetBranch {
        branch: String,
        commit: String,
        mode: ResetMode,
    },
    /// Recreates a deleted branch (without checking it out).
    #[serde(rename_all = "camelCase")]
    RecreateBranch { name: String, commit: String },
    /// Deletes a branch again (redo of a branch deletion).
    #[serde(rename_all = "camelCase")]
    DeleteBranch { name: String },
    /// Switches back to a branch.
    #[serde(rename_all = "camelCase")]
    Checkout { target: String },
    /// Restores a discarded stash from its commit.
    #[serde(rename_all = "camelCase")]
    RestoreStash { message: String, commit: String },
    /// Removes a stash by its commit id (redo of a stash drop).
    #[serde(rename_all = "camelCase")]
    DropStashByCommit { commit: String },
}

/// Entry in the multi-step undo stack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoEntry {
    /// Stable operation code for the UI translation (commit, amend, merge,
    /// rebase, rebaseInteractive, squash, cherryPick, revert, switchBranch,
    /// deleteBranch, stashDrop, restoreBackup).
    pub op: String,
    /// Optional parameter for display (e.g. a branch name).
    pub detail: Option<String>,
    /// Unix seconds.
    pub timestamp: i64,
    pub undo: UndoAction,
    pub redo: UndoAction,
}

/// Compact undo status for the UI (buttons/tooltips).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoStatus {
    /// Next undo entry (op + detail), if any.
    pub undo: Option<UndoEntry>,
    pub redo: Option<UndoEntry>,
    pub undo_count: usize,
    pub redo_count: usize,
}

/// An automatic backup (backup ref) taken before a history rewrite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupInfo {
    /// Full ref name (`refs/terra-git/backup/<op>-<unix>`).
    pub name: String,
    /// Triggering operation (squash, rebase, rebase-interactive, restore).
    pub op: String,
    /// Unix timestamp of the backup (seconds).
    pub timestamp: i64,
    /// Backed-up commit (the old HEAD before the rewrite).
    pub target_id: String,
    /// Subject line of the backed-up commit.
    pub subject: String,
}

/// Multi-step operation in progress (drives the UI banner and its actions).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RepoOpState {
    Clean,
    Merge,
    Rebase,
    Cherrypick,
    Revert,
    /// A running `git bisect` session.
    Bisect,
}

/// A line of the blame view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlameLine {
    /// 1-based line number.
    pub line_no: u32,
    pub commit_id: String,
    pub short_id: String,
    pub author: String,
    /// Author timestamp as Unix seconds.
    pub time: i64,
    pub content: String,
}

/// A git worktree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeInfo {
    pub path: String,
    pub branch: Option<String>,
    pub head_id: Option<String>,
    /// `true` for the repository's main worktree.
    pub is_main: bool,
}

/// A submodule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmoduleInfo {
    pub name: String,
    pub path: String,
    pub url: Option<String>,
}

/// Sparse-checkout state of a repo (large-repo lever).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SparseStatus {
    /// Is sparse checkout active (core.sparseCheckout=true)?
    pub enabled: bool,
    /// Active cone directories (empty when disabled).
    pub patterns: Vec<String>,
    /// Top-level directories of the HEAD tree (the choices offered by the UI).
    pub top_dirs: Vec<String>,
}

/// Image diff: both sides as data URLs (base64), where available.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageDiff {
    /// Old version (HEAD or index), as a data: URL.
    pub old_data_url: Option<String>,
    /// New version (index or workdir), as a data: URL.
    pub new_data_url: Option<String>,
}

/// Diff of a single file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileDiff {
    /// Path relative to the repo root (on rename: the new path).
    pub path: String,
    pub old_path: Option<String>,
    pub is_binary: bool,
    pub hunks: Vec<DiffHunk>,
    /// `true` if the engine truncated the diff (huge files) — this keeps
    /// unbounded amounts of data from crossing the IPC boundary.
    #[serde(default)]
    pub truncated: bool,
    /// Byte size of the old/new version (mainly for binaries without a text diff).
    #[serde(default)]
    pub old_size: Option<u64>,
    #[serde(default)]
    pub new_size: Option<u64>,
}

/// One step of an interactive-rebase plan (order = order of application,
/// oldest first). `action` is one of the git todo actions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RebaseStep {
    /// "pick" | "reword" | "squash" | "fixup" | "drop".
    pub action: String,
    /// Full or abbreviated commit hash.
    pub commit_id: String,
    /// New commit message — only set for "reword".
    #[serde(default)]
    pub message: Option<String>,
    /// New author as `Name <email>` — only set when the author should change.
    /// Triggers an amend (like "reword"), even without a new message.
    #[serde(default)]
    pub author: Option<String>,
}

/// A segment of a conflicted file for the conflict editor.
/// `kind == "context"`: undisputed text in `lines`.
/// `kind == "conflict"`: `ours`/`theirs` (and `base` in diff3 style) separated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictSegment {
    pub kind: String,
    #[serde(default)]
    pub lines: Vec<String>,
    #[serde(default)]
    pub ours: Vec<String>,
    #[serde(default)]
    pub theirs: Vec<String>,
    #[serde(default)]
    pub base: Option<Vec<String>>,
}

/// Context of the multi-step operation in progress, for the conflict workshop:
/// names both sides understandably (branch/commit instead of ours/theirs).
/// All fields except `kind` are best effort — missing data makes the UI show
/// generic texts instead of blocking the workshop.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpContext {
    pub kind: RepoOpState,
    /// Label of the "ours" side: on merge the current branch, on rebase the NEW
    /// BASE (onto) — there, ours is precisely not "yours".
    pub ours_label: Option<String>,
    /// Label of the "theirs" side: on merge the incoming branch, on rebase the
    /// commit of your branch currently being replayed, on cherry-pick/revert
    /// the source commit.
    pub theirs_label: Option<String>,
    /// Subject of the commit behind the theirs side (if determinable).
    pub theirs_summary: Option<String>,
    /// Rebase: current step (1-based) and total number of steps.
    pub step: Option<u32>,
    pub total: Option<u32>,
}

/// A conflicted file broken down for the in-app editor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictFile {
    pub file: String,
    pub segments: Vec<ConflictSegment>,
    /// Line ending of the original file: "lf" or "crlf" (for lossless saving).
    pub eol: String,
    /// `true` if conflict markers were actually found.
    pub has_conflicts: bool,
}
