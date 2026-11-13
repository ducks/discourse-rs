// The live stream's out-of-band rows (page.js's live-rows extension): a
// row the page does not hold is left out of the message, where htmx
// would log htmx:oobErrorNoTarget, and the rows it does hold are drawn.
const { test, expect, login, topic, label, member } = require("./helpers");

test("a live sidebar row the page lacks is dropped without an htmx error", async ({ page, context, session }) => {
  const m = member();
  await login(context, m.name);
  const errors = [];
  page.on("console", (message) => message.type() === "error" && /htmx/.test(message.text()) && errors.push(message.text()));
  await page.goto("/latest");
  const everything = page.locator("#d-sidebar li[data-list-item-name='everything']");
  await expect(everything).toBeVisible();
  // A category row gone from the page, as a tag row is that was not a
  // top tag when the page was drawn.
  const rows = page.locator("#sidebar-section-content-categories > li[data-category-id]");
  await expect(rows.first()).toBeVisible();
  const before = await rows.count();
  await rows.first().evaluate((row) => row.remove());

  // Someone's new topic redraws the member's tracked links.
  const title = label("Live row");
  await topic(await session("user1"), title, label("A topic that the sidebar counts"));
  await expect(page.locator(".topic-list-item, .show-more.has-topics, .show-more").filter({ hasText: /./ }).first()).toBeVisible();
  await expect(everything.locator(".sidebar-section-link-suffix, .sidebar-section-link-content-badge").first()).toBeVisible();
  await expect(rows).toHaveCount(before - 1);
  expect(errors).toEqual([]);
});
