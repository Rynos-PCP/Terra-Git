// Flat config (ESLint 10) for the Svelte 5 frontend.
// Goal: correctness rules (unused variables, obvious mistakes, Svelte specifics)
// as a CI gate — pure formatting is left to the Prettier run by
// `eslint-config-prettier`. Deliberately no type-checked preset (svelte-check
// already covers the types; that keeps the lint fast and free of tsconfig project coupling).
import js from "@eslint/js";
import ts from "typescript-eslint";
import svelte from "eslint-plugin-svelte";
import prettier from "eslint-config-prettier";
import globals from "globals";

export default ts.config(
  {
    // Generated/bundled artifacts, the Rust side and config files.
    ignores: ["dist/", "node_modules/", "src-tauri/", "e2e/", "*.config.*", "mock.html"],
  },
  js.configs.recommended,
  ...ts.configs.recommended,
  ...svelte.configs["flat/recommended"],
  prettier,
  ...svelte.configs["flat/prettier"],
  {
    languageOptions: {
      globals: { ...globals.browser },
    },
    rules: {
      // Unused arguments with a leading _ are allowed (callback signatures).
      "@typescript-eslint/no-unused-vars": [
        "error",
        { argsIgnorePattern: "^_", varsIgnorePattern: "^_" },
      ],
      // Opinionated Svelte 5 recommendations: sensible, but auto-rewriting the
      // existing components (SvelteSet/SvelteMap, each keys) carries subtle
      // reactivity/render changes. As a hint (warn) instead of a CI blocker —
      // cleaning that up is its own frontend ticket, not a merge gate.
      "svelte/prefer-svelte-reactivity": "warn",
      "svelte/require-each-key": "warn",
    },
  },
  {
    files: ["**/*.svelte", "**/*.svelte.ts"],
    languageOptions: {
      parserOptions: { parser: ts.parser },
    },
  },
  {
    // The diff/blame views render exclusively SELF-produced markup of local file
    // contents, escaped by highlight.js — never foreign HTML.
    // The {@html} XSS rule deliberately does not apply here.
    files: ["**/DiffView.svelte", "**/BlameView.svelte"],
    rules: { "svelte/no-at-html-tags": "off" },
  },
);
