// The search log term page (admin-search-logs/term.gjs): a term from the
// search logs opens its page with the searches per day drawn as a bar
// chart, and the period and search type menus refilter it.
const { test, expect, login, sql, label } = require("./helpers");

test("a term's page charts its searches and refilters", async ({ page, context }) => {
  const term = label("term").replace(/[^a-z0-9]+/gi, "").toLowerCase();
  await sql(
    "INSERT INTO search_logs (term, ip_address, search_type, created_at) VALUES (:'p1', '10.0.0.1', 1, now()), (:'p1', '10.0.0.2', 2, now())",
    term,
  );
  await login(context, "admin");
  await page.goto("/admin/logs/search_logs");
  await page.locator(`.search-logs-list .term a:text-is("${term}")`).click();

  await expect(page).toHaveURL(new RegExp(`/admin/logs/search_logs/term\\?term=${term}`));
  await expect(page.locator(".admin-search-logs h2 a")).toHaveText(term);
  const chart = page.locator("canvas.search-log-term-chart");
  await expect(chart).toBeVisible();
  await expect.poll(() => page.evaluate(() => !!window.Chart && !!window.Chart.getChart(document.querySelector("canvas.search-log-term-chart")))).toBe(true);
  const total = () =>
    page.evaluate(() => {
      const c = window.Chart.getChart(document.querySelector("canvas.search-log-term-chart"));
      return c.data.datasets[0].data.reduce((a, b) => a + b, 0);
    });
  expect(await total()).toBe(2);

  // The search type menu: full page only counts one.
  await page.locator(".search-logs-filter summary").click();
  await page.locator('.search-logs-filter .select-kit-row[data-value="full_page"]').click();
  await expect(page).toHaveURL(/searchType=full_page/);
  await expect.poll(() => page.evaluate(() => !!window.Chart && !!window.Chart.getChart(document.querySelector("canvas.search-log-term-chart")))).toBe(true);
  expect(await total()).toBe(1);
  await expect(page.locator(".search-logs-filter .select-kit-selected-name")).toHaveText("Full page");
});
