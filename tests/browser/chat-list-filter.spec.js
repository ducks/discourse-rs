// A channel list's filter (ChatChannelListPreferences): the menu's choice
// is saved, a filter that leaves the list empty draws
// ChatSidebarChannelListFilterEmptyState, whose button and the filter
// toggles (ChatChannelListFilterToggle, the sidebar's toggleChannelFilter)
// set the filter aside and apply it again without saving, and choosing a
// filter applies it. Full page with the sidebar, in the drawer and in the
// mobile view. Every test has members and direct messages of its own.
const { test, expect, login, api, sql, member } = require("./helpers");

const prefer = (page, mode) =>
  page.evaluate((mode) => localStorage.setItem("discourse_chat_preferred_mode", JSON.stringify(mode)), mode);

// A direct message between the context's member and another, without a
// message (so with nothing unread). Returns its channel's id.
async function directMessage(context, other) {
  const dm = await api(context, "POST", "/chat/api/direct-message-channels.json", { "target_usernames[]": other.name });
  return dm.channel.id;
}

// ChatChannelListFilters: all 0, active 1, unread 2, mentions 3.
const savedFilters = (user) =>
  sql(
    "SELECT chat_channel_list_filter, chat_channel_list_filter_starred, chat_channel_list_filter_dms FROM user_options WHERE user_id = :'p1'",
    user.id,
  )[0];

const saveFilter = (user, column, value) =>
  sql(`UPDATE user_options SET ${column} = :'p2' WHERE user_id = :'p1'`, user.id, value);

// The member's options saved by the menu: the path alone names them (a
// direct message's address holds the other member's name).
const optionsSaved = (page, user) =>
  page.waitForResponse((r) => r.request().method() === "PUT" && new URL(r.url()).pathname === `/u/${user.name}.json`);

// Chooses a filter from a list's options menu, and waits for its save.
async function chooseFilter(page, button, user, value) {
  await button.click();
  const menu = page.locator(".chat-channel-list-options-menu");
  await menu.locator("[data-menu-option-id='filterChannels']").click();
  const choice = page.locator(`.chat-channel-list-filter-menu [data-menu-option-id='${value}']`);
  await expect(choice).toBeVisible();
  const saved = optionsSaved(page, user);
  await choice.click();
  await saved;
  await expect(menu).toHaveCount(0);
}

test("the direct messages list's filter, its empty state, and the toggles", async ({ page, context }) => {
  const me = member();
  const other = member();
  await login(context, me.name);
  const id = await directMessage(context, other);
  await page.goto("/chat/direct-messages");

  const list = page.locator("#direct-message-channels");
  const row = list.locator(`.chat-channel-row[data-chat-channel-id='${id}']`);
  const empty = list.locator(".empty-state__container.--empty-channels-list");
  const divider = page.locator(".c-routes.--direct-messages .chat-channel-divider");
  const toggle = divider.locator(".chat-channel-list-filter-toggle");
  const section = page.locator(".sidebar-section[data-section-name='chat-dms']");
  const link = section.locator(`.sidebar-section-link.channel-${id}`);
  const sidebarToggle = section.locator("[data-sidebar-action-id='toggleChannelFilter']");

  // Unfiltered: the row, its sidebar link, and no toggle.
  await expect(row).toContainText(other.name);
  await expect(link).toBeVisible();
  await expect(toggle).toHaveCount(0);
  await expect(sidebarToggle).toHaveCount(0);

  // Unreads leaves nothing: the empty state where the rows were, and the
  // toggles that show all channels.
  await chooseFilter(page, divider.locator(".chat-channel-list-options-button"), me, "unread");
  await expect(empty.locator(".empty-state__title")).toHaveText("No channels match this filter.");
  await expect(empty.locator(".empty-state__cta .btn-primary")).toHaveText("Show all channels");
  await expect(empty.locator(".empty-state__image svg")).toBeVisible();
  await expect(row).toHaveCount(0);
  await expect(link).toHaveCount(0);
  await expect(toggle).toHaveAttribute("title", "Show all channels");
  await expect(toggle.locator("svg")).toHaveClass(/d-icon-filter-circle-xmark/);
  await expect(sidebarToggle).toHaveAttribute("title", "Show all channels");
  expect(savedFilters(me)).toEqual(["0", "0", "2"]);

  // The menu marks the saved filter.
  await divider.locator(".chat-channel-list-options-button").click();
  const menu = page.locator(".chat-channel-list-options-menu");
  await expect(menu.locator("[data-menu-option-id='filterChannels'] .d-button-label")).toHaveText("Unreads");
  await page.keyboard.press("Escape");
  await expect(menu).toHaveCount(0);

  // Show all channels sets the filter aside without saving it (and is
  // not the list's new message button).
  await empty.locator(".empty-state__cta .btn-primary").click();
  await expect(row).toBeVisible();
  await expect(empty).toHaveCount(0);
  await expect(link).toBeVisible();
  await expect(page.locator(".chat-modal-new-message")).toHaveCount(0);
  await expect(toggle).toHaveAttribute("title", "Apply filters to channels");
  await expect(toggle.locator("svg")).toHaveClass(/d-icon-filter(\s|$)/);
  await expect(sidebarToggle).toHaveAttribute("title", "Apply filters to channels");
  expect(savedFilters(me)).toEqual(["0", "0", "2"]);

  // The list's toggle applies the filter again.
  await toggle.click();
  await expect(empty).toBeVisible();
  await expect(row).toHaveCount(0);
  await expect(toggle).toHaveAttribute("title", "Show all channels");
  await expect(sidebarToggle).toHaveAttribute("title", "Show all channels");

  // The sidebar's toggle shows all channels, on another page too.
  await sidebarToggle.click();
  await expect(row).toBeVisible();
  await expect(sidebarToggle).toHaveAttribute("title", "Apply filters to channels");
  await page.goto("/latest");
  await expect(link).toBeVisible();
  await expect(sidebarToggle).toHaveAttribute("title", "Apply filters to channels");
  await sidebarToggle.click();
  await expect(link).toHaveCount(0);
  await expect(sidebarToggle).toHaveAttribute("title", "Show all channels");
  await sidebarToggle.click();
  await expect(link).toBeVisible();

  // Choosing a filter applies it, the one set aside too.
  await page.goto("/chat/direct-messages");
  await expect(row).toBeVisible();
  await chooseFilter(page, divider.locator(".chat-channel-list-options-button"), me, "unread");
  await expect(empty).toBeVisible();
  await expect(row).toHaveCount(0);
  await expect(toggle).toHaveAttribute("title", "Show all channels");

  // All channels resets it: the row, and no toggle.
  await chooseFilter(page, divider.locator(".chat-channel-list-options-button"), me, "all");
  await expect(row).toBeVisible();
  await expect(empty).toHaveCount(0);
  await expect(toggle).toHaveCount(0);
  await expect(sidebarToggle).toHaveCount(0);
  expect(savedFilters(me)).toEqual(["0", "0", "0"]);
});

test("a list without direct messages offers a new one, whatever the filter", async ({ page, context }) => {
  const me = member();
  saveFilter(me, "chat_channel_list_filter_dms", 2);
  await login(context, me.name);
  await page.goto("/chat/direct-messages");

  const empty = page.locator("#direct-message-channels .empty-state__container.--empty-channels-list");
  await expect(empty.locator(".empty-state__title")).toHaveText("You have not joined any direct messages yet!");
  await empty.locator(".empty-state__cta .btn-primary").click();
  await expect(page.locator(".chat-modal-new-message .chat-message-creator__search-input__input")).toBeFocused();
  expect(savedFilters(me)).toEqual(["0", "0", "2"]);
});

test("the drawer's lists show all channels and apply the filter in place", async ({ page, context }) => {
  const me = member();
  const other = member();
  const starred = member();
  await login(context, me.name);
  const id = await directMessage(context, other);
  const starredId = await directMessage(context, starred);
  await api(context, "PUT", `/chat/api/channels/${starredId}/memberships/me`, { starred: "true" });
  saveFilter(me, "chat_channel_list_filter_dms", 3);
  saveFilter(me, "chat_channel_list_filter_starred", 2);

  await page.goto("/latest");
  await prefer(page, "DRAWER_CHAT");
  await page.locator(".d-header .chat-header-icon a").first().click();
  const drawer = page.locator(".chat-drawer");
  await expect(drawer).toBeVisible();

  // The direct messages, the starred one among them.
  await drawer.locator("#c-footer-direct-messages").click();
  const dms = drawer.locator(".c-drawer-routes.--direct-messages");
  const row = dms.locator(`.chat-channel-row[data-chat-channel-id='${id}']`);
  const empty = dms.locator(".empty-state__container.--empty-channels-list");
  const toggle = dms.locator(".chat-channel-list-filter-toggle");
  await expect(empty.locator(".empty-state__title")).toHaveText("No channels match this filter.");
  await expect(dms.locator(".chat-channel-row")).toHaveCount(0);
  await expect(toggle).toHaveAttribute("title", "Show all channels");

  await empty.locator(".empty-state__cta .btn-primary").click();
  await expect(row).toBeVisible();
  await expect(dms.locator(".chat-channel-row")).toHaveCount(2);
  await expect(empty).toHaveCount(0);
  await expect(toggle).toHaveAttribute("title", "Apply filters to channels");
  await expect(page.locator(".chat-modal-new-message")).toHaveCount(0);
  // The page under the drawer has the direct message in its sidebar now.
  await expect(page.locator(`.sidebar-section[data-section-name='chat-dms'] .sidebar-section-link.channel-${id}`)).toBeVisible();
  await expect(page).toHaveURL(/\/latest$/);

  // The starred list has a filter of its own, still applied.
  await drawer.locator("#c-footer-starred").click();
  const stars = drawer.locator(".c-drawer-routes.--starred-channels");
  const starredRow = stars.locator(`.chat-channel-row[data-chat-channel-id='${starredId}']`);
  const starredEmpty = stars.locator(".empty-state__container.--empty-channels-list");
  const starredToggle = stars.locator(".chat-channel-list-filter-toggle");
  await expect(starredEmpty.locator(".empty-state__cta .btn-primary")).toHaveText("Show all channels");
  await expect(starredRow).toHaveCount(0);
  await starredToggle.click();
  await expect(starredRow).toBeVisible();
  await expect(starredToggle).toHaveAttribute("title", "Apply filters to channels");
  await starredToggle.click();
  await expect(starredEmpty).toBeVisible();
  await expect(starredRow).toHaveCount(0);

  // The direct messages' filter is still set aside, until its toggle.
  await drawer.locator("#c-footer-direct-messages").click();
  await expect(row).toBeVisible();
  await toggle.click();
  await expect(empty).toBeVisible();
  await expect(dms.locator(".chat-channel-row")).toHaveCount(0);
  await expect(toggle).toHaveAttribute("title", "Show all channels");
  expect(savedFilters(me)).toEqual(["0", "2", "3"]);
});

test("the mobile lists' navbar toggles the filter", async ({ page, context }) => {
  const me = member();
  const other = member();
  await login(context, me.name);
  const id = await directMessage(context, other);
  await api(context, "PUT", `/chat/api/channels/${id}/memberships/me`, { starred: "true" });
  saveFilter(me, "chat_channel_list_filter_dms", 2);
  saveFilter(me, "chat_channel_list_filter_starred", 3);
  // A phone's viewport, which page.js keeps the mobile_view cookie with.
  await page.setViewportSize({ width: 400, height: 760 });
  const base = new URL(test.info().project.use.baseURL);
  await context.addCookies([{ name: "mobile_view", value: "1", domain: base.hostname, path: "/" }]);

  await page.goto("/chat/direct-messages");
  await expect(page.locator("html")).toHaveClass(/mobile-view/);
  const dms = page.locator(".c-routes.--direct-messages");
  const row = dms.locator(`.chat-channel-row[data-chat-channel-id='${id}']`);
  const empty = dms.locator(".empty-state__container.--empty-channels-list");
  const toggle = dms.locator(".c-navbar__actions .chat-channel-list-filter-toggle");
  await expect(empty.locator(".empty-state__title")).toHaveText("No channels match this filter.");
  await expect(row).toHaveCount(0);
  await expect(toggle).toHaveAttribute("title", "Show all channels");

  await empty.locator(".empty-state__cta .btn-primary").click();
  await expect(row).toBeVisible();
  await expect(empty).toHaveCount(0);
  await expect(toggle).toHaveAttribute("title", "Apply filters to channels");
  await expect(page.locator(".chat-modal-new-message")).toHaveCount(0);

  await toggle.click();
  await expect(empty).toBeVisible();
  await expect(row).toHaveCount(0);

  // The starred list, by its own filter.
  await page.goto("/chat/starred-channels");
  const stars = page.locator(".c-routes.--starred-channels");
  const starredRow = stars.locator(`.chat-channel-row[data-chat-channel-id='${id}']`);
  const starredEmpty = stars.locator(".empty-state__container.--empty-channels-list");
  const starredToggle = stars.locator(".c-navbar__actions .chat-channel-list-filter-toggle");
  await expect(starredEmpty).toBeVisible();
  await expect(starredRow).toHaveCount(0);
  await starredToggle.click();
  await expect(starredRow).toBeVisible();
  await expect(starredToggle).toHaveAttribute("title", "Apply filters to channels");

  // Choosing a filter from the navbar's menu applies it.
  await chooseFilter(page, stars.locator(".c-navbar__actions .chat-channel-list-options-button"), me, "unread");
  await expect(starredEmpty).toBeVisible();
  await expect(starredRow).toHaveCount(0);
  await expect(starredToggle).toHaveAttribute("title", "Show all channels");
  expect(savedFilters(me)).toEqual(["0", "2", "2"]);
});
