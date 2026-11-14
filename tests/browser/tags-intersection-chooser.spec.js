// BreadCrumbs' TagsIntersectionChooser on /tags/intersection/:tag/*others
// (static/js/navigation-bar.js over mini-tag-chooser.js, Ember's
// tags-intersection-chooser): the chooser holds the intersection's tags,
// searches /tags/filter/search as MiniTagChooser does, and each change
// goes to the page of what is left: the wider or narrower intersection,
// the main tag's own list (/tag/:slug/:id) once it is alone, the other
// tag's (/tag/:name) when the main one goes. Each test has its own tags
// and topics.
const { test, expect, login, sql, label, member } = require("./helpers");

const CHOOSER = ".category-breadcrumb details.tags-intersection-chooser";

function tagName(suffix) {
  return `ix${Math.floor(Math.random() * 1e9).toString(36)}${suffix}`;
}

// A topic by the admin carrying the tags (created with it). Returns its id.
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

// Three tags and a topic for each prefix of them: `all` has a, b and c,
// `two` a and b, `one` a alone.
async function setup(session) {
  const admin = await session("admin");
  const [a, b, c] = [tagName("a"), tagName("b"), tagName("c")];
  const all = await tagged(admin, "Intersection of three", [a, b, c]);
  const two = await tagged(admin, "Intersection of two", [a, b]);
  const one = await tagged(admin, "Intersection of one", [a]);
  const id = (name) => sql("SELECT id FROM tags WHERE name = :'p1'", name)[0][0];
  return { a, b, c, ids: { a: id(a), b: id(b), c: id(c) }, all, two, one };
}

function row(page, id) {
  return page.locator(`.topic-list a.raw-topic-link[data-topic-id="${id}"]`);
}

test("a visitor's chooser holds the tags, widens and narrows the intersection", async ({ page, session }) => {
  const s = await setup(session);
  await page.goto(`/tags/intersection/${s.a}/${s.b}`);
  const chooser = page.locator(CHOOSER);
  await expect(chooser.locator(".formatted-selection")).toHaveText(`${s.a}, ${s.b}`);
  // No pills, no heading of the port's own.
  await expect(page.locator("#navigation-bar")).toHaveCount(0);
  await expect(page.locator(".tag-heading")).toHaveCount(0);
  await expect(row(page, s.all)).toBeVisible();
  await expect(row(page, s.two)).toBeVisible();
  await expect(row(page, s.one)).toHaveCount(0);

  // Open: the selected tags as choices, the search as MiniTagChooser's.
  const opened = page.waitForRequest((r) => r.url().includes("/tags/filter/search"));
  await chooser.locator("summary").click();
  const first = new URL((await opened).url()).searchParams;
  expect(first.get("q")).toBe("");
  expect(first.get("filterForInput")).toBe("true");
  expect(first.getAll("selected_tag_ids[]")).toEqual([s.ids.a]);
  await expect(chooser.locator(".selected-content .selected-choice")).toHaveText([s.a, s.b]);

  const searched = page.waitForRequest(
    (r) => r.url().includes("/tags/filter/search") && new URL(r.url()).searchParams.get("q") === s.c,
  );
  await chooser.locator(".filter-input").fill(s.c);
  await searched;
  // The selected tags are never offered.
  await expect(chooser.locator(`.select-kit-row[data-name="${s.a}"]`)).toHaveCount(0);
  await chooser.locator(`.select-kit-row[data-name="${s.c}"]`).click();

  await expect(page).toHaveURL(new RegExp(`/tags/intersection/${s.a}/${s.b}/${s.c}$`));
  await expect(chooser.locator(".formatted-selection")).toHaveText(`${s.a}, ${s.b}, ${s.c}`);
  await expect(row(page, s.all)).toBeVisible();
  await expect(row(page, s.two)).toHaveCount(0);

  // Narrower: the last tag's choice removed.
  await chooser.locator("summary").click();
  await chooser.locator(`.selected-choice[data-name="${s.c}"]`).click();
  await expect(page).toHaveURL(new RegExp(`/tags/intersection/${s.a}/${s.b}$`));
  await expect(chooser.locator(".formatted-selection")).toHaveText(`${s.a}, ${s.b}`);
  await expect(row(page, s.two)).toBeVisible();

  // The main tag alone: its own list, at its canonical address.
  await chooser.locator("summary").click();
  await chooser.locator(`.selected-choice[data-name="${s.b}"]`).click();
  await expect(page).toHaveURL(new RegExp(`/tag/${s.a}/${s.ids.a}$`));
  await expect(page.locator(CHOOSER)).toHaveCount(0);
  await expect(page.locator(".category-breadcrumb details.tag-drop summary")).toHaveAttribute("data-name", s.a);
  await expect(page.locator("#navigation-bar")).toBeVisible();
  await expect(row(page, s.one)).toBeVisible();
});

test("a member removing the main tag goes to the other tag's list", async ({ page, context, session }) => {
  const s = await setup(session);
  await login(context, member().name);
  await page.goto(`/tags/intersection/${s.a}/${s.b}`);
  const chooser = page.locator(CHOOSER);
  await expect(chooser.locator(".formatted-selection")).toHaveText(`${s.a}, ${s.b}`);
  // The composer's own chooser is another one.
  await expect(page.locator("#reply-control details.mini-tag-chooser")).toHaveCount(1);
  await chooser.locator("summary").click();
  await expect(chooser.locator(".selected-content .selected-choice")).toHaveText([s.a, s.b]);
  await chooser.locator(`.selected-choice[data-name="${s.a}"]`).click();
  // /tag/:name, which the server sends on to the tag's canonical address.
  await expect(page.locator(".category-breadcrumb details.tag-drop summary")).toHaveAttribute("data-name", s.b);
  await expect(page).toHaveURL(new RegExp(`/tag/${s.b}/${s.ids.b}$`));
  await expect(row(page, s.all)).toBeVisible();
  await expect(row(page, s.two)).toBeVisible();
  await expect(row(page, s.one)).toHaveCount(0);
});

test("the maximum of tags per topic closes the chooser to more", async ({ page, session }) => {
  const s = await setup(session);
  const max = Number(sql("SELECT COALESCE((SELECT value FROM site_settings WHERE name = 'max_tags_per_topic'), '5')")[0][0]);
  const names = [s.a, s.b, s.c];
  while (names.length < max) {
    names.push(tagName(`m${names.length}`));
  }
  await page.goto(`/tags/intersection/${names.slice(0, max).join("/")}`);
  const chooser = page.locator(CHOOSER);
  await expect(chooser.locator(".formatted-selection")).toHaveText(names.slice(0, max).join(", "));
  // MiniTagChooser#caretIcon: no plus at the maximum.
  await expect(chooser.locator("summary svg.d-icon-plus")).toHaveCount(0);
  const searched = page.waitForRequest((r) => r.url().includes("/tags/filter/search"));
  await chooser.locator("summary").click();
  await searched;
  await expect(chooser.locator(".selected-content .selected-choice")).toHaveCount(max);
});
