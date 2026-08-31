<script lang="ts">
  import type { CommitInfo } from "../api";
  import { avatarColor, initials, timeAgo } from "../format";
  import { buildGraph } from "../historyGraph";
  import { i18n, t, tn } from "../i18n.svelte";
  import {
    checkoutCommit,
    cherryPick,
    loadMoreHistory,
    revertCommit,
    runSearch,
    savePrefs,
    selectCommit,
    showInfo,
    startBisect,
    ui,
  } from "../state.svelte";
  import DiffView from "./DiffView.svelte";
  import HistoryOverview from "./HistoryOverview.svelte";
  import Icon from "./Icon.svelte";
  import Menu from "./Menu.svelte";
  import Splitter from "./Splitter.svelte";
  import VirtualList from "./VirtualList.svelte";

  const ROW_H = 46;
  const LANE_W = 12;
  // Cap of the visible lanes. Generous so a history sidebar dragged wider
  // (splitter) also shows more graph lanes instead of clamping them.
  const MAX_LANES = 24;

  const shownCommits = $derived(ui.searchResults ?? ui.history);
  const searching = $derived(ui.searchResults !== null);

  // ---- Commit graph: lane assignment (only meaningful without an active search) ----
  // The layout logic lives as a pure, tested function in historyGraph.ts.
  const graph = $derived(searching ? [] : buildGraph(ui.history));

  const graphWidth = $derived(
    Math.min(
      MAX_LANES,
      Math.max(1, ...graph.map((r) => Math.max(r.before.length, r.after.length))),
    ) *
      LANE_W +
      4,
  );

  const laneColor = (i: number) => `var(--graph-${(i % 8) + 1})`;
  const laneX = (i: number) => i * LANE_W + LANE_W / 2;

  async function copyId(id: string) {
    try {
      await navigator.clipboard.writeText(id);
      showInfo(t("history.idCopied", { id: id.slice(0, 12) }));
    } catch {
      showInfo(id);
    }
  }

  // ---- Ref labels: branch tips (local/remote/HEAD) per commit ----
  interface RefLabel {
    name: string;
    kind: "head" | "local" | "remote";
  }

  const branchLabels = $derived.by(() => {
    // A pure intermediate result of a $derived — reactivity comes from
    // `ui.branches`, the map itself is never mutated after it is built.
    // eslint-disable-next-line svelte/prefer-svelte-reactivity
    const map = new Map<string, RefLabel[]>();
    for (const b of ui.branches) {
      if (!b.targetId) continue;
      const list = map.get(b.targetId) ?? [];
      list.push({ name: b.name, kind: b.isHead ? "head" : b.isRemote ? "remote" : "local" });
      map.set(b.targetId, list);
    }
    // HEAD first, then local, then remote.
    const rank = { head: 0, local: 1, remote: 2 };
    for (const list of map.values()) list.sort((a, b) => rank[a.kind] - rank[b.kind]);
    return map;
  });

  // ---- Date groups: a header row whenever the calendar day changes ----
  type HRow =
    | { t: "date"; key: string; label: string; lanes: (string | null)[] }
    | { t: "commit"; key: string; commit: CommitInfo; idx: number; showAuthor: boolean };

  function dayLabel(unixSeconds: number): string {
    const d = new Date(unixSeconds * 1000);
    const startOfDay = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
    const diffDays = Math.round((startOfDay(new Date()) - startOfDay(d)) / 86_400_000);
    if (diffDays <= 0) return t("history.today");
    if (diffDays === 1) return t("history.yesterday");
    return d.toLocaleDateString(i18n.lang === "de" ? "de-DE" : "en-US", {
      day: "numeric",
      month: "long",
      year: "numeric",
    });
  }

  const listRows = $derived.by<HRow[]>(() => {
    const rows: HRow[] = [];
    let lastLabel = "";
    // Show the author only on a change (quieter rows); show it again after every
    // date row so a group never starts without context.
    let lastAuthor: string | null = null;
    shownCommits.forEach((commit, idx) => {
      const label = dayLabel(commit.time);
      if (label !== lastLabel) {
        lastLabel = label;
        lastAuthor = null;
        rows.push({
          t: "date",
          key: `d:${rows.length}:${label}`,
          label,
          // Draw the lanes of the next commit through so the graph does not tear.
          lanes: searching ? [] : (graph[idx]?.before ?? []),
        });
      }
      rows.push({
        t: "commit",
        key: commit.id,
        commit,
        idx,
        showAuthor: commit.authorName !== lastAuthor,
      });
      lastAuthor = commit.authorName;
    });
    return rows;
  });

  function tagsFor(commit: CommitInfo): string[] {
    return ui.tags.filter((t) => t.targetId === commit.id).map((t) => t.name);
  }

  const headId = $derived(ui.branches.find((b) => b.isHead)?.targetId ?? null);

  // Squash/interactive rebase expect the contiguous first-parent chain of the
  // CURRENT branch at the start of the list. In the all-refs history, though,
  // the topmost n rows are not automatically "the last n commits of HEAD" —
  // foreign branch tips can sit in between. Hence: anchor on HEAD + an unbroken
  // parent chain + no merges, otherwise no offer.
  function isLinearHeadRange(idx: number): boolean {
    if (!headId || ui.history[0]?.id !== headId) return false;
    for (let k = 0; k <= idx; k++) {
      const c = ui.history[k];
      if (!c || c.parentIds.length > 1) return false;
      if (k < idx && c.parentIds[0] !== ui.history[k + 1]?.id) return false;
    }
    return true;
  }

  // Infinite scroll: when the viewport approaches the end of the list, load the
  // next page (guards against search/a running load/the end).
  function maybeLoadMore() {
    if (!searching && !ui.historyComplete && !ui.historyLoading) loadMoreHistory();
  }
</script>

<div class="split">
  <aside class="side" style:width="{ui.historyPanelWidth}px">
    <div class="search">
      <div class="field">
        <Icon name="search" size={13} />
        <input
          type="text"
          placeholder={t("history.searchPlaceholder")}
          bind:value={ui.searchQuery}
          onkeydown={(e) => e.key === "Enter" && runSearch()}
          oninput={() => {
            if (ui.searchQuery.trim() === "") ui.searchResults = null;
          }}
        />
        {#if searching}
          <button
            class="ghost clear"
            onclick={() => {
              ui.searchQuery = "";
              ui.searchResults = null;
            }}
            title={t("history.clearSearch")}
          >
            <Icon name="x" size={13} />
          </button>
        {/if}
      </div>
    </div>

    {#if ui.historyPreparing}
      <!-- A fresh huge clone without a commit graph: the first page can
           take longer once. A delayed fade-in (animation-delay)
           prevents a flash on small repos. -->
      <div class="prep-hint" role="status">
        <span class="spin"></span>
        {t("history.preparing")}
      </div>
    {/if}

    {#if searching}
      <div class="search-info">
        {tn("history.matches", shownCommits.length)}
      </div>
    {/if}

    {#snippet commitRow(r: HRow)}
      {#if r.t === "date"}
        <div class="date-row">
          {#if !searching && r.lanes.length > 0}
            <svg class="lanes" width={graphWidth} height={ROW_H} aria-hidden="true">
              {#each r.lanes.slice(0, MAX_LANES) as lane, i (i)}
                {#if lane !== null}
                  <line
                    x1={laneX(i)}
                    y1="0"
                    x2={laneX(i)}
                    y2={ROW_H}
                    stroke={laneColor(i)}
                    stroke-width="2"
                  />
                {/if}
              {/each}
            </svg>
          {/if}
          <span class="date-label">{r.label}</span>
          <span class="date-line"></span>
        </div>
      {:else}
        {@const commit = r.commit}
        {@const idx = r.idx}
        {@const row = searching ? null : graph[idx]}
        {@const commitTags = tagsFor(commit)}
        <div
          class="commit"
          class:selected={ui.selectedCommit?.id === commit.id}
          style:height="{ROW_H}px"
        >
          {#if row}
            <svg class="lanes" width={graphWidth} height={ROW_H} aria-hidden="true">
              <!-- Lanes passing through -->
              {#each row.before.slice(0, MAX_LANES) as expected, i (i)}
                {#if expected !== null && expected !== commit.id && row.after[i] === expected}
                  <line
                    x1={laneX(i)}
                    y1="0"
                    x2={laneX(i)}
                    y2={ROW_H}
                    stroke={laneColor(i)}
                    stroke-width="2"
                  />
                {:else if expected === commit.id && i !== row.lane}
                  <!-- Edge merging into the commit dot -->
                  <path
                    d="M {laneX(i)} 0 C {laneX(i)} {ROW_H / 2}, {laneX(row.lane)} {ROW_H *
                      0.2}, {laneX(row.lane)} {ROW_H / 2}"
                    fill="none"
                    stroke={laneColor(i)}
                    stroke-width="2"
                  />
                {/if}
              {/each}
              {#if row.before[row.lane] === commit.id || row.before[row.lane] === undefined || row.before[row.lane] === null}
                {#if row.before[row.lane] === commit.id}
                  <line
                    x1={laneX(row.lane)}
                    y1="0"
                    x2={laneX(row.lane)}
                    y2={ROW_H / 2}
                    stroke={laneColor(row.lane)}
                    stroke-width="2"
                  />
                {/if}
              {/if}
              <!-- Edges to the parents -->
              {#each commit.parentIds as p (p)}
                {@const j = row.after.indexOf(p)}
                {#if j !== -1 && j < MAX_LANES}
                  {#if j === row.lane}
                    <line
                      x1={laneX(row.lane)}
                      y1={ROW_H / 2}
                      x2={laneX(row.lane)}
                      y2={ROW_H}
                      stroke={laneColor(row.lane)}
                      stroke-width="2"
                    />
                  {:else}
                    <path
                      d="M {laneX(row.lane)} {ROW_H / 2} C {laneX(row.lane)} {ROW_H * 0.8}, {laneX(
                        j,
                      )} {ROW_H / 2}, {laneX(j)} {ROW_H}"
                      fill="none"
                      stroke={laneColor(j)}
                      stroke-width="2"
                    />
                  {/if}
                {/if}
              {/each}
              <!-- Commit dot (clamped to the edge for lanes >= MAX_LANES,
                   so it never becomes invisible) -->
              <circle
                cx={laneX(Math.min(row.lane, MAX_LANES - 1))}
                cy={ROW_H / 2}
                r={commit.parentIds.length > 1 ? 5 : 4}
                fill={laneColor(Math.min(row.lane, MAX_LANES - 1))}
              />
            </svg>
          {/if}

          <button class="body ghost" onclick={() => selectCommit(commit)}>
            {#if r.showAuthor}
              <span class="avatar" style:background={avatarColor(commit.authorName)}>
                {initials(commit.authorName)}
              </span>
            {:else}
              <!-- A placeholder holds the column when the author repeats. -->
              <span class="avatar-spacer" aria-hidden="true"></span>
            {/if}
            <span class="text">
              <span class="summary">
                <!-- Its own span so a long subject truncates ITSELF and
                     does not push the ref chips out of the row. -->
                <span class="subject">{commit.summary || t("history.noTitle")}</span>
                {#each (branchLabels.get(commit.id) ?? []).slice(0, 3) as bl (bl.kind + ":" + bl.name)}
                  <span class="ref-badge {bl.kind}" title={bl.name}>
                    <Icon name="branch" size={10} />{bl.name}
                  </span>
                {/each}
                {#if (branchLabels.get(commit.id) ?? []).length > 3}
                  <span class="ref-badge overflow"
                    >+{(branchLabels.get(commit.id) ?? []).length - 3}</span
                  >
                {/if}
                {#each commitTags as t (t)}
                  <span class="tag-badge"><Icon name="tag" size={10} />{t}</span>
                {/each}
              </span>
              <span class="meta">
                {#if r.showAuthor}
                  {commit.authorName}
                  <span class="dot">·</span>
                {/if}
                {timeAgo(commit.time)}
                {#if commit.parentIds.length > 1}
                  <span class="badge">{t("history.mergeBadge")}</span>
                {/if}
              </span>
            </span>
            <!-- Short id quietly at the right edge: mono, without a chip. -->
            <span class="sha">{commit.shortId}</span>
          </button>

          <Menu align="right" width="280px">
            {#snippet trigger({ toggle })}
              <button class="ghost ctx" aria-label={t("history.commitActions")} onclick={toggle}>
                <Icon name="more" size={14} />
              </button>
            {/snippet}
            <button class="item ghost" role="menuitem" onclick={() => cherryPick(commit.id)}>
              <Icon name="commit" size={14} />
              {t("history.cherryPickCurrent")}
            </button>
            <button
              class="item ghost"
              role="menuitem"
              onclick={() => (ui.modal = { kind: "cherryPickTo", commitId: commit.id })}
            >
              <Icon name="branch" size={14} />
              {t("history.cherryPickOther")}
            </button>
            <button class="item ghost" role="menuitem" onclick={() => revertCommit(commit.id)}>
              <Icon name="undo" size={14} />
              {t("history.revertCommit")}
            </button>
            <div class="sep-h" role="separator"></div>
            <button
              class="item ghost"
              role="menuitem"
              onclick={() => (ui.modal = { kind: "branchFrom", commitId: commit.id })}
            >
              <Icon name="branch" size={14} />
              {t("history.branchFromHere")}
            </button>
            <button
              class="item ghost"
              role="menuitem"
              onclick={() => (ui.modal = { kind: "tagAt", commitId: commit.id })}
            >
              <Icon name="tag" size={14} />
              {t("history.tagHere")}
            </button>
            <button class="item ghost" role="menuitem" onclick={() => checkoutCommit(commit.id)}>
              <Icon name="history" size={14} />
              {t("history.checkoutDetached")}
            </button>
            {#if ui.status?.opState !== "bisect"}
              <div class="sep-h" role="separator"></div>
              <button class="item ghost" role="menuitem" onclick={() => startBisect(commit.id)}>
                <Icon name="search" size={14} />
                {t("bisect.startHere")}
              </button>
            {/if}
            {#if !searching && idx > 0 && isLinearHeadRange(idx) && commit.parentIds.length > 0}
              <div class="sep-h" role="separator"></div>
              <button
                class="item ghost"
                role="menuitem"
                onclick={() => (ui.modal = { kind: "squash", count: idx + 1, oldestId: commit.id })}
              >
                <Icon name="merge" size={14} />
                {t("history.squashLast", { n: idx + 1 })}
              </button>
              <button
                class="item ghost"
                role="menuitem"
                onclick={() =>
                  (ui.modal = {
                    kind: "rebase",
                    baseId: commit.parentIds[0],
                    commits: shownCommits.slice(0, idx + 1),
                  })}
              >
                <Icon name="history" size={14} />
                {t("history.interactiveRebase")}
              </button>
            {/if}
            <div class="sep-h" role="separator"></div>
            <button class="item ghost" role="menuitem" onclick={() => copyId(commit.id)}>
              <Icon name="copy" size={14} />
              {t("history.copyCommitId")}
            </button>
          </Menu>
        </div>
      {/if}
    {/snippet}

    {#snippet moreFooter()}
      {#if !searching && !ui.historyComplete && ui.history.length > 0}
        <button class="more" disabled={ui.historyLoading} onclick={() => loadMoreHistory()}>
          {#if ui.historyLoading}<span class="spin"></span>{/if}
          {t("history.loadMore")}
        </button>
      {/if}
    {/snippet}

    <div class="commits">
      {#if shownCommits.length === 0}
        <div class="empty" class:idle={!ui.historyLoading}>
          {#if ui.historyLoading}
            <span class="spin"></span> {t("history.loading")}
          {:else}
            <Icon name="history" size={26} strokeWidth={1.2} />
            {searching ? t("history.noMatches") : t("history.noCommits")}
          {/if}
        </div>
      {:else}
        <VirtualList
          items={listRows}
          rowHeight={ROW_H}
          getKey={(r) => r.key}
          row={commitRow}
          footer={moreFooter}
          onnearend={maybeLoadMore}
        />
      {/if}
    </div>
  </aside>

  <Splitter
    value={ui.historyPanelWidth}
    min={320}
    max={640}
    onresize={(w) => (ui.historyPanelWidth = w)}
    ondone={savePrefs}
  />

  <div class="main">
    {#if ui.selectedCommit}
      <div class="commit-head">
        <span class="avatar" style:background={avatarColor(ui.selectedCommit.authorName)}>
          {initials(ui.selectedCommit.authorName)}
        </span>
        <div class="head-text">
          <strong class="sel-summary">{ui.selectedCommit.summary}</strong>
          <span class="meta">
            {ui.selectedCommit.authorName} &lt;{ui.selectedCommit.authorEmail}&gt; · {timeAgo(
              ui.selectedCommit.time,
            )} · <code>{ui.selectedCommit.shortId}</code>
          </span>
        </div>
        <button
          class="ghost"
          title={ui.diffMode === "unified" ? t("history.viewSplit") : t("history.viewUnified")}
          onclick={() => {
            ui.diffMode = ui.diffMode === "unified" ? "split" : "unified";
            savePrefs();
          }}
        >
          <Icon name="split" size={14} />
        </button>
      </div>
    {/if}
    {#if ui.selectedCommit}
      <div class="diff-holder">
        {#if ui.commitDiffTotal !== null && ui.commitDiffTotal > ui.commitDiff.length}
          <div class="cap-note">
            {t("history.largeCommitCapped", {
              shown: ui.commitDiff.length,
              total: ui.commitDiffTotal,
            })}
          </div>
        {/if}
        <DiffView
          diffs={ui.commitDiff}
          loading={ui.commitDiff.length === 0}
          emptyText={t("history.selectCommitHint")}
        />
      </div>
    {:else if ui.history.length > 0}
      <!-- Without a selection: a large repository overview instead of empty space
           (user decision). -->
      <HistoryOverview onselect={selectCommit} />
    {:else}
      <div class="diff-holder">
        <DiffView diffs={[]} loading={false} emptyText={t("history.selectCommitHint")} />
      </div>
    {/if}
  </div>
</div>

<style>
  .split {
    display: flex;
    height: 100%;
    min-height: 0;
  }

  .side {
    /* The width comes from ui.historyPanelWidth (splitter); do not flex. */
    flex: none;
    border-right: 1px solid var(--border);
    background: var(--bg-panel);
    min-height: 0;
    display: flex;
    flex-direction: column;
  }

  .search {
    padding: var(--space-2);
    border-bottom: 1px solid var(--border);
  }

  /* Framed search field — the same pattern as the file filter of the changes list. */
  .field {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding-left: var(--space-2);
    background: var(--bg-inset);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    color: var(--text-faint);
    transition:
      border-color 0.12s ease,
      box-shadow 0.12s ease;
  }

  .field:focus-within {
    border-color: var(--accent-dim);
    box-shadow: 0 0 0 3px var(--focus-ring);
  }

  .field input {
    background: transparent;
    border: none;
    box-shadow: none;
    min-height: 26px;
    padding-left: 0;
  }

  .field .clear {
    margin-right: 2px;
    padding: 2px 5px;
    min-height: 22px;
  }

  .search-info {
    padding: var(--space-1) var(--space-3);
    color: var(--text-muted);
    font-size: 12px;
  }

  /* Preparing hint: subtly accent-tinted; appears only after 0.8 s
     (animation-delay) so it does not flash on small repos. */
  .prep-hint {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin: 0 var(--space-2) var(--space-1);
    padding: 6px 10px;
    font-size: 12px;
    color: var(--text-muted);
    background: color-mix(in srgb, var(--accent) 8%, transparent);
    border: 1px solid color-mix(in srgb, var(--accent) 25%, var(--border));
    border-radius: var(--radius);
    animation: prep-hint-in 0.2s ease 0.8s backwards;
  }

  @keyframes prep-hint-in {
    from {
      opacity: 0;
    }
  }

  .commits {
    /* The VirtualList handles scrolling (window-based rendering). */
    flex: 1;
    min-height: 0;
    padding: 0 var(--space-2);
  }

  .commits :global(.viewport) {
    padding: var(--space-2) 0;
  }

  .commit {
    display: flex;
    align-items: stretch;
    border-radius: var(--radius);
    position: relative;
  }

  .commit:hover {
    background: var(--bg-hover);
  }

  .commit.selected {
    background: var(--bg-selected);
  }

  .lanes {
    flex-shrink: 0;
    display: block;
  }

  .body {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: flex-start;
    gap: var(--space-2);
    text-align: left;
    overflow: hidden;
    padding: 4px var(--space-2);
    border-radius: 0;
  }

  .body:hover {
    background: transparent;
  }

  .text {
    flex: 1;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    line-height: 1.35;
  }

  .avatar-spacer {
    flex: none;
    width: 22px;
    height: 22px;
  }

  /* Short id quietly at the right edge — mono, without a chip. */
  .sha {
    flex: none;
    font-family: var(--mono);
    font-size: 10.5px;
    color: var(--text-faint);
  }

  .summary {
    color: var(--text-primary);
    font-weight: 550;
    overflow: hidden;
    white-space: nowrap;
    display: flex;
    align-items: center;
    gap: 6px;
  }

  /* The subject truncates itself — the ref/tag chips behind it stay visible. */
  .subject {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  /* Date group row */
  .date-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    height: 100%;
  }

  .date-row .lanes {
    flex-shrink: 0;
  }

  .date-label {
    font-size: 11px;
    font-weight: 650;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--text-faint);
    white-space: nowrap;
    padding-left: var(--space-1);
  }

  .date-line {
    flex: 1;
    height: 1px;
    background: var(--border);
    margin-right: var(--space-2);
  }

  /* Branch/HEAD labels on the commit */
  .ref-badge {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    font-size: 10.5px;
    font-weight: 600;
    border-radius: 999px;
    padding: 0 6px;
    flex-shrink: 0;
    max-width: 160px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .ref-badge.head {
    background: var(--accent-dim);
    color: var(--accent-text);
  }

  .ref-badge.local {
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    border: 1px solid color-mix(in srgb, var(--accent) 40%, transparent);
  }

  .ref-badge.remote {
    color: var(--blue);
    background: color-mix(in srgb, var(--blue) 10%, transparent);
    border: 1px solid color-mix(in srgb, var(--blue) 35%, transparent);
  }

  .ref-badge.overflow {
    color: var(--text-muted);
    border: 1px solid var(--border);
  }

  .tag-badge {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    font-size: 10.5px;
    font-weight: 600;
    /* Its own ochre token instead of --modified: the diff semantic colours stay
       untouched. */
    color: var(--ref-tag);
    background: color-mix(in srgb, var(--ref-tag) 12%, transparent);
    border: 1px solid color-mix(in srgb, var(--ref-tag) 40%, transparent);
    border-radius: 999px;
    padding: 0 6px;
    flex-shrink: 0;
  }

  .meta {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--text-muted);
    font-size: 11.5px;
    font-variant-numeric: tabular-nums;
  }

  .meta code {
    font-family: var(--mono);
    font-size: 10.5px;
    background: var(--bg-inset);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: 0 4px;
    line-height: 1.5;
  }

  .dot {
    opacity: 0.5;
  }

  .ctx {
    opacity: 0;
    align-self: center;
    padding: 4px 6px;
  }

  .commit:hover .ctx,
  .commit:focus-within .ctx {
    opacity: 1;
  }

  .commit :global(.menu-root) {
    align-self: center;
  }

  .item {
    width: 100%;
    justify-content: flex-start;
    text-align: left;
    padding: 6px 10px;
    color: var(--text-primary);
  }

  .sep-h {
    height: 1px;
    background: var(--border);
    margin: var(--space-1) 0;
  }

  .empty {
    color: var(--text-muted);
    padding: var(--space-4);
    text-align: center;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    height: 100%;
  }

  .empty.idle {
    flex-direction: column;
    gap: var(--space-3);
    color: var(--text-faint);
  }

  .empty.idle :global(svg) {
    opacity: 0.55;
  }

  .more {
    margin: var(--space-2);
  }

  .main {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
  }

  .commit-head {
    padding: var(--space-2) var(--space-4);
    border-bottom: 1px solid var(--border);
    display: flex;
    align-items: center;
    gap: var(--space-3);
    background: var(--bg-panel);
  }

  .head-text {
    display: flex;
    flex-direction: column;
    gap: 1px;
    flex: 1;
    overflow: hidden;
  }

  .sel-summary {
    -webkit-user-select: text;
    user-select: text;
  }

  .commit-head .meta {
    -webkit-user-select: text;
    user-select: text;
  }

  .diff-holder {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }

  .diff-holder :global(.diff-wrap) {
    flex: 1;
    min-height: 0;
  }

  .cap-note {
    padding: var(--space-1) var(--space-3);
    font-size: 12px;
    color: var(--modified);
    background: color-mix(in srgb, var(--modified) 10%, transparent);
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }
</style>
