<script lang="ts">
  // The reflog: every position HEAD or a branch had — the
  // place where commits lost to a reset or rebase are found again.
  import { api, type BackupInfo, type ReflogEntry } from "../api";
  import { timeAgo } from "../format";
  import { t } from "../i18n.svelte";
  import { checkoutCommit, openCompare, showError, ui } from "../state.svelte";
  import Icon from "./Icon.svelte";
  import Modal from "./Modal.svelte";

  let { onclose }: { onclose: () => void } = $props();

  const LIMIT = 300;
  const ZERO = "0000000000000000000000000000000000000000";

  let reference = $state("HEAD");
  let entries = $state<ReflogEntry[] | null>(null);
  const locals = $derived(ui.branches.filter((b) => !b.isRemote).map((b) => b.name));
  /** terra-git's own backup refs — shown above the reflog, they outlive it. */
  let backups = $state<BackupInfo[]>([]);

  $effect(() => {
    const repo = ui.repo;
    if (!repo) return;
    api
      .backups(repo.path)
      .then((b) => (backups = b))
      .catch(() => (backups = []));
  });

  $effect(() => {
    const repo = ui.repo;
    const ref = reference;
    if (!repo) return;
    entries = null;
    api
      .reflog(repo.path, ref, LIMIT)
      .then((e) => ref === reference && (entries = e))
      .catch((e) => {
        entries = [];
        showError(e);
      });
  });

  function showChanges(e: ReflogEntry) {
    openCompare({
      from: e.oldId === ZERO ? e.id : e.oldId,
      to: e.id,
      fromLabel: e.oldId === ZERO ? e.id.slice(0, 8) : e.oldId.slice(0, 8),
      toLabel: e.id.slice(0, 8),
    });
  }

  function compareWithHead(id: string) {
    openCompare({ from: "HEAD", to: id, fromLabel: "HEAD", toLabel: id.slice(0, 8) });
  }

  function branchHere(id: string) {
    ui.modal = { kind: "branchFrom", commitId: id };
  }

  function resetHere(id: string, subject: string) {
    ui.modal = { kind: "resetTo", commitId: id, subject };
  }

  async function checkout(id: string) {
    onclose();
    await checkoutCommit(id);
  }
</script>

<Modal title={t("reflog.title")} width="760px" {onclose}>
  <p class="hint">{t("reflog.hint")}</p>
  <label class="pick">
    <span>{t("reflog.of")}</span>
    <select bind:value={reference}>
      <option value="HEAD">HEAD</option>
      {#each locals as name (name)}
        <option value={`refs/heads/${name}`}>{name}</option>
      {/each}
    </select>
  </label>
  {#if reference === "HEAD" && backups.length > 0}
    <h3 class="group">{t("reflog.backups")}</h3>
    <p class="hint">{t("reflog.backupsHint")}</p>
    <div class="list backups">
      {#each backups as b (b.name)}
        <div class="row">
          <span class="action">{b.op}</span>
          <code class="sha">{b.targetId.slice(0, 8)}</code>
          <span class="what">{b.subject}</span>
          <span class="when">{timeAgo(b.timestamp)}</span>
          <div class="acts">
            <button
              class="ghost"
              title={t("compare.withHead")}
              onclick={() => compareWithHead(b.targetId)}
            >
              <Icon name="split" size={13} />
            </button>
            <button
              class="ghost"
              title={t("history.branchFromHere")}
              onclick={() => branchHere(b.targetId)}
            >
              <Icon name="branch" size={13} />
            </button>
            <button
              class="ghost"
              title={t("reset.menu")}
              onclick={() => resetHere(b.targetId, b.subject)}
            >
              <Icon name="undo" size={13} />
            </button>
          </div>
        </div>
      {/each}
    </div>
    <h3 class="group">{t("reflog.title")}</h3>
  {/if}
  <div class="list">
    {#if entries === null}
      <p class="hint"><span class="spin"></span> {t("reflog.loading")}</p>
    {:else}
      {#each entries as e (e.index)}
        <div class="row">
          <span class="action">{e.action || "—"}</span>
          <code class="sha">{e.id.slice(0, 8)}</code>
          <span class="what" title={e.message}>{e.summary ?? e.message}</span>
          <span class="when">{timeAgo(e.time)}</span>
          <div class="acts">
            <button class="ghost" title={t("reflog.showChanges")} onclick={() => showChanges(e)}>
              <Icon name="split" size={13} />
            </button>
            <button
              class="ghost"
              title={t("history.branchFromHere")}
              onclick={() => branchHere(e.id)}
            >
              <Icon name="branch" size={13} />
            </button>
            <button class="ghost" title={t("reflog.checkout")} onclick={() => checkout(e.id)}>
              <Icon name="history" size={13} />
            </button>
            <button
              class="ghost"
              title={t("reset.menu")}
              onclick={() => resetHere(e.id, e.summary ?? e.message)}
            >
              <Icon name="undo" size={13} />
            </button>
          </div>
        </div>
      {:else}
        <p class="hint">{t("reflog.empty")}</p>
      {/each}
    {/if}
  </div>
</Modal>

<style>
  .hint {
    color: var(--text-muted);
    font-size: 12px;
  }

  .pick {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin: var(--space-2) 0;
    font-size: 13px;
    white-space: nowrap;
  }

  .pick select {
    flex: 1;
  }

  .group {
    margin: var(--space-3) 0 0;
    font-size: 12px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-muted);
  }

  .list.backups {
    max-height: 160px;
  }

  .list {
    max-height: 420px;
    overflow-y: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
  }

  .row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 4px 8px;
    border-bottom: 1px solid var(--border);
  }

  .row:last-child {
    border-bottom: none;
  }

  .action {
    min-width: 80px;
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-muted);
  }

  .sha {
    font-family: var(--mono);
    font-size: 12px;
    color: var(--text-muted);
  }

  .what {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .when {
    color: var(--text-faint);
    font-size: 12px;
    white-space: nowrap;
  }

  .acts {
    display: flex;
    gap: 2px;
  }
</style>
