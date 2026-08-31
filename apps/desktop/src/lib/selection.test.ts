import { describe, expect, it } from "vitest";
import { selectionPaths } from "./selection";

describe("selectionPaths", () => {
  const entries = [{ path: "a.txt" }, { path: "b.txt" }, { path: "c.txt" }];
  it("returns the selected paths in list order", () => {
    expect(selectionPaths(entries, new Set(["c.txt", "a.txt"]))).toEqual(["a.txt", "c.txt"]);
  });
  it("ignores a selection that no longer exists", () => {
    expect(selectionPaths(entries, new Set(["x.txt"]))).toEqual([]);
  });
  it("empty selection -> empty", () => {
    expect(selectionPaths(entries, new Set())).toEqual([]);
  });
});
