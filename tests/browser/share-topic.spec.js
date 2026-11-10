// The share modal (share-topic.js, components/modal/share-topic.gjs), as
// acceptance/share-topic-test.js opens it from the topic footer: the
// topic's link selected in its input, the copy button, the share_links
// sources and the ways to close it.
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
  const sources = modal(page).locator(".link-share-actions .sources button");
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
  await expect(modal(page).locator(".link-share-actions .sources button")).toHaveClass([/share-email/]);
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
