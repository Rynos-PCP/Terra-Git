import { describe, expect, it } from "vitest";
import { buildGraph } from "./historyGraph";
import type { CommitInfo } from "./api";

/** Minimal commit for the layout: only id + parentIds matter. */
const c = (id: string, parents: string[] = []): CommitInfo => ({
  id,
  shortId: id.slice(0, 7),
  summary: id,
  authorName: "T",
  authorEmail: "t@t",
  time: 0,
  parentIds: parents,
});

describe("buildGraph()", () => {
  it("keeps a linear chain on lane 0", () => {
    const rows = buildGraph([c("c", ["b"]), c("b", ["a"]), c("a")]);
    expect(rows.map((r) => r.lane)).toEqual([0, 0, 0]);
    // After the root everything is free.
    expect(rows[2].after).toEqual([]);
  });

  it("gives the second parent of a merge its own lane", () => {
    // merge -> (a, b); a -> root; b -> root
    const rows = buildGraph([
      c("merge", ["a", "b"]),
      c("a", ["root"]),
      c("b", ["root"]),
      c("root"),
    ]);
    expect(rows[0].lane).toBe(0);
    expect(rows[0].after).toEqual(["a", "b"]); // both parents expected
    expect(rows[1].lane).toBe(0);
    expect(rows[2].lane).toBe(1); // b runs on the second track
    // root merges both tracks; after that everything is free.
    expect(rows[3].lane).toBe(0);
    expect(rows[3].after).toEqual([]);
  });

  it("puts independent tips (all-refs graph) on separate lanes", () => {
    // Two branches without a shared history in one list.
    const rows = buildGraph([c("f1", ["f0"]), c("m1", ["m0"]), c("f0"), c("m0")]);
    expect(rows[0].lane).toBe(0); // feature tip
    expect(rows[1].lane).toBe(1); // main tip next to it
    expect(rows[2].lane).toBe(0);
    expect(rows[3].lane).toBe(1);
  });

  it("reuses lanes that became free", () => {
    // A short side strand ends (root), then a new tip begins: it should get the
    // freed lane 0, not a third one.
    const rows = buildGraph([c("short"), c("new", ["base"]), c("base")]);
    expect(rows[0].lane).toBe(0);
    expect(rows[0].after).toEqual([]); // root without a parent: lane free immediately
    expect(rows[1].lane).toBe(0); // reused
  });

  it("ends its own lane when the first parent is already expected", () => {
    // b and side both point at a — the second edge merges in instead of
    // creating a phantom lane.
    const rows = buildGraph([c("b", ["a"]), c("side", ["a"]), c("a")]);
    expect(rows[1].lane).toBe(1);
    expect(rows[1].after).toEqual(["a"]); // only ONE track expects a
    expect(rows[2].lane).toBe(0);
    expect(rows[2].after).toEqual([]);
  });
});
