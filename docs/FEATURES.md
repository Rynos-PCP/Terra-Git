# terra-git — Feature overview

As of: 2026-08-31. Reference: GitHub Desktop parity research + community wish list
(evidence = GitHub issue numbers from `desktop/desktop`).

## GitHub Desktop parity

| Feature | GitHub Desktop | terra-git | Implementation |
|---|---|---|---|
| Open/add repository | ✅ | ✅ | folder dialog + recents |
| Clone repository (URL) | ✅ | ✅ | sidecar clone (system credentials, 1h timeout) |
| New repository (init) | ✅ | ✅ | init dialog |
| Changes list with status | ✅ | ✅ | staged/unstaged, conflict marking |
| File filter in Changes | ✅ (2025) | ✅ | filter field |
| Line/hunk selection in the diff | ✅ | ✅ | hunk buttons + clickable line numbers, direction-aware partial patch |
| Discard (file/hunk/all) | ✅ | ✅ | with confirmation; discard hunk in the diff |
| Choice on branch switch (carry along / leave here) | ✅ | ✅ | dialog before the switch; "carry along" takes the direct route as long as no file collides |
| Stash on branch switch | ✅ (implicit, 1 stash) | ✅ **better**: full stash list; changes left behind come back on their own when switching back | see community wishes |
| Ignore (.gitignore) from the UI | ✅ | ✅ | file + *.extension |
| Commit with summary/description | ✅ | ✅ | incl. 72-character warning |
| Co-authors | ✅ | ✅ | Co-authored-by trailer |
| Amend | ✅ | ✅ | checkbox |
| Undo commit | ✅ | ✅ | soft reset, button below the commit box |
| History with commit details | ✅ | ✅ | + multi-file diff |
| Cherry-pick | ✅ (drag & drop) | ✅ (context menu) | sidecar, conflict flow |
| Squash | ✅ (drag & drop) | ✅ (context menu, last N) | reset --soft + commit |
| Revert commit | ✅ | ✅ | context menu |
| Branch from commit | ✅ | ✅ | context menu + dialog |
| Check out commit (detached) | ✅ | ✅ | context menu |
| Branches: create/switch | ✅ | ✅ | branch menu |
| Branches: rename/delete | ✅ | ✅ | incl. merged check + force prompt |
| Merge into current branch | ✅ | ✅ | branch menu action |
| Rebase | ✅ | ✅ | branch menu action |
| Force push after rebase | ✅ | ✅ | push dropdown, `--force-with-lease` |
| Conflict resolution UI | ✅ | ✅ | banner + ours/theirs/mergetool per file, continue/abort |
| Pull/push/fetch | ✅ | ✅ | sidecar (credential helper), ahead/behind badges |
| Create PR | ✅ (GitHub) | ✅ **broader**: in-app via API (GitHub incl. GHES, GitLab self-hosted, Gitea/Forgejo incl. Codeberg) with draft + default target branch; "Create in browser…" in the PR/MR list as an alternative | dialog from PR/MR list, toolbar, palette |
| Syntax highlighting in the diff | ✅ | ✅ | highlight.js, 21 languages, theme-aware |
| Image diffs | ✅ | ✅ | before/after as data URLs |
| Open editor/shell/explorer | ✅ | ✅ | injection-free direct launches |
| Light/dark themes | ✅ | ✅ | design tokens, toggle |
| Git config settings | ✅ | ✅ | name/e-mail repo/global |
| Show tags | ✅ (badges) | ✅ | badges in history + management dialog |
| Repository list/recents | ✅ | ✅ | repo menu |
| Auto status refresh | ✅ | ✅ | file watcher (primary) + 60 s fallback poll, with race guards |
| Remote management (add/rename/URL/remove) | ❌ (#15797) | ✅ | "Manage remotes" modal, inline edit |
| Command palette | ❌ | ✅ | Ctrl+K, state-dependent commands |
| Amend warning "already pushed" | ✅ | ✅ | hint in the commit box (ahead=0 + upstream) |
| Commit message history | ❌ | ✅ | "History" menu, per repo (max. 30) |
| Auto backup before history rewrites | ❌ | ✅ | backup refs + restore UI ("Backups…") |
| Language switch EN/DE without restart | ❌ (English only) | ✅ | settings + palette, default = system language |
| Accessibility: font size/contrast/motion | 🟡 (OS zoom only) | ✅ | Settings → Accessibility, takes effect immediately |
| PR/MR list with CI status inline | ✅ (GitHub only) | ✅ **broader**: GitHub (incl. GHES) + GitLab (self-hosted!) + Gitea/Forgejo (incl. Codeberg) | PAT accounts, tokens in the OS keychain |
| Test pipeline locally (job + live log) | ❌ | ✅ | via gitlab-ci-local/act, detected rather than bundled |
| Progress display for remote ops | ✅ | ✅ | phase + percent (git --progress), thin bar below the toolbar |
| Clone: open immediately, then load data | ✅ (sidebar) | ✅ | init→open→fetch with clone overlay + progress bar |
| Multi-select discard | ✅ (checkboxes) | ✅ | Ctrl/Shift-click, Ctrl+A, right-click "Discard (N)" |

**Deliberately different/deferred:** sign-in to a provider account is by personal
access token — terra-git deliberately relies on system credentials and the OS
keychain instead of its own OAuth silo; OAuth joins the PAT flow later. Deeper
provider integration (issues, review comments, notifications) follows with the
provider layer, see [../ROADMAP.md](../ROADMAP.md). Drag & drop for
cherry-pick/squash/reorder: implemented as a context menu (functionally equivalent).

## Top 10 most-requested GitHub Desktop features → in terra-git

| # | Wish (evidence) | Status in terra-git |
|---|---|---|
| 1 | Linux support (#1525, 4,835 reactions) | ✅ shipped: AppImage, deb and rpm built on ubuntu-22.04 by the release workflow, no Chromium fork needed |
| 2 | Multiple accounts (#3707, 1,349) | ✅ by design: system credential helpers allow any number of hosts/accounts |
| 3 | GPG/SSH commit signing (#78, 340) | ✅ `commit.gpgsign=true` → commits go through system git (signing + hooks); toggle in settings |
| 4 | Commit graph (#9452, 292) | ✅ colored SVG lanes with merge curves in the history |
| 5 | Multiple windows (#3606, 219) | ✅ "New window" in the menu |
| 6 | Partial stash (#11531, 198) | ✅ `stash push` with file selection — per file from its ⋯ menu in the changes list |
| 7 | External diff/merge tool (#1765, 143) | ✅ "Open in merge tool" per conflicted file (`git mergetool`) |
| 8 | Worktrees (#907, 139) | ✅ "Worktrees…" dialog: list, create, remove, open |
| 9 | Stash list (#12699, 127) | ✅ full stash manager (list/apply/pop/drop) |
| 10 | History search (#7022, 97) | ✅ search across message/author/ID |
| +11 | GitLab/generic remotes (the fork's reason to exist) | ✅ core feature of terra-git |
| +12 | Tree view for changes (#2417 et al.) | ✅ switchable tree view |
| +13 | Submodules (#20921 et al.) | ✅ basics: list + `update --init --recursive` |
| +14 | Blame (#community) | ✅ blame view per file |
| +15 | Split diff (#10617) | ✅ switchable side-by-side view in the history |
| +16 | Bisect (#community) | ✅ bisect assistant: start from a commit, then Good/Bad/Skip/Abort in a banner with the remaining steps; the result is parsed locale-free (git ≤ 2.54 and ≥ 2.55 wording) |
| +17 | Sparse checkout (#community) | ✅ "Sparse checkout…" dialog: pick the top-level directories, apply or disable |

## Community wishes under scrutiny (reviewed 2026-07-08, updated 2026-08-31)

Seven frequently voiced wishes for Git clients, checked against the actual state:

| Wish | Finding | State / plan |
|---|---|---|
| Linux support | The architecture holds: Tauri; system integrations (explorer/terminal/editor) have Linux and macOS branches in `commands.rs`; frontend path joins separator-neutral since 2026-07-08. CI runs the full workspace (tg-app included) on Linux; the release workflow builds AppImage, deb and rpm | Open: no Linux E2E smoke test (the WebDriver harness is Windows/WebView2 only), bundles not code-signed |
| Performance on large repos | Measured, not hoped: `status` ~87 ms at 50k files (git2), sidecar fast path from 30k index entries, commit-graph write after fetch/clone — see [PERFORMANCE.md](PERFORMANCE.md); UI: virtualized lists, history paging, diff streaming with a cap | Largely done; the budgets are enforced as CI gates |
| Merge conflict presentation | Dedicated conflict editor (ours/theirs/base side by side, segment-wise adoption), conflict banner with continue/abort, mergetool handoff per file | Present; expansion (v2): syntax highlighting in the conflict editor |
| Integrated terminal | Deliberate non-goal: GUI focus, no ambition to replace the terminal; "Open terminal here" instead | Not planned |
| Plugin system | Deliberate non-goal: dilutes focus and the resource target | Not planned |
| Multiple Git providers | Remote ops are provider-neutral (system git sidecar + the system's credential helpers → any host, any number of accounts); PR/MR: GitHub, GitLab and, since 2026-07-18, Gitea/Forgejo/Codeberg as a full API provider (list, CI status, creation) | Bitbucket stays excluded by project decision; deeper provider APIs (issues, review comments, notifications) follow with the provider layer — see [../ROADMAP.md](../ROADMAP.md) |
| Commit history & visualization | Colored lane graph with merge curves, history search (message/author/ID), tag badges, branch/HEAD ref badges, date grouping, split diff, infinite scroll | Complete for v1 |

## Design system (overhaul 2026-07-06)

- Semantic CSS tokens, light + dark via `data-theme`, toggle in the toolbar
- SVG icon set (stroke, 16px, currentColor) instead of emojis
- 4px grid (spacing/radii), GitHub status colors for file kinds
- Commit avatars (initials, deterministic colors), tag badges
- Diff: alpha tints over syntax colors, hunk actions on hover, focus rings
