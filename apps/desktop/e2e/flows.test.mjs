// E2E flows of the destructive paths of the 1.1 package:
// the real app (WebView2 through tauri-driver) against fixture repositories
// built with the git CLI.
//
// Prerequisites as for the smoke test (docs/DEVELOPMENT.md). The app runs
// with a config directory of its own: the real recent list stays untouched.
import assert from "node:assert/strict";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { after, before, describe, it } from "node:test";
import {
  cleanupTemp,
  click,
  commitFile,
  git,
  gitTry,
  initRepo,
  openInApp,
  originWithClones,
  present,
  requirePrerequisites,
  startApp,
  tempDir,
} from "./helpers.mjs";

requirePrerequisites();

const read = (repo, file) => readFileSync(join(repo, file), "utf8");
const noMarkers = (text) => !/^(<{7}|={7}|>{7})/m.test(text);
const rebasing = (repo) =>
  existsSync(join(repo, ".git", "rebase-merge")) || existsSync(join(repo, ".git", "rebase-apply"));

/** main and feature both change a.txt; HEAD on feature. */
function rebaseFixture() {
  const dir = initRepo(tempDir("rebase"));
  commitFile(dir, "a.txt", "base\n", "base");
  git(dir, "switch", "-q", "-c", "feature");
  commitFile(dir, "a.txt", "feature\n", "feature change");
  git(dir, "switch", "-q", "main");
  commitFile(dir, "a.txt", "main\n", "main change");
  git(dir, "switch", "-q", "feature");
  return dir;
}

describe("destructive flows", () => {
  let s;

  before(async () => {
    s = await startApp();
    await s.waitFor(() => s.exec("return document.title"), "app window");
  });

  after(async () => {
    await s.quit();
    cleanupTemp();
  });

  const open = (dir) => openInApp(s, dir);

  async function rebaseOntoMain() {
    await click(s, '[data-testid="branch-menu"]');
    await click(s, '.row[data-branch="main"] [data-testid="rebase-onto"]');
    // The workshop opens by itself on conflicts.
    await s
      .waitFor(() => present(s, ".workshop"), "conflict workshop opened by itself")
      .catch(async (e) => {
        const view = await s.exec(
          "return [document.querySelector('.toast')?.innerText ?? '', !!document.querySelector('[data-testid=\"branch-menu\"]')]",
        );
        throw new Error(`${e.message} — app shows: ${JSON.stringify(view)}`);
      });
  }

  it("1a: rebase conflict → workshop → continue", async () => {
    const dir = rebaseFixture();
    await open(dir);
    await rebaseOntoMain();

    // In a rebase "theirs" is the commit being replayed — keep it (taking
    // main's side would leave the commit empty, and git drops it).
    await click(s, '[data-testid="ws-take-theirs"]');
    await click(s, '[data-testid="ws-continue"]');

    await s.waitFor(() => !rebasing(dir), "rebase finished");
    assert.equal(git(dir, "log", "--format=%s", "-3"), "feature change\nmain change\nbase");
    assert.ok(noMarkers(read(dir, "a.txt")), "no conflict markers left");
    assert.equal(git(dir, "status", "--porcelain"), "", "clean afterwards");
  });

  it("1b: rebase conflict → workshop → abort", async () => {
    const dir = rebaseFixture();
    const before = git(dir, "rev-parse", "HEAD");
    await open(dir);
    await rebaseOntoMain();

    await click(s, '[data-testid="ws-abort"]');

    await s.waitFor(() => !rebasing(dir), "rebase aborted");
    assert.equal(git(dir, "rev-parse", "HEAD"), before, "back where it started");
    assert.equal(read(dir, "a.txt"), "feature\n");
  });

  it("2: pull with local changes → stash conflict → workshop → done; the stash goes only then", async () => {
    const {
      clones: [mine, theirs],
    } = originWithClones("pull", 2);
    commitFile(theirs, "a.txt", "remote\n", "remote change");
    git(theirs, "push", "-q");
    writeFileSync(join(mine, "a.txt"), "local\n"); // uncommitted, in the way
    await open(mine);

    await click(s, '[data-testid="pull"]');
    // git refuses: the dialog offers to carry the changes through.
    await click(s, ".modal .opt.recommended");

    await s.waitFor(() => present(s, ".workshop"), "workshop for the stash conflict");
    assert.equal(git(mine, "log", "-1", "--format=%s"), "remote change", "the pull happened");
    assert.equal(
      git(mine, "stash", "list").split("\n").filter(Boolean).length,
      1,
      "the stash is kept while its conflicts are open",
    );

    await click(s, '[data-testid="ws-take-ours"]');
    await click(s, '[data-testid="ws-finish"]');

    await s.waitFor(() => git(mine, "stash", "list") === "", "stash dropped after done");
    assert.ok(noMarkers(read(mine, "a.txt")), "no conflict markers left");
  });

  it("3: push rejected → pull & merge → push", async () => {
    const {
      origin,
      clones: [mine, theirs],
    } = originWithClones("push", 2);
    commitFile(theirs, "b.txt", "theirs\n", "remote work");
    git(theirs, "push", "-q");
    commitFile(mine, "c.txt", "mine\n", "local work");
    await open(mine);

    await click(s, '[data-testid="push"]');
    // Rejected: the dialog's recommended way is pull & merge, then push.
    await click(s, ".modal .opt.recommended");

    await s.waitFor(() => {
      const log = gitTry(origin, "log", "--format=%s", "main") ?? "";
      return log.includes("local work") && log.includes("remote work");
    }, "both commits on the remote");
    assert.equal(git(mine, "status", "--porcelain"), "");
  });

  it("5: remote branch deleted → delete the merged local branch from the error", async () => {
    const {
      clones: [mine, theirs],
    } = originWithClones("gone", 2);
    git(mine, "switch", "-q", "-c", "feature");
    git(mine, "push", "-q", "-u", "origin", "feature");
    git(theirs, "push", "-q", "origin", "--delete", "feature");
    await open(mine);

    await click(s, '[data-testid="pull"]');
    // upstream_gone: the third way out.
    await click(s, '[data-testid="error-action-deleteGoneBranch"]');

    await s.waitFor(() => git(mine, "branch", "--show-current") === "main", "switched to main");
    await s.waitFor(
      () => gitTry(mine, "rev-parse", "-q", "--verify", "refs/heads/feature") === null,
      "the merged local branch is gone",
    );
    // It asked first (answered by the app itself under test, see dialogs.ts).
    const asked = await s.exec("return window.__e2eDialogs ?? []");
    assert.ok(
      asked.some((m) => m.includes("feature")),
      JSON.stringify(asked),
    );
  });

  it("4: undo the very first commit", async () => {
    const dir = initRepo(tempDir("root"));
    commitFile(dir, "a.txt", "a\n", "first");
    await open(dir);

    await click(s, '[data-testid="undo-last-commit"]');

    await s.waitFor(
      () => gitTry(dir, "rev-parse", "-q", "--verify", "HEAD") === null,
      "no commit left",
    );
    assert.equal(git(dir, "diff", "--cached", "--name-only"), "a.txt", "the file stays staged");
  });
});
