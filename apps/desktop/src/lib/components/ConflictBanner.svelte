<script lang="ts">
  import { t, tn } from "../i18n.svelte";
  import { abortOperation, continueOperation, openConflicts, ui } from "../state.svelte";
  import Icon from "./Icon.svelte";

  // Operation names are git terms and are not translated.
  const labels: Record<string, string> = {
    merge: "Merge",
    rebase: "Rebase",
    cherrypick: "Cherry-pick",
    revert: "Revert",
  };

  const opState = $derived(ui.status?.opState ?? "clean");
  const conflicts = $derived(
    (ui.status?.unstaged ?? []).filter((e) => e.kind === "conflicted").length,
  );
</script>

{#if opState !== "clean" && opState !== "bisect"}
  <div class="banner" class:conflicts={conflicts > 0} role="alert">
    <Icon name="merge" />
    <div class="text">
      <strong>{t("conflict.opRunning", { op: labels[opState] })}</strong>
      {#if conflicts > 0}
        — {tn("conflict.filesConflicted", conflicts)}
      {:else}
        — {t("conflict.allResolved")}
      {/if}
    </div>
    {#if conflicts > 0}
      <!-- With open conflicts the workshop is the primary route. -->
      <button class="primary" onclick={openConflicts}>
        {t("conflictws.open")}
      </button>
    {/if}
    <button class="primary" disabled={conflicts > 0} onclick={continueOperation}>
      {t("conflict.continue")}
    </button>
    <button class="danger" onclick={abortOperation}>{t("conflict.abort")}</button>
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
  }

  .banner.conflicts {
    background: var(--del-bg);
  }

  .text {
    flex: 1;
  }
</style>
