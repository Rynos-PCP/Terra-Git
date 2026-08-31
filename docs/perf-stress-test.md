# Stress test: terra-git against a huge repo

Goal: verify that terra-git stays fluid even on repos of the Linux-kernel
class (>1 million commits, >80k files) — status, history, diff, blame,
search and the commit workshop. The test is **safe**: it runs in a
separate directory, read-only or with local throwaway commits, and you
have no push rights on the test repo.

## 1. Obtain the test object

```powershell
mkdir C:\terra-git-stress
git clone --no-checkout https://github.com/torvalds/linux.git C:\terra-git-stress\linux
git -C C:\terra-git-stress\linux checkout master
```

Key figures (as of 2026): ~1.4 million commits, ~90k files, ~6 GB `.git` +
~1.6 GB working directory. Depending on your connection the clone takes
10–40 min. `--no-checkout` + a separate `checkout` separates network time
from disk time.

Smaller/larger alternatives: `llvm-project` (~550k commits), `gecko-dev`
(~9 GB, even harder). **Important:** do NOT use `--depth` or
`--filter=blob:none` — both distort the measurement (shallow history, or
blobs fetched over the network in the middle of a diff).

## 2. Engine measurement (automated)

The report test measures the app's core reads directly against the Rust
engine (warm run: warm up once, then measure) and prints the timings:

```powershell
$env:TG_PERF_REPO = "C:\terra-git-stress\linux"
cargo test -p tg-git-engine --test perf_budget real_repo_report -- --ignored --nocapture
```

Measured: `status`, `log` (first page + a deep page via
`TG_PERF_DEEP_SKIP`, default 100000), `commit_diff` of HEAD, `branches`,
`unpushed_commits`, `search_log "fix"` and `blame` (file via
`TG_PERF_BLAME`, default `MAINTAINERS` — at ~25k lines with a long
history a brutal blame case).

**Interpretation** (targets, warm cache):

| Operation | Target | Pain threshold |
|---|---|---|
| status | < 200 ms | > 1 s |
| log page | < 200 ms | > 1 s |
| commit_diff (ordinary commit) | < 100 ms | > 1 s |
| blame of a huge file | < 3 s | > 10 s |
| search_log (100 hits) | < 2 s | > 10 s |

A cold cache (first access after a reboot) may cost a multiple of that —
so measure once right after a reboot and compare.

## 3. App measurement (by hand)

1. Start terra-git, open `C:\terra-git-stress\linux`. Stopwatch: time
   until status + history are on screen.
2. Scroll quickly through the history (the VirtualList must not stutter),
   click a merge commit with many files.
3. Open blame on `MAINTAINERS`; search for `fix` in the search box.
4. Task Manager next to it: watch the app's RAM (stable? does it grow
   without bound while scrolling?), CPU back to ~0 % once the action is done.
5. **Workshop stress test** (local, safe — you cannot push):

   ```powershell
   cd C:\terra-git-stress\linux
   "test" >> MAINTAINERS; git commit -aqm "wip: test 1"
   "test" >> README;      git commit -aqm "wip: test 2"
   "test" >> Makefile;    git commit -aqm "wip: test 3"
   ```

   Open the commit workshop in terra-git: timings for loading, then
   combine reword + squash + drop and apply — the rebase runs on the huge
   working directory. Afterwards check: is the backup ref present under
   "Backups…"? Clean up:

   ```powershell
   git reset --hard origin/master
   ```

## 4. Clean up

```powershell
Remove-Item -Recurse -Force C:\terra-git-stress
```

## Measured reference values

Results of previous runs (warm cache) for comparison — note machine, date
and commit, otherwise the numbers are worthless:

| Date | Machine | Repo | status | log 1st page | log deep | diff HEAD | blame MAINTAINERS | search 100 |
|---|---|---|---|---|---|---|---|---|
| 2026-07-22 | Win11, NVMe SSD | linux@master (1.46 million commits) | 0.42 s | 68.8 s | 69.8 s | 0.52 s | 48.5 s | 67.5 s |
| 2026-07-22 (after the sidecar streaming fix) | Win11, NVMe SSD | linux@master (1.46 million commits) | 0.78 s | **87 ms** | **343 ms** | 0.95 s | 48.5 s | **77 ms** |

Baseline git CLI on the same day/repo: log 1st page 61 ms, log
`--skip=100000` 1.2 s, `--grep=fix` 66 ms, blame `--porcelain` 49.4 s.

## Findings from 2026-07-22

1. **[FIXED]** `log()`/`search_log()` now stream
   `git log --topo-order` through the sidecar (the libgit2 fallback stays);
   on repo open a background process maintains the commit graph.
   Result: history page 87 ms, search 77 ms, deep page 343 ms.
   Remaining edge: on the VERY FIRST open of a freshly cloned huge repo
   there is no commit graph yet — until the background write is through
   (~17 s on the kernel), `git log --topo-order` falls back to the full
   walk (~15 s). Original finding:

   **History/search are unusable on huge repos** (68 s instead of the
   200 ms target; git CLI: 61 ms). Cause verified: the libgit2 revwalk
   buffers the complete graph first under `Sort::TOPOLOGICAL` AND under
   `Sort::TIME`; a commit-graph file (`git commit-graph write --reachable`)
   changes nothing about that. Only `Sort::NONE` streams: first page then
   **31.8 ms** (factor ~2100), deep page 4.6 s. But `Sort::NONE` does not
   guarantee children-before-parents order — the lane algorithm of the
   history graph could glitch at merge diamonds. Fix options:
   (a) stream `git log` through the sidecar (recommended — correct AND
   fast, the `LC_ALL=C` discipline already exists), (b) `Sort::NONE` +
   an order-tolerant lane algorithm, (c) a hybrid with a commit-count
   threshold.

   Re-measurement of the CLI with sorting (200 commits): `--topo-order`
   **53 ms** (identical to the default — git streams the topo order thanks
   to generation numbers from the commit graph), without commit graph
   14.9 s, `--topo-order --skip=100000` 280 ms. That makes option (a)
   compromise-free: exactly today's children-before-parents order, factor
   1300. Prerequisite: terra-git ensures a commit graph in the background
   on repo open (`git commit-graph write --reachable`; on the kernel 17 s
   once, incremental afterwards) — the same pattern as the status
   accelerators in `enable_status_accelerators`.
2. **Blame is at CLI parity** (48.5 s vs. 49.4 s — it runs through the
   sidecar after all). No engine deficit, but the UI needs visible
   progress + cancel for it. Caution: the engine caps at 5000 lines
   (MAINTAINERS has more) — the UI should point out the cap.
3. **status 0.42 s** — slightly above the 200 ms target, usable. The 13
   "modified" files in the fresh clone are NTFS case collisions of the
   kernel repo (`xt_CONNMARK.h` vs. `xt_connmark.h`); the git CLI shows
   the same ones. A good candidate for the "explain the cause" mechanism
   of the diff view.
4. **branches/unpushed_commits < 7 ms** — excellent. `commit_diff` HEAD
   0.52 s: above the 100 ms target, uncritical.
