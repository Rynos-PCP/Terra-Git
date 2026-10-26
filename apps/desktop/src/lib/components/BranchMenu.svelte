<script lang="ts">
  import { confirm } from "../dialogs";
  import type { BranchInfo } from "../api";
  import { t, tn } from "../i18n.svelte";
  import {
    createBranch,
    deleteBranch,
    mergeBranch,
    openCompare,
    rebaseOnto,
    rememberedUpdateSource,
    renameBranch,
    switchBranch,
    ui,
  } from "../state.svelte";
  import { api } from "../api";
  import Icon from "./Icon.svelte";
  import Menu from "./Menu.svelte";

  let open = $state(false);
  let filter = $state("");
  let newName = $state("");
  /** "Publish right away (branch only)" for the new branch. */
  let publishNew = $state(false);
  /** Branch whose rename field is currently open. */
  let renaming = $state<string | null>(null);
  let renameValue = $state("");

  const currentBranch = $derived(
    ui.status?.branch ??
      ui.repo?.currentBranch ??
      (ui.repo?.headDetached ? "HEAD (detached)" : "—"),
  );

  const visible = $derived.by(() => {
    const locals = ui.branches.filter((b) => !b.isRemote);
    const localNames = new Set(locals.map((b) => b.name));
    const remoteOnly = ui.branches.filter(
      (b) => b.isRemote && b.shortName !== null && !localNames.has(b.shortName),
    );
    const all = [...locals, ...remoteOnly];
    const f = filter.trim().toLowerCase();
    return f ? all.filter((b) => b.name.toLowerCase().includes(f)) : all;
  });

  function close() {
    open = false;
    filter = "";
    renaming = null;
  }

  function choose(branch: BranchInfo) {
    close();
    switchBranch(branch.isRemote ? (branch.shortName ?? branch.name) : branch.name);
  }

  function create() {
    const name = newName.trim();
    if (!name) return;
    close();
    newName = "";
    createBranch(name, publishNew);
  }

  async function removeBranch(branch: BranchInfo) {
    const result = await deleteBranch(branch.name, false);
    if (result === "needs-force") {
      const force = await confirm(t("branch.deleteNotMerged", { name: branch.name }), {
        title: t("branch.deleteTitle"),
        kind: "warning",
      });
      if (force) await deleteBranch(branch.name, true);
    }
  }

  function startRename(branch: BranchInfo) {
    renaming = branch.name;
    renameValue = branch.name;
  }

  async function commitRename() {
    const target = renaming;
    const value = renameValue.trim();
    renaming = null;
    if (target && value && value !== target) {
      await renameBranch(target, value);
    }
  }

  async function mergeIntoCurrent(branch: BranchInfo) {
    close();
    await mergeBranch(branch.name);
  }

  async function rebaseOn(branch: BranchInfo) {
    close();
    await rebaseOnto(branch.name);
  }

  function openUpdateFrom() {
    close();
    ui.modal = { kind: "updateFrom" };
  }

  // "n behind <source>": against the source last used to update this branch,
  // counted when the menu opens.
  let behind = $state<{ source: string; n: number } | null>(null);
  $effect(() => {
    const repo = ui.repo;
    const source = open ? rememberedUpdateSource() : undefined;
    if (!repo || !source) {
      behind = null;
      return;
    }
    api
      .countCommits(repo.path, "HEAD", source)
      .then((n) => (behind = { source, n }))
      .catch(() => (behind = null));
  });
</script>

<!-- No role="menu": this popup contains input fields and list entries,
     no menuitem buttons — marked up as a menu, the ARIA tree would be
     invalid and the arrow-key navigation would lead nowhere. -->
<Menu bind:open width="360px" role="dialog" ariaLabel={t("branch.switch")}>
  {#snippet trigger({ toggle })}
    <button class="segment" data-testid="branch-menu" onclick={toggle} title={t("branch.switch")}>
      <span class="glyph"><Icon name="branch" /></span>
      <span class="col">
        <span class="label">{t("branch.label")}</span>
        <strong>{currentBranch}</strong>
      </span>
      <Icon name="chevronDown" size={12} />
    </button>
  {/snippet}

  <div class="update">
    <button class="ghost" onclick={openUpdateFrom} disabled={!ui.repo || ui.branches.length === 0}>
      <Icon name="merge" size={13} />
      {t("updateFrom.menu")}
    </button>
    {#if behind && behind.n > 0}
      <span class="behind">{tn("updateFrom.behind", behind.n, { source: behind.source })}</span>
    {/if}
  </div>

  <input type="text" placeholder={t("branch.filterPlaceholder")} bind:value={filter} />

  <div class="list">
    {#each visible as b (b.name)}
      <div class="row" data-branch={b.name} class:head={b.isHead} class:gone={b.upstreamGone}>
        {#if renaming === b.name}
          <input
            type="text"
            bind:value={renameValue}
            onkeydown={(e) => {
              if (e.key === "Enter") commitRename();
              if (e.key === "Escape") renaming = null;
            }}
          />
          <button class="ghost" title={t("branch.applyRename")} onclick={commitRename}>
            <Icon name="check" size={13} />
          </button>
        {:else}
          <button class="name ghost" onclick={() => choose(b)} title={b.name}>
            <Icon name="branch" size={13} />
            <span class="txt">{b.name}</span>
            {#if b.isHead}<span class="badge">{t("branch.badgeCurrent")}</span>{/if}
            {#if b.isRemote}<span class="badge">{t("branch.badgeRemote")}</span>{/if}
            {#if b.upstreamGone}<span class="badge gone" title={t("branch.goneTooltip")}
                >{t("branch.badgeGone")}</span
              >{/if}
          </button>
          {#if !b.isHead}
            <div class="actions">
              <button
                class="ghost"
                title={t("branch.mergeIntoCurrent")}
                onclick={() => mergeIntoCurrent(b)}
              >
                <Icon name="merge" size={13} />
              </button>
              <button
                class="ghost"
                title={t("compare.withCurrent")}
                onclick={() => {
                  close();
                  openCompare({
                    from: "HEAD",
                    to: b.name,
                    fromLabel: currentBranch,
                    toLabel: b.name,
                    branch: b.name,
                  });
                }}
              >
                <Icon name="split" size={13} />
              </button>
              <!-- Rebase onto remote branches too (e.g. onto origin/main). -->
              <button
                class="ghost"
                data-testid="rebase-onto"
                title={t("branch.rebaseOnto")}
                onclick={() => rebaseOn(b)}
              >
                <Icon name="history" size={13} />
              </button>
              {#if !b.isRemote}
                <button class="ghost" title={t("branch.rename")} onclick={() => startRename(b)}>
                  <Icon name="edit" size={13} />
                </button>
                <button
                  class="ghost danger"
                  title={t("common.delete")}
                  onclick={() => removeBranch(b)}
                >
                  <Icon name="trash" size={13} />
                </button>
              {/if}
            </div>
          {/if}
        {/if}
      </div>
    {:else}
      <div class="empty">
        {#if !filter.trim() && ui.branches.length === 0}
          <!-- Fresh repo (unborn HEAD): explain instead of "nothing found" —
               creating/renaming works, the branch comes into being with the
               first commit. -->
          {t("branch.emptyUnborn", { name: currentBranch })}
        {:else}
          {t("branch.noneFound")}
        {/if}
      </div>
    {/each}
  </div>

  <div class="create">
    <input
      type="text"
      placeholder={t("branch.newNamePlaceholder")}
      bind:value={newName}
      onkeydown={(e) => e.key === "Enter" && create()}
    />
    <button class="primary" onclick={create} disabled={!newName.trim()}>{t("common.create")}</button
    >
  </div>
  {#if ui.remotes.length > 0}
    <label class="publish">
      <input type="checkbox" bind:checked={publishNew} />
      {t("branch.publishRightAway")}
    </label>
  {/if}
</Menu>

<style>
  .publish {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: var(--space-1);
    font-size: 12px;
    color: var(--text-muted);
    cursor: pointer;
  }

  .update {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-bottom: var(--space-2);
  }

  .behind {
    color: var(--text-muted);
    font-size: 12px;
  }

  .segment {
    background: transparent;
    border-color: transparent;
    box-shadow: none;
    padding: 4px 10px;
  }

  .segment:hover {
    background: var(--bg-hover);
  }

  .glyph {
    color: var(--accent);
  }

  .col {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    line-height: 1.2;
  }

  .label {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.07em;
    color: var(--text-muted);
  }

  .list {
    max-height: 320px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 1px;
    margin-top: var(--space-1);
  }

  .row {
    display: flex;
    align-items: center;
    gap: 2px;
    border-radius: var(--radius);
  }

  .row:hover {
    background: var(--bg-hover);
  }

  .name {
    flex: 1;
    justify-content: flex-start;
    overflow: hidden;
    color: var(--text-primary);
    padding: 5px 8px;
  }

  .name:hover {
    background: transparent;
  }

  .txt {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .row.head .txt {
    color: var(--accent);
    font-weight: 600;
  }

  .row.gone .txt {
    color: var(--warn, #d59b35);
  }

  /* A fixed amber surface with dark text — as a standalone chip in both
     themes readable (independent of the theme-dependent --warn text tone). */
  .badge.gone {
    background: #d59b35;
    color: #1a1a1a;
  }

  .actions {
    display: flex;
    gap: 0;
    opacity: 0;
    transition: opacity 0.1s ease;
  }

  .row:hover .actions,
  .row:focus-within .actions {
    opacity: 1;
  }

  .actions button {
    padding: 4px 5px;
  }

  .empty {
    color: var(--text-muted);
    padding: var(--space-3);
    text-align: center;
  }

  .create {
    display: flex;
    gap: var(--space-2);
    border-top: 1px solid var(--border);
    padding-top: var(--space-2);
    margin-top: var(--space-1);
  }
</style>
