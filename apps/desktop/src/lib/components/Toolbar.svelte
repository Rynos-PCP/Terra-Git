<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { api } from "../api";
  import { workshopAvailable } from "../conflictWorkshop";
  import { t } from "../i18n.svelte";
  import { tooltip } from "../tooltip";
  import {
    cancelRemoteOp,
    closeRepo,
    gitFetch,
    gitPull,
    gitPush,
    gitPushForce,
    gitPushTo,
    openConflicts,
    openPipeline,
    openRepo,
    openWorkshop,
    prProvider,
    redoLast,
    savePrefs,
    showError,
    ui,
    undoLabel,
    undoLast,
  } from "../state.svelte";
  import BranchMenu from "./BranchMenu.svelte";
  import Icon from "./Icon.svelte";
  import Menu from "./Menu.svelte";

  /** Fires a system integration and reports errors instead of swallowing them. */
  function sys(p: Promise<unknown>) {
    p.catch(showError);
  }

  let repoMenuOpen = $state(false);
  let syncMenuOpen = $state(false);
  let moreMenuOpen = $state(false);

  async function browse() {
    repoMenuOpen = false;
    const dir = await open({ directory: true, title: t("dialog.openRepo") });
    if (typeof dir === "string") await openRepo(dir);
  }

  const ahead = $derived(ui.status?.ahead ?? 0);
  const behind = $derived(ui.status?.behind ?? 0);
  const hasUpstream = $derived(!!ui.status?.upstream);

  // Conflict workshop: without a running operation there is nothing to resolve —
  // the entry stays visible (only greyed out) so the way there is known BEFORE
  // the first conflict arrives.
  const conflictsReady = $derived(workshopAvailable(ui.status?.opState));
  const conflictCount = $derived(
    (ui.status?.unstaged ?? []).filter((e) => e.kind === "conflicted").length,
  );

  function setTheme(t: "dark" | "light" | "system") {
    ui.theme = t;
    savePrefs();
  }

  const prLabel = $derived(prProvider() === "gitlab" ? "Merge Request" : "Pull Request");
</script>

<header class="toolbar">
  <!-- Segment 1: repository -->
  <Menu bind:open={repoMenuOpen} width="300px">
    {#snippet trigger({ toggle })}
      <button class="segment" onclick={toggle} use:tooltip={ui.repo?.path ?? ""}>
        <Icon name="folder" />
        <span class="col">
          <span class="label">{t("toolbar.repoLabel")}</span>
          <strong>{ui.repo?.name}</strong>
        </span>
        <Icon name="chevronDown" size={12} />
      </button>
    {/snippet}
    <button class="item ghost" role="menuitem" onclick={browse}>
      <Icon name="folder" size={14} />
      {t("toolbar.openOther")}
    </button>
    <button
      class="item ghost"
      role="menuitem"
      onclick={() => {
        repoMenuOpen = false;
        ui.modal = { kind: "clone" };
      }}
    >
      <Icon name="external" size={14} />
      {t("toolbar.clone")}
    </button>
    <button
      class="item ghost"
      role="menuitem"
      onclick={() => {
        repoMenuOpen = false;
        ui.modal = { kind: "init" };
      }}
    >
      <Icon name="plus" size={14} />
      {t("toolbar.init")}
    </button>
    <div class="sep-h" role="separator"></div>
    <button
      class="item ghost"
      role="menuitem"
      onclick={() => {
        repoMenuOpen = false;
        closeRepo();
      }}
    >
      <Icon name="x" size={14} />
      {t("toolbar.closeRepo")}
    </button>
    {#if ui.recents.filter((r) => r.path !== ui.repo?.path).length > 0}
      <div class="sep-h" role="separator"></div>
      <span class="menu-label">{t("toolbar.recents")}</span>
      {#each ui.recents.filter((r) => r.path !== ui.repo?.path) as recent (recent.path)}
        <button
          class="item ghost recent"
          role="menuitem"
          use:tooltip={recent.path}
          onclick={() => {
            repoMenuOpen = false;
            openRepo(recent.path);
          }}
        >
          {recent.path
            .replace(/[\\/]+$/, "")
            .split(/[\\/]/)
            .pop()}
        </button>
      {/each}
    {/if}
  </Menu>

  <div class="sep"></div>

  <!-- Segment 2: branch -->
  <BranchMenu />

  <div class="sep"></div>

  <!-- Segment 3: actions -->
  <button
    class="ghost"
    disabled={!ui.undoStatus?.undo || !!ui.busy || ui.working > 0}
    use:tooltip={ui.undoStatus?.undo
      ? t("undo.tooltip", { label: undoLabel(ui.undoStatus.undo) })
      : t("undo.nothing")}
    onclick={undoLast}
  >
    <Icon name="undo" size={14} />
  </button>
  <button
    class="ghost"
    disabled={!ui.undoStatus?.redo || !!ui.busy || ui.working > 0}
    use:tooltip={ui.undoStatus?.redo
      ? t("undo.redoTooltip", { label: undoLabel(ui.undoStatus.redo) })
      : t("undo.nothingRedo")}
    onclick={redoLast}
  >
    <Icon name="redo" size={14} />
  </button>

  <button
    class="ghost"
    onclick={() => (ui.modal = { kind: "stash" })}
    use:tooltip={t("toolbar.manageStashes")}
  >
    <Icon name="stash" />
    {#if ui.stashes.length > 0}<span class="badge">{ui.stashes.length}</span>{/if}
  </button>

  <button
    class="ghost"
    onclick={() => (ui.modal = { kind: "changeRequests" })}
    use:tooltip={t("toolbar.viewPrs", { label: `${prLabel}s` })}
  >
    <Icon name="pr" />
  </button>

  <span class="spacer"></span>

  {#if ui.busy}
    <span class="busy">
      <span class="spin"></span>
      {#if ui.progress}
        {ui.busy} — {t(`progress.${ui.progress.phase}` as Parameters<typeof t>[0])}
        {ui.progress.percent}%
      {:else}
        {ui.busy}…
      {/if}
      {#if ui.busyCancellable}
        <!-- Only cancellable ops (fetch/pull/push) show "cancel"; an in-process
             checkout (branch switch) cannot be cancelled. -->
        <button class="cancel-op" onclick={cancelRemoteOp} use:tooltip={t("toolbar.cancelOp")}>
          {t("state.cancel")}
        </button>
      {/if}
    </span>
  {/if}

  {#if ui.busy}
    <!-- Narrow progress bar under the toolbar (GitHub Desktop style);
         indeterminate/pulsing until the first percentage arrives. The shimmer on
         the fill runs ALWAYS — even when the bar itself stands still. -->
    <div class="progress-track" class:indeterminate={!ui.progress}>
      <div class="progress-fill" style:width="{ui.progress?.percent ?? 0}%"></div>
    </div>
  {/if}

  <div class="sync">
    <button
      onclick={gitFetch}
      disabled={!!ui.busy || !!ui.cloning}
      use:tooltip={"git fetch --prune"}
    >
      <Icon name="refresh" size={14} />
      {t("toolbar.fetch")}
    </button>
    <button
      onclick={gitPull}
      disabled={!!ui.busy || !!ui.cloning || !hasUpstream}
      use:tooltip={hasUpstream ? "git pull" : t("toolbar.noUpstream")}
    >
      <Icon name="arrowDown" size={14} />
      {t("toolbar.pull")}
      {#if behind > 0}<span class="badge pull">{behind}</span>{/if}
    </button>
    <div class="push-group" class:primary-group={ahead > 0 || !hasUpstream}>
      <button
        class:primary={ahead > 0 || !hasUpstream}
        onclick={gitPush}
        disabled={!!ui.busy || !!ui.cloning}
        use:tooltip={hasUpstream ? "git push" : "git push --set-upstream origin <branch>"}
      >
        <Icon name="arrowUp" size={14} />
        {t("toolbar.push")}
        {#if ahead > 0}<span class="badge push">{ahead}</span>{/if}
      </button>
      <Menu bind:open={syncMenuOpen} align="right" width="260px">
        {#snippet trigger({ toggle })}
          <button
            class="chev"
            class:primary={ahead > 0 || !hasUpstream}
            onclick={toggle}
            disabled={!!ui.busy}
            aria-label={t("toolbar.pushOptions")}
          >
            <Icon name="chevronDown" size={12} />
          </button>
        {/snippet}
        {#each ui.remotes as remote (remote.name)}
          <button
            class="item ghost"
            role="menuitem"
            onclick={() => {
              syncMenuOpen = false;
              gitPushTo(remote.name);
            }}
          >
            <Icon name="arrowUp" size={14} />
            {t("toolbar.pushTo", { name: remote.name })}
            <span class="muted"
              >{remote.url.length > 30 ? remote.url.slice(0, 30) + "…" : remote.url}</span
            >
          </button>
        {/each}
        <div class="sep-h" role="separator"></div>
        <button
          class="item ghost"
          role="menuitem"
          onclick={() => {
            syncMenuOpen = false;
            gitPushForce();
          }}
        >
          <Icon name="arrowUp" size={14} />
          {t("toolbar.forcePush")}
        </button>
        <div class="sep-h" role="separator"></div>
        <button
          class="item ghost"
          role="menuitem"
          onclick={() => {
            syncMenuOpen = false;
            ui.modal = { kind: "remotes" };
          }}
        >
          <Icon name="globe" size={14} />
          {t("toolbar.manageRemotes")}
        </button>
      </Menu>
    </div>
    {#if ahead > 0 || !hasUpstream}
      <button class="ghost" onclick={openWorkshop} use:tooltip={t("workshop.open")}>
        <Icon name="edit" size={14} />
        {t("workshop.title")}
      </button>
    {/if}
  </div>

  <!-- ⋯ menu -->
  <Menu bind:open={moreMenuOpen} align="right" width="280px">
    {#snippet trigger({ toggle })}
      <button class="ghost" onclick={toggle} aria-label={t("toolbar.moreActions")}>
        <Icon name="more" />
      </button>
    {/snippet}
    <!-- Tools: the big functions are ALWAYS findable here — the
         situational workshop button in the toolbar replaces no fixed place. -->
    <span class="menu-label">{t("toolbar.tools")}</span>
    <button
      class="item ghost"
      role="menuitem"
      onclick={() => {
        moreMenuOpen = false;
        openWorkshop();
      }}
    >
      <Icon name="edit" size={14} />
      {t("workshop.title")}
    </button>
    <button
      class="item ghost"
      role="menuitem"
      onclick={() => {
        moreMenuOpen = false;
        openPipeline();
      }}
    >
      <Icon name="terminal" size={14} />
      {t("pipe.menu")}
    </button>
    <button
      class="item ghost"
      role="menuitem"
      disabled={!conflictsReady}
      onclick={() => {
        moreMenuOpen = false;
        openConflicts();
      }}
    >
      <Icon name="merge" size={14} />
      {t("conflictws.title")}
      <!-- The reason stands visibly next to it: a tooltip would reach the
           greyed-out entry never (disabled fires no mouse events and
           is not focusable). -->
      <span class="muted">
        {conflictsReady ? t("conflictws.openN", { n: conflictCount }) : t("conflictws.idle")}
      </span>
    </button>
    <div class="sep-h" role="separator"></div>
    <span class="menu-label">{t("toolbar.manage")}</span>
    <button
      class="item ghost"
      role="menuitem"
      onclick={() => {
        moreMenuOpen = false;
        ui.modal = { kind: "tags" };
      }}
    >
      <Icon name="tag" size={14} />
      {t("toolbar.manageTags")}
    </button>
    <button
      class="item ghost"
      role="menuitem"
      onclick={() => {
        moreMenuOpen = false;
        ui.modal = { kind: "submodules" };
      }}
    >
      <Icon name="tree" size={14} />
      {t("toolbar.submodules")}
    </button>
    <button
      class="item ghost"
      role="menuitem"
      onclick={() => {
        moreMenuOpen = false;
        ui.modal = { kind: "worktrees" };
      }}
    >
      <Icon name="folder" size={14} />
      {t("toolbar.worktrees")}
    </button>
    <button
      class="item ghost"
      role="menuitem"
      onclick={() => {
        moreMenuOpen = false;
        ui.modal = { kind: "sparse" };
      }}
    >
      <Icon name="split" size={14} />
      {t("sparse.menu")}
    </button>
    <button
      class="item ghost"
      role="menuitem"
      onclick={() => {
        moreMenuOpen = false;
        ui.modal = { kind: "remotes" };
      }}
    >
      <Icon name="globe" size={14} />
      {t("toolbar.manageRemotes")}
    </button>
    <button
      class="item ghost"
      role="menuitem"
      onclick={() => {
        moreMenuOpen = false;
        ui.modal = { kind: "backups" };
      }}
    >
      <Icon name="undo" size={14} />
      {t("toolbar.backups")}
    </button>
    <div class="sep-h" role="separator"></div>
    <span class="menu-label">{t("toolbar.open")}</span>
    <button
      class="item ghost"
      role="menuitem"
      onclick={() => {
        moreMenuOpen = false;
        if (ui.repo) sys(api.openInExplorer(ui.repo.path));
      }}
    >
      <Icon name="folder" size={14} />
      {t("toolbar.openExplorer")}
    </button>
    <button
      class="item ghost"
      role="menuitem"
      onclick={() => {
        moreMenuOpen = false;
        if (ui.repo) sys(api.openInEditor(ui.repo.path, ui.editorCmd));
      }}
    >
      <Icon name="edit" size={14} />
      {t("toolbar.openEditor")}
    </button>
    <button
      class="item ghost"
      role="menuitem"
      onclick={() => {
        moreMenuOpen = false;
        if (ui.repo) sys(api.openTerminal(ui.repo.path));
      }}
    >
      <Icon name="terminal" size={14} />
      {t("toolbar.openTerminal")}
    </button>
    <button
      class="item ghost"
      role="menuitem"
      onclick={() => {
        moreMenuOpen = false;
        sys(api.newWindow());
      }}
    >
      <Icon name="window" size={14} />
      {t("toolbar.newWindow")}
    </button>
    <button
      class="item ghost"
      role="menuitem"
      onclick={() => {
        moreMenuOpen = false;
        sys(api.openLogs());
      }}
    >
      <Icon name="folder" size={14} />
      {t("toolbar.openLogs")}
    </button>
    <div class="sep-h" role="separator"></div>
    <span class="menu-label">{t("theme.title")}</span>
    <button class="item ghost" role="menuitem" onclick={() => setTheme("dark")}>
      <Icon name="moon" size={14} />
      {t("theme.dark")}
      {#if ui.theme === "dark"}<span class="active-mark"><Icon name="check" size={12} /></span>{/if}
    </button>
    <button class="item ghost" role="menuitem" onclick={() => setTheme("light")}>
      <Icon name="sun" size={14} />
      {t("theme.light")}
      {#if ui.theme === "light"}<span class="active-mark"><Icon name="check" size={12} /></span
        >{/if}
    </button>
    <button class="item ghost" role="menuitem" onclick={() => setTheme("system")}>
      <Icon name="window" size={14} />
      {t("theme.system")}
      {#if ui.theme === "system"}<span class="active-mark"><Icon name="check" size={12} /></span
        >{/if}
    </button>
    <button
      class="item ghost"
      role="menuitem"
      onclick={() => {
        moreMenuOpen = false;
        ui.view = "settings";
      }}
    >
      <Icon name="settings" size={14} />
      {t("nav.settings")}
    </button>
  </Menu>
</header>

<style>
  .toolbar {
    position: relative;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    background: var(--bg-panel);
    border-bottom: 1px solid var(--border);
    min-height: 48px;
  }

  /* Narrow progress bar at the bottom edge of the toolbar. */
  .progress-track {
    position: absolute;
    left: 0;
    right: 0;
    bottom: -1px;
    height: 3px;
    overflow: hidden;
    background: color-mix(in srgb, var(--accent) 20%, transparent);
  }

  .progress-fill {
    height: 100%;
    background: var(--accent);
    transition: width 0.15s ease;
    position: relative;
    overflow: hidden;
  }

  /* Permanent shimmer on the fill: signals life even when
     nothing happens between two progress events (6.8.4 item 4). */
  .progress-fill::after {
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
    .progress-fill::after {
      animation: none;
      content: none;
    }
  }

  :global([data-reduce-motion="on"]) .progress-fill::after {
    animation: none;
    content: none;
  }

  /* Until the first percentage arrives: a pulsing bar (indeterminate). */
  .progress-track.indeterminate .progress-fill {
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

  /* Reduced motion: the frozen indeterminate animation would otherwise sit with
     margin-left -35 % INVISIBLY outside the bar (user finding
     2026-08-14) — instead a calm, visible full fill. */
  @media (prefers-reduced-motion: reduce) {
    .progress-track.indeterminate .progress-fill {
      animation: none;
      margin-left: 0;
      width: 100% !important;
      opacity: 0.45;
    }
  }

  :global([data-reduce-motion="on"]) .progress-track.indeterminate .progress-fill {
    animation: none;
    margin-left: 0;
    width: 100% !important;
    opacity: 0.45;
  }

  .segment {
    background: transparent;
    border-color: transparent;
    box-shadow: none;
    padding: 4px 10px;
  }

  .segment:hover {
    background: var(--bg-hover);
  }

  .col {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    line-height: 1.2;
  }

  .label {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.07em;
    color: var(--text-muted);
  }

  .sep {
    width: 1px;
    height: 28px;
    background: var(--border);
  }

  .sep-h {
    height: 1px;
    background: var(--border);
    margin: var(--space-1) 0;
  }

  .spacer {
    flex: 1;
  }

  .busy {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    color: var(--text-muted);
    margin-right: var(--space-2);
  }

  /* Running seconds: tabular figures against the jitter. */
  .cancel-op {
    padding: 2px 8px;
    font-size: 0.85em;
    color: var(--text);
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    cursor: pointer;
  }
  .cancel-op:hover {
    border-color: var(--danger, #c0392b);
    color: var(--danger, #c0392b);
  }

  /* Fetch/pull/push as one coherent sync group. */
  .sync {
    display: flex;
    align-items: stretch;
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    box-shadow: var(--btn-shadow);
  }

  .sync > button,
  .push-group > button,
  .push-group .chev {
    border: none;
    border-radius: 0;
    background: transparent;
    box-shadow: none;
    min-height: 26px;
  }

  .sync > button:first-child {
    border-radius: 5px 0 0 5px;
  }

  .sync > * + * {
    border-left: 1px solid var(--border);
  }

  .push-group {
    display: flex;
    align-items: stretch;
  }

  .push-group > :global(.menu-root) {
    border-left: 1px solid var(--border);
  }

  .push-group .chev {
    padding: 4px 6px;
    border-radius: 0 5px 5px 0;
    height: 100%;
  }

  .sync button:hover:not(:disabled) {
    background: var(--bg-hover);
  }

  .sync button.primary,
  .push-group .chev.primary {
    background: var(--accent-dim);
    color: var(--accent-text);
    font-weight: 600;
  }

  .sync button.primary:hover:not(:disabled) {
    background: var(--accent);
  }

  .item {
    width: 100%;
    justify-content: flex-start;
    text-align: left;
    padding: 6px 10px;
    border-radius: var(--radius);
    color: var(--text-primary);
  }

  .item .muted {
    color: var(--text-faint);
    font-size: 11px;
    margin-left: auto;
  }

  .recent {
    font-weight: 550;
  }

  .active-mark {
    margin-left: auto;
    display: inline-flex;
    color: var(--accent);
  }

  .menu-label {
    display: block;
    padding: 4px 10px 2px;
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-faint);
  }

  /* Tint derived from the accent token so light/dark stay consistent. */
  .badge.push {
    background: color-mix(in srgb, var(--accent) 15%, transparent);
    color: var(--accent);
    border-color: transparent;
  }

  /* On the primary push (accent background) the badge needs its own contrast. */
  .sync button.primary .badge {
    background: rgba(0, 0, 0, 0.22);
    color: var(--accent-text);
    border-color: transparent;
  }

  .badge.pull {
    background: color-mix(in srgb, var(--blue) 15%, transparent);
    color: var(--blue);
    border-color: transparent;
  }
</style>
