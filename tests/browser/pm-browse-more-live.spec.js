// A message's browse more line (more-topics/browse-more.gjs,
// privateMessageBrowseMoreMessage) kept live: Ember counts it from
// pmTopicTrackingState, which hears the member's own and their groups'
// /private-message-topic-tracking-state channels. Here GET /live counts
// it again from the same messages and swaps the line in.
const { test, expect, login, api, sql, label, member } = require("./helpers");

function unique(prefix) {
  return prefix + Date.now().toString(36) + Math.floor(Math.random() * 1e6).toString(36);
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

async function reply(context, topicId) {
  await api(context, "POST", "/posts.json", {
    topic_id: String(topicId),
    raw: `${label("A reply")} for the member watching this to find unread.`,
  });
}

function topicUser(topicId, userId) {
  const [found] = sql(
    `SELECT COALESCE(tu.last_read_post_number, 0), t.highest_post_number FROM topic_users tu
     JOIN topics t ON t.id = tu.topic_id WHERE tu.topic_id = ${Number(topicId)} AND tu.user_id = ${Number(userId)}`,
  );
  return found ? found.map(Number) : [0, 0];
}

// The list drawn again after the action (Discourse.refresh).
async function redrawnAfter(page, act) {
  await page.evaluate(() => {
    window.browseMoreRedrawn = new Promise((resolve) =>
      document.addEventListener("discourse:redrawn", resolve, { once: true }),
    );
  });
  await act();
  await page.evaluate(() => window.browseMoreRedrawn);
}

function line(page) {
  return page.locator(".more-topics__container > h3.more-topics__browse-more");
}

// The line's links, as [href, text].
function links(page) {
  return line(page)
    .locator("a")
    .evaluateAll((all) => all.map((a) => [a.getAttribute("href"), a.textContent.trim()]));
}

const BROWSE = "Want to read more? Browse other messages in personal messages.";

test("a message's browse more line counts a new message as it arrives, and its reading", async ({
  page,
  context,
  session,
}) => {
  const sender = await session("user1");
  const reader = member();
  await login(context, reader.name);
  const messages = `/u/${reader.name}/messages`;
  const open = await message(sender, reader.name, "The message read");
  const other = await message(sender, reader.name, "Another message");

  await page.goto(`/t/${open}`);
  // The other message is new; this one is read as the page shows it.
  await expect(line(page)).toHaveText(
    "There is 1 new message remaining, or browse other personal messages",
  );
  expect(await links(page)).toEqual([
    [`${messages}/new`, "1 new"],
    [messages, "personal messages"],
  ]);

  // A message arrives: pmTopicTrackingState's new_topic.
  const arrived = await message(sender, reader.name, "A message arriving");
  await expect(line(page)).toHaveText(
    "There are 2 new messages remaining, or browse other personal messages",
  );
  expect(await links(page)).toEqual([
    [`${messages}/new`, "2 new"],
    [messages, "personal messages"],
  ]);
  await expect(line(page)).toHaveCount(1);

  // The member reads both in another tab: each read comes back over
  // their channel, and the line counts what is left.
  const tab = await context.newPage();
  await tab.goto(`/t/${other}`);
  await expect(line(page)).toHaveText(
    "There is 1 new message remaining, or browse other personal messages",
  );
  expect(topicUser(other, reader.id)).toEqual([1, 1]);
  await tab.goto(`/t/${arrived}`);
  await expect(line(page)).toHaveText(BROWSE);
  expect(await links(page)).toEqual([[messages, "personal messages"]]);
  expect(topicUser(arrived, reader.id)).toEqual([1, 1]);
  await tab.close();
});

test("a reply to a message read counts as unread on another message's line, until dismissed", async ({
  page,
  context,
  session,
}) => {
  const sender = await session("user1");
  const reader = member();
  await login(context, reader.name);
  const messages = `/u/${reader.name}/messages`;
  const open = await message(sender, reader.name, "The message read");
  const replied = await message(sender, reader.name, "A message with a reply");
  const fresh = await message(sender, reader.name, "A new message");
  await api(context, "POST", "/topics/timings", {
    topic_id: String(replied),
    topic_time: "1000",
    "timings[1]": "1000",
  });

  await page.goto(`/t/${open}`);
  await expect(line(page)).toHaveText(
    "There is 1 new message remaining, or browse other personal messages",
  );

  // pmTopicTrackingState's unread, from the reply's job.
  await reply(sender, replied);
  await expect(line(page)).toHaveText(
    "There is 1 unread and 1 new message remaining, or browse other personal messages",
  );
  expect(await links(page)).toEqual([
    [`${messages}/unread`, "1 unread"],
    [`${messages}/new`, "1 new"],
    [messages, "personal messages"],
  ]);

  // Dismissed on the unread list, the message is read: the line drawn
  // for the page counts the new one alone.
  await page.goto(`${messages}/unread`);
  await page.locator("#dismiss-topics-top").click();
  const modal = page.locator(".d-modal.dismiss-read-modal");
  await redrawnAfter(page, () => modal.locator("#dismiss-read-confirm").click());
  await expect(modal).toHaveCount(0);
  expect(topicUser(replied, reader.id)).toEqual([2, 2]);
  await page.goto(`/t/${open}`);
  await expect(line(page)).toHaveText(
    "There is 1 new message remaining, or browse other personal messages",
  );
  expect(topicUser(fresh, reader.id)[0]).toBe(0);
});

test("a group message's line counts its group's inbox, live", async ({ page, context, session }) => {
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
  const inbox = `/u/${reader.name}/messages/group/${group}`;
  const open = await message(sender, group, "The group message read");
  await message(sender, group, "Another group message");

  await page.goto(`/t/${open}`);
  await expect(line(page)).toHaveText(
    `There is 1 new message remaining, or browse other messages in ${group}`,
  );
  expect(await links(page)).toEqual([
    [`${inbox}/new`, "1 new"],
    [inbox, group],
  ]);
  await expect(line(page).locator("a.group-link svg.d-icon-users")).toHaveCount(1);

  // A message to the member alone is their own inbox's, not the group's.
  await message(sender, reader.name, "A personal message");
  // One to the group arrives on the group's channel.
  await message(sender, group, "A group message arriving");
  await expect(line(page)).toHaveText(
    `There are 2 new messages remaining, or browse other messages in ${group}`,
  );
  expect(await links(page)).toEqual([
    [`${inbox}/new`, "2 new"],
    [inbox, group],
  ]);
});
