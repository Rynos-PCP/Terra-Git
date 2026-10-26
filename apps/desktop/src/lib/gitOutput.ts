// Reading facts out of git's own output (it runs with English messages, see
// sidecar.rs) — pure functions for the dialogs.

/** The note the backend puts where it shortened a long output. */
const OMITTED = /^\[… (\d+) lines omitted/;

/**
 * The files git names when it refuses because local changes are in the way:
 * tracked ones ("Your local changes to the following files
 * would be overwritten by merge:") and untracked ones ("The following
 * untracked working tree files would be overwritten by checkout:"). git
 * lists them indented below that line. A long list arrives shortened in the
 * middle (see `clean_detail`): `omitted` counts the entries left out there.
 */
export function filesInTheWayWithCount(output: string | null | undefined): {
  files: string[];
  omitted: number;
} {
  const files: string[] = [];
  let omitted = 0;
  if (!output) return { files, omitted };
  let listing = false;
  for (const line of output.split("\n")) {
    if (/would be overwritten by/i.test(line)) {
      listing = true;
      continue;
    }
    const gap = OMITTED.exec(line);
    if (listing && gap) {
      omitted += Number(gap[1]);
      continue;
    }
    if (listing && /^\s+\S/.test(line)) {
      const file = line.trim();
      if (!files.includes(file)) files.push(file);
    } else {
      listing = false;
    }
  }
  return { files, omitted };
}

/** The files of [`filesInTheWayWithCount`] alone. */
export function filesInTheWay(output: string | null | undefined): string[] {
  return filesInTheWayWithCount(output).files;
}

/**
 * The lock file git names when it cannot take it ("Unable to create
 * '/repo/.git/modules/lib/index.lock': File exists.") — a submodule's or a
 * linked worktree's lock is not the repository's own `.git/index.lock`. The
 * backend removes it only when it lies inside the repository's git directory.
 */
export function lockFileNamed(output: string | null | undefined): string | null {
  const m = output?.match(/unable to create '([^'\n]*index\.lock)'/i);
  return m ? m[1] : null;
}
