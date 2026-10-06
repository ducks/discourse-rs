# discourse-rs

A Rust port of [Discourse](https://github.com/discourse/discourse), aiming
for feature parity: same database, same JSON API. The backend comes first;
the browser client is to be Rust too rather than Ember (ROADMAP.md,
milestone 4.5), and keeping Rails' API means Discourse's own clients
still work against it.

The pre-rewrite Actix/Diesel version is tagged `v20260610.0.1`.

## Principles

- **Discourse owns the schema.** `vendor/discourse/` holds files copied
  verbatim from a pinned Discourse commit (`vendor/discourse/DISCOURSE_REF`):
  `db/structure.sql`, `config/site_settings.yml`, `server.en.yml` and the base colors.
  discourse-rs has no migrations and refuses to start against a database
  missing any of them.
- **Parity is measured, not claimed.** Every ported endpoint gets a case in
  `parity/cases`, diffed against real Discourse responses.
- **Port behavior from the Rails source**, citing the file it came from, and
  port the matching request specs as tests.

## Stack

axum, tokio, sqlx (Postgres 16 + pgvector), clap, serde.

## Setup

```bash
nix-shell          # toolchain, Postgres on port 5442, DATABASE_URL/TEST_DATABASE_URL
db_start
make db-load       # schema + seeds -> discourse_rs_development (FORCE=1 to replace)
make db-test       # schema + seeds -> discourse_rs_test (template for tests)
cargo run          # http://127.0.0.1:8080  (BIND_ADDR to change)
make test          # cargo nextest run: test binaries side by side
```

`seed/fresh_install.sql` is a data snapshot of a freshly provisioned Discourse (a
dv agent at the vendored commit), taken with `make snapshot-dv AGENT=<name>`.
It is the database the parity golden files were recorded from, so tests and
recordings see the same data.

Configuration mirrors Discourse: `RAILS_ENV` (default development, as in
Rails) changes URL generation the same way, and every `DISCOURSE_*` env var is
a GlobalSetting, shadowing the site setting of the same name
(`DISCOURSE_HOSTNAME`, `DISCOURSE_CDN_URL`, `DISCOURSE_TITLE`, ...).

To run against a real Discourse database, point `DATABASE_URL` at it. Extra
migrations (newer Discourse, third-party plugins) are logged and tolerated.

Tests clone the `discourse_rs_test` template into a private database per test,
so they can write freely and run in parallel.

## Vendoring

```bash
make vendor-discourse DISCOURSE=~/discourse/discourse [REF=<sha>]
```

Copies the files as committed at REF (default HEAD), so uncommitted local
changes never leak in: the schema, core's settings and locales, and the
bundled plugins' settings and client locales. Re-snapshot the seed from an
agent at the same commit and reload the databases afterwards.

## Parity

Writes are measured separately: `scripts/record-writes <agent>` runs the
requests in `parity/writes/cases.json` on the reference inside a
rolled-back transaction and records responses, row changes and enqueued
jobs; `tests/writes.rs` replays them on the seed and compares
(`WRITES_DIFF=1` prints the differences). Cases replay side by side, each on
its own database copy, four at a time (`WRITES_JOBS=n` to change it).
A case with `run_jobs` also runs those of its enqueued jobs (on Rails inside
the same transaction, here through the job queue), so a job's writes and
the jobs it enqueues in turn are measured the same way.
A case can turn settings on (`settings`, set with `SiteSetting.set` inside
the transaction) and add fixture rows (`setup`, SQL run on both sides
before the snapshot), as the reply-by-email cases do for their reply key.

Incoming mail has two corpora of its own: the email_reply_trimmer gem's
tests (`parity/email_reply_trimmer`, `tests/reply_trimmer.rs`), and sample
emails with what Rails parses from them and what Email::Cleaner stores
(`parity/incoming_mail`, recorded by `scripts/record-incoming-mail <agent>`,
compared by `tests/incoming_mail.rs`). HtmlToMarkdown is measured on the HTML
in Discourse's own spec plus samples (`parity/html_to_markdown`, recorded by
`scripts/record-html-to-markdown <agent>`, `tests/html_to_markdown.rs`).

Outgoing mail goes over SMTP as configured by the `DISCOURSE_SMTP_*`
settings Discourse uses (address, port, user name, password, authentication
plain/login, STARTTLS or forced TLS, certificate verification, HELO domain).
Without an address Rails falls back to sendmail; that is refused, and the
job fails with the reason. Captured messages are compared with what Rails
rendered (`run_jobs` cases ending in `_email`).

Background jobs run from `discourse_rs.jobs` in Postgres: a worker runs
beside the web server unless `DISCOURSE_RS_JOBS=off`. A job kind that is
not ported fails at once and keeps the reason in `last_error`.

`parity/cases` lists requests, one per line:

```
GET /latest.json ignore=/topic_list/topics/*/bumped_at
```

`ignore` takes JSON pointers (`*` matches any element or key) for fields that
legitimately differ between runs. JSON is compared structurally; status and
media type must match.

```bash
make parity RAILS_URL=http://127.0.0.1:3000   # live: Rails vs discourse-rs
make parity-record RAILS_URL=...              # save Rails responses to parity/golden/
make parity-check                             # running discourse-rs vs golden
```

`cargo test` also replays every golden file against the in-process router
(`tests/parity.rs`) on the snapshot database with the recorded environment,
with the clock pinned to the golden's `recorded_at` (the port's
`clock::now()` and the database's `now()`), so edit windows and "recent"
lists answer as they did when Rails was recorded.

Recording workflow: create a fresh dv agent, run `scripts/reference-env
<agent>` (no Sidekiq or scheduler, so its database only changes when a
recording changes it; see `parity/reference.env`), vendor its commit,
snapshot its database, write its env to `parity/environment`, then
`make parity-record`.
Live comparison needs both sides on the same database and configuration:
run discourse-rs against a `make db-load` of the snapshot with
`env $(grep -v ^# parity/environment) cargo run`.
## Bench

`make bench TARGETS="rs=URL rails=URL"` and `make bench-startup` compare the
port with a Discourse serving the same backup; BENCH.md keeps the runs and the
caveats (production-mode reference, Rails' anonymous cache, search rate limits).


## Ported

What comes next, and in what order, is in [ROADMAP.md](ROADMAP.md).

| Route| Route | Discourse source | Notes |
|---|---|---|
| `GET /srv/status` | `forums_controller.rb` | |
| `GET /site/basic-info` | `site_controller.rb#basic_info` | S3 upload CDN not supported (explicit 500) |
| `GET /site` | `site_controller.rb#site`, `Site.json_for`, `SiteSerializer`, `SiteCategorySerializer` | anonymous and logged-in (groups by visibility, permission and can_edit on categories, staff and admin keys); plugin-added keys not yet (see `parity/cases`) |
| `GET /latest` | `list_controller.rb#latest`, `TopicQuery#list_latest`, `TopicListSerializer`, `TopicListItemSerializer` | anonymous and logged-in (`topic_users` keys, muted topics/categories/tags, cleared pins, secure categories, unlisted topics for staff); `page`, `per_page`, `order`, `ascending`; plugin keys not yet |
| `GET /t/:slug/:id(/:post_number)` | `topics_controller.rb#show`, `TopicView`, `TopicViewSerializer`, `PostSerializer` | anonymous and logged-in (read state, bookmarks, `can_*` on the topic and posts, `actions_summary` with `can_act`, notices), private messages for participants (related and suggested messages, allowed users and groups); deleted replies for staff and ignored users refused; slug and page redirects, 404 JSON; suggested topics deterministic (Rails randomizes); plugin keys not yet |
| `GET /c/:slug_path/:id(/l/latest)` | `list_controller.rb#category_default`, `#category_latest` | anonymous and logged-in, every list filter (`/l/unread`, `/l/new`, ...), restricted categories for members; subcategory scoping, `/none`, `/l/top`, `/l/hot`, category pins and sort, slug redirects |
| `GET /top`, `GET /hot` | `list_controller.rb#top`, `#hot`, `TopicQuery#list_top_for`, `#list_hot` | anonymous and logged-in; period selection (`best_period_for`), `/top/:period` redirects |
| `GET /unread`, `/new`, `/unseen`, `/read`, `/posted`, `/bookmarks` (+ `.json`, `/c/.../l/<filter>`) | `list_controller.rb`, `TopicQuery#list_unread`, `#list_new` (unified new), `#list_unseen`, `#list_read`, `#list_posted`, `#list_bookmarks` | logged in only (anonymous: JSON 403, HTML 404); `enable_unified_new` with its group gate; `subset=` not ported |
| `GET /notifications(.json)` | `notifications_controller.rb#index`, `Notification.prioritized_list`, `NotificationSerializer` | logged in only; `recent` (with the `seen_notification_id` bump unless `silent`), paged with `filter`/`limit`/`offset`, `filter_by_types`, admins' `username`; the accessible-topic and disabled-badge filters; `populate_acting_user` settings, pending reviewables and DiscourseConnect not ported |
| `GET /u/:username/bookmarks(.json)`, `GET /u/:username/user-menu-bookmarks(.json)` | `users_controller.rb#bookmarks`, `#user_menu_bookmarks`, `BookmarkQuery`, `PostBookmarkable`, `TopicBookmarkable`, `UserBookmarkListSerializer` | the owner and admins; post and topic bookmarks as the viewer may see them (secure categories, the owner's private messages, whispers, hidden first posts), pinned and reminder ordering, `q`, `page`, `limit`; the user menu's unread reminder notifications with the bookmarks they are not about; the `.ics` feed, plugin bookmarkables (chat messages) and `lazy_load_categories` not ported |
| `GET /topics/private-messages{,-sent,-archive,-unread,-new,-warnings}/:username(.json)`, `GET /topics/private-messages-group/:username/:group(/archive,/new,/unread).json` | `list_controller.rb#private_messages*`, `TopicQuery::PrivateMessageLists` | logged in only; the mailbox owner vs the viewer (admins read any inbox), `message_route`'s 403/404s, participants and allowed users on items; `publish_read_state` groups and tag lists not ported |
| `GET /categories` | `categories_controller.rb#index`, `CategoryList`, `CategoryDetailedSerializer` | anonymous and logged-in (secure categories, permission, can_edit, muted topics, cleared pins); featured topics; no pagination, parent, tag filter or `subcategory_list` yet |
| `GET /tag/:name`, `GET /tag/:slug/:id(/l/:filter)`, `GET /tags/c/:slug_path/:id(/none)/:tag` | `tags_controller.rb#show_*`, `TopicQuery#filter_by_tags`, `TagSerializer`, `DiscourseTagging.visible_tags` | anonymous and logged-in (the viewer's tag groups and categories); `tags[]` intersections, synonyms, tag-group and category visibility, canonical redirects; `tags_listed_by_group` not ported |
| `GET /tags` | `tags_controller.rb#index` | anonymous and logged-in (staff counts, admins see unused tags, `pm_count`); `tags_listed_by_group` off |
| any route, `login_required` on | `application_controller.rb#redirect_to_login_if_required` | JSON 403 `not_logged_in` (topics add extras), HTML 302 to `/login` with `destination_url` cookie, `/` and `/login` render a login page; `/srv/status`, `/site/basic-info` and static files stay open; `auth_immediately` with DiscourseConnect or a single external login not ported |
| `GET /search(.json)?q=&page=`, `GET /search/query(.json)?term=&type_filter=` | `search_controller.rb#show`, `#query`, `lib/search.rb`, `GroupedSearchResults`, `SearchLog` | anonymous and logged-in (secure categories, whispers, per-user topic keys on `/search.json`, the log keyed by user); plain words and quoted phrases, relevance ranking with category priority and closed/archived penalties, per-topic aggregation, blurbs, user/category/tag/group facets, search log with the 5 s per-IP fold; advanced syntax (`in:`, `status:`, `category:`, `#`, `@`, `order:`, ...), search contexts, `search_for_id`, rate limits and pg headlines not ported |
| `GET /u/:username(.json)` (+ `/summary`, `/activity`, `/badges`...), `GET /user_actions.json` | `users_controller.rb#show`, `#summary`, `posts_controller.rb#user_posts_feed` (`/activity.json`), `UserSerializer`, `HiddenProfileSerializer`, `UserSummary`, `UserAction.stream` | every viewer: mute/ignore/PM flags and visible groups for other members, the private block (emails, 2FA, notification buckets, auth tokens, preferences as `user_option`) for the user themself and staff, the badge post side-load for admins; badges side-loads, profile view tracking, hidden-profile rules; suspended/silenced users, bios, featured topics, user status, `include_post_count_for`, `/card.json` not ported |
| `GET /robots.txt`, `GET /robots-builder.json` | `robots_txt_controller.rb` | allowed/blocked crawler agents, `allow_index_in_robots_txt`, `overridden_robots_txt`, the Sitemap line |
| `GET /sitemap.xml`, `/sitemap_:n.xml`, `/sitemap_recent.xml`, `/news.xml` | `sitemap_controller.rb`, `Sitemap` | the index regenerates the `sitemaps` rows the hourly job would; recent/news touch theirs |
| `POST /session`, `DELETE /session/:username`, `GET /session/csrf`, `GET /session/current`, `POST /login` | `session_controller.rb`, `Auth::DefaultCurrentUserProvider`, `UserAuthToken`, `CurrentUserSerializer` | local logins (PBKDF2), Rails-compatible encrypted `_t` and `_forum_session` cookies (same `secret_key_base` keeps sessions across a cutover), CSRF tokens, token rotation and expiry, logout, last-seen tracking; 2FA users get Rails' failure payload; rate limits, screened IPs, suspended-user messages, DiscourseConnect not ported |
| `POST /posts(.json)`, `PUT /posts/:id(.json)`, `GET /posts/:id/revisions/:n(.json)`, `GET /posts/:id/revisions/latest(.json)` | `posts_controller.rb#create`, `#update`, `#revisions`, `#latest_revision`, `NewPostManager`, `PostCreator`, `TopicCreator`, `PostRevisor`, `PostValidator`, `SearchIndexer`, `TopicLink.extract_from`, `PostRevisionSerializer`, `DiscourseDiff` | replies and regular topics by trust level 1+ and staff, edits of the raw and edit reason, revision diffs; every table write measured against Rails (`tests/writes.rs`). Private messages: new ones to users (target_recipients) and replies in them, with their allowed users, watching, user actions and the private_message notification and email. Refused: the review queue, watched words, new users (TL0), group messages and messages to email addresses, tags, quotes, uploads, oneboxes, grace-period edits (Rails keeps the original in Redis), title/category edits, fancy titles, hidden revisions. Links to topics are stored with their target and the reflection in the linked topic. Jobs (notifications, post processing) are not run |
| Admin API keys (`Api-Key` with `Api-Username` or `Api-User-Id`) | `Auth::DefaultCurrentUserProvider#lookup_api_user`, `ApiKey#request_allowed?` | the key's user or the one named, allowed IPs, revoked keys, `last_used_at`, no CSRF check, no draft advance or first-post checks on posts; the email:receive_emails scope for handle_mail (what mail-receiver's key needs); measured against Rails. Refused: other granular scopes, user API keys, keys in query parameters, external ids; the admin API rate limit and the API post memoizer are not ported |
| `POST /post_actions(.json)`, `DELETE /post_actions/:id(.json)` (likes and flags) | `post_actions_controller.rb`, `PostActionCreator`, `PostActionDestroyer`, `PostAction#update_counters`, `ReviewableFlaggedPost`, `Reviewable#add_score`, `ReviewableScore`, `SpamRule::AutoSilence`, `UserActionManager`, `PostActionNotifier`, `GivenDailyLike` | like and unlike, reviving an undone like; the post's like count and score, the topic's like count, the liker's topic user, likes given and received, daily likes, LIKE and WAS_LIKED actions, the liked notification (frequency rules, removed on unlike); answered as `render_post_json`; measured against Rails. Flags (off topic, inappropriate, spam): the post action and the post's count, the ReviewableFlaggedPost with its created history and the flag's score (trust level, accuracy bonus), the topic's reviewable score, notify_reviewable; the auto close, auto hide and auto silence thresholds are checked. Refused: likes in messages, liked notifications that would consolidate or be rebuilt from other likers, unliking as staff; flags that send a message (notify user, notify moderators, illegal), queueing for review as staff, flagging topics, flags on posts already in review, flags in messages, undoing flags, and the thresholds when reached for closing the topic, hiding on a trusted spam flag and silencing (hiding past the threshold and staff taking action, which agrees with the flags, are ported); the action rate limits and badge queue are not ported |
| `GET /review.json` | `reviewables_controller.rb#index`, `Reviewable.list_for`, `ReviewableFlaggedPostSerializer`, `ReviewableScoreSerializer`, `ReviewableBundledActionSerializer`, `ReviewableActionSerializer`, `FlaggedUserSerializer`, `ListableTopicSerializer` | admins (others 403, moderators refused); flagged posts by `status`, `type`, `topic_id`, `category_id`, `username`, `flagged_by`, `reviewed_by`, `claimed_by`, `score_type`, `priority`, `sort_order` and `offset` (10 a page, the `load_more_reviewables` link); side-loaded users (flagged authors with their counts, the flagger with custom fields, scorers and history authors), topics, scores with their score types, the agree and disagree bundles with their actions, histories; meta with the score types and counts; measured against Rails. Refused: other reviewable types, the `ids`, date and additional filters, claimed topics, notes, score reasons and conversations, penalized authors, posts the user deleted, potential spam and illegal content, system users, content localization, `GET /review` (HTML) |
| `PUT /review/:reviewable_id/perform/:action_id` | `reviewables_controller.rb#perform`, `Reviewable#perform`, `ReviewableFlaggedPost`, `Post#hide!`, `ReviewablePerformResultSerializer` | staff acting on a pending flagged post: agree and keep, agree and hide (hidden with the reason, the author's post count, the post_hidden system message job, the topic's bumped_at), disagree (the post's flag counts zeroed), ignore; the version check (409), the flags agreed, disagreed or deferred, the transition with its history, the scores settled, the flaggers' flag stats, notify_reviewable; answered with the result and the staff reviewable counts; measured against Rails. Refused: other reviewable types, deleted or hidden posts (restoring, unhiding), deleting, editing, penalties and user deletion from the queue, claiming, group reviewables, category group moderation |
| `PUT /t/:topic_id/status`, `PUT /t/:slug/:topic_id/status` | `topics_controller.rb#status`, `Topic#update_status`, `TopicStatusUpdater`, `Topic#add_moderator_post`, `PostCreator` (small actions), `StaffActionLogger` | closed, archived, visible and pinned (and pinned_globally) on or off, for staff and group moderators: the column (only when it changes), the small action post with its action code (post number from the posts, the topic's counters and last poster untouched, bumped only on opening, its reply action, topic user and timing, the category's latest post, post_alert, feature_topic_users and process_post), featured topics and hot scores removed, the category's topic count and the author's topic count on (un)listing, the visibility reason, scheduled unpins cancelled, the topic_closed/opened/archived/unarchived staff log; measured against Rails. Refused: messages, topics with timers, pinning until a time |
| `DELETE /posts/:id`, `PUT /posts/:post_id/recover`, `DELETE /t/:id` | `posts_controller.rb#destroy`, `#recover`, `topics_controller.rb#destroy`, `PostDestroyer`, `PostRevisor`, `StaffActionLogger` | staff (and group moderators) deleting a reply or a topic: trashed, the notice dropped, the topic's last post and counts (`reset_highest`), poster flag, user actions, reply links, notifications, the category's latest post (with its reindex job), the author's and repliers' counts and last posted date, the category and tag topic counts, NEW_TOPIC, bumped_at, the delete_post/delete_topic staff log; an author deleting their own reply (marked deleted: the raw replaced through PostRevisor, user_deleted); staff recovering a reply (links extracted again, counts, the REPLY action, featured users, the recover_post log, answered with the post); measured against Rails. Refused: permanent deletion, messages, posts with links, likes or pending flags, embedded or published topics, recovering topics and replies with quotes or reply targets |
| `PUT /admin/users/:user_id/suspend`, `/unsuspend`, `/silence`, `/unsilence` | `admin/users_controller.rb`, `User::Suspend`, `User::Silence`, `UserSuspender`, `UserSilencer`, `SystemMessage`, `StaffActionLogger` | staff penalizing a non-staff user: the contract (reason and until required, 400 with its messages), suspended_at/suspended_till or silenced_till, the suspend_user/silence_user staff log with the reason and message, the user_suspension_expired job at the end, push subscriptions cleared, the account_suspended/account_silenced email jobs, the silenced_by_staff and unsilenced system messages (a private message from the site contact through PostCreator without validations, archived for the sender), the unsuspend_user/unsilence_user logs; non-staff get a 404 as the admin routes do; measured against Rails. Refused: several users at once, acting on a post, review queue penalties, penalizing an already penalized user, staff reasons with markup, a site contact group |
| `PUT /admin/site_settings/:id` | `admin/site_settings_controller.rb#update`, `SiteSetting::Update`, `SiteSettings::TypeSupervisor`, `SiteSetting.set_and_log`, `StaffActionLogger` | admins changing a core setting of a plain type (string, integer, float, bool, enum with listed choices): the value stripped and cast, an unknown name and hidden settings refused with Rails' messages, nothing when unchanged, the integer and string bounds with their messages, the override saved (a value back at its default still stored), the change_site_setting log; non-admins get a 404 as AdminConstraint does; measured against Rails. Refused: plugin, themeable and upcoming change settings, other types, validator classes and custom validations, regex and JSON schema strings, default_* preferences, client strings with markup, global shadowing, bulk updates, an archived site, and settings whose change handlers write (title, site_description, must_approve_users, emoji_set, slug_generation_method and others) |
| `POST /topics/timings`, `POST /t/:topic_id/timings`, `PUT /notifications/mark-read`, `PUT /notifications/read` | `topics_controller.rb#timings`, `PostTiming.process_timings`, `TopicUser.update_last_read`, `UserStat.update_time_read!`, `notifications_controller.rb#mark_read` | reading a topic the user can see: timings capped per post (by the batch and the account age) and dropped past the topic's end, existing timings added to, new ones counting a read of the post and a post read for the user, the time read since the last report (a user-last-seen value kept as Rails keeps it in Redis), the notifications on the posts read, the topic user's last read post and time viewed with auto tracking past the threshold, or a first topic user, the day's visit and days visited; marking notifications read by id, all, or by type (invalid types a 400), with seen_notification_id bumped; measured against Rails. Refused: messages read through a group |
| `POST /drafts`, `GET /drafts/:id`, `DELETE /drafts/:id` | `drafts_controller.rb`, `Draft.set`, `Draft.get`, `Draft.clear`, `DraftSequence`, `UserStat.update_draft_count` | a user saving a draft: the data checked (a JSON string within max_draft_length, else 400), max_drafts_per_user (403 with its description), saved under the draft sequence (an existing draft moves the sequence on, a stale sequence is a 409 unless force_save, a missing draft retries at the current sequence), the draft count, and for a draft of an edit the conflict check (the last editor when the post's raw or first post's title changed); reading the current draft and its sequence; clearing at the current sequence (any other sequence still succeeds, as Rails rescues it); measured against Rails. Refused: drafts backed up to a message, drafts with uploads, the conflict check on tags, reading at a given sequence, the drafts list, another user's drafts |
| `POST /bookmarks`, `PUT /bookmarks/:id`, `PUT /bookmarks/:bookmark_id/toggle_pin`, `DELETE /bookmarks/:id` | `bookmarks_controller.rb`, `BookmarkManager`, `PostBookmarkable`, `TopicBookmarkable`, `Bookmark` | bookmarking a post or topic the user can see (else 403; other types a 400): Bookmark's validations in order (once per bookmarkable, no reminder in the past or beyond 10 years, max_bookmarks_per_user, a name within 100), the auto delete preference (given, the user's, or clearing the reminder), the reminder set time, the topic user's bookmarked flag; updating the name and reminder (resetting its last sending when it changes, the pinned option saved as nil as Rails does), pinning, destroying with `topic_bookmarked`; only the owner (403), a missing bookmark 404; measured against Rails. Refused: chat message bookmarks, reminder times not in ISO 8601; the rate limit is not ported |
| `PUT /u/:username(.json)` | `users_controller.rb#update`, `UserUpdater`, `UserOption`, `UserProfile`, `CategoryUser.batch_set`, `StaffActionLogger#log_name_change` | the user or staff (else 403, unknown users 404): `OPTION_ATTR` options (booleans as Rails reads them, `text_size` with its sequence, the hide profile flag, mailing list mode turning digests off, `TrackedTopicsUpdater` on a new auto track threshold), the profile (website with its scheme, the bio cooked with the hotlinked images job), the name (with `can_edit_name?`, the change logged and the display name job) and locale, category tracking with auto watch and track, muted users and allowed PM senders, the search index; the response's user as `GET /u/:username.json` serializes it, now with the bio excerpt; measured against Rails. Refused: user and custom fields, external ids, backgrounds, the notification schedule, titles, primary and flair groups, date of birth, tag tracking, sidebar links, user status, themes and array options, uploads in the bio, watched words, failing validations (the errors JSON) |
| `POST /uploads.json` | `uploads_controller.rb#create`, `UploadCreator`, `UploadValidator`, `FileStore::LocalStore`, `UploadSerializer`, `ImageSizer`, `Upload#calculate_dominant_color!` | attachments and GIFs, which Rails stores as sent: the authorized extensions (staff ones too) and size limits, the duplicate check by SHA1 and the user's upload link, the local store path, GIF size, animation and thumbnail size, the dominant colour (with ImageMagick 7's `magick`, as Rails runs it); measured against Rails, stored files included. Refused: PNG, JPEG, WebP and the other images Rails rewrites (image_optim, re-encoding, HEIF conversion, SVG cleaning, downsizing), avatars and cropped types, site setting uploads, uploads from a url, secure uploads, S3; the upload rate limit is not ported. Only `.json` is routed (`/uploads` is the static file tree) |
| `POST /admin/email/handle_mail(.json)`, Jobs::ProcessEmail | `admin/email_controller.rb#handle_mail`, `Email::Processor`, `Email::Receiver`, `Email::Cleaner`, `EmailReplyTrimmer`, `PostCreator` | a reply by a known, active user to a reply key, plain text or HTML (HtmlToMarkdown, with the per-client extracters for Gmail, Outlook, Word, Exchange, Apple Mail, Thunderbird, ProtonMail, Zimbra, Newton and Front): the incoming_emails row (the raw as the mail gem re-serializes it), the body with Discourse's markers cut and the quote trimmed, the post by email (its date, raw email and outbound Message-ID) and its jobs; measured against Rails. Rejections email the sender as Email::Processor does (once a day per address and kind) and keep the message on the incoming email. Refused: staged users, attachments, bounces, forwarded emails, likes and notification levels by email, replies to messages, email_in addresses, POP3 polling |
| `GET /session/hp(.json)`, `POST /u(.json)`, `PUT /u/activate-account/:token(.json)`, `POST /u/email-login(.json)`, `POST /session/forgot_password(.json)`, `POST /session/password-reset-code/verify(.json)`, `PUT /u/password-reset/:token(.json)` | `session_controller.rb#get_honeypot_value`, `#forgot_password`, `users_controller.rb#create`, `#perform_account_activation`, `#email_login`, `#password_reset_update`, `UserActivator`, `EmailToken`, `EmailLoginCode`, `UsernameValidator`, `UserPasswordValidator`, `SpamHandler`, `UserNotifications` (signup, email_login, forgot_password, set_password, the login and reset codes) | local signup with the honeypot and challenge, its rows (user, email, password, stats, options, profile, avatar row, trust level group, default category and sidebar preferences, search index) and the activation email; activation and login; email login links; password reset by code or link; every table write and email measured against Rails. Refused: invite codes, user fields, staged users, approval, OAuth, random avatars, screened emails, unicode usernames, validation errors other than the username's, second factors on reset, welcome messages |

Supporting ports: SiteSetting (YAML defaults, `locale_default`, typed DB
rows, GlobalSetting shadowing, upcoming-change promotion), server-side I18n
(en), UrlHelper/GlobalPath URL generation, SiteIconManager,
ColorScheme (hex_for_name, ColorSchemeSerializer with ColorMath), the
anonymous Guardian, FlagSerializer, sidebar sections, user themes,
Site#categories (plain-paragraph descriptions only), TopicQuery (latest),
TopicPostersSummary, avatar templates (letter avatar colors, uploaded,
system), Emoji unicode lookup from the discourse-emojis gem, TopicView
(paged and near-post chunks, timeline lookup, participants, flags summary).

Behaviors the port hits but hasn't implemented return an explicit 500
(`Unsupported`) rather than a guess: watched words, computed fancy titles
(HtmlPrettify), post link counts, hidden posts, topic timers, thumbnails,
user fields, enabled auth providers, user-selectable color schemes, group
flair uploads, S3 CDN.

Not yet: plugin registries, themeable settings, response
headers.

## Cooking

Rails cooks a post by running Discourse's JavaScript markdown bundle in
V8. discourse-rs does not vendor that bundle or embed a JavaScript engine:
`pretty_text::cook` (`PrettyText.cook`: `markdown`, then `cleanup`) is
Discourse's markdown features ported as Rust
rules on the markdown-it crate, measured against Rails.

- `pretty_text::render`: the rules (anchors, code, tables, typographer,
  mentions, hashtags, emoji, linkify and onebox marking, bbcode, quotes,
  uploads and images, and the bundled plugins' poll, details, spoiler,
  checklist and footnotes) and the sanitizer (pretty-text's allow list
  on a port of the xss library's tag scanner).
- `pretty_text::options` and `pretty_text::helpers`: the options Rails
  hands the renderer, and `PrettyText::Helpers`, the lookups made while
  cooking. A cook renders once to learn what it refers to (quoted users
  and topics, hashtags, uploads), resolves that from Postgres, then
  renders again with the answers.
- `make record-pretty-text AGENT=rs-parity` records, from Rails on the
  reference: those options, every helper call with its result, and
  `PrettyText.markdown`'s HTML for a corpus of 51 feature samples and
  the seeded posts, each also through `PrettyText.cook`.
  `tests/pretty_text.rs` requires the options, the
  helpers and the cooked HTML to match; 79 of 80 entries do.
- `pretty_text::cleanup`: `PrettyText.cleanup`, on html5ever: link `rel`
  attributes, mention links for users and groups, hidden direction marks
  in code, video thumbnails, and Nokogiri's HTML5 re-serialization.
- `pretty_text::cooked_post_processor::cooked_column(post_id)`: the
  `cooked` column, `Post#cook` (the post's id, its last editor, nofollow
  by its author's trust) then what the post processor job writes: quotes
  marked missing or modified, local urls made absolute, `u=` taken off
  links to the site, nofollow enforced. Oneboxes, images other than emoji
  and uploaded videos are refused: they need network fetches and image
  processing. The recorder runs Rails' processor on each post in a
  rolled-back transaction to know what it writes.
- A restored backup's posts can be recorded and cooked the same way
  (`scripts/record-pretty-text <agent> <dir>`, then the ignored
  `backup_corpus_matches_rails` test); the Faker backup's 482 posts all
  cook byte-equal and get the column Rails writes.

Refused with an explicit error rather than answered differently from
Rails: custom emoji, the emoji deny list, watched words, secure uploads,
uploads behind a CDN or S3, a hashtag chat would resolve to a channel
the cooking user can see, a local date moment would only read through
the browser's `Date` (a time like `9:00`, or no year), and the block
tags of the plugins not ported. What plugins add to the options
(chat's, discobot's iframe) is not produced.

## Pages

Anonymous readers get server-rendered HTML (askama templates in
`templates/`, one stylesheet in `static/`) built from the same documents the
JSON endpoints return: `/` and `/latest` (the topic list, paged), `/c/...`
(a category's list with its subcategories), `/tag/...` (a tag's list), `/search?q=` (results), `/u/:username` (a profile), and every page carries a canonical link, description and OpenGraph/Twitter meta like the crawler layout (with the non-canonical `noindex` header),
`/tags` (the tag index), `/categories` (the index with featured topics), and `/t/:slug/:id` (the topic with its posts). Requests ending in `.json` get the
API document instead. The structure follows Discourse's crawler views.

`PUBLIC_DIR` (default `public`) is served at `/images` and `/uploads`: point
it at a Discourse `public/` directory or a restored backup's.

## Serving a backup

```bash
scripts/restore-backup site-backup.tar.gz mysite            # -> backups/mysite/
PUBLIC_DIR=backups/mysite DISCOURSE_SRC=~/discourse/discourse \
  DATABASE_URL=postgresql://localhost:5442/mysite?host=$PGDATA cargo run
```

`restore-backup` loads the backup's `dump.sql.gz` into a fresh database and
unpacks its `uploads/` where `PUBLIC_DIR` serves them. `DISCOURSE_SRC` (a
Discourse checkout with its bundle installed) supplies the stock images and
the emoji set a backup doesn't carry.

discourse-rs refuses a database behind the vendored schema. For an older
backup, vendor Discourse at the commit that matches the backup's migration
version (`meta.json`):

```bash
scripts/vendor-discourse ~/discourse/discourse \
  $(scripts/discourse-commit-for-migration ~/discourse/discourse <version>)
```

## Deploying

One host, systemd, Postgres 16 with pgvector, Caddy in front. Every
release has a static x86_64 Linux binary attached, and its tarball carries
`deploy/`, so the host needs no toolchain:

```bash
tar xzf discourse-rs-v<version>-x86_64-linux.tar.gz
cd discourse-rs-v<version>-x86_64-linux
sudo deploy/install        # from a checkout: make build && sudo make install
```

That installs `/usr/local/bin/discourse-rs`, `discourse-rs.service`, the
`discourse-rs` account (systemd-sysusers) and `/etc/discourse-rs/env` from
`deploy/env.example`. Running it again upgrades the binary and keeps the
env file. Then:

1. Restore the backup (`scripts/restore-backup`, above) and put its
   `uploads/` under `/var/lib/discourse-rs/public`.
2. Give the service a database role. It reads everything and writes
   sessions, visits, search logs and sitemaps:
   `createuser discourse-rs`, then
   `GRANT pg_read_all_data, pg_write_all_data TO "discourse-rs"`.
3. Fill in `/etc/discourse-rs/env`: `DATABASE_URL`, `DISCOURSE_HOSTNAME`
   and `DISCOURSE_SECRET_KEY_BASE`. The secret must be the Rails site's
   for its sessions to survive a cutover; Rails keeps it in redis when it
   is not configured (`rails runner 'puts GlobalSetting.safe_secret_key_base'`).
4. `systemctl enable --now discourse-rs`, and add `deploy/Caddyfile` to
   Caddy's config with the real hostname.

The server binds to loopback and trusts `X-Forwarded-For`, so only the
proxy may reach it. `systemctl stop` (SIGTERM) lets in-flight requests
finish. A release binary embeds the schema vendored at its tag; a backup
older than that needs a build from a checkout vendored at the backup's
commit.
