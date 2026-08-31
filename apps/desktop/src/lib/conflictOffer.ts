// Workshop offer in the error toast: detects conflict candidates and decides
// when the toast may offer the jump into the conflict workshop.
//
//
// The question "is an operation with two sides running?" is answered by
// `workshopAvailable` (conflictWorkshop.ts) — the same condition drives the
// entry in the tools menu and the one in the command palette.
//
// Two pure functions (same pattern as conflictWorkshop.ts): candidate detection
// runs when the error is reported (state.showError), the display decision runs
// reactively in the toast — so the offer disappears by itself as soon as the
// workshop no longer applies (operation finished, repo switched or closed).
//

import { workshopAvailable } from "./conflictWorkshop";

/**
 * CAN this backend error be a conflict?
 *
 * Deliberately a candidate check, not a reliable detection: git localizes its
 * messages by system locale ("CONFLICT" / "CONFLIT" / "КОНФЛИКТ" …), so a text
 * match would only be dependable for English. Therefore every error path on
 * which conflicts really occur counts as a candidate — whether the offer REALLY
 * appears is decided solely by the structural gate offerConflictWorkshop
 * (running operation + open conflicted files):
 * - `merge_conflict`: classified pull error (sidecar.rs, English only).
 * - `sidecar_failed`: merge/rebase/cherry-pick/revert through the sidecar —
 *   raw, locale-dependent git output.
 * - `git_error`: libgit2 error, e.g. "n conflicts prevent checkout".
 *   (Stash apply/pop also goes through libgit2 — but a stash conflict sets no
 *   multi-step state, opState stays "clean", so the gate deliberately blocks
 *   there: the workshop would bounce straight back.)
 * - A text match as a safety net for the remaining paths with an English
 *   message.
 */
export function isConflictCandidate(
  code: string | undefined,
  message: string | undefined,
): boolean {
  if (code === "merge_conflict" || code === "sidecar_failed" || code === "git_error") return true;
  return /conflict|konflikt/i.test(message ?? "");
}

/**
 * May the error toast offer the conflict workshop?
 *
 * Only when the workshop really helps right now: a multi-step operation is
 * running (the workshop bounces straight back to the workspace on `clean`) and
 * there are open conflicted files — the same condition as the workshop button in
 * the ConflictBanner. Inside the workshop itself the offer is pointless. The
 * status arrives asynchronously after the error; thanks to the reactive
 * evaluation the button appears as soon as opState/conflicts are reported.
 */
export function offerConflictWorkshop(
  errorAction: string | null,
  opState: string,
  conflictedCount: number,
  view: string,
): boolean {
  return (
    errorAction === "conflicts" &&
    workshopAvailable(opState) &&
    conflictedCount > 0 &&
    view !== "conflicts"
  );
}
