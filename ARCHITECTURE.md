# terra-git — Architecture

terra-git was designed in detail before a line of it was written. This file records
how the code is actually put together, and where it **deliberately** departs from that
original design — so the reasoning behind each choice is on the record instead of
buried in silent code. New deviations belong here.

## Actual structure

```
terra-git/
├─ crates/
│  ├─ tg-domain        # I/O-free, serializable domain types (wire contract)
│  ├─ tg-git-engine    # git2 (local) + system-git sidecar (remote/exotic)
│  ├─ tg-auth          # OS keychain (keyring 3), git-credential helper
│  └─ tg-providers     # GitHub/GitLab/Gitea REST (reqwest)
└─ apps/desktop
   ├─ src-tauri (tg-app)  # Tauri command layer + orchestration
   └─ src                 # Svelte 5 frontend (runes)
```

The crate graph is acyclic and points strictly downwards (tg-domain at the very bottom).

Inside `tg-git-engine`, besides the git2 engine (`lib.rs`, `ops.rs`) and the
system-git sidecar (`sidecar.rs`):

| Module | What it does |
|---|---|
| `discarded.rs` | The discard store: a copy of every discarded file (whole files and single hunks), restore and redo, the storage limit |
| `repo_commands.rs` | What a repository's own configuration may make git run — see "Foreign repositories" below |
| `lfs.rs` | Git LFS: detection, staging through the clean filter, pointer-aware diffs, tracking, downloads |
| `remedies.rs` | The backend half of the error toast's ways out (remove a lock, trust a folder, fetch the full history …) |
| `ssh.rs` | The SSH key manager and the host-key trust dialog (TOFU) |
| `status_fast.rs` | The system-git `status` fast path |

Error codes are stable strings the frontend maps to texts and ways out:
`errorCodes.ts` is the one list (a Rust test checks the backend emits nothing
else), `errorRegistry.ts` the titles and buttons per code.

## Deliberate deviations (as of 1.1.0)

| Area | Original design | What ships | Why |
|---|---|---|---|
| **Git read engine** | gix (gitoxide) for the read fast path | git2 + system-git fast path (`status --porcelain=v2` from 30k index entries, and always in a repository with filter drivers such as Git LFS — only git runs the clean filter) | A gix read path would introduce a third view of the object model (divergence risk, cf. the CRLF incident). Deferred until a budget forces it; the numbers behind the decision are in [docs/PERFORMANCE.md](docs/PERFORMANCE.md). |
| **Provider APIs** | octocrab + gitlab crate, incl. GraphQL | lean reqwest/rustls client | Only a few REST endpoints needed; saves two large deps; the system trust store covers enterprise CAs, `insecure_tls` covers self-signed. |
| **Diff/code view** | CodeMirror 6 + `imara-diff`/`similar` in a `tg-diff` crate | highlight.js + git2 line diff + a word diff in the frontend (`wordDiff.ts`) | No `tg-diff` crate. The changed words inside a changed line are marked by a token LCS in TypeScript, within a time budget per view, so a huge or crafted diff stays responsive. |
| **Virtualization** | TanStack Virtual v3 | own `VirtualList.svelte` | Fixed row heights make the dependency unnecessary. |
| **Sidecar model** | “long-lived worker, not forked per action” | **process per action** (`git …` per op) | An honest deviation. The real hot-path reads (status/log/diff < 30k) run in-process through git2; the sidecar is the exception path (fetch/push/merge/rebase, commits in repositories with hooks or signing, staging of filtered paths, Git LFS — a resident worker brings ~zero benefit there). A persistent worker is backlog, not a v1 blocker. |
| **status budget “0 new processes”** | 0 new processes in the read fast path | git2 in-process below 30k index entries; **≥ 30k** forks the system-git `status` (faster than single-threaded libgit2) | The “0 processes” budget is therefore only fully met with the **gix read fast path** (deferred, see the first row). Until then, deliberately: the git spawn is faster than libgit2 on large repos. What was reduced is the spawn **frequency** (only ONE git2 open per refresh instead of two; app poll 30 s → 60 s, since the file watcher incl. PollWatcher fallback is the primary route). |
| **Engine abstraction** | `GitEngine` trait for later gix substitution | The trait exists but is **nowhere** used polymorphically (concrete `Git2Engine` everywhere) | Documentary. Either get serious later (`&dyn GitEngineExt` + DI for mocks) or honestly collapse it to “one impl”. |
| **IPC layer** | `tg-ipc` crate with DTOs, decoupled from tg-domain | tg-domain types are the wire contract directly (camelCase serde) | No DTO intermediate layer; `api.ts` mirrors the types by hand. Field renames break the contract silently. Backlog: DTO codegen (ts-rs/specta). |
| **Persistence** | SQLite (`settings`/`repositories`/`cache_*`) in a `tg-persistence` crate | JSON files (`providers.json`, `recent_repos.json`) + `localStorage`; per repository the discard store (`.git/terra-git/discarded/`) and backup refs (`refs/terra-git/backup/`) | No `tg-persistence` crate, no SQLite. Sufficient for the v1 scope; cache tables (e.g. CI status) are missing. The discard store is a folder rather than git objects so its storage limit is exact: deleting a copy frees the space at once. |
| **Test runner** | cargo-nextest | `cargo test` | Minor; nextest adds little value here. |

## Not yet built

OAuth/PKCE sign-in (needs a registered client_id per instance), a crash-report
server, auto-update plus code signing/notarization, memory-leak gates in CI.
Signing, auto-update and OAuth are sequenced in [ROADMAP.md](ROADMAP.md); the
crash-report server, the memory-leak gates and the gix read path are backlog
with no place in that order yet.

## Security baseline

deny-by-default capabilities, strict CSP (`script-src 'self'`), all git sidecar calls
with `-c protocol.ext.allow=never` + argument/URL validation, credential helper only over
`https`, host-bound `sslVerify=false` only for hosts explicitly marked opt-in,
pipeline job names against a strict allowlist. Supply chain: `cargo-deny` (advisories/
licenses), third-party notices in three parts — `cargo-about` for the Rust crates,
`scripts/gen-npm-notices.mjs` for the npm packages that ship in the bundle and
`scripts/gen-vendored-notices.mjs` for the C libraries compiled in from vendored
sources (libgit2, zlib).

The CSP additionally sets `base-uri 'self'`, `form-action 'self'` and `object-src 'none'` —
these three do **not** inherit from `default-src` and therefore have to be set explicitly.
`style-src` deliberately keeps `'unsafe-inline'`: Svelte generates inline styles for `style:`
directives and transitions, and a nonce/hash scheme would not be practical here. That is
acceptable because `script-src` stays locked down (no inline scripts, no external sources),
which closes the main vector.

### Foreign repositories: what is hardened, what residual risk remains

From git's point of view, a repo-local `.git/config` is executable code: `mergetool.<t>.cmd`,
`filter.<d>.clean/smudge`, `diff.<d>.command`, `merge.<d>.driver` and `core.fsmonitor`
start commands. It is **not** transferred when cloning — the only dangerous case is a
repository whose `.git` directory somebody else supplied (unpacked archive,
network share, USB stick).

Hardened is the path that terra-git deliberately starts as an external command: `open_mergetool`
resolves the tool name itself, checks it against `[A-Za-z0-9._+-]` (git sources
`<exec-path>/mergetools/<name>` as a sh script — a name containing `/` or `..` would be a
path traversal into a foreign file) and passes it as `--tool=<name>`, which bypasses git's own
config resolution. `mergetool.<name>.cmd` and `mergetool.<name>.path` are forced via `-c`
to the value from the global or system configuration (empty if none is set
there); an empty value leaves git's built-in tool definitions untouched. Repo-local
values of these keys are therefore never executed.

Hardened as well are the remote operations: **fetch, pull, push and clone refuse to start git
at all when the repository's own config defines a program for the transfer** — a
`credential.helper` (also URL-scoped, `credential.<url>.helper`), `core.askPass`,
`core.gitProxy`, `remote.<n>.uploadpack`/`receivepack` of a remote that is a local path
(on ssh/https they name a program on the server), or a `core.sshCommand` that is more than
a plain `ssh` call. `credential.helper` is multi-valued — the app's own
`-c credential.helper=…` is only APPENDED to the list, so a repo-local
`helper = "!<command>"` would still run (first, even) on the first 401 from the server. That
would be arbitrary code execution from a single Fetch click on a foreign `.git` directory.
The check reads the repo-local and the worktree level, files they include among them.
Global and system settings (Windows Credential Manager, osxkeychain, libsecret, your own
ssh command) are unaffected; the rejection happens before git is started, following the
same pattern `open_mergetool` uses for repo-local tool definitions. A plain per-repo ssh
call (`ssh -i ~/.ssh/work_key -o IdentitiesOnly=yes`) stays allowed: the program has to be
`ssh` from the `PATH`, with identity, port, user and harmless `-o` options only —
`ProxyCommand`, `LocalCommand`, `-F` and the like disqualify it.

**Opening a repository runs nothing it defines** (`tg_git_engine::repo_commands`). Calls the
app makes on its own — status refresh, history, commit graph, the status accelerators, the
automatic fetch — run with `core.hooksPath` pointing at a file (so no hook can exist) and
with every repo-local filter command emptied, except the exact values `git lfs install
--local` writes. A repo-local `core.fsmonitor` that names a program is switched off for
every call; the boolean (git's builtin daemon, which terra-git enables itself) stays. The
overrides travel as `GIT_CONFIG_COUNT`/`GIT_CONFIG_KEY_n`/`GIT_CONFIG_VALUE_n`, which beat
every config file and — unlike `-c key=value` — cannot be confused by a driver name
containing `=`. Without this, a supplied folder with `filter=` in its `.gitattributes`
(which routes the status through system git, see the status row above) and a repo-local
`core.fsmonitor` command would run that command the moment the repository was opened.

Residual risk (deliberate, no trust prompt): **what you start behaves exactly as with git
itself.** A commit runs the repository's hooks (`pre-commit`, `commit-msg`, …, also from a
repo-local `core.hooksPath` — husky and pre-commit depend on it), a merge, checkout or
stash runs its filter and merge drivers, a push runs `pre-push`, and signing runs
`gpg.program`. git offers **no** switch that ignores only the repo-local config level —
`GIT_CONFIG_NOSYSTEM` and `GIT_CONFIG_GLOBAL=/dev/null` work exactly the other way round —
so neutralizing these for user actions would break every repository that legitimately
configures them. The credential helper the app registers is inherited by hooks during
pull and push (they could ask it for the token); hooks are code execution anyway.
`core.pager` is **not** a vector: git starts the pager only on a TTY, and the sidecar
output is always attached to a pipe. `GIT_CONFIG_NOSYSTEM` is deliberately **not** set: the
system config is not attacker-controlled and on Windows carries `http.sslBackend=schannel`,
`filter.lfs.*` and `core.autocrlf` — switching it off would break the TLS backend, Git LFS
and line endings.

The *Trust folder* way out (git's "dubious ownership" error) adds exactly the repository's
root to the global `safe.directory`, never a wildcard; its confirmation says that trusting
a folder lets its configuration and hooks run programs. Only confirm it for folders whose
origin you know.
