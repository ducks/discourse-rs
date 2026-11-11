// The share modal (share-topic.js, components/modal/share-topic.gjs), as
// acceptance/share-topic-test.js opens it from the topic footer: the
// topic's link selected in its input, the copy button, the share_links
// sources, the new topic button, the restricted category's warning, the
// modal for a post (its date link and its menu's share) and the ways to
// close it.
const { test, expect, login, api, topic, sql, label, member } = require("./helpers");

const modal = (page) => page.locator(".d-modal.share-topic-modal");
const link = (page) => modal(page).locator("input.invite-link");

// A topic by user1 and a member of the test's own reading it, the modal
// open from the footer's share.
async function share(page, context, session) {
  const user1 = await session("user1");
  const t = await topic(user1, label("Share topic"), label("A topic the share modal links to"));
  const reader = member();
  await login(context, reader.name);
  await page.goto(t.url);
  await page.locator("#topic-footer-button-share-and-invite").click();
  await expect(modal(page)).toBeVisible();
  return { t, reader, url: new URL(`${t.url}?u=${reader.name}`, page.url()).href };
}

test("Topic footer button", async ({ page, context, session }) => {
  const { url } = await share(page, context, session);
  await expect(modal(page).locator("#discourse-modal-title")).toHaveText("Share Topic");
  await expect(modal(page).locator("label[for='invite-link']")).toHaveText("Share a link to this topic:");
  await expect(link(page), "shows the topic sharing url").toHaveValue(url);
  await expect(link(page)).toHaveAttribute("readonly", "");
  await expect(link(page)).toBeFocused();
  expect(
    await link(page).evaluate((el) => [el.selectionStart, el.selectionEnd]),
    "the url is selected",
  ).toEqual([0, url.length]);
});

test("the copy button copies the link and shows a check", async ({ page, context, session }) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  const { url } = await share(page, context, session);
  const copy = modal(page).locator(".link-share-container .copy-button");
  await expect(copy).toHaveAttribute("aria-label", "Copy and share URL");
  await expect(copy.locator("svg.d-icon-copy")).toBeVisible();

  await copy.click();
  await expect(copy.locator("svg.d-icon-check")).toBeVisible();
  await expect(copy).toHaveClass(/\bok\b/);
  await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe(url);
});

test("the sources are the share_links setting's", async ({ page, context, session }) => {
  const sources = modal(page).locator(".link-share-actions .sources button[class*='share-']");
  const { t, url } = await share(page, context, session);
  await expect(sources).toHaveClass([/share-twitter/, /share-facebook/, /share-email/]);
  await expect(sources.nth(0)).toHaveAttribute("title", "Share on X");

  // Sharing.shareSource: X's intent in a popup.
  await page.evaluate(() => {
    window.opened = [];
    window.open = (href) => {
      window.opened.push(href);
    };
  });
  await sources.nth(0).click();
  const opened = await page.evaluate(() => window.opened);
  expect(opened).toHaveLength(1);
  expect(opened[0]).toContain(`http://x.com/intent/tweet?url=${encodeURIComponent(url)}&text=`);

  const admin = await session("admin");
  const [[before]] = sql(
    "SELECT COALESCE((SELECT value FROM site_settings WHERE name = 'share_links'), 'twitter|facebook|email')",
  );
  await api(admin, "PUT", "/admin/site_settings/share_links", { share_links: "email|twitter" });
  try {
    await page.goto(t.url);
    await page.locator("#topic-footer-button-share-and-invite").click();
    await expect(sources).toHaveClass([/share-email/, /share-twitter/]);
  } finally {
    await api(admin, "PUT", "/admin/site_settings/share_links", { share_links: before });
  }
});

test("a message shares by email alone", async ({ page, context }) => {
  const sender = member();
  await login(context, sender.name);
  const pm = await api(context, "POST", "/posts.json", {
    title: label("A message to share"),
    raw: label("A message whose share modal has no public sources"),
    archetype: "private_message",
    target_recipients: "user0",
  });
  await page.goto(`/t/${pm.topic_slug}/${pm.topic_id}`);
  await page.locator("#topic-footer-button-share-and-invite").click();
  await expect(modal(page)).toBeVisible();
  await expect(modal(page).locator(".link-share-actions .sources button[class*='share-']")).toHaveClass([/share-email/]);
});

test("the close button, Escape and the backdrop close it", async ({ page, context, session }) => {
  await share(page, context, session);
  await modal(page).locator(".modal-close").click();
  await expect(modal(page)).toHaveCount(0);

  await page.locator("#topic-footer-button-share-and-invite").click();
  await expect(modal(page)).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(modal(page)).toHaveCount(0);

  await page.locator("#topic-footer-button-share-and-invite").click();
  await expect(modal(page)).toBeVisible();
  // .d-modal lets the pointer through to the backdrop around the dialog.
  await page.locator(".d-modal__backdrop").click({ position: { x: 5, y: 5 } });
  await expect(modal(page)).toHaveCount(0);
});

const editor = (page) => page.locator("#reply-control.open textarea.d-editor-input");

test("New topic button", async ({ page, context, session }) => {
  const { t } = await share(page, context, session);
  await expect(modal(page).locator("#modal-alert"), "no warning in a category everyone sees").toHaveCount(0);
  const button = modal(page).locator(".link-share-actions .new-topic");
  await expect(button).toHaveText("New Topic");
  await expect(button).toHaveAttribute("title", "Reply as linked Topic");
  await button.click();
  await expect(modal(page)).toHaveCount(0);
  await expect(page.locator("#reply-control.open.composer-action-create-topic")).toBeVisible();
  const [[title]] = sql("SELECT title FROM topics WHERE id = :'p1'", t.id);
  const link = new URL(t.url, page.url()).href;
  await expect(editor(page)).toHaveValue(`Continuing the discussion from [${title}](${link}):\n\n`);
});

test("a message's new topic button starts a message to its recipients", async ({ page, context }) => {
  const sender = member();
  await login(context, sender.name);
  const title = label("A message to continue");
  const pm = await api(context, "POST", "/posts.json", {
    title,
    raw: label("A message whose share modal starts another"),
    archetype: "private_message",
    target_recipients: "user0",
  });
  const url = `/t/${pm.topic_slug}/${pm.topic_id}`;
  await page.goto(url);
  await page.locator("#topic-footer-button-share-and-invite").click();
  const button = modal(page).locator(".link-share-actions .new-topic");
  await expect(button).toHaveText("New Message");
  await expect(button).toHaveAttribute("title", "Reply as new message to the same recipients");
  await button.click();
  await expect(modal(page)).toHaveCount(0);
  const composer = page.locator("#reply-control.open.composer-action-private-message");
  await expect(composer).toBeVisible();
  await expect(editor(page)).toHaveValue(
    `Continuing the discussion from [${title}](${new URL(url, page.url()).href}):\n\n`,
  );
  await expect(composer.locator(".users-input")).toContainText("user0");
  await expect(composer.locator(".users-input")).toContainText(sender.name);
});

// A topic with a reply, a member of the test's own reading it.
async function replied(page, context, session) {
  const user1 = await session("user1");
  const t = await topic(user1, label("Share post"), label("A topic whose reply is shared"), [
    { context: await session("user0"), raw: label("The reply the share modal links to") },
  ]);
  const reader = member();
  await login(context, reader.name);
  await page.goto(t.url);
  return { t, reader, url: new URL(`${t.url}/2?u=${reader.name}`, page.url()).href };
}

test("Post date link", async ({ page, context, session }) => {
  const { t, reader, url } = await replied(page, context, session);
  const date = page.locator("#post_2 .post-info.post-date a");
  await expect(date).toHaveAttribute("href", `${t.url}/2?u=${reader.name}`);
  const before = page.url();
  await date.click();
  await expect(modal(page), "shows the share modal").toBeVisible();
  expect(page.url(), "it does not route to post #2").toBe(before);
  await expect(modal(page).locator("#discourse-modal-title")).toHaveText("Share Post #2");
  await expect(modal(page).locator("label[for='invite-link']")).toHaveText("Share a link to this post:");
  await expect(link(page)).toHaveValue(url);
  // longDateNoYear of a post made this year: "Nov 11, 3:04 PM".
  await expect(modal(page).locator(".d-modal__subtitle-text")).toHaveText(/^[A-Z][a-z]{2} \d{1,2}, \d{1,2}:\d{2} [AP]M$/);

  // replyAsNewTopic continues from the post.
  await modal(page).locator(".link-share-actions .new-topic").click();
  const [[title]] = sql("SELECT title FROM topics WHERE id = :'p1'", t.id);
  await expect(editor(page)).toHaveValue(
    `Continuing the discussion from [${title}](${new URL(`${t.url}/2`, page.url()).href}):\n\n`,
  );
});

// PostMenuShareButton is not in the default post_menu (tests/sessions.rs
// draws it from the setting, which the specs share), so the button is put
// in the post's menu here for topic.js' handler.
test("the post menu's share opens the modal for the post without a native share", async ({ page, context, session }) => {
  const { url } = await replied(page, context, session);
  await page.locator("#post_2 nav.post-controls .actions").evaluate((actions) => {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "btn no-text btn-icon post-action-menu__share share btn-flat";
    actions.prepend(button);
  });
  await page.locator("#post_2 .post-action-menu__share").click();
  await expect(modal(page).locator("#discourse-modal-title")).toHaveText("Share Post #2");
  await expect(link(page)).toHaveValue(url);
});

test("Share topic in a restricted category", async ({ page, context, session }) => {
  const admin = await session("admin");
  // The fixture's Staff category, which the staff group alone sees.
  const first = await api(admin, "POST", "/posts.json", {
    title: label("Share restricted"),
    raw: label("A topic only the staff group sees"),
    category: "3",
  });
  await login(context, "admin");
  await page.goto(`/t/${first.topic_slug}/${first.topic_id}`);
  await page.locator("#topic-footer-button-share-and-invite").click();
  await expect(modal(page)).toBeVisible();
  const alert = modal(page).locator("#modal-alert.alert-warning");
  await expect(alert, "it shows restricted warning").toHaveText("Only visible to members of group: staff");
  await expect(alert).toHaveAttribute("role", "alert");
  // A restricted category is a private context: email alone.
  await expect(modal(page).locator(".link-share-actions .sources button[class*='share-']")).toHaveClass([/share-email/]);
});
