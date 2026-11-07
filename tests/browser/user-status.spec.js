// User status (user-status.js, crate::users::views::status): the user
// menu's Set custom status item opening UserStatusModal, its emoji
// picker, description and clear-after tiles, saving and deleting the
// status (the row, the header bubble); the emoji drawn by a poster's
// name with its tooltip, on a mention and on the user card; the account
// preferences' status saved with the tab; a status change redrawing a
// watched topic's posts (by the user or mentioning them) live; chat's
// statuses (the author, mentions, a DM's title and sidebar link, the
// members list, the composer's autocomplete). enable_user_status is on
// for the suite (setup.sql); each test works on a member of its own
// (cloned from user2), so no other spec meets a status.
const { test, expect, login, api, topic, sql, label, member } = require("./helpers");

function status(id) {
  return sql("SELECT emoji, description, ends_at IS NOT NULL FROM user_statuses WHERE user_id = :'p1'", id);
}

test("the user menu sets a status in the modal and deletes it", async ({ page, context }) => {
  const m = member();
  await login(context, m.name);
  await page.goto("/latest");

  await page.locator("#toggle-current-user").click();
  await page.locator("#user-menu-button-profile").click();
  const item = page.locator(".set-user-status button");
  await expect(item).toHaveText("Set custom status");
  await item.click();

  const modal = page.locator(".d-modal.user-status");
  await expect(modal.locator(".d-modal__title-text")).toHaveText("Set custom status");
  const save = modal.locator(".d-modal__footer .btn-primary");
  await expect(save).toBeDisabled();
  await expect(modal.locator(".delete-status")).toHaveCount(0);
  await expect(modal.locator(".pause-notifications input")).not.toBeChecked();
  const tiles = modal.locator(".tap-tile .tap-tile-title");
  await expect(tiles).toHaveText(["In one hour", "In two hours", "Tomorrow", "Custom date and time", "Never"]);

  // updateDescription: typing picks speech_balloon until an emoji is.
  const description = label("Gone fishing");
  await modal.locator(".user-status-description").fill(description);
  await expect(modal.locator(".emoji-picker-trigger img")).toHaveAttribute("alt", "speech_balloon");
  await expect(save).toBeEnabled();
  await modal.locator(".emoji-picker-trigger").click();
  const picker = page.locator(".emoji-picker");
  await picker.locator(".filter-input").fill("fishing_pole");
  await picker.locator('.emoji-picker__section.filtered img.emoji[data-emoji="fishing_pole"]').click();
  await expect(modal.locator(".emoji-picker-trigger img")).toHaveAttribute("alt", "fishing_pole");

  await modal.locator("#tap_tile_one_hour").click();
  await expect(modal.locator("#tap_tile_one_hour")).toHaveClass(/active/);
  await save.click();
  await expect(modal).toHaveCount(0);
  await expect.poll(() => status(m.id)).toEqual([["fishing_pole", description, "t"]]);
  const [[minutes]] = sql(
    "SELECT round(extract(epoch FROM ends_at - now()) / 60) FROM user_statuses WHERE user_id = :'p1'",
    m.id,
  );
  expect(Math.abs(Number(minutes) - 60)).toBeLessThanOrEqual(2);
  const bubble = page.locator("#toggle-current-user .user-status-background img");
  await expect(bubble).toHaveAttribute("alt", ":fishing_pole:");

  // The item shows it now; the modal opens on it with the delete button.
  await page.goto("/latest");
  await expect(bubble).toHaveAttribute("title", new RegExp(`^${description}\\nUntil: `));
  await page.locator("#toggle-current-user").click();
  await page.locator("#user-menu-button-profile").click();
  await expect(page.locator(".set-user-status .item-label")).toContainText(description);
  await page.locator(".set-user-status button").click();
  await expect(modal.locator(".user-status-description")).toHaveValue(description);
  await expect(modal.locator("#tap_tile_custom")).toHaveClass(/active/);
  await modal.locator(".delete-status").click();
  await expect(modal).toHaveCount(0);
  await expect.poll(() => status(m.id)).toEqual([]);
  await expect(page.locator("#toggle-current-user .user-status-background")).toHaveCount(0);
});

test("the custom date tile is refused by name", async ({ page, context }) => {
  const m = member();
  await login(context, m.name);
  await page.goto("/latest");
  await page.locator("#toggle-current-user").click();
  await page.locator("#user-menu-button-profile").click();
  await page.locator(".set-user-status button").click();
  await page.locator(".d-modal.user-status #tap_tile_custom").click();
  await expect(page.getByText("Not ported yet: the custom date and time of a status's end.")).toBeVisible();
});

test("a status shows by the poster's name, on mentions and on the user card, and live", async ({
  page,
  context,
  session,
}) => {
  const m = member();
  const poster = await session(m.name);
  await api(poster, "PUT", "/user-status.json", { description: "At the dentist", emoji: "tooth" });
  const user1 = await session("user1");
  const t = await topic(poster, label("A topic with a status"), label("A first post by a member with a status"), [
    { context: user1, raw: label(`A reply mentioning @${m.name} for the status`) },
  ]);

  await login(context, "user0");
  await page.goto(t.url);
  const first = page.locator(`article[data-post-id="${t.posts[0]}"]`);
  const message = first.locator(".topic-meta-data .names .user-status-message-wrap .user-status-message");
  await expect(message.locator("img.emoji")).toHaveAttribute("alt", "tooth");
  await message.hover();
  const tooltip = page.locator('[data-identifier="user-status-message-tooltip"][data-content]');
  await expect(tooltip.locator(".user-status-tooltip-description")).toHaveText("At the dentist");
  await expect(tooltip.locator(".user-status-tooltip-until")).toHaveCount(0);

  const mention = page.locator(`article[data-post-id="${t.posts[1]}"] .cooked a.mention`);
  await expect(mention.locator(".user-status-message-wrapper img.emoji")).toHaveAttribute("alt", "tooth");

  await first.locator(".topic-avatar [data-user-card]").first().click();
  const card = page.locator("#user-card .user-status");
  await expect(card.locator(".user-status__description")).toHaveText("At the dentist");
  await expect(card.locator("img.emoji")).toHaveAttribute("alt", "tooth");
  await page.keyboard.press("Escape");

  // UserStatusManager: the poster's new status redraws their posts.
  // The mentions decorator tracks the mentioned user: their mention too.
  await api(poster, "PUT", "/user-status.json", { description: "Back at work", emoji: "computer" });
  await expect(message.locator("img.emoji")).toHaveAttribute("alt", "computer");
  await expect(mention.locator(".user-status-message-wrapper img.emoji")).toHaveAttribute("alt", "computer");
  await api(poster, "DELETE", "/user-status.json");
  await expect(first.locator(".user-status-message-wrap")).toHaveCount(0);
  await expect(mention.locator(".user-status-message-wrapper")).toHaveCount(0);
});

test("the account preferences save the status with the tab", async ({ page, context }) => {
  const m = member();
  await login(context, m.name);
  await page.goto(`/u/${m.name}/preferences/account`);
  const row = page.locator(".pref-user-status");
  await expect(row.locator(".control-label")).toHaveText("Custom Status");
  await expect(row.locator(".static")).toHaveText("Not set");
  await row.locator(".controls > .btn").click();

  const modal = page.locator(".d-modal.user-status");
  await expect(modal.locator(".pause-notifications")).toHaveCount(0);
  await modal.locator(".user-status-description").fill("Reading");
  await modal.locator("#tap_tile_none").click();
  await modal.locator(".d-modal__footer .btn-primary").click();
  await expect(row.locator(".user-status-message .user-status-message-description")).toHaveText("Reading");
  expect(status(m.id)).toEqual([]);

  await page.locator(".save-changes").click();
  await expect(page.locator(".save-button .saved")).toBeVisible();
  expect(status(m.id)).toEqual([["speech_balloon", "Reading", "f"]]);
});

test("chat draws a status by the author, on mentions, in the DM's title, the sidebar and the members", async ({
  page,
  context,
  session,
}) => {
  const m = member();
  const them = await session(m.name);
  await api(them, "PUT", "/user-status.json", { description: "At the dentist", emoji: "tooth" });
  const user0 = await session("user0");
  const dm = await api(user0, "POST", "/chat/api/direct-message-channels.json", { "target_usernames[]": m.name });
  const id = dm.channel.id;
  await api(them, "POST", `/chat/${id}.json`, { message: label("a message from the dentist") });
  await api(user0, "POST", `/chat/${id}.json`, { message: label(`hello @${m.name} there`) });

  await login(context, "user0");
  await page.goto(`/chat/c/-/${id}`);
  // ChatMessageInfo: the emoji by the author's name, its tooltip on hover.
  const info = page.locator(".chat-message-info__status .user-status-message").first();
  await expect(info.locator("img.emoji")).toHaveAttribute("alt", "tooth");
  await info.hover();
  const tooltip = page.locator('[data-identifier="user-status-message-tooltip"][data-content]');
  await expect(tooltip.locator(".user-status-tooltip-description")).toHaveText("At the dentist");
  // updateUserStatusOnMention: a UserStatusMessage in the mention.
  const mention = page.locator(`.chat-cooked a.mention[href="/u/${m.name.toLowerCase()}"]`);
  await expect(mention.locator(".user-status-message img.emoji")).toHaveAttribute("alt", "tooth");
  // ChatChannelName's status for a DM with one other user, and the
  // sidebar link's (ChatSidebarIndicators).
  await expect(
    page.locator(".chat-channel-title .chat-channel-name__label .chat-channel__user-status-message img.emoji"),
  ).toHaveAttribute("alt", "tooth");
  await expect(
    page.locator(`.sidebar-section[data-section-name="chat-dms"] .channel-${id} .user-status-message img.emoji`),
  ).toHaveAttribute("alt", "tooth");

  // ChatUserInfo in the members list, with its description.
  await page.goto(`/chat/c/-/${id}/info/members`);
  await expect(
    page.locator(".c-channel-members__list-item .user-status .user-status-message-description"),
  ).toHaveText("At the dentist");
});

test("the chat composer's mention autocomplete shows a status", async ({ page, context }) => {
  const m = member();
  sql(
    "INSERT INTO user_statuses (user_id, emoji, description, set_at) VALUES (:'p1'::int, 'tooth', 'At the dentist', now())",
    m.id,
  );
  await login(context, "user0");
  await page.goto("/chat/c/general/2");
  const composer = page.locator(".full-page-chat .chat-composer__input");
  await composer.pressSequentially(`@${m.name}`, { delay: 20 });
  // initUserStatusHtml and renderUserStatusHtml: the user's item carries
  // its index and the status with its description.
  const item = page.locator(".autocomplete.ac-user li[data-index]");
  await expect(item.locator(".user-status-message img.emoji")).toHaveAttribute("alt", "tooth");
  await expect(item.locator(".user-status-message .user-status-message-description")).toHaveText("At the dentist");
  await composer.press("Escape");
});
