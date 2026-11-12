// The topic footer's grouped bookmarks menu (topic-bookmarks-menu.js,
// TopicBookmarksMenu): its post rows and their submenu (jump, delete),
// deleting the topic's bookmark, bookmarking the topic, clearing them all
// after asking, and editing through BookmarkModal. Each change is
// checked in the database and on the footer's button, drawn again.
const { test, expect, login, api, topic, sql, label } = require("./helpers");

async function setup(session, context, { topicBookmark }) {
  const user1 = await session("user1");
  const user0 = await session("user0");
  const t = await topic(user1, label("Bookmarks menu topic"), label("A topic for the bookmarks menu spec"), [
    { context: user0, raw: label("A reply by user0 for the bookmarks menu") },
  ]);
  await login(context, "admin");
  await api(context, "POST", "/bookmarks.json", {
    bookmarkable_id: String(t.posts[1]),
    bookmarkable_type: "Post",
    name: "Read later",
  });
  if (topicBookmark) {
    await api(context, "POST", "/bookmarks.json", { bookmarkable_id: String(t.id), bookmarkable_type: "Topic" });
  }
  return t;
}

function adminBookmarks(t) {
  return sql(
    "SELECT b.bookmarkable_type, b.name FROM bookmarks b WHERE b.user_id = 1 AND \
       ((b.bookmarkable_type = 'Topic' AND b.bookmarkable_id = :'p1') OR \
        (b.bookmarkable_type = 'Post' AND b.bookmarkable_id IN (SELECT id FROM posts WHERE topic_id = :'p1'))) \
     ORDER BY b.bookmarkable_type",
    t.id,
  );
}

const TRIGGER = "#topic-footer-buttons .bookmark-menu__trigger";
const MENU = ".topic-bookmarks-menu-content";

function row(page, id) {
  return page.locator(`.bookmark-menu__row[data-menu-option-id="${id}"] .bookmark-menu__row-btn`);
}

test("a post's row: its submenu jumps to the post and deletes its bookmark", async ({ page, context, session }) => {
  const t = await setup(session, context, { topicBookmark: true });
  await page.goto(t.url);
  const trigger = page.locator(TRIGGER);
  await expect(trigger).toHaveClass(/topic-bookmarks-menu-trigger/);
  await expect(trigger).toHaveText("Edit bookmarks");
  await expect(trigger).toHaveAttribute("title", "Edit bookmarks");

  await trigger.click();
  const menu = page.locator(MENU);
  await expect(menu).toBeVisible();
  const ids = await menu.locator(".bookmark-menu__row").evaluateAll((rows) => rows.map((r) => r.dataset.menuOptionId));
  expect(ids).toEqual(["post-2", "edit-topic-bookmark", "delete-topic-bookmark", "clear-all"]);
  await expect(menu.locator('[data-menu-option-id="post-2"] .bookmark-menu__row-description')).toHaveText("Read later");

  await row(page, "post-2").click();
  const submenu = page.locator(".topic-bookmark-post-submenu");
  await expect(submenu).toBeVisible();
  await expect(submenu.locator(".--jump .bookmark-menu__row-label")).toHaveText("Jump to #2");
  // The submenu is still being placed beside its row when it shows: a
  // click whose press and release straddle a move focuses the row
  // without clicking it, so click until the menus have closed.
  await expect(async () => {
    await row(page, "jump").click({ timeout: 2000 });
    await expect(menu).toHaveCount(0, { timeout: 2000 });
  }).toPass();
  await expect(submenu).toHaveCount(0);
  await expect(page).toHaveURL(new RegExp(`/t/[^/]+/${t.id}/2$`));

  await trigger.click();
  await row(page, "post-2").click();
  await row(page, "delete").click();
  await expect(page.locator(".fk-d-toast")).toContainText("Bookmark deleted!");
  await expect(menu).toHaveCount(0);
  await expect.poll(() => adminBookmarks(t)).toEqual([["Topic", ""]]);
  // The topic's bookmark alone: BookmarkMenu's button, and the post's
  // bookmark button drawn again.
  await expect(page.locator(TRIGGER)).toHaveClass(/bookmark-menu-trigger/);
  await expect(page.locator(TRIGGER)).toHaveText("Edit bookmark");
  await expect(page.locator(`article[data-post-id="${t.posts[1]}"] .post-action-menu__bookmark.bookmarked`)).toHaveCount(0);
});

test("the topic's bookmark: bookmarked, deleted, then all cleared", async ({ page, context, session }) => {
  const t = await setup(session, context, { topicBookmark: false });
  await page.goto(t.url);
  const trigger = page.locator(TRIGGER);
  await expect(trigger).toHaveText("Edit bookmark");
  await expect(trigger).toHaveClass(/--post-bookmarked-only/);
  await expect(trigger).toHaveAttribute("title", "You've bookmarked this. Read later");

  await trigger.click();
  const ids = await page
    .locator(`${MENU} .bookmark-menu__row`)
    .evaluateAll((rows) => rows.map((r) => r.dataset.menuOptionId));
  expect(ids).toEqual(["post-2", "bookmark-topic"]);
  await row(page, "bookmark-topic").click();
  await expect(page.locator(".fk-d-toast")).toContainText("Bookmarked!");
  await expect.poll(() => adminBookmarks(t)).toEqual([["Post", "Read later"], ["Topic", ""]]);
  await expect(page.locator(TRIGGER)).toHaveText("Edit bookmarks");
  await expect(page.locator(TRIGGER)).not.toHaveClass(/--post-bookmarked-only/);

  await page.locator(TRIGGER).click();
  await row(page, "delete-topic-bookmark").click();
  await expect(page.locator(".fk-d-toast").last()).toContainText("Bookmark deleted!");
  await expect.poll(() => adminBookmarks(t)).toEqual([["Post", "Read later"]]);
  await expect(page.locator(TRIGGER)).toHaveText("Edit bookmark");

  // Delete all bookmarks, with two: asked first.
  await api(context, "POST", "/bookmarks.json", { bookmarkable_id: String(t.id), bookmarkable_type: "Topic" });
  await page.reload();
  await page.locator(TRIGGER).click();
  await row(page, "clear-all").click();
  const dialog = page.locator("#dialog-holder");
  await expect(dialog).toContainText("Are you sure you want to delete all your bookmarks from this topic?");
  await dialog.locator(".dialog-footer .btn-primary").click();
  await expect.poll(() => adminBookmarks(t)).toEqual([]);
  await expect(page.locator(TRIGGER)).toHaveText("Bookmark");
  await expect(page.locator(TRIGGER)).not.toHaveClass(/bookmarked/);
  await expect(page.locator(`article[data-post-id="${t.posts[1]}"] .post-action-menu__bookmark.bookmarked`)).toHaveCount(0);
  expect(sql("SELECT bookmarked FROM topic_users WHERE user_id = 1 AND topic_id = :'p1'", t.id)).toEqual([["f"]]);
});

test("editing a bookmark opens BookmarkModal", async ({ page, context, session }) => {
  const t = await setup(session, context, { topicBookmark: true });
  await page.goto(t.url);
  const modal = page.locator("#bookmark-reminder-modal");
  await page.locator(TRIGGER).click();
  await row(page, "edit-topic-bookmark").click();
  await expect(page.locator(MENU)).toHaveCount(0);
  await expect(modal.locator("#discourse-modal-title")).toHaveText("Edit bookmark");
  await modal.locator("#bookmark-name").fill("The whole topic");
  await modal.locator("#save-bookmark").click();
  await expect(modal).toHaveCount(0);
  await expect.poll(() => adminBookmarks(t)).toEqual([["Post", "Read later"], ["Topic", "The whole topic"]]);

  await page.locator(TRIGGER).click();
  await row(page, "post-2").click();
  await expect(async () => {
    await row(page, "edit").click({ timeout: 2000 });
    await expect(page.locator(MENU)).toHaveCount(0, { timeout: 2000 });
  }).toPass();
  await expect(page.locator(".topic-bookmark-post-submenu")).toHaveCount(0);
  await expect(modal.locator("#bookmark-name")).toHaveValue("Read later");
  await modal.locator("#bookmark-name").fill("Read sooner");
  await modal.locator("#bookmark-name").press("Enter");
  await expect(modal).toHaveCount(0);
  await expect.poll(() => adminBookmarks(t)).toEqual([["Post", "Read sooner"], ["Topic", "The whole topic"]]);
});

test("on a phone the menu and the submenu are bottom sheets", async ({ page, context, session }) => {
  const t = await setup(session, context, { topicBookmark: true });
  await page.setViewportSize({ width: 400, height: 800 });
  const base = new URL(test.info().project.use.baseURL);
  await context.addCookies([{ name: "mobile_view", value: "1", domain: base.hostname, path: "/" }]);
  await page.goto(t.url);
  await page.locator(TRIGGER).click();
  const sheet = page.locator(".fk-d-menu-modal.topic-bookmarks-menu-content");
  await expect(sheet).toBeVisible();
  await sheet.locator('[data-menu-option-id="post-2"] .bookmark-menu__row-btn').click();
  const submenu = page.locator(".fk-d-menu-modal.topic-bookmark-post-submenu-content");
  await expect(submenu).toBeVisible();
  await expect(sheet).toHaveCount(0);
  await submenu.locator('[data-menu-option-id="delete"] .btn').click();
  await expect(page.locator(".fk-d-toast")).toContainText("Bookmark deleted!");
  await expect(submenu).toHaveCount(0);
  await expect.poll(() => adminBookmarks(t)).toEqual([["Topic", ""]]);
});

test("a post bookmarked from its menu: the footer counts it", async ({ page, context, session }) => {
  const t = await setup(session, context, { topicBookmark: false });
  await page.goto(t.url);
  await expect(page.locator(TRIGGER)).toHaveText("Edit bookmark");
  const post = page.locator(`article[data-post-id="${t.posts[0]}"]`);
  await post.locator(".post-action-menu__show-more").click();
  // BookmarkMenu bookmarks as it shows; the footer follows once it closes.
  await post.locator(".post-action-menu__bookmark").click();
  await expect(page.locator(".bookmark-menu-content .bookmark-menu__title")).toHaveText("Bookmarked!");
  await page.keyboard.press("Escape");
  await expect(post.locator(".post-action-menu__bookmark.bookmarked[data-bookmark]")).toBeVisible();
  await expect(page.locator(TRIGGER)).toHaveText("Edit bookmarks");
  await page.locator(TRIGGER).click();
  const ids = await page
    .locator(`${MENU} .bookmark-menu__row`)
    .evaluateAll((rows) => rows.map((r) => r.dataset.menuOptionId));
  expect(ids).toEqual(["post-1", "post-2", "bookmark-topic", "clear-all"]);
});
