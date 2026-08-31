import { vitePreprocess } from "@sveltejs/vite-plugin-svelte";

export default {
  // Process TypeScript in <script lang="ts"> through the Vite pipeline.
  preprocess: vitePreprocess(),
};
