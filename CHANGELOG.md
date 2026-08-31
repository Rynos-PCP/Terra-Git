# Changelog

All notable changes to terra-git are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), the versioning
[Semantic Versioning](https://semver.org/).

## [Unreleased]

Nothing yet — changes land here until the next tag.

## [1.0.0] — 2026-08-31

First public version: a fast, resource-friendly Git desktop client
(Tauri 2 + Rust + Svelte 5) with full GitHub Desktop parity and
first-class support for self-hosted GitLab.

### Added

- **Working:** change list with status and filter, staging down to
  hunk and line level, discarding with confirmation, change overview,
  tree and list view, image diffs, syntax highlighting (21 languages).
- **Committing:** summary/description with 72-character warning, co-authors,
  amend with a warning for already-pushed commits, per-repository message
  history, GPG/SSH signing through system `git`.
- **History:** whole-repository graph across all branches with tag and
  ref badges, search across message, author and ID, switchable side-by-side
  diff, blame, bisect assistant.
- **Rewriting history:** cherry-pick, revert, squash, interactive rebase as a
  UI, multi-step undo/redo — and before every intervention an automatic
  backup (backup refs) with its own restore view.
- **Branches & remotes:** create, switch, rename, delete (with
  merged check), merge, rebase, force-push with `--force-with-lease` and
  confirmation, full remote management.
- **Switching branches:** every path that checks something out — the branch
  menu, “Check out here” in the history, “Cherry-pick onto branch” — asks the
  same question about uncommitted changes, and names the changes that stay
  behind on the target branch instead of leaving them unmentioned.
- **Conflicts:** a dedicated conflict workbench that labels both sides with branch
  names instead of ours/theirs, segment-by-segment take-over in the editor, handoff to
  the external merge tool per file.
- **Stash:** full manager including partial stash; changes left behind
  when switching branches come back by themselves.
- **Providers:** create and list pull requests and merge requests in the app —
  GitHub (incl. GHES), GitLab (incl. self-hosted, subpath, custom CA) and
  Gitea/Forgejo (incl. Codeberg), CI status inline; the list additionally
  offers “Create in browser…”, which opens the host's own form.
  Tokens exclusively in the OS keychain.
- **Local pipeline cockpit:** run `.gitlab-ci.yml`/Actions jobs locally
  (via `gitlab-ci-local`/`act`, detected rather than bundled), with job graph,
  status chips and live log.
- **Large repositories:** in-process Git with a sidecar fast path from 30,000
  index entries, commit-graph maintenance, file watcher, virtualized lists,
  diff streaming.
- **Usability:** command palette (Ctrl+K), worktrees, submodules,
  sparse checkout, SSH key manager with TOFU, multiple windows, light and
  dark theme, accessibility settings (font size, contrast,
  motion), German and English without a restart.

### Known limitations

- The installers are **not signed** — Windows SmartScreen and macOS
  Gatekeeper warn about an “unknown publisher”.
- **No auto-update**; updating is done manually.
- Hunk staging has a known race condition: reading the file, showing its hunks
  and applying your choice are three separate steps — if another program
  rewrites the file in between, the hunk that gets staged or discarded can
  differ from the one you saw.
- Sign-in only via personal access token, not yet via OAuth.

[Unreleased]: https://github.com/Rynos-PCP/Terra-Git/compare/v1.0.0...HEAD
[1.0.0]: https://github.com/Rynos-PCP/Terra-Git/releases/tag/v1.0.0
