import { defineConfig, devices } from "@playwright/test";
import { tmpdir } from "node:os";
import { join } from "node:path";

// The server under test: the release binary (CI) or any build given in AGENTTABLE_BIN.
// Two servers: one starts empty (a person builds their own configuration from the
// components), one starts with the purchasing sample installed.
const binary = process.env.AGENTTABLE_BIN ?? "../../target/release/agenttable";
const builderPort = Number(process.env.AGENTTABLE_PORT ?? 18080);
const samplePort = builderPort + 1;
const data = (port: number) => join(tmpdir(), `agenttable-e2e-${port}.json`);

const browser = {
  ...devices["Desktop Chrome"],
  viewport: { width: 1440, height: 900 },
  deviceScaleFactor: 2,
  trace: "off" as const,
  screenshot: "only-on-failure" as const,
};

export default defineConfig({
  testDir: "e2e",
  outputDir: "e2e/results",
  fullyParallel: false,
  workers: 1,
  // Each walk-through builds on its own earlier steps, so a failed step is not retried.
  retries: 0,
  timeout: 60_000,
  expect: { timeout: 10_000 },
  reporter: [["list"], ["html", { outputFolder: "e2e/report", open: "never" }]],
  projects: [
    { name: "build", testMatch: /build\.spec\.ts/, use: { ...browser, baseURL: `http://127.0.0.1:${builderPort}` } },
    { name: "sample", testMatch: /product\.spec\.ts/, use: { ...browser, baseURL: `http://127.0.0.1:${samplePort}` } },
  ],
  webServer: [
    {
      command: `"${binary}" --port ${builderPort} --data "${data(builderPort)}" --fresh`,
      url: `http://127.0.0.1:${builderPort}/api/status`,
      reuseExistingServer: false,
      timeout: 60_000,
      stdout: "pipe",
    },
    {
      command: `"${binary}" --port ${samplePort} --data "${data(samplePort)}" --fresh --demo`,
      url: `http://127.0.0.1:${samplePort}/api/status`,
      reuseExistingServer: false,
      timeout: 60_000,
      stdout: "pipe",
    },
  ],
});
