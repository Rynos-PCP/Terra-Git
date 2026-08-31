<script lang="ts">
  import type { CommitInfo } from "../api";
  import { timeAgo } from "../format";
  import { t, tn } from "../i18n.svelte";
  import { buildHistoryOverview } from "../historyOverview";
  import { ui } from "../state.svelte";
  import Icon from "./Icon.svelte";

  // History overview (core-sample reading after the user's
  // decision of 2026-08-14): a large VERTICAL repository graph in the empty diff
  // area — the newest commit at the top, the history sinking downwards into the
  // depths; lanes as columns with quarter-arc corners, the age scale on the left,
  // the ref/tag chips in a fixed column on the right. Deliberately a
  // supplementary VISUALIZATION (aria-hidden): the accessible route to the
  // commits stays the history list on the left.

  let { onselect }: { onselect: (commit: CommitInfo) => void } = $props();

  const model = $derived(
    buildHistoryOverview(ui.history, ui.branches, ui.tags, Math.floor(Date.now() / 1000)),
  );
  const laneColor = (i: number) => `var(--graph-${(i % 8) + 1})`;

  // Start at the top (newest commits) — loading more appends older commits at
  // the bottom, so the visible section stays stable by itself.

  function nodeTitle(idx: number): string {
    const c = ui.history[idx];
    if (!c) return "";
    return `${c.summary || t("history.noTitle")}\n${c.authorName} · ${timeAgo(c.time)} · ${c.shortId}`;
  }
</script>

<div class="overview">
  <div class="head">
    <Icon name="history" size={14} />
    <span class="title">{t("history.overviewTitle")}</span>
    <span class="count">
      {tn("history.overviewLoaded", ui.history.length)}{#if !ui.historyComplete}&nbsp;{t(
          "history.overviewPartial",
        )}{/if}
    </span>
    <span class="hint">{t("history.overviewHint")}</span>
  </div>

  <div class="scroller">
    <div class="canvas" style:width="{model.width}px" style:height="{model.height}px">
      <svg width={model.width} height={model.height} aria-hidden="true">
        <!-- Age scale on the left: depth = the past (core-sample reading). -->
        {#if model.ruler.marks.length > 0}
          <line
            class="ruler-line"
            x1={model.ruler.x}
            y1={model.ruler.y1}
            x2={model.ruler.x}
            y2={model.ruler.y2}
          />
          {#each model.ruler.marks as m (m.y)}
            <line class="ruler-tick" x1={model.ruler.x - 5} y1={m.y} x2={model.ruler.x} y2={m.y} />
            <text class="ruler-text" text-anchor="end" x={model.ruler.x - 9} y={m.y + 3}>
              {m.text}
            </text>
          {/each}
        {/if}
        {#each model.edges as e, i (i)}
          <path
            d={e.path}
            fill="none"
            stroke={laneColor(e.lane)}
            stroke-width="2"
            opacity={e.stub ? 0.4 : 1}
            stroke-dasharray={e.stub ? "3 3" : undefined}
          />
        {/each}
        {#each model.nodes as node (node.id)}
          <!-- Halo in the surface colour: lifts the node off crossing
               edges (core-sample bead). -->
          <circle class="halo" cx={node.x} cy={node.y} r={(node.isMerge ? 5.5 : 4.5) + 2.5} />
          <!-- Mouse convenience on an aria-hidden visualization: the
               keyboard-/screen-reader-capable route to the commits is the
               history list on the left (listbox), not this graph. -->
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <circle
            class="node"
            class:selected={ui.selectedCommit?.id === node.id}
            cx={node.x}
            cy={node.y}
            r={node.isMerge ? 5.5 : 4.5}
            fill={laneColor(node.lane)}
            onclick={() => onselect(ui.history[node.idx])}
          >
            <title>{nodeTitle(node.idx)}</title>
          </circle>
        {/each}
      </svg>
      {#each model.labels as label, i (i)}
        <span
          class="chip {label.kind}"
          style:left="{label.x}px"
          style:top="{label.y}px"
          title={label.name}
        >
          {#if label.kind === "tag"}
            <Icon name="tag" size={9} />
          {:else if label.kind !== "overflow"}
            <Icon name="branch" size={9} />
          {/if}
          {label.name}
        </span>
      {/each}
    </div>
  </div>
</div>

<style>
  .overview {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }

  .head {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-4);
    border-bottom: 1px solid var(--border);
    background: var(--bg-panel);
    color: var(--text-muted);
    font-size: 12px;
  }

  .title {
    color: var(--text-primary);
    font-weight: 600;
  }

  .count {
    color: var(--text-faint);
  }

  .hint {
    margin-left: auto;
    color: var(--text-faint);
  }

  .scroller {
    flex: 1;
    min-height: 0;
    overflow: auto;
    display: flex;
  }

  .canvas {
    position: relative;
    flex: none;
    /* Centred while the graph is smaller than the area; on overflow
       the auto margins collapse — the start (top) stays reachable. */
    margin: auto;
  }

  /* Age scale: the same look as the core-box ruler on the welcome screen. */
  .ruler-line {
    stroke: var(--border-strong);
    stroke-opacity: 0.35;
  }

  .ruler-tick {
    stroke: var(--border-strong);
    stroke-opacity: 0.6;
  }

  .ruler-text {
    font-family: var(--mono);
    font-size: 9px;
    fill: var(--text-faint);
  }

  .halo {
    fill: var(--bg-app);
  }

  .node {
    cursor: pointer;
  }

  .node:hover {
    stroke: var(--text-primary);
    stroke-width: 1.5;
  }

  .node.selected {
    stroke: var(--accent);
    stroke-width: 2;
  }

  /* Ref chips: the same semantic colours as in the history list. */
  .chip {
    position: absolute;
    transform: translateY(-50%);
    display: inline-flex;
    align-items: center;
    gap: 3px;
    font-size: 10px;
    font-weight: 600;
    line-height: 1;
    border-radius: 999px;
    padding: 2px 6px;
    max-width: 130px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    background: var(--bg-panel);
    pointer-events: none;
  }

  .chip.head {
    background: var(--accent-dim);
    color: var(--accent-text);
  }

  .chip.local {
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 12%, var(--bg-panel));
    border: 1px solid color-mix(in srgb, var(--accent) 40%, transparent);
  }

  .chip.remote {
    color: var(--blue);
    background: color-mix(in srgb, var(--blue) 10%, var(--bg-panel));
    border: 1px solid color-mix(in srgb, var(--blue) 35%, transparent);
  }

  .chip.tag {
    color: var(--ref-tag);
    background: color-mix(in srgb, var(--ref-tag) 12%, var(--bg-panel));
    border: 1px solid color-mix(in srgb, var(--ref-tag) 40%, transparent);
  }

  .chip.overflow {
    color: var(--text-muted);
    border: 1px solid var(--border);
  }
</style>
