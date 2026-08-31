// Wiring of the workshop offer in state.svelte.ts: showError has to set the
// offer for conflict errors (and refresh the status silently, because the pull
// error path does not refresh itself), showInfo/clearToast have to clear it
// again.
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { RepoInfo, RepoStatus } from "./api";

vi.mock("@tauri-apps/plugin-dialog", () => ({ confirm: vi.fn(async () => false) }));

vi.mock("./api", () => ({
  api: {
    status: vi.fn(async (): Promise<RepoStatus> => statusWithConflict),
    undoStatus: vi.fn(async () => null),
  },
}));

import { api } from "./api";
import { resetRepoSlices } from "./repoReset";
import { clearToast, openConflicts, showError, showInfo, ui } from "./state.svelte";

const mockedApi = vi.mocked(api);

const statusWithConflict: RepoStatus = {
  staged: [],
  unstaged: [{ path: "a.txt", kind: "conflicted", staged: false } as never],
  branch: "main",
  upstream: null,
  ahead: 0,
  behind: 0,
  opState: "merge",
};

const repo: RepoInfo = {
  path: "/repo",
  name: "repo",
  currentBranch: "main",
  headDetached: false,
  isEmpty: false,
  historyPrepared: true,
};

beforeEach(() => {
  vi.clearAllMocks();
  ui.repo = null;
  ui.status = null;
  ui.error = null;
  ui.info = null;
  ui.errorAction = null;
});

describe("showError() + workshop offer", () => {
  it("sets the offer on a conflict error and refreshes the status silently", async () => {
    ui.repo = repo;
    showError({ code: "merge_conflict", message: "The pull created conflicts — …" });

    expect(ui.error).toMatch(/konflikt|conflict/i);
    expect(ui.errorAction).toEqual({ kind: "conflicts" });
    await vi.waitFor(() => expect(mockedApi.status).toHaveBeenCalledWith("/repo"));
  });

  it("sets NO offer for non-conflict errors — even if one stood before", () => {
    ui.errorAction = { kind: "conflicts" };
    showError({ code: "network", message: "The remote is unreachable." });

    expect(ui.errorAction).toBeNull();
    expect(mockedApi.status).not.toHaveBeenCalled();
  });

  it("leaves the status alone when no repo is open", () => {
    showError({ code: "git_error", message: "1 conflict prevents checkout" });

    expect(ui.errorAction).toEqual({ kind: "conflicts" });
    expect(mockedApi.status).not.toHaveBeenCalled();
  });

  it("a repo switch/close clears the offer — the message stays", () => {
    ui.error = "The pull created conflicts — …";
    ui.errorAction = { kind: "conflicts" };

    // openRepo and closeRepo both go through resetRepoSlices.
    resetRepoSlices(ui);

    expect(ui.errorAction).toBeNull();
    expect(ui.error).toBe("The pull created conflicts — …");
  });

  it("showInfo and clearToast clear the offer", () => {
    ui.error = "x";
    ui.errorAction = { kind: "conflicts" };
    showInfo("done");
    expect(ui.errorAction).toBeNull();

    ui.error = "x";
    ui.errorAction = { kind: "conflicts" };
    clearToast();
    expect(ui.errorAction).toBeNull();
    expect(ui.error).toBeNull();
  });
});

// ONE entry point for the banner, the tools menu, the palette and the toast.
describe("openConflicts()", () => {
  it("switches to the workshop and closes the toast that was followed", () => {
    ui.view = "repo";
    ui.error = "The pull created conflicts — …";
    ui.errorAction = { kind: "conflicts" };

    openConflicts();

    expect(ui.view).toBe("conflicts");
    expect(ui.error).toBeNull();
    expect(ui.errorAction).toBeNull();
  });

  // Finding B3 (adversarial counter-check 2026-08-17): openConflicts() nulled
  // ui.modal in its first version. That closed the modal but discarded the manual
  // work in the conflict editor in the process (the resolutions live only in the
  // component until saved) — a NEW data-loss path that did not exist before.
  it("leaves an open modal standing — unsaved work can live there", () => {
    ui.view = "repo";
    ui.modal = { kind: "conflictEditor", file: "a.txt" };
    ui.error = "The merge created conflicts";
    ui.errorAction = { kind: "conflicts" };

    openConflicts();

    expect(ui.view).toBe("conflicts");
    expect(ui.modal).toEqual({ kind: "conflictEditor", file: "a.txt" });
  });
});
