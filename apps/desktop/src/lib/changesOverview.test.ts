import { describe, expect, it } from "vitest";
import { buildOverview, conventionalTypes } from "./changesOverview";
import type { FileLineStats, StatusEntry } from "./api";

const entry = (path: string, kind: StatusEntry["kind"] = "modified"): StatusEntry => ({
  path,
  origPath: null,
  kind,
});

const stat = (path: string, added: number, deleted: number, binary = false): FileLineStats => ({
  path,
  added,
  deleted,
  binary,
});

describe("buildOverview()", () => {
  it("merges both groups into one row per file", () => {
    const model = buildOverview(
      {
        staged: [entry("a.txt"), entry("both.txt")],
        unstaged: [entry("both.txt"), entry("b.txt", "untracked")],
      },
      null,
    );
    expect(model.rows.map((r) => [r.path, r.staged])).toEqual([
      ["a.txt", "full"],
      ["b.txt", "none"],
      ["both.txt", "partial"],
    ]);
    expect(model.totals.files).toBe(3);
  });

  it("returns the line balance, totals and maxTotal from numstat", () => {
    const model = buildOverview(
      { staged: [], unstaged: [entry("a.txt"), entry("b.bin"), entry("c.txt")] },
      [stat("a.txt", 5, 2), stat("b.bin", 0, 0, true)],
    );
    expect(model.rows.find((r) => r.path === "a.txt")?.stats).toEqual(stat("a.txt", 5, 2));
    // c.txt has no balance (yet) — e.g. numstat older than the status.
    expect(model.rows.find((r) => r.path === "c.txt")?.stats).toBeNull();
    expect(model.totals).toEqual({ files: 3, added: 5, deleted: 2 });
    expect(model.maxTotal).toBe(7);
  });

  it("is empty without a status", () => {
    expect(buildOverview(null, null).rows).toEqual([]);
  });
});

describe("conventionalTypes()", () => {
  it("detects dominant conventional-commit types, most frequent first", () => {
    expect(
      conventionalTypes([
        "fix(ui): a",
        "feat: b",
        "fix: c",
        "docs: d",
        "fix(core)!: e",
        "something else",
      ]),
    ).toEqual(["fix", "docs", "feat"]);
  });

  it("returns null when the convention does not dominate", () => {
    expect(conventionalTypes(["Something", "Other", "fix: only one"])).toBeNull();
    expect(conventionalTypes([])).toBeNull();
    // A single hit is not enough (protection against coincidence).
    expect(conventionalTypes(["fix: alone"])).toBeNull();
  });
});
