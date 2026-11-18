import { defineConfig, devices } from "@playwright/test";
import { tmpdir } from "node:os";
import { join } from "node:path";

// The server under test: the release binary (CI) or any build given in AGENTTABLE_BIN.
const binary = process.env.AGENTTABLE_BIN ?? "../../target/release/agenttable";
const port = Number(process.env.AGENTTABLE_PORT ?? 18080);
const data = join(tmpdir(), `agenttable-e2e-${port}.json`);

export default defineConfig({
  testDir: "e2e",
  outputDir: "e2e/results",
  fullyParallel: false,
  workers: 1,
  // The walk-through builds on its own earlier steps, so a failed step is not retried.
  retries: 0,
  timeout: 60_000,
  expect: { timeout: 10_000 },
  reporter: [["list"], ["html", { outputFolder: "e2e/report", open: "never" }]],
  use: {
    baseURL: `http://127.0.0.1:${port}`,
    ...devices["Desktop Chrome"],
    viewport: { width: 1440, height: 900 },
    deviceScaleFactor: 2,
    trace: "off",
    screenshot: "only-on-failure",
  },
  webServer: {
    command: `"${binary}" --port ${port} --data "${data}" --fresh`,
    url: `http://127.0.0.1:${port}/api/status`,
    reuseExistingServer: false,
    timeout: 60_000,
    stdout: "pipe",
  },
});
