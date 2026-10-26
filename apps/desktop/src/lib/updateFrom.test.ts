import { describe, expect, it } from "vitest";
import type { BranchInfo } from "./api";
import { effectiveSource, preselect, rememberKey, updateCandidates } from "./updateFrom";

const b = (name: string, extra: Partial<BranchInfo> = {}): BranchInfo => ({
  name,
  isHead: false,
  isRemote: name.includes("/") && !name.startsWith("feature/") && !name.startsWith("release/"),
  upstream: null,
  shortName: null,
  targetId: null,
  upstreamGone: false,
  ...extra,
});

const branches: BranchInfo[] = [
  b("release/1.4", { isHead: true, upstream: "origin/release/1.4" }),
  b("main", { upstream: "origin/main" }),
  b("feature/x"),
  b("origin/main", { isRemote: true, shortName: "main" }),
  b("origin/release/1.4", { isRemote: true, shortName: "release/1.4" }),
  b("origin/HEAD", { isRemote: true, shortName: "HEAD" }),
];

describe("updateCandidates()", () => {
  const list = updateCandidates(branches, "release/1.4", "origin/release/1.4");

  it("offers every other branch, locals first, remotes even next to their local copy", () => {
    expect(list.map((c) => c.name)).toEqual(["main", "feature/x", "origin/main"]);
  });

  it("leaves out the current branch, its own upstream and origin/HEAD", () => {
    const names = list.map((c) => c.name);
    expect(names).not.toContain("release/1.4");
    expect(names).not.toContain("origin/release/1.4");
    expect(names).not.toContain("origin/HEAD");
  });

  it("keeps the upstream of local candidates", () => {
    expect(list[0]).toEqual({ name: "main", isRemote: false, upstream: "origin/main" });
  });
});

describe("preselect()", () => {
  const list = updateCandidates(branches, "release/1.4", "origin/release/1.4");

  it("prefers the source remembered for this branch", () => {
    expect(preselect(list, "feature/x", "origin/main")).toBe("feature/x");
  });

  it("falls back to the local copy of the default branch", () => {
    expect(preselect(list, undefined, "origin/main")).toBe("main");
  });

  it("uses the remote default branch when there is no local copy", () => {
    const remoteOnly = list.filter((c) => c.name !== "main");
    expect(preselect(remoteOnly, undefined, "origin/main")).toBe("origin/main");
  });

  it("ignores a remembered source that no longer exists", () => {
    expect(preselect(list, "gone/branch", null)).toBeNull();
  });
});

describe("effectiveSource()", () => {
  it("uses the upstream of a local branch when asked to", () => {
    const main = { name: "main", isRemote: false, upstream: "origin/main" };
    expect(effectiveSource(main, true)).toBe("origin/main");
    expect(effectiveSource(main, false)).toBe("main");
  });

  it("keeps branches without an upstream as they are", () => {
    expect(effectiveSource({ name: "feature/x", isRemote: false, upstream: null }, true)).toBe(
      "feature/x",
    );
  });
});

it("remembers per repository and branch", () => {
  expect(rememberKey("/repo", "release/1.4")).toBe("/repo::release/1.4");
});
