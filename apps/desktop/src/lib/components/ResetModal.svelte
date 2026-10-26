<script lang="ts">
  // Reset the current branch to a commit: the three modes in
  // plain words, a warning when commits already on the remote would leave the
  // branch, and the copy of uncommitted work before a hard reset.
  import { api, type ResetMode } from "../api";
  import { worktreeDirty } from "../branchSwitch";
  import { t } from "../i18n.svelte";
  import { resetBranchTo, ui } from "../state.svelte";
  import Modal from "./Modal.svelte";

  let { commitId, subject, onclose }: { commitId: string; subject: string; onclose: () => void } =
    $props();

  let mode = $state<ResetMode>("mixed");
  /** Commits the branch loses / of those, already on the remote. */
  let removed = $state<number | null>(null);
  let removedPushed = $state(0);

  $effect(() => {
    const repo = ui.repo;
    if (!repo) return;
    api
      .countCommits(repo.path, commitId, "HEAD")
      .then((n) => (removed = n))
      .catch(() => (removed = null));
    const upstream = ui.status?.upstream;
    if (upstream)
      api
        .mergeBase(repo.path, "HEAD", upstream)
        .then((base) => api.countCommits(repo.path, commitId, base))
        .then((n) => (removedPushed = n))
        .catch(() => (removedPushed = 0));
  });

  const dirty = $derived(worktreeDirty(ui.status));
  const branch = $derived(ui.status?.branch ?? ui.repo?.currentBranch ?? "HEAD");
</script>

<Modal title={t("reset.title", { branch })} width="520px" {onclose}>
  <p class="hint">
    {t("reset.target", { id: commitId.slice(0, 8), subject })}
    {#if removed !== null && removed > 0}
      — {t("reset.removes", { n: removed })}
    {/if}
  </p>
  <label class="opt" class:on={mode === "soft"}>
    <input type="radio" bind:group={mode} value="soft" />
    <span><strong>{t("reset.soft")}</strong> — {t("reset.softHint")}</span>
  </label>
  <label class="opt" class:on={mode === "mixed"}>
    <input type="radio" bind:group={mode} value="mixed" />
    <span><strong>{t("reset.mixed")}</strong> — {t("reset.mixedHint")}</span>
  </label>
  <label class="opt danger" class:on={mode === "hard"}>
    <input type="radio" bind:group={mode} value="hard" />
    <span><strong>{t("reset.hard")}</strong> — {t("reset.hardHint")}</span>
  </label>
  {#if mode === "hard" && dirty}
    <p class="note">{t("reset.hardDirty")}</p>
  {/if}
  {#if removedPushed > 0}
    <p class="warn">{t("reset.pushedWarning", { n: removedPushed })}</p>
  {/if}
  <p class="note">{t("reset.backupNote")}</p>
  <div class="actions">
    <button onclick={onclose}>{t("common.cancel")}</button>
    <button
      class:danger={mode === "hard"}
      class="primary"
      onclick={() => resetBranchTo(commitId, mode)}
    >
      {t("reset.run")}
    </button>
  </div>
</Modal>

<style>
  .hint {
    color: var(--text-muted);
    font-size: 12px;
  }

  .opt {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 8px;
    margin-top: 6px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    font-size: 13px;
    cursor: pointer;
  }

  .opt.on {
    border-color: var(--accent);
  }

  .opt.danger strong {
    color: var(--deleted);
  }

  .note {
    margin: var(--space-2) 0 0;
    color: var(--text-muted);
    font-size: 12px;
  }

  .warn {
    margin: var(--space-2) 0 0;
    color: var(--deleted);
    font-size: 12px;
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
    margin-top: var(--space-4);
  }
</style>
