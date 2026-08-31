<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { t } from "./lib/i18n.svelte";
  import {
    autoFetchTick,
    browseForRepo,
    gitFetch,
    gitPull,
    gitPush,
    loadRecents,
    markHistoryPrepared,
    openRepo,
    redoLast,
    refreshStatus,
    ui,
    undoLast,
  } from "./lib/state.svelte";
  import { nextTab } from "./lib/tabNav";
  import BisectBanner from "./lib/components/BisectBanner.svelte";
  import ChangesPanel from "./lib/components/ChangesPanel.svelte";
  import CloneView from "./lib/components/CloneView.svelte";
  import CommandPalette from "./lib/components/CommandPalette.svelte";
  import CommitsWorkshopView from "./lib/components/CommitsWorkshopView.svelte";
  import ConflictBanner from "./lib/components/ConflictBanner.svelte";
  import ConflictsWorkshopView from "./lib/components/ConflictsWorkshopView.svelte";
  import EditMenu from "./lib/components/EditMenu.svelte";
  import HistoryPanel from "./lib/components/HistoryPanel.svelte";
  import Icon from "./lib/components/Icon.svelte";
  import Modals from "./lib/components/Modals.svelte";
  import PipelineView from "./lib/components/PipelineView.svelte";
  import SettingsView from "./lib/components/SettingsView.svelte";
  import Toast from "./lib/components/Toast.svelte";
  import Toolbar from "./lib/components/Toolbar.svelte";
  import WelcomeView from "./lib/components/WelcomeView.svelte";

  onMount(() => {
    loadRecents();

    // Primary: the file watcher reports workdir/ref changes immediately
    // (Rust event "repo-changed", debounced). Operations of our own refresh
    // themselves — do not do it twice then.
    const unlistenPromise = listen<string>("repo-changed", (event) => {
      if (ui.repo && event.payload === ui.repo.path && !ui.busy && ui.working === 0) {
        refreshStatus(true);
      }
    });

    // Commit-graph maintenance finished (open_repository background task):
    // clears the "preparing history" hint.
    const unlistenPreparedPromise = listen<string>("history-prepared", (event) => {
      markHistoryPrepared(event.payload);
    });

    // Coarse fallback poll (60 s): only catches the remaining cases in which the
    // watcher delivers nothing at all. Deliberately rare, because the file
    // watcher now has a poll fallback of its own (PollWatcher on an inotify
    // limit) and is therefore the reliable primary route — every poll refresh
    // spawns a git status process on large worktrees (fast path), so as rarely
    // as defensible.
    const interval = setInterval(() => {
      if (ui.repo && !ui.busy && ui.working === 0) refreshStatus(true);
    }, 60000);

    // Auto fetch (opt-in, every 5 min): keeps ahead/behind current.
    const fetchInterval = setInterval(() => {
      if (ui.autoFetch) autoFetchTick();
    }, 300000);

    // ---- Suppress browser behavior: the app is not a website ----
    // Spellcheck inherits from body onto every input field (red squiggles off).
    document.body.spellcheck = false;
    // Turn off the native WebView context menu entirely — text inputs get their
    // own translated edit menu (EditMenu.svelte, capture phase).
    const onContextMenu = (e: MouseEvent) => e.preventDefault();
    const onKeydown = (e: KeyboardEvent) => {
      const ctrl = e.ctrlKey || e.metaKey;
      const k = e.key.toLowerCase();
      // Block devtools in release builds (they stay open in dev builds).
      // The devtools feature is not enabled in Cargo.toml anyway — this block
      // is defense in depth against the WebView's default shortcuts.
      if (
        import.meta.env.PROD &&
        (k === "f12" || (ctrl && e.shiftKey && ["i", "j", "c"].includes(k)))
      ) {
        e.preventDefault();
        return;
      }
      // Repurpose reload sensibly: F5/Ctrl+R = refresh the status, not a WebView reload.
      if (k === "f5" || (ctrl && k === "r")) {
        e.preventDefault();
        if (ui.repo && !ui.busy && ui.working === 0) refreshStatus(true);
        return;
      }
      // Sync shortcuts (the global shortcut layer, M2): Ctrl+Shift+F/U/P.
      // MUST come before the Ctrl+P/U blocking and the Ctrl+F search.
      if (ctrl && e.shiftKey && ui.repo && !ui.modal && !ui.busy && ui.working === 0) {
        if (k === "f") {
          e.preventDefault();
          gitFetch();
          return;
        }
        if (k === "u" && ui.status?.upstream) {
          e.preventDefault();
          gitPull();
          return;
        }
        if (k === "p") {
          e.preventDefault();
          gitPush();
          return;
        }
      }
      // Print, view-source, browser zoom, history navigation: off.
      if (ctrl && ["p", "u", "+", "-", "=", "0"].includes(k)) {
        e.preventDefault();
        return;
      }
      if (e.altKey && (k === "arrowleft" || k === "arrowright")) {
        e.preventDefault();
        return;
      }
      // Command palette (Ctrl/Cmd+K) — like Ctrl+O, not across an open modal.
      // The palette would otherwise lay itself OVER it (z-index 90 vs. backdrop
      // 80), and its navigation commands would switch the view out from under
      // the modal: the jump would look inconsequential, and in the conflict
      // editor the unsaved manual work would be lost.
      if (ctrl && k === "k") {
        e.preventDefault();
        if (!ui.modal) window.dispatchEvent(new CustomEvent("app-palette"));
        return;
      }
      // Open repository (Ctrl/Cmd+O) — the fastest way from the welcome screen
      // as well as from the workspace; not across an open modal.
      if (ctrl && k === "o") {
        e.preventDefault();
        if (!ui.modal && !ui.busy && !ui.cloning) browseForRepo();
        return;
      }
      // Multi-level undo/redo — only outside input fields (the WebView's native
      // text undo applies there) and not with a modal open: a repo undo would
      // be a destructive surprise there.
      const inEditable = (e.target as HTMLElement)?.closest?.(
        "input, textarea, [contenteditable='true']",
      );
      if (ctrl && !inEditable && ui.repo && !ui.modal) {
        if (k === "z" && !e.shiftKey) {
          e.preventDefault();
          undoLast();
          return;
        }
        if (k === "y" || (k === "z" && e.shiftKey)) {
          e.preventDefault();
          redoLast();
          return;
        }
      }
      // Our own search instead of the browser search.
      if (ctrl && k === "f") {
        e.preventDefault();
        window.dispatchEvent(new CustomEvent("app-find"));
        return;
      }
      if (ctrl && k === "g") {
        e.preventDefault();
        window.dispatchEvent(new CustomEvent("app-goto"));
        return;
      }
      if (k === "f3") {
        e.preventDefault();
        window.dispatchEvent(new CustomEvent("app-find-next", { detail: e.shiftKey ? -1 : 1 }));
      }
    };
    const onWheel = (e: WheelEvent) => {
      if (e.ctrlKey) e.preventDefault(); // Ctrl+mouse-wheel zoom of the WebView
    };
    // Prevent drag & drop navigation: a file/URL dragged into the app must
    // never navigate the WebView away from the interface.
    const onDrag = (e: DragEvent) => e.preventDefault();

    // Native folder drop (Tauri): a folder dragged onto the window is opened as
    // a repo. Not a repo folder -> open_repository reports it as a toast.
    // In mock/browser mode there is no Tauri webview: skip silently.
    let unlistenDrop: (() => void) | undefined;
    try {
      getCurrentWebview()
        .onDragDropEvent((event) => {
          if (event.payload.type !== "drop" || ui.busy || ui.cloning || ui.modal) return;
          const first = event.payload.paths[0];
          if (first) openRepo(first);
        })
        .then((un) => (unlistenDrop = un))
        .catch(() => {});
    } catch {
      // no Tauri context (e.g. mock.html in the browser)
    }

    window.addEventListener("contextmenu", onContextMenu);
    window.addEventListener("keydown", onKeydown, true);
    window.addEventListener("wheel", onWheel, { passive: false });
    window.addEventListener("dragover", onDrag);
    window.addEventListener("drop", onDrag);

    return () => {
      clearInterval(interval);
      clearInterval(fetchInterval);
      unlistenPromise.then((unlisten) => unlisten());
      unlistenPreparedPromise.then((unlisten) => unlisten());
      unlistenDrop?.();
      window.removeEventListener("contextmenu", onContextMenu);
      window.removeEventListener("keydown", onKeydown, true);
      window.removeEventListener("wheel", onWheel);
      window.removeEventListener("dragover", onDrag);
      window.removeEventListener("drop", onDrag);
    };
  });

  // Mirror accessibility onto the root element: UI scaling (zoom also scales px
  // sizes), forced reduced motion, increased contrast.
  $effect(() => {
    const root = document.documentElement;
    root.style.setProperty("zoom", String(ui.uiScale));
    root.dataset.reduceMotion = ui.reduceMotion ? "on" : "off";
    root.dataset.contrast = ui.highContrast ? "high" : "normal";
  });

  // Mirror the theme onto the root element; "system" follows the OS theme (live).
  $effect(() => {
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const apply = () => {
      document.documentElement.dataset.theme =
        ui.theme === "system" ? (mq.matches ? "dark" : "light") : ui.theme;
    };
    apply();
    if (ui.theme === "system") {
      mq.addEventListener("change", apply);
      return () => mq.removeEventListener("change", apply);
    }
  });

  const changeCount = $derived((ui.status?.staged.length ?? 0) + (ui.status?.unstaged.length ?? 0));

  // Arrow keys in the tab strip (WAI-ARIA tabs, automatic activation).
  // Deliberately on the container instead of globally: the global keydown
  // handler only touches arrow keys together with Alt and there calls ONLY
  // preventDefault — the event bubbles up here. The guard sits in nextTab().
  function onTabsKeydown(e: KeyboardEvent) {
    const next = nextTab(ui.tab, e);
    if (!next) return;
    e.preventDefault();
    ui.tab = next;
    (e.currentTarget as HTMLElement).querySelector<HTMLElement>(`#app-tab-${next}`)?.focus();
  }
</script>

<div class="shell">
  {#if !ui.repo}
    <WelcomeView />
  {:else if ui.view === "settings"}
    <Toolbar />
    <CloneView />
    <SettingsView />
  {:else if ui.view === "commits"}
    <Toolbar />
    <CloneView />
    <CommitsWorkshopView />
  {:else if ui.view === "conflicts"}
    <Toolbar />
    <CloneView />
    <ConflictsWorkshopView />
  {:else if ui.view === "pipeline"}
    <Toolbar />
    <CloneView />
    <PipelineView />
  {:else}
    <Toolbar />
    <CloneView />
    <ConflictBanner />
    <BisectBanner />
    <div
      class="tabs"
      role="tablist"
      tabindex="-1"
      aria-label={t("app.tabs.label")}
      onkeydown={onTabsKeydown}
    >
      <button
        class="tab"
        role="tab"
        id="app-tab-changes"
        aria-controls="app-tabpanel"
        aria-selected={ui.tab === "changes"}
        tabindex={ui.tab === "changes" ? 0 : -1}
        class:active={ui.tab === "changes"}
        onclick={() => (ui.tab = "changes")}
      >
        <Icon name="edit" size={14} />
        {t("app.tab.changes")}
        {#if changeCount > 0}
          <span class="badge">{changeCount}</span>
        {/if}
      </button>
      <button
        class="tab"
        role="tab"
        id="app-tab-history"
        aria-controls="app-tabpanel"
        aria-selected={ui.tab === "history"}
        tabindex={ui.tab === "history" ? 0 : -1}
        class:active={ui.tab === "history"}
        onclick={() => (ui.tab = "history")}
      >
        <Icon name="history" size={14} />
        {t("app.tab.history")}
      </button>
    </div>
    <main class="content">
      <div class="tabpanel" id="app-tabpanel" role="tabpanel" aria-labelledby="app-tab-{ui.tab}">
        {#if ui.tab === "changes"}
          <ChangesPanel />
        {:else}
          <HistoryPanel />
        {/if}
      </div>
    </main>
  {/if}
  <Toast />
  <Modals />
  <CommandPalette />
  <EditMenu />
</div>

<style>
  .shell {
    display: flex;
    flex-direction: column;
    height: 100%;
  }

  .tabs {
    display: flex;
    gap: 2px;
    padding: 0 var(--space-3);
    background: var(--bg-panel);
    border-bottom: 1px solid var(--border);
  }

  .tab {
    background: transparent;
    border: none;
    border-bottom: 2px solid transparent;
    border-radius: 0;
    box-shadow: none;
    color: var(--text-muted);
    padding: 9px 14px 7px;
    display: flex;
    align-items: center;
    gap: 7px;
  }

  .tab:hover:not(.active) {
    color: var(--text-primary);
    background: transparent;
    border-bottom-color: var(--border-strong);
  }

  .tab:active {
    background: transparent;
  }

  .tab.active {
    color: var(--text-primary);
    border-bottom-color: var(--accent);
  }

  .tab.active .badge {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    border-color: transparent;
    color: var(--accent);
  }

  .content {
    flex: 1;
    min-height: 0;
  }

  /* The tabpanel wrapper must not break the height chain: the
     panels (.split in ChangesPanel/HistoryPanel) count on height:100%. */
  .tabpanel {
    height: 100%;
  }
</style>
