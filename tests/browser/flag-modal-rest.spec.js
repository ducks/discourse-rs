// The flag modal's other buttons (flag-modal.js): an official warning,
// queueing a post for review (restored from /review; a first post's
// topic unlisted meanwhile), a flag inside a message, Delete Spammer
// with the email looked up (moderators_view_emails) and, over a chat
// message, take action through the penalty modal. Each test writes in a
// topic, message or channel of its own and flags members of its own
// (helpers.js member()), and reads what it wrote from the database.
const { test, expect, login, api, topic, sql, label, member } = require("./helpers");

function article(page, id) {
  return page.locator(`article[data-post-id="${id}"]`);
}

// The flag modal over a post, its opening animation done.
async function openFlag(page, id) {
  const post = article(page, id);
  const flag = post.locator(".post-action-menu__flag");
  if (!(await flag.isVisible())) {
    await post.locator(".post-action-menu__show-more").click();
  }
  await flag.click();
  const modal = page.locator(".d-modal:has(.flag-modal-body)");
  await expect(modal).toBeVisible();
  await modal.evaluate((el) => Promise.all(el.getAnimations({ subtree: true }).map((a) => a.finished)));
  return modal;
}

// A click that flags: the modal closes and the post is drawn again (the
// article marked here is gone once the redraw lands).
async function flagWith(page, id, modal, button) {
  await article(page, id).evaluate((el) => (el.dataset.stale = "1"));
  await modal.locator(button).click();
  await expect(modal).toHaveCount(0);
  await expect(page.locator(`article[data-post-id="${id}"][data-stale]`)).toHaveCount(0);
}

// A topic by user1 with a reply by a member of the test's own.
async function repliedTopic(session, what) {
  const author = member();
  const t = await topic(await session("user1"), label(what), label(`A topic for ${what}`), [
    { context: await session(author.name), raw: label(`A reply for ${what}`) },
  ]);
  return { author, t, reply: t.posts[1] };
}

test("staff send a message as an official warning", async ({ page, context, session }) => {
  const { author, t, reply } = await repliedTopic(session, "an official warning");
  await login(context, "admin");
  await page.goto(t.url);
  const modal = await openFlag(page, reply);
  // Only for a message to the user.
  await modal.locator("#radio_inappropriate").check();
  await expect(modal.locator(".flag-modal__send-warning")).toHaveCount(0);
  await modal.locator("#radio_notify_user").check();
  const warn = modal.locator(".flag-modal__send-warning");
  await expect(warn).toBeVisible();
  await expect(warn).toBeDisabled();
  const text = label("Please keep to the subject of the topic");
  await modal.locator("textarea.flag-message").fill(text);
  await expect(warn).toBeEnabled();
  await flagWith(page, reply, modal, ".flag-modal__send-warning");

  const [[topicId, createdBy, subtype, archetype]] = sql(
    "SELECT w.topic_id, w.created_by_id, t.subtype, t.archetype FROM user_warnings w \
     JOIN topics t ON t.id = w.topic_id WHERE w.user_id = :'p1'",
    author.id,
  );
  expect([createdBy, subtype, archetype]).toEqual(["1", "moderator_warning", "private_message"]);
  expect(
    sql(
      "SELECT p.topic_id, p.raw LIKE '%' || :'p2' || '%' FROM post_actions a JOIN posts p ON p.id = a.related_post_id \
       WHERE a.post_id = :'p1' AND a.user_id = 1 AND a.post_action_type_id = 6",
      reply,
      text,
    ),
  ).toEqual([[topicId, "t"]]);
  expect(
    sql("SELECT user_id FROM topic_allowed_users WHERE topic_id = :'p1' AND user_id <> 1", topicId),
  ).toEqual([[String(author.id)]]);
});

test("a member is offered no warning, take action or queue for review", async ({ page, context, session }) => {
  const { t, reply } = await repliedTopic(session, "a member's flag modal");
  const flagger = member();
  await login(context, flagger.name);
  await page.goto(t.url);
  const modal = await openFlag(page, reply);
  await modal.locator("#radio_notify_user").check();
  await expect(modal.locator(".flag-modal__create-flag")).toBeVisible();
  await expect(modal.locator(".flag-modal__send-warning")).toHaveCount(0);
  await modal.locator("#radio_inappropriate").check();
  await expect(modal.locator(".flag-modal__create-flag")).toBeEnabled();
  await expect(modal.locator(".flag-modal__flag-for-review")).toHaveCount(0);
  await expect(modal.locator(".reviewable-action-dropdown")).toHaveCount(0);
  await page.keyboard.press("Escape");
  await expect(modal).toHaveCount(0);
  expect(sql("SELECT count(*) FROM post_actions WHERE post_id = :'p1'", reply)).toEqual([["0"]]);
});

// The topic's queue in /review, and the item of the post with this text.
async function reviewItem(page, t, text) {
  await page.goto(`/review?topic_id=${t.id}`);
  const item = page.locator(".review-item.reviewable-flagged-post", { hasText: text });
  await expect(item).toBeVisible();
  return item;
}

// The No bundle's Restore post, then the item drawn again as rejected.
async function restoreFromReview(page, item) {
  await item.locator("details summary", { hasText: /^\s*No/ }).first().click();
  const row = page.locator(".select-kit.is-expanded .select-kit-row", { hasText: /Restore post/ });
  // A hidden post's No bundle: restore or delete, no keeping or ignoring.
  await expect(page.locator(".select-kit.is-expanded .select-kit-row", { hasText: /Keep post|Ignore/ })).toHaveCount(0);
  await row.first().click();
  await expect(item.locator("*", { hasText: /^\s*Flag rejected\s*$/ }).first()).toBeVisible();
}

test("staff queue a reply for review and restore it from /review", async ({ page, context, session }) => {
  const { author, t, reply } = await repliedTopic(session, "queueing for review");
  await login(context, "admin");
  await page.goto(t.url);
  let modal = await openFlag(page, reply);
  const queue = modal.locator(".flag-modal__flag-for-review");
  await expect(queue).toBeDisabled();
  await modal.locator("#radio_inappropriate").check();
  await expect(queue).toBeEnabled();
  await flagWith(page, reply, modal, ".flag-modal__flag-for-review");

  // Hidden at once (flag_threshold_reached), waiting in the queue.
  expect(sql("SELECT hidden, hidden_reason_id FROM posts WHERE id = :'p1'", reply)).toEqual([["t", "1"]]);
  expect(
    sql(
      "SELECT a.post_action_type_id, a.staff_took_action, r.status, s.reason FROM post_actions a \
       JOIN reviewables r ON r.target_type = 'Post' AND r.target_id = a.post_id \
       JOIN reviewable_scores s ON s.reviewable_id = r.id WHERE a.post_id = :'p1' AND a.user_id = 1",
      reply,
    ),
  ).toEqual([["4", "f", "0", "queued_by_staff"]]);
  expect(sql("SELECT visible FROM topics WHERE id = :'p1'", t.id)).toEqual([["t"]]);
  await expect(article(page, reply).locator("xpath=..")).toHaveClass(/post-hidden/);

  // /review: the hidden post is restored (disagree_and_restore).
  const item = await reviewItem(page, t, "A reply for queueing for review");
  await restoreFromReview(page, item);
  expect(
    sql(
      "SELECT p.hidden, p.inappropriate_count, r.status, s.status, a.disagreed_by_id, a.disagreed_at IS NOT NULL \
       FROM posts p JOIN reviewables r ON r.target_type = 'Post' AND r.target_id = p.id \
       JOIN reviewable_scores s ON s.reviewable_id = r.id JOIN post_actions a ON a.post_id = p.id \
       WHERE p.id = :'p1'",
      reply,
    ),
  ).toEqual([["f", "0", "2", "2", "1", "t"]]);
  // Its author is told (flags_disagreed, once the job has run), and
  // anyone reads it on the topic again.
  await expect
    .poll(
      () =>
        sql(
          "SELECT count(*) FROM topics t JOIN topic_allowed_users a ON a.topic_id = t.id \
           WHERE t.subtype = 'system_message' AND t.title = 'Flagged post restored by staff' AND a.user_id = :'p1'",
          author.id,
        ),
      { timeout: 20000 },
    )
    .toEqual([["1"]]);
  await page.goto(t.url);
  await expect(article(page, reply)).toBeVisible();
  await expect(article(page, reply).locator("xpath=..")).not.toHaveClass(/post-hidden/);
  await expect(article(page, reply).locator(".cooked")).toContainText("A reply for queueing for review");
});

test("staff queue a first post for review: the topic unlisted, listed again when restored", async ({ page, context, session }) => {
  const { t } = await repliedTopic(session, "queueing a first post");
  const first = t.posts[0];
  await login(context, "admin");
  await page.goto(t.url);
  const modal = await openFlag(page, first);
  await modal.locator("#radio_inappropriate").check();
  await modal.locator(".flag-modal__flag-for-review").click();
  await expect(modal).toHaveCount(0);
  // A first post's row is drawn again, not its wrapper (the topic map's):
  // it shows as hidden once the page is loaded again.
  await expect(article(page, first)).toContainText("You flagged this as inappropriate");
  await page.goto(t.url);
  await expect(article(page, first).locator("xpath=..")).toHaveClass(/post-hidden/);

  // Hidden, its topic unlisted by the flagger (op_flag_threshold_reached).
  expect(
    sql(
      "SELECT p.hidden, t.visible, t.visibility_reason_id FROM posts p JOIN topics t ON t.id = p.topic_id WHERE p.id = :'p1'",
      first,
    ),
  ).toEqual([["t", "f", "0"]]);
  expect(
    sql("SELECT user_id, action_code FROM posts WHERE topic_id = :'p1' AND post_type = 3 ORDER BY id", t.id),
  ).toEqual([["1", "visible.disabled"]]);

  const item = await reviewItem(page, t, "A topic for queueing a first post");
  await restoreFromReview(page, item);
  // Listed again by the system user (op_unhidden).
  expect(
    sql(
      "SELECT p.hidden, t.visible, t.visibility_reason_id, r.status FROM posts p JOIN topics t ON t.id = p.topic_id \
       JOIN reviewables r ON r.target_type = 'Post' AND r.target_id = p.id WHERE p.id = :'p1'",
      first,
    ),
  ).toEqual([["f", "t", "1", "2"]]);
  expect(
    sql("SELECT user_id, action_code FROM posts WHERE topic_id = :'p1' AND post_type = 3 ORDER BY id", t.id),
  ).toEqual([
    ["1", "visible.disabled"],
    ["-1", "visible.enabled"],
  ]);
  await page.goto(t.url);
  await expect(article(page, first).locator("xpath=..")).not.toHaveClass(/post-hidden/);
  await expect(article(page, first).locator(".cooked")).toContainText("A topic for queueing a first post");
});

test("a post in a message is flagged", async ({ page, context, session }) => {
  const sender = member();
  const flagger = member();
  const first = await api(await session(sender.name), "POST", "/posts.json", {
    title: label("A message to flag"),
    raw: label("A message for the flag modal spec"),
    archetype: "private_message",
    target_recipients: flagger.name,
  });
  await login(context, flagger.name);
  await page.goto(`/t/${first.topic_slug}/${first.topic_id}`);
  const modal = await openFlag(page, first.id);
  await modal.locator("#radio_inappropriate").check();
  await modal.locator(".flag-modal__create-flag").click();
  await expect(modal).toHaveCount(0);
  // Flagged once: the button is gone from the post drawn again.
  await expect(article(page, first.id).locator(".post-action-menu__flag")).toHaveCount(0);

  expect(
    sql(
      "SELECT a.user_id, a.post_action_type_id, r.type, r.status, r.topic_id, r.category_id IS NULL \
       FROM post_actions a JOIN reviewables r ON r.target_type = 'Post' AND r.target_id = a.post_id \
       WHERE a.post_id = :'p1'",
      first.id,
    ),
  ).toEqual([[String(flagger.id), "4", "ReviewableFlaggedPost", "0", String(first.topic_id), "t"]]);
});

test("delete spammer looks the email up where staff may see it", async ({ page, context, session }) => {
  const { author, t, reply } = await repliedTopic(session, "delete spammer");
  await login(context, "admin");
  const [[before]] = sql("SELECT COALESCE((SELECT value FROM site_settings WHERE name = 'moderators_view_emails'), 'f')");
  await api(context, "PUT", "/admin/site_settings/moderators_view_emails", { moderators_view_emails: "true" });
  try {
    await page.goto(t.url);
    const modal = await openFlag(page, reply);
    await modal.locator("#radio_spam").check();
    const button = modal.locator(".flag-modal__delete-spammer");
    await expect(button).toBeVisible();
    await button.click();
    const dialog = page.locator(".dialog-container.flagging-delete-spammer");
    await expect(dialog.locator(".dialog-body")).toContainText(`${author.name}@example.com`);
    // The lookup is logged with the page it came from (check_email).
    expect(
      sql(
        "SELECT count(*) FROM user_histories WHERE target_user_id = :'p1' AND acting_user_id = 1 AND context = :'p2'",
        author.id,
        new URL(page.url()).pathname,
      ),
    ).toEqual([["1"]]);
    await dialog.locator(".dialog-footer .btn-default").click();
    await expect(dialog).toHaveCount(0);
    expect(sql("SELECT count(*) FROM users WHERE id = :'p1'", author.id)).toEqual([["1"]]);
  } finally {
    await api(context, "PUT", "/admin/site_settings/moderators_view_emails", {
      moderators_view_emails: before === "t" ? "true" : "false",
    });
  }
});

test("staff take action on a chat message and suspend its author", async ({ page, context, session }) => {
  const author = member();
  const admin = await session("admin");
  const name = label("flag-rest").replace(/\s+/g, "-").slice(0, 40);
  const created = await api(admin, "POST", "/chat/api/channels.json", {
    "channel[chatable_id]": "2",
    "channel[name]": name,
  });
  const channel = created.channel;
  const authorContext = await session(author.name);
  await api(authorContext, "POST", `/chat/api/channels/${channel.id}/memberships/me.json`);
  const text = label("chat message to take action on");
  await api(authorContext, "POST", `/chat/${channel.id}.json`, { staged_id: label("staged"), message: text });
  const [[messageId]] = sql(
    "SELECT id FROM chat_messages WHERE chat_channel_id = :'p1' AND message = :'p2'",
    channel.id,
    text,
  );

  await login(context, "admin");
  await page.goto(`/chat/c/${channel.slug}/${channel.id}`);
  const message = page.locator(".chat-message-container", { hasText: text });
  await expect(message).toBeVisible();
  await expect(async () => {
    await message.hover();
    await message.locator(".chat-message-actions .more-actions-chat summary").click({ timeout: 2000 });
    await message.locator(".chat-message-actions .select-kit-row[data-value='flag']").click({ timeout: 2000 });
  }).toPass({ timeout: 15_000 });
  const modal = page.locator(".d-modal:has(.flag-modal-body)");
  await expect(modal).toBeVisible();
  await modal.evaluate((el) => Promise.all(el.getAnimations({ subtree: true }).map((a) => a.finished)));
  // Chat's flags leave these two unported, named.
  await modal.locator("#radio_notify_user").check();
  await modal.locator("textarea.flag-message").fill(label("A message to the author of the chat message"));
  await modal.locator(".flag-modal__send-warning").click();
  await expect(page.locator(".dialog-body")).toHaveText("Not ported yet: official warnings for chat messages.");
  await page.locator(".dialog-footer .btn").click();
  await expect(page.locator(".dialog-body")).toHaveCount(0);
  await modal.locator("#radio_inappropriate").check();
  await modal.locator(".flag-modal__flag-for-review").click();
  await expect(page.locator(".dialog-body")).toHaveText("Not ported yet: queueing a chat message for review.");
  await page.locator(".dialog-footer .btn").click();
  await expect(page.locator(".dialog-body")).toHaveCount(0);

  await modal.locator(".reviewable-action-dropdown summary").click();
  await modal.locator(".reviewable-action-dropdown .select-kit-row[data-value='agree_and_suspend']").click();
  const penalty = page.locator(".d-modal.suspend-user-modal");
  await expect(penalty).toBeVisible();
  await penalty.locator("input.suspend-reason").fill(label("Suspended from a chat flag"));
  // No post to act on: a chat message is not editable.
  await expect(penalty.locator(".penalty-post-controls")).toHaveCount(0);
  await penalty.locator(".future-date-input.suspend-until details summary").click();
  await page.locator(".future-date-input.suspend-until .select-kit-row").first().click();
  await penalty.locator(".perform-penalize").click();
  await expect(penalty).toHaveCount(0);

  // The flag agreed with at once (the message deleted), then the penalty.
  await expect
    .poll(() => sql("SELECT status FROM reviewables WHERE target_type = 'ChatMessage' AND target_id = :'p1'", messageId))
    .toEqual([["1"]]);
  expect(sql("SELECT deleted_at IS NOT NULL FROM chat_messages WHERE id = :'p1'", messageId)).toEqual([["t"]]);
  await expect
    .poll(() => sql("SELECT suspended_till IS NOT NULL FROM users WHERE id = :'p1'", author.id))
    .toEqual([["t"]]);
});
