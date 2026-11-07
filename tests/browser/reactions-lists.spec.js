// discourse-reactions' lists on the user pages (discourse-reactions.js):
// the Reactions tab and reactions received load their next page once the
// sentinel scrolls into view (UserActivityReactions#loadMore), with the
// cursors the drawn items leave, and reactions received keeps its
// acting_username and include_likes (the notification links' query) on
// every page. Members of the spec's own: an author with 25 posts, a
// reactor who claps at each, a liker who likes 22 of them.
const { test, expect, login, topic, sql, label, member } = require("./helpers");

const POSTS = 25;
const LIKES = 22;

async function setup(session) {
  const author = member();
  const reactor = member();
  const liker = member();
  const t = await topic(await session(author.name), label("Reactions lists topic"), label("The first of many posts"));
  // The rest of the posts as copies of the first, each a minute older.
  sql(
    "INSERT INTO posts SELECT (jsonb_populate_record(NULL::posts, to_jsonb(p) || jsonb_build_object( \
       'id', nextval('posts_id_seq'), 'post_number', n, 'created_at', now() - make_interval(mins => n), \
       'updated_at', now() - make_interval(mins => n)))).* \
     FROM posts p, generate_series(2, :'p2'::int) n WHERE p.id = :'p1'",
    t.posts[0],
    POSTS,
  );
  // A clap on each, the later posts' earlier; ids rise with the time, as
  // the cursors (ids) and the order (created_at) take for granted.
  sql(
    "INSERT INTO discourse_reactions_reactions (post_id, reaction_type, reaction_value, reaction_users_count, created_at, updated_at) \
     SELECT id, 0, 'clap', 1, now(), now() FROM posts WHERE topic_id = :'p1'",
    t.id,
  );
  sql(
    "INSERT INTO discourse_reactions_reaction_users (reaction_id, user_id, post_id, created_at, updated_at) \
     SELECT r.id, :'p2', r.post_id, now() - make_interval(secs => p.post_number), now() \
     FROM discourse_reactions_reactions r JOIN posts p ON p.id = r.post_id WHERE p.topic_id = :'p1' ORDER BY p.post_number DESC",
    t.id,
    reactor.id,
  );
  // Likes that are no reaction's shadow.
  sql(
    "INSERT INTO post_actions (post_id, user_id, post_action_type_id, created_at, updated_at) \
     SELECT id, :'p2', 2, now() - make_interval(secs => post_number), now() \
     FROM posts WHERE topic_id = :'p1' AND post_number <= :'p3'::int ORDER BY post_number DESC",
    t.id,
    liker.id,
    LIKES,
  );
  return { t, author, reactor, liker };
}

function postIds(page) {
  return page.locator("#user-content .post-list > .post-list-item .excerpt").evaluateAll((els) =>
    els.map((el) => el.dataset.postId),
  );
}

test("the Reactions tab loads past its first page", async ({ page, context, session }) => {
  const { t, reactor } = await setup(session);
  await login(context, reactor.name);
  await page.goto(`/u/${reactor.name}/activity/reactions`);
  const items = page.locator("#user-content .post-list > .post-list-item");
  await expect(items).toHaveCount(20);
  // A full page leaves the footer for the end.
  await expect(page.locator("template.deferred-footer")).not.toHaveCount(0);

  const next = page.waitForRequest(
    (r) => r.url().includes(`/u/${reactor.name}/activity/reactions?`) && r.url().includes("before_reaction_user_id="),
  );
  await page.evaluate(() => window.scrollTo(0, document.body.scrollHeight));
  await next;
  await expect(items).toHaveCount(POSTS);
  const ids = await postIds(page);
  expect(new Set(ids).size).toBe(POSTS);
  const all = sql("SELECT id FROM posts WHERE topic_id = :'p1' ORDER BY post_number", t.id).map((r) => r[0]);
  expect(ids).toEqual(all);

  // The page after that is empty: the list ends and the footer shows.
  await page.evaluate(() => window.scrollTo(0, 0));
  await page.evaluate(() => window.scrollTo(0, document.body.scrollHeight));
  await expect(page.locator("template.deferred-footer")).toHaveCount(0);
  await expect(items).toHaveCount(POSTS);
});

test("reactions received loads past its first page", async ({ page, context, session }) => {
  const { author, reactor } = await setup(session);
  await login(context, author.name);
  await page.goto(`/u/${author.name}/notifications/reactions-received`);
  const items = page.locator("#user-content .post-list > .post-list-item");
  await expect(items).toHaveCount(20);
  await expect(items.locator(".discourse-reactions-my-reaction img[alt='clap']")).toHaveCount(20);
  await page.evaluate(() => window.scrollTo(0, document.body.scrollHeight));
  await expect(items).toHaveCount(POSTS);
  const ids = await postIds(page);
  expect(new Set(ids).size).toBe(POSTS);
  await expect(items.locator(`.discourse-reactions-my-reaction [data-user-card='${reactor.name}']`)).toHaveCount(POSTS);
});

test("reactions received with acting_username and include_likes, as a notification links it", async ({
  page,
  context,
  session,
}) => {
  const { author, liker } = await setup(session);
  await login(context, author.name);
  await page.goto(`/u/${author.name}/notifications/reactions-received?acting_username=${liker.name}&include_likes=true`);
  const items = page.locator("#user-content .post-list > .post-list-item");
  // Only the liker's, as the main reaction.
  await expect(items).toHaveCount(20);
  await expect(items.locator(".discourse-reactions-my-reaction img[alt='heart']")).toHaveCount(20);
  await expect(items.locator(`.discourse-reactions-my-reaction [data-user-card='${liker.name}']`)).toHaveCount(20);

  // The next page goes on from the last like, with the same query.
  const next = page.waitForRequest((r) => {
    const url = new URL(r.url());
    return (
      url.pathname === `/u/${author.name}/notifications/reactions-received` &&
      url.searchParams.get("acting_username") === liker.name &&
      url.searchParams.get("include_likes") === "true" &&
      url.searchParams.has("before_like_id") &&
      !url.searchParams.has("before_reaction_user_id")
    );
  });
  await page.evaluate(() => window.scrollTo(0, document.body.scrollHeight));
  await next;
  await expect(items).toHaveCount(LIKES);
  expect(new Set(await postIds(page)).size).toBe(LIKES);
  await expect(items.locator(".discourse-reactions-my-reaction img[alt='heart']")).toHaveCount(LIKES);
});
