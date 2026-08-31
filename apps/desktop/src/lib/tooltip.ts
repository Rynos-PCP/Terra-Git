// Our own tooltip action: use:tooltip={text}. A single singleton overlay,
// delayed fade-in, positioning above/below the element.

export interface Rect {
  left: number;
  right: number;
  top: number;
  bottom: number;
  width: number;
  height: number;
}

/** Pure placement (Vitest-tested): prefers above, otherwise below; x centred + clamped. */
export function tooltipPosition(
  target: Rect,
  tip: { w: number; h: number },
  vp: { w: number; h: number },
  gap = 6,
  pad = 4,
): { x: number; y: number; placement: "top" | "bottom" } {
  const placement = target.top - tip.h - gap >= pad ? "top" : "bottom";
  const y = placement === "top" ? target.top - tip.h - gap : target.bottom + gap;
  const cx = target.left + target.width / 2 - tip.w / 2;
  const x = Math.max(pad, Math.min(cx, vp.w - tip.w - pad));
  return { x, y, placement };
}

/** ARIA fallback (Vitest-tested): icon-only buttons without a name of their own
 *  (aria-hidden icon + no text/aria-label) get the tooltip text as their
 *  aria-label; already named nodes do not. */
export function ariaLabelFor(text: string, hasOwnName: boolean): string | null {
  return hasOwnName ? null : text || null;
}

let el: HTMLDivElement | null = null;
let showTimer: ReturnType<typeof setTimeout> | null = null;
// The node that armed the (still pending) timer, or whose tooltip is currently
// visible. Only the node concerned may clear "its" timer or the overlay again —
// otherwise the destroy() of another tooltipped node (e.g. a FileRow that
// unmounts on every status mutation) would clear the tooltip of a node still
// waiting.
let armedNode: HTMLElement | null = null;
let shownNode: HTMLElement | null = null;

function ensureEl(): HTMLDivElement {
  if (!el) {
    el = document.createElement("div");
    el.className = "tt-overlay";
    el.setAttribute("role", "tooltip");
    document.body.appendChild(el);
  }
  return el;
}

/** Hides the timer/overlay only when the calling node is the one concerned. */
function hideFor(node: HTMLElement) {
  if (showTimer && node === armedNode) {
    clearTimeout(showTimer);
    showTimer = null;
    armedNode = null;
  }
  if (el && node === shownNode) {
    el.style.opacity = "0";
    shownNode = null;
  }
}

function showFor(node: HTMLElement, text: string) {
  const box = ensureEl();
  box.textContent = text;
  box.style.opacity = "0";
  box.style.left = "-9999px";
  // measure -> position
  const r = node.getBoundingClientRect();
  const tip = { w: box.offsetWidth, h: box.offsetHeight };
  const p = tooltipPosition(r, tip, { w: window.innerWidth, h: window.innerHeight });
  box.style.left = `${p.x}px`;
  box.style.top = `${p.y}px`;
  box.dataset.placement = p.placement;
  box.style.opacity = "1";
  shownNode = node;
}

export function tooltip(node: HTMLElement, text: string) {
  let current = text;
  // An accessible name of its own? textContent (the icon renders an aria-hidden
  // svg, which stays empty here) or an already set aria-label. Determined once
  // at init — BEFORE we possibly set a label ourselves.
  const hasOwnName = (node.textContent ?? "").trim() !== "" || node.hasAttribute("aria-label");
  let labelSet = false;
  const initLabel = ariaLabelFor(current, hasOwnName);
  if (initLabel !== null) {
    node.setAttribute("aria-label", initLabel);
    labelSet = true;
  }

  const enter = () => {
    if (!current) return;
    if (showTimer) clearTimeout(showTimer);
    armedNode = node;
    showTimer = setTimeout(() => {
      showTimer = null;
      armedNode = null;
      showFor(node, current);
    }, 450);
  };
  const leave = () => hideFor(node);
  const key = (e: KeyboardEvent) => {
    if (e.key === "Escape") hideFor(node);
  };
  node.addEventListener("mouseenter", enter);
  node.addEventListener("mouseleave", leave);
  node.addEventListener("focusin", enter);
  node.addEventListener("focusout", leave);
  node.addEventListener("keydown", key);
  return {
    update(next: string) {
      current = next;
      // Keep a self-set aria-label in sync (only when WE own it).
      if (labelSet) {
        const l = ariaLabelFor(next, false);
        if (l !== null) node.setAttribute("aria-label", l);
        else node.removeAttribute("aria-label");
      }
      // If this node's tooltip is currently visible, refresh the text right away.
      if (node === shownNode) {
        if (next) showFor(node, next);
        else hideFor(node);
      }
    },
    destroy() {
      node.removeEventListener("mouseenter", enter);
      node.removeEventListener("mouseleave", leave);
      node.removeEventListener("focusin", enter);
      node.removeEventListener("focusout", leave);
      node.removeEventListener("keydown", key);
      // Only remove our own aria-label again.
      if (labelSet) node.removeAttribute("aria-label");
      hideFor(node);
    },
  };
}
