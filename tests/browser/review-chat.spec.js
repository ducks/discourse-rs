// Chat flags in the review queue (148a): a member flags a chat message
// from its actions menu (flag-modal.js); the moderator sees it in /review
// beside a post flag, drawn as chat's reviewable/chat-message.gjs, and
// agrees with it (the message gone for members), and disagrees with or
// ignores others (review.js, each item redrawn by id). Each test makes its
// own channel, so the other specs' General is left alone.
const { test, expect, login, api, topic, sql, label } = require("./helpers");

// A category channel (Site Feedback) the given members follow.
async function channelFor(session, members) {
  const admin = await session("admin");
  const name = label("flags").replace(/\s+/g, "-").slice(0, 40);
  const created = await api(admin, "POST", "/chat/api/channels.json", {
    "channel[chatable_id]": "2",
    "channel[name]": name,
  });
  const id = created.channel.id;
  const contexts = { admin };
  for (const username of members) {
    const context = await session(username);
    await api(context, "POST", `/chat/api/channels/${id}/memberships/me.json`);
    contexts[username] = context;
  }
  return { id, slug: created.channel.slug, contexts };
}

async function send(channel, author, text) {
  await api(channel.contexts[author], "POST", `/chat/${channel.id}.json`, { staged_id: label("staged"), message: text });
  const [[id]] = sql("SELECT id FROM chat_messages WHERE chat_channel_id = :'p1' AND message = :'p2'", channel.id, text);
  return id;
}

// user1 flags the message as inappropriate from its actions menu.
async function flagFromChat(page, channel, text) {
  await page.goto(`/chat/c/${channel.slug}/${channel.id}`);
  const message = page.locator(".chat-message-container", { hasText: text });
  await expect(message).toBeVisible();
  await expect(async () => {
    await message.hover();
    await message.locator(".chat-message-actions .more-actions-chat summary").click({ timeout: 2000 });
    await message.locator(".chat-message-actions .select-kit-row[data-value='flag']").click({ timeout: 2000 });
  }).toPass({ timeout: 15_000 });
  const modal = page.locator(".d-modal .flag-modal-body");
  await modal.locator("#radio_inappropriate").check();
  await page.locator(".flag-modal__create-flag").click();
  await expect(modal).toHaveCount(0);
}

function reviewableOf(messageId) {
  return sql(
    "SELECT type, status FROM reviewables WHERE target_type = 'ChatMessage' AND target_id = :'p1'",
    messageId,
  );
}

// The author's auto silence a flag may bring, lifted for the other specs.
function unsilence() {
  sql("UPDATE users SET silenced_till = NULL WHERE username = 'user3'");
}

test("a chat flag lists beside a post flag; agreeing deletes the message for members", async ({
  page,
  context,
  session,
}) => {
  const channel = await channelFor(session, ["user3", "user1", "user2"]);
  const text = label("chat message to flag");
  const messageId = await send(channel, "user3", text);

  const flagger = await channel.contexts.user1.newPage();
  await flagFromChat(flagger, channel, text);
  await expect.poll(() => reviewableOf(messageId)).toEqual([["ReviewableChatMessage", "0"]]);
  unsilence();

  // A post flag, so the queue holds both kinds.
  const user0 = await session("user0");
  const marker = label("A reply flagged beside the chat flag");
  const t = await topic(user0, label("Chat review topic"), label("A topic for the chat review spec"), [
    { context: user0, raw: marker },
  ]);
  await api(channel.contexts.user1, "POST", "/post_actions.json", {
    id: String(t.posts[1]),
    post_action_type_id: "4",
    flag_topic: "false",
  });

  await login(context, "admin");
  await page.goto("/review");
  const postItem = page.locator(".review-item.reviewable-flagged-post", { hasText: marker });
  const item = page.locator(".review-item.reviewable-chat-message", { hasText: text });
  await expect(postItem).toBeVisible();
  await expect(item).toBeVisible();
  await expect(item.locator(".review-item__flag-label")).toHaveText("Chat flagged as");
  await expect(item.locator(".review-item__meta-topic-title a")).toHaveAttribute(
    "href",
    `/chat/c/${channel.slug}/${channel.id}/${messageId}`,
  );
  await expect(item.locator(".review-item__meta-flagged-user")).toContainText("user3");

  // Yes > Delete Message.
  await item.locator("details summary", { hasText: /^\s*Yes/ }).first().click();
  await page.locator(".select-kit.is-expanded .select-kit-row[data-value='chat_message-agree_and_delete']").click();
  await expect(item.locator(".review-item__status.--approved")).toBeVisible();
  await expect(item.locator(".review-item__status")).toHaveText("Flag approved");
  await expect.poll(() => reviewableOf(messageId)).toEqual([["ReviewableChatMessage", "1"]]);
  expect(sql("SELECT deleted_at IS NOT NULL FROM chat_messages WHERE id = :'p1'", messageId)).toEqual([["t"]]);
  // The post flag is still waiting.
  await expect(postItem.locator(".review-item__status.--pending")).toBeVisible();

  // Gone for another member.
  const member = await channel.contexts.user2.newPage();
  await member.goto(`/chat/c/${channel.slug}/${channel.id}`);
  await expect(member.locator(".chat-composer__input")).toBeVisible();
  await expect(member.locator(".chat-message-container", { hasText: text })).toHaveCount(0);
});

test("disagreeing with one chat flag and ignoring another", async ({ page, context, session }) => {
  const channel = await channelFor(session, ["user3", "user1"]);
  const kept = label("chat message kept");
  const ignored = label("chat message ignored");
  const keptId = await send(channel, "user3", kept);
  const ignoredId = await send(channel, "user3", ignored);
  const flagger = await channel.contexts.user1.newPage();
  await flagFromChat(flagger, channel, kept);
  await flagFromChat(flagger, channel, ignored);
  await expect.poll(() => reviewableOf(ignoredId)).toEqual([["ReviewableChatMessage", "0"]]);
  unsilence();

  await login(context, "admin");
  await page.goto("/review?type=Chat%3A%3AReviewableMessage");
  const keptItem = page.locator(".review-item.reviewable-chat-message", { hasText: kept });
  const ignoredItem = page.locator(".review-item.reviewable-chat-message", { hasText: ignored });
  await expect(keptItem).toBeVisible();
  await expect(ignoredItem).toBeVisible();

  // No > Keep message.
  await keptItem.locator("details summary", { hasText: /^\s*No/ }).first().click();
  await page.locator(".select-kit.is-expanded .select-kit-row[data-value='chat_message-disagree']").click();
  await expect(keptItem.locator(".review-item__status")).toHaveText("Flag rejected");
  await expect.poll(() => reviewableOf(keptId)).toEqual([["ReviewableChatMessage", "2"]]);

  // No > Ignore flag.
  await ignoredItem.locator("details summary", { hasText: /^\s*No/ }).first().click();
  await page.locator(".select-kit.is-expanded .select-kit-row[data-value='chat_message-ignore']").click();
  await expect(ignoredItem.locator(".review-item__status")).toHaveText("Flag ignored");
  await expect.poll(() => reviewableOf(ignoredId)).toEqual([["ReviewableChatMessage", "3"]]);

  // Both messages stay.
  expect(
    sql(
      "SELECT COUNT(*) FROM chat_messages WHERE id IN (:'p1', :'p2') AND deleted_at IS NULL",
      keptId,
      ignoredId,
    ),
  ).toEqual([["2"]]);
});
