// The topic admin menu's actions and modals (topic-admin-menu.js,
// topic-admin-modals.js): FeatureTopic (pinning until a date in the
// category and globally, unpinning, the banner), recovering a deleted
// topic, ChangeTimestamp, resetting the bump date, EditSlowMode and
// ConvertToPublicTopic. Each change is checked in the database, the page
// drawn again; what stays unported is refused, named.
const { test, expect, login, api, topic, sql, label } = require("./helpers");

async function setup(session, context) {
  const user1 = await session("user1");
  const user0 = await session("user0");
  const t = await topic(user1, label("Topic admin actions"), label("A topic for the topic admin actions spec"), [
    { context: user0, raw: label("A reply by user0 for the topic admin actions") },
  ]);
  await login(context, "admin");
  return t;
}

const MENU = ".topic-admin-menu-content";
// The page's next load: an action saved draws the page again.
let reloaded;

async function openMenu(page) {
  await page.locator(".topic-admin-menu-trigger:visible").first().click();
  await expect(page.locator(MENU)).toBeVisible();
}

async function menuItem(page, name) {
  await openMenu(page);
  await page.locator(`${MENU} .${name} .btn`).click();
}

function topicRow(t, columns) {
  return sql(`SELECT ${columns} FROM topics WHERE id = :'p1'`, t.id)[0];
}

test("FeatureTopic pins until a date, then unpins", async ({ page, context, session }) => {
  const t = await setup(session, context);
  await page.goto(t.url);
  await menuItem(page, "topic-admin-pin");
  const modal = page.locator(".d-modal.feature-topic");
  await expect(modal.locator(".d-modal__title-text")).toHaveText("Feature this topic");
  // The counts load: General's pinned topics, and the global ones.
  const sections = modal.locator("form.feature-section");
  await expect(sections).toHaveCount(2);
  await expect(sections.nth(0).locator(".feature-section__description p").first()).toContainText(/pinned in|no topics pinned/);
  await expect(sections.nth(0).locator(".feature-section__pin-message")).toContainText(
    "Make this topic appear at the top of the",
  );

  // The date is required.
  await sections.nth(0).locator(".form-kit__button").click();
  await expect(sections.nth(0).locator(".form-kit__errors")).toHaveText("A date is required to pin this topic.");
  expect(topicRow(t, "pinned_at IS NULL")).toEqual(["t"]);

  // A shortcut: next month.
  await sections.nth(0).locator(".future-date-input-selector summary").click();
  await page.locator('.future-date-input-selector-row[data-value="next_month"]').click();
  await expect(sections.nth(0).locator(".form-kit__errors")).toHaveCount(0);
  reloaded = page.waitForEvent("load");
  await sections.nth(0).locator(".form-kit__button").click();
  await expect(modal).toHaveCount(0);
  await expect
    .poll(() => topicRow(t, "pinned_at IS NOT NULL, pinned_globally, pinned_until > now() + interval '1 day'"))
    .toEqual(["t", "f", "t"]);
  expect(
    sql(
      "SELECT COUNT(*) FROM discourse_rs.jobs WHERE name = 'unpin_topic' AND args->>'topic_id' = :'p1' AND at_time",
      t.id,
    ),
  ).toEqual([["1"]]);

  // Pinned: the until date and Un-Pin Topic.
  await reloaded;
  await menuItem(page, "topic-admin-pin");
  await expect(modal.locator(".feature-section__description").first()).toContainText("or wait until");
  await modal.locator(".btn-primary", { hasText: "Un-Pin Topic" }).click();
  await expect.poll(() => topicRow(t, "pinned_at IS NULL, pinned_until IS NULL")).toEqual(["t", "t"]);
  expect(
    sql("SELECT COUNT(*) FROM discourse_rs.jobs WHERE name = 'unpin_topic' AND args->>'topic_id' = :'p1'", t.id),
  ).toEqual([["0"]]);
});

test("FeatureTopic pins globally and makes and removes the banner", async ({ page, context, session }) => {
  const t = await setup(session, context);
  await page.goto(t.url);
  await menuItem(page, "topic-admin-pin");
  const modal = page.locator(".d-modal.feature-topic");
  const global = modal.locator("form.feature-section").nth(1);
  await expect(global.locator(".feature-section__pin-message")).toContainText(
    "Make this topic appear at the top of all topic lists until",
  );
  await global.locator(".future-date-input-selector summary").click();
  await page.locator('.future-date-input-selector-row[data-value="two_weeks"]').click();
  reloaded = page.waitForEvent("load");
  await global.locator(".form-kit__button").click();
  const dialog = page.locator("#dialog-holder");
  // Past four global pins a confirmation comes first.
  if (await dialog.locator(".dialog-footer .btn-primary").isVisible().catch(() => false)) {
    await dialog.locator(".dialog-footer .btn-primary").click();
  }
  await expect.poll(() => topicRow(t, "pinned_at IS NOT NULL, pinned_globally")).toEqual(["t", "t"]);

  await reloaded;
  await menuItem(page, "topic-admin-pin");
  await expect(modal.locator(".feature-section__description").first()).toContainText(
    "Remove this topic from the top of all topic lists",
  );
  reloaded = page.waitForEvent("load");
  await modal.locator(".make-banner").click();
  await expect.poll(() => topicRow(t, "archetype")).toEqual(["banner"]);
  expect(
    sql("SELECT action_code FROM posts WHERE topic_id = :'p1' AND post_type = 3 ORDER BY post_number", t.id),
  ).toEqual([["pinned_globally.enabled"], ["banner.enabled"]]);

  await reloaded;
  await menuItem(page, "topic-admin-pin");
  await expect(modal).toContainText("Remove the banner that appears at the top of all pages.");
  await modal.locator(".btn-primary", { hasText: "Remove Banner Topic" }).click();
  await expect.poll(() => topicRow(t, "archetype")).toEqual(["regular"]);
  // Unpinned too, so the next run of a spec pinning globally counts the
  // same pins.
  await api(context, "PUT", `/t/${t.id}/status`, { status: "pinned_globally", enabled: "false" });
});

test("Recover PUTs /t/:id/recover", async ({ page, context, session }) => {
  const t = await setup(session, context);
  await page.goto(t.url);
  // A deleted topic's page is refused while discourse-post-voting is on
  // (Rails raises there too), and the browser setup turns it on: the
  // topic is deleted behind the page, whose menu then offers Recover as
  // a deleted topic's does.
  await api(context, "DELETE", `/t/${t.id}.json`, { context: "spec" });
  await page.evaluate(() => {
    const template = document.querySelector("template.topic-admin-menu-template");
    const item = template.content.querySelector(".topic-admin-delete");
    item.classList.replace("topic-admin-delete", "topic-admin-recover");
  });
  reloaded = page.waitForEvent("load");
  await menuItem(page, "topic-admin-recover");
  await expect.poll(() => topicRow(t, "deleted_at IS NULL")).toEqual(["t"]);
  expect(sql("SELECT deleted_at IS NULL FROM posts WHERE id = :'p1'", t.posts[0])).toEqual([["t"]]);
  expect(sql("SELECT COUNT(*) FROM user_histories WHERE action = 55 AND topic_id = :'p1'", t.id)).toEqual([["1"]]);
  await reloaded;
  await openMenu(page);
  await expect(page.locator(`${MENU} .topic-admin-delete`)).toHaveCount(1);
});

test("ChangeTimestamp moves the topic to a past date", async ({ page, context, session }) => {
  const t = await setup(session, context);
  await page.goto(t.url);
  await menuItem(page, "topic-admin-change-timestamp");
  const modal = page.locator(".d-modal");
  await expect(modal.locator(".d-modal__title-text")).toHaveText("Change Timestamp…");
  await expect(modal.locator(".d-modal__body.change-timestamp p").first()).toContainText("Please select the new timestamp of the topic.");
  // A time to come: the alert, and the button disabled.
  const future = await page.evaluate(() => {
    const d = new Date(Date.now() + 2 * 3600 * 1000);
    return d.getDate() === new Date().getDate() ? String(d.getHours()).padStart(2, "0") + ":00" : null;
  });
  const submit = modal.locator(".d-modal__footer .btn-primary");
  if (future) {
    await modal.locator('input[type="time"]').fill(future);
    await expect(modal.locator(".alert-error")).not.toHaveClass(/hidden/);
    await expect(submit).toBeDisabled();
  }
  // The first of this month at midnight.
  await modal.locator("#date-container .pika-button[data-pika-day='1']").first().click();
  await modal.locator('input[type="time"]').fill("00:00");
  await expect(modal.locator(".alert-error")).toHaveClass(/hidden/);
  await expect(submit).toBeEnabled();
  await submit.click();
  await expect(modal).toHaveCount(0);
  const expected = await page.evaluate(() => {
    const d = new Date();
    return Math.floor(new Date(d.getFullYear(), d.getMonth(), 1, 0, 0, 0).getTime() / 1000);
  });
  await expect.poll(() => topicRow(t, "EXTRACT(EPOCH FROM created_at)::bigint")).toEqual([String(expected)]);
  expect(sql("SELECT COUNT(*) FROM user_histories WHERE action = 68 AND topic_id = :'p1'", t.id)).toEqual([["1"]]);
});

test("reset bump date and slow mode", async ({ page, context, session }) => {
  const t = await setup(session, context);
  sql("UPDATE topics SET bumped_at = now() + interval '1 day' WHERE id = :'p1'", t.id);
  await page.goto(t.url);
  await menuItem(page, "topic-admin-reset-bump-date");
  await expect
    .poll(() => topicRow(t, "bumped_at = (SELECT max(created_at) FROM posts WHERE topic_id = topics.id AND post_type = 1)"))
    .toEqual(["t"]);

  await menuItem(page, "topic-admin-slow-mode");
  const modal = page.locator(".d-modal.edit-slow-mode-modal");
  await expect(modal.locator(".d-modal__title-text")).toHaveText("Slow mode");
  await modal.locator(".slow-mode-type summary").click();
  await page.locator('.slow-mode-type .select-kit-row[data-value="custom"]').click();
  await expect(modal.locator("input.input-small")).toHaveCount(3);
  await modal.locator(".slow-mode-type summary").click();
  await page.locator('.slow-mode-type .select-kit-row[data-value="600"]').click();
  await expect(modal.locator("input.input-small")).toHaveCount(0);
  await expect(modal.locator(".enabled-until label.slow-mode-label")).toHaveText("Enabled until:");
  reloaded = page.waitForEvent("load");
  await modal.locator(".d-modal__footer .btn-primary").click();
  await expect.poll(() => topicRow(t, "slow_mode_seconds")).toEqual(["600"]);

  await reloaded;
  await menuItem(page, "topic-admin-slow-mode");
  await expect(modal.locator(".slow-mode-type summary")).toHaveAttribute("data-value", "600");
  await expect(modal.locator(".d-modal__footer .btn-primary")).toHaveText("Update");
  await modal.locator(".d-modal__footer .btn-danger").click();
  await expect.poll(() => topicRow(t, "slow_mode_seconds")).toEqual(["0"]);
});

test("ConvertToPublicTopic makes a message a topic in the category chosen", async ({ page, context, session }) => {
  await login(context, "admin");
  const pm = await api(context, "POST", "/posts.json", {
    title: label("A message made public"),
    raw: label("A message for the make public modal"),
    archetype: "private_message",
    target_recipients: "user1",
  });
  await page.goto(`/t/${pm.topic_slug}/${pm.topic_id}`);
  await openMenu(page);
  await page.locator(`${MENU} .topic-admin-convert .btn`).click();
  const modal = page.locator(".d-modal.convert-to-public-topic");
  await expect(modal.locator(".d-modal__title-text")).toHaveText("Convert to Public Topic");
  await expect(modal.locator(".instructions")).toHaveText("Please choose a category for the public topic:");
  await modal.locator("details.category-chooser summary").click();
  await modal.locator('details.category-chooser .category-row[data-value="4"]').click();
  await expect(modal.locator("details.category-chooser summary")).toHaveAttribute("data-value", "4");
  await modal.locator(".d-modal__footer .btn-primary").click();
  await expect
    .poll(() => sql("SELECT archetype, category_id FROM topics WHERE id = :'p1'", pm.topic_id))
    .toEqual([["regular", "4"]]);
});

test("the timer and multi-select stay refused, named", async ({ page, context, session }) => {
  const t = await setup(session, context);
  await page.goto(t.url);
  await menuItem(page, "admin-topic-timer-update");
  await expect(page.locator("#page-error")).toHaveText("Not ported yet: the topic timer modal (EditTopicTimer, 194c1).");
  await menuItem(page, "topic-admin-multi-select");
  await expect(page.locator("#page-error")).toHaveText(
    "Not ported yet: selecting posts (the topic's multi-select mode, 194c1).",
  );
});
