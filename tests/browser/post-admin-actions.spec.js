// The post admin menu's actions (topic-admin-menu.js): the staff color,
// the post notice modal (ChangePostNoticeModal), unhiding, locking and
// unlocking, wiki and rebuilding the HTML. Each is checked in the database
// and on the post drawn again; what stays unported is refused, named.
const { test, expect, login, topic, sql, label } = require("./helpers");

async function setup(session, context) {
  const user1 = await session("user1");
  const user0 = await session("user0");
  const t = await topic(user1, label("Post admin actions"), label("A topic for the post admin actions spec"), [
    { context: user0, raw: label("A reply by user0 for the post admin actions") },
  ]);
  await login(context, "admin");
  return { t, reply: t.posts[1] };
}

// The menu service's content, which DMenu's content class only a DMenu trigger gets.
const MENU = '.fk-d-menu[data-identifier="admin-post-menu"]';

function article(page, id) {
  return page.locator(`article[data-post-id="${id}"]`);
}

// The post's wrapper, which carries its classes.
function wrapper(page, id) {
  return article(page, id).locator("xpath=..");
}

async function openMenu(page, id) {
  const post = article(page, id);
  const wrench = post.locator(".show-post-admin-menu");
  if (!(await wrench.isVisible())) {
    await post.locator(".post-action-menu__show-more").click();
  }
  await wrench.click();
  await expect(page.locator(MENU)).toBeVisible();
}

// A menu button that opens a modal or is refused, drawing nothing again.
async function menuButton(page, id, name) {
  await openMenu(page, id);
  await page.locator(`${MENU} button.${name}`).click();
  await expect(page.locator(MENU)).toHaveCount(0);
}

async function postAction(page, id, name) {
  // Each action draws the post again (redrawPost): the article marked
  // here is gone once the redraw lands.
  await article(page, id).evaluate((el) => (el.dataset.stale = "1"));
  await menuButton(page, id, name);
  await expect(page.locator(`article[data-post-id="${id}"][data-stale]`)).toHaveCount(0);
}

function postRow(id, columns) {
  return sql(`SELECT ${columns} FROM posts WHERE id = :'p1'`, id)[0];
}

test("the staff color makes a moderator action and back", async ({ page, context, session }) => {
  const { t, reply } = await setup(session, context);
  await page.goto(t.url);
  await openMenu(page, reply);
  await expect(page.locator(`${MENU} button.toggle-post-type`)).toHaveText("Add Staff Color");
  await page.locator(`${MENU} button.toggle-post-type`).click();
  await expect.poll(() => postRow(reply, "post_type, version")).toEqual(["2", "2"]);
  // The post is drawn again as a moderator action.
  await expect(wrapper(page, reply)).toHaveClass(/\bmoderator\b/);

  await openMenu(page, reply);
  await expect(page.locator(`${MENU} button.toggle-post-type`)).toHaveText("Remove Staff Color");
  await expect(page.locator(`${MENU} button.toggle-post-type`)).toHaveClass(/btn-success/);
  await page.locator(`${MENU} button.toggle-post-type`).click();
  await expect.poll(() => postRow(reply, "post_type")).toEqual(["1"]);
  await expect(wrapper(page, reply)).toHaveClass(/\bregular\b/);
});

test("wiki and unwiki", async ({ page, context, session }) => {
  const { t, reply } = await setup(session, context);
  await page.goto(t.url);
  await postAction(page, reply, "wiki");
  await expect.poll(() => postRow(reply, "wiki")).toEqual(["t"]);
  await expect(wrapper(page, reply)).toHaveClass(/\bwiki\b/);
  expect(
    sql("SELECT modifications LIKE '%wiki%' FROM post_revisions WHERE post_id = :'p1' ORDER BY number DESC LIMIT 1", reply),
  ).toEqual([["t"]]);

  await openMenu(page, reply);
  await expect(page.locator(`${MENU} button.wiki`)).toHaveText("Remove Wiki");
  await page.locator(`${MENU} button.wiki`).click();
  await expect.poll(() => postRow(reply, "wiki")).toEqual(["f"]);
  await expect(wrapper(page, reply)).not.toHaveClass(/\bwiki\b/);
});

test("lock and unlock a post", async ({ page, context, session }) => {
  const { t, reply } = await setup(session, context);
  await page.goto(t.url);
  await postAction(page, reply, "lock-post");
  await expect.poll(() => postRow(reply, "locked_by_id IS NOT NULL")).toEqual(["t"]);
  await openMenu(page, reply);
  await expect(page.locator(`${MENU} button.unlock-post`)).toHaveText("Unlock Post");
  await page.locator(`${MENU} button.unlock-post`).click();
  await expect.poll(() => postRow(reply, "locked_by_id IS NULL")).toEqual(["t"]);
  expect(
    sql("SELECT action FROM user_histories WHERE post_id = :'p1' AND action IN (49, 50) ORDER BY id", reply),
  ).toEqual([["49"], ["50"]]);
});

test("unhide a hidden post", async ({ page, context, session }) => {
  const { t, reply } = await setup(session, context);
  sql("UPDATE posts SET hidden = TRUE, hidden_at = now(), hidden_reason_id = 1 WHERE id = :'p1'", reply);
  await page.goto(t.url);
  await expect(wrapper(page, reply)).toHaveClass(/post-hidden/);
  await postAction(page, reply, "unhide-post");
  await expect.poll(() => postRow(reply, "hidden")).toEqual(["f"]);
  await expect(wrapper(page, reply)).not.toHaveClass(/post-hidden/);
});

test("rebuild a post's HTML", async ({ page, context, session }) => {
  const { t, reply } = await setup(session, context);
  const [[cooked]] = sql("SELECT cooked FROM posts WHERE id = :'p1'", reply);
  sql("UPDATE posts SET cooked = '<p>stale</p>', baked_at = NULL WHERE id = :'p1'", reply);
  await page.goto(t.url);
  await postAction(page, reply, "rebuild-html");
  await expect.poll(() => postRow(reply, "baked_at IS NOT NULL")).toEqual(["t"]);
  expect(postRow(reply, "cooked")).toEqual([cooked]);
});

test("the post notice modal adds, changes and deletes the notice", async ({ page, context, session }) => {
  const { t, reply } = await setup(session, context);
  await page.goto(t.url);
  await menuButton(page, reply, "add-notice");
  const modal = page.locator(".d-modal.change-post-notice-modal");
  await expect(modal.locator(".d-modal__title-text")).toHaveText("Add Official Notice…");
  const save = modal.locator(".d-modal__footer .btn-primary");
  await expect(save).toBeDisabled();
  await expect(modal.locator(".d-modal__footer .btn-danger")).toHaveCount(0);
  const text = label("An official notice");
  await modal.locator("textarea").fill(text);
  await expect(save).toBeEnabled();
  await save.click();
  await expect(modal).toHaveCount(0);
  await expect
    .poll(() => sql("SELECT value::json->>'type', value::json->>'raw' FROM post_custom_fields WHERE post_id = :'p1' AND name = 'notice'", reply))
    .toEqual([["custom", text]]);
  // Drawn on the post, with who added it for staff.
  const notice = article(page, reply).locator(".post-notice.custom .post-notice-message");
  await expect(notice).toContainText(text);
  await expect(notice.locator(".custom_created_by a")).toHaveAttribute("data-user-card", "admin");

  await menuButton(page, reply, "change-notice");
  await expect(modal.locator(".d-modal__title-text")).toHaveText("Change Official Notice…");
  await expect(modal.locator("textarea")).toHaveValue(text);
  // Unchanged, it can't be saved.
  await expect(save).toBeDisabled();
  await modal.locator(".d-modal__footer .btn-danger").click();
  await expect(modal).toHaveCount(0);
  await expect
    .poll(() => sql("SELECT COUNT(*) FROM post_custom_fields WHERE post_id = :'p1' AND name = 'notice'", reply))
    .toEqual([["0"]]);
  await expect(article(page, reply).locator(".post-notice")).toHaveCount(0);
  expect(
    sql("SELECT action FROM user_histories WHERE post_id = :'p1' AND action IN (95, 96) ORDER BY id", reply),
  ).toEqual([["95"], ["96"]]);
});

test("the modals not ported stay refused, named", async ({ page, context, session }) => {
  const { t, reply } = await setup(session, context);
  await page.goto(t.url);
  await menuButton(page, reply, "change-owner");
  await expect(page.locator("#page-error")).toHaveText(
    "Not ported yet: the change ownership modal (ChangePostOwner, 194d1).",
  );
  await menuButton(page, reply, "grant-badge");
  await expect(page.locator("#page-error")).toHaveText("Not ported yet: the grant badge modal (GrantBadgeModal, 194d1).");
});
