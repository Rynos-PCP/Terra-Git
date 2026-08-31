<script lang="ts">
  import type { ChangeKind, StatusEntry } from "../api";
  import { t, type MessageKey } from "../i18n.svelte";
  import {
    ignoreFile,
    openMergetool,
    resolveConflict,
    showBlame,
    stashPush,
    ui,
  } from "../state.svelte";
  import { tooltip } from "../tooltip";
  import Icon from "./Icon.svelte";
  import Menu from "./Menu.svelte";

  let {
    entry,
    selected,
    disabled = false,
    indent = 0,
    side = "unstaged",
    onselect,
    oncontext = null,
    onprimary,
    primaryLabel,
    primaryIsStage = true,
    ondiscard = null,
  }: {
    entry: StatusEntry;
    selected: boolean;
    disabled?: boolean;
    indent?: number;
    /** Which side the row belongs to (for focus-based Ctrl+A in the panel). */
    side?: "staged" | "unstaged";
    /** Modifier keys for the multi-selection (Ctrl/Shift). */
    onselect: (mods: { ctrl: boolean; shift: boolean }) => void;
    /** Right-click context menu (null = no menu, e.g. for staged files). */
    oncontext?: ((e: MouseEvent) => void) | null;
    onprimary: () => void;
    primaryLabel: string;
    /** true = the primary action stages ("+"), false = it unstages ("−"). */
    primaryIsStage?: boolean;
    ondiscard?: (() => void) | null;
  } = $props();

  // Title as a message key so a language change applies reactively (t() in the template).
  const kindGlyph: Record<ChangeKind, { g: string; cls: string; titleKey: MessageKey }> = {
    added: { g: "A", cls: "kind-added", titleKey: "diff.kindAdded" },
    modified: { g: "M", cls: "kind-modified", titleKey: "diff.kindModified" },
    deleted: { g: "D", cls: "kind-deleted", titleKey: "diff.kindDeleted" },
    renamed: { g: "R", cls: "kind-renamed", titleKey: "diff.kindRenamed" },
    typechange: { g: "T", cls: "kind-modified", titleKey: "diff.kindTypechange" },
    conflicted: { g: "!", cls: "kind-conflicted", titleKey: "diff.kindConflicted" },
    untracked: { g: "U", cls: "kind-added", titleKey: "diff.kindUntracked" },
  };

  const k = $derived(kindGlyph[entry.kind]);
  const isConflict = $derived(entry.kind === "conflicted");
  const fileName = $derived(entry.path.split("/").pop() ?? entry.path);
  const ext = $derived(fileName.includes(".") ? fileName.split(".").pop() : null);

  // "+" for the staging action, "−" for unstaging — explicitly via a prop so
  // the glyph does not depend on the translation of the label.
  const stageGlyph = $derived(primaryIsStage ? "+" : "−");
</script>

<!-- role="presentation": the row is only a visual grouping. The
     semantics are carried by the .select button inside it — the row itself used
     to be role="button" and contained real buttons, which is invalid per
     HTML/ARIA (interactive descendants in an interactive role). The click
     handler stays on the whole row as a mouse convenience so no dead zones
     arise between the elements; keyboard operation lives on the button. -->
<div
  class="row"
  class:selected
  style:padding-left="{6 + indent * 14}px"
  data-side={side}
  role="presentation"
  onclick={(e) => onselect({ ctrl: e.ctrlKey || e.metaKey, shift: e.shiftKey })}
  oncontextmenu={(e) => {
    if (oncontext) {
      e.preventDefault();
      e.stopPropagation();
      oncontext(e);
    }
  }}
>
  {#if !isConflict}
    <button
      class="stage ghost"
      use:tooltip={primaryLabel}
      {disabled}
      onclick={(e) => {
        e.stopPropagation();
        onprimary();
      }}
    >
      {stageGlyph}
    </button>
  {:else}
    <span class="conflict-icon"><Icon name="merge" size={13} /></span>
  {/if}

  <button
    class="select"
    onclick={(e) => {
      e.stopPropagation();
      onselect({ ctrl: e.ctrlKey || e.metaKey, shift: e.shiftKey });
    }}
  >
    <span class="kind {k.cls}" use:tooltip={t(k.titleKey)}>{k.g}</span>
    <span
      class="path"
      use:tooltip={entry.origPath ? `${entry.origPath} → ${entry.path}` : entry.path}
    >
      {entry.path}
    </span>
  </button>

  {#if isConflict}
    <div class="conflict-actions">
      <button
        class="ghost"
        use:tooltip={t("diff.openConflictEditor")}
        {disabled}
        onclick={(e) => {
          e.stopPropagation();
          ui.modal = { kind: "conflictEditor", file: entry.path };
        }}
      >
        <Icon name="merge" size={12} />
        {t("diff.resolve")}
      </button>
      <button
        class="ghost"
        use:tooltip={t("diff.takeOurs")}
        {disabled}
        onclick={(e) => {
          e.stopPropagation();
          resolveConflict(entry.path, true);
        }}
      >
        {t("diff.ours")}
      </button>
      <button
        class="ghost"
        use:tooltip={t("diff.takeTheirs")}
        {disabled}
        onclick={(e) => {
          e.stopPropagation();
          resolveConflict(entry.path, false);
        }}
      >
        {t("diff.theirs")}
      </button>
      <button
        class="ghost"
        use:tooltip={t("diff.openMergetool")}
        {disabled}
        onclick={(e) => {
          e.stopPropagation();
          openMergetool(entry.path);
        }}
      >
        <Icon name="external" size={12} />
      </button>
    </div>
  {:else}
    {#if ondiscard}
      <button
        class="discard ghost danger"
        use:tooltip={t("diff.discardChanges")}
        {disabled}
        onclick={(e) => {
          e.stopPropagation();
          ondiscard!();
        }}
      >
        <Icon name="undo" size={13} />
      </button>
    {/if}
    <span class="row-menu">
      <Menu align="right" width="240px">
        {#snippet trigger({ toggle })}
          <button
            class="ghost more"
            aria-label={t("diff.fileActions")}
            onclick={(e) => {
              e.stopPropagation();
              toggle();
            }}
          >
            <Icon name="more" size={13} />
          </button>
        {/snippet}
        <button class="item ghost" role="menuitem" onclick={() => showBlame(entry.path)}>
          <Icon name="eye" size={14} />
          {t("diff.showBlame")}
        </button>
        <button
          class="item ghost"
          role="menuitem"
          {disabled}
          onclick={() => stashPush("", [entry.path])}
        >
          <Icon name="stash" size={14} />
          {t("diff.stashFile")}
        </button>
        <button class="item ghost" role="menuitem" onclick={() => ignoreFile(entry.path)}>
          <Icon name="x" size={14} />
          {t("diff.ignoreFile")}
        </button>
        {#if ext}
          <button class="item ghost" role="menuitem" onclick={() => ignoreFile(`*.${ext}`)}>
            <Icon name="x" size={14} />
            {t("diff.ignoreExt", { ext })}
          </button>
        {/if}
      </Menu>
    </span>
  {/if}
</div>

<style>
  .row {
    display: flex;
    align-items: center;
    gap: 5px;
    padding: 3px 6px;
    border-radius: var(--radius);
    cursor: pointer;
    min-height: 28px;
  }

  .row:hover {
    background: var(--bg-hover);
  }

  /* Selection button: carries focus and keyboard operation, but looks like
     mere row content. align-self: stretch so the full row height stays
     clickable (without stretch roughly a third of the row would be dead).
     The :hover/:active surfaces of the global button rules are neutralised,
     otherwise a second surface would flash up only behind glyph+path. */
  .select {
    display: flex;
    align-items: center;
    gap: 5px;
    flex: 1;
    min-width: 0;
    align-self: stretch;
    padding: 0;
    border: none;
    background: transparent;
    box-shadow: none;
    color: inherit;
    font: inherit;
    text-align: left;
    cursor: pointer;
  }

  .select:hover,
  .select:active:not(:disabled) {
    background: transparent;
  }

  .row.selected {
    background: var(--bg-selected);
    /* Additionally mark the selection with an accent bar — --bg-selected alone
       was barely discernible against --bg-panel (the M/U letter colours are
       numerically >= 4.5:1 in the light theme). */
    box-shadow: inset 2px 0 0 var(--accent);
  }

  .stage {
    width: 22px;
    padding: 1px 0;
    justify-content: center;
    font-family: var(--mono);
    font-size: 13px;
    flex-shrink: 0;
  }

  .conflict-icon {
    color: var(--conflicted);
    width: 22px;
    display: inline-flex;
    justify-content: center;
    flex-shrink: 0;
  }

  .kind {
    font-family: var(--mono);
    font-weight: 700;
    font-size: 11px;
    width: 14px;
    text-align: center;
    flex-shrink: 0;
  }

  .path {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    /* rtl truncates on the left (the file name stays visible); plaintext prevents
       the bidi algorithm from twisting dotfiles like ".gitignore" into "gitignore.". */
    direction: rtl;
    text-align: left;
    unicode-bidi: plaintext;
  }

  .conflict-actions {
    display: flex;
    gap: 1px;
    flex-shrink: 0;
  }

  .conflict-actions button {
    font-size: 11px;
    padding: 1px 6px;
  }

  .discard,
  .more {
    opacity: 0;
    padding: 2px 5px;
    flex-shrink: 0;
  }

  .row:hover .discard,
  .row:hover .more,
  .row:focus-within .discard,
  .row:focus-within .more {
    opacity: 1;
  }
</style>
