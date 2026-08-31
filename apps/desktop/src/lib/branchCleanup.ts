import type { BranchInfo } from "./api";

/** Names of the local branches that may safely be cleaned up automatically:
 *  orphaned (upstream gone), local and not the current branch. The deletion
 *  itself stays "safe" (force=false) — unmerged ones are rejected by the backend. */
export function goneDeletableCandidates(branches: BranchInfo[]): string[] {
  return branches.filter((b) => b.upstreamGone && !b.isRemote && !b.isHead).map((b) => b.name);
}
