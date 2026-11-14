// The channel list options menu's direct message items
// (ChatChannelListSidebarMenu: startDm, startGroupChat) and the
// members route's add members mode (ChatRouteChannelInfoMembers over
// MessageCreator's AddMembers), full page and in the drawer. Every test
// has members of its own, so no direct message is shared between specs.
const { test, expect, login, api, sql, member } = require("./helpers");

const prefer = (page, mode) =>
  page.evaluate((mode) => localStorage.setItem("discourse_chat_preferred_mode", JSON.stringify(mode)), mode);

// A direct message between the context's member and another, without a
// message. Returns its channel's id.
async function directMessage(context, other) {
  const dm = await api(context, "POST", "/chat/api/direct-message-channels.json", { "target_usernames[]": other.name });
  return dm.channel.id;
}

const dmUsers = (id) =>
  sql(
    "SELECT u.username FROM direct_message_users d JOIN chat_channels c ON c.chatable_id = d.direct_message_channel_id AND c.chatable_type = 'DirectMessage' JOIN users u ON u.id = d.user_id WHERE c.id = :'p1'",
    id,
  )
    .map((row) => row[0])
    .sort();

const following = (id, user) =>
  sql(
    "SELECT following, notification_level FROM user_chat_channel_memberships WHERE chat_channel_id = :'p1' AND user_id = :'p2'",
    id,
    user.id,
  );

test("the direct messages list's options start a direct message or a group chat", async ({ page, context }) => {
  const me = member();
  const other = member();
  await login(context, me.name);
  await directMessage(context, other);
  await page.goto("/chat/direct-messages");

  const button = page.locator(".chat-channel-list-options-button:visible").first();
  await button.click();
  const menu = page.locator(".chat-channel-list-options-menu");
  // The section's items in the menu's order: no channel to create or
  // browse here, the two ways to start, then the filter and the sort.
  await expect(menu.locator("[data-menu-option-id]")).toHaveCount(4);
  expect(await menu.locator("[data-menu-option-id]").evaluateAll((els) => els.map((el) => el.dataset.menuOptionId))).toEqual([
    "startDm",
    "startGroupChat",
    "filterChannels",
    "sortChannels",
  ]);
  await expect(menu.locator("[data-menu-option-id='startDm'] .d-button-label")).toHaveText("New direct message");
  await expect(menu.locator("[data-menu-option-id='startGroupChat'] .d-button-label")).toHaveText("New group chat");

  await menu.locator("[data-menu-option-id='startDm']").click();
  await expect(menu).toHaveCount(0);
  const modal = page.locator(".chat-modal-new-message");
  await expect(modal.locator(".chat-message-creator__search-input__input")).toBeFocused();
  await expect(modal.locator("#new-group-chat")).toBeVisible();
  await modal.locator(".chat-message-creator__search-input__cancel-button").click();
  await expect(modal).toHaveCount(0);

  await button.click();
  await menu.locator("[data-menu-option-id='startGroupChat']").click();
  await expect(modal.locator(".chat-message-creator__new-group-header__input")).toBeVisible();
  await expect(modal.locator(".chat-message-creator__members-input")).toBeFocused();
  await expect(modal.locator(".chat-message-creator__members-count")).toHaveText("0/20 members");
});

test("add member on a direct message's members adds the chosen members and opens the channel", async ({
  page,
  context,
}) => {
  const me = member();
  const other = member();
  const third = member();
  await login(context, me.name);
  const id = await directMessage(context, other);
  await page.goto(`/chat/c/-/${id}/info/members`);

  const route = page.locator(".c-routes.--channel-info-members");
  const list = route.locator(".c-channel-members");
  const add = list.locator(".c-channel-members__list-item.-add-member");
  await expect(add).toHaveText("Add Member");
  await expect(list.locator(".c-channel-members__list-item.-member")).toHaveCount(2);

  // The item stays through the filter's results, which are the members'.
  const filtered = page.waitForResponse((r) => r.url().includes("/info/members/page") && r.url().includes(other.name));
  await list.locator(".c-channel-members__filter input").fill(other.name);
  await filtered;
  await expect(list.locator(".c-channel-members__list-item.-member")).toHaveCount(1);
  await expect(add).toBeVisible();

  // The creator takes the list's place in the add members mode, counting
  // from the channel's members.
  await add.click();
  const creator = route.locator(".chat-message-creator__add-members");
  await expect(creator).toBeVisible();
  await expect(list).toBeHidden();
  await expect(creator.locator(".chat-message-creator__members-count")).toHaveText("2/20 members");
  await expect(creator.locator(".chat-message-creator__members-input")).toBeFocused();
  await expect(creator.locator(".chat-message-creator__add-members-footer")).toHaveCount(0);

  // Cancel: the list again, its filter cleared.
  await creator.locator(".chat-message-creator__add-members__close-btn").click();
  await expect(route.locator(".chat-message-creator-container")).toHaveCount(0);
  await expect(list).toBeVisible();
  await expect(list.locator(".c-channel-members__filter input")).toHaveValue("");
  await expect(list.locator(".c-channel-members__list-item.-member")).toHaveCount(2);
  await expect(add).toBeVisible();

  // The search leaves out who is in the channel already
  // (excluded_memberships_channel_id).
  await add.click();
  const input = creator.locator(".chat-message-creator__members-input");
  const searched = page.waitForResponse(
    (r) => r.url().includes("/chat/api/chatables") && r.url().includes(`term=${other.name}`),
  );
  await input.fill(other.name);
  expect(new URL((await searched).url()).searchParams.get("excluded_memberships_channel_id")).toBe(String(id));
  await expect(creator.locator(".chat-message-creator__list-item")).toHaveCount(0);

  await input.fill(third.name);
  await creator.locator(".chat-message-creator__list-item", { hasText: third.name }).click();
  await expect(creator.locator(".chat-message-creator__member-username")).toHaveText(third.name);
  await expect(creator.locator(".chat-message-creator__members-count")).toHaveText("3/20 members");
  await expect(input).toHaveValue("");

  // A chosen member comes off again, and the footer with the last one.
  await creator.locator(".chat-message-creator__member").click();
  await expect(creator.locator(".chat-message-creator__members-count")).toHaveText("2/20 members");
  await expect(creator.locator(".chat-message-creator__add-members-footer")).toHaveCount(0);
  await input.fill(third.name);
  await creator.locator(".chat-message-creator__list-item", { hasText: third.name }).click();

  const footer = creator.locator(".chat-message-creator__add-members-footer");
  await expect(footer.locator(".btn-flat")).toHaveText("Cancel");
  await expect(footer.locator(".add-to-channel")).toHaveText("Add to channel");
  const saved = page.waitForRequest((r) => r.method() === "POST" && r.url().endsWith(`/chat/api/channels/${id}/memberships`));
  await footer.locator(".add-to-channel").click();
  expect((await saved).postData()).toBe(`usernames%5B%5D=${third.name}`);

  // Saved: the toast, and the channel with the notice of who was added.
  await expect(page.locator(".fk-d-toasts .fk-d-default-toast__message")).toHaveText("Saved!");
  await expect(page.locator(".full-page-chat .chat-composer__input")).toBeVisible();
  await expect(page).toHaveURL(new RegExp(`/chat/c/[^/]+/${id}$`));
  await expect(page.locator(".chat-message-text", { hasText: `@${third.name}` })).toBeVisible();

  // With the system user, whose notice it is (as Rails leaves it).
  expect(dmUsers(id)).toEqual([me.name, other.name, third.name, "system"].sort());
  expect(following(id, third)).toEqual([["t", "2"]]);
  expect(sql("SELECT user_count FROM chat_channels WHERE id = :'p1'", id)).toEqual([["3"]]);
  expect(sql("SELECT user_id FROM chat_messages WHERE chat_channel_id = :'p1'", id)).toEqual([["-1"]]);

  // A one to one with a message (the notice) takes no more members.
  await page.goto(`/chat/c/-/${id}/info/members`);
  await expect(list.locator(".c-channel-members__list-item.-member")).toHaveCount(3);
  await expect(add).toHaveCount(0);
});

test("add member in the drawer, by the keyboard, on a group chat", async ({ page, context }) => {
  const me = member();
  const other = member();
  const third = member();
  await login(context, me.name);
  const id = await directMessage(context, other);
  // A group's members can be added to whatever its messages.
  sql(
    "UPDATE direct_message_channels SET \"group\" = TRUE WHERE id = (SELECT chatable_id FROM chat_channels WHERE id = :'p1')",
    id,
  );
  await api(context, "POST", `/chat/${id}.json`, { message: "hello there" });

  await page.goto("/latest");
  await prefer(page, "DRAWER_CHAT");
  await page.locator(".d-header .chat-header-icon a").first().click();
  const drawer = page.locator(".chat-drawer");
  await drawer.locator("#c-footer-direct-messages").click();
  const row = drawer.locator(".chat-channel-row", { hasText: other.name });
  await expect(row).toBeVisible();

  // The drawer's list has the menu too.
  await drawer.locator(".chat-channel-list-options-button").first().click();
  const menu = page.locator(".chat-channel-list-options-menu");
  await expect(menu.locator("[data-menu-option-id='startDm']")).toBeVisible();
  await menu.locator("[data-menu-option-id='startGroupChat']").click();
  const modal = page.locator(".chat-modal-new-message");
  await expect(modal.locator(".chat-message-creator__new-group-header__input")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(modal).toHaveCount(0);

  await row.click();
  await drawer.locator(".c-navbar__channel-title").click();
  await drawer.locator(".c-channel-info__nav a", { hasText: "Members" }).click();
  const members = drawer.locator(".c-drawer-routes.--channel-info-members");
  const add = members.locator(".c-channel-members__list-item.-add-member");
  await expect(add).toBeVisible();

  // Enter on the item, as its click.
  await add.focus();
  await page.keyboard.press("Enter");
  const creator = members.locator(".chat-message-creator__add-members");
  await expect(creator.locator(".chat-message-creator__members-count")).toHaveText("2/20 members");
  const input = creator.locator(".chat-message-creator__members-input");
  await expect(input).toBeFocused();
  await input.fill(third.name);
  await expect(creator.locator(".chat-message-creator__list-item.-highlighted")).toContainText(third.name);
  await page.keyboard.press("Enter");
  await expect(creator.locator(".chat-message-creator__member-username")).toHaveText(third.name);

  // The footer's cancel is the route's too: the list is back.
  await creator.locator(".chat-message-creator__add-members-footer .btn-flat").click();
  await expect(members.locator(".chat-message-creator-container")).toHaveCount(0);
  await expect(members.locator(".c-channel-members__list-item.-member")).toHaveCount(2);

  await add.click();
  await input.fill(third.name);
  await expect(creator.locator(".chat-message-creator__list-item.-highlighted")).toContainText(third.name);
  await page.keyboard.press("Enter");
  await creator.locator(".add-to-channel").click();

  // The drawer goes to the channel; the page under it stays.
  await expect(drawer.locator(".c-drawer-routes.--channel .chat-composer__input")).toBeVisible();
  await expect(drawer.locator(".chat-message-text", { hasText: `@${third.name}` })).toBeVisible();
  // With the system user, whose notice it is (as Rails leaves it).
  expect(dmUsers(id)).toEqual([me.name, other.name, third.name, "system"].sort());
  expect(following(id, third)).toEqual([["t", "2"]]);

  // A group still takes members after its messages.
  await drawer.locator(".c-navbar__channel-title").click();
  await drawer.locator(".c-channel-info__nav a", { hasText: "Members" }).click();
  await expect(members.locator(".c-channel-members__list-item.-member")).toHaveCount(3);
  await expect(add).toBeVisible();
});
