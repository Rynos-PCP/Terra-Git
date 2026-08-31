// Blocked branch switch (user finding 2026-08-15: "cannot switch branches
// because of conflicts, but no conflicts are shown").
// libgit2 calls locally modified files "conflicts" — the engine error now
// carries its own code `checkout_would_overwrite` for that. The wiring hangs
// here: the toast has to offer the way out including the target branch, and the
// way out has to stash FIRST and switch AFTERWARDS.
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { RepoInfo, RepoStatus } from "./api";

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
    stashPush: vi.fn(async () => "stashed"),
    stashList: vi.fn(async () => []),
    status: vi.fn(async () => cleanStatus),
    branches: vi.fn(async () => []),
    logAll: vi.fn(async () => []),
    openRepository: vi.fn(async () => repo),
    undoStatus: vi.fn(async () => null),
  },
}));

import { api } from "./api";
import { showError, stashAndSwitch, switchBranch, ui } from "./state.svelte";

const mockedApi = vi.mocked(api);

/** The error the engine returns for a blocked checkout. */
const blockedError = { code: "checkout_would_overwrite", message: "a.txt, b.txt" };

beforeEach(() => {
  vi.clearAllMocks();
  ui.repo = repo;
  ui.busy = null;
  ui.working = 0;
  ui.error = null;
  ui.info = null;
  ui.errorAction = null;
  mockedApi.checkoutBranch.mockImplementation(async () => {});
});

describe("switchBranch() on a blocked switch", () => {
  it("offers the way out — with the branch that was meant to be switched to", async () => {
    mockedApi.checkoutBranch.mockRejectedValueOnce(blockedError);

    await switchBranch("feature");

    // The message names the files (catalog text with {detail}) …
    expect(ui.error).toContain("a.txt, b.txt");
    // … and the toast knows where it was supposed to go.
    expect(ui.errorAction).toEqual({
      kind: "stashSwitch",
      target: { kind: "branch", name: "feature" },
    });
    // No conflict offer: no operation is running, the workshop would bounce
    // straight back.
    expect(ui.busy).toBeNull();
  });

  it("does NOT set the way out for other errors", async () => {
    mockedApi.checkoutBranch.mockRejectedValueOnce({
      code: "branch_not_found",
      message: "Branch not found: gone",
    });

    await switchBranch("gone");

    expect(ui.errorAction).toBeNull();
  });

  it("clears an old way out as soon as another error arrives", () => {
    ui.errorAction = { kind: "stashSwitch", target: { kind: "branch", name: "feature" } };

    showError({ code: "network", message: "Remote unreachable" });

    expect(ui.errorAction).toBeNull();
  });
});

describe("stashAndSwitch()", () => {
  it("stashes first and switches AFTERWARDS", async () => {
    const order: string[] = [];
    mockedApi.stashPush.mockImplementationOnce(async () => {
      order.push("stash");
      return "stashed";
    });
    mockedApi.checkoutBranch.mockImplementationOnce(async () => {
      order.push("checkout");
    });

    await stashAndSwitch({ kind: "branch", name: "feature" });

    expect(order).toEqual(["stash", "checkout"]);
    // Stash everything, untracked included: an empty file list means "everything".
    expect(mockedApi.stashPush).toHaveBeenCalledWith("/repo", expect.any(String), []);
    expect(mockedApi.checkoutBranch).toHaveBeenCalledWith("/repo", "feature", expect.any(Function));
  });

  it("does NOT switch when the stash already fails", async () => {
    mockedApi.stashPush.mockRejectedValueOnce({ code: "sidecar_failed", message: "broken" });

    await stashAndSwitch({ kind: "branch", name: "feature" });

    expect(mockedApi.checkoutBranch).not.toHaveBeenCalled();
    expect(ui.error).toContain("broken");
    // The counter must not get stuck (buttons would stay disabled).
    expect(ui.working).toBe(0);
  });
});
