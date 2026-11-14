// The tab title's count (static/js/document-title.js, Ember's
// document-title service): "(N) title" from the notifications that
// change while the tab is hidden, or with title_count_mode contextual
// from the latest list's banner and the posts others make in the topic
// being viewed in a hidden tab; the count kept on the next page's title,
// gone as the tab is shown again, hidden by do not disturb, and drawn on
// the favicon instead with the dynamic_favicon option. Each test works
// on a member of its own (cloned from user2), so the counts are only
// what the test makes.
//
// A hidden tab is document.visibilityState overridden and a
// visibilitychange event, what d-document's _focusChanged listens to.
const fs = require("fs");
const path = require("path");
const { test, expect, login, api, topic, sql, member, label } = require("./helpers");

async function setHidden(page, hidden) {
  await page.evaluate((hidden) => {
    Object.defineProperty(document, "visibilityState", { configurable: true, get: () => (hidden ? "hidden" : "visible") });
    Object.defineProperty(document, "hidden", { configurable: true, get: () => hidden });
    document.dispatchEvent(new Event("visibilitychange"));
  }, hidden);
}

function option(m, column, value) {
  sql(`UPDATE user_options SET ${column} = ${value} WHERE user_id = :'p1'`, m.id);
}

// A topic by user1 the others reply in, and a reply mentioning the member.
async function mentions(session, m) {
  const author = await session("user1");
  const t = await topic(author, label("Title counts"), label("The first post of a topic for the tab title's counts"));
  return {
    topic: t,
    author,
    mention: () => api(author, "POST", "/posts.json", { topic_id: String(t.id), raw: label(`@${m.name} a mention for the title`) }),
  };
}

// all_unread_notifications_count as the header avatar carries it, once
// it is at least `min`: a new member can be notified of more than the
// test's mentions (a badge granted).
async function unread(page, min) {
  const avatar = page.locator("#toggle-current-user img.avatar");
  await expect.poll(async () => Number(await avatar.getAttribute("data-all-unread"))).toBeGreaterThanOrEqual(min);
  return Number(await avatar.getAttribute("data-all-unread"));
}

// The title counting what the avatar carries, at least `min`.
async function counted(page, plain, min) {
  await expect
    .poll(() =>
      page.evaluate(
        ([plain, min]) => {
          const n = Number(document.querySelector("#toggle-current-user img.avatar").dataset.allUnread);
          return n >= min && document.title === `(${n}) ${plain}`;
        },
        [plain, min],
      ),
    )
    .toBe(true);
}

test("a member's title counts the notifications that change in a hidden tab", async ({ page, context, session }) => {
  const m = member();
  await login(context, m.name);
  const { mention } = await mentions(session, m);
  await page.goto("/latest");
  const plain = await page.title();
  expect(plain).not.toMatch(/^\(/);

  // Shown: the badge counts, the title doesn't (updateNotificationCount).
  await mention();
  await unread(page, 1);
  await expect(page).toHaveTitle(plain);

  // Hidden: all_unread_notifications_count, not only the new ones.
  await setHidden(page, true);
  await expect(page).toHaveTitle(plain);
  await mention();
  await counted(page, plain, 2);
  expect(await page.evaluate(() => Discourse.documentTitle.getTitle())).toBe(plain);

  // The next page's title takes the count, once.
  await page.locator("#navigation-bar a[href$='/categories']").click();
  await expect(page).toHaveURL(/\/categories$/);
  await expect(page).toHaveTitle(/^\(\d+\) (?!\()/);
  const categories = (await page.title()).replace(/^\(\d+\) /, "");
  expect(categories).not.toBe(plain);
  await mention();
  await counted(page, categories, 3);

  // Read elsewhere: the count goes with them.
  // (Read again if a badge's grant lands meanwhile: a new member's
  // first badges arrive from the job worker at their own time.)
  await expect(async () => {
    await api(context, "PUT", "/notifications/mark-read.json");
    await expect(page).toHaveTitle(categories, { timeout: 2000 });
  }).toPass();
  await mention();
  await counted(page, categories, 1);

  // Shown again: no count, and none from what arrives then.
  await setHidden(page, false);
  await expect(page).toHaveTitle(categories);
  await mention();
  await unread(page, 2);
  await expect(page).toHaveTitle(categories);
  await page.goBack();
  await expect(page).toHaveURL(/\/latest$/);
  await expect(page).toHaveTitle(plain);
});

test("do not disturb hides the count and its end shows it again", async ({ page, context, session }) => {
  const m = member();
  await login(context, m.name);
  const { mention } = await mentions(session, m);
  await page.goto("/latest");
  const plain = await page.title();
  await setHidden(page, true);
  await mention();
  await counted(page, plain, 1);

  await api(context, "POST", "/do-not-disturb.json", { duration: "30" });
  await expect(page.locator("#toggle-current-user .do-not-disturb-background")).toHaveCount(1);
  await expect(page).toHaveTitle(plain);
  await mention();
  await unread(page, 2);
  await expect(page).toHaveTitle(plain);

  await api(context, "DELETE", "/do-not-disturb.json");
  await counted(page, plain, 2);
});

test("the contextual count is the posts others make in the topic viewed in a hidden tab", async ({ page, context, session }) => {
  const m = member();
  option(m, "title_count_mode_key", 1);
  await login(context, m.name);
  const { topic: t, author, mention } = await mentions(session, m);
  const reply = (who) => api(who, "POST", "/posts.json", { topic_id: String(t.id), raw: label("A reply for the tab title's count") });
  await page.goto(t.url);
  const posts = page.locator("#posts > [data-post-number]");
  await expect(posts).toHaveCount(1);
  const plain = await page.title();

  // Shown: nothing counts.
  await reply(author);
  await expect(posts).toHaveCount(2);
  await expect(page).toHaveTitle(plain);

  // Hidden: not the member's own post; a mention as the post it is, not
  // as the notification.
  await setHidden(page, true);
  await reply(context);
  await expect(posts).toHaveCount(3);
  await expect(page).toHaveTitle(plain);
  await reply(author);
  await expect(page).toHaveTitle(`(1) ${plain}`);
  await mention();
  await expect(page).toHaveTitle(`(2) ${plain}`);
  await unread(page, 1);
  await expect(page).toHaveTitle(`(2) ${plain}`);

  await setHidden(page, false);
  await expect(page).toHaveTitle(plain);
});

test("the contextual count is the latest list's new or updated topics", async ({ page, context, session }) => {
  const m = member();
  option(m, "title_count_mode_key", 1);
  await login(context, m.name);
  await page.goto("/latest");
  // Other specs' topics count too, and may already have by now: the
  // title says what the banner does.
  const plain = (await page.title()).replace(/^\(\d+\) /, "");
  const author = await session("user1");
  await topic(author, label("Title list count"), label("A topic the latest list's banner counts"));
  const banner = page.locator("#list-updates > a[data-count]");
  await expect(banner).toBeVisible();
  await expect
    .poll(() => page.evaluate(() => [document.title, document.querySelector("#list-updates > a[data-count]")?.dataset.count]))
    .toEqual([expect.stringMatching(/^\(\d+\) /), expect.any(String)]);
  await expect
    .poll(() =>
      page.evaluate((plain) => {
        const count = document.querySelector("#list-updates > a[data-count]")?.dataset.count;
        return document.title === `(${count}) ${plain}`;
      }, plain),
    )
    .toBe(true);

  // The next page starts at 0.
  await page.locator("#navigation-bar a[href$='/categories']").click();
  await expect(page).toHaveURL(/\/categories$/);
  await expect(page).toHaveTitle(/^(?!\()/);
});

test("dynamic_favicon draws the count on the favicon and leaves the title", async ({ page, context, session }) => {
  const m = member();
  option(m, "dynamic_favicon", true);
  // The test site's favicon upload has no file.
  const png = fs.readFileSync(path.resolve(__dirname, "../../vendor/discourse/public/images/d-logo-sketch-small.png"));
  await context.route("**/uploads/**/optimized/**", (route) => route.fulfill({ status: 200, contentType: "image/png", body: png }));
  await login(context, m.name);
  const { mention } = await mentions(session, m);
  await page.goto("/latest");
  const plain = await page.title();
  const icon = page.locator("head link[rel=icon]");
  await expect(icon).toHaveCount(1);
  await expect(icon).not.toHaveAttribute("href", /^data:/);

  await setHidden(page, true);
  await expect(icon).toHaveAttribute("href", /^data:image\/png/);
  const none = await icon.getAttribute("href");
  await mention();
  await unread(page, 1);
  await expect(icon).not.toHaveAttribute("href", none);
  await expect(icon).toHaveAttribute("href", /^data:image\/png/);
  await expect(page).toHaveTitle(plain);

  await setHidden(page, false);
  await expect(icon).toHaveAttribute("href", none);
  await expect(page).toHaveTitle(plain);
});

test("a visitor's title never changes", async ({ page, session }) => {
  await page.goto("/latest");
  const plain = await page.title();
  await expect(page.locator("script[src*='document-title']")).toHaveCount(0);
  expect(await page.evaluate(() => typeof Discourse.documentTitle)).toBe("undefined");
  await setHidden(page, true);
  const author = await session("user1");
  await topic(author, label("Title visitor"), label("A topic a visitor's latest list hears of"));
  await expect(page.locator("#list-updates > a[data-count]")).toBeVisible();
  await expect(page).toHaveTitle(plain);
});
