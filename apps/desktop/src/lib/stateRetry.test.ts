// "Try again" and the other ways out of the error toast.
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { RepoInfo, RepoStatus } from "./api";

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

vi.mock("./api", () => ({
  api: {
    fetch: vi.fn(async () => ""),
    log: vi.fn(async () => [{ id: "b".repeat(40) }]),
    status: vi.fn(async () => cleanStatus),
    branches: vi.fn(async () => []),
    logAll: vi.fn(async () => []),
    undoStatus: vi.fn(async () => null),
    lfsState: vi.fn(async () => ({ used: false, installed: false, missing: 0 })),
    openExternal: vi.fn(async () => undefined),
    removeIndexLock: vi.fn(async () => true),
    trustDirectory: vi.fn(async () => undefined),
    unsetUpstream: vi.fn(async () => undefined),
    configSet: vi.fn(async () => undefined),
    mergeBranch: vi.fn(async () => ""),
    commit: vi.fn(async () => "c".repeat(40)),
    openRepository: vi.fn(async () => repo),
    stashList: vi.fn(async () => []),
    tags: vi.fn(async () => []),
    remotes: vi.fn(async () => []),
    recentRepos: vi.fn(async () => []),
    watchRepository: vi.fn(async () => undefined),
    unwatchRepository: vi.fn(async () => undefined),
  },
}));

import { confirm } from "@tauri-apps/plugin-dialog";
import { api } from "./api";
import {
  createCommit,
  gitFetch,
  mergeBranch,
  openRepo,
  runErrorAction,
  showError,
  ui,
} from "./state.svelte";

const mockedApi = vi.mocked(api);

beforeEach(() => {
  vi.clearAllMocks();
  ui.repo = repo;
  ui.status = cleanStatus;
  ui.busy = null;
  ui.modal = null;
  ui.view = "repo";
});

describe("Try again", () => {
  it("repeats a failed remote operation", async () => {
    mockedApi.fetch.mockRejectedValueOnce({ code: "network", message: "offline" });

    await gitFetch();
    expect(ui.errorMeta?.code).toBe("network");
    expect(ui.canRetry).toBe(true);

    await runErrorAction("retry");

    expect(mockedApi.fetch).toHaveBeenCalledTimes(2);
    expect(ui.error).toBeNull();
  });

  it("is not offered for an error without an operation to repeat", () => {
    showError({ code: "network", message: "offline" });
    expect(ui.canRetry).toBe(false);
  });
});

describe("other ways out", () => {
  it("leads a missing identity to the settings", async () => {
    await runErrorAction("openIdentity");
    expect(ui.view).toBe("settings");
  });

  it("offers a branch at HEAD for a push from a detached HEAD", async () => {
    await runErrorAction("branchHere");
    expect(ui.modal).toEqual({ kind: "branchFrom", commitId: "b".repeat(40) });
  });
});

describe("ways out with the backend", () => {
  it("removes a leftover lock only after a confirmation, then retries", async () => {
    mockedApi.fetch.mockRejectedValueOnce({ code: "index_locked", message: "locked" });
    await gitFetch();

    await runErrorAction("removeLock");
    expect(mockedApi.removeIndexLock).not.toHaveBeenCalled();

    mockedApi.fetch.mockRejectedValueOnce({ code: "index_locked", message: "locked" });
    await gitFetch();
    vi.mocked(confirm).mockResolvedValueOnce(true);
    await runErrorAction("removeLock");
    expect(mockedApi.removeIndexLock).toHaveBeenCalledWith("/repo", null);
    expect(mockedApi.fetch).toHaveBeenCalledTimes(3);
  });

  it("trusts the folder of a repo that could not be opened, and opens it", async () => {
    ui.repo = null;
    mockedApi.openRepository.mockRejectedValueOnce({
      code: "unsafe_repository",
      message: "dubious ownership",
    });
    await openRepo("D:/shared/r");
    expect(ui.errorMeta?.code).toBe("unsafe_repository");

    vi.mocked(confirm).mockResolvedValueOnce(true);
    await runErrorAction("trustFolder");

    expect(mockedApi.trustDirectory).toHaveBeenCalledWith("D:/shared/r");
    expect(mockedApi.openRepository).toHaveBeenLastCalledWith("D:/shared/r");
  });

  it("merges unrelated histories only on request", async () => {
    mockedApi.mergeBranch.mockRejectedValueOnce({
      code: "unrelated_histories",
      message: "refusing",
    });
    await mergeBranch("other");
    expect(ui.canMergeUnrelated).toBe(true);

    vi.mocked(confirm).mockResolvedValueOnce(true);
    await runErrorAction("mergeUnrelated");

    expect(mockedApi.mergeBranch).toHaveBeenLastCalledWith("/repo", "other", true);
  });

  it("does not offer the unrelated merge for other merge errors", async () => {
    mockedApi.mergeBranch.mockRejectedValueOnce({ code: "git_error", message: "x" });
    await mergeBranch("other");
    expect(ui.canMergeUnrelated).toBe(false);
    expect(ui.canRetry).toBe(true);
  });

  it("removes a gone upstream", async () => {
    await runErrorAction("unsetUpstream");
    expect(mockedApi.unsetUpstream).toHaveBeenCalledWith("/repo");
  });

  it("enables long paths for the repo", async () => {
    await runErrorAction("enableLongPaths");
    expect(mockedApi.configSet).toHaveBeenCalledWith("/repo", "core.longpaths", "true", false);
  });
});

describe("commits", () => {
  it("skips the signature once after signing failed, and counts the commit", async () => {
    mockedApi.commit.mockRejectedValueOnce({ code: "signing_failed", message: "gpg" });
    const before = ui.commitCount;

    expect(await createCommit("feat: x", false)).toBe(false);
    expect(ui.canCommitUnsigned).toBe(true);

    await runErrorAction("commitUnsigned");

    expect(mockedApi.commit).toHaveBeenLastCalledWith("/repo", "feat: x", false, true);
    expect(ui.commitCount).toBe(before + 1);
  });

  it("repeats a commit a hook stopped", async () => {
    mockedApi.commit.mockRejectedValueOnce({ code: "hook_failed", message: "lint" });

    await createCommit("feat: y", false);
    expect(ui.canRetry).toBe(true);
    expect(ui.canCommitUnsigned).toBe(false);

    await runErrorAction("retry");
    expect(mockedApi.commit).toHaveBeenLastCalledWith("/repo", "feat: y", false);
  });
});

describe("another repository opened (audit 2026-10-25)", () => {
  const other: RepoInfo = { ...repo, path: "/other", name: "other" };

  it("drops the ways out of the old toast — they would act on the new repository", async () => {
    mockedApi.commit.mockRejectedValueOnce({ code: "signing_failed", message: "gpg" });
    await createCommit("message of /repo", false);
    expect(ui.canCommitUnsigned).toBe(true);
    expect(ui.canRetry).toBe(true);

    mockedApi.openRepository.mockResolvedValueOnce(other);
    await openRepo("/other");

    expect(ui.canCommitUnsigned).toBe(false);
    expect(ui.canRetry).toBe(false);
    await runErrorAction("commitUnsigned");
    await runErrorAction("retry");
    expect(mockedApi.commit).toHaveBeenCalledTimes(1);
  });

  it("is not opened while an operation runs here", async () => {
    ui.busy = "Fetch";
    await openRepo("/other");
    expect(mockedApi.openRepository).not.toHaveBeenCalled();
    expect(ui.repo?.path).toBe("/repo");
  });
});
