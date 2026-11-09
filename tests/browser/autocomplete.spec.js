// static/js/autocomplete.js' sources, as the reference's QUnit tests check
// theirs: unit/lib/user-search-test.js (lib/user-search) and
// integration/components/user-autocomplete-results-test.gjs
// (UserAutocompleteResults), their bodies run in the page through the
// rich editor helper's harness (inPage) with pretender's handlers as
// routes.
const { test, expect, asset } = require("./helpers");
const { openEditorPage, inPage } = require("./rich-editor-helper");

function json(route, body, status = 200, headers = {}) {
  return route.fulfill({ status, headers, contentType: "application/json", body: JSON.stringify(body) });
}

// user-search-test.js' beforeEach responder.
function defaultSearch(route) {
  const url = route.request().url();
  // special responder for per category search
  const categoryMatch = url.match(/category_id=([0-9]+)/);
  if (categoryMatch) {
    if (categoryMatch[1] === "3") {
      return json(route, {});
    }
    return json(route, {
      users: [
        {
          username: `category_${categoryMatch[1]}`,
          name: "category user",
          avatar_template: "https://avatars.discourse.org/v3/letter/t/41988e/{size}.png",
        },
      ],
    });
  }

  if (url.match(/no-results/)) {
    return json(route, { users: [] });
  }

  return json(route, {
    users: [
      { username: "TeaMoe", name: "TeaMoe", avatar_template: "https://avatars.discourse.org/v3/letter/t/41988e/{size}.png" },
      { username: "TeamOneJ", name: "J Cobb", avatar_template: "https://avatars.discourse.org/v3/letter/t/3d9bf3/{size}.png" },
      { username: "kudos", name: "Team Blogeto.com", avatar_template: "/user_avatar/meta.discourse.org/kudos/{size}/62185_1.png" },
      { username: "RosieLinda", name: "Linda Teaman", avatar_template: "https://avatars.discourse.org/v3/letter/r/bc8723/{size}.png" },
      { username: "legalatom", name: "Team LegalAtom", avatar_template: "https://avatars.discourse.org/v3/letter/l/a9a28c/{size}.png" },
      { username: "dzsat_team", name: "Dz Sat Dz Sat", avatar_template: "https://avatars.discourse.org/v3/letter/d/eb9ed0/{size}.png" },
    ],
    groups: [
      { name: "bob", usernames: [] },
      { name: "team", usernames: [] },
    ],
  });
}

// The harness' page with autocomplete.js, its user search cache reset
// (resetUserSearchCache, as the test helpers' cleanup does), and
// /u/search/users answered by `handler` (the beforeEach's by default).
async function userSearchPage(page, handler = defaultSearch) {
  await page.route("**/u/search/users?**", handler);
  await openEditorPage(page);
  await page.addScriptTag({ content: asset("js/autocomplete.js") });
  await page.evaluate(() => window.Discourse.autocomplete.resetUserSearchCache());
}

async function run(page, body) {
  expect(await inPage(page, body)).toEqual([]);
}

test.describe("Unit | Utility | user-search", () => {
  test("it flushes cache when switching categories", async ({ page }) => {
    await userSearchPage(page);
    await run(page, async (assert) => {
      const userSearch = window.Discourse.autocomplete.userSearch;
      let results = await userSearch({ term: "hello", categoryId: 1 });
      assert.strictEqual(results[0].username, "category_1");
      assert.strictEqual(results.length, 1);

      // this is cached ... so let's check the cache is good
      results = await userSearch({ term: "hello", categoryId: 1 });
      assert.strictEqual(results[0].username, "category_1");
      assert.strictEqual(results.length, 1);

      results = await userSearch({ term: "hello", categoryId: 2 });
      assert.strictEqual(results[0].username, "category_2");
      assert.strictEqual(results.length, 1);
    });
  });

  test("it keeps mention search results separate from other search contexts", async ({ page }) => {
    const contexts = [];
    await userSearchPage(page, (route) => {
      const context = new URL(route.request().url()).searchParams.get("search_context") ?? undefined;
      contexts.push(context);
      return json(route, { users: [{ username: context === "mention" ? "mention_user" : "user" }] });
    });
    await run(page, async (assert) => {
      const userSearch = window.Discourse.autocomplete.userSearch;
      let results = await userSearch({ term: "user" });
      assert.strictEqual(results[0].username, "user", "an ordinary search uses the default context");

      results = await userSearch({ term: "user", searchContext: "mention" });
      assert.strictEqual(results[0].username, "mention_user", "mention searches do not reuse ordinary search results");

      results = await userSearch({ term: "user", searchContext: "mention" });
      assert.strictEqual(results[0].username, "mention_user", "results are cached within the mention context");
    });
    expect(contexts, "only mention searches send the mention context").toEqual([undefined, "mention"]);
  });

  test("it searches the given url without reusing results cached for another one", async ({ page }) => {
    await page.route("**/custom/search?**", (route) => json(route, { users: [{ username: "custom_user" }] }));
    await userSearchPage(page);
    await run(page, async (assert) => {
      const userSearch = window.Discourse.autocomplete.userSearch;
      let results = await userSearch({ term: "hello" });
      assert.strictEqual(results[0].username, "TeaMoe");

      results = await userSearch({ term: "hello", url: "/custom/search" });
      assert.deepEqual(
        results.map((r) => r.username),
        ["custom_user"]
      );
    });
  });

  test("it returns cancel when eager completing with no results", async ({ page }) => {
    await userSearchPage(page);
    await run(page, async (assert) => {
      const { userSearch, CANCELLED } = window.Discourse.autocomplete;
      // Do everything twice, to check the cache works correctly

      for (let i = 0; i < 2; i++) {
        // No topic or category, will always cancel
        let result = await userSearch({ term: "" });
        assert.strictEqual(result, CANCELLED);
      }

      for (let i = 0; i < 2; i++) {
        // Unsecured category, so has no recommendations
        let result = await userSearch({ term: "", categoryId: 3 });
        assert.strictEqual(result, CANCELLED);
      }

      for (let i = 0; i < 2; i++) {
        // Secured category, will have 1 recommendation
        let results = await userSearch({ term: "", categoryId: 1 });
        assert.strictEqual(results[0].username, "category_1");
        assert.strictEqual(results.length, 1);
      }
    });
  });

  test("it strips @ from the beginning", async ({ page }) => {
    await userSearchPage(page);
    await run(page, async (assert) => {
      const results = await window.Discourse.autocomplete.userSearch({ term: "@Team" });

      results.forEach((result) => {
        const identifier = result.username || result.name;
        assert.false(identifier.startsWith("@"), `Result '${identifier}' should not start with @`);
      });
    });
  });

  test("it skips a search depending on punctuation", async ({ page }) => {
    await userSearchPage(page);
    await run(page, async (assert) => {
      const userSearch = window.Discourse.autocomplete.userSearch;
      let results;
      let skippedTerms = [
        "@sam  s", // double space is not allowed
        "@sam;",
        "@sam,",
        "@sam:",
      ];

      for (let term of skippedTerms) {
        results = await userSearch({ term });
        assert.strictEqual(results.length, 0);
      }

      let allowedTerms = [
        "@sam sam", // double space is not allowed
        "@sam.sam",
        "@sam_sam",
        "@sam-sam",
        "@",
      ];

      let topicId = 100;

      for (let term of allowedTerms) {
        results = await userSearch({ term, topicId });
        assert.strictEqual(results.length, 6);
      }

      results = await userSearch({ term: "sam@sam.com", allowEmails: true });
      assert.strictEqual(results.length, 6);

      results = await userSearch({ term: "sam+test@sam.com", allowEmails: true });
      assert.strictEqual(results.length, 6);

      results = await userSearch({ term: "sam@sam.com" });
      assert.strictEqual(results.length, 0);

      results = await userSearch({
        term: "no-results@example.com",
        allowEmails: true,
      });
      assert.strictEqual(results.length, 1);
    });
  });

  test("it uses limit option", async ({ page }) => {
    await userSearchPage(page);
    await run(page, async (assert) => {
      const results = await window.Discourse.autocomplete.userSearch({ term: "te", limit: 2 });
      assert.strictEqual(results.length, 2);
    });
  });

  test("it orders results by priority: exact username, exact group, partial username, partial group, exact name, partial name, metadata", async ({
    page,
  }) => {
    await userSearchPage(page, (route) => {
      if (route.request().url().includes("term=test")) {
        return json(route, {
          users: [
            { username: "testing", name: "Testing User" }, // partial username match
            { username: "test", name: "Test User" }, // exact username match
            { username: "user123", name: "anon", title: "tester" }, // metadata match
            { username: "player", name: "test" }, // exact name match
            { username: "gamer", name: "testing games" }, // partial name match
          ],
          groups: [
            { name: "testers", usernames: [] }, // partial group match
            { name: "test", usernames: [] }, // exact group match
            { name: "group_2", full_name: "Testers", usernames: [] }, // partial group match
            { name: "group_4", full_name: "Test", usernames: [] }, // exact group match
          ],
        });
      }
      return json(route, { users: [], groups: [] });
    });
    await run(page, async (assert) => {
      const results = await window.Discourse.autocomplete.userSearch({
        term: "test",
        allowEmails: true,
        limit: 10,
      });

      // Expected order:
      // exact user username, exact group name,
      // partial user username, partial group name,
      // exact user name, partial user name
      // exact group full_name, partial group full_name
      assert.strictEqual(results[0].username, "test", "First: Exact user username match");
      assert.strictEqual(results[1].name, "test", "Second: Exact group name match");
      assert.strictEqual(results[2].username, "testing", "Third: Partial user username match");
      assert.strictEqual(results[3].name, "testers", "Fourth: Partial group name match");
      assert.strictEqual(results[4].username, "player", "Fifth: Exact user name match");
      assert.strictEqual(results[5].username, "gamer", "Sixth: Partial user name match");
      assert.strictEqual(results[6].full_name, "Test", "Seventh: Exact group full name match");
      assert.strictEqual(results[7].full_name, "Testers", "Eighth: Partial group full name match");
      assert.strictEqual(results[8].username, "user123", "Ninth/Last: Metadata match");
    });
  });

  test("it includes all matching groups when limit allows", async ({ page }) => {
    await userSearchPage(page, (route) => {
      if (route.request().url().includes("term=dev")) {
        return json(route, {
          users: [{ username: "developer", name: "Developer" }],
          groups: [
            { name: "devs", usernames: [] },
            { name: "development", usernames: [] },
            { name: "dev-team", usernames: [] },
          ],
        });
      }
      return json(route, { users: [], groups: [] });
    });
    await run(page, async (assert) => {
      const results = await window.Discourse.autocomplete.userSearch({ term: "dev", limit: 10 });
      const groupNames = results.groups.map((g) => g.name);

      // All groups should be included since limit allows
      ["devs", "development", "dev-team"].forEach((groupName) => {
        assert.true(groupNames.includes(groupName), `Should include '${groupName}' group`);
      });
    });
  });

  test("it stops searching while the advertised rate limit wait is running", async ({ page }) => {
    let requestCount = 0;
    // middlewareRateLimit(60)
    await userSearchPage(page, (route) => {
      requestCount++;
      return route.fulfill({
        status: 429,
        headers: { "Retry-After": "60", "Discourse-Rate-Limit-Error-Code": "user_10_secs_limit" },
        contentType: "text/plain",
        body: "Slow down, you're making too many requests.\nPlease retry again in 60 seconds.\n",
      });
    });
    await run(page, async (assert) => {
      const { userSearch, CANCELLED } = window.Discourse.autocomplete;
      let result = await userSearch({ term: "ratelimited" });
      assert.strictEqual(result, CANCELLED, "the rejection is handled instead of bubbling up");

      result = await userSearch({ term: "ratelimitedtoo" });
      assert.strictEqual(result, CANCELLED);
    });
    expect(requestCount, "no further request is sent before the wait is over").toBe(1);
  });

  test("it protects cached results from mutation by consumers", async ({ page }) => {
    await userSearchPage(page, (route) => {
      if (route.request().url().includes("term=test_cache")) {
        return json(route, {
          users: [
            {
              id: 123,
              username: "test_cache_user",
              name: "Test Cache User",
              avatar_template: "https://avatars.discourse.org/v3/letter/t/41988e/{size}.png",
            },
          ],
          groups: [],
        });
      }
      return json(route, { users: [], groups: [] });
    });
    await run(page, async (assert) => {
      const userSearch = window.Discourse.autocomplete.userSearch;
      // assert.present and assert.blank
      const present = (value, message) => assert.ok(value !== undefined && value !== null && value !== "", message);
      const blank = (value, message) => assert.ok(value === undefined || value === null || value === "", message);

      // First userSearch call - this will populate the cache
      let firstResults = await userSearch({ term: "test_cache" });
      assert.strictEqual(firstResults.length, 1, "First call should return 1 result");

      const userFromFirstCall = firstResults[0];
      assert.strictEqual(userFromFirstCall.username, "test_cache_user", "First call should have username");
      assert.strictEqual(userFromFirstCall.name, "Test Cache User", "First call should have name");
      present(userFromFirstCall.avatar_template, "First call should have avatar_template");

      const originalKeys = Object.keys(userFromFirstCall);
      // Simulate what can happen in consumers of the cache result that mutate that object
      Object.keys(userFromFirstCall).forEach((key) => {
        if (key === "id") {
          return;
        }
        delete userFromFirstCall[key];
      });

      // Verify the mutation happened
      present(userFromFirstCall.id, "User object should still have id");
      assert.strictEqual(Object.keys(userFromFirstCall).length, 1, "User object should be mutated to only have id");
      ["username", "name", "avatar_template"].forEach((key) => {
        blank(userFromFirstCall[key], `User object should not have ${key} after mutation`);
      });

      // Second userSearch call - this should return fresh objects from cache that are not mutated
      let secondResults = await userSearch({ term: "test_cache" });
      assert.strictEqual(secondResults.length, 1, "Second call should return 1 result");

      const userFromSecondCall = secondResults[0];
      assert.strictEqual(
        userFromSecondCall.username,
        "test_cache_user",
        "Second call should have username (cache should be protected from mutation)"
      );
      assert.strictEqual(
        userFromSecondCall.name,
        "Test Cache User",
        "Second call should have name (cache should be protected from mutation)"
      );
      present(
        userFromSecondCall.avatar_template,
        "Second call should have avatar_template (cache should be protected from mutation)"
      );
      assert.strictEqual(
        Object.keys(userFromSecondCall).length,
        originalKeys.length,
        "Second call should have all original properties"
      );
    });
  });
});

test.describe("Integration | Component | UserAutocompleteResults", () => {
  // render(<UserAutocompleteResults @results @selectedIndex />), in a
  // host of the test's own.
  const renderBody = async (assert, modules, args) => {
    const host = document.createElement("div");
    host.id = "user-autocomplete-host";
    document.body.appendChild(host);
    const user = {
      isUser: true,
      username: "sam",
      name: "Sam Saffron",
      avatar_template: "/letter_avatar_proxy/v4/letter/s/3be4f8/{size}.png",
      cssClasses: "is-online",
    };
    const email = { isEmail: true, username: "sam@example.com" };
    const group = {
      isGroup: true,
      name: "team",
      full_name: "The Team",
      // Only users carry custom classes, so this one must be ignored.
      cssClasses: "is-ignored",
    };
    host.innerHTML = window.Discourse.autocomplete.renderUsers([user, email, group], args.selectedIndex);
    const a = (n) => `#user-autocomplete-host li:nth-child(${n}) a`;

    if (args.selectedIndex === -1) {
      assert.dom(a(1)).hasAttribute("title", "Sam Saffron");
      assert.dom(a(2)).hasAttribute("title", "sam@example.com");
      assert.dom(a(3)).hasAttribute("title", "The Team");
    } else {
      assert.dom(a(1)).hasClass("is-online");
      assert.dom(a(1)).doesNotHaveClass("selected");
      // hasNoClass
      assert.strictEqual(document.querySelector(a(2)).classList.length, 0, "the email has no class");
      assert.dom(a(3)).doesNotHaveClass("is-ignored");
      assert.dom(a(3)).hasClass("selected");
    }
  };

  test("titles each kind of result by its own name field", async ({ page }) => {
    await openEditorPage(page);
    await page.addScriptTag({ content: asset("js/autocomplete.js") });
    expect(await inPage(page, renderBody, { selectedIndex: -1 })).toEqual([]);
  });

  test("applies custom classes to users only, plus the selection", async ({ page }) => {
    await openEditorPage(page);
    await page.addScriptTag({ content: asset("js/autocomplete.js") });
    expect(await inPage(page, renderBody, { selectedIndex: 2 })).toEqual([]);
  });
});
