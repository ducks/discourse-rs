// A tag list's New Topic and category drop (static/js/composer.js and
// navigation-bar.js; Ember's DiscoveryListController#createTopic and
// CategoryDrop with a tag): the composer opens with the list's tags (and
// the category of a tag's list in one), but for the staff-only ones a
// member may not use, and the drop's rows keep the tag. Each test has
// its own tags and topics, and a member of its own, who may tag topics as
// user2 does (tag_topic_allowed_groups through their groups).
const { test, expect, login, sql, label, member } = require("./helpers");

const control = (page) => page.locator("#reply-control");
const selection = (page) => control(page).locator("details.mini-tag-chooser summary .formatted-selection");
const categoryDrop = (page) => page.locator(".category-breadcrumb details.category-drop").first();

function tagName(suffix) {
  return `tc${Math.floor(Math.random() * 1e9).toString(36)}${suffix}`;
}

// A topic by the admin in General carrying the tags (created with it).
// Returns its id.
async function tagged(admin, what, tags) {
  const csrf = await admin.request.get("/session/csrf.json", {
    headers: { "X-Requested-With": "XMLHttpRequest" },
  });
  const params = new URLSearchParams({ title: label(what), raw: label(`${what} body`), category: "4" });
  tags.forEach((t) => params.append("tags[]", t));
  const res = await admin.request.post("/posts.json", {
    headers: {
      "X-CSRF-Token": (await csrf.json()).csrf,
      "X-Requested-With": "XMLHttpRequest",
      Accept: "application/json",
      "Content-Type": "application/x-www-form-urlencoded",
    },
    data: params.toString(),
  });
  const body = await res.text();
  expect(res.ok(), `POST /posts.json: ${res.status()} ${body.slice(0, 300)}`).toBeTruthy();
  return JSON.parse(body).topic_id;
}

// Two tags, a topic with both and one with the first alone.
async function setup(session) {
  const admin = await session("admin");
  const [a, b] = [tagName("a"), tagName("b")];
  const both = await tagged(admin, "Tag chrome both", [a, b]);
  const one = await tagged(admin, "Tag chrome one", [a]);
  const id = (name) => sql("SELECT id FROM tags WHERE name = :'p1'", name)[0][0];
  return { a, b, ids: { a: id(a), b: id(b) }, both, one };
}

function row(page, id) {
  return page.locator(`.topic-list a.raw-topic-link[data-topic-id="${id}"]`);
}

// The page's New Topic, once the composer has settled open.
async function newTopic(page) {
  await page.locator("#create-topic").click();
  await expect(control(page)).toHaveClass(/\bopen\b/);
  await expect(control(page).locator(".d-editor-input")).toBeEnabled();
}

// Fills and creates the open composer's topic. Returns its title.
async function create(page, what) {
  const title = label(what);
  await page.locator("#reply-title").fill(title);
  await control(page).locator(".d-editor-input").fill(`The first post of ${title}, started from a tag's list.`);
  await control(page).locator(".save-or-cancel .create").click();
  await expect(page).toHaveURL(/\/t\/[^/]+\/\d+/);
  await expect(page.locator("#topic-title h1")).toContainText(title);
  return title;
}

function created(title) {
  return sql(
    "SELECT t.category_id, COALESCE(string_agg(g.name, ',' ORDER BY g.name), '') FROM topics t \
     LEFT JOIN topic_tags tt ON tt.topic_id = t.id LEFT JOIN tags g ON g.id = tt.tag_id \
     WHERE t.title = :'p1' GROUP BY t.id",
    title,
  );
}

// The category drop opened, its rows loaded.
async function openCategories(page) {
  const drop = categoryDrop(page);
  await drop.locator("summary").click();
  await expect(drop.locator("a.select-kit-row").first()).toBeVisible();
  return drop;
}

test("New Topic on a tag's list starts with the tag", async ({ page, context, session }) => {
  const s = await setup(session);
  await login(context, member().name);
  await page.goto(`/tag/${s.a}/${s.ids.a}`);
  await expect(page.locator("#create-topic")).toHaveAttribute("data-tags", s.a);
  await expect(control(page).locator("details.mini-tag-chooser")).toHaveCount(1);
  await newTopic(page);
  await expect(selection(page)).toHaveText(s.a);
  // No category of the list's: the composer's default.
  const chooser = control(page).locator("details.category-chooser");
  await chooser.locator("summary").click();
  await chooser.locator(".category-row[data-value='4']").click();
  const title = await create(page, "On a tag list");
  expect(created(title)).toEqual([["4", s.a]]);
});

test("New Topic on a tag's list in a category starts with both", async ({ page, context, session }) => {
  const s = await setup(session);
  await login(context, member().name);
  await page.goto(`/tags/c/general/4/${s.a}/${s.ids.a}`);
  const button = page.locator("#create-topic");
  await expect(button).toHaveAttribute("data-tags", s.a);
  await expect(button).toHaveAttribute("data-category", "4");
  await newTopic(page);
  await expect(selection(page)).toHaveText(s.a);
  await expect(control(page).locator("details.category-chooser summary .select-kit-selected-name")).toContainText("General");
  await expect(control(page).locator("details.category-chooser summary")).toHaveAttribute("data-value", "4");
  const title = await create(page, "On a tag in a category");
  expect(created(title)).toEqual([["4", s.a]]);
});

test("New Topic on an intersection starts with all its tags", async ({ page, context, session }) => {
  const s = await setup(session);
  await login(context, member().name);
  await page.goto(`/tags/intersection/${s.a}/${s.b}`);
  await expect(page.locator("#create-topic")).toHaveAttribute("data-tags", `${s.a},${s.b}`);
  await newTopic(page);
  // In the order /tags/list.json answers in (composer.filterTags).
  await expect(selection(page)).toHaveText(new RegExp(`^(${s.a}, ${s.b}|${s.b}, ${s.a})$`));
  const chooser = control(page).locator("details.category-chooser");
  await chooser.locator("summary").click();
  await chooser.locator(".category-row[data-value='4']").click();
  const title = await create(page, "On an intersection");
  expect(created(title)).toEqual([["4", [s.a, s.b].sort().join(",")]]);
});

test("a staff-only tag and the untagged list prefill nothing", async ({ page, context, session }) => {
  const s = await setup(session);
  // composer.filterTags: `b` is staff's to use, everyone's to see.
  sql(
    "WITH g AS (INSERT INTO tag_groups (name, created_at, updated_at) VALUES (:'p1', now(), now()) RETURNING id), \
       m AS (INSERT INTO tag_group_memberships (tag_id, tag_group_id, created_at, updated_at) SELECT :'p2', id, now(), now() FROM g) \
     INSERT INTO tag_group_permissions (tag_group_id, group_id, permission_type, created_at, updated_at) \
     SELECT g.id, p.group_id, p.permission_type, now(), now() FROM g, (VALUES (0, 3), (3, 1)) AS p(group_id, permission_type)",
    `staff only ${s.b}`,
    s.ids.b,
  );
  await login(context, member().name);

  // Its own list: the button names it, the composer drops it.
  await page.goto(`/tag/${s.b}/${s.ids.b}`);
  await expect(page.locator("#create-topic")).toHaveAttribute("data-tags", s.b);
  const listed = page.waitForResponse((r) => r.url().includes("/tags/list.json"));
  await page.locator("#create-topic").click();
  expect((await listed).ok()).toBeTruthy();
  await expect(control(page)).toHaveClass(/\bopen\b/);
  await expect(control(page).locator(".d-editor-input")).toBeEnabled();
  await expect(control(page).locator("details.mini-tag-chooser")).not.toHaveClass(/has-selection/);
  await expect(selection(page)).not.toContainText(s.b);

  // With another tag: that one alone.
  await page.goto(`/tags/intersection/${s.a}/${s.b}`);
  await expect(page.locator("#create-topic")).toHaveAttribute("data-tags", `${s.a},${s.b}`);
  await newTopic(page);
  await expect(selection(page)).toHaveText(s.a);

  // The untagged list has its navigation and no tag to start with.
  await page.goto("/tag/none");
  await expect(page.locator(".category-breadcrumb details.tag-drop.tag-none summary")).toHaveAttribute("data-name", "No tags");
  await expect(page.locator("#navigation-bar a.active")).toHaveAttribute("href", "/tag/none/l/latest");
  const button = page.locator("#create-topic");
  await expect(button).toBeVisible();
  expect(await button.getAttribute("data-tags")).toBeNull();
  await newTopic(page);
  await expect(selection(page)).not.toContainText(s.a);
  await expect(control(page).locator("details.mini-tag-chooser")).not.toHaveClass(/has-selection/);
});

test("the category drop on a tag's list keeps the tag", async ({ page, session }) => {
  const s = await setup(session);
  await page.goto(`/tag/${s.a}/${s.ids.a}`);
  await expect(row(page, s.both)).toBeVisible();

  let drop = await openCategories(page);
  await expect(drop.locator("a.select-kit-row", { hasText: /^General$/ })).toHaveAttribute(
    "href",
    `/tags/c/general/4/${s.a}/${s.ids.a}`,
  );
  await drop.locator("a.select-kit-row", { hasText: /^General$/ }).click();
  await expect(page).toHaveURL(new RegExp(`/tags/c/general/4/${s.a}/${s.ids.a}$`));
  await expect(categoryDrop(page).locator("summary")).toHaveAttribute("data-value", "4");
  await expect(page.locator(".category-breadcrumb details.tag-drop summary")).toHaveAttribute("data-name", s.a);
  await expect(row(page, s.both)).toBeVisible();
  await expect(row(page, s.one)).toBeVisible();

  // And back to the tag's own list.
  drop = await openCategories(page);
  await drop.locator("a.select-kit-row", { hasText: /^All categories$/ }).click();
  await expect(page).toHaveURL(new RegExp(`/tag/${s.a}/${s.ids.a}$`));
  await expect(categoryDrop(page).locator("summary")).toHaveAttribute("data-value", "");
  await expect(page.locator(".category-breadcrumb details.tag-drop summary")).toHaveAttribute("data-name", s.a);
  await expect(row(page, s.one)).toBeVisible();
});

test("the category drop keeps `none` and an intersection's main tag", async ({ page, session }) => {
  const s = await setup(session);
  await page.goto("/tag/none");
  let drop = await openCategories(page);
  await drop.locator("a.select-kit-row", { hasText: /^General$/ }).click();
  await expect(page).toHaveURL(/\/tags\/c\/general\/4\/none$/);
  await expect(page.locator(".category-breadcrumb details.tag-drop.tag-none")).toHaveCount(1);
  await expect(categoryDrop(page).locator("summary")).toHaveAttribute("data-value", "4");
  drop = await openCategories(page);
  await drop.locator("a.select-kit-row", { hasText: /^All categories$/ }).click();
  await expect(page).toHaveURL(/\/tag\/none$/);
  await expect(page.locator(".category-breadcrumb details.tag-drop.tag-none")).toHaveCount(1);
  await expect(categoryDrop(page).locator("summary")).toHaveAttribute("data-value", "");

  // CategoryDrop's `@tag` on an intersection is its main tag alone.
  await page.goto(`/tags/intersection/${s.a}/${s.b}`);
  await expect(row(page, s.both)).toBeVisible();
  drop = await openCategories(page);
  await drop.locator("a.select-kit-row", { hasText: /^General$/ }).click();
  await expect(page).toHaveURL(new RegExp(`/tags/c/general/4/${s.a}/${s.ids.a}$`));
  await expect(page.locator(".category-breadcrumb details.tag-drop summary")).toHaveAttribute("data-name", s.a);
  await expect(row(page, s.both)).toBeVisible();
  await expect(row(page, s.one)).toBeVisible();
});
