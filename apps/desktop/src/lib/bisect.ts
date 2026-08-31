/** Analyses the git bisect output (stdout+stderr) for the commit found and the
 *  rough number of remaining steps. Pure — Vitest-tested. */
export function parseBisectOutput(out: string): {
  firstBad: string | null;
  stepsLeft: number | null;
} {
  // Both wordings on purpose: up to git 2.54 the line read "… is the first bad
  // commit"; since 2.55 the term is quoted — "… is the first 'bad' commit"
  // (bisect.c: `"%s is the first '%s' commit\n"`, term_bad). The term itself is
  // matched loosely so a session with --term-new is recognised too.
  const bad = out.match(/([0-9a-f]{7,40}) is the first '?(?:bad|new)'? commit/i);
  const steps = out.match(/roughly (\d+) steps?/i);
  return {
    firstBad: bad ? bad[1] : null,
    stepsLeft: steps ? Number(steps[1]) : null,
  };
}
