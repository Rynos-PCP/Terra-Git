// The tool/Docker status must not freeze.
//
// The failure seen in practice: the user opens the pipeline cockpit while Docker
// is not running yet -> the hint "Docker is not running" appears. Then they
// start Docker and click "Run pipeline" — the hint stays, because
// pipeline_detect only ran when entering the view. There was exactly one call in
// the whole frontend and no way to check again.
import { beforeEach, describe, expect, it, vi } from "vitest";
import { initialPipelineSlice } from "./pipelineModel";

vi.mock("@tauri-apps/plugin-dialog", () => ({ confirm: vi.fn(async () => false) }));

const info = (dockerRunning: boolean) => ({
  provider: "gitlab" as const,
  configFile: ".gitlab-ci.yml",
  runnersInstalled: { gitlab: true, github: false },
  dockerRunning,
  missingTools: [] as string[],
});

vi.mock("./api", () => ({
  api: {
    pipelineDetect: vi.fn(async () => info(true)),
    pipelineConfigs: vi.fn(async () => [{ path: ".gitlab-ci.yml", provider: "gitlab" }]),
    pipelineGraph: vi.fn(async () => ({
      provider: "gitlab",
      configFile: ".gitlab-ci.yml",
      stages: ["build"],
      jobs: [{ name: "build", stage: "build", needs: [], when: "", allowFailure: false }],
    })),
    pipelineRunScope: vi.fn(async () => 0),
  },
}));

import { api } from "./api";
import { refreshPipelineTools, runPipelineScope, ui } from "./state.svelte";

const mockedApi = vi.mocked(api);

beforeEach(() => {
  vi.clearAllMocks();
  ui.repo = {
    path: "/repo",
    name: "repo",
    currentBranch: "main",
    headDetached: false,
    isEmpty: false,
    historyPrepared: true,
  };
  ui.pipeline = initialPipelineSlice();
  const p = ui.pipeline;
  p.configs = [{ path: ".gitlab-ci.yml", provider: "gitlab" }];
  p.selected = ".gitlab-ci.yml";
  p.graph = {
    provider: "gitlab",
    configFile: ".gitlab-ci.yml",
    stages: ["build"],
    jobs: [{ name: "build", stage: "build", needs: [], when: "", allowFailure: false }],
  };
  // The state as after opening the view WITHOUT a running Docker.
  p.info = info(false);
});

describe("refreshing the tool status", () => {
  it("re-checks when a run starts so a Docker started in the meantime is detected", async () => {
    expect(ui.pipeline.info?.dockerRunning).toBe(false);

    await runPipelineScope("pipeline", null);

    expect(mockedApi.pipelineDetect).toHaveBeenCalledWith("/repo");
    expect(ui.pipeline.info?.dockerRunning, "hint must disappear").toBe(true);
  });

  it("still lets the run start when the status check fails", async () => {
    mockedApi.pipelineDetect.mockRejectedValueOnce(new Error("probe broken"));

    await runPipelineScope("pipeline", null);

    expect(mockedApi.pipelineRunScope, "run must not be blocked").toHaveBeenCalled();
  });

  it("offers a manual re-check", async () => {
    await refreshPipelineTools();

    expect(mockedApi.pipelineDetect).toHaveBeenCalledWith("/repo");
    expect(ui.pipeline.info?.dockerRunning).toBe(true);
  });
});
