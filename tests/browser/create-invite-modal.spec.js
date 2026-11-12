// CreateInvite (create-invite-modal.js, components/modal/create-invite.gjs):
// the simple mode's link, the form's options, an email invite, where it
// opens from (the sidebar's Invite link, the invites tab, ShareTopic, a
// page load of /new-invite), and an invite edited from its row on the
// invites tab. Each invite is checked in the database.
const { test, expect, login, topic, sql, label, member } = require("./helpers");

const modal = (page) => page.locator(".d-modal.create-invite-modal");

// The member's invites: description, email, domain, redemptions allowed,
// days until it expires (rounded) and its groups.
function invites(m) {
  return sql(
    "SELECT COALESCE(i.description, ''), COALESCE(i.email, ''), COALESCE(i.domain, ''), i.max_redemptions_allowed, \
       round(extract(epoch FROM i.expires_at - now()) / 86400), \
       COALESCE((SELECT string_agg(g.name, ',' ORDER BY g.name) FROM invited_groups ig JOIN groups g ON g.id = ig.group_id WHERE ig.invite_id = i.id), '') \
     FROM invites i WHERE i.invited_by_id = :'p1' AND i.deleted_at IS NULL ORDER BY i.id",
    m.id,
  );
}

async function inviter(page, context) {
  const m = member();
  await login(context, m.name);
  await page.goto("/latest");
  return m;
}

test("the sidebar's Invite link makes a link in the simple mode", async ({ page, context }) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  const m = await inviter(page, context);
  const before = page.url();
  await page.locator('.sidebar-section-link[data-link-name="invite"]').click();
  await expect(modal(page).locator("#discourse-modal-title")).toHaveText("Invite members");
  expect(page.url(), "the modal opens where the member is").toBe(before);
  await expect(modal(page).locator(".d-modal__body p").first()).toHaveText(
    "Create a link that can be shared to instantly grant access to this site.",
  );
  await expect(modal(page).locator(".link-limits-info")).toHaveText(/^Link is valid for up to \d+ users and expires in \d+ days\.$/);
  const create = modal(page).locator(".d-modal__footer .save-invite");
  await expect(create).toHaveText("Create link");
  await expect(create).toBeFocused();
  await create.click();

  const link = modal(page).locator(".link-share-container input.invite-link");
  await expect(link).toHaveValue(/\/invites\/[A-Za-z0-9]+$/);
  await expect(modal(page).locator(".d-modal__footer")).toBeHidden();
  const made = invites(m);
  expect(made).toHaveLength(1);
  expect(made[0].slice(0, 3)).toEqual(["", "", ""]);
  expect(Number(made[0][3])).toBeGreaterThan(1);
  const copy = modal(page).locator(".link-share-container .copy-button");
  await expect(copy).toHaveText("Copy link");
  await expect(copy).toBeFocused();
  await copy.click();
  await expect(copy).toHaveText("Link copied!");
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(await link.inputValue());

  // The options then edit the invite made (PUT), not another.
  await modal(page).locator(".edit-link-options").click();
  // An invite that exists has the moment it expires, not the days until.
  await expect(modal(page).locator('[data-name="expiresAt"] .date-picker')).toHaveValue(/^\d{4}-\d{2}-\d{2}$/);
  await expect(modal(page).locator('[data-name="expiresAfterDays"]')).toHaveCount(0);
  await expect(modal(page).locator(".d-modal__footer .save-invite")).toHaveText("Update");
  await modal(page).locator("#control-description-input").fill("For the neighbours");
  await modal(page).locator(".d-modal__footer .save-invite").click();
  await expect(modal(page).locator("#modal-alert.alert-success")).toContainText("Invite saved");
  await expect.poll(() => invites(m).map((i) => i[0])).toEqual(["For the neighbours"]);
});

test("the form's options: a description, a domain, the redemptions and the expiry", async ({ page, context }) => {
  const m = await inviter(page, context);
  await page.locator('.sidebar-section-link[data-link-name="invite"]').click();
  await modal(page).locator(".edit-link-options").click();
  const form = modal(page).locator("form.form-kit");
  await expect(form.locator("#control-description-input")).toBeFocused();
  await expect(form.locator('[data-name="customMessage"]')).toBeHidden();
  await expect(modal(page).locator(".save-invite-and-send-email")).toHaveCount(0);
  // canArriveAtTopic: a staff member's.
  await expect(form.locator('[data-name="inviteToTopic"]')).toHaveCount(0);

  // The description is at most 100 characters.
  await form.locator("#control-description-input").fill("d".repeat(101));
  await modal(page).locator(".d-modal__footer .save-invite").click();
  await expect(form.locator('[data-name="description"] .form-kit__errors')).toBeVisible();
  expect(invites(m)).toEqual([]);

  await form.locator("#control-description-input").fill("The book club");
  await form.locator("#control-restrictTo-input").fill("example.org");
  await form.locator("#control-maxRedemptions-input").fill("3");
  await form.locator("#control-expiresAfterDays-input").selectOption("7");
  await modal(page).locator(".d-modal__footer .save-invite").click();
  const alert = modal(page).locator("#modal-alert.alert-success");
  await expect(alert).toContainText("Invite saved");
  await expect(alert.locator("input.invite-link")).toHaveValue(/\/invites\/[A-Za-z0-9]+$/);
  expect(invites(m)).toEqual([["The book club", "", "example.org", "3", "7", ""]]);
});

test("an email invite has no redemptions, takes a message and may be sent", async ({ page, context }) => {
  const m = await inviter(page, context);
  await page.locator('.sidebar-section-link[data-link-name="invite"]').click();
  await modal(page).locator(".edit-link-options").click();
  const form = modal(page).locator("form.form-kit");
  const address = `${label("invitee").replace(/\s+/g, "")}@example.com`.toLowerCase();
  await form.locator("#control-restrictTo-input").fill(address);
  await expect(form.locator('[data-name="maxRedemptions"]')).toBeHidden();
  await expect(form.locator('[data-name="customMessage"]')).toBeVisible();
  await form.locator("#control-customMessage-input").fill("Come and read with us");
  const send = modal(page).locator(".save-invite-and-send-email");
  await expect(send).toHaveText("Create link and send email");
  await send.click();
  await expect(modal(page).locator("#modal-alert.alert-success")).toContainText("email");
  const made = invites(m);
  expect(made.map((i) => [i[1], i[3]])).toEqual([[address, "1"]]);
  expect(sql("SELECT custom_message FROM invites WHERE invited_by_id = :'p1'", m.id)).toEqual([["Come and read with us"]]);
});

test("the invites tab's button, ShareTopic's and a page load of /new-invite open it", async ({ page, context, session }) => {
  const m = await inviter(page, context);
  await page.goto(`/u/${m.name}/invited`);
  await page.locator(".invite-button").first().click();
  await modal(page).locator(".d-modal__footer .save-invite").click();
  await expect(modal(page).locator(".link-share-container input.invite-link")).toHaveValue(/\/invites\//);
  await modal(page).locator(".modal-close").click();
  // The list is drawn again with the invite made.
  await expect(page.locator("tr[data-invite-id]")).toHaveCount(1);

  const t = await topic(await session("user1"), label("Invite from share"), label("A topic shared with an invite"));
  await page.goto(t.url);
  await page.locator("#topic-footer-button-share-and-invite").click();
  await page.locator(".share-topic-modal .link-share-actions .invite").click();
  await expect(modal(page).locator("#discourse-modal-title")).toHaveText("Invite members");
  await modal(page).locator(".cancel-button").click();
  await expect(modal(page)).toHaveCount(0);

  await page.goto("/new-invite");
  await expect(modal(page)).toBeVisible();
  expect(new URL(page.url()).pathname + new URL(page.url()).hash).toBe("/");
});

test("an invite's Edit on the invites tab changes it", async ({ page, context }) => {
  const m = await inviter(page, context);
  await page.goto(`/u/${m.name}/invited`);
  await page.locator(".invite-button").first().click();
  await modal(page).locator(".d-modal__footer .save-invite").click();
  const made = await modal(page).locator(".link-share-container input.invite-link").inputValue();
  await modal(page).locator(".modal-close").click();

  await page.locator("tr[data-invite-id] .edit-invite").click();
  await expect(modal(page).locator("#discourse-modal-title")).toHaveText("Edit invite");
  // InviteModalAlert while editing: the link, to copy.
  const alert = modal(page).locator("#modal-alert");
  await expect(alert).toHaveClass(/alert-info/);
  await expect(alert.locator("label")).toHaveText("Copy the link below and share it to instantly grant access to this site.");
  await expect(alert.locator("input.invite-link")).toHaveValue(made);
  const form = modal(page).locator("form.form-kit");
  const update = modal(page).locator(".d-modal__footer .save-invite");
  await expect(update).toHaveText("Update");
  await expect(form.locator("#control-maxRedemptions-input")).toHaveValue(/^\d+$/);
  const date = form.locator('[data-name="expiresAt"] .date-picker');
  const time = form.locator('[data-name="expiresAt"] .time-input');
  await expect(date).toHaveValue(/^\d{4}-\d{2}-\d{2}$/);
  await expect(time).toHaveValue(/^\d{2}:\d{2}$/);

  // Three days from now at 09:30, as the browser's zone has it.
  const when = await page.evaluate(() => {
    const at = new Date(Date.now() + 3 * 86400000);
    const pad = (n) => String(n).padStart(2, "0");
    const day = `${at.getFullYear()}-${pad(at.getMonth() + 1)}-${pad(at.getDate())}`;
    return { day, epoch: new Date(`${day}T09:30:00`).getTime() / 1000 };
  });
  await form.locator("#control-description-input").fill("The allotment");
  await form.locator("#control-maxRedemptions-input").fill("5");
  await date.fill(when.day);
  await date.press("Tab");
  await time.fill("09:30");
  await update.click();
  await expect(modal(page).locator("#modal-alert.alert-success")).toContainText("Invite saved");
  expect(
    sql(
      "SELECT description, max_redemptions_allowed, extract(epoch FROM expires_at)::bigint FROM invites WHERE invited_by_id = :'p1'",
      m.id,
    ),
  ).toEqual([["The allotment", "5", String(when.epoch)]]);

  // No date is no time, and nothing to save.
  await date.fill("");
  await date.press("Tab");
  await expect(time).toBeDisabled();
  await update.click();
  await expect(form.locator('[data-name="expiresAt"] .form-kit__errors')).toBeVisible();

  // An address in place of the link: its buttons say Update too.
  await form.locator("#control-restrictTo-input").fill("someone@example.com");
  await expect(form.locator('[data-name="maxRedemptions"]')).toBeHidden();
  await expect(modal(page).locator(".save-invite-and-send-email")).toHaveText("Update and send email");

  // The list is drawn again with what was saved.
  await modal(page).locator(".cancel-button").click();
  await expect(page.locator("tr[data-invite-id] .invite-description")).toHaveText("The allotment");
});

test("a staff member chooses the topic an invite arrives at, and takes it away", async ({ page, context, session }) => {
  const title = label("Arrive here first");
  const t = await topic(await session("user1"), title, label("The topic an invite arrives at"));
  await login(context, "admin");
  await page.goto("/latest");
  const [[adminId]] = sql("SELECT id FROM users WHERE username = 'admin'");
  const description = label("With a topic");
  const arrives = () =>
    sql(
      "SELECT COALESCE((SELECT string_agg(ti.topic_id::text, ',') FROM topic_invites ti WHERE ti.invite_id = i.id), 'none') \
       FROM invites i WHERE i.invited_by_id = :'p1' AND i.description = :'p2'",
      adminId,
      description,
    );

  await page.locator('.sidebar-section-link[data-link-name="invite"]').click();
  await modal(page).locator(".edit-link-options").click();
  const form = modal(page).locator("form.form-kit");
  await form.locator("#control-description-input").fill(description);
  const chooser = form.locator('[data-name="inviteToTopic"] .topic-chooser');
  await expect(chooser.locator(".select-kit-header .name")).toHaveText("Select…");
  await chooser.locator("summary").click();
  const filter = chooser.locator(".filter-input");
  await expect(filter).toBeFocused();
  await expect(filter).toHaveAttribute("placeholder", "type the topic title, url or id here");
  // By id (searchForId), then by title.
  await filter.fill(String(t.id));
  await expect(chooser.locator(".select-kit-row .topic-title")).toHaveText([title]);
  await filter.fill(title);
  await chooser.locator(`.select-kit-row[data-value="${t.id}"]`).click();
  await expect(chooser.locator(".select-kit-header .name")).toHaveText(title);
  await expect(chooser.locator(".select-kit-body .select-kit-row")).toHaveCount(0);
  // The description was not taken for the filter's typing.
  await expect(form.locator("#control-description-input")).toHaveValue(description);
  await modal(page).locator(".d-modal__footer .save-invite").click();
  await expect(modal(page).locator("#modal-alert.alert-success")).toContainText("Invite saved");
  expect(arrives()).toEqual([[String(t.id)]]);

  // The form drawn again keeps it; cleared, the update takes it away.
  const again = modal(page).locator('[data-name="inviteToTopic"] .topic-chooser');
  await expect(again.locator(".select-kit-header .name")).toHaveText(title);
  await again.locator(".btn-clear").click();
  await expect(again.locator(".select-kit-header .name")).toHaveText("Select…");
  await modal(page).locator(".d-modal__footer .save-invite").click();
  await expect.poll(arrives).toEqual([["none"]]);
});

test("a visitor's page load of /new-invite is shown the login", async ({ page }) => {
  await page.goto("/new-invite");
  await expect(page).toHaveURL(/\/login$/);
});
