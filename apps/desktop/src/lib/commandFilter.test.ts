import { describe, expect, it } from "vitest";
import { filterCommands } from "./commandFilter";

const cmds = [
  { label: "Fetch", hint: "git fetch --prune" },
  { label: "Pull", hint: "git pull" },
  { label: "Force-Push", hint: "--force-with-lease" },
  { label: "Manage remotes…" },
  { label: "Switch branch: feature/palette" },
];

describe("filterCommands", () => {
  it("an empty search returns everything unchanged", () => {
    expect(filterCommands(cmds, "")).toEqual(cmds);
    expect(filterCommands(cmds, "   ")).toEqual(cmds);
  });

  it("filters case-insensitively over the label", () => {
    expect(filterCommands(cmds, "remotes").map((c) => c.label)).toEqual(["Manage remotes…"]);
  });

  it("all search words have to appear (in the hint too)", () => {
    expect(filterCommands(cmds, "force lease").map((c) => c.label)).toEqual(["Force-Push"]);
    expect(filterCommands(cmds, "force xyz")).toEqual([]);
  });

  it("label-prefix hits are sorted before substring hits", () => {
    const r = filterCommands(cmds, "p").map((c) => c.label);
    // "Pull" starts with p → before "Force-Push"/"Switch branch: feature/palette".
    expect(r[0]).toBe("Pull");
    expect(r).toContain("Force-Push");
  });
});
