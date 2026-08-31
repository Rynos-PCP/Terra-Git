// Branch switching with uncommitted changes (user request 2026-08-16: "the way
// GitHub Desktop does it"): ask first, then bring along OR leave here — and what
// was left behind comes back by itself when switching back.
//
// The wiring hangs here: that the question is asked at all, that every path takes
// its steps in the right ORDER, and that a failed step does not make the changes
// disappear.
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { RepoInfo, RepoStatus, StashInfo } from "./api";
import { AUTOSTASH_MARKER } from "./branchSwitch";

vi.mock("@tauri-apps/plugin-dialog", () => ({ confirm: vi.fn(async () => false) }));

const cleanStatus: RepoStatus = {
  staged: [],
  unstaged: [],
  branch: "main",
  upstream: null,
  ahead: 0,
  behind: 0,
  opState: "clean",
};

const dirtyStatus: RepoStatus = {
  ...cleanStatus,
  unstaged: [{ path: "a.txt", origPath: null, kind: "modified" }],
};

const repo: RepoInfo = {
  path: "/repo",
  name: "repo",
  currentBranch: "main",
  headDetached: false,
  isEmpty: false,
  historyPrepared: true,
};

vi.mock("./api", () => ({
  api: {
    checkoutBranch: vi.fn(async () => {}),
    checkoutCommit: vi.fn(async () => {}),
    cherryPick: vi.fn(async () => ""),
    stashPush: vi.fn(async () => "stashed"),
    stashList: vi.fn(async () => [] as StashInfo[]),
    stashPop: vi.fn(async () => {}),
    status: vi.fn(async () => cleanStatus),
    branches: vi.fn(async () => []),
    logAll: vi.fn(async () => []),
    openRepository: vi.fn(async () => repo),
    undoStatus: vi.fn(async () => null),
  },
}));

import { api } from "./api";
import type { SwitchTarget } from "./branchSwitch";
import { t } from "./i18n.svelte";
import {
  carryChangesAndSwitch,
  checkoutCommit,
  cherryPickOnto,
  stashAndSwitch,
  switchBranch,
  ui,
} from "./state.svelte";

const mockedApi = vi.mocked(api);

/** Shorthand for the most common target — a branch. */
const br = (name: string): SwitchTarget => ({ kind: "branch", name });

/** Text of the bring-along stash — built through t() so the test does not stick
 *  to one language (the backend finds it by exactly this text). */
const carryMessage = (name: string) => t("state.stashCarryOver", { name });

/** The error the engine returns for colliding files. */
const blocked = { code: "checkout_would_overwrite", message: "a.txt" };

/** A stash we created when leaving `branch`. */
const autoStash = (index: number, branch: string): StashInfo => ({
  index,
  // The way git stores it: its own "On <branch>: " in front.
  message: `On ${branch}: ${AUTOSTASH_MARKER}${branch}`,
  id: `${index}`.repeat(40),
});

beforeEach(() => {
  vi.clearAllMocks();
  ui.repo = repo;
  ui.status = cleanStatus;
  ui.busy = null;
  ui.working = 0;
  ui.error = null;
  ui.info = null;
  ui.errorAction = null;
  ui.modal = null;
  ui.stashes = [];
  mockedApi.checkoutBranch.mockImplementation(async () => {});
  mockedApi.stashList.mockImplementation(async () => []);
  mockedApi.stashPop.mockImplementation(async () => {});
  mockedApi.status.mockImplementation(async () => cleanStatus);
});

describe("switchBranch() — item 1: ask first", () => {
  it("switches without a question on a clean worktree", async () => {
    await switchBranch("feature");

    expect(ui.modal).toBeNull();
    expect(mockedApi.checkoutBranch).toHaveBeenCalledWith("/repo", "feature", expect.any(Function));
  });

  it("asks on uncommitted changes — and does NOT switch by itself", async () => {
    ui.status = dirtyStatus;

    await switchBranch("feature");

    expect(ui.modal).toEqual({ kind: "switchBranch", target: br("feature") });
    expect(mockedApi.checkoutBranch).not.toHaveBeenCalled();
  });

  it("does not ask during a running operation — git refuses anyway", async () => {
    ui.status = { ...dirtyStatus, opState: "merge" };

    await switchBranch("feature");

    expect(ui.modal).toBeNull();
    expect(mockedApi.checkoutBranch).toHaveBeenCalled();
  });
});

describe("stashAndSwitch() — item 2: leave here", () => {
  it("stashes with the marker of the branch being LEFT and switches afterwards", async () => {
    const order: string[] = [];
    mockedApi.stashPush.mockImplementationOnce(async () => {
      order.push("stash");
      return "stashed";
    });
    mockedApi.checkoutBranch.mockImplementationOnce(async () => {
      order.push("checkout");
    });
    ui.status = { ...dirtyStatus, branch: "main" };

    await stashAndSwitch(br("feature"));

    expect(order).toEqual(["stash", "checkout"]);
    // The marker carries "main" — that is where the changes should go back to,
    // not to the target.
    expect(mockedApi.stashPush).toHaveBeenCalledWith("/repo", `${AUTOSTASH_MARKER}main`, []);
  });

  it("does NOT mark on a detached HEAD — no branch name leads back there", async () => {
    ui.repo = { ...repo, headDetached: true };
    ui.status = { ...dirtyStatus, branch: "HEAD" };

    await stashAndSwitch(br("feature"));

    const message = mockedApi.stashPush.mock.calls[0][1];
    expect(message).not.toContain(AUTOSTASH_MARKER);
  });

  it("shows the way back when the switch fails after stashing", async () => {
    mockedApi.checkoutBranch.mockRejectedValueOnce({
      code: "branch_not_found",
      message: "Branch not found: gone",
    });

    await stashAndSwitch(br("gone"));

    // The changes have disappeared from the worktree — the toast has to say
    // where to, otherwise they look lost.
    expect(ui.errorAction).toEqual({ kind: "stashes" });
  });

  it("does NOT switch when the stash already fails", async () => {
    mockedApi.stashPush.mockRejectedValueOnce({ code: "sidecar_failed", message: "broken" });

    await stashAndSwitch(br("feature"));

    expect(mockedApi.checkoutBranch).not.toHaveBeenCalled();
    expect(ui.error).toContain("broken");
    expect(ui.working).toBe(0);
  });
});

describe("carryChangesAndSwitch() — item 3: bring along", () => {
  it("brings them along without a stash while no file collides", async () => {
    await carryChangesAndSwitch(br("feature"));

    expect(mockedApi.checkoutBranch).toHaveBeenCalledWith("/repo", "feature", expect.any(Function));
    // No unnecessary stash detour: git brings them along by itself.
    expect(mockedApi.stashPush).not.toHaveBeenCalled();
  });

  it("takes the detour on a collision: stash, switch, apply again", async () => {
    const order: string[] = [];
    mockedApi.checkoutBranch
      .mockImplementationOnce(async () => {
        order.push("checkout-blocked");
        throw blocked;
      })
      .mockImplementationOnce(async () => {
        order.push("checkout");
      });
    mockedApi.stashPush.mockImplementationOnce(async () => {
      order.push("stash");
      return "stashed";
    });
    // Our own stash is found again by its text, not by index 0 — otherwise a
    // foreign entry would be popped.
    mockedApi.stashList.mockImplementation(async () => [
      { index: 0, message: "On main: foreign stash", id: "f".repeat(40) },
      { index: 1, message: `On main: ${carryMessage("feature")}`, id: "e".repeat(40) },
    ]);
    mockedApi.stashPop.mockImplementationOnce(async () => {
      order.push("pop");
    });

    await carryChangesAndSwitch(br("feature"));

    expect(order).toEqual(["checkout-blocked", "stash", "checkout", "pop"]);
    expect(mockedApi.stashPop).toHaveBeenCalledWith("/repo", 1);
    expect(ui.error).toBeNull();
  });

  it("leaves the changes in the stash and shows the way back when applying fails", async () => {
    mockedApi.checkoutBranch.mockImplementationOnce(async () => {
      throw blocked;
    });
    mockedApi.stashList.mockImplementation(async () => [
      { index: 0, message: `On main: ${carryMessage("feature")}`, id: "e".repeat(40) },
    ]);
    mockedApi.stashPop.mockRejectedValueOnce({
      code: "git_error",
      message: "1 conflict prevents checkout",
    });

    await carryChangesAndSwitch(br("feature"));

    expect(ui.error).toContain("conflict");
    expect(ui.errorAction).toEqual({ kind: "stashes" });
    expect(ui.working).toBe(0);
  });

  it("reports other checkout errors directly, without stashing anything", async () => {
    mockedApi.checkoutBranch.mockRejectedValueOnce({
      code: "branch_not_found",
      message: "Branch not found: gone",
    });

    await carryChangesAndSwitch(br("gone"));

    expect(mockedApi.stashPush).not.toHaveBeenCalled();
    expect(ui.error).toContain("gone");
  });
});

describe("item 4: what was left behind comes back by itself", () => {
  it("applies the marked stash of the target branch after the switch", async () => {
    mockedApi.stashList.mockImplementation(async () => [autoStash(0, "feature")]);

    await switchBranch("feature");

    expect(mockedApi.stashPop).toHaveBeenCalledWith("/repo", 0);
    expect(ui.info).toBe(t("state.autoStashRestored", { name: "feature" }));
  });

  it("never touches foreign stashes — only the marker counts", async () => {
    mockedApi.stashList.mockImplementation(async () => [
      { index: 0, message: "On feature: WIP by hand", id: "f".repeat(40) },
      autoStash(1, "otherBranch"),
    ]);

    await switchBranch("feature");

    expect(mockedApi.stashPop).not.toHaveBeenCalled();
  });

  it("applies nothing when changes were brought along — the stash stays put", async () => {
    // After the switch something lies in the worktree (the changes brought along).
    mockedApi.status.mockImplementation(async () => dirtyStatus);
    mockedApi.stashList.mockImplementation(async () => [autoStash(0, "feature")]);

    await switchBranch("feature");

    expect(mockedApi.stashPop).not.toHaveBeenCalled();
    // And the hint does not keep quiet about the waiting stash.
    expect(ui.info).toBe(t("state.autoStashKept", { name: "feature" }));
  });

  it("does not run a failed switch through the restoration", async () => {
    mockedApi.checkoutBranch.mockRejectedValueOnce({ code: "branch_not_found", message: "gone" });
    mockedApi.stashList.mockImplementation(async () => [autoStash(0, "gone")]);

    await switchBranch("gone");

    expect(mockedApi.stashPop).not.toHaveBeenCalled();
  });

  it("does not report a stash list error as a failed switch", async () => {
    mockedApi.stashList.mockRejectedValue({ code: "sidecar_failed", message: "broken" });

    await switchBranch("feature");

    // The switch itself worked — the restoration is a convenience.
    expect(ui.error).toBeNull();
    expect(ui.info).toContain("feature");
  });
});

describe("item 5: EVERY checkout path asks the same question", () => {
  // switchBranch() used to ask while the two other
  // checkout paths did not — the same user state behaved differently depending on
  // the entry point (sometimes a dialog, sometimes a raw error message).

  it("checkoutCommit() asks on uncommitted changes", async () => {
    ui.status = dirtyStatus;

    await checkoutCommit("abc1234def");

    expect(ui.modal).toEqual({
      kind: "switchBranch",
      target: { kind: "commit", id: "abc1234def" },
    });
    expect(mockedApi.checkoutCommit).not.toHaveBeenCalled();
  });

  it("checkoutCommit() checks out directly on a clean worktree", async () => {
    await checkoutCommit("abc1234def");

    expect(ui.modal).toBeNull();
    expect(mockedApi.checkoutCommit).toHaveBeenCalledWith("/repo", "abc1234def");
  });

  it("checkoutCommit() restores NO auto stash — a commit carries no marker", async () => {
    // A marked stash is ready but belongs to a BRANCH. After a detached checkout
    // it must not be unpacked.
    mockedApi.stashList.mockImplementation(async () => [autoStash(0, "main")]);

    await checkoutCommit("abc1234def");

    expect(mockedApi.stashPop).not.toHaveBeenCalled();
  });

  it("cherryPickOnto() asks — and picks only AFTER the switch", async () => {
    ui.status = dirtyStatus;

    await cherryPickOnto("c0ffee00", "feature");

    expect(ui.modal).toEqual({
      kind: "switchBranch",
      target: br("feature"),
      andThen: { kind: "cherryPick", commitId: "c0ffee00" },
    });
    expect(mockedApi.checkoutBranch).not.toHaveBeenCalled();
    expect(mockedApi.cherryPick).not.toHaveBeenCalled();
  });

  it("cherryPickOnto() runs through without a question on a clean worktree", async () => {
    const order: string[] = [];
    mockedApi.checkoutBranch.mockImplementationOnce(async () => {
      order.push("checkout");
    });
    mockedApi.cherryPick.mockImplementationOnce(async () => {
      order.push("pick");
      return "";
    });

    await cherryPickOnto("c0ffee00", "feature");

    expect(order).toEqual(["checkout", "pick"]);
  });

  it("cherryPickOnto() does NOT pick when the switch already fails", async () => {
    mockedApi.checkoutBranch.mockRejectedValueOnce({
      code: "branch_not_found",
      message: "Branch not found: gone",
    });

    await cherryPickOnto("c0ffee00", "gone");

    expect(mockedApi.cherryPick).not.toHaveBeenCalled();
    expect(ui.error).toContain("gone");
  });

  it("choosing to leave changes behind still finishes the cherry-pick", async () => {
    const order: string[] = [];
    mockedApi.stashPush.mockImplementationOnce(async () => {
      order.push("stash");
      return "stashed";
    });
    mockedApi.checkoutBranch.mockImplementationOnce(async () => {
      order.push("checkout");
    });
    mockedApi.cherryPick.mockImplementationOnce(async () => {
      order.push("pick");
      return "";
    });
    ui.status = dirtyStatus;

    await stashAndSwitch(br("feature"), { kind: "cherryPick", commitId: "c0ffee00" });

    expect(order).toEqual(["stash", "checkout", "pick"]);
  });
});

describe("item 6: the bring-along detour does not hide what was left behind", () => {
  // carryViaStash() turns the restoration
  // off (correctly — the changes brought along must not be overwritten), but in
  // doing so it also kept quiet about a waiting auto stash of the target branch.
  // The user considered their older work lost.
  //

  /** Force a collision so the detour is taken at all. */
  function forceDetour() {
    mockedApi.checkoutBranch
      .mockImplementationOnce(async () => {
        throw blocked;
      })
      .mockImplementationOnce(async () => {});
  }

  it("names the waiting auto stash of the target branch", async () => {
    forceDetour();
    mockedApi.stashList.mockImplementation(async () => [
      { index: 0, message: `On main: ${carryMessage("feature")}`, id: "e".repeat(40) },
      autoStash(1, "feature"),
    ]);

    await carryChangesAndSwitch(br("feature"));

    expect(ui.info).toBe(t("state.autoStashKept", { name: "feature" }));
  });

  it("does NOT apply it — the changes brought along stay untouched", async () => {
    forceDetour();
    mockedApi.stashList.mockImplementation(async () => [
      { index: 0, message: `On main: ${carryMessage("feature")}`, id: "e".repeat(40) },
      autoStash(1, "feature"),
    ]);

    await carryChangesAndSwitch(br("feature"));

    // Exactly once: our own bring-along stash. The foreign one stays put.
    expect(mockedApi.stashPop).toHaveBeenCalledTimes(1);
    expect(mockedApi.stashPop).toHaveBeenCalledWith("/repo", 0);
  });

  it("reports nothing when no auto stash is waiting at all", async () => {
    forceDetour();
    mockedApi.stashList.mockImplementation(async () => [
      { index: 0, message: `On main: ${carryMessage("feature")}`, id: "e".repeat(40) },
    ]);

    await carryChangesAndSwitch(br("feature"));

    expect(ui.info).toBe(t("state.changesCarriedOver", { name: "feature" }));
  });
});

describe("item 7: the lock spans the WHOLE flow", () => {
  // Three related races shared one root: ui.busy covered
  // only the checkout itself. Between stash and checkout a parallel operation
  // could slip in, the leftover stash was looked up and popped after the
  // lock was released, and ui.repo was re-read after every await.

  /** A second repo the UI switches to while the flow is still running. */
  const other: RepoInfo = { ...repo, path: "/other", name: "other" };

  it("holds the lock from the stash on — a second switch during the stash is refused", async () => {
    let busyDuringStash: string | null = null;
    mockedApi.stashPush.mockImplementationOnce(async () => {
      busyDuringStash = ui.busy;
      // Someone clicks another branch while the stash is still running.
      await switchBranch("third");
      return "stashed";
    });
    ui.status = dirtyStatus;

    await stashAndSwitch(br("feature"));

    expect(busyDuringStash).not.toBeNull();
    // Exactly the switch that was started — the second one bounced off the lock.
    expect(mockedApi.checkoutBranch).toHaveBeenCalledTimes(1);
    expect(mockedApi.checkoutBranch).toHaveBeenCalledWith("/repo", "feature", expect.any(Function));
    // Released at the end, with no counter stuck.
    expect(ui.busy).toBeNull();
    expect(ui.working).toBe(0);
  });

  it("keeps the lock while the leftover stash is looked up — no switch can move HEAD in between", async () => {
    mockedApi.stashList.mockImplementationOnce(async () => {
      // The stash list is still loading when the next click arrives.
      await switchBranch("third");
      return [autoStash(0, "feature")];
    });

    await switchBranch("feature");

    expect(mockedApi.checkoutBranch).toHaveBeenCalledTimes(1);
    // The stash of "feature" is unpacked on "feature" — not on "third".
    expect(mockedApi.stashPop).toHaveBeenCalledWith("/repo", 0);
    expect(ui.busy).toBeNull();
  });

  it("does not unpack a leftover stash once the UI shows another repo", async () => {
    mockedApi.stashList.mockImplementationOnce(async () => {
      // The repo switch lands while the stash list of the old repo is loading.
      ui.repo = other;
      return [autoStash(0, "feature")];
    });

    await switchBranch("feature");

    // Looked up in the repo the flow started in …
    expect(mockedApi.stashList).toHaveBeenCalledWith("/repo");
    // … but nothing is popped anywhere: not into /other, and not blindly into /repo.
    expect(mockedApi.stashPop).not.toHaveBeenCalled();
  });

  it("finishes the bring-along detour in the repo it started in", async () => {
    mockedApi.openRepository.mockImplementationOnce(async (path: string) => ({ ...repo, path }));
    mockedApi.checkoutBranch
      .mockImplementationOnce(async () => {
        throw blocked;
      })
      .mockImplementationOnce(async () => {
        // The UI switches repos during the checkout.
        ui.repo = other;
      });
    mockedApi.stashList.mockImplementation(async () => [
      { index: 0, message: `On main: ${carryMessage("feature")}`, id: "e".repeat(40) },
    ]);

    await carryChangesAndSwitch(br("feature"));

    // Every mutating git call of the flow went to /repo — the stash is popped where it
    // was pushed, never in the repo the user never chose for this.
    const calls = [
      ...mockedApi.stashPush.mock.calls,
      ...mockedApi.stashPop.mock.calls,
      ...mockedApi.checkoutBranch.mock.calls,
    ];
    expect(calls.length).toBeGreaterThan(0);
    for (const call of calls) expect(call[0]).toBe("/repo");
    expect(mockedApi.stashPop).toHaveBeenCalledWith("/repo", 0);
    expect(ui.busy).toBeNull();
    expect(ui.working).toBe(0);
  });
});
