// Pure selection helpers for the file list (Vitest-tested).
/** Paths of the selected entries in list order (stable, deduplicated). */
export function selectionPaths(entries: { path: string }[], selection: Set<string>): string[] {
  return entries.filter((e) => selection.has(e.path)).map((e) => e.path);
}
