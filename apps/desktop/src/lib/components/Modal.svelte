<script lang="ts">
  import type { Snippet } from "svelte";
  import { t } from "../i18n.svelte";
  import Icon from "./Icon.svelte";

  let {
    title,
    width = "520px",
    onclose,
    children,
  }: {
    title: string;
    width?: string;
    onclose: () => void;
    children: Snippet;
  } = $props();

  const FOCUSABLE =
    'a[href], button:not([disabled]), input:not([disabled]), textarea:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex="-1"])';

  let dialogEl = $state<HTMLElement>();

  // On opening, move the focus into the dialog and give it back on closing so
  // keyboard/screen reader users do not end up in the background (aria-modal).
  $effect(() => {
    const el = dialogEl;
    if (!el) return;
    const previouslyFocused = document.activeElement as HTMLElement | null;
    const first = el.querySelector<HTMLElement>(FOCUSABLE);
    (first ?? el).focus();
    return () => previouslyFocused?.focus?.();
  });

  function onKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      // Already consumed by a higher overlay (palette/edit menu): the modal
      // below stays open (overlay hierarchy).
      if (e.defaultPrevented) return;
      onclose();
      return;
    }
    // Focus trap: redirect Tab at the dialog's edge cyclically.
    if (e.key === "Tab" && dialogEl) {
      const nodes = [...dialogEl.querySelectorAll<HTMLElement>(FOCUSABLE)].filter(
        (n) => n.offsetParent !== null,
      );
      if (nodes.length === 0) {
        e.preventDefault();
        dialogEl.focus();
        return;
      }
      const first = nodes[0];
      const last = nodes[nodes.length - 1];
      const active = document.activeElement;
      if (e.shiftKey && (active === first || !dialogEl.contains(active))) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && active === last) {
        e.preventDefault();
        first.focus();
      }
    }
  }
</script>

<svelte:window onkeydown={onKeydown} />

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="backdrop" onclick={(e) => e.target === e.currentTarget && onclose()}>
  <div
    class="modal"
    style:width
    role="dialog"
    aria-modal="true"
    aria-label={title}
    tabindex="-1"
    bind:this={dialogEl}
  >
    <header>
      <h2>{title}</h2>
      <button class="ghost" onclick={onclose} aria-label={t("common.close")}
        ><Icon name="x" /></button
      >
    </header>
    <div class="body">
      {@render children()}
    </div>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    background: var(--overlay);
    -webkit-backdrop-filter: blur(4px);
    backdrop-filter: blur(4px);
    display: flex;
    align-items: flex-start;
    justify-content: center;
    padding-top: 10vh;
    z-index: 80;
    animation: fade 0.15s ease;
  }

  @keyframes fade {
    from {
      opacity: 0;
    }
  }

  .modal {
    background: var(--bg-panel);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-xl);
    box-shadow: var(--shadow-modal);
    max-width: calc(100vw - 48px);
    max-height: 80vh;
    display: flex;
    flex-direction: column;
    animation: pop 0.18s cubic-bezier(0.2, 0.9, 0.3, 1);
  }

  @keyframes pop {
    from {
      opacity: 0;
      transform: translateY(-10px) scale(0.985);
    }
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-3) var(--space-4);
    border-bottom: 1px solid var(--border);
  }

  h2 {
    font-size: 14px;
    font-weight: 650;
  }

  .body {
    padding: var(--space-4);
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
</style>
