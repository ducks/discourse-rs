// ComposerMessages (static/js/composer-messages.js): the reference's
// acceptance tests composer-messages-test.js and composer-test.js'
// duplicate link and group mentioned notices, with pretender's responses
// as routes, and the system spec dont_feed_the_trolls_popup_spec.rb
// against the server. The rich editor's popup is in
// rich-editor-autocomplete.spec.js. Not ported: the similar topics test
// (GET /similar_topics is not routed).
const { test, expect, login, api, topic, label, member } = require("./helpers");

const editor = (page) => page.locator("#reply-control.open textarea.d-editor-input");
const popup = (page) => page.locator("#reply-control .composer-popup");

async function respond(page, pattern, body) {
  await page.route(pattern, (route) =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(body) })
  );
}

// triggerKeyEvent(".d-editor-input", "keyup", "Space"): the key's release
// alone.
async function keyup(page) {
  await editor(page).dispatchEvent("keyup", { key: " ", code: "Space" });
}

// A topic with a reply (a topic of one post skips the duplicate link
// check), and a member of the test's own replying to it, the composer
// open and its messages asked for.
async function replyToTopic(page, context, session) {
  const user0 = await session("user0");
  const user1 = await session("user1");
  const t = await topic(user0, label("Composer messages topic"), label("A topic the composer messages reply to"), [
    { context: user1, raw: label("A reply to the composer messages topic") },
  ]);
  const writer = member();
  await login(context, writer.name);
  await page.goto(t.url);
  const asked = page.waitForResponse((r) => new URL(r.url()).pathname === "/composer_messages");
  await page.locator("#topic-footer-buttons .create").click();
  await expect(editor(page)).toBeEnabled();
  await asked;
  return { t, writer };
}

test.describe("Composer - Messages", () => {
  test("Shows warning in composer if user hasn't been seen in a long time.", async ({ page, context }) => {
    let userNotSeenRequestCount = 0;
    await page.route("**/composer_messages/user_not_seen_in_a_while?**", (route) => {
      userNotSeenRequestCount += 1;
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ user_count: 1, usernames: ["charlie"], time_ago: "1 year ago" }),
      });
    });
    const writer = member();
    const charlie = member();
    await login(context, writer.name);
    await page.goto(`/u/${charlie.name}`);
    await page.locator("button.compose-pm").click();
    await expect(editor(page)).toBeEnabled();

    await expect(popup(page), "composer warning is not shown by default").toHaveCount(0);

    await keyup(page);

    await expect(popup(page), "shows composer warning message").toBeVisible();
    const body =
      "The person you are messaging, <b><a class=\"mention\" href=\"/u/charlie\">@charlie</a></b>, " +
      "hasn’t been seen here in a very long time – 1 year ago.";
    expect(await popup(page).innerHTML(), "warning message has correct body").toContain(body);

    expect(userNotSeenRequestCount, "one user not seen request is made to the server").toBe(1);

    await keyup(page);
    await page.waitForTimeout(500);

    expect(
      userNotSeenRequestCount,
      "does not make additional user not seen request to the server if the recipient names are the same"
    ).toBe(1);
  });
});

test.describe("Composer - Messages - Cannot see group", () => {
  test.beforeEach(async ({ page }) => {
    await respond(page, "**/composer/mentions?**", {
      users: [],
      user_reasons: {},
      groups: {
        staff: { user_count: 30 },
        staff2: { user_count: 30, notified_count: 10 },
      },
      group_reasons: { staff: "not_allowed", staff2: "some_not_allowed" },
      max_users_notified_per_group_mention: 100,
    });
  });

  test("Shows warning in composer if group hasn't been invited", async ({ page, context, session }) => {
    await replyToTopic(page, context, session);
    await expect(popup(page), "composer warning is not shown by default").toHaveCount(0);

    await editor(page).fill("Mention @staff");
    await expect(popup(page), "shows composer warning message").toBeVisible();
    await expect(popup(page), "warning message has correct body").toContainText(
      "You mentioned @staff but none of its members will be notified because they are unable to see this personal message."
    );
  });

  test("Shows warning in composer if group hasn't been invited, but some members have access already", async ({
    page,
    context,
    session,
  }) => {
    await replyToTopic(page, context, session);
    await expect(popup(page), "composer warning is not shown by default").toHaveCount(0);

    await editor(page).fill("Mention @staff2");
    await expect(popup(page), "shows composer warning message").toBeVisible();
    await expect(popup(page), "warning message has correct body").toContainText(
      "You mentioned @staff2 but only 10 members will be notified because the other members are unable to see this personal message."
    );
  });
});

test.describe("Composer - Messages - Duplicate links", () => {
  test("Shows the warning", async ({ page, context, session }) => {
    await respond(page, "**/inline-onebox?**", { "inline-oneboxes": [] });
    let receivedMessages = false;
    await page.route("**/composer_messages?**", (route) => {
      receivedMessages = true;
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          composer_messages: [],
          extras: {
            duplicate_lookup: {
              "test.localhost/t/testing-topic/123/4567": {
                domain: "test.localhost",
                username: "uwe_keim",
                posted_at: "2021-01-01T12:00:00.000Z",
                post_number: 1,
              },
            },
          },
        }),
      });
    });

    await replyToTopic(page, context, session);

    await expect(popup(page), "composer warning is not shown by default").toHaveCount(0);
    expect(receivedMessages).toBe(true);

    await editor(page).fill("Here's a link: https://test.localhost/t/testing-topic/123/4567");

    await expect(
      page.locator("#reply-control .composer-popup.duplicate-link-message"),
      "shows composer warning message"
    ).toBeVisible();
    await expect(popup(page)).toContainText(
      "It looks like your link to test.localhost was already posted in the topic by @uwe_keim in a reply on Jan 1, 2021"
    );
  });
});

test.describe("Composer - Messages - Education", () => {
  test("shows wait_for_typing message only once", async ({ page, context, session }) => {
    await respond(page, "**/composer_messages?**", {
      composer_messages: [
        {
          id: "education",
          templateName: "education",
          wait_for_typing: true,
          body: "Education message",
        },
      ],
    });

    await replyToTopic(page, context, session);

    await keyup(page);
    await expect(popup(page), "shows composer warning message").toBeVisible();
    await expect(popup(page)).toContainText("Education message");

    await popup(page).locator(".close").click();
    await expect(popup(page), "composer warning is closed").toHaveCount(0);

    await keyup(page);
    await page.waitForTimeout(500);
    await expect(popup(page), "composer warning does not show again").toHaveCount(0);
  });

  // Not in the reference's tests: hitEsc closes the message first.
  test("Esc dismisses the message", async ({ page, context, session }) => {
    await respond(page, "**/composer_messages?**", {
      composer_messages: [{ id: "education", templateName: "education", body: "<p>Education message</p>" }],
    });

    await replyToTopic(page, context, session);
    await expect(popup(page)).toContainText("Education message");
    await expect(popup(page).locator(".close")).toHaveAttribute("aria-label", "dismiss message");

    await editor(page).press("Escape");
    await expect(popup(page)).toHaveCount(0);
    await expect(page.locator("#reply-control.open")).toBeVisible();
  });
});

// Not in the reference's tests: composer-messages/dominating-topic.gjs's
// Share opens ShareTopic (share-topic.js) for the topic.
test.describe("Composer - Messages - Dominating topic", () => {
  test("Share opens the share modal", async ({ page, context, session }) => {
    await respond(page, "**/composer_messages?**", {
      composer_messages: [
        {
          id: "dominating_topic",
          templateName: "dominating-topic",
          wait_for_typing: true,
          extraClass: "education-message dominating-topic-message",
          body: "<p>Dominating topic message</p>",
        },
      ],
    });

    const { t, writer } = await replyToTopic(page, context, session);

    await keyup(page);
    await expect(popup(page)).toContainText("Dominating topic message");
    await popup(page).locator(".btn-primary[data-action='share']").click();

    const modal = page.locator(".d-modal.share-topic-modal");
    await expect(modal).toBeVisible();
    await expect(modal.locator("input.invite-link")).toHaveValue(
      new URL(`${t.url}?u=${writer.name}`, page.url()).href
    );
  });
});

test.describe("Composer - Messages - Private Messages", () => {
  // The member is the one logged in; the search answers them and sam.
  async function newMessage(page, context) {
    const me = member();
    await page.route("**/composer_messages/user_not_seen_in_a_while?**", (route) =>
      route.fulfill({ status: 200, contentType: "application/json", body: "{}" })
    );
    await respond(page, "**/u/search/users?**", {
      users: [{ username: me.name }, { username: "sam" }],
    });
    await login(context, me.name);
    await page.goto("/new-message");
    await expect(editor(page)).toBeEnabled();
    return me;
  }

  const privateMessageUsers = (page) => page.locator("#private-message-users");

  async function selectRecipient(page, name) {
    await privateMessageUsers(page).locator("summary").click();
    await privateMessageUsers(page).locator(".filter-input").fill(name);
    await privateMessageUsers(page).locator(`.select-kit-row[data-value='${name}']`).click();
    await page.keyboard.press("Escape");
    await expect(privateMessageUsers(page).locator(".select-kit-body")).toBeHidden();
  }

  test("Shows warning in the composer if the user is sending a message only to himself", async ({
    page,
    context,
  }) => {
    const me = await newMessage(page, context);

    expect(
      await privateMessageUsers(page).locator("summary").getAttribute("data-value"),
      "target recipients are empty"
    ).toBeFalsy();

    await page.locator("#reply-title").fill("Private message test title");
    await keyup(page);
    await page.waitForTimeout(500);

    await expect(popup(page), "composer warning is not shown if the target recipients are empty").toHaveCount(0);

    await selectRecipient(page, me.name);

    await keyup(page);

    await expect(popup(page), "shows composer warning message").toBeVisible();
    await expect(popup(page), "warning message has correct title").toContainText("Did you forget to add recipients?");
    await expect(popup(page), "warning message has correct body").toContainText(
      "Right now this message is only being sent to yourself!"
    );
  });

  test("Does not show a warning in the composer if the message is sent to other users", async ({ page, context }) => {
    const me = await newMessage(page, context);

    await page.locator("#reply-title").fill("Private message test title");
    await keyup(page);
    await page.waitForTimeout(500);

    await expect(popup(page), "composer warning is not shown if the target recipients are empty").toHaveCount(0);

    await selectRecipient(page, "sam");
    await keyup(page);
    await page.waitForTimeout(500);
    await expect(popup(page), "do not show it for other user").toHaveCount(0);

    await selectRecipient(page, me.name);
    await keyup(page);
    await page.waitForTimeout(500);
    await expect(
      popup(page),
      "do not show it when the current user is just one of the target recipients"
    ).toHaveCount(0);
  });
});

// composer-test.js
test.describe("Composer", () => {
  test("Shows duplicate_link notice", async ({ page, context, session }) => {
    await respond(page, "**/composer_messages?**", {
      composer_messages: [],
      extras: {
        duplicate_lookup: {
          "github.com": {
            domain: "github.com",
            username: "system",
            posted_at: "2021-01-01T12:00:00.000Z",
            post_number: 1,
          },
        },
      },
    });
    await replyToTopic(page, context, session);
    const preview = page.locator("#reply-control .d-editor-preview");

    await editor(page).fill("[](https://discourse.org)");
    await expect(preview.locator("a[href='https://discourse.org']")).toHaveCount(1);
    await expect(popup(page)).toHaveCount(0);

    await editor(page).fill("[quote][](https://github.com)[/quote]");
    await expect(preview.locator("aside.quote a[href='https://github.com']")).toHaveCount(1);
    await expect(popup(page)).toHaveCount(0);

    await editor(page).fill("[](https://github.com)");
    await expect(popup(page)).toBeVisible();
  });

  test("Shows the 'group_mentioned' notice", async ({ page, context, session }) => {
    await respond(page, "**/composer/mentions?**", {
      users: [],
      user_reasons: {},
      groups: { staff: { user_count: 3 } },
      group_reasons: {},
      max_users_notified_per_group_mention: 100,
    });
    await replyToTopic(page, context, session);
    const preview = page.locator("#reply-control .d-editor-preview");

    await editor(page).fill("[quote]\n@staff\n[/quote]");
    await expect(preview.locator("aside.quote a.mention-group")).toHaveCount(1);
    // past the warning's debounce
    await page.waitForTimeout(2500);
    await expect(popup(page), "Doesn't show the 'group_mentioned' notice in a quote").toHaveCount(0);

    await editor(page).fill("@staff");
    await expect(popup(page), "shows the 'group_mentioned' notice").toBeVisible();
    await expect(popup(page)).toContainText("Mentioning @staff will notify 3 people.");
  });
});

// spec/system/composer/dont_feed_the_trolls_popup_spec.rb
test.describe("Composer don't feed the trolls popup", () => {
  test("shows a popup when about to reply to a troll", async ({ page, context, session }) => {
    const user = member();
    const troll = member();
    const userContext = await session(user.name);
    const t = await topic(userContext, label("Trolls topic"), label("A topic a troll replies to"), [
      { context: await session(troll.name), raw: label("A reply from a troll") },
    ]);
    const reply = t.posts[1];
    // Inappropriate where the reference fabricates a spam flag: a spam
    // flag marks the reviewable potential spam, which /review refuses
    // (review/list.rs), and the specs share one queue. The finder counts
    // both alike (flag_types_without_additional_message).
    await api(userContext, "POST", "/post_actions.json", {
      id: String(reply),
      post_action_type_id: "4",
      flag_topic: "false",
    });

    await login(context, user.name);
    await page.goto(t.url);
    await page.locator(`article[data-post-id='${reply}'] .post-action-menu__reply`).click();

    await expect(popup(page)).toContainText(
      "This post was flagged for moderator attention. Replying to negative content can encourage more negative behavior - are you sure you want to continue?"
    );
  });
});
