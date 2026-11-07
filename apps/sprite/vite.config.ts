import { defineConfig } from "vite";

// Tauri serves the dev server at a fixed port and loads `dist` in builds. Two pages: her
// (index.html) and her chat window (chat.html).
export default defineConfig({
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  build: {
    target: "es2022",
    outDir: "dist",
    emptyOutDir: true,
    rollupOptions: { input: { main: "index.html", chat: "chat.html" } },
  },
});
