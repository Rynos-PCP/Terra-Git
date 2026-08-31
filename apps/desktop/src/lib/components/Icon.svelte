<script lang="ts" module>
  // Central SVG icon set (stroke-based, 16px, currentColor).
  // Paths based on Lucide (MIT).
  export const ICONS: Record<string, string> = {
    // The lower node used to sit at y 13..17 and was cut off by the 16 viewBox
    // (user finding 2026-08-15). Now everything lies within 2..14.
    branch:
      "M4 2v8m0 0a2 2 0 1 0 0 4 2 2 0 0 0 0-4zM12 2a2 2 0 1 0 0 4 2 2 0 0 0 0-4zm0 4a6 6 0 0 1-6 6",
    commit: "M8 5.5a2.5 2.5 0 1 0 0 5 2.5 2.5 0 0 0 0-5zM1.5 8h4m5 0h4",
    merge:
      "M5 3a2 2 0 1 0 0 4 2 2 0 0 0 0-4zm0 4v6m0 0a2 2 0 1 0 .01 0M5 9a8 8 0 0 0 6 3m0 0a2 2 0 1 0 4 0 2 2 0 0 0-4 0z",
    tag: "M2 2h5.6c.5 0 1 .2 1.4.6l5 5a2 2 0 0 1 0 2.8l-3.6 3.6a2 2 0 0 1-2.8 0l-5-5A2 2 0 0 1 2 7.6V2zm3.5 3.5h0",
    stash: "M2 5.5 8 2l6 3.5M2 5.5V11l6 3.5m-6-9L8 9m6-3.5V11l-6 3.5M14 5.5 8 9m0 5.5V9",
    search: "M7 2.5a4.5 4.5 0 1 0 0 9 4.5 4.5 0 0 0 0-9zm3.5 8L14 14",
    settings:
      "M8 5.5a2.5 2.5 0 1 0 0 5 2.5 2.5 0 0 0 0-5zM8 1.5v2m0 9v2M2.4 4.8l1.7 1M11.9 10.2l1.7 1m0-6.4-1.7 1M4.1 10.2l-1.7 1",
    folder:
      "M2 4a1.5 1.5 0 0 1 1.5-1.5H6l2 2h4.5A1.5 1.5 0 0 1 14 6v5.5A1.5 1.5 0 0 1 12.5 13h-9A1.5 1.5 0 0 1 2 11.5V4z",
    terminal: "M2.5 3.5 6 7l-3.5 3.5M8 12h5.5",
    external: "M6.5 3H3v10h10V9.5M9.5 2.5H13.5V6.5M13 3 7.5 8.5",
    plus: "M8 3v10M3 8h10",
    trash: "M2.5 4.5h11M6.5 4V2.5h3V4m-6 .5.5 9h8l.5-9M6.5 7v4m3-4v4",
    edit: "M9.5 3 13 6.5 6 13.5H2.5V10L9.5 3zM8 4.5 11.5 8",
    undo: "M3 6h7a3.5 3.5 0 0 1 0 7H6M3 6l2.5-2.5M3 6l2.5 2.5",
    redo: "M13 6H6a3.5 3.5 0 0 0 0 7h4M13 6l-2.5-2.5M13 6l-2.5 2.5",
    check: "M2.5 8.5 6 12l7.5-8",
    x: "M3.5 3.5l9 9m0-9-9 9",
    chevronDown: "M3.5 6 8 10.5 12.5 6",
    file: "M4 1.5h5L13 5.5v9a.5.5 0 0 1-.5.5h-8.5a.5.5 0 0 1-.5-.5v-13a.5.5 0 0 1 .5-.5zM9 1.5v4h4",
    split: "M2.5 2.5h11v11h-11v-11zM8 2.5v11",
    tree: "M3 3h3m-3 5h3m-3 5h3M8.5 3H14M8.5 8H14M8.5 13H14",
    more: "M3.5 8h.01M8 8h.01m4.49 0h.01",
    arrowUp: "M8 13V3m0 0L4 7m4-4 4 4",
    arrowDown: "M8 3v10m0 0 4-4m-4 4-4-4",
    refresh: "M13.5 8a5.5 5.5 0 1 1-1.6-3.9M13.5 2.5v3h-3",
    copy: "M6 6h7v7H6V6zM3 10V3h7",
    eye: "M1.5 8s2.5-4.5 6.5-4.5S14.5 8 14.5 8 12 12.5 8 12.5 1.5 8 1.5 8zm6.5-1.5a1.5 1.5 0 1 0 0 3 1.5 1.5 0 0 0 0-3z",
    window:
      "M2 3.5A1.5 1.5 0 0 1 3.5 2h9A1.5 1.5 0 0 1 14 3.5v9a1.5 1.5 0 0 1-1.5 1.5h-9A1.5 1.5 0 0 1 2 12.5v-9zM2 6h12",
    image: "M2.5 2.5h11v11h-11v-11zm2.5 8 2.5-3 2 2.5 1.5-1.5 2.5 2M6 6.5h.01",
    pr: "M4.5 3a1.5 1.5 0 1 0 0 3m0-3a1.5 1.5 0 1 1 0 3m0-3v7m0 0a1.5 1.5 0 1 0 0 3 1.5 1.5 0 0 0 0-3zM10.5 5H12a1 1 0 0 1 1 1v4m0 0a1.5 1.5 0 1 0 .01 0M10.5 5 12 3.5M10.5 5 12 6.5",
    sun: "M8 5a3 3 0 1 0 0 6 3 3 0 0 0 0-6zm0-3.5v1.5m0 10v1.5M2.5 8H1m14 0h-1.5M3.8 3.8l1 1m6.4 6.4 1 1m0-8.4-1 1m-6.4 6.4-1 1",
    moon: "M13.5 9.5A5.5 5.5 0 0 1 6.5 2.5a5.5 5.5 0 1 0 7 7z",
    history: "M8 4.5V8l2.5 1.5M8 1.5A6.5 6.5 0 1 1 1.5 8M1.5 8l2-2m-2 2-2-2",
    alert: "M8 2.5 14.5 13.5H1.5L8 2.5zM8 7v3.2m0 1.8h.01",
    info: "M8 1.5a6.5 6.5 0 1 0 0 13 6.5 6.5 0 0 0 0-13zM8 7.2v3.6M8 5.2h.01",
    globe:
      "M8 1.5a6.5 6.5 0 1 0 0 13 6.5 6.5 0 0 0 0-13zM1.5 8h13M8 1.5c1.8 1.6 2.7 3.9 2.7 6.5S9.8 12.9 8 14.5C6.2 12.9 5.3 10.6 5.3 8S6.2 3.1 8 1.5z",
    lock: "M5 7V4.75A2.75 2.75 0 0 1 8 2a2.75 2.75 0 0 1 3 2.75V7M3.5 7h9v6.5h-9V7zM8 9.5v1.5",
    pin: "M8 11.3v3.2M6 7.2c0 .5-.3 1-.7 1.2l-1.2.6c-.5.2-.8.7-.8 1.2v.5c0 .4.3.7.7.7h8c.4 0 .7-.3.7-.7v-.5c0-.5-.3-1-.8-1.2l-1.2-.6c-.4-.2-.7-.7-.7-1.2V4h.7a1.35 1.35 0 0 0 0-2.7H5.3a1.35 1.35 0 0 0 0 2.7H6v3.2z",
    shield: "M8 1.5 13.5 3.5V7.8c0 3.3-2.3 5.5-5.5 6.7-3.2-1.2-5.5-3.4-5.5-6.7V3.5L8 1.5z",
  };
</script>

<script lang="ts">
  let {
    name,
    size = 16,
    strokeWidth = 1.5,
  }: { name: string; size?: number; strokeWidth?: number } = $props();
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 16 16"
  fill="none"
  stroke="currentColor"
  stroke-width={strokeWidth}
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
>
  <path d={ICONS[name] ?? ""} />
</svg>

<style>
  svg {
    flex-shrink: 0;
    display: block;
  }
</style>
