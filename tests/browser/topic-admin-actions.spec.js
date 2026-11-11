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
  await settled(page);
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
  await settled(page);
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
  await settled(page);
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
  await settled(page);
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
  await settled(page);
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

// The site banner (DiscourseBanner, static/js/site-banner.js): the banner
// topic drawn atop the lists, the categories page, the tags index and the
// topic, dismissed (the member's dismissed_banner_key, a visitor's in
// localStorage) and drawn or taken away live as it is made and removed.
// Here, with the topic admin specs, so no other file makes a banner
// meanwhile; each test removes its banner.
async function bannerTopic(session) {
  const user1 = await session("user1");
  const text = label("The first post of the banner");
  const t = await topic(user1, label("Banner topic"), text);
  return { ...t, text };
}

function removeBanners() {
  sql("UPDATE topics SET archetype = 'regular' WHERE archetype = 'banner'");
}

test.afterEach(removeBanners);

test("the banner is drawn atop pages and a member dismisses it", async ({ page, context, session }) => {
  await login(context, "admin");
  const t = await bannerTopic(session);
  await api(context, "PUT", `/t/${t.id}/make-banner`, {});

  // Staff: the edit link to the topic, with its label on the desktop.
  await page.goto("/latest");
  const banner = page.locator("#banner");
  await expect(banner.locator("#banner-content")).toHaveText(t.text);
  await expect(banner.locator(".edit-banner")).toHaveText("Edit");
  await expect(banner.locator(".edit-banner")).toHaveAttribute("href", new RegExp(`/t/[^/]+/${t.id}$`));
  // Atop the discovery layout, before the list controls.
  expect(
    await page.evaluate(() => {
      const b = document.getElementById("banner").closest(".container");
      const controls = document.querySelector(".list-controls");
      return !!(b.compareDocumentPosition(controls) & Node.DOCUMENT_POSITION_FOLLOWING);
    }),
  ).toBe(true);

  const member = await session("user1");
  const memberPage = await member.newPage();
  for (const path of ["/latest", "/categories", "/tags", t.url]) {
    await memberPage.goto(path);
    await expect(memberPage.locator("#banner #banner-content"), path).toHaveText(t.text);
    await expect(memberPage.locator("#banner .edit-banner")).toHaveCount(0);
  }
  // The topic and the tags index draw it in a container of its own.
  await expect(memberPage.locator(".container > .container > .row > #banner")).toHaveCount(1);

  await memberPage.locator("#banner .close").click();
  await expect(memberPage.locator("#banner")).toHaveCount(0);
  await expect
    .poll(() =>
      sql("SELECT dismissed_banner_key FROM user_profiles WHERE user_id = (SELECT id FROM users WHERE username = 'user1')"),
    )
    .toEqual([[String(t.id)]]);
  await memberPage.goto("/latest");
  await expect(memberPage.locator(".topic-list")).toBeVisible();
  await expect(memberPage.locator("#banner")).toHaveCount(0);
  // The admin still has it.
  await page.reload();
  await expect(page.locator("#banner")).toHaveCount(1);
});

test("a visitor dismisses the banner until it changes", async ({ page, context, session }) => {
  const admin = await session("admin");
  const t = await bannerTopic(session);
  await api(admin, "PUT", `/t/${t.id}/make-banner`, {});
  await page.goto("/categories");
  await expect(page.locator("#banner #banner-content")).toHaveText(t.text);
  await page.locator("#banner .close").click();
  await expect(page.locator("#banner")).toHaveCount(0);
  expect(await page.evaluate(() => localStorage.getItem("discourse_dismissed_banner_key"))).toBe(String(t.id));
  await page.goto("/latest");
  await expect(page.locator(".topic-list")).toBeVisible();
  await expect(page.locator("#banner")).toHaveCount(0);

  // Another banner has another key.
  const other = await bannerTopic(session);
  await api(admin, "PUT", `/t/${other.id}/make-banner`, {});
  await page.goto("/latest");
  await expect(page.locator("#banner #banner-content")).toHaveText(other.text);
});

test("the banner arrives and goes live", async ({ page, context, session }) => {
  const admin = await session("admin");
  const t = await bannerTopic(session);
  await login(context, "user1");
  await page.goto("/latest");
  await expect(page.locator(".topic-list")).toBeVisible();
  await expect(page.locator("#banner")).toHaveCount(0);
  // A visitor on the categories page hears it too.
  const visitor = await session(null);
  const visitorPage = await visitor.newPage();
  await visitorPage.goto("/categories");
  await expect(visitorPage.locator("#banner")).toHaveCount(0);
  // The pages' streams are up before the banner is made.
  await page.waitForTimeout(1000);

  await api(admin, "PUT", `/t/${t.id}/make-banner`, {});
  await expect(page.locator("#banner #banner-content")).toHaveText(t.text);
  await expect(visitorPage.locator("#banner #banner-content")).toHaveText(t.text);

  await api(admin, "PUT", `/t/${t.id}/remove-banner`, {});
  await expect(page.locator("#banner")).toHaveCount(0);
  await expect(visitorPage.locator("#banner")).toHaveCount(0);
});
