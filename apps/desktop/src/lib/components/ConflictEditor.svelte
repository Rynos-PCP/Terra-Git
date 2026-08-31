<script lang="ts">
  import { api, type ConflictFile, type ConflictSegment, type OpContext } from "../api";
  import { workshopCopy } from "../conflictWorkshop";
  import { t } from "../i18n.svelte";
  import { saveConflictResolution, showError, ui } from "../state.svelte";
  import Modal from "./Modal.svelte";

  let { file, onclose }: { file: string; onclose: () => void } = $props();

  // Side naming as in the conflict workshop: get_op_context returns branch/
  // commit names instead of a generic "Mine (HEAD)/Incoming". Falls back to the
  // generic labels without a context.
  let ctx = $state<OpContext | null>(null);
  $effect(() => {
    const repo = ui.repo;
    if (!repo) return;
    api
      .opContext(repo.path)
      .then((c) => (ctx = c))
      .catch(() => (ctx = null));
  });
  const copy = $derived(workshopCopy(ctx));
  const oursHead = $derived(t(copy.ours.key, copy.ours.params));
  const theirsHead = $derived(t(copy.theirs.key, copy.theirs.params));

  let data = $state<ConflictFile | null>(null);
  let loadError = $state(false);
  // Resolution per conflict segment index (null = still open).
  let resolutions = $state<Record<number, string>>({});
  let submitting = $state(false);

  $effect(() => {
    const repo = ui.repo;
    if (!repo) return;
    api
      .readConflict(repo.path, file)
      .then((cf) => {
        data = cf;
        resolutions = {};
      })
      .catch((e) => {
        loadError = true;
        showError(e);
      });
  });

  const conflictIdxs = $derived(
    data ? data.segments.map((s, i) => (s.kind === "conflict" ? i : -1)).filter((i) => i >= 0) : [],
  );
  const resolvedCount = $derived(conflictIdxs.filter((i) => resolutions[i] !== undefined).length);
  const allResolved = $derived(conflictIdxs.length > 0 && resolvedCount === conflictIdxs.length);

  function choose(idx: number, seg: ConflictSegment, which: "ours" | "theirs" | "ot" | "to") {
    const parts =
      which === "ours"
        ? seg.ours
        : which === "theirs"
          ? seg.theirs
          : which === "ot"
            ? [...seg.ours, ...seg.theirs]
            : [...seg.theirs, ...seg.ours];
    resolutions = { ...resolutions, [idx]: parts.join("\n") };
  }

  function edit(idx: number, value: string) {
    resolutions = { ...resolutions, [idx]: value };
  }

  async function save() {
    if (!data || submitting || !allResolved) return;
    submitting = true;
    // Assemble the file from the context + the resolved conflicts.
    const out: string[] = [];
    data.segments.forEach((seg, i) => {
      if (seg.kind === "context") out.push(...seg.lines);
      else out.push(...(resolutions[i] ?? "").split("\n"));
    });
    const nl = data.eol === "crlf" ? "\r\n" : "\n";
    const content = out.join(nl) + nl;
    await saveConflictResolution(file, content);
    submitting = false;
    onclose();
  }
</script>

<Modal title={t("conflict.title", { file })} width="820px" {onclose}>
  {#if loadError}
    <p class="warn">{t("conflict.readFailed")}</p>
  {:else if !data}
    <div class="loading"><span class="spin"></span> {t("conflict.loading")}</div>
  {:else if !data.hasConflicts}
    <p class="hint">{t("conflict.noMarkers")}</p>
  {:else}
    <div class="status">
      <strong
        >{t("conflict.resolvedCount", { done: resolvedCount, total: conflictIdxs.length })}</strong
      >
      {t("conflict.resolvedSuffix")}
    </div>
    <div class="segments">
      {#each data.segments as seg, i (i)}
        {#if seg.kind === "context"}
          {#if seg.lines.length > 0}
            <pre class="context">{seg.lines.join("\n")}</pre>
          {/if}
        {:else}
          <div class="conflict" class:done={resolutions[i] !== undefined}>
            <div class="sides">
              <div class="side ours">
                <header>{oursHead}</header>
                <pre>{seg.ours.join("\n") || t("conflict.empty")}</pre>
              </div>
              {#if seg.base}
                <div class="side base">
                  <header>{t("conflict.base")}</header>
                  <pre>{seg.base.join("\n") || t("conflict.empty")}</pre>
                </div>
              {/if}
              <div class="side theirs">
                <header>{theirsHead}</header>
                <pre>{seg.theirs.join("\n") || t("conflict.empty")}</pre>
              </div>
            </div>
            <div class="choices">
              <button class="ghost" onclick={() => choose(i, seg, "ours")}
                >{t("conflict.ours")}</button
              >
              <button class="ghost" onclick={() => choose(i, seg, "theirs")}
                >{t("conflict.theirs")}</button
              >
              <button class="ghost" onclick={() => choose(i, seg, "ot")}
                >{t("conflict.bothOursFirst")}</button
              >
              <button class="ghost" onclick={() => choose(i, seg, "to")}
                >{t("conflict.bothTheirsFirst")}</button
              >
            </div>
            {#if resolutions[i] !== undefined}
              <textarea
                class="result"
                rows={Math.min(10, Math.max(2, resolutions[i].split("\n").length))}
                value={resolutions[i]}
                oninput={(e) => edit(i, (e.currentTarget as HTMLTextAreaElement).value)}></textarea>
            {:else}
              <div class="unresolved">{t("conflict.unresolved")}</div>
            {/if}
          </div>
        {/if}
      {/each}
    </div>
    <div class="actions">
      <button onclick={onclose}>{t("common.cancel")}</button>
      <button class="primary" disabled={!allResolved || submitting} onclick={save}>
        {#if submitting}<span class="spin"></span>{/if}
        {t("conflict.saveAndStage")}
      </button>
    </div>
  {/if}
</Modal>

<style>
  .status {
    font-size: 13px;
  }

  .hint,
  .warn {
    font-size: 12px;
    color: var(--text-muted);
  }
  .warn {
    color: var(--modified);
  }

  .loading {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    color: var(--text-muted);
  }

  .segments {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    max-height: 56vh;
    overflow-y: auto;
  }

  pre {
    font-family: var(--mono);
    font-size: 12px;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    margin: 0;
    -webkit-user-select: text;
    user-select: text;
  }

  .context {
    color: var(--text-muted);
    padding: 2px var(--space-2);
    border-left: 2px solid var(--border);
  }

  .conflict {
    border: 1px solid var(--border-strong);
    border-radius: var(--radius);
    overflow: hidden;
  }

  .conflict.done {
    border-color: var(--accent-dim);
  }

  .sides {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 1px;
    background: var(--border);
  }

  .sides:has(.base) {
    grid-template-columns: 1fr 1fr 1fr;
  }

  .side {
    background: var(--bg-inset);
    min-width: 0;
  }

  .side header {
    font-size: 11px;
    font-weight: 600;
    padding: 3px var(--space-2);
    color: var(--text-muted);
    border-bottom: 1px solid var(--border);
  }

  .side.ours header {
    color: var(--accent);
  }
  .side.theirs header {
    color: var(--blue);
  }

  .side pre {
    padding: var(--space-2);
    max-height: 160px;
    overflow: auto;
  }

  .choices {
    display: flex;
    gap: var(--space-1);
    padding: var(--space-1) var(--space-2);
    border-top: 1px solid var(--border);
  }

  .choices button {
    font-size: 11.5px;
    padding: 2px 8px;
  }

  .result {
    width: 100%;
    border: none;
    border-top: 1px solid var(--border);
    border-radius: 0;
    font-family: var(--mono);
    font-size: 12px;
  }

  .unresolved {
    padding: var(--space-1) var(--space-2);
    font-size: 12px;
    color: var(--modified);
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
  }
</style>
