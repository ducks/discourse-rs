// The composer's autocompletes (d-editor's _applyMentionAutocomplete,
// _applyHashtagAutocomplete and _applyEmojiAutocomplete over
// static/js/autocomplete.js) in the markdown editor: the reference's
// acceptance tests (composer-editor-mentions-test.js, emoji-test.js,
// hashtag-autocomplete-test.js) and system specs (composer_spec.rb's
// mention priorities, hashtag_autocomplete_spec.rb, emoji_deny_list_spec.rb),
// with pretender's responses as routes. The rich editor's are in
// rich-editor-autocomplete.spec.js.
const { test, expect, login, api, topic, sql, label, member } = require("./helpers");

const editor = (page) => page.locator("#reply-control.open textarea.d-editor-input");
const menu = (page) => page.locator(".fk-d-menu[data-identifier='d-autocomplete'] .autocomplete");

// qunit-helpers' simulateKeys: the keys typed one by one, "\r" Enter, "\t"
// Tab and "\b" Backspace. Each waits settled in the reference, the
// debounced search included: Enter and Tab wait for the menu.
async function simulateKeys(page, text) {
  for (const ch of text) {
    if (ch === "\r" || ch === "\t") {
      await expect(menu(page)).toBeVisible();
      await page.keyboard.press(ch === "\r" ? "Enter" : "Tab");
    } else if (ch === "\b") {
      await page.keyboard.press("Backspace");
    } else {
      await page.keyboard.type(ch);
    }
  }
}

// settled for the emoji search: the menu shows the term's own results
// (an earlier term's, a bare colon's defaults, may still be up).
async function settledEmoji(page, first) {
  await expect(page.locator(".autocomplete.ac-emoji .emoji-shortname").first()).toHaveText(first);
}

// A member of the test's own on the topic list, a new topic's composer
// open and the editor focused (click("#create-topic")).
async function createTopic(page, context) {
  const writer = member();
  await login(context, writer.name);
  await page.goto("/latest");
  await page.locator("#create-topic").click();
  await expect(editor(page)).toBeVisible();
  await expect(editor(page)).toBeEnabled();
  await editor(page).focus();
  return writer;
}

// A reply's composer on a topic of user0's.
async function replyToTopic(page, context, session) {
  const user0 = await session("user0");
  const t = await topic(user0, label("Autocomplete topic"), label("A topic the autocomplete specs reply to"));
  const writer = member();
  await login(context, writer.name);
  await page.goto(t.url);
  await page.locator("#topic-footer-buttons .create").click();
  await expect(editor(page)).toBeVisible();
  await expect(editor(page)).toBeEnabled();
  await editor(page).focus();
  return { t, writer };
}

async function respond(page, pattern, body) {
  await page.route(pattern, (route) =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(body) })
  );
}

test.describe("Composer - editor mentions", () => {
  const status = {
    emoji: "tooth",
    description: "off to dentist",
    ends_at: "2100-02-01T09:00:00.000Z",
  };

  const searchResponse = {
    users: [
      {
        username: "user",
        name: "Some User",
        avatar_template: "/letter_avatar_proxy/v4/letter/t/41988e/{size}.png",
        status,
      },
      {
        username: "user2",
        name: "Some User",
        avatar_template: "/letter_avatar_proxy/v4/letter/t/41988e/{size}.png",
      },
      {
        username: "johndoe",
        name: "John Doe Jr",
        avatar_template: "/letter_avatar_proxy/v4/letter/j/41988e/{size}.png",
      },
      {
        username: "foo",
        avatar_template: "/letter_avatar_proxy/v4/letter/t/41988e/{size}.png",
      },
    ],
    groups: [{ name: "user_group", full_name: "Group" }],
  };

  test.beforeEach(async ({ page }) => {
    await respond(page, "**/u/search/users?**", searchResponse);
  });

  test("selecting user mentions", async ({ page, context }) => {
    await createTopic(page, context);

    await simulateKeys(page, "abc @u\r");

    await expect(editor(page), "replaces mention correctly").toHaveValue("abc @user ");
  });

  test("selecting user mentions after deleting characters", async ({ page, context }) => {
    await createTopic(page, context);

    await simulateKeys(page, "abc @user a\b\b\r");

    await expect(editor(page), "replaces mention correctly").toHaveValue("abc @user ");
  });

  test("selecting user mentions after deleting characters mid sentence", async ({ page, context }) => {
    await createTopic(page, context);

    await simulateKeys(page, "abc @user 123");
    await editor(page).evaluate((el) => el.setSelectionRange(9, 9));
    await simulateKeys(page, "\b\b\r");

    await expect(editor(page), "replaces mention correctly").toHaveValue("abc @user 123");
  });

  test("shows status on search results when mentioning a user", async ({ page, context }) => {
    await createTopic(page, context);

    await simulateKeys(page, "@u");

    await expect(page.locator(`.autocomplete .emoji[alt='${status.emoji}']`), "status emoji is shown").toHaveCount(1);
    await expect(page.locator(".autocomplete .user-status-message-description"), "status description is shown").toHaveText(
      status.description
    );
  });

  test("metadata matches are moved to the end", async ({ page, context }) => {
    await createTopic(page, context);

    await simulateKeys(page, "abc @u");

    await expect(page.locator(".ac-user .username")).toHaveText(["user", "user2", "user_group", "johndoe", "foo"]);

    await simulateKeys(page, "\bf");

    await expect(page.locator(".ac-user .username")).toHaveText(["foo", "user", "user2", "johndoe"]);
  });

  test("shows users and groups immediately when @ is typed in a reply", async ({ page, context, session }) => {
    await replyToTopic(page, context, session);

    await simulateKeys(page, "abc @");

    await expect(page.locator(".ac-user .username")).toHaveText(["user", "user2", "johndoe", "foo", "user_group"]);
  });

  test("selecting user mentions by partial full name", async ({ page, context }) => {
    await createTopic(page, context);

    await simulateKeys(page, "abc @John Doe J\r");

    await expect(editor(page), "replaces full name mention correctly").toHaveValue("abc @johndoe ");
  });
});

// composer_spec.rb's "in a topic, the autocomplete prioritizes": the
// eager search ("@" alone) lists the topic's participants, the one replied
// to first.
test.describe("Composer - in a topic, the autocomplete prioritizes", () => {
  // A topic by a member of the test's own with a reply by another; the
  // topic's owner never seen, as the reference's fabricated one (the
  // others go by last_seen_at).
  async function setup(context, session) {
    const owner = member();
    const second = member();
    const t = await topic(await session(owner.name), label("Mention priority topic"), label("The first post of the mention priority topic"), [
      { context: await session(second.name), raw: label("The second reply") },
    ]);
    sql("UPDATE users SET last_seen_at = NULL WHERE id = :'p1'", owner.id);
    sql("UPDATE users SET last_seen_at = now() - interval '1 day' WHERE id = :'p1'", second.id);
    return { t, owner, second };
  }

  async function replyTo(page, postId) {
    const post = page.locator(`article[data-post-id="${postId}"]`);
    await post.locator(".post-action-menu__reply").click();
    await expect(editor(page)).toBeVisible();
    await expect(editor(page)).toBeEnabled();
    await editor(page).focus();
  }

  test("the topic owner if replying to topic", async ({ page, context, session }) => {
    const { t, owner, second } = await setup(context, session);
    const writer = member();
    await login(context, writer.name);
    await page.goto(t.url);

    await replyTo(page, t.posts[0]);
    await simulateKeys(page, "@");

    // must be first the topic owner
    await expect(page.locator(".ac-user .username")).toHaveText([owner.name, second.name]);
  });

  test("the recipient of the reply when replying", async ({ page, context, session }) => {
    const { t, owner, second } = await setup(context, session);
    const writer = member();
    await login(context, writer.name);
    await page.goto(t.url);

    await replyTo(page, t.posts[1]);
    await simulateKeys(page, "@");

    // must be first the reply user
    await expect(page.locator(".ac-user .username")).toHaveText([second.name, owner.name]);
  });

  test("the recipient of the reply when editing a reply", async ({ page, context, session }) => {
    const { t, owner, second } = await setup(context, session);
    const writer = member();
    await login(context, writer.name);
    const reply = await api(context, "POST", "/posts.json", {
      topic_id: String(t.id),
      reply_to_post_number: "2",
      raw: label("The writer's reply to the second"),
    });
    sql("UPDATE users SET last_seen_at = now() WHERE id = :'p1'", writer.id);

    await login(context, "admin");
    await page.goto(t.url);
    const post = page.locator(`article[data-post-id="${reply.id}"]`);
    const showMore = post.locator(".post-action-menu__show-more");
    if (await showMore.isVisible()) {
      await showMore.click();
    }
    await post.locator(".post-action-menu__edit").click();
    await expect(editor(page)).not.toHaveValue("");
    await editor(page).focus();
    await editor(page).press("End");

    await simulateKeys(page, " @");

    await expect(page.locator(".ac-user .username")).toHaveText([second.name, writer.name, owner.name]);
  });
});

// emoji-test.js
test.describe("Emoji", () => {
  test.beforeEach(async ({ page }) => {
    await respond(page, "**/emojis/search-aliases.json*", []);
  });

  const preview = (page) => page.locator("#reply-control .d-editor-preview");

  // The preview's emoji image as the reference's hasHtml checks it.
  async function expectEmoji(page, name, path) {
    const img = preview(page).locator("p img.emoji");
    await expect(img).toHaveCount(1);
    await expect(img).toHaveAttribute("src", new RegExp(`/images/emoji/twitter/${path}\\.png\\?v=`));
    await expect(img).toHaveAttribute("title", `:${name}:`);
    await expect(img).toHaveAttribute("alt", `:${name}:`);
    await expect(preview(page).locator("p")).toHaveText("a ");
  }

  test("emoji is cooked properly", async ({ page, context, session }) => {
    await replyToTopic(page, context, session);

    await simulateKeys(page, "a :blonde_woman\t");

    await expectEmoji(page, "blonde_woman", "blonde_woman");
  });

  test("emoji can be picked from the emoji-picker using the mouse", async ({ page, context, session }) => {
    await replyToTopic(page, context, session);

    await simulateKeys(page, "a :man_b");
    await expect(page.locator(".autocomplete.ac-emoji .emoji-shortname").first()).toHaveText(/^man_b/);

    // the 6th item in the list is the "more..."
    await page.locator(".autocomplete.ac-emoji ul li:nth-of-type(6) a").click();
    await page.locator(".emoji-picker__section.filtered img.emoji[data-emoji='man_biking']").click();

    await expectEmoji(page, "man_biking", "man_biking");
  });

  test("skin toned emoji is cooked properly", async ({ page, context, session }) => {
    await replyToTopic(page, context, session);

    await editor(page).fill("a :blonde_woman:t5:");

    await expectEmoji(page, "blonde_woman:t5", "blonde_woman/5");
  });

  test("emoji autocomplete after punctuation", async ({ page, context, session }) => {
    await replyToTopic(page, context, session);

    await simulateKeys(page, "Hello world,:smi");
    await settledEmoji(page, "smiley");

    await simulateKeys(page, "\t");

    await expect(editor(page), "emoji inserted at correct position after comma").toHaveValue("Hello world,:smiley: ");
  });

  test("emoji autocomplete should not trigger after whitespace", async ({ page, context, session }) => {
    await replyToTopic(page, context, session);

    await simulateKeys(page, ":id ");
    // The search the "d" started has answered.
    await page.waitForTimeout(500);
    await expect(
      page.locator(".autocomplete.ac-emoji"),
      "emoji autocomplete should close after whitespace is added to the term"
    ).toHaveCount(0);
  });

  for (const ellipsis of ["...", "…"]) {
    test(`emoji autocomplete after ${ellipsis}`, async ({ page, context, session }) => {
      await replyToTopic(page, context, session);

      await simulateKeys(page, `Hello world${ellipsis}:smi`);
      await settledEmoji(page, "smiley");

      await simulateKeys(page, "\t");

      await expect(editor(page)).toHaveValue(`Hello world${ellipsis}:smiley: `);
    });
  }

  test("emoji autocomplete after an ellipsis changes while the menu is open", async ({ page, context, session }) => {
    await replyToTopic(page, context, session);

    await simulateKeys(page, "Hello world...:smi");
    await settledEmoji(page, "smiley");

    await editor(page).fill("Hello world…:smi");
    await simulateKeys(page, "\t");

    await expect(editor(page)).toHaveValue("Hello world…:smiley: ");
  });

  test("emoji autocomplete inserts at the cursor if the trigger disappears", async ({ page, context, session }) => {
    await replyToTopic(page, context, session);

    await simulateKeys(page, "Hello world...:smi");
    await settledEmoji(page, "smiley");

    await editor(page).fill("Hello world");
    await editor(page).evaluate((el) => el.setSelectionRange(6, 6));
    await simulateKeys(page, "\t");

    await expect(editor(page)).toHaveValue("Hello :smiley: world");
  });

  test("siteSetting:emoji_autocomplete_min_chars", async ({ page, context, session }) => {
    const admin = await session("admin");
    const [[before]] = sql("SELECT COALESCE((SELECT value FROM site_settings WHERE name = 'emoji_autocomplete_min_chars'), '0')");
    await api(admin, "PUT", "/admin/site_settings/emoji_autocomplete_min_chars", { emoji_autocomplete_min_chars: "2" });
    try {
      await replyToTopic(page, context, session);

      await simulateKeys(page, ":s");
      await page.waitForTimeout(500);
      await expect(page.locator(".autocomplete.ac-emoji")).toHaveCount(0);

      await simulateKeys(page, "w");
      await expect(page.locator(".autocomplete.ac-emoji")).toBeVisible();
    } finally {
      await api(admin, "PUT", "/admin/site_settings/emoji_autocomplete_min_chars", { emoji_autocomplete_min_chars: before });
    }
  });
});

// emoji_deny_list_spec.rb's "when using composer"
test("hides denied emojis and aliases from emoji autocomplete", async ({ page, context, session }) => {
  const admin = await session("admin");
  await api(admin, "PUT", "/admin/site_settings/emoji_deny_list", { emoji_deny_list: "fu|poop" });
  try {
    await replyToTopic(page, context, session);

    // shows no results
    await simulateKeys(page, ":poop");
    await page.waitForTimeout(500);
    await expect(page.locator(".autocomplete.ac-emoji .emoji-shortname", { hasText: /^poop?$/ })).toHaveCount(0);

    await editor(page).fill("");

    // middle_finger is alias
    await simulateKeys(page, ":middle");
    await page.waitForTimeout(500);
    await expect(page.locator(".autocomplete.ac-emoji .emoji-shortname", { hasText: /^fu$/ })).toHaveCount(0);
  } finally {
    await api(admin, "PUT", "/admin/site_settings/emoji_deny_list", { emoji_deny_list: "" });
  }
});

// hashtag-autocomplete-test.js
test.describe("#hashtag autocompletion in composer", () => {
  const results = [
    {
      id: 28,
      text: ":bug: Other Languages",
      slug: "other-languages",
      colors: ["FF0000"],
      icon: "folder",
      relative_url: "/c/other-languages/28",
      ref: "other-languages",
      type: "category",
      style_type: "square",
    },
    {
      id: 99,
      text: "Fun stuff",
      slug: "fun-stuff",
      colors: ["55FF00"],
      icon: "bell",
      relative_url: "/c/fun-stuff/99",
      ref: "fun-stuff",
      type: "category",
      style_type: "icon",
    },
    {
      id: 123,
      text: "organization",
      slug: "organization",
      colors: ["0000FF", "00FFFF"],
      emoji: "joy",
      icon: "folder",
      relative_url: "/c/organization/123",
      ref: "organization",
      type: "category",
      style_type: "emoji",
    },
    {
      id: 300,
      text: "notes x 300",
      slug: "notes",
      icon: "tag",
      relative_url: "/tag/notes",
      ref: "notes",
      type: "tag",
      style_type: "icon",
    },
    {
      id: 281,
      text: "photos x 281",
      slug: "photos",
      icon: "tag",
      relative_url: "/tag/photos",
      ref: "photos",
      type: "tag",
      style_type: "icon",
    },
  ];

  test(":emoji: unescape in autocomplete search results", async ({ page, context, session }) => {
    await respond(page, "**/hashtags/search.json?**", { results });
    await replyToTopic(page, context, session);
    await simulateKeys(page, "abc #o");

    await expect(page.locator(".hashtag-autocomplete__option")).toHaveCount(5);
    await expect(
      page.locator('.hashtag-autocomplete__option .hashtag-autocomplete__text img.emoji[title="bug"]')
    ).toHaveCount(1);
  });

  test("escapes hashtag display text in the composer preview", async ({ page, context, session }) => {
    const name = '<b id="hashtag-injected">Meeting</b>';
    await respond(page, "**/hashtags?**", {
      category: [
        {
          id: 28,
          text: name,
          slug: "meeting-room",
          ref: "meeting-room",
          type: "category",
          style_type: "square",
          relative_url: "/c/meeting-room/28",
        },
      ],
    });
    await respond(page, "**/hashtags/search.json?**", { results });
    await replyToTopic(page, context, session);
    await simulateKeys(page, "Visit #meeting-room ");
    const cooked = page.locator("#reply-control .d-editor-preview a.hashtag-cooked");
    await expect(cooked).toBeVisible();

    await expect(cooked, "the name remains literal text").toHaveText(name);
    await expect(
      page.locator("#reply-control .d-editor-preview #hashtag-injected"),
      "the name does not create an element"
    ).toHaveCount(0);
  });

  test("autocomplete supports periods in the middle of tag names", async ({ page, context, session }) => {
    const terms = [];
    await page.route("**/hashtags/search.json?**", (route) => {
      terms.push(new URL(route.request().url()).searchParams.get("term"));
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          results: [
            {
              id: 301,
              text: "sam.i.am x 1",
              slug: "sam.i.am",
              icon: "tag",
              relative_url: "/tag/sam-i-am/301",
              ref: "sam.i.am",
              type: "tag",
              style_type: "icon",
            },
          ],
        }),
      });
    });

    await replyToTopic(page, context, session);
    await simulateKeys(page, "abc #sam.i");

    await expect(page.locator(".hashtag-autocomplete__option")).toHaveCount(1);
    expect(terms).toContain("sam.i");
    await expect(page.locator(".hashtag-autocomplete__option")).toHaveText("sam.i.am x 1");
  });

  test("iconHTML is correctly generated for various hashtag types and style_type configs", async ({
    page,
    context,
    session,
  }) => {
    await respond(page, "**/hashtags/search.json?**", { results });
    await replyToTopic(page, context, session);
    await simulateKeys(page, "abc #o");

    await expect(page.locator(".hashtag-autocomplete__option")).toHaveCount(5);
    await expect(page.locator(".hashtag-autocomplete__option .d-icon.d-icon-tag.hashtag-color--tag-300")).toHaveCount(1);
    await expect(
      page.locator(".hashtag-autocomplete__option .hashtag-category-square.hashtag-color--category-28")
    ).toHaveCount(1);
    await expect(
      page.locator(".hashtag-autocomplete__option .hashtag-category-icon.hashtag-color--category-99 svg.svg-icon.d-icon-bell")
    ).toHaveCount(1);
    await expect(
      page.locator(".hashtag-autocomplete__option .hashtag-category-emoji.hashtag-color--category-123 img.emoji[title='joy']")
    ).toHaveCount(1);
  });
});

// hashtag-autocomplete-test.js' "with types unknown to the client". The
// client here draws category, tag and channel, so the types requested are
// the composer's configured ones (site.hashtag_configurations), and an
// unknown type's row is drawn with no icon.
test.describe("#hashtag autocompletion with types unknown to the client", () => {
  const results = [
    {
      id: 1,
      text: "Music Lounge",
      slug: "music",
      icon: "comment",
      relative_url: "/chat/c/music/1",
      ref: "music",
      type: "unknown",
      style_type: "icon",
    },
    {
      id: 28,
      text: "Other Languages",
      slug: "other-languages",
      colors: ["FF0000"],
      icon: "folder",
      relative_url: "/c/other-languages/28",
      ref: "other-languages",
      type: "category",
      style_type: "square",
    },
  ];

  test("only requests types that have a client-side class registered", async ({ page, context, session }) => {
    const orders = [];
    await page.route("**/hashtags/search.json?**", (route) => {
      orders.push(new URL(route.request().url()).searchParams.getAll("order[]"));
      route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ results }) });
    });
    await replyToTopic(page, context, session);
    await simulateKeys(page, "abc #filter-known");
    await expect(page.locator(".hashtag-autocomplete__option")).toHaveCount(2);

    const configured = await page.evaluate(
      () => JSON.parse(document.getElementById("reply-control").dataset.richEditorContext).site.hashtag_configurations["topic-composer"]
    );
    expect(configured.slice(0, 2)).toEqual(["category", "tag"]);
    expect(orders[orders.length - 1]).toEqual(configured.filter((type) => ["category", "tag", "channel"].includes(type)));
  });

  test("does not crash when the server returns a type not registered on the client", async ({ page, context, session }) => {
    await respond(page, "**/hashtags/search.json?**", { results });
    await replyToTopic(page, context, session);
    await simulateKeys(page, "abc #unknown-type");

    // The row of the type the client doesn't know still renders, without
    // an icon, and the promise must not reject.
    await expect(page.locator(".hashtag-autocomplete__option")).toHaveCount(2);
    await expect(page.locator(".hashtag-autocomplete__option").first().locator("svg, img, span[class^='hashtag-category']")).toHaveCount(0);
  });
});

// hashtag_autocomplete_spec.rb, over a category and a tag of the test's
// own (the "co" both start with is a prefix of their own here).
test.describe("Using #hashtag autocompletion to search for and lookup categories and tags", () => {
  async function setup(page, context, session) {
    const prefix = "co" + Math.floor(Math.random() * 1e9).toString(36);
    const admin = await session("admin");
    const created = await api(admin, "POST", "/categories.json", {
      name: `Cool ${prefix}`,
      slug: `${prefix}-cat`,
      color: "0088CC",
      text_color: "FFFFFF",
    });
    const category = created.category;
    const [[tagId]] = sql(
      "INSERT INTO tags (name, slug, staff_topic_count, public_topic_count, created_at, updated_at) \
       VALUES (:'p1', :'p1', 324, 324, now(), now()) RETURNING id",
      `${prefix}tag`
    );
    await replyToTopic(page, context, session);
    return { prefix, category, tag: { id: tagId, name: `${prefix}tag` } };
  }

  // have_tag's `with: { class: }`: the classes are among the element's.
  async function hasClasses(locator, classes) {
    for (const name of classes.split(" ")) {
      await expect(locator).toHaveClass(new RegExp(`(^|\\s)${name}(\\s|$)`));
    }
  }

  async function initiate(page, text, count) {
    await simulateKeys(page, text);
    await expect(page.locator(".hashtag-autocomplete .hashtag-autocomplete__option .hashtag-autocomplete__link")).toHaveCount(
      count
    );
  }

  test("searches for categories and tags with # and prioritises categories in the results", async ({
    page,
    context,
    session,
  }) => {
    const { prefix } = await setup(page, context, session);
    await initiate(page, `something #${prefix}`, 2);

    await expect(page.locator(".hashtag-autocomplete__link")).toHaveText([`Cool ${prefix}`, `${prefix}tag (x324)`]);
  });

  test("cooks the selected category hashtag clientside in the composer preview with the correct url and icon", async ({
    page,
    context,
    session,
  }) => {
    const { prefix, category } = await setup(page, context, session);
    await initiate(page, `something #${prefix}`, 2);
    await page.locator(".hashtag-autocomplete__link").nth(0).click();

    const cooked = page.locator("#reply-control .d-editor-preview .hashtag-cooked");
    await expect(cooked).toBeVisible();
    await expect(cooked).toHaveAttribute("href", `/c/${category.slug}/${category.id}`);
    await expect(cooked).toHaveAttribute("data-type", "category");
    await expect(cooked).toHaveAttribute("data-slug", category.slug);
    await expect(cooked).toHaveAttribute("data-id", String(category.id));
    await hasClasses(cooked.locator("span").first(), `hashtag-category-square hashtag-color--category-${category.id}`);
  });

  test("cooks the selected tag hashtag clientside in the composer preview with the correct url and icon", async ({
    page,
    context,
    session,
  }) => {
    const { prefix, tag } = await setup(page, context, session);
    await initiate(page, `something #${prefix}`, 2);
    await page.locator(".hashtag-autocomplete__link").nth(1).click();

    const cooked = page.locator("#reply-control .d-editor-preview .hashtag-cooked");
    await expect(cooked).toBeVisible();
    await expect(cooked).toHaveAttribute("href", `/tag/${tag.name}/${tag.id}`);
    await expect(cooked).toHaveAttribute("data-type", "tag");
    await expect(cooked).toHaveAttribute("data-slug", tag.name);
    await expect(cooked).toHaveAttribute("data-id", String(tag.id));
    await hasClasses(cooked.locator("svg"), `fa d-icon d-icon-tag svg-icon hashtag-color--tag-${tag.id} svg-string`);
    await expect(cooked.locator("svg use")).toHaveAttribute("href", "#tag");
  });

  test("does not show hashtag autocomplete when typing a URL with anchor", async ({ page, context, session }) => {
    const { prefix } = await setup(page, context, session);

    // normal autocomplete works
    await simulateKeys(page, `#${prefix}`);
    await expect(page.locator(".hashtag-autocomplete")).toBeVisible();

    // autocomplete does not trigger within a URL
    await editor(page).fill("");
    await page.keyboard.press("Escape");
    await simulateKeys(page, `discourse.org/#${prefix}`);
    await page.waitForTimeout(2000);
    await expect(page.locator(".hashtag-autocomplete")).toHaveCount(0);
  });
});
