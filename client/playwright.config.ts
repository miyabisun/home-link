import { tmpdir } from "node:os";
import { join } from "node:path";

import { defineConfig } from "@playwright/test";

// The E2E tests run the built home-link (`cargo build`) against a mock matterjs-server.
const PORT = "5198";
const MATTER_PORT = "5598";
process.env.E2E_DB = join(tmpdir(), `home-link-e2e-${PORT}.db`);

export default defineConfig({
  testDir: "./e2e",
  workers: 1,
  use: { baseURL: `http://127.0.0.1:${PORT}`, browserName: "chromium" },
  webServer: {
    command: "node e2e/serve.mjs",
    url: `http://127.0.0.1:${MATTER_PORT}/`,
    env: { E2E_DB: process.env.E2E_DB, PORT, MATTER_PORT },
  },
});
