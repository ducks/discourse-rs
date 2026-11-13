// A tag's unread list and its Dismiss… (TagsController#show_unread, the
// discovery controller's dismissRead with the tag), and the dismiss
// buttons of a member's message lists (user-topics-list.gjs): the new
// list's resetNew (Topic.pmResetNew) and the unread list's DismissRead
// with the route's inbox, a group inbox's with its group.
const { test, expect, login, api, sql, label, member } = require("./helpers");

function unique(prefix) {
  return prefix + Date.now().toString(36) + Math.floor(Math.random() * 1e6).toString(36);
}

// The member has read the topic's first post and tracks it.
async function readAndTrack(context, topicId) {
  await api(context, "POST", "/topics/timings", {
    topic_id: String(topicId),
    topic_time: "1000",
    "timings[1]": "1000",
  });
  await api(context, "POST", `/t/${topicId}/notifications`, { notification_level: "2" });
}

async function reply(context, topicId) {
  await api(context, "POST", "/posts.json", {
    topic_id: String(topicId),
    raw: `${label("A reply")} for the member tracking this to find unread.`,
  });
}

// A message from `sender` to the recipients (usernames or a group).
async function message(sender, recipients, what) {
  const title = label(what);
  const post = await api(sender, "POST", "/posts.json", {
    title,
    raw: `The first post of ${title}, a message.`,
    archetype: "private_message",
    target_recipients: recipients,
  });
  return post.topic_id;
}

// The list drawn again after the action (Discourse.refresh).
async function redrawnAfter(page, act) {
  await page.evaluate(() => {
    window.dismissRedrawn = new Promise((resolve) =>
      document.addEventListener("discourse:redrawn", resolve, { once: true }),
    );
  });
  await act();
  await page.evaluate(() => window.dismissRedrawn);
}

function rows(page) {
  return page.locator("tbody.topic-list-body > tr[data-topic-id]");
}

function row(page, id) {
  return page.locator(`tbody.topic-list-body > tr[data-topic-id="${id}"]`);
}

function sent(page, path) {
  return page.waitForRequest((r) => r.method() === "PUT" && r.url().endsWith(path));
}

function topicUser(topicId, userId) {
  const [found] = sql(
    `SELECT tu.last_read_post_number, t.highest_post_number, tu.notification_level FROM topic_users tu
     JOIN topics t ON t.id = tu.topic_id WHERE tu.topic_id = ${Number(topicId)} AND tu.user_id = ${Number(userId)}`,
  );
  return found && found.map(Number);
}

function dismissed(topicId, userId) {
  return sql(
    `SELECT 1 FROM dismissed_topic_users WHERE topic_id = ${Number(topicId)} AND user_id = ${Number(userId)}`,
  ).length;
}

test("a tag's unread list shows the member's unread topic in it, and Dismiss… clears the tag's alone", async ({
  page,
  context,
  session,
}) => {
  const admin = await session("admin");
  const reader = member();
  await login(context, reader.name);
  const tag = unique("unread");
  const tagged = await api(admin, "POST", "/posts.json", {
    title: label("A tagged topic"),
    raw: `The first post of ${label("a tagged topic")}, to be read and replied to.`,
    category: "4",
    "tags[]": tag,
  });
  const other = await api(admin, "POST", "/posts.json", {
    title: label("An untagged topic"),
    raw: `The first post of ${label("an untagged topic")}, to be read and replied to.`,
    category: "4",
  });
  const [[tagId]] = sql("SELECT id FROM tags WHERE name = :'p1'", tag);
  for (const id of [tagged.topic_id, other.topic_id]) {
    await readAndTrack(context, id);
    await reply(admin, id);
  }

  await page.goto(`/tag/${tag}/${tagId}/l/unread`);
  await expect(rows(page)).toHaveCount(1);
  await expect(row(page, tagged.topic_id)).toBeVisible();
  const top = page.locator(".dismiss-container-top");
  await expect(top).toHaveAttribute("data-tag-name", tag);
  await expect(top.locator("button")).toHaveCount(1);
  await expect(top.locator("#dismiss-topics-top.btn.btn-default.dismiss-read .d-button-label")).toHaveText("Dismiss…");

  await top.locator("#dismiss-topics-top").click();
  const modal = page.locator(".d-modal.dismiss-read-modal");
  await expect(modal.locator(".d-modal__title-text")).toHaveText("Dismiss all unread");
  await expect(modal.locator(".dismiss-read-modal__stop-tracking input")).not.toBeChecked();
  const request = sent(page, "/topics/bulk");
  await redrawnAfter(page, () => modal.locator("#dismiss-read-confirm").click());
  // The tag's name: Ember sends the tag model's id here, which the
  // endpoint matches against no tag's name.
  expect((await request).postData()).toBe(
    `filter=unread&operation%5Btype%5D=dismiss_posts&tracked=false&include_subcategories=true&tag_name=${tag}`,
  );
  await expect(modal).toHaveCount(0);
  await expect(rows(page)).toHaveCount(0);
  await expect(page.locator(".dismiss-container-top button")).toHaveCount(0);
  expect(topicUser(tagged.topic_id, reader.id).slice(0, 2)).toEqual([2, 2]);
  expect(topicUser(other.topic_id, reader.id).slice(0, 2)).toEqual([1, 2]);

  // The untagged topic is still on the member's unread list.
  await page.goto("/unread");
  await expect(row(page, other.topic_id)).toBeVisible();
  await expect(row(page, tagged.topic_id)).toHaveCount(0);
});

test("a member's new messages list dismisses its messages", async ({ page, context, session }) => {
  const sender = await session("user1");
  const reader = member();
  await login(context, reader.name);
  const first = await message(sender, reader.name, "A new message");
  const second = await message(sender, reader.name, "Another new message");

  await page.goto(`/u/${reader.name}/messages/new`);
  await expect(rows(page)).toHaveCount(2);
  const top = page.locator(".dismiss-container-top");
  await expect(top).toHaveAttribute("data-pm-inbox", "user");
  await expect(top.locator("button")).toHaveCount(1);
  await expect(top.locator("#dismiss-new-top.btn.btn-icon-text.btn-default.dismiss-read .d-button-label")).toHaveText(
    /^Dismiss (all|New)$/,
  );
  await expect(top.locator("#dismiss-new-top svg.d-icon-check")).toHaveCount(1);
  // Two topics: no bottom row.
  await expect(page.locator(".dismiss-container-bottom")).toHaveCount(0);

  const request = sent(page, "/topics/pm-reset-new");
  await redrawnAfter(page, () => top.locator("#dismiss-new-top").click());
  expect((await request).postData()).toBe("inbox=user");
  await expect(rows(page)).toHaveCount(0);
  await expect(page.locator(".dismiss-container-top")).toHaveCount(0);
  await expect(page.locator(".empty-state__title")).toBeVisible();
  await expect(page).toHaveURL(new RegExp(`/u/${reader.name}/messages/new$`));
  expect(dismissed(first, reader.id)).toBe(1);
  expect(dismissed(second, reader.id)).toBe(1);
  // Dismissed, not read.
  const tu = topicUser(first, reader.id);
  expect(tu ? tu[0] : 0).toBe(0);
});

test("a member's unread messages list dismisses by reading, or by no longer tracking", async ({
  page,
  context,
  session,
}) => {
  const sender = await session("user1");
  const reader = member();
  await login(context, reader.name);
  const id = await message(sender, reader.name, "A message with a reply");
  await api(context, "POST", "/topics/timings", { topic_id: String(id), topic_time: "1000", "timings[1]": "1000" });
  await reply(sender, id);
  // A regular topic unread too, which the inbox's dismissal leaves.
  const admin = await session("admin");
  const regular = await api(admin, "POST", "/posts.json", {
    title: label("A regular topic"),
    raw: `The first post of ${label("a regular topic")}, to be read and replied to.`,
    category: "4",
  });
  await readAndTrack(context, regular.topic_id);
  await reply(admin, regular.topic_id);

  const path = `/u/${reader.name}/messages/unread`;
  await page.goto(path);
  await expect(rows(page)).toHaveCount(1);
  await expect(row(page, id)).toBeVisible();
  const top = page.locator(".dismiss-container-top");
  await expect(top).toHaveAttribute("data-pm-inbox", "user");
  await expect(top.locator("#dismiss-topics-top.btn.btn-default.dismiss-read .d-button-label")).toHaveText("Dismiss…");

  await top.locator("#dismiss-topics-top").click();
  const modal = page.locator(".d-modal.dismiss-read-modal");
  await expect(modal.locator(".d-modal__title-text")).toHaveText("Dismiss all unread");
  let request = sent(page, "/topics/bulk");
  await redrawnAfter(page, () => modal.locator("#dismiss-read-confirm").click());
  expect((await request).postData()).toBe(
    "filter=unread&operation%5Btype%5D=dismiss_posts&tracked=false&private_message_inbox=user",
  );
  await expect(modal).toHaveCount(0);
  await expect(rows(page)).toHaveCount(0);
  await expect(page.locator(".dismiss-container-top")).toHaveCount(0);
  expect(topicUser(id, reader.id)).toEqual([2, 2, 3]);
  expect(topicUser(regular.topic_id, reader.id).slice(0, 2)).toEqual([1, 2]);

  // Unread again: "Stop tracking these topics".
  await reply(sender, id);
  await page.goto(path);
  await expect(row(page, id)).toBeVisible();
  await page.locator("#dismiss-topics-top").click();
  await modal.locator(".dismiss-read-modal__stop-tracking input").check();
  request = sent(page, "/topics/bulk");
  await redrawnAfter(page, () => modal.locator("#dismiss-read-confirm").click());
  expect((await request).postData()).toBe(
    "filter=unread&operation%5Btype%5D=change_notification_level&operation%5Bnotification_level_id%5D=1&tracked=false&private_message_inbox=user",
  );
  await expect(rows(page)).toHaveCount(0);
  expect(topicUser(id, reader.id)).toEqual([2, 3, 1]);
  expect(topicUser(regular.topic_id, reader.id).slice(0, 2)).toEqual([1, 2]);
});

test("a group inbox's new and unread lists dismiss that group's messages", async ({ page, context, session }) => {
  const admin = await session("admin");
  const sender = await session("user1");
  const reader = member();
  await login(context, reader.name);
  const group = unique("inbox");
  await api(admin, "POST", "/admin/groups.json", {
    "group[name]": group,
    "group[messageable_level]": "99",
    "group[visibility_level]": "0",
    "group[usernames]": reader.name,
  });
  const fresh = await message(sender, group, "A new group message");
  const replied = await message(sender, group, "A group message with a reply");
  // A message of the member's own inbox, new as well.
  const personal = await message(sender, reader.name, "A personal message");
  await api(context, "POST", "/topics/timings", {
    topic_id: String(replied),
    topic_time: "1000",
    "timings[1]": "1000",
  });
  await api(context, "POST", `/t/${replied}/notifications`, { notification_level: "3" });
  await reply(sender, replied);

  await page.goto(`/u/${reader.name}/messages/group/${group}/unread`);
  await expect(rows(page)).toHaveCount(1);
  await expect(row(page, replied)).toBeVisible();
  let top = page.locator(".dismiss-container-top");
  await expect(top).toHaveAttribute("data-pm-inbox", "group");
  await expect(top).toHaveAttribute("data-group-name", group);
  await top.locator("#dismiss-topics-top").click();
  const modal = page.locator(".d-modal.dismiss-read-modal");
  let request = sent(page, "/topics/bulk");
  await redrawnAfter(page, () => modal.locator("#dismiss-read-confirm").click());
  expect((await request).postData()).toBe(
    `filter=unread&operation%5Btype%5D=dismiss_posts&tracked=false&private_message_inbox=group&group_name=${group}`,
  );
  await expect(rows(page)).toHaveCount(0);
  expect(topicUser(replied, reader.id).slice(0, 2)).toEqual([2, 2]);

  await page.goto(`/u/${reader.name}/messages/group/${group}/new`);
  await expect(rows(page)).toHaveCount(1);
  await expect(row(page, fresh)).toBeVisible();
  top = page.locator(".dismiss-container-top");
  await expect(top).toHaveAttribute("data-group-name", group);
  request = sent(page, "/topics/pm-reset-new");
  await redrawnAfter(page, () => top.locator("#dismiss-new-top").click());
  expect((await request).postData()).toBe(`inbox=group&group_name=${group}`);
  await expect(rows(page)).toHaveCount(0);
  expect(dismissed(fresh, reader.id)).toBe(1);
  // The member's own inbox keeps its new message.
  expect(dismissed(personal, reader.id)).toBe(0);
  await page.goto(`/u/${reader.name}/messages/new`);
  await expect(row(page, personal)).toBeVisible();
});
