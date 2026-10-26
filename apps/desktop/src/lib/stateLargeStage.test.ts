// User report 2026-10-13: staging ~25k files looked frozen. Large sets now run
// as a visible, cancellable operation with a file counter; the line balance no
// longer runs several times in parallel.
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { FileLineStats, GitProgress, RepoInfo, RepoStatus } from "./api";

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
    stage: vi.fn(async () => {}),
    status: vi.fn(async () => cleanStatus),
    statusNumstat: vi.fn(async () => []),
    undoStatus: vi.fn(async () => null),
  },
}));

import { api } from "./api";
import { t, tn } from "./i18n.svelte";
import { LARGE_STAGE, refreshNumstat, stageFiles, ui } from "./state.svelte";

const mockedApi = vi.mocked(api);
const many = (n: number) => Array.from({ length: n }, (_, i) => `f${i}.txt`);

beforeEach(() => {
  vi.clearAllMocks();
  ui.repo = repo;
  ui.status = cleanStatus;
  ui.busy = null;
  ui.busyCancellable = false;
  ui.progress = null;
  ui.working = 0;
  ui.error = null;
  ui.info = null;
  ui.numstat = null;
});

describe("stageFiles() with large sets", () => {
  it("runs as a cancellable busy operation and shows the file counter", async () => {
    const seen: (GitProgress | null)[] = [];
    mockedApi.stage.mockImplementationOnce(async (_p, _f, onProgress) => {
      expect(ui.busy).toBe(t("state.staging"));
      expect(ui.busyCancellable).toBe(true);
      onProgress?.({ phase: "staging", percent: 40, done: 1000, total: 2500 });
      seen.push(ui.progress);
    });

    await stageFiles(many(2500));

    expect(seen[0]).toEqual({ phase: "staging", percent: 40, done: 1000, total: 2500 });
    expect(ui.info).toBe(tn("state.filesStaged", 2500));
    expect(ui.busy).toBeNull();
    expect(ui.busyCancellable).toBe(false);
    expect(ui.progress).toBeNull();
    expect(mockedApi.status).toHaveBeenCalled();
  });

  it("treats a cancel as a neutral notice", async () => {
    mockedApi.stage.mockRejectedValueOnce({ code: "cancelled", message: "Operation cancelled" });

    await stageFiles(many(LARGE_STAGE));

    expect(ui.error).toBeNull();
    expect(ui.info).toBe(t("state.stagingCancelled"));
    expect(ui.busy).toBeNull();
  });

  it("keeps small sets a quick call without blocking the toolbar", async () => {
    mockedApi.stage.mockImplementationOnce(async () => {
      expect(ui.busy).toBeNull();
    });

    await stageFiles(many(LARGE_STAGE - 1));

    expect(mockedApi.stage).toHaveBeenCalledTimes(1);
    expect(ui.info).toBeNull();
  });

  it("does not start while another operation holds the toolbar", async () => {
    ui.busy = "Pull";

    await stageFiles(many(LARGE_STAGE));

    expect(mockedApi.stage).not.toHaveBeenCalled();
  });
});

describe("refreshNumstat()", () => {
  it("never runs twice at once — requests during a run collapse into one rerun", async () => {
    let release: (v: FileLineStats[]) => void = () => {};
    mockedApi.statusNumstat.mockImplementationOnce(
      () => new Promise<FileLineStats[]>((r) => (release = r)),
    );

    const first = refreshNumstat();
    void refreshNumstat();
    void refreshNumstat();
    expect(mockedApi.statusNumstat).toHaveBeenCalledTimes(1);

    release([]);
    await first;
    await vi.waitFor(() => expect(mockedApi.statusNumstat).toHaveBeenCalledTimes(2));
    // Nothing else queued: the rerun was the last one.
    await new Promise((r) => setTimeout(r, 0));
    expect(mockedApi.statusNumstat).toHaveBeenCalledTimes(2);
  });
});
