// /categories for staff: the list controls' New Category button
// (navigation-bar.js) opens the category editor, as Ember's
// createCategory transitions to newCategory.
const { test, expect, login } = require("./helpers");

test("New Category opens the new category page", async ({ page, context }) => {
  await login(context, "admin");
  await page.goto("/categories");
  await page.locator("#create-category").click();
  await expect(page).toHaveURL(/\/new-category/);
  await expect(page.locator(".edit-category-page input[name=name]")).toBeVisible();
});

test("the latest category and tag buttons filter and navigate after page swaps", async ({ page, context }) => {
  await login(context, "user1");
  const response = await context.request.get("/tags.json");
  expect(response.ok()).toBeTruthy();
  const tag = (await response.json()).tags.find((row) => row.count > 0);
  expect(tag).toBeTruthy();
  await page.goto("/latest");
  const categories = page.locator(".category-breadcrumb .category-drop").first();
  await categories.locator("summary").click();
  await expect(categories.getByRole("link", { name: "General", exact: true })).toBeVisible();
  await expect(categories.getByRole("link", { name: "Staff", exact: true })).toHaveCount(0);
  await categories.getByRole("searchbox").fill("no-such-category-for-browser-check");
  await expect(categories.getByRole("status")).toHaveText("No results");
  await categories.getByRole("searchbox").fill("General");
  await categories.getByRole("link", { name: "General", exact: true }).click();
  await expect(page).toHaveURL(/\/c\/general\/4/);
  // htmx replaces the controls, so the delegated listener must handle the
  // new category drop as well as the original one on /latest.
  await page.locator(".category-breadcrumb .category-drop").first().locator("summary").click();
  await page.locator(".category-breadcrumb").getByRole("link", { name: "All categories", exact: true }).click();
  await expect(page).toHaveURL(/\/latest$/);
  const tags = page.locator(".category-breadcrumb .tag-drop");
  await tags.locator("summary").click();
  await tags.getByRole("searchbox").fill(tag.name);
  await tags.getByRole("searchbox").press("Escape");
  await expect(tags).not.toHaveAttribute("open", "");
  await expect(tags.locator("summary")).toBeFocused();
  await tags.locator("summary").click();
  await tags.getByRole("link", { name: tag.name, exact: true }).click();
  await expect(page).toHaveURL(new RegExp("/tag/[^/]+/" + tag.id));
  await expect(page.locator(".topic-list-item").first()).toBeVisible();
});

test("a category menu reports a failed load and retries", async ({ page, context }) => {
  await login(context, "user1");
  await page.goto("/latest");
  let first = true;
  await page.route("**/site.json", async (route) => {
    if (first) {
      first = false;
      await route.fulfill({ status: 503, body: "temporarily unavailable" });
    } else {
      await route.continue();
    }
  });
  const categories = page.locator(".category-breadcrumb .category-drop").first();
  await categories.locator("summary").click();
  await expect(categories).toContainText("Could not load categories (503)");
  await categories.getByRole("button", { name: "Retry" }).click();
  await expect(categories.getByRole("link", { name: "General", exact: true })).toBeVisible();
});
