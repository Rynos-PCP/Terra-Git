import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  avatarColor,
  deriveCloneName,
  formatBytes,
  initials,
  shortenPath,
  timeAgo,
  webUrlOfRemote,
} from "./format";
import { setLang } from "./i18n.svelte";

describe("formatBytes", () => {
  it("scales", () => {
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(1536)).toBe("1.5 KB");
    expect(formatBytes(5 * 1024 * 1024)).toBe("5.0 MB");
  });
  it("rounds cascading across unit boundaries", () => {
    // 1048575 = 1024*1024 - 1: v = 1023.999 KB rounds to 1024 -> move up one
    // unit instead of "1024 KB".
    expect(formatBytes(1048575)).toBe("1.0 MB");
    expect(formatBytes(1048575).startsWith("1024 ")).toBe(false);
  });
});

describe("deriveCloneName", () => {
  it("derives the repo name from various URL forms", () => {
    expect(deriveCloneName("git@github.com:owner/repo.git")).toBe("repo");
    expect(deriveCloneName("https://github.com/owner/repo.git")).toBe("repo");
    expect(deriveCloneName("https://github.com/owner/repo")).toBe("repo");
    expect(deriveCloneName("https://host/owner/repo/")).toBe("repo");
    expect(deriveCloneName("  ssh://git@host:22/o/r.git  ")).toBe("r");
  });
  it("returns an empty string when nothing can be derived", () => {
    expect(deriveCloneName("")).toBe("");
    expect(deriveCloneName("   ")).toBe("");
  });
  it("never carries credentials of an https remote into the web URL", () => {
    expect(webUrlOfRemote("https://user:tok@en@gitlab.local/g/p.git")).toBe(
      "https://gitlab.local/g/p",
    );
    expect(webUrlOfRemote("http://oauth2:abc@host/x")).toBe("http://host/x");
    expect(webUrlOfRemote("git@github.com:o/r.git")).toBe("https://github.com/o/r");
    expect(webUrlOfRemote("https://github.com/o/r.git")).toBe("https://github.com/o/r");
    expect(webUrlOfRemote("/local/path")).toBe(null);
  });
  it("keeps only harmless characters — the URL decides the name", () => {
    expect(deriveCloneName("https://evil.example/x;calc")).toBe("x-calc");
    expect(deriveCloneName("https://h/o/a b&c$(d).git")).toBe("a-b-c-d");
    expect(deriveCloneName("https://h/o/..")).toBe("");
    expect(deriveCloneName("git@github.com:repo.git")).toBe("repo");
    expect(deriveCloneName("https://h/o/Übung_1.2")).toBe("Übung_1.2");
  });
});

describe("timeAgo", () => {
  // A fixed reference time so the thresholds are deterministic.
  const NOW = 1_700_000_000;

  beforeEach(() => {
    setLang("en");
    vi.spyOn(Date, "now").mockReturnValue(NOW * 1000);
  });
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it.each<[number, string]>([
    [NOW, "just now"],
    [NOW - 59, "just now"],
    [NOW - 60, "1 min ago"],
    [NOW - 59 * 60, "59 min ago"],
    [NOW - 60 * 60, "1 h ago"],
    [NOW - 23 * 3600, "23 h ago"],
    [NOW - 24 * 3600, "yesterday"],
    [NOW - 2 * 86400, "2 days ago"],
    [NOW - 29 * 86400, "29 days ago"],
  ])("thresholds minutes/hours/days: %i -> %s", (ts, expected) => {
    expect(timeAgo(ts)).toBe(expected);
  });

  it("ignores timestamps in the future (no negative delta)", () => {
    expect(timeAgo(NOW + 3600)).toBe("just now");
  });

  it("falls back to an absolute date from 30 days on", () => {
    const out = timeAgo(NOW - 30 * 86400);
    expect(out).toMatch(/\d{4}/); // contains a year
  });
});

describe("initials", () => {
  it.each<[string, string]>([
    // 0 name parts -> placeholder
    ["", "?"],
    ["   ", "?"],
    // 1 part -> the first two characters
    ["max", "MA"],
    // 2 parts -> the first + the last initial
    ["John Doe", "JD"],
    // 3+ parts -> the middle parts fall away
    ["Ada Byron King Lovelace", "AL"],
  ])("forms initials: %j -> %s", (name, expected) => {
    expect(initials(name)).toBe(expected);
  });
});

describe("shortenPath", () => {
  it("leaves short paths unchanged", () => {
    expect(shortenPath("C:/a/b/c/d", 60)).toBe("C:/a/b/c/d");
  });

  it("leaves paths with at most 3 segments unchanged (nothing to shorten)", () => {
    const p = "averyverylongsegment/secondverylongpart/third";
    expect(shortenPath(p, 10)).toBe(p);
  });

  it("shortens long paths in the middle and keeps head + file name", () => {
    const p = "C:/Users/dev/projects/terra-git/apps/desktop/src/lib/format.ts";
    const out = shortenPath(p, 40);
    expect(out.startsWith("C:/Users/…/")).toBe(true);
    expect(out.endsWith("/format.ts")).toBe(true);
    expect(out.length).toBeLessThanOrEqual(40);
  });

  it("takes as many segments from the back as fit the budget", () => {
    // The budget is enough for "a/b/…/e/f", but no longer for "…/d/e/f".
    expect(shortenPath("a/b/c/d/e/f", 10)).toBe("a/b/…/e/f");
  });
});

describe("avatarColor", () => {
  const HUES = [165, 208, 265, 38, 335, 100, 185, 18];

  it("is deterministic (same name -> same colour)", () => {
    expect(avatarColor("John Doe")).toBe(avatarColor("John Doe"));
  });

  it("always returns a valid hsl() value from the curated palette", () => {
    for (const name of ["", "a", "Z", "Ährenfeld", "John Doe"]) {
      const m = avatarColor(name).match(/^hsl\((\d+), 42%, 42%\)$/);
      expect(m, name).not.toBeNull();
      expect(HUES, name).toContain(Number(m![1]));
    }
  });
});
