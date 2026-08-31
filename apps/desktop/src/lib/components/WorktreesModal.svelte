<script lang="ts">
  import { onMount } from "svelte";
  import { confirm, open } from "@tauri-apps/plugin-dialog";
  import { api, type WorktreeInfo } from "../api";
  import { t } from "../i18n.svelte";
  import { openRepo, refreshBranches, showError, showInfo, ui } from "../state.svelte";
  import Icon from "./Icon.svelte";
  import Modal from "./Modal.svelte";

  let { onclose }: { onclose: () => void } = $props();

  let worktrees = $state<WorktreeInfo[]>([]);
  let loading = $state(true);
  /** Prevents double execution on a double click (pattern from Modals.svelte). */
  let submitting = $state(false);

  // New worktree: parent folder + folder name + an existing branch.
  // git worktree add expects an EXISTING branch (api.addWorktree has no -b) —
  // hence a selection instead of free text.
  let destParent = $state("");
  let destName = $state("");
  let nameEdited = $state(false);
  let branch = $state("");

  /** Path join with the separator of the chosen folder (pattern from Modals.svelte). */
  function joinPath(dir: string, name: string): string {
    const sep = dir.includes("\\") ? "\\" : "/";
    return dir.replace(/[\\/]+$/, "") + sep + name;
  }

  /** Path comparison for the "current" marker: git reports worktree paths with
   *  forward slashes, ui.repo.path with backslashes on Windows. */
  function samePath(a: string, b: string): boolean {
    const norm = (p: string) => p.replace(/\\/g, "/").replace(/\/+$/, "").toLowerCase();
    return norm(a) === norm(b);
  }

  const isCurrent = (w: WorktreeInfo) => !!ui.repo && samePath(w.path, ui.repo.path);

  // Branches not yet checked out in any worktree (every branch can only be
  // checked out once — git would reject the rest anyway).
  const availableBranches = $derived.by(() => {
    const used = new Set(worktrees.map((w) => w.branch).filter((b): b is string => b !== null));
    return ui.branches.filter((b) => !b.isRemote && !used.has(b.name));
  });

  // As long as the user has not touched the folder name, it follows the chosen
  // branch (slashes replaced to be path-safe).
  $effect(() => {
    if (!nameEdited) destName = branch.replace(/[\\/]/g, "-");
  });

  const target = $derived(
    destParent && destName.trim() ? joinPath(destParent, destName.trim()) : "",
  );

  async function load() {
    if (!ui.repo) return;
    loading = true;
    try {
      worktrees = await api.worktrees(ui.repo.path);
    } catch (e) {
      showError(e);
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    load();
  });

  async function pickParentDir() {
    const dir = await open({ directory: true, title: t("modals.chooseParentDir") });
    if (typeof dir === "string") destParent = dir;
  }

  async function doAdd() {
    if (!ui.repo || submitting || !target || !branch) return;
    submitting = true;
    try {
      await api.addWorktree(ui.repo.path, target, branch);
      showInfo(t("worktrees.added", { branch }));
      branch = "";
      destName = "";
      nameEdited = false;
      await load();
      // The branch is now checked out elsewhere — refresh the branch list.
      await refreshBranches();
    } catch (e) {
      showError(e);
    } finally {
      submitting = false;
    }
  }

  async function doRemove(w: WorktreeInfo) {
    if (submitting) return;
    const yes = await confirm(t("worktrees.removeConfirm", { path: w.path }), {
      title: t("worktrees.remove"),
      kind: "warning",
    });
    if (!yes || !ui.repo) return;
    submitting = true;
    try {
      await api.removeWorktree(ui.repo.path, w.path);
      showInfo(t("worktrees.removed"));
      await load();
      await refreshBranches();
    } catch (e) {
      showError(e);
    } finally {
      submitting = false;
    }
  }

  /** Opens the worktree as its own repository (pattern: repo switch). */
  function doOpen(w: WorktreeInfo) {
    onclose();
    openRepo(w.path);
  }
</script>

<Modal title={t("worktrees.title")} width="620px" {onclose}>
  <p class="hint">{t("worktrees.hint")}</p>
  {#if loading}
    <p class="hint"><span class="spin"></span> {t("worktrees.loading")}</p>
  {:else}
    {#each worktrees as w (w.path)}
      <div class="list-row">
        <Icon name="folder" size={14} />
        <strong class="branch-name">
          {w.branch ?? `${t("worktrees.detached")} · ${(w.headId ?? "").slice(0, 8)}`}
        </strong>
        {#if isCurrent(w)}
          <span class="badge current">{t("worktrees.current")}</span>
        {:else if w.isMain}
          <span class="badge">{t("worktrees.main")}</span>
        {/if}
        <span class="grow muted" title={w.path}>{w.path}</span>
        {#if !isCurrent(w)}
          <button class="ghost" title={t("worktrees.openHover")} onclick={() => doOpen(w)}>
            {t("worktrees.open")}
          </button>
        {/if}
        {#if !w.isMain && !isCurrent(w)}
          <button
            class="ghost danger"
            disabled={submitting}
            title={t("worktrees.remove")}
            onclick={() => doRemove(w)}
          >
            <Icon name="trash" size={13} />
          </button>
        {/if}
      </div>
    {:else}
      <p class="hint">{t("worktrees.none")}</p>
    {/each}

    <p class="section-label">{t("worktrees.addSection")}</p>
    {#if availableBranches.length === 0}
      <p class="hint">{t("worktrees.noBranches")}</p>
    {:else}
      <div class="row">
        <label class="grow-label">
          <span class="lbl">{t("branch.label")}</span>
          <select bind:value={branch}>
            <option value="" disabled hidden></option>
            {#each availableBranches as b (b.name)}
              <option value={b.name}>{b.name}</option>
            {/each}
          </select>
        </label>
      </div>
      <div class="row">
        <label class="grow-label">
          <span class="lbl">{t("modals.parentDirLabel")}</span>
          <div class="row">
            <input type="text" placeholder={t("modals.dirPlaceholder")} bind:value={destParent} />
            <button onclick={pickParentDir}><Icon name="folder" size={14} /></button>
          </div>
        </label>
        <label class="grow-label">
          <span class="lbl">{t("modals.nameLabel")}</span>
          <input type="text" bind:value={destName} oninput={() => (nameEdited = true)} />
        </label>
      </div>
      {#if target}
        <p class="hint">{t("worktrees.targetHint")} <code>{target}</code></p>
      {/if}
      <p class="hint">{t("worktrees.branchHint")}</p>
      <div class="actions">
        <button onclick={onclose}>{t("common.close")}</button>
        <button class="primary" disabled={submitting || !target || !branch} onclick={doAdd}>
          {#if submitting}<span class="spin"></span>{/if}
          <Icon name="plus" size={13} />
          {t("common.create")}
        </button>
      </div>
    {/if}
  {/if}
</Modal>

<style>
  .hint {
    color: var(--text-muted);
    font-size: 12px;
  }

  .hint code {
    font-family: var(--mono);
    color: var(--text-primary);
  }

  .lbl {
    display: block;
    font-size: 12px;
    color: var(--text-muted);
    margin-bottom: 4px;
  }

  .row {
    display: flex;
    gap: var(--space-2);
    align-items: flex-end;
  }

  .grow-label {
    flex: 1;
    min-width: 0;
  }

  .grow-label select,
  .grow-label input {
    width: 100%;
  }

  .list-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-1) 0;
    border-bottom: 1px solid var(--border);
  }

  .list-row:last-of-type {
    border-bottom: none;
  }

  .branch-name {
    flex-shrink: 0;
    max-width: 220px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .badge.current {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    border-color: transparent;
    color: var(--accent);
  }

  .grow {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .muted {
    color: var(--text-muted);
    font-size: 12px;
  }

  .section-label {
    margin-top: var(--space-2);
    font-size: 11px;
    font-weight: 650;
    text-transform: uppercase;
    letter-spacing: 0.07em;
    color: var(--text-faint);
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
    margin-top: var(--space-2);
  }
</style>
