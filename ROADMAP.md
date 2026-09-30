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
- [ ] Crawler hygiene: `robots.txt`, sitemap, canonical and meta tags
- [ ] Deploy: systemd unit, Caddy snippet, `make install`, first
      date-versioned release with a binary
- [ ] Bench harness: `make bench` (oha/wrk p50/p99, RSS, cold start,
      binary size) against a production-mode Discourse on the same backup;
      Rails' anonymous cache must be accounted for

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
- [ ] Group flair uploads, user fields, badges granted, auth providers,
      user color schemes on `/site.json`
- [ ] Muted defaults (`default_categories_muted`, `default_tags_muted`,
      `mute_all_categories_by_default`), shared drafts category
- [ ] `tags_listed_by_group`, login-only list filters
- [ ] Profiles: bios (PrettyText.excerpt), suspended/silenced users, user
      status, featured topics, letter avatars (`/letter_avatar_proxy` is a
      proxy to avatars.discourse.org; serve or generate them)
- [ ] Search: advanced filters (`in:`, `status:`, `category:`, `#`, `@`,
      `tags:`, `before:`/`after:`, `order:`), search contexts, `search_for_id`,
      rate limits, pg headlines
- [ ] Plugin-added keys (solved, voting, reactions, ...) behind an
      installed-plugins model instead of parity ignore lists
- [ ] Topics without a stored slug (`Slug.for`)

## Milestone 3: sessions

Logged-in readers, no writes yet.

- [ ] Login: local (bcrypt `user_passwords`), then OIDC; CSRF; cookies
      compatible with Rails' session so a cutover keeps sessions
- [ ] Logged-in `Guardian`: secure categories, group permissions, tag
      group permissions by group, staff visibility
- [ ] Read state: `topic_users`, unread/new lists, tracking levels
- [ ] Notifications list, bookmarks list, user preferences (read-only)
- [ ] Live updates: a MessageBus-compatible long-poll endpoint (`/message-bus/:client_id/poll`); nothing in the Rust ecosystem provides it, so it is ours to write

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
- A plugin system before writes exist
- Feature parity with admin (`/admin/...`)
