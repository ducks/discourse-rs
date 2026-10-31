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
