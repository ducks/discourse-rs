// 223c's drawer routes on a running server (chat-drawer.js over GET
// /live/chat/drawer): a channel's settings and members (from the channel
// title, the info nav and a browse card), its pins, its threads list and
// a thread (chat-thread.js in the drawer), the member's threads (the
// footer's item) and search (chat-search.js), each opened from the
// drawer's own UI, with the drawer's back button, and the browser's back
// to a thread's URL opening it in the drawer. Each test works in a channel
// of its own with threading, so the other specs' General stays as it is.
const { test, expect, login, api, sql, label } = require("./helpers");

const prefer = (page, mode) =>
  page.evaluate((mode) => localStorage.setItem("discourse_chat_preferred_mode", JSON.stringify(mode)), mode);

// A category channel of the test's own, with threading, which user0 and
// user1 follow: user0's message and user1's reply in its thread (both
// through the API, so the search index has them).
async function channelWithThread(session) {
  const name = label("Drawer routes");
  const slug = name.toLowerCase().replace(/[^a-z0-9]+/g, "-");
  const [[id]] = sql(
    "INSERT INTO chat_channels (chatable_type, chatable_id, type, status, user_count, name, slug, threading_enabled, created_at, updated_at) VALUES ('Category', 4, 'CategoryChannel', 0, 2, :'p1', :'p2', true, now(), now()) RETURNING id",
    name,
    slug,
  );
  sql(
    "INSERT INTO user_chat_channel_memberships (user_id, chat_channel_id, following, notification_level, created_at, updated_at) SELECT id, :'p1', true, 2, now(), now() FROM users WHERE username IN ('user0', 'user1')",
    id,
  );
  const original = label("original message");
  const reply = label("thread reply");
  await api(await session("user0"), "POST", `/chat/${id}.json`, { message: original, staged_id: label("staged") });
  const [[originalId]] = sql("SELECT id FROM chat_messages WHERE message = :'p1'", original);
  await api(await session("user1"), "POST", `/chat/${id}.json`, {
    message: reply,
    in_reply_to_id: originalId,
    staged_id: label("staged"),
  });
  await expect.poll(() => sql("SELECT thread_id FROM chat_messages WHERE message = :'p1'", reply)[0][0]).not.toBe("");
  const [[threadId]] = sql("SELECT thread_id FROM chat_messages WHERE message = :'p1'", reply);
  // The lists show a thread once Jobs::Chat::UpdateThreadReplyCount counts
  // its reply.
  await expect
    .poll(() => sql("SELECT replies_count FROM chat_threads WHERE id = :'p1'", threadId)[0][0], { timeout: 30_000 })
    .not.toBe("0");
  return { id, name, slug, original, originalId, reply, threadId, url: `/chat/c/${slug}/${id}` };
}

async function openDrawer(page) {
  await page.goto("/latest");
  await prefer(page, "DRAWER_CHAT");
  await page.locator(".d-header .chat-header-icon a").first().click();
  const drawer = page.locator(".chat-drawer");
  await expect(drawer).toBeVisible();
  return drawer;
}

async function openChannel(drawer, channel) {
  await drawer.locator(".chat-channel-row", { hasText: channel.name }).first().click();
  await expect(drawer.locator(".c-drawer-routes.--channel .chat-composer__input")).toBeVisible();
}

test("the channel title opens its settings, the nav its members, back returns to the channel", async ({
  page,
  context,
  session,
}) => {
  const channel = await channelWithThread(session);
  await login(context, "user1");
  const drawer = await openDrawer(page);
  await openChannel(drawer, channel);

  await drawer.locator(".c-navbar__channel-title").click();
  const settings = drawer.locator(".c-drawer-routes.--channel-info-settings");
  await expect(settings.locator(".c-channel-settings")).toBeVisible();
  await expect(settings.locator(".c-navbar__back-button")).toHaveAttribute("title", "Return to channel");
  await expect(settings.locator(".c-channel-info__nav a.active")).toHaveText("Settings");
  expect(await page.evaluate(() => location.pathname)).toBe("/latest");

  // The mute switch saves and the route is drawn again (chat-channel-info.js
  // through the drawer's reload).
  await settings.locator(".d-toggle-switch__label:has(.c-channel-settings__mute-switch)").click();
  await expect(drawer.locator(".c-channel-settings__mute-switch")).toHaveAttribute("aria-checked", "true");
  await expect
    .poll(() =>
      sql("SELECT muted FROM user_chat_channel_memberships m JOIN users u ON u.id = m.user_id WHERE u.username = 'user1' AND m.chat_channel_id = :'p1'", channel.id),
    )
    .toEqual([["t"]]);

  // The nav's members tab: the channel's members.
  await drawer.locator(".c-channel-info__nav a", { hasText: "Members" }).click();
  const members = drawer.locator(".c-drawer-routes.--channel-info-members");
  await expect(members.locator(".c-channel-members__list-item.-member[data-username='user0']")).toBeVisible();
  await expect(members.locator(".c-channel-members__list-item.-member[data-username='user1']")).toBeVisible();
  await expect(members.locator(".c-channel-info__nav a.active")).toContainText("Members");

  // The drawer's back button: the channel.
  await members.locator(".c-navbar__back-button").click();
  await expect(drawer.locator(".c-drawer-routes.--channel .chat-composer__input")).toBeVisible();

  // Collapsed and expanded on the info route: the navbar alone, then the
  // route again.
  await drawer.locator(".c-navbar__channel-title").click();
  await expect(drawer.locator(".c-channel-settings")).toBeVisible();
  await drawer.locator(".c-navbar__toggle-drawer-button").click();
  await expect(drawer).toHaveClass(/is-collapsed/);
  await expect(drawer.locator(".c-channel-settings")).toHaveCount(0);
  // Collapsed, the navbar's click expands (its toggle is visually hidden).
  await drawer.locator(".c-navbar-container.-clickable .chat-channel-name").click();
  await expect(drawer).toHaveClass(/is-expanded/);
  await expect(drawer.locator(".c-channel-settings")).toBeVisible();
});

test("a browse card's members link opens the channel's members in the drawer", async ({ page, context, session }) => {
  const channel = await channelWithThread(session);
  await login(context, "user1");
  const drawer = await openDrawer(page);
  // The channels list's options: Browse channels.
  await drawer.locator(".chat-channel-list-options-button").first().click();
  await page.locator("[data-menu-option-id='browseChannels']").click();
  await expect(drawer.locator(".c-drawer-routes.--browse")).toBeVisible();
  const card = drawer.locator(".chat-channel-card", { hasText: channel.name });
  await expect(card).toBeVisible();
  await card.locator(".chat-channel-card__members").click();
  const members = drawer.locator(".c-drawer-routes.--channel-info-members");
  await expect(members.locator(".c-channel-members__list-item.-member[data-username='user0']")).toBeVisible();
  // Back goes to the channel the route belongs to.
  await members.locator(".c-navbar__back-button").click();
  await expect(drawer.locator(".c-drawer-routes.--channel .chat-composer__input")).toBeVisible();
  await expect(drawer.locator(".c-navbar__channel-title")).toContainText(channel.name);
});

test("the pinned messages button opens the channel's pins, read once shown", async ({ page, context, session }) => {
  const channel = await channelWithThread(session);
  await api(await session("admin"), "POST", `/chat/api/channels/${channel.id}/messages/${channel.originalId}/pin`, {});
  await login(context, "user1");
  const drawer = await openDrawer(page);
  await openChannel(drawer, channel);

  await drawer.locator(".c-navbar__pinned-messages-btn").click();
  const pins = drawer.locator(".c-drawer-routes.--channel-pins");
  await expect(pins.locator(".c-navbar__title")).toHaveText(`Pinned messages - ${channel.name}`);
  await expect(pins.locator(".chat-pinned-message", { hasText: channel.original })).toBeVisible();
  await expect
    .poll(() =>
      sql(
        "SELECT last_viewed_pins_at IS NOT NULL FROM user_chat_channel_memberships m JOIN users u ON u.id = m.user_id WHERE u.username = 'user1' AND m.chat_channel_id = :'p1'",
        channel.id,
      ),
    )
    .toEqual([["t"]]);

  // A pin links to its message: the channel around it.
  await pins.locator(".chat-pinned-message__link").first().click();
  await expect(drawer.locator(".c-drawer-routes.--channel .chat-message-container", { hasText: channel.original })).toBeVisible();
  // And the back button from the pins: the channel.
  await drawer.locator(".c-navbar__pinned-messages-btn").click();
  await expect(drawer.locator(".c-drawer-routes.--channel-pins")).toBeVisible();
  await drawer.locator(".c-drawer-routes.--channel-pins .c-navbar__back-button").click();
  await expect(drawer.locator(".c-drawer-routes.--channel .chat-composer__input")).toBeVisible();
});

test("the threads list opens a thread, its reply sent from the drawer, back to the list and the channel", async ({
  page,
  context,
  session,
}) => {
  const channel = await channelWithThread(session);
  await login(context, "user1");
  const drawer = await openDrawer(page);
  await openChannel(drawer, channel);

  await drawer.locator(".c-navbar__threads-list-button").click();
  const list = drawer.locator(".c-drawer-routes.--channel-threads");
  await expect(list.locator(".c-navbar__title")).toHaveText(`Threads - ${channel.name}`);
  const item = list.locator(`.chat-thread-list-item[data-thread-id='${channel.threadId}']`);
  await expect(item).toBeVisible();

  await item.locator(".chat-thread-list-item__open-button").click();
  const thread = drawer.locator(".c-drawer-routes.--channel-thread");
  await expect(thread.locator(".chat-thread .chat-message-container", { hasText: channel.reply })).toBeVisible();
  await expect(thread.locator(".c-navbar__back-button")).toHaveAttribute("title", "Return to threads list");
  expect(await page.evaluate(() => location.pathname)).toBe("/latest");

  // A reply from the drawer's thread composer, into the thread.
  const message = label("drawer thread reply");
  const composer = thread.locator("#thread-composer");
  await expect(composer).toBeFocused();
  await composer.pressSequentially(message, { delay: 5 });
  await composer.press("Enter");
  await expect(thread.locator(".chat-message-container", { hasText: message })).toBeVisible();
  await expect
    .poll(() => sql("SELECT thread_id FROM chat_messages WHERE message = :'p1'", message))
    .toEqual([[channel.threadId]]);

  // Back: the threads list, then the channel.
  await thread.locator(".c-navbar__back-button").click();
  await expect(drawer.locator(".c-drawer-routes.--channel-threads .chat-thread-list-item")).toHaveCount(1);
  await drawer.locator(".c-drawer-routes.--channel-threads .c-navbar__back-button").click();
  await expect(drawer.locator(".c-drawer-routes.--channel .chat-composer__input")).toBeVisible();

  // Escape in the thread's composer goes back to the channel (handleEscape).
  await drawer.locator(".c-navbar__threads-list-button").click();
  await drawer.locator(".chat-thread-list-item__open-button").first().click();
  await expect(drawer.locator("#thread-composer")).toBeFocused();
  await drawer.locator("#thread-composer").press("Escape");
  await expect(drawer.locator(".c-drawer-routes.--channel .chat-composer__input")).toBeVisible();
  await expect(drawer).toBeVisible();
});

test("the footer's threads item lists the member's threads, a thread backs to them", async ({ page, context, session }) => {
  const channel = await channelWithThread(session);
  await login(context, "user1");
  const drawer = await openDrawer(page);
  await expect(drawer.locator(".c-drawer-routes.--channels")).toBeVisible();

  await drawer.locator("#c-footer-threads").click();
  const threads = drawer.locator(".c-drawer-routes.--threads");
  await expect(drawer.locator("#c-footer-threads")).toHaveClass(/--active/);
  const row = threads.locator(`.c-user-thread[data-id='${channel.threadId}']`);
  await expect(row).toBeVisible();

  await row.locator(".chat__thread-title__name").click();
  const thread = drawer.locator(".c-drawer-routes.--channel-thread");
  await expect(thread.locator(".chat-message-container", { hasText: channel.reply })).toBeVisible();
  await expect(thread.locator(".c-navbar__back-button")).toHaveAttribute("title", "My Threads");
  await thread.locator(".c-navbar__back-button").click();
  await expect(drawer.locator(`.c-drawer-routes.--threads .c-user-thread[data-id='${channel.threadId}']`)).toBeVisible();
});

test("search finds a message, sorts, opens it, and keeps its query", async ({ page, context, session }) => {
  const channel = await channelWithThread(session);
  await login(context, "user1");
  const drawer = await openDrawer(page);

  // The sidebar's search link, a chat route the drawer takes.
  await page.locator("#d-sidebar a[href$='/chat/search']").click();
  const search = drawer.locator(".c-drawer-routes.--search");
  const input = search.locator(".c-search .filter-input");
  await expect(input).toBeFocused();
  const word = channel.original.split(" ").pop();
  await input.fill(word);
  const entry = search.locator(".chat-message-search-entry", { hasText: channel.original });
  await expect(entry).toBeVisible();
  await expect(entry.locator(".chat-channel-title")).toContainText(channel.name);
  await expect(search.locator(".chat-message-search-entry")).toHaveCount(1);

  // The sort menu: latest, searched again.
  await search.locator(".search-sort-options-trigger").click();
  await page.locator(".search-sort-options-content [data-sort='latest']").click();
  await expect(search.locator(".search-sort-options-trigger .d-button-label")).toHaveText("Latest");
  await expect(entry).toBeVisible();

  // Nothing found: the alert.
  await input.fill(label("nothing").replace(/ /g, ""));
  await expect(search.locator(".alert.alert-info")).toHaveText("No results found");

  // A result opens its message in the drawer: the message is its
  // thread's original (ChatMessage#url with its thread_id), so the thread
  // around it.
  await input.fill(word);
  await expect(entry).toBeVisible();
  await entry.click();
  await expect(
    drawer.locator(".c-drawer-routes.--channel-thread .chat-message-container", { hasText: channel.original }),
  ).toBeVisible();

  // The query outlives the route (chatSearchQuery).
  await page.locator("#d-sidebar a[href$='/chat/search']").click();
  await expect(drawer.locator(".c-search .filter-input")).toHaveValue(word);
  await expect(drawer.locator(".chat-message-search-entry", { hasText: channel.original })).toBeVisible();
});

test("back to a thread's URL opens the thread in the drawer, forward restores the forum page", async ({
  page,
  context,
  session,
}) => {
  const channel = await channelWithThread(session);
  await login(context, "user1");
  await page.goto("/latest");
  await prefer(page, "FULL_PAGE_CHAT");
  const threadUrl = `${channel.url}/t/${channel.threadId}`;
  await page.goto(threadUrl);
  await expect(page.locator(".full-page-chat .chat-thread .chat-message-container", { hasText: channel.reply })).toBeVisible();
  await page.evaluate(() => (window.__sameDocument = true));

  // Full-page chat's open drawer button: the forum page, the drawer on
  // the thread.
  await page.locator(".c-navbar__open-drawer-button").click();
  await expect(page).toHaveURL(/\/latest$/);
  const drawer = page.locator(".chat-drawer");
  await expect(drawer.locator(".c-drawer-routes.--channel-thread .chat-message-container", { hasText: channel.reply })).toBeVisible();
  await drawer.locator(".c-navbar__close-drawer-button").click();
  await expect(drawer).toHaveCount(0);

  await page.goBack({ waitUntil: "commit" });
  await expect(page.locator(".chat-drawer .c-drawer-routes.--channel-thread .chat-message-container", { hasText: channel.reply })).toBeVisible();
  await expect(page).toHaveURL(new RegExp(`${threadUrl}$`));
  await expect(page.locator("#main-outlet .topic-list")).toBeVisible();
  expect(await page.evaluate(() => window.__sameDocument === true)).toBe(true);

  await page.goForward({ waitUntil: "commit" });
  await expect(page).toHaveURL(/\/latest$/);
  await expect(page.locator("#main-outlet .topic-list")).toBeVisible();
  expect(await page.evaluate(() => window.__sameDocument === true)).toBe(true);
});
