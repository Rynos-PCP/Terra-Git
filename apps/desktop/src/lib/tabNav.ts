// Keyboard navigation in the app tab strip (WAI-ARIA tabs).
//
// Extracted as a pure module so the decision is testable without a DOM — the
// same pattern as splitter.ts.

export const APP_TABS = ["changes", "history"] as const;
export type AppTab = (typeof APP_TABS)[number];

type KeyLike = Pick<KeyboardEvent, "key" | "altKey" | "ctrlKey" | "metaKey">;

/**
 * Next tab for arrow/Home/End keys (cyclic).
 *
 * `null` means: the key is none of our business — the caller must then NOT
 * intercept the event. Important for modifier combinations: Alt+arrow belongs to
 * the global shortcut layer (which uses it to suppress the WebView's history
 * navigation); if we answered here, the tab would change on the side.
 */
export function nextTab(current: AppTab, e: KeyLike): AppTab | null {
  if (e.altKey || e.ctrlKey || e.metaKey) return null;
  const i = APP_TABS.indexOf(current);
  if (i < 0) return null;
  switch (e.key) {
    case "ArrowRight":
      return APP_TABS[(i + 1) % APP_TABS.length];
    case "ArrowLeft":
      return APP_TABS[(i - 1 + APP_TABS.length) % APP_TABS.length];
    case "Home":
      return APP_TABS[0];
    case "End":
      return APP_TABS[APP_TABS.length - 1];
    default:
      return null;
  }
}
