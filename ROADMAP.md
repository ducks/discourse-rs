# Roadmap

Where discourse-rs is going and in what order. The goal is full parity:
every route Rails serves, admin included, answering as Rails answers.
Each slice is a branch merged `--no-ff` to main, measured against Rails
with the parity harness (see README: Parity). Update this file in the same
commit as the slice.

The port refuses rather than guesses: a request that needs a behavior not
ported yet gets an explicit `Unsupported(...)` 500 naming it. The list of
those markers (`grep -rho 'Unsupported("[^"]*")' src/`) is the fine-grained
backlog; the milestones below are the coarse one. Phase 9 groups the
markers into branches and phase 10 holds the rest of the milestones'
work, so the two together are the count of what is left.

## State as of 2026-10-20

On main at 16353d29. Open: 125 items, 42 S, 2 S/M, 63 M, 18 L (phase 8:
1, item 90; phase 9: 63; phase 10: 16, 257 and 258 found by the demo
smoke test; phase 11: 38; phase 12: 7, new in this sweep). Open inside them: 90g7a, 164b-164h, 188i3-188i5, 194c-194g, 228a-228c, 258a, 260a, 261b, 262a, 263a, 264a, 265a, 266a, 267a, 268a, 269a, 270b, 271a, 272a, 273a, 274a, 276a, 277a, 280a. Phases
9 to 12 together hold everything the markers name.

Markers: `grep -rho 'Unsupported("[^"]*")' src/` counts 1704 sites (1427
distinct reasons); 166 more put the literal on the line after
`Unsupported(`, which that grep misses, so 1870 in all. Six `not_yet`
write cases are left (145, 150, 152, 198, 252). By area, the grep's
count first and all of them second where they differ, with the items
that own them:

- Ruby error answers, rows gone, params of another shape, missing
  translations: 634 (719): 187
- Moderation (flags, review, deletion, penalties): 101 (106): 147-152, 194
- Accounts, signup, login and invites: 92 (104): 153, 154, 250, 251,
  253, 255, 261b
- Posting, editing and topic edits: 90 (95): 132, 134-138, 188
- Topic view and lists: 82 (86): 120, 124-129, 158, 159, 195, 197
- Ruby engines (regexps, URLs, charsets, YAML, sanitizers, email CSS):
  85 (95): 174-179
- Email (alerts, notification email, reply by email, group SMTP, push):
  85 (87): 145, 146, 165, 166, 186
- Anonymous mode: 72: 133, 211
- Users, groups, categories and tags: 64 (68): 130, 156, 157, 184, 252
- Chat: 63 (66): 160-162, 215, 216, 218, 224, 227
- Site settings and themes: 62 (78): 164, 169b, 170, 171, 173
- Uploads, media, CDN, S3: 62 (66): 121, 123, 139-141, 256
- Locales and content localization: 60 (64): 167, 168
- Badges, about, drafts, messages: 41 (42): 180-183
- Oneboxes: 38 (40): 142
- Admin pages and small leftovers: 28 (34): 90, 254
- Plugins and client leftovers: 28 (29): 202, 163, 196, 230, 233, 237,
  245, 248
- Search settings: 11 (12): 144; crawlers and MaxMind: 6: 185

Every marker has an open item but one, restoring a backup that needs
migrations (Not planned).

## Before the demo

The demo is the core forum, chat and admin; polls are low priority. Its
bar is tests/new_member_smoke.rs and tests/demo_smoke.rs green with no
ignored step (below), and the screenshot sweeps (scripts/parity-screenshots)
showing nothing a scripted walk-through would land on. This list is what is
left between here and there; check items off as their branches merge.

Blockers, a scripted walk-through would hit them:

- [x] The open composer doesn't set `html.composer-open`, so a page can't
  scroll clear of it (/new-topic 27.8% off): fix/composer-open-padding
  (276), done: 0.47%, the rest is the rich editor (196).
- [x] /new and /unread draw their Dismiss rows empty (270a): fix/list-dismiss,
  done; Ember draws no Dismiss New modal (no caller upstream).
- [x] /admin/badges: core's `.content-list` rules are missing, so the badge
  list is unstyled (4% desktop, 8-11% mobile), and a system badge's
  disabled select draws enabled: fix/content-list-css (277).
- [x] /admin/customize/themes: the seed's Foundation and Horizon screenshot
  uploads 404 on the port (their files exist only on the reference) and
  the design wizard card is unstyled: fix/theme-screenshot-files (278),
  done: 1.37% to 0.04 desktop, 5.81% to 0.07 mobile.
- [x] /password-reset over the homepage when login isn't required answers
  500: fix/password-reset-homepage (261a), done.
- [x] Chat: /chat/direct-messages and /chat/threads answer 405 with a blank
  page (99's leftovers), and a DM channel drops its "last visit" line when
  the newest message is the marker (plugins/chat/page.rs:185):
  fix/chat-dm-thread-pages (280), done.
- [x] Chat: the chat drawer (223): feat/chat-drawer (223a), done: the
  header icon, sidebar links, lists, browse and a channel in the drawer,
  kept across pages; resizing and the drawer's other routes are 223b-d.
- [x] The demo servers on 8080 and 8092 ran builds from 2026-10-19 and
  2026-10-20, before every item from 260 on, and were gone after a reboot:
  restarted on 1007bc82 with the large corpus (8080) and the Meta baseline
  (8092). docs/demo-servers.md has how to load and run them.
- [x] The demo servers ran with DISCOURSE_RS_JOBS=off, so a post was never
  processed (no lightbox, optimized images or video thumbnail) and replies
  and mentions notified nobody: the worker is on since 2026-10-23
  (fix/demo-jobs-on). With it on, a new topic whose first post has an
  image answered 500 once process_post set its image_upload_id (125):
  feat/topic-view-thumbnails, done. The demo units also lacked
  DISCOURSE_SRC (a broken logo, category icons and emoji) and a stable
  DISCOURSE_SECRET_KEY_BASE (a restart logged everyone out):
  fix/demo-discourse-src, done. Checked on both servers at e8d68487 by
  posting a photo, an MP4, a WebM and an MP3 in a new topic: the topic
  page draws, the lightbox opens, the video placeholder plays (the WebM
  in Playwright's Chromium, which has no H.264), the audio loads, the
  optimized images and og:image are there, and a mention notifies.
- [x] A new member's post held for approval (approve_post_count,
  approve_unless_trust_level, a watched word's require_approval) was
  refused on /review for admins and moderators: the queue serialized
  flagged posts only (75's leftover). feat/review-queued-posts, done:
  /review.json serializes queued posts (ReviewableQueuedPostSerializer:
  the payload, the raw cooked, the editable fields, the bundled actions
  per viewer) and reviewed posts (review_every_post), with the score
  reasons; the page draws the queued post's card as Ember does, and
  Approve, Reject, Request Revision (its modal) and Delete user work from
  it. Recorded on rs-ref-d (parity/writes/review: queued_list*,
  queued_approve_then_list*); the card and the modal compared with
  Playwright. Still refused: user reviewables (must_approve_users, 148),
  editing a queued post and the silence and suspend client actions (194e)
- [x] The admin's site settings answered 500 saving a list setting such as
  authorized_extensions (164), so an admin couldn't allow video or audio
  uploads; the demo sites have them set in the database
  (docs/demo-servers.md): feat/list-settings (164a), done: every list
  type saves with Rails' validations, and the page's list editors open
  and save them.

Visible if clicked, avoid them in a scripted demo:

- [x] Polls draw a "0 voters" stub and can't be voted on (228):
  feat/poll-ui-vote (228a), done: the widget votes, changes and removes
  votes; its results say they are not ported yet (228b), as do ranked
  choice polls and the actions menu (228b, 228c).
- [ ] No Build Poll (229).
- [x] A post with an uploaded video or audio file is refused (138, 141):
  feat/post-video-audio, done: such posts are created and edited, the
  video drawn as its placeholder with the play button and the audio as
  its player; the composer doesn't make a video's thumbnail yet (139).
- [ ] ~~Checklists can't be checked (242)~~ (done: feat/checklist-toggle,
  boxes toggle with their batching, retries and optimistic drawing); user
  status can't be set (130).
- [x] The review queue's claims and notes are refused (194), and the topic
  admin wrench isn't drawn (194, 262a): the wrench and the post admin
  menu, feat/topic-admin-menus (194a), and claims and notes,
  feat/review-claims-notes (194b), done; the menus' unported actions are
  refused (194c, 194d).
- [ ] The review queue's client actions (edit, suspend, silence), its
  reject reason modal and editing a reviewable are refused (194e); the
  revise modal is done (feat/review-queued-posts).
- [ ] The composer's rich editor and its toolbar buttons (196); the
  profile's About me editor follows it.
- [x] The add participant modal on a message (262a): feat/add-pm-participants
  (262b), done.
- [ ] The grouped bookmarks menu (274a).

Polish, from the second screenshot sweep:

- [ ] The topic footer's notifications button sometimes draws as a plain
  "Watching" box without its bell or caret (seen in both sweeps):
  fix/footer-notifications-flake narrowed it, cause not found. It shows
  only in a full-page (beyond the viewport) Chromium capture taken with
  the sweep's `transition: none !important` rule on every element, on
  short topics whose footer is in the first screen (/t/41 as admin
  30-60% of captures, /t/90 as user1 less; never /t/35, /t/91 or
  mobile). Viewport and element screenshots, resizes and real use always
  draw the icons. The markup, matched rules, computed styles and boxes
  equal rs-ref-b's, and rs-ref-b never drops them (0/92). It goes away
  with the icons `position: static`, with `#topic-footer-buttons`
  positioned, without the title's voting widget, or with the reference's
  stylesheets loaded in place of (or beside) ours; it doesn't follow our
  resize handlers, the body DOM (the reference's sidebar, header and
  main outlet swapped in still fail) or the footer's markup.
  Not a demo blocker: real use and 98 normal loads (cold, warm, boosted,
  desktop and mobile, anon, member, admin) never show it. The reference's
  stylesheets make it go away, so spike/core-css-compile may remove it.
- [x] Admin category settings draw `default_composer_category` as a plain
  combo box instead of CategoryChooser (2.41%): fix/category-setting-chooser.
  The header holds the category's badge; 0.02% on rs-ref-b.
- [x] Admin font previews all render in Inter: fix/admin-font-previews.
  Admins get the wizard stylesheet's fonts and /fonts serves the gem's
  others from DISCOURSE_SRC; 0.04% on rs-ref-b (was 0.53%).
- [x] The admin email style Ace editor height and template preview offset:
  fix/admin-email-editors. ace-editor.scss's wrapper border and the cooked
  rules' .d-editor-preview selectors; email style 0.05% (was 2.61%), the
  signup template 0.03% (was 1.63%) on rs-ref-b.
- [x] scripts/parity-screenshots: guard against a 164598px capture
  (PIL's DecompressionBombError) and let scripts/ui-compare log in:
  chore/screens-tool-guards. `--max-height` (30000px) clips the capture and
  a failed diff is the page's error; `ui-compare --as USER`. The 164598px
  page is the reference's development routing error page.

Done:

- [x] The login page and sign-in flow: Ember's full-page login, signup,
  password reset and account-created pages (261), desktop and mobile.
  scripts/check-login and scripts/check-signup drive them in a real browser
  (blank and refused logins, the password mask, a network failure, the forgot
  password modal, signing in from a topic back to it; the signup validations,
  a network failure, an account created through to account-created), with
  screenshots; the password reset by its emailed code through to the new
  password and the login was checked the same way (261's commit).

### Demo readiness

tests/demo_smoke.rs walks the demo over HTTP, one test per step (50,
50 passing). Each step below fails today and is ignored with the item
that fixes it; that item's branch removes the ignore. Run them all with
`cargo nextest run --test demo_smoke --run-ignored all`.

Core forum:

- [x] A reply linking another topic made post_alert refuse, so its
  mentions notified nobody (a_reply_linking_a_topic_notifies): 145
- [x] pull_hotlinked_images and crawl_topic_link are refused after every
  post with an image or a link (posting_runs_its_background_jobs): 140,
  127
- [x] The interface preferences page is a 500 once a palette is user
  selectable (offers_light_and_dark_palettes): 169
- [x] No dark mode: pages linked only the light color definitions
  (switches_to_dark_mode): 258

Chat: every step passes (chat page, a channel, sending, another member
receiving it, a DM and a reaction).

Admin:

- [x] The review queue's page /review (admin_review_queue_page); its
  claims, notes and client actions are refused: 194
- [x] Silencing a member: the system message notifies them and the
  account_silenced email is sent (a_silenced_member_is_told): 146

## Now: the parity queue (2026-10-13)

The next branches, in order. Each has a brief with the Rails reading
done (bf55a44: files, exact behavior, what the port reuses, fixture
gotchas, the case plan) in the working notes,
`~/agents/notes/discourse-rs/spec-<brief>-2026-10-13.md`; the brief is
named in brackets. Sizes: S one module and a few cases, M several
modules, L a cross-cutting change or several branches in one. Cross a
branch off here in the commit that merges it.

Working a branch: read the brief's fixture notes before writing cases
(setting units and defaults, PM or regular topics, rows the reference
already has, timestamps near a window's edge); start the next branch's
reading while the gate runs. The seed's rows date from 2026-07-25 to
2026-10-01: a case whose feature looks back over a window (digests'
month, "recent", days visited) inserts or updates its rows with
`now() - interval ...` in setup rather than relying on seed dates.

### Phase 0: fix first

Bugs or silent gaps in what is merged, small and independent.

1. ~~`fix/remote-ip`~~ (done) S: both `remote_ip`s (session/current.rs,
   routes/search.rs) trust the first X-Forwarded-For hop, which a client
   sets; take the address the trusted proxy saw, as ActionDispatch's
   RemoteIp does, in one shared function [rate-limiting]
2. ~~`fix/topic-timers-timerable-id`~~ (done) S: five queries read
   `topic_timers.topic_id`, which Rails no longer writes
   (`timerable_id`); timers are never found on real data [badges-timers]
3. ~~`fix/poll-guard`~~ (done) S: a `[poll]` post cooks and saves without its
   `polls` rows; refuse until poll-create [messages-dms]
4. ~~`fix/search-recent-offset`~~ (done) S: search refuses every query once
   `search_recent_regular_posts_offset_post_id` is set (any site past
   the offset size); port Search#aggregate_post_sql's `min_id`
   [scheduled-jobs-rest]
5. ~~`feat/chat-dm-reads`~~ (done) L: a DM channel row (restored backups have
   them) makes chat's channel list refuse, failing every page of that
   member; the DM reads, serializers and drawer list [messages-dms]
5b. ~~`feat/chat-dm-page`~~ (done) M: the full-page DM channel (header, title,
   avatars, messages and composer for a DM), refused today once the
   sidebar links to it [messages-dms]
6. ~~`feat/harness-fixture-dates`~~ (not needed): replay shifts each
   case's clocks to when Rails recorded it (tests/writes.rs), so
   recordings do not age with the seed; new windowed cases date their
   setup rows from now() instead (see "Working a branch")

### Phase 1: the scheduled jobs left (ready)

7. ~~`feat/jobs-heat-settings`~~ (done) S: SiteSetting set-and-log (a row even for
   a default, unchanged values skipped, /client_settings published),
   then update_heat_settings
8. ~~`feat/jobs-search-index`~~ (done) M: reindex_search,
   create_recent_post_search_indexes
9. ~~`feat/jobs-scores`~~ (done) S: ScoreCalculator, calculate_scores (weights as
   SQL numerics, not bound floats)
10. ~~`feat/jobs-periodical-updates`~~ (done) M: with feature_topics_for's early
    return (it rewrites rows every run today); rebakes refused while
    stale posts exist
11. ~~`feat/post-rebake`~~ (done) M/L
12. ~~`feat/jobs-consistency-counters`~~ (done) M, 13. ~~`feat/jobs-consistency-users`~~
    (done, one branch) M: ensure_db_consistency, refusals checked before any write
14. ~~`feat/harness-upload-files`~~ (done) S, 15. ~~`feat/jobs-upload-metadata`~~ (done) S,
    16. ~~`feat/jobs-clean-up-uploads`~~ (done) M (Upload#destroy; cases never
    destroy fixture uploads)

[scheduled-jobs-rest]

### Phase 2: rate limiting

17. ~~`feat/rate-limiter-core`~~ (done) M: a Postgres limiter in discourse_rs, the
    429/422 shapes and headers, off for existing cases; the recorder's
    per-case `"rate_limits": true` (Rails dev enforces them; the recorder
    disables them today)
18. ~~`feat/rate-limit-posting`~~ (done) L, 19. ~~`feat/rate-limit-post-actions`~~ (done) M,
    20. ~~`feat/rate-limit-auth`~~ (done) M, 21. ~~`feat/rate-limit-endpoints`~~ (done) S
22. ~~`feat/request-tracker-limits`~~ (done) M: the global per-IP limits, tested
    in Rust only (not loaded in Rails dev, loopback exempt); needs
    fix/remote-ip

[rate-limiting]

### Phase 3: messages

23. ~~`feat/user-comm-screener`~~ (done) S: gates PM creation, alerts, invites, DMs
24. ~~`feat/pm-tracking-state`~~ (done) S
25. ~~`feat/group-messages-create`~~ (done) M, 26. ~~`feat/group-messages-alerts`~~ (done) M
27. ~~`feat/jobs-group-messages`~~ (done) S: unblocks ignored_users_summary and
    pending_reviewables_reminder
28. ~~`feat/pm-archive`~~ (done) S, 29. ~~`feat/pm-small-actions`~~ (done) M,
    30. ~~`feat/pm-likes`~~ (done) S, 31. ~~`feat/pm-destroy`~~ (done) M,
    32. ~~`feat/pm-invite`~~ (done) M, 33. ~~`feat/pm-invite-group`~~ (done) S
33b. ~~`feat/messages-pages`~~ (done) M: the HTML of /u/:username/messages
    (inbox, sent, new, unread, archive, warnings, group inboxes), the
    inboxes dropdown and counts from PrivateMessageTopicTrackingState.report
    (also the sidebar's My messages dot), Ember's exception page for a
    list's 403. Left: the messages' tags pages (refused), changing a
    group's notification level (the trigger only), the dismiss and bulk
    select buttons' actions
33c. ~~`feat/group-message-topic`~~ (done) S: a group message's topic page
    refused its suggested messages (messages_for_groups_or_user), a 500;
    reading it refused TopicGroup.update_last_read
33d. ~~`perf/rate-limiter-storage`~~ (done) S/M: every public request performs two
    limiter statements through a 4-connection pool (BENCH.md,
    2026-10-13); keep the lists in the process, or size the pool

[messages-dms]

### Phase 4: chat DMs and polls

34. ~~`feat/chat-dm-create`~~ (done) M, 35. ~~`feat/chat-dm-messages`~~ (done) M (mention
    warnings fold in), 36. ~~`feat/chat-dm-groups`~~ (done) M,
    36b. ~~`feat/chat-chatables`~~ (done) M (GET /chat/api/chatables, the new
    message modal's search; category channel edits with it),
    37. ~~`feat/chat-dm-summary-email`~~ (done) S
38. ~~`feat/poll-create`~~ (done) M,
    39. ~~`feat/poll-vote`~~ (done) M (vote, remove, toggle_status,
    voters, grouped results, ClosePoll, and the serializer's vote-dependent
    keys; the `vote_poll` anonymous action waits on core's
    AnonymousAction, `/anonymous-actions`, not ported), 40.
    ~~`feat/poll-edit`~~ (done) S/M

[messages-dms]

### Phase 5: the client's infrastructure

41. ~~`feat/cooked-css-basics`~~ (done) S (also the composer textarea's
    line-height: foundation.css ports one of discourse.scss's two
    textarea blocks), 42. ~~`feat/hashtag-css`~~ (done) S/M
43. ~~`feat/onebox-local`~~ (done) M: the site's own topics, users,
    categories and uploads, inline topic titles, the onebox limit, the
    FirstOnebox and FirstEmoji badges (BadgeGranter.grant), QuotedPost
    after post processing, onebox.scss. Still refused until onebox-fetch:
    other sites' oneboxes and inline titles, own-site pages no route
    recognizes, chat's local oneboxes. Images of the site's own that
    aren't uploads (letter avatars) are treated as unreachable for
    FastImage, as the reference's container finds them; production Rails
    would size them
44. ~~`feat/cooked-decorators-js`~~ (done) M: run once per post, swapped-in posts
    included
45. ~~`feat/float-positioner`~~ (done) S, 46. ~~`feat/user-card`~~ (done) M
47. ~~`feat/lightbox`~~ (done) M (PhotoSwipe), 48. ~~`feat/highlight-js`~~ (done) M (posts, the
    composer's preview and chat messages; the rich editor's code blocks
    wait on the rich editor),
    49. ~~`feat/local-dates`~~ (done) M (moment and moment-timezone
    vendored, as Discourse renders: posts, the composer's preview and chat
    messages, the tooltip, "Add to calendar" with its modal and .ics; the
    `local_dates` custom field; emails show data-email-preview. Left: the
    composer's insert-date modal and Shift+. shortcut, moment locales
    other than English), 50. ~~`feat/inline-footnotes`~~ (done) S
51. ~~`feat/onebox-endpoints`~~ (done) S, 52. ~~`feat/onebox-fetch`~~ (done) L
    (FinalDestination: SSRF guard with DNS pinning, redirects, the https
    domain cache; RetrieveTitle; other sites' inline titles. Pages in
    charsets other than UTF-8 are refused; block oneboxes of other sites
    wait on the engines), 53. ~~`feat/onebox-engines-nofetch`~~ (done) M
    (FinalDestination#resolve, the matcher with all 69 engines, the
    media, Twitch, Youku, Typeform, Google Meet and Calendar engines,
    Preview with the Sanitize port, external_onebox with its caches,
    strategies and error previews; engines that fetch stay refused),
    54. ~~`feat/onebox-generic`~~ (done) L (AllowlistedGenericOnebox over
    StandardEmbed: OpenGraph, Twitter, oEmbed, JSON-LD movies, canonical
    links, favicons, the fragment's section, Date._parse and Time.parse
    for published times, the missing-data error previews; measured with
    fixture pages through the stubbed fetch boundary. Refused: pages that
    aren't UTF-8, dates Time.parse would read against today or roll over
    with a zone, canonical links Addressable would normalize past ASCII),
    55. ~~`feat/onebox-engines-api`~~ (done) L (every engine: GitHub
    and GitLab over the GitHub API client, the StandardEmbed media
    engines, the JSON and HTML ones, X/Twitter without API credentials,
    Wikipedia, Amazon, PubMed's XML, Google Maps' redirects;
    discourse-lazy-videos' YouTube, Vimeo and TikTok overrides; inline
    titles from Google Meet and GitHub with tokens. Refused: X posts
    through the Twitter API, inline pull requests with discourse-github's
    status, and the dates, URLs and values noted in the engines)

[client-infra]

### Phase 6: badges and topic timers

56. ~~`feat/badge-grant`~~ (done) M, 57. ~~`feat/badge-queue`~~ (done) M (Rails' queue is
    Redis: the harness cannot see a missed enqueue, so each enqueue
    site gets a backlog case), 58. ~~`feat/badge-jobs`~~ (done) M,
    59. ~~`feat/badges-read`~~ (done) M
60. ~~`feat/topic-timers`~~ (done) L, 61. ~~`feat/topic-timer-jobs`~~ (done) M,
    62. ~~`feat/category-auto-close`~~ (done) M

[badges-timers]

### Phase 7: users, digests, do not disturb

63. ~~`feat/user-destroyer`~~ (done) L (about 60 tables; self delete needs
    pm-destroy and deleting posts with links and likes),
    64. ~~`feat/jobs-user-cleanup`~~ (done) S, 65. ~~`feat/review-delete-user`~~ (done) S/M
66. ~~`refactor/notification-insert`~~ (done) S: one path for the ten
    places that insert notifications, which shelving hooks into
67. ~~`feat/dnd-shelving`~~ (done) M, 68. ~~`feat/dnd-schedules`~~ (done) M
69. ~~`feat/email-unsubscribe`~~ (done) M, 70. ~~`feat/digest-email`~~ (done) L,
    71. ~~`feat/jobs-digest`~~ (done) S, 72. ~~`feat/admin-email-digest`~~ (done) S
73. ~~`feat/about-stats`~~ (done) S/M (about.json with Stat.api_stats and
    their cache, the about_stats job, live_post_counts. The about page's
    HTML: ~~`feat/about-page`~~ (done). Left: category moderators, banner
    image, localized settings and visitor stats are refused; the extra
    groups are not drawn), 74. ~~`feat/directory`~~ (done) M (/directory_items with
    its filters, orders and pages, DirectoryItem.refresh_period! with the
    solved and gamification column queries, the directory_refresh job.
    The users page's HTML: ~~`feat/users-page`~~ (done). Left: searching
    by name (UserSearch), user field and plugin columns are refused; the
    Edit Directory Columns modal)
75. ~~`feat/review-reject-queued-post`~~ (done) M (queued posts:
    approve, reject and its aliases, revise and reject, delete, delete
    user (as a spammer, account_deleted email); reviewed posts: approve,
    restore, unhide, reject, keep deleted, delete; AutoQueueHandler's
    rejections. Left: approving into a closed topic or with tags, an
    email or plugin fields, unsilencing the author, unhiding a first post;
    the review list's queued and reviewed posts landed with
    feat/review-queued-posts),
    76. ~~`feat/user-destroyer-delete-posts`~~ (done) M (agreeing as spammer
    with queued and reviewed posts landed with 75; block_urls on onebox
    engines)
77. ~~`feat/username-changer`~~ (done) L (non-ASCII usernames and a
    category description a rewritten definition would change are
    refused), 78. ~~`feat/user-anonymizer`~~ (done) M (cases normalize
    the random `anon<n>` name; verbose token logging and non-address IPs
    refused),
    79. ~~`feat/user-merger`~~ (done) L (a source user with posts is
    refused until 79b)
79b. ~~`feat/post-owner-changer`~~ (done) M: PostOwnerChanger (each post
    revised to its new owner by PostRevisor's owner change, the new
    owner's like destroyed, topic users, statistics and last poster),
    merging a user with posts and POST /t/:id/change-owner (a first post
    without its author, category definition posts, a raw TextCleaner
    would change, staff_edit_locks_post, review_every_post for
    non-staff editors, slow mode for non-staff and the og image
    regeneration on a like count change refused)

[users-digests-dnd]

### Phase 8: what the port still refuses, then admin

Ordered by how many refusals each lifts (cooking and posting first),
then by what running a forum needs (milestone 6). Briefs:
`spec-<brief>-2026-10-14.md`.

79c. ~~`feat/readonly-mode`~~ (done) M: the read-only keys (user,
    staff-writes-only, restore, Postgres failover and forced) and
    `pg_force_readonly_mode`; every write route refused with the 503
    (JSON or the no_ember page) after the CSRF check and before the login
    checks, by the admin routes' constraints first, staff passing in
    staff-writes-only mode and the logins refusing others themselves;
    `Discourse-Readonly` on the rest; `PUT /admin/backups/readonly` with
    its staff log and `/site/read-only`; the pages' notice; no session
    refresh, cleared notifications, fancy title write, search log or
    scheduled job while read only, no last seen in failover; SQLSTATE
    25006 answered as read only. Left: the client's login, signup and
    logout dialogs and the notice following `/site/read-only` live
79d. ~~`feat/permalinks`~~ (done) M: the `*url` route under
    PermalinkConstraint, the not-found lookups (topics#show missing or
    deleted, list and tags set_category, tag not found,
    `/category/*path`), `/permalink-check` and the admin CRUD under
    /admin/permalinks and /admin/config/permalinks; topics#show's
    InvalidAccess rescue now runs Notification.remove_for. Refused:
    `permalink_normalizations` (Ruby regexes) and urls
    Addressable::URI.encode would change
80. ~~`feat/watched-words`~~ (done) L (the tag action, the CSV upload
    and words Ruby case-folds to several characters stay refused),
    80b. ~~`feat/watched-words-regex`~~ (done) M (watched_words_regular_expressions:
    admin regexps run as Ruby and as JavaScript; regexps outside the
    grammar the engines share are refused: backreferences, named groups,
    inline flags, atomic groups, possessive or nested quantifiers, class
    sets and POSIX brackets, variable lookbehinds, Ruby-only or JS-only
    escapes, patterns that can match nothing),
    81. ~~`feat/custom-emoji`~~ (done) M (admin index, create, destroy,
    /site/emoji, the rebake job; custom emoji and the deny list in
    cooking, emojiUnescape, codes_to_img, unicode titles and every emoji
    url; static images refused until the crop: 81b), 81b.
    ~~`feat/custom-emoji-crop`~~ (done) S (the 100x100> downsize of
    static custom emoji uploads, admin asset types kept in their format;
    a resampled image's dominant colour can be one off ImageMagick's),
    81c. ~~`feat/custom-emoji-import-export`~~ (done) S/M (zip and
    CSV export, import preview and confirm; the preview's rows kept in
    discourse_rs.cached_values; a file that isn't a zip and a manifest
    that isn't UTF-8 refused)
82. ~~`feat/admin-groups`~~ (done) L: creating, editing, deleting groups, members
    and owners in bulk, automatic membership by email domain. Split:
    82a. ~~`feat/groups-read`~~ (done) M: groups#index, search,
    check_name, show, members (and requesters), histories, mentionable,
    messageable and permissions as JSON; /groups and a group's members
    page (/g/:name) as HTML (permissions with categories refused until
    CategorySerializer; the activity, requests, messages and manage tabs
    refused until their routes),
    82b. ~~`feat/group-manager-bulk`~~ (done) M (GroupManager over many
    users, notification defaults, category publishes, next_best_title,
    chat auto join and leave on membership changes,
    bulk_grant_trust_level; a /categories update with categories the
    member may now see refused until CategorySerializer; the automatic
    groups' refreshes in promotion.rs, signup.rs and roles.rs still
    refuse titled, flaired, granting or defaulted groups),
    82c. ~~`feat/admin-groups`~~ (done) M (create, update, destroy,
    members and owners, primary, automatic_membership_count, their logs
    and jobs; refused: adding non-users by email (invites), group SMTP,
    flair uploads, rewriting a renamed group's mentions, an editor who
    can no longer see the group (the redirect), the admin's group forms
    and manage pages),
    82d. ~~`feat/group-email-domains`~~ (done) S
    (automatic_group_membership, set_automatic_groups on confirmation;
    DiscourseConnect external emails refused),
    82e. ~~`feat/group-membership-requests`~~ (done) S (join, leave,
    request_membership, handle_membership_request, set_notifications,
    the request message's topic custom field and requested_group_name;
    requests in locales other than English refused)
83. ~~`feat/web-hooks`~~ (done) L: web hooks, their events from core, deliveries
    and the scheduled jobs, split: 83a. ~~`fix/web-hook-guards`~~ (done)
    S (`web_hooks::active`, Rails' INNER JOIN on the event types and its
    event name translation, at every site; each core event the port
    writes refused while a hook would receive it), 83b.
    ~~`feat/web-hooks-admin`~~ (done) M (/admin/api/web_hooks CRUD with
    its validations, the SSRF lookup and the staff logs, the deprecated
    tag_names creating tags, the events list, bulk, ping, redeliver and
    failed_redeliver; emit_web_hook_event's delivery through
    FinalDestination's checked POST, its headers and signature, retries
    and deactivation; the five scheduled jobs; the admin pages refused),
    83c. ~~`feat/web-hooks-posts`~~ (done) M (post and topic created,
    edited, destroyed, recovered and status updated, small actions and
    email replies, post_liked and notification_created, with
    WebHookPost, WebHookTopicView, WebHookLike and Notification payloads
    of the objects as Rails holds them; PostSerializer's via_email), 83d.
    ~~`feat/web-hooks-users`~~ (done) M (logins and logouts, user
    created, confirmed, approved, updated, suspended, unsuspended,
    promoted, anonymized and destroyed with WebHookUserSerializer or
    UserSerializer and its penalty reasons; group created, updated and
    destroyed and membership through GroupManager, the admin group
    writes and membership requests; badges granted and revoked, tag
    destroyed; reviewables created, scored and transitioned for flagged
    posts and users; solved, topic voting and chat messages; refused:
    queued post reviewables' payloads);
84. ~~`feat/user-api-keys`~~ (done) M (the `User-Api-Key` header: lookup,
    the two limiters, scopes by route, last use and client id writes, no
    CSRF or session refresh; revoke and undo-revoke, the user's own key
    listing, `/user-api-key-client` show and create, the three jobs; push
    refusals narrowed to `push_clients_for`. Refused: a user key with an
    auth cookie or admin key, keys in request parameters, the bookmarks
    calendar feed, a listed key without a client),
    84b. ~~`feat/user-api-key-auth-flow`~~ (done) M (RSA parsing and
    PKCS#1/OAEP payloads, `/user-api-key/new` as JSON and as the port's
    page, `POST /user-api-key` with its redirect or result page, one-time
    passwords and `/session/otp/:token`; the harness decrypts payloads
    with the cases' fixed key. Left: calendar subscriptions, which need
    the bookmarks `.ics` feed), 84c. ~~`feat/user-api-key-device`~~
    (done) M (the device authorization flow: requests, polls, the
    activation page by code or request link, approval tokens, authorize
    and deny, its limiters; Redis's grants, indexes and locks as owned
    rows. Not ported: the debug traces, which only log),
    84d. ~~`feat/admin-api-keys`~~ (done) S/M (`/admin/api/keys` with
    its staff logs, every ApiKeyScope
    mapping matched against Rails' route table recorded from the
    reference, allowed parameters; the master-key badge refusal was dead
    code. Refused: scopes on routes behind home page, permalink or
    markdown constraints, the Ember admin page itself)
85. ~~`feat/admin-badges`~~ (done) M: creating, editing, deleting badges and
    groupings, badge SQL with its preview, the badges pages' HTML,
    85a. ~~`feat/admin-badges`~~ (done) M (Admin::BadgesController's
    JSON: index, types, create/update/destroy with Badge's validations,
    callbacks and staff logs, save_badge_groupings, preview with the
    contract checks, ActiveRecord's named binds and libpq's error text,
    mass award with MassAwardBadge and BulkUserTitleUpdate; refused:
    custom descriptions with markup or entities, CSV files with quotes
    or mixed line endings, error positions past Latin characters, the
    Ember admin pages),
    85b. ~~`feat/badges-pages`~~ (done) M (/badges, /badges/:id/:slug
    with its grants and `?username=`, the /u/:username/badges tab, the
    badge RSS feed and its auto-discovery link, badges.css; under 1%
    desktop and mobile on parity/screens/badges-pages; refused: the
    title picker of a badge the viewer may title themselves with, more
    grants than the first page, emoji in long descriptions,
    /user-badges/:username in a browser, Ember's unknown route),
    85c. ~~`feat/admin-badges-follow-up`~~ (done) S (Ruby CSV's quoting
    and malformed-row line numbers for the mass award, `sanitize_field`
    for badge descriptions and client string settings over the markup
    whose result follows from the markup alone: inline safe-list
    elements, quoted attributes, plain URLs, XML and numeric references;
    the rest refused (block elements, repairs, named HTML entities);
    libpq's error positions landed with 85a),
86. ~~`feat/admin-users-rest`~~ (done) M: granting admin (second factor or email
    confirmation), impersonation, penalties on several users, split:
    86a. ~~`feat/admin-users-penalties`~~ (done) M (other_user_ids,
    the post action: delete, delete_replies, edit without a revision,
    delete_all as Rails' no-op; reviewable_id on the log, the 409 for
    an already suspended or silenced user with AgeWords, suspend-bulk,
    penalty_history, similar-users; refused: a post delete for an
    existing reviewable (notify_deletion)),
    86b. ~~`feat/admin-grant-admin`~~ (done) M (grant_admin through
    AuthManager: the email confirmation with AdminConfirmation's keys,
    its job and mail, /u/confirm-admin JSON and page; the 2FA challenge
    in the server session, GET and POST /session/2fa with TOTP; refused:
    security keys and passkeys (webauthn), the 2FA page's HTML),
    86c. ~~`feat/impersonation`~~ (done) S (POST, GET and DELETE
    /admin/impersonate with their logs; the session's token resolving
    to the puppet while live and still allowed, is_impersonating and
    impersonation_expires_at on the current user, no last seen for the
    puppet; left for later: 3.5's smaller actions
    (disable_second_factor, reset-bounce-score, delete_posts_batch,
    delete_posts_decider, total-others-with-same-ip, approve-bulk))
87. ~~`feat/admin-lists`~~ (done) M: permalinks (79d),
    87a. ~~`fix/user-fields-field-type`~~ (done) S (user fields' type
    from field_type_enum, confirm values, every field for staff and the
    user, multiselect arrays),
    87b. ~~`feat/admin-user-fields`~~ (done) M (/admin/config/user_fields
    CRUD with options, sanitize_field, the required fields version and
    index job; /site.json user_fields; profile updates and signup with
    user fields; searchable fields in the user index; directory
    user_field_ids, ordering by a field, /directory-columns and
    /edit-directory-columns, user field columns on /u; the
    redirect_to_profile_if_required layer. Left: the user profile
    page's public fields in HTML),
    87c. ~~`feat/admin-flags`~~ (done) S (/admin/config/flags create,
    update, destroy, toggle, reorder with their policies and logs;
    flagging with a custom flag),
    87d. ~~`feat/admin-lists-rest`~~ (done) S/M (embeddable hosts and the
    embedding settings with a typed SiteSetting.set, a search term's
    details, incoming email details, form templates admin and public
    list. Refused: a valid form template preview and the public show
    (Psych's YAML.dump), YAML 1.1-only scalars, allowed paths only Ruby's
    regexp syntax accepts)
88. ~~`feat/admin-email`~~ (done) M (the email settings page's JSON:
    index, server-settings, smtp_should_reject; the test email through
    TestMailer with Email::Sender's BYPASS_DISABLE_TYPES; advanced-test
    over Email::Receiver#select_body; the email templates list with its
    titles, plural texts and interpolation keys, show, and revert with
    its change_site_text logs; email style show and update with Dart
    Sass's compressed output (grass) and its error lines, the custom
    template in every mail layout and the custom CSS read as CssParser
    reads it into style attributes. Refused: updating a template and
    anything showing or reverting an existing translation override
    (#90's I18n override layer), templates in other locales, at-rules,
    nesting left after compiling, and selectors past tags, classes, ids
    and the descendant and child combinators in the email CSS, the
    admin's Ember pages; the incoming email details stay with #87; the
    preview and send digest landed with #72)
89. ~~`feat/admin-dashboard`~~ (done) L: reports, problem checks, admin notices,
    the pageview jobs. Split:
    89a. ~~`feat/admin-problem-checks`~~ (done) M (the 35 core checks
    answering about the port itself, their trackers and notices with
    the notice sanitizer, POST /admin/dashboard/problems, the problem
    checks index, ignore and watch, dismissing a notice; run_problem_checks,
    run_problem_check with its retries, notify_admins_of_problems and
    cleanup_problem_check_trackers. Refused: what needs the network
    (a group's SMTP credentials, Twitter's login health), watched words
    with regular expressions, two upcoming changes whose display reads
    themes or categories),
    89b. ~~`feat/admin-reports`~~ (done) L (Report with as_json, report
    keys, read-only slow-query wrapping and the hour's cache, ListQuery,
    show and bulk, the 56 core reports and solved's and reactions'; refused:
    unported plugins' reports, associated accounts with an external
    login, MaxMind locations, remote storage stats),
    89c. ~~`feat/admin-dashboard-sections`~~ (done) M/L (the sections payload
    and the legacy one, general, configuration with its staff log, section
    settings, the mounted reports' listing, layout and bulk fetch, the
    highlights, reports, traffic, engagement, search and system sections,
    the vendored version (call_discourse_hub enqueued, never run),
    warm_dashboard_reports. Refused: data explorer's report source, the
    traffic explorer until 89d; what's new and toggle-feature done in
    90g9; solved's support section done in 163),
    89d. ~~`feat/pageview-tracking`~~ (done) L (the rollup and pageview
    housekeeping jobs; RequestTracker's logging where Rails loads it
    (outside development, or TRACK_REQUESTS): application_requests and
    web_crawler_requests counts flushed each minute, X-Discourse-TrackView,
    X-Runtime and Discourse-Logged-Out, the /srv/pv, /pageview and /srv/se
    beacons with their same-origin check, browser pageview events (queued
    while Postgres is read only, flush_browser_pageview_events), session
    engagement, topic views from browser page views (TopicViewItem.add);
    the site traffic explorer. Refused: CrawlerScorer, MaxMind countries
    and networks (none configured, as without one in Rails), IDNA hosts,
    enable_performance_http_headers' SQL and Redis timings)
90. `feat/themes` L: themes and components, color schemes, site texts
    (translation overrides), sidebar defaults, the /admin/config pages.
    Left (2026-10-21), everything else below merged: the rest of a
    theme's page and its code editor (90g7a); the setup wizard's
    and the admin design wizard's pages (routes/wizard.rs:62,163,247);
    the admin user's badges and TL3 requirements tabs
    (routes/admin_pages.rs:423); login's Discourse ID tab
    (routes/admin_config.rs:776); a search log term's page (90g5); the
    non-XHR requests to JSON-only admin endpoints that Ember would
    answer with its shell (the "(Ember)" refusals:
    routes/admin_api_keys.rs:47, admin_badges.rs:54,
    admin_embedding.rs:36, admin_flags.rs:46, admin_form_templates.rs:37,
    admin_user_fields.rs:50, admin_web_hooks.rs:69, color_schemes.rs:135,
    permalinks.rs:253,311, site_texts.rs:857, categories.rs:50;
    admin_dashboard_view.rs:319's plugin sections with 202). Theme SCSS
    and JS are 169 and 170, the plugins' own admin pages 221 and 202.
    Split: 90a. ~~`feat/site-texts`~~ (done) M (translation overrides
    over every `I18n.t` of the port, server and client strings alike,
    read again when the table changes; Admin::SiteTextsController's
    index with I18n.search over the reference's recorded English tree,
    glibc's merge sort for its ties, show, update with sanitize_field
    and TranslationOverride's validations and statuses, revert,
    dismiss_outdated, theme translations, the reseed options; overridden
    email templates (subjects with MessageBuilder's augmented arguments,
    bodies cooked without the notification view), digests and
    unsubscribe pages no longer refused. Refused: other locales,
    MessageFormat (`_MF`) keys, POST reseed, the Ember admin page),
    90b. ~~`feat/color-schemes`~~ (done) M (Admin::ColorSchemesController's
    index with the base palettes, create, update through
    ColorSchemeRevisor with diverge_from_remote for theme-owned schemes,
    the default theme's light and dark scheme, destroy; ActiveRecord's
    "colors is invalid" counts; /admin/config/colors/:id; schemes based
    on a DB scheme (remote copies) resolved; site.json's
    user_color_schemes. Left for 90d: /admin/config/colors, whose extras
    carry ThemeSerializer), 90c. ~~`feat/sidebar-defaults`~~ (done) M
    (SidebarSectionsController: index, show, create and update with the
    nested link attributes, SidebarUrl's hooks and validations and
    apply_links_order!'s free positions, destroy, reset of the community
    section, ReorderLinks, MoveLink, the public sections' staff logs; the
    default navigation menu categories and tags with update_existing_user,
    the backfill job and PUT /admin/site_settings/:id/user_count; their
    tags at signup and for anonymous visitors in site.json. Refused:
    content localization, user counts and backfills of the other user
    preference defaults), 90d. ~~`feat/themes`~~ (done, with its
    splits 90e and 90f) L: done are the
    reads (Admin::ThemesController index and show with ThemeSerializer,
    its settings of the bool, number, string and enum types, theme site
    settings, fields, translations, screenshots; /admin/config's
    customize themes, components and theme site settings, the color
    palettes with their sort, the welcome banner's themes), PUT
    /admin/themes/:id/site-setting, the 400-style refusal of themeable
    settings through PUT /admin/site_settings, scheme colors that set
    color_transformations' variables. Left, split on: 90e.
    ~~`feat/theme-writes`~~ (done) L (create, update with
    log_theme_change's strip_duplicates JSON, destroy, bulk_destroy,
    child and parent themes, components' enable and disable and the
    switch between theme and component, the default theme, theme
    settings through update and update_single_setting (with
    ThemeSetting's modifier refresh), theme translations, get
    translations and preview, user theme_ids on PUT /u; the theme's
    after_save: fields saved, dependents invalidated, html fields baked
    through Nokogiri's fragment with the CSP nonce, translation fields
    into their javascript caches, settings YAML validated, tests and
    migrations marked baked, `compiler_version` read from a baked row,
    and scss fields rebaked with a digest of their own: Rails' is the
    SHA1 of CSS whose source map names the compile's temporary
    directory, so no two compiles share it and cases compare it as
    `<digest>` (the `digests` key). Refused: new or changed scss values
    (their compile errors; since done in 169), extra_js
    (ThemeJavascriptCompiler), inline scripts (AssetProcessor),
    migrations, uploads and svg sprites, other locales' translations,
    settings that request a refresh, destroying a theme's own color
    schemes), 90f.
    ~~`feat/theme-imports`~~ (done) M, split: 90f1.
    ~~`feat/theme-imports`~~ (done) M (Admin::ThemesController#import of
    zip, tar, tar.gz and gz bundles through
    ThemeStore::ZipImporter (Compression's strategies and size budget,
    the root directory stripped, Dir.glob's file order) and
    RemoteTheme.update_theme: assets uploaded for themes, screenshots,
    about.json's metadata and version validations, modifiers (setting
    modifiers refreshed), color schemes (created, updated with their
    version bumped, the ones it no longer names destroyed, light and dark
    picked by only_theme_color_schemes), theme site settings, every file
    set as its field and the fields the bundle lost destroyed, imports
    into an existing theme (`theme_id`), `components` none or sync;
    export through ThemeStore::ZipExporter with generate_metadata_hash;
    upload_asset and theme upload fields (theme_upload_var and
    screenshots, their upload references, a `.js` upload's javascript
    cache, the icons-sprite's theme_svg_sprites row and XML check);
    RemoteThemeSerializer and `supported?` from the remote theme's
    versions; remote imports up to the clone (GitUrl.normalize,
    MAX_REMOTE_LENGTH, allowed_theme_repos, generate_key_pair's ssh key,
    unsupported schemes and keyless ssh as import errors, placeholders,
    `force`), update_source's checks, remote_check and remote_update of
    a theme that isn't git, the remote theme destroyed with its theme,
    git themes' fields refused; Jobs::CheckOutOfDateThemes scheduled.
    Refused (171): cloning a git repository, so remote imports, update
    checks, updates and source changes of git themes, and the child
    components about.json names; pax tar headers; sprites libxml2 would
    reject (its own error text)), 90f2. ~~`feat/theme-settings-types`~~
    (done) M (theme settings of every ThemeSetting type: lists (group
    lists' disallowed groups, the everyone alias under
    granular_anonymous_and_logged_in_groups_permissions), uploads (the
    default's upload field, urls to ids, the setting's upload reference),
    icons, objects through SchemaSettingsObjectValidator (each property
    type's checks and humanized messages, the ids looked up, uploads
    normalized and hydrated, disallowed groups removed, json_value and
    its upload references); ThemeSettingsValidator on values and on
    validate_yaml!'s defaults (create_opts' infinite bounds and the
    pluralized messages Rails joins as hashes, resolve_group_membership);
    ThemeSettingsSerializer's list_type, disallowed_groups,
    resolve_group_membership, json_schema and objects_schema;
    objects_setting_metadata; theme settings' upload references
    destroyed with the theme; site settings of the objects type read
    through hydrate_objects_setting_value. Refused (171, 187): settings
    YAML that doesn't parse (Psych's message), values of a shape Rails
    raises on), 90f3. ~~`feat/design-wizard`~~ (done) M (the admin
    design wizard: DesignWizardSerializer over the core themes
    (DesignWizard::PalettePairs, theme-owned pairs by their " Dark"
    names, else the built-in pairs, materialized or in memory;
    ThemeScreenshotThumbnails' generation job queued once an hour per
    theme; SiteSetting.homepage), DesignWizard::Apply (the contract's
    messages, palettes_available_to_theme, built-in palettes
    materialized with create_from_base, the theme's palettes and
    after_save, built-ins offered, palette selectability, the site
    settings in definition order with default_theme_id allowed though
    hidden, the theme's welcome banner and search experience); the
    setup wizard's JSON (WizardSerializer with its one `setup` step) and
    step updates (StepUpdater: ensure_changed on the title, the
    visibility, registration and approval settings with
    must_approve_users' approval of waiting users, the wizard_step
    log); Wizard.user_requires_completion? for site.json, account
    activation and password resets, with its bypass_wizard_check flip
    past 15 topics. Refused: a step changing the title (164, its
    welcome topic rewrite) or the locale (167); the
    generate_theme_screenshot_thumbnails job (171); the wizards' pages
    (90g)), 90g.
    `feat/admin-shell` L, open for 90g7's theme page and the leftovers
    named under 90 (the
    admin pages' HTML as Ember renders them, over the JSON the port
    already serves; /admin/customize/* and /admin/config/* refuse
    non-JSON requests today). Split by admin usage, each branch stacked
    on the one before, interactive parts through the existing JSON write
    endpoints with small scripts as the rest of the client: 90g1.
    ~~`feat/admin-shell-dashboard`~~ (done) L (the shell: Rails' admin
    stylesheet, vendored as compiled and served to staff on every page
    as Rails serves it; AdminSidebarPanel's sections from ADMIN_NAV_MAP
    with its amendments (upcoming changes, form templates and GIFs while
    enabled, the enabled bundled plugins' links with their
    setAdminPluginIcon icons) filtered for moderators, collapsed until
    opened unless holding the page, the review count and the news dots,
    Back to Forum, ToggleAllSections, the moderators' filter, the search
    box's mod+/ keycaps; `admin-area` and `admin-interface`, no Powered
    by Discourse; DPageHeader with DBreadcrumbsItem; the mobile view's
    header menu with the panel; GET /admin as the redesigned dashboard
    from /admin/dashboard.json for the controller's range: site advice
    with Ignore and Refresh, highlights, the pinned reports' cards
    loaded through /admin/dashboard/reports/bulk and drawn with Chart.js
    4.5.1 as AdminReportChart and AdminReportStackedChart draw them, a
    card's size toggle, site traffic with its stacked chart and top
    lists, engagement (headline, trust level pipeline, who's posting,
    activity by category with its sorting), search, system; the period
    picker with its presets and calendar and the configure menu, which
    reload the page for the new range or layout. Refused: the legacy
    dashboard (dashboard_improvements off), plugin dashboard sections,
    country names in the top countries (Intl.DisplayNames). Moderators
    get the dashboard under admin#index's 403 and the staff pages, the
    admins' ones not found (tests/moderator_pages.rs). Left, saying so
    when used: the manage reports,
    compare groups and category pickers, dragging a card's corner, the
    report modes other than the charts in a card and the design wizard
    (90f; the admin search done in 90g9); not drawn: the sidebar footer's
    keyboard shortcuts, as on the forum's sidebar), 90g2.
    ~~`feat/admin-shell-users`~~ (done) M (/admin/users and /admin/users/list
    sent on to the active list; /admin/users/list/:query for every tab
    from the index's users: the header with its actions and tabs (DMenu
    in the mobile view), Show emails, DFilterControls with the staff
    list's account type and the new list's activation, the sortable
    DResponsiveTable with the silenced and suspended lists' reasons, the
    empty states, DLoadMore's next pages, bulk select; the state the
    controller keeps is the URL's query; /admin/users/:id/:username from
    the show document: details, user fields, permissions, groups,
    activity, DiscourseConnect, exports and the danger controls, with
    log out, editing the name and title, activate, deactivate, approve,
    grant and revoke admin and moderation, the trust level and its lock,
    unsuspend, unsilence, clearing the penalty history, the primary
    group, impersonate, anonymize and delete working through the JSON
    endpoints, and the dialog service they confirm with. Refused: the
    settings tab (AdminAreaSettings, since drawn by 90g4), the user's badges and TL3
    requirements tabs. Left, saying so when used: the send invites
    modal, the user list export, the bulk suspend and delete
    confirmations, changing the username, revealing the email and the
    associated accounts, resetting the bounce score, the activation
    email, the IP lookup, disabling two-factor, the suspend, silence,
    upcoming changes and merge modals, the custom groups' picker,
    deleting every post, the archive export, deleting the
    DiscourseConnect record),
    90g3. ~~`feat/admin-shell-site-settings`~~ (done) L (/admin/site_settings
    sent on to the Required category with its query; /admin/site_settings/
    category/:id from the index's settings: the category nav (the
    mobile view's menu toggle opening it in a DMenu), the filter
    (SiteSettingFilter's name, keyword, description, value and fuzzy
    matches, Only show overridden and dependsOn, a category without a
    match going to All, the 100 results cap), each row with its staff
    log link, description, theme, upcoming change and dependency notes
    and dependents hidden until their parent is on; the form-kit editors
    (input, email, number, password with its toggle, textarea, checkbox,
    select with the none option as FKControlSelect adds it) and the
    legacy ones: the group ComboBox, the simple list (its values, shift
    and remove buttons, add input or the choices' ComboBox), ValueList
    for list, url_list and value_list, the closed MultiSelects of the
    compact, group, host and category lists, the tag chooser, the file
    types list with its add types buttons and toast, the category
    chooser, the font selector, the file size input with its unit, the
    color input, the upload editor with its preview, lightbox and Delete,
    the emoji list, the secret list, the uploaded image list's button;
    editing tracks each setting against its value with the save, cancel
    and undo controls and the changes banner, saving through PUT
    /admin/site_settings/:id or, from the banner, PUT
    /admin/site_settings/bulk_update (now ported: one transaction, the
    write cases bulk_update and bulk_update_invalid), with the
    requires_confirmation dialog and the backfill question's user count
    first. Refused: the backfill question when users would change (its
    modal), uploading a site setting file, the emoji picker on this page,
    the editors of the locale_list, tag_group_list, topic, date and icon
    settings and of the JSON and objects schema settings (their Edit
    buttons), the secret list's editing, the uploaded image list's
    modal, the category picker's dropdown and the locale list's chooser;
    the list editors (ListSetting's MultiSelect, ValueList, the category
    list's CategorySelector and the tag list's TagChooser) since edit and
    save in 164a. Left: the category chooser's selected
    category shows its name, not its badge; the font selector's fonts
    are not drawn in their own font), 90g4.
    ~~`feat/admin-shell-groups`~~ (done) S (/admin/groups: the admin
    groups header with its Settings and Groups tabs over GroupList as
    /groups draws it, with its filter, type menu and New Group; its JSON
    the empty index's 204; /admin/groups/settings and
    /admin/users/settings as AdminAreaSettings (the filter controls over
    the area's settings as one category, each setting's editor as on the
    site settings page, the changes banner), loaded as
    /admin/config/site_settings.json, now ported:
    Admin::Config::SiteSettingsController#index by categories, area,
    plugin and names with its allowed hidden settings and the
    InvalidParameters of none, measuring the upload settings' dimensions
    as filter_names' late select does (write cases config_index_*,
    admin_groups_json). The /admin/config areas' own settings tabs reuse
    AdminAreaSettings in 90g6. Left: Only show overridden stays in the
    URL's query on these tabs, where Ember keeps it in the component),
    90g5. ~~`feat/admin-shell-reports-logs`~~ (done, with
    feat/admin-shell-reports-logs-rest and feat/admin-shell-report-page) M (done:
    /admin/reports, AdminReports from /admin/reports.json less
    dashboard_hidden_reports, in REPORT_GROUPS' groups, the other core
    reports and each plugin's, under DFilterControls' text filter and
    group menu as the URL's `filter` and `group`, with its reset and no
    results state. Then (feat/admin-shell-reports-logs-rest) the Logs &
    screening pages: the staff action logs with their filters (the
    URL's `filters`, `startDate`, `endDate`), action menu, formatted
    details and their modal, next pages; screened emails (allow),
    screened URLs, screened IP addresses (filter, add, inline edit,
    delete, allow, block); search logs (period, search type); the email
    logs' five tabs with their filters, rows and next pages; and the
    site settings' pending edits kept across a layout redraw. Refused,
    saying so where used: the CSV exports, the theme and tag group
    changes' details, the incoming email modal. Then
    (feat/admin-shell-report-page) a report's page: AdminReport's chrome
    drawn by the server from the bulk request Ember makes (the back link,
    to the dashboard when it came from there; the period groupings,
    dates, the category, group, list, bool and category list filters;
    the average label, errors and no-data alert; the modes, refresh;
    the related users), its modes by static/js/admin-report.js (the
    table with sorting, pages, totals and the related users' summary
    menus; the line, stacked, stacked line and radar charts, shared with
    the dashboard in admin-report-charts.js; the inline table; posters
    by member type with its members report), the controls changing the
    route's query params as onParamsChange does. Refused, saying so
    where used: the CSV export, the compare groups modal, the counters
    and storage stats modes (no core report draws them on this page),
    discourse-solved's related solved topics; the summary menu opens on
    click only, not on a delayed hover. Left, refused as a page: a
    search log term's page),
    90g6. ~~`feat/admin-shell-config`~~ (done, the branches named
    below) L (done, in
    feat/admin-shell-config-settings: the areas that are their settings,
    localization, notifications, analytics and SEO, user defaults, trust
    levels, group permissions, site admin, legal, interface, navigation,
    security, spam, developer, rate limits, onebox, files, search,
    experimental and GIFs; a locale list setting drawn as Ember's
    compact list, whose chooser is still refused when opened (the
    other list settings' open and save since 164a). Then
    feat/admin-shell-config-flags: the flags
    list with its toggles, edit and its menu's move and delete, the
    settings tab, and the new and edit forms with FormKit's validation,
    counters and the applies-to MultiSelect. Then
    feat/admin-shell-config-permalinks: the permalinks list (the
    server's filter, copy, edit, delete), its settings tab and form with
    each type's field; the user fields list (move, delete) and form
    (the requirement and preferences rules, the options' ValueList, the
    confirmation for all users). Then
    feat/admin-shell-config-content-login: content's four tabs and
    login's settings, authenticators and DiscourseConnect tabs, and a
    setting's preview (tag_style's). Refused: login's Discourse ID tab
    (Admin::Config::DiscourseIdController is not ported); the plugins'
    login tabs (oauth2, oidc) wait for their plugins. Then
    feat/admin-shell-config-emoji: the emoji list (DFilterControls'
    name filter and groups dropdown, delete, selecting to export with
    the export's zip), the new emoji form (the group ComboBox with
    allowAny, each file posted with the name on the first), the
    import's preview and confirm, the settings tab; and
    discourse-reactions' like reaction leading its enabled reactions'
    list, not editable, as the plugin's modifyClass of EmojiValueList
    draws it. Then feat/admin-shell-config-logo-fonts: the logo page
    (AdminLogoForm's image fields with their upload, delete, lightbox
    and placeholders, the dark logos' toggles, the collapsed mobile,
    email and social media sections, the manifest screenshots' list)
    and the fonts page (AdminFontChooser with More fonts, the text
    sizes, the user count and backfill question), with
    Admin::Config::LogoController and FontsController's updates now
    ported: SiteSetting::Update over several settings in their declared
    order, upload settings by URL (logged as URLs, their upload
    references, SiteIconManager's missing optimized icons from the
    vendored stock images), the font and simple lists with their
    choices, enum classes' recorded values, x_summary_large_image's
    validation, the user options' defaults with their backfill and user
    count (write cases admin_config/logo_*, fonts_*). Refused: the topic
    OG image preview (TopicOgImageGenerator). Left: the font buttons
    are not drawn in their own font (only Inter's and JetBrains Mono's
    files are vendored). Then feat/admin-shell-config-welcome-banner:
    AdminWelcomeBannerForm (the enabled themes' DMultiSelect, the
    background image, FKControlColor, the page visibility and location,
    the language and its site texts), saving through the site settings,
    site texts and theme site setting endpoints; SiteSetting::Update
    takes color settings with CssColorValidator; DMultiSelect's
    stylesheet and the color input's own rule (discourse.scss) join the
    port's CSS; themes_with_setting answers a moderator InvalidAccess
    (write cases admin_config/welcome_banner_save, fonts_moderator).
    Then feat/admin-shell-config-about: the about page
    (AdminConfigAreasAbout's four cards, each form saving on its own,
    the choosers, the banner image, the language selector under content
    localization) with Admin::Config::AboutController ported: the
    update over its sections (hidden settings allowed, the description
    cooked, invalid values left unrendered as Rails leaves them) and the
    About page's localizations (SiteSettingLocalization's show and
    update, with their log); SiteSetting::Update takes the email,
    username, group and group list types with their validators; title
    and site_description no longer refused: their change handler
    rewrites the welcome topic (SeedData::Topics#update, crate::
    seed_topics) unless someone edited it, from the about page and the
    site settings page alike; Slug.for drops emoji codes and
    MaxEmojisValidator is ported for titles (unicode emoji in titles
    stay refused); TopicLink skips users#my_redirect links (write cases
    admin_config/about_*, site_settings/setting_title_*). Left: the
    description's composer is drawn in its Markdown mode (the rich
    editor is not ported); the wizard's title step still refuses (its
    rewrite isn't wired there yet).
    Then feat/admin-shell-config-category-management: the category
    management pages (the header with Create category and a tab for
    settings, all categories and each registered category type; each
    type's AdminCategoryManagementList with its filter, visibility,
    reset and next pages; the settings tab) with
    Admin::Config::CategoryManagementController#categories ported
    (ListAdminCategories over Categories::TypeRegistry's types and their
    matches, AdminCategorySerializer's badge chains, types and edit
    URLs); the config site settings JSON answers a moderator
    InvalidAccess (write cases admin_config/category_management_*).
    Left: the settings tab's category chooser still shows the name, not
    the badge (as on the site settings page).
    Then feat/admin-shell-config-upcoming-changes: the upcoming changes
    page (DFilterControls' text filter and drawer of status, type,
    impact role and enabled dropdowns, with their query params; each
    UpcomingChangeItem's plugin, description with the image preview and
    feedback link, the depends-on and permanent notices, badges with
    their tooltips, the "Enabled for..." select with its group
    selection, the related settings links) with
    Admin::Config::UpcomingChangesController ported: UpcomingChanges::
    List (the declared metadata, ConditionalDisplay's rules, the images
    recorded with the setting enums, the enabled-for target and groups,
    dependents, the last visit), UpcomingChanges::Toggle (the setting
    saved as add_override! saves it, its log and event, the groups
    cleared where not allowed, the plugin enabled setting's
    BackfillNotifiedEvents, and the :upcoming_change_enabled and
    :upcoming_change_disabled handlers: RemoveAndReplaceUncategorized
    Toggled with its snapshot, HorizonHighContextTopicCardsToggled) and
    SiteSetting::UpsertGroups (write cases admin_config/
    upcoming_changes_*). The write harness's `settings` now update a
    stored row in place, as DbProvider#save does. Refused: toggling
    voice_enabled (the voice plugin's on_enabled_change) and enabling
    enable_discourse_workflows (its PluginEnableHandler).
    Then feat/admin-mcp-backend: Admin::McpController (the overview,
    the group access rules with McpGroupScope.ensure_admins!, the
    primitives' policies and emergency blocks with the fresh consent
    they require), Admin::McpAuthorizationsController (the page with
    DiscourseMcp::AuthorizationStatus, revoking) and
    Admin::McpActivityController (McpActivityReport with its metrics),
    over the primitives core and the bundled plugins register, recorded
    from the reference with what their availability reads
    (scripts/record-mcp-primitives, write cases admin_mcp/*).
    Then feat/admin-mcp-clients: Admin::McpClientsController (the
    page, a pre-registered client with McpOauthClient's validations,
    blocking with its authorizations revoked) and a metadata client's
    refresh, ClientResolver.resolve! with the document fetched through
    crate::final_destination (write cases admin_mcp/clients*). Left:
    a refresh that fetches a document isn't parity-recorded (it needs
    an https fixture host); a client ID on localhost refuses with
    InvalidAccess where Rails answers 500 (its ECONNREFUSED escapes the
    resolver's rescue).
    Then feat/admin-shell-config-mcp: the MCP server pages (the
    overview with its setup checklist; the capabilities' primitive
    picker with its text, group by, impact and state filters, groups,
    visible selection, save with the floating actions and emergency
    blocks; the access rules with their delete and form (GroupChooser,
    the scopes' DMultiSelect with their primitive counts, the initial
    scope kept); the OAuth clients with their filter, next pages, block
    and refresh, the new client's presets and form, a client's details;
    the authorizations with their filter, next pages and revoke; the
    activity with its filter, outcome, DashboardDateRange (its picker
    now date-range-picker.js, shared with the dashboard) and next pages;
    the settings tab), common/base/_mcp.scss in static/css/admin-mcp.css,
    and UserSerializer#show_mcp_authorizations with the server on
    (DiscourseMcp::Access.eligible_for_exposed_primitive?, write case
    admin_mcp/show_authorizations). Left: the controls' generated ids,
    and values drawn as attributes where Ember sets the property.
    Then feat/admin-setting-links: linkifySettingLinks on the pages that
    draw setting descriptions (the site settings rows and checkboxes,
    the upcoming changes, the dashboard's site advice): each
    `{{setting:name}}` link goes to its plugin's settings page or the
    page of its area or category (the admin search's settingPageMap,
    crate::admin_setting_links), the JSON keeping the server's link.
    Then feat/admin-whats-new: /admin/whats-new (the page and its JSON,
    /admin/dashboard/whats-new too) and /admin/toggle-feature;
    DiscourseUpdates' new features in crate::new_features: the meta feed
    fetched through crate::final_destination (force_refresh, rate
    limited, and the daily Jobs::CheckNewFeatures with its new_features
    notifications, the previous ones deleted), filtered by version, by
    GitUtils.has_commit? (answered from vendor/discourse/commits.txt, the
    vendored commit's last year, scripts/record-discourse-commits) and by
    plugin, merged with the permanent upcoming changes; unseen state,
    seen marks and last viewed dates in discourse_rs.cached_values;
    Jobs::RefreshLatestNewFeature; the current user's
    has_unseen_features. The page cooks each description server side
    (DCookText) and groups the months in the browser's zone as moment
    does. The feed is a recorded fixture
    (parity/writes/files/new_features_feed.json), served by the fixtures
    server: record_writes.rb and tests/writes.rs take a case's `env`
    (the endpoint), fixture-file `redis` values and remote bodies, and
    a remote target without a query for any query), 90g7. ~~`feat/admin-shell-customize`~~ (done; a theme's page and its
    code editor in feat/admin-theme-page, their leftovers 90g7a) L (done,
    in feat/admin-shell-customize-content: /admin/customize/watched_words
    (on to the first action) and an action's page: the filter (the
    route's query param), the action nav with its counts and the mobile
    view's menu, the regular expression errors, WatchedWordForm (the
    creatable WatchedWords and TagChooser MultiSelects, the replacement
    and link fields, the case and HTML checkboxes, the duplicate check),
    the show words checkbox and the list with each word's delete, clear
    all, Add from file (the server's refusal of the upload stands), the
    download link and the test modal; /admin/customize/robots with
    Admin::RobotsTxtController now ported (show, update, reset and their
    logs; write cases robots/*); /admin/customize/embedding: the hosts
    (the full app toggle, the snippet's card, the table with each host's
    menu and delete, the empty list), the settings tab, the posts and
    topics and crawlers forms and the host form (CategoryChooser,
    TagChooser and UserChooser over their searches); /admin/customize/
    site_texts: the search, the language and theme ComboBoxes, the
    filters' drawer with its checkboxes and reset, the recommended texts,
    the results with their next pages, the Reseed modal (whose replace
    the server still refuses), and a text's edit page (the default text,
    the outdated and invalid notices with dismiss, DExpandingTextArea,
    the interpolation keys' pills, save and revert). Then
    feat/admin-shell-email-templates: /admin/email/templates/:id, the
    template's editor under the email pages' header (the subject, or the
    link to the site texts for a pluralized one; DEditor with its bold,
    italic, heading and list menus, blockquote and code, the preview from
    the markdown crate's WebAssembly; the interpolation keys; save and
    revert), with Admin::EmailTemplatesController#update now ported over
    the site texts' translation overrides (TranslationOverride.upsert!
    shared with them, the errors' format_with_full_message, the restore of
    both keys on an error, the logs) and index, show and revert reading
    the overrides (write cases admin_email/email_templates_update*).
    Refused, saying so: DEditor's link modal and emoji picker there. Then
    feat/admin-shell-color-palettes: /admin/config/colors, the list over
    its document (the subheader with New and its base palette modal,
    DFilterControls' text and type filters as the route's query params
    with reset and no results, ColorPaletteListItem with
    ColorPalettePreview's colors and their primary-low and tertiary-low
    as color-transformations.js makes them, the theme link, the default
    and user selectable badges, Edit or the built-in palette's tooltip,
    the menu's user selectable, set as light or dark default and delete);
    /admin/config/colors/:id, the palette's editor (the name's inline
    edit, Duplicate, Delete, the theme owner's note, the default light,
    default dark and user selectable toggles, ColorPaletteEditor's color
    and hex inputs with their checks and revert, the unsaved changes
    note, Copy to Clipboard, Save changes), a palette that isn't there
    going back to the list; /admin/customize/colors(/:id) on to them.
    Then feat/admin-shell-email-style-form-templates: Ace vendored
    (ace-builds 1.44.0, the modes, themes and search Discourse's
    ace-editor-bundle imports) and AceEditor in admin-customize.js;
    /admin/customize/email_style (on to html) and its html and css tabs:
    the editor, Reset to default, Save, the unsaved changes' discard or
    stay; /admin/customize/form-templates: the list (each template's
    categories, its view modal with edit and delete, edit, delete), New
    Template, and the new and edit forms (the name, the quick insert
    fields, the validations modal, the YAML editor, save, cancel,
    delete). Refused, saying so: the form template preview (the
    server's preview, Psych's YAML.dump, is not ported). Then
    feat/admin-shell-themes-list: /admin/config/customize/themes (the
    themes' grid: ThemeCardPreview's screenshots and their toggle or
    ThemesGridPlaceholder's drawings, the badges, Edit and the card menu's
    set default, user selectable, update, preview and delete; the install
    more card), /components (DFilterControls' name and status as the
    route's params drawn by the server, ComponentRow with its parent
    themes, enabled toggle and menu's preview, check for and install
    updates, export and delete; the next pages; the empty list) and
    /theme-site-settings (the current theme's help, the filter, the
    settings with their overriding themes' tooltips); InstallThemeModal
    (the popular themes, a file's upload, a git repository and create);
    /admin/customize, /themes and /components on to them, the theme
    card styles (theme-card.scss) now in the port's stylesheet. Refused,
    saying so: the design wizard's launch; a git install still meets the
    server's refusal of network fetches. Left, split three ways: a
    theme's page /admin/customize/themes/:id (settings, colors,
    components, uploads, translations, theme site settings), its code
    editor for the html fields, then theme SCSS compiled through grass
    and theme JS compiled (ThemeJavascriptCompiler) as their own items,
    since the server refuses both today; the base palette modal's ColorPalettePicker is a native
    select. Then feat/admin-theme-page: /admin/customize/themes/:id and
    /components/:id (themes#index's JSON for an XHR, as Rails routes
    them; a theme the list lacks on to the themes): show.gjs and
    show/index.gjs over themes#index's document (the back link, the
    name's inline edit, the serializer's errors, the remote theme's
    metadata and finish install, the required version and disabled
    alerts, the system and own palettes notes, the creator, the default,
    user selectable and auto update InlineEditCheckboxes, the light and
    dark ColorPalettePickers with their check and cancel, the included
    components or parent themes ThemeSettingRelativesSelector with Add
    all, Custom Code's edited sections and Edit Code, the uploads with
    remove and ThemeUploadAddModal, the extra files, the theme site
    settings, the theme settings and the translations as
    SiteSettingComponent rows with trackChanges off (legacy editors: text,
    textarea, integer, checkbox, enum ComboBox, compact list MultiSelect,
    value list, upload; each saved by its own endpoint with its toast),
    the translations' locale ComboBox, Preview, Export, Convert,
    Enable/Disable and Delete); the code editor
    /admin/customize/{themes,components}/:id/:target/:field_name/edit
    (a git theme on to the themes, a field the target lacks to its
    first): AdminThemeEditor's back button and theme link with the
    unsaved changes' discard or stay, Show advanced, Maximize, the
    target and field navs with their edited, error and icon marks, the
    field's compile error, its help, Ace in the field's mode (the JS
    initializer's template), the preview link and Save sending every
    field as saveChanges("theme_fields") does, edits kept across the
    fields until then. Compared with rs-ref-b: the pages' DOM (normalized
    outerHTML of #main-outlet from both, matching) and their behaviour
    (two playwright runs saving settings, translations, site settings,
    name, checkboxes, palettes, relatives, uploads, the editor's fields
    with an SCSS error, convert and delete, with the same results); write
    case themes/page_routes), 90g7a. `feat/admin-theme-page-rest` M: what
    the theme's page and editor still refuse or leave out, saying so:
    the theme settings editor dialog (ThemeSettingsEditor;
    static/js/admin-customize.js:3977), the objects setting's schema page
    and its edit button (themes#schema; routes/admin_customize.rs:1434,
    static/js/site-settings.js:1288), a remote theme's update, check for
    updates and change source (static/js/admin-customize.js:3980; the
    server refuses git fetches), uploading an upload setting's file
    (static/js/site-settings.js:1028), translations in other locales
    (routes/themes.rs:1064,1469; the locale ComboBox shows the server's
    refusal), saving theme JS (170; the editor shows the refusal's error),
    the color_definitions field's Ace placeholder, the
    ColorPalettePicker's and MultiSelect's open rows drawn without their
    swatches and filter; the remote theme branches (finish install,
    commits behind, the metadata links) are drawn from the serializer but
    not compared, no remote theme being installable on the reference
    without a network fetch, 90g8. ~~`feat/admin-shell-advanced`~~ (done; its left
    plugin pages are 221 and 202) M (done, one branch
    an area: `feat/admin-shell-backups`: /admin/backups with
    AdminBackupsActions (backup through the StartBackup modal, cancel,
    rollback, the read-only and archive mode toggles with their
    confirmations), the files' table with the download link email and the
    row's DMenu (restore, delete), UppyBackupUploader's resumable chunks,
    the restore notice; /admin/backups/logs with BackupRestore.logs (the
    bus backlog since start_logs_message_id, now saved as an operation
    starts) and the lines the bus brings; /admin/backups/settings as
    AdminAreaSettings; `feat/admin-shell-api-keys`: /admin/api (on to the
    keys), /admin/api/keys with ApiKeysList and ApiKeyItem (the empty
    list, the revoked label, the row's revoke and undo revoke), a key's
    page (ApiKeysShow: the description's inline edit, revoke, undo,
    delete, the scopes with their tooltips and Allowed URLs modal), the
    new key's FormKit form (EmailGroupUserChooser for a single user, the
    granular scopes' table, the required checks, the generated key with
    its copy button; no header, as the reference draws it on load), and
    the user_api settings tab; `feat/admin-shell-web-hooks`:
    /admin/api/web_hooks with WebhooksList (the delivery status label,
    the row's DMenu with delete), WebhooksForm for a new hook and an edit
    (the URL and secret checks, the event chooser, the category, tag and
    group filters), and a hook's events page (the status filter, ping,
    redeliver and redeliver failed, an event's request and response, the
    new events the bus brings); `feat/admin-shell-email`: /admin/email
    with its tabs (the email area's settings; the server settings with
    the test email; the summary preview with DatePickerPast, the single
    user's EmailGroupUserChooser, Refresh, the html and text toggle and
    sending it; the advanced test's run; the templates' list with
    DFilterControls and the overridden toggle);
    `feat/admin-shell-problem-checks`: /admin/problem-checks with
    ProblemChecksList and ProblemCheckItem (ignore, watch);
    `feat/admin-shell-badges`: /admin/badges with AdminBadges (New, Bulk
    award, Group settings' EditBadgeGroupings modal), AdminBadgesList's
    filter, AdminBadgesShow's FormKit form (the status question, a system
    badge's read-only texts, the icon or image choice with DIconGridPicker
    and the badge image's upload, the query section with its preview
    modal under enable_badge_sql, the grouping menu, the post header
    checkbox's dependency, submit and delete), AdminBadgesAward's CSV
    upload, and the badges settings tab; `feat/admin-shell-plugins`:
    /admin/plugins(.json) and /admin/plugins/:plugin_id(.json) from
    vendor/discourse/config/admin_plugins.json (scripts/record-admin-plugins;
    `enabled` read from each plugin's setting), the installed plugins list
    (DFilterControls with the status dropdown, the enabled switch, the
    settings links), and a plugin's settings page in
    AdminPluginConfigPage. Left (the email template editor done in 90g7):
    the configuration nav plugins' own admin pages
    (refused as a page: chat, AI, automation, data explorer and the
    rest the recording marks), enabling a bundled plugin the port
    doesn't implement (refused by the settings update)),
    90g9.
    ~~`feat/admin-shell-search`~~ (done) S (Admin::SearchController:
    /admin/search/all.json with Admin::Search::List's settings in
    basic_attributes, themes, reports and upcoming changes (the visit
    remembered); the /admin/search page and the modal the sidebar's
    search box and mod+/ open, on every admin page: AdminSearch with
    AdminSearchDataSource's items formatted server side (the pages from
    the visible nav with the nav map's descriptions, keywords and tabs,
    recorded from admin-nav-map.js by scripts/record-admin-nav-map, the
    plugins' configuration tabs; the settings with SettingLinkFormatter's
    labels and urls) and scored in admin-search.js as `search` does; the
    shortcut sentence where there is a keyboard; the moderators' panel
    filter matching the section headers and the links' keywords.
    Found on the way: all_settings puts default_locale first whatever
    filter_names says, and measures every mapped upload setting before
    filter_names selects), 90h.
    ~~`feat/template-client-strings`~~ (done) M (the templates' and
    views' literal English client strings through the locale with
    Ember's keys, site texts included: the header's buttons, the topic
    list's caption and columns, the categories table, the page titles,
    the suggested topics' columns, the select-kit filter label, the
    search page's advanced filters, the user card's label; the crawler
    header's server strings. Left as they are: the port's own controls
    with no Ember string (the topics' and lists' Next/Previous page
    links, the login page's form), and literal labels Ember writes
    literally too ("User primary")),
    91. ~~`feat/backups`~~ (done) L (creating, listing, downloading, restoring
    through the app; read-only mode is 79c), split:
    91a. ~~`feat/backup-files`~~ (done) M (the local store under
    public/backups/default: the list, status, the emailed download
    link with its token and the download, deleting, chunked uploads and
    backup_chunks_merger, staff logs 45/46, the read-only allowlist and
    ensure_backups_enabled. Refused: the S3 store, upload URLs and
    multipart uploads, the Ember index and logs pages),
    91b. ~~`feat/backup-create`~~ (done) M (POST /admin/backups queues
    what Rails spawns, run detached by the worker; the Creator:
    backup_metadata, pg_dump, the .sql.gz or the .tar.gz with the local
    uploads, the logs on /admin/backups/logs and in the system message
    (inline or as a log.txt.zip upload), cancel, delete_old;
    create_backup and schedule_backup. Refused: sites with S3 uploads,
    schedule_backup's failure message to the admins group; the logs go
    to every tab of the admin (no client_ids filter)),
    91c. ~~`feat/backup-restore`~~ (done) L (POST
    /admin/backups/:id/restore queues the Restorer, run detached: the
    archive unpacked, its version checked, read only and the job queue
    paused, Redis-held state flushed, the public tables moved to
    `backup`, the dump loaded with psql in one transaction, the
    restored settings (disable_emails), the uploads copied and remapped
    (DbHelper.remap), icons and rebakes, the logs and the message, the
    rollback on any failure; POST /admin/backups/rollback;
    drop_backup_schema. A Rails archive restores with 367 of 387 tables
    equal to the seed it came from, the rest the restore's own rows.
    Refused: backups that need migrations (no db:migrate), db:seed and
    recompiling theme fields (logged as Rails logs those failures), S3
    stores and S3 upload remaps)

[phase-8]

The mobile view, by page traffic. Discourse switches below a 40rem
viewport (Site#mobileView); the port renders the layout the
`mobile_view` cookie names, which the layout's head script keeps in step
with the viewport, guessing from the user agent without it
(src/mobile.rs). Measured with `scripts/parity-screenshots --mobile`.

92. ~~`feat/mobile-layout`~~ (done) L: the decision, `<html>` classes,
    the mobile stylesheets (mobile.css, menu-panel.css), the header with
    the hamburger's slide-in sidebar, the nav items as a bottom sheet,
    the topic list's mobile rows and last visit marker, the categories
    page's mobile style
92b. ~~`feat/mobile-redraw`~~ (done) M: crossing the 40rem breakpoint
    redraws the page in place, as Ember does, instead of reloading it:
    page.js writes the `mobile_view` cookie, asks for the current URL
    again and swaps the body and the `<html>` classes as for a boosted
    navigation (both layouts load the same head), keeping the post or
    topic row being read where it was, an open composer with its text,
    caret and state (composer.js takes the new layout's controls), and
    the sidebar's state. A first visit the user agent guessed wrong
    stays blank until the right layout is in; a failed request falls
    back to a reload, guarded against loops; a browser without cookies
    keeps the server's guess
93. ~~`feat/mobile-topic-page`~~ (done) M: the progress bar for the
    timeline, the topic map without participants, the post menu's icon
    Reply, suggested topics' mobile rows
94. ~~`feat/mobile-search-menu`~~ (done) M: in the mobile view the
    header's search button goes to the full-page search with the route's
    search context (Header#toggleSearchMenu), so the page is what was
    ported: /search in Ember's markup for both views (the types, the
    search bar, the context checkbox, the sort order, the result count,
    the entries with their highlights, more results as they scroll in,
    the users type), search.scss, the form label and input focus rules.
    Left: the categories & tags type (refused: Category.search and
    tags/filter/search), the advanced filters' panel, bulk select, the
    sort orders' `order:` terms (the search refuses advanced syntax), and
    the desktop header's search menu (search-menu.gjs, its drop-down and
    narrow-desktop slide-in) with the icon it hangs off
95. ~~`feat/mobile-user-menu`~~ (done) M: the user menu as Ember renders
    it in both views (a drop-down, or a slide-in over the cloak): the tabs
    with their unread counts (chat's included), the notification items
    with every core type's renderer and the reactions, voting and chat
    ones, the messages tab (UsersController#user_menu_messages, also as
    JSON), the bookmarks tab, the profile tab, show all, dismiss with its
    confirmation, the menu-panel.scss rules, the bookmarks tab with chat
    message bookmarks and their reminders (161). Left: the review queue tab
    (refused), the do not disturb and user status modals,
    show_user_menu_avatars (refused)
96. ~~`feat/mobile-composer`~~ (done) L: the mobile composer's controls
    (the toolbar toggle in place of the preview and fullscreen ones), its
    icon discard (trash can, an xmark for an edit) and upload button,
    DVirtualHeight's --composer-vh and keyboard-visible, the composer
    position fixes on phones. Left, on both views: the tags chooser
    (mini-tag-chooser and tags/filter/search), the composer actions
    menu, the rich editor (ProseMirror, the reference's default) and its
    toggle, the toolbar's heading, list and options menus and emoji
97. ~~`feat/mobile-user-pages`~~ (done) M: the user pages at a phone's
    width: DOverflowControls' chevrons and scroll-strip fade on the
    navs (and the composer's toolbar), the profile's
    UserNotificationsDropdown with users#notification_level (normal,
    mute; ignore refused), the summary stats' template whitespace, and
    the mobile rules a later base rule overrides in Discourse's bundle
    left out of mobile.css. Left, on both views: /u/:username/badges,
    /notifications and /activity/topics, which render the summary in
    place of their pages (ListController#topics_by, the badges and
    notifications lists aren't ported)
98. ~~`feat/mobile-menus`~~ (done) M: the topic footer's mobile branch
    (Flag and Mark unread in the "..." bottom sheet, the notifications
    button among the actions without reason or caret), the notification
    level menu as a bottom sheet, the progress bar's back button and its
    fullscreen timeline (TopicsController#excerpts for the post at the
    scroller), the user card fixed over its cloak and closed by a
    scroll, the live stream keeping the mobile nav trigger's label and
    sheet. Left: the drafts and post menus as sheets (the port's post
    menu has no admin menu, the drafts menu floats), the jump to post
    prompt, the fullscreen timeline's swipe, discourse-solved's
    notification level descriptions (desktop too)
99. ~~`feat/mobile-chat`~~ (done) M: chat's mobile layout and
    `chat_mobile` bundle: the chat header (back to the forum), the
    channel navbar's back button with the other channels' unread count,
    the mobile composer's layout, /chat as the channels list with the
    chat footer. Left: the footer's ~~DMs~~ (done, 280), search,
    starred and ~~threads~~ (done, 280; its threads refused with 160)
    routes, the channel list options menu (desktop's too), swipe and long
    press on a row, the list's empty states (refused)
101. ~~`feat/header-search-menu`~~ (done) M: the desktop header's search
    button (not while the welcome banner's field is in view, nor on the
    full-page search and login) and the search menu it opens
    (search-menu.gjs) as a drop-down, and the welcome banner's results
    panel: the random quick tip, recent searches (UsersController
    #recent_searches and #reset_recent_searches), the page's context
    items, the assistant's `#` (Category.search and the tags
    /tags/filter/search finds), `@` (user search with organizeResults)
    and `in:`/`status:`/`order:` suggestions, results by type as you type
    and topics on Enter, more, the "in this topic" context (Search with a
    topic as search_context, its posts highlighted on the page), clicks
    logged (SearchController#click), arrows, Escape, a click outside, "/"
    and Ctrl+Alt+F, search-menu.scss. Left: searching what the assistant
    suggests (the search refuses advanced filters, `#`, `@`), the private
    messages inbox, user, category and tag contexts of /search/query
    (refused), groups with a flair among the results (refused)
102. ~~`feat/narrow-desktop`~~ (done) M: the desktop view under 48rem
    (NarrowDesktop), decided like the mobile view from a cookie page.js
    keeps (`narrow_desktop`, beside `mobile_view`; a window crossing 48rem
    is redrawn in place as for 40rem, the reading position and composer
    kept): no sidebar, its toggle opening the hamburger slide-in over the
    cloak, the user and search menus as slide-ins, no drop-down-mode, the
    search button always shown; the topic list's 40rem to 48rem and 64rem
    rules (the latest poster alone, no views column, the smaller text);
    below 925px the progress bar in the timeline's place (on any desktop
    width, from a template the page swaps in, redrawn in place as the
    window crosses 925px)
103. ~~`feat/local-dates-composer`~~ (done) M: the composer toolbar's options
    menu (ToolbarPopupMenuOptions over Composer#popupMenuOptions, its
    shortcuts' keycaps) with discourse-local-dates' insert date modal
    (local-dates-create: the from and to date-times on Pikaday, vendored,
    the time input, the timezone combo box with the translated names, the
    advanced mode's recurrence, timezones and format, the preview while the
    timezone differs, Insert), its Ctrl/Cmd+Shift+. for now, and details'
    (Ctrl/Cmd+Shift+D), footnote's and spoiler's items; moment's locale for
    the request's locale (JsLocaleHelper's configureMoment: moment's locale
    file, the timezone names translated, the formats; all vendored).
    Left: the menu's insert table, change reply-to,
    format code, build poll and wrap items, which say they are not ported;
    the menu refuses to open with discourse-math, -events, -policy,
    -graphviz or -templates (for a member who can use them) on; editing a
    date from the rich editor (the rich editor is not ported); the locale
    from the Accept-Language header

[mobile-layout]

### Phase 9: what done items still refuse

Open in phases 9 and 10 (2026-10-15): 89 items, 120-208. By size: 23 S,
46 M, 20 L. Phase 9: 68 (21 S, 37 M, 10 L); phase 10: 21 (2 S, 9 M,
10 L). Not counted here: the open items above (90g, 92b), 100 (in Phase
10, dropped 2026-10-16) and 101-119.

Next, in order (2026-10-15): the gaps a logged-in member meets first,
so a member can use the port without telling it from Rails. 188 topic
edits (188a-188i2 done; 188i3-188i5, the edge cases, wait), 189 tags
writes (done), 193 the user pages (done, 193a-193f), 143 advanced
search (done, 143a-143i), 131 avatars (done).
Then 208 multisite (done, 208a-208d), ahead of the other large items: its cost grows with
every feature written against one AppState, and many forums in one
process is the port's clearest win for self-hosting.
Then the rest of Phase 10's core forum actions (190 categories writes,
191 external auth, 192 invites: all done by 2026-10-16); 100 was then
dropped (2026-10-16). The count of what is open now is in "State as of
2026-10-20" at the top.

The refusals of merged items, grouped by feature and ordered by how
often a real site meets them (reading and posting first, then
moderation, then admin, then the Ruby-compatibility engines). Counted
on main at cab8cde: 1512 `Unsupported("...")` sites with a literal
reason (1295 distinct) in 266 files of src/ and crates/, about 60 more
through helpers (`refuse_unported*`, crates/markdown's `ctx.refuse` and
sanitizer, `user_updater::REFUSED`, admin_user_show's checks,
jobs/consistency.rs's preflight, `web_hooks::refuse`), the 8 `not_yet`
cases under parity/writes, and 12 jobs enqueued as Rails enqueues them
that the worker fails as not ported (jobs/handlers.rs's fallback arm).
File references are under src/ unless they name a crate. Each item
names the refusals it lifts; a reason that appears in many places is
cited by its main sites.

Already owned elsewhere, so not repeated here: the admin's Ember pages
(28 "(Ember)" and "admin ... page" refusals, the wizards' two
(routes/wizard.rs:62,163) among them) are 90g; MessageBus's own ids and
positions were 100, dropped: they stay placeholders.

The enqueued jobs without a handler go with the item that owns them:
notify_mailing_list_subscribers (146),
index_category_for_search (144),
change_display_name (153),
update_hotlinked_raw and pull_user_profile_hotlinked_images (140),
user_suspension_expired (151),
generate_theme_screenshot_thumbnails (171), remap_hashtag (226).
notify_post_revision (188i2) and refresh_latest_new_feature (90g9)
have handlers now.

120. `feat/slug-for` M: topics without a stored slug (`Slug.for`),
    refused wherever a topic is linked (topic_view.rs:406,937;
    topic_list.rs:557; users.rs:1865; notifications.rs:568;
    bookmarks.rs:536; pretty_text/helpers.rs:275;
    topic_view/web_hook.rs:156,322; not_found_page.rs:190;
    review_list.rs:650; email/digest.rs:291,1058,1217;
    jobs/bookmark_reminders.rs:148; reports/counts.rs:1204;
    reports/lists.rs:440; plugins/reactions/users.rs:703;
    invites.rs:1165; routes/invite_redeem.rs:908,1248;
    routes/topic_update.rs:646; posting/validate.rs:1201;
    routes/chat_pages.rs:406), slug_generation_method other than ascii
    (posting/topic_save.rs:33; posting/create.rs:1755;
    posting/revise_topic.rs:1090; post_mover.rs:376;
    plugins/solved/by_user.rs:50), topics without a stored fancy_title
    on profiles (users.rs:1663). Titles with emoji codes are done
    (90g6's Slug.for). Non-English transliteration waits on 167
121. `feat/cdn` M: URLs behind a CDN (avatar.rs:66, the system
    avatar; pretty_text/helpers.rs:345;
    pretty_text/mod.rs:496; pretty_text/cleanup.rs:607;
    pretty_text/cooked_post_processor.rs:909; topic_list.rs:1038;
    users.rs:1074; badges_view.rs:114; reports/tables.rs:564)
122. ~~`feat/s3-store`~~ (done: feat/s3-store-interface, feat/s3-store-uploads,
    feat/s3-backups, feat/s3-direct-uploads) L: the S3 store for uploads
    and backups. src/object_storage (one Backend enum; the S3 API
    backend on rusty-s3 signing and the port's reqwest, for any
    S3-compatible service: endpoint, region, virtual-host or path-style,
    AWS hosts only without an endpoint) under FileStore::External and
    BackupStore::External; direct uploads (presigned PUT, multipart,
    completion) for uploads and backups; s3_cdn_url in the url helpers,
    cooking and reports; S3 backup metadata and remaps; avatars proxied.
    Measured on MinIO (`s3` write cases, scripts/s3-test-server,
    tests/object_storage.rs, tests/direct_uploads.rs). Left, now 256, each
    refused where it would run: s3_role_arn and instance-profile
    credentials without the AWS env keys (file_store.rs), restoring
    uploads into S3 and remapping AWS S3 urls (restorer.rs),
    include_s3_uploads_in_backups (creator.rs), promoting a direct upload
    of 100 MB or more (routes/external_uploads.rs), remote http(s) upload
    urls in the upload jobs; setting writes for the S3 settings wait on
    their validators (regex, enum class); GET /uploads/short-url (the
    external store's redirect, the local store's send_file) is not routed
123. `feat/secure-uploads` M, after 122: secure uploads and media
    (uploads.rs:388; posting/rebake.rs:49; pretty_text/helpers.rs:339;
    pretty_text/cooked_post_processor.rs:902; upload_references.rs:370;
    topic_list.rs:1026; email/styles.rs:243)
124. `feat/topic-view-staff` M (rewritten 2026-10-20: deleted topics and
    replies for staff with `post_stream.gaps`, `has_deleted`,
    `show_deleted`, `deleted_by`, `can_recover` and
    `can_permanently_delete`; ignored users' replies filtered as
    setup_filtered_posts does, ignored_user_like_counts in the topic
    view and the posts feed; the page's gaps (filled from
    /live/gap/:topic_id), deleted styling, recover button and the
    "Ignored content" first post, quotes of ignored users in
    quote-controls.js; detailed_404's 410 for a deleted topic, its 403
    and its NotLoggedIn for a message; left:) detailed_404 for a topic
    or category a group grants access to (not_in_group with the
    serialized group: routes/topics.rs:322; routes/categories.rs:139),
    deleted posts for staff outside the topic view
    (posting/revisions.rs:100), title_is_group
    (:2020), custom post notices (:2168), limit_suggested_to_category
    (:2800), suggested topics from other lists and for anonymous
    readers (topic_query.rs:839,887)
125. `feat/topic-thumbnails` M (the topic view's and web hook payloads'
    thumbnails done on feat/topic-view-thumbnails: Topic#thumbnail_info
    with the original and each optimized size, generate_topic_thumbnails
    enqueued once a minute when sizes are missing and the job ported;
    TopicView#image_url and #image_upload for a post number past the
    first, the page's og:image as get_absolute_image_url makes it with
    og:image:width, :height and :type; left:) theme thumbnail sizes
    (optimized_images.rs:115,146; topic_view.rs:1175 for the request
    theme; jobs/handlers.rs:978 for the job's extra_sizes; and
    jobs/consistency.rs's preflight), secure uploads in topic thumbnails
    (topic_view.rs:1179), topic images behind a CDN (topic_list.rs:46),
    topic images in digests (email/digest.rs:657)
126. `feat/featured-links` S: featured_link_root_domain
    (topic_view.rs:558; topic_list.rs:650,758;
    topic_view/web_hook.rs:436; email/digest.rs:660)
127. `feat/topic-links` S: TopicLink.topic_map with clicks
    (topic_view.rs:1655; its root_domain needs MiniSuffix, as 126 does),
    links to topic and site paths the router isn't ported for
    (posting/links.rs:65,481)
128. `feat/category-lists-rest` M: the subcategories_with_featured_topics
    styles and paginated category lists (category_list.rs:148,167),
    parent slugs needing CGI escapes (:120), category logos
    (categories_view.rs:203,396), lazy-loaded categories
    (categories.rs:189; bookmarks.rs:410; routes/drafts.rs:333),
    three-level nesting (topic_query.rs:386; hashtags.rs:424), encoded
    slugs and hidden parents in hashtags (pretty_text/helpers.rs:509,551;
    hashtags.rs:393), unknown style_types (pretty_text/helpers.rs:573;
    hashtags.rs:436)
129. `feat/muted-defaults` S: mute_all_categories_by_default,
    default_categories_muted and default_tags_muted in lists
    (topic_query.rs:359,362,369;
    category_list.rs:349), default tag preferences at signup
    (signup.rs:971), staff groups' category and tag notification
    defaults (roles.rs:163)
130. `feat/user-status` M: user status (UserStatus, the /user-status
    routes, the status modal) wherever a user is serialized (users.rs:540;
    current_user.rs:304; about.rs:253; bus.rs:476; directory.rs:650;
    topic_view.rs:1766; routes/groups.rs:867; routes/admin_api_keys.rs:74;
    routes/deleted_posts.rs:55; plugins/chat/direct_messages.rs:237;
    plugins/chat/membership.rs:128; plugins/chat/messages.rs:636;
    plugins/chat/channels.rs:642), the preferences' status
    (preferences_view/mod.rs:942) and PUT /u's `status`
    (user_updater.rs's REFUSED)
131. ~~`feat/avatars`~~ (done, 131a-c) M: generated letter avatars
    (avatar.rs:134, LetterAvatar), default_avatars and restrict_letter_avatar_colors
    (:113,122), avatar uploads and picking (uploads.rs:339), gravatar
    (user_updater.rs:554,556), random avatars at signup (signup.rs:406),
    system_user_avatar_template (site.rs:481), renaming or merging the
    system user with the logo as its avatar (username_changer.rs:171;
    user_merger.rs:229), the update_gravatar and create_avatar_thumbnails
    jobs. Split, stacked:
    131a. ~~`feat/avatars-letter`~~ (done) S: LetterAvatar drawn in Rust
    (`/letter_avatar/:username/:size/:version.png`, the cache paths and
    the full-size-first resize, the blank avatar for another version, the
    500 Rails gives a letter outside OptimizedImage's safe paths),
    User.system_avatar_template with a blank external_system_avatars_url,
    default_avatars (username_hash, split on newlines as Rails does),
    restrict_letter_avatar_colors, the users serializer's
    system_avatar_template, system_user_avatar_template on /site.json,
    renaming or merging into the system user with the logo as its avatar
    (with the primary email the save normalizes again). The letter is
    rasterized without FreeType's light hinting (README, Pages)
    131b. ~~`feat/avatars-uploads`~~ (done) M: avatar uploads (ICO
    refused as Rails refuses it, the crop to the largest avatar size,
    the create_avatar_thumbnails job), the upload serializer's
    thumbnail, `/user_avatar/:hostname/:username/:size/:version.png`
    (thumbnails made on demand, the redirect to the current avatar, the
    blank avatar, SVGs as attachments), PUT
    /u/:username/preferences/avatar/pick and /select (the system user's
    use_site_small_logo_as_system_avatar turned off, the user's and
    UserAvatar's upload references, user_updated, the quoted posts'
    rebake), DELETE preferences/user_image, the avatar selector modal in
    the preferences account tab (selectable avatars, the system, gravatar
    and uploaded choices, AvatarUploader); the avatar routes skip
    login_required as Rails' controller does
    131c. ~~`feat/avatars-gravatar`~~ (done) M: UserAvatar#update_gravatar!
    (FileHelper.download through FinalDestination, the 404 that is no
    error, the for_gravatar upload with its origin, the user's avatar
    following the old gravatar) and Jobs::UpdateGravatar, POST
    /user_avatar/:username/refresh_gravatar, after_save's refresh_avatar
    (the missing avatar row made, the gravatar job enqueued), random
    selectable avatars at signup. The success path is tested past the
    download (gravatar is https only, which the replay's fixture server
    is not); a gravatar the reference already holds becomes a new upload
    here, the port's encoders giving the processed file another sha1
132. ~~`feat/title-emoji`~~ (done) S: unicode emoji in titles
    (performEmojiEscape, MaxEmojisValidator's Emoji.unicode_unescape),
    emoji in a quoted topic's title (performEmojiUnescape), the emoji
    shortcut regex without a backtracking limit
133. `feat/anonymous-posting` M: anonymous posting mode (the shadow
    accounts) wherever a shadow acts (69 sites, 2026-10-20): posting
    (posting/validate.rs:80,152; posting/create.rs:239;
    posting/revise.rs:180; posting/revise_topic.rs:413;
    posting/enqueue.rs:88), likes, flags and bookmarks
    (post_actions.rs:235; likes.rs:187,519; flags.rs:107;
    bookmark_manager.rs:137), topic_guardian.rs:966, current_user.rs:307,
    read_tracking.rs:134, post_destroyer.rs:328,389,1255,
    pm_invite.rs:69,242, routes/pm_invite.rs:204, routes/invites.rs:193,
    topic_status.rs:137, post_owner_changer.rs:438, post_mover.rs:139,
    topic_converter.rs:153,456, topics_bulk.rs:99,
    routes/topics_bulk.rs:210,338,483, pm_lists.rs:59,
    topic_query.rs:290,839, search/mod.rs:667, review.rs:356,
    review_posts.rs:179, reviewable_user.rs:246, review_list.rs:175,
    the plugins (topic_voting/votes.rs:126,152,175,218;
    poll/vote.rs:263,485; reactions/toggle.rs:119; reactions/users.rs:559;
    routes/users.rs:741; solved/answers.rs:264; chat/mod.rs:83,129;
    chat/channels.rs:422; chat/publisher.rs:66; chat/dm_groups.rs:438),
    anonymous staff on admin routes (routes/admin_watched_words.rs:123,
    244,273; routes/directory_columns.rs:248;
    routes/admin_dashboard_sections.rs:217; routes/tag_admin.rs:213,475,
    1343; routes/tag_groups.rs:249,298,360;
    routes/admin_config_about.rs:211; user_penalties.rs:430,689,838,
    1018; routes/admin_users.rs:504,1837; site_setting_update.rs:224).
    The anonymous_action cookie's replay after a login is 211
134. `feat/posting-trust-levels` M (rewritten 2026-10-20: trust level 0
    authors post, held to PostValidator's new-user limits
    (newuser_max_links, _embedded_media, _attachments,
    _mentions_per_post, _replies_per_topic), links_require_trust with
    allowed_link_domains, has_host_spam? and its spam_post_blocked
    message, and silenced as fast typers (keep_posts); left:) posting
    as a staged user (posting/validate.rs:157), staged users on
    restricted categories (topic_guardian.rs:270), approving a staged
    user's queued post (posting/create.rs:331), max_consecutive_replies
    (posting/validate.rs:961), flagging a new user's linked posts as
    spam once has_host_spam? refuses a post (flag_linked_posts_as_spam,
    posting/create.rs:634), flag_sockpuppets (:681), silencing a TL0
    user without keep_posts (user_penalties.rs:929, hide_posts; chat's
    rate limit silence), TL0 bio links (users.rs:708, strip_links)
135. ~~`feat/quoted-posts`~~ (done, left: quoting into a new topic when
    the viewer can't reply (composer.js:1934, with the composer's reply
    as new topic); the selection's markdown for what the rich editor's
    serializer writes beyond paragraphs, emphasis, code, links, mentions,
    hashtags, emoji, images, lists, headings, block quotes, quotes and
    rules (tables, details, polls and the like come out as their text,
    quote-button.js:238,367); the q and e shortcuts (no keyboard
    shortcuts are ported) and the caret placed at the selection when Edit
    opens the composer; the quoted user's notification email, done in
    146; linked notifications from a quote of another topic's post when the
    quoted user wasn't notified, done in 145) M: QuotedPost
    on create, revise, recover and a moved first post's copy
    (quote_count, post_replies and reply counts for the posts quoted,
    reply_quoted), quote alerts and quotes in group messages (is_replying?),
    remove_full_quote_on_direct_reply, the rebake_quoted_posts_for_user
    job, the selected text's quote toolbar (Quote, Edit with the fast edit
    modal, Copy Quote, the share sources) and the composer's "Quote whole
    post"
136. `feat/slow-mode` S: slow mode when creating, enqueueing, editing and
    changing owners (posting/create.rs:542; posting/enqueue.rs:137;
    posting/revise.rs:263,448; post_owner_changer.rs:457), slow mode
    without seconds (routes/topic_timer.rs:252)
137. `feat/post-edit-rest` L (rewritten 2026-10-20: edits within the
    grace period, posts#update's title, repeated edit notifications and
    the notify_post_revision job landed with 188i1 and 188i2; a username
    change's definition description with 190c): hidden posts and those
    neither regular nor small actions, whispers and moderator actions
    (posting/revise.rs:239,244; small actions in 268), topic fields
    revised on a reply (:218), deleted posts (routes/posts.rs:355),
    staff_edit_locks_post
    (posting/revise.rs:760; post_owner_changer.rs:772), posts#update's
    image sizes and other fields (routes/posts.rs:304,316), revisions
    changing more than the body, title and edit reason
    (posting/revisions.rs:372), a category description synced
    from its definition post (posting/revise.rs:247;
    posting/rebake.rs:91; jobs/handlers.rs:1169;
    post_owner_changer.rs:454; user_destroyer.rs:403), revisions without
    a user or by the system user (posting/revisions.rs:381,383), a past
    version's post and raw (routes/posts.rs:548,603), an owner change's
    review_every_post and a hidden post's new owner unhiding it
    (post_owner_changer.rs:856,464)
138. `feat/posting-rest` M: replies to whispers (posting/create.rs:940),
    staff replying to closed topics (:562), duplicate titles and titles the
    cleaner changes (posting/validate.rs:1131; posting/topic_save.rs:30;
    post_owner_changer.rs:446), auto_close_topics_post_count
    (posting/create.rs:1396), returning-user post notices (:1621),
    featured topics with a category sort order (:2013),
    suppress_secured_categories_from_admin (:2016;
    jobs/post_alert.rs:1042), site_archived (posting/create.rs:242;
    routes/invite_redeem.rs:547), generate_topic_og_image and its
    regeneration (posting/create.rs:966; jobs/handlers.rs:1108;
    post_owner_changer.rs:372; post_mover.rs:901;
    posting/revise_topic.rs:1187; the admin's preview,
    routes/admin_config_branding.rs:173), posts#create's other params
    and archetypes (routes/posts.rs:85,114), process_post's recook
    and image sizes
    (jobs/handlers.rs:964,967), enqueueing a suspended user's post
    or one with polls (posting/enqueue.rs:94,97; approving one,
    review_posts.rs:624). Uploaded video and audio are done
    (feat/post-video-audio, with 141)
139. `feat/uploads-rest` M: image types Rails optimizes, cropping
    profile and card backgrounds (uploads.rs:399,409), keeping metadata
    (:412), replacing a failed upload (:488), missing extensions,
    outdated optimized images, SVG and AVIF thumbnails
    (optimized_images.rs:245,265,270), dominant colours with libvips and
    of AVIF, HEIC and JPEG XL (jobs/upload_jobs.rs:104,141), uploads not
    sent as multipart, site setting uploads, retain_hours, uploads from a
    url (routes/uploads.rs:46,67,188,209,216,222;
    routes/external_uploads.rs:921; routes/admin_emoji.rs:174), drafts
    with uploads (drafts.rs:51), uploads in user and group bios and
    profile backgrounds (promotion.rs:325,361; user_updater.rs:1712;
    discourse_connect.rs:898; jobs/periodical.rs:129;
    group_writes.rs:420), the composer's video thumbnail
    (composer-video-thumbnail-uppy.js: a frame of the uploaded video
    sent as a `thumbnail` upload named after its sha1, which
    UploadCreator stores without looking for the same sha1; the posts
    side, linking it and drawing it, is done in feat/post-video-audio)
140. `feat/hotlinked-media` M: SVG, HEIC, AVIF and JPEG XL images from
    other sites, and the broken headers FastImage raises on
    (fast_image.rs:246-247,360,424,452,478; the other images are sized
    over HTTP), remote uploads checked or downloaded
    (jobs/upload_jobs.rs:48,124), blocked and pulled hotlinked media
    (pretty_text/cooked_post_processor.rs:734,761;
    pretty_text/cleanup.rs:574), downloading hotlinked media in
    pull_hotlinked_images (jobs/hotlinked_images.rs:101; the job runs,
    downloads refused) and the update_hotlinked_raw job it schedules
    for a post with hotlinked media records, the
    pull_user_profile_hotlinked_images job. Needs: network fetches,
    through 52's FinalDestination
141. ~~`feat/optimized-videos`~~ (done in feat/post-video-audio) S:
    post_process_videos (an optimized video's url on its placeholder,
    the original in data-original-video-src, the post saved and
    reindexed; PostValidator, which that save runs, is not), a video's
    thumbnail linked to its post and made a first post's topic image
    with its thumbnails (link_post_uploads, handle_video_thumbnail),
    OptimizedVideo#destroy when its upload is destroyed, and destroying
    the optimized upload itself failing on the video's foreign key as
    Rails does. Left: oneboxes outside a post
    (pretty_text/cooked_post_processor.rs:915), a thumbnail for a video
    whose name is not a sha1 (pretty_text/cleanup.rs:610, Ruby's
    String#hex), a video thumbnail on a post whose topic is gone
    (upload_references.rs:446, a NoMethodError upstream); behind a CDN
    with 121
142. `feat/onebox-rest` M: X posts through the Twitter API
    (onebox/api_engines.rs:806), inline pull requests with
    discourse-github's status (onebox/github.rs:1143), the engines not
    ported (onebox/api_engines.rs:1358; onebox/embed_engines.rs:1691;
    onebox/github.rs:1053; onebox/engines.rs:623), chat channel and
    message oneboxes (onebox/mod.rs:250), the site's pages no route
    recognizes (:257), protocol-relative URLs (:238), localized topic
    oneboxes (:560, with 168), same-topic quotes outside a preview
    (:622), Time.parse's rollovers and missing years
    (onebox/ruby_time.rs), Ruby's float printing
    (onebox/embed_engines.rs:271; onebox/generic.rs:97). Needs: 175 and
    176 for the URL and charset cases, network for the APIs
143. ~~`feat/search-advanced`~~ (done, 143a-i) L: advanced filters and keywords
    (search/mod.rs:292,298: `in:`, `status:`, `category:`, `#`, `@`,
    `tags:`, `before:`, `after:`, `order:`...), search contexts
    (routes/search.rs:348,522), search_for_id (:529),
    restrict_to_archetype (:536), type_filter=all_topics (:546), the
    categories & tags type (:352, Category.search and
    tags/filter/search), URLs in terms (search/mod.rs:436), user search
    by groups among reviewers (user_search.rs:187), the directory's
    search by name (directory.rs:343, UserSearch). Split in the order the
    search menu suggests (`#`, `@`, `in:`, `status:`, `order:`), stacked
    branches `feat/search-advanced-<x>`; accents, CJK and the ranking
    settings stay with 144:
    143a. ~~`feat/search-advanced-category`~~ (done) M: posts_query as
    Rails builds it (Search#posts_query with apply_filters: the filters
    run over a term, or alone; the term squished as prepare_data does;
    the blank term's blurbs; find_grouped_results' rescue of a query
    Postgres rejects), the tokenizer's filters and keywords recognized
    as registered (core's, then the enabled plugins'), the `#` filter
    (`#slug`, `#parent:child`, `#=slug` without subcategories, a tag or
    its synonym's target, a tag group, the term put back otherwise) and
    `category:`/`categories:` (lists, `=`, ids, names), which skip
    categories_ignored once matched; the search menu's category, tag and
    user suggestions put their slug in the term (Ember's fullSlug), the
    full page's blurbs draw their emoji. Refused until their items:
    every other filter and keyword (search/advanced.rs), the solved
    plugin's prioritize_solved_topics_in_search (143e), a user search
    (type_filter=user) of only filters, which Rails fails on;
    search results for a member with lazy-loaded categories
    (GroupedSearchResultSerializer#extra)
    143b. ~~`feat/search-advanced-user`~~ (done) S: `@user` and `@me`,
    `user:` (a username or an id), `created:@` (first posts; a name
    nobody has matches those without a user, as Rails does), `group:`
    (visible groups whose members are visible, by name or id),
    `badge:` (by name or id)
    143c. ~~`feat/search-advanced-in`~~ (done) M: `in:` over the
    viewer's posts and topics: title (and `t`), first (and `f`),
    replies, pinned, wiki, likes, bookmarks, posted, created/mine,
    watching, tracking, seen, unseen, tagged, untagged, bots, humans,
    whispers, regular, all-posts, `include:unlisted`; poll's
    `in:polls`; `tags:` and `-tags:` (`+` for all, `,` for any,
    synonyms). The searches that list posts (in:all-posts, a topic's)
    load them as ActiveRecord eager-loads them: the distinct ids, then
    the ordered posts; posts that tie on every sort term come out as
    Postgres plans that query, on either side. discourse-solved's eager
    load follows solved_enabled now, where Rails reads it at boot
    143d. ~~`feat/search-advanced-status`~~ (done) S: `status:` open,
    closed, public, archived, noreplies, single_user, solved's solved
    and unsolved (the categories and tags enabling answers unless
    allow_solved_on_all_topics); posts_count, min_post_count,
    min_posts, max_posts, min_views, max_views, topic voting's
    min_vote_count, `with:images`, `filetypes:`, `before:`, `after:`
    (Search.word_to_date in UTC; they search every post, not only the
    recent ones), `locale:`
    143e. ~~`feat/search-advanced-order`~~ (done) S: `order:` latest,
    oldest, latest_topic, oldest_topic, views, likes (not aggregated,
    as in Rails), read, topic voting's votes, the `l` and `r` aliases,
    in:title's bumped order, a topic's search in the term's order;
    search_default_sort_order (taken from 144: it is only the default
    order); solved's prioritize_solved_topics_in_search in the
    relevance sort; the full page no longer draws like counts, which
    Ember's SearchResultEntry never gets the flag for
    143f. ~~`feat/search-advanced-messages`~~ (done) M: personal
    messages: `in:` messages, personal, personal-direct and all-pms,
    group_messages:, personal_messages:, type_filter=private_messages
    and its context (the search menu in an inbox), type_filter=all_topics
    and `in:all`, the `topic:` keyword, the user context on
    /search/query; Discourse::InvalidAccess as /search/query's and
    /search.json's 403. Refused: the full-page /search's HTML 403 page
    for such a search, and type_filter=all_topics for an anonymous
    search, which Rails fails on
    143g. ~~`feat/search-advanced-context`~~ (done) M: the category
    and tag contexts on /search/query, every context on the full-page
    /search (a context in the page's query, as its preloaded search),
    search_for_id (a topic id or a topics#show URL on this site, as
    Discourse.route_for recognizes it), restrict_to_archetype, URLs in
    terms (prepare_data's query string stripping, through Ruby 3.4's
    RFC3986 parser rules); the search menu names a tag page's context
    tag. Refused: the full-page /search's HTML error pages for a context
    it rejects (their JSON is ported)
    143h. ~~`feat/search-advanced-rest`~~ (done) M: the full-page
    search's categories & tags type (Category.search over the site's
    categories, then /tags/filter/search, as searchCategoryTag), user
    search by groups among reviewers (one join, as ActiveRecord keeps
    it) and a `groups` param given as a string, the directory's search
    by name (UserSearch with searchable user fields, staged users while
    the directory is on, the viewer listed with their matches); the
    port's category page heading rule no longer styles the search
    page's headings
    143i. ~~`feat/search-advanced-panel`~~ (done) M: the full-page
    search's advanced filters panel as Ember renders it
    (SearchAdvancedOptions, static/js/search-advanced.js: the category
    chooser, the tag chooser with "all the above tags", the topics/posts
    multi-select, the status combo box with solved's statuses, the
    posted-by user chooser, posted before/after with its date input,
    the posts and views counts), its filters written into the search
    term and read back from it (didReceiveAttrs, Ember's quirks kept),
    the count of filters in use on its toggle, the panel kept open by a
    sort order's search and closed by Enter and the Search button
    (94's leftover in 196); the no-results "Perhaps start a new
    topic?" link opening the composer for a new topic (createTopic).
    Refused: the category chooser under
    fixed_category_positions_on_create
144. `feat/search-settings` M: search_ignore_accents (unaccent) and CJK
    tokenization, searching and indexing (search/mod.rs:690,703,707;
    posting/search_index.rs:279,287), use_pg_headlines_for_excerpt,
    search_ranking_weights, search_prefer_recent_posts
    (search/mod.rs:693,696,699; the default sort order went to 143e),
    blurbs from old post_search_data rows (:1022,1029), human_only
    search logs (search_logs.rs:60), the index_category_for_search job
145. `feat/post-alert-rest` M (links to topics notifying someone and
    their consolidation, the skip_send_email option, posts by email
    skipping their to and cc addresses, populate_acting_user, mentions
    in flagged messages and warnings, group and @here mentions, nested
    replies done): post_alert options other than skip_send_email
    (jobs/post_alert.rs:214, none sent today); nested reply stats
    (HasNestedReplyStats: nested_view_post_stats on create, destroy and
    PostRevisor's reparent), which the port skips without refusing; the
    two `not_yet` cases posts/nested_reply_to_post and
    nested_bucket_consolidate differ only there
146. `feat/notification-email-rest` M (account_silenced and
    account_suspended, quotes, oneboxes and iframes restyled, replies to
    several posts or deeper than one level, previous replies, in-reply-to
    excerpts, the daily limit notice, lightboxes, upload links in the
    text part, prioritize_username_in_ux off, post_excerpts_in_emails
    and right-to-left styles done; left:) staged users of a topic started
    by email and private_email (email/notification.rs:184,198), group
    notification emails (:243), restricted tags in subjects (:312),
    lightboxes with secure uploads and Vimeo iframes (:940,955), posts of
    deleted users in the view (:1034), user oneboxes, iframes with an
    address outside the plain http(s) form and secure media
    (email/styles.rs:226,236,243), attachments and provider headers
    (email/sender.rs:447,465,514), the other email types
    (jobs/user_email.rs:93,101,105,507), sendmail delivery
    (email/mod.rs:169), mailing list mode (the
    notify_mailing_list_subscribers job)
147. `feat/flags-rest` M: flags that send a message (notify user,
    notify moderators, illegal: flags.rs:130), undoing flags
    (routes/post_actions.rs:181), flagging topics (:95), flags in
    messages (flags.rs:120), flags on posts already in review (:185),
    staff queueing a post for review (:176), hiding on a trusted user's
    spam flag (:380), silencing new users on spam flags and the scoring
    behind it (:417; guardian.rs:392; current_user.rs:909)
148. `feat/review-actions-rest` L: review actions that delete, edit,
    restore or penalize (review.rs:107; user_penalties.rs:509), on hidden
    or deleted posts (review.rs:85,258,575,617,620), hiding a topic's
    first or last post (:698), claims and notes (review_posts.rs:257;
    reviewable_user.rs:195,272,446), the other reviewable types
    (review.rs:247; review_posts.rs:211), user and plugin reviewables in
    the list (review_list.rs:594,599: ReviewableUserSerializer, the
    user.gjs card with its reject reason modal, approve_user), queued
    posts approved into closed or
    deleted topics, with tags, an email, plugin fields or polls,
    unsilencing their author, unhiding a first post
    (review_posts.rs:464,476,610,624,636,1262), user reviewables on
    invite-only sites or with an uploaded avatar and their rejection
    email (reviewable_user.rs:52,82,316), score conversations and types,
    reasons whose text the restricted cook would change, watched word
    scores without their words, the ids, date and additional filters
    (review_list.rs:205,208,216,1047,1100,1246,1266), reviewable web hook
    payloads (review_list.rs:602,2196,2233,2278 and `web_hooks::refuse` for
    the reviewable events), truncate_user_flag_stats (review.rs:534;
    review_posts.rs:1236)
149. `feat/category-group-moderation` M: enable_category_group_moderation
    (topic_guardian.rs:214; posting/validate.rs:449; review.rs:172;
    routes/review.rs:147,285,413; routes/review_claims.rs:215;
    reviewable_claims.rs:145; category_writes.rs:267;
    current_user.rs:548,982): its review queue for group members is
    refused before the list, though `Reviewable.viewable_by`'s group
    branch and `reviewable_post_scope`'s moderated categories are ported
    with the moderators' scope (reviewables.rs); moderators and
    owner-only groups on the admin view of a user
    (routes/admin_users.rs:1536), category moderators on the about page
    (about.rs:309). Moderators in the review queue, the user menu, a
    topic's queued_posts_count and flagged messages and warnings
    (`viewable_by`, `reviewable_post_scope`, `private_message_topic_scope`,
    DeleteInaccessibleNotifications) landed in `fix/moderator-reviewables`
150. `feat/deletion-rest` M: permanent deletion
    (routes/post_destroy.rs:80, force_destroy), recovering topics and likes
    (post_destroyer.rs:1281,1298), authors
    recovering their own posts (:1260), deleting flagged posts and
    embedded or published topics (post_destroyer.rs's
    refuse_unported_deletion), deleting replies as a non-staff user and
    deletes in a transaction (:310,391,400), likes on deleted posts and
    topics (likes.rs:536,558), destroying a category definition without
    its first post (the `not_yet` case
    scheduled/consistency_definition_without_op)
151. `feat/penalties-rest` S: staff reasons with markup
    (sanitize_staff_reason: user_penalties.rs:723; users.rs:1040;
    admin_users.rs:610; admin_user_show.rs:138; routes/session.rs:390),
    notifying responders of deleted replies (user_penalties.rs:543), the
    user_suspension_expired job. Penalties from the review queue are 148
152. `feat/topic-status-rest` S: pinning until a time
    (topic_status.rs:187, the unpin job), removing an expired banner (the
    `not_yet` case scheduled/consistency_expired_banner), loading a topic
    being moderated (topic_status.rs:116)
153. `feat/unicode-usernames` M: non-ASCII usernames (accounts.rs:65;
    signup.rs:99; routes/accounts.rs:162; routes/user_preferences.rs:174;
    username_changer.rs:92,568; user_name_suggester.rs:124,131;
    routes/invite_redeem.rs:403; pretty_text/mod.rs:491 for mentions;
    email/notification.rs:1248,1291; routes/required_fields.rs:206,321;
    group_message.rs:41), flag name keys from non-ASCII names
    (admin_flags.rs:207), titleizing a non-ASCII user field
    (plugins/poll/vote.rs:742), the change_display_name job
154. `feat/signup-rest` M: overlong usernames, staged users, approval
    and must_approve_users (signup.rs:217,240,248;
    routes/accounts.rs:911; roles.rs:121; discourse_connect.rs:1257;
    unstaging at login, session/login.rs:49 and
    discourse_connect.rs:636), the validation errors other than the
    username's and a taken email (signup.rs:341-379), screened emails
    and IPs (:363; routes/session.rs:404; routes/session_passkey.rs:137;
    discourse_connect.rs:1202; email_updater.rs:164), welcome messages
    (signup.rs:1085, while discourse-narrative-bot is off), automatic
    groups with titles, flair, granted levels or notification defaults
    (:894,908; promotion.rs:428; roles.rs:204,216), password algorithms
    other than pbkdf2-sha256 (session/token.rs:88,93,107), password
    reset errors (routes/accounts.rs:558,728,748). Invite codes were 192
155. ~~`feat/second-factor`~~ (done, 155a-c and
    feat/second-factor-modals; what the second factor code still refuses
    are records of a shape Rails raises on, 187, and 255's small
    leftovers) M: logging in with a second factor
    (routes/session.rs:415), backup codes (second_factor.rs:115;
    user_private.rs:120), security keys and passkeys (webauthn:
    second_factor.rs:102,112; routes/session_2fa.rs:84), the 2FA page's
    HTML (:64), password reset with second factors
    (routes/accounts.rs:580), the login-required 2FA redirect
    (enforce_second_factor), confirming an email change with second
    factors (routes/users_email.rs:404), the second factor page's and
    the security tab's writes (adding, editing and removing
    authenticators, security keys, backup codes and passkeys:
    routes/user_security.rs:297), confirming a session with a passkey
    (:163). Split:
    155a. ~~`feat/second-factor-totp`~~ (done) S: logging in with a TOTP
    code or a backup code (POST /session, the invite login code, the
    2FA page's backup codes), the webauthn challenge staged for a user
    with security keys so TOTP still works beside them,
    confirmed_2fa for a DiscourseConnect provider login,
    second_factor_remaining_backup_codes, the login page's second
    factor step
    155b. ~~`feat/second-factor-webauthn`~~ (done) M: security keys and
    passkeys (DiscourseWebauthn's authentication and registration, the
    cose gem's algorithms): as the second factor at login, on the 2FA
    page and with an invite login code, passkey login, confirming the
    session with a passkey, creating, registering, renaming, disabling
    and deleting keys and passkeys, the login page's security key step
    and passkey button
    155c. ~~`feat/second-factor-pages`~~ (done) M: the 2FA page's HTML,
    password reset, enforce_second_factor, the email change, the
    security tab's and second factor page's writes; then
    ~~`feat/second-factor-modals`~~ (done, "155d" in its commit) M: the
    second factor modals and passkey dialogs on those pages
156. `feat/user-profile-rest` M (rewritten 2026-10-20: the title,
    primary group, flair group, backgrounds and array options landed
    with 193a, external ids with 191a): featured topics (users.rs:678;
    current_user.rs:722), PUT /u's custom fields and date of birth
    (user_updater.rs's REFUSED; the date of birth is
    discourse-cakeday's, 202), user custom fields with several values
    (current_user.rs:467; routes/groups.rs:1122; review_list.rs:1411),
    clearing the login hint (user_updater.rs:911), the profile page's
    public user fields (87b), user tips and redirected_to_top
    (current_user.rs:1203,1226), view_user_route profile-hidden
    (routes/users.rs:673), setting another user's topic notification
    level (routes/topic_notifications.rs:56)
157. `feat/groups-rest` M: group flair uploads (site.rs:746; groups.rs:32;
    routes/groups.rs:620; group_writes.rs:565;
    routes/admin_groups.rs:1251; a flaired group in the search menu,
    routes/search_menu.rs:199), group custom fields and associated
    groups (routes/admin_groups.rs:224; user_destroyer.rs:470), rewriting
    a renamed group's mentions (jobs/handlers.rs:316,
    GroupMentionsUpdater), an editor who can no longer see the group
    (routes/admin_groups.rs:1455), ordering members by custom fields
    (routes/groups.rs:1031), random username suggestions (:441;
    user_name_suggester.rs:343), the group activity, messages and
    manage pages (routes/groups.rs:1600), membership request
    notifications and requested_group_name in web hook payloads
    (notification_create.rs:281; topic_view/web_hook.rs:531); group
    permissions with categories are done (190a)
158. `feat/shared-drafts` M: shared_drafts_category (topic_query.rs:371;
    topic_guardian.rs:399,823) and publishing them
    (jobs/topic_timers.rs:132, TopicPublisher)
159. `feat/tags-reads-rest` S: tags_listed_by_group
    (routes/tags.rs:661), tag list filters other than latest, top, hot
    and votes (:286), the messages' tags pages (routes/users.rs:85,1214),
    tag and hashtag search over localized tags (hashtags.rs:571;
    tagging.rs:569; routes/tag_search.rs:161; routes/search_menu.rs:380;
    routes/tag_admin.rs:1157, with 168)
160. `feat/chat-threads` L: chat threads (plugins/chat/notify.rs:102,482;
    plugins/chat/create.rs:213,216,226; plugins/chat/modify.rs:243;
    plugins/chat/channels.rs:348; plugins/chat/page.rs:589;
    plugins/chat/summary_email.rs:223; plugins/chat/publisher.rs:367;
    plugins/chat/dm_groups.rs:540; plugins/chat/membership.rs:542), the
    threads list (280a)
161. `feat/chat-uploads` M: chat message uploads
    (plugins/chat/messages.rs:276,588; plugins/chat/create.rs:99;
    plugins/chat/modify.rs:301; plugins/chat/page.rs:880,1315),
    oneboxes, images and videos in messages (plugins/chat/create.rs:747),
    ~~chat message bookmarks~~ (done, fix/chat-bookmarks: chat's registered
    bookmarkable, Chat::MessageBookmarkable, in plugins/chat/bookmarks.rs:
    its list query and search on /u/:username/bookmarks.json, the
    activity page and the user menu tab, the summary's bookmark_count,
    UserMessageBookmarkSerializer, the reminder notifications and their
    job, creating and destroying one, Bookmark.cleanup! only while chat is
    enabled; left refused where Rails fails with a NoMethodError: a
    bookmark of a message in a trashed channel (plugins/chat/bookmarks.rs:209),
    an unread reminder of a bookmark whose message is trashed
    (bookmarks.rs:831), chat bookmarks while chat is disabled
    (bookmarks.rs:704; jobs/bookmark_reminders.rs:119;
    bookmark_manager.rs:407), bookmarks of unregistered types
    (bookmarks.rs:863); channels of deleted categories or other chatables
    (plugins/chat/bookmarks.rs:176,183, with 162)), chat transcripts in
    posts (crates/markdown's bbcode refusal)
162. `feat/chat-rest` M: messages of deleted users
    (plugins/chat/notify.rs:76,487; plugins/chat/create.rs:293,721,763;
    plugins/chat/modify.rs:337,342; plugins/chat/summary_email.rs:459;
    plugins/chat/messages.rs:884), webhook messages and blocks
    (plugins/chat/messages.rs:598,601), bot changes
    (plugins/chat/modify.rs:166), archived channels and channels of
    deleted categories or other chatables
    (plugins/chat/channels.rs:141,536,713,718,749,797;
    plugins/chat/membership.rs:70; plugins/chat/publisher.rs:34),
    anonymous access (plugins/chat/mod.rs:129;
    plugins/chat/publisher.rs:66; plugins/chat/channels.rs:434), slug
    changes (plugins/chat/dm_groups.rs:478), the separate sidebar and
    empty states (plugins/chat/view.rs:276,341,345), the mobile footer's
    search and starred routes (99), chat's notification sounds (the
    preferences' sound preview, static/js/preferences.js)
163. `feat/solved-rest` S: discourse-solved's crawler schema markup and
    lists by solved status (its auto close timers, web hooks and search
    filters are done, 143d, 143e; the rest of solved's gaps: 233-235),
    ~~the dashboard's support section~~ (done, fix/admin-dashboard-solved:
    the section's data and drawing, its categories setting, the
    /admin/plugins/solved/dashboard-support refetch, its admin stylesheet,
    the accepted_solutions report's solved topics and table summaries;
    left: its category picker, refused as engagement's is,
    static/js/admin-dashboard.js:549; Discourse.cache of its data, not
    kept; odd params, plugins/solved/dashboard.rs:74,121,131 and
    routes/solved.rs:359), accepted solutions' related items
    without a viewer (reports/counts.rs:1145)
164. `feat/site-settings-types` L: setting types beyond string, number,
    bool, enum and upload through PUT /admin/site_settings/:id and the
    /admin/config pages' SiteSetting::Update (the same path). Split into
    the slices below, in the order an admin is likely to hit them. Bulk
    updates are done (90g3).
    164a. ~~`feat/list-settings`~~ (done) M: the list types (list of every
    list_type, url_list, host_list, value_list, simple_list, emoji_list,
    category_list, group_list, tag_list) through site_setting_update.rs
    and site_setting_lists.rs: TypeSupervisor#validate_value's choices
    for a list that allows no others (Ruby's split, the invalid ones
    named), the type's validator (a list's StringSettingValidator, a
    host list's HostListSettingValidator) or the setting's class
    (AllowedIframes, AtLeastOneGroup, ColorList, CspScriptSrc,
    EmojiPickerPinnedGroups, MarkdownLinkifyTlds,
    MarkdownTypographerQuotationMarks, RandomUsernameWordList,
    RegexpList for regexes both engines take, TopMenu, chat's
    DirectMessageEnabledGroups, discourse-reactions'
    ReactionsExcludedFromLike), validate_share_quote_buttons,
    validate_cors_origins and validate_slow_down_crawler_user_agents;
    add_override!'s sanitize_field for client lists, mandatory_values
    and disallowed_groups; the staff log with the value as given; the
    client setting published with the getter's value only when the
    stored value changed (site_setting_changed likewise); `refresh:
    true`'s Discourse.request_refresh! for every type; a bulk update in
    the settings' dependency order. Rails' Category, Group and Tag
    lists check no existence, and filter_value's hostnames never apply
    on this path (HOSTNAME_SETTINGS holds strings, the service passes
    symbols), so neither does the port. The default_* refusal is now
    SiteSetting::DEFAULT_USER_PREFERENCES' list (default_emoji_reactions,
    default_avatars and the like save). The page's editors open and save:
    ListSetting's MultiSelect for the compact, file types, host and
    group lists (filter, create, mandatory values read only, GroupList's
    everyone aliasing, HostList's wildcard refusal), ValueList for the
    list, url and value lists, the category list's CategorySelector, the
    tag list's TagChooser (/tags/filter/search). Write cases
    site_settings/list_*; tests/admin_pages.rs
    list_settings_save_log_and_publish and
    list_setting_editors_carry_their_content. Left here: RegexpList's
    message for a regex Ruby refuses (site_setting_lists.rs:197), the
    plugins' AssignStatuses, GithubBadgesRepo and discourse-events'
    validators (:249), the locale list (an enum class, 164d), the
    mcp_oauth_client_id_metadata_* settings (their change handler
    revokes clients, 164e), the emoji list's picker.
    164b. `feat/settings-file-size-and-pickers` M: the other plain types
    an admin meets on the common pages: file_size_restriction (the files
    page's sizes), category, topic, locale_enum (default_locale and its
    handlers), icon, date, datetime, regex, time
    (site_setting_update.rs:368).
    164c. `feat/settings-validators` M: the validator classes left
    (site_setting_update.rs:317): AuthProviderCredentials (the login
    providers, 12 settings), the username length and unicode ones, the
    reply by email, SSO and local login ones, the search ones, the
    selectable avatars, video conversion and upload ones, and the
    bundled plugins' (discourse-ai's, chat's, voice, events); custom
    validations (:313; s3, second factor, local logins, backups, secure
    uploads, image metadata, illegal content flags, anonymous likes).
    164d. `feat/settings-enum-classes` M: enum classes not recorded
    (:457; setting_enums.rs:128), lists of an enum class (:347, the
    content localization locales), list settings whose choices are not
    recorded (:411), regex and JSON schema validations (:1039).
    164e. `feat/settings-change-handlers` M: change handlers that write
    (:305: must_approve_users, emoji_set, slug_generation_method,
    reviewable priorities, splash_screen_image, simple_email_subject,
    content_localization_enabled, the MCP client id metadata settings)
    and the setup wizard's title (wizard.rs:345).
    164f. `feat/settings-default-backfills` M: default_* backfills and
    their user counts for the category and tag defaults and the
    preferences that are not user options (:320;
    routes/admin_site_settings.rs:78,174,194; sidebar_backfill.rs:64),
    with validate_default_categories_* and validate_default_tags_*.
    164g. `feat/settings-uploads-objects` M: uploaded image lists (:363),
    uploading a file through the page, objects settings (:366), tag
    group lists (:365).
    164h. `feat/settings-rest` S: enabling a bundled plugin the port
    hasn't started (:536, with 202), overriding deprecated settings
    (:283), upcoming change settings (:302), globally shadowed settings
    (:323), an archived site (:269).
165. `feat/reply-by-email-rest` L: bounces, forwarded emails, staged and
    unknown senders, attachments, unsubscribing, likes and notification
    levels by email, group and category addresses (email_in), replies to
    messages, spam and Authentication-Results headers, posts for the
    review queue or past a rate limit
    (email/receiver.rs:524,528,625,628,637,672,690,694,771,780,783,795,
    798,852,857), mailing list and resent emails and several senders
    (email/incoming.rs:418,441), POP3 polling, rebaking emailed posts
    (posting/rebake.rs:79; pretty_text/cooked_post_processor.rs:978,
    EmailCook)
166. `feat/group-smtp-imap` M: group SMTP and IMAP (email/sender.rs:305;
    jobs/post_alert.rs:555; routes/admin_groups.rs:1239;
    routes/groups.rs:642), the group SMTP credentials check
    (problem_checks.rs:342). Needs: network
167. `feat/locales` L: every locale but English: server strings
    (badge_granter.rs:409; system_message.rs:258; jobs/badges.rs:249;
    email/digest.rs:481; email/account.rs:118; email/sender.rs:170;
    email/notification.rs:195; review_posts.rs:941;
    routes/group_membership.rs:370; category_types.rs:457), site texts
    and reseeding (routes/site_texts.rs:65,863), the unsubscribe pages
    (routes/email.rs:45), email templates
    (routes/admin_email_templates.rs:222), theme translations
    (theme_writes.rs:332; routes/themes.rs:1064,1469), MessageFormat
    keys (routes/site_texts.rs:946), the default_locale entry
    (admin_site_settings.rs:246) and the setup wizard's locale
    (wizard.rs:354), slugs by transliteration
    (posting/text.rs:237), moment locales (49). Needs: the reference's
    other locale trees recorded (scripts/record-i18n-tree) and the
    client's
168. `feat/content-localization` M: content_localization_enabled
    (search/mod.rs:710; current_user.rs:1090;
    routes/sidebar_sections.rs:319; bookmarks.rs:414; review_list.rs:181;
    routes/drafts.rs:330; plugins/reactions/users.rs:665;
    plugins/solved/by_user.rs:47; about.rs:299; posting/rebake.rs:52;
    routes/users.rs:1604; routes/posts.rs:404; onebox/mod.rs:560;
    routes/admin_config_about.rs:241), category and tag localizations
    (category_writes.rs:274; routes/tag_admin.rs:319), 90c's sidebar
    content localization
169. ~~`feat/theme-scss`~~ (done) L: theme SCSS compiled as Rails
    compiles it (stylesheet/): grass, patched to print what dart-sass
    1.104 prints (crates/grass_compiler/PATCHES.md), against the vendored
    core stylesheets, with Stylesheet::Importer's colors, public image
    path, variables and mixins, Theme#scss_variables (settings and upload
    variables) and with_scss_load_paths' entry points, then
    AssetProcessor#post_css (autoprefixer's added and removed prefixes
    for Discourse's browsers, the light-dark() polyfill with progressive
    custom properties, postcss-nesting and the toggles' hash). New or
    changed scss fields save with their compile error
    (ensure_scss_compiles!), imported bundles with stylesheets too (90f1);
    the theme targets are served at /stylesheets/:target_:theme_:digest.css
    and linked as discourse_stylesheet_link_tag links them; the color
    definitions compile (the plugins' and the themes' color_definitions,
    WCAG, the fonts) in place of the hand port; members pick light and
    dark palettes in the interface preferences, previewed through
    /color-scheme-stylesheet. Measured in parity/writes/stylesheets (byte
    for byte but the toggles' hash and the source map annotation, which
    name Rails' digests) and parity/stylesheets/compile. Left, split into
    169a.
    169a. ~~`feat/theme-scss-rest`~~ (done; what it leaves is 169b) S:
    recompiling theme fields on restore (ThemeField.force_recompilation!,
    theme_writes::force_recompilation), Rtlcss for right-to-left locales
    (stylesheet/rtl.rs: the `_rtl` theme targets linked, served and
    flipped), the rest of autoprefixer (stylesheet/autoprefixer.rs on
    vendor/autoprefixer's tables for targets.js's browsers: value and
    selector prefixes, transitions, @supports, old flexbox, @viewport
    and resolution queries; run after the polyfill, as postcss's
    OnceExit runs), the polyfill's custom properties and `color-scheme:
    light dark` in at-rules, objects settings as SCSS variables (Ruby's
    inspect), and the toggles' hash of a stylesheet with a byte order
    mark. Measured in parity/stylesheets/postcss (scripts/record-postcss),
    parity/writes/stylesheets (objects, prefixes, rtl) and
    tests/backup_restore.rs, byte for byte; and against the reference's
    asset processor and rtlcss run by node on Discourse's core
    stylesheets and over 12,000 generated ones. Another theme's
    stylesheets are 258a's
    169b. `feat/theme-scss-maps` M: source maps: Rails serves each
    stylesheet's map (StylesheetsController#show_source_map, refused at
    routes/stylesheets.rs:128) and names it in the CSS's annotation,
    which the port leaves out (stylesheet/manager.rs:9). Not ported:
    grass makes no source maps, so it needs dart-sass's mappings tracked
    through grass's serializer, then postcss's map carried through
    autoprefixer, the polyfill and postcss-nesting; and Rails' maps name
    the compile's temporary load path directory and its digests, so they
    can only be compared by structure. Also refused here, none of which
    dart-sass's compressed output gives unless a theme asks for it:
    autoprefixer's control comments (`/*! autoprefixer: off */`,
    stylesheet/autoprefixer.rs:519), its visual cascade (:573) and a
    prefixed value in a transition it removes (:586); rtlcss directives
    (`/*! rtl:ignore */` and the others, stylesheet/rtl.rs:622), comments
    in a declaration it flips (:632) and a number it would write with an
    exponent (:87)
170. `feat/theme-javascript` L: extra_js (theme_writes.rs:326,532),
    so imported bundles with javascripts/ (90f1), inline scripts in
    theme html (:877, theme_html.rs:104, AssetProcessor; with them
    theme_translations_lookup), theme settings migrations (:329;
    an import's `skip_migrations` has nothing to skip until then).
    Needs: ThemeJavascriptCompiler's toolchain in
    process (Rails runs rollup, Babel and the Ember template compiler
    under mini_racer) and the theme-javascripts routes
171. `feat/theme-writes-rest` M: settings that request a refresh
    (routes/themes.rs:1144,1355,1410, Discourse.request_refresh!; 100
    was dropped, so the refresh goes over /bus),
    destroying a theme's own color schemes (theme_writes.rs:1865), a
    field over 1 MiB (:337), html that does not parse (:875;
    theme_html.rs:157), settings YAML that does not parse (:1244, Psych's message), svg sprites
    libxml2 rejects or whose file is gone (:690,699, their error texts),
    pax tar headers in imported bundles (theme_import.rs:426); git themes
    (90f1): cloning a repository for a remote import
    (theme_import.rs:1827,1833), update checks and updates
    (routes/themes.rs:1107; Jobs::CheckOutOfDateThemes,
    theme_import.rs:1857), source changes (routes/themes.rs:2069), the
    child components about.json names (theme_import.rs:1616,1663); the
    design wizard's generate_theme_screenshot_thumbnails job (90f3:
    an 800x450 webp of each core theme's screenshots). Needs: network
    (git over http(s) behind FinalDestination's SSRF guard, and over ssh
    with the deploy key), a webp encoder for the thumbnails
172. ~~`feat/admin-email-templates-rest`~~ (done in 90g7's
    feat/admin-shell-email-templates: update, show and revert over the
    translation overrides; templates in other locales are 167) S:
    updating, showing and reverting customized email templates,
    unblocked by 90a's override layer
173. `feat/upcoming-changes` S: UpcomingChanges::List is ported
    (crate::upcoming_changes, 90g6), and what's new with toggle-feature,
    unseen new features, their notifications and both jobs
    (crate::new_features, 90g9); left: undeclared changes and display
    conditions reading themes or categories (problem_checks.rs:558,588),
    upcoming change settings through PUT /admin/site_settings
    (site_setting_update.rs:253), toggling voice and enabling
    discourse-workflows (routes/admin_config_upcoming_changes.rs:202,205,
    their plugins' handlers, with 202)
174. `feat/ruby-regex` M: Ruby regexps the port can't run as Ruby does:
    watched words outside the engines' shared grammar, with full case
    folding or past the backtracking limit (word_watcher.rs:102,125,212,
    222,314,385,425,463,512; 80b), auto_silence_first_post_regex
    (posting/validate.rs:416,420), permalink_normalizations
    (permalinks.rs:37), embedding allowed paths (embedding.rs:154),
    ignore_by_title_regex and the receiver's address regexes
    (email/receiver.rs:325,465,494,886), group email domains
    (group_manager.rs:788), markdown_linkify_tlds
    (crates/markdown/src/render/mod.rs:143). Needs: Onigmo (the onig
    crate) or an equivalent
175. `feat/url-normalize` M: Addressable's normalizing and IDNA:
    canonical links and IDN hosts (onebox/html_doc.rs:91,118,135,149,175,
    248), onebox URLs that need encoding (onebox/mod.rs:909,1110;
    onebox/engines.rs:379; onebox/generic.rs:1067), non-ASCII URLs in
    links (crates/markdown's linkify: punycode and percent-encoding),
    permalink urls (permalinks.rs:356), pageview hostnames
    (browser_pageviews.rs:104,109; reports/pageviews.rs:411), nested
    query parameters (pretty_text/cooked_post_processor.rs:174),
    referers other than plain http(s) URLs (incoming_link.rs:89:
    URI::Generic hosts, user info, IP literals)
176. `feat/charsets` S: pages and email in charsets other than UTF-8
    (onebox/fetch.rs:164; retrieve_title.rs:141;
    email/incoming.rs:237,515,518,605,629). Needs: encoding_rs with
    Ruby's charset names
177. `feat/sanitizers-rest` M: the HTML sanitizers' remaining markup:
    sanitize_field's block elements, repairs and named entities
    (sanitize_field.rs; 85c), the admin notice sanitizer
    (problem_checks.rs:1028-1156), the cooking sanitizer's malformed
    markup (crates/markdown: comments, unclosed and misnested tags,
    character references), form template descriptions
    (form_templates.rs:125,249). Needs: Loofah's and Sanitize's behavior
    over html5ever, fragments parsed as Nokogiri parses them
178. `feat/email-css-rest` S: the email CSS beyond tags, classes, ids
    and two combinators: at-rules, escapes, nesting, other selectors and
    declarations (email/custom_css.rs:20-214)
179. `feat/psych-yaml` S: a form template's preview and public show
    (routes/admin_form_templates.rs:269,450, YAML.dump), YAML 1.1
    scalars, anchors and tags (form_templates.rs:95-201), revision
    values YAML can't hold (modifications.rs:22-55). Needs: a
    Psych-compatible YAML emitter
180. `feat/about-rest` S: the about page's visitor stats, banner image
    and localized settings (about.rs:125,302; about_view.rs:151), the
    welcome banner below the site header (html.rs:1701)
181. `feat/badges-rest` S: badge counts past a thousand, emoji in long
    descriptions, more grants than the first page, the title picker,
    grants without a time or topic (badges_page.rs:20,42,352,391,403,
    416), badge reason links to other routes
    (routes/user_badges.rs:126,169,173,175), grants for posts in messages
    (:237), badge preview columns of other types
    (badge_preview.rs:262,367), mass award CSV texts and encodings
    (admin_badges.rs:957; routes/admin_badges.rs:526,537)
182. `feat/drafts-rest` S: drafts backed up to a message (drafts.rs:86),
    clearing non-human users' drafts (:231,265), reading a draft at a
    sequence and clearing another user's drafts
    (routes/drafts.rs:358,385,438); the edit conflict check on tags is
    done (188i2)
183. `feat/messages-rest` S: group message lists with publish_read_state
    (pm_lists.rs:151), period on message lists (routes/messages.rs:171),
    group_pm_alert past group_pm_user_limit (pm_invite.rs:523), inviting
    a user who now ignores the inviter (:253), the inbox of a group the
    viewer isn't in (routes/users.rs:1045), the dismiss and bulk select
    buttons (33b; the user topic lists' too, 193c)
184. `feat/user-deletion-rest` S: blocking a deleted user's links
    (user_destroyer.rs:456, block_urls), MCP authorizations (:473; the
    preferences' list, preferences_view/security.rs:125), the
    user_delete_self log (user_destroyer.rs:264), deactivating with a
    pending review or associated accounts (routes/admin_users.rs:1317;
    jobs/accounts.rs:150), verbose auth token logging
    (routes/admin_users.rs:1237; routes/omniauth.rs:654;
    routes/user_security.rs:954; user_anonymizer.rs:130), non-address
    IPs (user_anonymizer.rs:177)
185. `feat/maxmind-crawlers` S: MaxMind locations for reports and
    suspicious logins (reports/tables.rs:386; jobs/handlers.rs:92; 89d),
    CrawlerScorer (jobs/rollups.rs:164), enable_performance_http_headers
    (request_tracker.rs:465), keep_old_ip_address_count
    (session/current.rs:867). Needs: a GeoLite2 database
186. `feat/push-notifications` M: push notifications
    (jobs/post_alert.rs:1463) by web push and through user API keys'
    push clients (84). Needs: network
187. `fix/rails-error-answers` L (was M), split by area when started: about 175
    refusals when written, about 720 on 2026-10-20 (634 of the
    `grep -rho` count, the rest written over two lines; every merged
    branch adds its own), of what Rails
    answers with an exception or never sees: params of the wrong shape
    (NoMethodError, ParameterMissing, InvalidParameters, TypeError),
    missing or vanished rows (RecordNotFound, a site without its system
    user, "gone once saved"), NOT NULL violations, staff actions missing
    from enums.json, onebox API values "of another shape", out of range
    numbers (routes/admin_embedding.rs, routes/admin_emoji.rs,
    routes/directory_columns.rs, routes/sidebar_sections.rs,
    routes/themes.rs, routes/color_schemes.rs, routes/admin_api_keys.rs,
    theme settings and about.json values of a shape Rails raises on
    (theme_writes.rs:1206,1221,1502; schema_objects.rs:391;
    themes.rs:656,671; theme_import.rs),
    user_fields.rs, user_merger.rs, jobs/handlers.rs,
    request_tracking.rs, admin_traffic_explorer.rs, email/incoming.rs,
    incoming_link.rs (request.host of a host-less request or an
    X-Forwarded-Host of commas only),
    onebox/generic.rs, onebox/api_engines.rs, onebox/github.rs,
    onebox/embed_engines.rs and others). Answer as Rails does where its
    answer is fixed (the 500 page, a 400, a 404); the rest stay refused

### Phase 10: what isn't built yet

The milestone work no phase item covered, and routes Rails serves that
the port doesn't route at all (a 404, not a refusal). Found by reading
src/routes/mod.rs against the milestones; no full diff against Rails'
route table has been made, so this list may be short.

188. `feat/topic-edits` L, left (2026-10-20): 188i3-188i5, the edge
    cases; 188a-188i2 merged, with the parent branch and
    feat/topic-edits-rest. Was: PUT /t/:slug/:id (title, category and tags
    edits, refused since milestone 4), moving, merging, splitting and
    converting topics, topic bulk actions (dismiss new and new posts);
    none of it was routed. Split by dependency, each branch stacked on the
    one before; tags on any of them wait for 189 and are refused until
    then: 188a. ~~`feat/topic-edits-title`~~ (done) M (PUT /t/:id and
    /t/:slug/:id, TopicsController#update: ensure_can_edit, the
    original_title edit conflict, PostRevisor's topic fields on the first
    post for the title, archetype and featured link: the revision with
    its topic diff, Topic's validations and before_validation cleanup,
    slug and fancy title, the search reindex, topic_edited web hooks, the
    reload_topic publish, keep_existing_draft; BasicTopicSerializer),
    188b. ~~`feat/topic-edits-category`~~ (done) M (category_id on PUT /t:
    TopicCategoryChangeValidator, can_move_topic_to_category,
    change_category_to_id with the topic counts, auto watching and
    tracking, notify_category_change, update_topic_upload_security,
    featured topics and reviewables, the category change whisper,
    safe_revision_errors), 188c. ~~`feat/topic-edits-convert`~~ (done) S (PUT
    /t/:id/convert-topic/:type, TopicConverter: the archetype and category
    through PostRevisor with a hidden revision, allowed users, user stats,
    the small actions, watching), 188d. ~~`feat/topic-edits-move`~~ (done) L (POST
    /t/:id/move-posts, PostMover: to a new topic (splitting) and to an
    existing one, moved_posts, the split_topic small action, counters,
    timings, topic users, notifications, reply links, quotes, bookmarks,
    reviewables, a full move closing the topic with its deletion,
    notify_moved_posts; chronological order, freeze_original, messages
    with participants), 188e. ~~`feat/topic-edits-merge`~~ (done) S (POST
    /t/:id/merge-topic: every post through 188d's mover), 188f.
    ~~`feat/topic-edits-dismiss`~~ (done) M (PUT /topics/reset-new,
    /topics/pm-reset-new and PUT /topics/bulk's dismiss_topics,
    dismiss_posts and destroy_post_timing, `filter=unread`), 188g.
    ~~`feat/topic-edits-bulk`~~ (done) M (the rest of TopicsBulkAction: change
    category, close, archive, unlist, relist, pin, unpin, notification
    levels, delete, archiving messages and moving them to the inbox,
    reset bump dates, converting, the nested view), 188h.
    ~~`feat/topic-edits-whispers`~~ (done) S (whispers from PostCreator:
    the category change whisper (create_post_for_category_and_tag_changes)
    and moving posts to a message: the whisper left behind, the mover and
    the posts' authors allowed in, participants, the new message's
    notification levels, a copied first post as a message reply), 188i.
    `feat/topic-edits-rest` L, merged as a branch, open for 188i3-188i5
    (what 188a-188h still refuse, with the tag
    edits on existing topics folded in from 189; split by dependency, each
    branch stacked on the one before): 188i1.
    ~~`feat/topic-edits-composer`~~ (done) M (title and category on
    posts#update, routes/posts.rs; edits within the editing grace period
    (the original kept in cached_values as Rails keeps it in Redis,
    update_revision); the composer's first-post edit with its title and
    category, sending PUT /t then PUT /posts as Ember does, the
    original_title edit conflict; the live title redrawn on
    reload_topic), 188i2. ~~`feat/topic-edits-tags`~~ (done) L
    (PostRevisor's tags field on PUT /t through 189's
    `tagging::tag_topic_by_names`, `apply` and `after_tags_saved`; PUT
    /t/:id/tags; the edit composer's tags and original_tags; the tag
    change whisper and notify_tag_change, with the notify_tag_change,
    notify_category_change and notify_post_revision job handlers; the
    revision page's tags_changes; the drafts tag edit conflict
    (drafts.rs); tags on a split's new topic (post_mover.rs); the bulk tag
    operations (topics_bulk.rs); moving tagged topics under tag groups or
    category tags (revise_topic.rs, TopicCategoryChangeValidator)), 188i3. `feat/topic-edits-revise-rest` M
    (shared drafts on PUT /t, routes/topic_update.rs:402;
    experimental_topic_category_change_notification, og image
    regeneration, review_every_post on topic edits,
    posting/revise_topic.rs:1529,1187,983),
    188i4. `feat/topic-edits-move-rest` M (a merge of a topic with votes
    (topic voting's TopicMerger), a copied first post's polls, reactions
    and flags, the split's topic rate limits, trashing a merged topic's
    embed, post_mover.rs:2105,817,996,2150,141,1836), 188i5.
    `feat/topic-edits-bulk-rest` M (a published page's topic converted,
    topic_converter.rs:236; a group's message inbox in pm-reset-new and
    bulk archiving, filter=unread over a message inbox, bulk deleting
    one's own topics or as a non-staff user, the nested view toggle
    while nested replies are on, routes/topics_bulk.rs:171,206,511;
    topics_bulk.rs:952,1094). A category definition's
    description changing on a title edit is published with 190a's
    CategorySerializer
189. ~~`feat/tags-writes`~~ (done, 189a-e; left: the tag settings page
    and the tag groups pages, which are 252, tag localizations 168, the
    remap_hashtag job 226) L: tags on topics (posts#create and update
    refuse them, routes/posts.rs; PUT /t, routes/topic_update.rs; a
    split's new topic, post_mover.rs; the bulk tag operations,
    topics_bulk.rs; moving tagged topics under tag groups or category
    tags, revise_topic.rs), the composer's tag chooser (96), tag
    CRUD, synonyms, tag groups and tag uploads, watched words' tag action
    (watched_words.rs:129,286; routes/admin_watched_words.rs:68;
    posting/create.rs:312), unused tags with synonyms
    (jobs/cleanup.rs:340). Split by dependency, each branch stacked on the
    one before. PUT /t's tag edits (PostRevisor's tags field, PUT
    /t/:id/tags, the edit composer's tags, NotifyTagChange) go with 188,
    whose PostRevisor calls 189a's `tagging::tag_topic`, `apply` and
    `after_tags_saved`: 189a. ~~`feat/tags-writes-topics`~~ (done) L
    (posts#create's `tags`, as names or `{id, name}` objects, queued
    and approved posts' tags too: reject_too_many_tags, TopicCreator#valid?'s
    validate_category_tags, DiscourseTagging.tag_topic_by_names with
    can_tag?, tags_for_saving and clean_tag, synonyms resolving to their
    target, readonly, hidden and permitted tags, filter_allowed_tags for
    the topic's category, creating new tags when allowed (Tag's slug,
    validations, search index and tag_created web hooks), one tag per
    group, mandatory parent tags, minimum_required_tags, required tag
    groups, tags restricted to other categories, tags on messages
    (pm_tags_allowed_for_groups); TopicTag's counters and
    category_tag_stats; TagUser.auto_watch and auto_track; first post
    alerts to tag watchers (jobs/post_alert.rs:556, taken from 145);
    watched words that tag topics (posting/create.rs:312)), 189b.
    ~~`feat/tags-writes-chooser`~~ (done) M (GET /tags/filter/search, Tags::Search
    with its required tag group and forbidden answers; the composer's
    mini-tag-chooser as Ember renders it on desktop and mobile, posting
    its tags), 189c. ~~`feat/tags-writes-watched-words`~~ (done) S (the tag action
    in the admin's watched words: creating, listing and site.json
    (watched_words.rs:129,286; routes/admin_watched_words.rs:68)), 189d.
    ~~`feat/tags-writes-admin`~~ (done) L (TagsController: update and rename with
    the renamed_tag log and hashtag remap, destroy, synonyms added and
    removed, upload CSV, bulk_create, the unused tags listed and deleted
    (jobs/cleanup.rs:340), info, settings and update_settings, list,
    personal_messages, and the notification level per tag (TagUser.change)
    with the tag page's notifications menu; also the tag page's
    navigation, info panel and edit button, tag descriptions cooked
    without oneboxes (jobs/periodical.rs:147); still refused: the tag
    settings page /tag/:slug/:id/edit (Ember, with 90g), tag
    localizations, and the remap_hashtag job it enqueues (HashtagRemapper,
    jobs/handlers.rs' fallback)), 189e.
    ~~`feat/tags-writes-groups`~~ (done) M (TagGroupsController: index, show,
    create, update, destroy and search, with their staff action logs,
    permissions, parent tag and one_per_topic)
190. ~~`feat/categories-writes`~~ (done, 190a-f; what they left is 252:
    the description change revising the definition post, the `not_yet`
    case categories/update_description, logos and backgrounds, the
    Events type and settings given per type, the editor's other tabs and
    entry points, deleting with embeddable hosts; email_in is 165,
    localizations 168, category group moderation 149, a chat channel's
    deletion 215, automations 202) L: POST, PUT and DELETE /categories,
    categories#show with CategorySerializer (the `not_yet` cases
    groups/read_permissions_anon and groups/read_permissions_user1;
    group_manager.rs:498; routes/groups.rs:1336), creating a category's
    definition topic (jobs/consistency.rs:157; the `not_yet` case
    scheduled/consistency_category_definition), saving a category without
    its settings (user_destroyer.rs:1147,1181; post_owner_changer.rs:282),
    category types without Horizon (category_types.rs:268), the category
    editor (topic voting's toggle). Split by dependency, each branch
    stacked on the one before: 190a. ~~`feat/categories-show`~~ (done) M
    (CategorySerializer with its plugin fields and the category
    guardian methods; categories#show, find_by_slug with the not-found
    page, visible_groups; group permissions with categories; a group
    member's /categories update (Category.set_permission!);
    publish_category, so a definition's description changing on a
    title edit no longer refuses; Support's schema on a site without
    Horizon), 190b. ~~`feat/categories-create`~~ (done) L (Category#save:
    category_params, ActiveRecord's casts, the validations in order with
    their messages, the before_save normalizations, permissions,
    settings, custom fields, tags, tag groups, required tag groups, form
    templates and posting review groups; POST /categories with position,
    Discussion's configure log, the created web hook, the index job and
    the creation log; create_category_definition, also for
    ensure_db_consistency (jobs/consistency.rs:157). Refused: logos and
    backgrounds, email_in, localizations, category group moderation,
    the plugin category types, automations on category_created_edited),
    190c. ~~`feat/categories-update`~~ (done) L (PUT /categories/:id: the
    move before the save, pending custom fields merged, the after_update
    callbacks (the definition retitled and reindexed, the old slug's
    permalink, the hashtag remap job after commit), the upload security
    job, the category_updated web hook and chat's auto join and leave,
    preload_user_fields!, log_category_settings_change; PUT
    /category/:id/slug; the user destroyer saves a deleted user's
    categories (user_destroyer.rs:1147, their settings built) and
    recovers a deleted definition (:1181, topic voting's reclaim); the
    owner changer's topic save into uncategorized (post_owner_changer.rs:282);
    a username change publishing a definition's new description.
    Refused: a description change, which revises the definition post
    (revise_category_definition, the `not_yet` case
    categories/update_description), changing the plugin category types,
    discourse-ai's translation of a renamed category), 190d.
    ~~`feat/categories-destroy`~~ (done) M (DELETE /categories/:id: the
    guardian's can_delete with chat's check, the category_destroyed
    payload taken inside HasDestroyedWebHook's around callback, the
    dependents in declaration order (topic voting's setting archiving
    votes), the definition trashed (VoteRelease), general_category_id and
    the category site settings cleared after all_settings measured the
    upload settings, the deletion published and logged. Refused: a
    category with a chat channel or embeddable hosts), 190e.
    ~~`feat/categories-types`~~ (done) M (Categories::Configure and
    Unconfigure for Discussion, Ideas (topic voting's toggle: the voting
    setting row, votes unarchived and archived) and Support (solved's
    custom fields; removing it saves the category): the plugin enabled,
    the schema's site settings through SiteSetting::Update for admins,
    the configure log; on create (category_type, category_types, the
    unavailable type's 422 rolling it back) and in update's
    manage_category_types (availability first, the types' own custom
    fields out of the pending ones). Refused: Events, site settings and
    category settings given per type), 190f. ~~`feat/categories-editor`~~
    (done) M (the category editor at /c/*slug/edit(/:tab) and
    /new-category: the General tab with the category types (Ideas is
    topic voting's toggle), Security's permissions table and Settings,
    saved as Ember's Category#save sends it, and the delete section.
    Left: the Images, Topic template, Tags and Localizations tabs, the
    emoji and icon pickers and colour palette, and the entry points on
    the category and categories pages (the edit wrench, New category))
191. ~~`feat/external-auth`~~ (done, 191a-g; what they left is 255:
    Facebook's signed request, a Google id_token or JSON body, Twitter's
    callback_url, a username other than the provider's, an inactive user
    by external id; unstaging is 154, accepting an invite with an
    external login 251, Patreon's reward groups 202) L: OAuth2 and OIDC logins (/auth/:provider),
    DiscourseConnect (/session/sso*), the authenticators on /site.json
    and associated accounts (site.rs:1235; user_private.rs:130;
    admin_user_show.rs:185; current_user.rs:641,644;
    reports/lists.rs:297), external ids (web_hooks/payloads.rs:95;
    notifications.rs:544; session/api_key.rs:135;
    routes/user_api_key_auth.rs:296; group_manager.rs:753),
    auth_immediately and logout with only external logins
    (routes/login_required.rs:132; routes/session.rs:605), Twitter's
    login health check (problem_checks.rs:488). Needs: network. Split
    into stacked branches `feat/external-auth-<x>`:
    191a. ~~`feat/external-auth-connect`~~ (done) M: DiscourseConnect,
    consumer and provider (/session/sso, /session/sso_login,
    /session/sso_provider with its 2FA confirmation and the login it
    waits for), external ids (the current user, notifications, web hook
    payloads, API requests, /u/by-external, admins' `external_ids` on
    PUT /u, email confirmation through the record), the admin's
    sync_sso, sso_record and delete_associated_accounts, sso-email and
    sso-payload, the authenticator registry (every builtin and bundled
    plugin provider: site.json's auth_providers, associated accounts on
    /emails, the user and the admin view, the report), auth_immediately,
    logout with only external logins. Refused: invites through
    DiscourseConnect (192), suspended users at its login, unstaging a
    staged user, validation messages other than the email's in sync_sso
    191b. ~~`feat/external-auth-oauth`~~ (done) M: the OmniAuth request and
    callback phases (/auth/:provider, the confirmation page,
    /auth/failure), discourse-oauth2-basic and discourse-openid-connect,
    connecting and revoking accounts (/associate/:token,
    preferences/revoke-account, the account tab), the login page's
    buttons, the authentication_data cookie read once (auth-complete),
    OIDC's RP-initiated logout. Refused: a provider's description as
    the bio, groups linked to a provider's groups, invite-only sites
    191c. ~~`feat/external-auth-signup`~~ (done) M: signing up with the
    authentication the callback kept (users#create, UserAuthenticator,
    LoginActivator), the client's authentication_data handling, the
    /signup page. Refused: a username other than the provider's when it
    overrides usernames (UsernameChanger saves the user early)
    191d. ~~`feat/external-auth-builtin`~~ (done) M: GitHub, Google
    (with its groups through a service account), Facebook, Discord,
    LinkedIn, Discourse ID, Microsoft and Amazon strategies. Refused: a
    Google id_token or JSON body handed to the callback, Facebook's
    signed request cookie
    191e. ~~`feat/external-auth-twitter`~~ (done) M: Twitter (OAuth 1.0a)
    with its login health check, nested hashes in the session cookie, a
    provider's description as the bio (191b's refusal). Refused:
    Twitter's callback_url param (a callback path of its own)
    191f. ~~`feat/external-auth-plugins`~~ (done) M: discourse-apple-auth,
    discourse-lti and discourse-patreon's logins, OmniAuth's compat
    patch for doubled subfolders, redirects without a content type, a
    string setting set to nil kept as NULL. Refused: Patreon's reward
    groups for a new patron (the plugin's group sync), an LTI token with
    several audiences (a NoMethodError upstream)
    191g. ~~`feat/external-auth-remaining`~~ (done) S: the refusals the
    earlier items left: an expired token refreshed, the admin IP
    allowlist, a suspended user's message, the email RecordInvalid
    failure, groups linked to a provider's groups, the associate page,
    Discourse ID's revoke endpoint and revoking its account
192. ~~`feat/invites`~~ (done, 192a-g; left: 250, 251) L: InvitesController (/invites, invite links,
    redeeming), invite codes at signup (signup.rs:194), adding group
    members by email (routes/admin_groups.rs:601), messages to email
    addresses (posting/create.rs:620; pm_invite.rs:69), invite-only
    sites (reviewable_user.rs:52), inviting to restricted categories
    (topic_guardian.rs:549), invalidating invites on password reset
    (routes/accounts.rs:662), the profile's invites pages
    (/u/:username/invited: routes/users.rs:173). Split in the order a
    member meets them, stacked branches `feat/invites-<x>`:
    192a. ~~`feat/invites-create`~~ (done) M: POST /invites, PUT and
    DELETE /invites/:id, /invites/retrieve, create-multiple,
    destroy-all-expired, reinvite and reinvite-all (Invite.generate with
    the model's validations, the invites-per-day, reinvite and
    admin-invite limiters, InviteSerializer), Jobs::InviteEmail
    (InviteMailer's four templates), users#invited's JSON (pending,
    expired and redeemed lists, search, counts), inviting to restricted
    categories as an owner of one of their groups (topic_guardian.rs)
    192b. ~~`feat/invites-redeem`~~ (done) M: GET /invites/:id (its
    invite_info as JSON, the signed-in redirect to the topic, no_ember's
    errors), PUT /invites/show/:id (InviteRedeemer for a new or
    signed-in user, its activation email or password instructions,
    Jobs::InvitePasswordInstructionsEmail, topic notifications), the
    login code for an invite (/session/login-code(/verify) with
    invite_key; plain code logins still refuse), redeeming on activation
    (EmailToken.confirm), invalidating on password reset, invite-only
    ReviewableUser, invite codes at signup. Refused: user fields and
    staged users on redemption
    192c. ~~`feat/invites-elsewhere`~~ (done) M: adding group members by
    email (cannot_add_emails, the invites with skip_email, the rate limit
    message), inviting to a message by email (Topic#invite's
    Invite.generate with the topic and groups), messages to email
    addresses (TopicCreator#add_emails: staged users created, the
    recipients in order) and the notification and email a staged
    recipient gets (user_posted_pm_staged, no unsubscribe link or respond
    instructions). Refused: a staged recipient of a topic started by
    email (IncomingEmail's subject)
    192d. ~~`feat/invites-pages`~~ (done) M: the accept page
    (invites/show: the progress bar, who invited, CodeLoginForm's email,
    code and username steps, the classic signup form without login
    codes, a member's Accept Invitation) and the profile's Invites tab
    (user-invited: pending, expired and redeemed lists, removing,
    copying, removing the expired, resending all, search) in Ember's
    markup with common.css's rules (static/css/invites.css),
    static/js/invite.js and user-invited.js; a staff member redeeming
    refreshes the staff groups. Left: the invite modals (create, edit,
    bulk) and the avatar picker while signing up, which say they are
    not ported; invites past the first page (DLoadMore)
    192e. ~~`feat/invites-bulk`~~ (done) S: POST /invites/upload_csv
    (the header row or the email, groups, topic_id columns,
    max_bulk_invites), Jobs::BulkInvite (groups by name the admin may
    edit, the topic, existing users added to the groups, the
    bulk_invite_succeeded or _failed message with its log) and
    ProcessBulkInviteEmails. Refused: user fields and locales in the CSV
    (staged users for them), the bulk upload modal on the Invites tab
    192f. ~~`feat/invites-staff`~~ (done) S: redeeming admin and
    moderator invites (InviteRedeemer's grant_moderator_from_invite and
    create_admin_grant_confirmation: a new user starts as a moderator,
    an existing one is granted moderation, an admin invite logs the
    grant and asks the inviter to confirm admin by email)
    192g. ~~`feat/invites-external`~~ (done) S, after 191: redeeming the
    invite a visitor came from through DiscourseConnect
    (validate_invitation! and redeem_invitation in sso_login: the user
    activated, logged on and sent to the invite's topic), OmniAuth's
    invite_required? on invite-only sites (requires_invite unless the
    login started on a redeemable invite's page), the kept external
    login's email on the invite's info (different_external_email),
    invites allowed with only external logins. Refused: accepting an
    invite with an external login (UserAuthenticator in
    create_user_from_invite: the provider's overrides, the account,
    avatar and profile, the session's authenticated_with_oauth), the
    invite page with external logins enabled (LoginButtons,
    social_login_available)
100. ~~`feat/message-bus`~~ (dropped, the owner's decision 2026-10-16)
    L, after 192, moved down from Phase 8: a compatibility layer, not a
    gap in the port's own pages, which get live updates over pg-bus's
    /bus already (MessageBus on pg-bus, so clients that speak Rails'
    protocol work against the port: bots and integrations on
    message-bus-client, apps acting through user API keys). Not ported:
    outside clients move to the port's /bus (GET /bus/events and
    /bus/poll, docs/live-updates.md), which carries the same channels
    and payloads with pg-bus positions instead of MessageBus ids. The
    fields Rails fills with MessageBus.last_id (topic view's
    `message_bus_last_id`, chat's `channel_message_bus_last_id` and
    `message_bus_last_ids`, user status's `message_bus_last_id`,
    `do_not_disturb_channel_position`) stay placeholders, with their
    parity ignores; backup logs still reach every tab of the admin
    (no client ids), and a presence state's `last_message_id` stays a
    pg-bus position. Carried over from 100b, as they need no new pg-bus
    API (`fix/bus-publish-and-audience`): TopicUser.change publishing
    notification_level_change whenever it sets a level, the level
    create_missing_record picks included, rather than its callers doing
    it (a new message's author and recipient watches were never
    published), and the tags a /bus subscriber holds following
    MessageBus.group_ids_lookup (admins every group, members
    logged_in_users and, without granular permissions, everyone,
    anonymous viewers anonymous_users). The plan as it stood, three
    branches done together:
    100a. pg-bus (its own repo, released with `make release`) M:
    per-channel ids as MessageBus numbers them (1, 2, 3... per channel),
    derived rather than stored, so publishing takes no lock: a message's
    id is its ordinal in the channel in commit order plus what trim has
    removed from that channel (a per-channel trimmed count trim keeps);
    `last_id(channel)` and "after id N" as counts over the existing
    (channel, xid, id) index; and `client:<id>` audience tags for
    MessageBus's client_ids,
    100b. discourse-rs S: the pg-bus bump; the real values in the JSON
    fields that carry them (topic view's `message_bus_last_id`, chat's
    `channel_message_bus_last_id` and `message_bus_last_ids`, user
    search, `notification_channel_position`...), their parity ignores
    dropped; chat's web hook counting of backlog rows and backup logs
    sent to every admin tab replaced by the per-channel ids and client
    tags,
    100c. discourse-rs L: `/message-bus/:client_id/poll` with Rails'
    semantics (long-poll and chunked streaming, `__seq`, status messages
    for channels asked from -1, backlog replay by per-channel id, user,
    group and client filtering, its headers), measured against the
    reference's own poll responses. The port's /bus/events and /bus/poll
    (global positions, its own pages) stay as they are
193. ~~`feat/user-pages-rest`~~ (done) L: the user pages the port renders as the
    summary or refuses: preferences (every tab), notifications,
    /u/:username/badges and /activity/topics (97, ListController#topics_by),
    the drafts page, the other view_user_route values
    (routes/users.rs:433), the user badges page (routes/badges.rs:751),
    voted_by's page (routes/topic_voting.rs:170). Split by how often a
    member opens each, stacked branches `feat/user-pages-<x>`; what
    belongs to another item (avatars 131, second factor 155, external
    auth 191, user status 130, invites 192, exports 200) is refused
    naming it:
    193a. ~~`feat/user-pages-prefs`~~ (done) M: /my/* (users#my_redirect,
    which the user menu and sidebar link to), the preferences shell (the profile
    header, the preferences navigation, `form-vertical`, Save Changes
    through PUT /u with each tab's saveAttrNames, Saved!) and its four
    most used tabs: account (username and its change form, the email
    list, name, title, primary group and flair group), profile (about
    me, timezone, location, website, user fields, hide profile, default
    calendar), notifications and interface; the select-kit combo box
    and checkbox controls they share; the title, primary group and flair
    group keys PUT /u refuses (156). Done with them: users#check_username
    and users#check_emails, user_params' permit (unpermitted keys,
    lists and hashes dropped), the profile and card backgrounds, the
    array options and the plugins' options (discourse-solved's, chat's)
    PUT /u refused or ignored, the text size class (option and cookie)
    on every page, the timezone a member without one saves on boot, the
    dialog service's yesNoConfirm and custom buttons. Refused, naming
    their items:
    choosing an avatar and removing a profile image (done in 131b), the user
    status (130), associated accounts and revoking them (191), exporting
    one's data (200), featuring a topic (156), the email change page and
    the alternate emails' actions (193d, since done), logging out a
    device (193e, since done),
    subscribing to push notifications (186), chat's notification sounds
    (162), choosing a theme (258a; a color palette since 169), hiding composer
    toolbar buttons and the bio's rich editor (196, drawn in markdown
    mode), content languages (168), a multiselect user field (196's
    MultiSelect)
    193b. ~~`feat/user-pages-notifications`~~ (done) S:
    /u/:username/notifications and its filters (responses,
    likes-received, mentions, edits, links, reactions), the Filter By
    dropdown and Dismiss All, a notification marked read on its click;
    another member's go to Ember's /404. Refused: the items' avatars
    under show_user_menu_avatars (194)
    193c. ~~`feat/user-pages-activity`~~ (done) M: the activity tabs
    drawn as the default page before: topics (ListController#topics_by,
    /topics/created-by/:username.json, its page Ember's not-found) and
    read as user topic lists (no posters, the columns' sorting, DLoadMore's
    next page, the empty states), drafts (PostList with the client's
    excerpt, resume into the composer or the post, remove, bulk select
    with PostListBulkControls and Delete selected through
    drafts#bulk_destroy; another member's drafts go to one's own,
    anonymous to /latest), pending (posts#pending, deleting one's queued
    post through reviewables#destroy; another member's go to /404),
    bookmarks (the search, BookmarkActionsDropdown's delete, clear
    reminder and pin, BookmarkList's bulk select with
    BulkSelectBookmarksDropdown's clear reminders and delete through
    bookmarks#bulk and their toasts, the empty, nothing found and
    permission denied states, the next page) and bookmarks-with-reminders'
    redirect; a 403 on read or pending leaves Ember's exception page.
    Done with them: dialog.deleteConfirm, a bookmark's destroy deleting
    its reminder notifications (dependent_reminder_notifications).
    Refused, naming their items: editing a bookmark (BookmarkModal, 196),
    resuming a message draft (the message composer, 196); the user
    topic lists' bulk select button is drawn without its actions, as on
    every topic list (183)
    193d. ~~`feat/user-pages-prefs-more`~~ (done) M: preferences emails
    (chat's email frequency, the digest and mailing list controls), the
    email change page (/preferences/email and ?new, validated and saved
    as EmailController does), users_email's create, update, primary and
    destroy (EmailUpdater: authorizing the old address for staff or
    require_change_email_confirmation, the new one otherwise, the
    critical_user_email confirm_old_email, confirm_new_email(_via_admin),
    notify_old_email and account_exists mails with their To address,
    the rate limits, the screened email Levenshtein check, user_updated
    web hooks), the confirm-old-email and confirm-new-email pages and
    their PUTs (automatic groups and the bounce score reset on
    completion), the account tab's resend, make primary and remove
    actions, tracking (the category and tag choosers, watched precedence
    over muted; PUT /u's tag tracking keys), tags, users (muted users,
    ignored users with the ignore duration modal and its time
    shortcuts, the allowed PM users; users#notification_level's ignore
    level and acting_user_id) and navigation menu (PUT /u's sidebar
    keys). Refused, naming their items: confirming an email change with
    second factors (155), finding the user by external id (191)
    193e. ~~`feat/user-pages-prefs-security`~~ (done) M: preferences
    security (the password reset request, removing a password once
    another login method exists, the passkeys list, the recently used
    devices with Show all, AuthTokenModal and logging one or all out
    through users#revoke_auth_token, UserApiKeys with revoke and undo),
    ConfirmSession (users#trusted_session and #confirm_session with a
    password), apps, calendar subscriptions (CalendarSubscriptionsController
    and its bookmarks feed URLs), chat's tab (its FormKit form with the
    custom reactions' emoji pickers), and the second factor page behind a
    confirmed session (users#list_second_factors). Refused, naming their
    items: managing second factors and passkeys and confirming a session
    with a passkey (155), the ForgotPassword modal a reset code opens
    (196), the MCP authorizations list
    193f. ~~`feat/user-pages-rest-routes`~~ (done) S: view_user_route's
    other values (badges drawn in place as activity is, deletedPosts
    going to its page, a route the app lacks falling back to the
    summary), /user-badges/:username's and /topics/voted-by's pages
    (Ember's not-found page, the app having no route), the deleted posts
    page (user.deletedPosts with PostsController#deleted_posts, Ember's
    exception page for one who isn't staff) and the profile's Admin
    button for staff (displayTopLevelAdminButton). Refused, naming their
    items: the invites pages (192), view_user_route profile-hidden
194. `feat/review-pages` M: the review queue's page done (review_view.rs,
    static/js/review.js: the queue and its filters, the bundled actions
    performed and the reviewable redrawn). The user menu's review tab and
    the pending reviewables of the recent notifications are done
    (reviewables.rs user_menu_list, with bump_last_seen_reviewable). Left,
    split by what a demo clicks first:
    194a. ~~`feat/topic-admin-menus`~~ (done) M: the staff topic admin menu
    (TopicAdminMenu, the wrench in the footer, the timeline's controls and
    the progress bar) and the post admin menu (AdminPostMenu, a post's
    wrench), drawn as Ember draws them for each role, in float-kit menus
    (bottom sheets in the mobile view; static/js/topic-admin-menu.js).
    Closing and opening, archiving, listing and unlisting (PUT
    /t/:id/status), deleting (DELETE /t/:id, with DeleteTopicConfirm past
    min_topic_views_for_delete_confirm), Make Personal Message (PUT
    convert-topic/private) and both menus' Moderation History links work;
    the page is drawn again after an action, where Ember re-renders in
    place. float.js now draws a DMenu's own trigger as DMenu does (its
    content classed `<identifier>-content` and labelled by the trigger's
    id, the trigger marked expanded, the grip only on its bottom sheet).
    What the menus open that is not ported is refused, named: 194c, 194d
    194b. ~~`feat/review-claims-notes`~~ (done) M: claiming
    (ReviewableClaimedTopicsController: POST and DELETE
    /reviewable_claimed_topics, reviewable_claims.rs with the claimed and
    unclaimed histories, /reviewable_claimed and the counts' job; the
    perform's claim_error, and the topic's manual claim released when a
    pending reviewable transitions, now in every transition the queue
    ports) and notes (ReviewableNotesController: POST and DELETE
    /review/:id/notes). /review.json serializes the notes and the claim
    (claimed_bies, the claimer side-loaded); the page draws the timeline's
    notes with their delete and the claim's buttons, and review.js claims,
    unclaims, adds and deletes notes, and claims automatically before a
    confirmation (releasing it after, or on cancel), the reviewable drawn
    again from the server. Recorded on rs-ref-c (parity/writes/review:
    claim_*, notes_*, perform_*claim*, review_list_notes and
    review_list_claimed*). The modals and client actions a claim also
    precedes are 194e
    194c. `feat/topic-admin-actions` M: what the topic admin menu refuses:
    the FeatureTopic modal (pinning with its date, globally, banners:
    /topics/feature_stats, make-banner, remove-banner; its pin form
    requires a future date and topic_status.rs refuses pinning until a
    time, the unpin job, so pinning needs that first), recovering a topic
    (PUT /t/:id/recover; post_destroyer refuses recovering topics),
    selecting posts (multi-select), the ConvertToPublicTopic modal, the
    EditTopicTimer and EditSlowMode modals (their endpoints are ported),
    ChangeTimestamp (PUT /t/:id/change-timestamp), reset bump date (PUT
    /t/:id/reset-bump-date) and the nested replies toggle
    194d. `feat/post-admin-actions` M: what the post admin menu refuses:
    the staff color (PUT /posts/:id/post_type), the post notice modal
    (PUT /posts/:id/notice), unhiding (PUT /posts/:id/unhide), the
    ChangePostOwner modal (POST /t/:id/change-owner is ported), the
    GrantBadgeModal, locking (PUT /posts/:id/locked), permanent deletion
    (force_destroy), wiki (PUT /posts/:id/wiki), PublishPageModal and
    rebaking (PUT /posts/:id/rebake)
    194e. `feat/review-client-actions` M: the review queue's client
    actions (the composer's edit, suspend, silence), the reject reason
    modal and editing a reviewable (a queued post's title, category, tags
    and raw: PUT /review/:id, ReviewablesController#update), and a queued
    post's raw email (RawEmailModal). The revise modal is done
    (feat/review-queued-posts)
    194f. `feat/review-pages-rest` M: the /review/topics, /review/settings
    and /review/:id pages (the user menu's review items link to the
    last), the insights tab (IpLookup), the pin toggle and loading past
    the first ten
    194g. `feat/header-avatar-badges` S: the header avatar's
    new-reviewables and new-pms badges and show_user_menu_avatars
    (routes/user_menu.rs:51)
195. `feat/rss` M: the .rss feeds of the lists, categories, tags, topics,
    a user's posts and activity, topic voting's votes feed, and the
    bookmarks .ics calendar (session/user_api_key.rs:117; 84b). Only the
    badge feed exists (85b)
196. `feat/client-rest` L: the client's leftovers named under done
    items: the rich editor (ProseMirror) and its toggle, the composer's
    actions, heading, list, options and emoji menus, the insert-date
    modal (49, 96), the desktop header's search menu (94; the advanced
    filters panel was 143i), the do not disturb and user status modals (95),
    the drafts and post menus as sheets, the jump to post prompt (98),
    the topic map's menus, the footer's flag, mark unread and pinned
    buttons, the sidebar's header actions and Customize (milestone 4.5),
    the login, signup and logout dialogs (79c), the Edit Directory
    Columns modal (74), BookmarkModal (editing a bookmark from the
    bookmarks tab, 193c), the message composer (resuming a message draft
    from the drafts tab, 193c), the ForgotPassword modal a password
    reset code opens (from the security tab, 193e). Split when started
197. `feat/filter-route` M: /filter (ListController#filter, not routed)
    with topic voting's votes filters and discourse-solved's
    status:solved and status:unsolved; the topic_ids and state list
    filters (routes/list.rs:54; topic_query.rs:368)
198. `feat/posts-show` S: GET /posts/:id.json (PostsController#show; the
    `not_yet` case solved/post_as_admin)
199. `feat/site-json-rest` S: the /site.json keys parity/cases still
    ignores: markdown_additional_options, hashtag_configurations,
    hashtag_icons, access_control, valid_flag_applies_to_types,
    admin_config_login_routes, users_on_holiday and the workflow keys
200. `feat/user-exports` M: user and admin CSV exports (/export_entity,
    ExportCsvFile), a recent export on the admin view of a user
    (admin_user_show.rs)
201. ~~`perf/topic-view-queries`~~ (done as perf/topic-page-queries and
    perf/topic-page-chrome, BENCH.md 2026-10-16) M: the topic view's
    per-post queries (milestone 2)
202. `feat/plugins-unported` L: the plugins the reference runs that the
    port doesn't: discourse-ai, gamification, calendar and events, data
    explorer (admin_dashboard.rs:1060), automation and
    discourse-workflows (username_changer.rs:314,323;
    user_anonymizer.rs:149,159; plugins/topic_voting/votes.rs:275;
    badge_granter.rs:1132), post-voting, voice, templates,
    narrative-bot's tutorials, math and graphviz in cooking
    (crates/markdown's bbcode refusal), zendesk; their data on user
    deletion (plugins/mod.rs:232,266), reports (reports/mod.rs:860,1036),
    directory columns (routes/directory.rs:200), unsubscribe keys
    (email/unsubscribe.rs:108) and the keys parity ignores. One branch
    per plugin when started
203. `feat/plugin-tier0` M: `plugin.toml` with settings, preloaded custom
    fields, assets, i18n, the tables the plugin owns; `discourse-rs
    plugins` (PLUGINS.md, tier 0)
204. `feat/plugin-host-api` M: the host functions: keyed batch reads of
    the plugin's own tables, the Discourse JSON API in process under the
    manifest's scopes
205. `feat/plugin-runtime` L: the third-party runtime (the Luau spike,
    spike/plugin-runtime) on the lifecycle phases: `serialize`, `html`,
    `modify` batched per response
206. `feat/plugin-routes-events-jobs` M: `route` with core's session and
    CSRF, `event`, `job`; the failure policy
207. `feat/hosted-plugins` M: the same API over HTTP with a scoped key,
    events as signed webhooks
208. ~~`feat/multisite`~~ (done, 208a-d) L, moved up to follow 131 (see "Next, in
    order"): several forums from one process, by hostname
    (RailsMultisite). One AppState and Router per site, built by today's
    startup, behind a service that picks the site per request, so the
    handlers' `state.pool` and `state.bus` call sites do not change; one
    job worker per site. The sweep, with every single-site assumption
    classified and the Rails and rails_multisite conventions it follows:
    docs/multisite.md. The queue, sessions, keys and pg-bus already live
    in the site's database (owned_schema.rs, bus.rs), so jobs need no
    site id; what remains is in-process state, the `default` db spelled
    out, the hostname from DISCOURSE_HOSTNAME, and startup. Split by
    dependency, each branch stacked on the one before and mergeable with
    one site behaving exactly as now: 208a. ~~`refactor/site-state`~~ (done) M (the
    process-wide state that is a site's moved into AppState:
    request_tracking.rs's COUNTS, FLUSHER and QUEUE, whose flusher today
    writes every count into the first pool it saw; jobs/periodical.rs's
    CALLS and NEXT_CATEGORY_ID; highlight_js.rs's version cache keyed by
    site; the resolved site settings cache out of the shared Definitions
    (site_settings.rs:331, whose fingerprint two fresh sites share), the
    translation overrides out of the shared I18n (i18n.rs:37), the
    last-seen gate out of Keys (session/current.rs:33); the rate limit
    MemoryStore split into a process-wide store for the `GLOBAL::` per-IP
    lists and a per-site one, with `load` taking the `GLOBAL::` lists
    back (rate_limiter.rs:77, :312)), 208b. ~~`feat/multisite-sites`~~ (done) L
    (multisite.yml at DISCOURSE_MULTISITE_CONFIG_PATH in Rails' shape:
    db key to database, host, port, username, password, pool,
    host_names, `default` refused and kept for the env site; Config
    split into the per-site part (database, host names, pool, db key)
    and the process-wide rest; main.rs's startup per site (schema
    verify, owned schema, pg-bus, the rate limit lists, the site icons
    at boot, the worker with its scheduled jobs' first runs staggered,
    the shutdown save and flush); the dispatcher: the Host header, `__ws`
    on asset hostnames, the path's hostname on /highlight-js and
    /user_avatar (routes/highlight_js.rs, routes/user_avatars.rs), 404
    `not found` for an unknown host, /srv/status on `default`;
    `Urls::current_hostname` from the site's first host name (url.rs:50)
    and the rollups' host list from all of them (jobs/rollups.rs:172);
    force_hostname in the host_names check and top referrers
    (problem_checks.rs:345, reports/pageviews.rs:432). Sites besides
    `default` are refused at startup until 208d), 208c.
    ~~`feat/multisite-paths`~~ (done) M (the site's db key for the `default`
    spelled out in file_store.rs:15, upload_references.rs:21,
    backup_restore/store.rs:13 with creator.rs's and restorer.rs's tmp
    and upload paths, reports/pageviews.rs:505,523; the cookie key per
    request host, the salt with the host appended as
    RailsMultisite::CookieSalt does (session/cookie.rs:52), one key per
    host name made at startup; scripts/restore-backup restoring into a
    site's db key under a shared PUBLIC_DIR with the uploads remap),
    208d. ~~`feat/multisite-ops`~~ (done) M (the refusal lifted; the connection
    budget: a site's `pool`, an idle timeout on every pool, a startup
    warning when the sites' pools plus the rate limiter's 4 and pg-bus's
    listener exceed `max_connections`; a site that fails to start is logged
    and skipped (decided 2026-10-15, as Rails' `safe_each_connection`
    keeps going): the other sites serve, and its hosts answer 503 until
    a restart brings it up; deploy/'s env, Caddyfile, unit
    and install for several hosts). No multisite reference instance
    exists, so each branch is verified by two-site integration tests
    (two cloned test databases, two AppStates, from 208b through the
    dispatcher) asserting isolation: settings, translation overrides,
    sessions and cookies, bus messages, anonymous cache, per-user and
    per-IP rate limits, request counts, pageview queue, jobs and
    scheduled state, upload and backup paths; paths, URLs and cookie
    salts against the Rails source (file_store/base_store.rb,
    backup_restore/local_backup_store.rb, lib/discourse.rb,
    rails_multisite's connection_management.rb and cookie_salt.rb)
209. ~~`feat/viewer-timezone`, `feat/viewer-timezone-rest`~~ (done,
    left: chat's channel list dates (calendar), which chat.js redraws
    after the first paint; the
    admin dashboard's day ranges and the report's day dates, kept on the
    server's day; the post bookmark button's reminder title
    (created_with_reminder_generic), not ported; mails, drawn on the
    server as in Rails) M, decided 2026-10-16: dates and times in the
    viewer's timezone, as Ember draws them (the browser's zone, or the
    user's timezone option where Ember reads it), not UTC: relative and
    absolute dates in lists, topics, posts, profiles, notifications,
    admin tables and tooltips, bookmarks' reminders and the date
    pickers. Server-rendered markup keeps UTC timestamps in its data
    attributes and a small script formats them in the browser, so
    cached pages stay shareable between viewers; the first paint must
    not flash UTC text
210. ~~`docs/plugin-audits`~~ (done, docs/plugin-audits.md) M, decided
    2026-10-16: a parity audit of every bundled plugin the port has
    started (crate::plugins::STARTED). A plugin is done only when it
    has all of the reference plugin's
    features; having its markdown, a few routes or most of its pages
    makes it started, not done. Each audit compares the reference
    plugin (plugin.rb, routes, controllers, serializers, jobs, events
    and modifiers, settings and their handlers, migrations, the Ember
    UI) with the port and turns every gap into a ROADMAP item named
    after the plugin. Started: chat, poll, reactions, solved, topic
    voting, presence, local dates, lazy videos, checklist, details,
    footnote, spoiler, LTI, OAuth2 basic, OpenID Connect, Apple. None is
    done: every one has gaps. Those no earlier item covered are 211-249
    (Phase 11): core's anonymous actions, MCP server, composer messages
    and CSP that several plugins wait on (211-214), chat (215-227), poll
    (228-230), reactions (231-232), solved (233-235), topic voting
    (236-237), presence (238), local dates (239, 244), lazy videos
    (240-241), checklist (242), footnote (243), spoiler and details
    (244), LTI (214), OAuth2 basic (245-247), OpenID Connect (245, 246,
    248, 249), Apple (247). The audit names the earlier items that cover
    the rest
257. ~~`fix/topic-track-visit`~~ (done) S: a member's topic page load (or
    the JSON with track_visit) records the visit after the response
    (TopicUser.track_visit!) and loads the member's draft; a page load
    records its IncomingLink (another site's referer, a `?u=` share
    link). Left: referers other than plain http(s) URLs (175).
    Found by tests/demo_smoke.rs (opening_a_topic_records_the_visit)
258. ~~`feat/color-modes`~~ (done) M: every page's head links the light
    and dark color definitions with their media queries (all or none
    when forced) and draws the theme-color and color-scheme meta tags
    (color_mode.rs: ApplicationHelper's scheme_id, dark_scheme_id,
    dark_color_scheme?, forced_light_mode? and forced_dark_mode?), from
    interface_color_mode, the member's color_scheme_id and
    dark_scheme_id, and the color_scheme_id, dark_scheme_id and
    forced_color_mode cookies; a member's own palette is served at
    /assets/color_definitions_scheme/:id.css. The interface color
    selector is drawn in the header or the sidebar footer
    (interface_color_selector) and switches the mode
    (static/js/interface-color.js). Measured in
    parity/writes/color_modes. Choosing a palette in the interface
    preferences came with 169. Found by tests/demo_smoke.rs
    (switches_to_dark_mode). Left, split into 258a.
    `feat/color-modes-themes` S: the helpers read the default theme, as
    the whole port does; Rails reads the request's theme
    (request.env[:resolved_theme_id]: a member's theme_ids, the
    theme_ids cookie, preview_theme_id, none under safe mode), so a
    member on another user selectable theme gets the default theme's
    palettes (and its custom_homepage modifier, which decides the
    current homepage and so the list titles: homepage.rs), and the
    interface preferences refuse the theme selector
    (preferences_view/interface.rs:85; another theme draws its own
    stylesheets and palettes, /color-scheme-stylesheet refuses one at
    routes/stylesheets.rs:233). The dark logos (site_logo_dark_url and
    site_mobile_logo_dark_url in Header::Logo's light/dark picture,
    application_logo_dark_url, a category's uploaded_logo_dark) are not
    drawn, so a dark page keeps the light logos
259. ~~`fix/front-page-homepage`~~ (done) S: `/` always drew latest
    whatever the current homepage was; it now goes where config/routes.rb's
    root routes under HomePageConstraint send it (list::front_page): the
    homepage's list (latest, top, hot, unread, new, unseen, read, posted,
    bookmarks, votes) or categories#index, HTML or JSON by `?format=json`
    or the Accept header, with the short site description in the title.
    The blank homepage (anonymous, login required) stays the login page
    the login_required gate draws, and its JSON is now the 403
    not_logged_in as Rails answers. A category the viewer sees under a
    parent they can't (left out of Site's categories) now goes to /404
    as BuildCategoryRoute does; one whose parent Site leaves out (a hidden
    grandparent), or found by lazy loading, is refused, as Ember's
    titleToken throws there. Refused, not ported: a theme's custom
    homepage (and custom_homepage_crawler_route), finish_installation
    while has_login_hint, and `?format=` other than json or html (Rails
    406s rss; markdown not ported)
260. ~~`feat/theme-html-fields`~~ (done) S: the theme's html fields drawn
    into the pages (theme_html.rs): Theme.lookup_field over the default
    theme and its enabled components (transform_ids), each theme's common
    field then its target's joined with newlines, a field not baked yet
    baked first (ensure_baked!), the CSP nonce placeholder dropped with
    its attribute (the port sends no CSP). The target is the user agent's
    (mobile_device?), not the viewport's, so the anonymous cache keys on
    it too. Drawn where Rails and Ember draw them: head_tag in the head
    ahead of the page's own head and the stylesheets, header before
    `<section id="main">`, after_header in DCustomHtml "top" first in
    #main-container (not on admin pages), footer in DCustomHtml
    `.custom-footer-content` after #main-outlet-wrapper where the footer
    shows (showFooter, now apart from enable_powered_by_discourse), and
    body_tag last in the body; the no_ember layout draws head_tag after
    its stylesheets, header before its section, footer and body_tag
    after it. Measured in parity/writes/theme_html (desktop, phone and
    iPad user agents and the no_ember 404), the Ember DOM checked in a
    browser against rs-ref-a. Left, split into 260a.
    260a. `feat/theme-html-fields-rest` S: inline scripts in the fields
    (theme_html.rs:104, a restored site's baked html pointing at
    /theme-javascripts; 170) and theme_translations_lookup's script
    (170); scripts that would run in after_header or footer
    (theme_html.rs:120), which Ember inserts as HTML where they never
    run; a field baked by another compiler version is drawn as baked,
    not rebaked; scripts in header and body_tag run again on every
    boosted navigation (page.js swaps the body) where Ember's single
    page load runs them once; the port has no crawler layout, so a
    crawler gets the fields where the Ember layout has them (footer in
    `.custom-footer-content`) rather than crawler.html.erb's places;
    another theme's fields (the member's theme_ids, preview, safe mode)
    are 258a's
261. ~~`feat/ember-login-page`~~ (done) M: the login flow's pages as the
    Ember app draws them (login_page_view.rs, static/js/login.js), in the
    desktop and mobile views: /login (templates/login.gjs with
    WelcomeHeader, LocalLoginForm, LoginPageCta, LoginButtons and the
    passkey button, NoLoginMethods; the flash, logging in and the hidden
    login form, the password mask and caps lock warning, the second factor
    and security key forms, the email login link, CodeLoginForm's email
    step behind the one-time code link and `?mode=code`, NotActivated,
    the ForgotPassword modal with its emailed code step; `?redirect=` and
    `?email=` as static#show takes them), /signup (the create-account form
    and its validations, users#check_username and the new
    users#check_email, the code signup's email step, an external login's
    new account, the login buttons posting signup=true), the
    login-required front page (discovery/login-required with
    static/login.html.erb), /login-preferences, /u/account-created
    (users#account_created), /u/password-reset/:token
    (users#password_reset_show with its second factors, and
    users#confirm_email_token) and /password-reset over the login page
    when login is required. The header leaves out search, log in, sign up
    and the menu (and its icons for an anonymous reader while login is
    required), the layout carries the hidden login form, and the login
    gate lets through what UsersController skips it for. Measured
    against rs-ref-b: the normalized DOM of each page and state (blank,
    refused and second factor logins, the code form and back, the forgot
    password modal and its code step, the classic and code signups with
    their validations, an external login's new account, external
    providers, account-created, the reset page with and without second
    factors, the used link's error page, the login-required page) matches
    but for FormKit's generated ids, Ember's user and group card shells
    and the header outlets the port leaves out everywhere; computed
    layout matches in both views. The login and signup rules
    login-signup-page.scss compiles after its mobile ones moved from
    mobile.css into invites.css's order, and `.btn-large` is ported. Left,
    split into 261a and 261b.
    261a. ~~`fix/password-reset-homepage`~~ (done) S: /password-reset
    answered 500 unless login was required. It is static#show's
    password_reset modal page, whose Ember route replaces itself with
    `homepageNavigationDestination()` and opens ForgotPassword over it.
    Now a page load draws the homepage (discovery.<homepage> with the
    discovery params it knows, as new-topic's replaceWith carries them)
    with #login-data telling login.js the URL to put in the address bar
    and to open the modal, the anonymous session's token shared with the
    list's request so one `_forum_session` goes out. `?email=` sets the
    cookie the modal prefills from on both branches (the login-required
    one dropped it). Refused: over XHR or JSON, a custom or blank
    homepage, an array query param.
    261b. `feat/login-pages-rest` S: the login code steps after the email
    step (static/js/login.js:1254) wait on 250's /session/login-code;
    resending and changing the activation email (users#send_activation_email,
    users#update_activation_email, ActivationEdit; login.js:1054,1060,
    2490,2496); a password reset code's second factor step (:1322); the
    login and signup routes in read-only mode, whose alert shows on the
    homepage (routes/login_required.rs:627,864); the signup page's user
    fields (UserField, :885); the static login page's dark logo
    (application_logo_dark_url, :774, with 258a); account_created's
    DiscourseConnect provider payload (:1177). A reset with a password
    the server refuses (the same as the current one, a common one) is
    154's (routes/accounts.rs:728)

263. ~~`fix/faq-guidelines`~~ (done) S: GET /faq and /guidelines answered
    404, so the sidebar's FAQ link (sidebar.rs:779) was dead. Now
    static#show for /faq, /guidelines and its /rules and /conduct
    aliases, /tos and /privacy, each also as `.html` and `.json`
    (routes/static_pages.rs): the login wall sending the guidelines to
    /login (the legal pages stay open, and none of them are behind the
    login or required fields gates), rename_faq_to_guidelines (per user,
    UpcomingChanges.enabled_for_user?) sending /faq (without a faq_url),
    /rules and /conduct to /guidelines, faq_url, tos_url and
    privacy_policy_url sending their page elsewhere (/guidelines always
    shows the topic), the topic guidelines_topic_id, tos_topic_id or
    privacy_topic_id names (not found when unset or deleted). An XHR (what
    StaticPage.find asks for) gets show.html.erb byte for byte
    (static_page_view.rs); a page load gets the Ember route around it
    (templates/faq.gjs or tos.gjs) with its titleToken, `static-<route>`
    body class, the sidebar's FAQ or Guidelines link active, and
    WatchRead's POST /u/read-faq once a member who has not read the
    guidelines scrolls to their end. Discourse.tos_url and
    privacy_policy_url no longer link a deleted topic (Site, the about
    page, the signup disclaimer). The fresh_install seed's "Guidelines"
    topic (guidelines_topic_id 4) is the demo's page; it has no terms or
    privacy topics, so those 404 as on Rails. Measured in
    parity/writes/static (11 cases, the recorder now clearing
    Discourse's url cache between cases) and against rs-ref-c: the
    normalized #main-outlet DOM matches anonymous, as user1 and as admin
    but for Ember's user and group card shells the port leaves out
    everywhere; screenshots 0.00-0.03% desktop (/guidelines rows in
    parity/screens/pages and summary.json) and mobile. Left, split into
    263a.
    263a. `feat/static-pages-rest` S: a page load of the `.html` and
    `.json` paths, which Rails serves in its layout for Ember's router to
    not-find (routes/static_pages.rs:269); the first post's translation
    under content localization (:166); a topic without posts, a first
    post or a stored slug (:176,185,191; Rails raises or computes the
    slug); plugins' add_topic_static_page pages wait on milestone 5. The
    redirects go to Discourse.base_url's host like the port's other
    redirects, where Rails' redirect_to uses the request's host
    (routes/static_pages.rs:59)

266. ~~`fix/docked-header-topic`~~ (done) S: scrolling down a topic left
    the header as it was; Ember's docks the topic in it. Now the topic
    page carries Header::Topic::Info and the minimized logo in a template
    (header_topic_view.rs) that static/js/header-topic-info.js docks:
    the title linked back to the first post (scrolled to the top when the
    page has it, else the boosted link loads the topic's start), its
    statuses, the category badges without their parents' classes (the
    grandparent's on the desktop only), renderTags' tags without the
    list's overlap filter, and for a message (not a warning) the envelope
    to the viewer's inbox and the participants (users then groups, five
    beside tags, else ten, and Ember's +N link), two-rows as Ember
    decides it; HomeLogo minimized draws site_logo_small_url (Chrome's
    logo_small_url) or the house icon, AuthButtons leaves out sign up and
    the page title is aria-hidden while docked (titleIsVisibleOnHeader).
    When: services/header.js topicInfoVisible as ported, with the title's
    IntersectionObserver at threshold 1, enterTopic's first guess (the
    first post on the page and no later post asked for), the scroll down
    latch, and ScrollDirection (paused a second after the page comes in,
    the 5px tolerance, the 100ms throttle, the bottom of the page), so
    the mobile view undocks while scrolling up; every page sets it up
    again, so a boosted navigation starts undocked. A participant's
    avatar opens their user card (user-card.js Discourse.showUserCard).
    Stylesheets: header.scss's topic info and participants, the house
    icon, tagging.scss:130 and :97, badges.scss:84, topic.scss:165, the
    uc-modernize title size and the fadein keyframes; the `.d-header`
    gets no class of its own in this Discourse (its contents change).
    Measured with playwright against rs-ref-b (topic 35 as user1 and
    anonymous, the pinned and closed 37, the subcategory 38, the messages
    43 and 90; desktop 1280 and mobile 390): docked or not at the same
    scroll offsets (67 no, 69 yes; 71 and 73 on mobile), scrolling up
    undocks on mobile only, the normalized docked DOM matches up to the
    panel, every docked box and computed font size matches, header
    screenshots 0.00% but the avatar's new-pms badge (194), and the title
    goes back to the top. Found by the screenshot run. Left, split into
    266a.
    266a. `feat/docked-header-topic-rest` S: a member's pin toggle in the
    statuses (TopicStatus' pin-toggle-button links and
    togglePinnedForUser's PUT /t/:id/clear-pin and re-pin) is drawn as a
    span here as on the title and lists (topic_list_view.rs:1299,1305); a
    group participant's card is refused
    (static/js/header-topic-info.js:225); the small logo's dark variant
    is 258a's (header_topic_view.rs:41); FeaturedLink never draws, since a
    topic with a featured link refuses at featured_link_root_domain
    (topic_view.rs:556; header_topic_view.rs:107); a live title edit
    redraws #topic-title but not the header's template (routes/live.rs:573).
    Around it, not the header's, done in ~~`fix/topic-post-navigation`~~:
    the topic page entered at a post number (/t/:slug/:id/:n, and
    /p/:post_id(/:user_id), posts#short_link, which answered 404 and now
    redirects there: not found for a missing or trashed post, forbidden for
    one the viewer may not see, a stored slug or Unsupported) now picks the
    closest post on the page (closestPostNumberFor), runs post:highlight on
    it and jumps to it (DiscourseURL.jumpToPost: the top for the first
    post, else LockOn's second of holding `#post_N`, or the address's
    anchor, at --header-offset), so it starts docked as Ember's does; the
    address follows the post at the eyeline (currentPostChanged and
    postChangedRoute: /t/:slug/:id/:n, the bare topic URL for the first
    post, the query and hash kept, replaced once the page has stopped
    scrolling for 500ms); the progress bar's back button and the timeline's
    scroll area go to a post on the page as routeTo's navigatedToPost does
    (the address, post:highlight, jumpToPost), where the back button looked
    for an `article[data-post-number]` neither DOM has. post:highlight in
    this Discourse is highlightPost: no `.highlighted` class or animation,
    a later post's `.topic-body` gets tabindex 0 and the focus; a post
    Ember has cloaked (more than a window's height away) has no body then,
    so a jump from far keeps the focus where it was, and a topic entered
    from another page of the app (a boosted link, back and forward) focuses
    nothing, as the port now does (static/js/topic.js, page.js
    Discourse.swapped). Not copied: Ember's cloaking itself, so a focused
    post scrolled far away keeps the focus here where Ember's goes to the
    body. Measured with playwright against rs-ref-b (topic 35 with 25 more
    replies, desktop 1280 anonymous and mobile 390 as user1): entering at
    5, 25 and 99, scrolling to 12, 20, 3, a wheel step, the top and the
    bottom, the bare topic and its back button, boosted links from /latest
    and within the topic (near and far), back and forward, the address, the
    top post's offset (52px, the header), the focused body, the docked
    header, the timeline and progress counts match at each step (but the
    bottom, where the port has only the entered chunk of posts drawn and
    Ember loads more); a layout switch's redraw is not an entry and does
    not jump

262. ~~`fix/pm-topic-view`~~ (done) M: a message's topic page drew as a
    regular topic. topic.gjs's body classes (AddCategoryTagClasses'
    category and tags, AddTopicStatusClasses', `archetype-<archetype>`)
    and DiscourseTopic's archetype class now come with every topic, and a
    message draws its own chrome: PrivateMessageGlyph in the title (to the
    viewer's inbox for it, User#pmPath, for who can send messages; none on
    a warning, so the topic view JSON now has is_warning), the topic map's
    PrivateMessageMap (its groups and users, the remove buttons for who
    may remove them or themself, the add participant button for who may
    invite), the footer's share and bookmark without labels on desktop and
    Archive or Move to Inbox (PUT archive-message and move-to-inbox, then
    the inbox, static/js/topic.js), the more topics tabs (Related
    Messages and Suggested Messages with their posters, the pick kept in
    localStorage as Ember's keyValueStore keeps it) and BrowseMore's line
    from pmTopicTrackingState's counts (pm_tracking_state.rs
    browse_more_counts). Removing a participant asks first for oneself
    (dialog.js) and goes to one's messages after. Stylesheets:
    base/personal-message.scss (personal-message.css, its topic map rules
    after components/topic-map.scss in topic.css as Rails compiles them),
    the glyph's, more-topics.scss's tabs and topic-post.scss's visited
    line. Along the way: details.allowed_users comes in the order Rails'
    join returns (the rows as added, not by user id), the post stream
    draws the visited line (shouldShowVisitedLine) after the post the
    reader had read up to, and the timeline asks displayTimeLineScrollArea
    and checks its dock on topic:current-post-scrolled only, as Ember's
    container does, and starts its scroller at the entered post
    (calculatePosition), so a short one-post message keeps its scroll
    area as Ember's does. Found by the screenshot run (/t/90 as admin and
    as user1, parity/screens/pages). Left, split into 262a.
    262a. `feat/pm-topic-view-rest` S: the browse more line is drawn once with the
    page's own message read through the posts shown, not kept live from
    the pmTopicTrackingState bus messages; the title's edit pencil and
    title editing (TopicTitleEditor, the edit-title glyph's styles) are
    not drawn for any topic (the admin menu's wrench is 194a); the
    header's topic title once scrolled past, with its glyph, is 266's
    262b. ~~`feat/add-pm-participants`~~ (done) S: the add participant
    button's AddPmParticipants modal (InvitePanel,
    static/js/add-pm-participants.js): its EmailGroupUserChooser over
    /u/search/users (emails when the viewer may invite by email, the
    messageable groups, ranked as organizeResults ranks them), Send
    Invite (POST /t/:id/invite with an email's custom message, or
    /t/:id/invite-group), the server's error in the panel, and on success
    the toast and the topic map's participants drawn again from the
    server (Ember adds the user to its list). Compared on rs-ref-c (admin
    on /t/43: the modal, user, email and group rows, the selection, the
    custom message and an existing participant's error); its
    email-group-user-chooser.scss and add-pm-participants.scss rules are
    in personal-message.css
264. ~~`fix/categories-tracking`~~ (done) S: the categories page ignored a
    member's read state, found by a screenshot run (/categories as anon,
    admin, user1 and mobile). `.category__badges` was always empty; now
    CategoryUnread draws the "N unread" and "N new" links (to the
    category's /l/unread and /l/new) beside each category and
    subcategory from the page's TopicTrackingState report, counted as
    countUnread and countNew do (subcategories included, a definition
    topic only for its own category, new skipping muted categories).
    The Latest column's rows took visited from any last read post and
    linked the topic; now Topic#visited (read through the highest post),
    dTopicLink to lastUnreadUrl with its screen reader "read", and
    TopicPostBadges' unread count and new dot. `.muted-categories` sat
    38px too high (16px on mobile): the page lacked Navigation's
    `navigation-categories` body class, whose rule (category-list.scss:634)
    puts 3em under the categories table, on mobile too (after :543 as in
    the bundle).
    Measured against rs-ref-c: the normalized `.categories-and-latest`
    DOM matches as user1 and admin but for the pin toggle (264a);
    the muted box's offset from the table matches desktop and mobile
    (ui-compare and box measurements). Screenshots before/after:
    desktop anon 7.68/7.68%, admin 8.62/8.65%, user1 5.43/5.40%; mobile
    anon 9.52/9.52%, user1 5.30/4.80%; what is left of them is the
    header logo, the nav row 4px higher and text widths, none of it
    this page's. Left, split into 264a.
    264a. `feat/categories-tracking-rest` S: TopicStatus' pin toggle for
    a member (`<a class="pin-toggle-button">` with clearPin and rePin,
    PUT /t/:id/clear-pin and /re-pin, unported) is drawn as the
    visitor's span on every list (topic_list_view.rs:1264); the
    category badges don't follow live tracking updates, which refresh
    only the nav pills and the sidebar (routes/live.rs:799)
265. ~~`fix/top-period-chooser`~~ (done) S: the top lists (/top,
    `?period=`, /c/:slug/l/top, /tag/:tag/l/top) drew no period heading,
    and the port added an active Top pill Ember draws only when top_menu
    has top. Now DiscoveryTopics' `.top-lists` PeriodChooser heads every
    top list from the list JSON's `for_period` (html.rs latest_page,
    reusing directory_view's chooser, with _topic-list.scss's top-lists
    rules): the period's name and its date range in the viewer's zone
    (periodTitle with fullDay false), a row for each of Site.periods, and
    static/js/top-period.js opening it as select-kit does and going to
    `?period=` with the discovery query params in Ember's order
    (DiscoveryListController#changePeriod). ListController.best_period_for
    now takes the member's previous_visit_at (best_periods_for's daily to
    yearly ladder) where it took none. The navigation pills are top_menu's
    alone: NavItem.buildList adds the current list from `args.filterMode`,
    which DNavigation never passes, so a list the menu lacks (top, and
    bookmarks, posted, read and the rest) has no pill, and the mobile
    menu's trigger falls back to the first pill (selectedNavItem). A list
    with no topics draws no topic list (DiscoveryTopics#hasTopics). /new's
    `subset` (TopicQuery#list_new with unified new: topics is
    new_results, replies unread_results) filters the list and rides the
    next-page URL; the suggested topics' "new topics remaining" links
    already pointed there. Ember's "see more" footer for top lists is gone
    upstream (topic-list-bottom draws only the dismiss buttons and the
    empty-filter education). Measured against rs-ref-a: screenshots of
    /top, /top?period=weekly (anonymous and as user1, chooser open too),
    /c/general/4/l/top?period=monthly and /tag/guide/l/top went from
    0.99-1.30% to 0.01-0.29% (the category's 0.29% is /c/general/4's
    own), with /top rows in parity/screens/pages and summary.json; the
    chooser's computed layout matches and a row goes to the same URL.
    Left, split into 265a.
    265a. `feat/new-list-subset-toggle` S: /new's All / Topics / Replies
    toggle (NewListHeaderControlsWrapper, with unified new, which is on
    for everyone) with the member's new and unread counts within the
    category or tag, kept live, and its bulk select variant in the list
    header (templates/latest.html:25); EmptyTopicFilter's education in a
    member's empty list footer is done (270)
269. ~~`fix/new-topic-route`~~ (done) S: GET /new-topic and /new-message
    answered 404 (the PWA manifest's New Topic shortcut and embedding's
    create link point there). Now new_topic#index (routes/new_topic.rs)
    as Ember's new-topic and new-message routes leave a page load: an
    anonymous visitor gets the login page, which keeps the URL as the
    destination_url and takes /login (login.js, the login route's
    beforeModel reading the URL its transition starts from, so the
    forgot-password route's login page takes /login too, its
    ForgotPassword modal now waiting for the deferred modal.js, without
    which it never opened on a page load), and after logging in lands
    back on it; a member gets the page the route
    replaces itself with, the category asked for (`category_id`, else
    `category` by name then slug, `parent/child` under the parent, among
    the site's categories) drawn as /c/:slug/:id under the URL /c/:id,
    else the homepage (discovery.<homepage>) with the query params its
    route knows (the discovery list's and `filter`, discourse-solved's
    `solved`; only `filter` for categories) sorted and encoded as Ember's
    router writes them. Before the composer the page carries
    `#composer-open`, which composer.js reads: it replaces the URL and
    opens the composer as openNewTopic does, the title within
    max_topic_title_length, the body, the tags escaped, cut to
    max_tags_per_topic and max_tag_length and, for non-staff, kept to
    those /tags/list.json finds, the category, only for a member who may
    create topics, saving the draft once it settles. Query params are
    read as route-recognizer reads them (a pair's first two parts,
    "true" without `=`, the last of a name). A link to either route from
    another page opens the composer in place, a category asked for going
    to its page. /new-message checks a group (Group.messageable, the
    cant_send_pm and create_message_error alerts); the composer's alerts
    load dialog.js on a page without it. The composer's category chooser
    now shows a subcategory's parent badge before its own, as Ember's
    header does. "Reply as new topic" and the share-target modal open the
    composer themselves and never use these URLs. Compared with
    playwright against rs-ref-c, anonymous and as user1 and admin,
    desktop 1280 and mobile 390: 12 URLs (no params, title, body and
    tags, each category form, a missing category, a long title, staff
    tags, the message forms) give the same URL afterwards, page title,
    page under it and composer state (title, body, category, tags,
    focus, the saved draft), and the same login, logging in through the
    form back to the composer, and in-place links from a topic. Found by
    the screenshot run. Left, split into 269a.
    269a. `feat/new-topic-route-rest` S: the message composer itself
    (196; static/js/composer.js:954,970); creating a shared draft
    (CREATE_SHARED_DRAFT, :917) and a category the composer shows
    read-only (readOnlyCategoryId, :921); lazy-loaded categories
    (Category.asyncFindById and asyncFindBySlugPath,
    routes/new_topic.rs:323); a custom or blank homepage under it (:172);
    array params (`tags[]=`, :218), which Ember collects into arrays the
    composer trips over; asked for as JSON (:101); formTemplateInitialValues
    waits on the composer's form templates. Around it, not the route's:
    the lists under it ignore the discovery params they are handed
    (tags, status, q and the rest; routes/list.rs:62 reads page,
    per_page, order, ascending, state and subset), and the list pages
    draw no navigation-*, category-* or welcome-banner body classes. A
    link with a category the site doesn't have goes to the homepage
    where Ember opens the composer in place, and a link with one keeps
    the page left in history where Ember's replaceWith replaces it
    (static/js/composer.js:1062)

267. ~~`fix/header-logo-nav-offset`~~ (done) S: screenshot runs against an
    agent other than rs-parity drew the header logo broken and every
    list's navigation row 4px above the reference's. Both came from
    absolute URLs: Discourse.base_url carries the `port` setting, which
    parity/environment's DISCOURSE_PORT=3042 (rs-parity's) set, so the
    logo (site_logo_url) and the @font-face URLs pointed at whatever ran
    on 3042. Pointed at the port itself they still failed for the fonts:
    /fonts answered without `Access-Control-Allow-Origin: *`, which
    nginx.sample.conf's font location and development.rb's
    public_file_server headers send, so a page on another origin than
    the base URL's (a CDN, 127.0.0.1 against localhost) never loaded
    Inter and fell back to Arial, whose shorter line boxes made the
    breadcrumb's select-kits and the welcome banner's search 2px short
    each. The font route now sends it (stylesheets.rs font), and
    scripts/parity-screenshots says to run the port with its own
    DISCOURSE_PORT and refuses to capture when our logo doesn't load.
    With the fonts in, the categories page's last pixel row came from
    _topic-list.scss's `.table-heading` border (1px under uc-modernize),
    which topic-list.css lacked. Measured against rs-ref-a: /categories
    1.13% to 0.01% (as admin 1.29 to 0.05, as user1 1.40 to 0.08, mobile
    as user1 1.27 to 0.16), /t/999999 0.61 to 0.00, the topic pages a
    few hundredths lower, in summary.json and summary-mobile.json; the
    list controls' boxes all match in ui-compare. Left, split into 267a.
    267a. `fix/static-cors-rest` S: nginx.sample.conf's other
    `Access-Control-Allow-Origin: *` locations (/assets, /plugins,
    /javascripts, /images/emoji, uploads' images, optimized images and
    stylesheet cache) still answer without it here; nothing the port
    draws fetches them with CORS today, but a CDN in front would need it
    as the fonts did
268. ~~`fix/small-action-buttons`~~ (done) S: small action posts (closed,
    archived, pinned, unlisted, split...) drew `.small-action-buttons`
    empty, so staff had no edit, delete or recover on them. Now
    post/small-action's buttons are drawn as Ember draws them
    (post_view.rs small_action_buttons): recover when Post#canRecover
    (recoverable and can_recover, not the topic's), else edit when
    can_edit, then delete when Post#canDelete (can_delete, not deleted, a
    member, staff or not user deleted), each a flat small no-text DButton
    with its icon (arrow-rotate-left, pencil, trash-can), title
    (post.controls.undelete, edit, delete) and classes, delete with
    btn-danger; desktop and mobile alike. topic.js wires delete (DELETE
    /posts/:id with the page as its context, Post#destroy) and recover
    (PUT /posts/:id/recover), then draws the post again from
    /live/post/:id; composer.js opens the edit composer on the small
    action's custom text. Saving it reached revise's refusal of every
    post but regular ones: posts#update now skips validations for staff
    editing a small action (PostsController#update's skip_validations)
    and the revisor takes small actions, with Post's edit reason length
    check (validates :edit_reason and before_save
    ensure_edit_reason_length, which holds with validations skipped, for
    every post) and SearchIndexer's `raw.present?` (a blanked small
    action isn't indexed). Write cases recorded on rs-ref-b:
    posts/small_action_edit, _edit_blank, _edit_member (403),
    _edit_reason_limit (the request spec's 422 then 200, SPEC_COVERAGE
    row 943), _delete and _delete_recover. Compared with playwright
    against rs-ref-b: the normalized DOM of the small actions on /t/37,
    /t/39 and /t/41 as admin and user1, desktop 1280 and mobile (Pixel 5),
    matches but for the mobile avatar's 72px source (Ember's
    devicePixelRatio, every avatar's); on /t/39 as admin, delete, recover,
    edit and save drew the same DOM after each step but for 268a's
    tabindex. Found by the screenshot run on /t/41. Left, split into
    268a.
    268a. `feat/small-action-buttons-rest` S: staff deleting a small
    action with replies get Ember's reply-ids dialog (delete replies or
    not, GET /posts/:id/reply-ids, not routed): refused
    (static/js/topic.js:828; post_view.rs:1531 marks it); ~~after an
    edit is saved the composer doesn't DiscourseURL.routeTo the post~~
    (done in `fix/composer-save-jump`: every save goes to its post as
    the composer service does, through topic.js's routeToPost: an edit
    of any post (small actions, a first post retitled, under its new
    slug) or a reply replaces the address with /t/:slug/:id/:n, gets
    post:highlight's tabindex and focus when the page already drew the
    post (an edit, or a reply staged from the desktop preview, which
    the live stream brings), and jumpToPost with skipIfOnScreen
    (offsetCalculator ported); a post the page doesn't draw is the page
    around it, swapped in in the address's place without the focus,
    another topic's is that topic's page, a new topic its page; a reply
    saved from another topic's page asks the reply-where dialog
    (TopicReplyChoiceDialog, its buttons drawn by the server,
    post_view::reply_where_topic); a post sent for review shows
    PostEnqueuedModal and goes nowhere; a page entered at a post whose
    window reaches the end is loaded to the end (footer, suggested
    topics, new posts land there). Compared with playwright against
    rs-ref-c, desktop 1280 and mobile 390: URL, post top offset, focus
    and tabindex after each save matched but where noted below); Ember's
    edit warning for a localized post under content_localization_enabled
    is not asked before the composer opens, as for regular posts
    (static/js/composer.js:2390). Left from the save jump: the topic's
    pending posts (TopicPendingPostSerializer, the stream's pending post
    with its Delete) are not drawn, on load or after a queued reply; a
    reply on a page loaded to the end is either left in the window or
    locked under the header by skipIfOnScreen depending on where the
    page stands when the jump runs (Ember varied between runs too: 53px
    and 0px below the header, the port 104px); a staged reply is drawn
    by the live stream after the save rather than at the click, so the
    focus scroll can differ by where the page stands; Ember's cloaking
    drops a far post's highlight tabindex, the port keeps it; the
    dialog focuses its first footer button (Ember the dialog container)
    and the modal its body (Ember none); "reply as new topic" is not in
    the port's composer, so its save is not reached
271. ~~`fix/list-query-filters`~~ (done) M: the discovery lists read only
    page, per_page, order, ascending, state and subset, so /latest?tags=,
    ?status=closed, ?category=, ?before= and the rest of
    TopicQuery.public_valid_options returned every topic where Rails
    filters (found by the new-topic branch). Now build_topic_list_options
    (routes/list.rs) parses the query as Rack does (strings, `k[]` arrays,
    `k[x]` hashes, `tag_name` put in front of `tags`), runs
    TopicQuery.validators in public_valid_options' order (the first
    invalid one is the 400 naming it) and hands TopicQuery every option:
    before and bumped_before (days ago), topic_ids (param_to_integer_list,
    an empty one matching nothing), category (an id or a top-level slug;
    an unknown slug filters nothing and keeps the definitions),
    min_posts, max_posts, status (TopicsFilter#filter_status: open,
    closed, archived, listed, unlisted, deleted for those who may see
    deleted topics, public, noreplies, single_user, scheduled, and
    discourse-solved's solved and unsolved while it is enabled; anything
    else leaves the list alone), filter or f (liked, tracked), state (the
    topic notification levels and watching_first_post, and muted lists
    keep muted topics), search (post_search_data), tags (any of them,
    `joins(:tags)` with the eager load's one row per topic, or all with
    match_all_tags; a string splits on whitespace), no_tags,
    exclude_tag, no_subcategories, discourse-solved's `solved`; q and
    group_name are only carried along. The next-page URL carries the
    list options as the request gave them with page_params (username,
    group_name, groupname, period), escaped and sorted as Hash#to_query
    writes them. apply_ordering now follows Rails: an order outside
    SORTABLE_MAPPING is "default" (bumped_at, no pinning) where it was
    refused, the request's order and ascending apply to top, hot, new,
    unread and read ahead of their own order, and a category's default
    sort applies only to latest, unseen, unread and new without a
    `filter` naming another list (posted and bookmarks keep bumped_at).
    new_results' remove_muted no longer excludes the list's category or
    keeps a muted tag's list, and topic voting's my_votes only narrows
    latest and unseen, as their callbacks' options have it. Compared
    against rs-ref-b: parity/writes/lists (25 cases, each option valid
    and invalid, a member's state, filter and muted tag, the category
    sort, the category and top routes), all matching; tests in
    tests/latest_json.rs and tests/html_pages.rs. Left, split into 271a.
    271a. `feat/list-query-filters-rest` S: tag lists take only page,
    per_page, order, ascending, state, subset and the tags (TagsController
    merges the rest raw, blank or not, and match_all_tags=false makes an
    any tag list; routes/list.rs:113,118,123); discourse-assign's
    `assigned` (routes/list.rs:256, the plugin is not ported);
    discourse-events' sort by event start date for a category
    (topic_query.rs:455); `search` with search_ignore_accents
    (topic_query.rs:416); a before or bumped_before reaching past year 1
    (topic_query.rs:886); page_params' username or group name under
    unicode_usernames, or not a string, and plugin params given as hashes
    (routes/list.rs:465,471,479); list params Rack rejects, given as both
    a value and an array or hash, or arrays of hashes (Rails' 400 Bad
    Request; routes/list.rs:182,188,192), and tag_name that is not a
    string (routes/list.rs:208,215)
270. ~~`fix/topic-list-footer`~~ (done) S: the discovery lists (/latest,
    /new, /unread, /top, /hot, the category and tag lists) drew no
    `footer.topic-list-bottom`, found by a screenshot run: the page footer
    sat 11px high under /latest. Now DiscoveryTopics' footer closes
    `#list-area` on every list, as Ember draws it (html.rs list_bottom):
    DConditionalLoadingSpinner's `.loading-container`, empty (the page is
    never drawn while loading more), with _topic-list.scss:359's margins;
    once the list is all loaded, TopicDismissButtons' bottom row on desktop
    past five topics (showBottomDismissButtons, its buttons left, 270a);
    and EmptyTopicFilter for a member whose list is empty
    (showEmptyFilterEducationInFooter, which was 265a's): DEmptyState with
    SvgDocumentsCheckmark, the unread, unified new, new or generic
    education, "Browse latest topics" (to /latest), the tracking
    preferences tip under the base path on new and unread, and under
    unified new an empty `subset` offering the other one while the
    member's tracking state has some of it in the list's category
    (countCategoryByState with noSubcategories, now
    Tracking::count_scoped), static/js/empty-topic-filter.js sending
    either button where Ember's action goes. The list also ends with
    DLoadMore's `.load-more-sentinel`. Ember's old "There are no more
    latest topics." footer messages (topics.none.*) are gone upstream;
    nothing draws them. Measured against rs-ref-a: ui-compare finds no
    difference in the footer's or #list-area's boxes and computed styles
    on /latest, /c/general/4 and /tag/guide; an empty /unread as a
    member, desktop and mobile, matches box for box (image, title,
    button, tip, page footer). Screenshots against summary.json: desktop
    /latest 1.25 to 0.01% (the port's page 963px tall, now the
    reference's 974), as admin 1.07 to 0.06; mobile /latest 0.31 to 0.00,
    as admin 0.37 to 0.12, as user1 0.37 to 0.16, /c/general/4 0.96 to
    0.67, as user1 1.21 to 0.89, those rows updated; the rest unchanged.
    Left, split into 270a.
    270a. ~~`fix/list-dismiss`~~ (done) S: the new and unread lists (and
    their category variants) drew TopicDismissButtons' rows empty, so a
    member could not dismiss from them. Now html::dismiss_buttons draws
    them as Ember does, in the navigation controls (on every list, empty
    where there is nothing to dismiss, as DNavigation always renders it)
    and in the footer past five topics on desktop: under unified new the
    DComboButton ("Dismiss all", "Dismiss new topics" or "Dismiss new
    replies" by `subset`) whose menu, but for the topics subset, holds
    "Dismiss and stop tracking" (a bottom sheet on mobile); the classic
    "Dismiss New" otherwise; on unread "Dismiss…", which opens DismissRead
    with its "Stop tracking these topics" checkbox. static/js/topic-dismiss.js
    sends what the discovery controller sends (Topic.resetNew to PUT
    /topics/reset-new with tracked, the category and include_subcategories,
    dismiss_posts, dismiss_topics and untrack; Topic.bulkOperationByFilter
    to PUT /topics/bulk with filter=unread and dismiss_posts or a change to
    the regular level), then refreshes the page in place
    (Discourse.refresh in page.js, the router's refresh), which redraws
    the list, the nav pills' counts and the sidebar's dots. The server
    side was 188f's; five new write cases record the exact bodies the
    buttons send (valid, a category with and without subcategories,
    stop tracking, invalid filter and operation, anonymous). Compared in
    playwright against rs-ref-a, desktop and mobile, before and after each
    click (/new, its topics and replies subsets, /c/general/4/l/new,
    /unread with and without stop tracking, the bottom row with eleven
    new topics): the rows', menu's and modal's DOM, the request bodies,
    the list, pills and sidebar after the refresh, and the buttons',
    menu's and modal's boxes and styles all match. Ember's DismissNew
    modal has no caller upstream and is not drawn. Left, split into 270b.
    270b. `feat/topic-list-load-more` S: loading more on scroll (DLoadMore
    with the footer's spinner): the port pages with its "Next page" link
    instead (templates/latest.html:54); the dismiss buttons' selected
    variants ("Dismiss (n)", topic_ids) wait for the discovery lists'
    bulk select, which is drawn without its actions (templates/latest.html:36,
    183); tag new and unread lists, and so their dismiss buttons, are
    refused (src/routes/tags.rs:292); the messages lists' dismiss buttons
    are drawn without actions (src/messages_view.rs:353, 183)
272. ~~`fix/avatar-dpr`~~ (done) S: on a phone or any screen with a
    device pixel ratio above 1.1 every avatar loaded a smaller image than
    Ember's (a Pixel 5's 48px post avatar 48.png where Ember asks for
    144.png). Ember's avatarImg (discourse/lib/avatar-utils) draws width
    and height at translateSize (tiny and small 24, medium and large 48,
    extra_large 96, huge 144) and the src at getRawAvatarSize: the size
    times 1, 2 (ratio above 1.1, below 2.1) or 3, rounded up to the next
    of avatar_sizes, else the largest (DPR 3: 24 to 72.png, 48 to
    144.png, 96 and 144 to 288.png). No srcset; Rails' crawler views
    draw fixed sizes and Ember's first render already reads the ratio,
    so the visible DOM always has the scaled src. Now static/js/avatars.js
    loads in every page's head with avatar_sizes (data-sizes) and, as the
    parser, a swap or a fetched fragment inserts an img with a width
    whose avatar URL (user_avatar, letter_avatar, letter_avatar_proxy) is
    at that width or its size at ratio 1, rewrites the size segment to
    getRawAvatarSize; cooked content keeps its src (Rails cooks quote
    avatars at 48 for width 24) as do the user API key pages (a fixed
    24). The redeemed invites list drew its avatar at 20 with
    loading=lazy and no title; now dAvatar's tiny (24, the display name
    as title). Compared with playwright against rs-ref-c, desktop 1280
    at DPR 1, 2 and 3 and the Pixel 5 profile at DPR 1, 2, 2.75 and 3:
    every avatar's width, height and src size (and the loaded image's
    natural width) match on /latest, a topic, /u/user1/summary, /u,
    /about, a topic with a cooked quote, a user card opened from a post,
    a topic reached from /latest by an in-app link, anonymous and as
    user1 (/latest, the user menu, notifications, redeemed invites);
    element screenshots of the topic and profile avatars are identical
    bytes at each ratio. Found by the small-action branch (268). Left,
    split into 272a.
    272a. `fix/avatar-dpr-rest` S: chat's avatars (messages, DM
    channels, the members list) were not measured, the seed having no
    messages; the pages not listed above (search, review, groups,
    admin, badges) were checked only by their markup following the rule
    avatars.js reads (tests/html_pages.rs), not against Ember; the
    browser's preload scanner still fetches the ratio 1 image before the
    redraw, a request Ember doesn't make

273. ~~`fix/list-chrome`~~ (done) S: the discovery lists drew none of
    Ember's body classes but the categories page's, the breadcrumbs'
    selected drops lacked their border, and a leaf category's rows kept
    the category badge (found by the screenshot runs and the new-topic
    branch). Now each list adds them in the order rs-ref-b's body ends up
    with (Chrome#discovery_body_classes, which skips one already there as
    classList.add does): WelcomeBanner's `welcome-banner--enabled`, then
    Navigation's `navigation-topics` (latest, top, hot, new and the other
    filters), `navigation-categories categories-list` or, on a category's
    list, AddCategoryTagClasses' `category category-<fullSlug>` and
    `navigation-category`, with Rails' layout's `category-<slug_path>`
    (ApplicationHelper#body_classes) first on the body; a tag's list gets
    `tag-<name> tags-page`, a tag in a category `tags-page category
    category-<slug> tag-<name>`; then `welcome-banner--visible` on a
    desktop page that can search, which static/js/welcome-banner.js keeps
    as WelcomeBanner's checkViewport does (the banner's search below
    --header-offset, rechecked as it crosses the header, off below 40rem
    where the search is hidden). GlimmerSiteHeader's `staff` follows the
    upcoming change classes for a staff member on every page.
    select-kit/category-drop.scss and tag-drop.scss were never ported:
    navs.css has them now, so a selected drop takes
    --d-nav-border-color--active (rgb(0, 136, 204)) and the breadcrumb's
    badge its normal weight at --font-0, which was the 2-4px. A category
    list's rows hide the badge, desktop and mobile, under
    TopicList.hideUniformCategory (routes/list.rs hide_uniform_category:
    no subcategory the viewer can see, and not the shared drafts
    category for one who may see shared drafts, which no list reaches
    while topic lists refuse shared_drafts_category, topic_query.rs:419);
    parent categories, /none and tag lists keep it. Compared against
    rs-ref-b with playwright (anonymous, user1 and admin, 1280 and a
    390 phone): the body classes of /latest, /top, /hot, /new,
    /categories, /c/general/4 (and /none, /l/top),
    /c/general/sub-general/34, /c/site-feedback/2,
    /c/uncategorized/1, /tag/guide and /tags/c/general/4/guide match but
    for 273a's chat class, the rows' badges match on all of them, and
    ui-compare finds no difference in the breadcrumbs' boxes and styles
    on /c/general/sub-general/34, /c/general/4 and /tag/guide (mobile
    /c/general/4's drops match in width and border too). Screenshots
    (summary.json): /c/general/4 0.29% to 0.00, /c/general/sub-general/34
    0.97 to 0.00, /c/general/4/none 0.33 to 0.02, /c/general/4 as user1
    0.49 to 0.10, /tag/guide 0.03 to 0.00; mobile /c/general/4 0.67 to
    0.00, as user1 0.89 to 0.16 (on top of 270's footer); the other list
    rows held or dropped a hundredth, none got worse.
    Tests in tests/html_pages.rs. Left, split into 273a.
    273a. `fix/list-chrome-rest` S: chat's ChatBodyClass
    `has-preloaded-chat-channels`, which every member page with chat gets
    once the channels load (src/html.rs:1099 adds only chat-enabled);
    Rails' `primary-group-<name>` for a member with a primary group
    (src/html.rs:800); a tag in a category draws no breadcrumbs or pills
    (src/routes/tags.rs:568); the welcome banner and its classes on a
    category's or a tag's list under welcome_banner_page_visibility
    discovery or all_pages (only latest, the filters and categories ask
    for it, src/html.rs:1914); /tags/intersection/:tag/*additional_tags
    (`tags-intersection`) is not routed; /filter's `navigation-filter`
    comes with item 197
274. ~~`fix/profile-polish`~~ (done) S: the profile pages' gaps a
    screenshot run found. UserNotificationsDropdown (Normal / Mute /
    Ignore) drew its header spread across the button with a grey bell:
    dropdown-select-box.scss's `.user-notifications-dropdown` rules (the
    header's icons without the angle's auto margin, the wrapper
    centered) were missing, now in kit.css after the box's own; and the
    bell took `.select-kit .d-icon`'s primary-medium where Rails gives
    `.btn .d-icon`'s colour, because the port loaded select-kit.scss
    after buttons.scss while Rails imports common/select-kit before
    common/components and common/base. select-kit, single-select,
    combo-box, category-row and category-chooser's rules moved from
    composer.css to select-kit.css, right after the foundation. On a
    phone the user nav lacked UserNav's Manage user item for staff
    (`isMobileView` and `isStaff`, the desktop's Admin button's stand-in),
    so it never overflowed and overflow-controls.js (DOverflowControls
    and DHorizontalOverflowNav's revealActive, already ported for every
    horizontal overflow nav the port draws: the user nav, the activity,
    messages, notifications, preferences and invites sub-navs, the group
    nav, the admin and review tabs, search's types) had nothing to
    reveal; compared with playwright on a 390px phone against rs-ref-a
    (the user nav, preferences, activity and messages sub-navs), the
    scroll offset, edge attributes and chevrons match on load and after
    each chevron. The footer's bookmark button said Edit bookmark with
    several bookmarks in the topic: it is TopicBookmarksMenu now, its
    label and title counting every bookmark of the viewer's in the topic
    (edit_bookmark's count), its icon and classes (with-reminder,
    --post-bookmarked-only) as its getters have them, the grouped
    menu's trigger (topic-bookmarks-menu) when a post's bookmark is
    among them, BookmarkMenu's toggle for the topic's alone, its title
    carrying the bookmark's name. DUserStat's and CollapsedInfo's
    template whitespace is drawn (around the stats' values and labels,
    around the dates and between the inline-flex items of the profile's
    secondary row). UserSummary#top_categories plucks its categories
    with no ORDER BY and sorts with Ruby's stable sort_by, so ties keep
    the table's order; the port ordered them by id. The About me
    editor's 12px is 196's: Ember draws the bio in the rich editor,
    whose ProseMirror input is 49px where the port's markdown textarea
    is 61px; the toolbar's missing toggle, heading and emoji
    buttons don't change the row's height. Screenshots against rs-ref-a:
    /u/admin 0.36% to 0.00, /u/user1 0.53 to 0.00, /u/user2/summary as
    user0 0.55 to 0.03, /u/user0/summary 0.35 to 0.03, the user1
    activity and user2 solved tabs 0.11 to 0.05, the messages and
    account preferences as admin 0.10 to 0.04, mobile account
    preferences as admin 1.00 to 0.11, in summary.json and
    summary-mobile.json; the rest of pages, pages-mobile, admin-pages
    and admin-pages-mobile held (the rows that got worse scored the same
    on main's binary when rerun together: reference state). Left, split
    into 274a.
    274a. `fix/profile-polish-rest` S: the grouped bookmarks menu's
    content (its post rows and their submenu, edit and delete the
    topic's bookmark, bookmark the topic, clear all): its trigger is
    drawn and refused (post_view.rs:2300, static/js/topic.js:609); a
    single bookmark with a reminder titles the footer and post buttons
    created_generic where Ember says created_with_reminder_generic with
    BookmarkFormData's formattedReminder (post_view.rs:1399,2237); GET
    /g/:name/members as a page answers the members JSON route's 404
    where Ember redirects to the group's page, its members
    (routes/mod.rs:2467)
276. ~~`fix/composer-open-padding`~~ (done) S: with the composer open
    the page could not scroll its end clear of it: Ember's
    Composer#composeStateChanged puts html.composer-open on the document
    while the compose state is open (core's compose.scss pads #main-outlet
    by --composer-height for it) and html.fullscreen-composer while
    fullscreen, and the port set neither on <html> (fullscreen-composer
    went on the body). /new-topic scored 27.8% for the reference's 400px
    padding. composer.js now follows the composer service: open sets the
    default height (reply 255px, new topic 400px, or the kept one) as the
    model's composerHeight and --composer-height; collapse sets it to
    40px and drops composer-open; unshrink and openIfDraft put the
    model's height back; close drops both classes and the property.
    Fullscreen is its own compose state, #reply-control.fullscreen
    without .open, as composer-body's class binding draws it.
    composer-container's htmlClass composer-has-preview follows the
    composer showing with its preview. The grippie is DResizeSeparator's
    now: data-resize-axis and data-pointer-drag in the markup, a primary
    button drag reported once a frame and clamped to the composer's
    min-height and the viewport below the header (a press without a move
    is a click), the arrow keys stepping 16px and Home and End going to
    the bounds, aria-valuenow, -min and -max measured as
    resize-measurements does, body.d-resizing-ns and
    #reply-control.clear-transitions while it lasts, the dragged height
    the model's (a minimized composer opens again at it) and kept in
    localStorage once it ends. page.js kept the server's <html> classes
    on a boosted page change, dropping the composer's state with the
    on-screen keyboard's: those stay now while the composer does. Core
    rules that read them and were missing: compose.scss:740's touch
    block (the progress bar above an open composer, the keyboard-visible
    composer), topic-footer.scss:116, discourse.scss:634, and the
    d-pointer-drag and d-resize-separator rules. Compared with
    playwright against rs-ref-c, desktop 1280x900 and a 390px phone:
    the html and #reply-control classes, --composer-height, #main-outlet's
    padding and the composer's box through open, minimize, expand,
    fullscreen and back, three grippie drags (up 120px, past the minimum,
    past the maximum), minimize and expand after a drag, close, and a
    page change with the composer open and minimized, all equal; with
    the composer open on a topic and on /latest, scrolled to the bottom,
    the last post's, the footer buttons' and the last row's distance to
    the composer's top equal. Screenshots against rs-ref-c: /new-topic
    as user1 27.8% to 0.47 (the rest is the rich editor, 196), the
    topic's reply composer 1.50 to 1.07, /latest's 0.63 to 0.64. Tests in
    tests/new_topic.rs. Left, split into 276a.
    276a. `fix/composer-open-padding-rest` S: the saving compose state
    (#reply-control.saving, composer-open dropped while a post saves) is
    not modeled, composer.js keeps .open and disables the button; a
    closed composer keeps its composer-action-* and topic classes where
    composer-body drops them; the topic timeline's composer:resized
    sizing (topic-navigation's heightQuery, the scrollarea's height)
    does not read the composer; footer-nav.scss's offset waits for the
    footer nav, which the port does not draw
278. ~~`fix/theme-screenshot-files`~~ (done) S: /admin/config/customize/themes
    drew Foundation's and Horizon's cards without their screenshots: the
    seed's uploads 1 to 4 (original/1X/<sha1>.png) are the files
    SystemThemesManager.sync! (db/fixtures/600_themes.rb) stores on a
    fresh install, ThemeScreenshotsHandler passing each screenshot
    about.json lists through UploadCreator unchanged, and they existed
    only on the reference's disk. The four themes/*/screenshots files
    are vendored at the schema's commit (scripts/vendor-discourse; their
    sha1s are the seed rows'), and a site's boot writes each one whose
    theme_screenshot_upload_var upload has the vendored file's sha1 and
    no file in the local store yet (crate::system_themes, next to the
    site icons' ensure_optimized), so /uploads serves them as Rails'
    local store does. The design wizard card's rules
    (design-wizard.scss:878, the .admin-config-area-themes__design-wizard
    block and its description) are ported after form-kit, where common.css
    has them; the panel's rules wait for the panel. Compared with
    playwright against rs-ref-d at 1280 and 390px: the card's and the
    description's computed styles and boxes and the cards' images (src,
    natural size) equal. Screenshots against rs-ref-d: desktop 1.37% to
    0.04, mobile 5.81% to 0.07 (the page's height now equal). Test in
    tests/theme_screenshots.rs.
277. ~~`fix/content-list-css`~~ (done) S: /admin/badges was 3.9-4.4%
    off on desktop and 8-11% on a phone. Core's `.content-list`
    (common/base/discourse.scss:743) was in none of the port's
    stylesheets, so `ul.admin-badge-list` took the browser's 20px
    padding and its rows were unbordered and 28px tall; it is in
    menus.css after discourse.scss:735's spinner, its slot in Rails'
    cascade, so the badge list and the web hook events list (the port's
    two users; Ember's third, tag-groups.gjs, is not drawn yet) pick it
    up. A system badge's selects carried `disabled` already but drew as
    enabled: discourse.scss:505's disabled and read-only controls
    (`input, select, textarea` with `[disabled]` or `[readonly]`: the
    muted text, background and border, not-allowed cursor) and its
    `:focus:required:invalid` rule are in foundation.css. The bulk award
    form's file input was 2.4px taller than Rails': the port's
    normalize.css gave `::-webkit-file-upload-button` the upstream
    `-webkit-appearance: button`, where Discourse's normalize.scss says
    `appearance: auto`, so the button took the inherited line-height.
    Compared with playwright against rs-ref-c as admin (the list, its
    rows, links and active link, the disabled badge type select and its
    field), desktop and phone: every box and style matches.
    Screenshots against rs-ref-c (admin-advanced-pages and its mobile
    list, main's binary then the branch's): /admin/badges 3.92% to 0.03,
    /admin/badges/1 1.10 to 0.04, /admin/badges/new 1.08 to 0.05,
    /admin/badges/award/new 3.92 to 0.03, /admin/badges/award/1 4.36 to
    0.07; mobile 8.60 to 0.11, 9.93 to 0.22, 8.24 to 0.29, 8.12 to 0.12,
    10.71 to 0.19. admin-pages and admin-pages-mobile held; the rows
    that improved are in summary-admin.json and summary-admin-mobile.json.
    The rows that moved the other way move on main's binary too between
    runs: the admin sidebar's scroll offset (sidebar.js's
    revealActiveLink lands 4px apart from one load to the next on
    /admin/config/flags, permalinks, user-fields, rate-limits and
    screened IPs), the reference still loading /admin/api/keys/new, and
    the dashboard's dated charts on a phone. Left, split into 277a.
    277a. `fix/admin-badges-form-rest` S: the badge form on a phone
    (/admin/badges/new 0.29%, /admin/badges/1 0.22): the `(optional)`
    label's spacing beside the description titles, the icon picker's
    caret, and the Group combo-box's width. The rest of the port's
    normalize.css still differs from Discourse's normalize.scss as Rails
    compiles it (`text-size-adjust`, `font-family: monospace` without
    the doubled family, abbr's `underline dotted`, `appearance: auto` on
    the buttons and `appearance: textfield`/`none` on search inputs);
    each touches every page, so each wants a full sweep. The sidebar's
    revealActiveLink offset varies between loads (above), which makes
    those admin rows flake.
280. ~~`fix/chat-dm-thread-pages`~~ (done) S: GET /chat/direct-messages
    and /chat/threads answered 405 with a blank page: POST /chat/:id
    (Chat::Api::ChannelMessagesController#create) matched both paths, and
    neither page was routed. Both are full-page chat now, as Ember draws
    them. The DMs page (ChatDirectMessagesRoute, ChannelsListDirect; its
    beforeModel's desktop redirect to the last channel finds no DMs yet
    on a page load, so the reference stays on the list): the navbar with
    its open drawer button, the section's
    divider with the filter toggle and list options, and each DM's row
    (ChatChannelRow: the first user's avatar or the member count, the
    title, the unread indicator, urgent for a DM, the last message's
    date, the leave button, which chat.js sends to
    DELETE .../memberships/me/follows); in the mobile view the section's
    buttons in the navbar, the divider only without public channels,
    each row's last message excerpt (dReplaceEmoji) and the chat footer
    with DMs active. Rows follow the member's DM list filter and sort,
    fifty at most; without DMs the empty state with its Start a
    conversation button. The threads page (ChatRoutesThreads,
    UserThreads): the navbar and, as GET /chat/api/me/threads answers
    when the member has no thread to list, the list's empty state, and
    the footer on a phone. GET /chat/api/me/threads
    (CurrentUserThreadsController, Chat::LookupUserThreads) answers
    on_model_not_found's `{success, threads: []}` without a viewable
    thread (Chat::Thread.viewable_by_user) and is refused with one. /chat
    follows chat_preferred_index (ChatIndexRoute#redirect): the DMs page
    for direct_messages, or without public channels, the browser
    otherwise, refusing my_threads for a member with threads; on desktop
    it goes on to a last channel that is a DM too (it went to the
    browser). A DM (any
    channel) now draws the last visit line on its last message too:
    processMessages skips it only when the message `===` the channel's
    lastMessage, and the message it just created never is, so the line
    goes on the first message past the membership's last read one (none
    without a membership); verified on rs-ref-a with a fresh DM whose
    unread message is its last. chat.js moves `-last-read` to the message
    it marks read, as the membership's lastReadMessageId moves in Ember.
    Compared with playwright against rs-ref-a on its database with a 1:1
    DM (one unread message), a group DM and an empty DM (user1) and a
    member without DMs (user0), desktop and mobile: the pages' main
    outlets equal after normalizing but for the port's data-at and
    data-href hooks, the channel page's messages too (the line on the
    last message on both). Write cases parity/writes/chat/me_threads*;
    tests in tests/plugin_pages.rs. Left, split into 280a.
    280a. `fix/chat-dm-thread-pages-rest` S: the threads list with
    threads (UserThreads' rows, ThreadPreview, the load more; refused at
    plugins/chat/view.rs threads_route and routes/chat.rs me_threads,
    with 160's threads), the Start a conversation button's new message
    modal (222), the DM list's filtered empty state
    (ChatSidebarChannelListFilterEmptyState, refused), long press and
    swipe on a row (99), the filter toggle's and list options' menus
    (99). On rs-ref-a's headless Chromium `(hover: none)` matches on
    Discourse's pages (not on about:blank or the port's), so the
    reference shows the leave button without a hover; the CSS is the
    same and the port shows it on hover, as on a desktop with a mouse.
281. ~~`fix/topic-post-number-route`~~ (done) S: /t/35/3 answered as
    topic 3 (a 301 to its canonical URL) and /t/35/last and
    /t/:slug/:id/last were refused: the router had no
    `t/:topic_id/:post_number` route, so two numbers matched
    `t/:slug/:topic_id`. Rails declares `t/:topic_id/:post_number` and
    `t/:topic_id/last` (post_number 99_999_999) first, so show_with_slug
    now reads two numbers, or a number and `last`, as a topic and a post
    number, with params[:topic_id] passed to show explicitly; show_post
    maps `last` too. The page without a slug redirects to the canonical
    URL with the post kept, JSON answers 200. Recorded as
    topics/show_topic_and_post_number (rs-ref-b); tests/topic_json.rs.

### Phase 11: the bundled plugins' gaps (210)

From the plugin parity audit (docs/plugin-audits.md, 2026-10-17): every
gap a started plugin has against the reference plugin that no earlier
item covers, as branches named after the plugin. The audit's section for
each plugin names the Rails file:line of every gap and the existing items
it leaves to (160-164, 120, 133, 143, 148, 150, 167, 168, 188i4, 195, 196,
197, 200, 202). A plugin is done when its section's gaps and these items
are all crossed off; none is today. Four core pieces several plugins
wait on come first (211-214). 39 items: 16 S, 1 S/M, 18 M, 4 L.

211. `feat/anonymous-actions` M: core's AnonymousAction: the
    `anonymous_action` cookie, POST /anonymous-actions and its replay in
    log_on_user (session/login.rs:25-27; routes/user_api_key_auth.rs:1167;
    routes/invite_redeem.rs:841, all refusing the cookie today), with the
    three plugin handlers and their client deferrals: poll's `vote_poll`
    (single choice only, DiscoursePoll::Error swallowed; the item 39
    note), discourse-reactions' `react_to_post` with the picker usable
    logged out instead of the plain /login link
    (plugins/reactions/view.rs:181), topic voting's `vote_topic`, doing
    nothing on a closed or archived topic (plugins/topic_voting/view.rs:84)
212. `feat/mcp-server` L: the port has no MCP server and no item named
    one. Since the audit, the admin side landed with 90g6
    (feat/admin-mcp-backend, feat/admin-mcp-clients,
    feat/admin-shell-config-mcp: Admin::McpController's overview, access
    rules and primitive policies over the recorded primitives, the
    authorizations page and revoking, the OAuth clients, the activity
    report); left: core's MCP endpoint itself with its OAuth
    authorization flow (184 names the authorizations' deletion) and
    register_mcp_tool's registry, then the
    bundled plugins' tools: chat's chat_channel_list,
    discourse_get_chat_messages and chat_message_create
    (plugins/chat/plugin.rb:660-753), discourse-reactions'
    discourse_reactions_post_reaction_set, discourse-solved's
    discourse_solved_solution_set and topic voting's
    discourse_topic_voting_vote_set, each with its scope, output schema
    and availability. Split core from the tools when started
213. `feat/composer-messages` M: GET /composer_messages
    (ComposerMessagesFinder), not routed, with discourse-solved's
    check_topic_is_solved education message and
    disable_solved_education_message
214. `feat/csp` M: the content_security_policy settings and header with a
    per-request nonce (only the problem check reads the setting,
    problem_checks.rs:227), and discourse-lti's resubmit page script
    taking that nonce (plugins/lti.rs:54; routes/omniauth.rs:676-693) so
    its auto-submit runs under strict-dynamic
215. `feat/chat-channel-settings` M: a channel's info pages and what they
    edit: /chat/c/:slug/:id/info, /info/settings and /info/members
    (routes/chat_pages.rs:410 404s them; the navbar title,
    plugins/chat/page.rs:544, and the card's members, :1791, link there),
    the name, slug and description modals, auto join, channel-wide
    mentions, PUT notifications-settings/me (levels and mute), PUT
    .../status with its publish, DELETE /chat/api/channels/:id
    (TrashChannel, Jobs::Chat::ChannelDelete), leaving, the members list
    with add members and invites (POST .../invites, chat_invitation
    notifications), PUT /chat/user_chat_enabled/:user_id, the global
    channel subscriptions (new channel, status, edits, kick;
    routes/live.rs:146), deleting a category whose channel is empty
    (category_writes.rs:2876), editing messages of deleted channels
    (plugins/chat/modify.rs:107)
216. `feat/chat-channel-create` M: POST /chat/api/channels
    (CreateCategoryChannel, the auto join limiter) with the create
    channel modal behind the browse page's New channel button
    (plugins/chat/page.rs:1624), GET category-chatables/:id/permissions,
    archiving (POST archives, ChannelArchiveService,
    Jobs::Chat::ChannelArchive, chat_allow_archiving_channels,
    chat_archive_destination_topic_status), the channel list options
    menus (plugins/chat/view.rs:350,732,739: create, browse, new DM, new
    group, filter, sort) and the sidebar channel hover menu (:788),
    channel and DM validation errors (plugins/chat/dm_groups.rs:498;
    plugins/chat/dm_create.rs:285). 162's archived channel reads land
    with it
217. `feat/chat-moderation` L: flagging (POST messages/:id/flags,
    FlagMessage, ReviewQueue) with the flag menu entry and the flag mark
    in the info line (static/js/chat.js:1468 drops the entry;
    plugins/chat/page.rs:1228's data is drawn nowhere),
    Chat::ReviewableMessage's actions and its queue entries
    (review_list.rs:570, with 148), auto silence from flags
    (chat_auto_silence_from_flags_duration,
    chat_silence_user_sensitivity), select mode with bulk delete (DELETE
    .../messages, TrashMessages) and move to channel (POST
    messages/moves, MessageMover, MoveMessageToChannel), the
    self_flagged bus message (static/js/chat.js:1805)
218. `feat/chat-message-actions` M: the message actions left after 217:
    select mode's quote to topic and copy (ChatToTopicSelector, POST
    /chat/:id/quote, TranscriptService), the reaction users popover (GET
    /chat/:id/:message_id/reactions-users), stop streaming (DELETE
    messages/:id/streaming), message interactions (POST
    messages/:id/interactions, the :chat_message_interaction event), the
    composer's group mention warnings (GET /chat/api/mentions/groups),
    copy text and the mobile long-press action sheet, the author's name
    under enable_names (plugins/chat/page.rs:1449), the
    DuplicateMessageValidator's exemptions for bots, upload-only messages
    and small DMs (plugins/chat/create.rs:257; plugins/chat/modify.rs:363,
    which check every message today), non-ISO target dates
    (plugins/chat/messages.rs:1027), edits too large to diff
    (plugins/chat/modify.rs:431)
219. `feat/chat-pins` S: GET pins, PUT pins/read, POST and DELETE
    messages/:id/pin (ListChannelPins, PinMessage, UnpinMessage,
    MarkPinsAsRead, chat_pinning_messages_allowed_groups) and their
    publishes, the pins route (routes/chat_pages.rs:412 404s it), the
    pin and unpin menu entries (static/js/chat.js:1468), the pinned
    message bar and the pin and unpin bus messages (:1805)
220. `feat/chat-search` S: GET /chat/api/search (SearchMessage over
    chat_message_search_data, which posting/search_index.rs:631 already
    writes and nothing reads); the /chat/search page stays with 162
221. `feat/chat-admin` M: chat's admin plugin page
    (routes/admin_plugins.rs:110)
    with its configuration nav; incoming webhooks: POST /chat/hooks/:key
    and /:key/slack (SlackCompatibility, the per-hook rate limit,
    chat_debug_webhook_payloads), the six /admin/plugins/chat/hooks
    routes with the list, new, edit and delete pages; the messages
    export (POST /chat/api/admin/export/messages, Chat::MessagesExporter,
    log_entity_export, with 200); the category editor's Chat tab
    (category_edit_view.rs:369). Drawing webhook messages stays with 162
    (plugins/chat/messages.rs:598)
222. `feat/chat-new-message` M: /chat/new-message and the new message
    modal (the message creator's chatable search, new group, add
    members), the Cmd/Ctrl+K quick palette, the user card's and
    profile's Chat buttons (user_card_view.rs:338;
    user_profile_view.rs:485), the browse empty state's button
    (plugins/chat/page.rs:1705)
223. `feat/chat-drawer` L: the chat drawer in the below-footer outlet with
    its routes (browse, channel, info, pins, DMs, starred, search), its
    size and resizer, the full page or drawer preference and the header
    icon toggling it, the navbar's open-drawer, full page and close
    buttons (plugins/chat/page.rs:572,1630), the "-" and Esc shortcuts,
    kept across page loads (milestone 4.5's "the chat drawer next"),
    ChatHistory's back-to-forum URL (routes/chat_pages.rs:118). Split
    into the slices below, in the order a demo needs them.
    223a. ~~`feat/chat-drawer`~~ (done) M: the drawer a demo walks
    through. Every page draws chat's below-footer outlet
    (ChatDrawerOutlet: `.below-footer-outlet.chat-drawer-outlet` and its
    container after the theme's footer, for visitors too while the
    plugin is on), kept across page swaps with hx-preserve as Ember keeps
    the drawer across routes, and lost on a reload as Ember's state is.
    static/js/chat-drawer.js keeps ChatStateManager's state (open,
    expanded, the preferred mode in localStorage
    discourse_chat_preferred_mode, the last chat and forum URLs,
    lastKnownChatURL's starred default), ChatHistory (the routes visited,
    which the channel's back button follows) and the chat route's
    beforeModel: a navigation to a chat URL the drawer handles, from a
    page that isn't full-page chat, opens it in the drawer while the
    drawer is preferred (desktop without FULL_PAGE_CHAT): the header icon
    (ChatHeaderIcon#openChat: its target URL and href, active while the
    drawer is open), the sidebar's channel links, the drawer's own links,
    footer and empty state button, any boosted link; otherwise the page
    goes to full-page chat and the drawer closes, and full-page chat's
    activate and deactivate keep the forum and chat URLs. The navbar
    toggles on a click (Navbar @onClick), and its toggle (collapse and
    expand), full page and close buttons act as ChatDrawer's; full-page
    chat's open drawer button goes back to the last forum page and opens
    the drawer at the last chat URL. "-" toggles the drawer outside
    inputs and Esc closes it, after the composer, menus and the emoji
    picker take theirs. The drawer's size comes from
    discourse_chat_drawer_size_* (400 by 530), and it sits beside or
    above an open composer (_checkSize's --composer-right and
    above-composer, following the composer's class and size). The
    expanded drawer's channel is the sidebar's active link
    (chat-sidebar.js classNames), and the mobile chat header's back to
    the forum link is ChatHeader's previousURL (the last forum page after
    the first; it was ChatHeader, not ChatHistory). The routes,
    server-rendered by GET /live/chat/drawer (plugins/chat/drawer.rs, the
    routes visited and the channel in response headers), expanded or
    collapsed: chat (ROUTES.chat's redirect to the DMs for
    direct_messages or without public channels), channels
    (ChannelsListPublic on desktop with its divider, or its empty state
    offering the browser), direct messages (titled Chat), starred
    channels (the starred section by its list preferences, a DM with its
    leave button; to the channels without any), browse and its tabs, a
    channel and a message in it (ChatChannel as full-page chat draws it,
    the slug unchecked as chatChannelsManager.find ignores it), each with
    the drawer's navbar (the back button and full page button while
    expanded; collapsed, a channel's title unlinked and none of its
    buttons) and the footer on the lists. chat.js runs a channel in the
    drawer (loaded with its scripts when the drawer first opens): its
    refresh draws the drawer's route again, presence is entered while
    the drawer is open, and the composer takes the focus when the
    channel is drawn, not when a page swaps in beside it. Compared with
    playwright against rs-ref-a (user1, desktop and a phone, a visitor):
    the outlet and drawer markup equal at every step after normalizing
    whitespace and the port's hooks (the outlet's id and hx-preserve,
    data-href on the footer and the empty state button, the channel's
    data attributes and templates), as do the html and body classes,
    the drawer's box, the URL, the preferred mode and the sidebar link,
    through opening from the header icon and the sidebar, a channel,
    collapse and expand by button and by the navbar, a forum page swap,
    the back button, a link to the browser, Esc, "-", the full page and
    open drawer buttons, the full page preference's icon and sidebar
    links, a reload, the composer opened beside it, and messages sent
    from the drawer and arriving live across a page swap. Tests in
    tests/plugin_pages.rs. Refused (Unsupported, by drawer.rs's route
    table): the threads, search, channel threads, thread, message in a
    thread, pins, channel settings and members routes (223c), a channel
    the member can't open (Ember pops the request's error over an empty
    route), the threads index ROUTES.chat redirects to for my_threads,
    and the lists' filtered empty states and the threads list button as
    full-page chat refuses them. Left, split into 223b-d.
    223b. `feat/chat-drawer-resize` S: the resizer (chatResizableNode on
    `.chat-drawer-resizer`: dragging the drawer's top left corner,
    ChatDrawerSize's 250 by 300 minimums written to
    discourse_chat_drawer_size_*), and the composer's live resize
    (composer:div-resizing, resize-started and resize-ended with
    clear-transitions); chat-drawer.js reads the stored size and follows
    the composer's open and closed states already.
    223c. `feat/chat-drawer-routes` M: the drawer's other routes, each
    with its full-page counterpart: a channel's settings and members
    (channel info, with the channel title's and the browse cards' member
    links), pinned messages, a channel's threads and a thread (with 160's
    threads), my threads (ChatDrawerRoutesThreads), search (with chat
    search); refused at plugins/chat/drawer.rs recognize.
    223d. `fix/chat-drawer-history` S: the browser's back and forward to
    a chat URL while the drawer is preferred (Ember aborts the transition
    and opens the drawer; the port's history restore loads full-page
    chat, htmx skipping htmx:beforeRequest), and the focus a forum
    navigation takes from the drawer's composer on the reference (the
    port's stays in it).
224. `feat/chat-pane` M: the channel pane's paging and jumps: older and
    newer messages on scroll, jumping to and highlighting a target
    message, the scroll-to-bottom arrow (plugins/chat/page.rs:256),
    /chat/message/:id, the /chat/channel/... redirects, a channel
    without a slug (routes/chat_pages.rs:406) and a target message that
    can't load (plugins/chat/page.rs:126); reactions without an emoji
    image (:1478), the retention reminder's dismiss (:510), chat notices
    with mention_without_membership's invite (:173), the composer's "+"
    menu (:811: GIFs, local dates, the mobile emoji button), the
    in-channel filter (:565), the stop-streaming button, the message
    decorators chat.js lacks (spoiler, hashtag icons, code block
    buttons; static/js/chat.js:170), the channels list without a member
    or chat options (plugins/chat/view.rs:319,322)
225. `feat/chat-notify-client` S: desktop notifications for chat, the new
    message sound (ChatAudioManager), the title counter,
    ChatNewMessageAnnouncer under chat_announce_new_messages, the chat
    keyboard shortcuts (channel switching, composer formatting, Shift+Esc
    to mark all read) and their section in the shortcuts help
226. `feat/chat-site-integration` M: chat's keys on core: /site.json's
    markdown_additional_options.chat (emitted as `{}`),
    hashtag_configurations (chat-composer, the channel type in
    topic-composer) and hashtag_icons.channel (site.rs:454,1024), GET
    /notifications/totals with chat_notifications (not routed),
    chat_quoted notifications from posts quoting messages
    (PostNotificationHandler) and the transcript post stripper, the
    remap_hashtag job with chat's MessageHashtagStore (no handler;
    category_writes.rs:2461; tag_writes.rs:274), secure_uploads' change
    handler and the chat-composer public upload type, the chat_summary
    unsubscriber (email/unsubscribe.rs:108), the chat:create_message API
    key scope, the four stats, Chat::Seeder, and chat's keys dropped
    from the parity ignore lists (parity/cases:12-15,97-116,163-166,
    247-249,321-327) with cases for them
227. `feat/chat-pages-rest` S: the small refusals no item names: DMs
    missing their row or with multi-valued custom fields
    (plugins/chat/direct_messages.rs:63,319), DM titles without a viewer
    or with every participant deleted (:132,393;
    routes/chat_pages.rs:388; plugins/chat/notify.rs:409;
    plugins/chat/create.rs:568), a web hook whose message is gone
    (plugins/chat/web_hooks.rs:37), a site without its system user
    (plugins/chat/dm_groups.rs:323)
228. `feat/poll-ui` L: poll has no client: nothing in static/js or the
    templates draws a poll, so a post shows its cooked list with a "0
    voters" stub. Split into the slices below. Export results waits for
    data explorer (202).
    228a. ~~`feat/poll-ui-vote`~~ (done) M: each cooked `div.poll` of a
    post in the topic's stream is replaced by Ember's widget
    (attachPolls: not one whose parent is a blockquote, a name the post
    has; its `.poll-title` and data-poll-dynamic carried over), drawn by
    the server from the post's polls and polls_votes
    (plugins/poll/view.rs) in the state the component starts in
    (initialShowResults, the saved vote), and again by GET
    /live/poll/:post_id/:poll_name in the state static/js/poll.js
    brought it to (the vote in progress, the results toggle, amending,
    kept per post and poll as Ember keeps them on the poll object across
    redraws). Polls in an expanded quote are drawn for the quoted post,
    whose topic is never archived as `post.quoted` has no topic. Ported:
    PollOptions for single, multiple and number polls (the radio and
    checkbox icons, a click on a local date left to its popup),
    toggleOption (a single choice cast on its click, its icon at once,
    clicked again to remove or keep the vote; a multiple choice's picks
    kept until Cast Votes), castVotes and its error (the pick kept,
    popupAjaxError in dialog.js), removeVote, amendVote and keepVote,
    the results toggle, PollVotedChoices for results that wait for the
    close or staff, the voting groups alert, PollInfo (the voters and
    total votes counts, the dynamic, vote recorded, multiple help,
    automatic close, closed by, results on vote, on close, staff only and
    public hints; the countdown as moment().to(close, true) and the
    closed age as relativeAge with addAgo, kept current by dates.js; the
    title in UTC as `moment.utc(...).format("LLL")` draws it), the
    buttons (keep vote, Cast Votes with its update label, remove vote,
    amend, hide and show results with their conditions),
    PollButtonsDropdown's buttons (one as a button, more behind the gear
    in a float-kit menu), the `/polls/<topic_id>` subscription (each of
    the post's widgets drawn again, after a redraw on its way when there
    is one), and poll.scss (static/css/poll.css, Rails' compiled poll and
    poll_desktop CSS, equal but for whitespace with the desktop rules
    under `(width >= 40rem)` as Rails links them). Measured with playwright
    against rs-ref-d on a topic with a public single choice, a multiple
    choice closing in 2099 and an on_close poll, as user1, the author,
    admin and anonymous: the normalized widget DOM equal at every step
    (vote, hide results, remove, picks, Cast Votes, amend, keep, both
    polls closed by the author, a vote by another member arriving live,
    the polls of an expanded quote, the gear menu's content) but the
    results body, and the widgets' screenshots pixel-equal at 1280 and
    390. Tests in tests/plugin_pages.rs. Refused, said in place ("Not
    ported yet: ..."): the results body wherever the widget shows the
    results (plugins/poll/view.rs:566: PollResultsTabs, PollResultsPie
    and the number poll's average, 228b), ranked choice's options
    (view.rs:628, 228b), and the dropdown's actions, which alert when
    clicked (static/js/poll.js:452: the tally toggle, 228b; close and
    open, the breakdown, 228c; export, 202). An anonymous click goes to
    /login (poll.js:321) where Ember defers the vote with
    deferAnonymousAction (211).
    228b. ~~`feat/poll-ui-results`~~ (done) M: the results body is drawn
    where 228a refused it (plugins/poll/view.rs): PollResultsTabs (Votes
    unless ranked choice is private, Outcome for ranked choice, a public
    ranked choice starting on Outcome, the tab kept while the results
    show), PollResultsStandard (the options by votes then by their HTML
    as JavaScript compares strings, even-round percentages from
    lib/even-round.js and floored ones for a multiple choice, the bars,
    the chosen class from the vote, the tally as `poll.votes` once
    toggleDisplayMode switched it, no figures or bars for ranked choice),
    PollVoters and PollVotersRankedChoice (the public voters at
    getRawAvatarSize(24), the placeholder without a template, ranked
    choice's groups in Object.keys order with Abstain's ban icon, the load
    more while an option has more votes than voters drawn, its spinner
    while loading), PollResultsRankedChoice (the rounds table, the winner
    or the tie with its candidates), the number poll's averageRating
    (round(x, -2) as decimal-adjust shifts it through the string), and
    PollResultsPie's canvas and legend, filled by static/js/poll.js with
    Chart.js loaded on the first pie as loadChartJS does (lib/chart-colors'
    getColors with the --poll-pie-color-N fallbacks to the cool gradient,
    the HTML legend plugin with buildLegendIcon swatches and the click
    that hides a slice, the legend decorated). Ranked choice's options are
    drawn too (PollOptionRankedChoice: the rank DMenu with Abstain and
    each rank, the highest and lowest priority named, every option given
    a rank as _toggleOption does, the login button for anonymous readers).
    poll.js keeps the component's state across redraws (showTally, the
    active tab, preloadedVoters once a load more expanded them, as
    voterListExpanded), fetches /polls/voters.json a page at a time
    (FETCH_VOTERS_COUNT, deduplicated by username, a regular poll's voter
    dropped from the other options, popupAjaxError or
    poll.error_while_fetching_voters), and leaves the gear menu open
    across the tally toggle as Ember does, its content the other mode's.
    Measured with playwright against rs-ref-d on a topic with a public
    regular poll (30 voters, 27 on one option), a public multiple choice,
    a number poll, a pie, a public and a private ranked choice, as user1,
    user2, admin and anonymous: the normalized widget DOM equal at every
    step (results shown, the tally both ways from the button and the gear
    menu, load more on a regular, multiple and ranked choice option, the
    tabs both ways, results hidden and shown again, ranks picked in the
    dropdowns, a ranked vote cast and removed, single and multiple votes
    cast with their chosen rows and removed), the rank dropdown's and gear
    menu's items equal, and the widgets' screenshots pixel-equal at 1280
    and 390 (the pie's chart included) but single anti-aliased pixels at
    letter avatar edges in two of them. Left: the float menu's own
    wrapper differs as 228a's gear menu does (static/js/float.js gives it
    data-identifier and no `-content` class or aria-labelledby), and
    option HTML drawn again by a redraw is not run through the cooked
    decorators (decoratePollOption's applyHtmlDecorators; 228a's options
    the same). Tests in tests/plugin_pages.rs and plugins::poll::view.
    Refused still, said in place: the dropdown's close and open, the
    breakdown (228c) and export (202).
    228c. `feat/poll-ui-breakdown` S: the actions dropdown's close and
    open with their confirm (toggleStatus, with the on_close refresh)
    and the breakdown modal over /polls/grouped_poll_results with
    per-group pies, poll-breakdown.scss (already in poll.css).
229. `feat/poll-builder` M: the Build Poll modal from the composer's
    options menu (composer_view.rs:280-285 marks it not ported): the type
    tabs, title, options editor with its count help, min, max and step
    validation, results visibility, public defaulting to
    poll_default_public, bar or pie, the groups chooser, automatic close,
    dynamic, Insert as BBCode; the menu's condition gains
    pm_with_non_human_user (html.rs:1156). The rich editor's node view
    follows 196
230. `feat/poll-rest` M: poll's server leftovers: reduce_cooked in emails
    (the options without ids, the link_to_poll link, polls of trashed
    posts dropped; email/notification.rs:901-945 passes the raw widget
    markup), the clean_up_inactive_users modifier sparing poll voters
    (jobs/user_cleanup.rs:106-119 would delete them), reduce_excerpt
    through `I18n.t("poll.poll")` (excerpt.rs:240-243 hardcodes English),
    extraction cooking with the post's topic and user ids
    (plugins/poll/mod.rs:143-147), non-integer ranked choice ranks cast as
    Rails casts them (plugins/poll/vote.rs:243), a poll whose content is
    not a list (crates/markdown/src/render/poll.rs:202), and
    current_user's can_create_poll dropped from the ignore lists
    (parity/cases:163-166) with a case per role
231. `feat/reactions-lifecycle` S/M: discourse-reactions' server hooks:
    a destroyed user's reaction users deleted (plugins/mod.rs:200), the
    regular Jobs::DiscourseReactions::LikeSynchronizer and the
    excluded_from_like change handler re-enqueueing it five minutes out
    (plugins/mod.rs:310; jobs/mod.rs:596; it fires once 164 saves list
    settings), the sync's MigrationReport logging
    (plugins/reactions/like_sync.rs:22), op_reactions_data on topic list
    items and suggested topics behind the two
    include_discourse_reactions_data modifiers, the post_action_users_list
    filter with core's /post_action_users (not routed). Also corrects the
    stale "reactions in messages are refused" note
    (plugins/reactions/toggle.rs:11): no refusal is left and the react_pm
    cases cover them
232. `feat/reactions-ui` M: the allow-any-emoji picker button
    (plugins/reactions/view.rs:190), infinite scroll on the Reactions tab
    and the reactions received page past the first 20
    (routes/users.rs:757; routes/user_notifications.rs:145;
    user_profile_view.rs:1418), reactions received's acting_username and
    include_likes, which notification links send
    (routes/user_notifications.rs:148-153), the `l` shortcut, the admin
    emoji list's locked like reaction (admin_site_settings_view.rs:1486),
    touch long press, vibration and the animations
    (static/js/discourse-reactions.js:7), the mobile users modal and
    stylesheet, the user nav links once core's user nav panel exists
233. `feat/solved-lifecycle` M: what core's events and services owe
    discourse-solved: unaccepting a destroyed answer
    (on(:post_destroyed), absent from post_destroyer.rs), unaccepting
    when a revision takes the topic out of solved and the refresh_stream
    flag (before_post_publish_changes; posting/revise_topic.rs:958 calls
    only voting), PostMoverExtension re-homing or unaccepting moved
    answers (post_mover.rs), auto bump skipping solved topics
    (jobs/periodical.rs:251), AcceptAnswer's reload_topic message with a
    new auto close timer (routes/solved.rs:167), accepting a post whose
    user is gone (plugins/solved/answers.rs:266), by_user's negative
    limit (routes/solved.rs:244), SeedAdminDashboardReports with
    discourse_solved_admin_dashboard_seeded; the stale "Not ported: web
    hooks" note dropped (plugins/solved/answers.rs:8)
234. `feat/solved-ui-rest` M: show_who_marked_solved's tooltip
    (plugins/solved/view.rs:9), the confetti
    (static/js/discourse-solved.js:7), the notification.solved.* icons
    (user_menu_view.rs:586), solved's tracking and watching descriptions
    (left in 98), SolvedRemovalConfirmation with its don't-show-again key
    and the stream refresh on a solved change, MoveSolutionConfirmation
    (after core's move-to-topic modal), the accepted_solutions related
    items under the report (static/js/admin-report.js:1607), the solutions
    column on the users page (routes/directory.rs:200)
235. `feat/solved-category-fields` S: the preloaded category custom fields
    on /site.json and /categories.json: solved's enable_accepted_answers,
    notify_on_staff_accept_solved, empty_box_on_unsolved and
    enable_shared_issues, topic voting's enable_topic_voting
    (categories.rs:311), with `/categories/*/custom_fields` dropped from
    the ignore lists (parity/cases:16,44,201,222-224,242-243)
236. `feat/topic-voting-ui-rest` M: the docked header's vote box (its
    header-topic-info__before connector; core's docked topic info is
    266's, static/js/header-topic-info.js), full-page search's
    Most Votes order (search_view.rs:45; static/js/search-page.js:16),
    votes among the category editor's default view and sort order
    choices (category_edit_view.rs:307, free text today), the
    voting_closed icon as `lock` (user_menu_view.rs:617 draws
    far-envelope), the voters modal on mobile
237. `feat/topic-voting-rest` S: the vote and unvote integer check, a
    400 where the port 404s (routes/topic_voting.rs:86), voted_by with
    `period` (:210); the stale "Not ported: web hooks" note dropped
    (plugins/topic_voting/votes.rs:5)
238. `feat/presence-translate` S, after 168: the composer's translate
    state: Composer.ADD_TRANSLATION, ComposerPresenceDisplay's
    `/discourse-presence/translate/<post>` subscription and announce
    (static/js/topic-presence.js:132-141,171-179), the
    presence.translating strings in the composer's data
    (html.rs:1135-1103). The whisper state follows 196's whisper toggle
    (topic-presence.js:139)
239. `feat/local-dates-cook-rest` S: the dates the markdown refuses and
    Rails cooks, each recorded against Rails: times moment reads through
    Date (`9:00`) and loose offsets
    (crates/markdown/src/render/local_dates.rs:134,141,164), years past
    9999 (:67), `[date]` without a date (:585), era tokens in a format
    (:528), email timezones chrono-tz lacks or without an abbreviation
    (:590,601)
240. `feat/lazy-videos-ui` S: discourse-lazy-videos' client: LazyVideo's
    thumbnail button, title bar, dominant color, provider icon and
    play_video label, LazyIframe's YouTube (list, start time), Vimeo and
    TikTok embeds, the per-provider settings, lazy-videos.scss
    (src/assets.rs); the email reduce_cooked turning each video into a
    paragraph link (email/notification.rs:913-945)
241. `feat/lazy-videos-crawler` S: CrawlerPostEnd's VideoObject JSON-LD
    per video at the end of each crawler post
    (server:topic-show-crawler-post-end); with 163's solved schema
    markup, it needs the crawler view's per-post markup
    (topics/show.html.erb), which the port's crawler topic page
    (routes/topics.rs:705) doesn't draw
242. ~~`feat/checklist-toggle`~~ (done: PUT /checklist/toggle(.json) in
    routes/checklist.rs and plugins/checklist.rs, recorded as 20
    checklist/ cases: the 200s with and without a revision, by source and
    by index, a batch, a staff edit of another's post with its revision and
    staff log; the 400 contract messages Rails casts and orders, nils
    dropped as deep_munge does; 403, 404 for missing, deleted and disabled;
    the 409s with `retryable` for stale states, text and time, a changed
    count and a missing box; the 422 for an `[X]`. CheckboxLocator,
    SourceMap and Checkbox ported with their specs as unit tests, the
    fallback cook as Post#cook. PostRevisor takes `expected_raw` (checked
    before validating and again under FOR UPDATE, `Outcome::EditConflict`)
    and `preserve_cooked_token` (on the revised message); the live
    stream's post event carries the token and static/js/checklist.js drops
    the swap it made, as consumeOptimisticPostUpdate does. The decorator
    reads the post model from `data-checklist-post` on the post's
    div.cooked and fetches the raw from posts#by_number while posts#show
    is not routed (198); without Ember's cooked on the model, a post
    changed since drawing is refreshed rather than compared by its cooked
    boxes. The system user's toggle saves and answers 200, as Rails'
    does (revise! returns successfully_saved_post_and_topic; its
    SYSTEM_USER_ID return is inside alert_users; checked at bf55a44 with
    a spec in a dv container); the recorder's Jobs.enqueue now returns a
    job id, as Sidekiq's does, instead of nil. Refused: toggle shapes Rails 500s on
    (non-pair arrays), float or structured string params, emailed posts'
    fallback cook) M: checking a box: PUT /checklist/toggle
    (ToggleCheckbox's contract, policies, rate limit, its
    200/400/403/404/409/422 answers), CheckboxLocator and SourceMap
    resolving `data-chk-src` and index targets in the raw,
    Checkbox#replace_in, the revise with bypass_bump and expected_raw and
    core PostRevisor's preserve_cooked_token; the client decorator (a new
    static/js/checklist.js: clickable boxes, accessibility, quote
    exclusion, debounced batches, conflict retries, optimistic updates)
    and the checklist strings. The list menu item and rich editor stay
    with 196
243. ~~`feat/footnote-post-process`~~ (done: in the post processor, recorded as
    footnote/edit_post_scoped_ids) S: footnote's before_post_process_cooked
    rewrite of `#fn<n>`/`#fnref<n>` to post-scoped ids
    (plugins/footnote/plugin.rb:18-46), absent from the process_post job
    (jobs/handlers.rs:840 runs only local dates'), so two footnoted posts
    on one page collide; gated on enable_markdown_footnotes, with
    static/js/inline-footnotes.js finding the new ids
244. `feat/spoiler-email` S: spoiler-alert's reduce_cooked in emails
    (each spoiler a link to the post with excerpt_spoiler;
    email/notification.rs:913-945 sends spoilers in the clear), and
    discourse-details' email reduce_cooked (email/notification.rs:913)
    gated on its enabled setting as Plugin#on gates every handler
    (lib/plugin/instance.rb:692). The excerpt's gates (spoiler removal,
    poll, local dates' " (UTC)") are done: `fix/excerpt-plugins-enabled`
245. `feat/oauth2-basic-rest` S: oauth2_debug_auth (an OAuth2LogRedactor
    and every `log` call) and the always-on token and user_json failure
    warnings (plugins/oauth2_basic.rs:349-355; routes/omniauth.rs:1118),
    oauth2_request_timeout_seconds from the global settings
    (plugins/oauth2_basic.rs:19, 10s hardcoded), every
    oauth2_authorize_options entry forwarded (:75-79, only scope today),
    `recurse` over the AccessToken's other readers (:90-112), a token
    endpoint answering with a redirect followed as the oauth2 gem does
    (auth/oauth2.rs:567, OIDC's too). The additional and required JSON
    path registries wait for 203-206 (only third-party plugins fill them)
246. `feat/oauth2-oidc-admin-login-tabs` S: admin_config_login_routes
    with `oauth2` and `oidc` (site.rs:508 returns `[]`), the
    /admin/config/login-and-authentication/oauth2 and /oidc tabs with
    their setting areas (routes/admin_config.rs:669-707), the admin
    sidebar's links and labels. Their list and objects settings need 164
247. ~~`feat/auth-login-buttons`~~ (done in 261) S: the external login
    buttons on /signup posting `signup=true`, so
    oauth2_authorize_signup_url can be reached; the single-external-login
    /signup redirect; the buttons' aria-label from
    `js.login.<name>.sr_title` (Apple's one gap besides 164)
248. `feat/openid-connect-mtls` M: mutual TLS client authentication for
    the token and userinfo requests: the PEM certificate and key with
    passcode, the tls_client_auth scheme, the parse failure logged then
    raised (auth/oauth2.rs:532-533; plugins/openid_connect.rs:7,126-127);
    needs a FinalDestination path carrying a client identity
249. `feat/openid-connect-logging` S: openid_connect_verbose_logging (the
    request and response formatter and every verbose_log site, the
    logout hook's too), the error-level groups claim and request failure
    logs (plugins/openid_connect.rs:572-584; routes/omniauth.rs:1112-1116),
    openid_connect_request_timeout_seconds from the global settings
    (plugins/openid_connect.rs:18), the groups claim falling back to the
    ID token when the userinfo claim is false (:566-571)

### Phase 12: refusals no item owned (2026-10-20)

Found by regrouping every `Unsupported("...")` marker against the items
above, the 165 written over two lines included (`grep -rho` misses
them): what merged branches left refused with no open item naming it.
The other leftovers of done items went into the open items that own
them (137, 138, 154, 156, 157, 164, 168, 173, 184). 7 items: 4 S, 1
S/M, 2 M.

250. `feat/login-codes` S/M: logging in with an emailed code without an
    invite (/session/login-code and its verify,
    routes/invite_redeem.rs:1800,1908), completing a login code signup
    (signup_token, :1904), a verification failing its contract (:1924),
    a signed-in user asking for a code (:1726), a suspended user
    verifying one (:2053), DiscourseConnect provider handoffs (:2225).
    The invite's login code is done (192b)
251. `feat/invites-rest` M: what 192a-g left: user fields on redemption
    and in bulk invites (routes/invite_redeem.rs:596; bulk_invite.rs:166),
    staged users (redeeming as one, an invite to one's email, their
    usernames: routes/invite_redeem.rs:578,1337,505,507), auto-approved
    email domains (:582), a verified invitee's username and the one
    suggested from the email (:562,568,2123), validation errors other
    than the email and a password User refuses (:646,650,620), an invite
    failing to redeem on activation (:747), the accept page with
    registrations closed (:1027) or external logins (:1533) and
    accepting with an external login (:1046, UserAuthenticator), the
    page's rate limit as JSON (:1641); the invite modals (create, edit,
    bulk) and invites past the first page (192d)
252. `feat/categories-rest` M: what 190a-f and 189d left: a description
    change revising the definition post (category_writes.rs:1465; the
    `not_yet` case categories/update_description), logos and
    backgrounds (:606), the Events category type and settings given per
    type (category_type_config.rs:146,160,206,244,337;
    category_types.rs:378; routes/categories.rs:470,531), deleting a
    category or tag that embeddable hosts use and a tagged or embedded
    definition (category_writes.rs:2884,2984; tag_writes.rs:35;
    user_destroyer.rs:1225), the category editor's Images, Topic
    template, Tags and Localizations tabs (routes/categories.rs:823)
    with its emoji and icon pickers and colour palette, the editor's
    entry points (the category page's wrench, New category), the tag
    settings page /tag/:slug/:id/edit (routes/tags.rs:210) and the tag
    groups pages (routes/tag_groups.rs:61,86); discourse-ai's
    translation of a renamed category (category_writes.rs:2306) goes
    with 202
253. `feat/api-keys-rest` S: what 84 left: API keys in request
    parameters (session/current.rs:366), a user API key with an admin
    key or an auth cookie (:380,383), a client id header or a listed key
    without a client (session/user_api_key.rs:269; user_private.rs:503),
    granular scopes on constrained routes (api_key_scopes.rs:266),
    impersonating through an API key (routes/impersonate.rs:83)
254. `feat/small-leftovers` S: small refusals of done items no other item
    owns: reviewing every new post (review_every_post,
    posting/create.rs:357, refused; until 2026-10-20 it went unchecked
    and posts skipped the queue), sorting admin users by email and admin
    user lists without
    stats (admin_users.rs:111; routes/admin_users.rs:259), activating a
    user without an email (routes/admin_users.rs:1276), uploading
    watched words from a file (routes/admin_watched_words.rs:334, 80's
    CSV upload), reseeding categories and topics
    (routes/site_texts.rs:932), a color scheme with no default theme
    (routes/color_schemes.rs:457), a report the port doesn't draw
    (reports/mod.rs:992), schedule_backup's failure message to the
    admins (backup_restore/jobs.rs:221), the full-page search's error
    and 403 pages and its category chooser under
    fixed_category_positions_on_create (routes/search.rs:418,456,895),
    markdown_typographer_quotation_marks other than the default
    (pretty_text/mod.rs:487)
255. `feat/auth-rest` S: what 155 and 191 left: Facebook's signed request
    cookie and a Google id_token or JSON body at the callback
    (auth/builtin.rs:369,407,423), Twitter's callback_url
    (routes/omniauth.rs:827), a username other than the provider's when
    it overrides usernames (signup.rs:281), an inactive user or an
    invitee found by external id (routes/users.rs:312;
    routes/invites.rs:792), the second factor page without local logins
    (routes/user_preferences.rs:532), a passkey login for an inactive
    user (routes/session_passkey.rs:114), enabling an authenticator
    never staged (routes/user_security.rs:672). The guards at
    routes/omniauth.rs:187,1300,1447 refuse a provider outside the
    ported list, which every registered provider now is
256. `feat/s3-rest` S: what 122 left: s3_role_arn and instance-profile
    credentials (file_store.rs:677,684), restoring uploads into S3 and
    remapping AWS S3 urls (backup_restore/restorer.rs:839,962),
    include_s3_uploads_in_backups (backup_restore/creator.rs:238),
    presigned PUTs of backups and a backup store neither local nor S3
    (routes/external_uploads.rs:91,448), promoting a direct upload of
    100 MB or more (:1062), GET /uploads/short-url (not routed); remote
    upload urls in the upload jobs are 140

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
- [x] Unicode emoji in titles (`performEmojiEscape` turns them into codes)
- [ ] Hidden and deleted posts (`hidden`, `deleted_at`, staff whispers stay
      hidden). Hidden posts serialize as Rails does (the review actions),
      and the topic view shows staff deleted posts and topics (124);
      deleted posts for staff outside the topic view: 124
- [ ] Topic thumbnails and topic images behind a CDN, secure uploads: 125,
      121, 123
- [x] Watched words (`WordWatcher` regexps and actions and regular
      expression mode)
- [ ] Topic timers (done: 60-62), topic links with clicks
      (`TopicLink.topic_map`): 127
- [ ] Featured links (`featured_link_root_domain`): 126
- [ ] `subcategory_list` styles, paginated category lists: 128
- [x] Performance: batch the list serializer's per-topic queries (99 to
      11 per /latest.json, 137 to 332 req/s at one connection)
- [x] Performance: the topic view's per-post queries (169 to 143 per
      logged-in topic page, 4 fewer for each further post): 201
- [ ] Group flair uploads, user fields, badges granted, auth providers,
      user color schemes on `/site.json`. User fields (87b) and user color
      schemes (90b) and auth providers (191a) done; group flair uploads:
      157; the keys parity still ignores: 199
- [ ] Muted defaults (`default_categories_muted`, `default_tags_muted`,
      `mute_all_categories_by_default`), shared drafts category: 129, 158
- [ ] `tags_listed_by_group`: 159
- [ ] Profiles: bios (PrettyText.excerpt), suspended/silenced users, user
      status, featured topics, generated letter avatars (`/letter_avatar`,
      for sites that turn the proxy off). Bios and suspended and silenced
      users done; user status: 130; featured topics: 156; letter
      avatars done (131a)
- [ ] Search: advanced filters (`in:`, `status:`, `category:`, `#`, `@`,
      `tags:`, `before:`/`after:`, `order:`), search contexts, `search_for_id`,
      rate limits, pg headlines. Rate limits (21) and the advanced
      filters, contexts and `search_for_id` (143) done; pg headlines and
      the ranking settings: 144
- [ ] Plugin-added keys (solved, voting, reactions, ...) through the plugin
      protocol (milestone 5) instead of parity ignore lists: 202, 199
- [ ] Topics without a stored slug (`Slug.for`): 120
- [ ] RSS feeds (`.rss` for the lists, categories, tags, topics, a
      user's posts and activity): 195
- [x] Multisite: several forums from one process, by hostname
      (RailsMultisite): 208

## Milestone 3: sessions

Logged-in readers, no writes yet.

- [x] Login: local (PBKDF2 `user_passwords`), CSRF, the `_t` and
      `_forum_session` cookies in Rails' own encrypted format so a cutover
      keeps sessions; `/session/current.json` byte-equal for members and
      admins
- [x] OIDC / external logins, 2FA. The TOTP challenge for granting admin
      (86b), logging in with a second factor, backup codes and webauthn
      (155), OIDC and the other external logins (191) done; their
      leftovers: 255
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
      required-fields redirects, pending reviewables. Group PMs' related
      messages (33c), the required-fields redirect (87b), the 2FA
      redirect (155c), deleted topics and posts and ignored users (124)
      done, pending reviewables (194); category group moderators: 149;
      shared drafts: 158
- [x] Bookmarks: `/u/:username/bookmarks.json` and
      `/u/:username/user-menu-bookmarks` (core's post and topic bookmarks
      and chat's message bookmarks; the `.ics` feed is refused)
- [x] The other user-menu endpoints (`/u/:username/user-menu-private-messages`,
      `/review/user-menu-list`): the messages one (95), the review queue
      (194)
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
      refused explicitly today. Chat transcripts: 161; the rest: 202
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
      definition. Oneboxes (43, 51-55), badges (57) and sizing images
      from other sites (FastImage) done; SVG, HEIC, AVIF and JPEG XL
      ones and hotlinked media: 140; optimized videos: 141; the
      category description: 137
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
      restoring and unhiding from the queue, penalties. The reviewed and
      queued posts' actions landed with 75; the rest: 147, 148
- [x] The review queue (GET /review.json) for admins: flagged, queued and
      reviewed posts with
      `list_for`'s status, type, topic, category, user, priority and sort
      filters, paging, the side-loaded users, topics, scores, score types,
      bundled actions and histories, and the meta. Measured against Rails.
      Refused: category group moderators (149), user and plugin
      reviewables (148), penalized authors, the date and additional filters.
      Moderators see `Reviewable.viewable_by`: what moderators may review
      outside categories and messages they can't see and posts they can't
      review, in the list, the counts, the user menu and the actions
      (write cases review/moderator_*)
- [x] Topic status (PUT /t/:id/status): close, open, archive, unarchive,
      unlist, relist, pin, unpin, pin globally, with the small action post
      (PostCreator's small action path), featured topics, the category's
      count, hot scores, visibility reasons and the staff action log.
      Measured against Rails
- [ ] Topic status that goes further: messages, topic timers, pinning
      until a time. Messages and timers done (60); pinning until a time:
      152
- [x] Deleting posts and topics (DELETE /posts/:id, DELETE /t/:id) and
      recovering replies (PUT /posts/:id/recover): PostDestroyer for
      staff, the author marking their own post deleted through
      PostRevisor, the counters, user actions, notifications, category
      latest posts, user stats and staff logs. Measured against Rails
- [ ] Deletion that goes further: permanent deletion, messages, posts with
      links, likes or pending flags, recovering topics, quotes and replies.
      Messages (31) and posts with links and likes (63) done; the
      rest: 150
- [x] Suspending, unsuspending, silencing and unsilencing users
      (PUT /admin/users/:id/...): the user, the staff log, the expiry job, the
      penalty emails, and SystemMessage (a private message from the site
      contact through PostCreator), which also runs send_system_message
      (the post_hidden message). Measured against Rails
- [ ] Penalties that go further: several users at once, acting on a post,
      penalties from the review queue, re-penalizing (its "how long ago"
      message), auto-silencing new users on spam flags. Several users,
      acting on a post and re-penalizing done (86a); from the review
      queue: 148; auto-silencing on spam flags: 147
- [x] Changing a site setting (PUT /admin/site_settings/:id) for core
      settings of the plain types: the cast, the hidden and archive checks,
      TypeSupervisor's validation, the stored override and the
      change_site_setting log. Measured against Rails
- [ ] Site settings that go further: lists, uploads, groups, categories,
      plugin settings, bulk updates, user preference backfills, and the
      settings whose change handlers write (title, site_description...):
      164 (the sidebar defaults' backfill landed with 90c)
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
      thumbnails, avatars, S3. Post images' optimized images and
      thumbnails done (the post processor, above), avatars (131) and S3
      (122) done; the rest: 139, 256
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
      POP3 polling, group and category addresses (email_in): 165
- [x] PM emails, previous-replies context (previous replies done in 146)
- [ ] Jobs, next: digests, pull_hotlinked_images,
      notify_mailing_list_subscribers, scheduled jobs (category stats, top
      topics, digests), rate limits. Digests (70, 71), the scheduled jobs
      (phase 1) and rate limits (phase 2) done; pull_hotlinked_images: 140;
      notify_mailing_list_subscribers: 146 (the enqueued jobs without a
      handler are listed under phase 9)
- [x] Name the places a plugin would attach as they are built
      (NewPostManager modifiers, post events); nothing calls them until
      milestone 5. Done as the lifecycle phases and DiscourseEvent
      functions of src/plugins/mod.rs (PLUGINS.md decision 6), which the
      bundled plugins use

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
      and the chat drawer (223a)
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
- [x] The user menu, notifications, bookmarks, messages, preferences. The
      user menu with its bookmarks and messages tabs (95), the messages
      pages (33b), the notifications, preferences and the other user
      pages (193) done
- [ ] Review queue and topic status for staff: 194

## Milestone 5: plugins

After writes (moved 2026-10-01, was 3.5): the plugin API is mostly the
write endpoints. The design and its open decisions are in PLUGINS.md.

- [x] Decide the runtime (PLUGINS.md decision 6, 2026-10-08): bundled
      plugins are Rust in src/plugins/ on lifecycle phases; third-party
      plugins a sandboxed runtime later (Luau spike on
      spike/plugin-runtime)
- [ ] The bundled plugins: all sixteen started (crate::plugins::STARTED),
      none done. The parity audit (210, docs/plugin-audits.md, 2026-10-17)
      lists every gap of chat, poll, reactions, solved, topic voting,
      presence, local dates, lazy videos, checklist, details, footnote,
      spoiler, LTI, OAuth2 basic, OpenID Connect and Apple against the
      reference plugin; the uncovered ones are 211-249, the rest earlier
      items (chat 160-162, solved 163, voting and reactions core's 143,
      150, 188i4, 195, 197, the settings 164). Templates, narrative-bot's
      tutorials and the plugins not started: 202
- [ ] Tier 0: `plugin.toml` with settings, preloaded custom fields,
      assets, i18n, the tables the plugin owns; `discourse-rs plugins`:
      203
- [ ] The host functions: keyed batch reads of the plugin's own tables,
      and the Discourse JSON API in-process under manifest scopes: 204
- [ ] The third-party runtime on the same phases: `serialize`, `html`,
      `modify` batched per response: 205
- [ ] `route` with core's session and CSRF, `event`, `job`; the failure
      policy: 206
- [ ] Hosted plugins: the same API over HTTP with a scoped key, events
      as signed webhooks: 207

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
- [ ] Site settings: the setting types milestone 4 refuses: 164
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
- [x] Users: granting admin (its second factor or email confirmation),
      deleting, anonymizing and merging, impersonating, penalties on several users
      (86b, 63, 78, 79, 79b, 86c, 86a)
- [x] Groups: creating, editing and deleting, members and owners in bulk,
      automatic membership by email domain (82a-82e)
- [x] Logs: staff actions, screened emails, IPs and URLs, search logs,
      email logs (sent, skipped, bounced, received, rejected), measured
      against Rails
- [x] Logs: a search term's details, incoming email details
- [x] Backups: creating, listing, downloading and restoring through the
      app, read-only mode (S3, and backups that need migrations, refused)
- [x] Badges: creating, editing and deleting, groupings, granting and
      revoking, badge SQL, and the BadgeGranter jobs that award them
      (85a-85c, 56-58)
- [x] Permalinks, embeddable hosts, user fields, form
      templates, custom flags, custom emoji (form template previews
      wait for a Psych YAML emitter)
- [x] API keys and their scopes, web hooks and their deliveries (the
      events core sends to them) (84d, 83a-83d)
- [x] Email: the settings test, previews, templates and email style (88,
      72; updating and reverting a customized template: 172)
- [x] Dashboard, reports and problem checks, admin notices (89a-89d)
- [ ] Customize: themes and components, color schemes, site texts
      (translation overrides), sidebar defaults; the `/admin/config` pages.
      Site texts (90a), color schemes (90b), sidebar defaults (90c) and
      the themes' reads and writes (90d, 90e), imports, exports,
      setting types and the wizards (90f) and the admin pages (90g, a theme's page in
      90g7) done; left: 90g7a's leftovers of a theme's page and the
      leftovers named under 90, and 169-171

## Not planned

- Running Discourse migrations: Discourse owns the schema; re-vendor
  `structure.sql` and re-snapshot instead
- Plugins with the host's permissions: native code in the process, or a
  subprocess with its own database connection (PLUGINS.md decision 3)
