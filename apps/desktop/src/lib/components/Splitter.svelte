<script lang="ts">
  import { clampWidth } from "../splitter";
  import { t } from "../i18n.svelte";
  let {
    value,
    min,
    max,
    onresize,
    ondone,
  }: {
    value: number;
    min: number;
    max: number;
    onresize: (w: number) => void;
    ondone?: () => void;
  } = $props();

  let dragging = $state(false);
  let startX = 0;
  let startW = 0;

  function down(e: PointerEvent) {
    dragging = true;
    startX = e.clientX;
    startW = value;
    (e.target as HTMLElement).setPointerCapture(e.pointerId);
  }
  function move(e: PointerEvent) {
    if (!dragging) return;
    onresize(clampWidth(startW + (e.clientX - startX), min, max));
  }
  function up() {
    if (!dragging) return;
    dragging = false;
    ondone?.();
  }
</script>

<!--
  Deliberately a focusable separator adjustable with the arrow keys (the window
  splitter pattern): role="separator" + tabindex + aria-value* makes it an
  interactive widget per WAI-ARIA. The svelte a11y check does not know this
  special case and classifies "separator" as non-interactive across the board —
  hence suppressed only for this one element.
-->
<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div
  class="splitter"
  class:dragging
  role="separator"
  aria-orientation="vertical"
  aria-label={t("changes.resizePanel")}
  aria-valuenow={value}
  aria-valuemin={min}
  aria-valuemax={max}
  tabindex="0"
  onpointerdown={down}
  onpointermove={move}
  onpointerup={up}
  ondblclick={() => {
    onresize(clampWidth(360, min, max));
    ondone?.();
  }}
  onkeydown={(e) => {
    if (e.key === "ArrowLeft") {
      onresize(clampWidth(value - 16, min, max));
      ondone?.();
    }
    if (e.key === "ArrowRight") {
      onresize(clampWidth(value + 16, min, max));
      ondone?.();
    }
  }}
></div>

<style>
  .splitter {
    width: 6px;
    margin: 0 -3px;
    cursor: col-resize;
    background: transparent;
    z-index: 2;
    flex: none;
  }
  .splitter:hover,
  .splitter.dragging,
  .splitter:focus-visible {
    background: var(--accent);
    opacity: 0.35;
  }
</style>
