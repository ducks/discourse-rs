// The MoveToTopic modal (select-posts.js), opened by Move to in the
// multi-select mode's SelectedPosts: the destinations the selection and
// the viewer are offered (a new topic, an existing topic, for an admin a
// new message; for a message a new or an existing message), each with its
// form, the button waiting for what its destination needs, the request
// Topic.movePosts or mergeTopic makes and the destination topic the page
// then goes to. Each move is checked in the database.
const { test, expect, login, api, topic, sql, label, member } = require("./helpers");

const MENU = ".topic-admin-menu-content";
const PANEL = ".container.posts > .selected-posts";
const MODAL = ".d-modal.choose-topic-modal";

// A topic by user1 with three replies by user0.
async function source(session, what = "Move posts") {
  const user1 = await session("user1");
  const user0 = await session("user0");
  return topic(
    user1,
    label(what),
    label("A topic for the move to topic spec"),
    [1, 2, 3].map((n) => ({ context: user0, raw: label(`Reply ${n} by user0 for the move to topic spec`) })),
  );
}

// As select-posts.spec.js: the page's scroll and height at rest before
// the menu opens.
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

// The topic with the mode on and `ids` selected ("all": select all),
// then Move to.
async function open(page, t, ids) {
  await page.goto(t.url);
  await expect(page.locator("#posts article[data-post-id]")).toHaveCount(t.posts.length);
  await settled(page);
  await page.locator(".topic-admin-menu-trigger:visible").first().click();
  await page.locator(`${MENU} .topic-admin-multi-select .btn`).click();
  await expect(page.locator(PANEL)).toBeVisible();
  if (ids === "all") {
    await page.locator(`${PANEL} a.select-all`).click();
  } else {
    for (const id of ids) {
      await page.locator(`article[data-post-id="${id}"] .select-posts .select-post`).click();
    }
  }
  await page.locator(`${PANEL} .btn.move-to-topic`).click();
  await expect(page.locator(MODAL)).toBeVisible();
  return page.locator(MODAL);
}

function radios(modal) {
  return modal.locator(".radios .radio-label").allTextContents();
}

// The topic of each post, in the order given.
function topicsOf(posts) {
  return posts.map((id) => Number(sql("SELECT topic_id FROM posts WHERE id = :'p1'", id)[0][0]));
}

function topicRow(id) {
  return sql("SELECT title, category_id, archetype FROM topics WHERE id = :'p1'", id)[0];
}

// The small action a move leaves in the topic the posts left.
function smallActions(t) {
  return sql("SELECT action_code FROM posts WHERE topic_id = :'p1' AND post_type = 3 ORDER BY id", t.id).map(
    (r) => r[0],
  );
}

function form(request) {
  return decodeURIComponent(request.postData().replace(/\+/g, " "));
}

test("selected posts move to a new topic with its title and category", async ({ page, context, session }) => {
  const t = await source(session);
  await login(context, "admin");
  const modal = await open(page, t, [t.posts[1], t.posts[2]]);
  const submit = modal.locator(".d-modal__footer .btn-primary");

  await expect(modal).toHaveAttribute("id", "choosing-topic");
  await expect(modal.locator(".d-modal__title-text")).toHaveText("Move to");
  expect(await radios(modal)).toEqual(["New Topic", "Existing Topic", "New Message"]);
  await expect(modal.locator("#move-to-new-topic")).toBeChecked();
  expect(await modal.locator(".d-modal__body > p").innerHTML()).toBe(
    "You are about to create a new topic and populate it with the <b>2</b> posts you've selected.",
  );
  await expect(modal.locator("form.split-new-topic-form .control-group > label").first()).toHaveText(
    "New Topic Title",
  );
  await expect(modal.locator("#split-topic-name")).toHaveAttribute("placeholder", /.+/);
  await expect(submit.locator(".d-icon-right-from-bracket")).toHaveCount(1);
  await expect(submit).toHaveText("Move to New Topic");
  // Nothing to move to until it has a title.
  await expect(submit).toBeDisabled();

  const title = label("Posts moved to a new topic");
  await modal.locator("#split-topic-name").fill(title);
  await expect(submit).toBeEnabled();
  await modal.locator("#split-topic-name").fill("");
  await expect(submit).toBeDisabled();
  await modal.locator("#split-topic-name").fill(title);

  // A category other than the topic's.
  const chooser = modal.locator("details.category-chooser.small");
  await chooser.locator("summary").click();
  const row = chooser.locator('.category-row:not([data-value="4"])').first();
  const category = await row.getAttribute("data-value");
  await row.click();
  await expect(chooser).toHaveClass(/has-selection/);

  const moved = page.waitForRequest((r) => r.url().endsWith(`/t/${t.id}/move-posts`));
  await submit.click();
  const request = await moved;
  expect(request.method()).toBe("POST");
  expect(form(request)).toBe(
    `title=${title}&post_ids[]=${t.posts[1]}&post_ids[]=${t.posts[2]}&category_id=${category}`,
  );
  const result = await (await request.response()).json();
  expect(result.success).toBe(true);

  // The page goes to the new topic.
  const [first, second] = topicsOf([t.posts[1], t.posts[2]]);
  expect(first).not.toBe(t.id);
  expect(second).toBe(first);
  await page.waitForURL((url) => url.pathname.endsWith(`/${first}`));
  expect(new URL(page.url()).pathname).toBe(result.url);
  await expect(page.locator("#posts article[data-post-id]")).toHaveCount(2);
  await expect(page.locator(".selected-posts, .d-modal")).toHaveCount(0);
  expect(topicRow(first)).toEqual([title, category, "regular"]);
  expect(topicsOf([t.posts[0], t.posts[3]])).toEqual([t.id, t.id]);
  expect(smallActions(t)).toEqual(["split_topic"]);
});

test("selected posts move to an existing topic chosen by search", async ({ page, context, session }) => {
  const marker = label("Destination");
  const destination = await topic(
    await session("user2"),
    `${marker} of moved posts`,
    label("A topic the move to topic spec moves posts into"),
  );
  const t = await source(session);
  await login(context, "admin");
  const modal = await open(page, t, [t.posts[2], t.posts[3]]);
  const submit = modal.locator(".d-modal__footer .btn-primary");

  await modal.locator("#move-to-existing-topic").check();
  expect(await modal.locator(".d-modal__body > p").innerHTML()).toBe(
    "Please choose the topic you'd like to move those <b>2</b> posts to.",
  );
  await expect(submit).toHaveText("Move to Existing Topic");
  await expect(submit).toBeDisabled();
  await expect(modal.locator("#split-topic-name")).toHaveCount(0);
  await expect(modal.locator(".choose-topic > label")).toHaveText("Search for a Topic");
  await expect(modal.locator("#chronological-order")).toHaveCount(0);

  const input = modal.locator("#choose-topic-title");
  await expect(input).toHaveAttribute("placeholder", "type the topic title, url or id here");
  // Nothing by that title.
  await input.fill(label("nothing is titled"));
  await expect(modal.locator(".choose-topic__search-results")).toHaveText("No topics found.");
  await expect(submit).toBeDisabled();

  // By its title, once the search index has it; the one result is
  // chosen.
  const choice = modal.locator(`.choose-topic-list .existing-topic #choose-topic-${destination.id}`);
  await expect(async () => {
    await input.fill("");
    await input.fill(marker);
    await expect(choice).toBeVisible({ timeout: 2000 });
  }).toPass();
  await expect(choice).toBeChecked();
  await expect(modal.locator(".existing-topic .topic-title")).toHaveText(`${marker} of moved posts`);
  await expect(modal.locator(".existing-topic .topic-categories .badge-category__wrapper")).toHaveCount(1);
  await expect(submit).toBeEnabled();

  // A new search drops the choice; the topic's own id finds nothing.
  await input.fill(String(t.id));
  await expect(submit).toBeDisabled();
  await expect(modal.locator(".choose-topic__search-results")).toHaveText("No topics found.");
  await expect(modal.locator("#chronological-order")).toHaveCount(0);

  // By its id.
  await input.fill(String(destination.id));
  await expect(choice).toBeChecked();
  await expect(submit).toBeEnabled();
  await expect(modal.locator("form > hr")).toHaveCount(1);
  await expect(modal.locator("label[for='chronological-order']")).toHaveText(
    "preserve chronological order after merging",
  );
  await modal.locator("#chronological-order").check();

  const moved = page.waitForRequest((r) => r.url().endsWith(`/t/${t.id}/move-posts`));
  await submit.click();
  expect(form(await moved)).toBe(
    `post_ids[]=${t.posts[2]}&post_ids[]=${t.posts[3]}&destination_topic_id=${destination.id}&chronological_order=true`,
  );
  await page.waitForURL((url) => url.pathname.endsWith(`/${destination.id}`));
  await expect(page.locator("#posts article[data-post-id]")).toHaveCount(3);
  expect(topicsOf(t.posts)).toEqual([t.id, t.id, destination.id, destination.id]);
  expect(smallActions(t)).toEqual(["split_topic"]);
});

test("an admin moves selected posts to a new message", async ({ page, context, session }) => {
  const t = await source(session);
  await login(context, "admin");
  const modal = await open(page, t, [t.posts[1]]);
  const submit = modal.locator(".d-modal__footer .btn-primary");

  // The title typed for a new topic is the new message's.
  const title = label("A post moved to a new message");
  await modal.locator("#split-topic-name").fill(title);
  await modal.locator("#move-to-new-message").check();
  expect(await modal.locator(".d-modal__body > p").innerHTML()).toBe(
    "You are about to create a new message and populate it with the post you've selected.",
  );
  await expect(modal.locator("form > label").first()).toHaveText("New Message Title");
  await expect(modal.locator("#split-topic-name")).toHaveValue(title);
  await expect(modal.locator(".category-chooser")).toHaveCount(0);
  await expect(submit).toHaveText("Move to New Message");
  await expect(submit).toBeEnabled();

  const moved = page.waitForRequest((r) => r.url().endsWith(`/t/${t.id}/move-posts`));
  await submit.click();
  const request = await moved;
  expect(form(request)).toBe(`title=${title}&post_ids[]=${t.posts[1]}&archetype=private_message`);
  await request.response();
  const [message] = topicsOf([t.posts[1]]);
  expect(message).not.toBe(t.id);
  await page.waitForURL((url) => url.pathname.endsWith(`/${message}`));
  await expect(page.locator("#posts article[data-post-id]")).toHaveCount(1);
  expect(topicRow(message)).toEqual([title, "", "private_message"]);
  expect(topicsOf([t.posts[0], t.posts[2], t.posts[3]])).toEqual([t.id, t.id, t.id]);
});

test("every post selected merges the topic into an existing one", async ({ page, context, session }) => {
  const destination = await topic(
    await session("user2"),
    label("Merged into"),
    label("A topic the move to topic spec merges a topic into"),
  );
  const t = await source(session, "Merged away");
  await login(context, "admin");
  const modal = await open(page, t, "all");
  const submit = modal.locator(".d-modal__footer .btn-primary");

  // Nothing to split off: an existing topic is the one destination.
  expect(await radios(modal)).toEqual(["Existing Topic"]);
  await expect(modal.locator("#move-to-existing-topic")).toBeChecked();
  expect(await modal.locator(".d-modal__body > p").innerHTML()).toBe(
    "Please choose the topic you'd like to move those <b>4</b> posts to.",
  );
  await expect(submit).toHaveText("Move to Existing Topic");
  await expect(submit).toBeDisabled();
  await modal.locator("#choose-topic-title").fill(String(destination.id));
  await expect(modal.locator(`#choose-topic-${destination.id}`)).toBeChecked();

  // Topic.mergeTopic: the destination, no post ids.
  const merged = page.waitForRequest((r) => r.url().endsWith(`/t/${t.id}/merge-topic`));
  await submit.click();
  expect(form(await merged)).toBe(`destination_topic_id=${destination.id}&chronological_order=false`);
  await page.waitForURL((url) => url.pathname.endsWith(`/${destination.id}`));
  await expect(page.locator("#posts article[data-post-id]")).toHaveCount(5);
  // The replies moved; the first post is copied over and stays behind
  // (PostMover).
  expect(topicsOf(t.posts)).toEqual([t.id, destination.id, destination.id, destination.id]);
  expect(sql("SELECT COUNT(*) FROM posts WHERE topic_id = :'p1' AND post_type = 1", destination.id)).toEqual([["5"]]);
  expect(topicRow(destination.id)[2]).toBe("regular");
});

test("what the server refuses is the modal's flash", async ({ page, context, session }) => {
  const t = await source(session);
  await login(context, "admin");
  const modal = await open(page, t, [t.posts[1], t.posts[2]]);
  const submit = modal.locator(".d-modal__footer .btn-primary");
  await expect(modal.locator("#modal-alert")).toHaveCount(0);

  // A title too short for a topic.
  await modal.locator("#split-topic-name").fill("x");
  const refused = page.waitForResponse((r) => r.url().endsWith(`/t/${t.id}/move-posts`));
  await submit.click();
  const response = await refused;
  expect(response.status()).toBe(422);
  const { errors } = await response.json();
  expect(errors.length).toBeGreaterThan(0);
  const flash = modal.locator("#modal-alert.alert.alert-error[role='alert']");
  await expect(flash).toHaveText(
    errors.length === 1
      ? `An error occurred: ${errors[0]}`
      : `Multiple errors occurred: ${errors.map((e, i) => `${i + 1}) ${e}`).join(" ")}`,
  );
  // Before the body, the form as it was and the button back.
  expect(await flash.evaluate((el) => el.nextElementSibling.className)).toContain("d-modal__body");
  await expect(modal.locator("#split-topic-name")).toHaveValue("x");
  await expect(submit).toBeEnabled();
  await expect(page.locator(PANEL)).toBeVisible();
  expect(topicsOf(t.posts)).toEqual(t.posts.map(() => t.id));
  expect(smallActions(t)).toEqual([]);

  // The next try, with a title and a category, clears it.
  const title = label("Moved after a refusal");
  await modal.locator("#split-topic-name").fill(title);
  await modal.locator("details.category-chooser summary").click();
  await modal.locator('.category-row[data-value="4"]').click();
  const accepted = page.waitForResponse((r) => r.url().endsWith(`/t/${t.id}/move-posts`));
  await submit.click();
  expect((await (await accepted).json()).success).toBe(true);
  await page.waitForURL((url) => !url.pathname.endsWith(`/${t.id}`));
  const [moved] = topicsOf([t.posts[1]]);
  expect(topicRow(moved)[0]).toBe(title);
});

test("the destinations follow the viewer and the selection", async ({ page, context, session }) => {
  const t = await source(session);
  // A moderator who is no admin: no new message.
  const moderator = member();
  sql("UPDATE users SET moderator = true WHERE id = :'p1'", moderator.id);
  await login(context, moderator.name);
  let modal = await open(page, t, [t.posts[1]]);
  expect(await radios(modal)).toEqual(["New Topic", "Existing Topic"]);
  await expect(modal.locator("#move-to-new-topic")).toBeChecked();
  await modal.locator(".modal-close").click();
  await expect(modal).toHaveCount(0);
  // The selection is as it was.
  await expect(page.locator(`${PANEL} p`).first()).toHaveText("You have selected 1 post.");

  // A selection starting with a moderator action is no new topic's
  // first post.
  sql("UPDATE posts SET post_type = 2 WHERE id = :'p1'", t.posts[1]);
  modal = await open(page, t, [t.posts[2], t.posts[1]]);
  expect(await radios(modal)).toEqual(["Existing Topic"]);
  await expect(modal.locator("#move-to-existing-topic")).toBeChecked();
  await expect(modal.locator(".d-modal__footer .btn-primary")).toHaveText("Move to Existing Topic");
  await expect(modal.locator("#split-topic-name")).toHaveCount(0);
  expect(topicsOf(t.posts)).toEqual(t.posts.map(() => t.id));
});

test("a message's posts move to an existing message", async ({ page, context, session }) => {
  const user1 = await session("user1");
  const user0 = await session("user0");
  const message = async (title, replies) => {
    const first = await api(user1, "POST", "/posts.json", {
      title: label(title),
      raw: label("A message for the move to topic spec"),
      archetype: "private_message",
      target_recipients: "user0,admin",
    });
    const posts = [first.id];
    for (let n = 1; n <= replies; n++) {
      const reply = await api(user0, "POST", "/posts.json", {
        topic_id: String(first.topic_id),
        raw: label(`Reply ${n} in a message for the move to topic spec`),
      });
      posts.push(reply.id);
    }
    return { id: first.topic_id, url: `/t/${first.topic_slug}/${first.topic_id}`, posts };
  };
  const destination = await message("A message moved into", 0);
  const t = await message("A message moved from", 2);
  await login(context, "admin");
  const modal = await open(page, t, [t.posts[1]]);
  const submit = modal.locator(".d-modal__footer .btn-primary");

  // A message's destinations are messages, the new one first.
  expect(await radios(modal)).toEqual(["New Message", "Existing Message"]);
  await expect(modal.locator("#move-to-new-message")).toBeChecked();
  await expect(submit).toHaveText("Move to New Message");
  await expect(submit).toBeDisabled();

  await modal.locator("#move-to-existing-message").check();
  expect(await modal.locator(".d-modal__body > p").innerHTML()).toBe(
    "Please choose the message you'd like to move that post to.",
  );
  await expect(submit).toHaveText("Move to Existing Message");
  await expect(submit).toBeDisabled();
  await expect(modal.locator(".choose-message > label")).toHaveText("Search for a Message");
  await expect(modal.locator("#choose-message-title")).toHaveAttribute(
    "placeholder",
    "type the message title, url or id here",
  );
  await expect(modal.locator("form > label").first()).toHaveText("Participants");
  await expect(modal.locator("details.email-group-user-chooser.participant-selector")).toHaveCount(1);

  await modal.locator("#choose-message-title").fill(String(destination.id));
  const choice = modal.locator(".choose-message__search-results > .existing-message input[name='choose_message_id']");
  await expect(choice).toHaveCount(1);
  await expect(choice).toBeChecked();
  await expect(modal.locator("#chronological-order")).toHaveCount(1);
  await expect(submit).toBeEnabled();

  // A participant the destination does not have yet.
  const chooser = modal.locator("details.participant-selector");
  await chooser.locator("summary").click();
  await chooser.locator(".filter-input").fill("user2");
  await chooser.locator('.select-kit-row[data-value="user2"]').click();
  await modal.locator(".d-modal__title-text").click();

  const moved = page.waitForRequest((r) => r.url().endsWith(`/t/${t.id}/move-posts`));
  await submit.click();
  expect(form(await moved)).toBe(
    `post_ids[]=${t.posts[1]}&destination_topic_id=${destination.id}&participants=user2&archetype=private_message&chronological_order=false`,
  );
  await page.waitForURL((url) => url.pathname.endsWith(`/${destination.id}`));
  await expect(page.locator("#posts article[data-post-id]")).toHaveCount(2);
  expect(topicsOf(t.posts)).toEqual([t.id, destination.id, t.id]);
  expect(
    sql(
      "SELECT COUNT(*) FROM topic_allowed_users a JOIN users u ON u.id = a.user_id WHERE a.topic_id = :'p1' AND u.username = 'user2'",
      destination.id,
    ),
  ).toEqual([["1"]]);
  expect(topicRow(destination.id)[2]).toBe("private_message");
});

test("a new topic takes the tags chosen", async ({ page, context, session }) => {
  const name = label("moved").replace(/ /g, "-").slice(0, 20);
  const [[tag]] = sql(
    "INSERT INTO tags (name, slug, created_at, updated_at) VALUES (:'p1', :'p1', now(), now()) RETURNING id",
    name,
  );
  const t = await source(session);
  await login(context, "admin");
  const modal = await open(page, t, [t.posts[3]]);
  const submit = modal.locator(".d-modal__footer .btn-primary");

  // The tags after the category, searched as the composer's are.
  await expect(modal.locator("form.split-new-topic-form .control-group > label")).toHaveText([
    "New Topic Title",
    "Category",
    "Tags",
  ]);
  const title = label("A post moved with a tag");
  await modal.locator("#split-topic-name").fill(title);
  await modal.locator("details.category-chooser summary").click();
  await modal.locator('.category-row[data-value="4"]').click();
  const chooser = modal.locator("details.mini-tag-chooser");
  await chooser.locator("summary").click();
  await chooser.locator(".filter-input").fill(name);
  await chooser.locator(`.select-kit-row[data-name="${name}"], .select-kit-row[data-value="${name}"]`).first().click();
  await expect(chooser.locator("summary")).toContainText(name);
  await modal.locator(".d-modal__title-text").click();

  const moved = page.waitForRequest((r) => r.url().endsWith(`/t/${t.id}/move-posts`));
  await submit.click();
  const request = await moved;
  expect(form(request)).toBe(`title=${title}&post_ids[]=${t.posts[3]}&category_id=4&tag_ids[]=${tag}`);
  expect((await (await request.response()).json()).success).toBe(true);
  const [topicId] = topicsOf([t.posts[3]]);
  expect(sql("SELECT tag_id FROM topic_tags WHERE topic_id = :'p1'", topicId)).toEqual([[tag]]);
});
