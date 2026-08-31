// closeRepo has to clear the repo state completely: stashes, tags,
// remotes, undoStatus, blame, imageDiff, searchQuery/searchResults, messageLog,
// modal and historyComplete used to stay behind.
// In practice that went unnoticed because openRepo re-sets most of it on the
// next open — but the invariant "no repo → no repo state" did not hold.
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/plugin-dialog", () => ({ confirm: vi.fn(async () => false) }));

vi.mock("./api", () => ({
  api: {
    openRepository: vi.fn(async (path: string) => ({
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
  },
}));

import { closeRepo, ui } from "./state.svelte";

beforeEach(() => {
  vi.clearAllMocks();
  ui.repo = null;
  ui.view = "repo";
  ui.error = null;
  ui.info = null;
});

/** Fills all repo-bound slices with sample data. */
function fillRepoState() {
  ui.repo = {
    path: "/x",
    name: "x",
    currentBranch: "main",
    headDetached: false,
    isEmpty: false,
    historyPrepared: true,
  };
  ui.stashes = [{ index: 0, message: "wip", id: "abc123" }];
  ui.tags = [{ name: "v1", targetId: "abc", message: null, isAnnotated: false }];
  ui.remotes = [{ name: "origin", url: "https://example.invalid/x.git" }];
  ui.undoStatus = {
    undo: { op: "commit", detail: null, timestamp: 0 },
    redo: null,
    undoCount: 1,
    redoCount: 0,
  };
  ui.blame = { file: "a.txt", lines: [] };
  ui.imageDiff = { oldDataUrl: null, newDataUrl: null };
  ui.searchQuery = "fix";
  ui.searchResults = [];
  ui.messageLog = ["feat: old subject"];
  ui.modal = { kind: "tags" };
  ui.historyComplete = true;
  ui.history = [];
}

describe("closeRepo()", () => {
  it("clears all repo-bound slices (invariant: no repo -> no repo state)", () => {
    fillRepoState();
    closeRepo();

    expect(ui.repo).toBeNull();
    expect(ui.stashes).toEqual([]);
    expect(ui.tags).toEqual([]);
    expect(ui.remotes).toEqual([]);
    expect(ui.undoStatus).toBeNull();
    expect(ui.blame).toBeNull();
    expect(ui.imageDiff).toBeNull();
    expect(ui.searchQuery).toBe("");
    expect(ui.searchResults).toBeNull();
    expect(ui.messageLog).toEqual([]);
    expect(ui.modal).toBeNull();
    expect(ui.historyComplete).toBe(false);
  });

  // Counter-check: an over-aggressive reset would be a real bug. Global settings
  // and accounts do NOT belong to the repo.
  it("leaves global settings and accounts untouched", () => {
    fillRepoState();
    ui.recents = [{ path: "/a", lastOpened: null, pinned: false }];
    ui.autoFetch = true;
    ui.uiScale = 1.25;
    ui.reduceMotion = true;

    closeRepo();

    expect(ui.recents).toHaveLength(1);
    expect(ui.autoFetch).toBe(true);
    expect(ui.uiScale).toBe(1.25);
    expect(ui.reduceMotion).toBe(true);
  });
});
