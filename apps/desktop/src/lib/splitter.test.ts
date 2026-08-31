import { describe, expect, it } from "vitest";
import { clampWidth } from "./splitter";

describe("clampWidth", () => {
  it("clamps into [min,max]", () => {
    expect(clampWidth(100, 260, 560)).toBe(260);
    expect(clampWidth(999, 260, 560)).toBe(560);
    expect(clampWidth(360, 260, 560)).toBe(360);
  });
  it("rounds to whole pixels", () => {
    expect(clampWidth(360.6, 260, 560)).toBe(361);
  });
});
