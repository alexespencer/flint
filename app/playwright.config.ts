import { defineConfig } from "@playwright/test";

// The app runs in plain Chromium against the Vite dev server, with the Tauri side faked (e2e/mockTauri.ts).
export default defineConfig({
  testDir: "e2e",
  testMatch: "**/*.e2e.ts",
  outputDir: "e2e/results",
  use: { baseURL: "http://localhost:1420", viewport: { width: 1280, height: 800 }, deviceScaleFactor: 2 },
  webServer: { command: "bun run dev", url: "http://localhost:1420", reuseExistingServer: true },
});
