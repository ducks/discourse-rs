// Demo step 5: a member logs in from a topic's header and comes back to it
// (login.js: the modal, POST /session.json, the redirect back).
const { test, expect, PASSWORD } = require("./helpers");

test("logging in from a topic comes back to the topic", async ({ page }) => {
  await page.goto("/t/parity-fixture-replies-and-posters/35");
  await expect(page.locator(".topic-post").first()).toBeVisible();
  await expect(page.locator("#current-user")).toHaveCount(0);

  await page.locator(".d-header .login-button").first().click();
  const name = page.locator("#login-account-name");
  await expect(name).toBeVisible();
  await expect(name).toBeFocused();
  await name.fill("user1");
  await page.locator("#login-account-password").fill(PASSWORD);
  await page.locator("#login-button").click();

  await expect(page.locator("#current-user")).toBeVisible();
  await expect(page).toHaveURL(/\/t\/parity-fixture-replies-and-posters\/35/);
  await expect(page.locator(".topic-post").first()).toBeVisible();
});

test("a wrong password is refused with the flash, and the form stays", async ({ page }) => {
  await page.goto("/latest");
  await page.locator(".d-header .login-button").first().click();
  await expect(page.locator("#login-account-name")).toBeFocused();
  await page.locator("#login-account-name").fill("user1");
  await page.locator("#login-account-password").fill("not the password");
  await page.locator("#login-button").click();
  // login.js's setFlash: DFlashMessage above the form.
  await expect(page.locator(".alert.alert-error")).toHaveText(/Incorrect username, email or password/);
  await expect(page.locator("#login-account-name")).toHaveValue("user1");
  await expect(page.locator("#current-user")).toHaveCount(0);
});

test("the login form the header swaps in is live before the page's own script loads", async ({ page }) => {
  await page.goto("/latest");
  // The login route's copy of login.js never arrives: the head's has the
  // form, as Ember's bundle has LoginPage before the route renders.
  await page.route(/\/assets\/login\.js/, (route) => route.abort());
  await page.locator(".d-header .login-button").first().click();
  await expect(page.locator("#login-account-name")).toBeFocused();
  await page.locator("#login-account-name").fill("user1");
  await page.locator("#login-account-password").fill("not the password");
  await page.locator("#login-button").click();
  await expect(page.locator(".alert.alert-error")).toHaveText(/Incorrect username, email or password/);
  await expect(page).toHaveURL(/\/login$/);
});
