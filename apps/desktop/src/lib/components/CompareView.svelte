<script lang="ts">
  // Compare two commits or branches: what only one side has,
  // and what the other side would bring — the diff since they forked.
  import { api, type CommitInfo, type FileDiff } from "../api";
  import { timeAgo } from "../format";
  import { t } from "../i18n.svelte";
  import { mergeBranch, prProvider, showError, ui } from "../state.svelte";
  import DiffView from "./DiffView.svelte";
  import Icon from "./Icon.svelte";

  /** Cap like the commit diff: a huge comparison must not freeze the view. */
  const MAX_FILES = 300;
  const MAX_COMMITS = 500;

  type Tab = "onlyTo" | "onlyFrom" | "changes";
  let tab = $state<Tab>("changes");

  let onlyFrom = $state<CommitInfo[] | null>(null);
  let onlyTo = $state<CommitInfo[] | null>(null);
  let diffs = $state<FileDiff[]>([]);
  let diffTotal = $state<number | null>(null);
  let loadingDiff = $state(true);

  // Load everything once per pair; a newer pair wins (sequence token).
  let seq = 0;
  $effect(() => {
    const c = ui.compare;
    const repo = ui.repo;
    if (!c || !repo) return;
    const mine = ++seq;
    onlyFrom = null;
    onlyTo = null;
    diffs = [];
    diffTotal = null;
    loadingDiff = true;
    api
      .logRange(repo.path, c.to, c.from, MAX_COMMITS)
      .then((l) => mine === seq && (onlyFrom = l))
      .catch((e) => mine === seq && showError(e));
    api
      .logRange(repo.path, c.from, c.to, MAX_COMMITS)
      .then((l) => mine === seq && (onlyTo = l))
      .catch((e) => mine === seq && showError(e));
    // "Changes": what `to` brings compared to where both forked — the diff a
    // merge of `to` into `from` would add.
    api
      .mergeBase(repo.path, c.from, c.to)
      .catch(() => c.from)
      .then((base) =>
        api.rangeDiffStream(repo.path, base, c.to, MAX_FILES, (fd) => {
          if (mine === seq) diffs = [...diffs, fd];
        }),
      )
      .then((n) => mine === seq && (diffTotal = n))
      .catch((e) => mine === seq && showError(e))
      .finally(() => mine === seq && (loadingDiff = false));
  });

  function back() {
    ui.view = "repo";
    ui.compare = null;
  }

  // With the current branch on the left, the other branch can be brought in
  // or proposed as a PR/MR target right from here.
  const current = $derived(ui.status?.branch ?? null);
  const other = $derived(
    ui.compare?.branch && current && ui.compare.branch !== current ? ui.compare.branch : null,
  );
  const crLabel = $derived(prProvider() === "gitlab" ? "Merge Request" : "Pull Request");

  async function mergeOther(name: string) {
    back();
    await mergeBranch(name);
  }

  function updateFromOther(name: string) {
    ui.modal = { kind: "updateFrom", source: name };
  }

  function createCr(name: string) {
    const remote = ui.branches.find((b) => b.name === name)?.isRemote;
    ui.modal = { kind: "createCr", target: remote ? name.slice(name.indexOf("/") + 1) : name };
  }
</script>

{#if ui.compare}
  {@const c = ui.compare}
  <div class="compare">
    <header class="head">
      <Icon name="split" size={18} />
      <div class="titles">
        <h2>{t("compare.title", { from: c.fromLabel, to: c.toLabel })}</h2>
        <p class="sub">{t("compare.sub", { from: c.fromLabel, to: c.toLabel })}</p>
      </div>
      <button onclick={back}>{t("conflictws.back")}</button>
    </header>

    {#if other}
      <div class="acts">
        <button onclick={() => mergeOther(other)}>
          <Icon name="merge" size={14} />
          {t("compare.merge")}
        </button>
        <button onclick={() => updateFromOther(other)}>{t("compare.updateFrom")}</button>
        <button onclick={() => createCr(other)}>
          <Icon name="pr" size={14} />
          {t("compare.createCr", { label: crLabel })}
        </button>
      </div>
    {/if}

    <div class="tabs" role="tablist">
      <button
        role="tab"
        class:on={tab === "changes"}
        aria-selected={tab === "changes"}
        onclick={() => (tab = "changes")}
      >
        {t("compare.changes")}
        {#if diffTotal !== null}<span class="badge">{diffTotal}</span>{/if}
      </button>
      <button
        role="tab"
        class:on={tab === "onlyTo"}
        aria-selected={tab === "onlyTo"}
        onclick={() => (tab = "onlyTo")}
      >
        {t("compare.onlyIn", { name: c.toLabel })}
        {#if onlyTo}<span class="badge">{onlyTo.length}</span>{/if}
      </button>
      <button
        role="tab"
        class:on={tab === "onlyFrom"}
        aria-selected={tab === "onlyFrom"}
        onclick={() => (tab = "onlyFrom")}
      >
        {t("compare.onlyIn", { name: c.fromLabel })}
        {#if onlyFrom}<span class="badge">{onlyFrom.length}</span>{/if}
      </button>
    </div>

    <div class="body">
      {#if tab === "changes"}
        {#if diffTotal !== null && diffTotal > diffs.length}
          <div class="cap-note">
            {t("history.largeCommitCapped", { shown: diffs.length, total: diffTotal })}
          </div>
        {/if}
        <DiffView {diffs} loading={loadingDiff} emptyText={t("compare.noChanges")} />
      {:else}
        {@const list = tab === "onlyTo" ? onlyTo : onlyFrom}
        {#if list === null}
          <p class="hint"><span class="spin"></span> {t("compare.loading")}</p>
        {:else if list.length === 0}
          <p class="hint">{t("compare.noCommits")}</p>
        {:else}
          <ul class="commits">
            {#each list as commit (commit.id)}
              <li>
                <code class="sha">{commit.shortId}</code>
                <span class="subject">{commit.summary}</span>
                <span class="meta">{commit.authorName} · {timeAgo(commit.time)}</span>
              </li>
            {/each}
          </ul>
        {/if}
      {/if}
    </div>
  </div>
{/if}

<style>
  .compare {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
  }

  .head {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    border-bottom: 1px solid var(--border);
  }

  .titles {
    flex: 1;
    min-width: 0;
  }

  h2 {
    margin: 0;
    font-size: 16px;
  }

  .sub {
    margin: 2px 0 0;
    color: var(--text-muted);
    font-size: 12px;
  }

  .acts {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-4);
    border-bottom: 1px solid var(--border);
  }

  .tabs {
    display: flex;
    gap: 4px;
    padding: var(--space-2) var(--space-4) 0;
    border-bottom: 1px solid var(--border);
  }

  .tabs button {
    background: transparent;
    border: none;
    border-bottom: 2px solid transparent;
    border-radius: 0;
    box-shadow: none;
  }

  .tabs button.on {
    border-bottom-color: var(--accent);
    color: var(--text-primary);
  }

  .badge {
    margin-left: 6px;
    font-size: 11px;
    color: var(--text-muted);
  }

  .body {
    flex: 1;
    min-height: 0;
    overflow: auto;
    display: flex;
    flex-direction: column;
  }

  .cap-note,
  .hint {
    padding: var(--space-2) var(--space-4);
    color: var(--text-muted);
    font-size: 12px;
  }

  .commits {
    list-style: none;
    margin: 0;
    padding: var(--space-2) var(--space-4);
  }

  .commits li {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    padding: 6px 0;
    border-bottom: 1px solid var(--border);
  }

  .sha {
    font-family: var(--mono);
    color: var(--text-muted);
    font-size: 12px;
  }

  .subject {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .meta {
    color: var(--text-faint);
    font-size: 12px;
    white-space: nowrap;
  }
</style>
