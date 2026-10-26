// E2E smoke test: starts the REAL app (WebView2) through tauri-driver and checks
// the core entry point. Prerequisites: see the E2E section of docs/DEVELOPMENT.md:
//   1. cargo build -p tg-app --features custom-protocol  (+ npm run build)
//   2. cargo install tauri-driver
//   3. e2e/drivers/msedgedriver.exe matching the WebView2 version
import assert from "node:assert/strict";
import test from "node:test";
import { cleanupTemp, requirePrerequisites, startApp } from "./helpers.mjs";

requirePrerequisites();

test("app starts, renders the UI and responds", async () => {
  const s = await startApp();
  try {
    // The window + WebView are alive: the title is right.
    const title = await s.waitFor(() => s.exec("return document.title"), "document title");
    assert.equal(title, "terra-git");

    // The initial state renders (the start screen — the app runs with a
    // config directory of its own, see helpers.startApp).
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
    cleanupTemp();
  }
});
