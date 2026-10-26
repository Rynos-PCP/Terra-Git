# Changelog

All notable changes to terra-git are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), the versioning
[Semantic Versioning](https://semver.org/).

## [Unreleased]

## [1.1.0] — 2026-10-26

### Added

- **Discarding can be undone.** Before every discard — whole files, a
  multi-selection or a single change block — terra-git keeps a copy of what is
  about to go, untracked files included. `Ctrl+Z` brings it back, and the
  Backups dialog lists every discard under *Discarded changes* with Restore
  and Delete. A restore never overwrites files changed since without asking,
  and if you confirm, their current state is kept as a copy too. Every
  discard copies all of its files, and copies are never deleted
  automatically. They live in their own folder inside `.git`, within a
  storage limit you set in the settings (100 MB to 5 GB per repository,
  500 MB by default; the Backups dialog shows how much is in use). When a
  copy no longer fits, the discard is refused with a notice that names the
  sizes and offers **Manage backups** or **Raise limit** — nothing is ever
  discarded without a copy.

- **The very first commit can be changed and taken back.** *Undo last
  commit* on it takes the branch back to "no commit yet" with every file
  staged (undoable with `Ctrl+Z`); squash, interactive rebase, reword and the
  commit workshop now reach down to it (`git rebase --root`); reverting it
  asks first.

- **Every conflict ends up in the conflict workshop.** It opens by itself
  when a merge, pull, cherry-pick, revert, rebase or a commit-workshop rewrite
  stops on conflicts; *Resolve* on a conflicted row opens it on that file
  (the separate conflict dialog is gone). Conflicts from putting a stash back
  are handled there too, with *Done* (removes a popped stash only once
  everything is resolved) and *Give up conflicts* (back to the last commit,
  with a copy under Backups; the stash stays). The base version is shown as a
  third column whenever the two sides have one.

- **Pull, push and merge no longer end in a dead end.** Diverged branches
  ask *Merge* or *Rebase* (optionally remembered per repository; the default
  is a setting, which a `pull.rebase`/`pull.ff` in git overrides — your answer
  in the dialog overrides both). Local
  changes in the way can be brought along through a stash — conflicts on the
  way open the conflict workshop, and the changes come back once the
  operation is finished. A rejected push offers *Pull & merge, then push*,
  *Rebase onto the remote, then push* or a force push with lease; after
  conflicts the push is offered once they are resolved.

- **Update the current branch from any other branch.** *Update current
  branch from…* (branch menu, palette) lists every branch with the number of
  commits it would bring in, preselects the one used last for this branch
  (else the default branch), fetches first when the remote state is wanted,
  and merges or rebases. The branch menu shows how far the branch is behind
  that source. *Rebase onto* now works with remote branches too.

- **Pull and fetch from any remote; publish a branch without its commits.**
  The pull button has a dropdown like push: pull from another remote, fetch
  all remotes or a single one. A branch that is not on the remote yet can be
  published with its commits or as the branch only (at its fork point, with
  upstream) — also right when creating it. The push tooltip names the actual
  target remote.

- **Reflog, reset and compare.** *Reflog…* (`…` menu, palette) lists every
  position HEAD and each local branch had, with terra-git's own backups on
  top — lost commits can be shown, branched from, checked out or reset to.
  *Reset current branch to here…* offers soft, mixed and hard in plain
  words, warns when commits already on the remote leave the branch, writes
  a backup first, copies uncommitted work before a hard reset, and is one
  undo step. Two commits (`Ctrl`+click), a commit and HEAD, a branch and the
  current one (*Compare with the current branch* in the branch menu) or any two
  branches (*Compare branches…* in the palette) open a compare view: the
  changes since the fork point and the commits only one side has, with
  merge, update-from and PR/MR right there.

- **Git LFS help.** A banner says when a repository uses Git LFS but
  `git-lfs` is not installed (with a link to install it), and when LFS files
  were not downloaded — **Download LFS files** runs `git lfs pull` with
  progress. Errors caused by a missing `git-lfs` get their own message.
  *Track with Git LFS* (a file or all files with its extension) in the file
  menu writes and stages `.gitattributes` and stages the file again as a
  pointer.

- **Changed words are highlighted.** In a changed line, the diff now marks
  the words that actually differ from the line it replaces; the conflict
  workshop marks what each side changed against the base.

### Changed

- **Error messages are bounded and say where they come from.** The heading
  reads *REPO : code — title*, the explanation follows in your language, and
  git's own output sits in a scrollable box of its own instead of stretching
  the message past the top of the window. **Copy details** and **Open log
  folder** sit in the heading; long output shows its start and its end, the
  full text goes to the log. Every error code now has a title and a text in English and
  German, and errors that have a way out offer it as a button (manage
  accounts, SSH keys, remotes, backups).
- **Errors are recognised with a non-English git as well.** Fetch, pull,
  push, clone and commits run with git's English messages, so a German git no
  longer turns every rejected push or failed sign-in into a generic "git
  command failed", or a missing identity into a failed hook — and the progress
  bar names the phase again.

- **Common errors are named and offer a way out.** A lock file left by
  another git process, a folder owned by another user, a missing git
  identity, an untrusted certificate, unrelated histories, a deleted remote
  branch, a push from a shallow clone or a detached HEAD, a path too long
  for Windows, a failed signature and an SSH host key type the client no
  longer accepts each get their own title and explanation instead of *git
  command failed*. A failed fetch, pull or push offers **Try again**, and
  most of them a fix right in the message: remove the lock file, trust the
  folder, enable long paths, publish again, remove the upstream or delete a
  merged local branch whose remote branch is gone, fetch the full history,
  or merge unrelated histories anyway. When local changes are in the way,
  the dialog lists the files git named.

- **Commits run your git hooks.** In a repository with commit hooks (or
  `core.hooksPath`, as husky sets it), terra-git commits through git, so
  `pre-commit`, `commit-msg` & co. run as on the command line. A hook that
  refuses the commit — or a `pre-push` hook that refuses the push — is
  reported with its output and **Try again**; a
  failed signature offers **Commit without a signature** for this once.

### Fixed

- **Fetch, pull, push and clone no longer hang on an unknown SSH host.** ssh
  asked "Are you sure you want to continue connecting?" on a console nobody
  could see, and the operation waited until the one-hour timeout. terra-git
  now starts ssh so that it never asks: an unknown host leads to the trust
  dialog at once, a locked key to *SSH authentication failed*, an
  unreachable server to a network error after 30 seconds. The fingerprint
  for the dialog now comes from the same ssh git uses — the `ssh-keyscan`
  shipped with Windows fails against OpenSSH 10 servers
  ("unsupported KEX method"), so no dialog appeared at all. Your own SSH
  program (`core.sshCommand`, `GIT_SSH`) is left alone.

- **Git LFS files are committed as LFS pointers again.** terra-git staged
  files through libgit2, which runs no filter drivers: in an LFS repository
  the whole file went into the commit instead of the pointer. Files behind
  a filter (`filter=lfs` or any other) are now staged through git, the
  status of such repositories is read through git, and switching branches,
  checking out a commit, discarding, putting a stash back, resetting hard
  and restoring a backup write the real file content instead of the
  pointer text. The diff of an LFS file no longer shows the pointer text:
  it says *Git LFS file* and compares the sizes of the real content.

- **Huge change sets no longer freeze the app.** With ~25,000 changed files,
  the status, staging and the line balance spent minutes in libgit2's rename
  detection, which compares every new with every deleted file. Rename
  detection now has a budget, as in git; beyond it, additions and deletions
  are shown as such. The status of 24,780 changes takes 0.3 s instead of
  minutes. Staging or discarding 200 files or more shows a file counter in
  the toolbar and can be cancelled (a cancelled discard discards nothing),
  and the line balance no longer runs several times in parallel.

- **A stash is no longer lost when re-applying it conflicts.** libgit2 reports
  a conflicted stash apply as a success, and its pop then dropped the stash.
  This hit "Bring changes along" when switching branches, the automatic
  restore of changes left behind, and Pop in the stash list: the app said
  "changes brought over" while the stash was gone and the files were full of
  conflict markers. Now the stash is dropped only after a conflict-free apply;
  otherwise it stays in the list and the message says how many files are in
  conflict.

- **Discarding, resetting and restoring lose nothing any more.** An untracked
  repository inside the repository is no longer discarded — its copy could
  never be restored, and deleting it took its whole history; the same goes for
  names that are not valid UTF-8. A hard reset also copies untracked and
  ignored files it overwrites at paths the target commit holds; undo and
  restoring a backup refuse in that case. Discarding `d/a` keeps a copy of a
  file `d` that took the folder's place, and restoring puts it back. A restore
  checks every file before it writes the first one, so it no longer stops
  half-way. The storage limit now counts both copies a discard can need and
  stays a hard limit. Squash refuses while something unrelated is staged (it
  went into the squashed commit), taking back the first commit keeps the
  branch's reflog, *Track with LFS* refuses a file staged only in parts, and
  *Cancel* is only offered while it can still stop the discard.
- **Follow-up steps stay in their repository.** Another repository can no
  longer be opened while an operation runs; switching drops what the old one
  left behind — the ways out of its error toast ("Commit without a signature"
  would have amended the new repository's HEAD), a waiting push, carried
  changes. A merge finished with the Commit button brings carried changes back
  and offers the waiting push.
- **The sync dialogs do what they say.** *Merge* and *Rebase* in the
  "diverged" and "push rejected" dialogs — and the choice remembered for a
  repository — now override a `pull.ff=only` or `pull.rebase` in git, which
  sent *Merge* back into the same dialog. The dialog keeps the remote you
  pulled from; after a rejected push to another remote, the pull comes from
  that remote. *Update current branch from…* fetches the remote the source
  comes from, also for a remote source, and stays runnable when only the fetch
  will bring the commits. Applying or popping a stash checks the list has not
  changed meanwhile (a stash made in a terminal shifts every index), *Done*
  never runs twice, and without a running operation the workshop offers *Done*
  instead of *Continue*. Dropping every commit down to the first one is
  refused instead of leaving an empty commit behind.
- **Error messages are more accurate.** A leftover lock is told apart from a
  folder git may not write to and from a lock in a submodule (the way out
  removes exactly the lock git named, and refuses while any git program
  runs); certificate errors from Windows' own TLS backend are recognised; a
  failed push in a Git LFS repository is no longer blamed on a pre-push hook;
  a long list of files in the way keeps its heading.
- **The SSH trust dialog works with ssh_config aliases.** A host reached
  through `Host gh-work` → `HostName github.com`, a port from the config or an
  explicit `:22` could never be trusted — the dialog came back every time. The
  dialog, the probe and `ssh-keygen` now use the names ssh itself records and
  the ssh git itself runs (Git for Windows' own). Your own OpenSSH command —
  also one from an `includeIf` — keeps its options and stops asking too, and so
  do submodule updates, LFS downloads and checkouts of a blobless clone.
- **Word highlighting stays fast and accurate.** A huge or crafted diff can no
  longer freeze the window; in the conflict workshop, lines after an inserted
  one are compared with the right partner; Chinese and Japanese text is
  compared character by character, and accents written as combining marks stay
  with their letter.
- A detached HEAD is labelled in German on the branch button and in the
  worktree list too.

### Security

- **Dependencies.** `rustls` 0.23.46 closes four advisories
  (RUSTSEC-2026-0285, -0346, -0348, -0349) and `rustls-webpki` 0.103.16 a fifth
  (RUSTSEC-2026-0382, distrusted intermediate certificates) in the connections
  to GitHub, GitLab and Gitea; `tokio` 1.54.1 closes RUSTSEC-2026-0386. Tauri
  2.12, Svelte, Vite, Vitest and the other dependencies are on their current
  patch releases, and `npm audit` reports nothing.
- **Opening a repository runs nothing it defines.** A `.git` folder someone
  else hands you (an archive, a network share — not a clone) is executable code
  to git. Since 1.1 the status of a repository with filter drivers is read
  through git, so a repository-local `core.fsmonitor` command or filter would
  have run the moment it was opened. Now everything the app does on its own —
  status, history, the commit graph, the automatic fetch — runs without hooks
  and without repository-local filter commands (Git LFS's own excepted), and a
  repository-local `core.fsmonitor` command never runs.
- **Transfers refuse more programs a repository names for itself:** credential
  helpers for a single URL, the worktree configuration level, `core.gitProxy`,
  upload/receive-pack programs of a local remote and an ssh command that is
  more than a plain `ssh` call (`ssh -i key -o IdentitiesOnly=yes` stays
  allowed).
- **A server can no longer choose an error's way out.** Local causes (a lock
  file, an untrusted folder, a deleted remote branch …) are read only from
  git's own lines, never from the text a server sends along.
- **Test switches only in debug builds.** Two environment variables meant for
  the E2E tests — one answered every confirmation with OK — have no effect in a
  release build.
- **Smaller hardening.** "Open terminal here" no longer passes a folder name
  with `;` to Windows Terminal (which splits its command line there), and
  folder names derived from a clone URL keep only harmless characters. *Trust
  folder* adds exactly the repository root, never a wildcard, and its question
  says what trusting means. The SSH host-key probe works in a private temporary
  folder, commit messages for hooks go into a file only you can read, git's
  output is kept and logged within limits, and credentials in an https remote
  never reach the browser. Only the settings the app writes can be written
  through its configuration interface, the paths of a discard are checked
  before anything is copied, and nothing is written or deleted through a
  symlinked folder.

### Known limitations

- The installers are **not signed** — Windows SmartScreen and macOS
  Gatekeeper warn about an “unknown publisher”.
- **No auto-update**; updating is done manually.
- Hunk staging has a known race condition: reading the file, showing its hunks
  and applying your choice are three separate steps — if another program
  rewrites the file in between, the hunk that gets staged or discarded can
  differ from the one you saw.
- Sign-in only via personal access token, not yet via OAuth.
- Git LFS file locks are not supported yet.
- What you start in a repository behaves as with git itself: a commit runs its
  hooks, a merge or a checkout its filter and merge drivers. Opening a
  repository and the app's own refreshes run none of them.

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

[Unreleased]: https://github.com/Rynos-PCP/Terra-Git/compare/v1.1.0...HEAD
[1.1.0]: https://github.com/Rynos-PCP/Terra-Git/compare/1.0.0...v1.1.0
[1.0.0]: https://github.com/Rynos-PCP/Terra-Git/releases/tag/1.0.0
