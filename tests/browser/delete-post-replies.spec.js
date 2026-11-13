// Deleting a post from its menu (topic.js, the topic controller's
// deletePost): staff deleting a post with replies are asked about the
// replies (the delete replies dialog, GET /posts/:id/reply-ids.json and
// Post.deleteMany); a post without replies and a member's own go
// unasked. Each choice is checked in the database.
const { test, expect, login, api, topic, sql, label } = require("./helpers");

// A topic by user1 with a post by user0, a reply to it by user1, a reply
// to that by user0 and a post nobody replied to.
async function setup(session) {
  const user1 = await session("user1");
  const user0 = await session("user0");
  const t = await topic(user1, label("Delete replies topic"), label("A topic for the delete replies spec"), [
    { context: user0, raw: label("A post by user0 that gets replies") },
  ]);
  const reply = async (context, number, what) =>
    (
      await api(context, "POST", "/posts.json", {
        topic_id: String(t.id),
        raw: label(what),
        reply_to_post_number: String(number),
      })
    ).id;
  const post = t.posts[1];
  const direct = await reply(user1, 2, "A direct reply by user1");
  const nested = await reply(user0, 3, "A nested reply by user0");
  const alone = (await api(user1, "POST", "/posts.json", { topic_id: String(t.id), raw: label("A post without replies") }))
    .id;
  return { t, post, direct, nested, alone };
}

function article(page, id) {
  return page.locator(`article[data-post-id="${id}"]`);
}

// The topic's deleted posts and those their author removed.
function deleted(t) {
  return sql(
    "SELECT id FROM posts WHERE topic_id = :'p1' AND (deleted_at IS NOT NULL OR user_deleted) ORDER BY id",
    t.id,
  ).map(([id]) => Number(id));
}

async function clickDelete(page, id) {
  const post = article(page, id);
  const button = post.locator(".post-action-menu__delete");
  if (!(await button.isVisible())) {
    await post.locator(".post-action-menu__show-more").click();
  }
  await button.click();
}

const DIALOG = "#dialog-holder.dialog-container";

async function askedAboutReplies(page, labels) {
  const dialog = page.locator(DIALOG);
  await expect(dialog.locator(".dialog-header h3#dialog-title")).toHaveText(
    "Do you also want to delete the replies to this post?",
  );
  await expect(dialog.locator(".dialog-body")).toHaveCount(0);
  await expect(dialog.locator(".dialog-footer .btn .d-button-label")).toHaveText(labels);
  return dialog;
}

const CHOICES = ["Yes, and 1 direct reply", "Yes, and all 2 replies", "No, just this post", "Cancel"];

test("staff are asked about a post's replies, in Ember's words", async ({ page, context, session }) => {
  const { t, post } = await setup(session);
  await login(context, "admin");
  await page.goto(t.url);
  await clickDelete(page, post);
  const dialog = await askedAboutReplies(page, CHOICES);
  const classes = await dialog
    .locator(".dialog-footer .btn")
    .evaluateAll((buttons) => buttons.map((b) => b.className));
  expect(classes).toEqual(["btn btn-primary", "btn", "btn", "btn btn-flat"]);
  await expect(dialog.locator(".dialog-header .dialog-close")).toBeVisible();
  expect(deleted(t)).toEqual([]);
});

test("just this post deletes it alone", async ({ page, context, session }) => {
  const { t, post, direct, nested } = await setup(session);
  await login(context, "admin");
  await page.goto(t.url);
  await clickDelete(page, post);
  const dialog = await askedAboutReplies(page, CHOICES);
  await dialog.locator(".dialog-footer .btn", { hasText: "No, just this post" }).click();
  await expect(dialog).toHaveCount(0);
  await expect(article(page, post).locator(".post-action-menu__recover")).toBeAttached();
  await expect(article(page, direct).locator(".post-action-menu__recover")).toHaveCount(0);
  await expect(article(page, nested).locator(".post-action-menu__recover")).toHaveCount(0);
  expect(deleted(t)).toEqual([post]);
});

test("the direct replies go with the post, the nested stay", async ({ page, context, session }) => {
  const { t, post, direct, nested } = await setup(session);
  await login(context, "admin");
  await page.goto(t.url);
  await clickDelete(page, post);
  const dialog = await askedAboutReplies(page, CHOICES);
  const sent = page.waitForRequest((r) => r.method() === "DELETE" && r.url().endsWith("/posts/destroy_many"));
  await dialog.locator(".dialog-footer .btn-primary").click();
  expect((await sent).postData()).toBe(
    `post_ids%5B%5D=${post}&post_ids%5B%5D=${direct}&agree_with_first_reply_flag=true`,
  );
  await expect(dialog).toHaveCount(0);
  await expect(article(page, post).locator(".post-action-menu__recover")).toBeAttached();
  await expect(article(page, direct).locator(".post-action-menu__recover")).toBeAttached();
  await expect(article(page, nested).locator(".post-action-menu__recover")).toHaveCount(0);
  expect(deleted(t)).toEqual([post, direct]);
});

test("all the replies go with the post", async ({ page, context, session }) => {
  const { t, post, direct, nested, alone } = await setup(session);
  await login(context, "admin");
  await page.goto(t.url);
  await clickDelete(page, post);
  const dialog = await askedAboutReplies(page, CHOICES);
  await dialog.locator(".dialog-footer .btn", { hasText: "Yes, and all 2 replies" }).click();
  await expect(dialog).toHaveCount(0);
  for (const id of [post, direct, nested]) {
    await expect(article(page, id).locator(".post-action-menu__recover")).toBeAttached();
  }
  await expect(article(page, alone).locator(".post-action-menu__recover")).toHaveCount(0);
  expect(deleted(t)).toEqual([post, direct, nested]);
});

test("cancel, the close button and Escape delete nothing", async ({ page, context, session }) => {
  const { t, post } = await setup(session);
  await login(context, "admin");
  await page.goto(t.url);
  await clickDelete(page, post);
  let dialog = await askedAboutReplies(page, CHOICES);
  await dialog.locator(".dialog-footer .btn-flat").click();
  await expect(dialog).toHaveCount(0);
  await clickDelete(page, post);
  dialog = await askedAboutReplies(page, CHOICES);
  await dialog.locator(".dialog-close").click();
  await expect(dialog).toHaveCount(0);
  await clickDelete(page, post);
  await askedAboutReplies(page, CHOICES);
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  await expect(article(page, post).locator(".post-action-menu__delete")).toBeAttached();
  expect(deleted(t)).toEqual([]);
});

test("a post with direct replies only is not offered all of them", async ({ page, context, session }) => {
  const { t, direct, nested } = await setup(session);
  await login(context, "admin");
  await page.goto(t.url);
  await clickDelete(page, direct);
  const dialog = await askedAboutReplies(page, ["Yes, and 1 direct reply", "No, just this post", "Cancel"]);
  await dialog.locator(".dialog-footer .btn-primary").click();
  await expect(article(page, nested).locator(".post-action-menu__recover")).toBeAttached();
  expect(deleted(t)).toEqual([direct, nested]);
});

test("a post without replies is deleted unasked", async ({ page, context, session }) => {
  const { t, alone } = await setup(session);
  await login(context, "admin");
  await page.goto(t.url);
  await expect(article(page, alone).locator(".post-action-menu__delete[data-has-replies]")).toHaveCount(0);
  await clickDelete(page, alone);
  await expect(article(page, alone).locator(".post-action-menu__recover")).toBeAttached();
  await expect(page.locator(DIALOG)).toHaveCount(0);
  expect(deleted(t)).toEqual([alone]);
});

test("a post whose replies are gone is deleted unasked", async ({ page, context, session }) => {
  const { t, direct, nested } = await setup(session);
  await login(context, "admin");
  await page.goto(t.url);
  // The reply goes after the page drew its parent with a reply.
  await api(context, "DELETE", `/posts/${nested}`);
  const asked = page.waitForResponse((r) => r.url().endsWith(`/posts/${direct}/reply-ids.json`));
  await clickDelete(page, direct);
  expect(await (await asked).json()).toEqual([]);
  await expect(article(page, direct).locator(".post-action-menu__recover")).toBeAttached();
  await expect(page.locator(DIALOG)).toHaveCount(0);
  expect(deleted(t)).toEqual([direct, nested]);
});

test("a member deleting their own post with replies is not asked", async ({ page, context, session }) => {
  const { t, post } = await setup(session);
  await login(context, "user0");
  await page.goto(t.url);
  await expect(article(page, post).locator(".post-action-menu__delete[data-has-replies]")).toHaveCount(0);
  let asked = false;
  page.on("request", (r) => (asked = asked || r.url().includes("/reply-ids")));
  await clickDelete(page, post);
  // The author's delete marks the post (user_deleted) for the job to remove.
  await expect.poll(() => deleted(t)).toEqual([post]);
  expect(sql("SELECT user_deleted, deleted_at IS NULL FROM posts WHERE id = :'p1'", post)).toEqual([["t", "t"]]);
  await expect(page.locator(DIALOG)).toHaveCount(0);
  expect(asked).toBe(false);
});

test("staff deleting a small action with a reply are asked too", async ({ page, context, session }) => {
  const user1 = await session("user1");
  const user0 = await session("user0");
  const admin = await session("admin");
  const t = await topic(user1, label("Delete replies small action"), label("A topic whose small action gets a reply"));
  await api(admin, "PUT", `/t/${t.id}/status`, { status: "closed", enabled: "true" });
  await api(admin, "PUT", `/t/${t.id}/status`, { status: "closed", enabled: "false" });
  const [[action]] = sql("SELECT id FROM posts WHERE topic_id = :'p1' AND post_number = 2 AND post_type = 3", t.id);
  const answer = await api(user0, "POST", "/posts.json", {
    topic_id: String(t.id),
    raw: label("A reply to the small action"),
    reply_to_post_number: "2",
  });
  await login(context, "admin");
  await page.goto(t.url);
  await article(page, action).locator(".small-action-delete[data-has-replies]").click();
  const dialog = await askedAboutReplies(page, ["Yes, and 1 direct reply", "No, just this post", "Cancel"]);
  await dialog.locator(".dialog-footer .btn-primary").click();
  await expect(article(page, action).locator(".small-action-recover")).toBeAttached();
  await expect(article(page, answer.id).locator(".post-action-menu__recover")).toBeAttached();
  expect(deleted(t)).toEqual([Number(action), answer.id]);
});

test("a delete the server refuses is shown and deletes nothing", async ({ page, context, session }) => {
  const { t, post, nested } = await setup(session);
  // A reply with a pending flag: destroy_many refuses it (not ported).
  const user2 = await session("user2");
  await api(user2, "POST", "/post_actions", { id: String(nested), post_action_type_id: "4" });
  await login(context, "admin");
  await page.goto(t.url);
  await clickDelete(page, post);
  const dialog = await askedAboutReplies(page, CHOICES);
  const refused = page.waitForResponse((r) => r.url().endsWith("/posts/destroy_many"));
  await dialog.locator(".dialog-footer .btn", { hasText: "Yes, and all 2 replies" }).click();
  expect((await refused).status()).toBe(500);
  await expect(page.locator(`${DIALOG} .dialog-body p`)).toHaveText("Internal Server Error");
  await page.locator(`${DIALOG} .dialog-footer .btn-primary`).click();
  await expect(article(page, post).locator(".post-action-menu__delete")).toBeAttached();
  expect(deleted(t)).toEqual([]);
});
