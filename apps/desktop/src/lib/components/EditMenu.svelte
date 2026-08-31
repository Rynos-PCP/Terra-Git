<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "../i18n.svelte";
  import Icon from "./Icon.svelte";

  // Our own edit context menu for input fields (M6 browser hardening): replaces
  // the native WebView menu with translated, app-conformant entries.
  // Clipboard access via navigator.clipboard (the Tauri scheme is a secure
  // context); after every mutation an input event is fired so bind:value applies.

  type EditTarget = HTMLInputElement | HTMLTextAreaElement;

  let open = $state(false);
  let x = $state(0);
  let y = $state(0);
  let target = $state<EditTarget | null>(null);
  let hasSelection = $state(false);
  let writable = $state(false);
  let menuEl = $state<HTMLElement>();

  onMount(() => {
    const onContextMenu = (e: MouseEvent) => {
      const el = (e.target as HTMLElement)?.closest?.("input, textarea") as EditTarget | null;
      // Text inputs only (no checkboxes/selects); otherwise the global handler in
      // App.svelte applies (the native menu stays off everywhere).
      if (
        !el ||
        (el instanceof HTMLInputElement &&
          !["text", "password", "number", "search"].includes(el.type))
      ) {
        return;
      }
      e.preventDefault();
      target = el;
      hasSelection = (el.selectionStart ?? 0) !== (el.selectionEnd ?? 0);
      writable = !el.readOnly && !el.disabled;
      // Clamp the position so the menu does not stick out of the window.
      x = Math.min(e.clientX, window.innerWidth - 190);
      y = Math.min(e.clientY, window.innerHeight - 170);
      open = true;
    };
    const onClose = (e: Event) => {
      if (open && menuEl && !menuEl.contains(e.target as Node)) open = false;
    };
    const onKey = (e: KeyboardEvent) => {
      if (open && e.key === "Escape") {
        // Capture phase + preventDefault/stopPropagation: Escape closes only the
        // topmost overlay — a modal below it (window bubble listener) must not
        // see the event any more (it would otherwise lose form input).
        e.preventDefault();
        e.stopPropagation();
        open = false;
        target?.focus();
      }
    };
    window.addEventListener("contextmenu", onContextMenu, true);
    window.addEventListener("pointerdown", onClose);
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("contextmenu", onContextMenu, true);
      window.removeEventListener("pointerdown", onClose);
      window.removeEventListener("keydown", onKey, true);
    };
  });

  function selectedText(el: EditTarget): string {
    return el.value.slice(el.selectionStart ?? 0, el.selectionEnd ?? 0);
  }

  /** Replaces the selection and notifies Svelte (bind:value). */
  function replaceSelection(el: EditTarget, text: string) {
    el.setRangeText(text, el.selectionStart ?? 0, el.selectionEnd ?? 0, "end");
    el.dispatchEvent(new Event("input", { bubbles: true }));
  }

  async function doCut() {
    if (!target || !writable) return;
    await navigator.clipboard.writeText(selectedText(target)).catch(() => {});
    replaceSelection(target, "");
    close();
  }

  async function doCopy() {
    if (!target) return;
    await navigator.clipboard.writeText(selectedText(target)).catch(() => {});
    close();
  }

  async function doPaste() {
    if (!target || !writable) return;
    const text = await navigator.clipboard.readText().catch(() => "");
    if (text) replaceSelection(target, text);
    close();
  }

  function doSelectAll() {
    target?.select();
    close();
  }

  function close() {
    open = false;
    target?.focus();
  }
</script>

{#if open}
  <div class="edit-menu" bind:this={menuEl} role="menu" style:left="{x}px" style:top="{y}px">
    <button
      class="item ghost"
      role="menuitem"
      disabled={!hasSelection || !writable}
      onclick={doCut}
    >
      <Icon name="split" size={13} />
      {t("edit.cut")}
    </button>
    <button class="item ghost" role="menuitem" disabled={!hasSelection} onclick={doCopy}>
      <Icon name="copy" size={13} />
      {t("edit.copy")}
    </button>
    <button class="item ghost" role="menuitem" disabled={!writable} onclick={doPaste}>
      <Icon name="file" size={13} />
      {t("edit.paste")}
    </button>
    <button class="item ghost" role="menuitem" onclick={doSelectAll}>
      <Icon name="check" size={13} />
      {t("edit.selectAll")}
    </button>
  </div>
{/if}

<style>
  .edit-menu {
    position: fixed;
    z-index: 100;
    width: 180px;
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-menu);
    padding: var(--space-2);
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
</style>
