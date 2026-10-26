# terra-git — Read-path performance

Measurement basis for the performance budgets and for the question whether a
**gix read fast path** is needed. Benchmarks: `cargo bench -p tg-git-engine`
([benches/read_path.rs](../crates/tg-git-engine/benches/read_path.rs)),
size controllable via `TG_BENCH_FILES` / `TG_BENCH_COMMITS`.

## Budget

- Cold start < 1.5 s
- `status` at 10k files < 500 ms

## Measurements (2026-07-06, Windows 11, NTFS)

Fixture: tracked files in folders of 100, 5 % modified + 5 % untracked.

| Scenario | `status` (git2) | `status` (system git sidecar) |
|---|---|---|
| 15,000 files (1,500 modified) | **~28 ms** | ~54 ms |
| 50,000 files (5,000 modified) | ~87 ms | **~55 ms** |

With git2, `status` scales roughly linearly (~1.7 ms per 1,000 files); the
sidecar is nearly flat at ~55 ms (spawn-dominated). **Break-even ≈ 30,000 files.**

`log` (first 100) no longer runs on the libgit2 revwalk that the few-ms figure
of 2026-07-06 was measured on: since 2026-07-22 `log()` tries the system-git
sidecar first (`git log --topo-order`, which streams thanks to the commit-graph
file) and keeps the revwalk only as the fallback. The benchmark
(`log_first_100`) therefore measures the sidecar now, and the cost is
spawn-dominated like the sidecar `status` — on the Linux kernel the first page
takes 87 ms, see [perf-stress-test.md](perf-stress-test.md).

### Interpretation

- **The 500 ms budget is undercut by a factor of ~5.7 even at 50k files**
  (git2 ~87 ms) — the libgit2 status is fast enough at all realistic
  repo sizes.
- **The sidecar only wins above ~30k files**: below that, the process spawn
  (~50 ms floor) costs more than the parallel `lstat` saves. That is why
  `FAST_PATH_MIN_INDEX_ENTRIES = 30_000` — exactly at the measured
  crossover. At 100k+ (monorepos) the git2 scan keeps growing while the
  sidecar stays flat → the lead widens.
- **Commit graph:** terra-git writes the file after fetch/clone
  (`commit-graph write --reachable --split`); libgit2 and system git use it
  automatically for the history walk and ahead/behind.

### Consequence for the gix read fast path

Based on these numbers, gix is **not required**. For `status`, git2 is within
budget by a factor of ~5.7 even at 50k files and the cheap sidecar lever covers
the extreme sizes (>30k); `FAST_PATH_MIN_INDEX_ENTRIES` picks between the two.
For `log` there is no threshold at all: the sidecar is already the default path
and the libgit2 revwalk only the fallback, because that revwalk buffers the
complete graph (68 s on the kernel, see
[perf-stress-test.md](perf-stress-test.md)). Both levers work without a third
Git implementation and without divergence risk. The gix fast path stays
deferred until a repo of kernel magnitude **demonstrably** violates the budget;
the decision is on the record in [../ARCHITECTURE.md](../ARCHITECTURE.md). To
re-measure:
`TG_BENCH_FILES=100000 TG_BENCH_COMMITS=50000 cargo bench -p tg-git-engine`.

## Huge change sets: staging and rename detection (2026-10-13)

User report: staging ~24,800 changed files (+8.4 M / −11.6 M lines) looked
frozen. Reproduced with a synthetic change set of the same shape (modified,
deleted and new files; Windows 11, NTFS, release build):

| Step | Before | After |
|---|---|---|
| Status, 9,780 changes | 20 s | 0.02 s |
| Status, 24,780 changes | minutes | 0.3 s |
| Line balance, 9,780 changes | 40 s | 8.8 s (and never two at once) |
| Unstage all, 24,780 | up to 33 s | 0.1 s |
| Stage all, 24,780, first time | minutes, no feedback | ~50 s with a file counter, cancellable |

**Cause:** libgit2's inexact rename detection compares every added with every
deleted file — git itself gives up beyond `diff.renameLimit²`. It ran in the
status, before every stage/unstage/discard (`rename_pairs`) and in the line
balance. **Fix:** a budget of 250,000 candidate pairs per status side
(`RENAME_PAIR_BUDGET`, ≈0.6 s worst case); beyond it additions and deletions
are shown as such, as git does.

**Staging itself** is bound by writing one new object file per changed file.
Measured on fresh fixtures, libgit2 stages twice as fast as `git add`
(16.7 s vs. 33.5 s for 9,780 files; `core.fsync` makes no difference), and
writing the objects on parallel threads did not help (disk/virus-scanner
bound). So staging stays on libgit2 and, from 200 files on, reports a file
counter in the toolbar and can be cancelled — the index is written only at the
end, so a cancelled run stages nothing.

## Beyond the benchmark fixture

The fixture above stops at 50k files. For repos of Linux-kernel magnitude
(~1.46 million commits, ~90k files) there is a reproducible method with
measured before/after numbers and the open weaknesses in
[perf-stress-test.md](perf-stress-test.md).
