<script lang="ts">
  import { onMount } from "svelte";
  import type { BlameLine } from "../api";
  import { avatarColor, initials, timeAgo } from "../format";
  import { highlightLine, langForFile } from "../highlight";
  import { t } from "../i18n.svelte";
  import { ui } from "../state.svelte";
  import { findTargetActive } from "../findScope";
  import FindBar from "./FindBar.svelte";

  let { file, lines }: { file: string; lines: BlameLine[] } = $props();

  /** Consecutive lines of the same commit as one block. */
  interface Group {
    commitId: string;
    shortId: string;
    author: string;
    time: number;
    lines: BlameLine[];
  }

  const groups = $derived.by<Group[]>(() => {
    const gs: Group[] = [];
    for (const l of lines) {
      const last = gs[gs.length - 1];
      if (last && last.commitId === l.commitId) {
        last.lines.push(l);
      } else {
        gs.push({
          commitId: l.commitId,
          shortId: l.shortId,
          author: l.author,
          time: l.time,
          lines: [l],
        });
      }
    }
    return gs;
  });

  // Age scale: the newest commit gets the full author colour, the oldest fades
  // clearly towards grey (but never invisible — a lower bound of 25%).
  const heatFor = $derived.by(() => {
    const times = groups.map((g) => g.time);
    const min = Math.min(...times);
    const span = Math.max(1, Math.max(...times) - min);
    return (t: number) => 25 + Math.round(75 * ((t - min) / span));
  });

  const lang = $derived(langForFile(file));

  // ---- Search (Ctrl+F) & "go to line" (Ctrl+G) — key = line number ----
  let rootEl = $state<HTMLElement>();
  let findOpen = $state(false);
  let findMode = $state<"text" | "goto">("text");
  let findHits = $state<string[]>([]);
  let findCur = $state(0);

  const findHitSet = $derived(new Set(findHits));
  const findCurKey = $derived(findCur > 0 ? findHits[findCur - 1] : null);

  function closeFind() {
    findOpen = false;
    findHits = [];
    findCur = 0;
  }

  function runFindQuery(q: string) {
    if (findMode === "goto") {
      const n = parseInt(q.trim(), 10);
      if (Number.isFinite(n) && lines.some((l) => l.lineNo === n)) {
        findHits = [String(n)];
        findCur = 1;
        scrollToHit(String(n));
      }
      return;
    }
    const needle = q.trim().toLowerCase();
    const hits = needle
      ? lines.filter((l) => l.content.toLowerCase().includes(needle)).map((l) => String(l.lineNo))
      : [];
    findHits = hits;
    findCur = hits.length > 0 ? 1 : 0;
    if (hits.length > 0) scrollToHit(hits[0]);
  }

  function navFind(dir: 1 | -1) {
    if (findHits.length === 0) return;
    findCur = ((findCur - 1 + dir + findHits.length) % findHits.length) + 1;
    scrollToHit(findHits[findCur - 1]);
  }

  function scrollToHit(key: string) {
    requestAnimationFrame(() => {
      rootEl?.querySelector(`[data-line="${key}"]`)?.scrollIntoView({ block: "center" });
    });
  }

  onMount(() => {
    // The same rule as in DiffView (findScope.ts) so there is only ONE
    // responsibility logic for the global search.
    const canHandle = () => findTargetActive("blame", ui.modal?.kind ?? null);
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
      if (findOpen) navFind(((e as CustomEvent).detail as 1 | -1) ?? 1);
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

<div class="blame" bind:this={rootEl}>
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
  <p class="legend">{t("blame.legend")}</p>

  {#each groups as g (`${g.commitId}:${g.lines[0].lineNo}`)}
    <section class="group" style:--author={avatarColor(g.author)} style:--heat="{heatFor(g.time)}%">
      <header title={g.commitId}>
        <span class="avatar mini">{initials(g.author)}</span>
        <strong class="name">{g.author}</strong>
        <span class="dot">·</span>
        <span class="when">{timeAgo(g.time)}</span>
        <span class="spacer"></span>
        <code class="sha">{g.shortId}</code>
      </header>
      <table>
        <tbody>
          {#each g.lines as line (line.lineNo)}
            <tr
              data-line={String(line.lineNo)}
              class:find-hit={findOpen && findHitSet.has(String(line.lineNo))}
              class:find-cur={findOpen && findCurKey === String(line.lineNo)}
            >
              <td class="ln">{line.lineNo}</td>
              <!-- eslint-disable-next-line svelte/no-at-html-tags — highlightLine always escapes -->
              <td class="src">{@html highlightLine(line.content, lang)}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </section>
  {/each}
</div>

<style>
  .blame {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .legend {
    color: var(--text-faint);
    font-size: 11.5px;
  }

  .group {
    /* Author colour, mixed towards grey with age. */
    --tint: color-mix(in srgb, var(--author) var(--heat), var(--text-faint));
    border-left: 3px solid var(--tint);
    border-radius: var(--radius-sm);
    background: var(--bg-inset);
    overflow: hidden;
  }

  .group header {
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 4px 10px 4px 8px;
    background: color-mix(in srgb, var(--tint) 8%, transparent);
    font-size: 12px;
  }

  .avatar.mini {
    width: 18px;
    height: 18px;
    font-size: 8.5px;
    background: var(--tint);
  }

  .name {
    color: var(--text-primary);
    font-weight: 600;
  }

  .dot {
    color: var(--text-faint);
  }

  .when {
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }

  .spacer {
    flex: 1;
  }

  .sha {
    font-family: var(--mono);
    font-size: 10.5px;
    color: var(--text-muted);
    background: var(--bg-panel);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: 0 5px;
    -webkit-user-select: text;
    user-select: text;
  }

  table {
    width: 100%;
    border-collapse: collapse;
    font-family: var(--mono);
    font-size: 12px;
    line-height: 1.55;
  }

  .ln {
    width: 44px;
    min-width: 44px;
    padding: 0 8px;
    text-align: right;
    color: var(--text-faint);
    border-right: 1px solid var(--border);
    vertical-align: top;
    -webkit-user-select: none;
    user-select: none;
    font-variant-numeric: tabular-nums;
  }

  .src {
    padding: 0 10px;
    white-space: pre-wrap;
    word-break: break-all;
    -webkit-user-select: text;
    user-select: text;
    cursor: text;
    width: 100%;
  }

  .find-holder {
    position: sticky;
    top: 0;
    z-index: 5;
    align-self: flex-end;
  }

  tr.find-hit td.src {
    box-shadow: inset 3px 0 0 var(--modified);
  }

  tr.find-cur td.src {
    background: color-mix(in srgb, var(--modified) 20%, transparent);
  }
</style>
