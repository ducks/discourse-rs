// EditTopicTimer (topic-admin-modals.js), opened from the topic admin
// menu's Set Topic Timer: the types the topic's state offers, a time
// tile, the duration types' relative picker, publishing to a category,
// the flashes, the TopicTimerInfo preview, setting and removing. Each
// change is checked in topic_timers, the page drawn again.
const { test, expect, login, api, topic, sql, label, topicAdminMenu } = require("./helpers");

const MENU = ".topic-admin-menu-content";
const MODAL = ".d-modal.edit-topic-timer-modal";

async function setup(session, context) {
  const user1 = await session("user1");
  const t = await topic(user1, label("Topic timer modal"), label("A topic for the topic timer modal spec"));
  await login(context, "admin");
  return t;
}

// After a reload the browser restores the scroll and the suggested
// topics arrive, both after the load event. A menu opened before that
// moves under the pointer, and the click on its item lands nowhere.
async function settled(page) {
  let last;
  await expect
    .poll(
      async () => {
        const now = await page.evaluate(() => window.scrollY + ":" + document.documentElement.scrollHeight);
        const same = now === last;
        last = now;
        return same;
      },
      { intervals: [250] },
    )
    .toBe(true);
}

async function openModal(page) {
  await topicAdminMenu(page);
  await page.locator(`${MENU} .admin-topic-timer-update .btn`).click();
  const modal = page.locator(MODAL);
  await expect(modal.locator(".d-modal__title-text")).toHaveText("Topic Timer");
  return modal;
}

function types(modal) {
  return modal.locator("select.timer-type option").evaluateAll((options) => options.map((o) => [o.value, o.textContent]));
}

// The topic's live timers.
function timers(id, columns) {
  return sql(
    `SELECT ${columns} FROM topic_timers WHERE timerable_id = :'p1' AND type = 'TopicTimer' AND deleted_at IS NULL`,
    id,
  );
}

test("an open topic's types, the required time frame, a tile sets the timer and Remove Timer removes it", async ({
  page,
  context,
  session,
}) => {
  const t = await setup(session, context);
  await page.goto(t.url);
  const modal = await openModal(page);
  await expect(modal).toHaveAttribute("id", "topic-timer-modal");
  expect(await types(modal)).toEqual([
    ["__NONE__", "None"],
    ["close", "Auto-Close Topic"],
    ["close_after_last_post", "Auto-Close Topic After Last Post"],
    ["delete", "Auto-Delete Topic"],
    ["delete_after_last_post", "Auto-Delete Topic After Last Post"],
    ["bump", "Auto-Bump Topic"],
    ["delete_replies", "Auto-Delete Replies"],
    ["open", "Close Temporarily"],
  ]);
  await expect(modal.locator("select.timer-type")).toHaveValue("close");

  // timeOptions less the hidden ones: no Later Today, no None.
  const tiles = await modal.locator(".tap-tile-grid .tap-tile").evaluateAll((els) => els.map((el) => el.dataset.tile));
  expect(tiles).toEqual(expect.arrayContaining(["tomorrow", "start_of_next_business_week", "two_weeks", "next_month", "six_months", "custom"]));
  expect(tiles).not.toContain("none");
  expect(tiles).not.toContain("later_today");
  expect(tiles).not.toContain("later_this_week");
  await expect(modal.locator("#tap_tile_two_weeks .tap-tile-title")).toHaveText("Two weeks");
  await expect(modal.locator("#tap_tile_six_months .tap-tile-title")).toHaveText("Six months");
  await expect(modal.locator(".d-modal__footer .btn-danger")).toHaveCount(0);
  await expect(modal.locator(".modal-topic-timer-info")).toHaveCount(0);

  // Nothing chosen.
  await modal.locator(".d-modal__footer .btn-primary").click();
  await expect(modal.locator("#modal-alert")).toHaveText("Please select a time frame");
  expect(timers(t.id, "id")).toEqual([]);

  await modal.locator("#tap_tile_tomorrow").click();
  await expect(modal.locator("#tap_tile_tomorrow")).toHaveClass(/active/);
  await expect(modal.locator(".modal-topic-timer-info .topic-timer-info .topic-timer-heading")).toContainText(
    "This topic will automatically close in",
  );
  let reloaded = page.waitForEvent("load");
  await modal.locator(".d-modal__footer .btn-primary").click();
  await expect(modal).toHaveCount(0);
  await expect
    .poll(() =>
      timers(
        t.id,
        "status_type, based_on_last_post, execute_at > now(), execute_at < now() + interval '2 days', public_type",
      ),
    )
    .toEqual([["1", "f", "t", "t", "t"]]);

  // The timer there: its time in the custom fields, and Remove Timer.
  await reloaded;
  await settled(page);
  const again = await openModal(page);
  await expect(again.locator("select.timer-type")).toHaveValue("close");
  await expect(again.locator("#tap_tile_custom")).toHaveClass(/active/);
  await expect(again.locator("#custom-date")).toHaveValue(/^\d{4}-\d{2}-\d{2}$/);
  await expect(again.locator("#custom-time")).toHaveValue("08:00");
  const remove = again.locator(".d-modal__footer .btn-danger");
  await expect(remove).toHaveText("Remove Timer");
  reloaded = page.waitForEvent("load");
  await remove.click();
  await expect(again).toHaveCount(0);
  await expect.poll(() => timers(t.id, "id")).toEqual([]);
  expect(sql("SELECT COUNT(*) FROM topic_timers WHERE timerable_id = :'p1' AND deleted_at IS NOT NULL", t.id)).toEqual([
    ["1"],
  ]);
  await reloaded;
  await settled(page);
  await expect((await openModal(page)).locator(".d-modal__footer .btn-danger")).toHaveCount(0);
});

test("closing after the last post takes a duration", async ({ page, context, session }) => {
  const t = await setup(session, context);
  // The last post three hours ago.
  sql("UPDATE topics SET last_posted_at = now() - interval '3 hours' WHERE id = :'p1'", t.id);
  await page.goto(t.url);
  const modal = await openModal(page);
  await modal.locator("select.timer-type").selectOption("close_after_last_post");
  await expect(modal.locator(".tap-tile-grid")).toHaveCount(0);
  await expect(modal.locator(".control-group .control-label", { hasText: "Duration" })).toBeVisible();
  const duration = modal.locator(".relative-time-picker .relative-time-duration");
  const intervals = modal.locator(".relative-time-picker .relative-time-intervals");
  await expect(intervals.locator("summary")).toHaveAttribute("data-value", "days");

  // Two hours after a post three hours old: at once.
  await intervals.locator("summary").click();
  await intervals.locator('.select-kit-body [data-value="hours"]').click();
  await duration.fill("2");
  await duration.dispatchEvent("change");
  await expect(modal.locator("form > .warning")).toHaveText(
    "The last post in the topic is already 3 hours old, so the topic will be closed immediately.",
  );
  await expect(modal.locator(".modal-topic-timer-info")).toHaveCount(0);

  await duration.fill("5");
  await duration.dispatchEvent("change");
  await expect(modal.locator("form > .warning")).toHaveCount(0);
  await expect(modal.locator(".modal-topic-timer-info .topic-timer-heading")).toHaveText(
    "This topic will close 5 hours after the last reply.",
  );
  await modal.locator(".d-modal__footer .btn-primary").click();
  await expect(modal).toHaveCount(0);
  await expect
    .poll(() => timers(t.id, "status_type, based_on_last_post, duration_minutes, execute_at > now()"))
    .toEqual([["1", "t", "300", "t"]]);
});

test("a closed topic's types, and deleting the replies after days", async ({ page, context, session }) => {
  const t = await setup(session, context);
  await api(context, "PUT", `/t/${t.id}/status`, { status: "closed", enabled: "true" });
  await page.goto(t.url);
  const modal = await openModal(page);
  expect((await types(modal)).slice(1)).toEqual([
    ["open", "Auto-Open Topic"],
    ["delete", "Auto-Delete Topic"],
    ["delete_after_last_post", "Auto-Delete Topic After Last Post"],
    ["bump", "Auto-Bump Topic"],
    ["delete_replies", "Auto-Delete Replies"],
    ["close", "Open Temporarily"],
  ]);
  await expect(modal.locator("select.timer-type")).toHaveValue("open");
  await modal.locator("select.timer-type").selectOption("delete_replies");
  const duration = modal.locator(".relative-time-picker .relative-time-duration");
  await duration.fill("2");
  await duration.dispatchEvent("change");
  await expect(modal.locator(".modal-topic-timer-info .topic-timer-heading")).toHaveText(
    "Replies on this topic are automatically deleted after 2 days.",
  );
  await modal.locator(".d-modal__footer .btn-primary").click();
  await expect(modal).toHaveCount(0);
  await expect
    .poll(() => timers(t.id, "status_type, based_on_last_post, duration_minutes"))
    .toEqual([["7", "f", "2880"]]);
});

test("a message is published to a category, which is required", async ({ page, context }) => {
  await login(context, "admin");
  const pm = await api(context, "POST", "/posts.json", {
    title: label("A message to publish"),
    raw: label("A message for the topic timer modal"),
    archetype: "private_message",
    target_recipients: "user1",
  });
  await page.goto(`/t/${pm.topic_slug}/${pm.topic_id}`);
  const modal = await openModal(page);
  expect((await types(modal)).pop()).toEqual(["publish_to_category", "Schedule Publishing"]);
  await modal.locator("select.timer-type").selectOption("publish_to_category");
  await expect(modal.locator(".control-group .control-label", { hasText: "Publish To:" })).toBeVisible();
  await modal.locator("#tap_tile_start_of_next_business_week").click();
  // No category yet: no preview, and none saved.
  await expect(modal.locator(".modal-topic-timer-info")).toHaveCount(0);
  await modal.locator(".d-modal__footer .btn-primary").click();
  await expect(modal.locator("#modal-alert")).toHaveText("Please select a category");
  expect(timers(pm.topic_id, "id")).toEqual([]);

  await modal.locator("details.category-chooser summary").click();
  await modal.locator('details.category-chooser .category-row[data-value="4"]').click();
  await expect(modal.locator("details.category-chooser summary")).toHaveAttribute("data-value", "4");
  const [[slug]] = sql("SELECT slug FROM categories WHERE id = 4");
  const link = modal.locator(".modal-topic-timer-info .topic-timer-heading a");
  await expect(link).toHaveText(`#${slug}`);
  await expect(link).toHaveAttribute("href", `/c/${slug}/4`);
  await expect(modal.locator(".modal-topic-timer-info .topic-timer-heading")).toContainText("This topic will be published to");
  const reloaded = page.waitForEvent("load");
  await modal.locator(".d-modal__footer .btn-primary").click();
  await expect(modal).toHaveCount(0);
  await expect
    .poll(() => timers(pm.topic_id, "status_type, category_id, execute_at > now()"))
    .toEqual([["3", "4", "t"]]);

  // The timer there: its category chosen.
  await reloaded;
  await settled(page);
  const again = await openModal(page);
  await expect(again.locator("select.timer-type")).toHaveValue("publish_to_category");
  await expect(again.locator("details.category-chooser summary")).toHaveAttribute("data-value", "4");
});

// The CI failure: the item clicked while the menu still floats down.
// Playwright's fourth try at a moving item scrolls it to the top of the
// window, here past the timeline's dock: the timeline undocks a frame
// later and the menu jumps with its trigger under the click. At rest
// the item is clicked where it is and the page never scrolls.
test("Set Topic Timer is clicked in a menu at rest, the page not scrolled to reach it", async ({
  page,
  context,
  session,
}) => {
  const user0 = await session("user0");
  const user1 = await session("user1");
  const t = await topic(
    user1,
    label("Topic timer modal"),
    label("A topic for the topic timer modal spec"),
    [1, 2].map((n) => ({ context: user0, raw: label(`Reply ${n} for the topic timer modal spec`) })),
  );
  await login(context, "admin");
  await page.goto(t.url);
  await expect(page.locator("#posts article[data-post-id]")).toHaveCount(3);
  // Long enough to leave the dock behind.
  await expect(page.locator(".timeline-container")).toHaveClass(/timeline-docked/);
  expect(await page.evaluate(() => document.documentElement.scrollHeight - window.innerHeight)).toBeGreaterThan(150);

  const menu = await topicAdminMenu(page);
  expect(await menu.evaluate((el) => el.getAnimations().filter((a) => a.playState !== "finished").length)).toBe(0);
  await page.evaluate(() => window.addEventListener("scroll", () => (window.scrolledForClick = true), { once: true }));
  await menu.locator(".admin-topic-timer-update .btn").click();
  await expect(page.locator(`${MODAL} .d-modal__title-text`)).toHaveText("Topic Timer");
  expect(await page.evaluate(() => window.scrolledForClick)).toBeUndefined();
});
