// Facts from git's output.
import { describe, expect, it } from "vitest";
import { filesInTheWay, filesInTheWayWithCount, lockFileNamed } from "./gitOutput";

describe("filesInTheWay()", () => {
  it("reads tracked and untracked files git names, without duplicates", () => {
    const out = [
      "Updating 1a2b3c4..5d6e7f8",
      "error: Your local changes to the following files would be overwritten by merge:",
      "\tsrc/app.ts",
      "\tREADME.md",
      "Please commit your changes or stash them before you merge.",
      "error: The following untracked working tree files would be overwritten by merge:",
      "\tnotes/todo.txt",
      "\tREADME.md",
      "Please move or remove them before you merge.",
      "Aborting",
    ].join("\n");
    expect(filesInTheWay(out)).toEqual(["src/app.ts", "README.md", "notes/todo.txt"]);
  });

  it("finds nothing in other output", () => {
    expect(filesInTheWay("fatal: refusing to merge unrelated histories")).toEqual([]);
    expect(filesInTheWay(undefined)).toEqual([]);
  });
});

describe("filesInTheWayWithCount()", () => {
  it("counts the entries the backend left out of a long list", () => {
    const out = [
      "error: Your local changes to the following files would be overwritten by checkout:",
      "\ta.txt",
      "\tb.txt",
      "[… 312 lines omitted — the full output is in the log]",
      "\ty.txt",
      "\tz.txt",
      "Please commit your changes or stash them before you switch branches.",
      "Aborting",
    ].join("\n");
    expect(filesInTheWayWithCount(out)).toEqual({
      files: ["a.txt", "b.txt", "y.txt", "z.txt"],
      omitted: 312,
    });
  });
});

describe("lockFileNamed()", () => {
  it("reads the lock git could not take", () => {
    expect(
      lockFileNamed("fatal: Unable to create 'C:/r/.git/modules/lib/index.lock': File exists."),
    ).toBe("C:/r/.git/modules/lib/index.lock");
    expect(lockFileNamed("fatal: something else")).toBe(null);
    expect(lockFileNamed(null)).toBe(null);
  });
});
