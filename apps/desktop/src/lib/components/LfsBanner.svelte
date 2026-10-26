<script lang="ts">
  // Git LFS hint above the repo view: git-lfs missing for
  // a repo that uses it, or LFS files whose content was never downloaded.
  import { api } from "../api";
  import { tn, t } from "../i18n.svelte";
  import { LFS_INSTALL_URL } from "../lfs";
  import { lfsPull, ui } from "../state.svelte";
  import Icon from "./Icon.svelte";

  const lfs = $derived(ui.lfs);
  const blocked = $derived(!!ui.busy || ui.working > 0);
</script>

{#if lfs?.used && !lfs.installed}
  <div class="banner warn" role="status">
    <Icon name="alert" />
    <div class="text">{t("lfs.notInstalled")}</div>
    <button onclick={() => api.openExternal(LFS_INSTALL_URL).catch(() => {})}>
      <Icon name="external" size={14} />
      {t("lfs.install")}
    </button>
  </div>
{:else if lfs?.used && lfs.missing > 0}
  <div class="banner" role="status">
    <Icon name="info" />
    <div class="text">{tn("lfs.missing", lfs.missing)}</div>
    <button class="primary" disabled={blocked} onclick={lfsPull}>
      <Icon name="arrowDown" size={14} />
      {t("lfs.pull")}
    </button>
  </div>
{/if}

<style>
  .banner {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-2) var(--space-4);
    background: var(--hunk-bg);
    border-bottom: 1px solid var(--border);
    color: var(--text-primary);
    font-size: 13px;
  }

  .banner.warn {
    background: var(--del-bg);
  }

  .text {
    flex: 1;
  }
</style>
