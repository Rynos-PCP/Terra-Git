// Static a11y guard for the tab strip.
//
// role="tablist"/role="tab" without a matching role="tabpanel" is half an ARIA
// tree: screen readers announce the tabs but cannot lead the user to the
// matching content, because aria-controls is missing.
//
// A source test for the same reason as menuA11y.test.ts: the project
// deliberately has no DOM test environment. Sources through import.meta.glob,
// not node:fs (no @types/node, tsconfig pins types → `npm run check` would be red).
import { describe, expect, it } from "vitest";

const app = Object.values(
  import.meta.glob("../App.svelte", {
    query: "?raw",
    import: "default",
    eager: true,
  }) as Record<string, string>,
)[0];

describe("app tab strip", () => {
  it("links every tab to a tabpanel via aria-controls", () => {
    // Every role="tab" carries aria-controls …
    const tabs = app.match(/role="tab"/g) ?? [];
    const controls = app.match(/aria-controls="app-tabpanel"/g) ?? [];
    expect(tabs.length, "role=tab occurrences").toBeGreaterThan(0);
    expect(controls.length, "aria-controls per tab").toBe(tabs.length);

    // … and the target really exists as a tabpanel with that id.
    expect(app).toMatch(/id="app-tabpanel"/);
    expect(app).toMatch(/role="tabpanel"/);
    // The panel names itself through the active tab.
    expect(app).toMatch(/aria-labelledby="app-tab-\{ui\.tab\}"/);
  });

  it("uses a roving tabindex so Tab enters and leaves the strip", () => {
    // Exactly the active tab is reachable by Tab (0), the others -1.
    const roving = app.match(/tabindex=\{ui\.tab === "\w+" \? 0 : -1\}/g) ?? [];
    expect(roving.length, "roving tabindex per tab").toBeGreaterThanOrEqual(2);
  });

  it("handles arrow keys on the tablist container", () => {
    expect(app).toMatch(/role="tablist"[\s\S]{0,200}onkeydown=\{onTabsKeydown\}/);
  });
});
