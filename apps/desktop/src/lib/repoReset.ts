// Shared reset of the repo-bound state — the ONE source of truth for openRepo
// (repo switch) and closeRepo (back to the welcome screen).
//
// It used to be duplicated, and the two copies had drifted apart: closeRepo left
// stashes, tags, remotes, undoStatus, blame, imageDiff, searchQuery/searchResults,
// messageLog, modal and historyComplete standing. That duplication is
// resolved here instead of adding the missing assignments — otherwise the next
// new field would drift apart again.
//
//
// Typed structurally instead of importing from state.svelte.ts (same pattern as
// resetPipelineOnRepoSwitch in pipelineModel.ts): no cycle, testable without
// runes.
import { resetPipelineOnRepoSwitch, type PipelineSlice } from "./pipelineModel";

/** Only the fields this reset touches — deliberately kept narrow. */
export interface RepoSlices {
  view: string;
  pipeline: PipelineSlice;
  status: unknown;
  branches: unknown[];
  history: unknown[];
  historyComplete: boolean;
  historyPreparing: boolean;
  selectedFile: unknown;
  numstat: unknown;
  fileDiff: unknown;
  unchangedInfo: unknown;
  imageDiff: unknown;
  selectedCommit: unknown;
  commitDiff: unknown[];
  commitDiffTotal: number | null;
  stashes: unknown[];
  tags: unknown[];
  remotes: unknown[];
  searchQuery: string;
  searchResults: unknown;
  messageLog: string[];
  undoStatus: unknown;
  blame: unknown;
  unpushed: unknown[];
  workshopEdits: Record<string, unknown>;
  workshopOrder: string[];
  workshopError: boolean;
  bisect: { stepsLeft: number | null; firstBad: string | null };
  modal: unknown;
  errorAction: unknown;
}

/**
 * Clears everything that belongs to the opened repository.
 *
 * DELIBERATELY NOT touched because it is not repo-bound:
 * - `repo`, `tab` — set by the callers themselves (openRepo needs the new repo
 *   before the reset, closeRepo sets null).
 * - `recents`, `accounts`, `sshKeys` — application-wide, they outlive the repo.
 * - all settings (theme, diffMode, changesView, editorCmd, autoFetch,
 *   pruneOnPull, toastDuration, panel widths, uiScale, reduceMotion,
 *   highContrast) — user preferences, not repo state.
 * - `working`, `busy`, `busyCancellable`, `progress`, `cloning`,
 *   `historyLoading` — they belong to running operations, not to the repo;
 *   nulling them here would only hide a running operation, not end it.
 * - `error`, `info` — messages should survive a repo switch (e.g. the error
 *   that led to the close). The ACTION attached to them (`errorAction`) is
 *   repo-bound, however, and is cleared below.
 */
export function resetRepoSlices(ui: RepoSlices): void {
  ui.status = null;
  ui.branches = [];
  ui.history = [];
  ui.historyComplete = false;
  ui.historyPreparing = false;
  ui.selectedFile = null;
  ui.numstat = null;
  ui.fileDiff = null;
  ui.unchangedInfo = null;
  ui.imageDiff = null;
  ui.selectedCommit = null;
  ui.commitDiff = [];
  ui.commitDiffTotal = null;
  ui.stashes = [];
  ui.tags = [];
  ui.remotes = [];
  ui.searchQuery = "";
  ui.searchResults = null;
  ui.messageLog = [];
  ui.undoStatus = null;
  ui.blame = null;
  ui.unpushed = [];
  ui.workshopEdits = {};
  ui.workshopOrder = [];
  ui.workshopError = false;
  ui.bisect = { stepsLeft: null, firstBad: null };
  ui.modal = null;
  // The workshop offer of the error toast points into the workshop of the
  // CURRENT repo — in the next repo it would be misleading (the message itself
  // may stay, see the doc above).
  ui.errorAction = null;
  // The pipeline cockpit belongs to the old repo: initialize the slice and close
  // an open pipeline view. The view semantics stay in ONE place
  // (pipelineModel.ts).
  resetPipelineOnRepoSwitch(ui);
}
