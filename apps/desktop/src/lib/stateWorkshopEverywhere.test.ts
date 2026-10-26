// The conflict workshop is THE place for conflicts: it opens
// by itself when an operation ends in conflicts, it also handles conflicts that
// belong to no operation (a stash applied with conflicts), and its Done / Give
// up finish them — a popped stash is only dropped once everything is resolved.
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { RepoInfo, RepoStatus, StashApplyOutcome, StashInfo } from "./api";

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
const conflictStatus = (opState: RepoStatus["opState"]): RepoStatus => ({
  ...cleanStatus,
  opState,
  unstaged: [{ path: "a.txt", origPath: null, kind: "conflicted" }],
});

const repo: RepoInfo = {
  path: "/repo",
  name: "repo",
  currentBranch: "main",
  headDetached: false,
  isEmpty: false,
  historyPrepared: true,
};

const stash: StashInfo = { index: 0, message: "On main: WIP", id: "s".repeat(40) };
const conflicted: StashApplyOutcome = { conflicted: ["a.txt"], dropped: false };

vi.mock("./api", () => ({
  api: {
    status: vi.fn(async () => cleanStatus),
    stashList: vi.fn(async () => [] as StashInfo[]),
    stashPop: vi.fn(async () => ({ conflicted: [], dropped: true })),
    stashApply: vi.fn(async () => ({ conflicted: [], dropped: false })),
    stashDrop: vi.fn(async () => {}),
    abandonConflicts: vi.fn(async () => "overwrite-1-0"),
    undoStatus: vi.fn(async () => null),
  },
}));

import { workshopAvailable } from "./conflictWorkshop";
import { api } from "./api";
import { t } from "./i18n.svelte";
import {
  abandonConflicts,
  clearToast,
  finishConflicts,
  refreshStatus,
  showError,
  stashApply,
  stashPop,
  ui,
} from "./state.svelte";

const mockedApi = vi.mocked(api);

beforeEach(() => {
  vi.clearAllMocks();
  ui.repo = repo;
  ui.status = cleanStatus;
  ui.view = "repo";
  ui.working = 0;
  ui.stashes = [stash];
  ui.stashConflict = null;
  ui.conflictFocus = null;
  clearToast();
  mockedApi.status.mockImplementation(async () => cleanStatus);
  mockedApi.stashList.mockImplementation(async () => [stash]);
});

describe("workshopAvailable()", () => {
  it("also counts conflicts without a running operation", () => {
    expect(workshopAvailable("clean", 1)).toBe(true);
    expect(workshopAvailable("clean", 0)).toBe(false);
    expect(workshopAvailable("merge", 0)).toBe(true);
  });
});

describe("an operation ending in conflicts opens the workshop", () => {
  it("goes straight into the workshop once the status shows the conflicts", async () => {
    mockedApi.status.mockImplementation(async () => conflictStatus("merge"));

    showError({
      code: "sidecar_failed",
      message: "git command failed",
      detail: "CONFLICT (content): Merge conflict in a.txt",
    });

    await vi.waitFor(() => expect(ui.view).toBe("conflicts"));
  });

  it("still opens when the operation's own refresh overtakes the error's", async () => {
    // Found by an E2E flow: runRebase's `finally` refreshes right after
    // showError did, the first answer is discarded as a late one — the
    // decision must follow the status that is actually applied.
    let release!: () => void;
    const gate = new Promise<void>((r) => (release = r));
    mockedApi.status
      // The error's own refresh: answers first, but is already stale.
      .mockImplementationOnce(async () => conflictStatus("rebase"))
      // The operation's refresh, started right after: answers later.
      .mockImplementationOnce(async () => {
        await gate;
        return conflictStatus("rebase");
      });

    showError({ code: "merge_conflict", message: "conflicts" });
    const later = refreshStatus();
    await new Promise((r) => setTimeout(r, 0));
    release();
    await later;

    await vi.waitFor(() => expect(ui.view).toBe("conflicts"));
  });

  it("stays put when the error left no conflicts behind", async () => {
    showError({ code: "sidecar_failed", message: "git command failed", detail: "conflict?" });

    await new Promise((r) => setTimeout(r, 0));
    expect(ui.view).toBe("repo");
  });
});

describe("stash conflicts in the workshop", () => {
  it("a conflicted pop opens the workshop in its stash mode", async () => {
    mockedApi.stashPop.mockResolvedValueOnce(conflicted);

    await stashPop(0);

    expect(ui.view).toBe("conflicts");
    expect(ui.stashConflict).toEqual({ id: stash.id, message: stash.message, pop: true });
    // The note that the stash is kept stays visible.
    expect(ui.error).toBe(t("state.stashConflictsKeptOne"));
  });

  it("an apply is marked as such — Done will not drop its stash", async () => {
    mockedApi.stashApply.mockResolvedValueOnce(conflicted);

    await stashApply(0);

    expect(ui.stashConflict?.pop).toBe(false);
  });

  it("Done drops a popped stash — found by its id, not its index", async () => {
    ui.stashConflict = { id: stash.id, message: stash.message, pop: true };
    mockedApi.stashList.mockImplementation(async () => [
      { index: 0, message: "On main: newer", id: "n".repeat(40) },
      { ...stash, index: 1 },
    ]);

    await finishConflicts();

    expect(mockedApi.stashDrop).toHaveBeenCalledWith("/repo", 1);
    expect(ui.stashConflict).toBeNull();
    expect(ui.view).toBe("repo");
    expect(ui.info).toBe(t("conflictws.doneStashDropped"));
  });

  it("Done keeps the stash of an apply", async () => {
    ui.stashConflict = { id: stash.id, message: stash.message, pop: false };

    await finishConflicts();

    expect(mockedApi.stashDrop).not.toHaveBeenCalled();
    expect(ui.info).toBe(t("conflictws.done"));
  });

  it("Give up resets the conflicted files and keeps the stash", async () => {
    ui.status = conflictStatus("clean");
    ui.stashConflict = { id: stash.id, message: stash.message, pop: true };

    await abandonConflicts();

    expect(mockedApi.abandonConflicts).toHaveBeenCalledWith("/repo", ["a.txt"], 500 * 1024 * 1024);
    expect(mockedApi.stashDrop).not.toHaveBeenCalled();
    expect(ui.stashConflict).toBeNull();
    expect(ui.info).toBe(t("conflictws.abandonedStashKept"));
  });

  it("Give up stops on a full backup store", async () => {
    ui.status = conflictStatus("clean");
    mockedApi.abandonConflicts.mockRejectedValueOnce({
      code: "discard_store_full",
      message: "10 495 500",
    });

    await abandonConflicts();

    expect(ui.errorMeta?.code).toBe("discard_store_full");
  });
});
