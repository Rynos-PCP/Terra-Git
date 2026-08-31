// Word filter of the command palette. A pure function — covered by Vitest
// (commandFilter.test.ts).

export interface Filterable {
  label: string;
  hint?: string;
}

/**
 * Simple word filtering: all search words have to appear in the label or the
 * hint; hits at the start of the label are sorted first.
 */
export function filterCommands<T extends Filterable>(items: T[], query: string): T[] {
  const q = query.trim().toLowerCase();
  if (!q) return items;
  const words = q.split(/\s+/);
  return items
    .flatMap((c) => {
      const label = c.label.toLowerCase();
      const hay = c.hint ? `${label} ${c.hint.toLowerCase()}` : label;
      if (!words.every((w) => hay.includes(w))) return [];
      return [{ c, score: label.startsWith(words[0]) ? 0 : 1 }];
    })
    .sort((a, b) => a.score - b.score)
    .map((x) => x.c);
}
