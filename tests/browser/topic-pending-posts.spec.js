// A topic's pending posts (templates/topic.gjs, post_view::pending_posts,
// topic.js): a member's reply sent for review shows PostEnqueued's modal
// and joins the pending posts under the stream without a reload
// (Composer#save), a reload draws it again from TopicView#pending_posts,
// and its Delete (deletePending, DELETE /review/:id) takes it off the
// page and marks the reviewable deleted. Staff read how many of the
// topic's posts wait. The reply is held by a watched word of the test's
// own, by a member of its own (helpers.js member()): the word's row is
// the site's, but holds only a post that has it.
const { test, expect, login, api, topic, sql, label, member } = require("./helpers");

const LIST = "#topic .pending-posts";
const ITEM = `${LIST} .reviewable-item`;
const MODAL = ".d-modal.post-enqueued-modal";

function queued(authorId) {
  return sql(
    "SELECT id, status, payload->>'raw' FROM reviewables WHERE type = 'ReviewableQueuedPost' AND target_created_by_id = :'p1' ORDER BY id",
    authorId,
  );
}

test("a reply sent for review is drawn under the stream, again after a reload, and its Delete removes it", async ({
  page,
  context,
  session,
}) => {
  const author = member();
  const word = label("tppword").replace(" ", "");
  const admin = await session("admin");
  await api(admin, "POST", "/admin/customize/watched_words.json", { "words[]": word, action_key: "require_approval" });
  const t = await topic(await session("user1"), label("Pending posts topic"), label("A topic for the pending posts spec"));
  await login(context, author.name);

  await page.goto(t.url);
  await expect(page.locator(".topic-post")).toHaveCount(1);
  await expect(page.locator(LIST)).toHaveCount(0);

  await page.locator("#topic-footer-buttons .create").click();
  const editor = page.locator("#reply-control.open .d-editor-input");
  await expect(editor).toBeFocused();
  const raw = `A reply with **${word}** that waits for approval.`;
  await editor.fill(raw);
  await expect(page.locator("#reply-control .d-editor-preview strong")).toHaveText(word);
  await page.locator("#reply-control .save-or-cancel .create").click();

  // PostEnqueued, over the page the pending post is drawn on.
  const modal = page.locator(MODAL);
  await expect(modal).toBeVisible();
  await expect(modal.locator(".d-modal__body")).toContainText("1 post pending");
  await expect(page.locator("#reply-control.open")).toHaveCount(0);
  const item = page.locator(ITEM);
  await expect(item).toHaveCount(1);
  const rows = queued(author.id);
  expect(rows).toEqual([[rows[0][0], "0", raw]]);
  const id = rows[0][0];
  await expect(item).toHaveAttribute("data-reviewable-id", id);
  await modal.locator(".d-modal__footer .btn-primary").click();
  await expect(modal).toHaveCount(0);

  const drawn = async () => {
    await expect(item).toHaveCount(1);
    await expect(item).toHaveAttribute("data-reviewable-id", id);
    await expect(item.locator(".reviewable-meta-data .reviewable-type")).toHaveText("Awaiting Approval");
    await expect(item.locator(".reviewable-meta-data .created-at .relative-date")).toHaveCount(1);
    await expect(item.locator(".created-by a[data-user-card] img.avatar")).toHaveAttribute("width", "48");
    await expect(item.locator(".post-contents .names .username a")).toHaveText(author.name);
    await expect(item.locator(".post-body strong")).toHaveText(word);
    const button = item.locator(".reviewable-actions button.btn-danger");
    await expect(button).toHaveText(/^\s*Delete\s*$/);
    await expect(button.locator("svg.d-icon-trash-can")).toHaveCount(1);
    // Not a post of the stream, and first under it.
    await expect(page.locator(".topic-post")).toHaveCount(1);
    await expect(page.locator(`${LIST} ~ .topic-timer-info`)).toHaveCount(1);
    // topic.scss's width, reviewables.scss's item.
    await expect(page.locator(LIST)).not.toHaveCSS("max-width", "none");
  };
  await drawn();

  // TopicView#pending_posts: the page draws it on its own.
  await page.reload();
  await expect(page.locator(".topic-post")).toHaveCount(1);
  await drawn();

  // The author's alone: another member sees none, staff the count.
  const other = await (await session("user1")).newPage();
  await other.goto(t.url);
  await expect(other.locator(".topic-post")).toHaveCount(1);
  await expect(other.locator(LIST)).toHaveCount(0);
  await expect(other.locator("#topic .has-pending-posts")).toHaveCount(0);
  await other.close();
  const staff = await admin.newPage();
  await staff.goto(t.url);
  await expect(staff.locator(LIST)).toHaveCount(0);
  const notice = staff.locator("#topic .has-pending-posts");
  await expect(notice.locator("div")).toHaveText("This topic has 1 post pending approval");
  await expect(notice.locator("a")).toHaveText("view pending");
  await expect(notice.locator("a")).toHaveAttribute("href", `/review?type=ReviewableQueuedPost&topic_id=${t.id}`);
  await staff.close();

  // deletePending: no confirmation, the post and its container gone.
  await item.locator(".reviewable-actions button.btn-danger").click();
  await expect(page.locator(LIST)).toHaveCount(0);
  // Reviewable.statuses[:deleted]
  await expect.poll(() => queued(author.id)).toEqual([[id, "4", raw]]);
  await page.reload();
  await expect(page.locator(".topic-post")).toHaveCount(1);
  await expect(page.locator(LIST)).toHaveCount(0);
});
