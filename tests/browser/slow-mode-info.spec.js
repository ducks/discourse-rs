// SlowModeInfo under a topic's posts (post_view::slow_mode_info,
// topic-admin-menu.js): the wait between posts for every viewer of an
// open topic in slow mode, before the topic timer's notice, and the
// trash can for who may manage topics, which turns slow mode off. Each
// change is checked in the topics row.
const { test, expect, login, api, topic, sql, label } = require("./helpers");

const MENU = ".topic-admin-menu-content";
const MODAL = ".d-modal.edit-slow-mode-modal";
const INFO = "#topic .topic-status-info";
const HEADING = `${INFO} .slow-mode-heading`;
// The notice itself: the heading less its button (its zero width space).
const NOTICE = `${HEADING} > span`;
const REMOVE = `${HEADING} > button.slow-mode-remove`;

async function setup(session, context) {
  const user1 = await session("user1");
  const t = await topic(user1, label("Slow mode info"), label("A topic for the slow mode info spec"));
  await login(context, "admin");
  return { ...t, author: user1 };
}

function seconds(id) {
  return sql("SELECT slow_mode_seconds FROM topics WHERE id = :'p1'", id);
}

function notice(duration) {
  return new RegExp(`^\\s*Please wait ${duration} between your posts in this topic\\s*$`);
}

test("slow mode's notice for staff, a member and a visitor; the trash can turns it off", async ({
  page,
  context,
  session,
}) => {
  const t = await setup(session, context);
  await page.goto(t.url);
  await expect(page.locator(INFO)).toHaveCount(0);

  // Set through EditSlowMode: the page is drawn again with the notice.
  await page.locator(".topic-admin-menu-trigger:visible").first().click();
  await page.locator(`${MENU} .topic-admin-slow-mode .btn`).click();
  const modal = page.locator(MODAL);
  await modal.locator(".slow-mode-type summary").click();
  await page.locator('.slow-mode-type .select-kit-row[data-value="600"]').click();
  let reloaded = page.waitForEvent("load");
  await modal.locator(".d-modal__footer .btn-primary").click();
  await expect.poll(() => seconds(t.id)).toEqual([["600"]]);
  await reloaded;

  await expect(page.locator(NOTICE)).toHaveText(notice("10 minutes"));
  await expect(page.locator(`${NOTICE} svg.d-icon-hourglass-start`)).toHaveCount(1);
  // canManageTopic's button, at the end of topic.scss's heading.
  const remove = page.locator(REMOVE);
  await expect(remove).toHaveClass("btn no-text btn-icon slow-mode-remove");
  await expect(remove.locator("svg.d-icon-trash-can")).toHaveCount(1);
  await expect(page.locator(HEADING)).toHaveCSS("display", "flex");
  const headingBox = await page.locator(HEADING).boundingBox();
  const removeBox = await remove.boundingBox();
  expect(Math.abs(headingBox.x + headingBox.width - (removeBox.x + removeBox.width))).toBeLessThan(2);
  // Before TopicTimerInfo, in the same container.
  await expect(page.locator(`${INFO} + .topic-timer-info`)).toHaveCount(1);

  // A visitor and the topic's author: the notice, no button.
  for (const other of [await session(null), t.author]) {
    const theirs = await other.newPage();
    await theirs.goto(t.url);
    await expect(theirs.locator(NOTICE)).toHaveText(notice("10 minutes"));
    await expect(theirs.locator(`${INFO} button`)).toHaveCount(0);
    await theirs.close();
  }

  // disableSlowMode: no seconds, the page drawn again without the notice.
  reloaded = page.waitForEvent("load");
  await remove.click();
  await expect.poll(() => seconds(t.id)).toEqual([["0"]]);
  await reloaded;
  await expect(page.locator(INFO)).toHaveCount(0);
  await expect(page.locator("#topic .topic-timer-info")).toHaveCount(1);
});

test("the notice reads moment's duration, stands before the timer's and is not drawn for a closed topic", async ({
  page,
  context,
  session,
}) => {
  const t = await setup(session, context);
  await page.goto(t.url);
  await page.evaluate(() => window.Discourse.timeShortcutPicker.load());
  for (const s of [30, 60, 2700, 14400, 86400]) {
    sql(`UPDATE topics SET slow_mode_seconds = ${s} WHERE id = :'p1'`, t.id);
    // durationTextFromSeconds
    const duration = await page.evaluate((n) => window.moment.duration(n, "seconds").humanize(), s);
    await page.reload();
    await expect(page.locator(NOTICE)).toHaveText(notice(duration));
    await page.evaluate(() => window.Discourse.timeShortcutPicker.load());
  }
  await expect(page.locator(NOTICE)).toHaveText(notice("a day"));

  // With a timer: both notices, slow mode's first, each with its buttons.
  await api(context, "POST", `/t/${t.id}/timer`, { time: "24", status_type: "close" });
  await page.reload();
  await expect(page.locator(`${INFO} + .topic-timer-info .topic-timer-heading`)).toHaveCount(1);
  await expect(page.locator(REMOVE)).toHaveCount(1);
  await expect(page.locator("#topic .topic-timer-info .topic-timer-modify button")).toHaveCount(2);
  const reloaded = page.waitForEvent("load");
  await page.locator(REMOVE).click();
  await expect.poll(() => seconds(t.id)).toEqual([["0"]]);
  await reloaded;
  await expect(page.locator(INFO)).toHaveCount(0);
  await expect(page.locator("#topic .topic-timer-info .topic-timer-heading")).toHaveCount(1);
  expect(
    sql("SELECT COUNT(*) FROM topic_timers WHERE timerable_id = :'p1' AND status_type = 1 AND deleted_at IS NULL", t.id),
  ).toEqual([["1"]]);

  // A closed topic in slow mode: nothing, for staff too.
  sql("UPDATE topics SET slow_mode_seconds = 600, closed = TRUE WHERE id = :'p1'", t.id);
  await page.reload();
  expect(seconds(t.id)).toEqual([["600"]]);
  await expect(page.locator(INFO)).toHaveCount(0);
});
