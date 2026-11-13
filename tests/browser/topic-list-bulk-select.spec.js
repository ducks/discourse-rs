// The topic lists' bulk select mode (topic-list-bulk-select.js, with
// topic-dismiss.js for the dismissals): the toggle turns it on and off,
// rows are selected by their checkbox, by a click on the row and by a
// shift range, a topic manager's Bulk Actions menu offers what Ember's
// rules allow and its actions change exactly the selected topics, a
// member on the new and unread lists dismisses what they selected, and
// what is not ported says so. Each change is checked in the database.
// Every test works in a category of its own, on topics found by id.
const { test, expect, login, api, sql, label, member } = require("./helpers");

const TABLE = "#list-area table.topic-list";
const TOGGLE = "#list-area thead.topic-list-header button.bulk-select";
const MENU = ".fk-d-menu.bulk-select-topics-dropdown-content";
const MODAL = ".d-modal.topic-bulk-actions-modal";
const COUNT = "#list-area thead .bulk-select-topic-dropdown__count";
const TRIGGER = "#list-area thead .bulk-select-topics-dropdown-trigger";
const SUBSET = "#list-area .contents > .topic-replies-toggle-wrapper";

async function category(admin, what = "Bulk") {
  const name = label(what);
  const created = await api(admin, "POST", "/categories.json", { name, color: "0088CC", text_color: "FFFFFF" });
  const cat = created.category;
  cat.path = `/c/${cat.slug}/${cat.id}`;
  cat.definition = cat.topic_url ? Number(cat.topic_url.split("/").pop()) : null;
  return cat;
}

// `count` topics by `author` in the category; their ids, oldest first.
async function topicsIn(author, cat, count, what = "A bulk select topic") {
  const ids = [];
  for (let i = 0; i < count; i++) {
    const title = label(`${what} ${i}`);
    const post = await api(author, "POST", "/posts.json", {
      title,
      raw: `The first post of ${title}, in the bulk select category.`,
      category: String(cat.id),
    });
    ids.push(post.topic_id);
  }
  return ids;
}

function row(page, id) {
  return page.locator(`#list-area tbody.topic-list-body > tr[data-topic-id="${id}"]`);
}

function box(page, id) {
  return page.locator(`#list-area input#bulk-select-${id}`);
}

// The ids of the rows drawn selected, sorted.
function selected(page) {
  return page
    .locator("#list-area tbody.topic-list-body > tr.bulk-selected")
    .evaluateAll((rows) => rows.map((r) => Number(r.dataset.topicId)).sort((a, b) => a - b));
}

function sorted(ids) {
  return [...ids].sort((a, b) => a - b);
}

async function enter(page, path, ids) {
  await page.goto(path);
  for (const id of ids) {
    await expect(row(page, id)).toBeVisible();
  }
  await page.locator(TOGGLE).click();
  await expect(page.locator(TABLE)).toHaveClass("topic-list sticky-header bulk-select-enabled");
}

async function select(page, ids) {
  for (const id of ids) {
    await box(page, id).click();
    await expect(row(page, id)).toHaveClass(/(^| )bulk-selected( |$)/);
  }
}

// The Bulk Actions menu's items, by their classes.
async function items(page) {
  await page.locator(TRIGGER).click();
  await expect(page.locator(MENU)).toBeVisible();
  return page
    .locator(`${MENU} .dropdown-menu__item > button`)
    .evaluateAll((all) => all.map((b) => b.className.replace(/^btn btn-icon-text btn-transparent /, "")));
}

async function choose(page, name) {
  await page.locator(TRIGGER).click();
  await page.locator(`${MENU} button.${name}`).click();
  await expect(page.locator(MENU)).toHaveCount(0);
}

// The modal's confirm button, then the list drawn again out of the mode
// under the toast.
async function confirm(page, text) {
  const button = page.locator(`${MODAL} #bulk-topics-confirm`);
  await expect(button).toHaveText(text);
  await button.click();
  await completed(page);
}

async function completed(page) {
  await expect(page.locator(".fk-d-default-toast.-success .fk-d-default-toast__message")).toHaveText(
    "Bulk operations completed successfully!",
  );
  await expect(page.locator(MODAL)).toHaveCount(0);
  await expect(page.locator(TABLE)).toHaveClass("topic-list");
  await expect(page.locator("#list-area input.bulk-select")).toHaveCount(0);
}

// A topics column of the given ids, as [id, value] in id order.
function column(ids, name) {
  return sql(`SELECT id, ${name} FROM topics WHERE id IN (${ids.join(",")}) ORDER BY id`).map((r) => [
    Number(r[0]),
    r[1],
  ]);
}

function userId(name) {
  return Number(sql("SELECT id FROM users WHERE username = :'p1'", name)[0][0]);
}

test("the toggle turns the mode on and off, and rows select by checkbox, row click and shift range", async ({
  page,
  context,
  session,
}) => {
  const admin = await session("admin");
  const cat = await category(admin);
  const ids = await topicsIn(await session("user1"), cat, 3);
  await login(context, "admin");
  await page.goto(cat.path);
  await expect(row(page, ids[0])).toBeVisible();
  const header = page.locator("#list-area thead.topic-list-header > tr");
  await expect(header.locator("th.default > span")).toHaveText("Topic");
  await expect(page.locator(TOGGLE)).toHaveAttribute("title", "toggle bulk selection of topics");

  await page.locator(TOGGLE).click();
  await expect(page.locator(TABLE)).toHaveClass("topic-list sticky-header bulk-select-enabled");
  // HeaderBulkSelectCell first, the toggle in it; the column names gone.
  await expect(header.locator("> th").first()).toHaveClass("bulk-select topic-list-data");
  await expect(header.locator("> th.bulk-select > button")).toHaveClass("btn-transparent bulk-select no-text --primary");
  await expect(header.locator("th.default > button.bulk-select")).toHaveCount(0);
  await expect(header.locator("th.default > span:not(.bulk-select-topics)")).toHaveCount(0);
  await expect(header.locator("th.posts button")).toHaveCount(0);
  await expect(header.locator(".bulk-select-topics-actions > button")).toHaveText(["Select All", "Clear All"]);
  await expect(page.locator(TRIGGER)).toHaveCount(0);
  for (const id of ids) {
    await expect(row(page, id)).toHaveClass(/(^| )bulk-selecting( |$)/);
    await expect(row(page, id).locator("> td").first()).toHaveClass("bulk-select topic-list-data");
    await expect(row(page, id).locator(`> td.bulk-select > label[for="bulk-select-${id}"] > input.bulk-select`)).not.toBeChecked();
  }

  // A checkbox, then a click on another row's title, which does not go
  // to the topic.
  await box(page, ids[0]).click();
  await expect(page.locator(COUNT)).toHaveText("1 selected");
  await expect(page.locator(`${TRIGGER} .d-button-label`)).toHaveText("Bulk Actions");
  await row(page, ids[1]).locator("a.raw-topic-link").click();
  await expect(page.locator(COUNT)).toHaveText("2 selected");
  await expect(page).toHaveURL(new RegExp(`${cat.path}$`));
  await expect(box(page, ids[1])).toBeChecked();
  expect(await selected(page)).toEqual(sorted([ids[0], ids[1]]));
  // And a click on it again unselects it.
  await row(page, ids[1]).locator("a.raw-topic-link").click();
  await expect(page.locator(COUNT)).toHaveText("1 selected");
  await expect(box(page, ids[1])).not.toBeChecked();

  // Clear All takes the menu away with the selection; Select All takes
  // every row.
  await header.locator(".bulk-clear-all").click();
  await expect(page.locator(TRIGGER)).toHaveCount(0);
  expect(await selected(page)).toEqual([]);
  await header.locator(".bulk-select-all").click();
  const all = await page.locator("#list-area tbody.topic-list-body > tr[data-topic-id]").count();
  expect(all).toBeGreaterThanOrEqual(3);
  await expect(page.locator(COUNT)).toHaveText(`${all} selected`);
  await expect(page.locator("#list-area input.bulk-select:checked")).toHaveCount(all);
  await header.locator(".bulk-clear-all").click();
  await expect(page.locator("#list-area input.bulk-select:checked")).toHaveCount(0);

  // Shift on a checkbox: the rows between it and the last one checked.
  // The list is newest first.
  await box(page, ids[2]).click();
  await box(page, ids[0]).click({ modifiers: ["Shift"] });
  await expect(page.locator(COUNT)).toHaveText("3 selected");
  expect(await selected(page)).toEqual(sorted(ids));

  // Off: the list as it was drawn.
  await header.locator("> th.bulk-select > button").click();
  await expect(page.locator(TABLE)).toHaveClass("topic-list");
  await expect(page.locator("#list-area input.bulk-select")).toHaveCount(0);
  await expect(page.locator("#list-area th.bulk-select, #list-area td.bulk-select")).toHaveCount(0);
  await expect(page.locator("#list-area tr.bulk-selecting, #list-area tr.bulk-selected")).toHaveCount(0);
  await expect(header.locator("th.default > span")).toHaveText("Topic");
  await expect(header.locator("th.posts button")).toHaveText("Replies");
  // On again nothing is selected.
  await page.locator(TOGGLE).click();
  await expect(page.locator("#list-area input.bulk-select")).toHaveCount(all);
  await expect(page.locator("#list-area input.bulk-select:checked")).toHaveCount(0);
});

test("a visitor and a member on latest get no toggle", async ({ page, context }) => {
  await page.goto("/latest");
  await expect(page.locator("#list-area tbody.topic-list-body > tr[data-topic-id]").first()).toBeVisible();
  await expect(page.locator("button.bulk-select")).toHaveCount(0);
  await expect(page.locator("#topic-bulk-select")).toHaveCount(0);
  await login(context, "user1");
  await page.goto("/latest");
  await expect(page.locator("#list-area tbody.topic-list-body > tr[data-topic-id]").first()).toBeVisible();
  await expect(page.locator("button.bulk-select")).toHaveCount(0);
  await expect(page.locator("#topic-bulk-select")).toHaveCount(0);
});

test("an admin closes, archives, unlists and relists exactly the selected topics", async ({ page, context, session }) => {
  const admin = await session("admin");
  const cat = await category(admin);
  const ids = await topicsIn(await session("user1"), cat, 3);
  const [a, b, c] = ids;
  await login(context, "admin");

  await enter(page, cat.path, ids);
  await select(page, [a, b]);
  const config = await page.evaluate(() => JSON.parse(document.getElementById("topic-bulk-select").textContent));
  // BulkSelectTopicsDropdown#buttons for staff on a latest list, two
  // listed and unpinned topics selected.
  expect(await items(page)).toEqual(
    [
      "update-category",
      "update-notifications",
      "reset-bump-dates",
      "defer",
      "close-topics",
      config.manage_tags && "manage-tags",
      "pin-topics",
      "archive-topics",
      "convert-to-private-message",
      "unlist-topics",
      "delete-topics",
    ].filter(Boolean),
  );
  await expect(page.locator(`${MENU} .dropdown-menu__item > button .d-button-label`).first()).toHaveText("Update Category");
  await page.keyboard.press("Escape");
  await expect(page.locator(MENU)).toHaveCount(0);

  await choose(page, "close-topics");
  await expect(page.locator(`${MODAL} .d-modal__title-text`)).toHaveText("Close topics");
  await expect(page.locator(`${MODAL} .topic-bulk-actions-modal__description`)).toContainText("Closed topics prevent new replies.");
  await expect(page.locator(`${MODAL} #bulk-close-note`)).toBeVisible();
  // Cancel changes nothing and leaves the mode on.
  await page.locator(`${MODAL} #bulk-topics-cancel`).click();
  await expect(page.locator(MODAL)).toHaveCount(0);
  await expect(page.locator(COUNT)).toHaveText("2 selected");
  expect(column(ids, "closed")).toEqual(sorted(ids).map((id) => [id, "f"]));

  await choose(page, "close-topics");
  await confirm(page, "Close 2 topics");
  expect(column(ids, "closed")).toEqual(sorted(ids).map((id) => [id, id === c ? "f" : "t"]));
  await expect(row(page, a)).toHaveClass(/(^| )closed( |$)/);
  await expect(row(page, c)).not.toHaveClass(/(^| )closed( |$)/);

  await page.locator(TOGGLE).click();
  await select(page, [a, b]);
  await choose(page, "archive-topics");
  await expect(page.locator(`${MODAL} .d-modal__title-text`)).toHaveText("Archive topics");
  await confirm(page, "Archive 2 topics");
  expect(column(ids, "archived")).toEqual(sorted(ids).map((id) => [id, id === c ? "f" : "t"]));

  // Unlist one; then Relist is offered for it, Unlist only with a
  // listed topic selected too.
  await page.locator(TOGGLE).click();
  await select(page, [b]);
  await choose(page, "unlist-topics");
  await expect(page.locator(`${MODAL} .d-modal__title-text`)).toHaveText("Unlist topics");
  await confirm(page, "Unlist 1 topic");
  expect(column(ids, "visible")).toEqual(sorted(ids).map((id) => [id, id === b ? "f" : "t"]));

  await expect(row(page, b).locator(".topic-status.--invisible")).toHaveCount(1);
  await page.locator(TOGGLE).click();
  await select(page, [b]);
  let offered = await items(page);
  expect(offered).toContain("relist-topics");
  expect(offered).not.toContain("unlist-topics");
  await page.keyboard.press("Escape");
  await expect(page.locator(MENU)).toHaveCount(0);
  await select(page, [c]);
  offered = await items(page);
  expect(offered).toContain("relist-topics");
  expect(offered).toContain("unlist-topics");
  await page.keyboard.press("Escape");
  await expect(page.locator(MENU)).toHaveCount(0);
  await box(page, c).click();
  await expect(page.locator(COUNT)).toHaveText("1 selected");
  await choose(page, "relist-topics");
  await expect(page.locator(`${MODAL} .d-modal__title-text`)).toHaveText("Relist topics");
  await expect(page.locator(`${MODAL} .topic-bulk-actions-modal__description`)).toHaveCount(0);
  await confirm(page, "Relist 1 topic");
  expect(column(ids, "visible")).toEqual(sorted(ids).map((id) => [id, "t"]));
});

test("an admin deletes the selected topics, and a refusal is listed in the modal", async ({ page, context, session }) => {
  const admin = await session("admin");
  const cat = await category(admin);
  const ids = await topicsIn(await session("user1"), cat, 3);
  const [a, b, c] = ids;
  const [own] = await topicsIn(admin, cat, 1, "An admin's own topic");
  await login(context, "admin");

  await enter(page, cat.path, [...ids, own]);
  await select(page, [a, c]);
  await choose(page, "delete-topics");
  await expect(page.locator(`${MODAL} .d-modal__title-text`)).toHaveText("Delete topics");
  await confirm(page, "Delete 2 topics");
  expect(column([...ids, own], "deleted_at IS NOT NULL")).toEqual(
    sorted([...ids, own]).map((id) => [id, id === a || id === c ? "t" : "f"]),
  );

  // The port's endpoint refuses deleting one's own topic: the modal
  // lists the failure (_showErrors), and closing it leaves the mode.
  await page.locator(TOGGLE).click();
  await select(page, [own]);
  await choose(page, "delete-topics");
  await page.locator(`${MODAL} #bulk-topics-confirm`).click();
  const errors = page.locator(`${MODAL} .topic-bulk-actions-modal__errors`);
  await expect(errors.locator("p")).toHaveText("1 topic could not be updated:");
  await expect(errors.locator("li")).toHaveText(/^500 .*\(1 topic\)$/);
  await page.locator(`${MODAL} #bulk-topics-close`).click();
  await expect(page.locator(MODAL)).toHaveCount(0);
  await expect(page.locator("#list-area input.bulk-select")).toHaveCount(0);
  expect(column([own, b], "deleted_at IS NOT NULL")).toEqual(sorted([own, b]).map((id) => [id, "f"]));
});

test("an admin moves the selected topics to a category and sets their notification level", async ({
  page,
  context,
  session,
}) => {
  const admin = await session("admin");
  const cat = await category(admin);
  const target = await category(admin, "Bulk target");
  const ids = await topicsIn(await session("user1"), cat, 3);
  const [a, b, c] = ids;
  await login(context, "admin");

  await enter(page, cat.path, ids);
  await select(page, [a, b]);
  await choose(page, "update-category");
  await expect(page.locator(`${MODAL} .d-modal__title-text`)).toHaveText("Update category");
  await expect(page.locator(`${MODAL} .topic-bulk-actions-modal__description`)).toHaveText(
    "Move all selected topics to the following category:",
  );
  await expect(page.locator(`${MODAL} label[for="topic-bulk-action-options__notify"]`)).toHaveText(
    "Notify users of this change",
  );
  await page.locator(`${MODAL} details.category-chooser > summary`).click();
  await page.locator(`${MODAL} details.category-chooser .category-row[data-value="${target.id}"]`).click();
  await confirm(page, "Update 2 topics");
  expect(column(ids, "category_id")).toEqual(
    sorted(ids).map((id) => [id, String(id === c ? cat.id : target.id)]),
  );
  await expect(row(page, a)).toHaveCount(0);
  await expect(row(page, c)).toBeVisible();

  // Update Notifications: Confirm waits for a level.
  await page.locator(TOGGLE).click();
  await select(page, [c]);
  await choose(page, "update-notifications");
  await expect(page.locator(`${MODAL} .d-modal__title-text`)).toHaveText("Update notifications");
  await expect(page.locator(`${MODAL} .bulk-notification-list .notification-level-radio strong`)).toHaveText([
    "Watching",
    "Tracking",
    "Normal",
    "Muted",
  ]);
  await expect(page.locator(`${MODAL} #bulk-topics-confirm`)).toBeDisabled();
  await page.locator(`${MODAL} input[name="notification_level"][value="3"]`).check();
  await confirm(page, "Update 1 topic");
  const levels = sql(
    `SELECT topic_id, notification_level FROM topic_users WHERE user_id = :'p1' AND topic_id IN (${ids.join(",")}) AND notification_level = 3`,
    userId("admin"),
  ).map((r) => Number(r[0]));
  expect(levels).toEqual([c]);
});

test("an admin marks the selected topics unread and resets their bump dates", async ({ page, context, session }) => {
  const admin = await session("admin");
  const cat = await category(admin);
  const ids = await topicsIn(await session("user1"), cat, 2);
  const [a, b] = ids;
  await login(context, "admin");
  for (const id of ids) {
    await api(context, "POST", "/topics/timings", { topic_id: String(id), topic_time: "1000", "timings[1]": "1000" });
  }
  const timings = () =>
    sql(
      `SELECT topic_id FROM post_timings WHERE user_id = :'p1' AND topic_id IN (${ids.join(",")}) ORDER BY topic_id`,
      userId("admin"),
    ).map((r) => Number(r[0]));
  expect(timings()).toEqual(sorted(ids));

  // Mark Unread (defer) asks nothing.
  await enter(page, cat.path, ids);
  await select(page, [a]);
  await choose(page, "defer");
  await completed(page);
  expect(timings()).toEqual([b]);

  sql(`UPDATE topics SET bumped_at = bumped_at + interval '1 day' WHERE id IN (${ids.join(",")})`);
  await page.goto(cat.path);
  await expect(row(page, b)).toBeVisible();
  await page.locator(TOGGLE).click();
  await select(page, [b]);
  await choose(page, "reset-bump-dates");
  await expect(page.locator(`${MODAL} .d-modal__title-text`)).toHaveText("Reset bump dates");
  await confirm(page, "Update 1 topic");
  expect(column(ids, "bumped_at > now()")).toEqual(sorted(ids).map((id) => [id, id === a ? "t" : "f"]));
});

test("what is not ported says so by name", async ({ page, context, session }) => {
  const admin = await session("admin");
  const cat = await category(admin);
  const ids = await topicsIn(await session("user1"), cat, 1);
  await login(context, "admin");
  await enter(page, cat.path, ids);
  await select(page, ids);
  const config = await page.evaluate(() => JSON.parse(document.getElementById("topic-bulk-select").textContent));
  await choose(page, "pin-topics");
  await expect(page.locator("#page-error")).toHaveText(
    "Not ported yet: the bulk actions' Pin (BulkPinOptions, GET /topics/feature_stats).",
  );
  await expect(page.locator(MODAL)).toHaveCount(0);
  if (config.manage_tags) {
    await choose(page, "manage-tags");
    await expect(page.locator("#page-error")).toHaveText(
      "Not ported yet: the bulk actions' Manage tags (ManageTagsForm and its tag choosers).",
    );
  }
  // Nothing was sent, the mode stays.
  await expect(page.locator(COUNT)).toHaveText("1 selected");
});

test("a row loaded later gets its checkbox", async ({ page, context, session }) => {
  const admin = await session("admin");
  const cat = await category(admin);
  const ids = await topicsIn(await session("user1"), cat, 3);
  await login(context, "admin");
  // The next page waits until the mode is on.
  let release;
  const gate = new Promise((resolve) => (release = resolve));
  await page.route(
    (url) => url.searchParams.get("page") === "1",
    async (route) => {
      await gate;
      await route.continue();
    },
  );
  await page.goto(`${cat.path}?per_page=2`);
  const rows = page.locator("#list-area tbody.topic-list-body > tr[data-topic-id]");
  await expect(rows).toHaveCount(2);
  await page.locator(TOGGLE).click();
  await expect(page.locator("#list-area input.bulk-select")).toHaveCount(2);
  const first = Number(await rows.first().getAttribute("data-topic-id"));
  await box(page, first).click();
  release();
  await expect(row(page, ids[0])).toBeVisible();
  const total = await rows.count();
  expect(total).toBeGreaterThan(2);
  await expect(page.locator("#list-area tr.bulk-selecting input.bulk-select")).toHaveCount(total);
  await expect(page.locator("#list-area input.bulk-select:checked")).toHaveCount(1);
  await expect(box(page, first)).toBeChecked();
  // The late row selects like the others.
  await box(page, ids[0]).click();
  await expect(page.locator(COUNT)).toHaveText("2 selected");
});

test("a member on the new list gets the toggle, the subset toggle hides in the mode, and Dismiss takes the selected topics", async ({
  page,
  context,
  session,
}) => {
  const admin = await session("admin");
  const cat = await category(admin);
  const ids = await topicsIn(await session("user1"), cat, 3);
  const [a, b, c] = ids;
  const reader = member();
  await login(context, reader.name);

  // No toggle on the category's latest list.
  await page.goto(cat.path);
  await expect(row(page, a)).toBeVisible();
  await expect(page.locator("button.bulk-select")).toHaveCount(0);

  const path = `${cat.path}/l/new`;
  await page.goto(path);
  await expect(row(page, a)).toBeVisible();
  await expect(page.locator(SUBSET)).toBeVisible();
  const dismiss = page.locator("#dismiss-new-top .d-button-label");
  await expect(dismiss).toHaveText("Dismiss all");

  await page.locator(TOGGLE).click();
  await expect(page.locator(TABLE)).toHaveClass("topic-list sticky-header bulk-select-enabled");
  await expect(page.locator(SUBSET)).toBeHidden();
  // Off again it is back.
  await page.locator("#list-area thead th.bulk-select > button").click();
  await expect(page.locator(SUBSET)).toBeVisible();
  await page.locator(TOGGLE).click();
  await expect(page.locator(SUBSET)).toBeHidden();

  // A member has no Bulk Actions menu: the dismiss button counts.
  await select(page, [a, b]);
  await expect(page.locator(TRIGGER)).toHaveCount(0);
  await expect(page.locator(COUNT)).toHaveCount(0);
  await expect(dismiss).toHaveText("Dismiss (2)");
  await box(page, b).click();
  await expect(dismiss).toHaveText("Dismiss (1)");
  await box(page, b).click();
  await expect(dismiss).toHaveText("Dismiss (2)");

  const sent = page.waitForRequest((r) => r.method() === "PUT" && r.url().endsWith("/topics/reset-new"));
  await page.locator("#dismiss-new-top").click();
  const body = new URLSearchParams((await sent).postData());
  expect(body.getAll("topic_ids[]")).toEqual([String(a), String(b)]);
  expect(body.get("category_id")).toBe(String(cat.id));

  // The list drawn again: out of the mode, the dismissed topics gone.
  await expect(row(page, a)).toHaveCount(0);
  await expect(row(page, b)).toHaveCount(0);
  await expect(row(page, c)).toBeVisible();
  await expect(page.locator(TABLE)).toHaveClass("topic-list");
  await expect(page.locator("#list-area input.bulk-select")).toHaveCount(0);
  await expect(page.locator(SUBSET)).toBeVisible();
  await expect(page.locator("#dismiss-new-top .d-button-label")).toHaveText("Dismiss all");
  const dismissed = sql(
    `SELECT topic_id FROM dismissed_topic_users WHERE user_id = :'p1' AND topic_id IN (${ids.join(",")}) ORDER BY topic_id`,
    reader.id,
  ).map((r) => Number(r[0]));
  expect(dismissed).toEqual(sorted([a, b]));
});

test("a member on the unread list dismisses the selected topic's posts", async ({ page, context, session }) => {
  const admin = await session("admin");
  const author = await session("user1");
  const cat = await category(admin);
  const ids = await topicsIn(author, cat, 2);
  const [a, b] = ids;
  const reader = member();
  await login(context, reader.name);
  // Both read and tracked, then replied to: unread.
  for (const id of ids) {
    await api(context, "POST", "/topics/timings", { topic_id: String(id), topic_time: "1000", "timings[1]": "1000" });
    await api(context, "POST", `/t/${id}/notifications`, { notification_level: "2" });
    await api(author, "POST", "/posts.json", {
      topic_id: String(id),
      raw: `${label("A reply")} for the member tracking this topic to find unread.`,
    });
  }
  const read = () =>
    sql(
      `SELECT topic_id, last_read_post_number FROM topic_users WHERE user_id = :'p1' AND topic_id IN (${ids.join(",")}) ORDER BY topic_id`,
      reader.id,
    ).map((r) => [Number(r[0]), Number(r[1])]);
  expect(read()).toEqual(sorted(ids).map((id) => [id, 1]));

  await page.goto(`${cat.path}/l/unread`);
  await expect(row(page, a)).toBeVisible();
  await expect(row(page, b)).toBeVisible();
  const dismiss = page.locator("#dismiss-topics-top .d-button-label");
  await expect(dismiss).toHaveText("Dismiss…");
  await page.locator(TOGGLE).click();
  await select(page, [a]);
  await expect(dismiss).toHaveText("Dismiss (1)…");
  await expect(page.locator(TRIGGER)).toHaveCount(0);

  await page.locator("#dismiss-topics-top").click();
  await expect(page.locator(".d-modal.dismiss-read-modal .d-modal__title-text")).toHaveText("Dismiss 1 unread");
  const sent = page.waitForRequest((r) => r.method() === "PUT" && r.url().endsWith("/topics/bulk"));
  await page.locator("#dismiss-read-confirm").click();
  const body = new URLSearchParams((await sent).postData());
  expect([...body.keys()]).toEqual(["topic_ids[]", "operation[type]", "tracked"]);
  expect(body.getAll("topic_ids[]")).toEqual([String(a)]);
  expect(body.get("operation[type]")).toBe("dismiss_posts");

  await expect(row(page, a)).toHaveCount(0);
  await expect(row(page, b)).toBeVisible();
  await expect(page.locator("#list-area input.bulk-select")).toHaveCount(0);
  expect(read()).toEqual(sorted(ids).map((id) => [id, id === a ? 2 : 1]));
});

test("a topic manager's menu on the new list offers Dismiss for the selected topics", async ({ page, context, session }) => {
  const admin = await session("admin");
  const cat = await category(admin);
  const ids = await topicsIn(await session("user1"), cat, 2);
  const [a, b] = ids;
  await login(context, "admin");
  await enter(page, `${cat.path}/l/new`, ids);
  await select(page, [a]);
  const offered = await items(page);
  expect(offered[0]).toBe("dismiss-new");
  expect(offered).not.toContain("dismiss-unread");
  await expect(page.locator(`${MENU} button.dismiss-new .d-button-label`)).toHaveText("Dismiss");
  const sent = page.waitForRequest((r) => r.method() === "PUT" && r.url().endsWith("/topics/reset-new"));
  await page.locator(`${MENU} button.dismiss-new`).click();
  expect(new URLSearchParams((await sent).postData()).getAll("topic_ids[]")).toEqual([String(a)]);
  await expect(row(page, a)).toHaveCount(0);
  await expect(row(page, b)).toBeVisible();
  await expect(page.locator("#list-area input.bulk-select")).toHaveCount(0);
  const dismissed = sql(
    `SELECT topic_id FROM dismissed_topic_users WHERE user_id = :'p1' AND topic_id IN (${ids.join(",")})`,
    userId("admin"),
  ).map((r) => Number(r[0]));
  expect(dismissed).toEqual([a]);
});
