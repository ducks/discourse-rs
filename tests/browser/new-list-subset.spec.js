// The new list's All / Topics / Replies toggle (DiscoveryTopics'
// NewListHeaderControlsWrapper under unified new): the member's new
// topics and unread replies within the list's category on its buttons,
// a button going to its `subset` of the list, and the counts kept by
// the page's stream as replies arrive.
const { test, expect, login, api, label, member } = require("./helpers");

// A category of the test's own, so the counts are its topics' alone
// whatever the other specs post meanwhile.
async function category(admin) {
  const name = label("Subset");
  const created = await api(admin, "POST", "/categories.json", { name, color: "0088CC", text_color: "FFFFFF" });
  return created.category;
}

// A topic by `author` in the category.
async function topicIn(author, categoryId, what) {
  const title = label(what);
  const post = await api(author, "POST", "/posts.json", {
    title,
    raw: `The first post of ${title}, in the subset category.`,
    category: String(categoryId),
  });
  return { id: post.topic_id, title };
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
    raw: `${label("A reply")} for the member tracking this topic to find unread.`,
  });
}

function buttons(page) {
  const wrapper = page.locator("#list-area .contents > .topic-replies-toggle-wrapper");
  return {
    all: wrapper.locator("button.topics-replies-toggle.--all"),
    topics: wrapper.locator("button.topics-replies-toggle.--topics"),
    replies: wrapper.locator("button.topics-replies-toggle.--replies"),
  };
}

async function expectPressed(page, name) {
  const toggle = buttons(page);
  for (const key of ["all", "topics", "replies"]) {
    await expect(toggle[key]).toHaveAttribute("aria-pressed", String(key === name));
    if (key === name) {
      await expect(toggle[key]).toHaveClass(/(^| )active( |$)/);
    } else {
      await expect(toggle[key]).not.toHaveClass(/(^| )active( |$)/);
    }
  }
}

function rows(page) {
  return page.locator("#list-area tbody.topic-list-body > tr[data-topic-id]");
}

async function rowIds(page) {
  return rows(page).evaluateAll((trs) => trs.map((tr) => Number(tr.dataset.topicId)).sort((a, b) => a - b));
}

test("the toggle counts the category's new topics and replies, filters the list and follows a new reply", async ({
  page,
  context,
  session,
}) => {
  const admin = await session("admin");
  const author = await session("user1");
  const cat = await category(admin);
  const reader = member();
  await login(context, reader.name);

  const fresh = await topicIn(author, cat.id, "A new topic");
  const replied = await topicIn(author, cat.id, "A topic with a new reply");
  const quiet = await topicIn(author, cat.id, "A topic read to its end");
  // The category's own definition is read too, so it is no new topic.
  const definition = cat.topic_url ? Number(cat.topic_url.split("/").pop()) : null;
  for (const id of [replied.id, quiet.id, definition]) {
    if (id) {
      await readAndTrack(context, id);
    }
  }
  await reply(author, replied.id);

  const path = `/c/${cat.slug}/${cat.id}/l/new`;
  await page.goto(path);
  const toggle = buttons(page);
  await expect(toggle.all).toHaveText("All");
  await expect(toggle.topics).toHaveText("Topics (1)");
  await expect(toggle.replies).toHaveText("Replies (1)");
  await expect(toggle.all).toHaveAttribute("title", "All new topics and topics replied to in the last few days");
  await expect(toggle.topics).toHaveAttribute("title", "New topics");
  await expect(toggle.replies).toHaveAttribute("title", "New replies");
  await expectPressed(page, "all");
  expect(await rowIds(page)).toEqual([fresh.id, replied.id].sort((a, b) => a - b));

  // changeNewListSubset replaces the URL: no history entry per subset.
  const entries = await page.evaluate(() => history.length);
  await toggle.topics.click();
  await expect(page).toHaveURL(new RegExp(`${path}\\?subset=topics$`));
  await expectPressed(page, "topics");
  await expect(rows(page)).toHaveCount(1);
  expect(await rowIds(page)).toEqual([fresh.id]);
  await expect(buttons(page).topics).toHaveText("Topics (1)");

  await buttons(page).replies.click();
  await expect(page).toHaveURL(new RegExp(`${path}\\?subset=replies$`));
  await expectPressed(page, "replies");
  await expect(rows(page)).toHaveCount(1);
  expect(await rowIds(page)).toEqual([replied.id]);
  expect(await page.evaluate(() => history.length)).toBe(entries);

  // A reply to another tracked topic arrives on the page's stream: the
  // count follows without a reload (the list itself is left alone).
  await page.evaluate(() => (window.__sameDocument = true));
  await reply(author, quiet.id);
  await expect(buttons(page).replies).toHaveText("Replies (2)");
  await expect(buttons(page).topics).toHaveText("Topics (1)");
  await expectPressed(page, "replies");
  expect(await page.evaluate(() => window.__sameDocument)).toBe(true);

  // A new topic counts on Topics.
  await topicIn(author, cat.id, "Another new topic");
  await expect(buttons(page).topics).toHaveText("Topics (2)");

  await buttons(page).all.click();
  await expect(page).toHaveURL(new RegExp(`${path}$`));
  await expectPressed(page, "all");
  await expect(rows(page)).toHaveCount(4);
  await expect(buttons(page).replies).toHaveText("Replies (2)");

  // The pressed button goes nowhere.
  await buttons(page).all.click();
  await expect(page).toHaveURL(new RegExp(`${path}$`));
});

test("/new has the toggle with the member's counts, and a subset keeps the list's other params", async ({
  page,
  context,
  session,
}) => {
  const admin = await session("admin");
  const author = await session("user1");
  const cat = await category(admin);
  const reader = member();
  await login(context, reader.name);
  const fresh = await topicIn(author, cat.id, "A new topic");
  const replied = await topicIn(author, cat.id, "A topic with a new reply");
  await readAndTrack(context, replied.id);
  await reply(author, replied.id);

  // The other specs' topics are new here too: the counts are at least
  // this test's own.
  const count = async (button) => Number(((await button.textContent()).match(/\((\d+)\)/) || [0, 0])[1]);
  await page.goto("/new?order=activity");
  const toggle = buttons(page);
  await expect(toggle.all).toHaveText("All");
  await expect(toggle.topics).toHaveText(/^Topics \(\d+\)$/);
  await expect(toggle.replies).toHaveText("Replies (1)");
  expect(await count(toggle.topics)).toBeGreaterThanOrEqual(1);
  await expectPressed(page, "all");

  await toggle.replies.click();
  await expect(page).toHaveURL(/\/new\?order=activity&subset=replies$/);
  await expectPressed(page, "replies");
  expect(await rowIds(page)).toEqual([replied.id]);

  await buttons(page).topics.click();
  await expect(page).toHaveURL(/\/new\?order=activity&subset=topics$/);
  await expectPressed(page, "topics");
  await expect(rows(page).and(page.locator(`[data-topic-id="${fresh.id}"]`))).toHaveCount(1);
  await expect(rows(page).and(page.locator(`[data-topic-id="${replied.id}"]`))).toHaveCount(0);

  await buttons(page).all.click();
  await expect(page).toHaveURL(/\/new\?order=activity$/);
  await expectPressed(page, "all");

  // No toggle off the new list.
  await page.goto("/latest");
  await expect(page.locator(".topic-replies-toggle-wrapper")).toHaveCount(0);
});
