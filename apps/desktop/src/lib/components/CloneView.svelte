<script lang="ts">
  import { t } from "../i18n.svelte";
  import { cancelRemoteOp, ui } from "../state.svelte";
  import Icon from "./Icon.svelte";
</script>

<!--
  Non-blocking clone banner: appears above the (already opened) repo view while
  the download runs in the background. Self-gated through ui.cloning — it can
  therefore be included unconditionally.
-->
{#if ui.cloning}
  <div class="clone-banner" role="status">
    <Icon name="external" size={15} />
    <span class="label">{t("clone.cloning", { name: ui.cloning })}</span>
    <div class="track" class:indeterminate={!ui.progress}>
      <div class="fill" style:width="{ui.progress?.percent ?? 0}%"></div>
    </div>
    <span class="phase">
      {#if ui.progress}
        {t(`progress.${ui.progress.phase}` as Parameters<typeof t>[0])} · {ui.progress.percent}%
      {:else}
        {t("clone.connecting")}
      {/if}
    </span>
    <button class="cancel" onclick={cancelRemoteOp}>{t("state.cancel")}</button>
  </div>
{/if}

<style>
  .clone-banner {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: 6px var(--space-3);
    background: color-mix(in srgb, var(--accent) 8%, transparent);
    border-bottom: 1px solid color-mix(in srgb, var(--accent) 25%, var(--border));
    color: var(--text);
    font-size: 12.5px;
  }

  .label {
    font-weight: 600;
    white-space: nowrap;
  }

  .track {
    flex: 1;
    max-width: 320px;
    height: 5px;
    border-radius: 999px;
    overflow: hidden;
    background: color-mix(in srgb, var(--accent) 18%, transparent);
  }

  .fill {
    height: 100%;
    background: var(--accent);
    border-radius: 999px;
    transition: width 0.15s ease;
    position: relative;
    overflow: hidden;
  }

  /* Permanent shimmer: a sign of life even between two progress
     events (the same pattern as the toolbar). */
  .fill::after {
    content: "";
    position: absolute;
    inset: 0;
    background: linear-gradient(
      90deg,
      transparent,
      color-mix(in srgb, var(--accent-text) 45%, transparent),
      transparent
    );
    transform: translateX(-100%);
    animation: sheen 1.4s linear infinite;
  }

  @keyframes sheen {
    to {
      transform: translateX(100%);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .fill::after {
      animation: none;
      content: none;
    }
  }

  .track.indeterminate .fill {
    width: 35% !important;
    animation: indet 1.1s ease-in-out infinite;
  }

  @keyframes indet {
    0% {
      margin-left: -35%;
    }
    100% {
      margin-left: 100%;
    }
  }

  .phase {
    color: var(--text-muted);
    white-space: nowrap;
  }

  .cancel {
    margin-left: auto;
    padding: 3px 12px;
    color: var(--text);
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    cursor: pointer;
    white-space: nowrap;
  }
  .cancel:hover {
    border-color: var(--danger, #c0392b);
    color: var(--danger, #c0392b);
  }
</style>
