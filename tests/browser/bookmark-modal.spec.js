// BookmarkModal (bookmark-modal.js, components/modal/bookmark.gjs) and its
// DTimeShortcutPicker, editing a post's bookmark from the topic footer's
// bookmarks menu and from the member's bookmarks tab: the name, what
// happens after the reminder, the reminder's tiles, its custom date and
// time, its relative time, removing it, saving on a click outside, and
// deleting. Each is checked in the database.
const { test, expect, login, api, topic, sql, label, member } = require("./helpers");

const modal = (page) => page.locator("#bookmark-reminder-modal");

// The bookmark's name, auto delete preference, and its reminder as
// minutes from now (rounded), "" without one.
function saved(id) {
  return sql(
    "SELECT COALESCE(name, ''), auto_delete_preference, \
       COALESCE(round(extract(epoch FROM reminder_at - now()) / 60)::text, 'none') FROM bookmarks WHERE id = :'p1'",
    id,
  )[0];
}

// A member in UTC with a bookmark on a reply, reading the topic.
async function bookmarked(page, context, session) {
  const user1 = await session("user1");
  const t = await topic(user1, label("Bookmark modal"), label("A topic for the bookmark modal"), [
    { context: await session("user0"), raw: label("The reply the member bookmarks") },
  ]);
  const m = member();
  sql("UPDATE user_options SET timezone = 'UTC' WHERE user_id = :'p1'", m.id);
  await login(context, m.name);
  const created = await api(context, "POST", "/bookmarks.json", {
    bookmarkable_id: String(t.posts[1]),
    bookmarkable_type: "Post",
    name: "Read later",
  });
  await page.goto(t.url);
  return { t, m, id: created.id };
}

// The modal for the post's bookmark, from the footer's menu.
async function edit(page) {
  await page.locator("#topic-footer-buttons .bookmark-menu__trigger").click();
  await page.locator('.bookmark-menu__row[data-menu-option-id="post-2"] .bookmark-menu__row-btn').click();
  await expect(async () => {
    await page.locator('.bookmark-menu__row[data-menu-option-id="edit"] .bookmark-menu__row-btn').click({ timeout: 2000 });
    await expect(page.locator(".topic-bookmarks-menu-content")).toHaveCount(0, { timeout: 2000 });
  }).toPass();
  await expect(modal(page)).toBeVisible();
}

test("a tile sets the reminder and saves at once", async ({ page, context, session }) => {
  const { t, id } = await bookmarked(page, context, session);
  // The page asked for again after a save (the footer and the menu drawn
  // again) answers late here: the menu opened before it lands must hold
  // what was saved already.
  await page.route(new RegExp(`${t.url}$`), async (route) => {
    await new Promise((resolve) => setTimeout(resolve, 1500));
    await route.continue();
  });
  await edit(page);
  await expect(modal(page)).toHaveAttribute("data-bookmark-id", String(id));
  await expect(modal(page).locator("#discourse-modal-title")).toHaveText("Edit bookmark");
  await expect(modal(page).locator("#bookmark-name")).toHaveValue("Read later");
  await expect(modal(page).locator("#bookmark-name")).toBeFocused();
  await expect(modal(page).locator("#bookmark-auto-delete-preference .selected-name")).toHaveText(
    "Keep bookmark and clear reminder",
  );
  // Without a reminder there is none to remove, and no custom inputs yet.
  await expect(modal(page).locator("#tap_tile_none")).toHaveCount(0);
  await expect(modal(page).locator(".custom-date-time-wrap")).toHaveCount(0);
  await expect(modal(page).locator("#tap_tile_tomorrow .tap-tile-title")).toHaveText("Tomorrow");
  await expect(modal(page).locator("#tap_tile_tomorrow .tap-tile-date")).toHaveText(/^[A-Z][a-z]{2}, 8:00 am$/);

  await modal(page).locator("#bookmark-name").fill("Read tomorrow");
  await modal(page).locator("#tap_tile_tomorrow").click();
  await expect(modal(page)).toHaveCount(0);
  const [name, preference, minutes] = saved(id);
  expect([name, preference]).toEqual(["Read tomorrow", "3"]);
  // Tomorrow at 8 in UTC: between 8 and 32 hours away.
  expect(Number(minutes)).toBeGreaterThan(8 * 60 - 2);
  expect(Number(minutes)).toBeLessThan(32 * 60 + 2);
  // The post's button is drawn again, with the reminder.
  await expect(page.locator(`article[data-post-id="${t.posts[1]}"] .post-action-menu__bookmark.with-reminder`)).toBeVisible();

  // Opened again: the reminder there is, as the custom time, and a tile
  // to remove it; removing it saves.
  await edit(page);
  await expect(modal(page).locator(".existing-reminder-at-alert")).toContainText("You have a reminder set for this bookmark");
  await expect(modal(page).locator("#tap_tile_custom")).toHaveClass(/active/);
  await expect(modal(page).locator("#custom-time")).toHaveValue("08:00");
  await expect(modal(page).locator("#tap_tile_none .tap-tile-title")).toHaveText("Remove reminder and keep bookmark");
  await modal(page).locator("#tap_tile_none").click();
  await expect(modal(page)).toHaveCount(0);
  expect(saved(id)).toEqual(["Read tomorrow", "3", "none"]);
});

test("the custom date and time and the relative time wait for Save", async ({ page, context, session }) => {
  const { id } = await bookmarked(page, context, session);
  // The date is typed here, so the date picker is kept from loading: the
  // field takes a typed date with or without it.
  await page.route(/\/assets\/pikaday[^/]*\.js/, (route) => route.abort());
  await edit(page);
  await modal(page).locator("#tap_tile_custom").click();
  // The custom tile fills in an hour from now and does not save.
  await expect(modal(page).locator(".custom-date-time-wrap.custom-input-wrap")).toBeVisible();
  await expect(modal(page).locator("#custom-date")).toHaveValue(/^\d{4}-\d{2}-\d{2}$/);
  await expect(modal(page).locator("#custom-time")).toHaveValue(/^\d{2}:\d{2}$/);
  await expect(modal(page)).toBeVisible();
  expect(saved(id)[2]).toBe("none");

  // A date two days on at 09:30, typed as the date input takes it.
  const [[date]] = sql("SELECT to_char(now() AT TIME ZONE 'UTC' + interval '2 days', 'YYYY-MM-DD')");
  await modal(page).locator("#custom-date").evaluate((input, value) => {
    input.value = value;
    input.dispatchEvent(new Event("change", { bubbles: true }));
  }, date);
  await modal(page).locator("#custom-time").fill("09:30");
  await modal(page).locator("#save-bookmark").click();
  await expect(modal(page)).toHaveCount(0);
  const [[at]] = sql("SELECT to_char(reminder_at AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI') FROM bookmarks WHERE id = :'p1'", id);
  expect(at).toBe(`${date} 09:30`);

  // The relative time: three days from now.
  await edit(page);
  await modal(page).locator("#bookmark-relative-time-picker").fill("3");
  await modal(page).locator("#bookmark-relative-time-picker").dispatchEvent("change");
  await expect(modal(page).locator(".relative-time-picker .selected-name")).toHaveText("days");
  await modal(page).locator("#save-bookmark").click();
  await expect(modal(page)).toHaveCount(0);
  const minutes = Number(saved(id)[2]);
  expect(Math.abs(minutes - 3 * 24 * 60)).toBeLessThan(3);
});

test("the auto delete choice, a click outside saving, Cancel not, and delete", async ({ page, context, session }) => {
  const { t, id } = await bookmarked(page, context, session);
  await edit(page);
  await modal(page).locator("#bookmark-auto-delete-preference summary").click();
  await modal(page).locator('#bookmark-auto-delete-preference .select-kit-row[data-value="1"]').click();
  await expect(modal(page).locator("#bookmark-auto-delete-preference .selected-name")).toHaveText("Delete bookmark");
  await modal(page).locator("#bookmark-name").fill("Kept by a click outside");
  await page.locator(".d-modal__backdrop").click({ position: { x: 5, y: 5 } });
  await expect(modal(page)).toHaveCount(0);
  expect(saved(id)).toEqual(["Kept by a click outside", "1", "none"]);

  await edit(page);
  await modal(page).locator("#bookmark-name").fill("Dropped by Cancel");
  await modal(page).locator(".d-modal-cancel").click();
  await expect(modal(page)).toHaveCount(0);
  expect(saved(id)[0]).toBe("Kept by a click outside");

  // No reminder: the trash deletes without asking.
  await edit(page);
  await modal(page).locator("#delete-bookmark").click();
  await expect(modal(page)).toHaveCount(0);
  await expect.poll(() => sql("SELECT count(*) FROM bookmarks WHERE id = :'p1'", id)).toEqual([["0"]]);
  await expect(page.locator(`article[data-post-id="${t.posts[1]}"] .post-action-menu__bookmark.bookmarked`)).toHaveCount(0);
});

test("the bookmarks tab edits a bookmark, and deleting one with a reminder asks first", async ({ page, context, session }) => {
  const { m, id } = await bookmarked(page, context, session);
  sql("UPDATE bookmarks SET reminder_at = now() + interval '2 days' WHERE id = :'p1'", id);
  await page.goto(`/u/${m.name}/activity/bookmarks`);
  const item = page.locator(`tr.bookmark-list-item[data-bookmark-id="${id}"]`);
  await item.locator(".bookmark-actions-dropdown summary").click();
  await item.locator('.bookmark-actions-dropdown .select-kit-row[data-value="edit"]').click();
  await expect(modal(page)).toBeVisible();
  await modal(page).locator("#bookmark-name").fill("From the tab");
  await modal(page).locator("#save-bookmark").click();
  await expect(modal(page)).toHaveCount(0);
  // The name changed, the reminder kept.
  const [name, , minutes] = saved(id);
  expect(name).toBe("From the tab");
  expect(Math.abs(Number(minutes) - 2 * 24 * 60)).toBeLessThan(3);
  await expect(item.locator(".bookmark-metadata")).toContainText("From the tab");

  await item.locator(".bookmark-actions-dropdown summary").click();
  await item.locator('.bookmark-actions-dropdown .select-kit-row[data-value="edit"]').click();
  await modal(page).locator("#delete-bookmark").click();
  await expect(page.locator(".dialog-body")).toContainText("Are you sure you want to delete this bookmark?");
  await page.locator(".dialog-footer .btn-danger").click();
  await expect(modal(page)).toHaveCount(0);
  await expect.poll(() => sql("SELECT count(*) FROM bookmarks WHERE id = :'p1'", id)).toEqual([["0"]]);
  await expect(item).toHaveCount(0);
});
