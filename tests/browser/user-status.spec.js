// User status (user-status.js, crate::users::views::status): the user
// menu's Set custom status item opening UserStatusModal, its emoji
// picker, description and clear-after tiles, saving and deleting the
// status (the row, the header bubble); the emoji drawn by a poster's
// name with its tooltip, on a mention and on the user card; the account
// preferences' status saved with the tab; a status change redrawing a
// watched topic's posts live. enable_user_status is on for the suite
// (setup.sql); each test works on a member of its own (cloned from
// user2), so no other spec meets a status.
const { test, expect, login, api, topic, sql, label } = require("./helpers");

// A member of the test's own, as user2 is (password, groups, options).
function member() {
  const name = "st" + Math.floor(Math.random() * 1e9).toString(36);
  const [[id]] = sql(
    "INSERT INTO users SELECT (jsonb_populate_record(NULL::users, to_jsonb(u) || jsonb_build_object( \
       'id', nextval('users_id_seq'), 'username', :'p1', 'username_lower', lower(:'p1'), 'name', :'p1', \
       'secure_identifier', NULL))).* FROM users u WHERE username = 'user2' RETURNING id",
    name,
  );
  const copy = (table, extra, where) =>
    sql(
      `INSERT INTO ${table} SELECT (jsonb_populate_record(NULL::${table}, to_jsonb(t) || jsonb_build_object(${extra}))).* \
       FROM ${table} t WHERE user_id = (SELECT id FROM users WHERE username = 'user2')${where || ""}`,
      name,
      id,
    );
  copy(
    "user_emails",
    "'id', nextval('user_emails_id_seq'), 'user_id', :'p2'::int, 'email', :'p1' || '@example.com', 'normalized_email', :'p1' || '@example.com'",
    ' AND "primary"',
  );
  copy("user_passwords", "'id', nextval('user_passwords_id_seq'), 'user_id', :'p2'::int");
  copy("user_options", "'user_id', :'p2'::int");
  copy("user_stats", "'user_id', :'p2'::int");
  copy("user_profiles", "'user_id', :'p2'::int");
  copy("group_users", "'id', nextval('group_users_id_seq'), 'user_id', :'p2'::int");
  return { name, id };
}

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
  await api(poster, "PUT", "/user-status.json", { description: "Back at work", emoji: "computer" });
  await expect(message.locator("img.emoji")).toHaveAttribute("alt", "computer");
  await api(poster, "DELETE", "/user-status.json");
  await expect(first.locator(".user-status-message-wrap")).toHaveCount(0);
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
