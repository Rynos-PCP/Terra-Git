// Branch switching with uncommitted changes: the decision "bring along or
// leave here?" and recognizing changes that were left behind.
//
// Pure functions (same pattern as conflictOffer.ts / repoReset.ts): typed
// structurally instead of importing from state.svelte.ts — no cycle, testable
// without runes. The wiring (stash, switch, apply) lives in state.svelte.ts.

/**
 * Where a checkout goes: onto a branch or onto a commit (detached HEAD).
 *
 * Both cases share the same path through the app — question dialog, auto stash,
 * error handling — but differ in the command and in the label. A tagged value
 * instead of two parallel functions: that way no caller can extend one path and
 * forget the other — exactly the bug this guards against.
 */

export type SwitchTarget = { kind: "branch"; name: string } | { kind: "commit"; id: string };

/** How the target is named in texts: branch name or short id. */
export function switchTargetLabel(target: SwitchTarget): string {
  return target.kind === "branch" ? target.name : target.id.slice(0, 8);
}

/** Only the status fields the decision needs. */
export interface SwitchStatus {
  staged: unknown[];
  unstaged: unknown[];
  opState: string;
}

/** Only the stash fields the recognition needs. */
export interface StashEntry {
  index: number;
  message: string;
}

/**
 * Marker in the stash text by which a change left behind is assigned to its
 * branch.
 *
 * DELIBERATELY not translated: the marker is read again when switching back — a
 * catalog text would no longer be recognizable after a language change (and the
 * user would have silently lost their changes).
 * The READABLE version is built from it by the UI (modals.autoStashLabel).
 */
export const AUTOSTASH_MARKER = "terra-git-autostash:";

/** Stash text for changes that should stay behind on `branch`. */
export function autoStashMessage(branch: string): string {
  return `${AUTOSTASH_MARKER}${branch}`;
}

/**
 * Which branch does this stash belong to — or to none (null)?
 *
 * Searches for the marker ANYWHERE in the text, because `git stash push -m`
 * prepends an "On <branch>: " of its own; the rest of the line is the branch
 * name (which may contain practically any character except the line ending).
 */
export function parseAutoStashBranch(message: string): string | null {
  const at = message.indexOf(AUTOSTASH_MARKER);
  if (at < 0) return null;
  const branch = message.slice(at + AUTOSTASH_MARKER.length).trim();
  return branch === "" ? null : branch;
}

/**
 * The stash most recently left behind for `branch` (git lists newest first,
 * index 0), or null.
 */
export function findAutoStash<T extends StashEntry>(stashes: T[], branch: string): T | null {
  return stashes.find((s) => parseAutoStashBranch(s.message) === branch) ?? null;
}

/** Is there anything uncommitted in the worktree at all? */
export function worktreeDirty(status: SwitchStatus | null | undefined): boolean {
  if (!status) return false;
  return status.staged.length + status.unstaged.length > 0;
}

/**
 * Does the user have to be asked where the changes belong before switching?
 *
 * Only with a clean `opState`: while a multi-step operation is running (merge,
 * rebase, cherry-pick …) git refuses the switch anyway — the question would be a
 * dead end then, and the real error is the more helpful answer.
 * Without a status (not loaded yet) nothing is asked: the checkout itself is the
 * safe path, it refuses on collisions by itself.
 */
export function needsSwitchChoice(status: SwitchStatus | null | undefined): boolean {
  if (!status || status.opState !== "clean") return false;
  return worktreeDirty(status);
}
