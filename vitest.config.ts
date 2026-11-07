import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import { fileURLToPath, URL } from "node:url";

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      "@": fileURLToPath(new URL("./src", import.meta.url)),
    },
  },
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./src/test/setup.ts"],
    include: ["src/**/*.{test,spec}.{ts,tsx}"],
    restoreMocks: true,
    // Cold jsdom starts can time out when too many workers read a slow disk;
    // two workers keep the suite deterministic. Vitest 5 has no `minWorkers`
    // anymore, so this is a cap, not a floor.
    pool: "forks",
    maxWorkers: 2,
  },
});
