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
    email or plugin fields, unsilencing the author, unhiding a first post,
    moderators on post reviews; the review list still shows flagged posts
    only),
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
    warm_dashboard_reports. Refused: discourse-solved's support section,
    data explorer's report source, what's new and toggle-feature (the
    upcoming changes list), the traffic explorer until 89d),
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
    preference defaults), 90d. `feat/themes` L, started: done are the
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
    (their compile errors), extra_js (ThemeJavascriptCompiler), inline
    scripts (AssetProcessor), migrations, uploads and svg sprites,
    other locales' translations, settings that request a refresh,
    destroying a theme's own color schemes), 90f.
    `feat/theme-imports` M (import, export, remote themes, uploads,
    objects settings, the design wizard), 90g. `feat/admin-shell` L (the
    admin pages' HTML: the port has none; /admin/customize/* and
    /admin/config/* refuse non-JSON requests today), 90h.
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
    confirmation, the menu-panel.scss rules. Left: the review queue tab
    (refused), the bookmarks tab with a chat message bookmark (refused by
    the bookmarks list), the do not disturb and user status modals,
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
    chat footer. Left: the footer's DMs, search, starred and threads
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
    Left: the menu's quote whole post, insert table, change reply-to,
    format code, build poll and wrap items, which say they are not ported;
    the menu refuses to open with discourse-math, -events, -policy,
    -graphviz or -templates (for a member who can use them) on; editing a
    date from the rich editor (the rich editor is not ported); the locale
    from the Accept-Language header

[mobile-layout]

### Phase 9: what done items still refuse

Open in phases 9 and 10 (2026-10-14): 89 items, 120-208. By size: 24 S,
46 M, 19 L. Phase 9: 68 (22 S, 36 M, 10 L); phase 10: 21 (2 S, 10 M,
9 L). Not counted here: the open items above (90f, 90g, 92b), 100 (in
Phase 10, after 192) and 101-119.

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
(26 "(Ember)" and "admin ... page" refusals) are 90g; remote themes,
theme uploads and objects settings, the wizard (themes.rs:305;
theme_writes.rs:196,1213,1232; routes/themes.rs:1040;
admin_site_settings.rs:218; site.rs:566; routes/accounts.rs:1062) are
90f; MessageBus's own ids and positions are 100.

The enqueued jobs without a handler go with the item that owns them:
notify_mailing_list_subscribers (146), notify_post_revision (137),
crawl_topic_link (127), index_category_for_search (144),
rebake_quoted_posts_for_user (135), change_display_name (153),
update_gravatar and create_avatar_thumbnails (131),
pull_hotlinked_images and pull_user_profile_hotlinked_images (140),
refresh_latest_new_feature (173), user_suspension_expired (151).

120. `feat/slug-for` M: topics without a stored slug (`Slug.for`),
    refused wherever a topic is linked (topic_view.rs:292,771;
    topic_list.rs:544; users.rs:1864; notifications.rs:558;
    bookmarks.rs:532; pretty_text/helpers.rs:275;
    topic_view/web_hook.rs:154,320; not_found_page.rs:190;
    review_list.rs:650; email/digest.rs:291,1058,1217;
    jobs/bookmark_reminders.rs:147; reports/counts.rs:1204;
    reports/lists.rs:409; plugins/reactions/users.rs:703;
    routes/chat_pages.rs:406), slug_generation_method other than ascii
    (posting/topic_save.rs:33; posting/create.rs:1403;
    plugins/solved/by_user.rs:50), titles with emoji codes
    (posting/text.rs:336), topics without a stored fancy_title on
    profiles (users.rs:1662). Non-English transliteration waits on 167
121. `feat/cdn` M: URLs behind a CDN (avatar.rs:67;
    pretty_text/helpers.rs:342; pretty_text/mod.rs:466;
    pretty_text/cleanup.rs:607; pretty_text/cooked_post_processor.rs:759;
    topic_list.rs:1025; users.rs:1074; badges_view.rs:112;
    reports/tables.rs:561)
122. `feat/s3-store` L: the S3 store for uploads and backups
    (file_store.rs:88; plugins/mod.rs:73; backup_restore/store.rs:55;
    backup_restore/creator.rs:221; backup_restore/restorer.rs:831,933;
    routes/backups.rs:522,534; reports/pageviews.rs:497,501;
    jobs/upload_jobs.rs:48,116). Needs: an S3 client and an S3-compatible
    store beside the reference for the harness
123. `feat/secure-uploads` M, after 122: secure uploads and media
    (uploads.rs:342; posting/rebake.rs:49; pretty_text/helpers.rs:338;
    pretty_text/cooked_post_processor.rs:756; upload_references.rs:373;
    topic_list.rs:1013; email/styles.rs:132)
124. `feat/topic-view-staff` M: deleted topics and posts for staff
    (topic_view.rs:289,687 with `post_stream.gaps`;
    posting/revisions.rs:100), ignored users (topic_view.rs:675;
    routes/users.rs:1290; the `not_yet` case
    reactions/topic_view_ignoring_a_reactor), detailed_404
    (topic_view.rs:284), queued_posts_count for moderators (:516),
    title_is_group (:1747), custom post notices (:1876),
    limit_suggested_to_category (:2518), suggested topics from other
    lists and for anonymous readers (topic_query.rs:814,862)
125. `feat/topic-thumbnails` M: topic thumbnails in the topic view and
    web hook payloads (topic_view.rs:549; topic_view/web_hook.rs:550),
    theme thumbnail sizes (optimized_images.rs:131 and
    jobs/consistency.rs's preflight), topic images in digests
    (email/digest.rs:657)
126. `feat/featured-links` S: featured_link_root_domain
    (topic_view.rs:409; topic_list.rs:650,758;
    topic_view/web_hook.rs:434; email/digest.rs:660)
127. `feat/topic-links` S: TopicLink.topic_map with clicks
    (topic_view.rs:1402), links to topic and site paths the router isn't
    ported for (posting/links.rs:65,449), links from video embeds
    (:219), the crawl_topic_link job
128. `feat/category-lists-rest` M: the subcategories_with_featured_topics
    styles and paginated category lists (category_list.rs:148,167),
    parent slugs needing CGI escapes (:120), category logos
    (categories_view.rs:203,396), lazy-loaded categories
    (categories.rs:187; bookmarks.rs:406; routes/drafts.rs:290),
    three-level nesting (topic_query.rs:375; hashtags.rs:424), encoded
    slugs and hidden parents in hashtags (pretty_text/helpers.rs:505,547;
    hashtags.rs:393), unknown style_types (pretty_text/helpers.rs:569;
    hashtags.rs:436)
129. `feat/muted-defaults` S: mute_all_categories_by_default,
    default_categories_muted and default_tags_muted in lists
    (topic_query.rs:348,351,358; category_list.rs:349), default tag
    preferences at signup (signup.rs:730), staff groups' category and tag
    notification defaults (roles.rs:163)
130. `feat/user-status` M: user status (UserStatus, the /user-status
    routes, the status modal) wherever a user is serialized (users.rs:540;
    current_user.rs:306; about.rs:253; bus.rs:447; directory.rs:611;
    topic_view.rs:1509; routes/groups.rs:835; routes/admin_api_keys.rs:74;
    plugins/chat/direct_messages.rs:237; plugins/chat/membership.rs:128;
    plugins/chat/messages.rs:636; plugins/chat/channels.rs:654) and PUT
    /u's `status` (user_updater.rs's REFUSED)
131. `feat/avatars` M: generated letter avatars (avatar.rs:134,
    LetterAvatar), default_avatars and restrict_letter_avatar_colors
    (:113,122), avatar uploads and picking (uploads.rs:339), gravatar
    (user_updater.rs:554,556), random avatars at signup (signup.rs:406),
    system_user_avatar_template (site.rs:479), renaming or merging the
    system user with the logo as its avatar (username_changer.rs:171;
    user_merger.rs:229), the update_gravatar and create_avatar_thumbnails
    jobs. Needs: network fetches (gravatar), letter rendering equal to
    the reference's
132. `feat/title-emoji` S: unicode emoji in titles (posting/text.rs:283,
    performEmojiEscape; posting/validate.rs:741), emoji in a quoted
    topic's title (crates/markdown), the emoji shortcut regex's
    backtracking (posting/text.rs:296)
133. `feat/anonymous-posting` M: anonymous posting mode (the shadow
    accounts) wherever a shadow acts: posting (posting/validate.rs:80,153;
    posting/create.rs:218; posting/revise.rs:81; posting/enqueue.rs:88),
    likes, flags and bookmarks (post_actions.rs:235; likes.rs:187,519;
    flags.rs:107; bookmark_manager.rs:130), topic_guardian.rs:913,
    current_user.rs:309, read_tracking.rs:134,
    post_destroyer.rs:328,389,1251, pm_invite.rs:66,243,
    topic_status.rs:134, post_owner_changer.rs:446, review.rs:356,
    review_posts.rs:179, reviewable_user.rs:247, the plugins
    (topic_voting/votes.rs:126,152,175,218; poll/vote.rs:263,485;
    reactions/toggle.rs:119; solved/answers.rs:264; chat/mod.rs:83;
    chat/dm_groups.rs:438), anonymous staff on admin routes
    (routes/admin_watched_words.rs:82,149,178;
    routes/directory_columns.rs:248;
    routes/admin_dashboard_sections.rs:172; user_penalties.rs:430,688,
    837,1014; routes/admin_users.rs:504,1837; site_setting_update.rs:142)
134. `feat/posting-trust-levels` M: posting as a new (TL0) or staged
    user (posting/validate.rs:158,161), links from users outside
    post_links_allowed_groups (:610), max_consecutive_replies (:641),
    staged users on restricted categories (topic_guardian.rs:253),
    approving a staged user's queued post (posting/create.rs:317),
    silencing a TL0 user (user_penalties.rs:925, hide_posts), TL0 bio
    links (users.rs:708, strip_links)
135. `feat/quoted-posts` M: QuotedPost on create and revise
    (posting/create.rs:549,552; posting/revise.rs:213), quote alerts and
    quotes in group messages (jobs/post_alert.rs:398,467), recovering
    posts with quotes (post_destroyer.rs:1299), the
    rebake_quoted_posts_for_user job
136. `feat/slow-mode` S: slow mode when creating, enqueueing, editing and
    changing owners (posting/create.rs:486; posting/enqueue.rs:137;
    posting/revise.rs:142; post_owner_changer.rs:465), slow mode without
    seconds (routes/topic_timer.rs:252)
137. `feat/post-edit-rest` L: edits within the grace period
    (posting/revise.rs:180,197; Rails keeps the original in Redis),
    hidden and non-regular posts (:104,107), deleted posts
    (routes/posts.rs:309), staff_edit_locks_post (posting/revise.rs:574;
    post_owner_changer.rs:780), posts#update's title, image sizes and
    bypass_bump (routes/posts.rs:276,281), a category description synced
    from its definition post (posting/revise.rs:115;
    posting/rebake.rs:91; jobs/handlers.rs:941;
    username_changer.rs:792,798; post_owner_changer.rs:462), revisions
    without a user, by the system user or of hidden posts
    (posting/revisions.rs:253,379,381), a past version's post and raw
    (routes/posts.rs:425,480), repeated edit notifications
    (jobs/post_alert.rs:778), the notify_post_revision job
138. `feat/posting-rest` M: replies to whispers (posting/create.rs:773),
    staff replying to closed topics (:506), uploaded video or audio
    (:546; posting/revise.rs:253), duplicate titles and titles the
    cleaner changes (posting/validate.rs:753; posting/topic_save.rs:30),
    auto_close_topics_post_count (posting/create.rs:1263), returning-user
    post notices (:1382), featured topics with a category sort order
    (:1615), suppress_secured_categories_from_admin (:1618;
    jobs/post_alert.rs:724), site_archived (posting/create.rs:221),
    generate_topic_og_image (:799; jobs/handlers.rs:880;
    post_owner_changer.rs:318), posts#create's other params and
    archetypes (routes/posts.rs:76,105), process_post's recook, image
    sizes and remove_full_quote_on_direct_reply
    (jobs/handlers.rs:749,752,797), enqueueing a suspended user's post or
    one with polls (posting/enqueue.rs:94,97)
139. `feat/uploads-rest` M: image types Rails optimizes and crops
    (uploads.rs:353,361), keeping metadata (:364), replacing a failed
    upload (:430), upload thumbnails' serializer (:688), missing
    extensions, outdated optimized images, SVG and AVIF thumbnails
    (optimized_images.rs:202,222,227), dominant colours with libvips and
    of AVIF, HEIC and JPEG XL (jobs/upload_jobs.rs:98,133), uploads not
    sent as multipart, site setting uploads, retain_hours, uploads from a
    url (routes/uploads.rs:46,67,188,209,216,222), drafts with uploads
    (drafts.rs:51), uploads in user and group bios and profile
    backgrounds (promotion.rs:325,361; user_updater.rs:1046,1096;
    jobs/periodical.rs:116; group_writes.rs:420)
140. `feat/hotlinked-media` M: images from other sites sized over HTTP
    (pretty_text/cooked_post_processor.rs:369,458, FastImage), oneboxed
    images of other sites or without a size (onebox/mod.rs:1521,1573),
    blocked and pulled hotlinked media
    (pretty_text/cooked_post_processor.rs:689,716;
    pretty_text/cleanup.rs:574), the pull_hotlinked_images and
    pull_user_profile_hotlinked_images jobs. Needs: network fetches,
    through 52's FinalDestination
141. `feat/optimized-videos` S: videos in the post processor
    (pretty_text/cooked_post_processor.rs:806), video thumbnails
    (upload_references.rs:390; pretty_text/cleanup.rs:610), destroying an
    upload an optimized video was made into (jobs/upload_cleanup.rs:39),
    oneboxes outside a post (pretty_text/cooked_post_processor.rs:766)
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
143. `feat/search-advanced` L: advanced filters and keywords
    (search/mod.rs:292,298: `in:`, `status:`, `category:`, `#`, `@`,
    `tags:`, `before:`, `after:`, `order:`...), search contexts
    (routes/search.rs:348,522), search_for_id (:529),
    restrict_to_archetype (:536), type_filter=all_topics (:546), the
    categories & tags type (:352, Category.search and
    tags/filter/search), URLs in terms (search/mod.rs:436), user search
    by groups among reviewers (user_search.rs:187), the directory's
    search by name (directory.rs:343, UserSearch)
144. `feat/search-settings` M: search_ignore_accents (unaccent) and CJK
    tokenization, searching and indexing (search/mod.rs:542,555,559;
    posting/search_index.rs:267,275), use_pg_headlines_for_excerpt,
    search_ranking_weights, search_prefer_recent_posts, a default sort
    other than relevance (search/mod.rs:545,548,551,565), blurbs from old
    post_search_data rows (:1027,1034), human_only search logs
    (search_logs.rs:60), the index_category_for_search job
145. `feat/post-alert-rest` M: group and @here mentions
    (jobs/post_alert.rs:300,303), nested replies (:276), links to topics
    (:408), first post alerts for tagged topics (:556), linked
    notifications and their consolidation (:714;
    notification_create.rs:230), staged users (jobs/post_alert.rs:733),
    mentions in flagged messages and warnings (:330), post_alert's
    options (:204), populate_acting_user (notifications.rs:360,416)
146. `feat/notification-email-rest` M: previous replies and in-reply-to
    excerpts (email/notification.rs:324,352), staged users and
    private_email (:178,181), group notification emails (:226),
    restricted tags in subjects (:289), the daily limit notice (:348),
    lightboxes and iframes (:828,831), prioritize_username_in_ux off
    (:908), quotes, oneboxes and iframes restyled and right-to-left
    styles (email/styles.rs:126,221), replies to several posts or deeper
    than one level, uploads, attachments and provider headers
    (email/sender.rs:251,344,361,431,449,498), the other email types
    (jobs/user_email.rs:90,98,101,470), sendmail delivery
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
    first or last post (:698), claims and notes (review.rs:190;
    review_posts.rs:257; reviewable_user.rs:195,272,446;
    review_list.rs:589), reviewables for groups (review.rs:186), the
    other reviewable types (review.rs:247; review_posts.rs:211;
    review_list.rs:570,573), queued posts approved into closed or
    deleted topics, with tags, an email, plugin fields or polls,
    unsilencing their author, unhiding a first post
    (review_posts.rs:464,476,610,624,627,1262), user reviewables on
    invite-only sites or with an uploaded avatar and their rejection
    email (reviewable_user.rs:52,82,316), score conversations, reasons
    and types, the ids, date and additional filters
    (review_list.rs:204,207,215,882,894,931,954), reviewable web hook
    payloads (review_list.rs:1520,1558,1603 and `web_hooks::refuse` for
    the reviewable events), truncate_user_flag_stats (review.rs:534;
    review_posts.rs:1236)
149. `feat/category-group-moderation` M: enable_category_group_moderation
    and moderators in the review queue (topic_guardian.rs:191;
    posting/validate.rs:211; review.rs:170; review_posts.rs:413,1138;
    review_list.rs:178; routes/review.rs:147; user_destroyer.rs:620;
    current_user.rs:552,972; reviewables.rs:91), moderators' view of
    flagged messages and warnings (guardian.rs:282;
    topic_guardian.rs:242; jobs/handlers.rs:1230), moderators and
    owner-only groups on the admin view of a user
    (routes/admin_users.rs:1536), category moderators on the about page
    (about.rs:309)
150. `feat/deletion-rest` M: permanent deletion
    (routes/post_destroy.rs:80, force_destroy), recovering topics, likes,
    replies and quotes (post_destroyer.rs:1277,1296,1299), authors
    recovering their own posts (:1256), deleting flagged posts and
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
153. `feat/unicode-usernames` M: non-ASCII usernames (accounts.rs:64;
    signup.rs:99; routes/accounts.rs:162; username_changer.rs:93,564;
    pretty_text/mod.rs:461 for mentions; email/notification.rs:1014,1056;
    routes/required_fields.rs:190; posting/enqueue.rs:378), flag name
    keys from non-ASCII names (admin_flags.rs:207), titleizing a
    non-ASCII user field (plugins/poll/vote.rs:742), the
    change_display_name job
154. `feat/signup-rest` M: overlong usernames, staged users, approval
    and must_approve_users (signup.rs:209,226,234;
    routes/accounts.rs:808), signup with local logins off (signup.rs:249),
    the validation errors other than the username's and a taken email
    (:289-327), screened emails and IPs (:311; routes/session.rs:398),
    welcome messages (signup.rs:833), automatic groups with titles,
    flair, granted levels or notification defaults (:668;
    promotion.rs:428; roles.rs:204,216), password algorithms other than
    pbkdf2-sha256 (session/token.rs:88,93,107), password reset errors
    (routes/accounts.rs:558,613,635). Invite codes are 192
155. `feat/second-factor` M: logging in with a second factor
    (routes/session.rs:415), backup codes (second_factor.rs:115;
    user_private.rs:120), security keys and passkeys (webauthn:
    second_factor.rs:102,112; routes/session_2fa.rs:84), the 2FA page's
    HTML (:64), password reset with second factors
    (routes/accounts.rs:580), the login-required 2FA redirect
    (enforce_second_factor)
156. `feat/user-profile-rest` M: featured topics (users.rs:678;
    current_user.rs:709), the title, primary group and flair group
    (user_updater.rs:296), tag tracking (:330), PUT /u's custom fields,
    external ids, backgrounds, date of birth and sidebar links
    (user_updater.rs's REFUSED), array options (:821), the ignore
    notification level (routes/user_update.rs:265), another acting
    user's notification level (:257), the profile page's public user
    fields (87b), user tips and redirected_to_top
    (current_user.rs:1191,1214)
157. `feat/groups-rest` M: group flair uploads (site.rs:751; groups.rs:32;
    routes/groups.rs:588; group_writes.rs:565;
    routes/admin_groups.rs:1175), group custom fields and associated
    groups (routes/admin_groups.rs:224; user_destroyer.rs:472), rewriting
    a renamed group's mentions (jobs/handlers.rs:267,
    GroupMentionsUpdater), an editor who can no longer see the group
    (routes/admin_groups.rs:1379), ordering members by custom fields
    (routes/groups.rs:999), random username suggestions (:409), the
    group activity, messages and manage pages (:1541), group permissions
    with categories (:1336, after 190's CategorySerializer)
158. `feat/shared-drafts` M: shared_drafts_category (topic_query.rs:371;
    topic_guardian.rs:373,770) and publishing them
    (jobs/topic_timers.rs:132, TopicPublisher)
159. `feat/tags-reads-rest` S: tags_listed_by_group
    (routes/tags.rs:621), tag list filters other than latest, top, hot
    and votes (:268), the messages' tags pages (routes/users.rs:82,916),
    hashtag search over localized tags (hashtags.rs:571, with 168)
160. `feat/chat-threads` L: chat threads (plugins/chat/notify.rs:102,482;
    plugins/chat/create.rs:213,216,226; plugins/chat/modify.rs:243;
    plugins/chat/channels.rs:348; plugins/chat/page.rs:589;
    plugins/chat/summary_email.rs:223; plugins/chat/publisher.rs:367;
    plugins/chat/dm_groups.rs:540; plugins/chat/membership.rs:542), the
    threads and DMs indexes (routes/chat_pages.rs:200)
161. `feat/chat-uploads` M: chat message uploads
    (plugins/chat/messages.rs:276,588; plugins/chat/create.rs:99;
    plugins/chat/modify.rs:301; plugins/chat/page.rs:880,1315),
    oneboxes, images and videos in messages (plugins/chat/create.rs:747),
    chat message bookmarks (bookmarks.rs:266,743;
    bookmark_manager.rs:168), chat transcripts in posts (crates/markdown's
    bbcode refusal)
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
    search and starred routes (99)
163. `feat/solved-rest` S: discourse-solved's auto close timers, web
    hooks, crawler schema markup, search filters and lists by solved
    status (milestone 5), the dashboard's support section
    (admin_dashboard.rs:290,1350), accepted solutions' related items
    without a viewer (reports/counts.rs:1145)
164. `feat/site-settings-types` L: setting types beyond string, number,
    bool and enum: lists, uploads, groups, categories, objects
    (site_setting_update.rs:211,424,438), enum classes, regex and JSON
    schema validations (:534,588; setting_enums.rs:128), plugin settings
    (site_setting_update.rs:163), change handlers that write and custom
    validations (:177,180), default_* backfills and their user counts
    (:183; routes/admin_site_settings.rs:51,145,164;
    sidebar_backfill.rs:64), deprecated, upcoming and globally shadowed
    settings (site_setting_update.rs:152,174,186), bulk updates
    (routes/admin_site_settings.rs:43), an archived site
    (site_setting_update.rs:138)
165. `feat/reply-by-email-rest` L: bounces, forwarded emails, staged and
    unknown senders, attachments, unsubscribing, likes and notification
    levels by email, group and category addresses (email_in), replies to
    messages, spam and Authentication-Results headers, posts for the
    review queue or past a rate limit
    (email/receiver.rs:524,528,625,628,637,672,690,694,771,780,783,795,
    798,852,857), mailing list and resent emails and several senders
    (email/incoming.rs:418,441), POP3 polling, rebaking emailed posts
    (posting/rebake.rs:79; pretty_text/cooked_post_processor.rs:929,
    EmailCook)
166. `feat/group-smtp-imap` M: group SMTP and IMAP (email/sender.rs:287;
    jobs/post_alert.rs:443; routes/admin_groups.rs:1163;
    routes/groups.rs:610), the group SMTP credentials check
    (problem_checks.rs:342). Needs: network
167. `feat/locales` L: every locale but English: server strings
    (badge_granter.rs:409; system_message.rs:258; jobs/badges.rs:249;
    email/digest.rs:481; email/account.rs:50; review_posts.rs:931;
    routes/group_membership.rs:370; category_types.rs:453), site texts
    and reseeding (routes/site_texts.rs:64,915), the unsubscribe pages
    (routes/email.rs:45), email templates
    (routes/admin_email_templates.rs:221), theme translations
    (theme_writes.rs:273; routes/themes.rs:1107,1437), MessageFormat
    keys (routes/site_texts.rs:603), the default_locale entry
    (admin_site_settings.rs:145), slugs by transliteration
    (posting/text.rs:332), moment locales (49). Needs: the reference's
    other locale trees recorded (scripts/record-i18n-tree) and the
    client's
168. `feat/content-localization` M: content_localization_enabled
    (search/mod.rs:562; current_user.rs:1078;
    routes/sidebar_sections.rs:319; bookmarks.rs:410; review_list.rs:181;
    routes/drafts.rs:287; plugins/reactions/users.rs:665;
    plugins/solved/by_user.rs:47; about.rs:299; posting/rebake.rs:52;
    routes/users.rs:1279), 90c's sidebar content localization
169. `feat/theme-scss` L: new or changed scss fields
    (theme_writes.rs:250,263) and recompiling theme fields on restore
    (91c). Needs: an SCSS compiler with Discourse's theme pipeline
    (grass, as 88's email style uses, plus the color definitions, theme
    variables and imports Stylesheet::Compiler adds) and the stylesheet
    routes the port doesn't serve
170. `feat/theme-javascript` L: extra_js (theme_writes.rs:267,437),
    inline scripts in theme html (:608, AssetProcessor), theme settings
    migrations (:270). Needs: ThemeJavascriptCompiler's toolchain in
    process (Rails runs rollup, Babel and the Ember template compiler
    under mini_racer) and the theme-javascripts routes
171. `feat/theme-writes-rest` S: settings that request a refresh
    (routes/themes.rs:1163,1378, Discourse.request_refresh!, with 100),
    destroying a theme's own color schemes (theme_writes.rs:1258), a
    field over 1 MiB (:278), svg sprites (:539, with 90f's uploads), html
    that does not parse (:606)
172. `feat/admin-email-templates-rest` S: updating, showing and reverting
    customized email templates (routes/admin_email_templates.rs:407,444,
    478), unblocked by 90a's override layer
173. `feat/upcoming-changes` S: UpcomingChanges::List: what's new and
    toggle-feature (routes/admin_dashboard_sections.rs:439; the
    `not_yet` case admin_dashboard/dashboard_whats_new), undeclared
    changes and display conditions reading themes or categories
    (problem_checks.rs:546,576), upcoming change settings
    (site_setting_update.rs:174), unseen new features and their
    notifications (current_user.rs:779; notification_create.rs:231), the
    refresh_latest_new_feature job
174. `feat/ruby-regex` M: Ruby regexps the port can't run as Ruby does:
    watched words outside the engines' shared grammar, with full case
    folding or past the backtracking limit (word_watcher.rs:102,125,212,
    222,314,385,425,463,512; 80b), auto_silence_first_post_regex
    (posting/validate.rs:178,182), permalink_normalizations
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
    query parameters (pretty_text/cooked_post_processor.rs:174)
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
    (routes/admin_form_templates.rs:236,414, YAML.dump), YAML 1.1
    scalars, anchors and tags (form_templates.rs:95-201), revision
    values YAML can't hold (modifications.rs:22-55). Needs: a
    Psych-compatible YAML emitter
180. `feat/about-rest` S: the about page's visitor stats, banner image
    and localized settings (about.rs:125,302; about_view.rs:151), the
    welcome banner (html.rs:1236)
181. `feat/badges-rest` S: badge counts past a thousand, emoji in long
    descriptions, more grants than the first page, the title picker,
    grants without a time or topic (badges_page.rs:20,42,352,391,403,
    416), badge reason links to other routes
    (routes/user_badges.rs:126,169,173,175), grants for posts in messages
    (:237), badge preview columns of other types
    (badge_preview.rs:262,367), mass award CSV texts and encodings
    (admin_badges.rs:957; routes/admin_badges.rs:526,537)
182. `feat/drafts-rest` S: drafts backed up to a message (drafts.rs:86),
    clearing non-human users' drafts (:231), the edit conflict check on
    tags (routes/drafts.rs:231), reading a draft at a sequence and
    clearing another user's drafts (:315,342)
183. `feat/messages-rest` S: group message lists with publish_read_state
    (pm_lists.rs:151), period on message lists (routes/messages.rs:171),
    group_pm_alert past group_pm_user_limit (pm_invite.rs:524), inviting
    a user who now ignores the inviter (:254), the inbox of a group the
    viewer isn't in (routes/users.rs:747), the dismiss and bulk select
    buttons (33b)
184. `feat/user-deletion-rest` S: blocking a deleted user's links
    (user_destroyer.rs:458, block_urls), MCP authorizations (:475;
    user_private.rs:164), the user_delete_self log (user_destroyer.rs:266),
    deactivating with a pending review or associated accounts
    (routes/admin_users.rs:1318; jobs/accounts.rs:151), verbose auth
    token logging (routes/admin_users.rs:1237; user_anonymizer.rs:130),
    non-address IPs (user_anonymizer.rs:177)
185. `feat/maxmind-crawlers` S: MaxMind locations for reports and
    suspicious logins (reports/tables.rs:386; jobs/handlers.rs:92; 89d),
    CrawlerScorer (jobs/rollups.rs:164), enable_performance_http_headers
    (request_tracker.rs:465), keep_old_ip_address_count
    (session/current.rs:867). Needs: a GeoLite2 database
186. `feat/push-notifications` M: push notifications
    (jobs/post_alert.rs:1010) by web push and through user API keys'
    push clients (84). Needs: network
187. `fix/rails-error-answers` M: about 175 refusals of what Rails
    answers with an exception or never sees: params of the wrong shape
    (NoMethodError, ParameterMissing, InvalidParameters, TypeError),
    missing or vanished rows (RecordNotFound, a site without its system
    user, "gone once saved"), NOT NULL violations, staff actions missing
    from enums.json, onebox API values "of another shape", out of range
    numbers (routes/admin_embedding.rs, routes/admin_emoji.rs,
    routes/directory_columns.rs, routes/sidebar_sections.rs,
    routes/themes.rs, routes/color_schemes.rs, routes/admin_api_keys.rs,
    user_fields.rs, user_merger.rs, jobs/handlers.rs,
    request_tracking.rs, admin_traffic_explorer.rs, email/incoming.rs,
    onebox/generic.rs, onebox/api_engines.rs, onebox/github.rs,
    onebox/embed_engines.rs and others). Answer as Rails does where its
    answer is fixed (the 500 page, a 400, a 404); the rest stay refused

### Phase 10: what isn't built yet

The milestone work no phase item covered, and routes Rails serves that
the port doesn't route at all (a 404, not a refusal). Found by reading
src/routes/mod.rs against the milestones; no full diff against Rails'
route table has been made, so this list may be short.

188. `feat/topic-edits` L: PUT /t/:slug/:id (title, category and tags
    edits, refused since milestone 4), moving, merging, splitting and
    converting topics, topic bulk actions (dismiss new and new posts);
    none of it is routed
189. `feat/tags-writes` L: tags on topics (posts#create and update
    refuse them, routes/posts.rs), the composer's tag chooser (96), tag
    CRUD, synonyms, tag groups and tag uploads, watched words' tag action
    (watched_words.rs:129,286; routes/admin_watched_words.rs:68;
    posting/create.rs:312), unused tags with synonyms
    (jobs/cleanup.rs:340)
190. `feat/categories-writes` L: POST, PUT and DELETE /categories,
    categories#show with CategorySerializer (the `not_yet` cases
    groups/read_permissions_anon and groups/read_permissions_user1;
    group_manager.rs:498; routes/groups.rs:1336), creating a category's
    definition topic (jobs/consistency.rs:157; the `not_yet` case
    scheduled/consistency_category_definition), saving a category without
    its settings (user_destroyer.rs:1147,1181; post_owner_changer.rs:282),
    category types without Horizon (category_types.rs:268), the category
    editor (topic voting's toggle)
191. `feat/external-auth` L: OAuth2 and OIDC logins (/auth/:provider),
    DiscourseConnect (/session/sso*), the authenticators on /site.json
    and associated accounts (site.rs:1263; user_private.rs:130;
    admin_user_show.rs:185; current_user.rs:641,644;
    reports/lists.rs:297), external ids (web_hooks/payloads.rs:95;
    notifications.rs:538; session/api_key.rs:135;
    routes/user_api_key_auth.rs:296; group_manager.rs:753),
    auth_immediately and logout with only external logins
    (routes/login_required.rs:132; routes/session.rs:605), Twitter's
    login health check (problem_checks.rs:488). Needs: network
192. `feat/invites` L: InvitesController (/invites, invite links,
    redeeming), invite codes at signup (signup.rs:194), adding group
    members by email (routes/admin_groups.rs:601), messages to email
    addresses (posting/create.rs:620; pm_invite.rs:69), invite-only
    sites (reviewable_user.rs:52), inviting to restricted categories
    (topic_guardian.rs:519), invalidating invites on password reset
    (routes/accounts.rs:662)
100. `feat/message-bus` L, after 192, moved down from Phase 8: a
    compatibility layer, not a gap in the port's own pages, which get
    live updates over pg-bus's /bus already (MessageBus on pg-bus, so
    clients that speak Rails' protocol work against the port: bots and
    integrations on message-bus-client, apps acting through user API
    keys). Three branches, done together:
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
193. `feat/user-pages-rest` M: the user pages the port renders as the
    summary or refuses: preferences (every tab), notifications,
    /u/:username/badges and /activity/topics (97, ListController#topics_by),
    the drafts page, the other view_user_route values
    (routes/users.rs:433), the user badges page (routes/badges.rs:751),
    voted_by's page (routes/topic_voting.rs:170)
194. `feat/review-pages` M: the review queue's page (routes/review.rs:187),
    the user menu's review tab (routes/user_menu.rs:88;
    notifications.rs:270, pending_reviewables), the staff topic and post
    admin menus, show_user_menu_avatars (routes/user_menu.rs:50)
195. `feat/rss` M: the .rss feeds of the lists, categories, tags, topics,
    a user's posts and activity, topic voting's votes feed, and the
    bookmarks .ics calendar (session/user_api_key.rs:117; 84b). Only the
    badge feed exists (85b)
196. `feat/client-rest` L: the client's leftovers named under done
    items: the rich editor (ProseMirror) and its toggle, the composer's
    actions, heading, list, options and emoji menus, the insert-date
    modal (49, 96), the desktop header's search menu and the advanced
    filters panel (94), the do not disturb and user status modals (95),
    the drafts and post menus as sheets, the jump to post prompt (98),
    the topic map's menus, the footer's flag, mark unread and pinned
    buttons, the sidebar's header actions and Customize (milestone 4.5),
    the login, signup and logout dialogs (79c), the Edit Directory
    Columns modal (74). Split when started
197. `feat/filter-route` M: /filter (ListController#filter, not routed)
    with topic voting's votes filters; the topic_ids and state list
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
201. `perf/topic-view-queries` M: the topic view's per-post queries
    (milestone 2); bench above two connections needs a machine that
    holds its clocks (BENCH.md)
202. `feat/plugins-unported` L: the plugins the reference runs that the
    port doesn't: discourse-ai, gamification, calendar and events, data
    explorer (admin_dashboard.rs:1060), automation and
    discourse-workflows (username_changer.rs:310,319;
    user_anonymizer.rs:149,159; plugins/topic_voting/votes.rs:275;
    badge_granter.rs:1132), post-voting, voice, templates,
    narrative-bot's tutorials, math and graphviz in cooking
    (crates/markdown's bbcode refusal), zendesk; their data on user
    deletion (plugins/mod.rs:217,251), reports (reports/mod.rs:860,1036),
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
208. `feat/multisite` L: several forums from one process, by hostname
    (RailsMultisite)

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
- [ ] Unicode emoji in titles (`performEmojiEscape` turns them into codes):
      132
- [ ] Hidden and deleted posts (`hidden`, `deleted_at`, staff whispers stay
      hidden). Hidden posts serialize as Rails does (the review actions);
      deleted posts and topics for staff: 124
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
- [ ] Performance: the topic view's per-post queries; bench above two
      connections needs a machine that holds its clocks (BENCH.md): 201
- [ ] Group flair uploads, user fields, badges granted, auth providers,
      user color schemes on `/site.json`. User fields (87b) and user color
      schemes (90b) done; group flair uploads: 157; auth providers: 191;
      the keys parity still ignores: 199
- [ ] Muted defaults (`default_categories_muted`, `default_tags_muted`,
      `mute_all_categories_by_default`), shared drafts category: 129, 158
- [ ] `tags_listed_by_group`: 159
- [ ] Profiles: bios (PrettyText.excerpt), suspended/silenced users, user
      status, featured topics, generated letter avatars (`/letter_avatar`,
      for sites that turn the proxy off). Bios and suspended and silenced
      users done; user status: 130; featured topics: 156; letter
      avatars: 131
- [ ] Search: advanced filters (`in:`, `status:`, `category:`, `#`, `@`,
      `tags:`, `before:`/`after:`, `order:`), search contexts, `search_for_id`,
      rate limits, pg headlines. Rate limits done (21); the rest: 143, 144
- [ ] Plugin-added keys (solved, voting, reactions, ...) through the plugin
      protocol (milestone 5) instead of parity ignore lists: 202, 199
- [ ] Topics without a stored slug (`Slug.for`): 120
- [ ] RSS feeds (`.rss` for the lists, categories, tags, topics, a
      user's posts and activity): 195
- [ ] Multisite: several forums from one process, by hostname
      (RailsMultisite): 208

## Milestone 3: sessions

Logged-in readers, no writes yet.

- [x] Login: local (PBKDF2 `user_passwords`), CSRF, the `_t` and
      `_forum_session` cookies in Rails' own encrypted format so a cutover
      keeps sessions; `/session/current.json` byte-equal for members and
      admins
- [ ] OIDC / external logins, 2FA. The TOTP challenge for granting admin
      is done (86b); logging in with a second factor, backup codes and
      webauthn: 155; OIDC and the other external logins: 191
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
      messages (33c) and the required-fields redirect (87b) done; deleted
      topics and posts, ignored users: 124; category group moderators:
      149; shared drafts: 158; the 2FA redirect: 155; pending
      reviewables: 194
- [x] Bookmarks: `/u/:username/bookmarks.json` and
      `/u/:username/user-menu-bookmarks` (core's post and topic bookmarks;
      the `.ics` feed and plugin bookmarkables are refused)
- [ ] The other user-menu endpoints (`/u/:username/user-menu-private-messages`,
      `/review/user-menu-list`). The messages one is done (95);
      `/review/user-menu-list`: 194
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
      definition. Oneboxes (43, 51-55) and badges (57) done; images from
      other sites and hotlinked media: 140; optimized videos: 141; the
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
      thumbnails done (the post processor, above); the rest: 139, 131, 122
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
- [ ] PM emails, previous-replies context. PM emails done (above); the
      previous replies: 146
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
- [ ] The user menu, notifications, bookmarks, messages, preferences. The
      user menu with its bookmarks and messages tabs (95) and the
      messages pages (33b) done; the notifications, preferences and the
      other user pages: 193
- [ ] Review queue and topic status for staff: 194

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
      ported), presence, templates, narrative-bot, chat, poll's voting.
      Presence and poll's voting (39) done; solved's leftovers: 163;
      chat's: 160-162; voting's and reactions' follow core's items (195,
      143, 197); templates, narrative-bot's tutorials and the plugins not
      started: 202
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
      the themes' reads and writes (90d, 90e) done; left: 90f, 90g and
      169-171

## Not planned

- Running Discourse migrations: Discourse owns the schema; re-vendor
  `structure.sql` and re-snapshot instead
- Plugins with the host's permissions: native code in the process, or a
  subprocess with its own database connection (PLUGINS.md decision 3)
