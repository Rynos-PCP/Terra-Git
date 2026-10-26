<script lang="ts">
  // "Update current branch from…": any branch can feed the
  // current one — e.g. a release branch from main, a hotfix from a release.
  // The list says how many commits each candidate would bring in; the source
  // used last for this branch is preselected.
  import { api } from "../api";
  import { t, tn } from "../i18n.svelte";
  import { rememberedUpdateSource, ui, updateBranchFrom } from "../state.svelte";
  import { effectiveSource, preselect, updateCandidates } from "../updateFrom";
  import Modal from "./Modal.svelte";

  let { onclose, initial }: { onclose: () => void; initial?: string } = $props();

  const current = $derived(ui.status?.branch ?? ui.repo?.currentBranch ?? null);
  const candidates = $derived(updateCandidates(ui.branches, current, ui.status?.upstream ?? null));

  let filter = $state("");
  const shown = $derived(
    candidates.filter((c) => c.name.toLowerCase().includes(filter.trim().toLowerCase())),
  );

  let selected = $state<string | null>(null);
  let method = $state<"merge" | "rebase">(ui.pullStrategy === "rebase" ? "rebase" : "merge");
  let useRemoteState = $state(true);
  /** Commits each candidate would bring in (by effective ref); undefined = counting.
   *  Keys carry a prefix: a branch may be called `constructor` or `__proto__`. */
  let counts = $state<Record<string, number | null>>({});
  const key = (ref: string) => `ref:${ref}`;
  const countOf = (ref: string) => (Object.hasOwn(counts, key(ref)) ? counts[key(ref)] : undefined);

  // Preselection: remembered for this branch, else the default branch.
  $effect(() => {
    const repo = ui.repo;
    if (!repo || selected !== null) return;
    api
      .defaultBranch(repo.path)
      .catch(() => null)
      .then((def) => {
        if (selected !== null) return;
        selected =
          initial && candidates.some((c) => c.name === initial)
            ? initial
            : preselect(candidates, rememberedUpdateSource(), def);
      });
  });

  // Count lazily, once per effective ref (sequential: a big repo with many
  // branches must not start dozens of revwalks at once).
  $effect(() => {
    const repo = ui.repo;
    if (!repo) return;
    const refs = candidates.map((c) => effectiveSource(c, useRemoteState));
    void (async () => {
      for (const ref of refs) {
        if (Object.hasOwn(counts, key(ref))) continue;
        try {
          counts[key(ref)] = await api.countCommits(repo.path, "HEAD", ref);
        } catch {
          counts[key(ref)] = null;
        }
      }
    })();
  });

  const chosen = $derived(candidates.find((c) => c.name === selected) ?? null);
  const source = $derived(chosen ? effectiveSource(chosen, useRemoteState) : null);
  const pending = $derived(source ? countOf(source) : undefined);
  /** The run fetches first whenever the source is a remote state — the count
   *  then only knows the last fetch, so "up to date" may not be the last word. */
  const fetchFirst = $derived(
    !!chosen && (chosen.isRemote || (!!chosen.upstream && useRemoteState)),
  );

  async function run() {
    if (!chosen || !source) return;
    onclose();
    await updateBranchFrom(chosen.name, source, method, fetchFirst);
  }
</script>

<Modal title={t("updateFrom.title", { name: current ?? "HEAD" })} width="560px" {onclose}>
  <p class="hint">{t("updateFrom.hint")}</p>
  <input type="text" placeholder={t("updateFrom.filter")} bind:value={filter} />
  <div class="list" role="listbox" aria-label={t("updateFrom.source")}>
    {#each shown as c (c.name)}
      {@const ref = effectiveSource(c, useRemoteState)}
      {@const n = countOf(ref)}
      <button
        class="cand ghost"
        class:on={selected === c.name}
        class:done={n === 0}
        role="option"
        aria-selected={selected === c.name}
        onclick={() => (selected = c.name)}
      >
        <span class="name">{c.name}</span>
        {#if c.isRemote}<span class="badge">{t("branch.badgeRemote")}</span>{/if}
        <span class="count">
          {#if n === undefined}…{:else if n === null}—{:else if n === 0}{t(
              "updateFrom.upToDate",
            )}{:else}{tn("updateFrom.commits", n)}{/if}
        </span>
      </button>
    {:else}
      <p class="hint">{t("branch.noneFound")}</p>
    {/each}
  </div>

  {#if chosen?.upstream}
    <label class="check">
      <input type="checkbox" bind:checked={useRemoteState} />
      {t("updateFrom.useRemote", { upstream: chosen.upstream })}
    </label>
  {/if}

  <fieldset class="method">
    <legend>{t("updateFrom.method")}</legend>
    <label class="check">
      <input type="radio" bind:group={method} value="merge" />
      {t("sync.merge")} — {t("sync.mergeHint")}
    </label>
    <label class="check">
      <input type="radio" bind:group={method} value="rebase" />
      {t("sync.rebase")} — {t("sync.rebaseHint")}
    </label>
  </fieldset>

  {#if source && typeof pending === "number" && pending > 0}
    <p class="preview">
      {method === "merge"
        ? tn("updateFrom.previewMerge", pending, { source, target: current ?? "HEAD" })
        : tn("updateFrom.previewRebase", pending, { source, target: current ?? "HEAD" })}
    </p>
  {/if}

  <div class="actions">
    <button onclick={onclose}>{t("common.cancel")}</button>
    <button class="primary" disabled={!source || (pending === 0 && !fetchFirst)} onclick={run}>
      {t("updateFrom.run")}
    </button>
  </div>
</Modal>

<style>
  /* Same as the shared dialogs in Modals.svelte (styles there are scoped). */
  .hint {
    color: var(--text-muted);
    font-size: 12px;
  }

  .check {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
    cursor: pointer;
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
    margin-top: var(--space-4);
  }

  .list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    max-height: 260px;
    overflow-y: auto;
    margin: var(--space-2) 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: 4px;
  }

  .cand {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    text-align: left;
    justify-content: flex-start;
  }

  .cand.on {
    background: var(--bg-selected);
    box-shadow: inset 3px 0 0 var(--accent);
  }

  .cand.done .name,
  .cand.done .count {
    color: var(--text-faint);
  }

  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--mono);
    font-size: 12px;
  }

  .count {
    color: var(--text-muted);
    font-size: 12px;
    white-space: nowrap;
  }

  .method {
    border: none;
    margin: var(--space-2) 0 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .method legend {
    font-weight: 600;
    margin-bottom: 4px;
  }

  .preview {
    margin: var(--space-3) 0 0;
    font-weight: 600;
  }
</style>
