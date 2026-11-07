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
    // Cold parallel jsdom startup timed out on the project's HDD before files
    // could run; two workers keep the suite deterministic on that machine.
    // Vitest 5 has no `minWorkers` anymore: this is a cap, not a floor.
    pool: "forks",
    maxWorkers: 2,
  },
});
