// A group page's Request and Message buttons (groups.js):
// GroupMembershipButton#showRequestMembershipForm with
// RequestGroupMembershipForm (modal/request-group-membership-form.gjs),
// and GroupController#messageGroup. The request and its message to the
// owners are checked in the database.
const { test, expect, login, sql, member } = require("./helpers");

const modal = (page) => page.locator(".d-modal.request-group-membership-form");

// A group of the test's own that takes membership requests, with a new
// member for its owner. Returns { id, name, owner }.
function group({ template = null, messageable = 0 } = {}) {
  const name = "rq" + Math.floor(Math.random() * 1e12).toString(36);
  const owner = member();
  const [[id]] = sql(
    "INSERT INTO groups (name, visibility_level, members_visibility_level, allow_membership_requests, \
       membership_request_template, messageable_level, user_count, created_at, updated_at) \
     VALUES (:'p1', 0, 0, TRUE, NULLIF(:'p2', ''), :'p3', 1, now(), now()) RETURNING id",
    name,
    template || "",
    messageable,
  );
  sql(
    "INSERT INTO group_users (group_id, user_id, owner, created_at, updated_at) VALUES (:'p1', :'p2', TRUE, now(), now())",
    id,
    owner.id,
  );
  return { id, name, owner };
}

const requests = (g, m) =>
  sql("SELECT reason FROM group_requests WHERE group_id = :'p1' AND user_id = :'p2'", g.id, m.id);

// The request's messages by the member: title, archetype, first post,
// the users allowed, the groups allowed and the requested_group_id field.
const messages = (g, m) =>
  sql(
    "SELECT t.id, t.title, t.archetype, (SELECT raw FROM posts WHERE topic_id = t.id AND post_number = 1), \
       (SELECT string_agg(u.username, ',' ORDER BY u.username) FROM topic_allowed_users a JOIN users u ON u.id = a.user_id WHERE a.topic_id = t.id), \
       (SELECT count(*) FROM topic_allowed_groups WHERE topic_id = t.id), \
       (SELECT value FROM topic_custom_fields WHERE topic_id = t.id AND name = 'requested_group_id') \
     FROM topics t WHERE t.user_id = :'p2' AND t.title = 'Membership Request for @' || :'p1' ORDER BY t.id",
    g.name,
    m.id,
  );

async function openForm(page) {
  await page.locator(".group-details-button .group-index-request").click();
  await expect(modal(page)).toBeVisible();
  await modal(page).evaluate((el) => Promise.all(el.getAnimations({ subtree: true }).map((a) => a.finished)));
}

test("Request opens the form with the group's template and sends the request", async ({ page, context }) => {
  const g = group({ template: "Why I belong: " });
  const m = member();
  await login(context, m.name);
  await page.goto(`/g/${g.name}`);
  await expect(page.locator(".group-details-button .group-index-request")).toHaveText("Request");
  await expect(page.locator(".group-message-button"), "nobody may message the group").toHaveCount(0);
  await openForm(page);

  await expect(modal(page).locator("#discourse-modal-title")).toHaveText(`Request to join @${g.name}`);
  await expect(modal(page).locator(".control-group label")).toHaveText(
    "Let the group owners know why you belong in this group",
  );
  const reason = modal(page).locator(".expanding-text-area textarea.--expandable");
  await expect(reason).toHaveValue("Why I belong: ");
  await expect(reason).toHaveAttribute("maxlength", "5000");
  const submit = modal(page).locator(".d-modal__footer .btn-primary");
  await expect(submit).toHaveText("Submit Request");
  await expect(submit).toBeEnabled();
  await expect(modal(page).locator(".d-modal__footer .d-modal-cancel")).toHaveText("Cancel");

  // disableSubmit: an empty reason.
  await reason.fill("");
  await expect(submit).toBeDisabled();
  await reason.fill("Why I belong: I test things");
  await expect(submit).toBeEnabled();

  const [response] = await Promise.all([
    page.waitForResponse((r) => r.url().includes(`/groups/${g.name}/request_membership.json`)),
    submit.click(),
  ]);
  expect(response.request().method()).toBe("POST");
  expect(response.request().postData()).toBe("reason=" + encodeURIComponent("Why I belong: I test things"));
  expect(response.status()).toBe(200);

  // DiscourseURL.routeTo(result.relative_url): the message's topic.
  await expect(page).toHaveURL(/\/t\/membership-request-for-[^/]+\/\d+$/);
  await expect(modal(page)).toHaveCount(0);
  expect(requests(g, m)).toEqual([["Why I belong: I test things"]]);
  const sent = messages(g, m);
  expect(sent).toHaveLength(1);
  expect(sent[0].slice(1)).toEqual([
    `Membership Request for @${g.name}`,
    "private_message",
    "Why I belong: I test things",
    [m.name, g.owner.name].sort().join(","),
    "0",
    String(g.id),
  ]);
  expect(page.url()).toContain(`/${sent[0][0]}`);
});

test("a group without a template starts empty, and a second request is refused", async ({ page, context }) => {
  const g = group();
  const m = member();
  await login(context, m.name);
  await page.goto(`/g/${g.name}`);
  await openForm(page);
  const reason = modal(page).locator("textarea");
  const submit = modal(page).locator(".d-modal__footer .btn-primary");
  await expect(reason).toHaveValue("");
  await expect(submit).toBeDisabled();

  // Cancel closes it with nothing sent.
  await modal(page).locator(".d-modal-cancel").click();
  await expect(modal(page)).toHaveCount(0);
  expect(requests(g, m)).toEqual([]);

  // A request that is already there (GroupMembershipRequester's
  // AlreadyRequested, a 409): popupAjaxError says so and the form stays.
  sql(
    "INSERT INTO group_requests (group_id, user_id, reason, created_at, updated_at) VALUES (:'p1', :'p2', 'earlier', now(), now())",
    g.id,
    m.id,
  );
  await openForm(page);
  await modal(page).locator("textarea").fill("again");
  const [response] = await Promise.all([
    page.waitForResponse((r) => r.url().includes("/request_membership.json")),
    modal(page).locator(".d-modal__footer .btn-primary").click(),
  ]);
  expect(response.status()).toBe(409);
  await expect(page.locator("#dialog-holder .dialog-body")).toHaveText(
    "You have already requested membership for this group.",
  );
  await expect(modal(page)).toBeVisible();
  await expect(modal(page).locator(".d-modal__footer .btn-primary")).toBeEnabled();
  expect(page.url()).toContain(`/g/${g.name}`);
  expect(requests(g, m)).toEqual([["earlier"]]);
  expect(messages(g, m)).toEqual([]);
});

test("Discourse.groupMembership.request opens the form for a group", async ({ page, context }) => {
  const g = group({ template: "From the card" });
  const m = member();
  await login(context, m.name);
  await page.goto(`/g/${g.name}`);
  await page.evaluate(
    (name) => window.Discourse.groupMembership.request({ name, membership_request_template: "From the card" }),
    g.name,
  );
  await expect(modal(page).locator("#discourse-modal-title")).toHaveText(`Request to join @${g.name}`);
  await expect(modal(page).locator("textarea")).toHaveValue("From the card");
});

test("Message opens the composer for a message to the group", async ({ page, context }) => {
  const g = group({ messageable: 99 });
  const m = member();
  await login(context, m.name);
  await page.goto(`/g/${g.name}`);
  const message = page.locator(".group-details-button .group-message-button");
  await expect(message).toHaveText("Message");
  await message.click();
  await expect(page.locator("#reply-control.open")).toBeVisible();
  await expect(page.locator("#private-message-users summary")).toHaveAttribute("data-value", g.name);
  expect(page.url()).toContain(`/g/${g.name}`);
});

test("a visitor's Request goes to the login", async ({ page }) => {
  const g = group({ template: "Why I belong" });
  await page.goto(`/g/${g.name}`);
  const request = page.locator(".group-details-button .group-index-request");
  await expect(request).toHaveText("Request");
  expect(await request.getAttribute("data-membership-request-template"), "the template is a member's").toBeNull();
  await expect(page.locator("#group-membership-strings")).toHaveCount(0);
  await expect(page.locator(".group-message-button")).toHaveCount(0);
  await request.click();
  await expect(page).toHaveURL(/\/login$/);
  await expect(modal(page)).toHaveCount(0);
});
