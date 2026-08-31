<script lang="ts">
  import { t } from "../i18n.svelte";
  import Icon from "./Icon.svelte";

  // Compact search/go-to bar (Ctrl+F / Ctrl+G). The match logic belongs to the
  // parent view (diff, blame, …) — only input + navigation here.
  let {
    mode = "text",
    count = 0,
    current = 0,
    onquery,
    onnav,
    onclose,
  }: {
    /** "text" = full-text search (live), "goto" = line number (Enter). */
    mode?: "text" | "goto";
    count?: number;
    /** 1-based current match, 0 = none. */
    current?: number;
    onquery: (q: string) => void;
    onnav: (dir: 1 | -1) => void;
    onclose: () => void;
  } = $props();

  let value = $state("");
  let inputEl = $state<HTMLInputElement>();

  // On a mode change (Ctrl+F -> Ctrl+G) clear the field and refocus it.
  $effect(() => {
    void mode;
    value = "";
    inputEl?.focus();
    inputEl?.select();
  });

  function onKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      onclose();
    } else if (e.key === "Enter") {
      e.preventDefault();
      if (mode === "goto") onquery(value);
      else onnav(e.shiftKey ? -1 : 1);
    }
  }
</script>

<div class="findbar" role="search">
  <Icon name="search" size={13} />
  <input
    bind:this={inputEl}
    bind:value
    type="text"
    placeholder={mode === "goto" ? t("find.gotoPlaceholder") : t("find.searchPlaceholder")}
    inputmode={mode === "goto" ? "numeric" : undefined}
    oninput={() => mode === "text" && onquery(value)}
    onkeydown={onKeydown}
  />
  {#if mode === "text"}
    <span class="count" class:none={count === 0 && value !== ""}>
      {count === 0 ? (value ? t("find.noHits") : "") : `${current}/${count}`}
    </span>
    <button class="ghost" title={t("find.prev")} disabled={count === 0} onclick={() => onnav(-1)}>
      <span class="up"><Icon name="chevronDown" size={12} /></span>
    </button>
    <button class="ghost" title={t("find.next")} disabled={count === 0} onclick={() => onnav(1)}>
      <Icon name="chevronDown" size={12} />
    </button>
  {/if}
  <button class="ghost" title={t("find.close")} onclick={onclose}>
    <Icon name="x" size={12} />
  </button>
</div>

<style>
  .findbar {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 4px 6px 4px 10px;
    background: var(--bg-elevated);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-menu);
    color: var(--text-faint);
  }

  input {
    width: 190px;
    background: transparent;
    border: none;
    box-shadow: none;
    min-height: 24px;
    padding: 2px 4px;
  }

  input:focus {
    border: none;
    box-shadow: none;
  }

  .count {
    font-size: 11px;
    font-variant-numeric: tabular-nums;
    color: var(--text-muted);
    white-space: nowrap;
    min-width: 34px;
    text-align: right;
  }

  .count.none {
    color: var(--deleted);
  }

  .findbar button {
    padding: 2px 5px;
  }

  .up {
    display: inline-flex;
    transform: rotate(180deg);
  }
</style>
