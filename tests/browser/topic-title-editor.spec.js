// The topic title's editor (static/js/topic-title-editor.js, Ember's
// TopicTitle, TopicTitleEditor and TopicMetadata): who may edit the topic
// has the edit pencil in the title, whose click swaps the title for
// #edit-title, the category chooser, the tag chooser and the save and
// cancel buttons. Escape and cancel leave the topic alone, the save
// button and Ctrl+Enter send what changed (PUT /t/:slug/:id as JSON) and
// draw the title, the address and the document's title again; a refusal
// is a dialog over the open editor, and a live redraw of the title waits
// for the editor to close. Each write is checked in the database.
const { test, expect, login, api, topic, sql, label } = require("./helpers");

const TITLE = "#topic-title h1 a.fancy-title";
const PENCIL = `${TITLE} .edit-topic__wrapper svg.d-icon-pencil.edit-topic`;
const EDITOR = "#topic-title .title-wrapper .edit-topic-title";
const INPUT = `${EDITOR} .edit-title__wrapper input#edit-title`;

function row(id) {
  return sql("SELECT title, slug, category_id FROM topics WHERE id = :'p1'", id)[0];
}

// A topic by user1, long enough to scroll its title away.
async function setup(session, what) {
  const user1 = await session("user1");
  const title = label(what);
  const body = Array.from({ length: 40 }, (_, i) => label(`Paragraph ${i} of the title editor spec`)).join("\n\n");
  const t = await topic(user1, title, body);
  return { ...t, title, user1 };
}

async function openEditor(page) {
  await page.locator(PENCIL).click();
  await expect(page.locator(INPUT)).toBeFocused();
}

// The PUT the editor sends: its JSON body.
function updates(page, id) {
  return page.waitForRequest((r) => r.method() === "PUT" && new RegExp(`/t/[^/]+/${id}$`).test(new URL(r.url()).pathname));
}

test("the author and an admin have the pencil, another member and a visitor do not", async ({ page, context, session }) => {
  const t = await setup(session, "Title editor pencil");
  await login(context, "user1");
  await page.goto(t.url);
  await expect(page.locator(TITLE)).toHaveText(t.title);
  await expect(page.locator(PENCIL)).toHaveCount(1);

  for (const [who, count] of [
    ["admin", 1],
    ["user2", 0],
    [null, 0],
  ]) {
    const other = await (await session(who)).newPage();
    await other.goto(t.url);
    await expect(other.locator(TITLE)).toHaveText(t.title);
    await expect(other.locator(PENCIL), String(who)).toHaveCount(count);
    await expect(other.locator("script.topic-title-editor-config"), String(who)).toHaveCount(count);
  }
});

test("a save is drawn though the address moved on while the topic was read again", async ({ page, context, session }) => {
  const t = await setup(session, "Title editor address");
  await login(context, "user1");
  await page.goto(t.url);
  await openEditor(page);
  let release;
  const held = new Promise((resolve) => (release = resolve));
  let reloading;
  const reload = new Promise((resolve) => (reloading = resolve));
  await page.route(new RegExp(`/t/[^/]+/${t.id}(/\\d+)?$`), async (route) => {
    if (route.request().method() !== "GET") {
      return route.continue();
    }
    reloading();
    await held;
    await route.continue();
  });
  const title = label("Title editor readdressed");
  await page.locator(INPUT).fill(title);
  await page.locator(`${EDITOR} .submit-edit`).click();
  await reload;
  // The post being read changing the address, as scrolling does.
  await page.evaluate((id) => {
    const path = location.pathname.replace(new RegExp(`(/${id})(/\\d+)?$`), "$1/2");
    history.replaceState(history.state, "", path);
  }, t.id);
  release();
  await expect(page).toHaveTitle(new RegExp(`^${title} - `));
  await expect(page.locator(TITLE)).toHaveText(title);
});

test("the editor saves a new title and draws it", async ({ page, context, session }) => {
  const t = await setup(session, "Title editor save");
  await login(context, "user1");
  await page.goto(t.url);
  await openEditor(page);

  // TopicTitleEditor and TopicMetadata in the title's place.
  await expect(page.locator("#topic-title h1")).toHaveCount(0);
  await expect(page.locator("#topic-title .topic-category")).toHaveCount(0);
  await expect(page.locator(INPUT)).toHaveValue(t.title);
  await expect(page.locator(INPUT)).toHaveAttribute("maxlength", "255");
  await expect(page.locator(`${EDITOR} .edit-category__wrapper details.category-chooser.small`)).toHaveCount(1);
  await expect(page.locator(`${EDITOR} .edit-category__wrapper summary`)).toHaveAttribute("data-value", "4");
  await expect(page.locator(`${EDITOR} .edit-controls > button`)).toHaveClass([
    /\bbtn-primary submit-edit\b/,
    /\bbtn-default cancel-edit\b/,
  ]);
  await expect(page.locator(`${EDITOR} .submit-edit`)).toHaveAttribute("aria-label", "Save Edit");
  await expect(page.locator(`${EDITOR} .cancel-edit`)).toHaveAttribute("aria-label", "Cancel");

  const title = label("Title editor saved");
  await page.locator(INPUT).fill(title);
  const sent = updates(page, t.id);
  await page.locator(`${EDITOR} .submit-edit`).click();
  const request = await sent;
  expect(request.headers()["content-type"]).toContain("application/json");
  expect(request.postDataJSON()).toEqual({ title, keep_existing_draft: true });

  await expect(page.locator(EDITOR)).toHaveCount(0);
  await expect(page.locator(TITLE)).toHaveText(title);
  await expect(page.locator(PENCIL)).toHaveCount(1);
  await expect.poll(() => row(t.id)[0]).toBe(title);
  const slug = row(t.id)[1];
  expect(slug).not.toBe(t.url.split("/")[2]);
  await expect(page).toHaveURL(new RegExp(`/t/${slug}/${t.id}(/1)?$`));
  await expect(page).toHaveTitle(new RegExp(`^${title} - `));
  await expect(page.locator(TITLE)).toHaveAttribute("href", `/t/${slug}/${t.id}`);

  // The docked header's topic is the edited one.
  await page.evaluate(() => window.scrollTo(0, document.body.scrollHeight));
  await expect(page.locator(".d-header .extra-info-wrapper .topic-link")).toHaveText(title);

  // And the editor opens again on the title as saved.
  await page.evaluate(() => window.scrollTo(0, 0));
  await expect(page.locator(".d-header .extra-info-wrapper")).toHaveCount(0);
  await openEditor(page);
  await expect(page.locator(INPUT)).toHaveValue(title);
});

test("Escape and cancel leave the topic alone, Ctrl+Enter saves", async ({ page, context, session }) => {
  const t = await setup(session, "Title editor keys");
  await login(context, "user1");
  await page.goto(t.url);
  let puts = 0;
  page.on("request", (r) => {
    if (r.method() === "PUT" && new URL(r.url()).pathname.endsWith(`/${t.id}`)) {
      puts += 1;
    }
  });

  await openEditor(page);
  await page.locator(INPUT).fill(label("Title editor escaped"));
  await page.keyboard.press("Escape");
  await expect(page.locator(EDITOR)).toHaveCount(0);
  await expect(page.locator(TITLE)).toHaveText(t.title);
  await expect(page.locator("#topic-title .topic-category .badge-category__wrapper")).toHaveCount(1);

  // The buffer is discarded: the editor opens on the topic's title.
  await openEditor(page);
  await expect(page.locator(INPUT)).toHaveValue(t.title);
  await page.locator(INPUT).fill(label("Title editor cancelled"));
  await page.locator(`${EDITOR} .cancel-edit`).click();
  await expect(page.locator(EDITOR)).toHaveCount(0);
  await expect(page.locator(TITLE)).toHaveText(t.title);

  // TopicTitle#keyDown: a plain Enter is not a save.
  await openEditor(page);
  const title = label("Title editor entered");
  await page.locator(INPUT).fill(title);
  await page.keyboard.press("Enter");
  await expect(page.locator(INPUT)).toHaveValue(title);
  expect(puts).toBe(0);
  expect(row(t.id)[0]).toBe(t.title);

  const sent = updates(page, t.id);
  await page.keyboard.press("Control+Enter");
  await sent;
  await expect(page.locator(EDITOR)).toHaveCount(0);
  await expect(page.locator(TITLE)).toHaveText(title);
  await expect.poll(() => row(t.id)[0]).toBe(title);
  expect(puts).toBe(1);
});

test("the category and the tags chosen are saved", async ({ page, context, session }) => {
  const t = await setup(session, "Title editor category");
  const admin = await session("admin");
  const created = await api(admin, "POST", "/categories.json", {
    name: label("Titled"),
    color: "0088CC",
    text_color: "FFFFFF",
  });
  const category = created.category;
  const name = label("titled").replace(/ /g, "-").slice(0, 20);
  const [[tag]] = sql(
    "INSERT INTO tags (name, slug, created_at, updated_at) VALUES (:'p1', :'p1', now(), now()) RETURNING id",
    name,
  );
  await login(context, "admin");
  await page.goto(t.url);
  await openEditor(page);

  const chooser = page.locator(`${EDITOR} .edit-category__wrapper details.category-chooser`);
  await chooser.locator("summary").click();
  await chooser.locator(`.category-row[data-value="${category.id}"]`).click();
  await expect(chooser.locator("summary")).toHaveAttribute("data-value", String(category.id));

  const tags = page.locator(`${EDITOR} .edit-tags__wrapper details.mini-tag-chooser`);
  // An open chooser's Escape closes the chooser, not the editor.
  await tags.locator("summary").click();
  await tags.locator(".filter-input").press("Escape");
  await expect(tags).not.toHaveAttribute("open", "");
  await expect(page.locator(INPUT)).toBeVisible();
  await tags.locator("summary").click();
  const searched = page.waitForRequest(
    (r) => r.url().includes("/tags/filter/search") && new URL(r.url()).searchParams.get("q") === name,
  );
  await tags.locator(".filter-input").fill(name);
  // The search is scoped to the category chosen.
  expect(new URL((await searched).url()).searchParams.get("categoryId")).toBe(String(category.id));
  await tags.locator(`.select-kit-row[data-name="${name}"], .select-kit-row[data-value="${name}"]`).first().click();
  await expect(tags.locator("summary")).toContainText(name);

  const sent = updates(page, t.id);
  await page.locator(`${EDITOR} .submit-edit`).click();
  expect((await sent).postDataJSON()).toEqual({
    category_id: category.id,
    tags: [{ id: Number(tag), name }],
    keep_existing_draft: true,
  });

  await expect(page.locator(EDITOR)).toHaveCount(0);
  await expect.poll(() => row(t.id)[2]).toBe(String(category.id));
  expect(row(t.id)[0]).toBe(t.title);
  expect(sql("SELECT tag_id FROM topic_tags WHERE topic_id = :'p1'", t.id)).toEqual([[tag]]);
  await expect(page.locator("#topic-title .topic-category .badge-category__name")).toHaveText(category.name);
  await expect(page.locator(`#topic-title .topic-category .discourse-tag[data-tag-name="${name}"]`)).toHaveCount(1);
  await expect(page.locator("body")).toHaveClass(new RegExp(`\\bcategory-${category.slug}\\b`));
  await expect(page.locator("body")).toHaveClass(new RegExp(`\\btag-${name}\\b`));

  // The editor opens on what was saved.
  await openEditor(page);
  await expect(chooser.locator("summary")).toHaveAttribute("data-value", String(category.id));
  await expect(tags.locator("summary")).toContainText(name);
});

test("a title the server refuses is a dialog over the open editor", async ({ page, context, session }) => {
  const t = await setup(session, "Title editor refused");
  await login(context, "user1");
  await page.goto(t.url);
  await openEditor(page);
  await page.locator(INPUT).fill("Too short");
  const answered = page.waitForResponse(
    (r) => r.request().method() === "PUT" && new URL(r.url()).pathname.endsWith(`/${t.id}`),
  );
  await page.locator(`${EDITOR} .submit-edit`).click();
  expect((await answered).status()).toBe(422);

  // popupAjaxError
  const dialog = page.locator("#dialog-holder .dialog-body");
  await expect(dialog).toContainText("Title is too short (minimum is 15 characters)");
  await page.locator("#dialog-holder .dialog-footer .btn-primary").click();
  await expect(page.locator("#dialog-holder")).toHaveCount(0);
  await expect(page.locator(INPUT)).toHaveValue("Too short");
  expect(row(t.id)[0]).toBe(t.title);

  // And it saves once it is long enough.
  const title = label("Title editor accepted");
  await page.locator(INPUT).fill(title);
  await page.locator(`${EDITOR} .submit-edit`).click();
  await expect(page.locator(TITLE)).toHaveText(title);
  await expect.poll(() => row(t.id)[0]).toBe(title);
});

test("a revision by someone else waits for the open editor", async ({ page, context, session }) => {
  const t = await setup(session, "Title editor live");
  const admin = await session("admin");
  await login(context, "user1");
  await page.goto(t.url);
  await openEditor(page);
  const typed = label("Title editor typed");
  await page.locator(INPUT).fill(typed);

  // The live stream's redraw of the title, as it arrives.
  await page.evaluate(() => {
    window.titleRedraws = 0;
    document.addEventListener("htmx:oobBeforeSwap", (event) => {
      if (event.detail.target.id === "topic-title") {
        window.titleRedraws += 1;
      }
    });
  });
  const theirs = label("Title editor theirs");
  await api(admin, "PUT", `/t/-/${t.id}`, { title: theirs });
  await expect.poll(() => page.evaluate(() => window.titleRedraws)).toBeGreaterThan(0);

  await expect(page.locator(INPUT)).toHaveValue(typed);
  await expect(page.locator(INPUT)).toBeFocused();
  await expect(page.locator("#topic-title h1")).toHaveCount(0);

  // Cancelled, the title is theirs.
  await page.locator(`${EDITOR} .cancel-edit`).click();
  await expect(page.locator(TITLE)).toHaveText(theirs);
  await expect(page.locator(PENCIL)).toHaveCount(1);
  await openEditor(page);
  await expect(page.locator(INPUT)).toHaveValue(theirs);
  expect(row(t.id)[0]).toBe(theirs);
});
