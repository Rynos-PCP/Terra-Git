// F21/F22: late answers of old repos must not overwrite the state after a repo
// switch/close — neither diff streams (fileDiffSeq/commitDiffSeq) nor the list
// slices (branches/stashes/…) nor an already cleared search field.
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { BranchInfo, CommitInfo, FileDiff, RepoInfo, StashInfo } from "./api";

vi.mock("@tauri-apps/plugin-dialog", () => ({ confirm: vi.fn(async () => false) }));

vi.mock("./api", () => ({
  api: {
    openRepository: vi.fn(async (path: string): Promise<RepoInfo> => ({
      path,
      name: "repo",
      currentBranch: "main",
      headDetached: false,
      isEmpty: false,
      historyPrepared: true,
    })),
    recentRepos: vi.fn(async () => []),
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
    stashList: vi.fn(async () => []),
    tags: vi.fn(async () => []),
    remotes: vi.fn(async () => []),
    undoStatus: vi.fn(async () => ({ undo: null, redo: null, undoCount: 0, redoCount: 0 })),
    watchRepository: vi.fn(async () => {}),
    unwatchRepository: vi.fn(async () => {}),
    fileDiff: vi.fn(),
    commitDiffStream: vi.fn(),
    searchLog: vi.fn(),
  },
}));

import { api } from "./api";
import {
  closeRepo,
  openRepo,
  refreshBranches,
  refreshStashes,
  runSearch,
  selectCommit,
  selectFile,
  ui,
} from "./state.svelte";

const mockedApi = vi.mocked(api);

const commitOf = (id: string): CommitInfo => ({
  id,
  shortId: id.slice(0, 8),
  summary: "s",
  authorName: "a",
  authorEmail: "a@b",
  time: 0,
  parentIds: [],
});

const diffOf = (path: string): FileDiff => ({
  path,
  oldPath: null,
  isBinary: false,
  // Non-empty so selectFile does not run into the explainUnchanged diagnosis.
  hunks: [{ header: "@@ -1 +1 @@", lines: [] }],
  truncated: false,
});

beforeEach(() => {
  vi.clearAllMocks();
  ui.repo = null;
  ui.busy = null;
  ui.error = null;
  ui.info = null;
  ui.searchQuery = "";
  ui.searchResults = null;
});

describe("diff sequences on a repo switch", () => {
  it("late commit diff files of the old repo do not appear in the new one", async () => {
    let emit!: (fd: FileDiff) => void;
    let resolveTotal!: (n: number) => void;
    mockedApi.commitDiffStream.mockImplementationOnce(
      (_p, _id, _max, onFile) =>
        new Promise<number>((r) => {
          emit = onFile;
          resolveTotal = r;
        }),
    );
    await openRepo("/a");
    const pending = selectCommit(commitOf("c1"));

    await openRepo("/b");
    // Only now does the stream of the old repo arrive.
    emit(diffOf("stale.txt"));
    resolveTotal(1);
    await pending;

    expect(ui.commitDiff).toEqual([]);
    expect(ui.commitDiffTotal).toBeNull();
  });

  it("a late file diff after closeRepo is discarded", async () => {
    let resolveDiff!: (d: FileDiff) => void;
    mockedApi.fileDiff.mockImplementationOnce(
      () => new Promise<FileDiff>((r) => (resolveDiff = r)),
    );
    await openRepo("/a");
    const pending = selectFile("a.txt", false);

    closeRepo();
    resolveDiff(diffOf("a.txt"));
    await pending;

    expect(ui.fileDiff).toBeNull();
  });
});

describe("list slices on a repo switch", () => {
  it("a late branch list after closeRepo is discarded", async () => {
    await openRepo("/a");
    let resolveBranches!: (b: BranchInfo[]) => void;
    mockedApi.branches.mockImplementationOnce(
      () => new Promise<BranchInfo[]>((r) => (resolveBranches = r)),
    );
    const pending = refreshBranches();

    closeRepo();
    resolveBranches([
      {
        name: "main",
        isHead: true,
        isRemote: false,
        upstream: null,
        shortName: null,
        targetId: null,
        upstreamGone: false,
      },
    ]);
    await pending;

    expect(ui.branches).toEqual([]);
  });

  it("a late stash list of the old repo does not survive the switch", async () => {
    await openRepo("/a");
    let resolveStashes!: (s: StashInfo[]) => void;
    mockedApi.stashList.mockImplementationOnce(
      () => new Promise<StashInfo[]>((r) => (resolveStashes = r)),
    );
    const pending = refreshStashes();

    await openRepo("/b");
    resolveStashes([{ index: 0, message: "stale", id: "s1" }]);
    await pending;

    expect(ui.stashes).toEqual([]);
  });
});

describe("runSearch race", () => {
  it("a cleared search field is not refilled by a late answer", async () => {
    await openRepo("/a");
    ui.searchQuery = "foo";
    let resolveSearch!: (c: CommitInfo[]) => void;
    mockedApi.searchLog.mockImplementationOnce(
      () => new Promise<CommitInfo[]>((r) => (resolveSearch = r)),
    );
    const pending = runSearch();

    // The user clears the field — the empty search resets the result.
    ui.searchQuery = "";
    await runSearch();
    resolveSearch([commitOf("c1")]);
    await pending;

    expect(ui.searchResults).toBeNull();
  });
});
