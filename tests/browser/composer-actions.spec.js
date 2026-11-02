// The composer actions menu (the caret on the composer's title: its items
// per action and role, what they switch the composer to, the whisper, do
// not bump and unlisted toggles, discourse-post-voting's toggle) and the
// toolbar's Build poll (discourse-poll's builder), in composer.js and
// poll-builder.js.
const { test, expect, login, api, topic, sql, label } = require("./helpers");

const trigger = (page) => page.locator("#reply-control .composer-actions-trigger");
const menu = (page) => page.locator(".composer-actions-dropdown");

// The menu's items and toggles, by id, in order.
async function menuIds(page) {
  await trigger(page).click();
  await expect(menu(page)).toBeVisible();
  const items = await menu(page).locator("[data-action-id]").evaluateAll((els) => els.map((e) => e.dataset.actionId));
  const toggles = await menu(page).locator("[data-toggle-id]").evaluateAll((els) => els.map((e) => e.dataset.toggleId));
  return { items, toggles };
}

async function closeMenu(page) {
  await page.keyboard.press("Escape");
  await expect(menu(page)).toHaveCount(0);
}

async function pick(page, id) {
  if (!(await menu(page).count())) {
    await trigger(page).click();
  }
  await menu(page).locator(`[data-action-id='${id}']`).click();
  await expect(menu(page)).toHaveCount(0);
}

async function discard(page) {
  // The discard confirmation, when there is text to lose, is accepted by
  // the test's dialog handler.
  await page.locator("#reply-control .discard-button").click();
  await expect(page.locator("#reply-control.open")).toHaveCount(0);
}

test("a member's actions menu lists each action's items and switches between them", async ({ page, context, session }) => {
  const user0 = await session("user0");
  const user2 = await session("user2");
  const t = await topic(user0, label("Composer actions topic"), label("A topic for the actions menu"), [
    { context: user2, raw: label("A reply by user2 for the actions menu") },
  ]);
  await login(context, "user1");
  page.on("dialog", (d) => d.accept());

  // Creating a topic: message and post voting, no toggles for a member.
  await page.goto("/latest");
  await page.locator("#create-topic").click();
  await expect(trigger(page)).toContainText("Create topic");
  await expect(trigger(page).locator(".composer-actions-caret")).toHaveCount(1);
  expect(await menuIds(page)).toEqual({ items: ["create_private_message", "togglePostVoting"], toggles: [] });
  await closeMenu(page);

  // To a message: the title goes along; back to a topic from there.
  const title = label("A title that goes along");
  await page.locator("#reply-title").fill(title);
  await pick(page, "create_private_message");
  await expect(page.locator("#reply-control.private-message.open")).toBeVisible();
  await expect(trigger(page)).toContainText("Create message");
  await expect(page.locator("#reply-title")).toHaveValue(title);
  expect(await menuIds(page)).toEqual({ items: ["create_topic"], toggles: [] });
  await closeMenu(page);
  await discard(page);

  // Replying to the topic: a linked topic.
  await page.goto(t.url);
  await page.locator("#topic-footer-buttons .create").click();
  await expect(trigger(page)).toContainText("Topic");
  expect(await menuIds(page)).toEqual({ items: ["reply_as_new_topic"], toggles: [] });
  await closeMenu(page);
  await discard(page);

  // Replying to a post: to the topic instead, then back to the post, the
  // text kept.
  const reply = page.locator(`article[data-post-id='${t.posts[1]}'] .post-action-menu__reply`);
  await reply.click();
  await expect(trigger(page)).toContainText("user2");
  const editor = page.locator("#reply-control .d-editor-input");
  await editor.fill("Words that follow the switches.");
  expect(await menuIds(page)).toEqual({ items: ["reply_to_topic", "reply_as_new_topic"], toggles: [] });
  await pick(page, "reply_to_topic");
  await expect(trigger(page)).toContainText("Topic");
  await expect(editor).toHaveValue("Words that follow the switches.");
  expect(await menuIds(page)).toEqual({ items: ["reply_to_post", "reply_as_new_topic"], toggles: [] });
  await expect(menu(page).locator("[data-action-id='reply_to_post']")).toContainText("Reply to a post by user2");
  await pick(page, "reply_to_post");
  await expect(trigger(page)).toContainText("user2");
  await expect(editor).toHaveValue("Words that follow the switches.");
  await discard(page);
});

test("replying as a linked topic carries the quote and links back to the post", async ({ page, context, session }) => {
  const user0 = await session("user0");
  const user2 = await session("user2");
  const topicTitle = label("Linked topic source");
  const t = await topic(user0, topicTitle, label("The topic a linked topic continues"), [
    { context: user2, raw: label("The post the linked topic quotes") },
  ]);
  await login(context, "user1");
  await page.goto(t.url);
  await page.locator(`article[data-post-id='${t.posts[1]}'] .post-action-menu__reply`).click();
  const editor = page.locator("#reply-control .d-editor-input");
  const quote = `[quote="user2, post:2, topic:${t.id}"]\nThe quoted words\n[/quote]\n\nMy own words.`;
  await editor.fill(quote);
  await pick(page, "reply_as_new_topic");

  await expect(page.locator("#reply-control.composer-action-create-topic.open")).toBeVisible();
  await expect(trigger(page)).toContainText("Create topic");
  const origin = new URL(page.url()).origin;
  await expect(editor).toHaveValue(
    `Continuing the discussion from [${topicTitle}](${origin}${t.url}/2):\n\n${quote}`
  );
  // The topic's category, and the reply's topic and post to go back to.
  await expect(page.locator("#reply-control .category-chooser summary")).toHaveAttribute("data-value", "4");
  expect(await menuIds(page)).toEqual({
    items: ["reply_to_post", "reply_to_topic", "create_private_message", "togglePostVoting"],
    toggles: [],
  });
  await closeMenu(page);
  // The reply's draft went (destroyDraft).
  expect(sql("SELECT count(*) FROM drafts d JOIN users u ON u.id = d.user_id WHERE u.username = 'user1' AND d.draft_key = :'p1'", `topic_${t.id}`)).toEqual([["0"]]);

  const newTitle = label("A linked topic");
  await page.locator("#reply-title").fill(newTitle);
  await page.locator("#reply-control .save-or-cancel .create").click();
  await expect(page.locator("#topic-title h1")).toContainText(newTitle);
  const first = page.locator(".topic-post .cooked").first();
  await expect(first).toContainText("Continuing the discussion from");
  await expect(first.locator(`a[href$='${t.url}/2']`).first()).toBeVisible();
  await expect(first.locator("aside.quote")).toBeVisible();
});

test("staff whisper from the menu, reply without bumping, and create an unlisted topic", async ({ page, context, session }) => {
  const user0 = await session("user0");
  const t = await topic(user0, label("Whisper topic"), label("A topic staff whisper in"));
  await login(context, "admin");
  await page.goto(t.url);
  await page.locator("#topic-footer-buttons .create").click();
  expect(await menuIds(page)).toEqual({ items: ["reply_as_new_topic"], toggles: ["toggle_whisper", "toggle_topic_bump"] });
  const indicator = page.locator("#reply-control .composer-whisper-indicator");
  await expect(indicator).toHaveClass(/--public/);
  await expect(indicator).toHaveText("Visible");

  await menu(page).locator("[data-toggle-id='toggle_whisper']").click();
  await expect(menu(page).locator("[data-toggle-id='toggle_whisper'] .d-toggle-switch__checkbox")).toHaveAttribute("aria-checked", "true");
  await closeMenu(page);
  await expect(indicator).toHaveClass(/--whispering/);
  await expect(indicator).toHaveText("Whispering");
  await expect(page.locator("#reply-control .save-or-cancel .create")).toHaveText("Whisper");
  await expect(trigger(page).locator(".d-icon-far-eye-slash")).toHaveCount(1);
  // The indicator toggles too.
  await indicator.click();
  await expect(indicator).toHaveClass(/--public/);
  await indicator.click();
  await expect(indicator).toHaveClass(/--whispering/);

  const whisper = label("A staff whisper");
  await page.locator("#reply-control .d-editor-input").fill(whisper);
  await page.locator("#reply-control .save-or-cancel .create").click();
  await expect(page.locator("#reply-control.open")).toHaveCount(0);
  await expect(page.locator(".topic-post .cooked", { hasText: whisper })).toBeVisible();
  expect(sql("SELECT post_type, post_number FROM posts WHERE topic_id = :'p1' AND raw = :'p2'", t.id, whisper)).toEqual([["4", "2"]]);
  expect(sql("SELECT highest_post_number, highest_staff_post_number, posts_count FROM topics WHERE id = :'p1'", t.id)).toEqual([["1", "2", "1"]]);

  // Do not bump: the reply leaves bumped_at.
  const [[bumped]] = sql("SELECT bumped_at FROM topics WHERE id = :'p1'", t.id);
  await page.locator("#topic-footer-buttons .create").click();
  await trigger(page).click();
  await menu(page).locator("[data-toggle-id='toggle_topic_bump']").click();
  await closeMenu(page);
  const quiet = label("A reply that does not bump");
  await page.locator("#reply-control .d-editor-input").fill(quiet);
  await page.locator("#reply-control .save-or-cancel .create").click();
  await expect(page.locator(".topic-post .cooked", { hasText: quiet })).toBeVisible();
  expect(sql("SELECT post_type FROM posts WHERE topic_id = :'p1' AND raw = :'p2'", t.id, quiet)).toEqual([["1"]]);
  expect(sql("SELECT bumped_at FROM topics WHERE id = :'p1'", t.id)).toEqual([[bumped]]);

  // Unlisted, from a new topic's toggles.
  await page.goto("/latest");
  await page.locator("#create-topic").click();
  const ids = await menuIds(page);
  expect(ids.toggles).toEqual(["toggle_unlisted"]);
  await menu(page).locator("[data-toggle-id='toggle_unlisted']").click();
  await closeMenu(page);
  await expect(page.locator("#reply-control .reply-to .unlist")).toHaveText("(unlisted)");
  const unlisted = label("An unlisted topic");
  await page.locator("#reply-title").fill(unlisted);
  await page.locator("#reply-control .d-editor-input").fill(label("The first post of an unlisted topic"));
  await page.locator("#reply-control .save-or-cancel .create").click();
  await expect(page.locator("#topic-title h1")).toContainText(unlisted);
  expect(sql("SELECT visible FROM topics WHERE title = :'p1'", unlisted)).toEqual([["f"]]);
});

test("a post voting topic from the actions menu draws its voting", async ({ page, context }) => {
  await login(context, "user1");
  await page.goto("/latest");
  await page.locator("#create-topic").click();
  await pick(page, "togglePostVoting");
  await expect(trigger(page)).toContainText("Create post voting topic");
  await expect(page.locator("#reply-control .save-or-cancel .create")).toHaveText("Create post voting topic");
  await trigger(page).click();
  await expect(menu(page).locator("[data-action-id='togglePostVoting']")).toContainText("Remove post voting");
  await closeMenu(page);

  const title = label("A post voting question");
  await page.locator("#reply-title").fill(title);
  await page.locator("#reply-control .d-editor-input").fill(label("Which of these answers is best?"));
  await page.locator("#reply-control .save-or-cancel .create").click();
  await expect(page.locator("#topic-title h1")).toContainText(title);
  expect(sql("SELECT subtype FROM topics WHERE title = :'p1'", title)).toEqual([["question_answer"]]);
  await expect(page.locator(".topic-post").first().locator(".post-voting-post")).toBeVisible();
  // The reply button answers.
  await page.locator("#topic-footer-buttons .create").click();
  await expect(trigger(page)).toContainText("Answer");
  await expect(page.locator("#reply-control .save-or-cancel .create")).toHaveText("Answer");
});

test("Build poll inserts the poll it builds, which draws once posted", async ({ page, context }) => {
  await login(context, "user1");
  await page.goto("/latest");
  await page.locator("#create-topic").click();
  const editor = page.locator("#reply-control .d-editor-input");
  await editor.fill("Pick your languages.\n\n");

  await page.locator("#reply-control .toolbar-menu__options-trigger").click();
  await page.locator(".toolbar-menu__options-content button[title='Build poll']").click();
  const modal = page.locator(".d-modal.poll-ui-builder");
  await expect(modal).toBeVisible();
  const insert = modal.locator(".insert-poll");
  await expect(insert).toBeDisabled();
  await expect(modal.locator(".poll-type-value-regular")).toHaveClass(/active/);
  // Enter adds the next option.
  await modal.locator(".poll-option-value input").first().fill("Rust");
  await modal.locator(".poll-option-value input").first().press("Enter");
  await expect(modal.locator(".poll-option-value input")).toHaveCount(2);
  await expect(modal.locator(".poll-option-value input").nth(1)).toBeFocused();
  await modal.locator(".poll-option-value input").nth(1).fill("Ruby");
  await modal.locator(".poll-option-add").click();
  await modal.locator(".poll-option-value input").nth(2).fill("Both");
  await expect(insert).toBeEnabled();

  // Multiple choice: min and max, bounded by the options.
  await modal.locator(".poll-type-value-multiple").click();
  await expect(modal.locator(".poll-options-min")).toHaveValue("1");
  await expect(modal.locator(".poll-options-max")).toHaveValue("2");
  // The options bound the max (enforceMinMaxValues).
  await modal.locator(".poll-options-max").fill("5");
  await expect(modal.locator(".poll-options-max")).toHaveValue("3");
  await modal.locator(".poll-options-max").fill("2");
  await expect(insert).toBeEnabled();
  await insert.click();
  await expect(modal).toHaveCount(0);
  await expect(editor).toHaveValue(
    "Pick your languages.\n\n[poll type=multiple results=always min=1 max=2 public=true chartType=bar]\n* Rust\n* Ruby\n* Both\n[/poll]\n"
  );

  // Advanced mode: a number rating with results after voting, named
  // after the poll already there.
  await page.locator("#reply-control .toolbar-menu__options-trigger").click();
  await page.locator(".toolbar-menu__options-content button[title='Build poll']").click();
  await expect(modal).toBeVisible();
  await modal.locator(".advanced-mode-btn").click();
  await expect(modal.locator(".poll-title input")).toBeVisible();
  await expect(modal.locator(".poll-options textarea")).toBeVisible();
  await modal.locator(".poll-type-value-number").click();
  await expect(modal.locator(".poll-options")).toHaveCount(0);
  await expect(modal.locator(".poll-options-max")).toHaveValue("20");
  await modal.locator(".poll-options-max").fill("50");
  await expect(modal.locator(".tip.bad")).toContainText("Enter at most 20 options.");
  await expect(modal.locator(".insert-poll")).toBeDisabled();
  await modal.locator(".poll-options-max").fill("10");
  await expect(modal.locator(".tip.bad")).toHaveCount(0);
  await modal.locator(".poll-title input").fill("Rate it");
  await modal.locator(".poll-result").selectOption("on_vote");
  await expect(modal.locator("input[name='poll-chart-type']")).toHaveCount(0);
  await modal.locator(".insert-poll").click();
  await expect(editor).toHaveValue(
    /\[poll name=poll2 type=number results=on_vote min=1 max=10 step=1 public=true\]\n# Rate it\n\[\/poll\]\n$/
  );

  const title = label("A topic with built polls");
  await page.locator("#reply-title").fill(title);
  await page.locator("#reply-control .save-or-cancel .create").click();
  await expect(page.locator("#topic-title h1")).toContainText(title);
  await expect(page.locator(".cooked .poll")).toHaveCount(2);
  await expect(page.locator(".cooked .poll").first().locator("li[data-poll-option-id]")).toHaveCount(3);
  expect(
    sql(
      "SELECT p.name, p.type, p.min, p.max, p.results FROM polls p JOIN posts ps ON ps.id = p.post_id JOIN topics t ON t.id = ps.topic_id WHERE t.title = :'p1' ORDER BY p.id",
      title
    )
  ).toEqual([
    ["poll", "1", "1", "2", "0"],
    ["poll2", "2", "1", "10", "1"],
  ]);
});

test("a reply in a group message starts a new message to the same people", async ({ page, context }) => {
  await login(context, "user1");
  const title = label("A message to two");
  const pm = await api(context, "POST", "/posts.json", {
    title,
    raw: label("A message user1 sends to user0 and user2"),
    archetype: "private_message",
    target_recipients: "user0,user2",
  });
  await page.goto(`/t/${pm.topic_slug}/${pm.topic_id}`);
  await page.locator("#topic-footer-buttons .create").click();
  await expect(trigger(page)).toContainText("Message");
  expect(await menuIds(page)).toEqual({ items: ["reply_as_new_group_message"], toggles: [] });
  await closeMenu(page);
  await page.locator("#reply-control .d-editor-input").fill("Starting over with everyone.");
  await pick(page, "reply_as_new_group_message");

  await expect(page.locator("#reply-control.private-message.open")).toBeVisible();
  await expect(trigger(page)).toContainText("Create message");
  const to = await page.locator("#private-message-users summary").getAttribute("data-value");
  expect(to.split(",").sort()).toEqual(["user0", "user1", "user2"]);
  const origin = new URL(page.url()).origin;
  await expect(page.locator("#reply-control .d-editor-input")).toHaveValue(
    `Continuing the discussion from [${title}](${origin}/t/${pm.topic_slug}/${pm.topic_id}):\n\nStarting over with everyone.`
  );
});
