// Sync flows: a refusal of git becomes a dialog with ways
// out — diverged branches (merge or rebase?), local changes in the way (bring
// them along), a rejected push (pull & merge / rebase, then push; or force).
// Conflicts on the way open the workshop; what waited (carried changes, the
// push) follows once the operation is finished.
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { RepoInfo, RepoStatus, StashInfo } from "./api";

vi.mock("@tauri-apps/plugin-dialog", () => ({ confirm: vi.fn(async () => false) }));

const cleanStatus: RepoStatus = {
  staged: [],
  unstaged: [],
  branch: "main",
  upstream: "origin/main",
  ahead: 1,
  behind: 1,
  opState: "clean",
};
const mergeConflict: RepoStatus = {
  ...cleanStatus,
  opState: "merge",
  unstaged: [{ path: "a.txt", origPath: null, kind: "conflicted" }],
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
    pull: vi.fn(async () => ""),
    push: vi.fn(async () => ""),
    pushRemote: vi.fn(async () => ""),
    mergeBranch: vi.fn(async () => ""),
    rebaseOnto: vi.fn(async () => ""),
    fetch: vi.fn(async () => ""),
    publishBranchOnly: vi.fn(async () => ""),
    createBranch: vi.fn(async () => {}),
    openRepository: vi.fn(async () => repo),
    continueOperation: vi.fn(async () => ""),
    stashPush: vi.fn(async () => "stashed"),
    stashList: vi.fn(async () => [] as StashInfo[]),
    stashPop: vi.fn(async () => ({ conflicted: [], dropped: true })),
    status: vi.fn(async () => cleanStatus),
    branches: vi.fn(async () => []),
    logAll: vi.fn(async () => []),
    undoStatus: vi.fn(async () => null),
  },
}));

import { confirm } from "@tauri-apps/plugin-dialog";
import { api } from "./api";
import { t } from "./i18n.svelte";
import {
  carryChangesThrough,
  chooseDivergent,
  clearToast,
  continueOperation,
  createBranch,
  gitFetchFrom,
  gitPull,
  gitPullFrom,
  gitPush,
  gitPushTo,
  mergeBranch,
  publishBranchOnly,
  rememberedUpdateSource,
  resolvePushRejected,
  ui,
  updateBranchFrom,
} from "./state.svelte";

const mockedApi = vi.mocked(api);
const mockedConfirm = vi.mocked(confirm);

/** The stash our carry creates, as git lists it. */
const carried = (): StashInfo => ({
  index: 0,
  message: `On main: ${t("sync.carryStash", { op: t("toolbar.pull") })}`,
  id: "c".repeat(40),
});

beforeEach(() => {
  vi.clearAllMocks();
  vi.stubGlobal("localStorage", { setItem: vi.fn(), getItem: vi.fn(() => null) });
  ui.repo = repo;
  ui.status = cleanStatus;
  ui.busy = null;
  ui.cloning = null;
  ui.working = 0;
  ui.modal = null;
  ui.view = "repo";
  ui.pullStrategy = "merge";
  ui.pullStrategyByRepo = {};
  ui.updateFromByBranch = {};
  clearToast();
  mockedApi.status.mockImplementation(async () => cleanStatus);
  mockedApi.stashList.mockImplementation(async () => [carried()]);
  mockedConfirm.mockImplementation(async () => false);
});

describe("pull with an explicit strategy", () => {
  it("passes the setting on", async () => {
    await gitPull();
    expect(mockedApi.pull).toHaveBeenCalledWith(
      "/repo",
      false,
      "merge",
      expect.any(Function),
      undefined,
    );
  });

  it("asks merge or rebase when the branches diverged — and can remember it", async () => {
    mockedApi.pull.mockRejectedValueOnce({ code: "divergent", message: "diverged" });

    await gitPull();
    expect(ui.modal).toEqual({ kind: "divergent", from: undefined });
    expect(ui.error).toBeNull();

    // The answer beats git's own pull.rebase / pull.ff ("-chosen") …
    await chooseDivergent("rebase", true);
    expect(mockedApi.pull).toHaveBeenLastCalledWith(
      "/repo",
      false,
      "rebase-chosen",
      expect.any(Function),
      undefined,
    );
    expect(ui.pullStrategyByRepo).toEqual({ "/repo": "rebase" });

    // … and so does the choice remembered for this repository.
    await gitPull();
    expect(mockedApi.pull).toHaveBeenLastCalledWith(
      "/repo",
      false,
      "rebase-chosen",
      expect.any(Function),
      undefined,
    );
  });
});

describe("local changes in the way of a pull", () => {
  it("offers to bring them along instead of an error", async () => {
    mockedApi.pull.mockRejectedValueOnce({
      code: "local_changes_overwritten",
      message: "would be overwritten",
    });

    await gitPull();

    expect(ui.modal).toEqual({ kind: "localChanges", then: { kind: "pull", strategy: undefined } });
    expect(ui.error).toBeNull();
  });

  it("names the files git listed as in the way", async () => {
    mockedApi.pull.mockRejectedValueOnce({
      code: "local_changes_overwritten",
      message: "would be overwritten",
      detail: [
        "error: The following untracked working tree files would be overwritten by merge:",
        "\tnotes.txt",
        "Please move or remove them before you merge.",
        "Aborting",
      ].join("\n"),
    });

    await gitPull();

    expect(ui.modal).toEqual({
      kind: "localChanges",
      then: { kind: "pull", strategy: undefined },
      files: ["notes.txt"],
    });
  });

  it("brings them along: stash, pull, put back", async () => {
    await carryChangesThrough({ kind: "pull" });

    expect(mockedApi.stashPush).toHaveBeenCalledTimes(1);
    expect(mockedApi.pull).toHaveBeenCalledTimes(1);
    expect(mockedApi.stashPop).toHaveBeenCalledWith("/repo", 0);
  });

  it("keeps them aside while the pull's conflicts are open — and brings them back after", async () => {
    mockedApi.pull.mockRejectedValueOnce({ code: "merge_conflict", message: "conflict" });
    mockedApi.status.mockImplementation(async () => mergeConflict);

    await carryChangesThrough({ kind: "pull" });
    expect(mockedApi.stashPop).not.toHaveBeenCalled();

    mockedApi.status.mockImplementation(async () => cleanStatus);
    await continueOperation();
    expect(mockedApi.stashPop).toHaveBeenCalledWith("/repo", 0);
  });
});

describe("local changes in the way of a merge", () => {
  it("offers the same dialog and carries the changes through the merge", async () => {
    mockedApi.mergeBranch.mockRejectedValueOnce({
      code: "local_changes_overwritten",
      message: "would be overwritten",
    });

    await mergeBranch("feature");
    expect(ui.modal).toEqual({ kind: "localChanges", then: { kind: "merge", name: "feature" } });

    mockedApi.stashList.mockImplementation(async () => [
      {
        index: 0,
        message: `On main: ${t("sync.carryStash", { op: t("sync.opMerge", { name: "feature" }) })}`,
        id: "m".repeat(40),
      },
    ]);
    await carryChangesThrough({ kind: "merge", name: "feature" });
    expect(mockedApi.mergeBranch).toHaveBeenLastCalledWith("/repo", "feature");
    expect(mockedApi.stashPop).toHaveBeenCalledWith("/repo", 0);
  });
});

describe("a rejected push", () => {
  const rejected = { code: "non_fast_forward", message: "remote is ahead" };

  it("a push to another remote pulls from THAT remote before pushing again", async () => {
    mockedApi.pushRemote.mockRejectedValueOnce(rejected);
    await gitPushTo("backup");

    await resolvePushRejected("merge");

    expect(mockedApi.pull).toHaveBeenCalledWith(
      "/repo",
      false,
      "merge-chosen",
      expect.any(Function),
      { remote: "backup", branch: "main" },
    );
    expect(mockedApi.pushRemote).toHaveBeenCalledTimes(2);
  });

  it("pull & merge, then push again", async () => {
    mockedApi.push.mockRejectedValueOnce(rejected);
    await gitPush();

    await resolvePushRejected("merge");

    expect(mockedApi.pull).toHaveBeenCalledWith(
      "/repo",
      false,
      "merge-chosen",
      expect.any(Function),
      undefined,
    );
    expect(mockedApi.push).toHaveBeenCalledTimes(2);
    expect(mockedApi.pushRemote).not.toHaveBeenCalled();
  });

  it("rebase, then push again", async () => {
    mockedApi.push.mockRejectedValueOnce(rejected);
    await gitPush();

    await resolvePushRejected("rebase");

    expect(mockedApi.pull).toHaveBeenCalledWith(
      "/repo",
      false,
      "rebase-chosen",
      expect.any(Function),
      undefined,
    );
    expect(mockedApi.push).toHaveBeenCalledTimes(2);
  });

  it("waits for the conflicts of the pull and offers the push afterwards", async () => {
    mockedApi.push.mockRejectedValueOnce(rejected);
    await gitPush();
    mockedApi.pull.mockRejectedValueOnce({ code: "merge_conflict", message: "conflict" });
    mockedApi.status.mockImplementation(async () => mergeConflict);

    await resolvePushRejected("merge");
    expect(mockedApi.push).toHaveBeenCalledTimes(1);

    mockedApi.status.mockImplementation(async () => cleanStatus);
    mockedConfirm.mockResolvedValueOnce(true);
    await continueOperation();

    expect(mockedConfirm).toHaveBeenCalledWith(t("sync.pushNow"), expect.anything());
    expect(mockedApi.push).toHaveBeenCalledTimes(2);
  });
});

describe("update the current branch from another one", () => {
  it("fetches first, merges the remote state and remembers the choice", async () => {
    await updateBranchFrom("main", "origin/main", "merge", true);

    expect(mockedApi.fetch).toHaveBeenCalledTimes(1);
    expect(mockedApi.mergeBranch).toHaveBeenCalledWith("/repo", "origin/main");
    expect(rememberedUpdateSource()).toBe("main");
  });

  it("rebases without a fetch when asked to", async () => {
    await updateBranchFrom("feature/x", "feature/x", "rebase", false);

    expect(mockedApi.fetch).not.toHaveBeenCalled();
    expect(mockedApi.rebaseOnto).toHaveBeenCalledWith("/repo", "feature/x");
  });

  it("stops when the fetch fails", async () => {
    mockedApi.fetch.mockRejectedValueOnce({ code: "network", message: "offline" });

    await updateBranchFrom("main", "origin/main", "merge", true);

    expect(mockedApi.mergeBranch).not.toHaveBeenCalled();
  });

  it("local changes in the way get the bring-along dialog", async () => {
    mockedApi.mergeBranch.mockRejectedValueOnce({
      code: "local_changes_overwritten",
      message: "would be overwritten",
    });

    await updateBranchFrom("main", "main", "merge", false);

    expect(ui.modal).toEqual({ kind: "localChanges", then: { kind: "merge", name: "main" } });
  });
});

describe("pull and fetch from a chosen remote", () => {
  it("pulls from the chosen remote and branch", async () => {
    await gitPullFrom("mirror", "main");
    expect(mockedApi.pull).toHaveBeenCalledWith("/repo", false, "merge", expect.any(Function), {
      remote: "mirror",
      branch: "main",
    });
  });

  it("fetches one remote, or all of them", async () => {
    await gitFetchFrom("mirror");
    expect(mockedApi.fetch).toHaveBeenLastCalledWith("/repo", expect.any(Function), {
      remote: "mirror",
    });
    await gitFetchFrom();
    expect(mockedApi.fetch).toHaveBeenLastCalledWith("/repo", expect.any(Function), {
      all: true,
    });
  });
});

describe("publish the branch only", () => {
  it("publishes to the push target remote", async () => {
    ui.status = { ...cleanStatus, upstream: null };
    ui.remotes = [{ name: "origin", url: "https://example.com/r.git" }];

    await publishBranchOnly();

    expect(mockedApi.publishBranchOnly).toHaveBeenCalledWith(
      "/repo",
      "origin",
      expect.any(Function),
    );
  });

  it("a new branch can be published right away", async () => {
    ui.remotes = [{ name: "origin", url: "https://example.com/r.git" }];

    await createBranch("feature/new", true);
    expect(mockedApi.publishBranchOnly).toHaveBeenCalledTimes(1);

    await createBranch("feature/other");
    expect(mockedApi.publishBranchOnly).toHaveBeenCalledTimes(1);
  });
});
