// Demo steps 14 and 15 on a running server: the chat drawer
// (chat-drawer.js) opened from the header, a channel in it (chat.js), a
// message sent and arriving live in another member's session over the
// message bus, a reaction; the drawer kept across page swaps, collapsed
// and expanded, switched to full-page chat and back, closed; its resizer
// (223b) on the real drawer, the size kept for the next page.
const { test, expect, login, api, sql, label } = require("./helpers");

const CHANNEL = 2; // the seed's General channel

async function openDrawer(page) {
  await page.locator(".d-header .chat-header-icon a, .d-header .chat-header-icon button").first().click();
  const drawer = page.locator(".chat-drawer");
  await expect(drawer).toBeVisible();
  return drawer;
}

async function openGeneral(drawer) {
  await drawer.locator(".chat-channel-row", { hasText: "General" }).first().click();
  const composer = drawer.locator(".chat-composer__input");
  await expect(composer).toBeVisible();
  return composer;
}

test("a message sent from the drawer arrives live in another member's full-page chat", async ({
  page,
  context,
  session,
}) => {
  const message = label("drawer message");
  await login(context, "user1");
  const user0 = await session("user0");
  const other = await user0.newPage();
  await other.goto(`/chat/c/general/${CHANNEL}`);
  await expect(other.locator(".full-page-chat .chat-composer__input")).toBeVisible();

  await page.goto("/latest");
  const drawer = await openDrawer(page);
  const composer = await openGeneral(drawer);
  await expect(composer).toBeFocused();
  await composer.pressSequentially(message, { delay: 5 });
  await composer.press("Enter");

  await expect(drawer.locator(".chat-message-container", { hasText: message })).toBeVisible();
  await expect(composer).toHaveValue("");
  await expect
    .poll(() =>
      sql(
        "SELECT u.username FROM chat_messages m JOIN users u ON u.id = m.user_id WHERE m.chat_channel_id = :'p1' AND m.message = :'p2'",
        CHANNEL,
        message,
      ),
    )
    .toEqual([["user1"]]);
  // No reload on the other side: the message bus brings it.
  await expect(other.locator(".chat-message-container", { hasText: message })).toBeVisible({ timeout: 20_000 });
});

test("a reaction from the message's hover actions", async ({ page, context, session }) => {
  const message = label("message to react to");
  await login(context, "user1");
  const user0 = await session("user0");
  const other = await user0.newPage();
  await other.goto(`/chat/c/general/${CHANNEL}`);
  const otherComposer = other.locator(".full-page-chat .chat-composer__input");
  await otherComposer.fill(message);
  await otherComposer.press("Enter");
  await expect(other.locator(".chat-message-container", { hasText: message })).toBeVisible();

  await page.goto(`/chat/c/general/${CHANNEL}`);
  const sent = page.locator(".chat-message-container", { hasText: message });
  await expect(sent).toBeVisible();
  await sent.hover();
  await page.locator(".chat-message-actions .react-btn").click();
  const picker = page.locator(".emoji-picker");
  await expect(picker).toBeVisible();
  await picker.locator("input.filter-input, input[type=search]").first().fill("tada");
  await picker.locator("img.emoji[data-emoji='tada']:visible").first().click();
  await expect(sent.locator(".chat-message-reaction")).toBeVisible();
  await expect
    .poll(() =>
      sql(
        "SELECT r.emoji, u.username FROM chat_message_reactions r JOIN chat_messages m ON m.id = r.chat_message_id JOIN users u ON u.id = r.user_id WHERE m.message = :'p1'",
        message,
      ),
    )
    .toEqual([["tada", "user1"]]);
  // The author sees it arrive.
  await expect(other.locator(".chat-message-container", { hasText: message }).locator(".chat-message-reaction")).toBeVisible({
    timeout: 20_000,
  });
});

test("a refused read marker neither alerts nor throws", async ({ page, context, session }) => {
  // Another tab or session can read further first; the server then
  // refuses the older id (ensure_message_id_recency, a 400), and Ember's
  // updateLastReadMessage shows nothing.
  await login(context, "user1");
  let refused = 0;
  await page.route("**/chat/api/channels/*/read?*", (route) => {
    refused += 1;
    return route.fulfill({ status: 400, contentType: "application/json", body: '{"errors":["stale"]}' });
  });
  const dialogs = [];
  page.on("dialog", (d) => {
    dialogs.push(d.message());
    d.dismiss();
  });
  await page.goto(`/chat/c/general/${CHANNEL}`);
  await expect(page.locator(".full-page-chat .chat-composer__input")).toBeVisible();

  // A message arriving live, read once the pane settles (didResizePane's
  // read on a resize, as on a scroll).
  const message = label("message read while the marker is refused");
  await api(await session("user0"), "POST", `/chat/${CHANNEL}.json`, { message, staged_id: label("staged") });
  await expect(page.locator(".chat-message-container", { hasText: message })).toBeVisible({ timeout: 20_000 });
  await page.setViewportSize({ width: 1380, height: 880 });
  await expect.poll(() => refused).toBeGreaterThan(0);
  await page.waitForTimeout(300);
  expect(dialogs).toEqual([]);
});

test("the drawer across page swaps, collapsed and expanded, full page and back, closed", async ({ page, context }) => {
  await login(context, "user1");
  await page.goto("/latest");
  const drawer = await openDrawer(page);
  await openGeneral(drawer);
  await expect(drawer).toHaveClass(/is-expanded/);
  const outletId = await page.locator(".chat-drawer-outlet").getAttribute("id");

  // A forum page swapped in keeps the drawer and its channel (hx-preserve).
  await page.evaluate(() => (window.__noReload = true));
  await page.locator("#main-outlet a.title, #main-outlet .topic-list a.title").first().click();
  // Settled: at the top, the scroll tracker drops a /1 the list's link
  // may carry (the last read post), as Ember does for the first post.
  await expect(page).toHaveURL(/\/t\/[^/]+\/\d+$/);
  await expect(page.locator(".topic-post").first()).toBeVisible();
  await expect(drawer.locator(".chat-composer__input")).toBeVisible();
  expect(await page.evaluate(() => window.__noReload)).toBe(true);
  expect(await page.locator(".chat-drawer-outlet").getAttribute("id")).toBe(outletId);

  // Collapse with the toggle; collapsed, the toggle is visually hidden and
  // a click on the navbar expands (Navbar @onClick).
  await drawer.locator(".c-navbar__toggle-drawer-button").click();
  await expect(drawer).toHaveClass(/is-collapsed/);
  await expect(drawer.locator(".chat-composer__input")).toHaveCount(0);
  await drawer.locator(".c-navbar-container.-clickable .chat-channel-name").click();
  await expect(drawer).toHaveClass(/is-expanded/);
  await expect(drawer.locator(".chat-composer__input")).toBeVisible();

  // Full page, then back to the forum page with the drawer open.
  const forumUrl = page.url();
  await drawer.locator(".c-navbar__full-page-button").click();
  await expect(page).toHaveURL(new RegExp(`/chat/c/general/${CHANNEL}$`));
  await expect(page.locator(".full-page-chat .chat-composer__input")).toBeVisible();
  await expect(page.locator(".chat-drawer")).toHaveCount(0);
  await page.locator(".c-navbar__open-drawer-button").click();
  await expect(page).toHaveURL(forumUrl);
  await expect(page.locator(".chat-drawer .chat-composer__input")).toBeVisible();
  expect(await page.evaluate(() => localStorage.getItem("discourse_chat_preferred_mode"))).toBe(
    JSON.stringify("DRAWER_CHAT"),
  );

  // Close, then Esc after "-" reopens it.
  await page.locator(".chat-drawer .c-navbar__close-drawer-button").click();
  await expect(page.locator(".chat-drawer")).toHaveCount(0);
  await page.locator("body").press("-");
  await expect(page.locator(".chat-drawer")).toBeVisible();
  await page.locator("body").click({ position: { x: 5, y: 300 } });
  await page.keyboard.press("Escape");
  await expect(page.locator(".chat-drawer")).toHaveCount(0);
});

test("the drawer's resizer sets its size, which the next page keeps", async ({ page, context }) => {
  await login(context, "user1");
  await page.goto("/latest");
  const drawer = await openDrawer(page);
  await expect(drawer).toHaveAttribute("style", /width: 400px;\s*height: 530px;/);

  const box = await drawer.locator(".chat-drawer-resizer").boundingBox();
  const x = box.x + box.width / 2;
  const y = box.y + box.height / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x - 60, y - 40, { steps: 5 });
  await page.mouse.move(x - 100, y - 50, { steps: 5 });
  await page.mouse.up();
  await expect(drawer).toHaveAttribute("style", /width: 500px/);
  await expect(drawer).toHaveAttribute("style", /height: 580px/);
  expect(
    await page.evaluate(() => [
      localStorage.getItem("discourse_chat_drawer_size_width"),
      localStorage.getItem("discourse_chat_drawer_size_height"),
    ]),
  ).toEqual(["500", "580"]);

  // A reload loses the drawer as Ember does; opened again, it has the size.
  await page.reload();
  await expect(page.locator(".chat-drawer")).toHaveCount(0);
  const again = await openDrawer(page);
  await expect(again).toHaveAttribute("style", /width: 500px;\s*height: 580px;/);
});
