import { buildCommitMessage, parseCommitMessage } from "./commitMessage";
import type { RebaseStep, UnpushedCommit } from "./api";

export interface WorkshopEdit {
  subject: string;
  body: string; // body WITHOUT the co-author trailer
  coAuthors: string; // comma-separated, as in the commit box
  authorName: string;
  authorEmail: string;
  dropped: boolean;
  /** Fold into the older neighbouring commit (git squash). dropped wins. */
  squashed: boolean;
}

/**
 * Original values of a commit as the editing starting point — MUST go through
 * the same parse→build pipeline as `loadUnpushed`, otherwise the unchanged
 * comparison wrongly reports a change (co-author trailer).
 */
export function baselineOf(commit: UnpushedCommit): WorkshopEdit {
  const p = parseCommitMessage(`${commit.subject}\n\n${commit.body}`);
  return {
    subject: commit.subject,
    body: p.description,
    coAuthors: p.coAuthors,
    authorName: commit.authorName,
    authorEmail: commit.authorEmail,
    dropped: false,
    squashed: false,
  };
}

/** Built target message of an edit buffer. */
function builtMessage(e: WorkshopEdit): string {
  return buildCommitMessage(e.subject, e.body, e.coAuthors);
}

function changed(commit: UnpushedCommit, e: WorkshopEdit): boolean {
  if (e.dropped || e.squashed) return true;
  const base = baselineOf(commit);
  return (
    builtMessage(e) !== builtMessage(base) ||
    e.authorName !== base.authorName ||
    e.authorEmail !== base.authorEmail
  );
}

/**
 * Does this one commit have a change against its baseline (message/author
 * changed OR dropped)? The basis for the "changed" marker per card in the
 * workshop list.
 */
export function commitChanged(commit: UnpushedCommit, edit: WorkshopEdit | undefined): boolean {
  return changed(commit, edit ?? baselineOf(commit));
}

/**
 * Number of commits with an actual change (changed OR dropped). The basis for
 * the pending counter and the enable logic — independent of whether
 * buildWorkshopSteps ends up producing a `pick` step for unchanged neighbouring
 * commits.
 */
export function changedCommitCount(
  commits: UnpushedCommit[],
  edits: Record<string, WorkshopEdit>,
): number {
  return commits.reduce((n, c) => n + (commitChanged(c, edits[c.id]) ? 1 : 0), 0);
}

/**
 * Validity of an author field pair: name and email (trimmed) non-empty and free
 * of "<"/">" (which are reserved as separators in `Name <mail>`).
 */
export function authorValid(name: string, email: string): boolean {
  const n = name.trim();
  const m = email.trim();
  if (!n || !m) return false;
  if (n.includes("<") || n.includes(">")) return false;
  if (m.includes("<") || m.includes(">")) return false;
  return true;
}

/**
 * Validates a display order (newest first) against the commit set; on gaps or
 * foreign ids the natural order applies.
 */
function effectiveOrder(commits: UnpushedCommit[], order?: string[]): string[] {
  const ids = commits.map((c) => c.id);
  if (!order || order.length !== ids.length) return ids;
  const set = new Set(ids);
  return order.every((id) => set.has(id)) && new Set(order).size === order.length ? order : ids;
}

/** Does the display order deviate from the natural commit order? */
export function workshopOrderChanged(commits: UnpushedCommit[], order?: string[]): boolean {
  const eff = effectiveOrder(commits, order);
  return commits.some((c, i) => c.id !== eff[i]);
}

/**
 * Application order (oldest first) WITHOUT the root commit, which stays
 * read-only as the base and is pinned to the end of the list in the UI.
 */
function rewritableIds(commits: UnpushedCommit[], order?: string[]): string[] {
  const oldest = commits[commits.length - 1];
  const rootInRange = oldest.parentIds.length === 0;
  return [...effectiveOrder(commits, order)]
    .reverse()
    .filter((id) => !(rootInRange && id === oldest.id));
}

/**
 * The oldest KEPT (non-drop) step must not be a squash — there is no older
 * commit in the range for it to fall into. Mirrors the engine validation ("the
 * first non-drop action has to be pick") into the UI.
 */
export function firstKeptIsSquash(
  commits: UnpushedCommit[],
  edits: Record<string, WorkshopEdit>,
  order?: string[],
): boolean {
  if (commits.length === 0) return false;
  for (const id of rewritableIds(commits, order)) {
    const c = commits.find((x) => x.id === id)!;
    const e = edits[id] ?? baselineOf(c);
    if (e.dropped) continue;
    return !!e.squashed;
  }
  return false;
}

/**
 * Builds rebase steps for the ENTIRE unpushed range (data-loss protection).
 * `commits` is newest-first; `order` is the (possibly reordered) display order
 * and becomes the new application order. Returns `null` when there is nothing to
 * do. base = first parent of the oldest commit; if the oldest one is a root (no
 * parents), it becomes the base (read-only) and falls out of the steps.
 */
export function buildWorkshopSteps(
  commits: UnpushedCommit[],
  edits: Record<string, WorkshopEdit>,
  order?: string[],
): { baseId: string; steps: RebaseStep[] } | null {
  if (commits.length === 0) return null;
  const anyChange =
    workshopOrderChanged(commits, order) ||
    commits.some((c) => changed(c, edits[c.id] ?? baselineOf(c)));
  if (!anyChange) return null;

  const oldest = commits[commits.length - 1];
  const rootInRange = oldest.parentIds.length === 0;
  const baseId = rootInRange ? oldest.id : oldest.parentIds[0];

  const byId = new Map(commits.map((c) => [c.id, c]));
  const steps: RebaseStep[] = rewritableIds(commits, order).map((id) => {
    const c = byId.get(id)!;
    const e = edits[c.id] ?? baselineOf(c);
    if (e.dropped) return { action: "drop", commitId: c.id };
    // Squash folds the commit AND the message into its predecessor — this
    // commit's own message/author edits are then deliberately irrelevant.
    if (e.squashed) return { action: "squash", commitId: c.id };
    const msgChanged = builtMessage(e) !== builtMessage(baselineOf(c));
    const authorChanged = e.authorName !== c.authorName || e.authorEmail !== c.authorEmail;
    const author = authorChanged ? `${e.authorName} <${e.authorEmail}>` : undefined;
    if (msgChanged) return { action: "reword", commitId: c.id, message: builtMessage(e), author };
    if (author) return { action: "pick", commitId: c.id, author };
    return { action: "pick", commitId: c.id };
  });
  return { baseId, steps };
}
