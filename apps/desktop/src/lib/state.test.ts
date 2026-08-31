// Lifecycle behaviour of state.svelte.ts with a mocked IPC layer:
// deleting the CURRENTLY open repo has to release the watcher and close the
// repo state before trashing. Another repo stays untouched.
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { RepoInfo } from "./api";

vi.mock("@tauri-apps/plugin-dialog", () => ({ confirm: vi.fn(async () => false) }));

vi.mock("./api", () => ({
  api: {
    deleteRepo: vi.fn(async () => {}),
    unwatchRepository: vi.fn(async () => {}),
  },
}));

import { api } from "./api";
import { closeRepo, deleteRepoFromDisk, ui } from "./state.svelte";

const mockedApi = vi.mocked(api);

const recentOf = (path: string) => ({ path, lastOpened: null, pinned: false });

const repoOf = (path: string): RepoInfo => ({
  path,
  name: path.split("/").pop() ?? path,
  currentBranch: "main",
  headDetached: false,
  isEmpty: false,
  historyPrepared: true,
});

beforeEach(() => {
  vi.clearAllMocks();
  ui.repo = null;
  ui.recents = [];
  ui.modal = null;
  ui.error = null;
  ui.info = null;
  ui.view = "repo";
});

describe("closeRepo", () => {
  it("resets ALL repo-bound slices (invariant: no repo -> no repo state, F21)", () => {
    ui.repo = repoOf("/x");
    ui.tab = "history";
    ui.stashes = [{ index: 0, id: "s1", message: "wip" } as never];
    ui.tags = [{ name: "v1" } as never];
    ui.remotes = [{ name: "origin", url: "u" } as never];
    ui.searchQuery = "fix";
    ui.searchResults = [];
    ui.messageLog = ["feat: x"];
    ui.undoStatus = { undo: null, redo: null } as never;
    ui.blame = { file: "a.ts", lines: [] };
    ui.imageDiff = { oldDataUrl: null, newDataUrl: null } as never;
    ui.modal = { kind: "stash" };
    ui.unpushed = [{ id: "c1" } as never];
    ui.workshopEdits = { c1: {} as never };
    ui.workshopOrder = ["c1"];
    ui.workshopError = true;

    closeRepo();

    expect(mockedApi.unwatchRepository).toHaveBeenCalled();
    expect(ui.repo).toBeNull();
    expect(ui.tab).toBe("changes");
    expect(ui.stashes).toEqual([]);
    expect(ui.tags).toEqual([]);
    expect(ui.remotes).toEqual([]);
    expect(ui.searchQuery).toBe("");
    expect(ui.searchResults).toBeNull();
    expect(ui.messageLog).toEqual([]);
    expect(ui.undoStatus).toBeNull();
    expect(ui.blame).toBeNull();
    expect(ui.imageDiff).toBeNull();
    expect(ui.modal).toBeNull();
    expect(ui.unpushed).toEqual([]);
    expect(ui.workshopEdits).toEqual({});
    expect(ui.workshopOrder).toEqual([]);
    expect(ui.workshopError).toBe(false);
  });
});

describe("deleteRepoFromDisk", () => {
  it("closes the OPEN repo (watcher released, state nulled) before trashing", async () => {
    ui.repo = repoOf("/x");
    ui.recents = [recentOf("/x"), recentOf("/y")];

    await deleteRepoFromDisk("/x");

    expect(mockedApi.unwatchRepository).toHaveBeenCalled();
    expect(mockedApi.deleteRepo).toHaveBeenCalledWith("/x");
    // closeRepo ran -> ui.repo nulled, the list cleaned up.
    expect(ui.repo).toBeNull();
    expect(ui.recents).toEqual([recentOf("/y")]);
  });

  it("leaves ui.repo untouched when a DIFFERENT repo is deleted", async () => {
    ui.repo = repoOf("/open");
    ui.recents = [recentOf("/open"), recentOf("/other")];

    await deleteRepoFromDisk("/other");

    expect(ui.repo?.path).toBe("/open");
    expect(mockedApi.unwatchRepository).not.toHaveBeenCalled();
    expect(mockedApi.deleteRepo).toHaveBeenCalledWith("/other");
    expect(ui.recents).toEqual([recentOf("/open")]);
  });
});
