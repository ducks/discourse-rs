// The composer actions menu (the caret on the composer's title: its items
// per action and role, what they switch the composer to, the whisper, do
// not bump and unlisted toggles, discourse-post-voting's toggle) and the
// toolbar's Build poll (discourse-poll's builder, with its group chooser,
// date and time input and results combo box, and the item's condition in
// a PM with a non-human user), in composer.js and poll-builder.js.
const { test, expect, login, api, topic, sql, label, member } = require("./helpers");

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
  // A member of its own: a repeated run would post the same body as user1
  // within unique_posts_mins.
  await login(context, member().name);
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
  await modal.locator(".poll-result .select-kit-header").click();
  await modal.locator(".poll-result .select-kit-row[data-value='on_vote']").click();
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

// The builder's advanced fields as Ember draws them: GroupChooser,
// DDateTimeInput (DDateInput and DTimeInput) and the results ComboBox.
test("Build poll's group chooser, automatic close and results pick what the poll gets", async ({ page, context }) => {
  await login(context, "user1");
  await page.goto("/latest");
  await page.locator("#create-topic").click();
  const editor = page.locator("#reply-control .d-editor-input");
  await page.locator("#reply-control .toolbar-menu__options-trigger").click();
  await page.locator(".toolbar-menu__options-content button[title='Build poll']").click();
  const modal = page.locator(".d-modal.poll-ui-builder");
  await expect(modal).toBeVisible();
  await modal.locator(".advanced-mode-btn").click();
  await modal.locator(".poll-options textarea").fill("Yes\nNo");

  // GroupChooser: every group but everyone, none chosen.
  const groups = modal.locator(".poll-allowed-groups details.select-kit.multi-select.group-chooser");
  await expect(groups.locator(".formatted-selection")).toHaveText("Select…");
  await groups.locator(".select-kit-header").click();
  await expect(groups.locator(".select-kit-row")).toHaveCount(10);
  await expect(groups.locator(".select-kit-row[data-value='everyone']")).toHaveCount(0);
  const filter = groups.locator(".filter-input");
  await expect(filter).toBeFocused();
  await filter.fill("trust_level");
  await expect(groups.locator(".select-kit-row")).toHaveCount(5);
  await expect(groups.locator(".select-kit-row").first()).toHaveClass(/is-highlighted/);
  await expect(groups.locator(".results-count")).toHaveText("5 results");
  // A row chosen joins the choices and the menu stays open, without it.
  await groups.locator(".select-kit-row[data-value='trust_level_1']").click();
  await expect(groups).toHaveAttribute("open", "");
  await expect(groups.locator(".selected-choice")).toHaveText(["trust_level_1"]);
  await expect(groups.locator(".select-kit-row")).toHaveCount(9);
  await expect(filter).toHaveValue("");
  // Enter takes the highlighted row.
  await filter.fill("level_2");
  await filter.press("Enter");
  await expect(groups.locator(".selected-choice")).toHaveText(["trust_level_1", "trust_level_2"]);
  await expect(groups.locator(".formatted-selection")).toHaveText("trust_level_1, trust_level_2");
  await filter.fill("nothing like it");
  await expect(groups.locator(".no-content")).toHaveText("No matches found");
  // A choice clicked leaves; Escape closes the chooser, not the modal.
  await groups.locator(".selected-choice[data-value='trust_level_1']").click();
  await expect(groups.locator(".selected-choice")).toHaveText(["trust_level_2"]);
  await page.keyboard.press("Escape");
  await expect(groups).not.toHaveAttribute("open", "");
  await expect(modal).toBeVisible();
  await expect(groups.locator(".formatted-selection")).toHaveText("trust_level_2");

  // DDateTimeInput: a time without a date is today's; clearing empties
  // both; a date keeps the time chosen.
  const close = modal.locator(".poll-date .d-date-time-input");
  const date = close.locator(".d-date-input input.date-picker");
  const time = close.locator(".d-time-input details.combo-box");
  await expect(date).toHaveValue("");
  await expect(time.locator(".select-kit-selected-name")).toHaveText("--:--");
  await time.locator(".select-kit-header").click();
  await expect(time.locator(".select-kit-row")).toHaveCount(96);
  await expect(time.locator(".filter-input")).toHaveAttribute("placeholder", "--:--");
  await time.locator(".select-kit-row[data-name='2:30 PM']").click();
  await expect(time.locator(".select-kit-selected-name")).toHaveText("2:30 PM");
  const today = await page.evaluate(() => {
    const d = new Date();
    return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
  });
  await expect(date).toHaveValue(today);
  await close.locator(".clear-date-time").click();
  await expect(date).toHaveValue("");
  await expect(time.locator(".select-kit-selected-name")).toHaveText("--:--");
  await date.fill("2030-01-15");
  await expect(time.locator(".select-kit-selected-name")).toHaveText("12:00 AM");
  // allowAny: a typed time is offered as its own row.
  await time.locator(".select-kit-header").click();
  await time.locator(".filter-input").fill("2:35 pm");
  await expect(time.locator(".select-kit-row.create")).toHaveText("Create: '2:35 pm'");
  await time.locator(".filter-input").press("Enter");
  await expect(time.locator(".select-kit-selected-name")).toHaveText("2:35 PM");
  await expect(date).toHaveValue("2030-01-15");
  // The typed time joins the options.
  await time.locator(".select-kit-header").click();
  await expect(time.locator(".select-kit-row")).toHaveCount(97);
  await expect(time.locator(".select-kit-row.is-selected")).toHaveText("2:35 PM");
  await page.keyboard.press("Escape");
  await expect(modal).toBeVisible();

  // The results ComboBox: a member's choices, without staff only.
  const results = modal.locator("details.combo-box.poll-result");
  await expect(results.locator(".select-kit-selected-name")).toHaveText("Always visible");
  await results.locator(".select-kit-header").click();
  await expect(results.locator(".select-kit-row")).toHaveCount(3);
  await results.locator(".select-kit-row[data-value='on_close']").click();
  await expect(results).not.toHaveAttribute("open", "");
  await expect(results.locator(".select-kit-header")).toHaveAttribute("data-value", "on_close");

  const closeAt = await page.evaluate(() => new Date(2030, 0, 15, 14, 35).toISOString());
  await modal.locator(".insert-poll").click();
  await expect(modal).toHaveCount(0);
  await expect(editor).toHaveValue(
    `[poll type=regular results=on_close public=true chartType=bar groups=trust_level_2 close=${closeAt}]\n* Yes\n* No\n[/poll]\n`
  );
});

// add-poll-ui-builder's condition: a member who may not create polls
// sees Build poll only replying in a PM with a non-human user.
test("Build poll shows in a PM with a non-human user for a member who may not create polls", async ({ page, context, session }) => {
  const admin = await session("admin");
  await api(admin, "PUT", "/admin/site_settings/poll_create_allowed_groups", {
    poll_create_allowed_groups: "14",
  });
  try {
    // A member of its own: user1 may have posted the same poll within
    // unique_posts_mins, which refuses a repeated body.
    await login(context, member().name);
    const options = page.locator(".toolbar-menu__options-content");
    const buildPoll = options.locator("button[title='Build poll']");

    // A new topic: no item.
    await page.goto("/latest");
    await page.locator("#create-topic").click();
    await page.locator("#reply-control .toolbar-menu__options-trigger").click();
    await expect(options).toBeVisible();
    await expect(buildPoll).toHaveCount(0);
    await page.keyboard.press("Escape");
    await discard(page);

    // A message with another member: no item.
    const human = await api(context, "POST", "/posts.json", {
      title: label("A message to a person"),
      raw: label("A message user1 sends to user2"),
      archetype: "private_message",
      target_recipients: "user2",
    });
    await page.goto(`/t/${human.topic_slug}/${human.topic_id}`);
    await page.locator("#topic-footer-buttons .create").click();
    await expect(trigger(page)).toContainText("Message");
    await page.locator("#reply-control .toolbar-menu__options-trigger").click();
    await expect(options).toBeVisible();
    await expect(buildPoll).toHaveCount(0);
    await page.keyboard.press("Escape");
    await discard(page);

    // A message with discobot alone: the item, and a poll it builds posts.
    const bot = await api(context, "POST", "/posts.json", {
      title: label("A message to the bot"),
      raw: label("A message user1 sends to discobot"),
      archetype: "private_message",
      target_recipients: "discobot",
    });
    await page.goto(`/t/${bot.topic_slug}/${bot.topic_id}`);
    await page.locator("#topic-footer-buttons .create").click();
    await expect(trigger(page)).toContainText("Message");
    await page.locator("#reply-control .toolbar-menu__options-trigger").click();
    await expect(buildPoll).toBeVisible();
    await buildPoll.click();
    const modal = page.locator(".d-modal.poll-ui-builder");
    await expect(modal).toBeVisible();
    await modal.locator(".poll-option-value input").first().fill(label("Tea"));
    await modal.locator(".poll-option-add").click();
    await modal.locator(".poll-option-value input").nth(1).fill(label("Coffee"));
    await modal.locator(".insert-poll").click();
    await expect(modal).toHaveCount(0);
    await page.locator("#reply-control .save-or-cancel .create").click();
    await expect(page.locator("#reply-control.open")).toHaveCount(0);
    await expect(page.locator(".topic-post .cooked .poll li[data-poll-option-id]")).toHaveCount(2);
  } finally {
    await api(admin, "PUT", "/admin/site_settings/poll_create_allowed_groups", {
      poll_create_allowed_groups: "11",
    });
  }
});

// The rich editor's Edit poll (196r4): the builder over a poll's settings,
// its options and title left to the document, as poll-ui-builder-test's
// editing cases render it with a model of { poll, onSave }.
test.describe("Edit poll in the builder", () => {
  // The builder shown with the composer's config over `poll`; resolves to
  // what Save hands onSave.
  async function editPoll(page, poll, before) {
    await page.evaluate(
      async ({ poll }) => {
        const control = document.getElementById("reply-control");
        const config = JSON.parse(control.dataset.pollBuilder);
        if (!window.Discourse.pollBuilder) {
          await new Promise((resolve, reject) => {
            const script = document.createElement("script");
            script.src = config.src;
            script.onload = resolve;
            script.onerror = reject;
            document.head.appendChild(script);
          });
        }
        window.savedPolls = [];
        window.Discourse.pollBuilder.show(
          config,
          { getText: () => "", addText: () => {} },
          { poll, onSave: (attrs) => window.savedPolls.push(attrs) }
        );
      },
      { poll }
    );
    const modal = page.locator(".d-modal.poll-ui-builder");
    await expect(modal).toBeVisible();
    if (before) {
      await before(modal);
    }
    await modal.locator(".insert-poll").click();
    await expect(modal).toHaveCount(0);
    return page.evaluate(() => window.savedPolls[0]);
  }

  test.beforeEach(async ({ page, context }) => {
    await login(context, "user1");
    await page.goto("/latest");
    await page.locator("#create-topic").click();
    await expect(page.locator("#reply-control.open")).toBeVisible();
  });

  test("editing keeps the settings the builder does not show", async ({ page }) => {
    const saved = await editPoll(
      page,
      {
        name: "existing",
        type: "regular",
        optionCount: 2,
        public: "false",
        groups: "custom_group",
        close: "2050-01-01",
        status: "closed",
        order: "desc",
      },
      async (modal) => {
        await expect(modal.locator(".d-modal__title-text")).toHaveText("Edit poll");
        // options are edited in the document, not in the builder
        await expect(modal.locator(".poll-options")).toHaveCount(0);
        // neither is the title
        await expect(modal.locator(".poll-title")).toHaveCount(0);
        // every remaining setting is shown, so there is no advanced mode
        await expect(modal.locator(".advanced-mode-btn")).toHaveCount(0);
        // the close date is one of them
        await expect(modal.locator(".poll-date")).toHaveCount(1);
        await expect(modal.locator(".insert-poll")).toHaveText("Save Changes");
      }
    );
    // keeps what it carried and adds no default it lacked
    expect(saved).toEqual({
      name: "existing",
      type: "regular",
      results: null,
      min: null,
      max: null,
      step: null,
      public: "false",
      chartType: null,
      dynamic: null,
      groups: "custom_group",
      close: "2050-01-01",
      status: "closed",
      order: "desc",
    });
  });

  test("editing writes no attribute that matches its default", async ({ page }) => {
    // saving an untouched poll leaves its markdown alone
    expect(await editPoll(page, { optionCount: 2 })).toEqual({
      name: null,
      type: null,
      results: null,
      min: null,
      max: null,
      step: null,
      public: null,
      chartType: null,
      dynamic: null,
      groups: null,
      close: null,
      status: null,
      order: null,
    });
  });

  test("editing leaves a range the poll did not carry to the options", async ({ page }) => {
    const saved = await editPoll(page, { type: "multiple", optionCount: 2 });
    // no minimum, so one option stays enough; no maximum, so a later
    // option is still selectable
    expect([saved.min, saved.max]).toEqual([null, null]);
  });

  test("editing keeps a range the poll spelled out", async ({ page }) => {
    const saved = await editPoll(page, { type: "multiple", optionCount: 3, min: "1", max: "2" });
    expect([saved.min, saved.max]).toEqual(["1", "2"]);
  });

  test("editing a number poll leaves its implied range out", async ({ page }) => {
    const saved = await editPoll(page, { type: "number" });
    // the range still follows the setting, and no step of one
    expect([saved.min, saved.max, saved.step]).toEqual([null, null, null]);
  });

  test("editing keeps a number poll's own step", async ({ page }) => {
    expect((await editPoll(page, { type: "number", step: "5" })).step).toBe("5");
  });

  test("a range field cannot be dropped by emptying it", async ({ page }) => {
    await editPoll(page, { type: "number", min: "1", max: "5", step: "1" }, async (modal) => {
      const insert = modal.locator(".insert-poll");
      // the poll starts out saveable
      await expect(insert).toBeEnabled();
      // an empty maximum is refused, not read as having none
      await modal.locator(".poll-options-max").fill("");
      await expect(insert).toBeDisabled();
      // and an empty minimum is not read as zero
      await modal.locator(".poll-options-max").fill("5");
      await modal.locator(".poll-options-min").fill("");
      await expect(insert).toBeDisabled();
      await modal.locator(".poll-options-min").fill("1");
    });
  });

  test("editing a poll with more options than the site allows", async ({ page }) => {
    // the option count belongs to the document, not the builder: the
    // settings still save
    const saved = await editPoll(page, { type: "regular", optionCount: 25 }, async (modal) => {
      await expect(modal.locator(".insert-poll")).toBeEnabled();
    });
    expect(saved).toBeTruthy();
  });

  test("editing a ranked choice poll leaves range attributes out", async ({ page }) => {
    const saved = await editPoll(page, { type: "ranked_choice", optionCount: 2, min: "3" }, async (modal) => {
      // a stale range does not block saving a type without one
      await expect(modal.locator(".insert-poll")).toBeEnabled();
    });
    expect([saved.min, saved.max]).toEqual([null, null]);
  });
});
