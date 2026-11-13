// The topic lists' bulk select mode (topic-list-bulk-select.js, with
// topic-dismiss.js for the dismissals): the toggle turns it on and off,
// rows are selected by their checkbox, by a click on the row and by a
// shift range, a topic manager's Bulk Actions menu offers what Ember's
// rules allow and its actions change exactly the selected topics
// (Manage tags' form and Pin's options among them), a member on the new
// and unread lists dismisses what they selected, and what is not ported
// says so. Each change is checked in the database.
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

// A tag of this test's own: { id, name }.
function newTag(what) {
  const name = label(what).replace(/ /g, "-").slice(0, 20);
  const [[id]] = sql(
    "INSERT INTO tags (name, slug, created_at, updated_at) VALUES (:'p1', :'p1', now(), now()) RETURNING id",
    name,
  );
  return { id: Number(id), name };
}

function tagTopic(topicId, ...tags) {
  for (const tag of tags) {
    sql(
      "INSERT INTO topic_tags (topic_id, tag_id, created_at, updated_at) VALUES (:'p1', :'p2', now(), now())",
      topicId,
      tag.id,
    );
  }
}

// The topics' tags as [topic id, tag id], sorted.
function tagsOf(ids) {
  return sql(
    `SELECT topic_id, tag_id FROM topic_tags WHERE topic_id IN (${ids.join(",")}) ORDER BY topic_id, tag_id`,
  ).map((r) => [Number(r[0]), Number(r[1])]);
}

function pairs(list) {
  return [...list].sort((x, y) => x[0] - y[0] || x[1] - y[1]);
}

// A ManageTagsForm field's TagChooser.
function chooserOf(page, field) {
  return page.locator(`${MODAL} #control-${field} details.tag-chooser`);
}

// Picks the tag in the field's chooser; the search it sent.
async function pickTag(page, field, tag) {
  const chooser = chooserOf(page, field);
  await chooser.locator("summary").click();
  const searched = page.waitForRequest(
    (r) => r.url().includes("/tags/filter/search") && new URL(r.url()).searchParams.get("q") === tag.name,
  );
  await chooser.locator(".filter-input").fill(tag.name);
  const request = await searched;
  await chooser.locator(`.select-kit-row.tag-chooser-row[data-name="${tag.name}"]`).click();
  await expect(chooser.locator("summary .formatted-selection")).toContainText(tag.name);
  if (await chooser.evaluate((d) => d.open)) {
    await page.locator(`${MODAL} .d-modal__title-text`).click();
  }
  await expect(chooser).not.toHaveAttribute("open", "");
  return new URL(request.url()).searchParams;
}

function bulkRequest(page) {
  return page.waitForRequest((r) => r.url().endsWith("/topics/bulk") && r.method() === "PUT");
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

test("an admin adds, removes and replaces the selected topics' tags", async ({ page, context, session }) => {
  const admin = await session("admin");
  const cat = await category(admin);
  const ids = await topicsIn(await session("user1"), cat, 3);
  const [a, b, c] = ids;
  const [kept, removed, added, replacement] = ["kept", "gone", "new", "swap"].map((what) => newTag(what));
  tagTopic(a, kept, removed);
  tagTopic(b, kept);
  tagTopic(c, kept, removed);
  await login(context, "admin");

  await enter(page, cat.path, ids);
  await select(page, [a, b]);
  expect(await items(page)).toContain("manage-tags");
  await page.keyboard.press("Escape");
  await expect(page.locator(MENU)).toHaveCount(0);
  await choose(page, "manage-tags");
  await expect(page.locator(`${MODAL} .d-modal__title-text`)).toHaveText("Manage tags");
  await expect(page.locator(`${MODAL} form.manage-tags-form .form-kit__container-title`)).toHaveText([
    "Remove tags",
    "Remove all tags",
    "Add tags",
    "Replace tags",
  ]);
  await expect(page.locator(`${MODAL} label[for="topic-bulk-action-options__notify"]`)).toHaveText(
    "Notify users of this change",
  );
  // Nothing set: Confirm waits (setSubmitDisabled).
  const confirmButton = page.locator(`${MODAL} #bulk-topics-confirm`);
  await expect(confirmButton).toHaveText("Apply to 2 topics");
  await expect(confirmButton).toBeDisabled();

  // Add tags searches the topics' one category, for its input; a tag
  // that does not exist is not offered (no allowCreate).
  const add = chooserOf(page, "add_tags");
  await add.locator("summary").click();
  await expect(add.locator(".filter-input")).toHaveAttribute("placeholder", "Optional tags…");
  await add.locator(".filter-input").fill(`${added.name}zz`);
  await expect(add.locator(".no-content")).toBeVisible();
  await expect(add.locator(".select-kit-row")).toHaveCount(0);
  await page.locator(`${MODAL} .d-modal__title-text`).click();
  let search = await pickTag(page, "add_tags", added);
  expect([search.get("categoryId"), search.get("filterForInput")]).toEqual([String(cat.id), "true"]);
  await expect(confirmButton).toBeEnabled();

  // Remove tags searches every tag.
  search = await pickTag(page, "remove_tags", removed);
  expect([search.get("categoryId"), search.get("filterForInput")]).toEqual([null, null]);

  // A replacement: one tag each side, the other side's left out.
  search = await pickTag(page, "replace_rows-0-from", kept);
  expect([search.get("categoryId"), search.get("filterForInput")]).toEqual([null, null]);
  const to = chooserOf(page, "replace_rows-0-to");
  await to.locator("summary").click();
  await expect(to.locator(".filter-input")).toHaveAttribute("placeholder", "Replacement tag");
  const blocked = page.waitForResponse(
    (r) => r.url().includes("/tags/filter/search") && new URL(r.url()).searchParams.get("q") === kept.name,
  );
  await to.locator(".filter-input").fill(kept.name);
  expect(new URL((await blocked).url()).searchParams.getAll("selected_tag_ids[]")).toEqual([String(kept.id)]);
  await expect(to.locator(`.select-kit-row[data-name="${kept.name}"]`)).toHaveCount(0);
  await page.locator(`${MODAL} .d-modal__title-text`).click();
  // A second choice takes the first one's place (maximum 1).
  await pickTag(page, "replace_rows-0-to", added);
  search = await pickTag(page, "replace_rows-0-to", replacement);
  expect(search.get("categoryId")).toBe(String(cat.id));
  await expect(to.locator("summary")).toHaveAttribute("data-name", replacement.name);

  const sent = bulkRequest(page);
  await confirm(page, "Apply to 2 topics");
  const request = await sent;
  expect(request.headers()["content-type"]).toBe("application/json");
  expect(request.postDataJSON()).toEqual({
    topic_ids: [a, b],
    operation: {
      type: "manage_tags",
      add_tag_ids: [added.id],
      remove_tag_ids: [removed.id],
      remove_all_tags: false,
      replace_tags: [{ from_tag_id: kept.id, to_tag_id: replacement.id }],
      silent: true,
    },
  });
  expect(tagsOf(ids)).toEqual(
    pairs([
      [a, added.id],
      [a, replacement.id],
      [b, added.id],
      [b, replacement.id],
      [c, kept.id],
      [c, removed.id],
    ]),
  );
});

test("manage tags validates its replacements and removes all tags", async ({ page, context, session }) => {
  const admin = await session("admin");
  const cat = await category(admin);
  const ids = await topicsIn(await session("user1"), cat, 2);
  const [a, b] = ids;
  const [one, two] = ["one", "two"].map((what) => newTag(what));
  tagTopic(a, one, two);
  tagTopic(b, one);
  await login(context, "admin");

  await enter(page, cat.path, ids);
  await select(page, [a]);
  await choose(page, "manage-tags");
  const form = page.locator(`${MODAL} form.manage-tags-form`);
  const confirmButton = page.locator(`${MODAL} #bulk-topics-confirm`);
  await expect(confirmButton).toHaveText("Apply to 1 topic");

  // A replacement without its other tag is refused by the form.
  await pickTag(page, "replace_rows-0-from", one);
  await expect(confirmButton).toBeEnabled();
  let requests = 0;
  const count = (r) => {
    if (r.url().endsWith("/topics/bulk")) {
      requests += 1;
    }
  };
  page.on("request", count);
  await confirmButton.click();
  await expect(form.locator(".form-kit__errors-summary-title")).toHaveText("Fix this error to continue");
  await expect(form.locator(".form-kit__errors-summary-list li")).toHaveText("Replace tags: A replacement tag is required");
  await expect(form.locator("#control-replace_rows-0-to")).toHaveClass(/(^| )has-error( |$)/);
  await expect(form.locator("#control-replace_rows-0-to .form-kit__errors")).toHaveText("A replacement tag is required");
  await expect(form.locator("#control-replace_rows-0-from .form-kit__errors")).toHaveCount(0);

  // The row removed takes its errors along and leaves nothing to apply;
  // Add replacement brings an empty row.
  await form.locator(".manage-tags-form__replace-row-remove").click();
  await expect(form.locator(".form-kit__errors-summary")).toHaveCount(0);
  await expect(form.locator(".form-kit__collection")).toHaveCount(0);
  await expect(confirmButton).toBeDisabled();
  await form.locator(".manage-tags-form__replace-row-add").click();
  await expect(form.locator(".form-kit__collection .form-kit__row")).toHaveCount(1);
  await expect(confirmButton).toBeDisabled();
  page.off("request", count);
  expect(requests).toBe(0);
  expect(tagsOf(ids)).toEqual(pairs([[a, one.id], [a, two.id], [b, one.id]]));

  // Remove all tags: the warning in the chooser's place.
  const toggle = form.locator("#control-remove_all_tags .d-toggle-switch__checkbox");
  await expect(toggle).toHaveAttribute("aria-checked", "false");
  await form.locator("#control-remove_all_tags .d-toggle-switch__checkbox-slider").click();
  await expect(form.locator("#control-remove_all_tags .d-toggle-switch__checkbox")).toHaveAttribute("aria-checked", "true");
  await expect(form.locator("#control-remove_tags")).toHaveCount(0);
  await expect(form.locator(".manage-tags-form__warning.alert-error")).toHaveText(
    "All existing tags will be removed from the selected topics. This cannot be undone.",
  );
  await expect(confirmButton).toBeEnabled();
  const sent = bulkRequest(page);
  await confirm(page, "Apply to 1 topic");
  expect((await sent).postDataJSON().operation).toEqual({
    type: "manage_tags",
    add_tag_ids: [],
    remove_tag_ids: [],
    remove_all_tags: true,
    replace_tags: [],
    silent: true,
  });
  expect(tagsOf(ids)).toEqual([[b, one.id]]);
});

test("a tag a category refuses is listed in the modal, for a leader", async ({ page, context, session }) => {
  const admin = await session("admin");
  const home = await category(admin, "Bulk tags home");
  const other = await category(admin, "Bulk tags other");
  const author = await session("user1");
  const [inHome] = await topicsIn(author, home, 1);
  const [inOther] = await topicsIn(author, other, 1);
  const ids = [inHome, inOther];
  const restricted = newTag("home");
  // The tag is the home category's alone, and the other category wants
  // a tag of its topics: the revision left without one is refused there.
  sql(
    "INSERT INTO category_tags (category_id, tag_id, created_at, updated_at) VALUES (:'p1', :'p2', now(), now())",
    home.id,
    restricted.id,
  );
  sql("UPDATE categories SET minimum_required_tags = 1 WHERE id = :'p1'", other.id);
  // A topic manager who is not staff (staff skip the required tags).
  const leader = member();
  sql("UPDATE users SET trust_level = 4 WHERE id = :'p1'", leader.id);
  sql(
    "INSERT INTO group_users (group_id, user_id, created_at, updated_at) SELECT g, :'p1', now(), now() FROM unnest(ARRAY[13, 14]) g \
     WHERE NOT EXISTS (SELECT 1 FROM group_users WHERE group_id = g AND user_id = :'p1')",
    leader.id,
  );
  await login(context, leader.name);

  // Latest has both categories' topics: no category scopes the
  // choosers, and Pin has no category to count.
  await enter(page, "/latest", ids);
  await select(page, ids);
  await choose(page, "pin-topics");
  const sections = page.locator(`${MODAL} form.feature-section`);
  await expect(sections).toHaveCount(2);
  await expect(sections.nth(0).locator(".feature-section__description > p")).toHaveCount(2);
  await expect(sections.nth(1).locator(".feature-section__description > p").first()).toContainText(/pinned globally/);
  await page.locator(`${MODAL} .modal-close`).click();
  await expect(page.locator(MODAL)).toHaveCount(0);

  await choose(page, "manage-tags");
  const search = await pickTag(page, "add_tags", restricted);
  expect([search.get("categoryId"), search.get("filterForInput")]).toEqual([null, null]);
  await page.locator(`${MODAL} #bulk-topics-confirm`).click();
  const errors = page.locator(`${MODAL} .topic-bulk-actions-modal__errors`);
  await expect(errors.locator("p")).toHaveText(["1 topic was updated.", "1 topic could not be updated:"]);
  await expect(errors.locator("li")).toHaveText(`${restricted.name} isn't allowed in the ${other.name} category. (1 topic)`);
  await expect(errors.locator(`li a.discourse-tag[data-tag-name="${restricted.name}"]`)).toHaveCount(1);
  await expect(errors.locator(`li a.badge-category__wrapper .badge-category[data-category-id="${other.id}"]`)).toHaveCount(1);
  await page.locator(`${MODAL} #bulk-topics-close`).click();
  await expect(page.locator(MODAL)).toHaveCount(0);
  await expect(page.locator("#list-area input.bulk-select")).toHaveCount(0);
  expect(tagsOf(ids)).toEqual([[inHome, restricted.id]]);
});

test("an admin pins the selected topics in their category until a date", async ({ page, context, session }) => {
  const admin = await session("admin");
  const cat = await category(admin);
  const ids = await topicsIn(await session("user1"), cat, 3);
  const [a, b, c] = ids;
  await login(context, "admin");

  try {
    await enter(page, cat.path, ids);
    await select(page, [a, b]);
    const counted = page.waitForRequest((r) => r.url().includes("/topics/feature_stats.json"));
    await choose(page, "pin-topics");
    expect(new URL((await counted).url()).search).toBe(`?category_id=${cat.id}`);
    await expect(page.locator(`${MODAL} .d-modal__title-text`)).toHaveText("Feature these topics");
    // showFooter false: the sections' own buttons pin.
    await expect(page.locator(`${MODAL} .d-modal__footer button`)).toHaveCount(0);
    const sections = page.locator(`${MODAL} form.feature-section`);
    await expect(sections).toHaveCount(2);
    const inCategory = sections.nth(0);
    // The category's own pins (its definition topic's), counted.
    const pinned = () =>
      Number(
        sql(
          "SELECT COUNT(*) FROM topics WHERE category_id = :'p1' AND pinned_at IS NOT NULL AND NOT pinned_globally AND visible AND deleted_at IS NULL",
          cat.id,
        )[0][0],
      );
    const counts = (n) =>
      n === 0 ? `There are no topics pinned in ${cat.name}.` : `Topics currently pinned in ${cat.name}: ${n}`;
    const before = pinned();
    const stats = inCategory.locator(".feature-section__description > p").first();
    await expect(stats).toHaveText(counts(before));
    await expect(stats.locator(`a.badge-category__wrapper .badge-category[data-category-id="${cat.id}"]`)).toHaveCount(1);
    await expect(inCategory.locator(".feature-section__description > p").nth(1)).toHaveText(
      "Users can unpin topics individually for themselves.",
    );
    await expect(inCategory.locator(".feature-section__pin-message")).toContainText(
      "Make these topics appear at the top of the topic's category until",
    );
    await expect(inCategory.locator(".form-kit__button")).toHaveText("Pin topics");
    const globally = sections.nth(1);
    await expect(globally.locator(".feature-section__description > p").first()).toContainText(/pinned globally/);
    await expect(globally.locator(".feature-section__pin-message")).toContainText(
      "Make these topics appear at the top of all topic lists until",
    );
    await expect(globally.locator(".form-kit__button")).toHaveText("Pin topics globally");

    // The date is required.
    await inCategory.locator(".form-kit__button").click();
    await expect(inCategory.locator(".form-kit__errors")).toHaveText("A date is required to pin this topic.");
    expect(column(ids, "pinned_at IS NULL")).toEqual(sorted(ids).map((id) => [id, "t"]));

    await inCategory.locator(".future-date-input-selector summary").click();
    await page.locator('.future-date-input-selector-row[data-value="next_month"]').click();
    await expect(inCategory.locator(".form-kit__errors")).toHaveCount(0);
    const sent = bulkRequest(page);
    await inCategory.locator(".form-kit__button").click();
    const body = decodeURIComponent((await sent).postData());
    expect(body).toMatch(
      new RegExp(
        `^topic_ids\\[\\]=${a}&topic_ids\\[\\]=${b}&operation\\[type\\]=pin&operation\\[pinned_globally\\]=false&operation\\[pinned_until\\]=\\d{4}-\\d\\d-\\d\\d\\+\\d\\d:\\d\\d[+-]\\d\\d:\\d\\d&tracked=$`,
      ),
    );
    await completed(page);
    expect(
      sql(
        `SELECT id, pinned_at IS NOT NULL, pinned_globally, COALESCE(pinned_until > now() + interval '1 day', false) FROM topics WHERE id IN (${ids.join(",")}) ORDER BY id`,
      ).map((r) => [Number(r[0]), r[1], r[2], r[3]]),
    ).toEqual(sorted(ids).map((id) => (id === c ? [id, "f", "f", "f"] : [id, "t", "f", "t"])));

    // The category's pins are counted now. Pinning globally sends the
    // other flag: the request is stopped here, as a global pin would
    // reorder every other list, and its failure is listed.
    await page.locator(TOGGLE).click();
    await select(page, [c]);
    await choose(page, "pin-topics");
    expect(pinned()).toBe(before + 2);
    await expect(sections.nth(0).locator(".feature-section__description > p").first()).toHaveText(counts(before + 2));
    await sections.nth(1).locator(".future-date-input-selector summary").click();
    await page.locator('.future-date-input-selector-row[data-value="two_weeks"]').click();
    let stopped = null;
    await page.route("**/topics/bulk", (route) => {
      stopped = decodeURIComponent(route.request().postData());
      return route.abort();
    });
    await sections.nth(1).locator(".form-kit__button").click();
    const dialog = page.locator("#dialog-holder .dialog-footer .btn-primary");
    const failure = page.locator(`${MODAL} .topic-bulk-actions-modal__errors`);
    // Past four global pins a confirmation comes first.
    await expect(dialog.or(failure)).toBeVisible();
    if (await dialog.isVisible()) {
      await dialog.click();
    }
    await expect(failure.locator("p")).toHaveText("1 topic could not be updated:");
    await expect(failure.locator("li")).toHaveText("Sorry, an error has occurred. (1 topic)");
    expect(stopped).toMatch(
      new RegExp(`^topic_ids\\[\\]=${c}&operation\\[type\\]=pin&operation\\[pinned_globally\\]=true&operation\\[pinned_until\\]=\\d{4}-`),
    );
    await page.unroute("**/topics/bulk");
    await page.locator(`${MODAL} #bulk-topics-close`).click();
    await expect(page.locator(MODAL)).toHaveCount(0);
    expect(column([c], "pinned_at IS NULL")).toEqual([[c, "t"]]);
  } finally {
    for (const id of ids) {
      await api(admin, "PUT", `/t/${id}/status`, { status: "pinned", enabled: "false" });
    }
  }
});

test("the nested replies item, where offered, says it is not ported", async ({ page, context, session }) => {
  const admin = await session("admin");
  const cat = await category(admin);
  const ids = await topicsIn(await session("user1"), cat, 1);
  await login(context, "admin");
  await enter(page, cat.path, ids);
  await select(page, ids);
  const config = await page.evaluate(() => JSON.parse(document.getElementById("topic-bulk-select").textContent));
  const offered = await items(page);
  await page.keyboard.press("Escape");
  await expect(page.locator(MENU)).toHaveCount(0);
  expect(offered.includes("enable-nested-replies")).toBe(config.nested_replies);
  if (config.nested_replies) {
    await choose(page, "enable-nested-replies");
    await expect(page.locator("#page-error")).toHaveText(
      "Not ported yet: the bulk actions' Enable nested replies (NestedTopic::Toggle).",
    );
    await expect(page.locator(MODAL)).toHaveCount(0);
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
