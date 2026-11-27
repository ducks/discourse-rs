// The setup wizard (static/wizard's WizardStep): /wizard puts the start
// step's URL in place, the title is required, a radio marks its choice,
// the locale ComboBox selects, and Jump in PUTs the step's fields and goes
// home, or shows the fields' errors. The PUT is answered here: its server
// side is the wizard/setup parity case, and saving would change the
// shared site's settings.
const { test, expect, login } = require("./helpers");

test("the setup step validates, serializes its fields and goes home", async ({ page, context }) => {
  await login(context, "admin");
  await page.goto("/wizard");
  await expect(page).toHaveURL(/\/wizard\/steps\/setup$/);
  await expect(page.locator(".d-header")).toHaveCount(0);
  await expect(page.locator(".wizard-container__step.setup")).toBeVisible();

  // Required: an empty title stops Jump in.
  const title = page.locator("#title");
  await title.fill("");
  await page.locator(".jump-in").click();
  await expect(page.locator(".wizard-container__field.text-title")).toHaveClass(/invalid/);

  let sent = null;
  await page.route("**/wizard/steps/setup", async (route) => {
    sent = new URLSearchParams(route.request().postData());
    await route.fulfill({ status: 200, contentType: "application/json", body: '{"success":"OK"}' });
  });

  await title.fill("Wizard Test Site");
  const privateChoice = page.locator('.wizard-container__radio-choice[data-choice-id="private"]');
  await privateChoice.locator("input").check();
  await expect(privateChoice).toHaveClass(/--selected/);
  await expect(page.locator('.wizard-container__radio-choice[data-choice-id="public"]')).not.toHaveClass(/--selected/);

  const dropdown = page.locator("details.wizard-container__dropdown");
  await dropdown.locator("summary").click();
  await expect(dropdown).toHaveClass(/is-expanded/);
  await dropdown.locator('.select-kit-row[data-value="fr"]').click();
  await expect(dropdown.locator("summary")).toHaveAttribute("data-value", "fr");

  await page.locator(".jump-in").click();
  await expect(page).toHaveURL(/\/$/);
  expect(sent.get("fields[title]")).toBe("Wizard Test Site");
  expect(sent.get("fields[login_required]")).toBe("private");
  expect(sent.get("fields[default_locale]")).toBe("fr");
  expect(sent.get("fields[invite_only]")).toBe("sign_up");
  expect(sent.get("fields[must_approve_users]")).toBe("no");
});

test("a save's field errors show under their fields", async ({ page, context }) => {
  await login(context, "admin");
  await page.goto("/wizard/steps/setup");
  await page.route("**/wizard/steps/setup", (route) =>
    route.fulfill({
      status: 422,
      contentType: "application/json",
      body: JSON.stringify({ errors: [{ field: "title", description: "That title is taken." }] }),
    })
  );
  await page.locator("#title").fill("Anything");
  await page.locator(".jump-in").click();
  const field = page.locator(".wizard-container__field.text-title");
  await expect(field.locator(".wizard-container__description.error")).toHaveText("That title is taken.");
  await expect(field).toHaveClass(/invalid/);
  await expect(page).toHaveURL(/\/wizard\/steps\/setup$/);
});
