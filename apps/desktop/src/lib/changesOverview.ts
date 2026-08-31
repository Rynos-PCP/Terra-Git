// Model of the changes overview: merges the two status
// groups (staged/unstaged) into ONE row per file and enriches it with the line
// balance from `status_numstat`. Pure functions (no runes/DOM access) so it is
// testable without the Svelte runtime — same pattern as `welcomeVein`/
// `pipelineModel`.

import type { ChangeKind, FileLineStats, RepoStatus } from "./api";

/** Staging state of a file: fully, partially or not staged. */
export type StagedState = "full" | "partial" | "none";

export interface OverviewRow {
  path: string;
  kind: ChangeKind;
  staged: StagedState;
  /** Line balance; null while numstat is still loading or does not know the path. */
  stats: FileLineStats | null;
}

export interface OverviewModel {
  rows: OverviewRow[];
  /** Sums over all rows with a known balance (binary files count 0/0). */
  totals: { files: number; added: number; deleted: number };
  /** Largest line sum of a file — the reference for the delta bars. */
  maxTotal: number;
}

export function buildOverview(
  status: Pick<RepoStatus, "staged" | "unstaged"> | null,
  numstat: FileLineStats[] | null,
): OverviewModel {
  if (!status) return { rows: [], totals: { files: 0, added: 0, deleted: 0 }, maxTotal: 0 };
  const byPath = new Map<string, { kind: ChangeKind; inStaged: boolean; inUnstaged: boolean }>();
  for (const e of status.staged) {
    byPath.set(e.path, { kind: e.kind, inStaged: true, inUnstaged: false });
  }
  for (const e of status.unstaged) {
    const prev = byPath.get(e.path);
    // The working-tree view wins for the change kind (it is what the user would
    // stage next).
    byPath.set(e.path, { kind: e.kind, inStaged: prev?.inStaged ?? false, inUnstaged: true });
  }
  const stats = new Map((numstat ?? []).map((s) => [s.path, s]));
  const rows: OverviewRow[] = [...byPath.entries()]
    .map(([path, v]) => ({
      path,
      kind: v.kind,
      staged: (v.inStaged ? (v.inUnstaged ? "partial" : "full") : "none") as StagedState,
      stats: stats.get(path) ?? null,
    }))
    .sort((a, b) => a.path.localeCompare(b.path));
  let added = 0;
  let deleted = 0;
  let maxTotal = 0;
  for (const r of rows) {
    if (!r.stats) continue;
    added += r.stats.added;
    deleted += r.stats.deleted;
    maxTotal = Math.max(maxTotal, r.stats.added + r.stats.deleted);
  }
  return { rows, totals: { files: rows.length, added, deleted }, maxTotal };
}

/** Subject pattern of conventional commits: `type(scope)!: …` */
const CC_PATTERN = /^([a-z]+)(\([^)]*\))?!?:\s/;

/**
 * Detects from the message log (newest first, max. 30) whether the repo uses
 * conventional commits. Returns the most frequent types (max. 3) for the hint in
 * the overview — or null when the convention does not dominate (at least 2 hits
 * AND half of the messages considered).
 */
export function conventionalTypes(messageLog: string[], scan = 20): string[] | null {
  const sample = messageLog.slice(0, scan);
  if (sample.length === 0) return null;
  const counts = new Map<string, number>();
  let hits = 0;
  for (const msg of sample) {
    const m = CC_PATTERN.exec(msg);
    if (!m) continue;
    hits++;
    counts.set(m[1], (counts.get(m[1]) ?? 0) + 1);
  }
  if (hits < 2 || hits / sample.length < 0.5) return null;
  return [...counts.entries()]
    .sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
    .slice(0, 3)
    .map(([type]) => type);
}
