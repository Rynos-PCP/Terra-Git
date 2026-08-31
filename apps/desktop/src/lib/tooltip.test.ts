import { describe, expect, it } from "vitest";
import { ariaLabelFor, tooltipPosition } from "./tooltip";

const vp = { w: 1000, h: 800 };

describe("ariaLabelFor", () => {
  it("returns the text as a fallback when the node has no name of its own", () => {
    expect(ariaLabelFor("Undo", false)).toBe("Undo");
  });
  it("returns null when the node already has a name of its own", () => {
    expect(ariaLabelFor("Undo", true)).toBe(null);
  });
});
describe("tooltipPosition", () => {
  it("places it above when there is room", () => {
    const r = { left: 400, right: 460, top: 300, bottom: 320, width: 60, height: 20 };
    const p = tooltipPosition(r, { w: 100, h: 24 }, vp);
    expect(p.placement).toBe("top");
    expect(p.y).toBeLessThan(r.top);
  });
  it("moves below when there is no room above", () => {
    const r = { left: 400, right: 460, top: 4, bottom: 24, width: 60, height: 20 };
    const p = tooltipPosition(r, { w: 100, h: 24 }, vp);
    expect(p.placement).toBe("bottom");
    expect(p.y).toBeGreaterThan(r.bottom);
  });
  it("clamps x into the viewport", () => {
    const r = { left: 970, right: 1000, top: 300, bottom: 320, width: 30, height: 20 };
    const p = tooltipPosition(r, { w: 120, h: 24 }, vp);
    expect(p.x).toBeGreaterThanOrEqual(4);
    expect(p.x + 120).toBeLessThanOrEqual(vp.w - 4);
  });
});
