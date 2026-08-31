// Pipeline slice behaviour of state.svelte.ts with a mocked IPC layer: the
// repo-switch reset and sequence tokens against late graph answers.
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { PipelineGraph } from "./api";
import { initialPipelineSlice } from "./pipelineModel";

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
    pipelineGraph: vi.fn(),
  },
}));

import { api } from "./api";
import { openRepo, selectPipelineConfig, ui } from "./state.svelte";

const mockedApi = vi.mocked(api);

const graphOf = (configFile: string): PipelineGraph => ({
  provider: "gitlab",
  configFile,
  stages: ["build"],
  jobs: [{ name: "build", stage: "build", needs: [], when: "", allowFailure: false }],
});

beforeEach(() => {
  vi.clearAllMocks();
  ui.repo = null;
  ui.view = "repo";
  ui.error = null;
  ui.info = null;
  ui.pipeline = initialPipelineSlice();
});

describe("openRepo (repo switch)", () => {
  it("resets the pipeline slice and leaves the pipeline view", async () => {
    await openRepo("A");
    ui.view = "pipeline";
    const p = ui.pipeline;
    p.configs = [{ path: ".gitlab-ci.yml", provider: "gitlab" }];
    p.selected = ".gitlab-ci.yml";
    p.graph = graphOf(".gitlab-ci.yml");
    p.statuses = { build: "success" };
    p.logs = { build: [{ n: 1, text: "line" }] };
    p.logSeq = 1;
    p.activeLog = "build";
    p.exit = 0;

    await openRepo("B");

    expect(ui.repo?.path).toBe("B");
    expect(ui.pipeline).toEqual(initialPipelineSlice());
    expect(ui.view).toBe("repo");
  });
});

describe("selectPipelineConfig (graph race)", () => {
  it("a late answer of the old selection does not overwrite the newer one", async () => {
    await openRepo("A");
    ui.pipeline.configs = [
      { path: "a.yml", provider: "gitlab" },
      { path: "b.yml", provider: "gitlab" },
    ];

    let resolveA!: (g: PipelineGraph) => void;
    let resolveB!: (g: PipelineGraph) => void;
    mockedApi.pipelineGraph
      .mockImplementationOnce(() => new Promise<PipelineGraph>((r) => (resolveA = r)))
      .mockImplementationOnce(() => new Promise<PipelineGraph>((r) => (resolveB = r)));

    const first = selectPipelineConfig("a.yml");
    const second = selectPipelineConfig("b.yml");

    // The second (current) selection answers first …
    resolveB(graphOf("b.yml"));
    await second;
    expect(ui.pipeline.graph?.configFile).toBe("b.yml");

    // … the late first answer is discarded.
    resolveA(graphOf("a.yml"));
    await first;
    expect(ui.pipeline.graph?.configFile).toBe("b.yml");
    expect(ui.pipeline.selected).toBe("b.yml");
  });

  it("a late error of the old selection sets no error state", async () => {
    await openRepo("A");
    ui.pipeline.configs = [
      { path: "a.yml", provider: "gitlab" },
      { path: "b.yml", provider: "gitlab" },
    ];

    let rejectA!: (e: unknown) => void;
    mockedApi.pipelineGraph
      .mockImplementationOnce(() => new Promise<PipelineGraph>((_, rej) => (rejectA = rej)))
      .mockImplementationOnce(async () => graphOf("b.yml"));

    const first = selectPipelineConfig("a.yml");
    await selectPipelineConfig("b.yml");
    rejectA({ code: "runner_failed", message: "broken" });
    await first;

    expect(ui.pipeline.error).toBe(false);
    expect(ui.error).toBeNull();
    expect(ui.pipeline.graph?.configFile).toBe("b.yml");
  });
});
