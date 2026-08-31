<script lang="ts">
  import { t, tn } from "../i18n.svelte";
  import { markBisect, resetBisect, ui } from "../state.svelte";
  import Icon from "./Icon.svelte";

  const active = $derived(ui.status?.opState === "bisect");
  // While a mark/reset is running, disable the buttons — otherwise a double
  // click marks two commits with one verdict and falsifies the search.
  const busy = $derived(ui.working > 0);
  const firstBad = $derived(ui.bisect.firstBad);
  // The currently checked-out commit = HEAD = ui.history[0] (after loadMoreHistory).
  const current = $derived(ui.history[0]);
  const badCommit = $derived(
    firstBad
      ? (ui.history.find((c) => c.id.startsWith(firstBad) || firstBad.startsWith(c.id)) ?? null)
      : null,
  );
</script>

{#if active}
  <div class="banner" class:found={!!firstBad} role="alert">
    <Icon name="search" />
    {#if firstBad}
      <div class="text">
        <strong>{t("bisect.firstBad")}</strong>
        <code>{firstBad.slice(0, 8)}</code>
        {#if badCommit}— {badCommit.summary}{/if}
      </div>
      <button class="primary" onclick={resetBisect} disabled={busy}>{t("bisect.finish")}</button>
    {:else}
      <div class="text">
        <strong>{t("bisect.running")}</strong>
        {#if ui.bisect.stepsLeft != null}— {tn("bisect.stepsLeft", ui.bisect.stepsLeft)}{/if}
        {#if current}<br /><code>{current.shortId}</code> {current.summary}{/if}
      </div>
      <button class="good" onclick={() => markBisect("good")} disabled={busy}
        >{t("bisect.markGood")}</button
      >
      <button class="danger" onclick={() => markBisect("bad")} disabled={busy}
        >{t("bisect.markBad")}</button
      >
      <button class="ghost" onclick={() => markBisect("skip")} disabled={busy}
        >{t("bisect.skip")}</button
      >
      <button class="ghost" onclick={resetBisect} disabled={busy}>{t("bisect.abort")}</button>
    {/if}
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

  .banner.found {
    background: var(--del-bg);
  }

  .text {
    flex: 1;
  }

  code {
    font-family: var(--mono);
  }

  .good {
    background: var(--add-bg, #2ea04322);
    color: var(--text-primary);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 4px 10px;
    cursor: pointer;
  }

  .banner button:disabled {
    opacity: 0.5;
    cursor: default;
  }
</style>
