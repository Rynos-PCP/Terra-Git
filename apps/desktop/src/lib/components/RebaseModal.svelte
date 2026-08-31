<script lang="ts">
  import type { CommitInfo, RebaseAction, RebaseStep } from "../api";
  import { t, tn, type MessageKey } from "../i18n.svelte";
  import { rebaseInteractive } from "../state.svelte";
  import Icon from "./Icon.svelte";
  import Modal from "./Modal.svelte";

  let {
    baseId,
    commits,
    onclose,
  }: {
    baseId: string;
    /** Range commits, as listed in the history (newest first). */
    commits: CommitInfo[];
    onclose: () => void;
  } = $props();

  interface Row {
    commit: CommitInfo;
    action: RebaseAction;
    /** Editable message — only used for "reword". */
    message: string;
  }

  // git todo order: oldest first -> reverse the history (new->old).
  // `commits` is constant for the lifetime of the modal (a new selection = a new
  // modal instance), so taking it over once is correct.
  // svelte-ignore state_referenced_locally
  let rows = $state<Row[]>(
    [...commits].reverse().map((commit) => ({
      commit,
      action: "pick" as RebaseAction,
      message: commit.summary,
    })),
  );
  let submitting = $state(false);

  // Hints as keys so t() is resolved in the template (reactively).
  const ACTIONS: { value: RebaseAction; label: string; hintKey: MessageKey }[] = [
    { value: "pick", label: "Pick", hintKey: "rebase.hintPick" },
    { value: "reword", label: "Reword", hintKey: "rebase.hintReword" },
    { value: "squash", label: "Squash", hintKey: "rebase.hintSquash" },
    { value: "fixup", label: "Fixup", hintKey: "rebase.hintFixup" },
    { value: "drop", label: "Drop", hintKey: "rebase.hintDrop" },
  ];

  function move(i: number, dir: -1 | 1) {
    const j = i + dir;
    if (j < 0 || j >= rows.length) return;
    const next = [...rows];
    [next[i], next[j]] = [next[j], next[i]];
    rows = next;
  }

  // ---- Drag & drop reorder (the arrow buttons stay as a keyboard fallback) ----
  let dragIdx = $state<number | null>(null);
  let overIdx = $state<number | null>(null);

  function moveTo(from: number, to: number) {
    if (from === to) return;
    const next = [...rows];
    const [item] = next.splice(from, 1);
    next.splice(to, 0, item);
    rows = next;
  }

  function onDragStart(e: DragEvent, i: number) {
    dragIdx = i;
    if (e.dataTransfer) {
      e.dataTransfer.effectAllowed = "move";
      e.dataTransfer.setData("text/plain", String(i));
    }
  }

  function onDragOver(e: DragEvent, i: number) {
    e.preventDefault();
    overIdx = i;
    if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
  }

  function onDrop(e: DragEvent, i: number) {
    e.preventDefault();
    if (dragIdx !== null) moveTo(dragIdx, i);
    dragIdx = null;
    overIdx = null;
  }

  // Single-key shortcuts on the focused plan row: p/r/s/f/d set the action
  // directly. Deliberately NO 'e' — edit does not exist here.
  const KEY_ACTIONS: Record<string, RebaseAction> = {
    p: "pick",
    r: "reword",
    s: "squash",
    f: "fixup",
    d: "drop",
  };

  function onRowKeydown(e: KeyboardEvent, i: number) {
    if (e.ctrlKey || e.metaKey || e.altKey) return;
    // Not in input fields (the reword text) and not in the <select> — the
    // dropdown's native letter selection applies there.
    if ((e.target as HTMLElement).closest("input, textarea, select")) return;
    const action = KEY_ACTIONS[e.key.toLowerCase()];
    if (!action) return;
    e.preventDefault();
    rows[i].action = action;
  }

  // The first kept (non-drop) action has to be "pick"/"reword" — squash/fixup
  // needs a predecessor to fall into.
  const firstKeptIsSquash = $derived.by(() => {
    const first = rows.find((r) => r.action !== "drop");
    return !!first && first.action !== "pick" && first.action !== "reword";
  });
  const allDropped = $derived(rows.every((r) => r.action === "drop"));
  const kept = $derived(rows.filter((r) => r.action !== "drop").length);
  const rewordEmpty = $derived(
    rows.some((r) => r.action === "reword" && r.message.trim().length === 0),
  );

  async function run() {
    if (submitting || firstKeptIsSquash || rewordEmpty) return;
    submitting = true;
    const steps: RebaseStep[] = rows.map((r) => ({
      action: r.action,
      commitId: r.commit.id,
      message: r.action === "reword" ? r.message.trim() : null,
    }));
    await rebaseInteractive(baseId, steps);
    submitting = false;
    // On a conflict the modal state does not matter — the banner takes over; close.
    onclose();
  }
</script>

<Modal title={t("rebase.title", { n: commits.length })} width="620px" {onclose}>
  <p class="hint">{t("rebase.hint")}</p>

  <ul class="plan">
    {#each rows as row, i (row.commit.id)}
      <!-- The row itself is not focusable; the keydown listener catches
           only the single keys p/r/s/f/d bubbling up from the focusable
           children (buttons). -->
      <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
      <li
        class:drop={row.action === "drop"}
        class:dragging={dragIdx === i}
        class:drag-over={overIdx === i && dragIdx !== null && dragIdx !== i}
        draggable="true"
        ondragstart={(e) => onDragStart(e, i)}
        ondragover={(e) => onDragOver(e, i)}
        ondragleave={() => overIdx === i && (overIdx = null)}
        ondrop={(e) => onDrop(e, i)}
        ondragend={() => {
          dragIdx = null;
          overIdx = null;
        }}
        onkeydown={(e) => onRowKeydown(e, i)}
      >
        <span class="grip" title={t("rebase.dragToReorder")} aria-hidden="true">
          <Icon name="more" size={13} />
        </span>
        <span class="reorder">
          <button
            class="ghost"
            title={t("rebase.moveUp")}
            disabled={i === 0}
            onclick={() => move(i, -1)}
          >
            <Icon name="chevronDown" size={12} />
          </button>
          <button
            class="ghost"
            title={t("rebase.moveDown")}
            disabled={i === rows.length - 1}
            onclick={() => move(i, 1)}
          >
            <Icon name="chevronDown" size={12} />
          </button>
        </span>
        <select
          bind:value={row.action}
          aria-label={t("rebase.actionFor", { sha: row.commit.shortId })}
        >
          {#each ACTIONS as a (a.value)}
            <option value={a.value} title={t(a.hintKey)}>{a.label}</option>
          {/each}
        </select>
        <code class="sha">{row.commit.shortId}</code>
        {#if row.action === "reword"}
          <input
            class="reword-input"
            type="text"
            placeholder={t("rebase.newMessagePlaceholder")}
            bind:value={row.message}
            aria-label={t("rebase.newMessageFor", { sha: row.commit.shortId })}
          />
        {:else}
          <span class="summary" title={row.commit.summary}
            >{row.commit.summary || t("rebase.noTitle")}</span
          >
        {/if}
      </li>
    {/each}
  </ul>

  {#if firstKeptIsSquash}
    <p class="warn">{t("rebase.warnFirstSquash")}</p>
  {:else if rewordEmpty}
    <p class="warn">{t("rebase.warnRewordEmpty")}</p>
  {:else if allDropped}
    <p class="warn">{t("rebase.warnAllDropped")}</p>
  {/if}

  <div class="actions">
    <button onclick={onclose}>{t("common.cancel")}</button>
    <button class="primary" disabled={submitting || firstKeptIsSquash || rewordEmpty} onclick={run}>
      {#if submitting}<span class="spin"></span>{/if}
      {tn("rebase.run", kept)}
    </button>
  </div>
</Modal>

<style>
  .hint {
    color: var(--text-muted);
    font-size: 12px;
    line-height: 1.5;
  }

  .plan {
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 2px;
    max-height: 46vh;
    overflow-y: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: var(--space-1);
  }

  .plan li {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 3px 4px;
    border-radius: var(--radius-sm);
  }

  .plan li:hover {
    background: var(--bg-hover);
  }

  .plan li.drop {
    opacity: 0.5;
  }

  .plan li.dragging {
    opacity: 0.4;
  }

  /* Insertion indicator: an accent line at the top edge of the target element. */
  .plan li.drag-over {
    box-shadow: inset 0 2px 0 var(--accent);
  }

  .grip {
    display: inline-flex;
    color: var(--text-faint);
    cursor: grab;
    flex-shrink: 0;
    transform: rotate(90deg);
  }

  .plan li.dragging .grip {
    cursor: grabbing;
  }

  .plan li.drop .summary {
    text-decoration: line-through;
  }

  .reorder {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }

  .reorder button {
    padding: 0 3px;
    line-height: 1;
  }

  .reorder button:first-child :global(svg) {
    transform: rotate(180deg);
  }

  .plan select {
    width: 92px;
    flex-shrink: 0;
    padding: 3px 6px;
  }

  .sha {
    font-family: var(--mono);
    font-size: 11.5px;
    color: var(--blue);
    flex-shrink: 0;
  }

  .summary {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .reword-input {
    flex: 1;
    min-height: 24px;
    padding: 2px 6px;
    font-size: 12px;
  }

  .warn {
    color: var(--modified);
    font-size: 12px;
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
  }
</style>
