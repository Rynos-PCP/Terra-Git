import { describe, expect, it } from "vitest";
import { findTargetActive } from "./findScope";

describe("findTargetActive()", () => {
  it("lets the main view search while no modal is open", () => {
    expect(findTargetActive(null, null)).toBe(true);
  });

  // The original purpose of the rule: the main diff must not search along while
  // a modal lies in front of it — and that goes for EVERY modal, including one
  // without a search of its own (tags, remotes, clone …).
  it("silences the main view as soon as any modal is open", () => {
    for (const modal of ["stashPreview", "tags", "remotes", "clone", "blame"]) {
      expect(findTargetActive(null, modal), modal).toBe(false);
    }
  });

  // The actual bug: the diff INSIDE the modal was silent too.
  it("lets the diff inside the open modal serve the search", () => {
    expect(findTargetActive("stashPreview", "stashPreview")).toBe(true);
    expect(findTargetActive("blame", "blame")).toBe(true);
  });

  it("keeps an instance from a DIFFERENT modal silent", () => {
    expect(findTargetActive("stashPreview", "blame")).toBe(false);
    expect(findTargetActive("blame", "stashPreview")).toBe(false);
  });

  it("stays silent without content", () => {
    expect(findTargetActive(null, null, false)).toBe(false);
    expect(findTargetActive("stashPreview", "stashPreview", false)).toBe(false);
  });

  // The core assurance: with exactly one open modal at most ONE instance can be
  // active — independent of mount order or rerenders.
  it("activates at most one instance in every state", () => {
    const scopes = [null, "stashPreview", "blame"];
    for (const openModal of [null, "stashPreview", "blame", "tags"]) {
      const active = scopes.filter((s) => findTargetActive(s, openModal));
      expect(active.length, `open: ${openModal}`).toBeLessThanOrEqual(1);
    }
  });
});
