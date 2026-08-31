<script lang="ts">
  // Changes overview: fills the diff area while no file is
  // selected — one row per changed file (staging dot, path, delta bar, +x/−y), a
  // totals row, a keyboard row in the footer language of the welcome screen and
  // the conventional-commits hint from the message log.
  import { t, tn } from "../i18n.svelte";
  import { refreshNumstat, ui } from "../state.svelte";
  import { buildOverview, conventionalTypes, type OverviewRow } from "../changesOverview";
  import { tooltip } from "../tooltip";
  import Icon from "./Icon.svelte";
  import VirtualList from "./VirtualList.svelte";

  let {
    onopen,
    ontoggle,
    oncommitrequest,
  }: {
    /** Click/Enter: open the file in the diff. */
    onopen: (row: OverviewRow) => void;
    /** Space: stage or unstage. */
    ontoggle: (row: OverviewRow) => void;
    /** Ctrl+Enter from the overview. */
    oncommitrequest: () => void;
  } = $props();

  const ROW_H = 26;

  const model = $derived(buildOverview(ui.status, ui.numstat));
  const ccTypes = $derived(conventionalTypes(ui.messageLog));

  // Load (or reload) the line balance whenever the status changes —
  // deliberately HERE instead of in refreshStatus, so only the visible overview
  // pays for the second full diff.
  $effect(() => {
    void ui.status;
    void refreshNumstat();
  });

  let list = $state<{ scrollIndexIntoView: (i: number) => void }>();
  let cursor = $state(0);
  // Stay in range when the list shrinks (a file committed/discarded).
  const active = $derived(Math.max(0, Math.min(cursor, model.rows.length - 1)));

  function moveTo(index: number) {
    cursor = Math.max(0, Math.min(index, model.rows.length - 1));
    list?.scrollIndexIntoView(cursor);
  }

  function onKeydown(e: KeyboardEvent) {
    if ((e.ctrlKey || e.metaKey) && e.key === "Enter") {
      e.preventDefault();
      oncommitrequest();
      return;
    }
    if (e.ctrlKey || e.metaKey || e.altKey) return;
    const row = model.rows[active];
    switch (e.key) {
      case "ArrowDown":
        e.preventDefault();
        moveTo(active + 1);
        break;
      case "ArrowUp":
        e.preventDefault();
        moveTo(active - 1);
        break;
      case "Home":
        e.preventDefault();
        moveTo(0);
        break;
      case "End":
        e.preventDefault();
        moveTo(model.rows.length - 1);
        break;
      case " ":
        if (row) {
          e.preventDefault();
          ontoggle(row);
        }
        break;
      case "Enter":
        if (row) {
          e.preventDefault();
          onopen(row);
        }
        break;
    }
  }

  const DOT_TITLE = {
    full: "changes.sectionStaged",
    partial: "overview.partial",
    none: "changes.sectionUnstaged",
  } as const;

  // Delta bar: length ~ the square root of the share of the largest file (small
  // changes stay visible), split green/red by ratio.
  const BAR_W = 44;
  function barParts(row: OverviewRow): { add: number; del: number } | null {
    const s = row.stats;
    if (!s || s.binary) return null;
    const total = s.added + s.deleted;
    if (total === 0 || model.maxTotal === 0) return { add: 0, del: 0 };
    const len = Math.max(3, Math.round(BAR_W * Math.sqrt(total / model.maxTotal)));
    const add = Math.round((len * s.added) / total);
    return { add, del: len - add };
  }
</script>

<div class="overview">
  <div class="head">
    <span class="sum">{tn("overview.summary", model.totals.files)}</span>
    <span class="delta mono">
      <span class="add">+{model.totals.added}</span>
      <span class="del">−{model.totals.deleted}</span>
    </span>
  </div>

  <div
    class="list"
    role="listbox"
    tabindex="0"
    aria-label={t("overview.aria")}
    aria-activedescendant={model.rows.length ? `ov-row-${active}` : undefined}
    onkeydown={onKeydown}
  >
    <VirtualList
      bind:this={list}
      items={model.rows}
      rowHeight={ROW_H}
      getKey={(r) => r.path}
      {row}
    />
  </div>

  <div class="foot">
    {#if ccTypes}
      <span class="cc">
        <Icon name="commit" size={13} />
        {t("overview.ccTip", { types: ccTypes.join(", ") })}
      </span>
    {/if}
    <span class="keys">
      <span class="key"><kbd>↑</kbd><kbd>↓</kbd> {t("overview.kbdNavigate")}</span>
      <span class="key"><kbd>{t("overview.keySpace")}</kbd> {t("overview.kbdToggle")}</span>
      <span class="key"><kbd>Enter</kbd> {t("overview.kbdOpen")}</span>
      <span class="key"
        ><kbd>{t("app.keyCtrl")}</kbd><kbd>Enter</kbd> {t("overview.kbdCommit")}</span
      >
      <span class="key"><kbd>{t("app.keyCtrl")}</kbd><kbd>K</kbd> {t("welcome.kbdPalette")}</span>
    </span>
  </div>
</div>

{#snippet row(r: OverviewRow, i: number)}
  <!-- svelte-ignore a11y_click_events_have_key_events (the keyboard lives on the listbox container) -->
  <div
    class="row"
    role="option"
    id="ov-row-{i}"
    tabindex={-1}
    aria-selected={i === active}
    class:active={i === active}
    onclick={() => {
      cursor = i;
      onopen(r);
    }}
  >
    <span class="dot {r.staged}" use:tooltip={t(DOT_TITLE[r.staged])}></span>
    <span class="path" use:tooltip={r.path}>{r.path}</span>
    {#if r.stats?.binary}
      <span class="binary">{t("overview.binary")}</span>
    {:else if r.stats}
      {@const bar = barParts(r)}
      {#if bar}
        <span class="bar" aria-hidden="true">
          <span class="seg add" style:width="{bar.add}px"></span>
          <span class="seg del" style:width="{bar.del}px"></span>
        </span>
      {/if}
      <span class="nums mono">
        <span class="add">+{r.stats.added}</span>
        <span class="del">−{r.stats.deleted}</span>
      </span>
    {/if}
  </div>
{/snippet}

<style>
  .overview {
    height: 100%;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }

  .head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    border-bottom: 1px solid var(--border);
  }

  .sum {
    font-weight: 650;
    font-size: 13px;
  }

  .mono {
    font-family: var(--mono);
    font-size: 12px;
  }

  .add {
    color: var(--added);
  }

  .del {
    color: var(--deleted);
  }

  .list {
    flex: 1;
    min-height: 0;
    outline: none;
  }

  .list:focus-visible {
    box-shadow: inset 0 0 0 2px var(--accent);
  }

  .row {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    height: 100%;
    padding: 0 var(--space-4);
    cursor: pointer;
  }

  .row:hover {
    background: var(--bg-hover);
  }

  .row.active {
    background: var(--bg-selected);
  }

  .dot {
    flex: none;
    width: 9px;
    height: 9px;
    border-radius: 50%;
    border: 1.5px solid var(--accent);
  }

  .dot.full {
    background: var(--accent);
  }

  .dot.partial {
    background: linear-gradient(90deg, var(--accent) 50%, transparent 50%);
  }

  .dot.none {
    border-color: var(--text-faint);
  }

  .path {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 12px;
  }

  .bar {
    flex: none;
    display: inline-flex;
    height: 6px;
    border-radius: 3px;
    overflow: hidden;
  }

  .seg.add {
    background: var(--added);
  }

  .seg.del {
    background: var(--deleted);
  }

  .binary,
  .nums {
    flex: none;
    font-size: 12px;
    color: var(--text-faint);
    min-width: 76px;
    text-align: right;
  }

  .nums .add,
  .nums .del {
    color: inherit;
  }

  .nums .add {
    color: var(--added);
  }

  .nums .del {
    color: var(--deleted);
  }

  .foot {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4);
    border-top: 1px solid var(--border);
    color: var(--text-faint);
    font-size: 11px;
  }

  .cc {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }

  .keys {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
  }

  .key {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }

  kbd {
    font-family: var(--mono);
    font-size: 10px;
    padding: 1px 5px;
    border: 1px solid var(--border-strong);
    border-bottom-width: 2px;
    border-radius: 4px;
    background: var(--bg-panel);
  }
</style>
