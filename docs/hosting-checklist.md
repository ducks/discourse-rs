# Hosting checklist

What to settle when the team forum moves onto a host. The setup itself is
README's "Deploying on NixOS"; this is the list to walk before and after.

## The host

- One VPS, NixOS, x86_64 (the flake builds aarch64 too, untested). 2 GB
  of memory is enough for a team: the server, PostgreSQL 16 and Caddy.
- The host builds discourse-rs from source on `nixos-rebuild` (there is
  no binary cache for it): a release build with LTO takes a while and a
  few GB of disk. Building on a bigger machine and deploying the closure
  (`nixos-rebuild --target-host`) avoids that on a small VPS.
- The en_US.UTF-8 locale in `i18n.supportedLocales` (there by default):
  the local database is created with it, and the module refuses a
  configuration without it.

## DNS, Caddy and TLS

- An A (and AAAA) record for the forum's name pointing at the host before
  the first switch, so Caddy's ACME challenge passes.
- `services.discourse-rs.hostname` is that name and `caddy.enable = true`;
  ports 80 and 443 open (`networking.firewall.allowedTCPPorts`). Caddy
  fetches and renews the certificate; set `services.caddy.email` for
  Let's Encrypt's expiry notices.
- The server binds to 127.0.0.1:8080 and trusts `X-Forwarded-For`: never
  expose that port.
- `forceHttps` stays on (Secure cookies, https links).

## The secret key base

- `DISCOURSE_SECRET_KEY_BASE` in `/etc/discourse-rs/env` (root, 0600),
  `openssl rand -hex 64` for a new site, the Rails site's value when
  moving a forum over. Without it the server generates one and keeps it
  in the database (`discourse_rs.persistent_values`), as Rails keeps it
  in Redis.
- Keep a copy with the backups: losing it logs everyone out and voids
  outstanding email tokens.

## SMTP relay

- A relay that sends for the forum's domain (Postmark, SES, Mailgun, the
  host's own), with SPF, DKIM and DMARC records for that domain.
- In the env file: `DISCOURSE_SMTP_ADDRESS`, `DISCOURSE_SMTP_PORT` (587
  with STARTTLS, or 465 with `DISCOURSE_SMTP_FORCE_TLS=true`),
  `DISCOURSE_SMTP_USER_NAME`, `DISCOURSE_SMTP_PASSWORD`, and
  `DISCOURSE_SMTP_DOMAIN` if the relay wants a HELO name. The server
  logs the mailer it built at start (`outgoing mail`).
- The `notification_email` site setting is the From address; it must be
  one the relay may send as.
- Test it: invites, password resets and the finish installation page's
  activation email need it.

## Team-only lockdown

- The first admin: `initialAdmin.email` and `initialAdmin.passwordFile`
  (root, 0600, at least 15 characters) before the first switch.
  discourse-rs-db-init makes the account (`discourse-rs admin create`)
  right after `discourse-rs setup` loads the fresh site, which has no
  other users. Or leave them unset and register on the finish
  installation page with an address in `DISCOURSE_DEVELOPER_EMAILS`
  (needs SMTP). The password file is only read while there is no admin;
  it can go once the admin has logged in.
- The seeded welcome, guidelines and admin guide topics are Discourse's
  own; edit or delete them.
- `loginRequired = true` and `inviteOnly = true` in the module, then
  invite the team.
- Check from a logged-out browser that `/latest` asks for a login.

## The job worker

- On by default (`jobs.enable`), in the server process. Without it posts
  are never processed and nobody is notified or emailed.
- Failed and refused jobs stay in the queue:

  ```bash
  sudo -u postgres psql discourse -c \
    "SELECT name, attempts, last_error FROM discourse_rs.jobs WHERE last_error IS NOT NULL"
  ```

## Logs and "not ported yet"

- `journalctl -u discourse-rs -f`. A request or job the port doesn't
  handle yet fails with an error that starts with `not ported yet:`;
  watch for them in the first weeks and note which features the team
  hits:

  ```bash
  journalctl -u discourse-rs --since today | grep 'not ported yet'
  ```

- `RUST_LOG` (through `extraEnvironment`) raises the level, e.g.
  `info,discourse_rs=debug`.

## Backups

- The database: `services.postgresqlBackup` with
  `databases = [ "discourse" ]` dumps it nightly to
  `/var/backup/postgresql`.
- The uploads: `/var/lib/discourse-rs/public`.
- Both, plus the env file, off the host (restic or borg to object
  storage, `services.restic.backups`). Restore one onto a scratch VM
  once, so the procedure is known before it is needed.
- A database restored as `postgres` belongs to `postgres`: hand it to the
  service with `scripts/db-chown discourse discourse-rs` before starting
  it (README, "Deploying").

## Upgrading

- Bump the flake input (`nix flake update discourse-rs` in the host's
  flake, or pin a release tag in its url) and `nixos-rebuild switch`.
  The unit restarts on the new binary; SIGTERM lets requests finish.
- A release embeds the schema vendored at its tag and refuses a database
  behind it. `discourse-rs-db-init` runs `discourse-rs setup` once, on an
  empty database, and never migrates it, and the port runs no Discourse
  migrations: before an upgrade, compare `vendor/discourse/DISCOURSE_REF`
  between the two revisions. The same commit upgrades freely; a newer
  one has no upgrade path for the database yet.
- Roll back with `nixos-rebuild switch --rollback` (the database is not
  rolled back).
