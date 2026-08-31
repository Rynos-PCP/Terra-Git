<script lang="ts">
  import { slide } from "svelte/transition";
  import type { UnpushedCommit } from "../api";
  import { t } from "../i18n.svelte";
  import {
    applyWorkshop,
    cancelWorkshop,
    openWorkshop,
    refreshUnpushed,
    ui,
    uncommitTop,
  } from "../state.svelte";
  import {
    authorValid,
    changedCommitCount,
    commitChanged,
    firstKeptIsSquash,
    workshopOrderChanged,
    type WorkshopEdit,
  } from "../workshopSteps";
  import { timeAgo } from "../format";
  import { tooltip } from "../tooltip";
  import Icon from "./Icon.svelte";

  const hasMerge = $derived(ui.unpushed.some((c) => c.isMerge));
  const pending = $derived(changedCommitCount(ui.unpushed, ui.workshopEdits));
  const oldestIsRoot = $derived(
    ui.unpushed.length > 0 && ui.unpushed[ui.unpushed.length - 1].parentIds.length === 0,
  );

  function isRoot(id: string): boolean {
    return oldestIsRoot && ui.unpushed[ui.unpushed.length - 1].id === id;
  }

  /** The author was changed AND is invalid (empty / angle brackets). */
  function badAuthor(c: UnpushedCommit, e: WorkshopEdit | undefined): boolean {
    if (!e || e.dropped) return false;
    const changedAuthor = e.authorName !== c.authorName || e.authorEmail !== c.authorEmail;
    return changedAuthor && !authorValid(e.authorName, e.authorEmail);
  }

  const invalidAuthor = $derived(ui.unpushed.some((c) => badAuthor(c, ui.workshopEdits[c.id])));

  // Display order from the state (natural as a fallback while it is not loaded
  // yet); the root stays pinned to the end of the list by the move() guard.
  const ordered = $derived.by(() => {
    if (ui.workshopOrder.length !== ui.unpushed.length) return ui.unpushed;
    const byId = new Map(ui.unpushed.map((c) => [c.id, c]));
    const list = ui.workshopOrder.map((id) => byId.get(id)).filter((c) => c !== undefined);
    return list.length === ui.unpushed.length ? list : ui.unpushed;
  });
  const orderMoved = $derived(workshopOrderChanged(ui.unpushed, ui.workshopOrder));
  const squashFirst = $derived(firstKeptIsSquash(ui.unpushed, ui.workshopEdits, ui.workshopOrder));

  /** Swaps the commit with its display neighbour; the root stays at the bottom. */
  function move(id: string, dir: -1 | 1) {
    const arr = [...ui.workshopOrder];
    const i = arr.indexOf(id);
    const j = i + dir;
    if (i < 0 || j < 0 || j >= arr.length) return;
    if (isRoot(id) || isRoot(arr[j])) return;
    [arr[i], arr[j]] = [arr[j], arr[i]];
    ui.workshopOrder = arr;
  }

  /** A squash needs an older (non-root) commit below it. */
  function canSquashAt(i: number): boolean {
    const below = ordered[i + 1];
    return !!below && !isRoot(below.id);
  }

  // Expanded cards: only explicit user toggles land in the buffer; HEAD is open
  // by derivation (the most common case: correcting the last message). No
  // $effect — that way no expand animation plays while the page is being built.
  let expanded = $state<Record<string, boolean>>({});

  function isOpen(c: UnpushedCommit): boolean {
    return expanded[c.id] ?? (c.isHead && !c.isMerge);
  }

  function toggle(c: UnpushedCommit) {
    expanded[c.id] = !isOpen(c);
  }

  function toggleDrop(e: WorkshopEdit, id: string) {
    e.dropped = !e.dropped;
    if (e.dropped) expanded[id] = false;
  }
</script>

<div class="workshop">
  <header>
    <button class="ghost back" onclick={cancelWorkshop} use:tooltip={t("settings.back")}>
      <span class="back-icon"><Icon name="chevronDown" size={14} /></span>
      {t("settings.back")}
    </button>
    <h1>{t("workshop.title")}</h1>
    {#if ui.unpushed.length > 0}
      <span class="badge">{t("workshop.count", { n: String(ui.unpushed.length) })}</span>
    {/if}
    <span class="spacer"></span>
    <button class="ghost" onclick={refreshUnpushed} use:tooltip={t("workshop.refresh")}>
      <Icon name="refresh" size={14} />
    </button>
  </header>

  {#if ui.workshopError}
    <div class="empty">
      <span class="empty-icon err"><Icon name="alert" size={24} /></span>
      <p class="empty-title">{t("workshop.loadError")}</p>
      <button onclick={openWorkshop}>{t("workshop.retry")}</button>
    </div>
  {:else if ui.unpushed.length === 0}
    <div class="empty">
      <span class="empty-icon"><Icon name="check" size={24} /></span>
      <p class="empty-title">{t("workshop.empty")}</p>
      <p class="empty-hint">{t("workshop.emptyHint")}</p>
    </div>
  {:else}
    <p class="hint">{t("workshop.subtitle")}</p>
    {#if hasMerge}
      <p class="banner warn"><Icon name="alert" size={14} />{t("workshop.mergeBlocked")}</p>
    {/if}

    <!-- Strata rail: unpushed commits as loose layers above the
         bedrock (= the already pushed state, at the bottom as a plinth). -->
    <ul class="commits" class:no-base={oldestIsRoot}>
      {#each ordered as c, i (c.id)}
        {@const e = ui.workshopEdits[c.id]}
        {@const ro = c.isMerge || isRoot(c.id)}
        {@const bad = badAuthor(c, e)}
        {@const edited = commitChanged(c, e)}
        {@const open = isOpen(c)}
        <li class:dropped={e?.dropped} class:readonly={ro} class:invalid={bad}>
          <span
            class="node"
            class:n-changed={edited && !e?.dropped && !e?.squashed}
            class:n-squash={e?.squashed && !e?.dropped}
            class:n-dropped={e?.dropped}
            class:n-ro={ro}
          ></span>
          <div class="row1">
            <button class="expander" aria-expanded={open} onclick={() => toggle(c)}>
              <span class="chev" class:down={open}><Icon name="chevronDown" size={12} /></span>
              <code class="sha">{c.id.slice(0, 8)}</code>
              <span class="subject">{e?.subject || c.subject}</span>
              {#if c.isHead}<span class="tag">{t("workshop.head")}</span>{/if}
              {#if isRoot(c.id)}
                <span class="tag" use:tooltip={t("workshop.rootReadonly")}
                  >{t("workshop.root")}</span
                >
              {/if}
              {#if e?.dropped}
                <span class="tag drop-tag">{t("workshop.dropped")}</span>
              {:else if e?.squashed}
                <span class="tag squash-tag">{t("workshop.squashed")}</span>
              {:else if edited && !ro}
                <span class="tag changed-tag">{t("workshop.changed")}</span>
              {/if}
              <span class="when">{timeAgo(c.time)}</span>
            </button>
            {#if !ro && ordered.length > 1}
              <button
                class="ghost iconbtn"
                onclick={() => move(c.id, -1)}
                disabled={i === 0}
                use:tooltip={t("rebase.moveUp")}
              >
                <Icon name="arrowUp" size={13} />
              </button>
              <button
                class="ghost iconbtn"
                onclick={() => move(c.id, 1)}
                disabled={i === ordered.length - 1 || isRoot(ordered[i + 1].id)}
                use:tooltip={t("rebase.moveDown")}
              >
                <Icon name="arrowDown" size={13} />
              </button>
            {/if}
            {#if !ro && e}
              <button
                class="ghost iconbtn"
                class:squash-on={e.squashed}
                onclick={() => (e.squashed = !e.squashed)}
                disabled={e.dropped || (!e.squashed && !canSquashAt(i))}
                use:tooltip={e.squashed ? t("workshop.unsquash") : t("workshop.squash")}
              >
                <Icon name="merge" size={14} />
              </button>
              <button
                class="ghost iconbtn"
                onclick={() => toggleDrop(e, c.id)}
                use:tooltip={e.dropped ? t("workshop.restore") : t("workshop.drop")}
              >
                <Icon name={e.dropped ? "undo" : "trash"} size={14} />
              </button>
            {:else if ro}
              <span
                class="lock"
                use:tooltip={c.isMerge ? t("workshop.mergeReadonly") : t("workshop.rootReadonly")}
              >
                <Icon name="lock" size={13} />
              </span>
            {/if}
          </div>
          {#if open && e}
            <div class="form" transition:slide={{ duration: 140 }}>
              <label
                >{t("workshop.subject")}
                <input
                  type="text"
                  class="subject-input"
                  bind:value={e.subject}
                  disabled={ro || e.dropped || e.squashed}
                />
              </label>
              <label
                >{t("workshop.body")}
                <textarea rows="3" bind:value={e.body} disabled={ro || e.dropped || e.squashed}
                ></textarea>
              </label>
              <label
                >{t("workshop.coAuthors")}
                <input
                  type="text"
                  bind:value={e.coAuthors}
                  disabled={ro || e.dropped || e.squashed}
                />
              </label>
              <div class="author">
                <label
                  >{t("workshop.authorName")}
                  <input
                    type="text"
                    bind:value={e.authorName}
                    disabled={ro || e.dropped || e.squashed}
                  />
                </label>
                <label
                  >{t("workshop.authorEmail")}
                  <input
                    type="text"
                    bind:value={e.authorEmail}
                    disabled={ro || e.dropped || e.squashed}
                  />
                </label>
              </div>
              {#if bad}<p class="field-error">{t("workshop.authorInvalid")}</p>{/if}
              {#if c.isHead && !c.isMerge}
                <div class="actions">
                  <button class="ghost" onclick={uncommitTop} disabled={!!ui.busy}>
                    <Icon name="undo" size={14} />
                    {t("workshop.uncommit")}
                  </button>
                </div>
              {/if}
            </div>
          {/if}
        </li>
      {/each}
    </ul>

    {#if !oldestIsRoot}
      <div class="bedrock">
        <span class="rock"></span>
        <span class="base-label">
          {#if ui.status?.upstream}<code>{ui.status.upstream}</code><span aria-hidden="true">·</span
            >{/if}
          <span>{t("workshop.pushedBase")}</span>
        </span>
      </div>
    {/if}

    {#if invalidAuthor}
      <p class="banner error"><Icon name="alert" size={14} />{t("workshop.authorInvalid")}</p>
    {/if}
    {#if squashFirst}
      <p class="banner error"><Icon name="alert" size={14} />{t("rebase.warnFirstSquash")}</p>
    {/if}

    <footer>
      <span class="pending">{t("workshop.pending", { n: String(pending) })}</span>
      {#if orderMoved}<span class="tag order-tag">{t("workshop.orderChanged")}</span>{/if}
      <span class="safety"><Icon name="shield" size={13} />{t("workshop.backupNote")}</span>
      <button
        class="ghost"
        onclick={openWorkshop}
        disabled={!!ui.busy || (pending === 0 && !orderMoved)}
      >
        {t("workshop.reset")}
      </button>
      <button
        class="primary"
        onclick={applyWorkshop}
        disabled={!!ui.busy ||
          (pending === 0 && !orderMoved) ||
          hasMerge ||
          invalidAuthor ||
          squashFirst}
      >
        {#if ui.busy}<span class="spin"></span>{/if}
        {t("workshop.apply")}
      </button>
    </footer>
  {/if}
</div>

<style>
  .workshop {
    display: flex;
    flex-direction: column;
    height: 100%;
    padding: 0 var(--space-4) var(--space-3);
    gap: var(--space-2);
    overflow: auto;
  }

  /* Limit the page content to a readable width and centre it. */
  header,
  .hint,
  .banner,
  .commits,
  .bedrock,
  footer,
  .empty {
    width: 100%;
    max-width: 780px;
    margin-left: auto;
    margin-right: auto;
  }

  header {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    position: sticky;
    top: 0;
    z-index: 2;
    background: var(--bg-app);
    padding: var(--space-3) 0 var(--space-1);
  }
  header h1 {
    font-family: var(--display);
    font-size: 16px;
    font-weight: 650;
  }
  header .spacer {
    flex: 1;
  }
  .back-icon {
    display: inline-flex;
    transform: rotate(90deg);
  }

  .hint {
    color: var(--text-muted);
    font-size: 12px;
  }

  .banner {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 6px 10px;
    border-radius: var(--radius);
    font-size: 12px;
    border: 1px solid;
  }
  .banner.warn {
    color: var(--warn);
    border-color: color-mix(in srgb, var(--warn) 45%, var(--border));
    background: color-mix(in srgb, var(--warn) 10%, transparent);
  }
  .banner.error {
    color: var(--deleted);
    border-color: color-mix(in srgb, var(--deleted) 45%, var(--border));
    background: color-mix(in srgb, var(--deleted) 8%, transparent);
  }

  /* ---------- Commit rail ---------- */
  .commits {
    list-style: none;
    position: relative;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: 2px 0 0 30px;
  }
  .commits::before {
    content: "";
    position: absolute;
    left: 13px;
    top: 16px;
    bottom: -12px;
    width: 2px;
    background: color-mix(in srgb, var(--border-strong) 55%, var(--border));
  }
  /* Root within reach: no plinth — cut the line behind the last card. */
  .commits.no-base li:last-child::before {
    content: "";
    position: absolute;
    left: -22px;
    top: 26px;
    bottom: -10px;
    width: 12px;
    background: var(--bg-app);
  }

  li {
    position: relative;
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    background: var(--bg-panel);
  }
  li.dropped {
    border-style: dashed;
  }
  li.readonly {
    opacity: 0.75;
  }
  li.invalid {
    border-color: var(--deleted);
    box-shadow: 0 0 0 1px var(--deleted);
  }

  .node {
    position: absolute;
    left: -22px;
    top: 13px;
    width: 11px;
    height: 11px;
    border-radius: 50%;
    background: var(--bg-app);
    border: 2px solid var(--border-strong);
    z-index: 1;
  }
  .node.n-changed {
    background: var(--accent);
    border-color: var(--accent);
  }
  .node.n-dropped {
    background: var(--bg-app);
    border-color: var(--deleted);
  }
  .node.n-squash {
    background: var(--blue);
    border-color: var(--blue);
  }
  .node.n-ro {
    background: var(--border-strong);
  }

  /* ---------- Card head ---------- */
  .row1 {
    display: flex;
    align-items: center;
    gap: 2px;
    padding-right: 6px;
  }
  .expander {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    background: none;
    border: none;
    box-shadow: none;
    border-radius: var(--radius-lg);
    padding: 8px 6px 8px 10px;
    text-align: left;
    color: inherit;
  }
  .expander:hover:not(:disabled) {
    background: var(--bg-hover);
    border-color: transparent;
  }
  .chev {
    display: inline-flex;
    color: var(--text-faint);
    transform: rotate(-90deg);
    transition: transform 0.12s ease;
    flex-shrink: 0;
  }
  .chev.down {
    transform: none;
  }
  .sha {
    color: var(--text-faint);
    font-family: var(--mono);
    font-size: 12px;
    flex-shrink: 0;
  }
  .subject {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: 600;
  }
  .when {
    color: var(--text-faint);
    font-size: 12px;
    flex-shrink: 0;
  }
  li.dropped .subject {
    text-decoration: line-through;
    color: var(--text-muted);
    font-weight: 400;
  }
  li.dropped .sha {
    text-decoration: line-through;
  }

  .tag {
    font-size: 11px;
    line-height: 1.5;
    padding: 0 7px;
    border-radius: 999px;
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    color: var(--text-muted);
    flex-shrink: 0;
  }
  .changed-tag {
    color: var(--accent);
    border-color: color-mix(in srgb, var(--accent) 40%, var(--border));
    background: color-mix(in srgb, var(--accent) 12%, transparent);
  }
  .drop-tag {
    color: var(--deleted);
    border-color: color-mix(in srgb, var(--deleted) 40%, var(--border));
    background: color-mix(in srgb, var(--deleted) 10%, transparent);
  }
  .squash-tag,
  .order-tag {
    color: var(--blue);
    border-color: color-mix(in srgb, var(--blue) 40%, var(--border));
    background: color-mix(in srgb, var(--blue) 10%, transparent);
  }

  .iconbtn {
    padding: 4px 6px;
    flex-shrink: 0;
  }
  .iconbtn.squash-on {
    color: var(--blue);
  }
  .lock {
    display: inline-flex;
    color: var(--text-faint);
    padding: 4px 6px;
    flex-shrink: 0;
  }

  /* ---------- Form ---------- */
  .form {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: 0 var(--space-3) var(--space-3) 30px;
  }
  label {
    display: flex;
    flex-direction: column;
    font-size: 12px;
    color: var(--text-muted);
    gap: 2px;
  }
  .subject-input {
    font-weight: 600;
  }
  .author {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-2);
  }
  .field-error {
    color: var(--deleted);
    font-size: 12px;
  }
  .actions {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    margin-top: 2px;
  }

  /* ---------- Plinth (state already pushed) ---------- */
  .bedrock {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-top: -2px;
  }
  /* Plinth chip: visible layer stripes + a verdigris vein at the
     top edge (that is where the commits "land" on push). */
  .rock {
    width: 22px;
    height: 15px;
    margin-left: 3px;
    border-radius: 2px 2px 3px 3px;
    flex-shrink: 0;
    border: 1px solid var(--border-strong);
    background:
      linear-gradient(180deg, var(--strata-vein) 0 2px, transparent 2px),
      repeating-linear-gradient(180deg, var(--bg-elevated) 0 3px, var(--border-strong) 3px 4px);
  }
  .base-label {
    display: inline-flex;
    align-items: baseline;
    gap: 5px;
    font-size: 12px;
    color: var(--text-muted);
  }
  .base-label code {
    font-family: var(--mono);
    font-size: 11px;
    color: var(--text-primary);
  }

  /* ---------- Empty/error state ---------- */
  .empty {
    margin-top: auto;
    margin-bottom: auto;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
    text-align: center;
    max-width: 360px;
  }
  .empty-icon {
    display: inline-flex;
    width: 44px;
    height: 44px;
    border-radius: 50%;
    align-items: center;
    justify-content: center;
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    border: 1px solid color-mix(in srgb, var(--accent) 30%, var(--border));
  }
  .empty-icon.err {
    color: var(--deleted);
    background: color-mix(in srgb, var(--deleted) 10%, transparent);
    border-color: color-mix(in srgb, var(--deleted) 30%, var(--border));
  }
  .empty-title {
    font-family: var(--display);
    font-size: 15px;
    font-weight: 650;
  }
  .empty-hint {
    font-size: 12px;
    color: var(--text-muted);
  }

  /* ---------- Footer ---------- */
  footer {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    position: sticky;
    bottom: 0;
    margin-top: auto;
    padding: var(--space-2) 0;
    background: var(--bg-app);
    border-top: 1px solid var(--border);
  }
  .pending {
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
    flex-shrink: 0;
  }
  .safety {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--text-faint);
    font-size: 12px;
    flex: 1;
    min-width: 0;
  }
</style>
