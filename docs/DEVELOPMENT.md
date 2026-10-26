# terra-git — Development

Desktop Git client built on **Tauri 2.x** (Rust core) + **Svelte 5** (frontend).
Architecture and the deliberate deviations from the original design:
[../ARCHITECTURE.md](../ARCHITECTURE.md). Planned work: [../ROADMAP.md](../ROADMAP.md).

## Prerequisites

| Tool | Version | Purpose |
|---|---|---|
| Rust (rustup, MSVC toolchain) | stable | core, engine, Tauri app |
| Node.js + npm | 22.12 or newer (pinned in `.nvmrc`, same as CI; Vitest 5 and ESLint 10 need it) | frontend build (Vite), tests |
| Visual Studio Build Tools / VS with C++ workload | 2022+ | MSVC linker, libgit2 build |
| Git | ≥ 2.40 | sidecar for fetch/pull/push, merges, hooks, LFS |
| Git LFS (`git-lfs`) | any current | the LFS round trips in `lfs_tests.rs` (they are skipped without it; CI installs it) |
| WebView2 runtime | preinstalled (Win 11) | UI rendering |

> The table describes the Windows dev environment. For **Linux/macOS bundles** see
> [Release build (bundles)](#release-build-bundles) — that is where the system
> dependencies (webkit2gtk, patchelf …) are listed.

## Project structure

```
terra-git/
├─ Cargo.toml                  # workspace (tg-* crates)
├─ crates/
│  ├─ tg-domain/               # domain types (serde, camelCase)
│  ├─ tg-git-engine/           # GitEngine trait, Git2Engine, sidecar
│  │  └─ tests/                # integration tests against fixture repos
│  ├─ tg-auth/                 # token store (OS keychain via keyring)
│  └─ tg-providers/            # GitHub/GitLab/Gitea REST: PR/MR list + CI status
│     └─ tests/                # integration tests against a local HTTP stub
└─ apps/desktop/
   ├─ src/                     # Svelte 5 frontend (runes)
   │  ├─ lib/api.ts            # typed IPC layer
   │  ├─ lib/state.svelte.ts   # central UI state + actions
   │  ├─ lib/i18n.svelte.ts    # language layer (messages/de.ts + en.ts)
   │  └─ lib/components/       # Toolbar, Panels, DiffView, …
   └─ src-tauri/               # tg-app: Tauri commands, capabilities
```

**Architecture rules** (binding):

- `tg-domain` and `tg-git-engine` are **Tauri-free** (pure libs, `thiserror`).
- Local Git operations run **in-process** through git2 (vendored libgit2) —
  except where only git behaves like git: merges and rebases, commits in a
  repository with hooks or signing, staging and status in a repository with
  filter drivers (Git LFS). Those go through the sidecar.
- Remote operations (fetch/pull/push) go through the **system git sidecar** so
  that the system's credential helpers/SSH agent apply (self-hosted GitLab!).
  Every sidecar call starts ssh non-interactively (`BatchMode`): an unknown host
  ends in the trust dialog instead of a question on an invisible console.
- Calls the app makes on its own (status, history, the automatic fetch) are
  wrapped in `repo_commands::on_its_own`: no hooks, no repo-local filter
  commands. New background calls belong in there.
- Every Tauri command is `async` and wraps engine calls in `spawn_blocking`.
- Errors cross the IPC boundary only as `{ code, message, detail? }` — a stable
  code, a short message and, for failed git calls, git's cleaned-up output.
  A **new error code** needs an entry in `errorCodes.ts`, a title and a text in
  both catalogs and, if it has ways out, a registry entry (`errorRegistry.ts`);
  `errorCodes.test.ts` and a Rust test in `src-tauri/src/error.rs` fail
  otherwise.
- Provider APIs (PR/MR list, CI status) go through `tg-providers`
  (reqwest/rustls, system trust store); PATs live **only** in the OS keychain
  (`tg-auth`), account metadata in `providers.json` in the app config dir.
  On Linux the build needs `libdbus-1-dev` for this (Secret Service).
- The app doubles as its own git credential helper: for remote ops the sidecar
  appends `-c credential.helper=!'<exe>' __credential` (the leading `!` makes
  git run the value as a shell command); headless mode
  (`tg-app __credential get`) answers requests from providers.json + keychain.
  System helpers run first.
- SSH keys can be generated/managed in the settings (`tg-git-engine::ssh`,
  OpenSSH CLIs); unknown host keys are confirmed through a TOFU dialog on the
  first fetch, pull, push or clone.

## Developing

```bash
cd apps/desktop
npm ci               # once (exactly the lockfile, without install scripts)
npm run tauri dev    # app with hot reload (Vite + cargo)
```

## Checking (before every commit)

```bash
cargo fmt --all -- --check         # CI gate
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace             # ~460 unit and integration tests
cd apps/desktop
npm run lint                       # ESLint (CI gate)
npm run format:check               # Prettier (CI gate; `npm run format` rewrites)
npm run check                      # svelte-check (TS)
npm test                           # Vitest (pure frontend logic: lib/*.test.ts)
```

The feature set (GitHub Desktop parity + community wishes) is documented in
[FEATURES.md](FEATURES.md). The Git logic lives entirely in `tg-git-engine`
(trait `GitEngine` for core ops, `GitEngineExt` for the extended operations)
and is covered by the integration tests.

## Internationalization (i18n)

UI texts go through `src/lib/i18n.svelte.ts` (`t(key, params?)`, reactive —
a language switch takes effect immediately). Catalogs: `src/lib/messages/de.ts`
(reference, defines `MessageKey`) and `en.ts` (the type enforces completeness;
additionally a parity test in `i18n.test.ts`).

**Rules for new UI strings:**

- No German/English literal text in components — always `t("area.slug")`
  and add the key to **both** catalogs.
- Interpolation via `{param}`: `t("key", { n })`. Plurals via separate keys
  (`…One`/`…Many`).
- Never call `t()` at module top level (the result would be frozen) — only in
  the template, in `$derived` or in functions.
- Do not translate: git commands, stable error codes, proper names.
- Language selection: Settings → Language, or the palette; default = system
  language, persisted in localStorage (`terra-git-lang`). Backend/engine messages
  are English; the UI shows catalog texts keyed by the stable error code.

## E2E test against the real app (WebDriver)

`apps/desktop/e2e/` boots the **real app** (WebView2) through `tauri-driver` —
with a minimal, dependency-free WebDriver client built on Node built-ins (a
deliberate deviation from the original WebdriverIO design: we only need
session/find/exec, which saves the entire wdio stack).

One-time prerequisites:

```powershell
cargo install tauri-driver
# msedgedriver matching the WebView2 version (registry: EdgeUpdate pv), placed at
# apps/desktop/e2e/drivers/msedgedriver.exe:
#   https://msedgedriver.microsoft.com/<version>/edgedriver_win64.zip
```

Running (does not build on its own — build app + dist first):

```powershell
cd apps/desktop
npm run build
cargo build -p tg-app --features custom-protocol
npm run e2e     # node --test e2e/ — briefly opens a real app window
```

The app under test runs with a config directory of its own
(`TERRA_GIT_CONFIG_DIR`, set by `e2e/helpers.mjs`), so the recent list and
the provider accounts of your real profile are never touched — not even by a
test that fails half-way. (WebView storage — UI preferences — is still the
real one.)

- `smoke.test.mjs` — app startup, the rendered shell and the Ctrl+K palette.
- `flows.test.mjs` — the destructive flows added in 1.1: rebase conflict → workshop → continue / abort; pull with local
  changes → stash conflict → workshop → done (the stash goes only then);
  push rejected → pull & merge → push; undo the very first commit; remote
  branch deleted → delete the merged local branch from the error message.
  Fixture repositories are built with the git CLI in the temp folder; the
  app is driven through `data-testid` hooks, independent of the UI language.

Two environment variables, set only by `e2e/helpers.mjs` and honoured only
by debug builds (a release build ignores both), make the app
testable: `TERRA_GIT_CONFIG_DIR` (see above) and
`TERRA_GIT_E2E_ACCEPT_DIALOGS`, with which the app answers its own
confirmation questions with OK (`src/lib/dialogs.ts`) and records them in
`window.__e2eDialogs`. A native dialog is out of WebDriver's reach, and
Tauri locks its IPC objects (`__TAURI_INTERNALS__.invoke` is read-only), so
patching them from the page does not work. The test files run one after the
other (`--test-concurrency=1`): each starts its own `tauri-driver` on port
4444.

Add new tests as `e2e/*.test.mjs`, using `e2e/helpers.mjs`.

## UI smoke test without Tauri (headless)

`mock.html` + `src/main-mock.ts` provide a Tauri-free entry point with demo
data (IPC stub via `__TAURI_INTERNALS__`). The header comment of
`src/main-mock.ts` is the authoritative list of scenes; pick one with
`?scene=…`, the language with `&lang=de|en`, the theme with `&theme=dark|light`.

```bash
cd apps/desktop && npx vite --port 1421   # dev server
# Screenshot (Windows, Edge headless):
msedge --headless=new --screenshot=out.png --window-size=1280,800 \
  "http://localhost:1421/mock.html?scene=palette"
```

All handbook screenshots under `docs/images/` come from the same harness:
`node docs/gen-screenshots.mjs` renders the documented subset of scenes and
expects the dev server on port 1420 (`npm run dev`). `EDGE=` and `MOCK_BASE=`
override the Windows/Edge defaults — set `EDGE` on macOS/Linux.

The mock entry point is not bundled (Vite only builds `index.html`).

## Release build (bundles)

Produces installable packages in the workspace target (repo root
`target/release/bundle/`). **v1 bundles are unsigned** — signing/notarization
and auto-update follow as a separate step (see [../ROADMAP.md](../ROADMAP.md)).
Before a release, regenerate `THIRD-PARTY-NOTICES.txt`: `bundle.resources` in
`tauri.conf.json` embeds it into every bundle, and it has three parts
(cargo-about for the Rust graph, `scripts/gen-npm-notices.mjs` for the npm
packages in the shipped frontend, `scripts/gen-vendored-notices.mjs` for the C
libraries compiled in from vendored sources). Commands:
[../CONTRIBUTING.md](../CONTRIBUTING.md), section “Supply chain / licenses”.

**Windows** (standard dev environment):

```bash
cd apps/desktop
npm ci
npm run tauri build
#   target/release/tg-app.exe          (portable)
#   target/release/bundle/nsis/*.exe   (installer)
#   target/release/bundle/msi/*.msi
```

**Linux** (build natively, e.g. in a VM — cross-building from Windows does not work):

```bash
# 1) System dependencies (Debian/Ubuntu 22.04+):
sudo apt-get update && sudo apt-get install -y \
  libwebkit2gtk-4.1-dev libjavascriptcoregtk-4.1-dev libsoup-3.0-dev \
  libgtk-3-dev librsvg2-dev libssl-dev libayatana-appindicator3-dev \
  libdbus-1-dev pkg-config patchelf libfuse2 build-essential curl wget file
#    (Rust via rustup + Node ≥ 22.12 assumed.)

# 2) Build (network access required: the AppImage downloads linuxdeploy at
#    build time — it is itself an AppImage and needs FUSE 2, hence libfuse2):
cd apps/desktop
npm ci
npm run tauri build
#   target/release/bundle/appimage/*.AppImage
#   target/release/bundle/deb/*.deb
#   target/release/bundle/rpm/*.rpm
```

If the AppImage starts with nothing but a white window (some VMs/GPUs without DMABUF):

```bash
WEBKIT_DISABLE_DMABUF_RENDERER=1 ./terra-git_*.AppImage
```

**macOS:** `npm run tauri build` → `bundle/dmg/*.dmg` (Apple Silicon; Universal follows).

**CI:** [`.github/workflows/release.yml`](../.github/workflows/release.yml) builds all
three OSes — on **tag `v*`** as a draft GitHub release, or **manually**
(`workflow_dispatch`, Actions tab) as downloadable artifacts without a tag.

## Troubleshooting

**`failed to run 'cargo metadata' … program not found`** — Rust is missing or the
terminal does not know the PATH yet.

1. Check: `cargo --version`. If that fails even though Rust is installed, the
   terminal (or VS Code) was started before the installation →
   **close VS Code completely and reopen it** (a new terminal tab is not
   enough, VS Code inherits its old environment).
2. Repair the running session only:
   `$env:Path += ";$env:USERPROFILE\.cargo\bin"`
3. Rust is missing entirely: <https://rustup.rs> → `rustup-init.exe -y` installs
   the stable toolchain (MSVC) pinned in `rust-toolchain.toml` per user.

**The first build takes several minutes** — normal: vendored libgit2 gets
compiled, the release build runs with LTO. Subsequent builds are incremental.

**`tauri dev`: port 1420 in use** — kill the old Vite/`tauri dev` process
(the port is fixed via `strictPort`).

## Conventions

- Commits: Conventional Commits (`feat:`, `fix:`, `test:`, `docs:`, …), English.
- One commit per work step/checkpoint.
- Crate naming scheme: `tg-*` (binding).
- UI texts: English by default, German with a German system language (catalogs
  in `src/lib/messages/`). Error codes: English/stable (`not_a_repository`, …).
