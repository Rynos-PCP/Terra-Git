// Multi-selection logic of the changes list (file explorer semantics).
// Pure functions so the click/range logic is testable without a DOM
// (fileSelection.test.ts).

export interface ClickMods {
  ctrl: boolean;
  shift: boolean;
}

export interface SelectionState {
  selection: Set<string>;
  /** Reference point for Shift+click ranges; null when none is set. */
  anchor: string | null;
}

/**
 * New selection after a click:
 * - Ctrl+click toggles the file (anchor = the clicked file),
 * - Shift+click selects the range anchor…file (the anchor stays),
 * - a plain click selects exactly that one file (anchor = it).
 */
export function clickSelect(
  state: SelectionState,
  orderedPaths: string[],
  clicked: string,
  mods: ClickMods,
): SelectionState {
  if (mods.ctrl) {
    const selection = new Set(state.selection);
    if (selection.has(clicked)) selection.delete(clicked);
    else selection.add(clicked);
    return { selection, anchor: clicked };
  }
  if (mods.shift && state.anchor !== null) {
    const a = orderedPaths.indexOf(state.anchor);
    const b = orderedPaths.indexOf(clicked);
    if (a !== -1 && b !== -1) {
      const [lo, hi] = a <= b ? [a, b] : [b, a];
      return { selection: new Set(orderedPaths.slice(lo, hi + 1)), anchor: state.anchor };
    }
  }
  return { selection: new Set([clicked]), anchor: clicked };
}

/** Selects all paths (Ctrl+A); anchor = the last entry. */
export function selectAll(orderedPaths: string[]): SelectionState {
  return {
    selection: new Set(orderedPaths),
    anchor: orderedPaths[orderedPaths.length - 1] ?? null,
  };
}

/** Keeps only paths that still exist (after a status refresh). */
export function pruneSelection(selection: Set<string>, valid: string[]): Set<string> {
  const set = new Set(valid);
  return new Set([...selection].filter((p) => set.has(p)));
}
