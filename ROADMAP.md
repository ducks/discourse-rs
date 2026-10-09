# Roadmap

Where discourse-rs is going and in what order. The goal is full parity:
every route Rails serves, admin included, answering as Rails answers.
Each slice is a branch merged `--no-ff` to main, measured against Rails
with the parity harness (see README: Parity). Update this file in the same
commit as the slice.

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

- [x] Fancy titles for topics without a stored `fancy_title` (computed
      and written back on read)
- [ ] Unicode emoji in titles (`performEmojiEscape` turns them into codes)
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
      status, featured topics, generated letter avatars (`/letter_avatar`,
      for sites that turn the proxy off)
- [ ] Search: advanced filters (`in:`, `status:`, `category:`, `#`, `@`,
      `tags:`, `before:`/`after:`, `order:`), search contexts, `search_for_id`,
      rate limits, pg headlines
- [ ] Plugin-added keys (solved, voting, reactions, ...) through the plugin
      protocol (milestone 5) instead of parity ignore lists
- [ ] Topics without a stored slug (`Slug.for`)
- [ ] RSS feeds (`.rss` for the lists, categories, tags, topics, a
      user's posts and activity)
- [ ] Multisite: several forums from one process, by hostname
      (RailsMultisite)

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
- [x] Live updates, the transport: pg-bus (its own crate,
      github.com/ducks/pg-bus) in MessageBus's place, Postgres only: a
      message is written in the transaction that made the change and
      goes out when it commits, in commit order. GET /bus/events
      (server-sent events) and GET /bus/poll (long poll); MessageBus's
      user_ids and group_ids are audience tags (`src/bus.rs`). Not
      MessageBus's wire protocol: the client is ours (milestone 4.5)
- [x] Live updates: the notification state (`/notification/<id>`) from
      every place Rails publishes it
- [x] Live updates: `/topic/<id>` (Post#publish_change_to_clients! and
      the topic's stats) for replies, edits and recooks, likes and
      unlikes, staff deletion and recovery, to the topic's audience
      (everyone, a restricted category's groups, a message's
      participants and staff; whispers to staff and the author). Not
      yet: :acted (notify flags, unhiding), :read, :rebaked, permanent
      deletion, reload_topic with topic edits
- [x] Live updates: TopicTrackingState for regular topics, keeping
      lists and the new and unread counts current: `/new` for new
      topics, `/latest` (bumped, muted, unmuted), `/unread` for replies
      to the users tracking the topic, `/unread/<id>` for the reader's
      own position, `/delete` for deleted topics, and the reader's
      notification level change on `/topic/<id>`. The
      post_update_topic_tracking_state job does its part now. Not yet:
      messages (PrivateMessageTopicTrackingState, TopicGroup), dismiss
      new and dismiss new posts (topic bulk actions are not ported),
      recovering topics, category changes
- [x] Live updates: `/notification-alert/<id>`
      (PostAlerter.create_notification_alert) for a user's first
      notification of an alerting type on a post: the post's url, topic
      title, excerpt and who, to users seen in the last 30 days
- [x] Live updates: the starting position handed to the client with the
      page: `<meta name="bus-position">` on every server-rendered page,
      taken before the page reads its data, for /bus/events and
      /bus/poll. A client working from the JSON API opens its stream
      first and then loads, which needs no position. Rails' own position
      fields in JSON (`message_bus_last_id`,
      `notification_channel_position`) keep Rails' values

## Milestone 4: writes

- [x] Cooking, around the renderer: the options and `PrettyText::Helpers`
      from the database, equal to what Rails recorded (README: Cooking)
- [x] Cooking, the renderer: Discourse's markdown rules in Rust on the
      markdown-it crate (no vendored JavaScript, no embedded engine;
      decided 2026-10-02). All 102 recorded corpus entries cook
      byte-equal to Rails: core's features, the sanitizer, linkify, and
      the bundled plugins' poll, details, spoiler, checklist, footnotes,
      local dates, policy. Of 4232 posts from meta.discourse.org (kept
      local, scripts/fetch-discourse-corpus), 4231 cook byte-equal; the
      other is a discourse-ai quote the reference's plugins allow
- [ ] Cooking, plugins left: math, chat transcripts, events, graphviz;
      refused explicitly today
- [x] Cooking, `PrettyText.cleanup`: rel attributes, mention links,
      hidden direction marks, video thumbnails, HTML5 re-serialization;
      all 482 posts of the Faker backup cook byte-equal
- [x] The `cooked` column: `Post#cook` with the post's options, then the
      post processor's html steps (quote marks, local urls, user ids off
      links, nofollow); every post of the seed and the Faker backup gets
      the column Rails writes
- [x] Post processor, images: uploads sized, thumbnailed and given
      lightboxes (`convert_to_link!`, `OptimizedImage.create_for` in Rust),
      the post's and topic's image, topic thumbnails, upload references,
      the first post's excerpt after processing
- [ ] Post processor, the rest: oneboxes (network fetches, the onebox
      engines), images from other sites sized over HTTP, hotlinked media,
      optimized videos, badges, a category description synced from its
      definition
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
- [x] Review actions on flagged posts (PUT /review/:id/perform/:action):
      agree and keep, agree and hide (Post#hide!), disagree, ignore, with
      the version check, the flags settled, the transition and the
      flaggers' stats; staff taking action on a flag, and hiding past the
      threshold; hidden posts serialized as Rails does. Measured against
      Rails
- [ ] Flags and reviews that go further: closing topics and silencing new
      users on flags, notify user and notify moderators messages, illegal,
      undoing flags, flags on posts already in review, deleting, editing,
      restoring and unhiding from the queue, penalties
- [x] The review queue (GET /review.json) for admins: flagged posts with
      `list_for`'s status, type, topic, category, user, priority and sort
      filters, paging, the side-loaded users, topics, scores, score types,
      bundled actions and histories, and the meta. Measured against Rails.
      Refused: moderators and category group moderators, other reviewable
      types, notes, claims, score reasons, penalized authors, the date and
      additional filters
- [x] Topic status (PUT /t/:id/status): close, open, archive, unarchive,
      unlist, relist, pin, unpin, pin globally, with the small action post
      (PostCreator's small action path), featured topics, the category's
      count, hot scores, visibility reasons and the staff action log.
      Measured against Rails
- [ ] Topic status that goes further: messages, topic timers, pinning
      until a time
- [x] Deleting posts and topics (DELETE /posts/:id, DELETE /t/:id) and
      recovering replies (PUT /posts/:id/recover): PostDestroyer for
      staff, the author marking their own post deleted through
      PostRevisor, the counters, user actions, notifications, category
      latest posts, user stats and staff logs. Measured against Rails
- [ ] Deletion that goes further: permanent deletion, messages, posts with
      links, likes or pending flags, recovering topics, quotes and replies
- [x] Suspending, unsuspending, silencing and unsilencing users
      (PUT /admin/users/:id/...): the user, the staff log, the expiry job, the
      penalty emails, and SystemMessage (a private message from the site
      contact through PostCreator), which also runs send_system_message
      (the post_hidden message). Measured against Rails
- [ ] Penalties that go further: several users at once, acting on a post,
      penalties from the review queue, re-penalizing (its "how long ago"
      message), auto-silencing new users on spam flags
- [x] Changing a site setting (PUT /admin/site_settings/:id) for core
      settings of the plain types: the cast, the hidden and archive checks,
      TypeSupervisor's validation, the stored override and the
      change_site_setting log. Measured against Rails
- [ ] Site settings that go further: lists, uploads, groups, categories,
      plugin settings, bulk updates, user preference backfills, and the
      settings whose change handlers write (title, site_description...)
- [x] Read tracking: POST /topics/timings (post timings and reads, time
      read, the notifications on posts read, the topic user's last read
      post and auto tracking, the day's visit) and PUT
      /notifications/mark-read (one, all, by type). Measured against Rails
- [x] Drafts: POST /drafts (saved under the draft sequence, a stale sequence
      a 409, force_save, the draft count, the edit conflict check), GET
      /drafts/:id and DELETE /drafts/:id. Measured against Rails
- [x] Bookmark writes: POST /bookmarks for posts and topics (visibility,
      Bookmark's validations, the topic user's bookmarked flag), PUT
      /bookmarks/:id, PUT /bookmarks/:id/toggle_pin, DELETE /bookmarks/:id.
      Measured against Rails
- [x] User preferences: PUT /u/:username (options, profile with the bio
      cooked, name with its staff log, category tracking, muted users and
      allowed PM senders), for the user and staff. Measured against Rails.
      Refused: user fields, backgrounds, the notification schedule, titles
      and groups, tag tracking, themes, sidebar links, user status
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

## Milestone 4.5: the client

The whole application in Rust, the browser side included: not Ember on
top of the Rust API (decided 2026-10-05). The JSON API stays Rails'
shape, measured as before, so the client is one consumer of it and
Discourse's own clients keep working.

- [x] Decide the stack: the server-rendered askama pages grown into the
      app with htmx (decided 2026-10-05). Rust renders every piece of
      HTML, live updates included: a page's live endpoint turns bus
      messages into fragments for that viewer, sent as htmx out-of-band
      swaps over SSE. htmx and its SSE extension are vendored
      (static/vendor), no build step. WASM only where it pays, the
      composer preview first. Datastar was the alternative (SSE native,
      signals built in); the server side would be the same
- [x] Navigation without reloads, as Ember's router gives (2026-10-09):
      links are boosted (hx-boost) and swap the next page's body in,
      history included; static/js/page.js runs the page scripts across
      swaps (listeners once, per-page setup and teardown), brings over
      the body's attributes, swaps error pages in and leaves non-pages to
      the browser. What stays across pages is hx-preserve: the composer
      now, the chat drawer next
- [x] First slice, the topic page: new, edited, liked, deleted and
      recovered posts arrive live (GET /t/:id/live, on the last page),
      and members reply from a form on the page, the reply coming back
      over the stream. Checked in headless Firefox as well as the tests
- [x] Login from a page: /login (and the front page while login is
      required) has a username and password form posting to /session
      with htmx and the visitor's CSRF token, Rails' error shown when
      refused, then static#enter back to the page the login gate sent
      the reader from (destination_url). The gate now guards routed
      requests only, as Rails' before_action does, so an unknown path
      such as a browser's /favicon.ico is a 404 rather than a redirect
      that overwrites destination_url
- [x] Topic lists live: on /latest, /new and /unread, GET /live/lists
      sends the "N new or updated topics" banner (latest) and a member's
      unread and new counts in the nav, each the viewer's own list query
      run again on connect and after the tracking messages they may hear
      (a burst is one recount; nothing is sent when nothing changed). As
      Discourse does, the list is not reordered under the reader; the
      banner reloads it
- [x] The header: a member's unread notification count (on connect,
      then from their notification state) and the alert for a new
      notification (PostAlerter's payload, its user content escaped), on
      every page. One stream per page now, GET /live, carrying whatever
      the page follows: a topic's posts, a list's banner, a member's
      counts and notifications; it replaces /t/:id/live and /live/lists
- [x] Likes from the topic page: a member's like button (post_actions,
      undone with DELETE), its state from their actions_summary; the post
      comes back re-rendered on their stream, on whichever page of the
      topic shows it (topic pages now always connect; new posts are
      appended only on the last). A member's htmx requests carry their
      CSRF token from the body; refused requests show the server's
      errors in a page alert
- [x] Bookmarks from the topic page: a member's bookmark button
      (POST /bookmarks, DELETE /bookmarks/:id). Rails publishes nothing
      for bookmarks, so the page fetches the post again as the member now
      sees it, from GET /live/post/:id (the stream's renderer, a 404 for
      a post they cannot see). Reminders and names are not on the page
      yet
- [x] The user menu: the header's bell opens a member's recent
      notifications (GET /user-menu, the Ember client's
      `/notifications?recent=true` list as HTML), who, what and where
      linking to each post, unread ones marked, and "mark all read".
      Opening it marks them seen, and the published state clears the
      count over the live stream. Not yet: the menu's other tabs
      (replies, mentions, likes, messages, bookmarks, review), marking
      one notification read on click
- [x] Discourse's look, ported by hand from its SCSS (only the rules a
      page uses, compared against the reference with scripts/ui-compare):
      the color schemes, fonts and foundation, the header, the nav pills,
      the topic list as Ember renders it, letter avatars, and the sidebar
      (the community section with More and a member's own links, the
      categories and tags sections from the site's defaults or a member's
      own, active links, collapsed sections and the hidden sidebar kept in
      localStorage as Ember keeps them), and the topic page's title and posts
      (avatars, names, post infos, cooked content, small actions, time gaps,
      the post menu's like, copy link, bookmark and reply), the topic map,
      the timeline following the post being read, the footer buttons
      (share, the topic's bookmark, reply) and the suggested topics. Not
      yet: the topic map's menus; the footer's flag, mark unread,
      notifications and pinned buttons and the admin menu; the post
      menu's flag, edit, delete, admin, read and replies (modals and the
      composer); the sidebar's header actions and Customize (modals), the welcome banner, the category and tag dropdowns by the nav pills,
      the keyboard shortcuts modal behind the sidebar footer's button,
      icons beyond the static sprite (a category's icon style)
- [x] The composer (Ember's #reply-control in its markdown mode): reply to
      a topic or a post, create a topic in a chosen category, edit a post
      (its markdown from GET /raw/:topic_id/:post_number), with the
      toolbar's bold, italic, link, quote, code and list, minimizing,
      fullscreen and resizing. Not yet: tags, the heading, emoji and
      options menus, the composer actions menu, the rich text editor;
      edits within the grace period are refused by the post reviser
- [x] The composer's preview: the renderer (crates/markdown) built to
      WebAssembly by build.rs, served at /assets/markdown.wasm with the
      site's render settings at /assets/markdown-settings.json, rendering
      as the post cooks less what the server looks up (quoted avatars,
      hashtags, upload urls) and the rel attributes cleanup adds
- [x] Tracking state on the pages: TopicTrackingState.report for a
      member (topic_tracking_report), counted as topic-tracking-state.js
      counts, in the New and Unread pills (unified new folding unread
      into New) and as dots or counts on the sidebar's Topics, category
      and tag links (sidebar_show_count_of_new_items,
      sidebar_link_to_filtered_list), kept current by the page's stream.
      Reading is timed as screen-track does (static/js/screen-track.js,
      POST /topics/timings) and marks the posts read. Not yet: the
      topic list's per-row state from the tracking state, an anonymous
      reader's time, dismissing new
- [x] The composer's drafts: saved as Ember's composer saves them (two
      seconds after typing stops, at once past fifteen, on minimizing,
      by beacon when the page is left), restored for a reply or an edit
      on the topic's key, deleted on discard, kept by save and close,
      taken by the post (posts#create's draft_key); the draft status
      line for offline and conflicting saves; GET /drafts
      (Draft.stream, DraftSerializer) and the drafts menu beside New
      Topic. Not yet: the drafts page (/my/activity/drafts), the
      conflict user's avatar, the draft saved toast, multipart bodies
      (Ember's beacon sends FormData; ours sends a form)
- [x] The composer's uploads: the toolbar button and the file picker (the
      member's authorized extensions), paste and drop, the "Uploading:
      name…" placeholders and their numbering, the progress line with
      cancel, getUploadMarkdown's image, media and attachment markdown,
      and the preview's upload:// urls resolved (POST
      /uploads/lookup-urls). Not yet: checking extensions and sizes
      before sending (the server's errors are shown), simultaneous_uploads,
      grids of consecutive images, rich text paste, video thumbnails
- [ ] The user menu, notifications, bookmarks, messages, preferences
- [ ] Review queue and topic status for staff

## Milestone 5: plugins

After writes (moved 2026-10-01, was 3.5): the plugin API is mostly the
write endpoints. The design and its open decisions are in PLUGINS.md.

- [x] Decide the runtime (PLUGINS.md decision 6, 2026-10-08): bundled
      plugins are Rust in src/plugins/ on lifecycle phases; third-party
      plugins a sandboxed runtime later (Luau spike on
      spike/plugin-runtime)
- [ ] The bundled plugins, default-on first: solved (topic list, topic
      view, posts, users, accepting and unaccepting answers, the by_user
      list, shared issues, and the UI on the pages; left: auto close
      timers, web hooks, crawler schema markup, search filters, lists
      filtered by solved status and the admin dashboard),
      topic voting (done but for what core doesn't port yet: the votes
      RSS feed, search's min_vote_count: and order:votes, /filter's votes
      filters, the release and reclaim hooks on topic recovery, category
      edits and merges, the category editor's voting toggle, the docked
      header's vote box, and an anonymous vote cast after login; web hooks
      and discourse-workflows triggers are refused), reactions (done but
      for what core doesn't port yet: the Reactions received notifications
      page, the allow-any-emoji picker; reactions in messages are refused,
      as likes there are; the animations and touch gestures are not
      ported), presence, templates, narrative-bot, chat, poll's voting
- [ ] Tier 0: `plugin.toml` with settings, preloaded custom fields,
      assets, i18n, the tables the plugin owns; `discourse-rs plugins`
- [ ] The host functions: keyed batch reads of the plugin's own tables,
      and the Discourse JSON API in-process under manifest scopes
- [ ] The third-party runtime on the same phases: `serialize`, `html`,
      `modify` batched per response
- [ ] `route` with core's session and CSRF, `event`, `job`; the failure
      policy
- [ ] Hosted plugins: the same API over HTTP with a scoped key, events
      as signed webhooks

## Milestone 6: admin

Everything under `/admin` (Rails' admin namespace is about 30% of its
routes), JSON first and measured as the rest is, then its pages in the
client (milestone 4.5). Ordered by what running a forum needs first; the
early slices need not wait for milestone 5. Done so far: changing plain
site settings, suspending and silencing (milestone 4), `handle_mail` and
API keys for posting, and grant_moderation! for the first admin's login.

- [x] Site settings: the list (`GET /admin/site_settings`, by category,
      plugin and names): every visible setting, core and bundled plugins'
      (and the agent settings discourse-ai registers from Ruby), with its
      labels from the locale, value and default, type details and the
      default theme; enum classes and choices expressions recorded from
      Rails (scripts/record-setting-enums), the database-backed ones
      computed. Measured against Rails: all 1542 settings of the reference
- [ ] Site settings: the setting types milestone 4 refuses
- [x] Users: the lists and filters (`GET /admin/users/list`), measured
      against Rails
- [x] Users: the admin view of a user (`GET /admin/users/:id`), measured
      against Rails
- [x] Users: granting and revoking moderation, revoking admin (granting
      admin's confirmation flow is next)
- [x] Users: trust levels and their locks (`Promotion`), measured against
      Rails; the badge grant queue waits for badges
- [x] Users: approving, activating, deactivating and logging out, adding
      and removing groups, the primary group, measured against Rails
- [ ] Users: granting admin (its second factor or email confirmation),
      deleting, anonymizing and merging, impersonating, penalties on several users
- [ ] Groups: creating, editing and deleting, members and owners in bulk,
      automatic membership by email domain
- [x] Logs: staff actions, screened emails, IPs and URLs, search logs,
      email logs (sent, skipped, bounced, received, rejected), measured
      against Rails
- [ ] Logs: a search term's details, incoming email details
- [ ] Backups: creating, listing, downloading and restoring through the
      app (scripts/restore-backup restores today), read-only mode
- [ ] Badges: creating, editing and deleting, groupings, granting and
      revoking, badge SQL, and the BadgeGranter jobs that award them
- [ ] Watched words, permalinks, embeddable hosts, user fields, form
      templates, custom flags, custom emoji
- [ ] API keys and their scopes, web hooks and their deliveries (the
      events core sends to them)
- [ ] Email: the settings test, previews, templates and email style
- [ ] Dashboard, reports and problem checks, admin notices
- [ ] Customize: themes and components, color schemes, site texts
      (translation overrides), sidebar defaults; the `/admin/config` pages

## Not planned

- Running Discourse migrations: Discourse owns the schema; re-vendor
  `structure.sql` and re-snapshot instead
- Plugins with the host's permissions: native code in the process, or a
  subprocess with its own database connection (PLUGINS.md decision 3)
