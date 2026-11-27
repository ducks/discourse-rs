// The admin user's badges tab (static/js/admin-user-badges.js over
// templates/admin-user/badges.gjs): a badge granted twice shows as one
// group row until expanded, Grant adds a grant through the ComboBox, and
// Revoke confirms and takes one away. The badge is the spec's own, a
// manually grantable one that grants many times.
const { test, expect, login, sql, label } = require("./helpers");

test("grants group, expand, grant and revoke", async ({ page, context }) => {
  const name = label("Admin grant badge");
  const [[id]] = sql(
    "INSERT INTO badges (name, badge_type_id, enabled, multiple_grant, system, created_at, updated_at) \
     VALUES (:'p1', 3, true, true, false, now(), now()) RETURNING id",
    name
  );
  sql(
    "INSERT INTO user_badges (badge_id, user_id, granted_at, granted_by_id, seq, created_at) \
     SELECT :p1, u.id, now() - n * interval '1 hour', -1, n, now() \
     FROM generate_series(1, 2) n, users u WHERE u.username = 'user1'",
    id
  );
  await login(context, "admin");
  await page.goto("/admin/users/3/user1/badges");

  const root = page.locator(".admin-container.user-badges");
  const ours = root.locator(`#user-badges tr:visible:has(.user-badge[data-badge-name="${name}"])`);
  await expect(ours).toHaveCount(1);
  await expect(ours.locator(".count")).toHaveText(/\(×2\)/);
  await expect(ours.locator(".expand-badge")).toHaveText("Expand …");

  // Expand: the group's two grants in its place, each revocable.
  await ours.locator(".expand-badge").click();
  await expect(ours).toHaveCount(2);
  await expect(ours.locator(".revoke-badge")).toHaveCount(2);

  // Grant: pick the badge from the filterable ComboBox, give a reason.
  await root.locator("details.grant-badge-combo summary").click();
  await root.locator("details.grant-badge-combo .filter-input").fill(name);
  await root.locator(`details.grant-badge-combo .select-kit-row[data-name="${name}"]`).click();
  await expect(root.locator("details.grant-badge-combo .select-kit-selected-name")).toHaveText(name);
  await root.locator(".badge-reason").fill("");
  await root.locator("form.grant-badge-form button[type=submit]").click();
  await expect(ours.locator(".count")).toHaveText(/\(×3\)/);

  // Revoke one, after the confirmation.
  await ours.locator(".expand-badge").click();
  await ours.first().locator(".revoke-badge").click();
  await page.locator(".dialog-footer .btn-primary").click();
  await expect(ours.locator(".count")).toHaveText(/\(×2\)/);
  const [[left]] = sql("SELECT count(*) FROM user_badges WHERE badge_id = :p1", id);
  expect(Number(left)).toBe(2);
});

test("the tab reached from the user's page opens its ComboBox", async ({ page, context }) => {
  await login(context, "admin");
  await page.goto("/admin/users/3/user1");
  await page.locator('a[href$="/admin/users/3/user1/badges"]').click();
  await expect(page).toHaveURL(/\/admin\/users\/3\/user1\/badges$/);
  const combo = page.locator(".admin-container.user-badges details.grant-badge-combo");
  await combo.locator("summary").click();
  await expect(combo).toHaveClass(/is-expanded/);
  await expect(combo.locator(".select-kit-body")).toBeVisible();
});
