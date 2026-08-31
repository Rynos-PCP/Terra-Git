// E2E smoke test: starts the REAL app (WebView2) through tauri-driver and checks
// the core entry point. Prerequisites: see the E2E section of docs/DEVELOPMENT.md:
//   1. cargo build -p tg-app --features custom-protocol  (+ npm run build)
//   2. cargo install tauri-driver
//   3. e2e/drivers/msedgedriver.exe matching the WebView2 version
import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { startSession } from "./webdriver.mjs";

const ROOT = join(import.meta.dirname, "..", "..", "..");
const APP = join(ROOT, "target", "debug", "tg-app.exe");
const DRIVER = join(homedir(), ".cargo", "bin", "tauri-driver.exe");
const NATIVE = join(import.meta.dirname, "drivers", "msedgedriver.exe");

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

test("app starts, renders the UI and responds", async () => {
  const s = await startSession({
    application: APP,
    driverBin: DRIVER,
    nativeDriver: NATIVE,
  });
  try {
    // The window + WebView are alive: the title is right.
    const title = await s.waitFor(() => s.exec("return document.title"), "document title");
    assert.equal(title, "terra-git");

    // The initial state renders (welcome without a repo OR the workspace with a
    // repo — the test runs against the real profile and is robust against both).
    const shell = await s.waitFor(
      () =>
        s.exec(
          "return document.querySelector('.shell') ? document.querySelector('.shell').children.length : 0",
        ),
      "app shell rendered",
    );
    assert.ok(shell > 0, "app shell has content");

    // Interaction: Ctrl+K opens the command palette (the central handler).
    await s.exec(
      "window.dispatchEvent(new KeyboardEvent('keydown', { key: 'k', ctrlKey: true, bubbles: true }))",
    );
    await s.waitFor(
      () => s.exec("return !!document.querySelector('.palette')"),
      "command palette open",
    );
    // Escape closes it again.
    await s.exec(
      "document.querySelector('.palette input').dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))",
    );
    await s.waitFor(
      () => s.exec("return !document.querySelector('.palette')"),
      "command palette closed",
    );
  } finally {
    await s.quit();
  }
});
