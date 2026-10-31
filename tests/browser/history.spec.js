// The browser's back and forward on a running server: htmx's history
// restore of boosted forum pages, and 223d's chat URLs (chat-drawer.js on
// htmx:historyCacheMiss): back to a chat URL with the drawer preferred
// opens it in the drawer over the forum page left, forward restores that
// page, full page preferred loads full-page chat. chat-drawer-fixture.spec.js
// drives the same script over a fixture app; this runs it against the
// server's own pages, drawer route and history.
const { test, expect, login } = require("./helpers");

const CHANNEL = 2; // the seed's General channel

const prefer = (page, mode) =>
  page.evaluate((mode) => localStorage.setItem("discourse_chat_preferred_mode", JSON.stringify(mode)), mode);

// A mark on the window that a full load would drop: back and forward
// restore pages in the same document.
async function markDocument(page) {
  await page.evaluate(() => (window.__sameDocument = true));
}
async function expectSameDocument(page) {
  expect(await page.evaluate(() => window.__sameDocument === true), "the page reloaded").toBe(true);
}

test("back and forward between forum pages restore them in place", async ({ page, context }) => {
  await login(context, "user1");
  await page.goto("/latest");
  await markDocument(page);
  const link = page.locator("#main-outlet .topic-list a.title").first();
  const id = await link.locator("xpath=ancestor::tr[@data-topic-id]").getAttribute("data-topic-id");
  await link.click();
  // Settled: at the top, the scroll tracker drops a /1 the list's link
  // may carry (the last read post), as Ember does for the first post.
  await expect(page).toHaveURL(/\/t\/[^/]+\/\d+$/);
  const topicUrl = page.url();
  await expect(page.locator("#topic-title h1")).toHaveAttribute("data-topic-id", id);

  await page.goBack();
  await expect(page).toHaveURL(/\/latest$/);
  await expect(page.locator("#main-outlet .topic-list a.title").first()).toBeVisible();
  await expect(page.locator("#topic-title")).toHaveCount(0);
  await expectSameDocument(page);

  await page.goForward();
  await expect(page).toHaveURL(topicUrl);
  await expect(page.locator("#topic-title h1")).toHaveAttribute("data-topic-id", id);
  await expect(page.locator(".topic-post").first()).toBeVisible();
  await expectSameDocument(page);
});

// The live stream redraws sidebar sections out of band (outerHTML) as
// counts change: one replacing the clicked link while its page is on the
// way must not leave the address bar on the page before.
test("a link replaced by a live redraw while its page loads still pushes its URL", async ({ page, context }) => {
  await login(context, "admin");
  await page.goto("/latest");
  await markDocument(page);
  let release;
  const held = new Promise((resolve) => (release = resolve));
  await page.route(/\/review$/, async (route) => {
    await held;
    await route.continue();
  });
  const link = page.locator("#d-sidebar a[href='/review']");
  await link.click();
  // The section's redraw, as src/routes/live.rs swaps it.
  await page.evaluate(() => {
    const section = document.querySelector("#d-sidebar a[href='/review']").closest(".sidebar-section");
    const swap = section.cloneNode(true);
    swap.setAttribute("hx-swap-oob", `outerHTML:.sidebar-section[data-section-name='${section.dataset.sectionName}']`);
    window.htmx.swap(document.body, swap.outerHTML, { swapStyle: "none" });
  });
  release();
  await expect(page.locator(".reviewable-filters, .reviewables").first()).toBeVisible();
  await expect(page).toHaveURL(/\/review$/);
  await expectSameDocument(page);
});

test("back to a chat URL opens it in the drawer, forward restores the forum page", async ({ page, context }) => {
  await login(context, "user1");
  await page.goto("/latest");
  await prefer(page, "FULL_PAGE_CHAT");
  await page.goto(`/chat/c/general/${CHANNEL}`);
  await expect(page.locator(".full-page-chat .chat-composer__input")).toBeVisible();
  await markDocument(page);

  // Full-page chat's open drawer button: the forum page before it, the
  // drawer at the chat URL, the drawer preferred from now on.
  await page.locator(".c-navbar__open-drawer-button").click();
  await expect(page).toHaveURL(/\/latest$/);
  await expect(page.locator(".chat-drawer .chat-composer__input")).toBeVisible();
  await page.locator(".chat-drawer .c-navbar__close-drawer-button").click();
  await expect(page.locator(".chat-drawer")).toHaveCount(0);

  // Back to the chat URL: the drawer opens on it over the forum page, the
  // address bar on the chat URL as Ember leaves it.
  await page.goBack({ waitUntil: "commit" });
  await expect(page.locator(".chat-drawer .chat-composer__input")).toBeVisible();
  await expect(page).toHaveURL(new RegExp(`/chat/c/general/${CHANNEL}$`));
  await expect(page.locator("#main-outlet .topic-list")).toBeVisible();
  await expect(page.locator(".full-page-chat")).toHaveCount(0);
  await expectSameDocument(page);

  // Forward: the forum page again.
  await page.goForward({ waitUntil: "commit" });
  await expect(page).toHaveURL(/\/latest$/);
  await expect(page.locator("#main-outlet .topic-list")).toBeVisible();
  await expectSameDocument(page);

  // Full page preferred: back loads full-page chat, the drawer closed.
  await prefer(page, "FULL_PAGE_CHAT");
  await page.goBack({ waitUntil: "commit" });
  await expect(page.locator(".full-page-chat .chat-composer__input")).toBeVisible();
  await expect(page).toHaveURL(new RegExp(`/chat/c/general/${CHANNEL}$`));
  await expect(page.locator(".chat-drawer")).toHaveCount(0);
});

test("a forum page swapped in takes the focus from the drawer's composer", async ({ page, context }) => {
  await login(context, "user1");
  await page.goto("/latest");
  await prefer(page, "DRAWER_CHAT");
  await page.locator("body").press("-");
  const drawer = page.locator(".chat-drawer");
  await expect(drawer).toBeVisible();
  await drawer.locator(".chat-channel-row", { hasText: "General" }).first().click();
  const composer = drawer.locator(".chat-composer__input");
  await expect(composer).toBeFocused();

  await page.locator("#main-outlet .topic-list a.title").first().click();
  await expect(page).toHaveURL(/\/t\//);
  await expect(page.locator(".topic-post").first()).toBeVisible();
  await expect(composer).toBeVisible();
  expect(await page.evaluate(() => document.activeElement === document.body)).toBe(true);
});
