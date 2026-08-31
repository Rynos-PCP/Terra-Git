import { describe, expect, it } from "vitest";
import { parseBisectOutput } from "./bisect";

describe("parseBisectOutput", () => {
  const sha = "abc1234def5678abc1234def5678abc1234def56";

  it("detects the first bad commit (git up to 2.54)", () => {
    const out = `${sha} is the first bad commit\ncommit ...`;
    expect(parseBisectOutput(out).firstBad).toBe(sha);
  });

  // git 2.55 puts the term in quotes ("uses the selected terms more
  // consistently in its output"). Without this the bisect assistant silently
  // stopped reporting a result on a current git.
  it("detects the first bad commit (git 2.55 and later, term quoted)", () => {
    const out = `${sha} is the first 'bad' commit\ncommit ...`;
    expect(parseBisectOutput(out).firstBad).toBe(sha);
  });

  it("also accepts a session with the new/old terms", () => {
    expect(parseBisectOutput(`${sha} is the first 'new' commit`).firstBad).toBe(sha);
  });
  it("reads the remaining steps", () => {
    const out = "Bisecting: 3 revisions left to test after this (roughly 2 steps)\n[sha] subject";
    expect(parseBisectOutput(out).stepsLeft).toBe(2);
  });
  it("neither of the two -> null", () => {
    expect(parseBisectOutput("something")).toEqual({ firstBad: null, stepsLeft: null });
  });
});
