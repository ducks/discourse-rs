// CreateInviteBulk (user-invited.js, components/modal/create-invite-bulk.gjs):
// an admin's Bulk Invite on their invites tab takes a CSV file, refuses
// another kind, and uploads it; the invites it names are made by the job.
const { test, expect, login, sql, label } = require("./helpers");

const modal = (page) => page.locator(".d-modal.create-invite-bulk-modal");

test("an admin uploads a CSV of invites from the invites tab", async ({ page, context }) => {
  await login(context, "admin");
  await page.goto("/u/admin/invited");
  await page.locator(".bulk-invite-button").first().click();
  await expect(modal(page).locator("#discourse-modal-title")).toHaveText("Bulk Invite");
  await expect(modal(page).locator(".d-modal__body pre")).toHaveText("john@smith.com,first_group_name;second_group_name,topic_id");
  const upload = modal(page).locator(".d-modal__footer .bulk-invite-upload");
  await expect(upload).toHaveText("Bulk Invite");
  await expect(upload).toBeDisabled();

  // csvOnly: another kind of file is refused in a dialog.
  const input = modal(page).locator("#csv-file");
  await input.setInputFiles({ name: "people.txt", mimeType: "text/plain", buffer: Buffer.from("nobody@example.com\n") });
  await expect(page.locator(".dialog-body")).toHaveText("Sorry, file should be CSV format.");
  await page.locator(".dialog-footer .btn-primary").click();
  await expect(upload).toBeDisabled();

  const address = `${label("bulk").replace(/[^a-z0-9]+/gi, "")}@example.com`.toLowerCase();
  await input.setInputFiles({ name: "people.csv", mimeType: "text/csv", buffer: Buffer.from(`${address}\n`) });
  await expect(upload).toBeEnabled();
  await upload.click();
  await expect(modal(page).locator(".d-modal__body")).toHaveText(
    "File uploaded successfully. You will be notified via message when the process is complete.",
  );
  await expect(upload).toHaveCount(0);
  await expect
    .poll(() => sql("SELECT count(*) FROM invites WHERE email = :'p1'", address), { timeout: 20000 })
    .toEqual([["1"]]);
  await modal(page).locator(".d-modal__footer .bulk-invite-close").click();
  await expect(modal(page)).toHaveCount(0);
});
