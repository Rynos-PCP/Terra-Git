// Git LFS state and download.
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { LfsState, RepoInfo, RepoStatus } from "./api";

vi.mock("@tauri-apps/plugin-dialog", () => ({ confirm: vi.fn(async () => false) }));

const cleanStatus: RepoStatus = {
  staged: [],
  unstaged: [],
  branch: "main",
  upstream: "origin/main",
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

const missing: LfsState = { used: true, installed: true, missing: 3 };

vi.mock("./api", () => ({
  api: {
    lfsState: vi.fn(async () => missing),
    lfsPull: vi.fn(async () => ""),
    lfsTrack: vi.fn(async () => undefined),
    status: vi.fn(async () => cleanStatus),
    branches: vi.fn(async () => []),
    logAll: vi.fn(async () => []),
    undoStatus: vi.fn(async () => null),
  },
}));

import { api } from "./api";
import { lfsPull, refreshBranches, refreshLfs, trackWithLfs, ui } from "./state.svelte";
import { resetRepoSlices } from "./repoReset";

const mockedApi = vi.mocked(api);

beforeEach(() => {
  vi.clearAllMocks();
  ui.repo = repo;
  ui.status = cleanStatus;
  ui.lfs = null;
  ui.busy = null;
  ui.working = 0;
});

describe("refreshLfs()", () => {
  it("reads the LFS state of the open repo", async () => {
    await refreshLfs();

    expect(mockedApi.lfsState).toHaveBeenCalledWith("/repo");
    expect(ui.lfs).toEqual(missing);
  });

  it("runs along with every branch refresh", async () => {
    await refreshBranches();
    await vi.waitFor(() => expect(ui.lfs).toEqual(missing));
  });

  it("stays silent when the state cannot be read", async () => {
    mockedApi.lfsState.mockRejectedValueOnce({ code: "git_error", message: "x" });

    await refreshLfs();

    expect(ui.lfs).toBeNull();
    expect(ui.error).toBeNull();
  });

  it("is cleared by a repo switch", async () => {
    await refreshLfs();
    resetRepoSlices(ui);
    expect(ui.lfs).toBeNull();
  });
});

describe("lfsPull()", () => {
  it("downloads through git lfs pull and reads the state again", async () => {
    await lfsPull();

    expect(mockedApi.lfsPull).toHaveBeenCalledWith("/repo", expect.any(Function));
    await vi.waitFor(() => expect(mockedApi.lfsState).toHaveBeenCalled());
    expect(ui.busy).toBeNull();
  });
});

describe("trackWithLfs()", () => {
  it("tracks the pattern and says so", async () => {
    await trackWithLfs("*.psd", "art/cover.psd");

    expect(mockedApi.lfsTrack).toHaveBeenCalledWith("/repo", "*.psd", "art/cover.psd");
    expect(ui.info).toContain("*.psd");
  });

  it("shows the error when git-lfs is missing", async () => {
    mockedApi.lfsTrack.mockRejectedValueOnce({ code: "lfs_missing", message: "no lfs" });

    await trackWithLfs("*.psd", "art/cover.psd");

    expect(ui.errorMeta?.code).toBe("lfs_missing");
  });
});
