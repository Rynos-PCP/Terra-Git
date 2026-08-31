// Central UI state (Svelte 5 runes) + actions.
// All engine calls go through the typed API; errors end up as a toast in
// `ui.error`, success messages in `ui.info`.

import { confirm, open } from "@tauri-apps/plugin-dialog";
import {
  api,
  type BlameLine,
  type BranchInfo,
  type CloneOptions,
  type CommandError,
  type CommitInfo,
  type FileDiff,
  type FileLineStats,
  type GitProgress,
  type ImageDiff,
  type ProviderAccount,
  type ProviderKind,
  type RebaseStep,
  type RecentRepo,
  type RemoteInfo,
  type RepoInfo,
  type RepoStatus,
  type ScannedHost,
  type SshKey,
  type StashInfo,
  type TagInfo,
  type UnchangedInfo,
  type UndoEntry,
  type UndoStatus,
  type UnpushedCommit,
} from "./api";
import { parseBisectOutput } from "./bisect";
import { goneDeletableCandidates } from "./branchCleanup";
import {
  autoStashMessage,
  findAutoStash,
  needsSwitchChoice,
  type SwitchTarget,
  switchTargetLabel,
  worktreeDirty,
} from "./branchSwitch";
import { isConflictCandidate } from "./conflictOffer";
import { resolveErrorMessage, t, tn } from "./i18n.svelte";
import { applyPipelineEvent, initialPipelineSlice, type PipelineSlice } from "./pipelineModel";
import { resetRepoSlices } from "./repoReset";
import { parseSshHost } from "./sshHost";
import {
  authorValid,
  baselineOf,
  buildWorkshopSteps,
  firstKeptIsSquash,
  type WorkshopEdit,
} from "./workshopSteps";

const HISTORY_PAGE = 100;
/** Cap for multi-file commit diffs (DOM load; the rest is reported). */
const MAX_COMMIT_DIFF_FILES = 200;

/** Persisted UI preferences (localStorage). */
function loadPrefs() {
  try {
    return JSON.parse(localStorage.getItem("terra-git-prefs") ?? "{}");
  } catch {
    return {};
  }
}
const prefs = loadPrefs();

/** All modal kinds of `ui.modal` — for typed scope props. */
export type ModalKind = NonNullable<(typeof ui)["modal"]>["kind"];

export function savePrefs() {
  localStorage.setItem(
    "terra-git-prefs",
    JSON.stringify({
      theme: ui.theme,
      diffMode: ui.diffMode,
      changesView: ui.changesView,
      editorCmd: ui.editorCmd,
      autoFetch: ui.autoFetch,
      pruneOnPull: ui.pruneOnPull,
      uiScale: ui.uiScale,
      reduceMotion: ui.reduceMotion,
      highContrast: ui.highContrast,
      toastDuration: ui.toastDuration,
      changesPanelWidth: ui.changesPanelWidth,
      historyPanelWidth: ui.historyPanelWidth,
    }),
  );
}

// Sequence tokens: late answers (a race between the poll, actions and fast
// clicks) must never overwrite newer state. The list slices additionally check
// the repo path, because closeRepo triggers no new requests (the token alone
// would stay current there).
let statusSeq = 0;
let historySeq = 0;
let fileDiffSeq = 0;
let commitDiffSeq = 0;
let pipeGraphSeq = 0;
let branchesSeq = 0;
let stashesSeq = 0;
let tagsSeq = 0;
let remotesSeq = 0;
let searchSeq = 0;

export const ui = $state({
  repo: null as RepoInfo | null,
  status: null as RepoStatus | null,
  branches: [] as BranchInfo[],
  history: [] as CommitInfo[],
  historyComplete: false,
  historyLoading: false,
  /** The commit graph is still missing (fresh huge clone) — the history shows
   *  a preparing hint until the "history-prepared" event arrives. */
  historyPreparing: false,
  recents: [] as RecentRepo[],

  /** Number of running index mutations (stage/unstage/discard/commit). */
  working: 0,

  tab: "changes" as "changes" | "history",

  /** Main view: repo workspace, settings, commit workshop, pipeline cockpit or
   *  conflict workshop. */
  view: "repo" as "repo" | "settings" | "commits" | "pipeline" | "conflicts",

  selectedFile: null as { path: string; staged: boolean } | null,
  /** Line balance per changed file for the changes overview
      (null = not loaded yet; loads lazily while the overview is visible). */
  numstat: null as FileLineStats[] | null,
  fileDiff: null as FileDiff | null,
  /** Explanation when fileDiff has no hunks (null = none determined). */
  unchangedInfo: null as UnchangedInfo | null,
  selectedCommit: null as CommitInfo | null,
  commitDiff: [] as FileDiff[],
  /** Total number of files in the commit diff (null = the stream is still running). */
  commitDiffTotal: null as number | null,

  /** Running remote operation (fetch/pull/push) or null. */
  busy: null as string | null,
  /** Whether the running busy operation is cancellable (fetch/pull/push yes; an
   *  in-process checkout NO). Drives the toolbar's "Cancel" button. */
  busyCancellable: false,
  /** Live progress of the running remote operation (git --progress). */
  progress: null as GitProgress | null,
  /** Name of the repo currently being cloned (shows the clone overlay), otherwise null. */
  cloning: null as string | null,
  error: null as string | null,
  info: null as string | null,
  /** Offer in the error toast: either open the conflict workshop or free up the
   *  blocked branch switch via a stash. Unlike `error`/`info` it is REPO-BOUND
   *  and cleared in resetRepoSlices — otherwise, after a repo switch, the old
   *  message would carry a button into the NEW repo's workshop. Whether
   *  "conflicts" is visible is decided reactively by offerConflictWorkshop
   *  (running operation + open conflicted files).
   *  The target branch travels INSIDE the value, not in a second field: that way
   *  there can be no "stashSwitch without a branch". "stashes" points into the
   *  stash list: the way back when changes had to be left behind. */
  errorAction: null as
    | null
    | { kind: "conflicts" }
    | { kind: "stashSwitch"; target: SwitchTarget }
    | { kind: "stashes" },

  /** Is the command palette open? Next to `modal` it is the SECOND focus trap of
   *  the app (its own aria-modal, cyclic tab trapping) and lays itself over
   *  everything — the toast then has to hold back its actions just as it does
   *  for a modal, otherwise it offers a button only the mouse can reach.
   *  The palette reports its own state. */
  paletteOpen: false,

  // New feature slices
  stashes: [] as StashInfo[],
  tags: [] as TagInfo[],
  remotes: [] as RemoteInfo[],
  searchQuery: "",
  searchResults: null as CommitInfo[] | null,
  /** Previously written commit messages of the current repo (newest first). */
  messageLog: [] as string[],
  /** Stored provider accounts (tokens live in the OS keychain). */
  accounts: [] as ProviderAccount[],
  /** Local SSH keys (~/.ssh/*.pub), for the SSH section of the settings. */
  sshKeys: [] as SshKey[],
  /** Multi-level undo/redo status of the current repo. */
  undoStatus: null as UndoStatus | null,
  blame: null as { file: string; lines: BlameLine[] } | null,
  imageDiff: null as ImageDiff | null,
  /** Unpushed commits of the commit workshop (newest first). */
  unpushed: [] as UnpushedCommit[],
  /** Edit buffer per commit id, pre-filled from the original. */
  workshopEdits: {} as Record<string, WorkshopEdit>,
  /** Display order of the workshop (commit ids, newest first). */
  workshopOrder: [] as string[],
  /** loadUnpushed failed (separates the error state from the empty state). */
  workshopError: false,

  /** Running bisect session: a rough number of remaining steps + the first bad
   *  commit found (the active state comes from status.opState === "bisect"). */
  bisect: { stepsLeft: null as number | null, firstBad: null as string | null },

  /** Pipeline cockpit (local CI): detection, configs, graph, live run. */
  pipeline: initialPipelineSlice() as PipelineSlice,

  // UI preferences (persisted)
  theme: (prefs.theme ?? "dark") as "dark" | "light" | "system",
  diffMode: (prefs.diffMode ?? "unified") as "unified" | "split",
  changesView: (prefs.changesView ?? "flat") as "flat" | "tree",
  editorCmd: (prefs.editorCmd ?? "code") as string,
  /** Automatic background fetch (keeps ahead/behind current). */
  autoFetch: (prefs.autoFetch ?? false) as boolean,
  /** Clean up orphaned branches on pull (prune + safe deletion). */
  pruneOnPull: (prefs.pruneOnPull ?? false) as boolean,
  /** Auto-dismiss time for notice toasts in seconds (0 = never). */
  toastDuration: (prefs.toastDuration ?? 4) as number,
  /** Width of the changes list (splitter, px). */
  changesPanelWidth: (prefs.changesPanelWidth ?? 360) as number,
  /** Width of the history sidebar including the commit graph (splitter, px). */
  historyPanelWidth: (prefs.historyPanelWidth ?? 420) as number,

  // Accessibility (persisted)
  /** UI scaling (zoom): 0.9 | 1 | 1.1 | 1.25. */
  uiScale: (prefs.uiScale ?? 1) as number,
  /** Turn off animations app-side (the system setting always applies). */
  reduceMotion: (prefs.reduceMotion ?? false) as boolean,
  /** Stronger borders/secondary text for better readability. */
  highContrast: (prefs.highContrast ?? false) as boolean,

  /** Open modal (centrally managed). */
  modal: null as
    | null
    | { kind: "clone" }
    | { kind: "init" }
    | { kind: "stash" }
    | { kind: "tags" }
    | { kind: "remotes" }
    | { kind: "backups" }
    | { kind: "changeRequests" }
    | { kind: "createCr" }
    | { kind: "sparse" }
    | { kind: "blame"; file: string }
    | { kind: "stashPush" }
    | { kind: "stashPreview"; id: string; message: string }
    | { kind: "submodules" }
    | { kind: "worktrees" }
    | { kind: "switchBranch"; target: SwitchTarget; andThen?: SwitchFollowUp }
    | { kind: "squash"; count: number; oldestId: string }
    | { kind: "branchFrom"; commitId: string }
    | { kind: "tagAt"; commitId: string }
    | { kind: "cherryPickTo"; commitId: string }
    | { kind: "rebase"; baseId: string; commits: CommitInfo[] }
    | { kind: "conflictEditor"; file: string }
    | { kind: "deleteRepo"; path: string }
    | { kind: "sshTofu"; scan: ScannedHost; host: string; port: number | null },
});

export function showError(e: unknown) {
  const err = e as CommandError;
  // Show known error codes in the active language; messages carrying details
  // (paths, branch names) come through unchanged from the backend.
  const message = err?.message ?? String(e);
  ui.error = resolveErrorMessage(err?.code, message);
  ui.info = null;
  // Conflict candidates offer the jump into the workshop (whether it really
  // appears is decided by the structural gate in the toast). The status is
  // refreshed silently so the offer does not hang off a stale opState — the
  // pull error path (remoteOp) does not refresh itself, and the file watcher is
  // suppressed while ui.busy is set.
  ui.errorAction = isConflictCandidate(err?.code, message) ? { kind: "conflicts" } : null;
  if (ui.errorAction && ui.repo) void refreshStatus(true);
}

export function showInfo(msg: string) {
  ui.info = msg;
  ui.error = null;
  ui.errorAction = null;
}

export function clearToast() {
  ui.error = null;
  ui.info = null;
  ui.errorAction = null;
}

/** Open the conflict workshop — ONE way for all entry points (banner, tools
 *  menu, palette, error toast). The toast disappears in the process: it has
 *  served its purpose once you follow the hint. */
export function openConflicts() {
  clearToast();
  // Deliberately WITHOUT `ui.modal = null`: the first version of this fix did
  // that and thereby discarded the manual work in the conflict editor (the
  // resolutions live only in the component until saved) — a new data-loss path.
  // That no modal CAN still be open here is ensured by the two entry points:
  // the toast does not offer its actions while a modal is open (Toast.svelte),
  // and the palette can no longer be opened over a modal (App.svelte).
  ui.view = "conflicts";
}

export async function loadRecents() {
  try {
    ui.recents = await api.recentRepos();
  } catch {
    ui.recents = [];
  }
}

/** Removes a repo from the recently-opened list (deletes nothing on disk). */
export async function forgetRecent(path: string) {
  try {
    await api.removeRecent(path);
    ui.recents = ui.recents.filter((r) => r.path !== path);
  } catch (e) {
    showError(e);
  }
}

/** Pins a repo in the list or releases the pin (pinned ones come first and
 *  never fall out of the list). The order comes from the backend. */
export async function pinRecent(path: string, pinned: boolean) {
  try {
    await api.setRecentPinned(path, pinned);
    await loadRecents();
  } catch (e) {
    showError(e);
  }
}

/** Opens the folder dialog and loads the chosen repo — the shared path for the
 *  welcome button, the toolbar and the global Ctrl+O shortcut. */
export async function browseForRepo() {
  const dir = await open({ directory: true, title: t("dialog.openRepo") });
  if (typeof dir === "string") await openRepo(dir);
}

/** Moves the repository to the trash and removes it from the list. */
export async function deleteRepoFromDisk(path: string) {
  try {
    // If the CURRENTLY open repo is deleted, release the file watcher and close
    // the repo state first — otherwise the watcher holds a handle on the
    // directory (trashing fails) and the UI keeps showing a repo that no longer
    // exists.
    if (path === ui.repo?.path) {
      await api.unwatchRepository().catch(() => {});
      closeRepo();
    }
    await api.deleteRepo(path);
    ui.recents = ui.recents.filter((r) => r.path !== path);
    ui.modal = null;
    showInfo(t("welcome.repoDeleted"));
  } catch (e) {
    // The native OS confirmation dialog (the backend guard) was cancelled —
    // that is not an error but a deliberate no-op.
    if ((e as { code?: string })?.code === "cancelled") return;
    showError(e);
  }
}

// Paths whose "history-prepared" event has already arrived. Catches the race in
// which the background write of a small repo finishes BEFORE openRepo() has
// processed the answer with historyPrepared=false — otherwise the preparing hint
// would stay stuck. Deliberately not reactive.
// eslint-disable-next-line svelte/prefer-svelte-reactivity
const preparedHistoryPaths = new Set<string>();

/** Handles the backend's "history-prepared" event (App.svelte). */
export function markHistoryPrepared(path: string) {
  preparedHistoryPaths.add(path);
  if (ui.repo?.path === path) ui.historyPreparing = false;
}

/** Opens a repo and reloads all views. */
export async function openRepo(path: string) {
  try {
    ui.repo = await api.openRepository(path);
    ui.tab = "changes";
    // Reset everything BEFORE loading the new data — otherwise the panels
    // briefly show the (stale) state of the previous repo on a repo switch.
    // `status = null` is at the same time the signal for the loading skeletons.
    resetRepoSlices(ui);
    // AFTER the reset, otherwise it would overwrite the computed value with false.
    ui.historyPreparing = !ui.repo.historyPrepared && !preparedHistoryPaths.has(ui.repo.path);
    // Running diff requests/streams belong to the old repo: invalidate the
    // tokens, otherwise a late diff appears in the new repo.
    fileDiffSeq++;
    commitDiffSeq++;
    loadMessageLog();
    await Promise.all([
      refreshStatus(),
      refreshBranches(),
      loadMoreHistory(true),
      refreshStashes(),
      refreshTags(),
      refreshRemotes(),
    ]);
    await loadRecents();
    // File watcher: changes in the workdir trigger a status refresh immediately
    // (event "repo-changed"). If that fails (e.g. a network drive without change
    // notifications), the poll fallback stays active.
    api.watchRepository(ui.repo.path).catch(() => {});
  } catch (e) {
    showError(e);
  }
}

export function closeRepo() {
  api.unwatchRepository().catch(() => {});
  // Invalidate open requests — after closing, no late answer may write into the
  // (emptied) state. Status/history do not check a repo path (unlike the list
  // slices), so they need a fresh token here too.
  statusSeq++;
  historySeq++;
  fileDiffSeq++;
  commitDiffSeq++;
  ui.repo = null;
  ui.view = "repo";
  ui.tab = "changes";
  // Symmetrical to openRepo — invariant: no repo, no repo state.
  // What is deliberately NOT cleared, and why, is documented in repoReset.ts.
  resetRepoSlices(ui);
}

export async function refreshStatus(silent = false) {
  if (!ui.repo) return;
  const seq = ++statusSeq;
  refreshUndoStatus();
  try {
    const status = await api.status(ui.repo.path);
    // Late answer (e.g. a background poll after a stage): discard it.
    if (seq !== statusSeq) return;
    ui.status = status;
    // Clean up the selection when the file no longer appears in the status.
    if (ui.selectedFile) {
      const list = ui.selectedFile.staged ? ui.status.staged : ui.status.unstaged;
      if (!list.some((e) => e.path === ui.selectedFile!.path)) {
        ui.selectedFile = null;
        ui.fileDiff = null;
        ui.unchangedInfo = null;
      }
    }
  } catch (e) {
    if (!silent && seq === statusSeq) showError(e);
  }
}

let numstatSeq = 0;

/**
 * Loads the line balance for the changes overview. Deliberately NOT part of
 * refreshStatus: the overview only requests it while it is visible (no file
 * selected) — otherwise every status refresh would pay for a second full diff on
 * large repos.
 */
export async function refreshNumstat() {
  if (!ui.repo) return;
  const seq = ++numstatSeq;
  const path = ui.repo.path;
  try {
    const stats = await api.statusNumstat(path);
    // Late answer or the repo has changed in the meantime: discard it.
    if (seq !== numstatSeq || ui.repo?.path !== path) return;
    ui.numstat = stats;
  } catch {
    // The balance is a convenience: the overview works without the numbers too.
  }
}

export async function refreshBranches() {
  if (!ui.repo) return;
  const seq = ++branchesSeq;
  const path = ui.repo.path;
  try {
    const branches = await api.branches(path);
    // Late answer or a different/no repo in the meantime: discard it.
    if (seq !== branchesSeq || ui.repo?.path !== path) return;
    ui.branches = branches;
  } catch (e) {
    if (seq === branchesSeq && ui.repo?.path === path) showError(e);
  }
}

export async function loadMoreHistory(reset = false) {
  if (!ui.repo) return;
  // Double click on "load more" while a load is running: ignore it (otherwise
  // duplicate commit ids in the keyed each). Resets may always start and
  // invalidate all older answers through the token.
  if (!reset && ui.historyLoading) return;
  const seq = ++historySeq;
  ui.historyLoading = true;
  try {
    const skip = reset ? 0 : ui.history.length;
    // Whole-repository graph: all branches (local + remote), tags and HEAD —
    // not just the HEAD line.
    const page = await api.logAll(ui.repo.path, skip, HISTORY_PAGE);
    if (seq !== historySeq) return;
    ui.history = reset ? page : [...ui.history, ...page];
    ui.historyComplete = page.length < HISTORY_PAGE;
    // After a reset the selected commit may have disappeared (e.g. after a branch
    // switch or an amend). Any diff stream still running for it is invalidated
    // along with the token.
    if (ui.selectedCommit && !ui.history.some((c) => c.id === ui.selectedCommit!.id)) {
      commitDiffSeq++;
      ui.selectedCommit = null;
      ui.commitDiff = [];
      ui.commitDiffTotal = null;
    }
  } catch (e) {
    if (seq === historySeq) showError(e);
  } finally {
    if (seq === historySeq) ui.historyLoading = false;
  }
}

const IMAGE_RE = /\.(png|jpe?g|gif|webp|bmp|ico|svg)$/i;

export async function selectFile(path: string, staged: boolean) {
  if (!ui.repo) return;
  // A sequence token instead of object identity: ui.selectedFile is a Svelte 5
  // proxy, a `=== raw object` comparison would ALWAYS be false and the diff
  // would never be set.
  const seq = ++fileDiffSeq;
  ui.selectedFile = { path, staged };
  ui.fileDiff = null;
  ui.unchangedInfo = null;
  ui.imageDiff = null;
  try {
    const [diff] = await Promise.all([
      api.fileDiff(ui.repo.path, path, staged),
      IMAGE_RE.test(path) ? loadImageDiff(path, staged, seq) : Promise.resolve(),
    ]);
    // Only take it over when this selection is still the most recent one.
    if (seq === fileDiffSeq) ui.fileDiff = diff;

    // Reported as changed but without a content difference: supply the reason
    // instead of leaving it at "no content changes". Binary files also have null
    // hunks — those are not meant here.
    if (seq === fileDiffSeq && diff && !diff.isBinary && diff.hunks.length === 0) {
      try {
        const info = await api.explainUnchanged(ui.repo.path, path, staged);
        if (seq === fileDiffSeq) ui.unchangedInfo = info;
      } catch {
        // A failed diagnosis must not drag the diff down with it — the view then
        // falls back to the previous hint.
      }
    }
  } catch (e) {
    if (seq === fileDiffSeq) showError(e);
  }
}

export async function selectCommit(commit: CommitInfo) {
  if (!ui.repo) return;
  const seq = ++commitDiffSeq;
  ui.selectedCommit = commit;
  ui.commitDiff = [];
  ui.commitDiffTotal = null;
  try {
    // File-by-file streaming: large commits appear progressively instead of as
    // one huge IPC package; beyond MAX_COMMIT_DIFF_FILES it is truncated (the
    // history reports that through commitDiffTotal).
    const total = await api.commitDiffStream(
      ui.repo.path,
      commit.id,
      MAX_COMMIT_DIFF_FILES,
      (fd) => {
        if (seq === commitDiffSeq) ui.commitDiff.push(fd);
      },
    );
    if (seq === commitDiffSeq) ui.commitDiffTotal = total;
  } catch (e) {
    if (seq === commitDiffSeq) showError(e);
  }
}

/** Runs an index mutation with a working counter (the UI disables buttons). */
async function mutate(op: () => Promise<void>) {
  if (!ui.repo) return;
  ui.working++;
  try {
    await op();
    await refreshStatus();
  } catch (e) {
    showError(e);
  } finally {
    ui.working--;
  }
}

export const stageFiles = (files: string[]) => mutate(() => api.stage(ui.repo!.path, files));

export const unstageFiles = (files: string[]) => mutate(() => api.unstage(ui.repo!.path, files));

export const discardFiles = (files: string[]) => mutate(() => api.discard(ui.repo!.path, files));

export async function createCommit(message: string, amend: boolean): Promise<boolean> {
  if (!ui.repo) return false;
  ui.working++;
  try {
    const id = await api.commit(ui.repo.path, message, amend);
    logCommitMessage(ui.repo.path, message);
    showInfo(t("state.commitCreated", { id: id.slice(0, 8) }));
    await Promise.all([refreshStatus(), loadMoreHistory(true), refreshRepoInfo()]);
    return true;
  } catch (e) {
    showError(e);
    return false;
  } finally {
    ui.working--;
  }
}

// ================= Commit message log =================
// Keeps previously written commit messages per repo (localStorage) so recurring
// message blocks can be reused.

const MSGLOG_KEY = "terra-git-msglog";
const MSGLOG_MAX = 30;

function readMsgLog(): Record<string, string[]> {
  try {
    return JSON.parse(localStorage.getItem(MSGLOG_KEY) ?? "{}");
  } catch {
    return {};
  }
}

function logCommitMessage(repoPath: string, message: string) {
  const msg = message.trim();
  if (!msg) return;
  const log = readMsgLog();
  const list = log[repoPath] ?? [];
  const next = [msg, ...list.filter((m) => m !== msg)].slice(0, MSGLOG_MAX);
  log[repoPath] = next;
  try {
    localStorage.setItem(MSGLOG_KEY, JSON.stringify(log));
  } catch {
    // Storage full/locked — the log is a convenience, not data loss.
  }
  ui.messageLog = next;
}

export function loadMessageLog() {
  ui.messageLog = ui.repo ? (readMsgLog()[ui.repo.path] ?? []) : [];
}

async function refreshRepoInfo() {
  if (!ui.repo) return;
  try {
    ui.repo = await api.openRepository(ui.repo.path);
  } catch {
    /* Refreshing the repo info is uncritical */
  }
}

/**
 * Holds the switch lock for a WHOLE switch flow — stash, checkout, restore,
 * follow-up — not just for the checkout itself.
 *
 * Three related races shared one root: `ui.busy` covered only
 * `runCheckout`. A parallel operation (auto fetch, a second click) could slip
 * in between stash and checkout and turn the switch into a silent `false`
 * — the auto-stash of the previous branch was looked up and unpacked after
 * the lock was already released, i.e. on whatever branch was current by the
 * time the stash list arrived; and `ui.repo` was re-read after every
 * `await`, so a repo switch mid-flow moved the stash and the pop into a repo
 * the user never chose.
 *
 * Now the lock spans the flow, and every step works on the repo path captured
 * here — the flow finishes where it started, even if the UI shows another repo
 * by then. ui.busy at the same time drives the toolbar spinner + progress bar.
 */
async function withSwitchLock(
  target: SwitchTarget,
  flow: (repoPath: string) => Promise<void>,
): Promise<void> {
  if (!ui.repo || ui.busy) return;
  const path = ui.repo.path;
  ui.busy = t("state.switchingBranch", { name: switchTargetLabel(target) });
  ui.progress = null;
  try {
    await flow(path);
  } finally {
    ui.busy = null;
    ui.progress = null;
  }
}

/**
 * The plain switch including the reload — WITHOUT error handling: the backend
 * error propagates so callers can tell cases apart (the "bring along" path
 * reacts to `checkout_would_overwrite` with the stash detour instead of merely
 * reporting it). Runs under the switch lock.
 */
async function runCheckout(path: string, target: SwitchTarget) {
  const onProg = (p: GitProgress) => (ui.progress = p);
  if (target.kind === "branch") await api.checkoutBranch(path, target.name, onProg);
  else await api.checkoutCommit(path, target.id);
  // The commit selection (including any running diff stream) belongs to the
  // old branch state.
  commitDiffSeq++;
  ui.selectedCommit = null;
  ui.commitDiff = [];
  showInfo(
    target.kind === "branch"
      ? t("state.branchCheckedOut", { name: target.name })
      : t("state.commitCheckedOut", { id: switchTargetLabel(target) }),
  );
  // refreshRepoInfo is mandatory here: after a commit checkout HEAD is
  // detached, and the toolbar, the branch menu and the stash assignment all
  // hang off that.
  await Promise.all([refreshRepoInfo(), refreshStatus(), refreshBranches(), loadMoreHistory(true)]);
}

/**
 * Switches and reports errors as a toast; `true` on success. Runs under the
 * switch lock.
 *
 * `autoRestore` turns off restoring changes that were left behind — the stash
 * detour of `carryChangesAndSwitch` brings its own changes and must not get in
 * its own way.
 */
async function checkoutLocked(
  path: string,
  target: SwitchTarget,
  autoRestore = true,
): Promise<boolean> {
  try {
    await runCheckout(path, target);
  } catch (e) {
    showError(e);
    // Blocked by uncommitted changes: the toast gets the way out along with it
    // (showError has just set/cleared errorAction — hence AFTERWARDS). The target
    // travels inside the value.
    if ((e as CommandError)?.code === "checkout_would_overwrite") {
      ui.errorAction = { kind: "stashSwitch", target };
    }
    return false;
  }
  // Only branches carry an auto-stash marker; after a commit checkout there is
  // nothing to assign anything to.
  if (autoRestore && target.kind === "branch") await restoreAutoStash(path, target.name);
  return true;
}

/**
 * The ONE entry point for every checkout — branch menu, palette, history and
 * cherry-pick-onto-branch.
 *
 * When there is uncommitted work in the worktree, the switch does NOT happen
 * silently (git otherwise takes the changes along without comment or aborts with
 * a raw libgit2 message): the dialog asks first where they belong.
 * The condition for that lives as a pure function in branchSwitch.ts.
 *
 * `andThen` appends what still has to happen AFTER a successful switch — so a
 * composed operation (cherry-pick onto another branch) does not have to skip the
 * question in order to keep its second step.
 */
async function requestSwitch(target: SwitchTarget, andThen?: SwitchFollowUp) {
  if (!ui.repo || ui.busy) return;
  if (needsSwitchChoice(ui.status)) {
    ui.modal = { kind: "switchBranch", target, andThen };
    return;
  }
  await withSwitchLock(target, async (path) => {
    if (await checkoutLocked(path, target)) await runFollowUp(path, target, andThen);
  });
}

/** Switch branch (branch menu, palette, history). */
export async function switchBranch(name: string) {
  await requestSwitch({ kind: "branch", name });
}

/** What is still pending after a successful switch. */
export type SwitchFollowUp = { kind: "cherryPick"; commitId: string };

/** Runs the appended second step in the repo the flow started in — historyOp
 *  reports errors. */
async function runFollowUp(path: string, target: SwitchTarget, andThen?: SwitchFollowUp) {
  if (andThen?.kind !== "cherryPick") return;
  await historyOp(
    t("state.commitCherryPickedOnto", {
      id: andThen.commitId.slice(0, 8),
      branch: switchTargetLabel(target),
    }),
    (p) => api.cherryPick(p, andThen.commitId),
    path,
  );
}

/** Name of the branch we are CURRENTLY on (for the stash assignment). */
function currentBranchName(): string | null {
  if (!ui.repo || ui.repo.headDetached) return null;
  return ui.status?.branch ?? ui.repo.currentBranch ?? null;
}

/**
 * LEAVE the changes behind and switch: stash everything (untracked files
 * included), then switch. Only after a successful stash — otherwise the switch
 * would fail a second time on the same files.
 *
 * The stash carries the branch marker (branchSwitch.ts) and comes back by itself
 * when switching back. Only not on a detached HEAD: no branch name leads back
 * there, so the stash stays unmarked and waits in the list.
 */
export async function stashAndSwitch(target: SwitchTarget, andThen?: SwitchFollowUp) {
  await withSwitchLock(target, async (path) => {
    clearToast();
    const name = switchTargetLabel(target);
    const from = currentBranchName();
    const message = from ? autoStashMessage(from) : t("state.stashBeforeSwitch", { name });
    ui.working++;
    try {
      await api.stashPush(path, message, []);
      await Promise.all([refreshStatus(), refreshStashes()]);
    } catch (e) {
      showError(e);
      return;
    } finally {
      ui.working--;
    }
    if (await checkoutLocked(path, target)) {
      await runFollowUp(path, target, andThen);
      return;
    }
    // Stashed, but the switch failed: the changes have disappeared from the
    // worktree — the toast has to say where they went.
    ui.errorAction = { kind: "stashes" };
  });
}

/**
 * BRING the changes along and switch.
 *
 * The direct way first: as long as none of the changed files looks different in
 * the target branch, git takes them along by itself — without a stash detour and
 * without any risk. Only when the checkout fails on exactly that does the detour
 * come in (stash → switch → apply again).
 */
export async function carryChangesAndSwitch(target: SwitchTarget, andThen?: SwitchFollowUp) {
  await withSwitchLock(target, async (path) => {
    clearToast();
    try {
      await runCheckout(path, target);
    } catch (e) {
      if ((e as CommandError)?.code !== "checkout_would_overwrite") {
        showError(e);
        return;
      }
      if (await carryViaStash(path, target)) await runFollowUp(path, target, andThen);
      return;
    }
    if (target.kind === "branch") await restoreAutoStash(path, target.name);
    await runFollowUp(path, target, andThen);
  });
}

/**
 * The detour for colliding files: stash, switch, apply again — each step only
 * after the previous one succeeded.
 *
 * Our own stash is found again through its text instead of through index 0: a
 * wrong index would unpack someone else's changes. If applying fails (real
 * conflicts against the target branch), the stash stays fully intact and the
 * toast shows the way to it.
 */
async function carryViaStash(path: string, target: SwitchTarget): Promise<boolean> {
  const name = switchTargetLabel(target);
  const message = t("state.stashCarryOver", { name });
  // No placeholder value: the catch below returns, so the only path that
  // reaches the pop has assigned a real index.
  let index: number;
  ui.working++;
  try {
    await api.stashPush(path, message, []);
    const list = await api.stashList(path);
    const entry = list.find((s) => s.message.includes(message));
    if (!entry) throw { code: "stash_not_found", message } as CommandError;
    index = entry.index;
  } catch (e) {
    showError(e);
    await Promise.all([refreshStatus(), refreshStashes()]);
    return false;
  } finally {
    ui.working--;
  }

  if (!(await checkoutLocked(path, target, false))) {
    // The switch failed for a different reason: the changes are now in the stash
    // — the toast has to say that, otherwise they look lost.
    ui.errorAction = { kind: "stashes" };
    await refreshStashes();
    return false;
  }

  let applied = false;
  ui.working++;
  try {
    await api.stashPop(path, index);
    showInfo(t("state.changesCarriedOver", { name }));
    applied = true;
  } catch (e) {
    showError(e);
    ui.errorAction = { kind: "stashes" };
  } finally {
    ui.working--;
    await Promise.all([refreshStatus(), refreshStashes()]);
  }
  // Only now, with a fresh status: is something left behind still waiting for
  // the target?
  if (applied && target.kind === "branch") await noteLeftoverAutoStash(path, target.name);
  return applied;
}

/**
 * Pick up changes left behind: if a marked stash is waiting for the branch just
 * entered, it comes back automatically.
 *
 * Only onto a CLEAN worktree — otherwise applying would meet changes brought
 * along and could create conflicts nobody asked for. The stash then stays put,
 * and a hint names it (instead of silently forgetting it).
 *
 * Foreign stashes are never touched: only the marker counts. Runs under the
 * switch lock, so no second switch can move HEAD between the lookup and the pop.
 */
async function restoreAutoStash(path: string, branch: string) {
  const entry = await findLeftoverAutoStash(path, branch);
  if (!entry) return;

  if (worktreeDirty(ui.status)) {
    showInfo(t("state.autoStashKept", { name: branch }));
    await refreshStashes();
    return;
  }

  ui.working++;
  try {
    await api.stashPop(path, entry.index);
    showInfo(t("state.autoStashRestored", { name: branch }));
  } catch (e) {
    showError(e);
    ui.errorAction = { kind: "stashes" };
  } finally {
    ui.working--;
    await Promise.all([refreshStatus(), refreshStashes()]);
  }
}

/**
 * The marked stash of this branch — or null. Never throws: the restoration is a
 * convenience, and its failure must not make a successful switch look like an
 * error.
 */
async function findLeftoverAutoStash(path: string, branch: string): Promise<StashInfo | null> {
  try {
    const list = await api.stashList(path);
    // The repo has changed/closed in the meantime: the convenience is not worth
    // a pop into a repo the UI no longer shows. The marker stays, the stash comes
    // back on the next switch to that branch.
    if (ui.repo?.path !== path) return null;
    return findAutoStash(list, branch);
  } catch {
    return null;
  }
}

/**
 * Only REPORT, never apply.
 *
 * On the bring-along detour the user has just explicitly decided to "bring
 * along" — a second set of changes on top, unasked, would be exactly the conflict
 * `restoreAutoStash` otherwise avoids. Without this hint, though, the older work
 * would stay invisible in the stash: the user
 * saw "changes brought along" and considered them lost.
 *
 * The hint deliberately overwrites the brought-along message: it carries the new
 * information, the other one only confirms the expected.
 */
async function noteLeftoverAutoStash(path: string, branch: string) {
  if (!(await findLeftoverAutoStash(path, branch))) return;
  showInfo(t("state.autoStashKept", { name: branch }));
  await refreshStashes();
}

/** Open the stash list — the way back from the error toast. */
export function openStashes() {
  clearToast();
  ui.modal = { kind: "stash" };
}

export async function createBranch(name: string) {
  if (!ui.repo) return;
  try {
    await api.createBranch(ui.repo.path, name, true);
    showInfo(t("state.branchCreated", { name }));
    await Promise.all([refreshRepoInfo(), refreshStatus(), refreshBranches()]);
  } catch (e) {
    showError(e);
  }
}

type ProgressOp = (path: string, onProgress: (p: GitProgress) => void) => Promise<string>;

/** Cached retry of a remote operation that failed on an unknown SSH host key —
 *  executed again after the TOFU trust succeeded (see confirmSshTrust). */
let pendingSshRetry: (() => Promise<void>) | null = null;

/** Shared TOFU handling for `host_key` errors (remoteOp AND cloning): scans the
 *  host key, remembers the retry and opens the fingerprint dialog.
 *  After confirmation (confirmSshTrust) `retry` runs automatically.
 *  If OpenSSH is missing (ssh_tool_missing), a clear message instead of a generic one. */
async function handleHostKey(host: string, port: number | null, retry: () => Promise<void>) {
  try {
    const scan = await api.sshScanHost(host, port);
    pendingSshRetry = retry;
    ui.modal = { kind: "sshTofu", scan, host, port };
  } catch (scanErr) {
    if ((scanErr as CommandError)?.code === "ssh_tool_missing") {
      showError({ code: "ssh_tool_missing", message: t("state.sshToolMissing") });
    } else {
      showError(scanErr);
    }
  }
}

/** Determines the tracked remote from the upstream (e.g. "origin/main" ->
 *  "origin"); undefined when no upstream is set. */
function trackedRemote(): string | undefined {
  const up = ui.status?.upstream;
  return up ? up.split("/")[0] : undefined;
}

/** Target remote of a (force) push — mirrors the engine logic pick_push_remote:
 *  the branch's upstream remote, otherwise the only remote, otherwise "origin".
 *  A force push MUST hit the same remote as a normal push; remotes[0] would be
 *  the alphabetically first one with several remotes (e.g. "backup"). */
function pushTargetRemote(): string {
  return trackedRemote() ?? (ui.remotes.length === 1 ? ui.remotes[0].name : "origin");
}

async function remoteOp(
  label: string,
  op: ProgressOp,
  /** Only set for push: allows offering a force push on "non-fast-forward". */
  forceRetry?: ProgressOp,
  /** Name of the remote actually affected (for the TOFU host scan). Without it,
   *  the code falls back to origin or the first remote. */
  remoteName?: string,
  /** Runs AFTER the success refresh (e.g. auto cleanup after a pull). Passed
   *  through the force/TOFU retry paths so it applies there too. */
  onSuccess?: () => Promise<void>,
) {
  if (!ui.repo || ui.busy || ui.cloning) return;
  ui.busy = label;
  ui.busyCancellable = true; // fetch/pull/push run through the cancellable sidecar
  ui.progress = null;
  const onProg = (p: GitProgress) => (ui.progress = p);
  try {
    const output = await op(ui.repo.path, onProg);
    showInfo(output ? `${label}: ${firstLine(output)}` : t("state.opSuccess", { label }));
    await Promise.all([refreshStatus(), refreshBranches(), loadMoreHistory(true)]);
    if (onSuccess) await onSuccess();
  } catch (e) {
    const err = e as CommandError;
    // Cancelled by the user: no error toast, just a neutral notice.
    if (err?.code === "cancelled") {
      showInfo(t("state.opCancelled", { label }));
      return;
    }
    // Push rejected because the remote is ahead: offer a force push directly
    // (instead of making the user interpret the error message themselves).
    if (forceRetry && err?.code === "non_fast_forward") {
      ui.busy = null;
      ui.progress = null;
      const yes = await confirm(`${err.message}\n\n${t("state.forcePushQuestion")}`, {
        title: t("state.pushRejectedTitle"),
        kind: "warning",
      });
      if (yes)
        await remoteOp(t("state.opForce", { label }), forceRetry, undefined, remoteName, onSuccess);
      return;
    }
    // Unknown/changed SSH host key: scan the fingerprint and show the TOFU dialog
    // instead of just the raw error message. After confirmation
    // (confirmSshTrust) the same operation is repeated automatically.
    if (err?.code === "host_key" && ui.repo) {
      ui.busy = null;
      ui.progress = null;
      try {
        const remotes = await api.remotes(ui.repo.path);
        const origin = remotes.find((r) => r.name === "origin");
        // Scan the remote ACTUALLY affected (not blindly origin) — otherwise the
        // TOFU retry loops on a non-origin push.
        const target = remotes.find((r) => r.name === remoteName) ?? origin ?? remotes[0];
        const parsed = target ? parseSshHost(target.url) : null;
        if (!parsed) {
          showError(e);
          return;
        }
        await handleHostKey(parsed.host, parsed.port, () =>
          remoteOp(label, op, forceRetry, remoteName, onSuccess),
        );
      } catch (lookupErr) {
        showError(lookupErr);
      }
      return;
    }
    showError(e);
  } finally {
    ui.busy = null;
    ui.busyCancellable = false;
    ui.progress = null;
  }
}

/** Cancels the running remote operation (the toolbar's "Cancel" during a sync).
 *  The git child process is killed immediately in the backend; remoteOp catches
 *  the "cancelled" error and reports neutrally. */
export async function cancelRemoteOp() {
  // Applies to fetch/pull/push (ui.busy) AND to the running clone (ui.cloning).
  if (!ui.repo) return;
  try {
    await api.cancelOperation(ui.repo.path);
  } catch {
    // Cancelling is best effort — an error here is uncritical.
  }
}

function firstLine(s: string): string {
  const line = s.split("\n").find((l) => l.trim().length > 0) ?? "";
  return line.length > 120 ? line.slice(0, 117) + "…" : line;
}

export const gitFetch = () => remoteOp(t("toolbar.fetch"), api.fetch, undefined, trackedRemote());

/** Silent background fetch (auto fetch): updates ahead/behind without toasts.
 *  Network errors are deliberately swallowed (the remote may be offline). */
export async function autoFetchTick() {
  if (!ui.repo || ui.busy || ui.cloning || ui.working > 0 || ui.remotes.length === 0) return;
  try {
    await api.fetch(ui.repo.path);
    await refreshStatus(true);
  } catch {
    // ignore quietly — the next tick tries again
  }
}
export const gitPull = () =>
  remoteOp(
    t("toolbar.pull"),
    (path, onProg) => api.pull(path, ui.pruneOnPull, onProg),
    undefined,
    trackedRemote(),
    ui.pruneOnPull ? cleanupGoneBranches : undefined,
  );

/** Auto cleanup after a pull (only when pruneOnPull): deletes safe orphaned
 *  local branches. force=false -> the backend rejects unmerged ones (they stay);
 *  every deletion is an undo entry. */
async function cleanupGoneBranches() {
  if (!ui.repo) return;
  const candidates = goneDeletableCandidates(ui.branches);
  let deleted = 0;
  for (const name of candidates) {
    try {
      await api.deleteBranch(ui.repo.path, name, false);
      deleted++;
    } catch {
      // branch_not_merged etc. -> skip silently (no data loss).
    }
  }
  if (deleted > 0) {
    await Promise.all([refreshBranches(), refreshUndoStatus()]);
    showInfo(tn("branch.goneCleaned", deleted));
  }
}
export const gitPush = () =>
  remoteOp(
    t("toolbar.push"),
    api.push,
    // The force retry after non_fast_forward has to hit the same remote as the
    // failed normal push (pick_push_remote), not remotes[0].
    (p, on) => api.pushRemote(p, pushTargetRemote(), true, on),
    // api.push targets the upstream remote (pick_push_remote) — the TOFU scan has
    // to hit the same host, not the alphabetically first remote.
    trackedRemote(),
  );
/** Explicit force push (toolbar/palette): ALWAYS asks first and names the target
 *  remote. The non_fast_forward retry in remoteOp deliberately does NOT go
 *  through here — it has its own confirmation (no double question). */
export async function gitPushForce() {
  if (!ui.repo || ui.busy || ui.cloning) return;
  // Pin the remote down BEFORE the confirmation: the dialog text and the actual
  // push target must be guaranteed to match.
  const remote = pushTargetRemote();
  const yes = await confirm(t("state.forcePushConfirm", { remote }), {
    title: t("state.forcePush"),
    kind: "warning",
  });
  if (!yes) return;
  await remoteOp(
    t("state.forcePush"),
    (p, on) => api.pushRemote(p, remote, true, on),
    undefined,
    remote,
  );
}
export const gitPushTo = (remote: string) =>
  remoteOp(
    t("toolbar.pushTo", { name: remote }),
    (p, on) => api.pushRemote(p, remote, false, on),
    (p, on) => api.pushRemote(p, remote, true, on),
    remote,
  );

// ===================== Stash =====================

export async function refreshStashes() {
  if (!ui.repo) return;
  const seq = ++stashesSeq;
  const path = ui.repo.path;
  try {
    const stashes = await api.stashList(path);
    // Late answer or a different/no repo in the meantime: discard it.
    if (seq !== stashesSeq || ui.repo?.path !== path) return;
    ui.stashes = stashes;
  } catch {
    if (seq === stashesSeq && ui.repo?.path === path) ui.stashes = [];
  }
}

export async function stashPush(message: string, files: string[] = []) {
  if (!ui.repo) return;
  ui.working++;
  try {
    await api.stashPush(ui.repo.path, message, files);
    showInfo(files.length ? tn("state.filesStashed", files.length) : t("state.changesStashed"));
    await Promise.all([refreshStatus(), refreshStashes()]);
  } catch (e) {
    showError(e);
  } finally {
    ui.working--;
  }
}

async function stashOp(
  op: (path: string, i: number) => Promise<void>,
  index: number,
  label: string,
) {
  if (!ui.repo) return;
  ui.working++;
  try {
    await op(ui.repo.path, index);
    showInfo(label);
    await Promise.all([refreshStatus(), refreshStashes()]);
  } catch (e) {
    showError(e);
  } finally {
    ui.working--;
  }
}

export const stashApply = (i: number) => stashOp(api.stashApply, i, t("state.stashApplied"));
export const stashPop = (i: number) => stashOp(api.stashPop, i, t("state.stashPopped"));
export const stashDrop = (i: number) => stashOp(api.stashDrop, i, t("state.stashDropped"));

// ===================== Submodules =====================

/** Updates all submodules recursively (a network operation, may take a while). */
export async function updateSubmodules() {
  if (!ui.repo || ui.busy) return;
  ui.busy = t("state.submodules");
  try {
    const out = await api.updateSubmodules(ui.repo.path);
    showInfo(out ? firstLine(out) : t("state.submodulesUpdated"));
    await refreshStatus(true);
  } catch (e) {
    showError(e);
  } finally {
    ui.busy = null;
  }
}

// ===================== Tags =====================

export async function refreshTags() {
  if (!ui.repo) return;
  const seq = ++tagsSeq;
  const path = ui.repo.path;
  try {
    const tags = await api.tags(path);
    // Late answer or a different/no repo in the meantime: discard it.
    if (seq !== tagsSeq || ui.repo?.path !== path) return;
    ui.tags = tags;
  } catch {
    if (seq === tagsSeq && ui.repo?.path === path) ui.tags = [];
  }
}

export async function createTag(name: string, message: string, target: string) {
  if (!ui.repo) return;
  try {
    await api.createTag(ui.repo.path, name, message, target);
    showInfo(t("state.tagCreated", { name }));
    await refreshTags();
  } catch (e) {
    showError(e);
  }
}

export async function deleteTag(name: string) {
  if (!ui.repo) return;
  try {
    await api.deleteTag(ui.repo.path, name);
    showInfo(t("state.tagDeleted", { name }));
    await refreshTags();
  } catch (e) {
    showError(e);
  }
}

// ================= Branch management =================

export async function renameBranch(old: string, newName: string) {
  if (!ui.repo) return;
  try {
    await api.renameBranch(ui.repo.path, old, newName);
    showInfo(t("state.branchRenamed", { old, new: newName }));
    await Promise.all([refreshBranches(), refreshStatus()]);
  } catch (e) {
    showError(e);
  }
}

export async function deleteBranch(
  name: string,
  force: boolean,
): Promise<"ok" | "needs-force" | "error"> {
  if (!ui.repo) return "error";
  try {
    await api.deleteBranch(ui.repo.path, name, force);
    showInfo(t("state.branchDeleted", { name }));
    await refreshBranches();
    return "ok";
  } catch (e) {
    // Only "not merged" justifies the force question — other errors (e.g. the
    // current branch, I/O) are reported as errors directly.
    if (!force && (e as CommandError)?.code === "branch_not_merged") {
      return "needs-force";
    }
    showError(e);
    return "error";
  }
}

export async function mergeBranch(name: string) {
  if (!ui.repo) return;
  ui.working++;
  try {
    const out = await api.mergeBranch(ui.repo.path, name);
    showInfo(out ? firstLine(out) : t("state.branchMerged", { name }));
  } catch (e) {
    showError(e);
  } finally {
    ui.working--;
    await Promise.all([refreshStatus(), loadMoreHistory(true)]);
  }
}

export async function rebaseOnto(name: string) {
  if (!ui.repo) return;
  ui.working++;
  try {
    const out = await api.rebaseOnto(ui.repo.path, name);
    showInfo(out ? firstLine(out) : t("state.rebasedOnto", { name }));
  } catch (e) {
    showError(e);
  } finally {
    ui.working--;
    await Promise.all([refreshStatus(), loadMoreHistory(true), refreshBranches()]);
  }
}

// ============== Multi-step operations ==============

export async function abortOperation() {
  if (!ui.repo) return;
  try {
    await api.abortOperation(ui.repo.path);
    showInfo(t("state.operationAborted"));
  } catch (e) {
    showError(e);
  } finally {
    await Promise.all([refreshStatus(), loadMoreHistory(true)]);
  }
}

export async function continueOperation() {
  if (!ui.repo) return;
  try {
    const out = await api.continueOperation(ui.repo.path);
    showInfo(out ? firstLine(out) : t("state.operationContinued"));
  } catch (e) {
    showError(e);
  } finally {
    await Promise.all([refreshStatus(), loadMoreHistory(true)]);
  }
}

export async function resolveConflict(file: string, ours: boolean) {
  if (!ui.repo) return;
  try {
    await api.resolveConflict(ui.repo.path, file, ours);
    showInfo(
      ours
        ? t("state.conflictResolvedOurs", { file })
        : t("state.conflictResolvedTheirs", { file }),
    );
    await refreshStatus();
  } catch (e) {
    showError(e);
  }
}

export async function openMergetool(file: string) {
  if (!ui.repo) return;
  try {
    await api.openMergetool(ui.repo.path, file);
    await refreshStatus();
  } catch (e) {
    showError(e);
  }
}

/** Saves the content resolved in the editor and stages the file. */
export async function saveConflictResolution(file: string, content: string) {
  if (!ui.repo) return;
  ui.working++;
  try {
    await api.saveResolution(ui.repo.path, file, content);
    showInfo(t("state.conflictSaved", { file }));
  } catch (e) {
    showError(e);
  } finally {
    ui.working--;
    await refreshStatus();
  }
}

// ================ History operations ================

/**
 * Runs a history operation, reports the result as a toast and reloads.
 * `at` pins the repo path for flows that must finish where they started
 * (the switch flow) — by default the current repo.
 */
async function historyOp(label: string, op: (path: string) => Promise<unknown>, at?: string) {
  if (!ui.repo) return;
  ui.working++;
  try {
    await op(at ?? ui.repo.path);
    showInfo(label);
  } catch (e) {
    showError(e);
  } finally {
    ui.working--;
    await Promise.all([refreshStatus(), loadMoreHistory(true), refreshBranches()]);
  }
}

export const cherryPick = (id: string) =>
  historyOp(t("state.commitCherryPicked", { id: id.slice(0, 8) }), (p) => api.cherryPick(p, id));
export const revertCommit = (id: string) =>
  historyOp(t("state.commitReverted", { id: id.slice(0, 8) }), (p) => api.revertCommit(p, id));
export const undoLastCommit = () =>
  historyOp(t("state.lastCommitUndone"), (p) => api.undoLastCommit(p));
export const squashFrom = (oldestId: string, message: string) =>
  historyOp(t("state.commitsSquashed"), (p) => api.squashFrom(p, oldestId, message));
/** Check out a commit (detached HEAD) — through the same path as a branch
 *  switch so the question is asked here too. */
export async function checkoutCommit(id: string) {
  await requestSwitch({ kind: "commit", id });
}

/** Switches to another branch and cherry-picks the commit there.
 *  The switch goes through `requestSwitch` — if there is uncommitted work in the
 *  worktree, the dialog asks first and the pick hangs off it as a follow-up step.
 *  Conflicts during the pick are handled by the ConflictBanner as usual. */
export async function cherryPickOnto(commitId: string, branch: string) {
  await requestSwitch({ kind: "branch", name: branch }, { kind: "cherryPick", commitId });
}

/** Runs an interactive rebase. On conflicts the repo stays in the rebase state —
 *  the conflict banner takes over (continue/abort). */
export async function rebaseInteractive(baseId: string, steps: RebaseStep[]) {
  if (!ui.repo) return;
  ui.working++;
  try {
    await api.rebaseInteractive(ui.repo.path, baseId, steps);
    showInfo(t("state.interactiveRebaseDone"));
  } catch (e) {
    // Conflict: the repo is now in the rebase state; the banner shows continue/abort.
    showError(e);
  } finally {
    ui.working--;
    await Promise.all([refreshStatus(), loadMoreHistory(true), refreshBranches()]);
  }
}

// ================ Bisect (binary search) ================

/** Starts git bisect: `goodId` is known good, HEAD counts as bad.
 *  One-click entry from the history (pattern cherryPick/checkoutCommit). */
export async function startBisect(goodId: string) {
  if (!ui.repo || ui.working > 0 || ui.status?.opState === "bisect") return;
  ui.working++;
  try {
    const out = await api.bisectStart(ui.repo.path, goodId);
    const p = parseBisectOutput(out);
    ui.bisect = { stepsLeft: p.stepsLeft, firstBad: p.firstBad };
    await Promise.all([refreshStatus(), refreshBranches(), loadMoreHistory(true)]);
  } catch (e) {
    showError(e);
  } finally {
    ui.working--;
  }
}

/** Marks the currently checked-out bisect commit (good/bad/skip).
 *  Re-entrancy protection (ui.working): a double click on good/bad must not mark
 *  two commits with ONE verdict and thereby falsify the search. */
export async function markBisect(action: "good" | "bad" | "skip") {
  if (!ui.repo || ui.working > 0) return;
  ui.working++;
  try {
    const out = await api.bisectMark(ui.repo.path, action);
    const p = parseBisectOutput(out);
    ui.bisect = { stepsLeft: p.stepsLeft ?? ui.bisect.stepsLeft, firstBad: p.firstBad };
    await Promise.all([refreshStatus(), refreshBranches(), loadMoreHistory(true)]);
  } catch (e) {
    showError(e);
  } finally {
    ui.working--;
  }
}

/** Ends the bisect session and returns to the original branch. */
export async function resetBisect() {
  if (!ui.repo || ui.working > 0) return;
  ui.working++;
  try {
    await api.bisectReset(ui.repo.path);
    ui.bisect = { stepsLeft: null, firstBad: null };
    showInfo(t("bisect.ended"));
    await Promise.all([refreshStatus(), refreshBranches(), loadMoreHistory(true)]);
  } catch (e) {
    showError(e);
  } finally {
    ui.working--;
  }
}

// ================ Commit workshop ================

/** Opens the commit workshop and loads the unpushed commits. */
export async function openWorkshop() {
  if (!ui.repo) return;
  ui.view = "commits";
  await loadUnpushed();
}

export async function loadUnpushed(preserveEdits = false) {
  if (!ui.repo) return;
  try {
    const prev = ui.workshopEdits;
    ui.unpushed = await api.unpushedCommits(ui.repo.path);
    // Pre-fill the buffer from the original — the same parse→build pipeline as
    // buildWorkshopSteps (baselineOf parses co-authors from the body). On a
    // refresh, existing edits for still-present commits are kept so a refresh
    // never silently discards unsaved input.
    const buf: Record<string, WorkshopEdit> = {};
    for (const c of ui.unpushed) {
      buf[c.id] = preserveEdits && prev[c.id] ? prev[c.id] : baselineOf(c);
    }
    ui.workshopEdits = buf;
    // Keep the reordering only when the commit set is unchanged — with new or
    // vanished commits the natural order applies again.
    const ids = ui.unpushed.map((c) => c.id);
    const sameSet =
      preserveEdits &&
      ui.workshopOrder.length === ids.length &&
      ids.every((id) => ui.workshopOrder.includes(id));
    if (!sameSet) ui.workshopOrder = ids;
    ui.workshopError = false;
  } catch (e) {
    ui.workshopError = true;
    showError(e);
  }
}

/** Refreshes the unpushed commits without discarding unsaved edits. */
export async function refreshUnpushed() {
  await loadUnpushed(true);
}

export function cancelWorkshop() {
  ui.view = "repo";
  ui.unpushed = [];
  ui.workshopEdits = {};
  ui.workshopOrder = [];
}

/** Applies all collected changes as ONE rebase. */
export async function applyWorkshop() {
  if (!ui.repo || ui.busy) return;
  if (ui.unpushed.some((c) => c.isMerge)) {
    showError({ code: "invalid_operation", message: t("workshop.mergeBlocked") });
    return;
  }
  // Defensive pre-check: a changed, non-dropped author with an invalid field pair
  // must not trigger a rebaseInteractive call (a broken ident).
  const invalidAuthor = ui.unpushed.some((c) => {
    const e = ui.workshopEdits[c.id];
    if (!e || e.dropped) return false;
    const changedAuthor = e.authorName !== c.authorName || e.authorEmail !== c.authorEmail;
    return changedAuthor && !authorValid(e.authorName, e.authorEmail);
  });
  if (invalidAuthor) {
    showError({ code: "invalid_operation", message: t("workshop.authorInvalid") });
    return;
  }
  // Mirrors the engine validation: the oldest kept step must not be a squash
  // (there is no older commit in the range for it to fall into).
  if (firstKeptIsSquash(ui.unpushed, ui.workshopEdits, ui.workshopOrder)) {
    showError({ code: "invalid_operation", message: t("rebase.warnFirstSquash") });
    return;
  }
  const built = buildWorkshopSteps(ui.unpushed, ui.workshopEdits, ui.workshopOrder);
  if (!built) return; // nothing to do
  ui.busy = t("workshop.applying");
  try {
    await api.rebaseInteractive(ui.repo.path, built.baseId, built.steps);
    showInfo(t("workshop.applied"));
    cancelWorkshop();
    await Promise.all([refreshStatus(), refreshBranches(), loadMoreHistory(true)]);
  } catch (e) {
    showError(e);
    // Reload the status to distinguish: a real rebase conflict (the banner takes
    // over, close the workshop) vs. another error (e.g. an invalid author) — then
    // the repo stays clean and the workshop stays open so the user can correct
    // their input instead of losing all edits.
    await refreshStatus();
    if (ui.status?.opState === "rebase") {
      cancelWorkshop();
      await Promise.all([refreshBranches(), loadMoreHistory(true)]);
    }
  } finally {
    ui.busy = null;
  }
}

/** Uncommit of the topmost (HEAD) commit: changes go back to staging. */
export async function uncommitTop() {
  if (!ui.repo || ui.busy) return;
  ui.busy = t("workshop.uncommitting");
  try {
    await api.undoLastCommit(ui.repo.path);
    showInfo(t("workshop.uncommitted"));
    cancelWorkshop();
    await Promise.all([refreshStatus(), refreshBranches(), loadMoreHistory(true)]);
  } catch (e) {
    showError(e);
  } finally {
    ui.busy = null;
  }
}

// ================ Pipeline cockpit ================

/**
 * Re-determines runner/Docker availability.
 *
 * Has to be callable at EVERY point where the status could be stale: previously
 * `pipeline_detect` only ran when entering the view, so whoever started Docker
 * afterwards kept seeing the "Docker is not running" hint for the rest of the
 * session — with no way to check again.
 *
 * Swallows errors deliberately: a failed status check is a display problem and
 * must never prevent a run.
 */
export async function refreshPipelineTools(): Promise<void> {
  if (!ui.repo) return;
  try {
    ui.pipeline.info = await api.pipelineDetect(ui.repo.path);
  } catch {
    // The status stays as it was.
  }
}

export async function openPipeline() {
  if (!ui.repo) return;
  if (ui.pipeline.running) {
    // An active run keeps streaming events: only switch the view, otherwise
    // selectPipelineConfig() would reset statuses/logs/activeLog/graph and
    // irrecoverably discard already collected log lines.
    ui.view = "pipeline";
    return;
  }
  ui.view = "pipeline";
  const p = ui.pipeline;
  p.error = false;
  try {
    await refreshPipelineTools();
    p.configs = await api.pipelineConfigs(ui.repo.path);
    const keep = p.selected && p.configs.some((c) => c.path === p.selected);
    await selectPipelineConfig(keep ? p.selected! : (p.configs[0]?.path ?? null));
  } catch (e) {
    p.error = true;
    showError(e);
  }
}

export async function selectPipelineConfig(path: string | null) {
  if (!ui.repo) return;
  // Sequence token (pattern statusSeq): a slow graph answer of the old selection
  // must not overwrite the newer selection (neither graph nor error).
  const seq = ++pipeGraphSeq;
  const p = ui.pipeline;
  p.selected = path;
  p.graph = null;
  p.statuses = {};
  p.logs = {};
  p.logSeq = 0;
  p.activeLog = null;
  p.exit = null;
  p.canceled = false;
  const cfg = p.configs.find((c) => c.path === path);
  // NO runner gate here: that would hang off the auto-detected provider instead
  // of the chosen config (and an early return would leave the view stuck in the
  // loading spinner). The backend checks against the REQUESTED provider and
  // returns the stable code runner_not_installed -> error state with retry.
  if (!cfg) return;
  try {
    p.error = false;
    const graph = await api.pipelineGraph(ui.repo.path, cfg.provider, cfg.path);
    if (seq === pipeGraphSeq) p.graph = graph;
  } catch (e) {
    if (seq === pipeGraphSeq) {
      p.error = true;
      showError(e);
    }
  }
}

/** Choose a CI file manually through the file picker and take it over as the
 *  configuration (user request: "find the master file manually"). The backend
 *  derives the repo-relative path + provider and checks the security guards. */
export async function addPipelineConfigFile() {
  if (!ui.repo) return;
  const p = ui.pipeline;
  try {
    const file = await open({
      title: t("pipeline.chooseFileTitle"),
      filters: [{ name: "YAML", extensions: ["yml", "yaml"] }],
    });
    if (typeof file !== "string") return; // cancelled
    const cfg = await api.pipelineAddConfig(ui.repo.path, file);
    if (!p.configs.some((c) => c.path === cfg.path && c.provider === cfg.provider)) {
      p.configs = [...p.configs, cfg];
    }
    await selectPipelineConfig(cfg.path);
  } catch (e) {
    showError(e);
  }
}

export async function runPipelineScope(scope: "pipeline" | "stage" | "job", target: string | null) {
  const p = ui.pipeline;
  const cfg = p.configs.find((c) => c.path === p.selected);
  if (!ui.repo || !cfg || p.running || !p.graph) return;
  // Carry repoPath along in the run: cancelling has to hit the RUN, not whatever
  // repo happens to be open when "Cancel" is clicked.
  const repoPath = ui.repo.path;
  p.running = { scope, target, repoPath };
  p.exit = null;
  p.canceled = false;
  p.statuses = {};
  p.logs = {};
  p.logSeq = 0;
  // Point the log drawer at the run: a single job -> its log,
  // stage/pipeline -> the overall log.
  p.activeLog = scope === "job" ? target : null;
  // The user is acting NOW — so re-determine the tool status now, so that a
  // Docker started in the meantime clears the hint.
  await refreshPipelineTools();
  try {
    // Only pass the event for act/GitHub; GitLab has no trigger events.
    const event = cfg.provider === "gitlab" ? null : p.event;
    // Drop variables with an empty key; the backend filters by allowlist as well.
    const variables: [string, string][] = p.variables
      .filter((v) => v.key.trim() !== "")
      .map((v) => [v.key.trim(), v.value]);
    const code = await api.pipelineRunScope(
      repoPath,
      cfg.provider,
      cfg.path,
      scope,
      target,
      event,
      variables,
      (ev) => applyPipelineEvent(p, ev),
    );
    p.exit = code;
  } catch (e) {
    showError(e);
  } finally {
    p.running = null;
  }
}

export async function cancelPipelineRun() {
  const p = ui.pipeline;
  // Repo path OF THE RUN (fallback: the current repo) — after a repo switch the
  // cancellation would otherwise hit the wrong repo.
  const repoPath = p.running?.repoPath ?? ui.repo?.path;
  if (!repoPath) return;
  const ok = await api.pipelineCancel(repoPath).catch(() => false);
  if (ok) {
    p.canceled = true;
    showInfo(t("pipe.canceled"));
  } else {
    showError({ code: "pipeline_cancel_failed", message: t("pipe.cancelFailed") });
  }
}

export function closePipeline() {
  ui.view = "repo";
}

export async function branchFromCommit(name: string, commitId: string) {
  if (!ui.repo) return;
  try {
    await api.createBranchFromCommit(ui.repo.path, name, commitId, true);
    showInfo(t("state.branchFromCommitCreated", { name, id: commitId.slice(0, 8) }));
    await Promise.all([refreshStatus(), refreshBranches(), loadMoreHistory(true)]);
  } catch (e) {
    showError(e);
  }
}

export async function runSearch() {
  if (!ui.repo) return;
  const q = ui.searchQuery.trim();
  // ALWAYS take a token: even an empty search invalidates one still running — a
  // cleared field must not be refilled by its late answer.
  const seq = ++searchSeq;
  const path = ui.repo.path;
  if (!q) {
    ui.searchResults = null;
    return;
  }
  try {
    const results = await api.searchLog(path, q, 200);
    // Late answer, a different repo, or the search has changed meanwhile: discard.
    if (seq !== searchSeq || ui.repo?.path !== path || ui.searchQuery.trim() !== q) return;
    ui.searchResults = results;
  } catch (e) {
    if (seq === searchSeq && ui.repo?.path === path) showError(e);
  }
}

// ================ Hunk/line staging ================

async function reloadSelectedDiff() {
  if (ui.selectedFile) {
    await selectFile(ui.selectedFile.path, ui.selectedFile.staged);
  }
  await refreshStatus();
}

export async function applyHunk(file: string, hunkIndex: number, unstage: boolean) {
  if (!ui.repo) return;
  ui.working++;
  try {
    await api.applyHunk(ui.repo.path, file, hunkIndex, unstage);
    await reloadSelectedDiff();
  } catch (e) {
    showError(e);
  } finally {
    ui.working--;
  }
}

export async function discardHunk(file: string, hunkIndex: number) {
  if (!ui.repo) return;
  ui.working++;
  try {
    await api.discardHunk(ui.repo.path, file, hunkIndex);
    await reloadSelectedDiff();
  } catch (e) {
    showError(e);
  } finally {
    ui.working--;
  }
}

export async function applyLines(
  file: string,
  hunkIndex: number,
  lineIndices: number[],
  unstage: boolean,
) {
  if (!ui.repo) return;
  ui.working++;
  try {
    await api.applyLines(ui.repo.path, file, hunkIndex, lineIndices, unstage);
    await reloadSelectedDiff();
  } catch (e) {
    showError(e);
  } finally {
    ui.working--;
  }
}

// ================ Repo lifecycle & misc ================

export async function refreshRemotes() {
  if (!ui.repo) return;
  const seq = ++remotesSeq;
  const path = ui.repo.path;
  try {
    const remotes = await api.remotes(path);
    // Late answer or a different/no repo in the meantime: discard it.
    if (seq !== remotesSeq || ui.repo?.path !== path) return;
    ui.remotes = remotes;
  } catch {
    if (seq === remotesSeq && ui.repo?.path === path) ui.remotes = [];
  }
}

export async function addRemote(name: string, url: string) {
  if (!ui.repo) return;
  try {
    await api.addRemote(ui.repo.path, name, url);
    showInfo(t("state.remoteAdded", { name }));
    await refreshRemotes();
  } catch (e) {
    showError(e);
  }
}

/** Removes a remote. Tracking refs disappear with it → reload branches/status. */
export async function removeRemote(name: string) {
  if (!ui.repo) return;
  try {
    await api.removeRemote(ui.repo.path, name);
    showInfo(t("state.remoteRemoved", { name }));
    await Promise.all([refreshRemotes(), refreshBranches(), refreshStatus()]);
  } catch (e) {
    showError(e);
  }
}

export async function renameRemote(oldName: string, newName: string) {
  if (!ui.repo) return;
  try {
    await api.renameRemote(ui.repo.path, oldName, newName);
    showInfo(t("state.remoteRenamed", { old: oldName, new: newName }));
    await Promise.all([refreshRemotes(), refreshBranches(), refreshStatus()]);
  } catch (e) {
    showError(e);
  }
}

export async function setRemoteUrl(name: string, url: string) {
  if (!ui.repo) return;
  try {
    await api.setRemoteUrl(ui.repo.path, name, url);
    showInfo(t("state.remoteUrlChanged", { name }));
    await refreshRemotes();
  } catch (e) {
    showError(e);
  }
}

// ================= Multi-level undo/redo =================

async function refreshUndoStatus() {
  if (!ui.repo) return;
  try {
    ui.undoStatus = await api.undoStatus(ui.repo.path);
  } catch {
    ui.undoStatus = null;
  }
}

/** Display label of an undo entry ("Merge “feature/x”"). */
export function undoLabel(e: UndoEntry | null | undefined): string {
  if (!e) return "";
  const name = t(`undo.op.${e.op}` as Parameters<typeof t>[0]);
  return e.detail ? `${name} “${e.detail}”` : name;
}

export async function undoLast() {
  if (!ui.repo || ui.busy || ui.working > 0) return;
  ui.working++;
  try {
    const entry = await api.undoLast(ui.repo.path);
    showInfo(t("undo.done", { label: undoLabel(entry) }));
    await Promise.all([
      refreshStatus(),
      refreshBranches(),
      loadMoreHistory(true),
      refreshStashes(),
    ]);
  } catch (e) {
    showError(e);
  } finally {
    ui.working--;
  }
}

export async function redoLast() {
  if (!ui.repo || ui.busy || ui.working > 0) return;
  ui.working++;
  try {
    const entry = await api.redoLast(ui.repo.path);
    showInfo(t("undo.redone", { label: undoLabel(entry) }));
    await Promise.all([
      refreshStatus(),
      refreshBranches(),
      loadMoreHistory(true),
      refreshStashes(),
    ]);
  } catch (e) {
    showError(e);
  } finally {
    ui.working--;
  }
}

// ================= Provider accounts =================

export async function refreshAccounts() {
  try {
    ui.accounts = await api.providerAccounts();
  } catch {
    ui.accounts = [];
  }
}

/** Validates the token with the provider and stores it in the OS keychain. */
export async function addProviderAccount(
  host: string,
  kind: ProviderKind,
  token: string,
  insecureTls: boolean,
): Promise<boolean> {
  try {
    const acc = await api.providerAddAccount(host, kind, token, insecureTls);
    showInfo(t("state.accountAdded", { host: acc.host, user: acc.username }));
    await refreshAccounts();
    return true;
  } catch (e) {
    showError(e);
    return false;
  }
}

export async function removeProviderAccount(host: string) {
  try {
    await api.providerRemoveAccount(host);
    showInfo(t("state.accountRemoved", { host }));
    await refreshAccounts();
  } catch (e) {
    showError(e);
  }
}

// ================= SSH key manager =================

/** Loads the local SSH keys (~/.ssh/*.pub) for the settings. */
export async function loadSshKeys() {
  try {
    ui.sshKeys = await api.sshListKeys();
  } catch (e) {
    showError(e);
  }
}

/** Creates a new ed25519 key and reloads the list. */
export async function generateSshKey(name: string, comment: string, passphrase: string) {
  try {
    const key = await api.sshGenerateKey(name, comment, passphrase);
    showInfo(t("state.sshKeyGenerated", { name: key.name }));
    await loadSshKeys();
  } catch (e) {
    showError(e);
  }
}

/** Removes an SSH key. The backend confirms through a native OS dialog and moves
 *  the key pair to the trash; afterwards the list is reloaded. Cancelling in the
 *  dialog (code "cancelled") is not an error. */
export async function removeSshKey(name: string) {
  try {
    await api.sshRemoveKey(name);
    showInfo(t("state.sshKeyRemoved", { name }));
    await loadSshKeys();
  } catch (e) {
    if ((e as { code?: string })?.code === "cancelled") return;
    showError(e);
  }
}

/** Confirms the host key shown in the TOFU dialog (known_hosts entry) and then
 *  repeats the remote operation that originally failed. */
export async function confirmSshTrust() {
  const m = ui.modal;
  if (m?.kind !== "sshTofu") return;
  const scan = m.scan;
  const { host, port } = m;
  const retry = pendingSshRetry;
  ui.modal = null;
  pendingSshRetry = null;
  try {
    await api.sshTrustHost(host, port, scan.knownHostsLines, scan.changed);
    showInfo(t("state.sshTrusted"));
    if (retry) await retry();
  } catch (e) {
    showError(e);
  }
}

/** Cancels the TOFU dialog without trusting the host key. */
export function cancelSshTofu() {
  ui.modal = null;
  pendingSshRetry = null;
}

// ================= Backups (backup refs) =================

/** Hard-resets the current branch to a backup (the restore is itself backed up). */
export async function restoreBackup(refName: string): Promise<boolean> {
  if (!ui.repo) return false;
  ui.working++;
  try {
    const id = await api.restoreBackup(ui.repo.path, refName);
    showInfo(t("state.backupRestored", { id: id.slice(0, 8) }));
    await Promise.all([refreshStatus(), refreshBranches(), loadMoreHistory(true)]);
    return true;
  } catch (e) {
    showError(e);
    return false;
  } finally {
    ui.working--;
  }
}

/**
 * Cloning in two stages (user request: create/open immediately, then the data):
 * 1. `clone_prepare` creates the repo + remote → it is opened IMMEDIATELY,
 * 2. `clone_fetch` fetches the data with progress (CloneView overlay).
 * If (2) fails, the repo stays open as an empty local repo.
 */
export async function cloneRepository(
  url: string,
  destDir: string,
  options: CloneOptions,
): Promise<boolean> {
  let info: RepoInfo;
  try {
    info = await api.clonePrepare(url, destDir);
  } catch (e) {
    showError(e);
    return false;
  }

  // Open immediately: the (empty) repo view appears, the download keeps running
  // in the background and shows its progress as a non-blocking banner
  // (CloneView). ui.cloning drives the banner.
  ui.cloning = info.name;
  ui.progress = null;
  await openRepo(info.path);

  // Do NOT await the data -> cloneRepository returns immediately, the clone
  // dialog closes and the repo view is usable. cloneFetchPhase encapsulates
  // success/error/TOFU/cancel itself (including the ui.cloning reset in finally).
  void cloneFetchPhase(info, url, options);
  return true;
}

/**
 * Clone stage 2 (fetching the data) including TOFU. On an unknown/changed SSH
 * host key the fingerprint dialog is shown — as with fetch/pull/push — instead of
 * just a raw error message that used to force the user to the git CLI; after
 * confirmation `cloneFetch` is repeated automatically. The host is derived from
 * the clone URL (parseSshHost). Deliberately exported as its own unit,
 * independent of `openRepo` (state test of the clone TOFU route).
 */
export async function cloneFetchPhase(
  info: RepoInfo,
  url: string,
  options: CloneOptions,
): Promise<boolean> {
  ui.cloning = info.name;
  ui.progress = null;
  try {
    await api.cloneFetch(info.path, options, (p) => (ui.progress = p));
    showInfo(t("state.cloned", { name: info.name }));
    await Promise.all([refreshStatus(), refreshBranches(), loadMoreHistory(true)]);
    return true;
  } catch (e) {
    // Cancelled clone: a neutral notice instead of an error toast.
    if ((e as CommandError)?.code === "cancelled") {
      showInfo(t("state.opCancelled", { label: t("clone.cloning", { name: info.name }) }));
      return false;
    }
    // A new/unknown SSH host while cloning: show the same TOFU dialog;
    // retry = another cloneFetch (the repo is already open).
    if ((e as CommandError)?.code === "host_key") {
      const parsed = parseSshHost(url);
      if (parsed) {
        await handleHostKey(parsed.host, parsed.port, async () => {
          await cloneFetchPhase(info, url, options);
        });
        return false;
      }
    }
    showError(e);
    return false;
  } finally {
    ui.cloning = null;
    ui.progress = null;
  }
}

export async function initRepository(dir: string): Promise<boolean> {
  try {
    const info = await api.initRepository(dir);
    showInfo(t("state.repoInitialized", { name: info.name }));
    await openRepo(info.path);
    return true;
  } catch (e) {
    showError(e);
    return false;
  }
}

export async function ignoreFile(pattern: string) {
  if (!ui.repo) return;
  try {
    await api.ignorePattern(ui.repo.path, pattern);
    showInfo(t("state.addedToGitignore", { pattern }));
    await refreshStatus();
  } catch (e) {
    showError(e);
  }
}

export async function showBlame(file: string) {
  if (!ui.repo) return;
  try {
    const lines = await api.blameFile(ui.repo.path, file);
    ui.blame = { file, lines };
    ui.modal = { kind: "blame", file };
  } catch (e) {
    showError(e);
  }
}

export async function loadImageDiff(file: string, staged: boolean, seq?: number) {
  if (!ui.repo) return;
  ui.imageDiff = null;
  try {
    const img = await api.imageDiff(ui.repo.path, file, staged);
    // Race guard as for the text diff: a newer file selection (fileDiffSeq) must
    // not be overwritten by the later-arriving answer of an older (slowly
    // base64-encoded) image selection.
    if (seq === undefined || seq === fileDiffSeq) ui.imageDiff = img;
  } catch {
    if (seq === undefined || seq === fileDiffSeq) ui.imageDiff = null;
  }
}

/** Provider detection: returns the web URL of the first remote (GitHub/GitLab/self-hosted). */
export function remoteWebUrl(): string | null {
  const url = ui.remotes[0]?.url;
  if (!url) return null;
  // ssh: git@host:path.git -> https://host/path
  const ssh = url.match(/^(?:ssh:\/\/)?git@([^:/]+)[:/](.+?)(?:\.git)?$/);
  if (ssh) return `https://${ssh[1]}/${ssh[2]}`;
  // Keep the protocol (http/https) — self-hosted GitLab sometimes runs over http only.
  const web = url.match(/^(https?):\/\/(.+?)(?:\.git)?$/);
  if (web) return `${web[1]}://${web[2]}`;
  return null;
}

/** Provider family of the first remote — drives the PR/MR URL scheme and wording.
 *  Heuristic: GitHub exactly, the Gitea family through host names (Codeberg/
 *  gitea/forgejo); everything else terra-git treats as GitLab (self-hosted focus). */
export function prProvider(): "github" | "gitea" | "gitlab" {
  const host = (remoteWebUrl() ?? "")
    .replace(/^https?:\/\//, "")
    .split("/")[0]
    .toLowerCase();
  // A stored account for this host wins (self-hosted Gitea/GitLab with an
  // arbitrary host name), otherwise the host-name heuristic.
  const acc = ui.accounts.find((a) => a.host.toLowerCase() === host);
  if (acc) return acc.kind;
  if (host === "github.com") return "github";
  if (host === "codeberg.org" || host.includes("gitea") || host.includes("forgejo")) {
    return "gitea";
  }
  return "gitlab";
}

/** URL for "create PR/MR" depending on the provider (GitHub/Gitea vs GitLab scheme). */
export function createPrUrl(): string | null {
  const base = remoteWebUrl();
  const branch = ui.status?.branch;
  if (!base || !branch) return null;
  switch (prProvider()) {
    case "github":
      return `${base}/compare/${encodeURIComponent(branch)}?expand=1`;
    case "gitea":
      // Gitea/Forgejo/Codeberg: GitHub-like compare scheme (base = default branch)
      return `${base}/compare/${encodeURIComponent(branch)}`;
    default:
      // GitLab (self-hosted included): merge-request scheme
      return `${base}/-/merge_requests/new?merge_request%5Bsource_branch%5D=${encodeURIComponent(branch)}`;
  }
}
