# terra-git — Roadmap

What is already in, what is being worked on next, and what will deliberately
never be built. There are no dates in this file: terra-git is maintained by one
person in spare time, and a date would be a guess dressed up as a commitment.

## Shipped in 1.0.0

1.0.0 is the first public release. It covers full GitHub Desktop parity —
staging down to hunks and lines, branches, merge and rebase, conflict
resolution, cherry-pick, revert, squash, stash, tags, blame, commit graph,
signing — plus the parts GitHub Desktop does not have: self-hosted GitLab as a
first-class provider, a local pipeline cockpit, multi-step undo with automatic
backup refs before every history rewrite, a command palette, worktrees and
sparse checkout.

- Everything that is in it, in detail: [CHANGELOG.md](CHANGELOG.md).
- Feature by feature against GitHub Desktop: [docs/FEATURES.md](docs/FEATURES.md).
- Why it is built the way it is: [ARCHITECTURE.md](ARCHITECTURE.md).

The limitations that ship with it are listed in the changelog: the bundles are
not code-signed, there is no auto-update, sign-in is by personal access token
only, and hunk staging has a known race. All four have an entry under Next.

## Next

Roughly in the order they matter, not in the order they are easy.

- **Code signing and notarization.** The 1.0.0 bundles are unsigned, so Windows
  SmartScreen and macOS Gatekeeper greet every new user with an "unknown
  publisher" warning. This is the first thing anyone sees and therefore the top
  item; it is blocked on paying for certificates rather than on code.
- **Auto-update.** Today a new version has to be noticed on the releases page
  and installed by hand. The Tauri updater verifies a signature over each
  update, so this lands with signing, not before it.
- **Linux bundles proven on a real runner.** CI builds and tests the whole
  workspace on Linux, and the release workflow produces an AppImage, a `.deb`
  and an `.rpm` — but nothing ever launches the built app there. The E2E smoke
  test needs WebView2 and runs on Windows only. Until that gap is closed, the
  Linux artefacts are built but unproven.
- **OAuth sign-in alongside personal access tokens.** A PAT is currently the
  only way to connect a GitHub, GitLab or Gitea/Forgejo account, which is the
  steepest part of
  the first five minutes. PAT sign-in stays; OAuth joins it.
- **Deeper provider APIs.** Pull and merge requests can be listed and created,
  with CI status inline. Issues, review comments and notifications cannot.
- **Syntax highlighting in the conflict editor.** The diff view highlights 21
  languages; the conflict workbench still shows plain text. Same machinery, a
  different view to wire it into.
- **A virtualized history overview.** The large vertical graph renders every
  commit it has loaded. The lists beside it are virtualized; this one is not,
  and a long history pays for that in frames.
- **Closing the hunk-staging race.** Hunk staging reads the file, shows you the
  hunks, then applies your choice. If another program rewrites the file in
  between, what gets applied can differ from what was displayed. The fix is to
  check that the blob has not moved between rendering and applying, and to say
  so instead of guessing.
- **Precise performance tracking.** CI has a budget gate that catches
  catastrophic regressions — hangs and O(n²) blowups — but its thresholds are
  deliberately generous multiples of the real targets, so a gradual slide goes
  unnoticed. What is missing is a stored baseline (or a service such as
  CodSpeed) that compares against the previous run instead of a fixed ceiling.

## Explicit non-goals

These are decisions, not gaps. Feature requests for them will be closed with a
pointer to this section.

- **An integrated terminal.** terra-git is a GUI and has no ambition to replace
  the terminal; "Open terminal here" hands over to the one you already
  configured. Building a terminal in would cost focus and the resource budget
  that is the whole point of the project.
- **A plugin system.** It commits the project to a public API very early and
  puts a security boundary around third-party code inside a client that holds
  keychain credentials. For a single maintainer that is the entire budget,
  spent on something other than the client.
- **Bitbucket support.** A deliberate project decision: a separate API world
  for a small share of the users terra-git is built for. Bitbucket remotes
  still work, because fetch, pull, push and clone run through system git — what
  is not planned is a Bitbucket provider with merge-request lists and CI status.

## How priorities are set

One maintainer, spare time, no roadmap meetings. What moves up the list:
something that blocks a new user in the first five minutes, something that
removes a whole class of bug reports, or something that fits into an evening.
What moves down: anything that adds a dependency, a background process or a
setting nobody asked for.

Issues and Discussions genuinely feed into the order — a bug several people hit
outranks anything on this page. Neither guarantees that a thing gets built, and
nothing here is a promise.
