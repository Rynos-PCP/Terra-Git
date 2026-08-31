<script lang="ts">
  import { onMount } from "svelte";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { api } from "../api";
  import { filterCommands } from "../commandFilter";
  import { workshopAvailable } from "../conflictWorkshop";
  import { type MessageKey, setLang, t } from "../i18n.svelte";
  import {
    closeRepo,
    gitFetch,
    gitPull,
    gitPush,
    gitPushForce,
    openConflicts,
    openPipeline,
    openRepo,
    openWorkshop,
    prProvider,
    redoLast,
    savePrefs,
    showError,
    switchBranch,
    ui,
    undoLabel,
    undoLast,
  } from "../state.svelte";
  import Icon from "./Icon.svelte";

  interface Command {
    id: string;
    label: string;
    hint?: string;
    /** Group for the intermediate headings. */
    group: string;
    icon: string;
    run: () => void | Promise<void>;
  }

  let open = $state(false);
  let query = $state("");
  let selected = $state(0);
  let inputEl = $state<HTMLInputElement>();
  let listEl = $state<HTMLElement>();
  let paletteEl = $state<HTMLElement>();

  const FOCUSABLE =
    'a[href], button:not([disabled]), input:not([disabled]), textarea:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex="-1"])';

  const idle = $derived(!ui.busy && ui.working === 0);

  /** Available commands, depending on the app state (repo open, idle, …). */
  const commands = $derived.by<Command[]>(() => {
    const cmds: Command[] = [];
    if (ui.repo) {
      if (idle) {
        cmds.push(
          {
            id: "fetch",
            group: "sync",
            label: t("toolbar.fetch"),
            hint: "git fetch · Ctrl+Shift+F",
            icon: "refresh",
            run: gitFetch,
          },
          {
            id: "pull",
            group: "sync",
            label: t("toolbar.pull"),
            hint: "git pull · Ctrl+Shift+U",
            icon: "arrowDown",
            run: gitPull,
          },
          {
            id: "push",
            group: "sync",
            label: t("toolbar.push"),
            hint: "git push · Ctrl+Shift+P",
            icon: "arrowUp",
            run: gitPush,
          },
          {
            id: "force-push",
            group: "sync",
            label: t("palette.forcePush"),
            hint: "--force-with-lease",
            icon: "arrowUp",
            run: gitPushForce,
          },
        );
      }
      if (idle && ui.undoStatus?.undo) {
        cmds.push({
          id: "undo",
          group: "tools",
          label: `${t("undo.undo")}: ${undoLabel(ui.undoStatus.undo)}`,
          hint: "Ctrl+Z",
          icon: "undo",
          run: undoLast,
        });
      }
      if (idle && ui.undoStatus?.redo) {
        cmds.push({
          id: "redo",
          group: "tools",
          label: `${t("undo.redo")}: ${undoLabel(ui.undoStatus.redo)}`,
          hint: "Ctrl+Y",
          icon: "redo",
          run: redoLast,
        });
      }
      cmds.push({
        id: "workshop",
        group: "views",
        label: t("workshop.open"),
        icon: "edit",
        run: () => openWorkshop(),
      });
      cmds.push({
        id: "pipeline",
        group: "views",
        label: t("pipe.menu"),
        icon: "terminal",
        run: () => openPipeline(),
      });
      // Conflict workshop: only meaningful during a multi-step operation —
      // without one the view throws you straight back. The fixed slot in the
      // tools menu shows it always (greyed out there).
      if (workshopAvailable(ui.status?.opState)) {
        cmds.push({
          id: "conflicts",
          group: "views",
          label: t("conflictws.openLong"),
          icon: "merge",
          run: openConflicts,
        });
      }
      cmds.push(
        {
          id: "tab-changes",
          group: "views",
          label: t("palette.viewChanges"),
          icon: "edit",
          run: () => {
            ui.view = "repo";
            ui.tab = "changes";
          },
        },
        {
          id: "tab-history",
          group: "views",
          label: t("palette.viewHistory"),
          icon: "history",
          run: () => {
            ui.view = "repo";
            ui.tab = "history";
          },
        },
        {
          id: "stash-push",
          group: "manage",
          label: t("palette.stashChanges"),
          icon: "stash",
          run: () => {
            ui.modal = { kind: "stashPush" };
          },
        },
        {
          id: "stash",
          group: "manage",
          label: t("palette.manageStashes"),
          icon: "stash",
          run: () => {
            ui.modal = { kind: "stash" };
          },
        },
        {
          id: "tags",
          group: "manage",
          label: t("toolbar.manageTags"),
          icon: "tag",
          run: () => {
            ui.modal = { kind: "tags" };
          },
        },
        {
          id: "remotes",
          group: "manage",
          label: t("toolbar.manageRemotes"),
          icon: "globe",
          run: () => {
            ui.modal = { kind: "remotes" };
          },
        },
        {
          id: "backups",
          group: "manage",
          label: t("toolbar.backups"),
          icon: "undo",
          run: () => {
            ui.modal = { kind: "backups" };
          },
        },
        {
          id: "submodules",
          group: "manage",
          label: t("toolbar.submodules"),
          icon: "tree",
          run: () => {
            ui.modal = { kind: "submodules" };
          },
        },
        {
          id: "worktrees",
          group: "manage",
          label: t("toolbar.worktrees"),
          icon: "folder",
          run: () => {
            ui.modal = { kind: "worktrees" };
          },
        },
        {
          id: "change-requests",
          group: "manage",
          label: t("palette.changeRequests", {
            label: prProvider() === "gitlab" ? "Merge Requests" : "Pull Requests",
          }),
          icon: "pr",
          run: () => {
            ui.modal = { kind: "changeRequests" };
          },
        },
        {
          id: "pr",
          group: "manage",
          label: t("palette.createPr", {
            label: prProvider() === "gitlab" ? "Merge Request" : "Pull Request",
          }),
          icon: "pr",
          run: () => {
            ui.modal = { kind: "createCr" };
          },
        },
        {
          id: "explorer",
          group: "open",
          label: t("palette.openFileManager"),
          icon: "folder",
          run: () => api.openInExplorer(ui.repo!.path).catch(showError),
        },
        {
          id: "terminal",
          group: "open",
          label: t("toolbar.openTerminal"),
          icon: "terminal",
          run: () => api.openTerminal(ui.repo!.path).catch(showError),
        },
        {
          id: "editor",
          group: "open",
          label: t("toolbar.openEditor"),
          icon: "external",
          run: () => api.openInEditor(ui.repo!.path, ui.editorCmd || null).catch(showError),
        },
      );
      if (idle) {
        for (const b of ui.branches) {
          if (b.isRemote || b.isHead) continue;
          cmds.push({
            id: `branch-${b.name}`,
            group: "branches",
            label: t("palette.switchBranch", { name: b.name }),
            icon: "branch",
            run: () => switchBranch(b.name),
          });
        }
      }
      cmds.push({
        id: "close-repo",
        group: "app",
        label: t("palette.closeRepo"),
        icon: "x",
        run: closeRepo,
      });
    }
    cmds.push(
      {
        id: "open-repo",
        group: "app",
        label: t("palette.openRepo"),
        icon: "folder",
        run: async () => {
          const dir = await openDialog({ directory: true, title: t("dialog.openRepo") });
          if (typeof dir === "string") await openRepo(dir);
        },
      },
      {
        id: "clone",
        group: "app",
        label: t("toolbar.clone"),
        icon: "arrowDown",
        run: () => {
          ui.modal = { kind: "clone" };
        },
      },
      {
        id: "init",
        group: "app",
        label: t("toolbar.init"),
        icon: "plus",
        run: () => {
          ui.modal = { kind: "init" };
        },
      },
      {
        id: "settings",
        group: "app",
        label: t("nav.settings"),
        icon: "settings",
        run: () => {
          ui.view = "settings";
        },
      },
      {
        id: "theme-dark",
        group: "app",
        label: `${t("theme.title")}: ${t("theme.dark")}`,
        icon: "moon",
        run: () => {
          ui.theme = "dark";
          savePrefs();
        },
      },
      {
        id: "theme-light",
        group: "app",
        label: `${t("theme.title")}: ${t("theme.light")}`,
        icon: "sun",
        run: () => {
          ui.theme = "light";
          savePrefs();
        },
      },
      {
        id: "theme-system",
        group: "app",
        label: `${t("theme.title")}: ${t("theme.system")}`,
        icon: "window",
        run: () => {
          ui.theme = "system";
          savePrefs();
        },
      },
      {
        id: "lang-en",
        group: "app",
        label: t("palette.switchLangEn"),
        icon: "globe",
        run: () => setLang("en"),
      },
      {
        id: "lang-de",
        group: "app",
        label: t("palette.switchLangDe"),
        icon: "globe",
        run: () => setLang("de"),
      },
    );
    return cmds;
  });

  const filtered = $derived(filterCommands(commands, query));

  /** Display names of the command groups (the order comes from the builder). */
  const GROUP_LABELS: Record<string, MessageKey> = {
    sync: "palette.groupSync",
    tools: "toolbar.tools",
    views: "palette.groupViews",
    manage: "toolbar.manage",
    open: "toolbar.open",
    branches: "palette.groupBranches",
    app: "palette.groupApp",
  };

  function openPalette() {
    query = "";
    selected = 0;
    open = true;
    // The toast lies above us but cannot get past our tab trap — it should hold
    // back its actions for that long.
    ui.paletteOpen = true;
  }

  function closePalette() {
    open = false;
    ui.paletteOpen = false;
  }

  // Ctrl/Cmd+K is intercepted centrally in App.svelte and broadcast as an event.
  onMount(() => {
    const onPaletteEvent = () => (open ? closePalette() : openPalette());
    window.addEventListener("app-palette", onPaletteEvent);
    return () => window.removeEventListener("app-palette", onPaletteEvent);
  });

  // On opening, move the focus into the search field and give it back to the
  // previously focused element on closing (aria-modal — keyboard/screen reader
  // users must not end up in the background).
  $effect(() => {
    if (!open) return;
    const previouslyFocused = document.activeElement as HTMLElement | null;
    inputEl?.focus();
    return () => previouslyFocused?.focus?.();
  });

  // Escape centrally in the capture phase: closes ONLY the palette and stops the
  // event before an underlying modal (a bubble listener on window) sees it — the
  // palette can lie over an open modal. It also applies when the focus is not in
  // the search field. defaultPrevented respects an overlay that grabbed it even
  // earlier (the edit menu).
  function onEscapeCapture(e: KeyboardEvent) {
    if (!open || e.key !== "Escape" || e.defaultPrevented) return;
    e.preventDefault();
    e.stopPropagation();
    closePalette();
  }

  // Focus trap: redirect Tab at the palette's edge cyclically (only when open).
  function onPaletteKeydown(e: KeyboardEvent) {
    if (!open || e.key !== "Tab" || !paletteEl) return;
    const nodes = [...paletteEl.querySelectorAll<HTMLElement>(FOCUSABLE)].filter(
      (n) => n.offsetParent !== null,
    );
    if (nodes.length === 0) return;
    const first = nodes[0];
    const last = nodes[nodes.length - 1];
    const active = document.activeElement;
    if (e.shiftKey && (active === first || !paletteEl.contains(active))) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && active === last) {
      e.preventDefault();
      first.focus();
    }
  }

  // Keep the selection inside the filtered range.
  $effect(() => {
    if (selected >= filtered.length) selected = Math.max(0, filtered.length - 1);
  });

  async function runSelected() {
    const cmd = filtered[selected];
    if (!cmd) return;
    closePalette();
    await cmd.run();
  }

  function onInputKeydown(e: KeyboardEvent) {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      selected = Math.min(selected + 1, filtered.length - 1);
      scrollSelectedIntoView();
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      selected = Math.max(selected - 1, 0);
      scrollSelectedIntoView();
    } else if (e.key === "Enter") {
      e.preventDefault();
      runSelected();
    }
    // Escape is handled by onEscapeCapture (window, capture phase) — it never
    // arrives here.
  }

  function scrollSelectedIntoView() {
    // Let it render after the state update, then scroll.
    requestAnimationFrame(() => {
      listEl?.querySelector<HTMLElement>(".cmd.selected")?.scrollIntoView({ block: "nearest" });
    });
  }
</script>

<svelte:window onkeydown={onPaletteKeydown} onkeydowncapture={onEscapeCapture} />

{#if open}
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="backdrop" onclick={(e) => e.target === e.currentTarget && closePalette()}>
    <div
      class="palette"
      role="dialog"
      aria-modal="true"
      aria-label={t("palette.title")}
      bind:this={paletteEl}
    >
      <div class="search">
        <Icon name="search" size={14} />
        <input
          type="text"
          placeholder={t("palette.placeholder")}
          bind:value={query}
          bind:this={inputEl}
          oninput={() => (selected = 0)}
          onkeydown={onInputKeydown}
        />
        <kbd>Esc</kbd>
      </div>
      <div class="list" bind:this={listEl} role="listbox">
        {#each filtered as cmd, i (cmd.id)}
          {#if i === 0 || filtered[i - 1].group !== cmd.group}
            <span class="group-label">{t(GROUP_LABELS[cmd.group] ?? "palette.groupApp")}</span>
          {/if}
          <button
            class="cmd"
            class:selected={i === selected}
            role="option"
            aria-selected={i === selected}
            onpointermove={() => (selected = i)}
            onclick={runSelected}
          >
            <Icon name={cmd.icon} size={14} />
            <span class="label">{cmd.label}</span>
            {#if cmd.hint}<span class="hint">{cmd.hint}</span>{/if}
          </button>
        {:else}
          <p class="empty">{t("palette.empty")}</p>
        {/each}
      </div>
    </div>
  </div>
{/if}

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
    padding-top: 12vh;
    z-index: 90;
    animation: fade 0.12s ease;
  }

  @keyframes fade {
    from {
      opacity: 0;
    }
  }

  .palette {
    width: 560px;
    max-width: calc(100vw - 48px);
    background: var(--bg-panel);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-xl);
    box-shadow: var(--shadow-modal);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    animation: pop 0.15s cubic-bezier(0.2, 0.9, 0.3, 1);
  }

  @keyframes pop {
    from {
      opacity: 0;
      transform: translateY(-8px) scale(0.99);
    }
  }

  .search {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4);
    border-bottom: 1px solid var(--border);
    color: var(--text-muted);
  }

  .search input {
    flex: 1;
    background: transparent;
    border: none;
    box-shadow: none;
    outline: none;
    font-size: 14px;
    color: var(--text);
    padding: 0;
  }

  kbd {
    font-size: 10px;
    color: var(--text-faint);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: 1px 5px;
  }

  .list {
    max-height: 48vh;
    overflow-y: auto;
    padding: var(--space-2);
    display: flex;
    flex-direction: column;
    gap: 1px;
  }

  .cmd {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    width: 100%;
    text-align: left;
    background: transparent;
    border: none;
    box-shadow: none;
    border-radius: var(--radius-lg);
    padding: 7px var(--space-3);
    color: var(--text);
    cursor: pointer;
  }

  .cmd.selected {
    background: var(--bg-hover);
  }

  .label {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .hint {
    flex-shrink: 0;
    font-size: 11px;
    color: var(--text-faint);
  }

  .empty {
    padding: var(--space-4);
    color: var(--text-faint);
    font-size: 13px;
    text-align: center;
  }
  .group-label {
    display: block;
    padding: 6px 10px 2px;
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-faint);
  }
</style>
