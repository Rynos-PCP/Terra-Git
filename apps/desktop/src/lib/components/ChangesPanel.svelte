<script lang="ts">
  import { onMount } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import { confirm } from "@tauri-apps/plugin-dialog";
  import type { StatusEntry } from "../api";
  import { buildCommitMessage, parseCommitMessage } from "../commitMessage";
  import { clickSelect, pruneSelection, selectAll, type ClickMods } from "../fileSelection";
  import { t, tn } from "../i18n.svelte";
  import { selectionPaths } from "../selection";
  import {
    createCommit,
    discardFiles,
    savePrefs,
    selectFile,
    stageFiles,
    ui,
    undoLastCommit,
    unstageFiles,
  } from "../state.svelte";
  import type { OverviewRow } from "../changesOverview";
  import ChangesOverview from "./ChangesOverview.svelte";
  import DiffView from "./DiffView.svelte";
  import FileRow from "./FileRow.svelte";
  import Icon from "./Icon.svelte";
  import Menu from "./Menu.svelte";
  import Splitter from "./Splitter.svelte";
  import VirtualList from "./VirtualList.svelte";

  let summary = $state("");
  let summaryEl = $state<HTMLInputElement>();
  let description = $state("");
  let amend = $state(false);
  let committing = $state(false);
  let coAuthors = $state("");
  let showCoAuthors = $state(false);
  let filter = $state("");
  const collapsed = new SvelteSet<string>();
  let msgLogOpen = $state(false);

  /** Takes an earlier message into the fields (subject/body/co-authors). */
  function applyLoggedMessage(msg: string) {
    msgLogOpen = false;
    const parsed = parseCommitMessage(msg);
    summary = parsed.summary;
    description = parsed.description;
    coAuthors = parsed.coAuthors;
    showCoAuthors = parsed.coAuthors.length > 0;
  }

  const staged = $derived(ui.status?.staged ?? []);
  const unstaged = $derived(filterEntries(ui.status?.unstaged ?? []));
  const stagedFiltered = $derived(filterEntries(staged));

  // ---- Multi-selection of the unstaged changes (Ctrl/Shift/Ctrl+A) ----
  let selection = $state(new Set<string>());
  let anchor = $state<string | null>(null);
  // ---- Multi-selection of the staged changes (same idea) ----
  let stagedSelection = $state(new Set<string>());
  let stagedAnchor = $state<string | null>(null);
  /** Right-click context menu (screen position, affected paths + side). */
  let ctxMenu = $state<{ x: number; y: number; paths: string[]; staged: boolean } | null>(null);
  let ctxMenuEl = $state<HTMLElement>();
  // Which side last had focus/a click — decides which selection Ctrl+A fills
  // (see onFocusIn below).
  let lastFocusedSide = $state<"staged" | "unstaged">("unstaged");

  const unstagedPaths = $derived(unstaged.map((e) => e.path));
  const stagedPaths = $derived(stagedFiltered.map((e) => e.path));

  // After every status refresh, remove vanished paths from the selection.
  $effect(() => {
    const pruned = pruneSelection(selection, unstagedPaths);
    if (pruned.size !== selection.size) selection = pruned;
  });
  $effect(() => {
    const pruned = pruneSelection(stagedSelection, stagedPaths);
    if (pruned.size !== stagedSelection.size) stagedSelection = pruned;
  });

  /** Click on an unstaged file: advance the selection + show the diff. */
  function handleFileSelect(path: string, mods: ClickMods) {
    const next = clickSelect({ selection, anchor }, unstagedPaths, path, mods);
    selection = next.selection;
    anchor = next.anchor;
    selectFile(path, false);
  }

  /** Click on a staged file: advance the selection + show the diff. */
  function handleStagedFileSelect(path: string, mods: ClickMods) {
    const next = clickSelect(
      { selection: stagedSelection, anchor: stagedAnchor },
      stagedPaths,
      path,
      mods,
    );
    stagedSelection = next.selection;
    stagedAnchor = next.anchor;
    selectFile(path, true);
  }

  /** Right-click: act on the selection, otherwise only on this file. */
  function handleFileContext(path: string, e: MouseEvent) {
    const paths = selection.has(path) && selection.size > 1 ? [...selection] : [path];
    if (paths.length === 1) {
      selection = new Set(paths);
      anchor = path;
    }
    ctxMenu = {
      x: Math.min(e.clientX, window.innerWidth - 240),
      y: e.clientY,
      paths,
      staged: false,
    };
  }

  /** Right-click on the staged side: same idea, acts on `stagedSelection`. */
  function handleStagedFileContext(path: string, e: MouseEvent) {
    const paths =
      stagedSelection.has(path) && stagedSelection.size > 1 ? [...stagedSelection] : [path];
    if (paths.length === 1) {
      stagedSelection = new Set(paths);
      stagedAnchor = path;
    }
    ctxMenu = {
      x: Math.min(e.clientX, window.innerWidth - 240),
      y: e.clientY,
      paths,
      staged: true,
    };
  }

  async function discardFromMenu() {
    const paths = ctxMenu?.paths ?? [];
    ctxMenu = null;
    if (paths.length > 0) await discardWithConfirm(paths);
  }

  /** Stages the current selection of the unstaged list (also from the context menu). */
  async function stageSelected() {
    const files = selectionPaths(ui.status?.unstaged ?? [], selection);
    if (files.length) {
      await stageFiles(files);
      selection = new Set();
    }
  }

  /** Unstages the current selection of the staged list (also from the context menu). */
  async function unstageSelected() {
    const files = selectionPaths(ui.status?.staged ?? [], stagedSelection);
    if (files.length) {
      await unstageFiles(files);
      stagedSelection = new Set();
    }
  }

  async function stageSelectedFromMenu() {
    ctxMenu = null;
    await stageSelected();
  }

  async function unstageSelectedFromMenu() {
    ctxMenu = null;
    await unstageSelected();
  }

  // Ctrl+A selects all changes of the side last focused (staged or unstaged) —
  // but not while typing in an input field (the native select-all applies
  // there).
  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.ctrlKey || e.metaKey) || e.key.toLowerCase() !== "a") return;
      const el = document.activeElement as HTMLElement | null;
      if (el?.closest("input, textarea, [contenteditable='true']")) return;
      if (lastFocusedSide === "staged") {
        if (stagedPaths.length === 0) return;
        e.preventDefault();
        const next = selectAll(stagedPaths);
        stagedSelection = next.selection;
        stagedAnchor = next.anchor;
      } else {
        if (unstagedPaths.length === 0) return;
        e.preventDefault();
        const next = selectAll(unstagedPaths);
        selection = next.selection;
        anchor = next.anchor;
      }
    };
    // Remembers which side (staged/unstaged) was last active.
    //
    // Deliberately fed from focusin AND pointerdown: WebKit (the macOS build)
    // intentionally does NOT make form controls focusable on a mouse click
    // (HTMLFormControlElement::isMouseFocusable). Since the row selection hangs
    // off a real <button>, a pure focusin handler would never fire there —
    // Ctrl+A would then always fill the wrong side.
    const rememberSide = (target: EventTarget | null) => {
      const el = (target as HTMLElement | null)?.closest<HTMLElement>("[data-side]");
      if (el) lastFocusedSide = el.dataset.side === "staged" ? "staged" : "unstaged";
    };
    const onFocusIn = (e: FocusEvent) => rememberSide(e.target);
    const onPointerDown = (e: PointerEvent) => rememberSide(e.target);
    // Close the context menu when clicking outside or pressing Escape.
    const onClose = (e: Event) => {
      if (ctxMenu && ctxMenuEl && !ctxMenuEl.contains(e.target as Node)) ctxMenu = null;
    };
    const onEsc = (e: KeyboardEvent) => {
      if (ctxMenu && e.key === "Escape") ctxMenu = null;
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("keydown", onEsc);
    window.addEventListener("pointerdown", onClose);
    window.addEventListener("focusin", onFocusIn);
    window.addEventListener("pointerdown", onPointerDown, true);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("keydown", onEsc);
      window.removeEventListener("pointerdown", onClose);
      window.removeEventListener("focusin", onFocusIn);
      window.removeEventListener("pointerdown", onPointerDown, true);
    };
  });
  const locked = $derived(ui.working > 0 || !!ui.busy);
  const canCommit = $derived(
    !committing && !locked && summary.trim().length > 0 && (staged.length > 0 || amend),
  );
  const canUndo = $derived(
    !locked && ui.history.length > 0 && (ui.status?.opState ?? "clean") === "clean",
  );
  /** Warning as in GitHub Desktop: subject line > 72 characters. */
  const summaryTooLong = $derived(summary.length > 72);

  /**
   * An amend rewrites the last commit. If it is already on the upstream
   * (ahead=0), the local history diverges afterwards → a force push is needed.
   * ahead>0 means: the last commit is not pushed yet, an amend is harmless.
   */
  /** An amend rewrites HEAD — during a multi-step operation that replaced OUR OWN
   *  predecessor commit with the result of the operation.
   *  The engine now rejects it; here the checkbox is not even clickable, instead
   *  of letting the user run into the error message. */
  const amendBlocked = $derived((ui.status?.opState ?? "clean") !== "clean");
  $effect(() => {
    // An already ticked box would otherwise survive the start of an operation
    // invisibly (it is only reset after a SUCCESSFUL commit).
    if (amendBlocked && amend) amend = false;
  });

  const amendRewritesPushed = $derived(
    amend && !!ui.status?.upstream && (ui.status?.ahead ?? 0) === 0,
  );

  /** Explains to the user WHY the commit button is currently disabled. */
  const commitHint = $derived.by(() => {
    if (canCommit) return null;
    if (committing || locked) return t("changes.hintBusy");
    if (staged.length === 0 && !amend) return t("changes.hintStageFirst");
    if (summary.trim().length === 0) return t("changes.hintSummary");
    return null;
  });

  function filterEntries(entries: StatusEntry[]): StatusEntry[] {
    const f = filter.trim().toLowerCase();
    return f ? entries.filter((e) => e.path.toLowerCase().includes(f)) : entries;
  }

  /** Ctrl/Cmd+Enter in the message fields triggers the commit. */
  function commitOnCtrlEnter(e: KeyboardEvent) {
    if ((e.ctrlKey || e.metaKey) && e.key === "Enter") {
      e.preventDefault();
      doCommit();
    }
  }

  /** Overview: open a file like a click in the respective list. */
  function openFromOverview(row: OverviewRow) {
    if (row.staged === "full") handleStagedFileSelect(row.path, { ctrl: false, shift: false });
    else handleFileSelect(row.path, { ctrl: false, shift: false });
  }

  /** Overview: space stages or unstages (partial: stage the rest). */
  function toggleFromOverview(row: OverviewRow) {
    if (row.staged === "full") unstageFiles([row.path]);
    else stageFiles([row.path]);
  }

  /** Ctrl+Enter in the overview: commit — otherwise jump into the empty summary. */
  function commitFromOverview() {
    if (canCommit) doCommit();
    else summaryEl?.focus();
  }

  async function doCommit() {
    if (!canCommit) return;
    committing = true;
    const message = buildCommitMessage(summary, description, coAuthors);
    const ok = await createCommit(message, amend);
    committing = false;
    if (ok) {
      summary = "";
      description = "";
      coAuthors = "";
      amend = false;
      showCoAuthors = false;
    }
  }

  async function discardWithConfirm(paths: string[]) {
    const message = tn("changes.discardConfirm", paths.length, { path: paths[0] });
    const yes = await confirm(message, { title: t("changes.discardTitle"), kind: "warning" });
    if (yes) await discardFiles(paths);
  }

  function toggleView() {
    ui.changesView = ui.changesView === "flat" ? "tree" : "flat";
    savePrefs();
  }

  // ---- Tree view: group entries by directory ----
  interface TreeRow {
    kind: "dir" | "file";
    depth: number;
    dir?: string;
    entry?: StatusEntry;
  }

  function buildTree(entries: StatusEntry[]): TreeRow[] {
    const rows: TreeRow[] = [];
    const sorted = [...entries].sort((a, b) => a.path.localeCompare(b.path));
    // A pure computation cache, never state — deliberately a plain Set
    // (a SvelteSet would only cost signal overhead per entry here).
    // eslint-disable-next-line svelte/prefer-svelte-reactivity
    const seenDirs = new Set<string>();
    for (const entry of sorted) {
      const parts = entry.path.split("/");
      const dirs = parts.slice(0, -1);
      // A collapsed ancestor hides ALL deeper rows.
      const ancestorCollapsed = (depth: number) =>
        dirs.slice(0, depth).some((_, j) => collapsed.has(dirs.slice(0, j + 1).join("/")));

      for (let i = 0; i < dirs.length; i++) {
        const dirPath = dirs.slice(0, i + 1).join("/");
        if (!ancestorCollapsed(i) && !seenDirs.has(dirPath)) {
          seenDirs.add(dirPath);
          rows.push({ kind: "dir", depth: i, dir: dirPath });
        }
      }
      if (!ancestorCollapsed(dirs.length)) {
        rows.push({ kind: "file", depth: dirs.length, entry });
      }
    }
    return rows;
  }

  function toggleDir(dir: string) {
    if (collapsed.has(dir)) collapsed.delete(dir);
    else collapsed.add(dir);
  }

  // ---- Virtualization: both sections as ONE row model of fixed height.
  // That keeps the list fluid even with tens of thousands of changed files —
  // only the visible rows are in the DOM (VirtualList).
  const ROW_H = 28;

  type ListRow =
    | { t: "header"; staged: boolean }
    | { t: "empty"; staged: boolean }
    | { t: "gap" }
    | { t: "dir"; staged: boolean; depth: number; dir: string }
    | { t: "file"; staged: boolean; depth: number; entry: StatusEntry };

  function pushSection(rows: ListRow[], entries: StatusEntry[], staged: boolean) {
    rows.push({ t: "header", staged });
    if (entries.length === 0) {
      rows.push({ t: "empty", staged });
      return;
    }
    if (ui.changesView === "tree") {
      for (const r of buildTree(entries)) {
        if (r.kind === "dir") rows.push({ t: "dir", staged, depth: r.depth, dir: r.dir! });
        else rows.push({ t: "file", staged, depth: r.depth, entry: r.entry! });
      }
    } else {
      for (const entry of entries) rows.push({ t: "file", staged, depth: 0, entry });
    }
  }

  const listRows = $derived.by<ListRow[]>(() => {
    const rows: ListRow[] = [];
    pushSection(rows, stagedFiltered, true);
    rows.push({ t: "gap" });
    pushSection(rows, unstaged, false);
    return rows;
  });

  function rowKey(r: ListRow): string {
    switch (r.t) {
      case "header":
        return `h:${r.staged}`;
      case "empty":
        return `e:${r.staged}`;
      case "gap":
        return "gap";
      case "dir":
        return `d:${r.staged}:${r.dir}`;
      case "file":
        return `f:${r.staged}:${r.entry.path}`;
    }
  }

  const diffs = $derived(ui.fileDiff ? [ui.fileDiff] : []);
</script>

{#snippet listRow(r: ListRow)}
  {#if r.t === "header"}
    <header class="sec-head">
      <h3 class="section-title">
        {r.staged ? t("changes.sectionStaged") : t("changes.sectionUnstaged")}
        <span class="badge">{r.staged ? stagedFiltered.length : unstaged.length}</span>
      </h3>
      {#if r.staged && stagedFiltered.length > 0}
        <button
          class="ghost"
          disabled={locked}
          onclick={() => unstageFiles(stagedFiltered.map((e) => e.path))}
        >
          {filter.trim() ? t("changes.unstageMatches") : t("changes.unstageAll")}
        </button>
      {:else if !r.staged && unstaged.length > 0}
        <button
          class="ghost"
          disabled={locked}
          onclick={() => stageFiles(unstaged.map((e) => e.path))}
        >
          {filter.trim() ? t("changes.stageMatches") : t("changes.stageAll")}
        </button>
      {/if}
    </header>
  {:else if r.t === "empty"}
    <div class="empty">{r.staged ? t("changes.emptyStaged") : t("changes.emptyUnstaged")}</div>
  {:else if r.t === "dir"}
    <button
      class="dir ghost"
      style:padding-left="{6 + r.depth * 14}px"
      onclick={() => toggleDir(r.dir)}
    >
      <span class="chev" class:closed={collapsed.has(r.dir)}>
        <Icon name="chevronDown" size={11} />
      </span>
      <Icon name="folder" size={13} />
      {r.dir.split("/").pop()}
    </button>
  {:else if r.t === "file"}
    <FileRow
      entry={r.entry}
      indent={r.depth}
      side={r.staged ? "staged" : "unstaged"}
      selected={r.staged ? stagedSelection.has(r.entry.path) : selection.has(r.entry.path)}
      disabled={locked}
      onselect={r.staged
        ? (mods) => handleStagedFileSelect(r.entry.path, mods)
        : (mods) => handleFileSelect(r.entry.path, mods)}
      oncontext={r.staged
        ? (e) => handleStagedFileContext(r.entry.path, e)
        : (e) => handleFileContext(r.entry.path, e)}
      onprimary={() => (r.staged ? unstageFiles([r.entry.path]) : stageFiles([r.entry.path]))}
      primaryLabel={r.staged ? t("changes.unstage") : t("changes.stage")}
      primaryIsStage={!r.staged}
      ondiscard={r.staged ? null : () => discardWithConfirm([r.entry.path])}
    />
  {/if}
{/snippet}

<div class="split" style:--changes-w="{ui.changesPanelWidth}px">
  <aside class="side">
    <div class="list-tools">
      <div class="filter">
        <Icon name="search" size={13} />
        <input type="text" placeholder={t("changes.filterFiles")} bind:value={filter} />
      </div>
      <button
        class="ghost"
        title={ui.changesView === "flat" ? t("changes.treeView") : t("changes.listView")}
        onclick={toggleView}
      >
        <Icon name="tree" size={14} />
      </button>
      <button
        class="ghost"
        title={t("changes.stashAll")}
        disabled={locked || (staged.length === 0 && (ui.status?.unstaged ?? []).length === 0)}
        onclick={() => (ui.modal = { kind: "stashPush" })}
      >
        <Icon name="stash" size={14} />
      </button>
    </div>

    <div class="lists">
      {#if !ui.status}
        <!-- First load (large repos: the status can take a while) -->
        <div class="loading">
          <span class="spin"></span>
          {t("changes.loading")}
        </div>
      {:else}
        <VirtualList items={listRows} rowHeight={ROW_H} getKey={rowKey} row={listRow} />
      {/if}
    </div>

    <div class="commit-box">
      <div class="summary-row">
        <input
          type="text"
          placeholder={t("changes.summaryPlaceholder")}
          bind:value={summary}
          bind:this={summaryEl}
          class:warn={summaryTooLong}
          onkeydown={commitOnCtrlEnter}
        />
        {#if summaryTooLong}
          <span class="count" title={t("changes.summaryTooLong")}>
            {summary.length}
          </span>
        {/if}
      </div>
      <textarea
        rows="3"
        placeholder={t("changes.descriptionPlaceholder")}
        bind:value={description}
        onkeydown={commitOnCtrlEnter}></textarea>

      {#if showCoAuthors}
        <input type="text" placeholder={t("changes.coAuthorsPlaceholder")} bind:value={coAuthors} />
      {/if}

      <div class="commit-opts">
        <label class="opt" class:disabled={amendBlocked}>
          <input type="checkbox" bind:checked={amend} disabled={amendBlocked} />
          {t("changes.amend")}
        </label>
        {#if amendBlocked}
          <span class="amend-warn own-line">{t("changes.amendBlockedOp")}</span>
        {/if}
        {#if amendRewritesPushed}
          <span class="amend-warn" title={t("changes.amendPushedTitle")}>
            <Icon name="alert" size={13} />
            {t("changes.amendPushedWarn")}
          </span>
        {/if}
        <button
          class="ghost"
          class:active={showCoAuthors}
          title={t("changes.addCoAuthors")}
          onclick={() => (showCoAuthors = !showCoAuthors)}
        >
          <Icon name="plus" size={12} />
          {t("changes.coAuthors")}
        </button>
        {#if ui.messageLog.length > 0}
          <Menu bind:open={msgLogOpen} align="left" direction="up" width="320px">
            {#snippet trigger({ toggle })}
              <button class="ghost" title={t("changes.reuseMessages")} onclick={toggle}>
                <Icon name="history" size={12} />
                {t("app.tab.history")}
              </button>
            {/snippet}
            {#each ui.messageLog as msg (msg)}
              <button
                class="item ghost msg-item"
                role="menuitem"
                onclick={() => applyLoggedMessage(msg)}
              >
                <span class="msg-subject">{msg.split("\n")[0]}</span>
                {#if msg.includes("\n")}
                  <span class="msg-more">{t("changes.plusDescription")}</span>
                {/if}
              </button>
            {/each}
          </Menu>
        {/if}
      </div>

      <button class="primary" disabled={!canCommit} title={commitHint} onclick={doCommit}>
        {#if committing}<span class="spin"></span>{/if}
        {t("changes.commitTo", { branch: ui.status?.branch ?? "HEAD" })}
      </button>
      <button class="ghost undo" disabled={!canUndo} onclick={undoLastCommit}>
        <Icon name="undo" size={13} />
        {t("changes.undoLastCommit")}
      </button>
    </div>
  </aside>

  <Splitter
    value={ui.changesPanelWidth}
    min={260}
    max={560}
    onresize={(w) => (ui.changesPanelWidth = w)}
    ondone={savePrefs}
  />

  <div class="main">
    {#if !ui.selectedFile && (ui.status?.staged.length ?? 0) + (ui.status?.unstaged.length ?? 0) > 0}
      <!-- Without a selection, the largest area of the app shows the overview
           instead of an empty "Select a file…". -->
      <ChangesOverview
        onopen={openFromOverview}
        ontoggle={toggleFromOverview}
        oncommitrequest={commitFromOverview}
      />
    {:else}
      <DiffView
        {diffs}
        interactive
        staged={ui.selectedFile?.staged ?? false}
        loading={!!ui.selectedFile && !ui.fileDiff}
        emptyText={t("changes.selectFileForDiff")}
        unchangedInfo={ui.unchangedInfo}
      />
    {/if}
  </div>
</div>

{#if ctxMenu}
  <div
    class="ctx-menu"
    bind:this={ctxMenuEl}
    role="menu"
    style:left="{ctxMenu.x}px"
    style:top="{ctxMenu.y}px"
  >
    {#if ctxMenu.staged}
      <button class="item ghost" role="menuitem" onclick={unstageSelectedFromMenu}>
        <Icon name="check" size={13} />
        {t("changes.unstageSelected")}
      </button>
    {:else}
      <button class="item ghost" role="menuitem" onclick={stageSelectedFromMenu}>
        <Icon name="plus" size={13} />
        {t("changes.stageSelected")}
      </button>
      <button class="item ghost danger" role="menuitem" onclick={discardFromMenu}>
        <Icon name="undo" size={13} />
        {ctxMenu.paths.length === 1
          ? t("changes.discardMenu")
          : t("changes.discardMenuN", { n: ctxMenu.paths.length })}
      </button>
    {/if}
  </div>
{/if}

<style>
  .ctx-menu {
    position: fixed;
    z-index: 100;
    min-width: 200px;
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-menu);
    padding: var(--space-2);
  }

  .split {
    display: grid;
    grid-template-columns: var(--changes-w, 360px) auto 1fr;
    height: 100%;
  }

  .side {
    display: flex;
    flex-direction: column;
    border-right: 1px solid var(--border);
    background: var(--bg-panel);
    min-height: 0;
  }

  .list-tools {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    padding: var(--space-2);
    border-bottom: 1px solid var(--border);
  }

  /* Framed search field: the icon sits inside, the focus ring on the container. */
  .filter {
    flex: 1;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding-left: var(--space-2);
    background: var(--bg-inset);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    color: var(--text-faint);
    transition:
      border-color 0.12s ease,
      box-shadow 0.12s ease;
  }

  .filter:focus-within {
    border-color: var(--accent-dim);
    box-shadow: 0 0 0 3px var(--focus-ring);
  }

  .filter input {
    background: transparent;
    border: none;
    box-shadow: none;
    min-height: 26px;
    padding-left: 0;
  }

  .lists {
    /* The VirtualList handles scrolling (window-based rendering). */
    flex: 1;
    padding: 0 var(--space-2);
    min-height: 0;
  }

  .lists :global(.viewport) {
    padding: var(--space-2) 0;
  }

  .sec-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    height: 100%;
  }

  .loading {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    height: 100%;
    color: var(--text-muted);
    font-size: 12px;
  }

  .empty {
    color: var(--text-muted);
    font-size: 12px;
    padding: var(--space-1) var(--space-2);
  }

  .dir {
    width: 100%;
    justify-content: flex-start;
    gap: 5px;
    padding-top: 2px;
    padding-bottom: 2px;
    color: var(--text-muted);
    font-weight: 550;
  }

  .chev {
    display: inline-flex;
    transition: transform 0.12s ease;
  }

  .chev.closed {
    transform: rotate(-90deg);
  }

  .commit-box {
    border-top: 1px solid var(--border);
    padding: var(--space-3);
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .summary-row {
    position: relative;
  }

  input.warn {
    border-color: var(--modified);
  }

  .count {
    position: absolute;
    right: 8px;
    top: 50%;
    transform: translateY(-50%);
    font-size: 11px;
    color: var(--modified);
  }

  /* Wraps on purpose: the "amend blocked" hint is a full sentence and does not
     fit next to the checkbox in the narrow left panel. Without wrapping it was
     squeezed into a ~70px column and blew up the row height. */
  .commit-opts {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3);
  }

  .opt {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--text-muted);
    font-size: 12px;
  }

  .commit-opts button.active {
    color: var(--accent);
  }

  .opt.disabled {
    opacity: 0.55;
    cursor: not-allowed;
  }

  .amend-warn {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    min-width: 0;
    font-size: 11.5px;
    color: var(--modified);
    cursor: help;
  }

  /* A full sentence gets a line of its own below the options instead of being
     crushed into a narrow column by the flex row (the panel is ~350px wide).
     Short warnings with an icon keep sitting next to the checkbox. */
  .amend-warn.own-line {
    flex: 1 0 100%;
    order: 1;
  }

  .msg-item {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    width: 100%;
    text-align: left;
  }

  .msg-subject {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .msg-more {
    flex-shrink: 0;
    font-size: 10.5px;
    color: var(--text-faint);
  }

  .undo {
    justify-content: center;
    font-size: 12px;
  }

  .main {
    min-width: 0;
    min-height: 0;
  }
</style>
