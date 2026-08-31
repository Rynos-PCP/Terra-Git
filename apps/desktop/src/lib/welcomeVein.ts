// Geometry of the welcome vein: maps the repo sketch (peek_repo) onto the vein
// in the brand panel (WelcomeView). Since the design review of 2026-08-14 (the
// user's choice "core sample, horizontal", artifact afab021a) the vein is a
// horizontally lying core sample: the HEAD line runs as a STRAIGHT line through
// the rock, time from left (old) to right (new), the newest commit on the right.
// The nodes sit EVENLY distributed (user finding 2026-08-14: time-faithful
// placement clumped real bursts into unreadable blobs) and the sketch carries NO
// text — colour says what is going on: branches as parallel veins in their slot
// colour (quarter arc at the merge base), ANCESTOR branches (ahead 0, e.g. main
// behind the feature branch) as a coloured ring at the tip commit or collected
// at the left edge, tags as an ochre ring.
//
// Pure and DOM-free — for tests.

/** A node (commit), in viewBox coordinates (320 x 500). */
export interface VeinDot {
  x: number;
  y: number;
  r: number;
  /** At least one tag points at this commit (ochre ring). */
  hasTag: boolean;
}

/** A branch strand: quarter arc at the merge base, then a parallel vein. */
export interface VeinStrand {
  /** SVG path of the strand (V + arc + H; or H coming in from the left). */
  path: string;
  /** Sketch nodes on the strand (tip first, largest radius). */
  dots: VeinDot[];
  /** Colour slot of the graph palette (stable per branch name). */
  slot: number;
  /** Start/end x of the strand — drives the build-up animation left→right. */
  x0: number;
  x1: number;
  /** Path length in viewBox units — drives the drawing duration. */
  len: number;
}

/** Ring at the tip commit of an ancestor branch (ahead 0). */
export interface VeinRing {
  x: number;
  y: number;
  r: number;
  /** Colour slot of the graph palette (stable per branch name). */
  slot: number;
}

export interface VeinGeometry {
  /** SVG path of the core (fixed — the nodes move, not the vein). */
  main: string;
  /** Dashed continuation at the left edge ("older history"). */
  tail: string;
  /** Nodes on the core, in input order (newest first). */
  dots: VeinDot[];
  /** Branch veins (empty without sketched branches). */
  strands: VeinStrand[];
  /** Rings for ancestor branches, stacked radially (after the tag ring). */
  rings: VeinRing[];
}

interface SketchCommitIn {
  time: number;
  isMerge: boolean;
  hasTag?: boolean;
}

interface SketchBranchIn {
  name: string;
  baseIndex: number | null;
  ahead: number;
  tipTime: number;
}

/** More nodes would turn the sketch into a graph — cap them. */
const MAX_DOTS = 8;
/** Three vein tracks above the core; further branches are dropped. */
const MAX_LANES = 3;

/** Colour slots for strands: deliberately WITHOUT amber (4, close to the tag
 *  ochre) and orange (8, close to the red of the main strand) — no confusion. */
const STRAND_SLOTS = [2, 3, 5, 6, 7] as const;

/** Height of the core: just BELOW the l3 stratum seam (280..304) so the straight
 *  strand lies entirely within one stratum (halo = --strata-3). */
const MAIN_Y = 306;
/** Vein tracks above it; tracks 1 and 2 lie entirely in stratum 2. */
const LANE_YS = [274, 242, 210] as const;
/** Quarter-arc radius at the branch point. */
const RAD = 10;

/** x window of the commit nodes on the core (newest at X_MAX). */
const X_MAX = 290;
const X_MIN = 42;

export const VEIN_MAIN_PATH = "M20 306 H307";
export const VEIN_TAIL_PATH = "M4 306 H15";

/** Collection point at the left edge for ancestor tips before the window. */
const EDGE_X = 26;
/** ringCount key of the left edge (not a valid dot index). */
const EDGE_KEY = -1;

const round = (n: number) => Math.round(n * 10) / 10;

/** Stable colour slot per branch name (same logic as the repo avatars). */
export function strandSlot(name: string): number {
  let h = 0;
  for (let i = 0; i < name.length; i++) h = (h * 31 + name.charCodeAt(i)) | 0;
  return STRAND_SLOTS[Math.abs(h) % STRAND_SLOTS.length];
}

/** Compact, language-neutral age mark ("<1h", "2h", "3d") for the age scale of
 *  the history overview (historyOverview; the welcome vein itself has been
 *  text-free since the user finding of 2026-08-14). ROUND DOWN like timeAgo so
 *  the scale and the list text ("2 h ago") say the same number. */
export function ageText(ageSeconds: number): string {
  const h = ageSeconds / 3600;
  if (h < 1) return "<1h";
  if (h < 24) return `${Math.floor(h)}h`;
  return `${Math.floor(h / 24)}d`;
}

/** Decorative fallback: three calm nodes, no veins — the state before the first
 *  peek and for repos without a readable history. */
function decorative(): VeinGeometry {
  return {
    main: VEIN_MAIN_PATH,
    tail: VEIN_TAIL_PATH,
    dots: [0.25, 0.5, 0.75].map((f, i) => ({
      x: round(X_MIN + f * (X_MAX - X_MIN)),
      y: MAIN_Y,
      r: i === 1 ? 4 : 3,
      hasTag: false,
    })),
    strands: [],
    rings: [],
  };
}

/** Builds the vein geometry from the repo sketch (newest first, peek_repo). */
export function buildVein(
  commits: SketchCommitIn[],
  branches: SketchBranchIn[] = [],
): VeinGeometry {
  const shown = commits.slice(0, MAX_DOTS);
  if (shown.length === 0) return decorative();

  // Even distribution across the window, newest commit on the right — a fixed
  // distance per commit instead of time-faithful placement (user finding
  // 2026-08-14: real bursts clumped the nodes into unreadable blobs).
  const step = shown.length > 1 ? (X_MAX - X_MIN) / (shown.length - 1) : 0;
  const xs = shown.map((_, i) => X_MAX - i * step);

  const dots = shown.map((c, i) => ({
    x: round(xs[i]),
    y: MAIN_Y,
    r: i === 0 ? 5.5 : 3.8,
    hasTag: !!c.hasTag,
  }));

  // Tip time → x on the core (linear between the neighbouring commits).
  const oldest = shown[shown.length - 1].time;
  function tipXOf(tipTime: number): number {
    if (tipTime >= shown[0].time) return Math.min(298, X_MAX + 12);
    if (tipTime <= oldest) return Math.min(X_MIN, xs[shown.length - 1]);
    for (let k = 0; k < shown.length - 1; k++) {
      const a = shown[k].time;
      const b = shown[k + 1].time;
      if (tipTime <= a && tipTime >= b) {
        const f = a === b ? 0 : (tipTime - b) / (a - b);
        return xs[k + 1] + (xs[k] - xs[k + 1]) * f;
      }
    }
    return X_MIN;
  }

  const strands: VeinStrand[] = [];
  const rings: VeinRing[] = [];
  // Rings already assigned per node — further ones stack radially.
  const ringCount = new Map<number, number>();
  for (const b of branches) {
    const inWindow = b.baseIndex !== null && b.baseIndex < shown.length;

    // ahead 0 = the tip lies ON the core: a coloured ring instead of a vein.
    if (b.ahead <= 0) {
      if (inWindow) {
        const i = b.baseIndex as number;
        const d = dots[i];
        const n = ringCount.get(i) ?? 0;
        rings.push({
          x: d.x,
          y: MAIN_Y,
          r: round(d.r + 3.5 + (d.hasTag ? 2.2 : 0) + n * 2),
          slot: strandSlot(b.name),
        });
        ringCount.set(i, n + 1);
      } else {
        const n = ringCount.get(EDGE_KEY) ?? 0;
        rings.push({
          x: EDGE_X,
          y: MAIN_Y,
          r: round(4 + n * 2),
          slot: strandSlot(b.name),
        });
        ringCount.set(EDGE_KEY, n + 1);
      }
      continue;
    }

    if (strands.length >= MAX_LANES) continue;
    const laneY = LANE_YS[strands.length];
    const bx = inWindow ? xs[b.baseIndex as number] : 0;
    const x1 = Math.min(298, Math.max(tipXOf(b.tipTime), inWindow ? bx + 40 : 24));
    if (x1 - bx < (inWindow ? 24 : 12)) continue;

    // Quarter-arc branch point: vertically out of the core, arc, then horizontal —
    // branch points outside the window arrive as a straight vein from the left.
    let path: string;
    let len: number;
    if (inWindow) {
      path =
        `M${round(bx)} ${MAIN_Y} V${laneY + RAD} ` +
        `A${RAD} ${RAD} 0 0 1 ${round(bx + RAD)} ${laneY} H${round(x1)}`;
      len = MAIN_Y - laneY - RAD + 1.57 * RAD + (x1 - bx - RAD);
    } else {
      path = `M0 ${laneY} H${round(x1)}`;
      len = x1;
    }

    // Nodes near the tip (max 3, the tip first and larger).
    const strandDots: VeinDot[] = [];
    for (let j = 0; j < Math.min(3, b.ahead); j++) {
      const x = x1 - j * 16;
      if (x < (inWindow ? bx + RAD + 8 : 12)) break;
      strandDots.push({ x: round(x), y: laneY, r: j === 0 ? 4 : 2.8, hasTag: false });
    }

    strands.push({
      path,
      dots: strandDots,
      slot: strandSlot(b.name),
      x0: round(bx),
      x1: round(x1),
      len: round(len),
    });
  }

  return {
    main: VEIN_MAIN_PATH,
    tail: VEIN_TAIL_PATH,
    dots,
    strands,
    rings,
  };
}
