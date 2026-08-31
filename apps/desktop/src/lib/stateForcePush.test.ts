// F3/F19: a force push has to (1) hit the same remote as a normal push (the
// upstream remote, mirroring the engine logic pick_push_remote) instead of
// blindly remotes[0], and (2) be confirmed BEFORE the run — the
// non_fast_forward retry has its own confirmation and must not ask twice.
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { RemoteInfo, RepoInfo } from "./api";

vi.mock("@tauri-apps/plugin-dialog", () => ({ confirm: vi.fn(async () => false) }));

vi.mock("./api", () => ({
  api: {
    push: vi.fn(async () => ""),
    pushRemote: vi.fn(async () => ""),
    status: vi.fn(async () => ({
      staged: [],
      unstaged: [],
      branch: "main",
      upstream: null,
      ahead: 0,
      behind: 0,
      opState: "clean",
    })),
    branches: vi.fn(async () => []),
    log: vi.fn(async () => []),
    logAll: vi.fn(async () => []),
    undoStatus: vi.fn(async () => ({ undo: null, redo: null, undoCount: 0, redoCount: 0 })),
  },
}));

import { confirm } from "@tauri-apps/plugin-dialog";
import { api } from "./api";
import { gitPush, gitPushForce, ui } from "./state.svelte";

const mockedApi = vi.mocked(api);
const mockedConfirm = vi.mocked(confirm);

const repoOf = (path: string): RepoInfo => ({
  path,
  name: "r",
  currentBranch: "main",
  headDetached: false,
  isEmpty: false,
  historyPrepared: true,
});

// The scenario from the finding: the branch tracks origin, but "backup" is the
// (alphabetically) first remote in the list.
const remotes: RemoteInfo[] = [
  { name: "backup", url: "git@ci.intern:o/r.git" },
  { name: "origin", url: "git@github.com:o/r.git" },
];

beforeEach(() => {
  vi.clearAllMocks();
  ui.repo = repoOf("/repo");
  ui.status = { upstream: "origin/main" } as never;
  ui.remotes = remotes;
  ui.busy = null;
  ui.cloning = null;
  ui.error = null;
  ui.info = null;
});

describe("gitPushForce (explicit force push)", () => {
  it("asks first and does NOTHING on cancel", async () => {
    mockedConfirm.mockResolvedValueOnce(false);

    await gitPushForce();

    expect(mockedConfirm).toHaveBeenCalledTimes(1);
    expect(mockedApi.pushRemote).not.toHaveBeenCalled();
  });

  it("targets the upstream remote after confirmation (not remotes[0])", async () => {
    mockedConfirm.mockResolvedValueOnce(true);

    await gitPushForce();

    // The dialog names the target remote …
    expect(mockedConfirm).toHaveBeenCalledWith(
      expect.stringContaining("origin"),
      expect.anything(),
    );
    // … and the push hits EXACTLY that remote, not "backup".
    expect(mockedApi.pushRemote).toHaveBeenCalledTimes(1);
    expect(mockedApi.pushRemote).toHaveBeenCalledWith("/repo", "origin", true, expect.anything());
  });

  it("falls back to the only remote without an upstream (mirroring the engine)", async () => {
    ui.status = { upstream: null } as never;
    ui.remotes = [{ name: "backup", url: "git@ci.intern:o/r.git" }];
    mockedConfirm.mockResolvedValueOnce(true);

    await gitPushForce();

    expect(mockedApi.pushRemote).toHaveBeenCalledWith("/repo", "backup", true, expect.anything());
  });
});

describe("gitPush non_fast_forward retry", () => {
  it("the force retry hits the upstream remote and asks only ONCE", async () => {
    mockedApi.push.mockRejectedValueOnce({
      code: "non_fast_forward",
      message: "remote is ahead",
    });
    mockedConfirm.mockResolvedValueOnce(true);

    await gitPush();

    // Exactly one question (the non_fast_forward path's, no duplicate).
    expect(mockedConfirm).toHaveBeenCalledTimes(1);
    expect(mockedApi.pushRemote).toHaveBeenCalledTimes(1);
    expect(mockedApi.pushRemote).toHaveBeenCalledWith("/repo", "origin", true, expect.anything());
  });
});
