// Static a11y guard for the menu structure.
//
// `role="menu"` requires the entries to carry `role="menuitem"` — otherwise the
// ARIA tree is incomplete and screen readers announce the menu but find no
// entries in it.
//
// Why a SOURCE test and not a render test: the project deliberately has no DOM
// test environment (no jsdom/happy-dom/testing-library, vite.config.ts sets no
// `environment`) — all 16 test files check pure logic. A render test would need
// new infrastructure and would still only check ONE instance, not all menu
// entries in the tree. This test checks all of them at once and stays afterwards
// as a regression barrier.
//
// The sources come in through `import.meta.glob` instead of `node:fs`:
// `@types/node` is not installed and tsconfig pins `types: ["svelte",
// "vite/client"]` — a `node:fs` import would make `npm run check` red with
// "Cannot find module" (verified). `import.meta.glob` is typed through `vite/client`.
import { parse } from "svelte/compiler";
import { describe, expect, it } from "vitest";

const sources = import.meta.glob("../**/*.svelte", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

/** 1-based line number of a character offset. */
function lineOf(src: string, offset: number): number {
  return src.slice(0, offset).split("\n").length;
}

type Node = Record<string, unknown>;

function isNode(v: unknown): v is Node {
  return typeof v === "object" && v !== null;
}

/** Static attribute value, as long as it is plain text (not an expression). */
function staticAttr(node: Node, name: string): string | null {
  const attrs = node.attributes;
  if (!Array.isArray(attrs)) return null;
  for (const a of attrs) {
    if (!isNode(a) || a.type !== "Attribute" || a.name !== name) continue;
    const v = a.value;
    if (v === true) return "";
    if (Array.isArray(v) && v.length === 1 && isNode(v[0]) && v[0].type === "Text") {
      return String(v[0].data ?? "");
    }
    return null; // dynamic value — not statically decidable
  }
  return null;
}

/**
 * Walks the AST generically without knowing the block node types — that way
 * `{#if}`/`{#each}`/`{#snippet}` come along automatically and a Svelte update
 * does not break the guard.
 */
function walk(node: unknown, inMenu: boolean, visit: (n: Node, inMenu: boolean) => boolean): void {
  if (Array.isArray(node)) {
    for (const c of node) walk(c, inMenu, visit);
    return;
  }
  if (!isNode(node)) return;

  let next = inMenu;
  if (typeof node.type === "string") next = visit(node, inMenu);

  for (const [key, value] of Object.entries(node)) {
    if (key === "metadata" || key === "parent") continue;
    walk(value, next, visit);
  }
}

interface Scan {
  violations: string[];
  /** Menu containers without a single entry: {file:line} */
  emptyMenus: string[];
}

function scan(): Scan {
  const violations: string[] = [];
  const emptyMenus: string[] = [];

  for (const [file, src] of Object.entries(sources)) {
    const ast = parse(src, { modern: true });
    const menuStarts: number[] = [];
    let itemsSeen = 0;

    walk(ast.fragment, false, (node, inMenu) => {
      // Popup container: the Menu component (unless it is explicitly something
      // else through the role prop, e.g. role="dialog" for popups with input
      // fields) or a hand-built element with role="menu".
      const isMenu =
        (node.type === "Component" &&
          node.name === "Menu" &&
          (staticAttr(node, "role") ?? "menu") === "menu") ||
        (node.type === "RegularElement" && staticAttr(node, "role") === "menu");
      if (isMenu) {
        menuStarts.push(Number(node.start ?? 0));
        return true;
      }
      // The trigger sits outside the popup.
      if (node.type === "SnippetBlock") {
        const expr = node.expression;
        if (isNode(expr) && expr.name === "trigger") return false;
      }
      if (inMenu && node.type === "RegularElement" && node.name === "button") {
        const cls = staticAttr(node, "class") ?? "";
        if (cls.split(/\s+/).includes("item")) {
          itemsSeen++;
          if (staticAttr(node, "role") !== "menuitem") {
            violations.push(`${file}:${lineOf(src, Number(node.start ?? 0))}`);
          }
        }
      }
      return inMenu;
    });

    // A role="menu" with no entries at all would otherwise slip through.
    // Exception: Menu.svelte ITSELF is the reusable container — its entries are
    // supplied by the caller through a snippet, so none appear in that file.
    const isContainerDefinition = file.endsWith("/Menu.svelte");
    if (!isContainerDefinition && menuStarts.length > 0 && itemsSeen === 0) {
      for (const s of menuStarts) emptyMenus.push(`${file}:${lineOf(src, s)}`);
    }
  }
  return { violations, emptyMenus };
}

describe("menu accessibility", () => {
  it("every menu entry (button.item) carries role=menuitem", () => {
    expect(scan().violations).toEqual([]);
  });

  it("no menu container without a single menuitem", () => {
    expect(scan().emptyMenus).toEqual([]);
  });
});
