// defineConfig from vitest/config (instead of vite) knows the `test` field with types.
import { defineConfig } from "vitest/config";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// https://vite.dev/config/
export default defineConfig({
  plugins: [svelte()],

  // Tauri expects a fixed port; fail instead of silently switching ports.
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      // Rust changes do not trigger a frontend reload.
      ignored: ["**/src-tauri/**"],
    },
  },

  // Vitest only covers the pure frontend logic under src/. The e2e smoke tests
  // (e2e/*.test.mjs) run through node:test (`npm run e2e`), NOT through Vitest —
  // otherwise Vitest collects them with its default glob and aborts with
  // "No test suite found" (a red `npm test` / CI test stage).
  test: {
    include: ["src/**/*.{test,spec}.{js,ts}"],
  },
});
