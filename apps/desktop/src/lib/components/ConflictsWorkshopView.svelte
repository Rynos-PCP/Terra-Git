<script lang="ts">
  import { api, type OpContext } from "../api";
  import { OP_LABELS, workshopCopy } from "../conflictWorkshop";
  import { t } from "../i18n.svelte";
  import { abortOperation, continueOperation, refreshStatus, showError, ui } from "../state.svelte";
  import ConflictResolver from "./ConflictResolver.svelte";
  import Icon from "./Icon.svelte";

  // Conflict workshop: ONE place for the running multi-step operation — names
  // both sides understandably (branch/commit instead of ours/theirs), leads
  // through the conflicts file by file and only offers continue/abort once it is
  // clear where you stand.

  let ctx = $state<OpContext | null>(null);

  const conflicted = $derived(
    (ui.status?.unstaged ?? []).filter((e) => e.kind === "conflicted").map((e) => e.path),
  );

  // Conflicted files seen once in this session: the basis for the progress
  // (resolved = no longer reported as conflicted).
  let seen = $state<string[]>([]);
  $effect(() => {
    const add = conflicted.filter((p) => !seen.includes(p));
    if (add.length) seen = [...seen, ...add];
  });
  const resolved = $derived(seen.filter((p) => !conflicted.includes(p)));

  let selected = $state<string | null>(null);
  $effect(() => {
    // Repair the selection: a resolved/vanished file -> the next open one.
    if (!selected || !conflicted.includes(selected)) selected = conflicted[0] ?? null;
  });

  // Load the operation context — freshly after every continue, because a rebase
  // then jumps to the next step (step display, a different commit).
  $effect(() => {
    const repo = ui.repo;
    const op = ui.status?.opState;
    void conflicted.length; // re-fetch the context after every resolution
    if (!repo || !op || op === "clean") return;
    api
      .opContext(repo.path)
      .then((c) => (ctx = c))
      .catch(() => (ctx = null));
  });

  // Operation finished (continued or aborted): back to the workspace.
  $effect(() => {
    if (ui.status && ui.status.opState === "clean") ui.view = "repo";
  });

  const copy = $derived(workshopCopy(ctx));
  const oursHead = $derived(t(copy.ours.key, copy.ours.params));
  const theirsHead = $derived(t(copy.theirs.key, copy.theirs.params));
  const opLabel = $derived(OP_LABELS[ui.status?.opState ?? ""] ?? "");

  let working = $state(false);

  /** Resolve a whole file with one side (git checkout --ours/--theirs + add). */
  async function resolveWhole(ours: boolean) {
    if (!ui.repo || !selected || working) return;
    working = true;
    try {
      await api.resolveConflict(ui.repo.path, selected, ours);
      await refreshStatus();
    } catch (e) {
      showError(e);
    } finally {
      working = false;
    }
  }

  async function openMergetool() {
    if (!ui.repo || !selected || working) return;
    working = true;
    try {
      await api.openMergetool(ui.repo.path, selected);
      await refreshStatus();
    } catch (e) {
      showError(e);
    } finally {
      working = false;
    }
  }

  const fileName = (p: string) => p.split(/[\\/]/).pop() ?? p;
</script>

<div class="workshop">
  <header class="head">
    <Icon name="merge" size={18} />
    <div class="titles">
      <h2>
        {t("conflictws.title")}
        {#if opLabel}<span class="op">· {opLabel}</span>{/if}
        {#if copy.step}
          <span class="chip">{t("conflictws.step", copy.step)}</span>
        {/if}
      </h2>
      <p class="sub">
        {t(copy.subtitle.key, copy.subtitle.params)}
        {#if ctx?.theirsSummary}
          <span class="summary">{t("conflictws.at", { summary: ctx.theirsSummary })}</span>
        {/if}
      </p>
      {#if copy.hint}
        <p class="rebase-hint">
          <Icon name="alert" size={13} />
          {t(copy.hint.key, copy.hint.params)}
        </p>
      {/if}
    </div>
    <div class="head-actions">
      <button onclick={() => (ui.view = "repo")}>{t("conflictws.back")}</button>
      <button class="abort" onclick={abortOperation}>{t("conflict.abort")}</button>
      <button class="primary" disabled={conflicted.length > 0} onclick={continueOperation}>
        {t("conflict.continue")}
      </button>
    </div>
  </header>

  <!-- Progress across all files of the session -->
  {#if seen.length > 0}
    <div class="progress-row">
      <div
        class="bar"
        role="progressbar"
        aria-valuemin={0}
        aria-valuemax={seen.length}
        aria-valuenow={resolved.length}
        aria-label={t("conflictws.progress", { done: resolved.length, total: seen.length })}
      >
        <div
          class="fill"
          style:width="{seen.length ? (resolved.length / seen.length) * 100 : 0}%"
        ></div>
      </div>
      <span class="progress-text">
        {t("conflictws.progress", { done: resolved.length, total: seen.length })}
      </span>
    </div>
  {/if}

  <div class="body">
    <aside class="files">
      {#if conflicted.length > 0}
        <h3 class="group">{t("conflictws.filesOpen")}</h3>
        {#each conflicted as p (p)}
          <button
            class="file ghost"
            class:selected={selected === p}
            title={p}
            onclick={() => (selected = p)}
          >
            <Icon name="alert" size={13} />
            <span class="fname">{fileName(p)}</span>
          </button>
        {/each}
      {/if}
      {#if resolved.length > 0}
        <h3 class="group">{t("conflictws.filesResolved")}</h3>
        {#each resolved as p (p)}
          <div class="file done" title={p}>
            <Icon name="check" size={13} />
            <span class="fname">{fileName(p)}</span>
          </div>
        {/each}
      {/if}
    </aside>

    <section class="main">
      {#if selected}
        <div class="file-head">
          <span class="path" title={selected}>{selected}</span>
          <div class="file-actions">
            <button disabled={working} onclick={() => resolveWhole(true)}>
              {t("conflictws.whole", { side: oursHead })}
            </button>
            <button disabled={working} onclick={() => resolveWhole(false)}>
              {t("conflictws.whole", { side: theirsHead })}
            </button>
            <button disabled={working} onclick={openMergetool}>
              {t("conflictws.mergetool")}
            </button>
          </div>
        </div>
        <!-- key: switching the file resets the resolver state cleanly -->
        {#key selected}
          <ConflictResolver file={selected} {oursHead} {theirsHead} onresolved={refreshStatus} />
        {/key}
      {:else}
        <div class="all-done">
          <Icon name="check" size={28} />
          <strong>{t("conflictws.allDone")}</strong>
          <p>{t("conflictws.allDoneHint")}</p>
          <button class="primary" onclick={continueOperation}>{t("conflict.continue")}</button>
        </div>
      {/if}
    </section>
  </div>
</div>

<style>
  .workshop {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    background: var(--bg-app);
  }

  .head {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
    padding: var(--space-4) var(--space-5) var(--space-3);
    background: var(--bg-panel);
    border-bottom: 1px solid var(--border);
  }

  .titles {
    flex: 1;
    min-width: 0;
  }

  h2 {
    font-family: var(--display);
    font-size: 17px;
    font-weight: 650;
    letter-spacing: -0.01em;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }

  .op {
    color: var(--text-muted);
    font-weight: 500;
  }

  .chip {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-muted);
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: 999px;
    padding: 1px 8px;
  }

  .sub {
    margin-top: 2px;
    color: var(--text-muted);
    font-size: 12.5px;
  }

  .summary {
    color: var(--text-faint);
  }

  /* The rebase stumbling block deserves its own visible line. */
  .rebase-hint {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: var(--space-1);
    color: var(--warn);
    font-size: 12px;
  }

  .head-actions {
    display: flex;
    gap: var(--space-2);
    flex-shrink: 0;
  }

  .abort {
    color: var(--deleted);
  }

  .progress-row {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-2) var(--space-5);
    background: var(--bg-panel);
    border-bottom: 1px solid var(--border);
  }

  .bar {
    flex: 1;
    height: 6px;
    border-radius: 999px;
    background: var(--bg-inset);
    overflow: hidden;
  }

  .fill {
    height: 100%;
    background: var(--accent-dim);
    border-radius: 999px;
    transition: width 0.25s ease;
  }

  .progress-text {
    font-size: 11.5px;
    color: var(--text-faint);
    white-space: nowrap;
  }

  .body {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: 260px 1fr;
  }

  .files {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: var(--space-3);
    border-right: 1px solid var(--border);
    background: var(--bg-panel);
    overflow-y: auto;
  }

  .group {
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--text-faint);
    padding: var(--space-2) var(--space-2) var(--space-1);
  }

  .file {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 5px var(--space-2);
    border: none;
    box-shadow: none;
    background: transparent;
    border-radius: var(--radius);
    justify-content: flex-start;
    text-align: left;
    color: var(--warn);
    width: 100%;
  }

  .file:hover:not(.done) {
    background: var(--bg-hover);
  }

  .file.selected {
    background: var(--bg-selected);
  }

  .file.done {
    color: var(--text-faint);
  }

  .fname {
    color: var(--text-primary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .file.done .fname {
    color: var(--text-muted);
  }

  .main {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-4) var(--space-5);
    min-width: 0;
    min-height: 0;
  }

  .file-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    flex-wrap: wrap;
  }

  .path {
    font-family: var(--mono);
    font-size: 12.5px;
    color: var(--text-primary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }

  .file-actions {
    display: flex;
    gap: var(--space-2);
    flex-wrap: wrap;
  }

  .file-actions button {
    font-size: 12px;
    max-width: 260px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    display: inline-block;
  }

  .all-done {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    color: var(--accent);
  }

  .all-done p {
    color: var(--text-muted);
    font-size: 13px;
  }

  .all-done button {
    margin-top: var(--space-2);
  }

  @media (max-width: 760px) {
    .body {
      grid-template-columns: 1fr;
    }

    .files {
      border-right: none;
      border-bottom: 1px solid var(--border);
      max-height: 160px;
    }
  }
</style>
