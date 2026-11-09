// What the specs share: the test fixture (failing on any uncaught page
// error), logins, the database copy, labels.
const base = require("@playwright/test");
const { execFileSync } = require("child_process");
const fs = require("fs");
const path = require("path");

const ROOT = process.env.BROWSER_TEST_ROOT || path.resolve(__dirname, "../..");
// The seeded users' password (tests/demo_smoke.rs): user0 is trust level 1,
// user1 and user2 trust level 2, admin an admin.
const PASSWORD = "password";

// A page error is a bug in static/js whatever the spec asserts: collect
// them on every page of the test's contexts and fail the test on any.
const test = base.test.extend({
  pageErrors: [
    async ({}, use) => {
      const errors = [];
      await use(errors);
      base.expect(errors, "uncaught page errors").toEqual([]);
    },
    { scope: "test", auto: true },
  ],
  context: async ({ context, pageErrors }, use) => {
    watch(context, pageErrors);
    await emojiImages(context);
    await use(context);
  },
  // Another browser session (its own cookies and storage), logged in as
  // the username given, or a visitor for null: `await session("user0")`.
  // For live updates between members and for a spec's setup.
  session: async ({ browser, pageErrors }, use, testInfo) => {
    const opened = [];
    await use(async (username) => {
      const { baseURL, viewport, colorScheme } = testInfo.project.use;
      const context = await browser.newContext({ baseURL, viewport, colorScheme });
      opened.push(context);
      watch(context, pageErrors);
      await emojiImages(context);
      if (username) {
        await login(context, username);
      }
      return context;
    });
    for (const context of opened) {
      await context.close();
    }
  },
});

function watch(context, errors) {
  const onPage = (page) => {
    page.on("pageerror", (e) => errors.push(`${page.url()}: ${e.message}`));
    // BROWSER_TEST_DEBUG=1 lists the 4xx and 5xx answers a spec saw.
    if (process.env.BROWSER_TEST_DEBUG) {
      page.on("response", (r) => r.status() >= 400 && console.log("RESPONSE", r.status(), r.request().method(), r.url()));
    }
  };
  context.pages().forEach(onPage);
  context.on("page", onPage);
}

// The emoji images, as a checkout with the discourse-emojis gem serves
// them. Without the gem (CI) /images/emoji/* is a 404, and a broken
// <img class="emoji"> lays out as its alt text instead of the 24px box
// the stylesheets give it: each 404 that lands reflows the emoji around it,
// so a click aimed at one emoji in the picker can land on its neighbour.
const EMOJI_PNG = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=",
  "base64"
);

async function emojiImages(context) {
  await context.route("**/images/emoji/**", (route) =>
    route.fulfill({ status: 200, contentType: "image/png", body: EMOJI_PNG })
  );
}

// Logs the context in as the page's scripts do (login.js's POST
// /session.json with the CSRF token), without the login page. The login
// page itself has a spec of its own.
async function login(context, username) {
  const request = context.request;
  const csrf = await request.get("/session/csrf.json", {
    headers: { "X-Requested-With": "XMLHttpRequest" },
  });
  base.expect(csrf.ok(), "GET /session/csrf.json").toBeTruthy();
  const token = (await csrf.json()).csrf;
  const res = await request.post("/session.json", {
    headers: { "X-CSRF-Token": token, "X-Requested-With": "XMLHttpRequest" },
    form: { login: username, password: PASSWORD },
  });
  const body = await res.text();
  base.expect(res.ok(), `logging in ${username}: ${body}`).toBeTruthy();
  base.expect(body, `logging in ${username}`).not.toContain('"error"');
}

// A request as the page's scripts send it, in the context's session (for
// a spec's setup: the topic it works on, a post to flag). Returns the
// parsed JSON.
async function api(context, method, url, form) {
  const csrf = await context.request.get("/session/csrf.json", {
    headers: { "X-Requested-With": "XMLHttpRequest" },
  });
  const token = (await csrf.json()).csrf;
  const res = await context.request.fetch(url, {
    method,
    headers: { "X-CSRF-Token": token, "X-Requested-With": "XMLHttpRequest", Accept: "application/json" },
    form,
  });
  const body = await res.text();
  base.expect(res.ok(), `${method} ${url}: ${res.status()} ${body.slice(0, 500)}`).toBeTruthy();
  return body ? JSON.parse(body) : null;
}

// A new topic by the context's member, with replies by others' contexts:
// [{ context, raw }]. Returns { id, url, posts: [post ids in order] }.
// Bodies must differ from run to run (label()): Discourse refuses a body a
// member posted within unique_posts_mins.
async function topic(context, title, raw, replies = []) {
  const first = await api(context, "POST", "/posts.json", { title, raw, category: "4" });
  const posts = [first.id];
  for (const reply of replies) {
    const post = await api(reply.context, "POST", "/posts.json", {
      topic_id: String(first.topic_id),
      raw: reply.raw,
    });
    posts.push(post.id);
  }
  return { id: first.topic_id, url: `/t/${first.topic_slug}/${first.topic_id}`, posts };
}

// A query on the server's database copy (psql, which the shell carries),
// its rows as arrays of strings.
function sql(query, ...params) {
  const url = process.env.BROWSER_TEST_DATABASE_URL;
  if (!url) {
    throw new Error("BROWSER_TEST_DATABASE_URL is not set; run through scripts/browser-test");
  }
  const args = ["-X", "-q", "-A", "-t", "-F", "\t", "-v", "ON_ERROR_STOP=1"];
  params.forEach((p, i) => args.push("-v", `p${i + 1}=${p}`));
  const out = execFileSync("psql", [...args, url], { input: query, encoding: "utf8" });
  return out
    .split("\n")
    .filter((line) => line !== "")
    .map((line) => line.split("\t"));
}

// A member of the test's own, as user2 is (password, groups, options),
// for a spec whose writes would reach another spec through a shared user
// (a status, a penalty). Returns { name, id }.
function member() {
  const name = "st" + Math.floor(Math.random() * 1e9).toString(36);
  const [[id]] = sql(
    "INSERT INTO users SELECT (jsonb_populate_record(NULL::users, to_jsonb(u) || jsonb_build_object( \
       'id', nextval('users_id_seq'), 'username', :'p1', 'username_lower', lower(:'p1'), 'name', :'p1', \
       'secure_identifier', NULL))).* FROM users u WHERE username = 'user2' RETURNING id",
    name,
  );
  const copy = (table, extra, where) =>
    sql(
      `INSERT INTO ${table} SELECT (jsonb_populate_record(NULL::${table}, to_jsonb(t) || jsonb_build_object(${extra}))).* \
       FROM ${table} t WHERE user_id = (SELECT id FROM users WHERE username = 'user2')${where || ""}`,
      name,
      id,
    );
  copy(
    "user_emails",
    "'id', nextval('user_emails_id_seq'), 'user_id', :'p2'::int, 'email', :'p1' || '@example.com', 'normalized_email', :'p1' || '@example.com'",
    ' AND "primary"',
  );
  copy("user_passwords", "'id', nextval('user_passwords_id_seq'), 'user_id', :'p2'::int");
  copy("user_options", "'user_id', :'p2'::int");
  copy("user_stats", "'user_id', :'p2'::int");
  copy("user_profiles", "'user_id', :'p2'::int");
  copy("group_users", "'id', nextval('group_users_id_seq'), 'user_id', :'p2'::int");
  return { name, id };
}

// A label for what a test writes, unique across runs and workers.
function label(what) {
  return `${what} ${Date.now().toString(36)}${Math.floor(Math.random() * 1e6).toString(36)}`;
}

// A static asset's source, for the specs that drive a script over a
// fixture page.
function asset(rel) {
  return fs.readFileSync(path.join(ROOT, "static", rel), "utf8");
}

module.exports = { test, expect: base.expect, login, api, topic, sql, label, member, asset, PASSWORD };
