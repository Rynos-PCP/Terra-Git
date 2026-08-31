import { describe, expect, it } from "vitest";
import type { BranchInfo } from "./api";
import { goneDeletableCandidates } from "./branchCleanup";

const b = (over: Partial<BranchInfo>): BranchInfo => ({
  name: "x",
  isHead: false,
  isRemote: false,
  upstream: null,
  shortName: null,
  targetId: null,
  upstreamGone: false,
  ...over,
});

describe("goneDeletableCandidates", () => {
  it("returns only orphaned, local, non-current branches", () => {
    const branches = [
      b({ name: "feat-a", upstreamGone: true }),
      b({ name: "main", isHead: true, upstreamGone: true }), // current -> out
      b({ name: "feat-b", upstreamGone: false }), // not orphaned -> out
      b({ name: "origin/feat-a", isRemote: true, upstreamGone: true }), // remote -> out
    ];
    expect(goneDeletableCandidates(branches)).toEqual(["feat-a"]);
  });

  it("empty list without candidates", () => {
    expect(goneDeletableCandidates([b({ upstreamGone: false })])).toEqual([]);
  });
});
