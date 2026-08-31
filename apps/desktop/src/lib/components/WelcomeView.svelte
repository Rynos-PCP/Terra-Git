<script lang="ts">
  import { onMount } from "svelte";
  import { getVersion } from "@tauri-apps/api/app";
  import { api, type RepoPeek } from "../api";
  import { shortenPath, timeAgo } from "../format";
  import { t } from "../i18n.svelte";
  import { browseForRepo, forgetRecent, openRepo, pinRecent, ui } from "../state.svelte";
  import { buildVein } from "../welcomeVein";
  import Icon from "./Icon.svelte";
  import Menu from "./Menu.svelte";

  // ---- Repo short portraits (branch chip, dirty dot, vein sketch) ----
  // Component-local, keyed by path: late answers can never hit the wrong entry,
  // and only the welcome screen carries this load.
  let peeks = $state<Record<string, RepoPeek | null>>({});

  function ensurePeek(path: string) {
    if (path in peeks) return;
    peeks[path] = null;
    api
      .peekRepo(path)
      .then((p) => (peeks[path] = p))
      .catch(() => {
        // Moved/unreadable: no chip, a decorative vein — not an error state.
      });
  }

  $effect(() => {
    for (const r of ui.recents) ensurePeek(r.path);
  });

  // The vein shows the repo under the mouse (or with keyboard focus), otherwise
  // the most recently opened one. Briefly debounced so a sweep across the list
  // does not make the sketch flicker.
  let veinPath = $state<string | null>(null);
  let hoverTimer: ReturnType<typeof setTimeout> | undefined;

  function focusVein(path: string | null) {
    clearTimeout(hoverTimer);
    hoverTimer = setTimeout(() => (veinPath = path), 120);
  }

  const defaultVeinPath = $derived(
    ui.recents.length
      ? ui.recents.reduce((a, b) => ((b.lastOpened ?? 0) > (a.lastOpened ?? 0) ? b : a)).path
      : null,
  );
  /** Repo whose sketch the vein is currently showing. */
  const veinRepo = $derived(veinPath ?? defaultVeinPath ?? "");
  const vein = $derived(buildVein(peeks[veinRepo]?.commits ?? [], peeks[veinRepo]?.branches ?? []));
  /** Keys the build-up animation: redraw on a repo switch AND when the peek data
   *  ARRIVES — the IPC answer comes asynchronously after the first render, and
   *  the path alone does not change in the process (finding 2026-08-14: with the
   *  real backend the animation only ran on the decorative vein). */
  const veinKey = $derived(`${veinRepo}|${peeks[veinRepo] ? "data" : "empty"}`);

  // ---- Build-up animation of the vein: left -> right ----
  // The line draws itself across the panel width, nodes appear as soon as the
  // line reaches them. All timings derive from x/320.
  const DRAW_S = 0.9;
  const BASE_S = 0.35; // after the strata have risen
  const delayAt = (x: number) => BASE_S + (x / 320) * DRAW_S;

  /** Tinted strand colour per palette slot (line softer, nodes richer). */
  const strandStroke = (slot: number) =>
    `color-mix(in srgb, var(--graph-${slot}) 60%, transparent)`;
  const strandFill = (slot: number) => `color-mix(in srgb, var(--graph-${slot}) 85%, transparent)`;

  // The strata SVG is stretched to the panel size with preserveAspectRatio="none"
  // (the strata should fill it) — circles would turn into ovals in the process
  // (user finding 2026-08-14). The nodes are therefore ellipses with OPPOSING
  // radii (rx/ry divide the stretch back out) and all outlines run in screen
  // pixels with vector-effect="non-scaling-stroke".
  let brandW = $state(0);
  let brandH = $state(0);
  const sx = $derived(brandW > 0 ? brandW / 320 : 1);
  const sy = $derived(brandH > 0 ? brandH / 500 : 1);

  /** Deterministic colour slot (1..8, graph palette) per repo path. */
  function graphSlot(path: string): number {
    let h = 0;
    for (let i = 0; i < path.length; i++) h = (h * 31 + path.charCodeAt(i)) | 0;
    return (Math.abs(h) % 8) + 1;
  }

  const repoName = (path: string) =>
    path
      .replace(/[\\/]+$/, "")
      .split(/[\\/]/)
      .pop() ?? path;

  let version = $state("");
  onMount(() => {
    getVersion()
      .then((v) => (version = v))
      .catch(() => {
        // Mock/browser mode without the app plugin: a footer without a version.
      });
    return () => clearTimeout(hoverTimer);
  });
</script>

<div class="welcome">
  <div class="card">
    <!-- Brand panel: a geological outcrop — strata as slate, the
         vein as a "horizontal core sample" (user decision 2026-08-14): the HEAD
         strand (slightly red) lies as a straight core line in the rock, time
         from left (old) to right (new), nodes evenly spaced; branches as
         parallel veins above it with a quarter-arc branch-off, ancestors/tags
         as coloured rings — deliberately WITHOUT text, the colour carries the
         information (user finding 2026-08-14: names overlapped, time spacing
         clumped). Data: peek_repo of the repo under the mouse or of the most
         recently opened one; without data the vein stays decorative. -->
    <aside class="brand" aria-hidden="true" bind:clientWidth={brandW} bind:clientHeight={brandH}>
      <div class="lockup">
        <h1>terra<span class="hyphen">-</span>git</h1>
        <p class="tagline">{t("welcome.tagline1")}<br />{t("welcome.tagline2")}</p>
      </div>
      <svg class="strata" viewBox="0 0 320 500" preserveAspectRatio="none">
        <path
          class="layer l1"
          d="M0 148 C 60 138, 110 162, 170 154 S 280 134, 320 144 V500 H0 Z"
          fill="var(--strata-1)"
        />
        <path
          class="layer l2"
          d="M0 218 C 70 206, 120 230, 190 220 S 285 202, 320 212 V500 H0 Z"
          fill="var(--strata-2)"
        />
        <path
          class="layer l3"
          d="M0 292 C 55 282, 130 304, 195 296 S 290 280, 320 288 V500 H0 Z"
          fill="var(--strata-3)"
        />
        <path
          class="layer l4"
          d="M0 368 C 65 360, 125 380, 200 372 S 288 358, 320 364 V500 H0 Z"
          fill="var(--strata-4)"
        />
        <path
          class="layer l5"
          d="M0 438 C 70 432, 140 448, 210 442 S 292 432, 320 436 V500 H0 Z"
          fill="var(--strata-5)"
        />
        <!-- The vein as a horizontal core sample: a straight core (slightly red), veins
             above it. Draws itself left -> right; the key restarts the
             animation for each repo shown. -->

        {#key veinKey}
          <g class="vein">
            <!-- Dashed continuation on the left: "older history". -->
            <path
              class="tail"
              d={vein.tail}
              fill="none"
              stroke-width="1.2"
              vector-effect="non-scaling-stroke"
              style:animation-delay="{BASE_S + 0.1}s"
            />
            <path
              class="draw main-line"
              d={vein.main}
              pathLength="1"
              fill="none"
              stroke-width="1.6"
              vector-effect="non-scaling-stroke"
              style:animation-delay="{BASE_S}s"
              style:animation-duration="{DRAW_S}s"
            />
            {#each vein.strands as s (s.path)}
              <path
                class="draw"
                d={s.path}
                pathLength="1"
                fill="none"
                stroke={strandStroke(s.slot)}
                stroke-width="1.3"
                vector-effect="non-scaling-stroke"
                style:animation-delay="{delayAt(s.x0)}s"
                style:animation-duration="{Math.max(0.15, (s.len / 320) * DRAW_S)}s"
              />
            {/each}
            {#each vein.dots as d, i (i)}
              <!-- A halo in the stratum colour behind the core separates the node
                   from the rock; the youngest commit pulses slightly. -->
              <ellipse
                class="node halo"
                cx={d.x}
                cy={d.y}
                rx={(d.r + 2) / sx}
                ry={(d.r + 2) / sy}
                style:animation-delay="{delayAt(d.x)}s"
              />
              <ellipse
                class="node main-node"
                cx={d.x}
                cy={d.y}
                rx={d.r / sx}
                ry={d.r / sy}
                style:animation-delay="{delayAt(d.x)}s"
              />
              {#if i === 0}
                <ellipse
                  class="head-ring"
                  cx={d.x}
                  cy={d.y}
                  rx={(d.r + 5) / sx}
                  ry={(d.r + 5) / sy}
                  vector-effect="non-scaling-stroke"
                  style:animation-delay="{delayAt(d.x)}s"
                />
              {/if}
              {#if d.hasTag}
                <!-- Tag marker: ochre ring (--ref-tag), separate from the
                     strand colours. -->
                <ellipse
                  class="node tag-ring"
                  cx={d.x}
                  cy={d.y}
                  rx={(d.r + 3.2) / sx}
                  ry={(d.r + 3.2) / sy}
                  vector-effect="non-scaling-stroke"
                  style:animation-delay="{delayAt(d.x)}s"
                />
              {/if}
            {/each}
            {#each vein.strands as s (s.path)}
              {#each s.dots as d, i (i)}
                <ellipse
                  class="node"
                  cx={d.x}
                  cy={d.y}
                  rx={d.r / sx}
                  ry={d.r / sy}
                  fill={strandFill(s.slot)}
                  style:animation-delay="{delayAt(d.x)}s"
                />
              {/each}
            {/each}
            <!-- Ancestor branches (ahead 0, e.g. main behind the feature
                 branch): a coloured ring at the tip commit instead of a vein. -->
            {#each vein.rings as ring, i (i)}
              <ellipse
                class="node branch-ring"
                cx={ring.x}
                cy={ring.y}
                rx={ring.r / sx}
                ry={ring.r / sy}
                vector-effect="non-scaling-stroke"
                style:stroke={strandFill(ring.slot)}
                style:animation-delay="{delayAt(ring.x)}s"
              />
            {/each}
          </g>
        {/key}
      </svg>
    </aside>

    <section class="content">
      <div class="actions">
        <h2 class="section-title">{t("welcome.getStarted")}</h2>
        <button class="primary big" onclick={browseForRepo}>
          <Icon name="folder" />
          {t("welcome.openRepo")}
          <span class="kbd-hint" aria-hidden="true">{t("app.keyCtrl")}+O</span>
        </button>
        <div class="row">
          <button class="big" onclick={() => (ui.modal = { kind: "clone" })}>
            <Icon name="external" size={14} />
            {t("welcome.clone")}
          </button>
          <button class="big" onclick={() => (ui.modal = { kind: "init" })}>
            <Icon name="plus" size={14} />
            {t("toolbar.init")}
          </button>
        </div>
      </div>

      <div class="recents">
        <h2 class="section-title">{t("toolbar.recents")}</h2>
        {#if ui.recents.length > 0}
          <div class="recent-list">
            {#each ui.recents as entry (entry.path)}
              <div
                class="recent"
                role="button"
                tabindex="0"
                onclick={(e) => {
                  // Clicks on the action menu (kebab/entries) must NOT open the
                  // repo — the menu lives in a .menu-root inside the container.
                  if (!(e.target as HTMLElement).closest(".menu-root")) openRepo(entry.path);
                }}
                onkeydown={(e) => {
                  // Only react when the key lands directly on the container —
                  // Enter/Space on the menu would otherwise bubble up.
                  if (e.target === e.currentTarget && (e.key === "Enter" || e.key === " ")) {
                    e.preventDefault();
                    openRepo(entry.path);
                  }
                }}
                onmouseenter={() => focusVein(entry.path)}
                onmouseleave={() => focusVein(null)}
                onfocusin={() => focusVein(entry.path)}
                onfocusout={() => focusVein(null)}
                title={entry.path}
              >
                <span class="avatar av-{graphSlot(entry.path)}">
                  {repoName(entry.path).slice(0, 1)}
                </span>
                <span class="col">
                  <span class="toprow">
                    <span class="name">{repoName(entry.path)}</span>
                    {#if peeks[entry.path]?.branch}
                      <span class="chip">
                        <Icon name="branch" size={10} strokeWidth={2} />
                        <span class="chip-text">{peeks[entry.path]?.branch}</span>
                      </span>
                    {/if}
                    {#if peeks[entry.path]?.dirty}
                      <span
                        class="dot"
                        role="img"
                        aria-label={t("welcome.dirty")}
                        title={t("welcome.dirty")}
                      ></span>
                    {/if}
                  </span>
                  <span class="path">{shortenPath(entry.path.replace(/\\/g, "/"), 44)}</span>
                </span>
                <span class="meta">
                  {#if entry.pinned}
                    <span
                      class="pinned"
                      role="img"
                      aria-label={t("welcome.pinned")}
                      title={t("welcome.pinned")}
                    >
                      <Icon name="pin" size={12} />
                    </span>
                  {/if}
                  {#if entry.lastOpened}
                    <span class="time">{timeAgo(entry.lastOpened)}</span>
                  {/if}
                  <!-- A visible action menu instead of hover-hidden icons:
                       the harmless (pin, remove from list) and the dangerous
                       (recycle bin) action are clearly labelled and separated. -->
                  <Menu align="right" width="220px">
                    {#snippet trigger({ toggle })}
                      <button
                        class="ghost kebab"
                        title={t("welcome.repoActions")}
                        aria-label={t("welcome.repoActions")}
                        onclick={toggle}
                      >
                        <Icon name="more" size={16} />
                      </button>
                    {/snippet}
                    <button
                      class="item"
                      role="menuitem"
                      onclick={() => pinRecent(entry.path, !entry.pinned)}
                    >
                      <Icon name="pin" size={14} />
                      {entry.pinned ? t("welcome.unpin") : t("welcome.pin")}
                    </button>
                    <button class="item" role="menuitem" onclick={() => forgetRecent(entry.path)}>
                      <Icon name="x" size={14} />
                      {t("welcome.forgetRecent")}
                    </button>
                    <button
                      class="item danger"
                      role="menuitem"
                      onclick={() => (ui.modal = { kind: "deleteRepo", path: entry.path })}
                    >
                      <Icon name="trash" size={14} />
                      {t("welcome.deleteRepo")}
                    </button>
                  </Menu>
                </span>
              </div>
            {/each}
          </div>
        {:else}
          <p class="hint">{t("welcome.recentsHint")}</p>
        {/if}
      </div>
    </section>

    <!-- A meta level instead of leftover space: keyboard paths, drop hint, version. -->
    <div class="foot">
      <span class="keys">
        <span class="key">
          <kbd>{t("app.keyCtrl")}</kbd><kbd>O</kbd>
          {t("welcome.kbdOpen")}
        </span>
        <span class="key">
          <kbd>{t("app.keyCtrl")}</kbd><kbd>K</kbd>
          {t("welcome.kbdPalette")}
        </span>
        <span class="drop">{t("welcome.dropHint")}</span>
      </span>
      {#if version}
        <span>v{version}</span>
      {/if}
    </div>
  </div>
</div>

<style>
  .welcome {
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    overflow-y: auto;
    padding: var(--space-5);
  }

  .card {
    display: grid;
    grid-template-columns: 300px 1fr;
    grid-template-rows: 1fr auto;
    width: min(860px, 100%);
    min-height: 500px;
    max-height: min(640px, 100%);
    background: var(--bg-panel);
    border: 1px solid var(--border);
    border-radius: var(--radius-xl);
    box-shadow: var(--shadow-menu);
    overflow: hidden;
    animation: card-in 0.3s ease-out;
  }

  @keyframes card-in {
    from {
      opacity: 0;
      transform: translateY(8px);
    }
  }

  /* ---------- Brand panel ---------- */

  .brand {
    grid-row: 1 / 3;
    position: relative;
    background: var(--strata-0);
    border-right: 1px solid var(--border);
    overflow: hidden;
  }

  .lockup {
    position: relative;
    z-index: 1;
    padding: var(--space-6) var(--space-5);
  }

  h1 {
    font-family: var(--display);
    font-size: 30px;
    font-weight: 650;
    letter-spacing: -0.02em;
    line-height: 1.1;
  }

  .hyphen {
    color: var(--accent);
  }

  .tagline {
    margin-top: var(--space-3);
    color: var(--text-muted);
    font-size: 13px;
    line-height: 1.6;
  }

  .strata {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
  }

  .layer {
    animation: rise 0.55s cubic-bezier(0.2, 0.8, 0.3, 1) backwards;
  }

  .l1 {
    animation-delay: 0.05s;
  }
  .l2 {
    animation-delay: 0.12s;
  }
  .l3 {
    animation-delay: 0.19s;
  }
  .l4 {
    animation-delay: 0.26s;
  }
  .l5 {
    animation-delay: 0.33s;
  }

  @keyframes rise {
    from {
      opacity: 0;
      transform: translateY(14px);
    }
  }

  /* Build-up left -> right: paths draw themselves (pathLength=1 normalizes
     the dash length), nodes fade in as soon as the line reaches them.
     Duration/delay come inline from the geometry (x/320). */
  .vein .draw {
    stroke-dasharray: 1;
    stroke-dashoffset: 1;
    animation: vein-draw linear both;
  }

  @keyframes vein-draw {
    to {
      stroke-dashoffset: 0;
    }
  }

  /* HEAD strand slightly tinted red (product decision 6.8.4 item 1). */
  .vein .main-line {
    stroke: color-mix(in srgb, var(--deleted) 40%, var(--strata-vein));
  }

  .vein .node {
    opacity: 0;
    animation: vein-node 0.3s ease-out both;
  }

  @keyframes vein-node {
    to {
      opacity: 1;
    }
  }

  .vein .main-node {
    fill: color-mix(in srgb, var(--deleted) 34%, var(--accent));
  }

  /* Halo: the stratum the core lies in (MAIN_Y is always in stratum 3) —
     separates the node bead from the rock. */
  .vein .halo {
    fill: var(--strata-3);
  }

  /* Dashed continuation of the core at the left edge. */
  .vein .tail {
    stroke: var(--text-faint);
    stroke-opacity: 0.3;
    stroke-dasharray: 2 5;
    opacity: 0;
    animation: vein-node 0.4s ease-out both;
  }

  /* The youngest commit pulses gently — the sketch's sign of life. */
  .vein .head-ring {
    fill: none;
    stroke: var(--accent);
    stroke-width: 1;
    opacity: 0;
    animation:
      vein-node 0.3s ease-out both,
      vein-pulse 2.6s ease-in-out 1.8s infinite;
  }

  @keyframes vein-pulse {
    0%,
    100% {
      opacity: 0.16;
    }
    50% {
      opacity: 0.5;
    }
  }

  /* Tag marker: ochre ring — the same semantic colour as the tag chips. */
  .vein .tag-ring {
    fill: none;
    stroke: var(--ref-tag);
    stroke-width: 1.2;
  }

  /* Ancestor branch: a ring in the strand colour (stroke comes inline). */
  .vein .branch-ring {
    fill: none;
    stroke-width: 1.4;
  }

  @media (prefers-reduced-motion: reduce) {
    .vein .draw {
      animation: none;
      stroke-dashoffset: 0;
    }

    .vein .node,
    .vein .tail {
      animation: none;
      opacity: 1;
    }

    .vein .head-ring {
      animation: none;
      opacity: 0.35;
    }
  }

  /* ---------- Content ---------- */

  .content {
    grid-column: 2;
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
    padding: var(--space-6) var(--space-6) var(--space-4);
    min-height: 0;
  }

  .actions {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .actions .section-title {
    margin-bottom: var(--space-1);
  }

  .big {
    padding: 9px 16px;
    font-size: 13.5px;
    justify-content: center;
  }

  /* Shortcut chip in the primary button: teaches the fastest path right at the
     action; purely visual (aria-hidden), the shortcut itself hangs globally. */
  .primary {
    position: relative;
  }

  .kbd-hint {
    position: absolute;
    right: 10px;
    top: 50%;
    transform: translateY(-50%);
    font-size: 10.5px;
    font-weight: 600;
    padding: 1px 6px;
    border-radius: var(--radius-sm);
    background: color-mix(in srgb, var(--accent-text) 16%, transparent);
    color: color-mix(in srgb, var(--accent-text) 78%, transparent);
    border: 1px solid color-mix(in srgb, var(--accent-text) 20%, transparent);
  }

  .row {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-2);
  }

  .recents {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-height: 0;
    flex: 1;
  }

  .recent-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    overflow-y: auto;
    margin: 0 calc(-1 * var(--space-2));
  }

  /* The row is a div (it contains buttons of its own — nested
     <button> is invalid), so it carries its click affordance itself:
     the global button rules from app.css do not apply here. */
  .recent {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: 7px var(--space-2);
    border-radius: var(--radius-lg);
    text-align: left;
    color: var(--text-muted);
    flex-shrink: 0;
    cursor: pointer;
    transition: background 0.12s ease;
  }

  .recent:hover {
    background: var(--bg-hover);
  }

  .recent:active {
    background: var(--bg-selected);
  }

  .recent:focus-visible {
    outline: 2px solid var(--focus-ring);
    outline-offset: 1px;
  }

  /* Repo avatar: a deterministic graph colour per path — gives the list
     anchors without inventing new colours. */
  .avatar {
    width: 28px;
    height: 28px;
    border-radius: 7px;
    flex: none;
    display: flex;
    align-items: center;
    justify-content: center;
    font-family: var(--display);
    font-weight: 650;
    font-size: 13px;
  }

  .av-1 {
    background: color-mix(in srgb, var(--graph-1) 16%, var(--bg-elevated));
    color: var(--graph-1);
  }
  .av-2 {
    background: color-mix(in srgb, var(--graph-2) 16%, var(--bg-elevated));
    color: var(--graph-2);
  }
  .av-3 {
    background: color-mix(in srgb, var(--graph-3) 16%, var(--bg-elevated));
    color: var(--graph-3);
  }
  .av-4 {
    background: color-mix(in srgb, var(--graph-4) 16%, var(--bg-elevated));
    color: var(--graph-4);
  }
  .av-5 {
    background: color-mix(in srgb, var(--graph-5) 16%, var(--bg-elevated));
    color: var(--graph-5);
  }
  .av-6 {
    background: color-mix(in srgb, var(--graph-6) 16%, var(--bg-elevated));
    color: var(--graph-6);
  }
  .av-7 {
    background: color-mix(in srgb, var(--graph-7) 16%, var(--bg-elevated));
    color: var(--graph-7);
  }
  .av-8 {
    background: color-mix(in srgb, var(--graph-8) 16%, var(--bg-elevated));
    color: var(--graph-8);
  }

  .col {
    display: flex;
    flex-direction: column;
    gap: 1px;
    overflow: hidden;
    flex: 1;
  }

  .toprow {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }

  .recent .name {
    color: var(--text-primary);
    font-weight: 600;
    white-space: nowrap;
  }

  /* Branch chip: the repo identity at a glance. */
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    min-width: 0;
    font-size: 11px;
    color: var(--text-muted);
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: 999px;
    padding: 0 8px 0 6px;
    height: 18px;
    line-height: 1;
  }

  .chip-text {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 140px;
  }

  /* Amber dot = uncommitted changes (diff semantics "modified"). */
  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--modified);
    flex: none;
  }

  .recent .path {
    color: var(--text-faint);
    font-size: 12px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .meta {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex: none;
    color: var(--text-faint);
    font-size: 11.5px;
  }

  .pinned {
    display: flex;
    color: var(--accent);
  }

  .time {
    white-space: nowrap;
  }

  /* Always visible kebab trigger (subtle until hover) instead of hover-hidden icons. */
  .kebab {
    padding: 4px;
    color: var(--text-faint);
  }

  .kebab:hover {
    color: var(--text-primary);
  }

  /* Menu entries: labelled, left-aligned; the dangerous action in danger style. */
  .item {
    width: 100%;
    justify-content: flex-start;
    text-align: left;
    gap: 8px;
    padding: 6px 10px;
    color: var(--text-primary);
  }

  .item.danger {
    color: var(--deleted);
  }

  .hint {
    color: var(--text-faint);
    font-size: 12px;
  }

  /* ---------- Footer ---------- */

  .foot {
    grid-column: 2;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-4);
    padding: 10px var(--space-6);
    border-top: 1px solid var(--border);
    color: var(--text-faint);
    font-size: 11.5px;
  }

  .keys {
    display: flex;
    align-items: center;
    gap: 14px;
    min-width: 0;
  }

  .key {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    white-space: nowrap;
  }

  .drop {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  kbd {
    font-family: var(--sans);
    font-size: 10.5px;
    color: var(--text-muted);
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-bottom-color: var(--border-strong);
    border-radius: var(--radius-sm);
    padding: 0 5px;
    height: 17px;
    display: inline-flex;
    align-items: center;
  }

  /* Narrow windows: the brand panel gives way, the content stays usable. */
  @media (max-width: 720px) {
    .card {
      grid-template-columns: 1fr;
    }

    .brand {
      display: none;
    }

    .content,
    .foot {
      grid-column: 1;
    }

    .drop {
      display: none;
    }
  }
</style>
