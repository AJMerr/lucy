import { defineConfig } from "vite";

// Tauri serves the dev server at a fixed port and loads `dist` in builds.
export default defineConfig({
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  build: { target: "es2022", outDir: "dist", emptyOutDir: true },
});
