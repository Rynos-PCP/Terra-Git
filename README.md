# terra-git

A fast, resource-friendly Git desktop client for Windows, macOS and Linux — built
with **Tauri 2** (Rust core) and **Svelte 5**, with first-class support for
**self-hosted GitLab**.

[![CI](https://github.com/Rynos-PCP/Terra-Git/actions/workflows/ci.yml/badge.svg)](https://github.com/Rynos-PCP/Terra-Git/actions/workflows/ci.yml)
[![Latest release](https://img.shields.io/github/v/release/Rynos-PCP/Terra-Git)](https://github.com/Rynos-PCP/Terra-Git/releases/latest)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Platforms: Windows, macOS, Linux](https://img.shields.io/badge/platforms-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey.svg)](#download)

![terra-git](docs/images/changes.png)

## Why terra-git

- **Lightweight.** No bundled Chromium — the UI renders in the operating system's
  own WebView, and local Git work runs in-process through `git2`. `status` takes
  ~28 ms on 15,000 files and ~87 ms on 50,000; past 30,000 index entries a
  system-git fast path takes over at a near-flat ~55 ms. Method and full numbers:
  [docs/PERFORMANCE.md](docs/PERFORMANCE.md).
- **Self-hosted friendly.** Merge-request and pull-request lists with inline CI
  status for GitLab (self-hosted, subpath installations and custom CAs included),
  GitHub (including GitHub Enterprise Server) and Gitea/Forgejo (including
  Codeberg). Fetch, pull, push and clone go through your system `git`, so the
  credential helpers, SSH agents and however many accounts and hosts you already
  have keep working — terra-git never asks you into an OAuth silo of its own.
  Provider tokens live in the OS keychain and nowhere else.
- **Complete.** The whole GitHub Desktop day-to-day workflow, plus the things its
  issue tracker keeps asking for: hunk- and line-level staging, a real conflict
  workbench, interactive rebase, a full stash manager, worktrees, submodules,
  sparse checkout, blame and bisect. The parity matrix, with issue numbers and
  reaction counts: [docs/FEATURES.md](docs/FEATURES.md).

## Download

Bundles for all three platforms are on the
[releases page](https://github.com/Rynos-PCP/Terra-Git/releases):

| Platform | Files |
|---|---|
| Windows | `.msi`, NSIS `.exe` |
| macOS | `.dmg` (Apple Silicon — a Universal build is not produced yet) |
| Linux | `.AppImage`, `.deb`, `.rpm` |

Known limitations, stated up front:

- **The bundles are not code-signed.** Windows SmartScreen and macOS Gatekeeper
  will call terra-git an unknown publisher on first launch. That is deliberate
  for 1.0 — there is no budget for certificates and notarization yet.
- **There is no auto-update.** New versions are announced on the releases page;
  watch the repository if you want to hear about them.
- **Sign-in to GitHub, GitLab and Gitea/Forgejo is by personal access token**,
  not OAuth. The token lives in your OS keychain; OAuth is on the
  [roadmap](ROADMAP.md).
- **Hunk staging has a known race.** If another program rewrites a file between
  the diff being drawn and your click, the hunk that gets staged or discarded
  can differ from the one you saw.

### Requirements

- **Git 2.40 or newer on your `PATH`.** Remote operations shell out to your
  system `git` — that is what makes your existing credential helpers and SSH
  agents work. Everything local runs without it; fetch, pull, push and clone
  do not.
- **Windows:** the WebView2 runtime (preinstalled on Windows 11).
- **Linux:** WebKitGTK 4.1 (`libwebkit2gtk-4.1-0`) and GTK 3 (`libgtk-3-0`). On
  some VMs and GPUs the AppImage comes up as a blank white window — start it
  with `WEBKIT_DISABLE_DMABUF_RENDERER=1`.
- **macOS:** nothing beyond the OS itself.

## Features

- **Staging and diff** — stage or discard down to the hunk and the single line,
  with direction-aware partial patches; tree or list view; file filter;
  multi-select discard; syntax highlighting for 21 languages; image diffs.
- **Committing** — summary and description with the 72-character warning,
  co-authors, amend with a warning once the commit is already pushed, a
  per-repository message history, GPG/SSH signing through system `git`.
- **History** — a commit graph across all branches with tag and ref badges,
  search over message, author and ID, a switchable side-by-side diff, blame per
  file, a bisect assistant.
- **Rewriting history** — cherry-pick, revert, squash, interactive rebase as a
  UI, multi-step undo and redo. Before every rewrite terra-git writes a backup
  ref on its own, with a restore view to bring the old state back.
- **Conflicts** — a conflict workbench that labels the two sides with the actual
  branch names instead of "ours" and "theirs", takes changes over segment by
  segment, and hands off to your external merge tool per file.
- **Branches and remotes** — create, switch, rename, delete with a merged check,
  merge, rebase, force-push with `--force-with-lease` behind a confirmation, and
  full remote management (add, rename, change URL, remove) — still an open
  request on GitHub Desktop (#15797).
- **Providers** — create and list pull and merge requests in the app for GitHub
  (incl. GHES), GitLab (incl. self-hosted) and Gitea/Forgejo (incl. Codeberg),
  CI status inline; from the list, "Create in browser…" hands the request over to
  the host's own page instead.
- **Local pipeline cockpit** — run the jobs from `.gitlab-ci.yml` or GitHub
  Actions on your own machine through `gitlab-ci-local`/`act` (detected, not
  bundled), with a job graph, status chips and a live log.
- **Large repositories** — the system-git `status` fast path above 30,000 index
  entries, commit-graph maintenance after fetch and clone, a file watcher,
  virtualized lists, paged history, streamed diffs.
- **Getting around** — command palette on `Ctrl+K`, worktrees, submodules,
  sparse checkout, an SSH key manager with TOFU host confirmation, multiple
  windows, a stash manager that also does partial stashes.
- **Comfort** — light and dark theme, English and German switchable without a
  restart, accessibility settings for font size, contrast and motion.

![Repository history with the commit graph](docs/images/history.png)

The history covers the whole repository — every branch, merge curves, tag and ref
badges — and searches over message, author and commit ID.

![Local pipeline cockpit](docs/images/pipeline.png)

The pipeline cockpit reads the repository's CI configuration, draws the job graph
by stage and runs jobs on your own machine, so a red pipeline costs you a minute
instead of a push.

## Architecture

```
crates/tg-domain        domain types (serde, Tauri-free)
crates/tg-git-engine    GitEngine trait: git2 (in-process, vendored libgit2)
                        + system-git sidecar for fetch/pull/push/clone
crates/tg-auth          OS keychain, git-credential helper
crates/tg-providers     GitHub/GitLab/Gitea REST: PR/MR list + CI status
apps/desktop            Tauri 2 app: Svelte 5 frontend (runes) + tg-app (commands)
```

How the pieces fit together, and where the code deliberately departs from the
original design: [ARCHITECTURE.md](ARCHITECTURE.md). What comes next:
[ROADMAP.md](ROADMAP.md).

## Building from source

Prerequisites:

- **Rust**, stable, via [rustup](https://rustup.rs) — `rust-toolchain.toml` pins
  the channel and pulls in clippy and rustfmt.
- **Node.js 20.19 or newer** (CI builds on 22, pinned in `.nvmrc`) and npm.
- **A C toolchain** for the vendored libgit2: MSVC Build Tools 2022 with the C++
  workload on Windows, the Xcode command line tools on macOS, `build-essential`
  on Linux.
- **Linux only**, the Tauri 2 system dependencies: `libwebkit2gtk-4.1-dev`,
  `libjavascriptcoregtk-4.1-dev`, `libsoup-3.0-dev`, `libgtk-3-dev`,
  `librsvg2-dev`, `libssl-dev`, `libayatana-appindicator3-dev`, `libdbus-1-dev`,
  `pkg-config`, `patchelf`; plus `libfuse2` for `npm run tauri build`, because
  the AppImage step runs linuxdeploy, which is itself an AppImage.

```bash
cd apps/desktop
npm ci               # exactly the lockfile, no install scripts
npm run tauri dev    # app with hot reload (Vite + cargo)
npm run tauri build  # bundles into <repo root>/target/release/bundle/
```

The full set of gates CI runs, across four jobs — the Rust matrix on Windows,
macOS and Linux, the performance budget, the cargo-deny supply-chain check and
the frontend job on Linux. All of them have to be green:

```bash
cargo fmt --all -- --check
# Clippy before the tests, as in CI: it is the cheap, deterministic gate.
cargo clippy --workspace --all-targets -- -D warnings
# --no-fail-fast as in CI, so one run reports every failing test binary.
cargo test --workspace --no-fail-fast
cargo deny check   # supply chain; needs `cargo install cargo-deny`
# The performance job: the budget gate, plus a compile check of the benchmarks.
TG_PERF_FILES=5000 cargo test -p tg-git-engine --test perf_budget -- --ignored --nocapture
cargo bench -p tg-git-engine --no-run

cd apps/desktop
npm run lint          # ESLint
npm run format:check  # Prettier
npm run check         # svelte-check (types + a11y)
npm test              # Vitest
npm run build
```

Project layout, the E2E setup, the Tauri-free mock harness and the per-OS release
build: [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md).

## Documentation

| Document | What is in it |
|---|---|
| [docs/HANDBOOK.md](docs/HANDBOOK.md) | User handbook: everyday use, the power features, troubleshooting — with screenshots |
| [docs/FEATURES.md](docs/FEATURES.md) | Feature matrix against GitHub Desktop, plus the community wish list |
| [ARCHITECTURE.md](ARCHITECTURE.md) | The structure as built, and the deliberate deviations |
| [ROADMAP.md](ROADMAP.md) | Planned work and the explicit non-goals |
| [CHANGELOG.md](CHANGELOG.md) | What changed, per release |
| [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) | Building, testing, releasing |
| [docs/PERFORMANCE.md](docs/PERFORMANCE.md) | Read-path benchmarks and the budgets behind them |
| [CONTRIBUTING.md](CONTRIBUTING.md) | Conventions, commit format, what a pull request needs |
| [SUPPORT.md](SUPPORT.md) | Where to ask a question, how to report a bug |
| [SECURITY.md](SECURITY.md) | Reporting a vulnerability, and what is in scope |

## Contributing

Bug reports, feature requests and pull requests are welcome — start with
[CONTRIBUTING.md](CONTRIBUTING.md) for the conventions, and
[CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) for how we talk to each other. Found a
security problem? Do not open an issue; follow [SECURITY.md](SECURITY.md).

## License

MIT — see [LICENSE](LICENSE). The dependencies and their licenses are listed in
[THIRD-PARTY-NOTICES.txt](THIRD-PARTY-NOTICES.txt), which ships inside every
bundle. It has three parts: the Rust crates, the npm packages compiled into the
shipped frontend, and the C libraries compiled in from vendored sources —
libgit2 (GPL-2.0 with a linking exception) and zlib, whose own licences differ
from what their Rust wrapper crates declare.
