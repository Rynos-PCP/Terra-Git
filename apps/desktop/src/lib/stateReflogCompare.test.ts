// Reset to any commit and the compare view.
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
    resetTo: vi.fn(async () => ""),
    status: vi.fn(async () => cleanStatus),
    branches: vi.fn(async () => []),
    logAll: vi.fn(async () => []),
    undoStatus: vi.fn(async () => null),
  },
}));

import { api } from "./api";
import { t } from "./i18n.svelte";
import { clearToast, openCompare, resetBranchTo, ui } from "./state.svelte";
import { resetRepoSlices } from "./repoReset";

const mockedApi = vi.mocked(api);
const ID = "a".repeat(40);

beforeEach(() => {
  vi.clearAllMocks();
  ui.repo = repo;
  ui.status = cleanStatus;
  ui.view = "repo";
  ui.compare = null;
  ui.working = 0;
  ui.discardLimitMb = 500;
  clearToast();
});

describe("resetBranchTo()", () => {
  it("resets with the chosen mode and the backup limit", async () => {
    ui.modal = { kind: "resetTo", commitId: ID, subject: "x" };

    await resetBranchTo(ID, "hard");

    expect(mockedApi.resetTo).toHaveBeenCalledWith("/repo", ID, "hard", 500 * 1024 * 1024);
    expect(ui.modal).toBeNull();
    expect(ui.info).toBe(t("reset.done", { id: ID.slice(0, 8) }));
  });

  it("stops with the backup notice when the copy does not fit", async () => {
    mockedApi.resetTo.mockRejectedValueOnce({ code: "discard_store_full", message: "1 2 3" });

    await resetBranchTo(ID, "hard");

    expect(ui.errorMeta?.code).toBe("discard_store_full");
  });
});

describe("openCompare()", () => {
  it("opens the compare view for the pair", () => {
    openCompare({ from: "HEAD", to: "origin/main", fromLabel: "main", toLabel: "origin/main" });

    expect(ui.view).toBe("compare");
    expect(ui.compare?.to).toBe("origin/main");
  });

  it("is closed by a repo switch", () => {
    openCompare({ from: "a", to: "b", fromLabel: "a", toLabel: "b" });

    resetRepoSlices(ui);

    expect(ui.compare).toBeNull();
    expect(ui.view).toBe("repo");
  });
});
