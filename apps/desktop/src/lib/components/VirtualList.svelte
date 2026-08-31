<script lang="ts" generics="T">
  import type { Snippet } from "svelte";

  // Window-based rendering for long lists (large-repo capability): only the
  // visible rows (+ overscan) are in the DOM, placeholders above/below keep the
  // scroll height. Prerequisite: a fixed row height.
  let {
    items,
    rowHeight,
    getKey,
    row,
    footer,
    overscan = 10,
    onnearend,
    nearEndMargin = 600,
  }: {
    items: T[];
    /** Fixed height of every row in px (window maths). */
    rowHeight: number;
    /** Stable key per item (DOM identity while scrolling). */
    getKey: (item: T) => string;
    /** Row snippet: (item, absolute index in `items`). */
    row: Snippet<[T, number]>;
    /** Optional below the list (e.g. a "load more" button). */
    footer?: Snippet;
    /** Additional rows rendered above/below the viewport. */
    overscan?: number;
    /** Called when scrolling near the end of the list (infinite scroll). */
    onnearend?: () => void;
    /** Distance to the end in px at which `onnearend` fires. */
    nearEndMargin?: number;
  } = $props();

  let viewport = $state<HTMLElement>();
  let scrollTop = $state(0);
  let viewportH = $state(0);

  const start = $derived(Math.max(0, Math.floor(scrollTop / rowHeight) - overscan));
  const end = $derived(
    Math.min(items.length, Math.ceil((scrollTop + viewportH) / rowHeight) + overscan),
  );
  const visible = $derived(items.slice(start, end));

  function handleScroll() {
    if (!viewport) return;
    scrollTop = viewport.scrollTop;
    if (
      onnearend &&
      viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight < nearEndMargin
    ) {
      onnearend();
    }
  }

  /** Scrolls to the start of the list (e.g. after a repo switch). */
  export function scrollToTop() {
    viewport?.scrollTo({ top: 0 });
    scrollTop = 0;
  }

  /** Brings row `index` into the viewport (the caller's keyboard cursor). */
  export function scrollIndexIntoView(index: number) {
    if (!viewport) return;
    const top = index * rowHeight;
    const bottom = top + rowHeight;
    if (top < viewport.scrollTop) viewport.scrollTo({ top });
    else if (bottom > viewport.scrollTop + viewport.clientHeight)
      viewport.scrollTo({ top: bottom - viewport.clientHeight });
    scrollTop = viewport.scrollTop;
  }
</script>

<div class="viewport" bind:this={viewport} bind:clientHeight={viewportH} onscroll={handleScroll}>
  <div style:height="{start * rowHeight}px"></div>
  {#each visible as item, i (getKey(item))}
    <div class="vrow" style:height="{rowHeight}px">
      {@render row(item, start + i)}
    </div>
  {/each}
  <div style:height="{(items.length - end) * rowHeight}px"></div>
  {#if footer}{@render footer()}{/if}
</div>

<style>
  .viewport {
    height: 100%;
    overflow-y: auto;
    min-height: 0;
  }

  .vrow {
    /* Row content must not blow up the slot height (window maths),
       but dropdown menus must be allowed to stick out -> no overflow:hidden. */
    display: flex;
    flex-direction: column;
    justify-content: center;
  }

  .vrow > :global(*) {
    flex-shrink: 0;
  }
</style>
