// The post menu (topic.js, post-stream.js) and the plugin buttons in it:
// discourse-reactions' like and picker, discourse-solved's accept and
// unaccept, discourse-assign's modal and unassign; show more, copy link,
// bookmark and the flag modal. Each action is checked on the page and in
// the database.
const { test, expect, login, topic, sql, label } = require("./helpers");

async function setup(session, context) {
  const user1 = await session("user1");
  const user0 = await session("user0");
  const t = await topic(user1, label("Post menu topic"), label("A question for the post menu spec"), [
    { context: user0, raw: label("An answer by user0 for the post menu") },
  ]);
  await login(context, "admin");
  return { t, answer: t.posts[1] };
}

test("show more, copy link and bookmark", async ({ page, context, session }) => {
  const { t, answer } = await setup(session, context);
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await page.goto(t.url);
  const post = page.locator(`article[data-post-id="${answer}"]`);
  const nav = post.locator("nav.post-controls");
  await expect(nav).toHaveClass(/collapsed/);
  await expect(post.locator(".post-action-menu__flag")).toBeHidden();
  await expect(post.locator(".post-action-menu__bookmark")).toBeHidden();

  await post.locator(".post-action-menu__show-more").click();
  await expect(nav).toHaveClass(/expanded/);
  await expect(post.locator(".post-action-menu__show-more")).toHaveCount(0);
  for (const button of ["flag", "edit", "bookmark", "delete", "assign-post", "admin"]) {
    await expect(post.locator(`.post-action-menu__${button}`)).toBeVisible();
  }

  await post.locator(".post-action-menu__copy-link").click();
  await expect
    .poll(() => page.evaluate(() => navigator.clipboard.readText()))
    .toBe(new URL(`${t.url}/2?u=admin`, page.url()).href);

  await post.locator(".post-action-menu__bookmark").click();
  // Saved, and the post drawn again (refreshPost) as bookmarked.
  await expect(post.locator(".post-action-menu__bookmark.bookmarked")).toBeVisible();
  expect(
    sql(
      "SELECT count(*) FROM bookmarks WHERE bookmarkable_type = 'Post' AND bookmarkable_id = :'p1' AND user_id = 1",
      answer,
    ),
  ).toEqual([["1"]]);
  await post.locator(".post-action-menu__bookmark.bookmarked").click();
  await expect(post.locator(".post-action-menu__bookmark:not(.bookmarked)")).toBeAttached();
  expect(
    sql("SELECT count(*) FROM bookmarks WHERE bookmarkable_type = 'Post' AND bookmarkable_id = :'p1'", answer),
  ).toEqual([["0"]]);
});

test("discourse-reactions: like, then another reaction from the picker", async ({ page, context, session }) => {
  const { t, answer } = await setup(session, context);
  await page.goto(t.url);
  const post = page.locator(`article[data-post-id="${answer}"]`);
  const actions = post.locator(`#discourse-reactions-actions-${answer}-right`);
  await expect(actions).not.toHaveClass(/has-reacted/);

  await actions.locator(".discourse-reactions-reaction-button").click();
  await expect(actions).toHaveClass(/has-reacted/);
  await expect(actions).toHaveAttribute("data-current-reaction", "heart");
  await expect(post.locator(`#discourse-reactions-counter-${answer}-left`)).toHaveAttribute(
    "aria-label",
    "1 reaction",
  );
  // The main reaction is a like.
  await expect
    .poll(() =>
      sql(
        "SELECT count(*) FROM post_actions WHERE post_id = :'p1' AND user_id = 1 AND post_action_type_id = 2 AND deleted_at IS NULL",
        answer,
      ),
    )
    .toEqual([["1"]]);

  // Undo, then pick another from the picker the button opens on hover.
  await actions.locator(".discourse-reactions-reaction-button").click();
  await expect(actions).not.toHaveClass(/has-reacted/);
  await expect
    .poll(() =>
      sql(
        "SELECT count(*) FROM post_actions WHERE post_id = :'p1' AND user_id = 1 AND post_action_type_id = 2 AND deleted_at IS NULL",
        answer,
      ),
    )
    .toEqual([["0"]]);
  await page.mouse.move(0, 0);
  await actions.locator(".discourse-reactions-reaction-button").hover();
  const picker = actions.locator(".discourse-reactions-picker.is-expanded");
  await expect(picker).toBeVisible();
  await picker.locator(".pickable-reaction.clap").click();
  await expect(actions).toHaveAttribute("data-current-reaction", "clap");
  await expect(post.locator(`#discourse-reactions-list-emoji-${answer}-clap`)).toBeVisible();
  await expect
    .poll(() =>
      sql(
        "SELECT r.reaction_value FROM discourse_reactions_reaction_users ru JOIN discourse_reactions_reactions r ON r.id = ru.reaction_id WHERE ru.post_id = :'p1' AND ru.user_id = 1",
        answer,
      ),
    )
    .toEqual([["clap"]]);
});

test("discourse-solved: accept and unaccept the answer", async ({ page, context, session }) => {
  const { t, answer } = await setup(session, context);
  await page.goto(t.url);
  const post = page.locator(`article[data-post-id="${answer}"]`);
  const title = page.locator("#topic-title .topic-statuses");
  await expect(title.locator(".topic-status.--solved")).toHaveCount(0);

  await post.locator(".post-action-menu__solved-unaccepted").click();
  const accepted = post.locator(".post-action-menu__solved-accepted");
  await expect(accepted).toBeVisible();
  await expect(accepted).toHaveText("Solution");
  await expect(title.locator(".topic-status.--solved")).toHaveCount(1);
  expect(sql("SELECT a.answer_post_id, a.accepter_user_id FROM discourse_solved_topic_answers a JOIN discourse_solved_solved_topics s ON s.id = a.solved_topic_id WHERE s.topic_id = :'p1'", t.id)).toEqual([
    [String(answer), "1"],
  ]);

  await accepted.click();
  await expect(post.locator(".post-action-menu__solved-unaccepted")).toBeVisible();
  await expect(title.locator(".topic-status.--solved")).toHaveCount(0);
  expect(sql("SELECT count(*) FROM discourse_solved_topic_answers a JOIN discourse_solved_solved_topics s ON s.id = a.solved_topic_id WHERE s.topic_id = :'p1'", t.id)).toEqual([["0"]]);
});

// The page is redrawn in place, not reloaded: a mark set on the window
// before the action is still there after it.
async function markDocument(page) {
  await page.evaluate(() => (window.__sameDocument = true));
}
async function expectSameDocument(page) {
  expect(await page.evaluate(() => window.__sameDocument === true), "the page reloaded").toBe(true);
}

test("discourse-assign: assign the post from its modal, then unassign", async ({ page, context, session }) => {
  const { t, answer } = await setup(session, context);
  await page.goto(t.url);
  await markDocument(page);
  const post = page.locator(`article[data-post-id="${answer}"]`);
  await post.locator(".post-action-menu__show-more").click();
  await post.locator(".post-action-menu__assign-post").click();

  const modal = page.locator(".d-modal.assign");
  await expect(modal.locator(".d-modal__title-text")).toHaveText("Assign Post");
  // Submitting without an assignee says so and stays open.
  await modal.locator(".assign-submit").click();
  await expect(modal.locator(".error-label")).toBeVisible();
  await modal.locator("#assignee").fill("admin");
  await modal.locator("#assign-note").fill(label("assign note"));
  await modal.locator(".assign-submit").click();
  await expect(modal).toHaveCount(0);

  await expect(post.locator(".assigned-to .assigned-to-username")).toHaveText("admin");
  // The footer's Assign button became the topic-level menu listing the post.
  await expect(page.locator("#topic-footer-buttons .reassign-trigger")).toBeVisible();
  await expect(page.locator("#topic-footer-button-assign")).toHaveCount(0);
  expect(
    sql(
      "SELECT assigned_to_type, assigned_to_id, active FROM assignments WHERE target_type = 'Post' AND target_id = :'p1'",
      answer,
    ),
  ).toEqual([["User", "1", "t"]]);
  await expectSameDocument(page);

  await post.locator(".post-action-menu__unassign-post").click();
  await expect(post.locator(".assigned-to")).toHaveCount(0);
  await expect(post.locator(".post-action-menu__assign-post")).toBeAttached();
  await expect(page.locator("#topic-footer-button-assign")).toBeVisible();
  expect(sql("SELECT count(*) FROM assignments WHERE target_type = 'Post' AND target_id = :'p1'", answer)).toEqual([
    ["0"],
  ]);
  await expectSameDocument(page);
});

test("discourse-assign: assign the topic from the footer, then unassign from its menu", async ({
  page,
  context,
  session,
}) => {
  const { t } = await setup(session, context);
  await page.goto(t.url);
  await markDocument(page);
  await page.locator("#topic-footer-button-assign").click();
  const modal = page.locator(".d-modal.assign");
  await expect(modal.locator(".d-modal__title-text")).toHaveText("Assign Topic");
  // Staff only may be assigned (assign_allowed_on_groups).
  await modal.locator("#assignee").fill("admin");
  await modal.locator(".assign-submit").click();
  await expect(modal).toHaveCount(0);

  const trigger = page.locator("#topic-footer-buttons .reassign-trigger");
  await expect(trigger).toBeVisible();
  await expect(page.locator("#post_1 .assigned-to .assigned-to--user")).toHaveText("Assigned topic to admin");
  expect(
    sql(
      "SELECT a.assigned_to_type, u.username FROM assignments a JOIN users u ON u.id = a.assigned_to_id WHERE a.target_type = 'Topic' AND a.target_id = :'p1'",
      t.id,
    ),
  ).toEqual([["User", "admin"]]);
  await expectSameDocument(page);

  await trigger.click();
  await page.locator('.reassign-item[data-action="unassign"][data-target-type="Topic"]').click();
  await expect(page.locator("#topic-footer-button-assign")).toBeVisible();
  await expect(page.locator("#post_1 .assigned-to")).toHaveCount(0);
  expect(sql("SELECT count(*) FROM assignments WHERE target_type = 'Topic' AND target_id = :'p1'", t.id)).toEqual([
    ["0"],
  ]);
  await expectSameDocument(page);
});

test("the flag modal enables its button once a reason is chosen", async ({ page, context, session }) => {
  const { t, answer } = await setup(session, context);
  await page.goto(t.url);
  const post = page.locator(`article[data-post-id="${answer}"]`);
  await post.locator(".post-action-menu__show-more").click();
  await post.locator(".post-action-menu__flag").click();
  const modal = page.locator(".d-modal .flag-modal-body");
  await expect(modal).toBeVisible();
  const submit = page.locator(".flag-modal__create-flag");
  await expect(submit).toBeDisabled();
  await modal.locator("#radio_off_topic").check();
  await expect(submit).toBeEnabled();
  await page.keyboard.press("Escape");
  await expect(modal).toHaveCount(0);
  expect(sql("SELECT count(*) FROM post_actions WHERE post_id = :'p1' AND post_action_type_id = 3", answer)).toEqual([
    ["0"],
  ]);
});
