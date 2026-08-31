<script lang="ts">
  import { onMount } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import { confirm } from "@tauri-apps/plugin-dialog";
  import type { DiffHunk, DiffLine, FileDiff, UnchangedInfo } from "../api";
  import { formatBytes } from "../format";
  import { highlightLine, langForFile } from "../highlight";
  import { t, tn } from "../i18n.svelte";
  import {
    applyHunk,
    applyLines,
    discardHunk,
    showBlame,
    ui,
    type ModalKind,
  } from "../state.svelte";
  import { findTargetActive } from "../findScope";
  import FindBar from "./FindBar.svelte";
  import Icon from "./Icon.svelte";

  let {
    diffs,
    emptyText,
    interactive = false,
    staged = false,
    loading = false,
    unchangedInfo = null,
    findScope = null,
  }: {
    diffs: FileDiff[];
    /** Placeholder for an empty diff; without it t("diff.noChanges"). */
    emptyText?: string;
    /**
     * Explanation for a file reported as changed without a content diff.
     * Deliberately a prop instead of reaching into the global state: the
     * explanation applies to exactly the ONE selected working-tree file.
     * Commit and stash views render several foreign files and therefore leave
     * the prop out.
     */
    unchangedInfo?: UnchangedInfo | null;
    /** true in the changes view: hunk/line staging is active. */
    interactive?: boolean;
    /** Diff direction of the changes view (staged vs unstaged). */
    staged?: boolean;
    /** true while the diff is still being computed/loaded (spinner). */
    loading?: boolean;
    /**
     * The modal kind this instance is rendered in; null = the main view.
     * Controls which instance serves Ctrl+F/Ctrl+G when several DiffViews hang
     * in the DOM at the same time (the main diff behind an open modal).
     */
    findScope?: ModalKind | null;
  } = $props();

  // Reactive so a language change updates the fallback text immediately.
  const emptyLabel = $derived(emptyText ?? t("diff.noChanges"));

  const EOL_LABEL = {
    lf: "diff.eolLf",
    crlf: "diff.eolCrlf",
    mixed: "diff.eolMixed",
    none: "diff.eolNone",
  } as const;

  /**
   * Translates the engine's classification into a title, body text and details.
   * `null` while there is no explanation — the previous hint then stays.
   */
  const unchanged = $derived.by(() => {
    const info = unchangedInfo;
    if (!info) return null;
    const eol = (v: string | null | undefined) =>
      v ? t(EOL_LABEL[v as keyof typeof EOL_LABEL]) : null;
    const details: string[] = [];

    switch (info.reason) {
      case "eolOnly": {
        const base = eol(info.oldEol);
        const work = eol(info.newEol);
        const expected = eol(info.expectedEol);
        // On the unstaged path the left side is the index, not HEAD — "in the
        // repository" would simply be wrong there.
        if (base) {
          details.push(
            t(staged ? "diff.unchangedEolHead" : "diff.unchangedEolIndex", { eol: base }),
          );
        }
        if (work) details.push(t("diff.unchangedEolWorktree", { eol: work }));
        // Only show it when the checkout would actually write something else —
        // otherwise the line is just noise.
        if (expected && info.expectedEol !== info.newEol) {
          details.push(t("diff.unchangedEolExpected", { eol: expected }));
        }
        return {
          title: t("diff.unchangedEolTitle"),
          // Without a counterpart the content is identical and only differs from
          // what a checkout would write — that is a different statement than
          // "the two sides differ".
          body: base ? t("diff.unchangedEolBody") : t("diff.unchangedEolExpectedBody"),
          details,
        };
      }
      case "modeOnly":
        if (info.oldMode && info.newMode) {
          details.push(t("diff.unchangedModeChange", { old: info.oldMode, new: info.newMode }));
        }
        return {
          title: t("diff.unchangedModeTitle"),
          body: t("diff.unchangedModeBody"),
          details,
        };
      case "identical":
        return {
          title: t("diff.unchangedIdenticalTitle"),
          body: t("diff.unchangedIdenticalBody"),
          details,
        };
      default:
        return {
          title: t("diff.unchangedUnknownTitle"),
          body: t("diff.unchangedUnknownBody"),
          details,
        };
    }
  });

  // Safety net against huge diffs: render capped per file.
  const MAX_LINES_PER_FILE = 4000;

  // Line selection for partial staging: "hunkIdx:lineIdx".
  const selection = new SvelteSet<string>();

  $effect(() => {
    // Reset the selection when a different diff is loaded.
    void diffs;
    selection.clear();
  });

  const selectedByHunk = $derived.by(() => {
    // A pure intermediate result of a $derived — reactivity comes from
    // `selection`, the map itself is never mutated after it is built.
    // eslint-disable-next-line svelte/prefer-svelte-reactivity
    const map = new Map<number, number[]>();
    for (const key of selection) {
      const [h, l] = key.split(":").map(Number);
      map.set(h, [...(map.get(h) ?? []), l]);
    }
    return map;
  });

  function toggleLine(hunkIdx: number, lineIdx: number, line: DiffLine) {
    if (!interactive || line.kind === "context") return;
    const key = `${hunkIdx}:${lineIdx}`;
    if (selection.has(key)) selection.delete(key);
    else selection.add(key);
  }

  let applying = $state(false);

  async function stageSelection(file: string) {
    if (applying) return; // double-click protection
    applying = true;
    try {
      // IMPORTANT: process hunks in DESCENDING order. Every applyLines
      // recomputes the diff; a staged hunk disappears and would shift the
      // indices of higher hunks. From the back, the still open (lower) indices
      // stay stable.
      const hunks = [...selectedByHunk.entries()].sort((a, b) => b[0] - a[0]);
      for (const [hunkIdx, lines] of hunks) {
        await applyLines(
          file,
          hunkIdx,
          [...lines].sort((a, b) => a - b),
          staged,
        );
      }
      selection.clear();
    } finally {
      applying = false;
    }
  }

  async function confirmDiscardHunk(file: string, hunkIdx: number) {
    const yes = await confirm(t("diff.discardHunkConfirm", { file }), {
      title: t("diff.discardHunkTitle"),
      kind: "warning",
    });
    if (yes) await discardHunk(file, hunkIdx);
  }

  function isImage(path: string): boolean {
    return /\.(png|jpe?g|gif|webp|bmp|ico|svg)$/i.test(path);
  }

  // ---- Split view: arrange the lines of a hunk in pairs ----
  interface SplitRow {
    left: DiffLine | null;
    right: DiffLine | null;
    leftIdx: number;
    rightIdx: number;
  }

  function splitRows(hunk: DiffHunk): SplitRow[] {
    const rows: SplitRow[] = [];
    let pendingDel: { line: DiffLine; idx: number }[] = [];
    let pendingAdd: { line: DiffLine; idx: number }[] = [];

    const flush = () => {
      const n = Math.max(pendingDel.length, pendingAdd.length);
      for (let i = 0; i < n; i++) {
        rows.push({
          left: pendingDel[i]?.line ?? null,
          leftIdx: pendingDel[i]?.idx ?? -1,
          right: pendingAdd[i]?.line ?? null,
          rightIdx: pendingAdd[i]?.idx ?? -1,
        });
      }
      pendingDel = [];
      pendingAdd = [];
    };

    hunk.lines.forEach((line, idx) => {
      if (line.kind === "deletion") pendingDel.push({ line, idx });
      else if (line.kind === "addition") pendingAdd.push({ line, idx });
      else {
        flush();
        rows.push({ left: line, leftIdx: idx, right: line, rightIdx: idx });
      }
    });
    flush();
    return rows;
  }

  // ---- Search (Ctrl+F) & "go to line" (Ctrl+G) in the visible diff ----
  let rootEl = $state<HTMLElement>();
  let findOpen = $state(false);
  let findMode = $state<"text" | "goto">("text");
  let findHits = $state<string[]>([]); // keys "fi:hi:li"
  let findCur = $state(0); // 1-based, 0 = none

  const findHitSet = $derived(new Set(findHits));
  const findCurKey = $derived(findCur > 0 ? findHits[findCur - 1] : null);

  function closeFind() {
    findOpen = false;
    findHits = [];
    findCur = 0;
  }

  $effect(() => {
    // A different diff was loaded -> reset the search.
    void diffs;
    closeFind();
  });

  $effect(() => {
    // A modal lays itself over the main diff -> close the search bar left behind
    // so it does not stay open under the modal.
    if (!findTargetActive(findScope, ui.modal?.kind ?? null, diffs.length > 0)) closeFind();
  });

  function runFindQuery(q: string) {
    if (findMode === "goto") {
      gotoLine(parseInt(q.trim(), 10));
      return;
    }
    const needle = q.trim().toLowerCase();
    const hits: string[] = [];
    if (needle) {
      diffs.forEach((file, fi) => {
        let before = 0;
        for (const [hi, hunk] of file.hunks.entries()) {
          if (before >= MAX_LINES_PER_FILE) break;
          const take = Math.min(hunk.lines.length, MAX_LINES_PER_FILE - before);
          for (let li = 0; li < take; li++) {
            if (hunk.lines[li].content.toLowerCase().includes(needle)) {
              hits.push(`${fi}:${hi}:${li}`);
            }
          }
          before += hunk.lines.length;
        }
      });
    }
    findHits = hits;
    findCur = hits.length > 0 ? 1 : 0;
    if (hits.length > 0) scrollToHit(hits[0]);
  }

  function navFind(dir: 1 | -1) {
    if (findHits.length === 0) return;
    findCur = ((findCur - 1 + dir + findHits.length) % findHits.length) + 1;
    scrollToHit(findHits[findCur - 1]);
  }

  function gotoLine(n: number) {
    if (!Number.isFinite(n) || n < 1) return;
    for (const [fi, file] of diffs.entries()) {
      for (const [hi, hunk] of file.hunks.entries()) {
        for (const [li, line] of hunk.lines.entries()) {
          if (line.newLineno === n || (line.newLineno === null && line.oldLineno === n)) {
            const key = `${fi}:${hi}:${li}`;
            findHits = [key];
            findCur = 1;
            scrollToHit(key);
            return;
          }
        }
      }
    }
  }

  function scrollToHit(key: string) {
    // Let it render first (classes/attributes), then scroll.
    requestAnimationFrame(() => {
      rootEl
        ?.querySelector(`[data-line="${key}"], [data-line2="${key}"]`)
        ?.scrollIntoView({ block: "center" });
    });
  }

  onMount(() => {
    // Only the visible main diff reacts; open modals take precedence (the blame
    // view brings its own search).
    // Exactly the instance whose scope matches the open modal serves it — the
    // main diff stays silent while a modal is open.
    const canHandle = () => findTargetActive(findScope, ui.modal?.kind ?? null, diffs.length > 0);
    const onFind = () => {
      if (canHandle()) {
        findMode = "text";
        findOpen = true;
      }
    };
    const onGoto = () => {
      if (canHandle()) {
        findMode = "goto";
        findOpen = true;
      }
    };
    const onNext = (e: Event) => {
      // canHandle() is mandatory, not cosmetic: after the scoping, the main and
      // the modal diff can be findOpen at the same time, and F3 would otherwise
      // scroll the hidden main diff along in the background.
      if (findOpen && canHandle()) navFind(((e as CustomEvent).detail as 1 | -1) ?? 1);
    };
    window.addEventListener("app-find", onFind);
    window.addEventListener("app-goto", onGoto);
    window.addEventListener("app-find-next", onNext);
    return () => {
      window.removeEventListener("app-find", onFind);
      window.removeEventListener("app-goto", onGoto);
      window.removeEventListener("app-find-next", onNext);
    };
  });
</script>

{#snippet codeCell(line: DiffLine, lang: string | null)}
  <span class="sigil">{line.kind === "addition" ? "+" : line.kind === "deletion" ? "−" : " "}</span
  ><!-- eslint-disable-next-line svelte/no-at-html-tags — highlightLine always escapes -->{@html highlightLine(
    line.content,
    lang,
  )}
{/snippet}

<div class="diff-wrap" bind:this={rootEl}>
  {#if findOpen}
    <div class="find-holder">
      <FindBar
        mode={findMode}
        count={findHits.length}
        current={findCur}
        onquery={runFindQuery}
        onnav={navFind}
        onclose={closeFind}
      />
    </div>
  {/if}
  <div class="diff-scroll">
    {#if diffs.length === 0}
      <div class="placeholder" class:idle={!loading}>
        {#if loading}
          <span class="spin"></span>
          <span>{t("diff.loading")}</span>
        {:else}
          <Icon name="file" size={28} strokeWidth={1.2} />
          <span>{emptyLabel}</span>
        {/if}
      </div>
    {:else}
      {#each diffs as file, fi (file.path + (file.oldPath ?? ""))}
        {@const totalLines = file.hunks.reduce((n, h) => n + h.lines.length, 0)}
        {@const adds = file.hunks.reduce(
          (n, h) => n + h.lines.filter((l) => l.kind === "addition").length,
          0,
        )}
        {@const dels = file.hunks.reduce(
          (n, h) => n + h.lines.filter((l) => l.kind === "deletion").length,
          0,
        )}
        {@const lang = langForFile(file.path)}
        {@const selCount = selection.size}
        <section class="file">
          <header class="file-head">
            <Icon name={isImage(file.path) ? "image" : "file"} size={14} />
            <span class="path" title={file.path}>
              {#if file.oldPath}{file.oldPath} →
              {/if}{file.path}
            </span>
            {#if !file.isBinary && (adds > 0 || dels > 0)}
              <span class="stats" title={t("diff.stats", { adds, dels })}>
                <span class="plus">+{adds}</span>
                <span class="minus">−{dels}</span>
              </span>
            {/if}
            <span class="spacer"></span>
            {#if interactive && selCount > 0}
              <button
                class="primary sel-action"
                disabled={applying}
                onclick={() => stageSelection(file.path)}
              >
                {staged ? tn("diff.unstageLines", selCount) : tn("diff.stageLines", selCount)}
              </button>
              <button class="ghost" onclick={() => selection.clear()}
                >{t("diff.clearSelection")}</button
              >
            {/if}
            {#if interactive}
              <button
                class="ghost"
                title={t("diff.showBlame")}
                onclick={() => showBlame(file.path)}
              >
                <Icon name="eye" size={14} />
              </button>
            {/if}
          </header>

          {#if file.isBinary && isImage(file.path) && interactive && ui.imageDiff}
            <div class="image-diff">
              <div class="image-side">
                <span class="section-title">{t("diff.before")}</span>
                {#if ui.imageDiff.oldDataUrl}
                  <img src={ui.imageDiff.oldDataUrl} alt={t("diff.oldVersionAlt")} />
                {:else}
                  <div class="no-image">{t("diff.noOldVersion")}</div>
                {/if}
              </div>
              <div class="image-side">
                <span class="section-title">{t("diff.after")}</span>
                {#if ui.imageDiff.newDataUrl}
                  <img src={ui.imageDiff.newDataUrl} alt={t("diff.newVersionAlt")} />
                {:else}
                  <div class="no-image">{t("diff.imageDeleted")}</div>
                {/if}
              </div>
            </div>
          {:else if file.isBinary}
            <div class="note">
              <p>{t("diff.binaryNote")}</p>
              {#if file.oldSize != null && file.newSize != null}
                <!-- Changed: both sides present -> old → new (Δ) -->
                <p class="binary-size">
                  {formatBytes(file.oldSize)} → {formatBytes(file.newSize)}
                  <span class:danger-text={file.newSize < file.oldSize}>
                    (Δ {file.newSize >= file.oldSize ? "+" : "−"}{formatBytes(
                      Math.abs(file.newSize - file.oldSize),
                    )})
                  </span>
                </p>
              {:else if file.newSize != null}
                <!-- Added: no old size -> only the new side. -->
                <p class="binary-size">{t("diff.newFile")}: {formatBytes(file.newSize)}</p>
              {:else if file.oldSize != null}
                <!-- Deleted: no new size -> only the old side. -->
                <p class="binary-size">{t("diff.oldFile")}: {formatBytes(file.oldSize)}</p>
              {/if}
            </div>
          {:else if file.hunks.length === 0}
            <!-- Reported as changed, but without a content diff. The engine returns
               the cause afterwards; until then (or when it cannot be
               determined) the previous hint stays. -->
            {#if unchanged}
              <div class="note unchanged">
                <p class="unchanged-title">{unchanged.title}</p>
                <p>{unchanged.body}</p>
                {#if unchanged.details.length > 0}
                  <ul class="unchanged-details">
                    {#each unchanged.details as detail (detail)}
                      <li>{detail}</li>
                    {/each}
                  </ul>
                {/if}
              </div>
            {:else}
              <div class="note">{t("diff.noContentChanges")}</div>
            {/if}
          {:else}
            {#if file.truncated}
              <div class="note">{t("diff.truncatedByEngine")}</div>
            {:else if totalLines > MAX_LINES_PER_FILE}
              <div class="note">
                {t("diff.truncatedView", { n: MAX_LINES_PER_FILE })}
              </div>
            {/if}

            {#each file.hunks as hunk, hi (hi)}
              {@const linesBefore = file.hunks.slice(0, hi).reduce((n, h) => n + h.lines.length, 0)}
              {#if linesBefore < MAX_LINES_PER_FILE}
                <div class="hunk">
                  <div class="hunk-head">
                    <code>{hunk.header}</code>
                    <span class="spacer"></span>
                    {#if interactive}
                      <div class="hunk-actions">
                        <button
                          class="ghost"
                          disabled={ui.working > 0 || applying}
                          onclick={() => applyHunk(file.path, hi, staged)}
                          title={staged ? t("diff.unstageHunk") : t("diff.stageHunk")}
                        >
                          {staged ? `− ${t("diff.unstage")}` : `+ ${t("diff.stage")}`}
                        </button>
                        {#if !staged}
                          <button
                            class="ghost danger"
                            disabled={ui.working > 0 || applying}
                            onclick={() => confirmDiscardHunk(file.path, hi)}
                            title={t("diff.discardHunkTitle")}
                          >
                            <Icon name="undo" size={13} />
                            {t("diff.discard")}
                          </button>
                        {/if}
                      </div>
                    {/if}
                  </div>

                  {#if ui.diffMode === "split" && !interactive}
                    <!-- Split view (history): old on the left, new on the right -->
                    <table class="hunks split">
                      <tbody>
                        {#each splitRows(hunk) as row, ri (ri)}
                          <tr
                            data-line={row.leftIdx >= 0 ? `${fi}:${hi}:${row.leftIdx}` : undefined}
                            data-line2={row.rightIdx >= 0
                              ? `${fi}:${hi}:${row.rightIdx}`
                              : undefined}
                            class:find-hit={findOpen &&
                              (findHitSet.has(`${fi}:${hi}:${row.leftIdx}`) ||
                                findHitSet.has(`${fi}:${hi}:${row.rightIdx}`))}
                            class:find-cur={findOpen &&
                              (findCurKey === `${fi}:${hi}:${row.leftIdx}` ||
                                findCurKey === `${fi}:${hi}:${row.rightIdx}`)}
                          >
                            <td class="ln">{row.left?.oldLineno ?? ""}</td>
                            <td
                              class="code half {row.left
                                ? row.left.kind === 'deletion'
                                  ? 'deletion'
                                  : ''
                                : 'void'}"
                            >
                              {#if row.left}{@render codeCell(row.left, lang)}{/if}
                            </td>
                            <td class="ln">{row.right?.newLineno ?? ""}</td>
                            <td
                              class="code half {row.right
                                ? row.right.kind === 'addition'
                                  ? 'addition'
                                  : ''
                                : 'void'}"
                            >
                              {#if row.right}{@render codeCell(row.right, lang)}{/if}
                            </td>
                          </tr>
                        {/each}
                      </tbody>
                    </table>
                  {:else}
                    <table class="hunks">
                      <tbody>
                        {#each hunk.lines.slice(0, Math.max(0, MAX_LINES_PER_FILE - linesBefore)) as line, li (li)}
                          {@const selected = selection.has(`${hi}:${li}`)}
                          <tr
                            class={line.kind}
                            class:selected
                            class:selectable={interactive && line.kind !== "context"}
                            data-line={`${fi}:${hi}:${li}`}
                            class:find-hit={findOpen && findHitSet.has(`${fi}:${hi}:${li}`)}
                            class:find-cur={findOpen && findCurKey === `${fi}:${hi}:${li}`}
                          >
                            <td
                              class="ln"
                              onclick={() => toggleLine(hi, li, line)}
                              title={interactive && line.kind !== "context"
                                ? t("diff.selectLineTitle")
                                : undefined}>{line.oldLineno ?? ""}</td
                            >
                            <td class="ln" onclick={() => toggleLine(hi, li, line)}
                              >{line.newLineno ?? ""}</td
                            >
                            <td class="code">{@render codeCell(line, lang)}</td>
                          </tr>
                        {/each}
                      </tbody>
                    </table>
                  {/if}
                </div>
              {/if}
            {/each}
          {/if}
        </section>
      {/each}
    {/if}
  </div>
</div>

<style>
  .diff-wrap {
    position: relative;
    height: 100%;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }

  .find-holder {
    position: absolute;
    top: var(--space-2);
    right: var(--space-5);
    z-index: 20;
  }

  .diff-scroll {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: var(--space-3);
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .placeholder {
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    color: var(--text-muted);
  }

  .placeholder.idle {
    flex-direction: column;
    gap: var(--space-3);
    color: var(--text-faint);
  }

  .placeholder.idle :global(svg) {
    opacity: 0.55;
  }

  .file {
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    overflow: hidden;
    background: var(--bg-panel);
    flex-shrink: 0;
  }

  .file-head {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    background: var(--bg-elevated);
    border-bottom: 1px solid var(--border);
    font-family: var(--mono);
    font-size: 12px;
    color: var(--text-muted);
  }

  .path {
    -webkit-user-select: text;
    user-select: text;
    color: var(--text-primary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .stats {
    display: inline-flex;
    gap: 6px;
    font-variant-numeric: tabular-nums;
    font-size: 11.5px;
    flex-shrink: 0;
  }

  .stats .plus {
    color: var(--added);
  }

  .stats .minus {
    color: var(--deleted);
  }

  .spacer {
    flex: 1;
  }

  .sel-action {
    font-family: var(--sans);
  }

  .note {
    padding: var(--space-2) var(--space-3);
    color: var(--text-muted);
    font-size: 12px;
  }

  /* Explanation for a file reported as changed without a content diff:
     a bit more room than a mere hint, because a diagnosis stands here. */
  .unchanged {
    padding: var(--space-3);
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .unchanged-title {
    color: var(--text-primary);
    font-weight: 600;
    font-size: 13px;
  }

  .unchanged-details {
    margin-top: var(--space-1);
    padding-left: var(--space-3);
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 2px;
    color: var(--text-faint);
    font-variant-numeric: tabular-nums;
  }

  .unchanged-details li::before {
    content: "·";
    margin-right: var(--space-2);
  }

  .binary-size {
    margin-top: var(--space-1);
    font-variant-numeric: tabular-nums;
  }

  .danger-text {
    color: var(--deleted);
  }

  .hunk-head {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    background: var(--hunk-bg);
    padding: 2px var(--space-3);
    border-top: 1px solid var(--border);
  }

  .hunk:first-of-type .hunk-head {
    border-top: none;
  }

  .hunk-head code {
    font-family: var(--mono);
    font-size: 11.5px;
    color: var(--blue);
  }

  .hunk-actions {
    display: flex;
    gap: 2px;
    opacity: 0;
    transition: opacity 0.12s ease;
  }

  .hunk:hover .hunk-actions,
  .hunk-actions:focus-within {
    opacity: 1;
  }

  .hunk-actions button {
    font-size: 11.5px;
    padding: 1px 8px;
  }

  .hunks {
    width: 100%;
    border-collapse: collapse;
    font-family: var(--mono);
    font-size: 12px;
    line-height: 1.55;
  }

  .ln {
    width: 42px;
    min-width: 42px;
    padding: 0 8px;
    text-align: right;
    color: var(--text-faint);
    background: var(--bg-panel);
    border-right: 1px solid var(--border);
    vertical-align: top;
    -webkit-user-select: none;
    user-select: none;
  }

  tr.selectable .ln {
    cursor: pointer;
  }

  tr.selectable:hover .ln {
    color: var(--accent);
  }

  .code {
    padding: 0 10px;
    white-space: pre-wrap;
    word-break: break-all;
    -webkit-user-select: text;
    user-select: text;
    cursor: text;
    width: 100%;
  }

  .code.half {
    width: 50%;
  }

  .code.void {
    background: var(--bg-inset);
  }

  .sigil {
    display: inline-block;
    width: 13px;
    color: var(--text-faint);
    -webkit-user-select: none;
    user-select: none;
  }

  tr.addition .code,
  .code.addition {
    background: var(--add-bg);
  }

  tr.deletion .code,
  .code.deletion {
    background: var(--del-bg);
  }

  tr.addition .sigil {
    color: var(--added);
  }

  tr.deletion .sigil {
    color: var(--deleted);
  }

  tr.selected .code {
    background: var(--add-bg-strong);
  }

  tr.selected.deletion .code {
    background: var(--del-bg-strong);
  }

  tr.selected .ln {
    background: var(--bg-selected);
    color: var(--accent);
  }

  /* Image diff */
  .image-diff {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-3);
    padding: var(--space-3);
  }

  .image-side {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    align-items: center;
  }

  .image-side img {
    max-width: 100%;
    max-height: 420px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: repeating-conic-gradient(var(--bg-inset) 0% 25%, var(--bg-panel) 0% 50%) 0 0 / 16px
      16px;
  }

  .no-image {
    color: var(--text-faint);
    padding: var(--space-5);
  }

  /* Search hits (Ctrl+F): a mark at the line edge, the current hit tinted.
     Deliberately placed AFTER the addition/deletion rules (same specificity). */
  tr.find-hit td.code {
    box-shadow: inset 3px 0 0 var(--modified);
  }

  tr.find-cur td.code {
    background: color-mix(in srgb, var(--modified) 20%, transparent);
  }
</style>
