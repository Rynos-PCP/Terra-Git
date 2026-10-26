// Intra-line ("word") diff: which parts of a changed line
// actually changed — shared by the diff view and the conflict resolver.
//
// Lines are split into tokens (words, runs of whitespace, single punctuation
// marks) and compared with a longest-common-subsequence; the tokens outside
// it are the change. Pure functions, no DOM.

/** A changed stretch of a line: `[start, end)` in characters. */
export type Range = [number, number];

export interface WordDiff {
  old: Range[];
  new: Range[];
}

/** Longer lines are not compared: the table grows with the product of both
 *  token counts (about 300 tokens a side — a long paragraph of prose). */
const MAX_CELLS = 100_000;
/** A line longer than this is not read word by word (minified code). */
const MAX_LINE = 4_000;
/** Below this share of unchanged text the pair counts as rewritten — marking
 *  nearly the whole line would only add noise. */
const MIN_COMMON = 0.3;

/**
 * What one view may spend on word marking, in table cells (about 40 ms). The
 * marking runs while the diff renders: a crafted or huge diff — thousands of
 * long changed lines — must not freeze the window. Spent, the rest of the
 * view simply shows its lines without marks.
 */
export interface WordBudget {
  cells: number;
}
const VIEW_CELLS = 10_000_000;
export function wordBudget(): WordBudget {
  return { cells: VIEW_CELLS };
}

// Tokens: whitespace runs; a single CJK character (those scripts write no
// spaces, a whole sentence would otherwise be one "word"); words — letters,
// digits, `_` and combining marks, so `é` written as e + U+0301 stays whole;
// any other single character with its combining marks.
const CJK = String.raw`\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}`;
const TOKEN = new RegExp(
  String.raw`\s+|[${CJK}]\p{M}*|(?:(?![${CJK}])[\p{L}\p{M}\p{N}_])+|[^\s\p{L}\p{M}\p{N}_]\p{M}*`,
  "gu",
);

function tokens(line: string): string[] {
  return line.match(TOKEN) ?? [];
}

/** Ranges of the tokens not in `keep`, adjacent ones merged. */
function changedRanges(toks: string[], keep: boolean[]): Range[] {
  const out: Range[] = [];
  let pos = 0;
  toks.forEach((tok, i) => {
    const end = pos + tok.length;
    if (!keep[i]) {
      const last = out[out.length - 1];
      if (last && last[1] === pos) last[1] = end;
      else out.push([pos, end]);
    }
    pos = end;
  });
  return out;
}

/**
 * The changed parts of `a` (old) and `b` (new). `null` when there is nothing
 * worth marking: equal lines, lines too long to compare, lines that have too
 * little in common — or when `budget` is spent.
 */
export function wordDiff(a: string, b: string, budget?: WordBudget): WordDiff | null {
  if (a === b || a.length > MAX_LINE || b.length > MAX_LINE) return null;
  const ta = tokens(a);
  const tb = tokens(b);
  const n = ta.length;
  const m = tb.length;
  const cells = n * m;
  if (n === 0 || m === 0 || cells > MAX_CELLS) return null;
  if (budget) {
    if (budget.cells < cells) return null;
    budget.cells -= cells;
  }

  // LCS lengths, suffix-based: lcs[i][j] = LCS of ta[i..] and tb[j..].
  const lcs: Uint32Array[] = Array.from({ length: n + 1 }, () => new Uint32Array(m + 1));
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      lcs[i][j] = ta[i] === tb[j] ? lcs[i + 1][j + 1] + 1 : Math.max(lcs[i + 1][j], lcs[i][j + 1]);
    }
  }
  const keepA = new Array<boolean>(n).fill(false);
  const keepB = new Array<boolean>(m).fill(false);
  let common = 0;
  for (let i = 0, j = 0; i < n && j < m;) {
    if (ta[i] === tb[j]) {
      keepA[i] = keepB[j] = true;
      common += ta[i].length;
      i++;
      j++;
    } else if (lcs[i + 1][j] >= lcs[i][j + 1]) i++;
    else j++;
  }
  // Common whitespace alone does not make two lines similar.
  if (common / Math.max(a.length, b.length) < MIN_COMMON) return null;
  return { old: changedRanges(ta, keepA), new: changedRanges(tb, keepB) };
}

/** `text` cut into plain and changed pieces, for rendering as text nodes. */
export function pieces(text: string, ranges: Range[]): { text: string; changed: boolean }[] {
  const out: { text: string; changed: boolean }[] = [];
  let pos = 0;
  for (const [s, e] of ranges) {
    if (s > pos) out.push({ text: text.slice(pos, s), changed: false });
    out.push({ text: text.slice(s, e), changed: true });
    pos = e;
  }
  if (pos < text.length) out.push({ text: text.slice(pos), changed: false });
  return out;
}

/**
 * Wraps the characters in `ranges` of an already escaped/highlighted HTML line
 * in `<mark class="wd">`. Positions count text characters (an entity such as
 * `&lt;` is one); a mark is closed before every tag and reopened after it, so
 * the nesting stays valid whatever the highlighter produced.
 */
export function markHtml(html: string, ranges: Range[]): string {
  if (ranges.length === 0) return html;
  let out = "";
  let pos = 0;
  let r = 0;
  let open = false;
  for (let i = 0; i < html.length;) {
    if (html[i] === "<") {
      const close = html.indexOf(">", i);
      const end = close < 0 ? html.length : close + 1;
      if (open) {
        out += "</mark>";
        open = false;
      }
      out += html.slice(i, end);
      i = end;
      continue;
    }
    let len = 1;
    if (html[i] === "&") {
      const semi = html.indexOf(";", i);
      if (semi > i && semi - i <= 10) len = semi - i + 1;
    }
    while (r < ranges.length && ranges[r][1] <= pos) r++;
    const inside = r < ranges.length && ranges[r][0] <= pos;
    if (inside && !open) {
      out += '<mark class="wd">';
      open = true;
    } else if (!inside && open) {
      out += "</mark>";
      open = false;
    }
    out += html.slice(i, i + len);
    i += len;
    pos++;
  }
  if (open) out += "</mark>";
  return out;
}

/**
 * Word ranges for the lines of a diff hunk, by line index: a block of
 * deletions followed by additions pairs its n-th deletion with its n-th
 * addition — the same pairs the split view puts side by side.
 */
export function pairedWordRanges(
  lines: readonly { kind: string; content: string }[],
  budget: WordBudget = wordBudget(),
): Map<number, Range[]> {
  const ranges = new Map<number, Range[]>();
  let dels: number[] = [];
  let adds: number[] = [];
  const flush = () => {
    for (let i = 0; i < Math.min(dels.length, adds.length); i++) {
      const d = wordDiff(lines[dels[i]].content, lines[adds[i]].content, budget);
      if (d) {
        ranges.set(dels[i], d.old);
        ranges.set(adds[i], d.new);
      }
    }
    dels = [];
    adds = [];
  };
  lines.forEach((line, idx) => {
    if (line.kind === "deletion") {
      if (adds.length) flush();
      dels.push(idx);
    } else if (line.kind === "addition") adds.push(idx);
    else flush();
  });
  flush();
  return ranges;
}

/** Above this many line pairs the lines of a conflict are paired by position. */
const MAX_ALIGN_CELLS = 250_000;
/** Above this many candidate pairs a gap is paired by position. */
const MAX_GAP_CELLS = 2_500;

/** How much two lines share: common tokens (as a multiset) over the longer. */
function similarity(a: string, b: string): number {
  const ta = tokens(a).filter((t) => t.trim());
  const tb = tokens(b).filter((t) => t.trim());
  if (ta.length === 0 || tb.length === 0) return 0;
  const counts = new Map<string, number>();
  for (const t of ta) counts.set(t, (counts.get(t) ?? 0) + 1);
  let shared = 0;
  for (const t of tb) {
    const c = counts.get(t) ?? 0;
    if (c > 0) {
      shared++;
      counts.set(t, c - 1);
    }
  }
  return shared / Math.max(ta.length, tb.length);
}

/**
 * The pairs of a gap between two equal lines, in order: the most similar
 * lines pair up (an inserted line stays alone), dissimilar ones do not.
 * Returns index pairs into `a` and `b`.
 */
function pairGap(a: readonly string[], b: readonly string[]): [number, number][] {
  const p = a.length;
  const q = b.length;
  if (p === 0 || q === 0) return [];
  if (p * q > MAX_GAP_CELLS) {
    return Array.from({ length: Math.min(p, q) }, (_, k) => [k, k] as [number, number]);
  }
  const sim = a.map((x) => b.map((y) => similarity(x, y)));
  // best[i][j]: the best total similarity of a[i..] against b[j..].
  const best: Float64Array[] = Array.from({ length: p + 1 }, () => new Float64Array(q + 1));
  for (let i = p - 1; i >= 0; i--) {
    for (let j = q - 1; j >= 0; j--) {
      const take = sim[i][j] >= MIN_COMMON ? sim[i][j] + best[i + 1][j + 1] : 0;
      best[i][j] = Math.max(take, best[i + 1][j], best[i][j + 1]);
    }
  }
  const pairs: [number, number][] = [];
  for (let i = 0, j = 0; i < p && j < q;) {
    if (sim[i][j] >= MIN_COMMON && best[i][j] === sim[i][j] + best[i + 1][j + 1]) {
      pairs.push([i, j]);
      i++;
      j++;
    } else if (best[i + 1][j] >= best[i][j + 1]) i++;
    else j++;
  }
  return pairs;
}

/**
 * For each of `lines`, the line of `ref` it is compared with. Equal lines are
 * anchors (a longest common subsequence of whole lines); between two anchors
 * the n-th remaining line of each side pair up — pairing by position alone
 * would compare every line after an inserted one with the wrong partner.
 */
export function pairLines(lines: readonly string[], ref: readonly string[]): (string | null)[] {
  const n = lines.length;
  const m = ref.length;
  const out: (string | null)[] = new Array<string | null>(n).fill(null);
  if (n * m > MAX_ALIGN_CELLS) {
    for (let i = 0; i < Math.min(n, m); i++) out[i] = ref[i];
    return out;
  }
  const lcs: Uint32Array[] = Array.from({ length: n + 1 }, () => new Uint32Array(m + 1));
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      lcs[i][j] =
        lines[i] === ref[j] ? lcs[i + 1][j + 1] + 1 : Math.max(lcs[i + 1][j], lcs[i][j + 1]);
    }
  }
  let gapA: number[] = [];
  let gapB: number[] = [];
  const flush = () => {
    for (const [a, b] of pairGap(
      gapA.map((k) => lines[k]),
      gapB.map((k) => ref[k]),
    ))
      out[gapA[a]] = ref[gapB[b]];
    gapA = [];
    gapB = [];
  };
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (lines[i] === ref[j]) {
      flush();
      out[i] = ref[j];
      i++;
      j++;
    } else if (lcs[i + 1][j] >= lcs[i][j + 1]) gapA.push(i++);
    else gapB.push(j++);
  }
  while (i < n) gapA.push(i++);
  while (j < m) gapB.push(j++);
  flush();
  return out;
}

/**
 * One side of a conflict as pieces, joined with line breaks, with what it
 * changed against `ref` (the base, or the other side) marked line by line —
 * each line against its partner from [`pairLines`].
 */
export function sidePieces(
  lines: readonly string[],
  ref: readonly string[] | null,
  budget: WordBudget = wordBudget(),
): { text: string; changed: boolean }[] {
  const out: { text: string; changed: boolean }[] = [];
  const partners = ref ? pairLines(lines, ref) : [];
  lines.forEach((line, i) => {
    if (i > 0) out.push({ text: "\n", changed: false });
    const partner = partners[i];
    const ranges = partner != null ? wordDiff(partner, line, budget)?.new : undefined;
    out.push(...pieces(line, ranges ?? []));
  });
  return out;
}
