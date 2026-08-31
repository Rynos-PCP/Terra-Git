import { describe, expect, it } from "vitest";
import {
  applyPipelineEvent,
  edgePath,
  initialPipelineSlice,
  layoutGraph,
  PIPE_LOG_ALL,
  resetPipelineOnRepoSwitch,
  rollupStatus,
  type EdgeGeometry,
  type PipelineSlice,
} from "./pipelineModel";
import type { PipelineGraph } from "./api";

describe("rollupStatus", () => {
  it("precedence running > failed > canceled > pending > unknown > success > skipped", () => {
    expect(rollupStatus(["success", "running", "failed"])).toBe("running");
    expect(rollupStatus(["success", "failed", "skipped"])).toBe("failed");
    expect(rollupStatus(["canceled", "success"])).toBe("canceled");
    expect(rollupStatus(["pending", "success"])).toBe("pending");
    expect(rollupStatus(["success", "skipped"])).toBe("success");
    expect(rollupStatus(["skipped"])).toBe("skipped");
    expect(rollupStatus([])).toBe("unknown");
  });

  it("unknown beats success: a single-job run does not colour the pipeline green", () => {
    expect(rollupStatus(["success", "unknown"])).not.toBe("success");
    expect(rollupStatus(["success", "unknown"])).toBe("unknown");
    expect(rollupStatus(["unknown"])).toBe("unknown");
    // running/failed keep precedence over unknown
    expect(rollupStatus(["unknown", "running"])).toBe("running");
    expect(rollupStatus(["unknown", "failed"])).toBe("failed");
  });
});

const graph: PipelineGraph = {
  provider: "gitlab",
  configFile: ".gitlab-ci.yml",
  stages: ["build", "test", "ship"],
  jobs: [
    { name: "build", stage: "build", needs: [], when: "", allowFailure: false },
    { name: "lint", stage: "test", needs: [], when: "", allowFailure: true },
    { name: "unit", stage: "test", needs: ["build"], when: "", allowFailure: false },
    {
      name: "deploy",
      stage: "ship",
      needs: ["unit", "ghost"],
      when: "manual",
      allowFailure: false,
    },
  ],
};

describe("layoutGraph", () => {
  it("columns per stage in order, jobs in graph order", () => {
    const l = layoutGraph(graph);
    expect(l.columns.map((c) => c.stage)).toEqual(["build", "test", "ship"]);
    expect(l.columns[1].jobs).toEqual(["lint", "unit"]);
  });
  it("edges from needs with col/row indices; unknown targets ignored", () => {
    const l = layoutGraph(graph);
    // unit(needs build): from build[col0,row0] to unit[col1,row1]
    expect(l.edges).toContainEqual({ from: { col: 0, row: 0 }, to: { col: 1, row: 1 } });
    // deploy(needs unit): from unit[col1,row1] to deploy[col2,row0]; "ghost" is missing -> no edge
    expect(l.edges).toContainEqual({ from: { col: 1, row: 1 }, to: { col: 2, row: 0 } });
    expect(l.edges).toHaveLength(2);
  });
});

describe("edgePath", () => {
  const G: EdgeGeometry = { colW: 220, colGap: 48, rowH: 44, rowGap: 12, headH: 40 };
  const xCoords = (d: string) => [...d.matchAll(/(-?[\d.]+) (-?[\d.]+)/g)].map((m) => Number(m[1]));

  it("normal case: a bezier from the right source edge to the left target edge (unchanged)", () => {
    const d = edgePath({ from: { col: 0, row: 0 }, to: { col: 1, row: 1 } }, G);
    expect(d).toBe("M 220 62 C 244 62, 244 118, 268 118");
  });

  it("same-stage (from.col === to.col): a short arc at the left node edge", () => {
    const left = 1 * (G.colW + G.colGap); // left edge of column 1 = 268
    const d = edgePath({ from: { col: 1, row: 0 }, to: { col: 1, row: 2 } }, G);
    const xs = xCoords(d);
    // starts and ends at the left edge …
    expect(xs[0]).toBe(left);
    expect(xs[xs.length - 1]).toBe(left);
    // … and NEVER runs backwards through the nodes (all x <= the left edge)
    for (const x of xs) expect(x).toBeLessThanOrEqual(left);
  });
});

describe("resetPipelineOnRepoSwitch", () => {
  it("resets the slice to its initial state and closes an open pipeline view", () => {
    const filled: PipelineSlice = {
      ...initialPipelineSlice(),
      selected: ".gitlab-ci.yml",
      configs: [{ path: ".gitlab-ci.yml", provider: "gitlab" }],
      graph,
      statuses: { build: "success" },
      logs: { [PIPE_LOG_ALL]: [{ n: 1, text: "x" }] },
      logSeq: 1,
      activeLog: "build",
      exit: 0,
    };
    const state = { view: "pipeline", pipeline: filled };
    resetPipelineOnRepoSwitch(state);
    expect(state.pipeline).toEqual(initialPipelineSlice());
    expect(state.view).toBe("repo");
  });

  it("leaves other views untouched", () => {
    const state = { view: "settings", pipeline: initialPipelineSlice() };
    resetPipelineOnRepoSwitch(state);
    expect(state.view).toBe("settings");
  });

  it("keeps an ACTIVE run: the slice object stays the same", () => {
    // runPipelineScope streams into the slice object captured at start and nulls
    // running on it in the finally — a replacement slice would orphan the run
    // (display gone, cancelPipelineRun would no longer find repoPath).
    const active: PipelineSlice = {
      ...initialPipelineSlice(),
      configs: [{ path: ".gitlab-ci.yml", provider: "gitlab" }],
      selected: ".gitlab-ci.yml",
      graph,
      statuses: { build: "running" },
      running: { scope: "pipeline", target: null, repoPath: "/repo-a" },
    };
    const state = { view: "pipeline", pipeline: active };
    resetPipelineOnRepoSwitch(state);
    expect(state.pipeline).toBe(active);
    expect(state.pipeline.running?.repoPath).toBe("/repo-a");
    // The view closes anyway — the run stays reachable through the cockpit of
    // the new repo (openPipeline leaves it standing).
    expect(state.view).toBe("repo");
  });
});

describe("applyPipelineEvent", () => {
  const fresh = () => ({
    statuses: {} as PipelineSlice["statuses"],
    logs: {} as PipelineSlice["logs"],
    logSeq: 0,
  });

  it("puts attributed lines in the job drawer AND in the overall log", () => {
    const p = fresh();
    applyPipelineEvent(p, { kind: "line", job: "build", line: "hello" });
    applyPipelineEvent(p, { kind: "line", job: null, line: "meta" });
    applyPipelineEvent(p, { kind: "line", job: "unit", line: "world" });
    expect(p.logs["build"]).toEqual([{ n: 1, text: "hello" }]);
    expect(p.logs["unit"]).toEqual([{ n: 3, text: "world" }]);
    // overall log interleaved: all lines in arrival order
    expect(p.logs[PIPE_LOG_ALL].map((l) => l.text)).toEqual(["hello", "meta", "world"]);
  });

  it("a status event sets statuses", () => {
    const p = fresh();
    applyPipelineEvent(p, { kind: "status", job: "build", status: "running" });
    expect(p.statuses["build"]).toBe("running");
    expect(p.logs).toEqual({});
  });

  it("truncation applies to the job drawer and the overall log; numbers stay monotonic", () => {
    const p = fresh();
    for (let i = 1; i <= 5; i++) {
      applyPipelineEvent(p, { kind: "line", job: "j", line: `z${i}` }, 3);
    }
    expect(p.logs["j"].map((l) => l.text)).toEqual(["z3", "z4", "z5"]);
    expect(p.logs["j"].map((l) => l.n)).toEqual([3, 4, 5]);
    expect(p.logs[PIPE_LOG_ALL].map((l) => l.n)).toEqual([3, 4, 5]);
  });
});
