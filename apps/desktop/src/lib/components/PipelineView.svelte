<script lang="ts">
  import { t, tn } from "../i18n.svelte";
  import {
    cancelPipelineRun,
    addPipelineConfigFile,
    closePipeline,
    openPipeline,
    refreshPipelineTools,
    runPipelineScope,
    selectPipelineConfig,
    ui,
  } from "../state.svelte";
  import { edgePath, layoutGraph, PIPE_LOG_ALL, rollupStatus } from "../pipelineModel";
  import type { PipelineJobStatus } from "../api";
  import { tooltip } from "../tooltip";
  import Icon from "./Icon.svelte";

  const p = $derived(ui.pipeline);
  const layout = $derived(p.graph ? layoutGraph(p.graph) : null);
  const gitlab = $derived(p.graph?.provider === "gitlab");
  // The runner gate/banner hang off the CHOSEN config, not off the
  // auto-detected provider (a repo can have gitlab AND github configs).
  const selProvider = $derived(
    p.configs.find((c) => c.path === p.selected)?.provider ?? p.info?.provider ?? null,
  );
  const selRunnerInstalled = $derived(
    selProvider ? (p.info?.runnersInstalled[selProvider] ?? false) : false,
  );

  // CI variable editor (applies to gitlab AND act; file-based in the backend).
  let varsOpen = $state(false);
  const varCount = $derived(p.variables.filter((v) => v.key.trim() !== "").length);
  function addVar() {
    p.variables = [...p.variables, { key: "", value: "" }];
    varsOpen = true;
  }
  function removeVar(i: number) {
    p.variables = p.variables.filter((_, idx) => idx !== i);
  }

  function jobStatus(name: string): PipelineJobStatus {
    return p.statuses[name] ?? "unknown";
  }
  const stageStatuses = $derived(
    (layout?.columns ?? []).map((c) => rollupStatus(c.jobs.map(jobStatus))),
  );
  const overall = $derived(rollupStatus(stageStatuses));
  // Make a cancellation visible: an explicit flag (cancelPipelineRun) OR a
  // canceled job status from the run.
  const canceled = $derived(p.canceled || Object.values(p.statuses).includes("canceled"));
  const activeLines = $derived(
    p.activeLog ? (p.logs[p.activeLog] ?? []) : (p.logs[PIPE_LOG_ALL] ?? []),
  );

  // Geometry for nodes + SVG edges (one source for both layers).
  const COL_W = 220;
  const COL_GAP = 48;
  const ROW_H = 44;
  const ROW_GAP = 12;
  const HEAD_H = 40;
  // Left inner padding for the same-col edge arc of the first column.
  const PAD_X = 36;
  const GEOMETRY = {
    colW: COL_W,
    colGap: COL_GAP,
    rowH: ROW_H,
    rowGap: ROW_GAP,
    headH: HEAD_H,
    padX: PAD_X,
  };
  const nodeX = (col: number) => PAD_X + col * (COL_W + COL_GAP);
  const nodeY = (row: number) => HEAD_H + row * (ROW_H + ROW_GAP);
  const graphW = $derived(PAD_X + (layout?.columns.length ?? 0) * (COL_W + COL_GAP));
  const graphH = $derived(
    HEAD_H + Math.max(1, ...(layout?.columns ?? []).map((c) => c.jobs.length)) * (ROW_H + ROW_GAP),
  );
  // ---- Prerequisite chips instead of warning prose ----
  type ChipState = "ok" | "limited" | "blocked" | "note";
  interface PrereqChip {
    key: string;
    state: ChipState;
    icon: "check" | "x" | "alert" | "info";
    label: string;
    /** Chips without a disclosure (ok state) have no detail view. */
    expandable: boolean;
  }
  const chips = $derived.by<PrereqChip[]>(() => {
    const list: PrereqChip[] = [];
    if (p.info && selProvider) {
      list.push(
        selRunnerInstalled
          ? {
              key: "runner",
              state: "ok",
              icon: "check",
              label: t("pipe.chipRunnerOk"),
              expandable: false,
            }
          : {
              key: "runner",
              state: "blocked",
              icon: "x",
              label: t("pipe.chipRunnerMissing"),
              expandable: true,
            },
      );
      if (selProvider === "gitlab") {
        list.push(
          p.info.missingTools.length === 0
            ? {
                key: "tools",
                state: "ok",
                icon: "check",
                label: t("pipe.chipToolsOk"),
                expandable: false,
              }
            : {
                key: "tools",
                state: "blocked",
                icon: "x",
                label: tn("pipe.chipToolsMissing", p.info.missingTools.length),
                expandable: true,
              },
        );
      }
      list.push(
        p.info.dockerRunning
          ? {
              key: "docker",
              state: "ok",
              icon: "check",
              label: t("pipe.chipDockerOk"),
              expandable: false,
            }
          : {
              key: "docker",
              state: "limited",
              icon: "alert",
              label: t("pipe.chipDockerOff"),
              expandable: true,
            },
      );
    }
    // The approximation hint always applies — as a quiet info chip instead of a permanent line.
    list.push({
      key: "note",
      state: "note",
      icon: "info",
      label: t("pipe.chipNote"),
      expandable: true,
    });
    return list;
  });
  let openChip = $state<string | null>(null);
  // States can turn under the open chip (e.g. Docker started + re-checked):
  // close the disclosure then.
  $effect(() => {
    if (openChip && !chips.some((c) => c.key === openChip && c.expandable)) openChip = null;
  });

  // The log only becomes a full area from the first run on — a narrow bar before that.
  const hasRun = $derived(!!p.running || p.exit !== null || activeLines.length > 0);

  let logEl = $state<HTMLElement>();
  $effect(() => {
    void activeLines.length;
    const el = logEl;
    if (!el) return;
    // Auto-scroll only when the user is reading near the end anyway — otherwise
    // every new line would undo a manual scroll back.
    if (el.scrollHeight - el.scrollTop - el.clientHeight < 80) {
      el.scrollTo(0, el.scrollHeight);
    }
  });
</script>

<div class="pipeline">
  <header>
    <button class="ghost back" onclick={closePipeline} use:tooltip={t("settings.back")}>
      <Icon name="chevronDown" size={14} />
      {t("settings.back")}
    </button>
    <h1>{t("pipe.title")}</h1>
    <span class="badge st-{overall}">{t(`pipe.status.${overall}`)}</span>
    {#if p.configs.length > 1}
      <select
        value={p.selected}
        onchange={(e) => selectPipelineConfig(e.currentTarget.value)}
        disabled={!!p.running}
      >
        {#each p.configs as c (c.path)}<option value={c.path}>{c.path}</option>{/each}
      </select>
    {/if}
    <!-- Always visible: choose a CI file manually (even when nothing was discovered). -->
    <button
      class="ghost"
      onclick={addPipelineConfigFile}
      use:tooltip={t("pipeline.chooseFile")}
      disabled={!!p.running}
    >
      <Icon name="folder" size={14} />
    </button>
    <!-- Trigger event only for act/GitHub (GitLab has no events). -->
    {#if selProvider && selProvider !== "gitlab"}
      <select
        class="event"
        value={p.event}
        onchange={(e) => (p.event = e.currentTarget.value)}
        disabled={!!p.running}
        use:tooltip={t("pipe.eventHint")}
        aria-label={t("pipe.event")}
      >
        <option value="push">push</option>
        <option value="pull_request">pull_request</option>
        <option value="workflow_dispatch">workflow_dispatch</option>
        <option value="tag">tag</option>
      </select>
    {/if}
    <!-- CI variables (applies to gitlab AND act). -->
    {#if selProvider}
      <button
        class="ghost"
        class:active={varsOpen}
        onclick={() => (varsOpen = !varsOpen)}
        use:tooltip={t("pipe.varsHint")}
        aria-pressed={varsOpen}
      >
        <Icon name="settings" size={14} />
        {t("pipe.vars")}{varCount > 0 ? ` (${varCount})` : ""}
      </button>
    {/if}
    {#if p.running}
      <button class="danger" onclick={cancelPipelineRun}>{t("pipe.cancel")}</button>
    {:else}
      <button
        class="primary"
        onclick={() => runPipelineScope("pipeline", null)}
        disabled={!p.graph || p.graph.jobs.length === 0 || !selRunnerInstalled}
      >
        {t("pipe.runAll")}
      </button>
    {/if}
    <button
      class="ghost"
      onclick={openPipeline}
      use:tooltip={t("pipe.reload")}
      disabled={!!p.running}
    >
      <Icon name="refresh" size={14} />
    </button>
  </header>

  <!-- Prerequisites as status chips; the long instructions sit behind
       "Fix…"/"Details" instead of as permanent prose. -->
  <div class="chips">
    {#each chips as chip (chip.key)}
      {#if chip.expandable}
        <button
          class="chip {chip.state}"
          aria-expanded={openChip === chip.key}
          onclick={() => (openChip = openChip === chip.key ? null : chip.key)}
        >
          <Icon name={chip.icon} size={12} />
          {chip.label}
          <span class="chip-more">
            {chip.state === "note" ? t("pipe.chipDetails") : t("pipe.chipFix")}
          </span>
        </button>
      {:else}
        <span class="chip {chip.state}"><Icon name={chip.icon} size={12} /> {chip.label}</span>
      {/if}
    {/each}
  </div>
  {#if openChip === "runner"}
    <div class="chip-detail">
      <p class="danger-text">
        {t("pipe.runnerMissing", { cmd: selProvider === "gitlab" ? "gitlab-ci-local" : "act" })}
      </p>
      <p>{selProvider === "gitlab" ? t("pipe.installGitlab") : t("pipe.installGithub")}</p>
    </div>
  {:else if openChip === "tools"}
    <div class="chip-detail">
      <p class="danger-text">
        {t("pipe.toolsMissing", { tools: p.info?.missingTools.join(", ") ?? "" })}
      </p>
      <p>{t("pipe.toolsMissingHint")}</p>
    </div>
  {:else if openChip === "docker"}
    <div class="chip-detail">
      <p>
        {t("pipe.dockerOff")}
        <!-- Otherwise the status is only determined when entering the view: whoever
             starts Docker only afterwards needs a way to re-check. -->
        <button class="ghost recheck" onclick={() => refreshPipelineTools()}>
          {t("pipe.recheck")}
        </button>
      </p>
    </div>
  {:else if openChip === "note"}
    <div class="chip-detail"><p>{t("pipe.approx")}</p></div>
  {/if}

  {#if varsOpen && selProvider}
    <div class="vars">
      <div class="vars-head">
        <span>{t("pipe.varsTitle")}</span>
        <span class="hint">{t("pipe.varsFileHint")}</span>
      </div>
      {#each p.variables as row, i (i)}
        <div class="vars-row">
          <input
            class="k"
            placeholder={t("pipe.varKey")}
            bind:value={row.key}
            disabled={!!p.running}
            spellcheck="false"
            autocapitalize="off"
          />
          <span class="eq">=</span>
          <input
            class="v"
            placeholder={t("pipe.varValue")}
            bind:value={row.value}
            disabled={!!p.running}
            spellcheck="false"
          />
          <button
            class="ghost"
            onclick={() => removeVar(i)}
            disabled={!!p.running}
            use:tooltip={t("pipe.varRemove")}
            aria-label={t("pipe.varRemove")}
          >
            <Icon name="trash" size={14} />
          </button>
        </div>
      {/each}
      <button class="ghost add" onclick={addVar} disabled={!!p.running}>
        <Icon name="plus" size={14} />
        {t("pipe.varAdd")}
      </button>
    </div>
  {/if}

  {#if p.error}
    <div class="empty">
      <p>{t("pipe.loadError")}</p>
      <button onclick={openPipeline}>{t("pipe.retry")}</button>
    </div>
  {:else if p.configs.length === 0}
    <p class="empty">{t("pipe.none")}</p>
  {:else if !p.graph}
    <p class="empty"><span class="spin"></span> {t("pipe.jobsLoading")}</p>
  {:else if layout}
    <div class="graph-scroll">
      <div class="graph" style="width:{graphW}px;height:{graphH}px">
        <svg class="edges" width={graphW} height={graphH} aria-hidden="true">
          {#each layout.edges as e, i (i)}<path d={edgePath(e, GEOMETRY)} />{/each}
        </svg>
        {#each layout.columns as col, ci (col.stage)}
          <div class="stage-head" style="left:{nodeX(ci)}px;width:{COL_W}px">
            <span class="dot st-{stageStatuses[ci]}"></span>
            <strong>{col.stage}</strong>
            {#if gitlab}
              <button
                class="ghost mini"
                use:tooltip={t("pipe.runStage")}
                disabled={!!p.running}
                onclick={() => runPipelineScope("stage", col.stage)}>▶</button
              >
            {/if}
          </div>
          {#each col.jobs as job, ri (job)}
            {@const node = p.graph?.jobs.find((j) => j.name === job)}
            <!-- div instead of button: the start button is a REAL button inside it
                 (button-in-button is invalid HTML and fails the a11y check). -->
            <div
              class="node st-{jobStatus(job)}"
              class:active={p.activeLog === job}
              style="left:{nodeX(ci)}px;top:{nodeY(ri)}px;width:{COL_W}px;height:{ROW_H}px"
              role="button"
              tabindex="0"
              onclick={() => (p.activeLog = job)}
              onkeydown={(e) => {
                if (e.key === "Enter") p.activeLog = job;
              }}
            >
              <span class="dot st-{jobStatus(job)}"></span>
              <span class="name" use:tooltip={job}>{job}</span>
              {#if node?.when === "manual"}<span class="tag">{t("pipe.manual")}</span>{/if}
              {#if node?.allowFailure}<span class="tag">{t("pipe.allowFailure")}</span>{/if}
              <button
                class="run mini ghost"
                use:tooltip={t("pipe.run")}
                disabled={!!p.running}
                onclick={(e) => {
                  e.stopPropagation();
                  runPipelineScope("job", job);
                }}>▶</button
              >
            </div>
          {/each}
        {/each}
      </div>
    </div>

    <!-- Before the first run only a narrow bar instead of 1/3 of the height. -->
    <div class="log-drawer" class:collapsed={!hasRun}>
      <div class="log-head">
        {#if !hasRun}
          <span class="hint">{t("pipe.logIdle")}</span>
        {:else}
          <button
            class="ghost mini log-all"
            class:active={p.activeLog === null}
            onclick={() => (p.activeLog = null)}
          >
            {t("pipe.logAll")}
          </button>
        {/if}
        {#if p.activeLog}<strong>{p.activeLog}</strong>{/if}
        {#if p.running}
          <span class="hint"><span class="spin"></span> {t("pipe.runningScope")}</span>
        {:else if p.exit !== null}
          {#if canceled}
            <span class="hint">{t("pipe.exitCanceled")}</span>
          {:else}
            <span class="hint" class:danger-text={p.exit !== 0}>
              {p.exit === 0 ? t("pipe.exitOk") : t("pipe.exitFail", { code: p.exit })}
            </span>
          {/if}
        {/if}
      </div>
      {#if hasRun}
        <div class="pipe-log" bind:this={logEl}>
          {#each activeLines as line (line.n)}<div>{line.text}</div>{/each}
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .recheck {
    margin-left: var(--space-1);
    padding: 1px 8px;
    font-size: 11px;
  }

  .pipeline {
    display: flex;
    flex-direction: column;
    height: 100%;
    padding: var(--space-3);
    gap: var(--space-2);
    overflow: hidden;
  }
  header {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  header h1 {
    font-family: var(--display);
    font-size: 16px;
    font-weight: 650;
  }
  header select {
    margin-left: auto;
  }
  .badge {
    font-size: 11px;
    padding: 2px 8px;
    border-radius: 999px;
    border: 1px solid var(--border);
  }
  .hint {
    color: var(--text-muted);
    font-size: 12px;
  }
  .danger-text {
    color: var(--deleted);
  }
  header button.active {
    color: var(--accent);
    outline: 1px solid var(--accent);
  }
  .vars {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    margin-bottom: var(--space-2);
  }
  .vars-head {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
  }
  .vars-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .vars-row input {
    padding: 4px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--bg-elevated);
    color: var(--text-primary);
    font-family: var(--mono, ui-monospace, monospace);
    font-size: 12px;
  }
  .vars-row .k {
    flex: 0 0 34%;
    min-width: 0;
  }
  .vars-row .v {
    flex: 1;
    min-width: 0;
  }
  .vars-row .eq {
    color: var(--text-muted);
  }
  .vars .add {
    align-self: flex-start;
  }
  .empty {
    color: var(--text-muted);
    margin: auto;
    text-align: center;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .graph-scroll {
    flex: 1;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: var(--space-3);
  }
  .graph {
    position: relative;
  }
  .edges {
    position: absolute;
    inset: 0;
    pointer-events: none;
    /* Same-stage arcs stick out slightly to the left of the node column. */
    overflow: visible;
  }
  .edges path {
    fill: none;
    /* needs edges clearly visible. */
    stroke: var(--border-strong);
    stroke-width: 2;
  }
  .stage-head {
    position: absolute;
    top: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    height: 28px;
  }
  .node {
    position: absolute;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--bg-elevated);
    color: var(--text-primary);
    cursor: pointer;
    text-align: left;
  }
  .node.active {
    outline: 2px solid var(--accent);
  }
  .node .name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .node .run {
    opacity: 0.6;
  }
  .node .run:hover {
    opacity: 1;
  }
  .mini {
    padding: 0 4px;
    font-size: 11px;
  }
  .tag {
    font-size: 10px;
    padding: 0 5px;
    border-radius: 999px;
    border: 1px solid var(--border);
    color: var(--text-muted);
  }
  .dot {
    width: 9px;
    height: 9px;
    border-radius: 50%;
    background: var(--text-muted);
    flex: none;
  }
  .dot.st-running {
    background: var(--accent);
  }
  .dot.st-success {
    background: #3fb950;
  }
  .node.st-success {
    border-color: #3fb95055;
  }
  .dot.st-failed {
    background: #f85149;
  }
  .node.st-failed {
    border-color: #f8514955;
  }
  .dot.st-canceled {
    background: #d29922;
  }
  .dot.st-pending {
    background: #d29922aa;
  }
  .dot.st-skipped,
  .dot.st-unknown {
    background: var(--text-muted);
    opacity: 0.5;
  }
  .badge.st-failed {
    color: #f85149;
  }
  .badge.st-success {
    color: #3fb950;
  }
  .badge.st-running {
    color: var(--accent);
  }
  .log-drawer {
    height: 220px;
    display: flex;
    flex-direction: column;
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  .log-drawer.collapsed {
    height: auto;
  }
  .log-drawer.collapsed .log-head {
    border-bottom: none;
  }

  /* ---- Prerequisite chips ---- */
  .chips {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 11.5px;
    font-weight: 550;
    padding: 3px 10px;
    border-radius: 999px;
    border: 1px solid var(--border);
    background: transparent;
  }
  .chip.ok {
    color: var(--added);
    border-color: color-mix(in srgb, var(--added) 40%, transparent);
  }
  .chip.limited {
    color: var(--modified);
    border-color: color-mix(in srgb, var(--modified) 45%, transparent);
  }
  .chip.blocked {
    color: var(--deleted);
    border-color: color-mix(in srgb, var(--deleted) 45%, transparent);
  }
  .chip.note {
    color: var(--text-muted);
  }
  button.chip {
    cursor: pointer;
  }
  button.chip:hover {
    background: var(--bg-hover);
  }
  .chip-more {
    color: var(--text-faint);
    font-size: 10.5px;
  }
  .chip-detail {
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: var(--space-2) var(--space-3);
    font-size: 12px;
    color: var(--text-muted);
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .log-head {
    display: flex;
    gap: var(--space-2);
    align-items: center;
    padding: 4px 8px;
    border-bottom: 1px solid var(--border);
  }
  .log-all.active {
    color: var(--accent);
    outline: 1px solid var(--accent);
    border-radius: var(--radius);
  }
  .pipe-log {
    flex: 1;
    overflow: auto;
    font-family: var(--mono);
    font-size: 12px;
    padding: 6px 8px;
    white-space: pre-wrap;
  }
</style>
