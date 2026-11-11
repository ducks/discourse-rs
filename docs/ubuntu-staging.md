# Ubuntu private staging

The private demo runs at https://discourse.gnarlyvoid.com on
162.243.135.100 (`ssh discourse-rs-staging`). The host is Ubuntu 24.04,
with 1 vCPU and 2 GiB RAM. Nix supplies the tested runtime dependencies
and PostgreSQL 16 with pgvector; systemd manages the services. Caddy and
image/video tools are installed through Ubuntu packages.

The deployed release is `cb850410`, binary and frontend package alike.
Since the previous release (`1318ea47`) it adds the rich editor
(ProseMirror, with its extensions, plugins and toolbar) as the
composer's default, composer autocomplete for mentions, hashtags and
emoji, the composer messages with the similar topics popup, Esc to
minimize the composer, and the ShareTopic modal. It also fixes member
pages failing with a 500 for a category with a form template. The
binary's SHA-256 is
`48d6b2b755690bc5b1232b1ce90875678927cb9906b7cce2bdd9cd05dc82e97c`
and the package's manifest's is
`7edc78ad704c0183ad74188b212bff460bcb04ceea1aeb4a59150d7b8b6f7117`.
The November 10 local optimized build took 13 minutes, 7 seconds. The
CI gate was not green on `cb850410` at activation: one browser test
(chat-pane-rest's navbar filter) failed in its run. `cb850410` differs
from `2408987c`, whose gate passed, by one browser spec file only.
After activation the loopback health check passed, and a browser check
over HTTPS passed login as `jake`, the topic list, a topic, the share
modal, the composer opening in the rich editor and minimizing on Esc,
chat and admin, with no page errors and no 5xx responses. The previous
`1318ea47` binary and its `frontend-1318ea47` package are retained for
rollback.

## Access

The extra Caddy password prompt was removed at Jake's request. The forum
requires login and is invite-only. HTTPS is automatic;
only SSH and the web proxy listen publicly. The app binds to
127.0.0.1:8080 and PostgreSQL uses a local Unix socket.

The local credential file is
`~/.local/state/discourse-rs/staging-credentials.json` (0600). It contains
the dedicated `jake` administrator's credentials. The server copy is
`/root/discourse-rs-staging-credentials.json`. Keep both out of Git.
A regular `guest` account is active at trust level 1, has no admin or
moderator privileges, and follows the three seeded chat channels. Its
credentials are in `~/.local/state/discourse-rs/guest-credentials.json`
(0600); the server copy is `/root/discourse-rs-guest-credentials.json`.
The seeded test users also remain available.

The Name.com A record for `discourse.gnarlyvoid.com` is 265960772, TTL 300.
Its previous value was 159.223.206.131. Other DNS records, `yapper.forum`,
and the old droplet were left unchanged.

## Runtime

```bash
ssh discourse-rs-staging
systemctl status discourse-rs discourse-rs-postgresql caddy
journalctl -u discourse-rs -f
systemctl restart discourse-rs
```

All three services are enabled at boot. The binary is
`/opt/discourse-rs/discourse-rs-cb850410`, selected by
`/usr/local/bin/discourse-rs`; its frontend package is the one
`/opt/discourse-rs/frontend-current` points at (`DISCOURSE_FRONTEND_DIR`
in the env file). Nix GC roots under `/opt/discourse-rs/`
retain its libraries, PostgreSQL, and the pinned stock images/fonts/emoji.

Configuration lives in `/etc/discourse-rs/env` (0640, root:discourse-rs),
`/etc/caddy/Caddyfile`, and the two `discourse-rs*.service` units under
`/etc/systemd/system/`. State lives under `/var/lib/discourse-rs/` and
`/var/lib/postgresql/16/staging/`. PostgreSQL durability remains enabled,
with 128 MiB shared buffers, 4 MiB work memory, and 40 connections. The
application pool is five connections.

The imported `force_https` site setting must be true as well as the
environment setting: the snapshot had an explicit false override.
Staging request limits are 1,000 per IP per 10 seconds and 3,000 per minute,
with the separate asset burst limit set to 2,000 per 10 seconds.
The defaults rejected ordinary page loads because static asset and avatar
requests count toward the limit. Asset handling deserves a separate
production follow-up; staging retains forum login requirements and rate limiting.

## Data

The initial forum came from `bench/20261019-clean-corpus/forum.sql`:
1,089 topics and 20,576 posts. This contains sampled public Meta writing
expanded into a large corpus with fictional local authors, described in
[synthetic-forum.md](synthetic-forum.md). Its attribution manifest is
retained at `/root/attribution.json`.

Three additional channels, The Workshop, Coffee Break, and Demo Feedback,
contain 90 fictional messages in 18 threads, created through the running
app's API. The seed script and progress manifest are on the server at
`/root/discourse-rs-seed-chat.py` and `/root/discourse-rs-chat-seed.json`.
Browser smoke tests add a topic, a live chat message, and a direct
conversation. Counts increase as the demo is used.

Outgoing email is disabled. This is disposable staging data, with no
off-host backup schedule configured yet. The initial SQL snapshot remains
on the server and the development machine. The app's existing unsupported
job types remain visible in its journal; deployment does not imply full
feature parity.

Provisioning scripts and logs are retained under `/root/discourse-rs-*`.
The first import stopped on extension comments owned by PostgreSQL. The
successful import preserved that ownership, omitted those comments, and
restored application objects in one transaction as `discourse-rs`.

## Verification and follow-ups

The November 2 guest-account browser check passed category filtering and
navigation, returning to Latest, tag filtering and navigation, and clearing
the tag filter after page swaps. No uncaught page errors were reported.
The new binary passed its loopback health check. During activation the old
process remained in `stop-sigterm` long enough for the first browser attempt
to receive an HTTP failure; retrying after activation passed. Investigate
bounded shutdown of long-lived connections before promising brief restarts.

The deployed HTTPS browser check passed login as `jake`, browsing the
large topic list, creating a topic through the composer, joining the
seeded channels, a chat message arriving live in a second user's browser,
a two-way direct conversation, and opening admin. No uncaught page errors
were reported. After removing the extra gate, `/login` returns 200 and anonymous
`/latest.json` returns 403. Certificate verification succeeds.

The logged-out login page's SVG sprite request is incorrectly gated by
the app and redirected to an HTTP login URL, which the browser blocks.
Login itself and the authenticated workflows passed. Follow up on the
SVG route's login exemption and scheme-aware redirects before treating
this as production hosting. The same login gate currently blocks anonymous
invite redemption. Guest provisioning used the account CLI followed by the
normal admin-revocation endpoint, and its regular-user login was verified.
