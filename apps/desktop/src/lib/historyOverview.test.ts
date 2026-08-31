// Layout of the history overview (core-sample reading): the large vertical
// graph has to turn arbitrary commit lists into stable geometry — newest at the
// top, edges orthogonal along the lane occupancy, ref chips in the fixed column
// on the right, the age scale on the left (user decision 2026-08-14).
import { describe, expect, it } from "vitest";
import type { BranchInfo, CommitInfo, TagInfo } from "./api";
import { buildHistoryOverview, chipWidth, ROW_H } from "./historyOverview";

/** Minimal commit for the layout: only id + parentIds matter. */
const c = (id: string, parents: string[] = [], time = 0): CommitInfo => ({
  id,
  shortId: id.slice(0, 7),
  summary: id,
  authorName: "T",
  authorEmail: "t@t",
  time,
  parentIds: parents,
});

const branch = (name: string, targetId: string, over: Partial<BranchInfo> = {}): BranchInfo => ({
  name,
  isHead: false,
  isRemote: false,
  upstream: null,
  shortName: null,
  targetId,
  upstreamGone: false,
  ...over,
});

const tag = (name: string, targetId: string): TagInfo => ({
  name,
  targetId,
  message: null,
  isAnnotated: false,
});

describe("buildHistoryOverview", () => {
  it("puts a linear chain in one column, newest commit at the top", () => {
    const m = buildHistoryOverview([c("c", ["b"]), c("b", ["a"]), c("a")], [], []);
    expect(m.nodes).toHaveLength(3);
    expect(m.laneCount).toBe(1);
    // The newest (idx 0) at the very top, the same x column (core sample: depth = age).
    expect(m.nodes[0].y).toBeLessThan(m.nodes[1].y);
    expect(m.nodes[1].y).toBeLessThan(m.nodes[2].y);
    expect(new Set(m.nodes.map((n) => n.x)).size).toBe(1);
    expect(m.nodes[1].y - m.nodes[0].y).toBe(ROW_H);
    // Two loaded edges (c->b, b->a), no stub: a has no parents.
    expect(m.edges).toHaveLength(2);
    expect(m.edges.every((e) => !e.stub)).toBe(true);
  });

  it("routes the second parent of a merge through its own column with a quarter arc", () => {
    const m = buildHistoryOverview(
      [c("merge", ["a", "b"]), c("a", ["root"]), c("b", ["root"]), c("root")],
      [],
      [],
    );
    expect(m.laneCount).toBe(2);
    const merge = m.nodes[0];
    const b = m.nodes[2];
    expect(b.x).toBeGreaterThan(merge.x); // b in the second column
    expect(merge.isMerge).toBe(true);
    // The merge edge to b travels on lane 1 — orthogonal with an arc (A) instead
    // of a bezier wave (no C segment any more).
    const edge = m.edges.find((e) => e.lane === 1 && e.path.includes("A"));
    expect(edge).toBeDefined();
    expect(m.edges.every((e) => !e.path.includes("C"))).toBe(true);
  });

  it("draws a stub further into the depth for unloaded parents", () => {
    const m = buildHistoryOverview([c("b", ["a-not-loaded"])], [], []);
    expect(m.edges).toHaveLength(1);
    expect(m.edges[0].stub).toBe(true);
    // The stub runs downwards (older commits lie deeper).
    const ys = (m.edges[0].path.match(/-?\d+(?:\.\d+)?/g) ?? []).map(Number);
    expect(ys[3]).toBeGreaterThan(ys[1]);
  });

  it("puts ref chips in the fixed column on the right: HEAD first, +n cap", () => {
    const commits = [c("b", ["a"]), c("a")];
    const m = buildHistoryOverview(
      commits,
      [
        branch("main", "b", { isHead: true }),
        branch("origin/main", "b", { isRemote: true }),
        branch("feature", "b"),
        branch("alt", "b"),
      ],
      [tag("v1", "a")],
    );
    const anB = m.labels.filter((l) => l.commitId === "b");
    // 4 refs -> 3 shown + a "+1" cap; HEAD sorted to the front.
    expect(anB).toHaveLength(4);
    expect(anB[0].kind).toBe("head");
    expect(anB[3]).toMatchObject({ kind: "overflow", name: "+1" });
    // Chips as a row on the line of their commit, all to the right of the graph.
    expect(anB[0].y).toBe(m.nodes[0].y);
    expect(anB[1].x).toBeGreaterThan(anB[0].x);
    expect(new Set(anB.map((l) => l.y)).size).toBe(1);
    const anA = m.labels.filter((l) => l.commitId === "a");
    expect(anA).toHaveLength(1);
    expect(anA[0].kind).toBe("tag");
    // A fixed chip column: both rows start at the same x, right of the nodes.
    expect(anA[0].x).toBe(anB[0].x);
    expect(anA[0].x).toBeGreaterThan(m.nodes[0].x);
    // The overall frame extends beyond the widest chip row.
    const right = Math.max(...anB.map((l) => l.x + chipWidth(l.name, l.kind)));
    expect(m.width).toBeGreaterThanOrEqual(right);
  });

  it("reads the age off on the left: first mention per step, top to bottom", () => {
    const now = 1_000_000;
    const m = buildHistoryOverview(
      [
        c("d", ["c2"], now - 1_800), // <1h
        c("c2", ["b"], now - 7_200), // 2h
        c("b", ["a"], now - 7_500), // also 2h -> deduplicated
        c("a", [], now - 90_000), // 1d
      ],
      [],
      [],
      now,
    );
    expect(m.ruler.marks.map((x) => x.text)).toEqual(["<1h", "2h", "1d"]);
    const ys = m.ruler.marks.map((x) => x.y);
    for (let i = 1; i < ys.length; i++) expect(ys[i]).toBeGreaterThan(ys[i - 1]);
    // Marks sit on the rows of their commits.
    expect(ys[0]).toBe(m.nodes[0].y);
    expect(ys[2]).toBe(m.nodes[3].y);
  });

  it("ignores refs to unloaded commits and returns empty dimensions without commits", () => {
    const empty = buildHistoryOverview([], [branch("x", "gone")], [tag("v", "gone")]);
    expect(empty.nodes).toEqual([]);
    expect(empty.labels).toEqual([]);
    expect(empty.width).toBe(0);
    expect(empty.ruler.marks).toEqual([]);
  });
});
