<!--
Thanks for the contribution. The points below are not red tape; they are
what the project's stability hangs on — details in CONTRIBUTING.md.
-->

## What is this about?

<!-- What changes for users? For a fix: what went wrong before? -->

Fixes #

## Why this way?

<!-- The interesting part. Which approaches did you discard, and why?
     The project comments on decisions, not code — this is where that
     starts. -->

## Verified

<!-- Not just "tests green", but what you actually tried it with:
     which repo, which state, which operating system. -->

- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo fmt --all -- --check`
- [ ] `cd apps/desktop && npm run lint && npm run format:check && npm run check && npm test`
- [ ] For UI changes: looked at it in the real app (light **and** dark, de **and** en)

## Checklist

- [ ] **TDD:** the failing test first, then the fix. Every bug fix has
      a regression test that is red without the change.
- [ ] **DCO:** every commit is signed off with `git commit -s`
      (`Signed-off-by:` line) — see CONTRIBUTING.md.
- [ ] No emojis or Unicode pictograms in the UI; icons are SVG.
- [ ] New texts exist in **both** catalogs (`messages/de.ts` **and**
      `messages/en.ts`), no strings directly in the markup.
- [ ] Docs updated where needed (HANDBOOK, FEATURES, ARCHITECTURE).
