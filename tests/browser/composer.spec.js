// Demo steps 6 to 8: the composer (composer.js) opened from a topic and
// from the topic list, the preview while writing, posting, and the post
// arriving on the page without a reload; the mention reaching the other
// member's header live.
const { test, expect, login, topic, sql, label } = require("./helpers");

test("a reply written in the composer appears without a reload and notifies the mention live", async ({
  page,
  context,
  session,
}) => {
  const marker = label("composer reply");
  const user0 = await session("user0");
  const t = await topic(user0, label("Composer topic for a reply"), label("A topic user1 replies to, in the browser suite"));
  await login(context, "user1");

  // user3, mentioned, waits on another page. Its unread notifications in
  // the seed are all low priority, so the avatar shows their count (a
  // high priority one, as user0 has, shows its icon instead).
  const user3 = await session("user3");
  const other = await user3.newPage();
  await other.goto("/latest");
  await expect(other.locator("#current-user")).toBeVisible();
  const unread = other.locator("#current-user .badge-notification.unread-notifications");
  const before = (await unread.count()) ? Number(await unread.innerText()) : 0;

  await page.goto(t.url);
  await expect(page.locator(".topic-post")).toHaveCount(1);
  await page.evaluate(() => (window.__noReload = true));
  await page.locator("#topic-footer-buttons .create").click();
  const editor = page.locator("#reply-control.open .d-editor-input");
  await expect(editor).toBeVisible();
  await expect(editor).toBeFocused();
  // composer-open on html, so the page can scroll clear of it (276).
  await expect(page.locator("html")).toHaveClass(/composer-open/);

  await editor.fill(`**${marker}** with _formatting_ for @user3 :tada:`);
  const preview = page.locator("#reply-control .d-editor-preview");
  await expect(preview.locator("strong")).toHaveText(marker);
  await expect(preview.locator("em")).toHaveText("formatting");
  await expect(preview.locator("img.emoji[title=':tada:'], img.emoji[alt=':tada:']")).toHaveCount(1);

  await page.locator("#reply-control .save-or-cancel .create").click();
  await expect(page.locator("#reply-control.open")).toHaveCount(0);
  await expect(page.locator("html")).not.toHaveClass(/composer-open/);

  const post = page.locator(".topic-post", { has: page.locator(".cooked strong", { hasText: marker }) });
  await expect(post).toBeVisible();
  await expect(page.locator(".topic-post")).toHaveCount(2);
  const cooked = post.locator(".cooked");
  await expect(cooked.locator("em")).toHaveText("formatting");
  await expect(cooked.locator("a.mention")).toHaveText("@user3");
  await expect(cooked.locator("img.emoji")).toHaveCount(1);
  expect(await page.evaluate(() => window.__noReload)).toBe(true);

  const rows = sql(
    "SELECT u.username, p.post_number FROM posts p JOIN users u ON u.id = p.user_id WHERE p.topic_id = :'p1' AND p.raw LIKE '%' || :'p2' || '%'",
    t.id,
    marker,
  );
  expect(rows).toEqual([["user1", "2"]]);

  // The mention, over the message bus: the unread count on user3's avatar,
  // and the notification in the user menu.
  await expect(unread).toHaveText(String(before + 1), { timeout: 20_000 });
  await other.locator("#toggle-current-user").click();
  await expect(other.locator(`.user-menu li.notification.mentioned a[href^='${t.url}']`)).toBeVisible();
});

test("a new topic from the topic list opens on its own page", async ({ page, context }) => {
  await login(context, "user1");
  const title = label("Browser suite new topic");
  await page.goto("/latest");
  await page.locator("#create-topic").click();
  const titleInput = page.locator("#reply-title");
  await expect(titleInput).toBeVisible();
  await expect(titleInput).toBeFocused();
  await titleInput.fill(title);
  await page.locator("#reply-control .d-editor-input").fill(`The first post of ${title}, with a [link](https://example.com).`);
  await expect(page.locator("#reply-control .d-editor-preview a[href='https://example.com']")).toHaveText("link");
  await page.locator("#reply-control .save-or-cancel .create").click();

  await expect(page).toHaveURL(/\/t\/[^/]+\/\d+/);
  await expect(page.locator("#topic-title h1")).toContainText(title);
  await expect(page.locator(".topic-post .cooked").first()).toContainText(`The first post of ${title}`);
  const rows = sql("SELECT count(*) FROM topics WHERE title = :'p1' AND archetype = 'regular'", title);
  expect(rows).toEqual([["1"]]);
});

test("discarding a written draft asks first, and keeps it when declined", async ({ page, context }) => {
  await login(context, "user1");
  const title = label("A draft to discard");
  await page.goto("/latest");
  await page.locator("#create-topic").click();
  await page.locator("#reply-title").fill(title);
  await page.locator("#reply-control .d-editor-input").fill("Some words that would be lost.");

  const questions = [];
  page.once("dialog", (d) => {
    questions.push(d.message());
    d.dismiss();
  });
  await page.locator("#reply-control .discard-button").click();
  await expect.poll(() => questions.length).toBe(1);
  await expect(page.locator("#reply-control.open")).toBeVisible();
  await expect(page.locator("#reply-title")).toHaveValue(title);

  page.once("dialog", (d) => {
    questions.push(d.message());
    d.accept();
  });
  await page.locator("#reply-control .discard-button").click();
  await expect(page.locator("#reply-control.open")).toHaveCount(0);
  expect(questions[1]).toBe(questions[0]);
  // Opened again, the composer starts empty: the draft went with it.
  await page.locator("#create-topic").click();
  await expect(page.locator("#reply-title")).toHaveValue("");
});
