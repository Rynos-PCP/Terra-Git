import { describe, expect, it } from "vitest";
import { clickSelect, pruneSelection, selectAll } from "./fileSelection";

const order = ["a.txt", "b.txt", "c.txt", "d.txt"];

function sel(state: { selection: Set<string> }): string[] {
  return [...state.selection];
}

describe("clickSelect", () => {
  it("a plain click selects exactly one file and sets the anchor", () => {
    const r = clickSelect(
      { selection: new Set(["a.txt", "b.txt"]), anchor: "a.txt" },
      order,
      "c.txt",
      {
        ctrl: false,
        shift: false,
      },
    );
    expect(sel(r)).toEqual(["c.txt"]);
    expect(r.anchor).toBe("c.txt");
  });

  it("Ctrl+click toggles the file into the selection and out again", () => {
    const added = clickSelect({ selection: new Set(["a.txt"]), anchor: "a.txt" }, order, "c.txt", {
      ctrl: true,
      shift: false,
    });
    expect(sel(added).sort()).toEqual(["a.txt", "c.txt"]);
    expect(added.anchor).toBe("c.txt");

    const removed = clickSelect(added, order, "c.txt", { ctrl: true, shift: false });
    expect(sel(removed)).toEqual(["a.txt"]);
  });

  it("Shift+click selects the range from the anchor to the file (both directions)", () => {
    const down = clickSelect({ selection: new Set(["b.txt"]), anchor: "b.txt" }, order, "d.txt", {
      ctrl: false,
      shift: true,
    });
    expect(sel(down)).toEqual(["b.txt", "c.txt", "d.txt"]);
    expect(down.anchor).toBe("b.txt");

    const up = clickSelect({ selection: new Set(["d.txt"]), anchor: "d.txt" }, order, "b.txt", {
      ctrl: false,
      shift: true,
    });
    expect(sel(up)).toEqual(["b.txt", "c.txt", "d.txt"]);
  });

  it("Shift without a valid anchor falls back to a single selection", () => {
    const noAnchor = clickSelect({ selection: new Set(), anchor: null }, order, "c.txt", {
      ctrl: false,
      shift: true,
    });
    expect(sel(noAnchor)).toEqual(["c.txt"]);

    const staleAnchor = clickSelect({ selection: new Set(), anchor: "gone.txt" }, order, "c.txt", {
      ctrl: false,
      shift: true,
    });
    expect(sel(staleAnchor)).toEqual(["c.txt"]);
  });
});

describe("selectAll / pruneSelection", () => {
  it("selectAll selects all paths in order", () => {
    const r = selectAll(order);
    expect(sel(r).sort()).toEqual([...order].sort());
    expect(r.anchor).toBe("d.txt");
    expect(selectAll([]).anchor).toBeNull();
  });

  it("pruneSelection keeps only paths that still exist", () => {
    const pruned = pruneSelection(new Set(["a.txt", "gone.txt", "c.txt"]), order);
    expect([...pruned].sort()).toEqual(["a.txt", "c.txt"]);
  });
});
