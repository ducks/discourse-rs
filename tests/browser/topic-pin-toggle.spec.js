// TopicStatus' pin toggle for a member (static/js/topic-status.js,
// topic_list_view::topic_statuses): a.pin-toggle-button in a list row, at
// the topic's title and in the docked header clears the member's own pin
// (PUT /t/:id/clear-pin) and restores it (PUT /t/:id/re-pin), drawn for
// the new state in place; a list keeps its order until it loads again.
// Each change is checked in topic_users.cleared_pinned_at.
const { test, expect, login, topic, sql, label } = require("./helpers");

const PINNED = "This topic is pinned for you; it will display at the top of its category";
const UNPINNED = "This topic is unpinned for you; it will display in regular order";

// The topics this spec pinned, unpinned again for the specs after it.
const pinnedIds = [];
test.afterEach(() => {
  for (const id of pinnedIds.splice(0)) {
    sql("UPDATE topics SET pinned_at = NULL, pinned_globally = false WHERE id = :'p1'", id);
  }
});

// Two topics by user1, the older one pinned globally.
async function setup(session, context) {
  const user1 = await session("user1");
  const body = Array.from({ length: 40 }, (_, i) => label(`Paragraph ${i} of the pin toggle spec`)).join("\n\n");
  const pinned = await topic(user1, label("Pin toggle pinned"), body);
  const later = await topic(user1, label("Pin toggle later"), label("A later topic for the pin toggle spec"));
  sql("UPDATE topics SET pinned_at = now() - interval '1 minute', pinned_globally = true WHERE id = :'p1'", pinned.id);
  pinnedIds.push(pinned.id);
  await login(context, "user1");
  return { pinned, later };
}

function cleared(id) {
  return sql(
    "SELECT cleared_pinned_at IS NOT NULL FROM topic_users WHERE topic_id = :'p1' AND user_id = (SELECT id FROM users WHERE username = 'user1')",
    id
  );
}

// The rows' topic ids, in the list's order.
function order(page) {
  return page.locator(".topic-list tr.topic-list-item").evaluateAll((rows) => rows.map((r) => r.dataset.topicId));
}

async function before(page, first, second) {
  const ids = await order(page);
  expect(ids).toContain(String(first));
  expect(ids).toContain(String(second));
  return ids.indexOf(String(first)) < ids.indexOf(String(second));
}

async function expectPinned(toggle) {
  await expect(toggle).toHaveClass("topic-status --pinned pin-toggle-button");
  await expect(toggle).toHaveAttribute("title", PINNED);
  await expect(toggle.locator("svg.d-icon-thumbtack")).not.toHaveClass(/\bunpinned\b/);
}

async function expectUnpinned(toggle) {
  await expect(toggle).toHaveClass("topic-status --unpinned pin-toggle-button");
  await expect(toggle).toHaveAttribute("title", UNPINNED);
  await expect(toggle.locator("svg.d-icon-thumbtack")).toHaveClass(/\bunpinned\b/);
}

test("a member unpins and pins a topic again from its list row", async ({ page, context, session }) => {
  const t = await setup(session, context);
  await page.goto("/latest");
  const row = page.locator(`.topic-list tr.topic-list-item[data-topic-id="${t.pinned.id}"]`);
  const toggle = row.locator(".topic-statuses a.pin-toggle-button");
  await expectPinned(toggle);
  await expect(row).toHaveClass(/\bpinned\b/);
  await expect(row.locator("a.topic-excerpt")).toHaveCount(1);
  expect(await before(page, t.pinned.id, t.later.id)).toBe(true);

  // clearPin: drawn at once, the row where it was, less its excerpt.
  await toggle.click();
  await expectUnpinned(toggle);
  await expect(page).toHaveURL(/\/latest$/);
  await expect(row).not.toHaveClass(/\bpinned\b/);
  await expect(row).not.toHaveClass(/\bexcerpt-expanded\b/);
  await expect(row.locator("a.topic-excerpt")).toHaveCount(0);
  await expect.poll(() => cleared(t.pinned.id)).toEqual([["t"]]);
  expect(await before(page, t.pinned.id, t.later.id)).toBe(true);

  // rePin, before the list loads again: the excerpt is back.
  await toggle.click();
  await expectPinned(toggle);
  await expect(row).toHaveClass(/\bpinned\b/);
  await expect(row).toHaveClass(/\bexcerpt-expanded\b/);
  await expect(row.locator("a.topic-excerpt")).toHaveCount(1);
  await expect.poll(() => cleared(t.pinned.id)).toEqual([["f"]]);

  // Unpinned and loaded again: in the regular order, with the unpinned
  // toggle.
  await toggle.click();
  await expectUnpinned(toggle);
  await expect.poll(() => cleared(t.pinned.id)).toEqual([["t"]]);
  await page.reload();
  await expectUnpinned(toggle);
  await expect(row).not.toHaveClass(/\bpinned\b/);
  expect(await before(page, t.pinned.id, t.later.id)).toBe(false);

  // Pinned again from there, and held at the top once loaded.
  await toggle.click();
  await expectPinned(toggle);
  await expect.poll(() => cleared(t.pinned.id)).toEqual([["f"]]);
  expect(await before(page, t.pinned.id, t.later.id)).toBe(false);
  await page.reload();
  await expectPinned(toggle);
  expect(await before(page, t.pinned.id, t.later.id)).toBe(true);
});

test("the topic's title and the docked header toggle the same pin", async ({ page, context, session }) => {
  const t = await setup(session, context);
  await page.goto(t.pinned.url);
  const title = page.locator("#topic-title h1 .topic-statuses a.pin-toggle-button");
  await expectPinned(title);

  await title.click();
  await expectUnpinned(title);
  await expect(page).toHaveURL(new RegExp(`/${t.pinned.id}$`));
  await expect.poll(() => cleared(t.pinned.id)).toEqual([["t"]]);

  // Header::Topic::Info, docked once the title is out of view.
  await page.evaluate(() => window.scrollTo(0, document.body.scrollHeight));
  const header = page.locator(".d-header .extra-info-wrapper .topic-statuses a.pin-toggle-button");
  await expectUnpinned(header);
  await header.click();
  await expectPinned(header);
  await expectPinned(title);
  await expect(page).toHaveURL(new RegExp(`/${t.pinned.id}(/\\d+)?$`));
  await expect.poll(() => cleared(t.pinned.id)).toEqual([["f"]]);

  // A visitor's status is the span.
  const visitor = await session(null);
  const other = await visitor.newPage();
  await other.goto(t.pinned.url);
  await expect(other.locator("#topic-title h1 .topic-statuses span.topic-status.--pinned")).toHaveCount(1);
  await expect(other.locator("a.pin-toggle-button")).toHaveCount(0);
});

test("a failed request draws the topic pinned again", async ({ page, context, session }) => {
  const t = await setup(session, context);
  await page.route("**/clear-pin", (route) => route.fulfill({ status: 500, body: "" }));
  await page.goto("/latest");
  const row = page.locator(`.topic-list tr.topic-list-item[data-topic-id="${t.pinned.id}"]`);
  const toggle = row.locator(".topic-statuses a.pin-toggle-button");
  const failed = page.waitForResponse((r) => r.url().endsWith(`/t/${t.pinned.id}/clear-pin`));
  await toggle.click();
  await failed;
  await expectPinned(toggle);
  await expect(row).toHaveClass(/\bpinned\b/);
  await expect(row.locator("a.topic-excerpt")).toHaveCount(1);
});
