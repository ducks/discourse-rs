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
- [x] Cooking, the renderer: Discourse's markdown rules in Rust on the
      markdown-it crate (no vendored JavaScript, no embedded engine;
      decided 2026-10-02). 76 of the 77 recorded corpus entries cook
      byte-equal to Rails: core's features, the sanitizer, linkify, and
      the bundled plugins' poll, details, spoiler, checklist, footnotes
- [ ] Cooking, plugins left: local dates (needs a timezone database),
      math, chat transcripts, events, graphviz, policy; all refused
      explicitly today
- [x] Cooking, `PrettyText.cleanup`: rel attributes, mention links,
      hidden direction marks, video thumbnails, HTML5 re-serialization;
      all 482 posts of the Faker backup cook byte-equal
- [x] The `cooked` column: `Post#cook` with the post's options, then the
      post processor's html steps (quote marks, local urls, user ids off
      links, nofollow); every post of the seed and the Faker backup gets
      the column Rails writes
- [ ] Post processor, the rest: oneboxes (network fetches, the onebox
      engines), images (sizes, optimized images, lightboxes), optimized
      videos; and its writes (post/topic image, badges, upload links)
- [x] Posting, editing, revisions: replies, regular topics, raw and
      edit-reason edits, revision diffs; the 14 cases in parity/writes
      write the rows Rails writes (tests/writes.rs). Refused for now:
      review queue, watched words, TL0, PMs, tags, quotes, uploads,
      oneboxes, topic links, grace-period edits, title/category edits
- [x] Likes: like and unlike (POST and DELETE /post_actions), the counts,
      the liked flag, daily likes, user actions and the liked notification,
      measured against Rails. Refused: likes in messages, liked
      notifications that would consolidate; the like rate limit is not ported
- [x] Flags: off topic, inappropriate and spam (POST /post_actions), the
      post's count, the ReviewableFlaggedPost with its score and history,
      the topic's reviewable score, the auto close, auto hide and auto
      silence thresholds checked, measured against Rails
- [ ] Flags that act or message: taking action as staff (agree and hide),
      hiding past the threshold, closing topics and silencing new users,
      notify user and notify moderators messages, illegal, undoing flags,
      flags on posts already in review
- [x] Private messages: new messages to users and replies in them, the
      allowed users, the recipients watching, the PM user actions, the
      private_message notification and email (participants, `[PM]`
      subject), measured against Rails. Refused: group messages, messages to
      email addresses, recipients who screen the sender, membership requests
- [x] Uploads: `POST /uploads.json` for attachments and GIFs, stored
      locally as Rails stores them (dominant colour by `magick`), measured
      against Rails
- [ ] Uploads Rails optimizes (PNG, JPEG), optimized images and
      thumbnails, avatars, S3
- [x] Background job queue: Postgres (`discourse_rs.jobs`, the one table
      the port owns), a worker beside the web server, Sidekiq's retries;
      posting enqueues what Rails enqueues (measured)
- [x] Jobs: feature_topic_users, process_post, post_alert (mentions,
      replies, topic/category/tag watchers, first-post watchers, user
      actions, the user_email job); each measured by running it on Rails
      (`run_jobs` cases)
- [x] Notification email: user_email for replies, mentions, quotes,
      posted and first-post watching; the message (subject, text, headers,
      HTML with Discourse's inline styles) byte-identical to Rails, sent
      over SMTP from the `DISCOURSE_SMTP_*` settings with lettre; skipped
      emails logged like Rails
- [x] Accounts: signup (honeypot, validations, the rows User's callbacks
      write), activation, email login, password reset by code or link,
      and their emails (critical_user_email, send_email_login_code),
      byte-identical to Rails; ServerSession as
      `discourse_rs.server_sessions`
- [x] Reply by email, first slice: reply keys and VERP bounce addresses
      in notification emails; POST /admin/email/handle_mail and
      process_email for a plain-text reply by a known user to a reply
      key (the incoming_emails row with the gem's re-serialized raw, the
      reply trimmed by a port of email_reply_trimmer, the post by email).
      Measured: the gem's own trimmer corpus, a sample corpus of parsed
      and cleaned emails (parity/incoming_mail), and the write cases
- [x] Reply by email: HTML replies (HtmlToMarkdown and the per-client
      extracters), measured on Discourse's spec HTML and client samples
- [x] Reply by email: rejection emails (Email::Processor's templates,
      once a day per address and kind, the message kept on the incoming
      email)
- [x] Admin API keys (mail-receiver's handle_mail, and posting)
- [ ] Reply by email, next: staged users, attachments, bounces, likes
      and notification levels by email,
      POP3 polling, group and category addresses (email_in)
- [ ] PM emails, previous-replies context
- [ ] Jobs, next: digests, pull_hotlinked_images,
      notify_mailing_list_subscribers, scheduled jobs (category stats, top
      topics, digests), rate limits
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
