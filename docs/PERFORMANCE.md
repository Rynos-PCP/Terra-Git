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

## Beyond the benchmark fixture

The fixture above stops at 50k files. For repos of Linux-kernel magnitude
(~1.46 million commits, ~90k files) there is a reproducible method with
measured before/after numbers and the open weaknesses in
[perf-stress-test.md](perf-stress-test.md).
