// Shared pieces of the flow tests: fixture repositories built
// with the git CLI, and driving the real app through stable `data-testid`
// hooks, so the tests do not depend on the UI language.
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";
import { startSession } from "./webdriver.mjs";

const ROOT = join(import.meta.dirname, "..", "..", "..");
export const APP = join(ROOT, "target", "debug", "tg-app.exe");
const DRIVER = join(homedir(), ".cargo", "bin", "tauri-driver.exe");
const NATIVE = join(import.meta.dirname, "drivers", "msedgedriver.exe");

/** Exits with a hint when a prerequisite of docs/DEVELOPMENT.md is missing. */
export function requirePrerequisites() {
  for (const [what, p] of [
    ["App-Build", APP],
    ["tauri-driver", DRIVER],
    ["msedgedriver", NATIVE],
  ]) {
    if (!existsSync(p)) {
      console.error(`MISSING: ${what} (${p}) — see the E2E section of docs/DEVELOPMENT.md`);
      process.exit(1);
    }
  }
}

/**
 * Starts the app with a config directory of its own (`TERRA_GIT_CONFIG_DIR`),
 * so the recent list and the provider accounts of the real profile stay
 * untouched — whatever a test opens, and even when it fails half-way.
 */
export function startApp() {
  const config = tempDir("config");
  return startSession({
    application: APP,
    driverBin: DRIVER,
    nativeDriver: NATIVE,
    // The app answers its confirmation questions itself (src/lib/dialogs.ts):
    // a native dialog is out of WebDriver's reach.
    env: { TERRA_GIT_CONFIG_DIR: config, TERRA_GIT_E2E_ACCEPT_DIALOGS: "1" },
  });
}

// ---------------------------------------------------------------------------
// Fixtures

/** Runs git in `cwd` and returns its trimmed stdout. */
export function git(cwd, ...args) {
  return execFileSync("git", args, { cwd, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] })
    .toString()
    .trim();
}

/** Like `git`, but returns null instead of throwing. */
export function gitTry(cwd, ...args) {
  try {
    return git(cwd, ...args);
  } catch {
    return null;
  }
}

/** A fresh temp folder; `cleanup()` removes every folder made here. */
const made = [];
export function tempDir(prefix) {
  const dir = mkdtempSync(join(tmpdir(), `terra-e2e-${prefix}-`));
  made.push(dir);
  return dir;
}
export function cleanupTemp() {
  for (const d of made.splice(0)) rmSync(d, { recursive: true, force: true });
}

/** Hermetic local config, independent of the developer's git setup. */
function configure(repo) {
  for (const [k, v] of [
    ["user.name", "Terra E2E"],
    ["user.email", "e2e@terra.local"],
    ["commit.gpgsign", "false"],
    ["core.autocrlf", "false"],
    ["pull.rebase", "false"],
  ]) {
    git(repo, "config", k, v);
  }
}

export function initRepo(dir) {
  mkdirSync(dir, { recursive: true });
  git(dir, "init", "-q", "-b", "main");
  configure(dir);
  return dir;
}

export function commitFile(repo, file, content, message) {
  writeFileSync(join(repo, file), content);
  git(repo, "add", file);
  git(repo, "commit", "-q", "-m", message);
}

/** A bare `origin` with one commit, and `count` clones of it. */
export function originWithClones(prefix, count) {
  const base = tempDir(prefix);
  const seed = initRepo(join(base, "seed"));
  commitFile(seed, "a.txt", "base\n", "base");
  const origin = join(base, "origin.git");
  git(base, "clone", "-q", "--bare", seed, origin);
  const clones = [];
  for (let i = 0; i < count; i++) {
    const c = join(base, `clone${i}`);
    git(base, "clone", "-q", origin, c);
    configure(c);
    clones.push(c);
  }
  return { origin, clones };
}

// ---------------------------------------------------------------------------
// Driving the app

/** Clicks the first element matching `css` (in the page, not via WebDriver
 *  pointer actions — menus and dialogs stay where they are). */
export async function click(s, css) {
  await s.waitFor(
    () =>
      s.exec(
        `const el = document.querySelector(arguments[0]);
         if (!el || el.disabled) return false;
         el.click();
         return true;`,
        [css],
      ),
    `click ${css}`,
  );
}

export function present(s, css) {
  return s.exec("return !!document.querySelector(arguments[0])", [css]);
}

/**
 * Opens `path` like a user does: it lands in the recent list (through the
 * same command the app uses), the start screen is reloaded, and the entry is
 * clicked (the config directory is the test's own, see `startApp`).
 */
export async function openInApp(s, path) {
  const stored = await s.exec(
    "return window.__TAURI_INTERNALS__.invoke('open_repository', { path: arguments[0] }).then((r) => r.path)",
    [path],
  );
  await s.exec("location.reload()");
  await s.waitFor(
    () => s.exec("return !!document.querySelector('.shell')"),
    "app shell after reload",
  );
  // The list may spell the path with backslashes: compare normalised.
  await s.waitFor(
    () =>
      s.exec(
        `const norm = (p) => p.split(String.fromCharCode(92)).join("/").toLowerCase();
         const el = [...document.querySelectorAll(".recent")].find(
           (e) => norm(e.getAttribute("data-path") || "") === norm(arguments[0]),
         );
         if (!el) return false;
         el.click();
         return true;`,
        [stored],
      ),
    "recent entry of the fixture",
  );
  await s.waitFor(() => present(s, '[data-testid="push"]'), "repo view open");
  return stored;
}
