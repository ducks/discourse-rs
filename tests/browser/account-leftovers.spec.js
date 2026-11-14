// The account flows' last pieces: the ForgotPassword modal a password
// reset code opens from the security preferences (the change password
// button and ConfirmSession's forgot password link: prefs-security.js
// over login.js's modal), and AvatarSelectorModal (avatar-selector.js)
// in the account preferences and, deferred, for a new account on the
// invite page (invite.js). Each test works on a member, an invite and an
// address of its own.
const { test, expect, login, sql, member, captureCodes, codeFor } = require("./helpers");

function resetCodes(address) {
  return sql(
    "SELECT COUNT(*) FROM email_login_codes WHERE email = :'p1' AND purpose = 1 AND consumed_at IS NULL",
    address,
  )[0][0];
}

function resetTokens(id) {
  return sql("SELECT COUNT(*) FROM email_tokens WHERE user_id = :'p1' AND scope = 2 AND NOT expired", id)[0][0];
}

async function opened(modal) {
  await expect(modal).toBeVisible();
  await expect(modal).not.toHaveClass(/is-animating/);
}

test.beforeAll(captureCodes);

test("the change password button sends a reset code and opens ForgotPassword at its code step", async ({ page, context }) => {
  const m = member();
  const email = `${m.name}@example.com`;
  await login(context, m.name);
  await page.goto(`/u/${m.name}/preferences/security`);

  const asked = page.waitForRequest((r) => r.url().endsWith("/session/forgot_password.json") && r.method() === "POST");
  await page.locator("#change-password-button").click();
  expect(new URLSearchParams((await asked).postData()).get("login")).toBe(email);

  const modal = page.locator(".d-modal.forgot-password-modal");
  await opened(modal);
  await expect(modal.locator(".d-modal__title-text")).toHaveText("Password Reset");
  await expect(modal.locator(".code-login-form__code-step .code-login-form__title")).toHaveText("Check your email");
  await expect(modal.locator(".code-login-form__hidden-email")).toHaveValue(email);
  await expect(modal.locator(".d-modal__footer .btn")).toHaveCount(0);
  await expect(page.locator(".password-progress")).toHaveText("");
  expect(resetCodes(email)).toBe("1");
  expect(resetTokens(m.id)).toBe("0");

  // A wrong code is refused in the form; the emailed one is traded for
  // the password reset page.
  const wrong = codeFor(email) === "000000" ? "111111" : "000000";
  await modal.locator(".d-otp-input").fill(wrong);
  await expect(modal.locator(".code-login-form__error")).not.toHaveText("");
  await expect(modal.locator(".d-otp-input")).toHaveValue("");
  await modal.locator(".d-otp-input").fill(codeFor(email));
  await expect(page).toHaveURL(/\/u\/password-reset\/[0-9a-f]+$/);
  expect(resetCodes(email)).toBe("0");
  expect(resetTokens(m.id)).toBe("1");
});

test("ForgotPassword's change email goes back to its form, with the address", async ({ page, context }) => {
  const m = member();
  await login(context, m.name);
  await page.goto(`/u/${m.name}/preferences/security`);
  await page.locator("#change-password-button").click();
  const modal = page.locator(".d-modal.forgot-password-modal");
  await opened(modal);
  await modal.locator(".code-login-form__change-email").click();
  await expect(modal.locator("#username-or-email")).toHaveValue(`${m.name}@example.com`);
  await expect(modal.locator(".forgot-password-reset")).toBeEnabled();
});

// The link asks for the viewer's username (currentUser has no email),
// which hide_email_address_taken (on by default) takes only from staff.
test("ConfirmSession's forgot password link closes the dialog for the modal", async ({ page, context }) => {
  const m = member();
  const email = `${m.name}@example.com`;
  sql("UPDATE users SET moderator = TRUE WHERE id = :'p1'", m.id);
  await login(context, m.name);
  await page.goto(`/u/${m.name}/preferences/security`);
  // Managing second factors asks an unconfirmed session to confirm.
  await page.locator(".pref-second-factor .btn-second-factor").click();
  const dialog = page.locator(".dialog-container .confirm-session");
  await expect(dialog).toBeVisible();

  const asked = page.waitForRequest((r) => r.url().endsWith("/session/forgot_password.json") && r.method() === "POST");
  await dialog.locator(".confirm-session__reset-btn").click();
  expect(new URLSearchParams((await asked).postData()).get("login")).toBe(m.name);

  const modal = page.locator(".d-modal.forgot-password-modal");
  await opened(modal);
  await expect(dialog).toHaveCount(0);
  await expect(modal.locator(".code-login-form__code-step")).toBeVisible();
  await expect(modal.locator(".code-login-form__hidden-email")).toHaveValue(m.name);
  expect(resetCodes(email)).toBe("1");
});

test("ConfirmSession's forgot password link shows a member the refusal of their username", async ({ page, context }) => {
  const m = member();
  await login(context, m.name);
  await page.goto(`/u/${m.name}/preferences/security`);
  await page.locator(".pref-second-factor .btn-second-factor").click();
  const dialog = page.locator(".dialog-container");
  await dialog.locator(".confirm-session__reset-btn").click();
  await expect(dialog.locator(".alert-error")).toHaveText("You supplied invalid parameters to the request: login");
  await expect(page.locator(".d-modal.forgot-password-modal")).toHaveCount(0);
  expect(resetCodes(`${m.name}@example.com`)).toBe("0");
});

test("the account preferences' avatar selector saves the system avatar", async ({ page, context }) => {
  const m = member();
  await login(context, m.name);
  await page.goto(`/u/${m.name}/preferences/account`);
  await page.locator("#edit-avatar").click();
  const modal = page.locator(".d-modal.avatar-selector-modal");
  await opened(modal);
  await expect(modal.locator("#system-avatar")).toBeChecked();
  await expect(modal.locator(".avatar-uploader__button")).toBeVisible();
  await expect(modal.locator("#deferred-avatar-upload")).toHaveCount(0);

  const picked = page.waitForRequest((r) => r.url().endsWith("/preferences/avatar/pick") && r.method() === "PUT");
  await modal.locator(".d-modal__footer .btn-primary").click();
  const sent = new URLSearchParams((await picked).postData());
  expect(sent.get("type")).toBe("system");
  expect(sent.has("upload_id")).toBe(true);
});

test("a new account from an invite picks its avatar before it exists, saved once it does", async ({ page }) => {
  const key = "bt" + Math.floor(Math.random() * 1e12).toString(36);
  const name = "iv" + Math.floor(Math.random() * 1e9).toString(36);
  const email = `${name}@example.com`;
  sql(
    "INSERT INTO invites (invite_key, email, invited_by_id, created_at, updated_at, emailed_status, \
       max_redemptions_allowed, redemption_count, expires_at) \
     SELECT :'p1', NULL, id, now(), now(), 0, 5, 0, now() + interval '30 days' FROM users WHERE username = 'user1'",
    key,
  );
  await page.goto(`/invites/${key}`);
  await page.locator("#code-login-email").fill(email);
  await page.locator(".code-login-form__continue").click();
  await expect(page.locator(".code-login-form__code-step")).toBeVisible();
  await page.locator(".d-otp-input").fill(codeFor(email));

  const avatar = page.locator(".code-login-form__avatar");
  await expect(avatar.locator(".code-login-form__avatar-placeholder")).toBeVisible();
  await avatar.click();
  const modal = page.locator(".d-modal.avatar-selector-modal");
  await opened(modal);
  await expect(modal.locator(".d-modal__title-text")).toHaveText("Change your profile picture");
  await expect(modal.locator("#system-avatar")).toBeChecked();
  await expect(modal.locator(".avatar-choice--system .avatar-selector__placeholder")).toBeVisible();
  await expect(modal.locator(".avatar-choice--gravatar")).toHaveCount(0);
  const save = modal.locator(".d-modal__footer .btn-primary");
  // The upload's choice without a file can't be saved.
  await modal.locator("#uploaded-avatar").check();
  await expect(save).toBeDisabled();

  // A 1x1 png, its bytes this run's.
  const png = Buffer.concat([
    Buffer.from(
      "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
      "base64",
    ),
    Buffer.from(name),
  ]);
  await modal.locator("#deferred-avatar-upload").setInputFiles({ name: "me.png", mimeType: "image/png", buffer: png });
  await expect(modal.locator("#uploaded-avatar")).toBeChecked();
  await expect(modal.locator('label[for="uploaded-avatar"] img.avatar')).toHaveAttribute("src", /^blob:/);
  await expect(save).toBeEnabled();
  await save.click();
  await expect(modal).toHaveCount(0);
  await expect(avatar.locator("img.avatar")).toHaveAttribute("src", /^blob:/);
  // Nothing is saved before the account exists.
  expect(sql("SELECT COUNT(*) FROM users WHERE username = :'p1'", name)[0][0]).toBe("0");

  // The selector opens again on the choice made.
  await avatar.click();
  await opened(modal);
  await expect(modal.locator("#uploaded-avatar")).toBeChecked();
  await modal.locator(".d-modal-cancel").click();
  await expect(modal).toHaveCount(0);

  await page.locator("#code-login-username").fill(name);
  const picked = page.waitForRequest((r) => r.url().endsWith("/preferences/avatar/pick") && r.method() === "PUT");
  await page.locator(".code-login-form__continue-to-site").click();
  expect(new URLSearchParams((await picked).postData()).get("type")).toBe("custom");
  await expect(page.locator("#current-user")).toBeVisible();

  const [[uploaded, custom, origin]] = sql(
    "SELECT u.uploaded_avatar_id, a.custom_upload_id, (SELECT original_filename FROM uploads WHERE id = u.uploaded_avatar_id) \
     FROM users u JOIN user_avatars a ON a.user_id = u.id WHERE u.username = :'p1'",
    name,
  );
  expect(uploaded).not.toBe("");
  expect(custom).toBe(uploaded);
  expect(origin).toBe("me.png");
});
