// A topic's print view (/t/:slug/:id/print): the crawler layout's posts,
// then js/print-page opens the print dialog once the page has loaded.
const { test, expect } = require("./helpers");

test("the print view opens the print dialog", async ({ page }) => {
  await page.addInitScript(() => {
    window.__printed = 0;
    window.print = () => {
      window.__printed += 1;
    };
  });
  await page.goto("/t/parity-fixture-replies-and-posters/35/print");
  await expect(page.locator("body.crawler #main-outlet .crawler-post")).toHaveCount(5);
  await expect.poll(() => page.evaluate(() => window.__printed)).toBe(1);
});
