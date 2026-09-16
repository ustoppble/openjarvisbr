import { defineConfig } from "vite";

// Tauri espera que o dev server escute em host fixo e não abra o browser.
export default defineConfig(async () => ({
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    target: "esnext",
    minify: "esbuild",
    sourcemap: true,
    rollupOptions: {
      input: {
        main: "index.html",
        settings: "settings.html",
        overlay: "overlay.html",
      },
    },
  },
}));
