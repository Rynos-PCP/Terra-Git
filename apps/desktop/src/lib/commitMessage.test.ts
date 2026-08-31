import { describe, expect, it } from "vitest";
import { buildCommitMessage, parseCommitMessage } from "./commitMessage";

describe("buildCommitMessage", () => {
  it("subject only", () => {
    expect(buildCommitMessage("fix: x", "", "")).toBe("fix: x");
  });

  it("subject + description separated by a blank line", () => {
    expect(buildCommitMessage("feat: y", "Details.", "")).toBe("feat: y\n\nDetails.");
  });

  it("co-authors as a trailer, only entries containing @", () => {
    const msg = buildCommitMessage("a", "", "Max <max@example.com>, broken, Eva <eva@example.com>");
    expect(msg).toBe(
      "a\n\nCo-authored-by: Max <max@example.com>\nCo-authored-by: Eva <eva@example.com>",
    );
  });

  it("trims subject and description", () => {
    expect(buildCommitMessage("  a  ", "  b  ", "")).toBe("a\n\nb");
  });
});

describe("parseCommitMessage", () => {
  it("subject only", () => {
    expect(parseCommitMessage("fix: x")).toEqual({
      summary: "fix: x",
      description: "",
      coAuthors: "",
    });
  });

  it("subject + multi-line description", () => {
    const parsed = parseCommitMessage("feat: y\n\nLine 1\n\nLine 3");
    expect(parsed.summary).toBe("feat: y");
    expect(parsed.description).toBe("Line 1\n\nLine 3");
    expect(parsed.coAuthors).toBe("");
  });

  it("lifts the Co-authored-by trailer into the co-authors field", () => {
    const parsed = parseCommitMessage(
      "a\n\nBody.\nCo-authored-by: Max <max@example.com>\nCo-authored-by: Eva <eva@example.com>",
    );
    expect(parsed.description).toBe("Body.");
    expect(parsed.coAuthors).toBe("Max <max@example.com>, Eva <eva@example.com>");
  });

  it("roundtrip: build → parse returns the fields", () => {
    const msg = buildCommitMessage("feat: z", "Details\nmore.", "Max <max@example.com>");
    expect(parseCommitMessage(msg)).toEqual({
      summary: "feat: z",
      description: "Details\nmore.",
      coAuthors: "Max <max@example.com>",
    });
  });
});
