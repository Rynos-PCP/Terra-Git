// Text logic of the conflict workshop: picks the i18n keys and parameters per
// operation with which both sides are named UNDERSTANDABLY — branch/commit names
// instead of ours/theirs. The trickiest case is rebase: there "ours" is the new
// base and "theirs" is your own commit, i.e. exactly the other way around from
// what users know from merge. Pure and DOM-free — for tests.

import type { OpContext } from "./api";
import type { MessageKey } from "./i18n.svelte";

/** A translatable text: i18n key + parameters. */
export interface CopyRef {
  key: MessageKey;
  params?: Record<string, string | number>;
}

export interface WorkshopCopy {
  /** Explanatory sentence under the title. */
  subtitle: CopyRef;
  /** Additional warning (rebase only: the sides are swapped). */
  hint: CopyRef | null;
  /** Column header of the "ours" side (left). */
  ours: CopyRef;
  /** Column header of the "theirs" side (right). */
  theirs: CopyRef;
  /** Rebase progress (step x of y), otherwise null. */
  step: { step: number; total: number } | null;
}

/** Display names of the operations — git terms, not translated. */
export const OP_LABELS: Record<string, string> = {
  merge: "Merge",
  rebase: "Rebase",
  cherrypick: "Cherry-pick",
  revert: "Revert",
};

/**
 * Does the workshop have anything to show right now? Only during a multi-step
 * operation with two sides: without one the view throws you straight back to the
 * workspace. Bisect is multi-step too, but it has no conflict sides.
 *
 * ONE definition for all entry points — the tools menu, the palette and the
 * toast offer (`offerConflictWorkshop`) all ask the same thing.
 */

export function workshopAvailable(opState: string | null | undefined): boolean {
  return !!opState && opState !== "clean" && opState !== "bisect";
}

export function workshopCopy(ctx: OpContext | null): WorkshopCopy {
  const ours = ctx?.oursLabel ?? null;
  const theirs = ctx?.theirsLabel ?? null;
  const generic: WorkshopCopy = {
    subtitle: { key: "conflictws.sub.generic" },
    hint: null,
    ours: { key: "conflictws.ours.plain" },
    theirs: { key: "conflictws.theirs.plain" },
    step: null,
  };
  if (!ctx) return generic;

  switch (ctx.kind) {
    case "merge":
      return {
        subtitle:
          ours && theirs
            ? { key: "conflictws.sub.merge", params: { ours, theirs } }
            : generic.subtitle,
        hint: null,
        ours: ours ? { key: "conflictws.ours.merge", params: { ours } } : generic.ours,
        theirs: theirs ? { key: "conflictws.theirs.merge", params: { theirs } } : generic.theirs,
        step: null,
      };
    case "rebase":
      return {
        subtitle:
          ours && theirs
            ? { key: "conflictws.sub.rebase", params: { ours, theirs } }
            : generic.subtitle,
        // The classic stumbling block: on a rebase your own change is on the
        // RIGHT. Always explain it as soon as the base can be named.
        hint: ours ? { key: "conflictws.hint.rebase", params: { ours } } : null,
        ours: ours ? { key: "conflictws.ours.rebase", params: { ours } } : generic.ours,
        theirs: theirs
          ? { key: "conflictws.theirs.rebase", params: { theirs } }
          : { key: "conflictws.theirs.rebasePlain" },
        step: ctx.step && ctx.total ? { step: ctx.step, total: ctx.total } : null,
      };
    case "cherrypick":
    case "revert":
      return {
        subtitle:
          ours && theirs
            ? {
                key:
                  ctx.kind === "cherrypick" ? "conflictws.sub.cherrypick" : "conflictws.sub.revert",
                params: { ours, theirs },
              }
            : generic.subtitle,
        hint: null,
        ours: ours ? { key: "conflictws.ours.pick", params: { ours } } : generic.ours,
        theirs: theirs ? { key: "conflictws.theirs.pick", params: { theirs } } : generic.theirs,
        step: null,
      };
    default:
      return generic;
  }
}
