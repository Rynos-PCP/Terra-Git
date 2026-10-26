// Apply/Pop from the stash list: a conflicted apply is not a
// success. libgit2 reports one anyway, so the engine now returns the conflicted
// files and keeps the stash — the UI has to say that instead of "applied".
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { RepoInfo, RepoStatus, StashApplyOutcome } from "./api";

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

const clean: StashApplyOutcome = { conflicted: [], dropped: true };
const conflicted: StashApplyOutcome = { conflicted: ["a.txt", "b.txt"], dropped: false };

vi.mock("./api", () => ({
  api: {
    stashApply: vi.fn(async () => ({ conflicted: [], dropped: false })),
    stashPop: vi.fn(async () => ({ conflicted: [], dropped: true })),
    stashList: vi.fn(async () => []),
    status: vi.fn(async () => cleanStatus),
  },
}));

import { api } from "./api";
import { t, tn } from "./i18n.svelte";
import { stashApply, stashPop, ui } from "./state.svelte";

const mockedApi = vi.mocked(api);

const top = { index: 0, id: "a".repeat(40), message: "WIP on main" };

beforeEach(() => {
  vi.clearAllMocks();
  ui.repo = repo;
  ui.status = cleanStatus;
  ui.working = 0;
  ui.error = null;
  ui.info = null;
  ui.errorAction = null;
  ui.stashes = [top];
  mockedApi.stashList.mockResolvedValue([top]);
});

describe("stash list: apply/pop with conflicts", () => {
  it("reports a clean pop as success", async () => {
    mockedApi.stashPop.mockResolvedValueOnce(clean);

    await stashPop(0);

    expect(ui.info).toBe(t("state.stashPopped"));
    expect(ui.error).toBeNull();
  });

  it("does not claim success when the pop left conflicts — the stash is kept", async () => {
    mockedApi.stashPop.mockResolvedValueOnce(conflicted);

    await stashPop(0);

    expect(ui.info).toBeNull();
    expect(ui.error).toBe(tn("state.stashConflictsKept", 2));
    expect(ui.errorAction).toEqual({ kind: "stashes" });
    expect(ui.working).toBe(0);
  });

  it("reports conflicts after apply the same way", async () => {
    mockedApi.stashApply.mockResolvedValueOnce({ ...conflicted, dropped: false });

    await stashApply(0);

    expect(ui.info).toBeNull();
    expect(ui.error).toBe(tn("state.stashConflictsKept", 2));
  });
});

describe("stash list: a list grown meanwhile", () => {
  it("pops nothing when another stash took the index in the meantime", async () => {
    // A stash made in a terminal: index 0 is a different one now.
    mockedApi.stashList.mockResolvedValueOnce([
      { index: 0, id: "b".repeat(40), message: "made elsewhere" },
      { ...top, index: 1 },
    ]);

    await stashPop(0);

    expect(mockedApi.stashPop).not.toHaveBeenCalled();
    expect(ui.info).toBe(t("state.stashListChanged"));
  });
});
