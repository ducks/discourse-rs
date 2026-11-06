// Topic views (page-tracking in page.js): the first page's beacon and a
// page asked for in place both record the viewer's view of the topic
// (TopicViewItem.add through the request tracker), once per viewer.
const { test, expect, login, topic, sql, label } = require("./helpers");

function views(t, username) {
  return sql(
    "SELECT (SELECT views FROM topics WHERE id = :'p1'), \
            (SELECT count(*) FROM topic_views tv JOIN users u ON u.id = tv.user_id \
              WHERE tv.topic_id = :'p1' AND u.username = :'p2')",
    t.id,
    username,
  )[0].join(" ");
}

test("a topic opened as the first page counts the viewer's view once", async ({ page, context, session }) => {
  const title = label("Viewed first topic");
  const t = await topic(await session("user1"), title, label("A topic opened directly"));
  const before = Number(views(t, "user2").split(" ")[0]);
  await login(context, "user2");

  await page.goto(t.url);
  await expect(page.locator(".topic-post").first()).toBeVisible();
  await expect.poll(() => views(t, "user2")).toBe(`${before + 1} 1`);

  await page.reload();
  await expect(page.locator(".topic-post").first()).toBeVisible();
  // A second view the same day is not counted (topic_view_duration_hours).
  await page.waitForTimeout(500);
  expect(views(t, "user2")).toBe(`${before + 1} 1`);
});

test("a topic opened from the list counts the viewer's view", async ({ page, context, session }) => {
  const title = label("Viewed from list topic");
  const t = await topic(await session("user1"), title, label("A topic opened from the list"));
  const before = Number(views(t, "user0").split(" ")[0]);
  await login(context, "user0");
  await page.goto("/latest");

  const pageRequest = page.waitForRequest(
    (request) => request.url().includes(`/${t.id}`) && request.headers()["discourse-track-view"] === "true",
  );
  await page.locator(".topic-list-item a.title", { hasText: title }).click();
  const request = await pageRequest;
  expect(request.headers()["discourse-track-view-topic-id"]).toBe(String(t.id));
  await expect(page.locator(".topic-post").first()).toBeVisible();
  await expect.poll(() => views(t, "user0")).toBe(`${before + 1} 1`);
});
