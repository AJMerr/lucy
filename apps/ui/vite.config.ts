import { svelte } from "@sveltejs/vite-plugin-svelte";
import { defineConfig } from "vitest/config";

// Lucy's web front end. Pages: her sprite overlay (index.html, the Tauri app's first window)
// and the main window (app.html, also usable in a browser). Tauri serves the dev server at a
// fixed port and loads `dist` in builds.
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  build: {
    target: "es2022",
    outDir: "dist",
    emptyOutDir: true,
    rollupOptions: { input: { main: "index.html", app: "app.html" } },
  },
  // Component tests need a DOM and Svelte's browser build.
  resolve: process.env.VITEST ? { conditions: ["browser"] } : undefined,
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.ts"],
  },
});
