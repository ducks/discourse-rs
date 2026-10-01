# Roadmap

Where discourse-rs is going and in what order. Each slice is a branch
merged `--no-ff` to main, measured against Rails with the parity harness
(see README: Parity). Update this file in the same commit as the slice.

The port refuses rather than guesses: a request that needs a behavior not
ported yet gets an explicit `Unsupported(...)` 500 naming it. The list of
those markers (`grep -rho 'Unsupported("[^"]*")' src/`) is the fine-grained
backlog; the milestones below are the coarse one.

## Milestone 1: a hostable read-only mirror

Serve an anonymous, read-only copy of a Discourse backup that someone
could put behind Caddy and leave running.

- [x] Restore a backup archive (`scripts/restore-backup`), serve uploads
      and stock images
- [x] Site settings, I18n, URLs, color schemes, `/site.json`,
      `/site/basic-info`
- [x] Topic lists: latest, top, hot, `page`/`per_page`/`order`/`ascending`
- [x] Category lists (`/c/...`, `/none`, `/l/top`, `/l/hot`), categories
      index
- [x] Topics with posts, post-number pages, redirects, 404s
- [x] Tag lists (`/tag/...`, `/tags/c/...`), tags index, tag visibility
- [x] Server-rendered HTML for all of the above (askama)
- [x] `login_required` gate: a private forum's backup serves nothing
      anonymously (JSON 403, HTML 302 to `/login`)
- [x] Search: `/search?q=` and `/search/query` over `post_search_data`
      (plain terms and phrases; advanced filters are milestone 2)
- [x] User pages: `/u/:username`, summary, activity (posts link there)
- [x] Crawler hygiene: `robots.txt`, sitemaps, canonical and meta tags
- [ ] Deploy: systemd unit, Caddy snippet, `make install`, first
      date-versioned release with a binary (parked 2026-09-30); the unit
      needs DISCOURSE_SECRET_KEY_BASE (Rails keeps it in redis when not
      configured: `rails runner 'puts GlobalSetting.safe_secret_key_base'`)
- [x] Bench harness: `make bench` and `make bench-startup` (BENCH.md has
      the runs and the caveats); a production-mode Discourse reference is
      still needed for numbers that mean something

## Milestone 2: parity depth

Behaviors real backups hit that the mirror currently refuses. Roughly by
how often a backup trips them.

- [ ] Fancy titles (`HtmlPrettify`) for titles without a stored
      `fancy_title`
- [ ] Hidden and deleted posts (`hidden`, `deleted_at`, staff whispers stay
      hidden)
- [ ] Topic thumbnails and topic images behind a CDN, secure uploads
- [ ] Watched words (`WordWatcher` regexps and actions)
- [ ] Topic timers, topic links with clicks (`TopicLink.topic_map`)
- [ ] Featured links (`featured_link_root_domain`)
- [ ] `subcategory_list` styles, paginated category lists
- [x] Performance: batch the list serializer's per-topic queries (99 to
      11 per /latest.json, 137 to 332 req/s at one connection)
- [ ] Performance: the topic view's per-post queries; bench above two
      connections needs a machine that holds its clocks (BENCH.md)
- [ ] Group flair uploads, user fields, badges granted, auth providers,
      user color schemes on `/site.json`
- [ ] Muted defaults (`default_categories_muted`, `default_tags_muted`,
      `mute_all_categories_by_default`), shared drafts category
- [ ] `tags_listed_by_group`
- [ ] Profiles: bios (PrettyText.excerpt), suspended/silenced users, user
      status, featured topics, letter avatars (`/letter_avatar_proxy` is a
      proxy to avatars.discourse.org; serve or generate them)
- [ ] Search: advanced filters (`in:`, `status:`, `category:`, `#`, `@`,
      `tags:`, `before:`/`after:`, `order:`), search contexts, `search_for_id`,
      rate limits, pg headlines
- [ ] Plugin-added keys (solved, voting, reactions, ...) through the plugin
      protocol (PLUGINS.md) instead of parity ignore lists
- [ ] Topics without a stored slug (`Slug.for`)

## Milestone 3: sessions

Logged-in readers, no writes yet.

- [x] Login: local (PBKDF2 `user_passwords`), CSRF, the `_t` and
      `_forum_session` cookies in Rails' own encrypted format so a cutover
      keeps sessions; `/session/current.json` byte-equal for members and
      admins
- [ ] OIDC / external logins, 2FA
- [x] Logged-in `Guardian`: secure categories, group permissions, tag
      group permissions by group, staff visibility (unlisted topics, all
      tags, every group), can_* on topics and posts, flags (`post_can_act?`)
- [x] Read state: `topic_users` on lists, topics and search, muted
      topics/categories/tags, cleared pins, dismissals, `/unread`, `/new`,
      `/unseen`, `/read`, `/posted`, `/bookmarks` (and `/c/.../l/<filter>`)
- [x] The page shell for a member: csrf meta, logout form, no caching
- [ ] Own profile and staff views of profiles (the private attribute
      block: emails, 2FA, auth tokens, preferences), deleted topics and
      posts for staff (`post_stream.gaps`, `show_deleted`), ignored users,
      category group moderators, shared drafts, the login-required 2FA and
      required-fields redirects
- [ ] Notifications list, bookmarks list, user preferences (read-only)
- [ ] Live updates: a MessageBus-compatible long-poll endpoint (`/message-bus/:client_id/poll`); nothing in the Rust ecosystem provides it, so it is ours to write


## Milestone 3.5: plugins

The protocol in PLUGINS.md, built on the read side first so the parity
harness can judge it.

- [ ] Tier 0: `plugin.toml` with settings, preloaded custom fields,
      assets, i18n, SQL migrations; `discourse-rs plugins`
- [ ] Tier 1: subprocess over JSON-RPC (stdio or socket), supervised;
      `serialize`, `html`, `modify` batched per response, `route` with
      core's session and CSRF, `event` and `job`; the failure policy
- [ ] discourse-solved's read side as the first plugin (Ruby), its parity
      ignores deleted; the voting plugin second, in another language
- [ ] Write-side hooks designed with milestone 4 (NewPostManager
      modifiers, post events)
- [ ] Tier 2: the same hooks as WASI components in-process

## Milestone 4: writes

- [ ] Cooking pipeline: markdown-it with Discourse's rules (the hard part;
      likely embed the JS pretty-text bundle first, port later)
- [ ] Posting, editing, revisions, likes, flags
- [ ] Private messages
- [ ] Uploads (local first, S3 later), optimized images
- [ ] Background jobs (a Sidekiq replacement for cooking, notifications,
      digests), rate limits
- [ ] Ember frontend against the Rust API as a compatibility check, not a
      product goal

## Not planned

- Running Discourse migrations: Discourse owns the schema; re-vendor
  `structure.sql` and re-snapshot instead
- Plugins as code loaded into the process (see PLUGINS.md: they are
  programs behind a JSON-RPC protocol, in any language)
- Feature parity with admin (`/admin/...`)
