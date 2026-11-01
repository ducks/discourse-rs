// Archiving a chat channel (216a) against the server and its job worker:
// the settings' archive row opening ChatModalArchiveChannel
// (chat-channel-info.js), a new topic or an existing one chosen through
// ChooseTopic, Jobs::Chat::ChannelArchive quoting the messages into the
// topic, the channel archived and its status bar linking the topic; a
// failed archive retried from the status bar (chat.js), which hears the
// outcome over /chat/channel-archive-status.
const { test, expect, login, api, topic, sql, label } = require("./helpers");

// A channel of its own in General's category with `count` messages by
// the admin, archiving allowed. Returns { id, slug, path }.
async function channelWithMessages(context, count) {
  await api(context, "PUT", "/admin/site_settings/chat_allow_archiving_channels", {
    chat_allow_archiving_channels: "true",
  });
  const name = label("Archive channel").replace(/[^a-z0-9 ]/gi, "");
  const created = await api(context, "POST", "/chat/api/channels", {
    "channel[name]": name,
    "channel[chatable_id]": "4",
  });
  const channel = created.channel;
  for (let i = 1; i <= count; i++) {
    await api(context, "POST", `/chat/${channel.id}.json`, {
      message: `${name} message ${i}`,
      staged_id: `s${i}`,
    });
  }
  return { id: channel.id, slug: channel.slug, name, path: `/chat/c/${channel.slug}/${channel.id}` };
}

async function openArchiveModal(page, channel) {
  await page.goto(`${channel.path}/info/settings`);
  await page.locator(".c-channel-settings .archive-btn").click();
  const modal = page.locator(".chat-modal-archive-channel");
  await expect(modal).toBeVisible();
  return modal;
}

// The archive's destination once the job has completed it.
async function archivedTopic(channelId) {
  await expect
    .poll(() => sql("SELECT status FROM chat_channels WHERE id = :'p1'", channelId), { timeout: 30_000 })
    .toEqual([["3"]]);
  const [[topicId]] = sql(
    "SELECT destination_topic_id FROM chat_channel_archives WHERE chat_channel_id = :'p1'",
    channelId,
  );
  return topicId;
}

test("an admin archives a channel into a new topic from its settings", async ({ page, context }) => {
  await login(context, "admin");
  const channel = await channelWithMessages(context, 3);
  const title = label("Archived channel topic");
  const tag = `arch${Date.now().toString(36)}`;

  const modal = await openArchiveModal(page, channel);
  await expect(modal.locator(".chat-modal-archive-channel__instructions")).toContainText(channel.name);
  const confirm = page.locator("#chat-confirm-archive-channel");
  // Too short a title keeps the button off.
  await modal.locator("#split-topic-name").fill("short");
  await expect(confirm).toBeDisabled();
  await modal.locator("#split-topic-name").fill(title);
  await expect(modal.locator("#new-topic-category-selector option[value='4']")).toHaveCount(1);
  await modal.locator("#new-topic-category-selector").selectOption("4");
  await modal.locator(".tag-chooser").fill(tag);
  await expect(confirm).toBeEnabled();
  await confirm.click();
  await expect(page.locator(".d-modal__flash.alert-success")).toContainText("Archiving process has started");
  await expect(modal).toBeHidden({ timeout: 10_000 });

  const topicId = await archivedTopic(channel.id);
  expect(sql("SELECT title, category_id, archived FROM topics WHERE id = :'p1'", topicId)).toEqual([
    [title, "4", "t"],
  ]);
  expect(sql("SELECT destination_tags FROM chat_channel_archives WHERE chat_channel_id = :'p1'", channel.id)).toEqual([
    [`{${tag}}`],
  ]);
  const posts = sql("SELECT post_number, replace(raw, E'\n', ' ') FROM posts WHERE topic_id = :'p1' ORDER BY post_number", topicId);
  expect(posts.length).toBe(2);
  expect(posts[0][1]).toContain(`This topic is an archive of the [${channel.name}]`);
  expect(posts[1][1]).toContain('noLink="true"');
  expect(posts[1][1]).toContain(`${channel.name} message 3`);
  expect(sql("SELECT COUNT(*) FROM chat_messages WHERE chat_channel_id = :'p1' AND deleted_at IS NULL", channel.id)).toEqual([
    ["0"],
  ]);

  // The channel says it is archived and links the topic.
  await page.goto(channel.path);
  const link = page.locator(".chat-channel-archive-status a");
  await expect(link).toHaveAttribute("href", new RegExp(`/t/-/${topicId}$`));
  await expect(page.locator(".c-channel-settings .archive-btn")).toHaveCount(0);
  // The topic shows the transcript.
  await page.goto(`/t/-/${topicId}`);
  await expect(page.locator(".chat-transcript").first()).toContainText(`${channel.name} message 1`);
});

test("a channel archived into an existing topic chosen by search", async ({ page, context }) => {
  await login(context, "admin");
  const channel = await channelWithMessages(context, 2);
  const marker = `dest${Date.now().toString(36)}`;
  const destination = await topic(context, `Archive destination ${marker}`, label("The existing topic for an archive"));

  const modal = await openArchiveModal(page, channel);
  await modal.locator("#move-to-existing-topic").check();
  await expect(modal.locator("#split-topic-name")).toBeHidden();
  const confirm = page.locator("#chat-confirm-archive-channel");
  await expect(confirm).toBeDisabled();
  await modal.locator("#choose-topic-title").fill(marker);
  await modal.locator(`#choose-topic-${destination.id}`).check();
  await expect(confirm).toBeEnabled();
  await confirm.click();
  await expect(modal).toBeHidden({ timeout: 10_000 });

  const topicId = await archivedTopic(channel.id);
  expect(topicId).toBe(String(destination.id));
  const posts = sql("SELECT replace(raw, E'\n', ' ') FROM posts WHERE topic_id = :'p1' ORDER BY post_number", destination.id);
  expect(posts.length).toBe(2);
  expect(posts[1][0]).toContain(`${channel.name} message 2`);
  // An existing topic keeps its status.
  expect(sql("SELECT archived, closed FROM topics WHERE id = :'p1'", destination.id)).toEqual([["f", "f"]]);
});

test("a failed archive is retried from the channel's status bar", async ({ page, context }) => {
  await login(context, "admin");
  const channel = await channelWithMessages(context, 2);
  sql("UPDATE chat_channels SET status = 1 WHERE id = :'p1'", channel.id);
  sql(
    "INSERT INTO chat_channel_archives (chat_channel_id, archived_by_id, destination_topic_title, destination_category_id, total_messages, archived_messages, archive_error, created_at, updated_at) VALUES (:'p1', 1, :'p2', 4, 2, 1, 'Something went wrong', now(), now())",
    channel.id,
    label("Retried archive topic"),
  );

  await page.goto(channel.path);
  const failed = page.locator(".chat-channel-retry-archive");
  await expect(failed).toContainText("Archive channel failed. 1/2 messages have been archived");
  await failed.locator(".chat-channel-archive-failed-retry .btn").click();

  // The job runs and the page hears the outcome without a reload.
  const link = page.locator(".chat-channel-archive-status a");
  await expect(link).toBeVisible({ timeout: 30_000 });
  const topicId = await archivedTopic(channel.id);
  await expect(link).toHaveAttribute("href", new RegExp(`/t/-/${topicId}$`));
  expect(sql("SELECT COALESCE(archive_error, 'none') FROM chat_channel_archives WHERE chat_channel_id = :'p1'", channel.id)).toEqual([
    ["none"],
  ]);
});
