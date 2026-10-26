// The very first commit: reverting it removes every file it
// added — so it asks first; other commits revert without a question.
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
    revertCommit: vi.fn(async () => ""),
    status: vi.fn(async () => cleanStatus),
    branches: vi.fn(async () => []),
    logAll: vi.fn(async () => []),
    openRepository: vi.fn(async () => repo),
    undoStatus: vi.fn(async () => null),
  },
}));

import { confirm } from "@tauri-apps/plugin-dialog";
import { api } from "./api";
import { revertCommit, ui } from "./state.svelte";

const mockedConfirm = vi.mocked(confirm);
const ID = "a".repeat(40);

beforeEach(() => {
  vi.clearAllMocks();
  ui.repo = repo;
  ui.status = cleanStatus;
  ui.working = 0;
  mockedConfirm.mockImplementation(async () => false);
});

describe("revertCommit()", () => {
  it("asks before reverting the very first commit — and stops on no", async () => {
    await revertCommit(ID, true);

    expect(mockedConfirm).toHaveBeenCalledTimes(1);
    expect(api.revertCommit).not.toHaveBeenCalled();
  });

  it("reverts the first commit after a yes", async () => {
    mockedConfirm.mockResolvedValueOnce(true);

    await revertCommit(ID, true);

    expect(api.revertCommit).toHaveBeenCalledWith("/repo", ID);
  });

  it("does not ask for any other commit", async () => {
    await revertCommit(ID);

    expect(mockedConfirm).not.toHaveBeenCalled();
    expect(api.revertCommit).toHaveBeenCalledWith("/repo", ID);
  });
});
