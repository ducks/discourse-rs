// Demo steps 17 to 19: a member flags a post from the flag modal
// (flag-modal.js); the staff's review count; /review's actions
// (review.js): a note on the item, agreeing with the flag from its
// action menu, and approving a post a watched word held for approval.
const { test, expect, login, api, topic, sql, label } = require("./helpers");

test("a flag from the post menu, then a note and agreeing with it in /review", async ({ page, context, session }) => {
  const user1 = await session("user1");
  const marker = label("A reply to flag");
  const t = await topic(user1, label("Review queue topic"), label("A topic for the review spec"), [
    { context: user1, raw: marker },
  ]);
  const flagged = t.posts[1];

  // user0 flags the reply as inappropriate.
  const user0 = await session("user0");
  const flagger = await user0.newPage();
  await flagger.goto(t.url);
  const post = flagger.locator(`article[data-post-id="${flagged}"]`);
  await post.locator(".post-action-menu__show-more").click();
  await post.locator(".post-action-menu__flag").click();
  const modal = flagger.locator(".d-modal .flag-modal-body");
  await modal.locator("#radio_inappropriate").check();
  await flagger.locator(".flag-modal__create-flag").click();
  await expect(modal).toHaveCount(0);
  await expect
    .poll(() =>
      sql(
        "SELECT r.type, r.status FROM reviewables r WHERE r.target_type = 'Post' AND r.target_id = :'p1'",
        flagged,
      ),
    )
    .toEqual([["ReviewableFlaggedPost", "0"]]);

  // The admin: the count in the sidebar, the item in /review.
  await login(context, "admin");
  await page.goto("/latest");
  const reviewLink = page.locator("#d-sidebar a[href='/review']");
  await expect(reviewLink.locator(".sidebar-section-link-content-badge")).toBeVisible();
  await reviewLink.click();
  await expect(page).toHaveURL(/\/review/);
  const item = page.locator(".review-item.reviewable-flagged-post", { hasText: marker });
  await expect(item).toBeVisible();

  const note = label("Note from the browser suite");
  await item.locator(".reviewable-note-form__textarea").fill(note);
  await item.locator("button", { hasText: /^\s*Add note/ }).click();
  await expect(item.locator(".reviewable-timeline", { hasText: note })).toBeVisible();
  expect(
    sql(
      "SELECT n.content FROM reviewable_notes n JOIN reviewables r ON r.id = n.reviewable_id WHERE r.target_id = :'p1'",
      flagged,
    ),
  ).toEqual([[note]]);

  await item.locator("details summary", { hasText: /^\s*Yes/ }).first().click();
  await page.locator(".select-kit.is-expanded .select-kit-row", { hasText: /Agree with flag/i }).first().click();
  await expect(item.locator("*", { hasText: /^\s*Flag approved\s*$/ }).first()).toBeVisible();
  await expect
    .poll(() => sql("SELECT status FROM reviewables WHERE target_type = 'Post' AND target_id = :'p1'", flagged))
    .toEqual([["1"]]);
});

test("a post a watched word held is approved from /review", async ({ page, context, session }) => {
  await login(context, "admin");
  const word = label("rhword").replace(" ", "");
  await api(context, "POST", "/admin/customize/watched_words.json", { "words[]": word, action_key: "require_approval" });

  const user0 = await session("user0");
  const title = label("Held topic for the review spec");
  const held = await api(user0, "POST", "/posts.json", {
    title,
    raw: `This post mentions ${word}, which needs approval.`,
    category: "4",
  });
  expect(held.action).toBe("enqueued");

  await page.goto("/review");
  const item = page.locator(".review-item.reviewable-queued-post", { hasText: word });
  await expect(item).toBeVisible();
  await item.locator("button.approve-post").click();
  await expect(item.locator(".review-item__status.--approved")).toBeVisible();
  await expect
    .poll(() => sql("SELECT t.title, p.post_number FROM topics t JOIN posts p ON p.topic_id = t.id WHERE t.title = :'p1'", title))
    .toEqual([[title, "1"]]);
});
