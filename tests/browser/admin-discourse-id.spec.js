// The Discourse ID tab (static/js/admin-discourse-id.js over
// templates/admin-config/login/discourse-id.gjs): the switch saves
// enable_discourse_id, which needs the site's credentials there (without
// them EnableDiscourseIdValidator registers the site), and Regenerate is
// disabled until they are.
const { test, expect, login, sql } = require("./helpers");

const CREDENTIALS = ["discourse_id_client_id", "discourse_id_client_secret"];

test.afterEach(() => {
  sql(
    "DELETE FROM site_settings WHERE name IN ('discourse_id_client_id', 'discourse_id_client_secret', 'enable_discourse_id')"
  );
});

test("regenerate waits for the credentials", async ({ page, context }) => {
  await login(context, "admin");
  await page.goto("/admin/config/login-and-authentication/discourse-id");
  const card = page.locator(".discourse-id-admin");
  await expect(card.locator(".discourse-id-stats__item")).toHaveCount(3);
  await expect(card.locator(".discourse-id-regenerate")).toBeDisabled();
});

test("the switch saves enable_discourse_id", async ({ page, context }) => {
  for (const name of CREDENTIALS) {
    sql(
      "INSERT INTO site_settings (name, data_type, value, created_at, updated_at) VALUES (:'p1', 1, 'spec', now(), now())",
      name
    );
  }
  await login(context, "admin");
  await page.goto("/admin/config/login-and-authentication/discourse-id");
  const card = page.locator(".discourse-id-admin");
  await expect(card.locator(".discourse-id-regenerate")).toBeEnabled();
  const toggle = card.locator(".discourse-id-toggle");
  await expect(toggle).toHaveAttribute("aria-checked", "false");

  await card.locator(".d-toggle-switch__checkbox-slider").click();
  await expect(toggle).toHaveAttribute("aria-checked", "true");
  await expect
    .poll(() => sql("SELECT value FROM site_settings WHERE name = 'enable_discourse_id'")[0]?.[0])
    .toBe("t");

  await page.reload();
  await expect(card.locator(".discourse-id-toggle")).toHaveAttribute("aria-checked", "true");
  await card.locator(".d-toggle-switch__checkbox-slider").click();
  await expect
    .poll(() => sql("SELECT value FROM site_settings WHERE name = 'enable_discourse_id'")[0]?.[0])
    .toBe("f");
});
