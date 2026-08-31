// Static a11y guard for the file row.
//
// The row was a div[role="button"][tabindex="0"] and contained real <button>
// elements (stage, discard, menu, conflict actions). Interactive descendants
// inside an element with an interactive role are invalid per HTML/ARIA: screen
// readers announce "button", but inside it there are further buttons that cannot
// be reached in button mode.
//
// A source test like menuA11y.test.ts (no DOM in the project), sources through
// import.meta.glob instead of node:fs — @types/node is not installed and tsconfig
// pins types, so a node:fs import made `npm run check` red.
import { describe, expect, it } from "vitest";

// Normalize line endings: git checks the file out with CRLF depending on
// core.autocrlf — an LF search pattern would then find nothing (that is exactly
// how this test once went falsely red).
const src = Object.values(
  import.meta.glob("./components/FileRow.svelte", {
    query: "?raw",
    import: "default",
    eager: true,
  }) as Record<string, string>,
)[0].replace(/\r\n/g, "\n");

/** The opening tag of the row wrapper (the first <div class="row"). */
const wrapper = (() => {
  const i = src.indexOf('<div\n  class="row"');
  const start = i >= 0 ? i : src.indexOf('<div class="row"');
  expect(start, "row wrapper found").toBeGreaterThanOrEqual(0);
  return src.slice(start, src.indexOf(">", start) + 1);
})();

describe("FileRow accessibility", () => {
  it("does not mark the row with an interactive role", () => {
    // role="button" with buttons inside is the actual finding.
    expect(wrapper).not.toMatch(/role="button"/);
  });

  // Without this assurance the test could be made green by only removing the
  // role and losing the keyboard operation entirely.
  it("no longer keeps a keyboard substitute on the wrapper", () => {
    expect(wrapper).not.toMatch(/onkeydown/);
    expect(wrapper).not.toMatch(/tabindex/);
  });

  it("offers a real control for the selection instead", () => {
    // A real <button> carries focus, Enter/space and the screen-reader
    // announcement out of the box.
    expect(src).toMatch(/<button\s+class="select"/);
  });
});
