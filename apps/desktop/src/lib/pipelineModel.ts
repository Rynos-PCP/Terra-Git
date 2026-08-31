// Pure pipeline model functions (Vitest-tested): status rollup
// (job -> stage -> pipeline), graph layout (columns + needs edges), edge
// geometry, slice lifecycle and event application of the live run.
import type {
  PipelineConfig,
  PipelineEvent,
  PipelineGraph,
  PipelineInfo,
  PipelineJobStatus,
} from "./api";

/** Precedence for the aggregation: the first hit wins. "unknown" comes BEFORE
 *  "success" — a single-job run (9/10 jobs never started) must not colour the
 *  pipeline badge green. */
const PRECEDENCE: PipelineJobStatus[] = [
  "running",
  "failed",
  "canceled",
  "pending",
  "unknown",
  "success",
  "skipped",
];

export function rollupStatus(list: PipelineJobStatus[]): PipelineJobStatus {
  for (const s of PRECEDENCE) {
    if (list.includes(s)) return s;
  }
  return "unknown";
}

export interface GraphLayout {
  columns: { stage: string; jobs: string[] }[];
  edges: { from: { col: number; row: number }; to: { col: number; row: number } }[];
}

/** Columns per stage (graph order) + needs edges as indices. */
export function layoutGraph(graph: PipelineGraph): GraphLayout {
  const columns = graph.stages.map((stage) => ({
    stage,
    jobs: graph.jobs.filter((j) => j.stage === stage).map((j) => j.name),
  }));
  const pos = new Map<string, { col: number; row: number }>();
  columns.forEach((c, col) => c.jobs.forEach((name, row) => pos.set(name, { col, row })));
  const edges: GraphLayout["edges"] = [];
  for (const j of graph.jobs) {
    const to = pos.get(j.name);
    if (!to) continue;
    for (const n of j.needs) {
      const from = pos.get(n);
      if (from) edges.push({ from, to });
    }
  }
  return { columns, edges };
}

/** Geometry constants of the graph (one source for the node AND edge layer). */
export interface EdgeGeometry {
  colW: number;
  colGap: number;
  rowH: number;
  rowGap: number;
  headH: number;
  /** Left inner padding: gives the same-col arc of the FIRST column room
   *  (otherwise it would sit at a negative x and be clipped by the scroll
   *  container). Default 0. */
  padX?: number;
}

/**
 * SVG path of a needs edge. Normal case: a bezier from the right edge of the
 * source node to the left edge of the target node. Special case same-stage
 * (from.col === to.col, legal in GitLab): a short arc at the LEFT node edge
 * instead of a backwards bezier straight through both nodes.
 */
export function edgePath(
  e: { from: { col: number; row: number }; to: { col: number; row: number } },
  g: EdgeGeometry,
): string {
  const pad = g.padX ?? 0;
  const nodeX = (col: number) => pad + col * (g.colW + g.colGap);
  const nodeY = (row: number) => g.headH + row * (g.rowH + g.rowGap);
  const y1 = nodeY(e.from.row) + g.rowH / 2;
  const y2 = nodeY(e.to.row) + g.rowH / 2;
  if (e.from.col === e.to.col) {
    const x = nodeX(e.from.col);
    // Never let the arc go further left than x=0 (column 0 without padX would be clipped).
    const bow = Math.min(32, g.colGap, x);
    return `M ${x} ${y1} C ${x - bow} ${y1}, ${x - bow} ${y2}, ${x} ${y2}`;
  }
  const x1 = nodeX(e.from.col) + g.colW;
  const x2 = nodeX(e.to.col);
  const dx = Math.max(24, (x2 - x1) / 2);
  return `M ${x1} ${y1} C ${x1 + dx} ${y1}, ${x2 - dx} ${y2}, ${x2} ${y2}`;
}

// ================= Pipeline slice (state shape + lifecycle) =================

/** Log line with a monotonic number — a stable each key despite truncation
 *  (index keys would patch the whole list on a sliding window). */
export interface PipelineLogLine {
  n: number;
  text: string;
}

/** Key of the interleaved overall log (all jobs + unattributed lines). */
export const PIPE_LOG_ALL = "__pipeline__";

/** Cap of the log lines per drawer against DOM load on very long runs. */
export const PIPE_LOG_CAP = 2000;

/** One CI variable of the next run (key/value). Passed to the runner
 *  file-based in the backend, never inline on the command line. */
export interface PipelineVar {
  key: string;
  value: string;
}

export interface PipelineSlice {
  info: PipelineInfo | null;
  configs: PipelineConfig[];
  selected: string | null;
  /** act trigger event of the next run (push/pull_request/workflow_dispatch/
   *  tag). Only relevant for act/GitHub configs; GitLab ignores it. */
  event: string;
  /** CI variables of the next run (key/value editor). Empty keys are dropped
   *  on start. */
  variables: PipelineVar[];
  graph: PipelineGraph | null;
  statuses: Record<string, PipelineJobStatus>;
  logs: Record<string, PipelineLogLine[]>;
  /** Monotonic line counter (each keys; truncation keeps the numbers). */
  logSeq: number;
  activeLog: string | null;
  /** Running scope run; repoPath is the repo OF THE RUN (cancelling has to hit
   *  the right run even after a repo switch). */
  running: null | { scope: "pipeline" | "stage" | "job"; target: string | null; repoPath: string };
  exit: number | null;
  /** Cancelled by the user (the footer shows "cancelled" instead of an exit code). */
  canceled: boolean;
  error: boolean;
}

export function initialPipelineSlice(): PipelineSlice {
  return {
    info: null,
    configs: [],
    selected: null,
    event: "push",
    variables: [],
    graph: null,
    statuses: {},
    logs: {},
    logSeq: 0,
    activeLog: null,
    running: null,
    exit: null,
    canceled: false,
    error: false,
  };
}

/**
 * Repo switch: reset the pipeline slice to its initial state and close an open
 * pipeline view — the graph/configs of repo A must never affect repo B.
 * EXCEPTION for an active run: the slice stays THE SAME, because runPipelineScope
 * streams into the object captured at start and nulls `running` on it in the
 * finally — a replacement slice would orphan the run (the container would keep
 * running invisibly, cancelPipelineRun would no longer find running/repoPath).
 * openPipeline leaves a running slice standing anyway; after the run ends the
 * next openPipeline call cleans up normally.
 */
export function resetPipelineOnRepoSwitch(ui: { view: string; pipeline: PipelineSlice }): void {
  if (!ui.pipeline.running) ui.pipeline = initialPipelineSlice();
  if (ui.view === "pipeline") ui.view = "repo";
}

/**
 * Applies a run event to the slice (exported + pure so it is testable).
 * Log lines land in the job drawer AND interleaved in the overall log
 * (PIPE_LOG_ALL); both drawers are capped at `cap` lines.
 */
export function applyPipelineEvent(
  p: Pick<PipelineSlice, "statuses" | "logs" | "logSeq">,
  ev: PipelineEvent,
  cap: number = PIPE_LOG_CAP,
): void {
  if (ev.kind === "status") {
    p.statuses[ev.job] = ev.status;
    return;
  }
  const line: PipelineLogLine = { n: ++p.logSeq, text: ev.line };
  const push = (key: string) => {
    const cur = p.logs[key] ?? [];
    p.logs[key] = [...cur.slice(-(cap - 1)), line];
  };
  if (ev.job) push(ev.job);
  push(PIPE_LOG_ALL);
}
