// Renders the mock scenes headlessly to PNGs. Prerequisite: `npm run dev` is
// running (the Vite dev server under apps/desktop, port 1420) or MOCK_BASE points
// at a static server that serves `mock.html`.
// Invocation: node docs/gen-screenshots.mjs
// The EDGE default is the Windows install path; on macOS/Linux point EDGE at a
// Chromium-based browser, e.g. EDGE=/usr/bin/chromium node docs/gen-screenshots.mjs
import { execFile } from "node:child_process";
import { mkdirSync } from "node:fs";
import { resolve } from "node:path";

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
];

function shoot(scene) {
  // Headless Edge only writes screenshots reliably with an absolute path
  // (a relative path is not resolved on Windows headless).
  const outFile = resolve(OUT, `${scene}.png`);
  return new Promise((res) => {
    execFile(
      EDGE,
      [
        "--headless=new",
        "--disable-gpu",
        "--hide-scrollbars",
        "--window-size=1280,800",
        // Let animations (e.g. the vein drawing of the welcome screen) run to
        // completion in fast-forward before the image is taken.
        "--virtual-time-budget=5000",
        `--screenshot=${outFile}`,
        `${BASE}?scene=${scene}&lang=en`,
      ],
      (err) => {
        if (err) console.error(`  ${scene}: ERROR (${err.message})`);
        res();
      },
    );
  });
}

mkdirSync(OUT, { recursive: true });
console.log(`Rendering ${scenes.length} scenes from ${BASE} ...`);
for (const s of scenes) {
  await shoot(s);
}
console.log("Screenshots ->", OUT);
