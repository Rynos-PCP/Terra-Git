// Lane layout of the history graph: assigns every commit
// row a track (lane) and describes the track occupancy before and after the row
// — from that HistoryPanel draws one SVG segment per row (VirtualList-compatible,
// no giant SVG). Parent-based with lane reuse; works for the HEAD line as well
// as for the all-refs graph (several tips/roots). A pure function, testable
// without the Svelte runtime — same pattern as `welcomeVein`/`pipelineModel`.
//

import type { CommitInfo } from "./api";

export interface GraphRow {
  /** Track the commit dot of this row sits on. */
  lane: number;
  /** Track occupancy above the row (the commit id the track expects). */
  before: (string | null)[];
  /** Track occupancy below the row. */
  after: (string | null)[];
  id: string;
  parents: string[];
}

export function buildGraph(commits: CommitInfo[]): GraphRow[] {
  const lanes: (string | null)[] = [];
  const rows: GraphRow[] = [];
  for (const c of commits) {
    const before = [...lanes];
    let lane = lanes.findIndex((id) => id === c.id);
    if (lane === -1) {
      lane = lanes.findIndex((id) => id === null);
      if (lane === -1) {
        lane = lanes.length;
        lanes.push(null);
      }
    }
    // All lanes that expected this commit converge here.
    for (let i = 0; i < lanes.length; i++) {
      if (lanes[i] === c.id) lanes[i] = null;
    }
    // Put the first parent on our own lane — but only when no other lane
    // already expects it (otherwise a phantom lane + a double edge).
    const [p0, ...rest] = c.parentIds;
    if (p0 && lanes.includes(p0)) {
      lanes[lane] = null; // parent already runs elsewhere; own lane ends
    } else {
      lanes[lane] = p0 ?? null;
    }
    for (const p of rest) {
      if (!lanes.includes(p)) {
        const free = lanes.findIndex((id) => id === null);
        if (free === -1) lanes.push(p);
        else lanes[free] = p;
      }
    }
    while (lanes.length > 0 && lanes[lanes.length - 1] === null) lanes.pop();
    rows.push({ lane, before, after: [...lanes], id: c.id, parents: c.parentIds });
  }
  return rows;
}
