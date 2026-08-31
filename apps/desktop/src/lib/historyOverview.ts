// Layout of the history overview (core-sample reading after
// the user's decision of 2026-08-14): transposes the lane model from
// `historyGraph.ts` into one large VERTICAL repository graph — the newest commit
// is at the top, the history sinks downwards into the depths (time runs from
// bottom to top, as in the welcome screen's core sample).
// Lanes are columns (x), edges run orthogonally with quarter-arc corners instead
// of bezier waves. On the left an age scale reads off the depth (first mention
// per step, "<1h"/"2h"/"3d" as on the core-box ruler).
//
// Ref chips sit as a row in a fixed column to the RIGHT of the graph (one per
// commit on its line) — there they can cover neither nodes nor other chips, and
// the adaptive column width of the horizontal version is gone.
//
//
// Pure and DOM-free — for tests. Rendered by HistoryOverview.svelte.

import type { BranchInfo, CommitInfo, TagInfo } from "./api";
import { buildGraph } from "./historyGraph";
import { ageText } from "./welcomeVein";

/** Track width per lane (x distance of the columns). */
export const LANE_W = 26;
/** Row height per commit (y distance). */
export const ROW_H = 26;
/** Space on the left for the age scale. */
const RULER_W = 34;
const PAD_L = 12;
const PAD_R = 24;
const PAD_T = 20;
const PAD_B = 16;
/** Quarter-arc radius of the edge corners (core-sample reading). */
const ARC = 8;
/** Distance node column -> chip column, and chip -> chip. */
const CHIP_GAP = 6;
const CHIP_OFFSET = 18;

export interface OverviewNode {
  id: string;
  /** Index in the commit list (0 = newest, at the top). */
  idx: number;
  x: number;
  y: number;
  lane: number;
  isMerge: boolean;
}

export interface OverviewEdge {
  path: string;
  /** Track the edge runs in (color). */
  lane: number;
  /** The parent is not (or no longer) loaded: a short, fading stub. */
  stub: boolean;
}

export interface OverviewLabel {
  x: number;
  y: number;
  name: string;
  kind: "head" | "local" | "remote" | "tag" | "overflow";
  commitId: string;
}

/** Age mark of the scale on the left (first mention per step). */
export interface OverviewRulerMark {
  y: number;
  text: string;
}

export interface OverviewModel {
  width: number;
  height: number;
  nodes: OverviewNode[];
  edges: OverviewEdge[];
  labels: OverviewLabel[];
  laneCount: number;
  /** Age scale on the left: line x plus marks (empty without commits). */
  ruler: { x: number; y1: number; y2: number; marks: OverviewRulerMark[] };
}

const rnd = (n: number) => Math.round(n * 10) / 10;

/** Estimated chip width (icon + padding + ~5.8 px per character, capped like
 *  the chips' max-width) — for the layout, not pixel-exact. */
export function chipWidth(name: string, kind: OverviewLabel["kind"]): number {
  const icon = kind === "overflow" ? 0 : 13;
  return Math.min(130, 14 + icon + Math.ceil(name.length * 5.8));
}

/** Edge from (x1,y1) over the travel track xT to (x2,y2), y2 > y1 —
 *  orthogonal with quarter-arc corners: sideways out of the node, arc
 *  downwards, vertically through the depth, arc to the parent. */
function edgePath(x1: number, y1: number, xT: number, x2: number, y2: number): string {
  if (x1 === xT && xT === x2) return `M${x1} ${y1} L${x2} ${y2}`;
  if (y2 - y1 < 2 * ARC) return `M${x1} ${y1} L${x2} ${y2}`;
  let d = `M${x1} ${y1}`;
  let y = y1;
  if (x1 !== xT) {
    const s = xT > x1 ? 1 : -1;
    d += ` H${rnd(xT - s * ARC)} A${ARC} ${ARC} 0 0 ${s > 0 ? 1 : 0} ${xT} ${rnd(y1 + ARC)}`;
    y = y1 + ARC;
  }
  const yJoin = x2 !== xT ? y2 - ARC : y2;
  if (rnd(yJoin) > rnd(y)) d += ` V${rnd(yJoin)}`;
  if (x2 !== xT) {
    const s = x2 > xT ? 1 : -1;
    d += ` A${ARC} ${ARC} 0 0 ${s > 0 ? 0 : 1} ${rnd(xT + s * ARC)} ${y2} H${x2}`;
  }
  return d;
}

/** Builds the overview layout from the loaded commits + refs. `now` (Unix
 *  seconds) dates the age scale; without it the age counts relative to the
 *  newest commit (tests). */
export function buildHistoryOverview(
  commits: CommitInfo[],
  branches: BranchInfo[],
  tags: TagInfo[],
  now?: number,
): OverviewModel {
  const graph = buildGraph(commits);
  const n = commits.length;

  const idxById = new Map<string, number>();
  commits.forEach((c, i) => idxById.set(c.id, i));

  // ---- Collect ref chips: HEAD first, then local, remote, tags; +n. ----
  const byCommit = new Map<string, { name: string; kind: OverviewLabel["kind"] }[]>();
  const rank = { head: 0, local: 1, remote: 2, tag: 3, overflow: 4 } as const;
  for (const b of branches) {
    if (!b.targetId || !idxById.has(b.targetId)) continue;
    const list = byCommit.get(b.targetId) ?? [];
    list.push({ name: b.name, kind: b.isHead ? "head" : b.isRemote ? "remote" : "local" });
    byCommit.set(b.targetId, list);
  }
  for (const t of tags) {
    if (!idxById.has(t.targetId)) continue;
    const list = byCommit.get(t.targetId) ?? [];
    list.push({ name: t.name, kind: "tag" });
    byCommit.set(t.targetId, list);
  }
  const MAX_LABELS = 3;
  const chipRows = new Map<string, { name: string; kind: OverviewLabel["kind"] }[]>();
  for (const [id, list] of byCommit) {
    list.sort((a, b) => rank[a.kind] - rank[b.kind]);
    const shown = list.slice(0, MAX_LABELS);
    if (list.length > MAX_LABELS) {
      shown.push({ name: `+${list.length - MAX_LABELS}`, kind: "overflow" });
    }
    chipRows.set(id, shown);
  }

  // ---- Grid: one row per commit (newest at the top), one column per lane. ----
  const x = (lane: number) => PAD_L + RULER_W + lane * LANE_W;
  const y = (idx: number) => PAD_T + idx * ROW_H;

  let laneCount = 1;
  const nodes: OverviewNode[] = commits.map((c, i) => {
    const lane = graph[i].lane;
    laneCount = Math.max(laneCount, lane + 1, graph[i].after.length);
    return {
      id: c.id,
      idx: i,
      x: x(lane),
      y: y(i),
      lane,
      isMerge: c.parentIds.length > 1,
    };
  });

  const edges: OverviewEdge[] = [];
  commits.forEach((c, i) => {
    const row = graph[i];
    for (const p of c.parentIds) {
      const j = idxById.get(p);
      if (j === undefined) {
        // Parent outside the loaded page: a stub continuing into the depth.
        edges.push({
          path: `M${x(row.lane)} ${y(i)} L${x(row.lane)} ${rnd(y(i) + ROW_H * 0.7)}`,
          lane: row.lane,
          stub: true,
        });
        continue;
      }
      // Travel track as in the list renderer: the lane that expects the parent
      // below the row (it is reserved there until the parent).
      const travel = row.after.indexOf(p);
      const travelLane = travel === -1 ? graph[j].lane : travel;
      edges.push({
        path: edgePath(x(row.lane), y(i), x(travelLane), x(graph[j].lane), y(j)),
        lane: travelLane,
        stub: false,
      });
    }
  });

  // ---- Chips: a fixed column right of the graph, one row per commit. ----
  const chipX = n > 0 ? x(laneCount - 1) + CHIP_OFFSET : 0;
  const labels: OverviewLabel[] = [];
  let maxChipRow = 0;
  for (const [id, row] of chipRows) {
    const i = idxById.get(id) as number;
    let cx = chipX;
    for (const l of row) {
      labels.push({ x: cx, y: y(i), name: l.name, kind: l.kind, commitId: id });
      cx += chipWidth(l.name, l.kind) + CHIP_GAP;
    }
    maxChipRow = Math.max(maxChipRow, cx - CHIP_GAP - chipX);
  }

  // ---- Age scale on the left: first mention per step, top to bottom. ----
  const ref = now ?? commits[0]?.time ?? 0;
  const marks: OverviewRulerMark[] = [];
  const seen = new Set<string>();
  for (let i = 0; i < n; i++) {
    const text = ageText(Math.max(0, ref - commits[i].time));
    if (seen.has(text)) continue;
    seen.add(text);
    marks.push({ y: y(i), text });
  }

  return {
    width: n > 0 ? chipX + maxChipRow + PAD_R : 0,
    height: n > 0 ? PAD_T + (n - 1) * ROW_H + PAD_B + 10 : 0,
    nodes,
    edges,
    labels,
    laneCount,
    ruler: {
      x: PAD_L + RULER_W - 6,
      y1: PAD_T - 8,
      y2: n > 0 ? y(n - 1) + 8 : PAD_T,
      marks: n > 0 ? marks : [],
    },
  };
}
