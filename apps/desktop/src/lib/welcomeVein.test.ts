// Geometry of the welcome vein ("core sample, horizontal", the user's decision
// of 2026-08-14): the sketch has to turn arbitrary repo sketches (commits +
// branches) into stable, capped geometry — a horizontal core with EVENLY
// distributed nodes, orthogonal veins above it, colour instead of text — and to
// fall back decoratively without data.
import { describe, expect, it } from "vitest";
import { ageText, buildVein, strandSlot, VEIN_MAIN_PATH, VEIN_TAIL_PATH } from "./welcomeVein";

/** Commits newest-first with falling times (100 s apart). */
const commits = (n: number, tags: number[] = []) =>
  Array.from({ length: n }, (_, i) => ({
    time: 10_000 - i * 100,
    isMerge: false,
    hasTag: tags.includes(i),
  }));

const branch = (over: Partial<NonNullable<Parameters<typeof buildVein>[1]>[number]> = {}) => ({
  name: "feature",
  baseIndex: 2 as number | null,
  ahead: 2,
  tipTime: 9_950,
  ...over,
});

describe("buildVein", () => {
  it("falls back to the decorative vein without commits (3 nodes, nothing else)", () => {
    const g = buildVein([]);
    expect(g.main).toBe(VEIN_MAIN_PATH);
    expect(g.tail).toBe(VEIN_TAIL_PATH);
    expect(g.dots).toHaveLength(3);
    expect(g.strands).toEqual([]);
    expect(g.rings).toEqual([]);
  });

  it("puts one node ON the core per commit, newest on the right and larger; tags marked", () => {
    // Table: number of commits -> expected node count (capped at 8).
    for (const [n, expected] of [
      [1, 1],
      [4, 4],
      [12, 8],
    ] as const) {
      const g = buildVein(commits(n, [Math.min(1, n - 1)]));
      expect(g.dots, `n=${n}`).toHaveLength(expected);
      // The input is newest-first; the newest node sits furthest right.
      const xs = g.dots.map((d) => d.x);
      expect(Math.max(...xs), `n=${n}: newest on the right`).toBe(xs[0]);
      expect(g.dots[0].r, `n=${n}: newest larger`).toBeGreaterThan(4);
      expect(g.dots[Math.min(1, n - 1)].hasTag, `n=${n}: tag marker`).toBe(true);
      // The core is a straight line: all nodes at the same height.
      expect(new Set(g.dots.map((d) => d.y)).size, `n=${n}: horizontal`).toBe(1);
    }
  });

  it("distributes the nodes EVENLY — independent of the commit times", () => {
    const at = (t: number) => ({ time: t, isMerge: false, hasTag: false });
    // Three commits in a burst, then a long pause, then an old commit: this used
    // to clump the nodes into unreadable blobs (user finding 2026-08-14) — now
    // the distance per commit is fixed.
    const g = buildVein([at(10_000), at(9_990), at(9_980), at(5_000), at(100)]);
    const xs = g.dots.map((d) => d.x);
    const spacing = xs[0] - xs[1];
    for (let i = 1; i < xs.length; i++) {
      expect(Math.abs(xs[i - 1] - xs[i] - spacing)).toBeLessThan(0.5);
    }
  });

  it("branches off at the merge base with a quarter arc and runs horizontally in the track", () => {
    const g = buildVein(commits(6), [branch()]);
    expect(g.strands).toHaveLength(1);
    const s = g.strands[0];
    // The vein starts at the branch-point node (baseIndex 2) and runs right.
    expect(s.x0).toBe(g.dots[2].x);
    expect(s.x1).toBeGreaterThan(s.x0);
    expect(s.path.startsWith(`M${g.dots[2].x} ${g.dots[2].y}`)).toBe(true);
    // Orthogonal instead of a wavy line: vertically out, quarter arc, horizontal.
    expect(s.path).toContain(`A${10} ${10}`);
    expect(s.path).toContain("H");
    // Tip + one intermediate node (ahead 2), the tip first and larger.
    expect(s.dots).toHaveLength(2);
    expect(s.dots[0].r).toBeGreaterThan(s.dots[1].r);
    // All vein nodes lie exactly in the track — ABOVE the core.
    for (const d of s.dots) {
      expect(d.y).toBe(s.dots[0].y);
      expect(d.y).toBeLessThan(g.dots[0].y);
    }
    // All nodes stay inside the viewBox frame.
    for (const d of [...g.dots, ...s.dots]) {
      expect(d.x).toBeGreaterThanOrEqual(0);
      expect(d.x).toBeLessThanOrEqual(320);
      expect(d.y).toBeGreaterThanOrEqual(0);
      expect(d.y).toBeLessThanOrEqual(500);
    }
  });

  it("brings branch points outside the window in as a straight vein from the left (x0 = 0)", () => {
    const g = buildVein(commits(6), [branch({ baseIndex: null, tipTime: 9_800 })]);
    expect(g.strands).toHaveLength(1);
    expect(g.strands[0].x0).toBe(0);
    // Without a branch point in the window there is no arc — only the horizontal track.
    expect(g.strands[0].path).not.toContain("A");
  });

  it("uses its own track per strand and caps at 3 veins", () => {
    const many = Array.from({ length: 8 }, (_, i) =>
      branch({ name: `b${i}`, ahead: i === 0 ? 0 : 1, baseIndex: 3 }),
    );
    const g = buildVein(commits(8), many);
    expect(g.strands.length).toBeLessThanOrEqual(3);
    // Every vein lies in its own track (different heights above the core).
    const lanes = g.strands.map((s) => s.dots[0]?.y ?? -1);
    expect(new Set(lanes).size).toBe(g.strands.length);
    // b0 (ahead 0) has no vein but a coloured ring at the tip commit — the first
    // vein belongs to b1 (user finding: main/merged branches used to be
    // completely invisible).
    expect(g.strands[0].slot).toBe(strandSlot("b1"));
    expect(g.rings).toHaveLength(1);
    expect(g.rings[0].x).toBe(g.dots[3].x);
    expect(g.rings[0].y).toBe(g.dots[3].y);
    expect(g.rings[0].r).toBeGreaterThan(g.dots[3].r);
    expect(g.rings[0].slot).toBe(strandSlot("b0"));
  });

  it("stacks rings radially — after the tag ring", () => {
    // Commit 1 carries a tag AND two ancestor branch tips.
    const g = buildVein(commits(6, [1]), [
      branch({ name: "alt-a", ahead: 0, baseIndex: 1 }),
      branch({ name: "alt-b", ahead: 0, baseIndex: 1 }),
    ]);
    expect(g.rings).toHaveLength(2);
    // The first branch ring lies OUTSIDE the tag ring (dot.r + 3.2).
    expect(g.rings[0].r).toBeGreaterThan(g.dots[1].r + 3.2);
    expect(g.rings[1].r).toBeGreaterThan(g.rings[0].r);
  });

  it("collects ancestors BEFORE the window as rings at the left edge of the core", () => {
    // The normal case in real repos: the current branch is far ahead, main & co.
    // lie below the window (user finding 2026-08-14 — before that, exactly those
    // branches stayed invisible).
    const g = buildVein(commits(8), [
      branch({ name: "main", ahead: 0, baseIndex: null }),
      branch({ name: "alt", ahead: 0, baseIndex: null }),
    ]);
    expect(g.strands).toEqual([]);
    expect(g.rings).toHaveLength(2);
    // Left of the oldest commit node, stacked radially.
    const oldest = g.dots[g.dots.length - 1];
    for (const r of g.rings) expect(r.x).toBeLessThan(oldest.x);
    expect(g.rings[1].r).toBeGreaterThan(g.rings[0].r);
    expect(g.rings[0].slot).toBe(strandSlot("main"));
  });

  it("extends its tip beyond the newest commit when the branch is newer", () => {
    const g = buildVein(commits(6), [branch({ tipTime: 99_999, baseIndex: 1 })]);
    expect(g.strands[0].x1).toBeGreaterThan(g.dots[0].x);
  });

  it("assigns stable colour slots away from red/ochre (never 1, 4, 8)", () => {
    for (const name of ["main", "feature/x", "fix", "release-1.2", "a", "zz"]) {
      const slot = strandSlot(name);
      expect(slot).toBe(strandSlot(name)); // deterministic
      expect([2, 3, 5, 6, 7]).toContain(slot);
    }
  });

  it("returns age marks for the history overview — rounding down like timeAgo", () => {
    // ageText lives here but is only used by the history overview now.
    expect(ageText(1_800)).toBe("<1h");
    expect(ageText(9_000)).toBe("2h");
    expect(ageText(90_000)).toBe("1d");
  });
});
