// Responsibility rule for the global search (Ctrl+F / Ctrl+G / F3).
//
// Deliberately import-free (no state.svelte) so the rule is unit-testable
// without runes and without IPC — the same pattern as selection.ts or splitter.ts.

/**
 * May this searchable view (diff, blame) serve app-find/app-goto?
 *
 * `scope` = the modal kind the instance is rendered in (null = main view),
 * `openModal` = the kind of the currently open modal (null = none).
 *
 * Exactly the instance whose scope matches the open modal wins: the main diff
 * stays silent while any modal is open, and a diff INSIDE a modal serves the
 * search itself. Since `ui.modal` holds exactly one value, at most one instance
 * can be active by construction — independent of mount order, tab switches or
 * rerenders.
 */
export function findTargetActive(
  scope: string | null,
  openModal: string | null,
  hasContent = true,
): boolean {
  return hasContent && scope === openModal;
}
