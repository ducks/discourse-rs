// The review queue's client actions and editing (review.js): silencing
// a flagged post's author from the agree menu and suspending a queued
// post's author from the reject menu, both in the penalty modal
// (penalize-user.js) with the action performed first; agreeing with a
// flag by editing the post in the composer, staying on /review; editing
// a queued topic's title, category and raw in place (PUT /review/:id);
// a queued post's raw email (RawEmailModal). Each test penalizes or
// posts as a member of its own (helpers.js member()).
const { test, expect, login, api, topic, sql, label, member } = require("./helpers");

// A reply by a member of the test's own, flagged inappropriate by user0.
async function flaggedReply(session) {
  const author = member();
  const user1 = await session("user1");
  const authorContext = await session(author.name);
  const marker = label("A reply to act on");
  const t = await topic(user1, label("Client actions topic"), label("A topic for the client actions spec"), [
    { context: authorContext, raw: marker },
  ]);
  const postId = t.posts[1];
  const user0 = await session("user0");
  await api(user0, "POST", "/post_actions.json", { id: String(postId), post_action_type_id: "4", flag_topic: "false" });
  return { author, marker, postId };
}

// A new topic by a member of the test's own, held by a watched word.
async function queuedTopic(context, session) {
  const author = member();
  const word = label("rcaword").replace(" ", "");
  await api(context, "POST", "/admin/customize/watched_words.json", { "words[]": word, action_key: "require_approval" });
  const authorContext = await session(author.name);
  const title = label("Held topic for the client actions spec");
  const held = await api(authorContext, "POST", "/posts.json", {
    title,
    raw: `This topic mentions ${word}, which needs approval.`,
    category: "4",
  });
  expect(held.action).toBe("enqueued");
  const [[id]] = sql(
    "SELECT id FROM reviewables WHERE type = 'ReviewableQueuedPost' AND target_created_by_id = :'p1'",
    author.id,
  );
  return { author, word, title, id };
}

// The penalty modal: a custom reason and the first of the "until"
// shortcuts, then its submit.
async function penalize(page, type, reason) {
  const modal = page.locator(`.d-modal.${type}-user-modal`);
  await expect(modal).toBeVisible();
  await modal.locator(`input.${type}-reason`).fill(reason);
  // A post's penalty would delete it by default: do nothing to it.
  const postAction = modal.locator(".penalty-post-controls details summary");
  if (await postAction.count()) {
    await postAction.click();
    await page.locator(".penalty-post-controls .select-kit-row", { hasText: /Do nothing/i }).click();
  }
  await modal.locator(`.future-date-input.${type}-until details summary`).click();
  await page.locator(`.future-date-input.${type}-until .select-kit-row`).first().click();
  await modal.locator(".perform-penalize").click();
  await expect(modal).toHaveCount(0);
}

test("silencing a flagged post's author from the agree menu", async ({ page, context, session }) => {
  const { author, marker, postId } = await flaggedReply(session);
  await login(context, "admin");
  await page.goto("/review");
  const item = page.locator(".review-item.reviewable-flagged-post", { hasText: marker });
  await expect(item).toBeVisible();
  await item.locator("details summary", { hasText: /^\s*Yes/ }).first().click();
  await page.locator(".select-kit.is-expanded .select-kit-row", { hasText: /Silence user/ }).click();
  await penalize(page, "silence", label("Silenced from the queue"));

  // The flag agreed before the penalty, the silence logged for the review.
  await expect
    .poll(() => sql("SELECT status FROM reviewables WHERE target_type = 'Post' AND target_id = :'p1'", postId))
    .toEqual([["1"]]);
  await expect
    .poll(() => sql("SELECT silenced_till IS NOT NULL FROM users WHERE id = :'p1'", author.id))
    .toEqual([["t"]]);
  expect(
    sql(
      "SELECT count(*) FROM user_histories h JOIN reviewables r ON r.id = h.reviewable_id \
       WHERE h.target_user_id = :'p1' AND h.action = 30 AND r.target_id = :'p2'",
      author.id,
      postId,
    ),
  ).toEqual([["1"]]);
});

test("suspending a queued post's author from the reject menu", async ({ page, context, session }) => {
  await login(context, "admin");
  const queued = await queuedTopic(context, session);
  await page.goto("/review");
  const item = page.locator(`.review-item[data-reviewable-id="${queued.id}"]`);
  await expect(item).toBeVisible();
  await item.locator("details summary", { hasText: /^\s*No/ }).first().click();
  await page.locator(".select-kit.is-expanded .select-kit-row", { hasText: /Reject and suspend user/ }).click();
  await penalize(page, "suspend", label("Suspended from the queue"));

  await expect.poll(() => sql("SELECT status FROM reviewables WHERE id = :'p1'", queued.id)).toEqual([["2"]]);
  await expect
    .poll(() => sql("SELECT suspended_till IS NOT NULL FROM users WHERE id = :'p1'", queued.author.id))
    .toEqual([["t"]]);
  await expect(item.locator(".review-item__status.--rejected")).toBeVisible();
});

test("cancelling the penalty modal performs nothing", async ({ page, context, session }) => {
  const { author, marker, postId } = await flaggedReply(session);
  await login(context, "admin");
  await page.goto("/review");
  const item = page.locator(".review-item.reviewable-flagged-post", { hasText: marker });
  await item.locator("details summary", { hasText: /^\s*Yes/ }).first().click();
  await page.locator(".select-kit.is-expanded .select-kit-row", { hasText: /Suspend user/ }).click();
  const modal = page.locator(".d-modal.suspend-user-modal");
  await expect(modal).toBeVisible();
  await modal.locator(".d-modal-cancel").click();
  await expect(modal).toHaveCount(0);
  expect(sql("SELECT status FROM reviewables WHERE target_type = 'Post' AND target_id = :'p1'", postId)).toEqual([
    ["0"],
  ]);
  expect(sql("SELECT suspended_till IS NULL FROM users WHERE id = :'p1'", author.id)).toEqual([["t"]]);
});

test("agreeing with a flag by editing the post in the composer", async ({ page, context, session }) => {
  const { marker, postId } = await flaggedReply(session);
  await login(context, "admin");
  await page.goto("/review");
  const item = page.locator(".review-item.reviewable-flagged-post", { hasText: marker });
  await item.locator("details summary", { hasText: /^\s*Yes/ }).first().click();
  await page.locator(".select-kit.is-expanded .select-kit-row", { hasText: /Edit post/ }).click();

  const composer = page.locator("#reply-control.open");
  await expect(composer).toBeVisible();
  const textarea = composer.locator(".d-editor-input");
  await expect(textarea).toHaveValue(marker);
  const edited = label("Edited from the review queue");
  await textarea.fill(edited);
  await composer.locator(".save-or-cancel .create").click();
  await expect(page.locator("#reply-control.open")).toHaveCount(0);

  // Saved, then agreed with; the page stays the queue.
  await expect.poll(() => sql("SELECT raw FROM posts WHERE id = :'p1'", postId)).toEqual([[edited]]);
  await expect
    .poll(() => sql("SELECT status FROM reviewables WHERE target_type = 'Post' AND target_id = :'p1'", postId))
    .toEqual([["1"]]);
  await expect(page).toHaveURL(/\/review/);
});

test("editing a queued topic's title, category and raw in place", async ({ page, context, session }) => {
  await login(context, "admin");
  const queued = await queuedTopic(context, session);
  await page.goto("/review");
  const item = page.locator(`.review-item[data-reviewable-id="${queued.id}"]`);
  await expect(item).toBeVisible();

  // Cancel puts the reviewable back as it was.
  await item.locator("button.reviewable-action.edit").click();
  await expect(item.locator(".review-item__aside-title")).toHaveText("Editing post…");
  await expect(item.locator(".editable-fields")).toBeVisible();
  await item.locator("button.cancel-edit").click();
  await expect(item.locator(".editable-fields")).toHaveCount(0);
  await expect(item.locator(".review-item__post")).toContainText(queued.word);
  await expect(item.locator("button.reviewable-action.edit")).toBeVisible();

  await item.locator("button.reviewable-action.edit").click();
  const fields = item.locator(".editable-fields");
  const title = label("An edited queued title");
  await fields.locator('div[class="editable-field payload.title"] input.reviewable-input-text').fill(title);
  await fields.locator('div[class="editable-field payload.title"] input.reviewable-input-text').blur();
  await fields.locator(".editable-field.category-id details.category-chooser summary").click();
  await fields.locator('.editable-field.category-id .category-row[data-name="Site Feedback"]').click();
  await expect(fields.locator(".editable-field.category-id summary")).toHaveAttribute("data-value", "2");
  const raw = label("An edited queued body with **bold**");
  const editor = fields.locator('div[class="editable-field payload.raw"] .d-editor-input');
  await editor.fill(raw);
  await expect(fields.locator('div[class="editable-field payload.raw"] .d-editor-preview strong')).toHaveText("bold");
  await item.locator("button.save-edit").click();

  await expect(item.locator(".editable-fields")).toHaveCount(0);
  await expect(item).toContainText(title);
  await expect(item.locator(".review-item__post")).toContainText("An edited queued body with");
  expect(
    sql(
      "SELECT payload->>'title', payload->>'raw', category_id FROM reviewables WHERE id = :'p1'",
      queued.id,
    ),
  ).toEqual([[title, raw, "2"]]);
  expect(
    sql(
      "SELECT count(*) FROM reviewable_histories WHERE reviewable_id = :'p1' AND reviewable_history_type = 2",
      queued.id,
    ),
  ).toEqual([["1"]]);
});

test("a queued post by email shows its raw email", async ({ page, context, session }) => {
  await login(context, "admin");
  const queued = await queuedTopic(context, session);
  const rawEmail = `From: ${queued.author.name}@example.com\nSubject: ${queued.title}\n\nSent by email.`;
  sql(
    "UPDATE reviewables SET payload = payload::jsonb || jsonb_build_object('via_email', true, 'raw_email', :'p2'::text) \
     WHERE id = :'p1'",
    queued.id,
    rawEmail,
  );
  await page.goto("/review");
  const item = page.locator(`.review-item[data-reviewable-id="${queued.id}"]`);
  await item.locator("a.show-raw-email").click();
  const modal = page.locator(".d-modal.incoming-email-modal");
  await expect(modal.locator(".d-modal__title")).toContainText("Incoming Email");
  await expect(modal.locator(".incoming-email-tabs button.active")).toHaveText("Raw");
  await expect(modal.locator("pre.incoming-email-raw")).toHaveText(rawEmail);
});
