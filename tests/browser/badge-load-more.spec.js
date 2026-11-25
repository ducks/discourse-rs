// A badge's page with more grants than its first page (static/js/
// badge-page.js; Ember's BadgesShowController#loadMore): scrolling to the
// end loads the rest, then the spinner goes. The badge is the spec's own,
// a copy of Basic User that grants many times, given 100 times to user1.
const { test, expect, sql, label } = require("./helpers");

test("scrolling a badge's grants loads the rest", async ({ page }) => {
  const [[id]] = sql(
    "INSERT INTO badges SELECT (jsonb_populate_record(NULL::badges, to_jsonb(b) || jsonb_build_object( \
       'id', nextval('badges_id_seq'), 'name', :'p1', 'multiple_grant', true, 'system', false, \
       'grant_count', 100))).* FROM badges b WHERE id = 1 RETURNING id",
    label("Load more badge")
  );
  sql(
    "INSERT INTO user_badges (badge_id, user_id, granted_at, granted_by_id, seq, created_at) \
     SELECT :p1, u.id, now() - n * interval '1 minute', -1, n, now() \
     FROM generate_series(1, 100) n, users u WHERE u.username = 'user1'",
    id
  );

  await page.goto(`/badges/${id}`);
  const grants = page.locator(".show-badge .badges-granted > .badge-info");
  await expect(grants).toHaveCount(96);
  await expect(page.locator(".show-badge .loading-container.visible .spinner")).toBeVisible();

  await page.locator(".show-badge .load-more-sentinel").scrollIntoViewIfNeeded();
  await expect(grants).toHaveCount(100);
  await expect(page.locator(".show-badge .loading-container")).not.toHaveClass(/visible/);
  await expect(page.locator(".show-badge .load-more-sentinel")).not.toHaveAttribute(
    "data-next-offset",
    /.*/
  );
});
