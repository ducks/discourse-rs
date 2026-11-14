// An account that signed up and is not active yet: the activation email
// sent again and its address changed (login.js: ActivationControls on
// account-created with its resent and edit-email routes, and in the
// NotActivated modal a refused login opens, with ActivationResent and
// ActivationEdit; POST /u/action/send_activation_email, PUT
// /u/update-activation-email). Each test signs up an account of its own.
const { test, expect, sql, label } = require("./helpers");

const PASSWORD = "a-long-enough-pass-phrase";

// A username no other run or test takes.
function username() {
  return label("ae").replace(/[^a-z0-9]/g, "");
}

// The activation tokens the server emails, kept as they are enqueued: the
// worker deletes the job once the email is sent, and the table holds a
// hash.
function captureTokens() {
  sql(
    "SET client_min_messages = warning; \
     CREATE TABLE IF NOT EXISTS public.browser_test_signup_tokens (id bigserial PRIMARY KEY, user_id int, to_address text, token text); \
     CREATE OR REPLACE FUNCTION public.browser_test_keep_signup_token() RETURNS trigger LANGUAGE plpgsql AS $$ \
       BEGIN \
         IF NEW.name = 'critical_user_email' AND NEW.args->>'type' = 'signup' THEN \
           INSERT INTO public.browser_test_signup_tokens (user_id, to_address, token) \
             VALUES ((NEW.args->>'user_id')::int, NEW.args->>'to_address', NEW.args->>'email_token'); \
         END IF; \
         RETURN NEW; \
       END $$; \
     DROP TRIGGER IF EXISTS browser_test_keep_signup_token ON discourse_rs.jobs; \
     CREATE TRIGGER browser_test_keep_signup_token AFTER INSERT ON discourse_rs.jobs \
       FOR EACH ROW EXECUTE FUNCTION public.browser_test_keep_signup_token();",
  );
}

// The signup emails enqueued for the account, oldest first: [address,
// token]. The signup's own names no address (the account's, when sent).
function mailed(id) {
  return sql("SELECT coalesce(to_address, ''), token FROM public.browser_test_signup_tokens WHERE user_id = :'p1' ORDER BY id", id);
}

// The account's email tokens, oldest first: [email, expired]. A signup
// leaves two: User's own, expired by the one its activation email carries.
function tokens(id) {
  return sql("SELECT email, expired FROM email_tokens WHERE user_id = :'p1' ORDER BY id", id);
}

function primaryEmail(id) {
  return sql('SELECT email FROM user_emails WHERE user_id = :\'p1\' AND "primary"', id)[0][0];
}

// Signs up as SignupPageController does (POST /u.json with the honeypot's
// answer) in the page's session and opens account-created, where a signup
// lands. The suite's site signs up by emailed code (the
// enable_local_logins_via_code upcoming change), whose page makes
// accounts that are active from the start, so the form is not the way in.
// Returns { name, email, id }.
async function signUp(page) {
  const name = username();
  const email = `${name}@example.com`;
  const request = page.context().request;
  const headers = { "X-Requested-With": "XMLHttpRequest" };
  const csrf = await (await request.get("/session/csrf.json", { headers })).json();
  const hp = await (await request.get("/session/hp.json", { headers })).json();
  const res = await request.post("/u.json", {
    headers: { ...headers, "X-CSRF-Token": csrf.csrf },
    form: {
      name: "Activation Email",
      email,
      password: PASSWORD,
      username: name,
      password_confirmation: hp.value,
      challenge: hp.challenge.split("").reverse().join(""),
    },
  });
  const created = await res.json();
  expect(created.success, `signing up ${name}: ${created.message}`).toBe(true);
  expect(created.active).toBe(false);
  await page.goto("/u/account-created");
  await expect(page.locator(".account-created .success-info")).toContainText(email);
  const [[id]] = sql("SELECT id FROM users WHERE username = :'p1' AND NOT active", name);
  return { name, email, id };
}

// The modal in sight, its entering animation over.
async function modal(page, selector) {
  const el = page.locator(selector || ".d-modal");
  await expect(el).toBeVisible();
  await el.evaluate((node) => Promise.all(node.getAnimations({ subtree: true }).map((a) => a.finished)));
  return el;
}

// Tries the login form with the account's password, in a browser that
// never signed up. Returns the NotActivated modal.
async function refusedLogin(page, account) {
  await page.goto("/login");
  await page.locator("#login-account-name").fill(account.name);
  await page.locator("#login-account-password").fill(PASSWORD);
  await page.locator("#login-button").click();
  return modal(page, ".not-activated-modal");
}

// PUT /u/activate-account/:token as the activation page sends it. Returns
// the status.
async function activate(context, token) {
  const headers = { "X-Requested-With": "XMLHttpRequest" };
  const csrf = await (await context.request.get("/session/csrf.json", { headers })).json();
  const hp = await (await context.request.get("/session/hp.json", { headers })).json();
  const res = await context.request.put(`/u/activate-account/${token}.json`, {
    headers: { ...headers, "X-CSRF-Token": csrf.csrf },
    form: { password_confirmation: hp.value, challenge: hp.challenge.split("").reverse().join("") },
  });
  return res.status();
}

test.beforeAll(captureTokens);

test("account-created shows the controls, and resending sends the email again", async ({ page }) => {
  const account = await signUp(page);
  const root = page.locator(".account-created");
  await expect(root.locator(".signup-progress-bar__segment.--active")).toHaveCount(1);
  await expect(root.locator(".login-title")).toHaveText(/^Welcome to /);
  const controls = root.locator(".activation-controls");
  await expect(controls.locator(".btn-primary.resend .d-button-label")).toHaveText("Resend Activation Email");
  await expect(controls.locator(".btn-default.edit-email .d-button-label")).toHaveText("Change Email Address");
  expect(tokens(account.id)).toEqual([
    [account.email, "t"],
    [account.email, "f"],
  ]);

  await controls.locator(".resend").click();
  await expect(page).toHaveURL(/\/u\/account-created\/resent$/);
  await expect(root.locator(".ac-message")).toHaveText(
    `We sent another activation email to you at ${account.email}. It might take a few minutes for it to arrive; be sure to check your spam folder.`,
  );
  await expect(root.locator(".ac-message b")).toHaveText(account.email);
  await expect(root.locator(".signup-progress-bar")).toHaveCount(1);
  await expect(root.locator(".activation-controls, .success-info, .login-welcome-header")).toHaveCount(0);

  // Another signup token, the earlier ones expired, mailed to the address.
  expect(tokens(account.id)).toEqual([
    [account.email, "t"],
    [account.email, "t"],
    [account.email, "f"],
  ]);
  const mails = mailed(account.id);
  expect(mails.map((m) => m[0])).toEqual(["", account.email]);

  // Back on the index the controls are there again (the page keeps what
  // it loaded with); the session forgot the account, so another resend is
  // refused, and the page moves on behind the error as Ember's does.
  await page.goBack();
  await expect(page).toHaveURL(/\/u\/account-created$/);
  await expect(root.locator(".success-info")).toContainText(account.email);
  await root.locator(".activation-controls .resend").click();
  await expect(page.locator(".dialog-body")).toHaveText("You are not permitted to view the requested resource.");
  expect(tokens(account.id).length).toBe(3);

  // The route loaded afresh has no account to name.
  await page.goto("/u/account-created/resent");
  await expect(page.locator(".account-created .ac-message")).toHaveText(
    "We sent another activation email. It might take a few minutes for it to arrive; be sure to check your spam folder.",
  );
});

test("changing the address moves the account and its activation", async ({ page, context }) => {
  const account = await signUp(page);
  const root = page.locator(".account-created");
  const moved = `${username()}@example.com`;

  await root.locator(".activation-controls .edit-email").click();
  await expect(page).toHaveURL(/\/u\/account-created\/edit-email$/);
  await expect(root.locator(".ac-message p")).toHaveText("Provide a new address and we'll resend your confirmation email.");
  const input = root.locator("input.activate-new-email");
  const submit = root.locator(".activation-controls .btn-primary");
  await expect(input).toHaveValue(account.email);
  await expect(submit).toHaveText("Update Email Address");
  await expect(submit).toBeDisabled();
  await expect(root.locator(".activation-controls .edit-cancel")).toHaveText("Cancel");
  await expect(root.locator(".success-info, .login-welcome-header")).toHaveCount(0);

  // Cancel goes back to the index, which opens the form again.
  await root.locator(".edit-cancel").click();
  await expect(page).toHaveURL(/\/u\/account-created$/);
  await expect(root.locator(".success-info")).toContainText(account.email);
  await root.locator(".activation-controls .edit-email").click();
  await expect(input).toHaveValue(account.email);

  // Only another address can be sent.
  await input.fill(moved);
  await expect(submit).toBeEnabled();
  await input.fill(account.email);
  await expect(submit).toBeDisabled();
  await input.fill(moved);
  await submit.click();
  await expect(page).toHaveURL(/\/u\/account-created\/resent$/);
  await expect(root.locator(".ac-message b")).toHaveText(moved);

  expect(primaryEmail(account.id)).toBe(moved);
  // The old address's token went with it; the new one's is mailed there.
  expect(tokens(account.id)).toEqual([[moved, "f"]]);
  const mails = mailed(account.id);
  expect(mails.map((m) => m[0])).toEqual(["", moved]);

  // The token first mailed no longer activates the account; the new one does.
  expect(await activate(context, mails[0][1])).toBe(422);
  expect(sql("SELECT active FROM users WHERE id = :'p1'", account.id)).toEqual([["f"]]);
  expect(await activate(context, mails[1][1])).toBe(200);
  expect(sql("SELECT active FROM users WHERE id = :'p1'", account.id)).toEqual([["t"]]);

  // Back on the form: the address is the one the account moved to.
  await page.goBack();
  await expect(page).toHaveURL(/\/u\/account-created\/edit-email$/);
  await expect(root.locator("input.activate-new-email")).toHaveValue(moved);
});

test("an address the server refuses shows its error, and the form stays", async ({ page }) => {
  const account = await signUp(page);
  const root = page.locator(".account-created");
  await root.locator(".activation-controls .edit-email").click();
  const input = root.locator("input.activate-new-email");
  const submit = root.locator(".activation-controls .btn-primary");
  const dialog = page.locator(".dialog-body");

  await input.fill("user1@example.com");
  await submit.click();
  await expect(dialog).toHaveText("Email has already been taken");
  await page.locator(".dialog-footer .btn-primary").click();
  await expect(dialog).toHaveCount(0);
  await expect(page).toHaveURL(/\/u\/account-created\/edit-email$/);

  await input.fill("not an address");
  await submit.click();
  await expect(dialog).toHaveText("Email is invalid.");
  await page.locator(".dialog-footer .btn-primary").click();
  await expect(dialog).toHaveCount(0);
  await expect(input).toHaveValue("not an address");

  expect(primaryEmail(account.id)).toBe(account.email);
  expect(tokens(account.id)).toEqual([
    [account.email, "t"],
    [account.email, "f"],
  ]);
});

test("a refused login offers the controls, and resends from the modal", async ({ page, session }) => {
  const account = await signUp(page);
  const visitor = await (await session(null)).newPage();
  const notActivated = await refusedLogin(visitor, account);
  await expect(notActivated.locator(".d-modal__title-text")).toHaveText("Log In");
  await expect(notActivated.locator(".d-modal__body")).toContainText(
    `We previously sent an activation email to you at ${account.email}.`,
  );
  const controls = notActivated.locator(".d-modal__footer .activation-controls");
  await expect(controls.locator(".btn-primary.resend")).toHaveText("Resend Activation Email");
  await expect(controls.locator(".btn-default.edit-email")).toHaveText("Change Email Address");

  await controls.locator(".resend").click();
  await expect(visitor.locator(".not-activated-modal")).toHaveCount(0);
  const resent = await modal(visitor);
  await expect(resent.locator(".d-modal__title-text")).toHaveText("Log In");
  await expect(resent.locator(".d-modal__body")).toHaveText(
    `We sent another activation email to you at ${account.email}. It might take a few minutes for it to arrive; be sure to check your spam folder.`,
  );
  await expect(resent.locator(".d-modal__footer")).toHaveCount(0);
  expect(tokens(account.id)).toEqual([
    [account.email, "t"],
    [account.email, "t"],
    [account.email, "f"],
  ]);
  expect(mailed(account.id).map((m) => m[0])).toEqual(["", account.email]);

  await resent.locator(".modal-close").click();
  await expect(visitor.locator(".d-modal")).toHaveCount(0);
  await expect(visitor.locator("#current-user")).toHaveCount(0);
});

test("a refused login changes the address from the modal, with the server's errors", async ({ page, session }) => {
  const account = await signUp(page);
  const moved = `${username()}@example.com`;
  const visitor = await (await session(null)).newPage();
  const notActivated = await refusedLogin(visitor, account);
  await notActivated.locator(".activation-controls .edit-email").click();
  await expect(visitor.locator(".not-activated-modal")).toHaveCount(0);

  const edit = await modal(visitor);
  await expect(edit.locator(".d-modal__title-text")).toHaveText("Change Email Address");
  await expect(edit.locator(".d-modal__body p")).toHaveText("Provide a new address and we'll resend your confirmation email.");
  const input = edit.locator("input.activate-new-email");
  const submit = edit.locator(".d-modal__footer .btn-primary");
  await expect(input).toHaveValue(account.email);
  await expect(submit).toHaveText("Update Email Address");
  await expect(submit).toBeDisabled();
  await expect(edit.locator(".d-modal__footer .btn:not(.btn-primary)")).toHaveText("Close");

  // The server's error is the modal's flash.
  await input.fill("user1@example.com");
  await expect(submit).toBeEnabled();
  await submit.click();
  await expect(edit.locator("#modal-alert.alert-error")).toHaveText("Email has already been taken");
  expect(primaryEmail(account.id)).toBe(account.email);

  await input.fill(moved);
  await submit.click();
  const resent = await modal(visitor, ".d-modal:not(:has(.activate-new-email))");
  await expect(resent.locator(".d-modal__body")).toContainText(`We sent another activation email to you at ${moved}.`);
  expect(primaryEmail(account.id)).toBe(moved);
  expect(tokens(account.id)).toEqual([[moved, "f"]]);
  expect(mailed(account.id).map((m) => m[0])).toEqual(["", moved]);
});
