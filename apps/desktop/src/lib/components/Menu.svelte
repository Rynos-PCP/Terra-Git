<script lang="ts">
  import type { Snippet } from "svelte";

  // Generic dropdown: a trigger snippet + content; closes on a click outside.
  // open is bindable so parents can close it programmatically.
  let {
    open = $bindable(false),
    align = "left",
    direction = "down",
    width = "280px",
    role = "menu",
    ariaLabel,
    trigger,
    children,
  }: {
    open?: boolean;
    align?: "left" | "right";
    /** "up" opens the menu above the trigger (for triggers at the bottom edge). */
    direction?: "down" | "up";
    width?: string;
    /**
     * ARIA role of the popup. Default "menu" (the entries then have to carry
     * role="menuitem"). Popups with input fields instead of entries — e.g.
     * BranchMenu — set "dialog": a role="menu" with <input> children would be
     * ARIA-invalid.
     */
    role?: "menu" | "dialog";
    /** Mandatory with role="dialog": names the popup for screen readers. */
    ariaLabel?: string;
    trigger: Snippet<[{ toggle: () => void; open: boolean }]>;
    children: Snippet;
  } = $props();

  let root: HTMLElement;
  let menuEl = $state<HTMLElement>();
  let lastFocused: HTMLElement | null = null;

  function toggle() {
    // Remember the trigger focus when opening (for the Escape return).
    if (!open) lastFocused = document.activeElement as HTMLElement | null;
    open = !open;
  }

  // Full keyboard coverage (a11y): on opening, focus the first focusable
  // element (filter input or entry) …
  $effect(() => {
    if (open) menuEl?.querySelector<HTMLElement>("input, button.item")?.focus();
  });

  // … and navigate through the entries with the arrow keys/Home/End.
  function onMenuKeydown(e: KeyboardEvent) {
    if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(e.key)) return;
    const items = [...(menuEl?.querySelectorAll<HTMLElement>("button.item:not([disabled])") ?? [])];
    if (items.length === 0) return;
    e.preventDefault();
    const i = items.indexOf(document.activeElement as HTMLElement);
    const next =
      e.key === "ArrowDown"
        ? items[(i + 1) % items.length]
        : e.key === "ArrowUp"
          ? items[(i - 1 + items.length) % items.length]
          : e.key === "Home"
            ? items[0]
            : items[items.length - 1];
    next.focus();
  }

  function onWindowPointerDown(e: PointerEvent) {
    if (open && root && !root.contains(e.target as Node)) open = false;
  }

  function onWindowKeydown(e: KeyboardEvent) {
    if (open && e.key === "Escape") {
      open = false;
      lastFocused?.focus?.();
    }
  }
</script>

<svelte:window onpointerdown={onWindowPointerDown} onkeydown={onWindowKeydown} />

<div class="menu-root" bind:this={root}>
  {@render trigger({ toggle, open })}
  {#if open}
    <div
      class="menu"
      style:width
      class:right={align === "right"}
      class:up={direction === "up"}
      {role}
      aria-label={ariaLabel}
      tabindex="-1"
      bind:this={menuEl}
      onkeydown={onMenuKeydown}
      onclick={(e) => {
        // Menu entries (button.item) close the menu automatically.
        if ((e.target as HTMLElement).closest("button.item")) open = false;
      }}
    >
      {@render children()}
    </div>
  {/if}
</div>

<style>
  .menu-root {
    position: relative;
    display: inline-flex;
  }

  .menu {
    position: absolute;
    top: calc(100% + 6px);
    left: 0;
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-menu);
    padding: var(--space-2);
    z-index: 60;
    display: flex;
    flex-direction: column;
    gap: 2px;
    max-height: 70vh;
    overflow-y: auto;
  }

  .menu.right {
    left: auto;
    right: 0;
  }

  .menu.up {
    top: auto;
    bottom: calc(100% + 6px);
  }
</style>
