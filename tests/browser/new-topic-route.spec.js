// The new-topic route's page load (src/routes/new_topic.rs, composer.js
// routeArrived): array query params (`tags[]=`) as Ember's router
// collects them, and a category the member can't create topics in,
// which the composer's chooser lists read-only.
const { test, expect, login, api, sql, label, member } = require("./helpers");

const control = (page) => page.locator("#reply-control");
const drafts = (id) => sql("SELECT data FROM drafts WHERE user_id = :'p1' AND draft_key LIKE 'new_topic%'", id);

test("an array tags param opens no composer for a member", async ({ page, context }) => {
  const me = member();
  await login(context, me.name);
  const tagRequests = [];
  page.on("request", (r) => r.url().includes("/tags/list.json") && tagRequests.push(r.url()));

  await page.goto("/new-topic?title=Array+tags&tags[]=howto&tags%5B%5D=guide");
  // The discovery list keeps `tags`, the array joined.
  await expect(page).toHaveURL(/\/latest\?tags=howto%2Cguide$/);
  await expect(page.locator("#composer-open")).toHaveCount(0);
  // composer.filterTags throws on the array before anything opens.
  await expect(control(page)).not.toHaveClass(/\bopen\b/);
  expect(tagRequests).toEqual([]);
  expect(drafts(me.id)).toEqual([]);

  // The same link followed from a page, and a plain one after it.
  await page.evaluate(() => {
    const add = (id, href) => {
      const a = document.createElement("a");
      a.id = id;
      a.href = href;
      a.textContent = id;
      document.querySelector("#main-outlet").prepend(a);
    };
    add("array-link", "/new-topic?title=Array+link&tags[]=howto");
    add("plain-link", "/new-topic?title=Plain+link&tags=guide");
  });
  await page.locator("#array-link").click();
  await expect(page).toHaveURL(/\/latest\?tags=howto%2Cguide$/);
  await expect(control(page)).not.toHaveClass(/\bopen\b/);
  expect(tagRequests).toEqual([]);

  await page.locator("#plain-link").click();
  await expect(control(page)).toHaveClass(/\bopen\b/);
  await expect(page.locator("#reply-title")).toHaveValue("Plain link");
  await expect(control(page).locator("details.mini-tag-chooser summary .formatted-selection")).toHaveText("guide");
  await expect(page).toHaveURL(/\/latest\?tags=howto%2Cguide$/);
});

test("staff's array tags go on as given", async ({ page, context }) => {
  const me = member();
  sql("UPDATE users SET moderator = true WHERE id = :'p1'", me.id);
  sql(
    "INSERT INTO group_users (group_id, user_id, created_at, updated_at) \
     SELECT id, :'p1', now(), now() FROM groups WHERE name IN ('moderators', 'staff')",
    me.id,
  );
  await login(context, me.name);
  const token = label("arr").split(" ")[1];
  const saved = page.waitForResponse(
    (r) => r.url().includes("/drafts.json") && r.request().method() === "POST" && (r.request().postData() || "").includes(token),
  );
  await page.goto(`/new-topic?title=Staff+${token}&tags[]=howto&tags[]=guide&body=Array+tags+body`);
  await expect(page).toHaveURL(/\/latest\?tags=howto%2Cguide$/);
  await expect(control(page)).toHaveClass(/\bopen\b/);
  await expect(page.locator("#reply-title")).toHaveValue(`Staff ${token}`);
  await expect(control(page).locator(".d-editor-input")).toHaveValue("Array tags body");
  const selection = control(page).locator("details.mini-tag-chooser summary .formatted-selection");
  await expect(selection).toContainText("howto");
  await expect(selection).toContainText("guide");
  expect((await saved).ok()).toBeTruthy();
  const [[data]] = drafts(me.id);
  expect(JSON.parse(data).tags.map((t) => t.name)).toEqual(["howto", "guide"]);
});

test("a category the member can't post in is read-only in the chooser", async ({ page, context, session }) => {
  const admin = await session("admin");
  const name = label("Read only");
  const created = await api(admin, "POST", "/categories.json", { name, color: "0088CC", text_color: "FFFFFF" });
  const id = String(created.category.id);
  // Everyone may read it, only staff create topics in it.
  sql(
    "INSERT INTO category_groups (category_id, group_id, permission_type, created_at, updated_at) \
     SELECT :'p1', g.id, CASE g.name WHEN 'staff' THEN 1 ELSE 3 END, now(), now() \
     FROM groups g WHERE g.name IN ('everyone', 'staff')",
    id,
  );
  const me = member();
  await login(context, me.name);
  const title = label("In another category");

  await page.goto(`/new-topic?category_id=${id}&title=${encodeURIComponent(title)}`);
  await expect(page).toHaveURL(new RegExp(`/c/${id}$`));
  await expect(control(page)).toHaveClass(/\bopen\b/);
  await expect(page.locator("#reply-title")).toHaveValue(title);

  // Nothing chosen: the read-only category is never the topic's.
  const chooser = control(page).locator("details.category-chooser");
  const header = chooser.locator("summary .select-kit-selected-name");
  await expect(header).toHaveText("Category…");
  await expect(chooser).not.toHaveClass(/has-selection/);

  // Its row comes first, with the read-only pill, and is not to be chosen.
  await chooser.locator("summary").click();
  const rows = chooser.locator(".category-row");
  await expect(rows.first()).toHaveAttribute("data-value", id);
  const row = chooser.locator(`.category-row[data-value='${id}']`);
  await expect(row).toHaveCount(1);
  await expect(row).toHaveAttribute("title", "This category is read-only so you cannot choose it for this topic");
  await expect(row.locator(".read-only")).toHaveText("read-only");
  await row.click();
  await expect(chooser).toHaveAttribute("open", "");
  await expect(header).toHaveText("Category…");
  await expect(chooser).not.toHaveClass(/has-selection/);

  // Another category is, and the topic goes there.
  await chooser.locator(".category-row[data-value='4']").click();
  await expect(chooser).not.toHaveAttribute("open", "");
  await expect(header).toContainText("General");
  await control(page).locator(".d-editor-input").fill(`The first post of ${title}, written under a read-only category.`);
  await control(page).locator(".save-or-cancel .create").click();
  await expect(page).toHaveURL(/\/t\/[^/]+\/\d+/);
  await expect(page.locator("#topic-title h1")).toContainText(title);
  expect(sql("SELECT category_id FROM topics WHERE title = :'p1'", title)).toEqual([["4"]]);

  // The next composer, opened in place, lists the categories as the page
  // drew them.
  await page.evaluate(() => {
    const a = document.createElement("a");
    a.id = "again-link";
    a.href = "/new-topic?title=Again";
    a.textContent = "again";
    document.querySelector("#main-outlet").prepend(a);
  });
  await page.locator("#again-link").click();
  await expect(page.locator("#reply-title")).toHaveValue("Again");
  await expect(control(page).locator(".category-row").first()).toBeAttached();
  await expect(control(page).locator(`.category-row[data-value='${id}']`)).toHaveCount(0);
  await expect(control(page).locator("details.category-chooser summary .select-kit-selected-name")).toContainText("General");
});
