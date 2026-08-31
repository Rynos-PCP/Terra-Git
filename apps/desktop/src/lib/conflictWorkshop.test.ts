// Text logic of the conflict workshop: the sides have to be named correctly per
// operation (and explained SWAPPED on a rebase); missing labels fall back to
// generic keys instead of showing "(null)".
import { describe, expect, it } from "vitest";
import type { OpContext } from "./api";
import { workshopAvailable, workshopCopy } from "./conflictWorkshop";

const ctx = (over: Partial<OpContext>): OpContext => ({
  kind: "merge",
  oursLabel: null,
  theirsLabel: null,
  theirsSummary: null,
  step: null,
  total: null,
  ...over,
});

describe("workshopCopy", () => {
  it("merge: names both sides with branch names", () => {
    const c = workshopCopy(ctx({ kind: "merge", oursLabel: "main", theirsLabel: "feature/x" }));
    expect(c.subtitle).toEqual({
      key: "conflictws.sub.merge",
      params: { ours: "main", theirs: "feature/x" },
    });
    expect(c.ours).toEqual({ key: "conflictws.ours.merge", params: { ours: "main" } });
    expect(c.theirs).toEqual({ key: "conflictws.theirs.merge", params: { theirs: "feature/x" } });
    expect(c.hint).toBeNull();
    expect(c.step).toBeNull();
  });

  it("rebase: explains the swapped sides (base left, your own commit right)", () => {
    const c = workshopCopy(
      ctx({
        kind: "rebase",
        oursLabel: "origin/main",
        theirsLabel: "feature/x",
        step: 2,
        total: 5,
      }),
    );
    expect(c.ours).toEqual({ key: "conflictws.ours.rebase", params: { ours: "origin/main" } });
    expect(c.theirs).toEqual({ key: "conflictws.theirs.rebase", params: { theirs: "feature/x" } });
    expect(c.hint).toEqual({ key: "conflictws.hint.rebase", params: { ours: "origin/main" } });
    expect(c.step).toEqual({ step: 2, total: 5 });
  });

  it("cherry-pick and revert: commit against the current state", () => {
    for (const kind of ["cherrypick", "revert"] as const) {
      const c = workshopCopy(ctx({ kind, oursLabel: "main", theirsLabel: "abc12345" }));
      expect(c.subtitle.key, kind).toBe(
        kind === "cherrypick" ? "conflictws.sub.cherrypick" : "conflictws.sub.revert",
      );
      expect(c.ours.key, kind).toBe("conflictws.ours.pick");
      expect(c.theirs.key, kind).toBe("conflictws.theirs.pick");
    }
  });

  it("falls back to generic keys when labels are missing", () => {
    // Table: context variants that all have to stay generic.
    const cases: (OpContext | null)[] = [
      null,
      ctx({ kind: "merge" }), // no labels
      ctx({ kind: "clean" }),
      ctx({ kind: "bisect" }),
    ];
    for (const f of cases) {
      const c = workshopCopy(f);
      expect(c.ours.key, JSON.stringify(f)).toMatch(/plain$/);
      expect(c.theirs.key, JSON.stringify(f)).toMatch(/plain$/);
      expect(c.subtitle.key, JSON.stringify(f)).toBe("conflictws.sub.generic");
    }
  });

  it("rebase without a step counter shows no progress chip", () => {
    const c = workshopCopy(ctx({ kind: "rebase", oursLabel: "main", theirsLabel: "f", step: 3 }));
    expect(c.step).toBeNull(); // total missing -> no half chip
  });
});

// The gate for all entry points (tools menu, palette, toast offer).
describe("workshopAvailable", () => {
  it("applies to the operations with two sides", () => {
    for (const op of ["merge", "rebase", "cherrypick", "revert"]) {
      expect(workshopAvailable(op), op).toBe(true);
    }
  });

  it("does not apply without an operation — nor to bisect (no conflict case)", () => {
    expect(workshopAvailable("clean")).toBe(false);
    expect(workshopAvailable("bisect")).toBe(false);
    expect(workshopAvailable(null)).toBe(false);
    expect(workshopAvailable(undefined)).toBe(false);
  });
});
