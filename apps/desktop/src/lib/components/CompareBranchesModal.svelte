<script lang="ts">
  // "Compare branches…": pick any two branches, local or
  // remote; the current branch and its upstream are preselected.
  import { t } from "../i18n.svelte";
  import { openCompare, ui } from "../state.svelte";
  import Icon from "./Icon.svelte";
  import Modal from "./Modal.svelte";

  let { onclose }: { onclose: () => void } = $props();

  const current = $derived(ui.status?.branch ?? ui.repo?.currentBranch ?? null);
  const names = $derived(
    ui.branches.map((b) => b.name).filter((n) => !n.endsWith("/HEAD") && n !== "HEAD"),
  );

  let from = $state("");
  let to = $state("");

  // The branch list can arrive after the dialog opens: fill empty pickers
  // then — the current branch against its upstream, else the next branch.
  $effect(() => {
    const has = (n: string | null | undefined): n is string => !!n && names.includes(n);
    if (!from && has(current)) from = current;
    if (!to) {
      const upstream = ui.status?.upstream;
      to = has(upstream) ? upstream : (names.find((n) => n !== from) ?? "");
    }
  });

  function swap() {
    [from, to] = [to, from];
  }

  function run() {
    openCompare({
      from,
      to,
      fromLabel: from,
      toLabel: to,
      branch: from === current ? to : undefined,
    });
  }
</script>

<Modal title={t("compare.branchesTitle")} width="480px" {onclose}>
  <div class="pick">
    <label>
      <span>{t("compare.from")}</span>
      <select bind:value={from}>
        {#each names as name (name)}<option value={name}>{name}</option>{/each}
      </select>
    </label>
    <button
      class="ghost swap"
      title={t("compare.swap")}
      aria-label={t("compare.swap")}
      onclick={swap}
    >
      <Icon name="swap" size={14} />
    </button>
    <label>
      <span>{t("compare.to")}</span>
      <select bind:value={to}>
        {#each names as name (name)}<option value={name}>{name}</option>{/each}
      </select>
    </label>
  </div>
  {#if from && from === to}
    <p class="hint">{t("compare.same")}</p>
  {/if}
  <div class="actions">
    <button onclick={onclose}>{t("common.cancel")}</button>
    <button class="primary" disabled={!from || !to || from === to} onclick={run}>
      {t("compare.run")}
    </button>
  </div>
</Modal>

<style>
  .pick {
    display: flex;
    align-items: flex-end;
    gap: var(--space-2);
  }

  .pick label {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 12px;
    color: var(--text-muted);
  }

  .swap {
    margin-bottom: 2px;
  }

  .hint {
    color: var(--text-muted);
    font-size: 12px;
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
    margin-top: var(--space-4);
  }
</style>
