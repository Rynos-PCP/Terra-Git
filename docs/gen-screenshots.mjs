// Renders the mock scenes headlessly to PNGs. Prerequisite: `npm run dev` is
// running (the Vite dev server under apps/desktop, port 1420) or MOCK_BASE points
// at a static server that serves `mock.html`.
// Invocation: node docs/gen-screenshots.mjs [scene ...]
// The EDGE default is the Windows install path; on macOS/Linux point EDGE at a
// Chromium-based browser, e.g. EDGE=/usr/bin/chromium node docs/gen-screenshots.mjs
import { execFile } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";

const EDGE = process.env.EDGE ?? "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe";
const BASE = process.env.MOCK_BASE ?? "http://localhost:1420/mock.html";
const OUT = "docs/images";

// Scenes as in apps/desktop/src/main-mock.ts (?scene=…). "changes" is the default
// scene (no ?scene= needed, but explicit for clarity in the file name).
const scenes = [
  "welcome", // welcome screen: recents + vein sketch
  "changes", // main window: changes list + overview/diff, toolbar
  "history", // history: overall graph + repository overview
  "settings", // settings
  "pipeline", // pipeline cockpit (job graph, status chips)
  "remotes", // remote management (modal)
  "backups", // backups / history-rewrite restore (modal)
  "sparse", // sparse checkout (modal)
  "clone", // clone repository (modal)
  "createCr", // create merge/pull request (modal)
  "filecontext", // context menu of a file (also covers the multi-select docs)
  "conflicts", // conflict workshop (base column, word marks)
  "compare", // compare view (changes since the fork point, merge/update/PR)
  "reflog", // reflog with terra-git's backups on top (modal)
];

/** Attempts per scene: headless Edge now and then exits without an image. */
const ATTEMPTS = 4;

function shootOnce(scene, outFile) {
  // A profile of its own per attempt: a second headless Edge on a shared
  // profile waits for the first and often writes nothing.
  const profile = mkdtempSync(join(tmpdir(), "terra-shot-"));
  return new Promise((res) => {
    execFile(
      EDGE,
      [
        "--headless=new",
        `--user-data-dir=${profile}`,
        "--disable-gpu",
        "--hide-scrollbars",
        "--window-size=1280,800",
        // Let animations (e.g. the vein drawing of the welcome screen) run to
        // completion in fast-forward before the image is taken.
        "--virtual-time-budget=5000",
        `--screenshot=${outFile}`,
        `${BASE}?scene=${scene}&lang=en`,
      ],
      { timeout: 60_000 },
      (err) => {
        if (err) console.error(`  ${scene}: ${err.message}`);
        // Edge's helpers may still hold files a moment: best effort only.
        try {
          rmSync(profile, { recursive: true, force: true, maxRetries: 10, retryDelay: 300 });
        } catch {
          // a leftover temp folder is harmless
        }
        res();
      },
    );
  });
}

async function shoot(scene) {
  // Headless Edge only writes screenshots reliably with an absolute path
  // (a relative path is not resolved on Windows headless).
  const outFile = resolve(OUT, `${scene}.png`);
  rmSync(outFile, { force: true });
  for (let i = 1; i <= ATTEMPTS; i++) {
    await shootOnce(scene, outFile);
    // The image can land a moment after the browser process has exited.
    for (let w = 0; w < 20 && !existsSync(outFile); w++) await sleep(250);
    if (existsSync(outFile)) return;
  }
  console.error(`  ${scene}: no image after ${ATTEMPTS} attempts`);
}

// Optional scene names as arguments render only those.
const only = process.argv.slice(2);
const todo = only.length ? scenes.filter((s) => only.includes(s)) : scenes;

mkdirSync(OUT, { recursive: true });
console.log(`Rendering ${todo.length} scenes from ${BASE} ...`);
for (const s of todo) {
  await shoot(s);
}
console.log("Screenshots ->", OUT);
