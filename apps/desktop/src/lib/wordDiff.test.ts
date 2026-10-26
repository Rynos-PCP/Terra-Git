// Intra-line diff.
import { describe, expect, it } from "vitest";
import {
  markHtml,
  pairedWordRanges,
  pairLines,
  pieces,
  sidePieces,
  wordBudget,
  wordDiff,
} from "./wordDiff";

describe("wordDiff()", () => {
  it("marks only the changed word on each side", () => {
    const d = wordDiff("const limit = 500;", "const limit = 750;");
    expect(d).toEqual({ old: [[14, 17]], new: [[14, 17]] });
  });

  it("marks an insertion on the new side only", () => {
    const d = wordDiff("call(a, b)", "call(a, b, c)");
    expect(d?.old).toEqual([]);
    expect("call(a, b, c)".slice(...d!.new[0])).toBe(", c");
  });

  it("merges neighbouring changed tokens into one range", () => {
    // a, +, b are three changed tokens in a row — one range, not three.
    const d = wordDiff("let total = a+b;", "let total = c*d;");
    expect(d?.old).toEqual([[12, 15]]);
    // An unchanged token between two changes keeps them apart.
    expect(wordDiff("let x = foo.bar();", "let x = baz.qux();")?.old).toEqual([
      [8, 11],
      [12, 15],
    ]);
  });

  it("leaves equal and completely rewritten lines alone", () => {
    expect(wordDiff("same", "same")).toBeNull();
    expect(wordDiff("alpha beta gamma", "one two three four")).toBeNull();
    expect(wordDiff("", "x")).toBeNull();
  });

  it("does not compare lines too long for the table", () => {
    const long = "a ".repeat(400);
    expect(wordDiff(long, long + "b")).toBeNull();
  });

  it("treats letters with umlauts as word characters", () => {
    const d = wordDiff("Größe: 5", "Größe: 6");
    expect(d?.old).toEqual([[7, 8]]);
  });
});

describe("pieces()", () => {
  it("cuts a line into plain and changed parts", () => {
    expect(pieces("abcdef", [[2, 4]])).toEqual([
      { text: "ab", changed: false },
      { text: "cd", changed: true },
      { text: "ef", changed: false },
    ]);
  });
});

describe("markHtml()", () => {
  it("marks plain escaped text, counting an entity as one character", () => {
    expect(markHtml("a &lt; b", [[2, 3]])).toBe('a <mark class="wd">&lt;</mark> b');
  });

  it("keeps the nesting valid across highlighter spans", () => {
    const html = '<span class="hljs-keyword">let</span> x = <span class="hljs-number">500</span>;';
    const out = markHtml(html, [[8, 11]]);
    expect(out).toBe(
      '<span class="hljs-keyword">let</span> x = <span class="hljs-number"><mark class="wd">500</mark></span>;',
    );
  });

  it("closes a mark before a tag and reopens it after", () => {
    const out = markHtml("ab<i>cd</i>ef", [[1, 5]]);
    expect(out).toBe(
      'a<mark class="wd">b</mark><i><mark class="wd">cd</mark></i><mark class="wd">e</mark>f',
    );
  });

  it("returns the line unchanged without ranges", () => {
    expect(markHtml("<b>x</b>", [])).toBe("<b>x</b>");
  });
});

describe("pairedWordRanges()", () => {
  const line = (kind: string, content: string) => ({ kind, content });

  it("pairs the n-th deletion with the n-th addition of a block", () => {
    const r = pairedWordRanges([
      line("context", "fn main() {"),
      line("deletion", "  let a = 1;"),
      line("deletion", "  let b = 2;"),
      line("addition", "  let a = 10;"),
      line("addition", "  let b = 20;"),
      line("addition", "  let c = 3;"),
      line("context", "}"),
    ]);
    expect([...r.keys()].sort()).toEqual([1, 2, 3, 4]);
    expect(r.get(3)).toEqual([[10, 12]]);
  });

  it("does not pair across context lines", () => {
    const r = pairedWordRanges([
      line("deletion", "let a = 1;"),
      line("context", "x"),
      line("addition", "let a = 2;"),
    ]);
    expect(r.size).toBe(0);
  });
});

describe("sidePieces()", () => {
  it("marks what a conflict side changed against the base, line by line", () => {
    const p = sidePieces(["limit = 750", "same"], ["limit = 500", "same"]);
    expect(p.filter((x) => x.changed).map((x) => x.text)).toEqual(["750"]);
    expect(p.map((x) => x.text).join("")).toBe("limit = 750\nsame");
  });

  it("marks nothing without a reference", () => {
    expect(sidePieces(["a", "b"], null).some((x) => x.changed)).toBe(false);
  });
});

describe("audit 2026-10-25: budgets, scripts, alignment", () => {
  it("stops marking once the budget of a view is spent — no frozen window", () => {
    const line = (w: string) => Array.from({ length: 200 }, (_, i) => `${w}${i}`).join(" ");
    const budget = wordBudget();
    budget.cells = 50_000;
    // ~399 tokens a side: too long for one pair at all.
    expect(wordDiff(line("a"), line("b"), budget)).toBeNull();
    const short = (w: string) => Array.from({ length: 100 }, (_, i) => `${w}${i}`).join(" ");
    expect(wordDiff(short("a"), short("a").replace("a50", "b50"), budget)).not.toBeNull();
    expect(wordDiff(short("a"), short("a").replace("a51", "b51"), budget)).toBeNull();
  });

  it("marks single CJK characters instead of the whole sentence", () => {
    const d = wordDiff("今日は良い天気です", "今日は悪い天気です");
    expect(d?.old).toEqual([[3, 4]]);
    expect(d?.new).toEqual([[3, 4]]);
  });

  it("keeps a letter and its combining mark together", () => {
    // "café" with a decomposed é (e + U+0301) against "cafe".
    const d = wordDiff("le café noir", "le cafe noir".normalize("NFC"));
    expect(d?.old).toEqual([[3, 8]]);
  });

  it("pairs conflict lines across an inserted line", () => {
    const base = ["alpha", "beta one", "gamma"];
    const side = ["alpha", "inserted", "beta two", "gamma"];
    expect(pairLines(side, base)).toEqual(["alpha", null, "beta one", "gamma"]);
    const marked = sidePieces(side, base)
      .filter((p) => p.changed)
      .map((p) => p.text);
    expect(marked).toEqual(["two"]);
  });
});
