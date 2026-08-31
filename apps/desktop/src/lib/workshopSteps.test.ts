import { describe, expect, it } from "vitest";
import {
  authorValid,
  baselineOf,
  buildWorkshopSteps,
  changedCommitCount,
  commitChanged,
  firstKeptIsSquash,
  workshopOrderChanged,
  type WorkshopEdit,
} from "./workshopSteps";
import type { UnpushedCommit } from "./api";

const c = (id: string, parents: string[], over: Partial<UnpushedCommit> = {}): UnpushedCommit => ({
  id,
  subject: `S-${id}`,
  body: "",
  authorName: "A",
  authorEmail: "a@x.de",
  time: 1_700_000_000,
  parentIds: parents,
  isHead: false,
  isMerge: false,
  ...over,
});
const edit = (over: Partial<WorkshopEdit> = {}): WorkshopEdit => ({
  subject: "",
  body: "",
  coAuthors: "",
  authorName: "A",
  authorEmail: "a@x.de",
  dropped: false,
  squashed: false,
  ...over,
});

describe("buildWorkshopSteps", () => {
  // commits: newest first (as in unpushedCommits). C(top) -> B -> A(parent P).
  const commits = [c("C", ["B"]), c("B", ["A"]), c("A", ["P"])];

  it("null when nothing changed", () => {
    const edits = {
      C: edit({ subject: "S-C" }),
      B: edit({ subject: "S-B" }),
      A: edit({ subject: "S-A" }),
    };
    expect(buildWorkshopSteps(commits, edits)).toBeNull();
  });

  it("base = first parent of the oldest; covers the whole range (oldest first)", () => {
    const edits = {
      C: edit({ subject: "new-C" }), // reword
      B: edit({ subject: "S-B", authorName: "New", authorEmail: "n@x.de" }), // author-only -> pick+author
      A: edit({ subject: "S-A" }), // unchanged -> pick
    };
    const r = buildWorkshopSteps(commits, edits)!;
    expect(r.baseId).toBe("P");
    expect(r.steps.map((s) => [s.action, s.commitId])).toEqual([
      ["pick", "A"],
      ["pick", "B"],
      ["reword", "C"],
    ]);
    expect(r.steps[1].author).toBe("New <n@x.de>"); // B author changed
    expect(r.steps[1].message).toBeUndefined();
    expect(r.steps[2].message).toBe("new-C"); // C reword
  });

  it("discard -> drop", () => {
    const edits = {
      C: edit({ subject: "S-C", dropped: true }),
      B: edit({ subject: "S-B" }),
      A: edit({ subject: "S-A" }),
    };
    const r = buildWorkshopSteps(commits, edits)!;
    expect(r.steps.map((s) => s.action)).toEqual(["pick", "pick", "drop"]);
  });

  it("root within reach: the oldest is the root -> base=root, root read-only (not in steps)", () => {
    const rootRange = [c("C", ["B"]), c("B", ["A"]), c("A", [])]; // A is the root (no parents)
    const edits = {
      C: edit({ subject: "new" }),
      B: edit({ subject: "S-B" }),
      A: edit({ subject: "S-A" }),
    };
    const r = buildWorkshopSteps(rootRange, edits)!;
    expect(r.baseId).toBe("A");
    expect(r.steps.map((s) => s.commitId)).toEqual(["B", "C"]); // A (root) not included
  });

  it("subject AND author changed -> reword with message and author", () => {
    const commit = c("C", ["P"]);
    const edits = { C: edit({ subject: "new", authorName: "New", authorEmail: "n@x.de" }) };
    const r = buildWorkshopSteps([commit], edits)!;
    expect(r.steps).toHaveLength(1);
    expect(r.steps[0]).toMatchObject({
      action: "reword",
      message: "new",
      author: "New <n@x.de>",
    });
  });

  it("co-author roundtrip: an unchanged trailer -> no false positive (null)", () => {
    const commit = c("C", ["P"], {
      subject: "S-C",
      body: "Text\n\nCo-authored-by: X <x@x.de>",
    });
    const edits = { C: baselineOf(commit) };
    expect(buildWorkshopSteps([commit], edits)).toBeNull();
  });

  it("a coAuthors-only change -> a reword step", () => {
    const commit = c("C", ["P"], { subject: "S-C" });
    const base = baselineOf(commit);
    const edits = { C: { ...base, coAuthors: "X <x@x.de>" } };
    const r = buildWorkshopSteps([commit], edits)!;
    expect(r.steps).toHaveLength(1);
    expect(r.steps[0].action).toBe("reword");
  });
});

describe("authorValid", () => {
  it("valid for a non-empty name and email without angle brackets", () => {
    expect(authorValid("New", "n@x.de")).toBe(true);
  });

  it("invalid for empty, whitespace-only or angle brackets", () => {
    expect(authorValid("", "")).toBe(false);
    expect(authorValid(" ", "n@x.de")).toBe(false);
    expect(authorValid("New", "")).toBe(false);
    expect(authorValid("Ja<ne>", "n@x.de")).toBe(false);
  });
});

describe("buildWorkshopSteps with order and squash", () => {
  const commits = [c("C", ["B"]), c("B", ["A"]), c("A", ["P"])];
  const unchanged = {
    C: edit({ subject: "S-C" }),
    B: edit({ subject: "S-B" }),
    A: edit({ subject: "S-A" }),
  };

  it("a pure reorder yields steps in the new application order", () => {
    // Display (newest first): B, C, A -> application (oldest first): A, C, B
    const r = buildWorkshopSteps(commits, unchanged, ["B", "C", "A"])!;
    expect(r.steps.map((s) => s.commitId)).toEqual(["A", "C", "B"]);
    expect(r.steps.every((s) => s.action === "pick")).toBe(true);
  });

  it("the natural order without edits stays null", () => {
    expect(buildWorkshopSteps(commits, unchanged, ["C", "B", "A"])).toBeNull();
  });

  it("invalid order parameters fall back to the natural order", () => {
    expect(buildWorkshopSteps(commits, unchanged, ["C", "B"])).toBeNull();
    expect(buildWorkshopSteps(commits, unchanged, ["C", "B", "X"])).toBeNull();
    expect(buildWorkshopSteps(commits, unchanged, ["C", "C", "A"])).toBeNull();
  });

  it("squashed -> a squash step (the commit's own edits are irrelevant)", () => {
    const edits = { ...unchanged, B: edit({ subject: "whatever", squashed: true }) };
    const r = buildWorkshopSteps(commits, edits)!;
    expect(r.steps.map((s) => [s.action, s.commitId])).toEqual([
      ["pick", "A"],
      ["squash", "B"],
      ["pick", "C"],
    ]);
    expect(r.steps[1].message).toBeUndefined();
  });

  it("the root stays the base even on a reorder and falls out of the steps", () => {
    const rootRange = [c("C", ["B"]), c("B", ["A"]), c("A", [])];
    const edits = {
      C: edit({ subject: "S-C" }),
      B: edit({ subject: "S-B" }),
      A: edit({ subject: "S-A" }),
    };
    const r = buildWorkshopSteps(rootRange, edits, ["B", "C", "A"])!;
    expect(r.baseId).toBe("A");
    expect(r.steps.map((s) => s.commitId)).toEqual(["C", "B"]);
  });
});

describe("workshopOrderChanged", () => {
  const commits = [c("C", ["B"]), c("B", ["A"]), c("A", ["P"])];

  it("false for a natural/missing/invalid order, true for a real reorder", () => {
    expect(workshopOrderChanged(commits)).toBe(false);
    expect(workshopOrderChanged(commits, ["C", "B", "A"])).toBe(false);
    expect(workshopOrderChanged(commits, ["C", "B"])).toBe(false);
    expect(workshopOrderChanged(commits, ["B", "C", "A"])).toBe(true);
  });
});

describe("firstKeptIsSquash", () => {
  const commits = [c("C", ["B"]), c("B", ["A"]), c("A", ["P"])];
  const base = {
    C: edit({ subject: "S-C" }),
    B: edit({ subject: "S-B" }),
    A: edit({ subject: "S-A" }),
  };

  it("true when the oldest kept commit is squashed", () => {
    expect(
      firstKeptIsSquash(commits, { ...base, A: edit({ subject: "S-A", squashed: true }) }),
    ).toBe(true);
    // A is dropped -> B is the oldest one kept
    expect(
      firstKeptIsSquash(commits, {
        ...base,
        A: edit({ subject: "S-A", dropped: true }),
        B: edit({ subject: "S-B", squashed: true }),
      }),
    ).toBe(true);
  });

  it("false for a squash further up or without a squash", () => {
    expect(firstKeptIsSquash(commits, base)).toBe(false);
    expect(
      firstKeptIsSquash(commits, { ...base, C: edit({ subject: "S-C", squashed: true }) }),
    ).toBe(false);
  });

  it("honors the reorder (a squashed commit moved to the old end)", () => {
    // Display: C, A, B -> application: B, A, C; B squashed -> the first step is squash.
    expect(
      firstKeptIsSquash(commits, { ...base, B: edit({ subject: "S-B", squashed: true }) }, [
        "C",
        "A",
        "B",
      ]),
    ).toBe(true);
  });
});

describe("commitChanged", () => {
  it("false without an edit buffer and for the unchanged baseline", () => {
    const commit = c("C", ["P"]);
    expect(commitChanged(commit, undefined)).toBe(false);
    expect(commitChanged(commit, baselineOf(commit))).toBe(false);
  });

  it("true on a subject, author or drop change", () => {
    const commit = c("C", ["P"]);
    expect(commitChanged(commit, edit({ subject: "new" }))).toBe(true);
    expect(commitChanged(commit, edit({ subject: "S-C", authorName: "New" }))).toBe(true);
    expect(commitChanged(commit, edit({ subject: "S-C", dropped: true }))).toBe(true);
  });

  it("no false positive on the co-author trailer roundtrip", () => {
    const commit = c("C", ["P"], { body: "Text\n\nCo-authored-by: X <x@x.de>" });
    expect(commitChanged(commit, baselineOf(commit))).toBe(false);
  });
});

describe("changedCommitCount", () => {
  it("counts only actually changed commits (1 of 3)", () => {
    const commits = [c("C", ["B"]), c("B", ["A"]), c("A", ["P"])];
    const edits = {
      C: edit({ subject: "new-C" }), // changed
      B: edit({ subject: "S-B" }), // unchanged
      A: edit({ subject: "S-A" }), // unchanged
    };
    expect(changedCommitCount(commits, edits)).toBe(1);
  });
});
