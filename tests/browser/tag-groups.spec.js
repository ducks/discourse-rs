// The tag groups pages (src/tags/groups_view.rs, static/js/tag-groups.js):
// a new group with a typed tag (TagChooser's allowAny), saved, edited to
// be usable only by a group, then deleted.
const { test, expect, login, label } = require("./helpers");

test("a tag group is created, restricted to a group and deleted", async ({ page, context }) => {
  await login(context, "admin");
  const name = label("Group");
  const tag = label("tg").replace(/\s+/g, "-").toLowerCase();

  await page.goto("/tag_groups/new");
  await expect(page.locator(".tag-group-content h3")).toHaveText("Create new group");
  await page.locator(".group-name input[name=name]").fill(name);

  const tags = page.locator('details[data-field="tags"]');
  await tags.locator("summary").click();
  await tags.locator(".filter-input").fill(tag);
  await tags.locator(`.select-kit-row[data-name="${tag}"]`).click();
  await expect(tags.locator("summary")).toHaveAttribute("data-name", tag);
  await page.locator("body").click({ position: { x: 1, y: 1 } });

  await page.locator("[data-tag-group-save]").click();
  await expect(page).toHaveURL(/\/tag_groups$/);
  const link = page.locator(".tag-groups-sidebar a", { hasText: name });
  await expect(link).toBeVisible();

  await link.click();
  await expect(page.locator(".tag-group-content h3")).toHaveText("Edit tag group");
  await expect(page.locator('details[data-field="tags"] summary')).toHaveAttribute("data-name", tag);
  await expect(page.locator("#public-permission")).toBeChecked();

  // Usable only by staff: the first GroupChooser, mirrored by the second.
  await page.locator("#visible-permission").check();
  const choosers = page.locator("details[data-group-ids]");
  await choosers.first().locator("summary").click();
  await choosers.first().locator(".filter-input").fill("staff");
  await choosers.first().locator('.select-kit-row[data-name="staff"]').click();
  await expect(choosers.nth(1).locator("summary")).toHaveAttribute("data-name", "staff");
  await page.locator("body").click({ position: { x: 1, y: 1 } });
  const saved = page.waitForResponse((r) => r.request().method() === "PUT" && r.url().includes("/tag_groups/"));
  await page.locator("[data-tag-group-save]").click();
  const json = await (await saved).json();
  expect(json.tag_group.permissions).toEqual({ 0: 3, 3: 1 });

  await page.reload();
  await expect(page.locator("#visible-permission")).toBeChecked();
  await page.locator("[data-tag-group-delete]").click();
  await page.locator(".dialog-footer .btn-primary").click();
  await expect(page).toHaveURL(/\/tag_groups$/);
  await expect(page.locator(".tag-groups-sidebar a", { hasText: name })).toHaveCount(0);
});

test("an empty name is refused before saving", async ({ page, context }) => {
  await login(context, "admin");
  await page.goto("/tag_groups/new");
  await page.locator(".group-name input[name=name]").fill("");
  await page.locator("[data-tag-group-save]").click();
  await expect(page.locator(".dialog-body")).toContainText("Make sure the tag group name is not empty");
});
