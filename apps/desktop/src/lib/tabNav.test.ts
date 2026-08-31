import { describe, expect, it } from "vitest";
import { APP_TABS, nextTab } from "./tabNav";

/** Minimal KeyboardEvent double: nextTab only reads these four fields. */
const key = (k: string, mods: Partial<Record<"altKey" | "ctrlKey" | "metaKey", boolean>> = {}) => ({
  key: k,
  altKey: false,
  ctrlKey: false,
  metaKey: false,
  ...mods,
});

describe("nextTab()", () => {
  it("pages cyclically right and left", () => {
    expect(nextTab("changes", key("ArrowRight"))).toBe("history");
    expect(nextTab("history", key("ArrowRight"))).toBe("changes");
    expect(nextTab("history", key("ArrowLeft"))).toBe("changes");
    expect(nextTab("changes", key("ArrowLeft"))).toBe("history");
  });

  it("jumps to the start or the end with Home/End", () => {
    expect(nextTab("history", key("Home"))).toBe(APP_TABS[0]);
    expect(nextTab("changes", key("End"))).toBe(APP_TABS[APP_TABS.length - 1]);
  });

  it("is not responsible for other keys (null instead of a fallback)", () => {
    for (const k of ["ArrowUp", "ArrowDown", "Enter", " ", "a", "Tab", "Escape"]) {
      expect(nextTab("changes", key(k)), k).toBeNull();
    }
  });

  // Alt+arrow belongs to the global shortcut layer (App.svelte uses it to
  // suppress the WebView's history navigation). If nextTab answered here, the
  // tab would change on the side.
  it("leaves modifier combinations to the global shortcut layer", () => {
    expect(nextTab("changes", key("ArrowRight", { altKey: true }))).toBeNull();
    expect(nextTab("changes", key("ArrowRight", { ctrlKey: true }))).toBeNull();
    expect(nextTab("changes", key("ArrowRight", { metaKey: true }))).toBeNull();
  });
});
