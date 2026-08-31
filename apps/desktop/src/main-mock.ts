// Mock entry point for headless UI smoke tests (without a Tauri backend).
//
// This comment is the authoritative scene list; docs/gen-screenshots.mjs renders
// a subset of it into docs/images/.
//
// Scenes via ?scene=… (default: changes):
//   changes, unchanged, welcome, history, conflicts, conflictError,
//   checkoutBlocked, palette, settings, remotes, backups, crs, clone, sparse,
//   switchChoice, stashes, createCr, progress, cloning, pipeline, histprep,
//   workshop-empty, workshop, multiselect, filecontext, toolsmenu
//
// Language via ?lang=de|en, theme via ?theme=dark|light; the ⋯ menu opens on any
// scene via ?menu=tools.
import { mount } from "svelte";
import App from "./App.svelte";
import "./app.css";
import { setLang, t } from "./lib/i18n.svelte";
import { openPipeline, openWorkshop, selectFile, showError, ui } from "./lib/state.svelte";

const params = new URLSearchParams(location.search);
const lang = params.get("lang");
if (lang === "de" || lang === "en") setLang(lang);
const theme = params.get("theme");
if (theme === "dark" || theme === "light") ui.theme = theme;
const a11y = params.get("a11y") === "1";

mount(App, { target: document.getElementById("app")! });

const scene = params.get("scene") ?? "changes";

ui.repo = {
  path: "C:\\demo\\terra-git",
  name: "terra-git",
  currentBranch: "main",
  headDetached: false,
  isEmpty: false,
  historyPrepared: true,
};
ui.status = {
  staged: [{ path: "src/lib/api.ts", origPath: null, kind: "modified" }],
  unstaged: [
    { path: "README.md", origPath: null, kind: "modified" },
    { path: "src/lib/state.svelte.ts", origPath: null, kind: "modified" },
    { path: "src/lib/components/Toolbar.svelte", origPath: null, kind: "modified" },
    { path: "docs/HANDBOOK.md", origPath: null, kind: "modified" },
    { path: "new.txt", origPath: null, kind: "untracked" },
  ],
  branch: "main",
  upstream: "origin/main",
  ahead: 0,
  behind: 1,
  opState: "clean",
};
ui.branches = [
  {
    name: "main",
    isHead: true,
    isRemote: false,
    upstream: "origin/main",
    shortName: null,
    targetId: null,
    upstreamGone: false,
  },
  {
    name: "feature/palette",
    isHead: false,
    isRemote: false,
    upstream: null,
    shortName: null,
    targetId: null,
    upstreamGone: false,
  },
  {
    name: "fix/eol",
    isHead: false,
    isRemote: false,
    upstream: null,
    shortName: null,
    targetId: null,
    upstreamGone: true,
  },
];
ui.remotes = [
  { name: "origin", url: "https://github.com/demo/terra-git.git" },
  { name: "gitlab", url: "https://gitlab.local/demo/terra-git.git" },
];
ui.messageLog = [
  "feat(remotes): remote management in the GUI\n\nWith details in the body.",
  "fix(ui): drop-up menu in the commit box",
];

if (scene === "unchanged") {
  // Deliberately through selectFile instead of a direct assignment: that way the
  // real path runs, including loading the explanation and the sequence guard.
  ui.status!.unstaged = [
    { path: "apps/desktop/src-tauri/Cargo.toml", origPath: null, kind: "modified" },
  ];
  setTimeout(() => void selectFile("apps/desktop/src-tauri/Cargo.toml", false), 200);
} else if (scene === "welcome") {
  // Welcome screen: no repo opened.
  //
  // Do NOT set the recently-opened list here — loadRecents() in App.svelte
  // overwrites it asynchronously with the IPC result. It comes from the stub in
  // mock.html under the key `get_recent_repos` (that is what the command is
  // called in api.ts).
  ui.repo = null;
} else if (scene === "history") {
  // History tab with a filled graph (main + feature strand with a merge).
  ui.tab = "history";
  const now = Math.floor(Date.now() / 1000);
  const c = (
    id: string,
    summary: string,
    age: number,
    parents: string[],
  ): (typeof ui.history)[number] => ({
    id,
    shortId: id.slice(0, 7),
    summary,
    authorName: "Demo User",
    authorEmail: "dev@example.com",
    time: now - age,
    parentIds: parents,
  });
  ui.history = [
    c("a1".repeat(20), "feat(remotes): remote management in the GUI", 2700, ["b2".repeat(20)]),
    c("b2".repeat(20), "Merge branch 'feature/palette'", 9000, ["c3".repeat(20), "d4".repeat(20)]),
    c("c3".repeat(20), "fix(ui): drop-up menu in the commit box", 18000, ["e5".repeat(20)]),
    c("d4".repeat(20), "feat(palette): improve fuzzy search", 21000, ["e5".repeat(20)]),
    c("e5".repeat(20), "docs: extend the handbook", 90000, ["f6".repeat(20)]),
    c("f6".repeat(20), "fix(diff): CRLF detection for partial patches", 100000, ["a7".repeat(20)]),
    c("a7".repeat(20), "chore: update Cargo.lock", 260000, ["b8".repeat(20)]),
    c("b8".repeat(20), "feat(stash): stash management", 350000, ["c9".repeat(20)]),
    c("c9".repeat(20), "Initial commit", 500000, []),
  ];
  ui.historyComplete = true;
  // Decorations for the graph track and the history overview (chips at the tips,
  // one tag): point the branch targets at the scene's commits.
  ui.branches[0].targetId = "a1".repeat(20); // main (HEAD)
  ui.branches[1].targetId = "d4".repeat(20); // feature/palette (merged tip)
  ui.branches[2].targetId = "b8".repeat(20); // fix/eol
  ui.branches.push({
    name: "origin/main",
    isHead: false,
    isRemote: true,
    upstream: null,
    shortName: "main",
    targetId: "b2".repeat(20),
    upstreamGone: false,
  });
  ui.tags = [{ name: "v0.3", targetId: "e5".repeat(20), message: null, isAnnotated: true }];
} else if (scene === "conflicts") {
  // Conflict workshop: a running merge with three conflicted files.
  // Context + segment data come from the IPC stub (get_op_context/read_conflict).
  ui.status!.staged = [];
  ui.status!.unstaged = [
    { path: "src/lib/api.ts", origPath: null, kind: "conflicted" },
    { path: "src/lib/state.svelte.ts", origPath: null, kind: "conflicted" },
    { path: "README.md", origPath: null, kind: "conflicted" },
  ];
  ui.status!.opState = "merge";
  ui.view = "conflicts";
} else if (scene === "conflictError") {
  // The way INTO the workshop: a pull with conflicts. The error toast offers the
  // jump, and the tools menu keeps the fixed slot for it open.
  ui.status!.staged = [];
  ui.status!.unstaged = [
    { path: "src/lib/api.ts", origPath: null, kind: "conflicted" },
    { path: "README.md", origPath: null, kind: "conflicted" },
  ];
  ui.status!.opState = "merge";
  showError({ code: "merge_conflict", message: "Automatic merge failed" });
} else if (scene === "checkoutBlocked") {
  // The case libgit2 calls "n conflicts prevent checkout" even though there are
  // no conflicts: the switch would overwrite uncommitted changes. The message
  // names the files, the toast the way out.
  showError({
    code: "checkout_would_overwrite",
    message: "apps/desktop/src/lib/state.svelte.ts, docs/HANDBOOK.md",
  });
  ui.errorAction = { kind: "stashSwitch", target: { kind: "branch", name: "main" } };
} else if (scene === "palette") {
  // The listener is only registered after the effect flush (onMount) → delay it.
  setTimeout(() => window.dispatchEvent(new CustomEvent("app-palette")), 300);
} else if (scene === "settings") {
  ui.view = "settings";
}
if (a11y) {
  ui.uiScale = 1.1;
  ui.highContrast = true;
  ui.reduceMotion = true;
} else if (scene === "remotes") {
  ui.modal = { kind: "remotes" };
} else if (scene === "backups") {
  ui.modal = { kind: "backups" };
} else if (scene === "crs") {
  ui.modal = { kind: "changeRequests" };
} else if (scene === "clone") {
  ui.modal = { kind: "clone" };
} else if (scene === "sparse") {
  ui.modal = { kind: "sparse" };
} else if (scene === "switchChoice") {
  // The branch switch now asks where the uncommitted changes belong instead of
  // silently taking them along.
  ui.modal = { kind: "switchBranch", target: { kind: "branch", name: "feature/palette" } };
} else if (scene === "stashes") {
  // Stash list with one entry left behind during a switch: the technical marker
  // appears as a readable label, the raw text in the tooltip.
  ui.stashes = [
    { index: 0, message: "On main: terra-git-autostash:main", id: "a".repeat(40) },
    { index: 1, message: "On main: WIP toolbar rework", id: "b".repeat(40) },
  ];
  ui.modal = { kind: "stash" };
} else if (scene === "createCr") {
  ui.history = [
    {
      id: "abc123",
      shortId: "abc123",
      summary: "feat(palette): improve fuzzy search",
      authorName: "Demo",
      authorEmail: "demo@example.com",
      time: Math.floor(Date.now() / 1000),
      parentIds: [],
    },
  ];
  ui.modal = { kind: "createCr" };
} else if (scene === "progress") {
  ui.busy = "Push";
  ui.progress = { phase: "receiving", percent: 62 };
} else if (scene === "cloning") {
  ui.cloning = "terra-git";
  ui.progress = { phase: "receiving", percent: 38 };
} else if (scene === "pipeline") {
  // The cockpit page instead of a modal: loads configs + graph from the IPC stub.
  void openPipeline();
} else if (scene === "histprep") {
  // History tab with the "being prepared" hint (a fresh huge clone).
  ui.tab = "history";
  ui.historyPreparing = true;
} else if (scene === "workshop-empty") {
  // Empty state of the workshop: open the page directly, without loading commits.
  ui.view = "commits";
} else if (scene === "workshop") {
  // Commit workshop: loads the unpushed commits from the IPC stub and then seeds
  // demo edits (HEAD subject changed, WIP commit dropped) so the state markers
  // (changed/dropped) are visible.
  ui.status!.ahead = 4;
  void openWorkshop().then(() => {
    const head = ui.unpushed.find((c) => c.isHead);
    const wip = ui.unpushed.find((c) => c.subject.startsWith("wip:"));
    const fix = ui.unpushed.find((c) => c.subject.startsWith("fix(ui)"));
    const eh = head && ui.workshopEdits[head.id];
    if (eh) eh.subject = "feat(diff): name the cause of empty diffs";
    const ew = wip && ui.workshopEdits[wip.id];
    if (ew) ew.dropped = true;
    const ef = fix && ui.workshopEdits[fix.id];
    if (ef) ef.squashed = true;
  });
} else if (scene === "multiselect" || scene === "filecontext") {
  // Mark two unstaged rows with Ctrl+click (verifies the modifier evaluation),
  // and for "filecontext" additionally open the context menu.
  setTimeout(() => {
    const rows = [...document.querySelectorAll<HTMLElement>(".lists .row")];
    const byPath = (p: string) =>
      rows.find((r) => r.querySelector(".path")?.textContent?.trim() === p);
    const targets = [byPath("src/lib/state.svelte.ts"), byPath("docs/HANDBOOK.md")];
    targets.forEach((r) =>
      r?.dispatchEvent(new MouseEvent("click", { ctrlKey: true, bubbles: true })),
    );
    if (scene === "filecontext" && targets[1]) {
      const box = targets[1].getBoundingClientRect();
      targets[1].dispatchEvent(
        new MouseEvent("contextmenu", {
          bubbles: true,
          clientX: box.right - 40,
          clientY: box.bottom,
        }),
      );
    }
  }, 200);
}

// The toolbar's ⋯ menu (tools/manage) can be opened on ANY scene — that way the
// workshop entry can be checked with and without a running operation. Looked up
// through the i18n label, not through German plain text: the scenes also run
// with ?lang=en.
if (scene === "toolsmenu" || params.get("menu") === "tools") {
  setTimeout(() => {
    document
      .querySelector<HTMLElement>(`button[aria-label="${t("toolbar.moreActions")}"]`)
      ?.click();
  }, 300);
}
