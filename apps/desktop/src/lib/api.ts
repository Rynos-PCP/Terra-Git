// Typed IPC layer: mirrors the tg-domain types (camelCase via serde) and
// normalizes errors onto the CommandError format of the Rust edge.

import { Channel, invoke } from "@tauri-apps/api/core";

export interface RepoInfo {
  path: string;
  name: string;
  currentBranch: string | null;
  headDetached: boolean;
  isEmpty: boolean;
  /** Is a commit graph present? `false` for fresh huge clones — the UI then
   *  shows the preparing hint until the "history-prepared" event. */
  historyPrepared: boolean;
}

/** Entry of the recently-opened list (welcome screen + toolbar menu). */
export interface RecentRepo {
  path: string;
  /** Unix seconds of the last open; null for migrated legacy entries. */
  lastOpened: number | null;
  /** Pinned repos come first and never fall out of the list. */
  pinned: boolean;
}

/** Short portrait of a repo for the welcome screen: branch chip, dirty dot and
 *  the vein sketch (peek_repo). */
export interface RepoPeek {
  /** Current branch, null on detached HEAD or in an empty repo. */
  branch: string | null;
  /** Uncommitted changes present (staged OR unstaged). */
  dirty: boolean;
  /** The most recent commits on the HEAD line (newest first). */
  commits: { time: number; isMerge: boolean; hasTag: boolean }[];
  /** Local branches other than HEAD: branch point (merge-base index inside the
   *  window, null = older), ahead count and tip time. Newest first, capped. */
  branches: { name: string; baseIndex: number | null; ahead: number; tipTime: number }[];
}

export type ChangeKind =
  "added" | "modified" | "deleted" | "renamed" | "typechange" | "conflicted" | "untracked";

export interface StatusEntry {
  path: string;
  origPath: string | null;
  kind: ChangeKind;
}

export type RepoOpState = "clean" | "merge" | "rebase" | "cherrypick" | "revert" | "bisect";

export interface RepoStatus {
  staged: StatusEntry[];
  unstaged: StatusEntry[];
  branch: string | null;
  upstream: string | null;
  ahead: number;
  behind: number;
  opState: RepoOpState;
}

/** Line balance of a changed file (working tree + index against HEAD). */
export interface FileLineStats {
  path: string;
  added: number;
  deleted: number;
  /** Binary file: line counts are not meaningful (both 0). */
  binary: boolean;
}

export interface StashInfo {
  index: number;
  message: string;
  id: string;
}

export interface TagInfo {
  name: string;
  targetId: string;
  message: string | null;
  isAnnotated: boolean;
}

export interface RemoteInfo {
  name: string;
  url: string;
}

/** Automatic backup (backup ref) before a history rewrite. */
export interface BackupInfo {
  name: string;
  op: string;
  timestamp: number;
  targetId: string;
  subject: string;
}

// ---- Provider: accounts + change requests ----

export type ProviderKind = "github" | "gitlab" | "gitea";

export type CiStatus = "success" | "failed" | "running" | "pending" | "canceled" | "unknown";

export interface ProviderAccount {
  host: string;
  kind: ProviderKind;
  username: string;
  insecureTls: boolean;
}

/** Pull request (GitHub) or merge request (GitLab) — neutrally named. */
export interface ChangeRequest {
  number: number;
  title: string;
  author: string;
  sourceBranch: string;
  targetBranch: string;
  isDraft: boolean;
  webUrl: string;
  updatedAt: number;
  ciStatus: CiStatus;
}

export interface ChangeRequestList {
  host: string;
  repoPath: string;
  kind: ProviderKind;
  items: ChangeRequest[];
}

/** Progress of a remote operation (fetch/pull/push/clone). */
export interface GitProgress {
  phase: string;
  percent: number;
}

/** Creates a progress channel that forwards messages to `onProgress`. */
function progressChannel(onProgress?: (p: GitProgress) => void): Channel<GitProgress> {
  const ch = new Channel<GitProgress>();
  if (onProgress) ch.onmessage = onProgress;
  return ch;
}

/** Local pipeline testing: detected CI configuration + prerequisites. */
export interface PipelineInfo {
  provider: "gitlab" | "github" | null;
  configFile: string | null;
  /** Runner availability PER provider — gate/banner follow the CHOSEN config. */
  runnersInstalled: { gitlab: boolean; github: boolean };
  dockerRunning: boolean;
  /** Host tools the runner needs that are missing from PATH (e.g. rsync, bash). */
  missingTools: string[];
}

export type PipelineJobStatus =
  "pending" | "running" | "success" | "failed" | "skipped" | "canceled" | "unknown";

export interface PipelineConfig {
  path: string;
  provider: "gitlab" | "github";
}

export interface PipelineJobNode {
  name: string;
  stage: string;
  needs: string[];
  when: string;
  allowFailure: boolean;
  /** act: differing display name ("Job name") the runner logs under.
   *  Only set when it differs from the job id (serde skip_serializing_if). */
  displayName?: string;
}

export interface PipelineGraph {
  provider: string;
  configFile: string;
  stages: string[];
  jobs: PipelineJobNode[];
}

export type PipelineEvent =
  | { kind: "line"; job: string | null; line: string }
  | { kind: "status"; job: string; status: PipelineJobStatus };

/** State of the sparse checkout (cone mode). */
export interface SparseStatus {
  enabled: boolean;
  patterns: string[];
  topDirs: string[];
}

/** Clone scope: shallow depth and/or blobless (--filter=blob:none). */
export interface CloneOptions {
  depth: number | null;
  blobless: boolean;
  /** Clone only this branch (single-branch); null/omitted = the remote default. */
  branch?: string | null;
}

/** Entry in the multi-level undo stack (the action details stay in the backend). */
export interface UndoEntry {
  op: string;
  detail: string | null;
  timestamp: number;
}

export interface UndoStatus {
  undo: UndoEntry | null;
  redo: UndoEntry | null;
  undoCount: number;
  redoCount: number;
}

/** Input data for creating a change request. */
export interface NewChangeRequest {
  title: string;
  description: string;
  sourceBranch: string;
  targetBranch: string;
  draft: boolean;
}

export interface WorktreeInfo {
  path: string;
  branch: string | null;
  headId: string | null;
  isMain: boolean;
}

export interface SubmoduleInfo {
  name: string;
  path: string;
  url: string | null;
}

export interface BlameLine {
  lineNo: number;
  commitId: string;
  shortId: string;
  author: string;
  time: number;
  content: string;
}

export interface ImageDiff {
  oldDataUrl: string | null;
  newDataUrl: string | null;
}

export interface CommitInfo {
  id: string;
  shortId: string;
  summary: string;
  authorName: string;
  authorEmail: string;
  time: number;
  parentIds: string[];
}

export type RebaseAction = "pick" | "reword" | "squash" | "fixup" | "drop";

export interface RebaseStep {
  action: RebaseAction;
  commitId: string;
  /** New commit message — only set for "reword". */
  message?: string | null;
  /** New author ("Name <email>") — optional, independent of the action. */
  author?: string;
}

export interface UnpushedCommit {
  id: string;
  subject: string;
  body: string;
  authorName: string;
  authorEmail: string;
  /** Author timestamp as Unix seconds (like `CommitInfo.time`). */
  time: number;
  parentIds: string[];
  isHead: boolean;
  isMerge: boolean;
}

/** Context of the running multi-step operation (conflict workshop): names both
 *  sides understandably. All fields except `kind` are best effort. */
export interface OpContext {
  kind: RepoOpState;
  /** The "ours" side: on merge the current branch, on rebase the NEW base. */
  oursLabel: string | null;
  /** The "theirs" side: the incoming branch or the commit being applied. */
  theirsLabel: string | null;
  /** Subject of the commit behind the theirs side. */
  theirsSummary: string | null;
  /** Rebase: current step (1-based) and total count. */
  step: number | null;
  total: number | null;
}

export interface ConflictSegment {
  kind: "context" | "conflict";
  lines: string[];
  ours: string[];
  theirs: string[];
  base: string[] | null;
}

export interface ConflictFile {
  file: string;
  segments: ConflictSegment[];
  eol: "lf" | "crlf";
  hasConflicts: boolean;
}

export interface BranchInfo {
  name: string;
  isHead: boolean;
  isRemote: boolean;
  upstream: string | null;
  /** For remote branches the name without the remote prefix, otherwise null. */
  shortName: string | null;
  /** Commit OID of the branch tip (for labels in the history graph). */
  targetId: string | null;
  /** Local branch whose upstream was deleted on the remote. */
  upstreamGone: boolean;
}

export type LineKind = "context" | "addition" | "deletion";

export interface DiffLine {
  kind: LineKind;
  oldLineno: number | null;
  newLineno: number | null;
  content: string;
}

export interface DiffHunk {
  header: string;
  lines: DiffLine[];
}

export interface FileDiff {
  path: string;
  oldPath: string | null;
  isBinary: boolean;
  hunks: DiffHunk[];
  /** true when the engine truncated the diff (huge file). */
  truncated: boolean;
  /** Byte size of the old/new version (mainly for binaries). */
  oldSize?: number;
  newSize?: number;
}

export type EolStyle = "lf" | "crlf" | "mixed" | "none";

/** Why does a file count as changed even though the diff is empty? */
export type UnchangedReason = "modeOnly" | "identical" | "eolOnly" | "unknown";

export interface UnchangedInfo {
  reason: UnchangedReason;
  /** Line endings in the repository — only for "eolOnly". */
  oldEol?: EolStyle | null;
  /** Line endings in the working copy — only for "eolOnly". */
  newEol?: EolStyle | null;
  /** What a checkout would write; if it differs from newEol, that is the reason. */
  expectedEol?: EolStyle | null;
  /** Octal file mode — only for "modeOnly". */
  oldMode?: string | null;
  newMode?: string | null;
}

// ---- SSH key manager ----

/** A local SSH key (from ~/.ssh/*.pub). */
export interface SshKey {
  name: string;
  keyType: string;
  comment: string;
  publicKey: string;
  fingerprint: string;
}

/** Fingerprint of a host key (for the TOFU dialog). */
export interface SshHostFingerprint {
  keyType: string;
  sha256: string;
}

/** Result of an ssh-keyscan for a host (TOFU). */
export interface ScannedHost {
  host: string;
  /** true = a (differing) known_hosts entry already exists -> MITM warning. */
  changed: boolean;
  fingerprints: SshHostFingerprint[];
  knownHostsLines: string;
}

export interface CommandError {
  code: string;
  message: string;
}

/** Normalizes arbitrary invoke errors onto CommandError. */
function toCommandError(e: unknown): CommandError {
  if (e && typeof e === "object" && "message" in e && "code" in e) {
    return e as CommandError;
  }
  return { code: "unknown", message: String(e) };
}

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    throw toCommandError(e);
  }
}

export const api = {
  openRepository: (path: string) => call<RepoInfo>("open_repository", { path }),
  recentRepos: () => call<RecentRepo[]>("get_recent_repos"),
  removeRecent: (path: string) => call<void>("remove_recent_repo", { path }),
  setRecentPinned: (path: string, pinned: boolean) =>
    call<void>("set_recent_pinned", { path, pinned }),
  peekRepo: (path: string) => call<RepoPeek>("peek_repo", { path }),
  deleteRepo: (path: string) => call<void>("delete_repo", { path }),
  status: (path: string) => call<RepoStatus>("get_status", { path }),
  statusNumstat: (path: string) => call<FileLineStats[]>("status_numstat", { path }),
  log: (path: string, skip: number, limit: number) =>
    call<CommitInfo[]>("get_log", { path, skip, limit }),
  /** History across all branches (local + remote), tags and HEAD (the whole graph). */
  logAll: (path: string, skip: number, limit: number) =>
    call<CommitInfo[]>("get_log_all", { path, skip, limit }),
  explainUnchanged: (path: string, file: string, staged: boolean) =>
    call<UnchangedInfo>("explain_unchanged", { path, file, staged }),
  fileDiff: (path: string, file: string, staged: boolean) =>
    call<FileDiff | null>("get_file_diff", { path, file, staged }),
  commitDiff: (path: string, commitId: string) =>
    call<FileDiff[]>("get_commit_diff", { path, commitId }),
  /** Streams the commit diff file by file; returns the total number of files. */
  commitDiffStream: (
    path: string,
    commitId: string,
    maxFiles: number,
    onFile: (fd: FileDiff) => void,
  ) => {
    const channel = new Channel<FileDiff>();
    channel.onmessage = onFile;
    return call<number>("get_commit_diff_stream", { path, commitId, maxFiles, onFile: channel });
  },
  stage: (path: string, files: string[]) => call<void>("stage_files", { path, files }),
  unstage: (path: string, files: string[]) => call<void>("unstage_files", { path, files }),
  discard: (path: string, files: string[]) => call<void>("discard_files", { path, files }),
  commit: (path: string, message: string, amend: boolean) =>
    call<string>("create_commit", { path, message, amend }),
  branches: (path: string) => call<BranchInfo[]>("list_branches", { path }),
  createBranch: (path: string, name: string, checkout: boolean) =>
    call<void>("create_branch", { path, name, checkout }),
  checkoutBranch: (path: string, name: string, onProgress?: (p: GitProgress) => void) =>
    call<void>("checkout_branch", { path, name, onProgress: progressChannel(onProgress) }),
  fetch: (path: string, onProgress?: (p: GitProgress) => void) =>
    call<string>("git_fetch", { path, onProgress: progressChannel(onProgress) }),
  pull: (path: string, prune: boolean, onProgress?: (p: GitProgress) => void) =>
    call<string>("git_pull", { path, prune, onProgress: progressChannel(onProgress) }),
  push: (path: string, onProgress?: (p: GitProgress) => void) =>
    call<string>("git_push", { path, onProgress: progressChannel(onProgress) }),
  /** Cancels the running remote operation (fetch/pull/push/clone). */
  cancelOperation: (path: string) => call<boolean>("cancel_operation", { path }),

  // File watcher (changes arrive as a "repo-changed" event)
  watchRepository: (path: string) => call<void>("watch_repository", { path }),
  unwatchRepository: () => call<void>("unwatch_repository"),

  // Stash
  stashList: (path: string) => call<StashInfo[]>("stash_list", { path }),
  stashPush: (path: string, message: string, files: string[]) =>
    call<string>("stash_push", { path, message, files }),
  stashApply: (path: string, index: number) => call<void>("stash_apply", { path, index }),
  stashPop: (path: string, index: number) => call<void>("stash_pop", { path, index }),
  stashDrop: (path: string, index: number) => call<void>("stash_drop", { path, index }),

  // Tags
  tags: (path: string) => call<TagInfo[]>("list_tags", { path }),
  createTag: (path: string, name: string, message: string, target: string) =>
    call<void>("create_tag", { path, name, message, target }),
  deleteTag: (path: string, name: string) => call<void>("delete_tag", { path, name }),

  // Branch management
  renameBranch: (path: string, old: string, newName: string) =>
    call<void>("rename_branch", { path, old, new: newName }),
  deleteBranch: (path: string, name: string, force: boolean) =>
    call<void>("delete_branch", { path, name, force }),
  mergeBranch: (path: string, name: string) => call<string>("merge_branch", { path, name }),
  rebaseOnto: (path: string, name: string) => call<string>("rebase_onto", { path, name }),

  // Multi-step operations
  abortOperation: (path: string) => call<string>("abort_operation", { path }),
  continueOperation: (path: string) => call<string>("continue_operation", { path }),
  opContext: (path: string) => call<OpContext>("get_op_context", { path }),
  resolveConflict: (path: string, file: string, ours: boolean) =>
    call<void>("resolve_conflict", { path, file, ours }),
  openMergetool: (path: string, file: string) => call<string>("open_mergetool", { path, file }),
  readConflict: (path: string, file: string) => call<ConflictFile>("read_conflict", { path, file }),
  saveResolution: (path: string, file: string, content: string) =>
    call<void>("save_resolution", { path, file, content }),

  // History operations
  cherryPick: (path: string, commitId: string) => call<string>("cherry_pick", { path, commitId }),
  revertCommit: (path: string, commitId: string) =>
    call<string>("revert_commit", { path, commitId }),
  undoLastCommit: (path: string) => call<void>("undo_last_commit", { path }),
  squashFrom: (path: string, oldestId: string, message: string) =>
    call<string>("squash_from", { path, oldestId, message }),
  createBranchFromCommit: (path: string, name: string, commitId: string, checkout: boolean) =>
    call<void>("create_branch_from_commit", { path, name, commitId, checkout }),
  checkoutCommit: (path: string, commitId: string) =>
    call<void>("checkout_commit", { path, commitId }),
  searchLog: (path: string, query: string, limit: number) =>
    call<CommitInfo[]>("search_log", { path, query, limit }),
  rebaseInteractive: (path: string, baseId: string, steps: RebaseStep[]) =>
    call<string>("rebase_interactive", { path, baseId, steps }),
  unpushedCommits: (path: string) => call<UnpushedCommit[]>("unpushed_commits", { path }),

  // Bisect (binary search)
  bisectStart: (path: string, good: string, bad?: string) =>
    call<string>("bisect_start", { path, good, bad: bad ?? null }),
  bisectMark: (path: string, action: "good" | "bad" | "skip") =>
    call<string>("bisect_mark", { path, action }),
  bisectReset: (path: string) => call<void>("bisect_reset", { path }),

  // Hunk/line staging
  applyHunk: (path: string, file: string, hunkIndex: number, unstage: boolean) =>
    call<void>("apply_hunk", { path, file, hunkIndex, unstage }),
  discardHunk: (path: string, file: string, hunkIndex: number) =>
    call<void>("discard_hunk", { path, file, hunkIndex }),
  applyLines: (
    path: string,
    file: string,
    hunkIndex: number,
    lineIndices: number[],
    unstage: boolean,
  ) => call<void>("apply_lines", { path, file, hunkIndex, lineIndices, unstage }),

  // Remotes & repo lifecycle
  remotes: (path: string) => call<RemoteInfo[]>("list_remotes", { path }),
  pushRemote: (
    path: string,
    remote: string,
    force: boolean,
    onProgress?: (p: GitProgress) => void,
  ) =>
    call<string>("push_remote", { path, remote, force, onProgress: progressChannel(onProgress) }),
  addRemote: (path: string, name: string, url: string) =>
    call<void>("add_remote", { path, name, url }),
  removeRemote: (path: string, name: string) => call<void>("remove_remote", { path, name }),
  renameRemote: (path: string, oldName: string, newName: string) =>
    call<void>("rename_remote", { path, oldName, newName }),
  setRemoteUrl: (path: string, name: string, url: string) =>
    call<void>("set_remote_url", { path, name, url }),
  backups: (path: string) => call<BackupInfo[]>("list_backups", { path }),
  restoreBackup: (path: string, refName: string) =>
    call<string>("restore_backup", { path, refName }),
  deleteBackup: (path: string, refName: string) => call<void>("delete_backup", { path, refName }),

  // Provider accounts & change requests
  providerAccounts: () => call<ProviderAccount[]>("provider_accounts", {}),
  providerAddAccount: (host: string, kind: ProviderKind, token: string, insecureTls: boolean) =>
    call<ProviderAccount>("provider_add_account", { host, kind, token, insecureTls }),
  providerRemoveAccount: (host: string) => call<void>("provider_remove_account", { host }),
  changeRequests: (path: string) => call<ChangeRequestList>("list_change_requests", { path }),
  providerDefaultBranch: (path: string) => call<string>("provider_default_branch", { path }),

  // Local pipeline testing
  pipelineDetect: (path: string) => call<PipelineInfo>("pipeline_detect", { path }),
  pipelineConfigs: (path: string) => call<PipelineConfig[]>("pipeline_configs", { path }),
  /** Adds a manually chosen CI file (absolute path) as a config; the backend
   *  derives the repo-relative path + provider and checks the guards. */
  pipelineAddConfig: (path: string, filePath: string) =>
    call<PipelineConfig>("pipeline_add_config", { path, filePath }),
  pipelineGraph: (path: string, provider: string, config: string) =>
    call<PipelineGraph>("pipeline_graph", { path, provider, config }),
  pipelineRunScope: (
    path: string,
    provider: string,
    config: string,
    scope: "pipeline" | "stage" | "job",
    target: string | null,
    event: string | null,
    variables: [string, string][],
    onEventCb: (e: PipelineEvent) => void,
  ) => {
    const onEvent = new Channel<PipelineEvent>();
    onEvent.onmessage = onEventCb;
    return call<number>("pipeline_run_scope", {
      path,
      provider,
      config,
      scope,
      target,
      event,
      variables,
      onEvent,
    });
  },
  pipelineCancel: (path: string) => call<boolean>("pipeline_cancel", { path }),

  // Sparse checkout
  sparseStatus: (path: string) => call<SparseStatus>("sparse_status", { path }),
  sparseSet: (path: string, dirs: string[]) => call<void>("sparse_set", { path, dirs }),
  sparseDisable: (path: string) => call<void>("sparse_disable", { path }),

  // Multi-level undo/redo
  undoStatus: (path: string) => call<UndoStatus>("undo_status", { path }),
  undoLast: (path: string) => call<UndoEntry>("undo_last", { path }),
  redoLast: (path: string) => call<UndoEntry>("redo_last", { path }),
  createChangeRequest: (path: string, request: NewChangeRequest) =>
    call<ChangeRequest>("create_change_request", { path, request }),
  /** Clone stage 1: create + open the repo (immediate, no network). */
  clonePrepare: (url: string, destDir: string) => call<RepoInfo>("clone_prepare", { url, destDir }),
  /** Clone stage 2: fetch the data + check out the default branch (with progress). */
  cloneFetch: (path: string, options: CloneOptions, onProgress?: (p: GitProgress) => void) =>
    call<string>("clone_fetch", { path, options, onProgress: progressChannel(onProgress) }),
  initRepository: (dir: string) => call<RepoInfo>("init_repository", { dir }),
  ignorePattern: (path: string, pattern: string) => call<void>("ignore_pattern", { path, pattern }),

  // Views
  blameFile: (path: string, file: string) => call<BlameLine[]>("blame_file", { path, file }),
  imageDiff: (path: string, file: string, staged: boolean) =>
    call<ImageDiff>("get_image_diff", { path, file, staged }),

  // Worktrees & submodules
  worktrees: (path: string) => call<WorktreeInfo[]>("list_worktrees", { path }),
  addWorktree: (path: string, dest: string, branch: string) =>
    call<string>("add_worktree", { path, dest, branch }),
  removeWorktree: (path: string, worktreePath: string) =>
    call<string>("remove_worktree", { path, worktreePath }),
  submodules: (path: string) => call<SubmoduleInfo[]>("list_submodules", { path }),
  updateSubmodules: (path: string) => call<string>("update_submodules", { path }),

  // Configuration & system
  configGet: (path: string, key: string) => call<string | null>("config_get", { path, key }),
  configSet: (path: string, key: string, value: string, global: boolean) =>
    call<void>("config_set", { path, key, value, global }),
  checkSigning: (path: string) => call<string>("check_signing", { path }),
  openInExplorer: (path: string) => call<void>("open_in_explorer", { path }),
  openInEditor: (path: string, editor: string | null) =>
    call<void>("open_in_editor", { path, editor }),
  openTerminal: (path: string) => call<void>("open_terminal", { path }),
  openExternal: (url: string) => call<void>("open_external", { url }),
  newWindow: () => call<void>("new_window"),
  openLogs: () => call<void>("open_logs"),

  // SSH key manager
  sshListKeys: () => call<SshKey[]>("ssh_list_keys"),
  sshGenerateKey: (name: string, comment: string, passphrase: string) =>
    call<SshKey>("ssh_generate_key", { name, comment, passphrase }),
  sshScanHost: (host: string, port: number | null) =>
    call<ScannedHost>("ssh_scan_host", { host, port }),
  sshTrustHost: (host: string, port: number | null, lines: string, replace: boolean) =>
    call<void>("ssh_trust_host", { host, port, lines, replace }),
  /** Removes an SSH key. The backend shows a native confirmation dialog first;
   *  on cancel it throws CommandError{code:"cancelled"}. */
  sshRemoveKey: (name: string) => call<void>("ssh_remove_key", { name }),
};
