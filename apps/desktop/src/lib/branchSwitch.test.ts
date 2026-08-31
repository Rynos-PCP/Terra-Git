// Branch switching with uncommitted changes: when the question is asked, and
// how changes left behind are assigned back to their branch.
//
// The marker has to survive a language change — otherwise the changes would no
// longer find their way back. That is why it lives here and not in the message
// catalog.
import { describe, expect, it } from "vitest";
import {
  AUTOSTASH_MARKER,
  autoStashMessage,
  findAutoStash,
  needsSwitchChoice,
  parseAutoStashBranch,
  switchTargetLabel,
  worktreeDirty,
} from "./branchSwitch";

const clean = { staged: [], unstaged: [], opState: "clean" };
const dirty = { staged: [], unstaged: [{ path: "a.txt" }], opState: "clean" };

describe("needsSwitchChoice()", () => {
  it("asks as soon as something uncommitted is in the worktree", () => {
    expect(needsSwitchChoice(dirty)).toBe(true);
    expect(needsSwitchChoice({ staged: [{ path: "b.txt" }], unstaged: [], opState: "clean" })).toBe(
      true,
    );
  });

  it("does not ask on a clean worktree", () => {
    expect(needsSwitchChoice(clean)).toBe(false);
  });

  it("does not ask during a running operation — git refuses anyway", () => {
    expect(needsSwitchChoice({ ...dirty, opState: "merge" })).toBe(false);
    expect(needsSwitchChoice({ ...dirty, opState: "rebase" })).toBe(false);
  });

  it("does not ask without a status (not loaded yet)", () => {
    expect(needsSwitchChoice(null)).toBe(false);
    expect(needsSwitchChoice(undefined)).toBe(false);
  });
});

describe("worktreeDirty()", () => {
  it("counts both sides of the changes list", () => {
    expect(worktreeDirty(clean)).toBe(false);
    expect(worktreeDirty(dirty)).toBe(true);
    expect(worktreeDirty(null)).toBe(false);
  });
});

describe("auto-stash marker", () => {
  it("writes and reads the same branch", () => {
    const msg = autoStashMessage("feature/palette");
    expect(msg).toContain(AUTOSTASH_MARKER);
    expect(parseAutoStashBranch(msg)).toBe("feature/palette");
  });

  // `git stash push -m "…"` prepends an "On <branch>: " of its own — the marker
  // is then NOT at the start of the line.
  it("finds the marker behind the “On <branch>: ” that git prepends", () => {
    expect(parseAutoStashBranch("On main: terra-git-autostash:main")).toBe("main");
    expect(parseAutoStashBranch("On feature/x: terra-git-autostash:feature/x")).toBe("feature/x");
  });

  it("leaves foreign stashes alone", () => {
    expect(parseAutoStashBranch("On main: WIP before lunch")).toBeNull();
    expect(parseAutoStashBranch("")).toBeNull();
    // A marker without a branch after it does not count — it would otherwise pop everywhere.
    expect(parseAutoStashBranch("On main: terra-git-autostash:")).toBeNull();
  });

  it("copes with branch names containing colon-free special characters", () => {
    expect(parseAutoStashBranch(autoStashMessage("ui/conflict-entries"))).toBe(
      "ui/conflict-entries",
    );
  });
});

describe("findAutoStash()", () => {
  const stashes = [
    { index: 0, message: "On main: terra-git-autostash:main" },
    { index: 1, message: "On main: WIP by hand" },
    { index: 2, message: "On feature: terra-git-autostash:feature" },
    { index: 3, message: "On main: terra-git-autostash:main" },
  ];

  it("finds the stash of the branch", () => {
    expect(findAutoStash(stashes, "feature")?.index).toBe(2);
  });

  it("takes the newest of several (git lists newest first)", () => {
    expect(findAutoStash(stashes, "main")?.index).toBe(0);
  });

  it("returns null without a hit — foreign stashes are never touched", () => {
    expect(findAutoStash(stashes, "release")).toBeNull();
    expect(findAutoStash([{ index: 0, message: "WIP by hand" }], "main")).toBeNull();
  });
});

describe("switchTargetLabel()", () => {
  it("calls a branch by its name", () => {
    expect(switchTargetLabel({ kind: "branch", name: "feature/palette" })).toBe("feature/palette");
  });

  it("shortens a commit id to the usual short form", () => {
    expect(switchTargetLabel({ kind: "commit", id: "abc1234def567890" })).toBe("abc1234d");
  });

  it("leaves an already short id untouched", () => {
    expect(switchTargetLabel({ kind: "commit", id: "abc123" })).toBe("abc123");
  });
});
