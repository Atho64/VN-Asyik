import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { fileURLToPath, URL } from "node:url";

// Tauri serves the dev server on a fixed port and expects it to be ready before the window opens.
export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      "@": fileURLToPath(new URL("./src", import.meta.url)),
    },
  },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    // Bind IPv4 explicitly. Without this Vite listens on [::1] only, so the
    // documented http://localhost:1420 resolves to 127.0.0.1 and is refused.
    // Tauri's own templates set this for the same reason.
    host: "127.0.0.1",
    watch: {
      // src-tauri and crates are watched by cargo, not by vite.
      ignored: ["**/src-tauri/**", "**/crates/**"],
    },
  },
  envPrefix: ["VITE_", "TAURI_ENV_"],
  build: {
    // Windows uses WebView2 (Chromium), Android uses the system WebView.
    target: "es2022",
    // Vite 8 defaults to the Oxc minifier; esbuild is no longer bundled.
    minify: true,
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
  },
});
