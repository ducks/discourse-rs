// The topic's multi-select mode (select-posts.js): the topic admin menu's
// Select Posts turns it on and off, each post gets its select controls
// (select, +below, +replies), SelectedPosts counts the selection and
// offers what the topic controller's rules allow, Delete selected deletes
// the selected posts (with every post selected, the topic), Merge
// selected posts merges them, and the ChangeOwner modal hands the
// selection (or, from the post admin menu, one post) to another member.
// Each change is checked in the database. Move to opens the MoveToTopic
// modal (move-to-topic.spec.js).
const { test, expect, login, api, topic, sql, label } = require("./helpers");

const MENU = ".topic-admin-menu-content";
const PANEL = ".container.posts > .selected-posts";
const MODAL = ".d-modal.change-ownership-modal";

// A topic by user1 with three replies: two by user0, then one by user2
// to the first reply (which so has replies).
async function setup(session, context, viewer = "admin") {
  const user1 = await session("user1");
  const user0 = await session("user0");
  const user2 = await session("user2");
  const t = await topic(user1, label("Select posts"), label("A topic for the select posts spec"), [
    { context: user0, raw: label("A first reply by user0 for the select posts spec") },
    { context: user0, raw: label("A second reply by user0 for the select posts spec") },
  ]);
  const reply = await api(user2, "POST", "/posts.json", {
    topic_id: String(t.id),
    raw: label("A reply by user2 to the first reply for the select posts spec"),
    reply_to_post_number: "2",
  });
  t.posts.push(reply.id);
  if (viewer) {
    await login(context, viewer);
  }
  return t;
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

async function toggleMode(page) {
  await page.locator(".topic-admin-menu-trigger:visible").first().click();
  await expect(page.locator(MENU)).toBeVisible();
  await page.locator(`${MENU} .topic-admin-multi-select .btn`).click();
  await expect(page.locator(MENU)).toHaveCount(0);
}

async function enter(page, t) {
  await page.goto(t.url);
  await expect(page.locator("#posts article[data-post-id]")).toHaveCount(t.posts.length);
  await settled(page);
  await toggleMode(page);
  await expect(page.locator(PANEL)).toBeVisible();
}

function article(page, id) {
  return page.locator(`article[data-post-id="${id}"]`);
}

function controls(page, id) {
  return article(page, id).locator(".topic-meta-data .post-infos > .select-posts");
}

// The classes of a post's select buttons, in order.
function buttons(page, id) {
  return controls(page, id)
    .locator("button")
    .evaluateAll((all) => all.map((b) => b.className.replace(/^btn btn-text btn-flat /, "")));
}

// The ids of the posts drawn selected.
function selected(page) {
  return page
    .locator("#posts .topic-post.post--selected.selected > article")
    .evaluateAll((all) => all.map((a) => Number(a.dataset.postId)));
}

// The panel's buttons, by their labels.
function actions(page) {
  return page.locator(`${PANEL} .btn .d-button-label`).allTextContents();
}

function deleted(t) {
  return sql("SELECT id FROM posts WHERE topic_id = :'p1' AND deleted_at IS NOT NULL ORDER BY id", t.id).map((r) =>
    Number(r[0]),
  );
}

function owners(t) {
  return sql(
    "SELECT u.username FROM posts p JOIN users u ON u.id = p.user_id WHERE p.topic_id = :'p1' ORDER BY p.post_number",
    t.id,
  ).map((r) => r[0]);
}

test("Select Posts turns the mode on, cancel and the menu turn it off", async ({ page, context, session }) => {
  const t = await setup(session, context);
  await page.goto(t.url);
  await expect(page.locator(".selected-posts")).toHaveCount(0);
  await expect(page.locator(".select-posts")).toHaveCount(0);
  await settled(page);
  await toggleMode(page);

  // SelectedPosts: nothing selected offers select all and cancel only.
  const panel = page.locator(PANEL);
  await expect(panel).toBeVisible();
  await expect(panel.locator("p").first()).toHaveText("You have selected 0 posts.");
  expect(await panel.locator("p > span").first().innerHTML()).toBe("You have selected <b>0</b> posts.");
  await expect(panel.locator("a")).toHaveText(["select all", "cancel selecting"]);
  await expect(panel.locator("a.select-all")).toHaveCount(1);
  await expect(panel.locator(".btn")).toHaveCount(0);
  await expect(page.locator(".container.posts > :first-child")).toHaveClass("selected-posts");

  // The posts' controls: the first post's select alone, a reply's
  // +below, and +replies on the one with replies.
  expect(await buttons(page, t.posts[0])).toEqual(["select-post"]);
  expect(await buttons(page, t.posts[1])).toEqual(["select-replies", "select-below", "select-post"]);
  expect(await buttons(page, t.posts[2])).toEqual(["select-below", "select-post"]);
  expect(await buttons(page, t.posts[3])).toEqual(["select-below", "select-post"]);
  await expect(controls(page, t.posts[1]).locator("button")).toHaveText(["select +replies", "select +below", "select"]);
  await expect(controls(page, t.posts[1]).locator(".select-below")).toHaveAttribute(
    "title",
    "Add post and all after it to selection",
  );
  // In the post's infos before its date.
  expect(
    await article(page, t.posts[2])
      .locator(".post-infos > *")
      .evaluateAll((all) => all.map((el) => el.className.split(" ")[0] + (el.className.includes("post-date") ? " post-date" : ""))),
  ).toEqual(["select-posts", "post-info post-date", "read-state"]);
  expect(await selected(page)).toEqual([]);

  // Cancel selecting.
  await panel.locator("p.cancel a").click();
  await expect(page.locator(".selected-posts")).toHaveCount(0);
  await expect(page.locator(".select-posts")).toHaveCount(0);
  await expect(page).toHaveURL(new RegExp(`/t/[^/]+/${t.id}`));

  // On again with a post selected, then off from the menu: the
  // selection starts empty each time.
  await toggleMode(page);
  await controls(page, t.posts[1]).locator(".select-post").click();
  expect(await selected(page)).toEqual([t.posts[1]]);
  await toggleMode(page);
  await expect(page.locator(".selected-posts")).toHaveCount(0);
  await expect(page.locator(".select-posts")).toHaveCount(0);
  expect(await selected(page)).toEqual([]);
  await toggleMode(page);
  await expect(page.locator(`${PANEL} p`).first()).toHaveText("You have selected 0 posts.");
  expect(await selected(page)).toEqual([]);
});

test("posts are selected one by one and +below, the panel following the rules", async ({ page, context, session }) => {
  const t = await setup(session, context);
  await enter(page, t);
  const panel = page.locator(PANEL);

  // One post: its button says selected, +below and +replies are gone.
  await controls(page, t.posts[1]).locator(".select-post").click();
  await expect(panel.locator("p").first()).toHaveText("You have selected 1 post.");
  expect(await selected(page)).toEqual([t.posts[1]]);
  expect(await buttons(page, t.posts[1])).toEqual(["select-post"]);
  await expect(controls(page, t.posts[1]).locator(".select-post")).toHaveText("selected");
  await expect(controls(page, t.posts[1]).locator(".select-post")).toHaveAttribute(
    "title",
    "Click to remove post from selection",
  );
  await expect(controls(page, t.posts[1]).locator(".select-post")).toHaveCSS(
    "background-color",
    await page.evaluate(() => {
      const probe = document.createElement("i");
      probe.style.backgroundColor = "var(--tertiary)";
      document.body.appendChild(probe);
      const color = getComputedStyle(probe).backgroundColor;
      probe.remove();
      return color;
    }),
  );
  // Delete, Move to and Change Owner for one post; merging needs two.
  expect(await actions(page)).toEqual(["delete selected", "move to", "change ownership"]);
  await expect(panel.locator(".btn.btn-danger .d-icon-trash-can")).toHaveCount(1);
  await expect(panel.locator(".btn.btn-primary.move-to-topic .d-icon-right-from-bracket")).toHaveCount(1);
  for (const name of ["trash-can", "right-from-bracket", "user", "up-down"]) {
    expect(await page.locator(`symbol#${name}`).count(), name).toBe(1);
  }

  // A second post by the same member: they can be merged.
  await controls(page, t.posts[2]).locator(".select-post").click();
  await expect(panel.locator("p").first()).toHaveText("You have selected 2 posts.");
  expect(await selected(page)).toEqual([t.posts[1], t.posts[2]]);
  expect(await actions(page)).toEqual(["delete selected", "move to", "change ownership", "merge selected posts"]);

  // Unselected again.
  await controls(page, t.posts[1]).locator(".select-post").click();
  await expect(panel.locator("p").first()).toHaveText("You have selected 1 post.");
  expect(await selected(page)).toEqual([t.posts[2]]);
  expect(await buttons(page, t.posts[1])).toEqual(["select-replies", "select-below", "select-post"]);

  // +below from the first reply: it and every post after it, the one
  // already selected counted once. Two authors: no owner to change, no
  // merging.
  await controls(page, t.posts[1]).locator(".select-below").click();
  await expect(panel.locator("p").first()).toHaveText("You have selected 3 posts.");
  expect(await selected(page)).toEqual([t.posts[1], t.posts[2], t.posts[3]]);
  expect(await actions(page)).toEqual(["delete selected", "move to"]);
  await expect(panel.locator("a")).toHaveText(["select all", "cancel selecting"]);
  expect(deleted(t)).toEqual([]);
});

test("select all and deselect all", async ({ page, context, session }) => {
  const t = await setup(session, context);
  await enter(page, t);
  const panel = page.locator(PANEL);
  await panel.locator("a.select-all").click();
  await expect(panel.locator("p").first()).toHaveText("You have selected 4 posts.");
  expect(await selected(page)).toEqual(t.posts);
  // Every post selected: deselect all in select all's stead.
  await expect(panel.locator("a")).toHaveText(["deselect all", "cancel selecting"]);
  expect(await actions(page)).toEqual(["delete selected", "move to"]);
  for (const id of t.posts) {
    expect(await buttons(page, id)).toEqual(["select-post"]);
  }
  await panel.locator("a", { hasText: "deselect all" }).click();
  await expect(panel.locator("p").first()).toHaveText("You have selected 0 posts.");
  expect(await selected(page)).toEqual([]);
  await expect(panel.locator("a")).toHaveText(["select all", "cancel selecting"]);
  await expect(panel.locator(".btn")).toHaveCount(0);
  await expect(page).toHaveURL(new RegExp(`/t/[^/]+/${t.id}`));
});

test("the selection outlives a post drawn again and reaches posts that arrive", async ({ page, context, session }) => {
  const t = await setup(session, context);
  await enter(page, t);
  const panel = page.locator(PANEL);
  await controls(page, t.posts[2]).locator(".select-post").click();
  expect(await selected(page)).toEqual([t.posts[2]]);

  // The post drawn again (GET /live/post/:id, as its admin menu does).
  await article(page, t.posts[2]).evaluate((el) => (el.dataset.stale = "1"));
  await page.evaluate(
    (id) => window.htmx.ajax("GET", document.querySelector('meta[name="discourse-base-uri"]').content + "/live/post/" + id, { swap: "none" }),
    t.posts[2],
  );
  await expect(page.locator(`article[data-post-id="${t.posts[2]}"][data-stale]`)).toHaveCount(0);
  await expect(controls(page, t.posts[2]).locator(".select-post")).toHaveText("selected");
  expect(await selected(page)).toEqual([t.posts[2]]);
  expect(await buttons(page, t.posts[2])).toEqual(["select-post"]);

  // A reply arriving while the mode is on gets its controls; select all
  // reaches it.
  const user1 = await session("user1");
  const late = await api(user1, "POST", "/posts.json", {
    topic_id: String(t.id),
    raw: label("A late reply by user1 for the select posts spec"),
  });
  await expect(article(page, late.id)).toBeVisible();
  await expect(controls(page, late.id).locator("button")).toHaveText(["select +below", "select"]);
  await expect(panel.locator("p").first()).toHaveText("You have selected 1 post.");
  await panel.locator("a.select-all").click();
  await expect(panel.locator("p").first()).toHaveText("You have selected 5 posts.");
  expect(await selected(page)).toEqual([...t.posts, late.id]);
});

test("Delete selected with every post selected deletes the topic", async ({ page, context, session }) => {
  const t = await setup(session, context);
  await enter(page, t);
  await page.locator(`${PANEL} a.select-all`).click();
  await page.locator(`${PANEL} .btn-danger`).click();
  const dialog = page.locator(".dialog-content");
  await expect(dialog.locator(".dialog-body")).toHaveText("Are you sure you want to delete these 4 posts?");
  await expect(dialog.locator(".dialog-footer .btn")).toHaveText(["Yes", "No"]);
  // No leaves everything as it was.
  await dialog.locator(".dialog-footer .btn", { hasText: "No" }).click();
  await expect(dialog).toHaveCount(0);
  expect(sql("SELECT deleted_at IS NULL FROM topics WHERE id = :'p1'", t.id)).toEqual([["t"]]);
  expect(await selected(page)).toEqual(t.posts);

  await page.locator(`${PANEL} .btn-danger`).click();
  await dialog.locator(".dialog-footer .btn", { hasText: "Yes" }).click();
  await expect.poll(() => sql("SELECT deleted_at IS NOT NULL FROM topics WHERE id = :'p1'", t.id)).toEqual([["t"]]);
  // Staff stay on the topic, drawn again with the mode off. (A deleted
  // topic's page is refused while discourse-post-voting is on, so its
  // posts are not looked at here.)
  await expect(page.locator(".selected-posts")).toHaveCount(0);
});

test("ChangeOwner hands the selected posts to another member", async ({ page, context, session }) => {
  const t = await setup(session, context);
  await enter(page, t);
  await controls(page, t.posts[1]).locator(".select-post").click();
  await controls(page, t.posts[2]).locator(".select-post").click();
  await page.locator(`${PANEL} .btn`, { hasText: "change ownership" }).click();

  const modal = page.locator(MODAL);
  await expect(modal.locator(".d-modal__title-text")).toHaveText("Change Owner");
  await expect(modal.locator(".d-modal__body.change-ownership > span")).toHaveText(
    "Please choose a new owner for the 2 posts by @user0",
  );
  expect(await modal.locator(".d-modal__body > span b").innerText()).toBe("@user0");
  const submit = modal.locator(".d-modal__footer .btn-primary");
  await expect(submit).toHaveText("change ownership");
  await expect(submit).toBeDisabled();

  // The chooser: members only, one at most.
  const chooser = modal.locator("details.email-group-user-chooser");
  await chooser.locator("summary").click();
  const filter = chooser.locator(".filter-input");
  await expect(filter).toHaveAttribute("placeholder", "username of new owner");
  await filter.fill("user2");
  await chooser.locator('.select-kit-row[data-value="user2"]').click();
  await expect(chooser.locator("summary")).toHaveAttribute("data-value", "user2");
  await expect(chooser).not.toHaveAttribute("open", "");
  await expect(submit).toBeEnabled();

  await Promise.all([page.waitForEvent("load"), submit.click()]);
  expect(owners(t)).toEqual(["user1", "user2", "user2", "user2"]);
  // The page drawn again, the mode off.
  await expect(page.locator(".selected-posts")).toHaveCount(0);
  await expect(page.locator(".select-posts")).toHaveCount(0);
  await expect(article(page, t.posts[1])).toHaveAttribute("data-username", "user2");
});

test("the post admin menu's Change Ownership opens ChangeOwner for its post", async ({ page, context, session }) => {
  const t = await setup(session, context);
  await page.goto(t.url);
  await settled(page);
  const post = article(page, t.posts[3]);
  await post.hover();
  const more = post.locator("nav.post-controls .show-more-actions");
  if (await more.count()) {
    await more.click();
  }
  await post.locator(".show-post-admin-menu").click();
  await page.locator('.fk-d-menu[data-identifier="admin-post-menu"] button.change-owner').click();

  const modal = page.locator(MODAL);
  await expect(modal.locator(".d-modal__body > span")).toHaveText("Please choose a new owner for the post by @user2");
  const submit = modal.locator(".d-modal__footer .btn-primary");
  await expect(submit).toBeDisabled();
  const chooser = modal.locator("details.email-group-user-chooser");
  await chooser.locator("summary").click();
  await chooser.locator(".filter-input").fill("user0");
  await chooser.locator('.select-kit-row[data-value="user0"]').click();
  await Promise.all([page.waitForEvent("load"), submit.click()]);
  expect(owners(t)).toEqual(["user1", "user0", "user0", "user0"]);
});

test("ChangeOwner flashes its error when the change is refused", async ({ page, context, session }) => {
  const t = await setup(session, context);
  await enter(page, t);
  await controls(page, t.posts[3]).locator(".select-post").click();
  await page.locator(`${PANEL} .btn`, { hasText: "change ownership" }).click();
  const modal = page.locator(MODAL);
  const chooser = modal.locator("details.email-group-user-chooser");
  await chooser.locator("summary").click();
  await chooser.locator(".filter-input").fill("user0");
  await chooser.locator('.select-kit-row[data-value="user0"]').click();
  await page.route("**/change-owner", (route) =>
    route.fulfill({ status: 422, contentType: "application/json", body: '{"errors":["no"]}' }),
  );
  const submit = modal.locator(".d-modal__footer .btn-primary");
  await submit.click();
  await expect(modal.locator("#modal-alert.alert-error")).toHaveText(
    "There was an error changing the ownership of the posts.",
  );
  await expect(submit).toBeEnabled();
  await expect(submit).toHaveText("change ownership");
  expect(owners(t)).toEqual(["user1", "user0", "user0", "user2"]);
});

test("+replies selects the post and its replies", async ({ page, context, session }) => {
  const t = await setup(session, context);
  await enter(page, t);
  const panel = page.locator(PANEL);
  const replyIds = page.waitForResponse((r) => r.url().endsWith(`/posts/${t.posts[1]}/reply-ids.json`));
  await controls(page, t.posts[1]).locator(".select-replies").click();
  expect(await (await replyIds).json()).toEqual([{ id: t.posts[3], level: 1 }]);
  await expect(panel.locator("p").first()).toHaveText("You have selected 2 posts.");
  expect(await selected(page)).toEqual([t.posts[1], t.posts[3]]);
  expect(await buttons(page, t.posts[1])).toEqual(["select-post"]);
  expect(await buttons(page, t.posts[2])).toEqual(["select-below", "select-post"]);
  // Two authors: no owner to change, no merging.
  expect(await actions(page)).toEqual(["delete selected", "move to"]);

  // A reply already selected is counted once.
  await controls(page, t.posts[1]).locator(".select-post").click();
  await expect(panel.locator("p").first()).toHaveText("You have selected 1 post.");
  await controls(page, t.posts[1]).locator(".select-replies").click();
  await expect(panel.locator("p").first()).toHaveText("You have selected 2 posts.");
  expect(await selected(page)).toEqual([t.posts[1], t.posts[3]]);
  expect(deleted(t)).toEqual([]);
});

test("Delete selected deletes the selected posts", async ({ page, context, session }) => {
  const t = await setup(session, context);
  await enter(page, t);
  await controls(page, t.posts[1]).locator(".select-post").click();
  await controls(page, t.posts[3]).locator(".select-post").click();
  await page.locator(`${PANEL} .btn-danger`).click();
  const dialog = page.locator(".dialog-content");
  await expect(dialog.locator(".dialog-body")).toHaveText("Are you sure you want to delete these 2 posts?");
  // No leaves everything as it was.
  await dialog.locator(".dialog-footer .btn", { hasText: "No" }).click();
  await expect(dialog).toHaveCount(0);
  expect(deleted(t)).toEqual([]);
  expect(await selected(page)).toEqual([t.posts[1], t.posts[3]]);

  await page.locator(`${PANEL} .btn-danger`).click();
  const destroyed = page.waitForRequest((r) => r.url().endsWith("/posts/destroy_many"));
  await Promise.all([page.waitForEvent("load"), dialog.locator(".dialog-footer .btn", { hasText: "Yes" }).click()]);
  // Post.deleteMany's params, form encoded.
  const request = await destroyed;
  expect(request.method()).toBe("DELETE");
  expect(decodeURIComponent(request.postData())).toBe(
    `post_ids[]=${t.posts[1]}&post_ids[]=${t.posts[3]}&agree_with_first_reply_flag=true`,
  );
  expect(deleted(t)).toEqual([t.posts[1], t.posts[3]]);
  expect(sql("SELECT deleted_at IS NULL FROM topics WHERE id = :'p1'", t.id)).toEqual([["t"]]);
  // The page drawn again, the mode off.
  await expect(page.locator(".selected-posts")).toHaveCount(0);
  await expect(page.locator(".select-posts")).toHaveCount(0);
});

function raws(t) {
  return sql("SELECT replace(raw, E'\\n', '|') FROM posts WHERE topic_id = :'p1' ORDER BY post_number", t.id).map(
    (r) => r[0],
  );
}

test("Merge selected posts merges the posts of one author", async ({ page, context, session }) => {
  const t = await setup(session, context);
  const before = raws(t);
  await enter(page, t);
  await controls(page, t.posts[1]).locator(".select-post").click();
  await controls(page, t.posts[2]).locator(".select-post").click();
  await page.locator(`${PANEL} .btn`, { hasText: "merge selected posts" }).click();
  const dialog = page.locator(".dialog-content");
  await expect(dialog.locator(".dialog-body")).toHaveText("Are you sure you want to merge these 2 posts?");
  // No leaves everything as it was.
  await dialog.locator(".dialog-footer .btn", { hasText: "No" }).click();
  await expect(dialog).toHaveCount(0);
  expect(raws(t)).toEqual(before);
  expect(await selected(page)).toEqual([t.posts[1], t.posts[2]]);

  await page.locator(`${PANEL} .btn`, { hasText: "merge selected posts" }).click();
  await Promise.all([page.waitForEvent("load"), dialog.locator(".dialog-footer .btn", { hasText: "Yes" }).click()]);
  // The last of them holds them all, the others are deleted.
  expect(raws(t)).toEqual([before[0], before[1], `${before[1]}||${before[2]}`, before[3]]);
  expect(deleted(t)).toEqual([t.posts[1]]);
  await expect(page.locator(".selected-posts")).toHaveCount(0);
  await expect(page.locator(".select-posts")).toHaveCount(0);
});

test("Merge selected posts shows what the server refuses", async ({ page, context, session }) => {
  const t = await setup(session, context);
  const before = raws(t);
  await enter(page, t);
  await controls(page, t.posts[1]).locator(".select-post").click();
  await controls(page, t.posts[2]).locator(".select-post").click();
  // One of the two changes hands after the page was drawn, which still
  // offers the merge.
  sql("UPDATE posts SET user_id = (SELECT id FROM users WHERE username = 'user2') WHERE id = :'p1'", t.posts[2]);
  await page.locator(`${PANEL} .btn`, { hasText: "merge selected posts" }).click();
  const dialog = page.locator(".dialog-content");
  await dialog.locator(".dialog-footer .btn", { hasText: "Yes" }).click();
  await expect(dialog.locator(".dialog-body")).toHaveText("Posts belonging to different users cannot be merged.");
  // The mode went off with the confirm (toggleMultiSelect).
  await expect(page.locator(".selected-posts")).toHaveCount(0);
  expect(raws(t)).toEqual(before);
  expect(deleted(t)).toEqual([]);
  expect(owners(t)).toEqual(["user1", "user0", "user2", "user2"]);
});

test("Move to opens the MoveToTopic modal", async ({ page, context, session }) => {
  const t = await setup(session, context);
  await enter(page, t);

  await controls(page, t.posts[1]).locator(".select-post").click();
  await controls(page, t.posts[2]).locator(".select-post").click();
  await page.locator(`${PANEL} .btn.move-to-topic`).click();
  const modal = page.locator(".d-modal.choose-topic-modal");
  await expect(modal.locator(".d-modal__title-text")).toHaveText("Move to");
  await expect(page.locator("#page-error")).toBeHidden();

  // Closed, the mode and its selection are as they were.
  await modal.locator(".modal-close").click();
  await expect(modal).toHaveCount(0);
  expect(await selected(page)).toEqual([t.posts[1], t.posts[2]]);
  expect(deleted(t)).toEqual([]);
  expect(sql("SELECT COUNT(*) FROM posts WHERE topic_id = :'p1'", t.id)).toEqual([["4"]]);
  expect(owners(t)).toEqual(["user1", "user0", "user0", "user2"]);
});

test("a plain member does not get the mode", async ({ page, context, session }) => {
  const t = await setup(session, context, "user1");
  await page.goto(t.url);
  await expect(page.locator("#posts article[data-post-id]")).toHaveCount(4);
  await settled(page);
  await expect(page.locator(".topic-admin-menu-trigger")).toHaveCount(0);
  await expect(page.locator("#select-posts-config")).toHaveCount(0);
  expect(await page.evaluate(() => typeof window.Discourse.selectPosts)).toBe("undefined");
  await expect(page.locator(".selected-posts, .select-posts")).toHaveCount(0);
  await expect(article(page, t.posts[1])).not.toHaveAttribute("data-username");
});
