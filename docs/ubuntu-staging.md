# Ubuntu private staging

The private demo runs at https://discourse.gnarlyvoid.com on
162.243.135.100 (`ssh discourse-rs-staging`). The host is Ubuntu 24.04,
with 1 vCPU and 2 GiB RAM. Nix supplies the tested runtime dependencies
and PostgreSQL 16 with pgvector; systemd manages the services. Caddy and
image/video tools are installed through Ubuntu packages.

The deployed release is `1819a61f`, the build that passed the November 1
44-step rehearsal. Its SHA-256 is
`8440b5ebcc9ea74d26da68e8f1778f59e0e6397d9167fd3fe9df8ca09625686d`.
The later admin source reorganization is merged locally but is not part
of this deployed binary.

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
`/opt/discourse-rs/discourse-rs-1819a61f`, selected by
`/usr/local/bin/discourse-rs`. Nix GC roots under `/opt/discourse-rs/`
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
