// "Update current branch from…": which branches can feed the
// current one, which one is preselected, and which ref is actually merged or
// rebased onto. Pure and DOM-free — for tests.
import type { BranchInfo } from "./api";

/** A branch the current one can be updated from. */
export interface UpdateCandidate {
  /** The branch as listed (`main`, `origin/main`). */
  name: string;
  isRemote: boolean;
  /** The upstream of a local branch (`origin/main`), if it has one. */
  upstream: string | null;
}

/**
 * Every branch except the current one: local branches first, then remote ones
 * — remote ones even when a local branch of the same name exists (updating a
 * release branch from `origin/main` is the common case). The current branch's
 * own upstream is left out: that is what a pull is for.
 */
export function updateCandidates(
  branches: BranchInfo[],
  current: string | null,
  currentUpstream: string | null,
): UpdateCandidate[] {
  const keep = (b: BranchInfo) =>
    !b.isHead && b.name !== current && b.name !== currentUpstream && !b.name.endsWith("/HEAD");
  const toCandidate = (b: BranchInfo): UpdateCandidate => ({
    name: b.name,
    isRemote: b.isRemote,
    upstream: b.isRemote ? null : (b.upstream ?? null),
  });
  return [
    ...branches.filter((b) => !b.isRemote && keep(b)).map(toCandidate),
    ...branches.filter((b) => b.isRemote && keep(b)).map(toCandidate),
  ];
}

/**
 * The preselected source: the one last used for THIS branch, else the
 * default branch (its local copy when there is one, so "use the remote state"
 * can apply), else nothing.
 */
export function preselect(
  candidates: UpdateCandidate[],
  remembered: string | undefined,
  defaultBranch: string | null,
): string | null {
  const has = (name: string | null | undefined) =>
    !!name && candidates.some((c) => c.name === name);
  if (has(remembered)) return remembered!;
  if (defaultBranch) {
    const local = defaultBranch.includes("/")
      ? defaultBranch.slice(defaultBranch.indexOf("/") + 1)
      : defaultBranch;
    if (has(local)) return local;
    if (has(defaultBranch)) return defaultBranch;
  }
  return null;
}

/**
 * The ref that is actually merged or rebased onto: a local branch with an
 * upstream and "use the remote state" on → its upstream (fetched first);
 * otherwise the branch itself.
 */
export function effectiveSource(candidate: UpdateCandidate, useRemoteState: boolean): string {
  return useRemoteState && candidate.upstream ? candidate.upstream : candidate.name;
}

/** Key under which the chosen source is remembered per repo and branch. */
export function rememberKey(repoPath: string, branch: string): string {
  return `${repoPath}::${branch}`;
}
