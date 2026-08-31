# Contributing to terra-git

terra-git is a native, resource-friendly Git desktop client (Tauri 2 +
Rust core + Svelte 5). This file collects the **binding conventions** the project
is built on. They take precedence over personal style.

## Project layout

- `crates/tg-domain` — shared, serializable domain types (no logic dependencies).
- `crates/tg-git-engine` — Git engine (git2 locally, system-git sidecar for remote).
- `crates/tg-auth` — credentials, OS keychain (keyring 3).
- `crates/tg-providers` — hosting APIs (GitHub/GitLab/Gitea), neutral domain model.
- `apps/desktop/src-tauri` (`tg-app`) — Tauri command layer, thin edge.
- `apps/desktop/src` — Svelte 5 frontend (thin, virtualized view).

Guiding principle: **the Rust core is the single source of truth**; the frontend
holds no Git logic. Dependencies only point downwards.

## Rust conventions (binding)

- **Concurrency:** locks held across `await` points are
  `tokio::sync::Mutex`/`RwLock`, **never** `std::sync::Mutex` (the guard is not
  `Send` → compile errors/deadlocks).
- **Blocking work** (git2/FS) runs in `spawn_blocking`, never on the
  main/async thread — the UI never freezes.
- **Errors:** `thiserror` with typed enums in the library crates;
  `anyhow` only at the app edge. At the command boundary, errors are mapped to a
  serde-serializable frontend error with a **stable code**.
- **Logging:** `tracing` (span-based), no `println!` diagnostics.
- **Security:** validate free-text arguments from the frontend (paths, URLs, branch
  names) before handing them to the git CLI (no leading `-`, no
  `ext::`/`fd::` transports, pathspec injection). Secrets live only in the OS keychain,
  never in `tauri-plugin-store`/plain text.
- **Formatting:** `cargo fmt` (default profile) is mandatory — CI checks
  `cargo fmt --all -- --check`.

## Frontend conventions

- Svelte 5 **runes** (`$state`/`$derived`/`$props`), no legacy stores in new code.
- User-facing text goes through the i18n layer (`t(...)`); **de** and **en** must
  keep key parity (enforced by Vitest).
- Pure logic is pulled out of the component into a testable function and covered
  with Vitest.

## Tests & local gates

These commands should be green before a push — the same gates, in the same
order, as the Rust and frontend jobs in
[.github/workflows/ci.yml](.github/workflows/ci.yml):

```sh
# Rust (workspace)
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings   # CI runs this before the tests
cargo test --workspace --no-fail-fast                   # as in CI: one run reports every failure

# Frontend
cd apps/desktop
npm ci
npm run lint         # ESLint (correctness)
npm run format:check # Prettier — a CI gate; `npm run format` rewrites
npm run check        # svelte-check (types/a11y)
npm test             # Vitest (pure frontend logic)
npm run build
```

The E2E smoke tests (`npm run e2e`) boot the real app through tauri-driver
and need a window (locally, not in the container CI).

**Supply chain / licenses** (CI gate `cargo deny check`, see `deny.toml`):

```sh
cargo deny check            # RUSTSEC advisories + license compliance

# Regenerate the third-party notices for the bundle (before a release).
# Run all three from the REPOSITORY ROOT, with apps/desktop/node_modules
# installed. cargo-about keeps its binary behind the `cli` feature — without it,
# `cargo install cargo-about` compiles the library and installs nothing:
cargo install cargo-about --features cli
# All three parts are required, and each covers something the others cannot see:
#   1. cargo-about walks the Cargo graph.
#   2. the npm packages that are compiled into the shipped frontend.
#   3. the C libraries compiled in from vendored sources. cargo-about reads a
#      crate's declared `license`, which for a `-sys` crate describes the Rust
#      wrapper — libgit2-sys says "MIT OR Apache-2.0" while the libgit2 sources
#      it builds are GPL-2.0 with a linking exception, and libz-sys does the
#      same with zlib. Both land in the binary, so both licences must ship.
cargo about generate about.hbs > THIRD-PARTY-NOTICES.txt
node scripts/gen-npm-notices.mjs >> THIRD-PARTY-NOTICES.txt
node scripts/gen-vendored-notices.mjs >> THIRD-PARTY-NOTICES.txt
```

On the npm side, `apps/desktop/.npmrc` plays that role: `ignore-scripts=true`
switches off **all** lifecycle scripts (`preinstall`/`install`/`postinstall`/`prepare`)
of dependencies — the main way compromised npm packages get in. Exactly one
package in the lockfile still has an install script, `fsevents` (dev-only,
macOS-only, optional), and it does not need it: no exception is required,
build, lint, typecheck and tests all run without them. The scripts in
`package.json` (`dev`/`build`/`test`/…) are not affected.

Always use `npm ci` rather than `npm install` — it installs exactly the lockfile
and does not rewrite it. Also worthwhile before a release:

```sh
cd apps/desktop
npm audit signatures        # verify registry signatures/attestations of all packages
npm audit                   # known advisories
```

Should a package not work without its install script, do **not** water down
`.npmrc`; unlock that one package specifically and justify it there instead:
`npm rebuild <package> --ignore-scripts=false --foreground-scripts`.

**TDD is mandatory**: the failing test first, then the fix. Every
bug fix gets a regression test.

## Commits

- Conventional Commits (`feat:`, `fix:`, `chore:`, `docs:`, …), subject ≤ 72
  characters, written in English, imperative mood (“add”, not “added”). The
  scope in parentheses names the area: `feat(history): …`, `fix(sidecar): …`.
- Every destructive/history-rewriting action needs a safety net
  (confirmation plus a backup ref or undo entry) — that applies to code **and** reviews.

## Developer Certificate of Origin (DCO)

Contributions are made under the [Developer Certificate of Origin 1.1](https://developercertificate.org/).
With a `Signed-off-by` line you certify that you have the right to contribute
the change under the project's MIT license:

```sh
git commit -s          # appends "Signed-off-by: Name <mail>" automatically
git commit -s --amend  # retroactively, for the last commit
```

For `-s` to record the right name, `user.name` and `user.email` must be
set. The DCO keeps later licensing options open without contributors having
to sign a copyright assignment.

> CI does **not** enforce the line at the moment — it is a convention, not a
> gate. For contributions from outside the project, please include it consistently anyway.
