// The browser suite. Run it with `make browser-test` (scripts/browser-test),
// which boots the server and sets BASE_URL; running `playwright test` by
// hand needs a server and BASE_URL of your own.
const { defineConfig } = require("@playwright/test");
const path = require("path");

const root = process.env.BROWSER_TEST_ROOT || path.resolve(__dirname, "../..");

module.exports = defineConfig({
  testDir: __dirname,
  testMatch: "*.spec.js",
  // One server and database for every spec: files run side by side,
  // the tests in a file in order. Each spec writes only rows it labels
  // with its own marker, so files don't see each other's writes.
  fullyParallel: false,
  workers: process.env.BROWSER_TEST_WORKERS ? Number(process.env.BROWSER_TEST_WORKERS) : 3,
  retries: 0,
  forbidOnly: !!process.env.CI,
  timeout: 60_000,
  expect: { timeout: 10_000 },
  reporter: process.env.CI ? [["list"], ["github"]] : "list",
  outputDir: path.join(root, "tmp/browser-test-results"),
  use: {
    baseURL: process.env.BASE_URL,
    // nix/playwright.nix: the full Chromium in its new headless mode.
    channel: "chromium",
    viewport: { width: 1400, height: 900 },
    colorScheme: "light",
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
  },
});
