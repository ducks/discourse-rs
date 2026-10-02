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
- [x] Deploy: systemd unit, Caddy snippet and `make install` (`deploy/`),
      a static binary attached to each date-versioned release; the env
      file needs DISCOURSE_SECRET_KEY_BASE (README: Deploying)
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
      protocol (milestone 5) instead of parity ignore lists
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
- [x] Own profile and staff views of profiles (the private attribute
      block), private messages (lists and the topic view), notifications
- [ ] Deleted topics and posts for staff (`post_stream.gaps`,
      `show_deleted`), ignored users, category group moderators, shared
      drafts, group PMs' related messages, the login-required 2FA and
      required-fields redirects, pending reviewables
- [x] Bookmarks: `/u/:username/bookmarks.json` and
      `/u/:username/user-menu-bookmarks` (core's post and topic bookmarks;
      the `.ics` feed and plugin bookmarkables are refused)
- [ ] The other user-menu endpoints (`/u/:username/user-menu-private-messages`,
      `/review/user-menu-list`)
- [ ] Live updates: a MessageBus-compatible long-poll endpoint (`/message-bus/:client_id/poll`); nothing in the Rust ecosystem provides it, so it is ours to write

## Milestone 4: writes

- [x] Cooking, around the renderer: the options and `PrettyText::Helpers`
      from the database, equal to what Rails recorded (README: Cooking)
- [ ] Cooking, the renderer: Discourse's markdown rules in Rust (no
      vendored JavaScript, no embedded engine; decided 2026-10-02), until
      the recorded corpus cooks byte-equal. The plain `markdown-it` crate
      starts at 28 of 77 entries; the rules to port are
      `discourse-markdown-it` (anchors, quotes, mentions, hashtags,
      emoji, bbcode, uploads, oneboxes, tables, typographer changes), the
      sanitizer, and the bundled plugins' rules (poll, spoiler, details,
      footnote, checklist, local dates, math)
- [ ] Cooking, after the renderer: `PrettyText.cleanup` (mention links,
      rel attributes, hotlinked media) and CookedPostProcessor (oneboxes,
      image sizes, lightboxes)
- [ ] Posting, editing, revisions, likes, flags
- [ ] Private messages
- [ ] Uploads (local first, S3 later), optimized images
- [ ] Background jobs (a Sidekiq replacement for cooking, notifications,
      digests), rate limits
- [ ] Name the places a plugin would attach as they are built
      (NewPostManager modifiers, post events); nothing calls them until
      milestone 5
- [ ] Ember frontend against the Rust API as a compatibility check, not a
      product goal

## Milestone 5: plugins

After writes (moved 2026-10-01, was 3.5): the plugin API is mostly the
write endpoints. The design and its open decisions are in PLUGINS.md.

- [ ] Decide the runtime: WASI components or an embedded scripting
      language (PLUGINS.md decision 6)
- [ ] Tier 0: `plugin.toml` with settings, preloaded custom fields,
      assets, i18n, the tables the plugin owns; `discourse-rs plugins`
- [ ] The host functions: keyed batch reads of the plugin's own tables,
      and the Discourse JSON API in-process under manifest scopes
- [ ] `serialize`, `html`, `modify` batched per response, with
      discourse-solved's read side as the first plugin and its parity
      ignores deleted; the voting plugin second
- [ ] `route` with core's session and CSRF, `event`, `job`; the failure
      policy
- [ ] Hosted plugins: the same API over HTTP with a scoped key, events
      as signed webhooks

## Not planned

- Running Discourse migrations: Discourse owns the schema; re-vendor
  `structure.sql` and re-snapshot instead
- Plugins with the host's permissions: native code in the process, or a
  subprocess with its own database connection (PLUGINS.md decision 3)
- Feature parity with admin (`/admin/...`)
