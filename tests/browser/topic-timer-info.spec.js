// TopicTimerInfo under a topic's posts (post_view::topic_timer_info,
// topic-admin-menu.js, topic.js): the public timer's notice for every
// viewer, its time left and title in the browser's zone, the pencil and
// the trash can for who may set timers, and the notice going with its
// time. Each change is checked in topic_timers.
const { test, expect, login, api, topic, sql, label } = require("./helpers");

const MENU = ".topic-admin-menu-content";
const MODAL = ".d-modal.edit-topic-timer-modal";
const INFO = "#topic .topic-timer-info";
const HEADING = `${INFO} .topic-timer-heading`;
// The notice itself: the heading less its buttons (their zero width spaces).
const NOTICE = `${HEADING} > span`;

async function setup(session, context) {
  const user1 = await session("user1");
  const t = await topic(user1, label("Topic timer info"), label("A topic for the topic timer info spec"));
  await login(context, "admin");
  return { ...t, author: user1 };
}

// The topic's live timers.
function timers(id, columns) {
  return sql(
    `SELECT ${columns} FROM topic_timers WHERE timerable_id = :'p1' AND type = 'TopicTimer' AND deleted_at IS NULL`,
    id,
  );
}

test("a close timer's notice for staff, a member and a visitor; the pencil edits and the trash can removes it", async ({
  page,
  context,
  session,
}) => {
  const t = await setup(session, context);
  await page.goto(t.url);
  await expect(page.locator(INFO)).toBeEmpty();

  // Set through the modal: the page is drawn again with the notice.
  await page.locator(".topic-admin-menu-trigger:visible").first().click();
  await page.locator(`${MENU} .admin-topic-timer-update .btn`).click();
  const modal = page.locator(MODAL);
  await modal.locator("#tap_tile_tomorrow").click();
  let reloaded = page.waitForEvent("load");
  await modal.locator(".d-modal__footer .btn-primary").click();
  await reloaded;
  await expect.poll(() => timers(t.id, "status_type, public_type")).toEqual([["1", "t"]]);

  // Tomorrow at eight: within the day, as moment's humanize(true) reads.
  const notice = /^\s*This topic will automatically close in (a day|\d+ hours)\.\s*$/;
  await expect(page.locator(NOTICE)).toHaveText(notice);
  await expect(page.locator(`${NOTICE} svg.d-icon-far-clock`)).toHaveCount(1);
  // The title is moment's LLLL of the time, in the browser's zone.
  const [[ms]] = timers(t.id, "(extract(epoch from execute_at) * 1000)::bigint");
  await page.evaluate(() => window.Discourse.timeShortcutPicker.load());
  const full = await page.evaluate((at) => window.moment(at).format("LLLL"), Number(ms));
  expect(full).toMatch(/^\w+day, \w+ \d{1,2}, \d{4} \d{1,2}:\d{2} [AP]M$/);
  await expect(page.locator(NOTICE)).toHaveAttribute("title", full);
  await expect(page.locator(`${INFO} [data-tz-title], ${INFO} [data-tz-text]`)).toHaveCount(0);
  // canSetTopicTimer's buttons.
  const edit = page.locator(`${HEADING} .topic-timer-modify button.topic-timer-edit`);
  const remove = page.locator(`${HEADING} .topic-timer-modify button.topic-timer-remove`);
  await expect(edit).toHaveAttribute("title", "edit timer");
  await expect(remove).toHaveAttribute("title", "remove timer");
  await expect(edit.locator("svg.d-icon-pencil")).toHaveCount(1);
  await expect(remove.locator("svg.d-icon-trash-can")).toHaveCount(1);
  // topic.scss's heading: the buttons at its end.
  await expect(page.locator(HEADING)).toHaveCSS("display", "flex");
  const headingBox = await page.locator(HEADING).boundingBox();
  const removeBox = await remove.boundingBox();
  expect(Math.abs(headingBox.x + headingBox.width - (removeBox.x + removeBox.width))).toBeLessThan(2);

  // A visitor and the topic's author: the notice, no buttons.
  for (const other of [await session(null), t.author]) {
    const theirs = await other.newPage();
    await theirs.goto(t.url);
    await expect(theirs.locator(NOTICE)).toHaveText(notice);
    await expect(theirs.locator(NOTICE)).toHaveAttribute("title", full);
    await expect(theirs.locator(`${HEADING} .topic-timer-modify`)).toHaveCount(1);
    await expect(theirs.locator(`${INFO} button`)).toHaveCount(0);
    await theirs.close();
  }

  // The pencil: EditTopicTimer with the timer.
  await edit.click();
  await expect(modal.locator(".d-modal__title-text")).toHaveText("Topic Timer");
  await expect(modal.locator("select.timer-type")).toHaveValue("close");
  await expect(modal.locator(".d-modal__footer .btn-danger")).toHaveText("Remove Timer");
  await modal.locator(".d-modal__header .modal-close").click();
  await expect(modal).toHaveCount(0);
  expect(timers(t.id, "status_type")).toEqual([["1"]]);

  // The trash can: the timer trashed, the page drawn again without it.
  reloaded = page.waitForEvent("load");
  await remove.click();
  await expect.poll(() => timers(t.id, "id")).toEqual([]);
  expect(sql("SELECT COUNT(*) FROM topic_timers WHERE timerable_id = :'p1' AND deleted_at IS NOT NULL", t.id)).toEqual([
    ["1"],
  ]);
  await reloaded;
  await expect(page.locator(HEADING)).toHaveCount(0);
  await expect(page.locator(INFO)).toBeEmpty();
});

test("timers after the last post and of replies read their duration, and the trash can removes one", async ({
  page,
  context,
  session,
}) => {
  const t = await setup(session, context);
  await api(context, "POST", `/t/${t.id}/timer`, {
    time: "",
    status_type: "close",
    based_on_last_post: "true",
    duration_minutes: "300",
  });
  expect(timers(t.id, "status_type, based_on_last_post, duration_minutes")).toEqual([["1", "t", "300"]]);
  await page.goto(t.url);
  await expect(page.locator(NOTICE)).toHaveText(/^\s*This topic will close 5 hours after the last reply\.\s*$/);
  await expect(page.locator(NOTICE)).toHaveAttribute("title", /^\w+day, \w+ \d{1,2}, \d{4} \d{1,2}:\d{2} [AP]M$/);

  // Deleting replies stays past its time, with no time to go at.
  sql(
    "UPDATE topic_timers SET status_type = 7, based_on_last_post = FALSE, duration_minutes = 2880, execute_at = now() - interval '1 hour' WHERE timerable_id = :'p1'",
    t.id,
  );
  await page.reload();
  await expect(page.locator(NOTICE)).toHaveText(/^\s*Replies on this topic are automatically deleted after 2 days\.\s*$/);
  await expect(page.locator(INFO)).not.toHaveAttribute("data-execute-at");

  // removeTopicTimer sends the timer's own type.
  const reloaded = page.waitForEvent("load");
  await page.locator(`${HEADING} button.topic-timer-remove`).click();
  await expect.poll(() => timers(t.id, "id")).toEqual([]);
  await reloaded;
  await expect(page.locator(HEADING)).toHaveCount(0);
});

test("the notice goes once the timer's time has come, and is not drawn for the state the topic is in", async ({
  page,
  context,
  session,
}) => {
  const t = await setup(session, context);
  await api(context, "POST", `/t/${t.id}/timer`, { time: "24", status_type: "close" });
  const visitor = await (await session(null)).newPage();
  await visitor.clock.install();
  await visitor.goto(t.url);
  await expect(visitor.locator(NOTICE)).toHaveText(/^\s*This topic will automatically close in a day\.\s*$/);
  // Two days on in the page (the job has not run): renderTopicTimer
  // finds the time past.
  await visitor.clock.fastForward("48:00:00");
  await expect(visitor.locator(HEADING)).toHaveCount(0);
  await expect(visitor.locator(INFO)).toBeEmpty();

  // A close timer on a closed topic (the status toggled under it).
  sql("UPDATE topics SET closed = TRUE WHERE id = :'p1'", t.id);
  await page.goto(t.url);
  expect(timers(t.id, "status_type")).toEqual([["1"]]);
  await expect(page.locator(INFO)).toBeEmpty();
});
