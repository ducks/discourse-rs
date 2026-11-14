// Pause notifications (user-menu.js, crate::users::views::menu): the user
// menu's profile tab item opening DoNotDisturbModal, its DTapTiles saving
// a duration (POST /do-not-disturb.json: the timing row, the header's
// moon over the live stream, the item on with its end), the item then
// leaving do not disturb (DELETE /do-not-disturb.json), the modal's
// error and its notification schedule button. Each test works on a
// member of its own (cloned from user2).
const { test, expect, login, sql, member } = require("./helpers");

function timings(id) {
  return sql(
    "SELECT round(extract(epoch FROM ends_at - starts_at) / 60), scheduled FROM do_not_disturb_timings WHERE user_id = :'p1'",
    id,
  );
}

// The profile tab's item, once the menu and the tab are drawn and at rest.
async function pauseItem(page) {
  await page.locator("#toggle-current-user").click();
  await page.locator("#user-menu-button-profile").click();
  const item = page.locator(".user-menu #quick-access-profile li.do-not-disturb");
  await expect(item).toBeVisible();
  await page.locator(".user-menu").evaluate((el) => Promise.all(el.getAnimations({ subtree: true }).map((a) => a.finished)));
  return item;
}

// The modal the item opens, its entering animation over.
async function openModal(page) {
  const item = await pauseItem(page);
  await item.locator("button").click();
  const modal = page.locator(".d-modal.do-not-disturb-modal");
  await expect(modal).toBeVisible();
  await expect(modal).not.toHaveClass(/is-animating/);
  return modal;
}

// The requests the page makes to the endpoint.
function requests(page) {
  const seen = [];
  page.on("request", (r) => {
    if (r.url().includes("/do-not-disturb")) {
      seen.push(`${r.method()} ${new URL(r.url()).pathname} ${r.postData() || ""}`.trim());
    }
  });
  return seen;
}

test("the user menu pauses notifications in the modal and resumes them", async ({ page, context }) => {
  const m = member();
  await login(context, m.name);
  await page.goto("/latest");
  const seen = requests(page);
  const moon = page.locator("#toggle-current-user .do-not-disturb-background");
  await expect(moon).toHaveCount(0);

  const item = await pauseItem(page);
  await expect(item).not.toHaveClass(/enabled/);
  await expect(item.locator(".item-label")).toHaveText("Pause notifications");
  await expect(item.locator("svg.d-icon-toggle-off")).toHaveCount(1);
  await item.locator("button").click();
  // closeUserMenu, then the modal.
  await expect(page.locator(".user-menu")).toHaveCount(0);
  const modal = page.locator(".d-modal.do-not-disturb-modal");
  await expect(modal.locator(".d-modal__title-text")).toHaveText("Pause notifications for…");
  const tiles = modal.locator(".tap-tile-grid > .do-not-disturb-tile.tap-tile");
  await expect(tiles).toHaveText(["30 minutes", "1 hour", "2 hours", "Until tomorrow"]);
  expect(await tiles.evaluateAll((els) => els.map((el) => [el.id, el.getAttribute("role"), el.tabIndex, el.getAttribute("aria-pressed")]))).toEqual([
    ["tap_tile_30", "button", 0, "false"],
    ["tap_tile_60", "button", 0, "false"],
    ["tap_tile_120", "button", 0, "false"],
    ["tap_tile_tomorrow", "button", 0, "false"],
  ]);
  await expect(modal.locator(".d-modal__body > button.btn .d-button-label")).toHaveText("Set a notification schedule");
  await expect(modal.locator(".d-modal__footer")).toHaveCount(0);
  await expect(modal).not.toHaveClass(/is-animating/);

  await modal.locator("#tap_tile_30").click();
  await expect(modal).toHaveCount(0);
  expect(seen).toEqual(["POST /do-not-disturb.json duration=30"]);
  expect(timings(m.id)).toEqual([["30", "f"]]);
  // updateDoNotDisturbStatus: the header's moon.
  await expect(moon).toHaveAttribute("title", "Notifications paused");
  await expect(moon.locator("svg.d-icon-discourse-dnd")).toHaveCount(1);

  // The item is on, with its end.
  const on = await pauseItem(page);
  await expect(on).toHaveClass(/enabled/);
  await expect(on.locator("svg.d-icon-toggle-on")).toHaveCount(1);
  await expect(on.locator(".item-label > span").first()).toHaveText("Pause notifications");
  const end = on.locator(".item-label .relative-date");
  const [[endsAt]] = sql("SELECT round(extract(epoch FROM ends_at) * 1000) FROM do_not_disturb_timings WHERE user_id = :'p1'", m.id);
  expect(Math.abs(Number(await end.getAttribute("data-time")) - Number(endsAt))).toBeLessThanOrEqual(1);
  await expect(end).toHaveText(/^\d+m$/);

  // doNotDisturbClick while paused: leaveDoNotDisturb, no modal.
  await on.locator("button").click();
  await expect(page.locator(".user-menu li.do-not-disturb")).not.toHaveClass(/enabled/);
  expect(seen.slice(1)).toEqual(["DELETE /do-not-disturb.json"]);
  expect(timings(m.id)).toEqual([]);
  await expect(modal).toHaveCount(0);
  await expect(page.locator(".user-menu li.do-not-disturb .relative-date")).toHaveCount(0);
  await expect(moon).toHaveCount(0);
});

test("Escape closes the modal without a request, Enter on a tile saves it", async ({ page, context }) => {
  const m = member();
  await login(context, m.name);
  await page.goto("/latest");
  const seen = requests(page);

  let modal = await openModal(page);
  await page.keyboard.press("Escape");
  await expect(modal).toHaveCount(0);
  expect(seen).toEqual([]);
  expect(timings(m.id)).toEqual([]);
  await expect(page.locator("#toggle-current-user .do-not-disturb-background")).toHaveCount(0);

  // DTapTile's keyDown.
  modal = await openModal(page);
  await modal.locator("#tap_tile_tomorrow").focus();
  await page.keyboard.press("Enter");
  await expect(modal).toHaveCount(0);
  expect(seen).toEqual(["POST /do-not-disturb.json duration=tomorrow"]);
  // Time.now.end_of_day
  expect(
    sql("SELECT to_char(ends_at, 'HH24:MI:SS.US'), ends_at::date = starts_at::date FROM do_not_disturb_timings WHERE user_id = :'p1'", m.id),
  ).toEqual([["23:59:59.999999", "t"]]);
  await expect(page.locator("#toggle-current-user .do-not-disturb-background")).toHaveCount(1);
});

test("a refused duration shows in the modal, and the schedule button opens the preferences", async ({ page, context }) => {
  const m = member();
  await login(context, m.name);
  await page.goto("/latest");
  await page.route("**/do-not-disturb.json", (route) =>
    route.fulfill({ status: 422, contentType: "application/json", body: JSON.stringify({ errors: ["Ends at is invalid"] }) }),
  );
  const modal = await openModal(page);
  await modal.locator("#tap_tile_60").click();
  const flash = modal.locator(".d-modal__container > #modal-alert.alert");
  await expect(flash).toHaveText("Ends at is invalid");
  await expect(flash).toHaveAttribute("role", "alert");
  await expect(modal.locator("#tap_tile_60")).toBeVisible();
  expect(timings(m.id)).toEqual([]);

  // navigateToNotificationSchedule
  await modal.locator(".d-modal__body > button.btn").click();
  await expect(page).toHaveURL(new RegExp(`/u/${m.name}/preferences/notifications$`));
  await expect(page.locator(".d-modal")).toHaveCount(0);
});
