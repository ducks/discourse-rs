// The edit composer's reply target (ROADMAP 196r6): ComposerActionTitle's
// reply indicator as a button, the options menu's "Reply to another post"
// and ChangeReplyTo's timeline (static/js/change-reply-to.js), and the
// target the edit saves. The reference's composer-reply-to-test.js, and
// the picker's effect on the reply.
const { test, expect, login, api, topic, sql, label, member } = require("./helpers");

const textarea = (page) => page.locator("#reply-control textarea.d-editor-input");
const modal = (page) => page.locator(".change-reply-to-modal");

// A topic by user1 with a reply by user2 and a reply to that one by a
// member of the test's own, logged in on the context, who edits it.
// Returns { url, posts, editing }.
async function replyChain(context, session) {
  const editor = member();
  sql("UPDATE user_options SET composition_mode = 0 WHERE user_id = :'p1'", editor.id);
  await login(context, editor.name);
  const user1 = await session("user1");
  const user2 = await session("user2");
  const t = await topic(user1, label("Reply target topic"), label("The first post of the reply target topic"), [
    { context: user2, raw: label("The reply by user2 that the editor replies to") },
  ]);
  const replied = await api(context, "POST", "/posts.json", {
    topic_id: String(t.id),
    reply_to_post_number: "2",
    raw: label("The reply whose target changes"),
  });
  return { url: t.url, posts: t.posts, editing: replied };
}

async function openEdit(page, chain) {
  await page.goto(chain.url);
  const post = page.locator(`article[data-post-id="${chain.editing.id}"]`);
  const showMore = post.locator(".post-action-menu__show-more");
  if (await showMore.isVisible()) {
    await showMore.click();
  }
  await post.locator(".post-action-menu__edit").click();
  await expect(textarea(page)).not.toHaveValue("");
}

test.describe("Composer - Reply target picker", () => {
  test("the reply indicator in the edit title is a clickable button", async ({ page, context, session }) => {
    const chain = await replyChain(context, session);
    await openEdit(page, chain);

    await expect(
      page.locator("button.composer-edit-reply-to"),
      "renders the reply target as a button inside the action title",
    ).toBeVisible();
    await expect(page.locator("button.composer-edit-reply-to .original-username")).toHaveText("user2");
  });

  test("the toolbar options menu exposes a 'Reply to another post' entry", async ({ page, context, session }) => {
    const chain = await replyChain(context, session);
    await openEdit(page, chain);
    await page.locator("#reply-control .toolbar-menu__options-trigger").click();

    await expect(page.locator("[data-name='change-reply-to']"), "the menu item is available for editors").toBeVisible();

    await page.locator("[data-name='change-reply-to']").click();

    await expect(page.locator(".change-reply-to-modal .timeline-footer-controls")).toHaveCount(1);
    await expect(page.locator(".change-reply-to-modal .timeline-footer-controls")).toBeHidden();
  });

  // The modal's effect on the reply: the post picked on the timeline is
  // the target the edit saves, and Remove leaves it none.
  test("a post picked on the timeline becomes the reply's target", async ({ page, context, session }) => {
    const chain = await replyChain(context, session);
    await openEdit(page, chain);
    await page.locator("button.composer-edit-reply-to").click();
    await expect(modal(page).locator(".change-reply-to-modal__hint")).toHaveText(
      "Scroll the timeline to find the post you'd like to reply to.",
    );
    const submit = modal(page).locator(".d-modal__footer .btn-primary");
    await expect(submit).toBeDisabled();

    // the timeline's start: the topic's first post
    await modal(page).locator(".timeline-container .start-date").click();
    await expect(modal(page).locator(".change-reply-to-modal__selection")).toHaveText("Selected: post #1 by @user1");
    await expect(submit).toBeEnabled();

    // its end: the post being edited, which can't be its own target
    await modal(page).locator(".timeline-container .now-date").click();
    await expect(modal(page).locator("#modal-alert")).toHaveText(
      "Choose a post earlier than the one you're editing.",
    );
    await expect(submit).toBeDisabled();

    await modal(page).locator(".timeline-container .start-date").click();
    await expect(submit).toBeEnabled();
    await submit.click();
    await expect(modal(page)).toHaveCount(0);
    await expect(page.locator("button.composer-edit-reply-to .original-username")).toHaveText("user1");

    const saved = page.waitForResponse((r) => r.url().includes(`/posts/${chain.editing.id}`) && r.request().method() === "PUT");
    await page.locator("#reply-control .save-or-cancel .create").click();
    expect((await saved).ok()).toBeTruthy();
    expect(
      sql("SELECT reply_to_post_number, reply_to_user_id = (SELECT id FROM users WHERE username = 'user1') FROM posts WHERE id = :'p1'", chain.editing.id),
    ).toEqual([["1", "t"]]);
  });

  test("Remove reply target leaves the reply without one", async ({ page, context, session }) => {
    const chain = await replyChain(context, session);
    await openEdit(page, chain);
    await page.locator("#reply-control .toolbar-menu__options-trigger").click();
    await page.locator("[data-name='change-reply-to']").click();
    await modal(page).locator(".d-modal__footer .btn-danger").click();
    await expect(modal(page)).toHaveCount(0);
    await expect(page.locator("button.composer-edit-reply-to")).toHaveCount(0);

    const saved = page.waitForResponse((r) => r.url().includes(`/posts/${chain.editing.id}`) && r.request().method() === "PUT");
    await page.locator("#reply-control .save-or-cancel .create").click();
    expect((await saved).ok()).toBeTruthy();
    expect(sql("SELECT reply_to_post_number IS NULL, reply_to_user_id IS NULL FROM posts WHERE id = :'p1'", chain.editing.id)).toEqual([
      ["t", "t"],
    ]);
  });
});
