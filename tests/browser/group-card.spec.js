// The group card (static/js/user-card.js, components/group-card-contents.gjs
// on card-contents-base.js): a group mention in a post, or a group in the
// docked header of a message, opens it with the group's flair, names, bio,
// first members and the count of the rest; Join and Leave are
// GroupMembershipButton's, Message the composer's openNewMessage; Escape
// and a click outside close it.
const { test, expect, login, api, topic, sql, label, member } = require("./helpers");

const card = (page) => page.locator("#group-card");

// A group of the test's own that anyone sees and mentions, with the
// first `members` users as members. Returns { name, id }.
function group(columns = {}, members = 0) {
  const name = "gc" + Math.floor(Math.random() * 1e9).toString(36);
  const row = {
    visibility_level: 0,
    members_visibility_level: 0,
    mentionable_level: 99,
    messageable_level: 0,
    user_count: members,
    ...columns,
  };
  const keys = Object.keys(row);
  const [[id]] = sql(
    `INSERT INTO groups (name, ${keys.join(", ")}, created_at, updated_at) \
     VALUES (:'p1', ${keys.map((_, i) => `:'p${i + 2}'`).join(", ")}, now(), now()) RETURNING id`,
    name,
    ...keys.map((k) => String(row[k])),
  );
  if (members) {
    sql(
      "INSERT INTO group_users (group_id, user_id, created_at, updated_at) \
       SELECT :'p1'::int, id, now(), now() FROM users WHERE username IN ('admin', 'user0', 'user1') LIMIT :'p2'::int",
      id,
      members,
    );
  }
  return { name, id };
}

const membership = (g, user) =>
  sql("SELECT 1 FROM group_users WHERE group_id = :'p1' AND user_id = :'p2'", g.id, user.id).length;

// Opens the card on a mention and waits for the loaded card to stop
// moving (the menu's opening animation).
async function open(page, trigger, g) {
  await trigger.click();
  await expect(card(page)).toHaveClass(new RegExp(`\\bgroup-card-${g.name}\\b`));
  await expect(card(page).locator(".group-page-link")).toHaveText(g.name);
  await card(page).evaluate((el) => {
    const running = [];
    for (let node = el; node; node = node.parentElement) {
      running.push(...node.getAnimations());
    }
    return Promise.all(running.map((a) => a.finished));
  });
}

// A topic by user1 mentioning the groups, each in a paragraph of its own
// after one of plain text, read by a member of the test's own.
async function mentioning(page, context, session, groups) {
  const user1 = await session("user1");
  const raw = [label("A paragraph before the group mentions")]
    .concat(groups.map((g) => `Hello @${g.name}`))
    .join("\n\n");
  const t = await topic(user1, label("Group card"), raw);
  const reader = member();
  await login(context, reader.name);
  await page.goto(t.url);
  return { t, reader };
}

const mention = (page, g) => page.locator(`.cooked a.mention-group[href='/groups/${g.name}']`);

test("a group mention opens the group's card", async ({ page, context, session }) => {
  const first = group(
    { full_name: "The First Team", bio_raw: "We test cards.", bio_cooked: "<p>We test <b>cards</b>.</p>", user_count: 13 },
    3,
  );
  // 13 by the group's counter: three members drawn, the rest counted.
  sql("UPDATE groups SET user_count = 13 WHERE id = :'p1'", first.id);
  const second = group();
  await mentioning(page, context, session, [first, second]);

  // The mention as it is cooked.
  await expect(mention(page, first)).toHaveText(`@${first.name}`);
  await expect(mention(page, first)).toHaveClass(/\bmention-group\b/);

  const loaded = page.waitForResponse((r) => new URL(r.url()).pathname === `/group-card/${first.name}`);
  await open(page, mention(page, first), first);
  expect((await loaded).status()).toBe(200);
  expect(new URL(page.url()).pathname, "the mention's link is not followed").toMatch(/^\/t\//);
  await expect(card(page)).toHaveClass(/\bno-bg\b/);
  await expect(card(page).locator(".names__primary a.group-page-link")).toHaveAttribute("href", `/g/${first.name}`);
  await expect(card(page).locator(".names__secondary.full-name")).toHaveText("The First Team");
  await expect(card(page).locator(".group-card-avatar a.card-huge-avatar .avatar-flair svg.d-icon-users")).toBeVisible();
  await expect(card(page).locator(".second-row .bio")).toHaveText("We test cards.");
  await expect(card(page).locator(".third-row .members a.card-tiny-avatar img.avatar")).toHaveCount(3);
  await expect(card(page).locator(".more-members-link .more-members-count")).toHaveText(/^\+3\s+More$/);
  await expect(card(page).locator(".more-members-link")).toHaveAttribute("href", `/g/${first.name}`);
  // Nobody may join, leave or message it.
  await expect(card(page).locator(".usercard-controls button")).toHaveCount(0);

  // Escape closes it.
  await page.keyboard.press("Escape");
  await expect(card(page)).toHaveCount(0);

  // A click outside closes it.
  await open(page, mention(page, first), first);
  await page.locator(".cooked p").first().click({ position: { x: 3, y: 3 } });
  await expect(card(page)).toHaveCount(0);

  // The second mention opens its group's card: no full name, so its
  // name again, and no members.
  await open(page, mention(page, second), second);
  await expect(card(page).locator(".names__secondary.username")).toHaveText(second.name);
  await expect(card(page).locator(".third-row")).toHaveCount(0);
  await expect(card(page).locator(".second-row")).toHaveCount(0);

  // The user card still opens, and the group card goes.
  await page.locator(".topic-avatar [data-user-card]").first().click();
  await expect(page.locator("#user-card .user-profile-link")).toBeVisible();
  await expect(card(page)).toHaveCount(0);
  await page.keyboard.press("Escape");
  await expect(page.locator("#user-card")).toHaveCount(0);
});

test("a member joins and leaves the group from its card", async ({ page, context, session }) => {
  const g = group({ public_admission: true, public_exit: true }, 2);
  const { reader } = await mentioning(page, context, session, [g]);
  await open(page, mention(page, g), g);
  await expect(card(page).locator(".third-row a.card-tiny-avatar")).toHaveCount(2);
  const join = card(page).locator(".group-membership-button button.group-index-join");
  await expect(join).toHaveText("Join");
  expect(membership(g, reader)).toBe(0);

  // Joined: the card stays, with Leave and the new member.
  const joined = page.waitForResponse(
    (r) => r.request().method() === "PUT" && new URL(r.url()).pathname === `/groups/${g.id}/join.json`,
  );
  await join.click();
  expect((await joined).ok()).toBeTruthy();
  const leave = card(page).locator(".group-membership-button button.group-index-leave");
  await expect(leave).toHaveText("Leave");
  await expect(card(page).locator(".third-row a.card-tiny-avatar")).toHaveCount(3);
  await expect(card(page).locator(`.third-row a.card-tiny-avatar[href='/u/${reader.name}']`)).toHaveCount(1);
  expect(membership(g, reader)).toBe(1);
  expect(new URL(page.url()).pathname).toMatch(/^\/t\//);

  // Anyone may join again: leaving is not asked about.
  await leave.click();
  await expect(join).toBeVisible();
  await expect(card(page).locator(".third-row a.card-tiny-avatar")).toHaveCount(2);
  expect(membership(g, reader)).toBe(0);
});

test("leaving a group one can't join again is confirmed", async ({ page, context, session }) => {
  const g = group({ public_exit: true }, 1);
  const { reader } = await mentioning(page, context, session, [g]);
  sql(
    "INSERT INTO group_users (group_id, user_id, created_at, updated_at) VALUES (:'p1', :'p2', now(), now())",
    g.id,
    reader.id,
  );
  await open(page, mention(page, g), g);
  const leave = card(page).locator("button.group-index-leave");

  // Asked in the dialog, whose click is one outside the card. No: still
  // a member.
  const dialog = page.locator("#dialog-holder .dialog-content");
  await leave.click();
  await expect(dialog.locator(".dialog-body")).toHaveText("Are you sure you want to leave this group?");
  await dialog.locator(".dialog-footer .btn", { hasText: "No" }).click();
  await expect(dialog).toBeHidden();
  await expect(card(page)).toHaveCount(0);
  expect(membership(g, reader)).toBe(1);

  // Yes: no longer one, and nothing to do about it on the card.
  await open(page, mention(page, g), g);
  await leave.click();
  await dialog.locator(".dialog-footer .btn", { hasText: "Yes" }).click();
  await expect.poll(() => membership(g, reader)).toBe(0);
  await open(page, mention(page, g), g);
  await expect(card(page).locator(".usercard-controls button")).toHaveCount(0);
});

test("Message opens the composer to the group where it may be messaged", async ({ page, context, session }) => {
  const open_group = group({ messageable_level: 99 });
  const closed_group = group({ messageable_level: 0 });
  await mentioning(page, context, session, [closed_group, open_group]);

  await open(page, mention(page, closed_group), closed_group);
  await expect(card(page).locator(".group-message-button")).toHaveCount(0);
  await page.keyboard.press("Escape");
  await expect(card(page)).toHaveCount(0);

  await open(page, mention(page, open_group), open_group);
  const message = card(page).locator(".usercard-controls button.group-message-button");
  await expect(message).toHaveText("Message");
  await message.click();
  await expect(page.locator("#reply-control.open")).toBeVisible();
  await expect(page.locator("#reply-control .users-input summary")).toHaveAttribute("data-name", open_group.name);
  await expect(page.locator("#reply-control .users-input .formatted-selection")).toHaveText(open_group.name);
});

test("a visitor's Join goes to the login page", async ({ page, session }) => {
  const g = group({ public_admission: true }, 1);
  const user1 = await session("user1");
  const t = await topic(user1, label("Group card"), `${label("A paragraph before the mention")}\n\nHello @${g.name}`);
  await page.goto(t.url);
  await open(page, mention(page, g), g);
  await expect(card(page).locator(".group-message-button")).toHaveCount(0);
  await card(page).locator("button.group-index-join").click();
  await expect(page).toHaveURL(/\/login$/);
  expect(sql("SELECT count(*) FROM group_users WHERE group_id = :'p1'", g.id)[0][0]).toBe("1");
});

test("a message's group in the docked header opens its card", async ({ page, context }) => {
  const g = group({ messageable_level: 99, full_name: "Header Team" }, 1);
  const sender = member();
  await login(context, sender.name);
  const raw = Array.from({ length: 40 }, (_, i) => label(`Paragraph ${i} of the group card spec`)).join("\n\n");
  const post = await api(context, "POST", "/posts.json", {
    title: label("Group card message"),
    raw,
    archetype: "private_message",
    target_recipients: g.name,
  });
  await page.goto(`/t/${post.topic_slug}/${post.topic_id}`);
  await page.evaluate(() => window.scrollTo(0, document.body.scrollHeight));
  const entry = page.locator(".d-header .extra-info-wrapper .trigger-group-card a");
  await expect(entry).toHaveText(g.name);
  await open(page, entry, g);
  await expect(card(page).locator(".names__secondary.full-name")).toHaveText("Header Team");
  await expect(page.locator("#page-error")).toBeHidden();
  expect(new URL(page.url()).pathname).toMatch(/^\/t\//);
  await page.keyboard.press("Escape");
  await expect(card(page)).toHaveCount(0);
});
