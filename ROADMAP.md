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

## Next after the staging dropdown deploy: faster frontend delivery

Implemented and locally validated, including an optimized release build and
its focused browser checks. See [frontend packages](docs/frontend-assets.md) for the package
format, deployment path, checks and measured packaging times. Staging has
run frontend packages since November 3 (`35832017`) and took a JS-only
update (the `32f3f543` package) on November 6; see
[staging](docs/ubuntu-staging.md) for the current release.

The November 2 Categories/Tags fix required rebuilding the Rust application
because `src/assets.rs` embeds JavaScript and CSS. The release target also
had to compile dependencies. Fix this workflow before the next feature slice.

1. Measure clean, unchanged, JavaScript-only, CSS-only and Rust-only builds.
   Record toolchain, Cargo profile, flags, target directory and timings.
   Establish why the previous release artifacts were unavailable; do not
   assume every dependency rebuild is a Cargo cache invalidation problem.
2. Package frontend assets separately from the server. Start with the core
   JavaScript and CSS in `src/assets.rs`, preserving CSS ordering, content
   digests, versioned URLs, ETags and conditional responses. Load a validated
   asset manifest from an explicit release directory. Asset-only changes
   must not change Rust compiler inputs. Audit `build.rs`'s generated asset
   lists and markdown WASM separately before expanding the scope.
3. Deploy versioned asset directories with the existing server binary.
   Activate a matching manifest atomically, retain the previous release for
   rollback and keep older versioned assets available to already-open pages.
   A short service restart is acceptable initially; Rust compilation is not
   required for a JavaScript/CSS-only release.
4. Standardize release builds on the pinned Nix toolchain, consistent flags
   and a persistent target directory across worktrees. Keep test and release
   profiles distinct. Reuse the tested artifact for deployment rather than
   rebuilding it. Preserve the nested markdown WASM cache where valid.
5. Verify asset lookup, MIME types, missing files, digest changes, 304s and
   rollback behavior. Run focused browser tests against the packaged assets
   and repeat the guest-account staging check after deployment.

Done means a JavaScript/CSS-only edit can be tested and deployed without
invoking rustc, an unchanged release build compiles no crates, and a small
Rust-only edit reuses unchanged dependencies. Publish measured before/after
times rather than promising a build duration before measuring it.

## Current focus, 2026-11-01

The next delivery is the demo: core forum, usable chat and admin, then
bundled plugins, with a plain benchmark table at the end. The demo
script and rehearsal now cover 44 steps. Chat flags in /review (148a),
chat media and full-page search, and composer actions with the markdown
poll builder (196a) have merged. The committed browser suite exercises
these paths as well as navigation and live updates.

The November 1 integration passed 5,386 Rust tests (5 existing skips) and
70 browser tests. A subsequent chat attachment focus fix passed 14 focused
browser tests, including a new regression through the file picker and
lightbox. The final debug and optimized builds both passed all 44 demo
rehearsal steps with no failures or skips. Runtime source is at 1819a61f;
the local release report is
`tmp/rehearsal/release-20261101-focus/report/index.html`.

Remaining chat follow-ups include GIFs (224b) and further reviewable
cases (148b). Thread tracking (160b), the navbar's live thread indicators
(160b2), live user status (130c2) and the drawer's routes (223e) are done. The demo checklist's one open blocker is the rich
editor (196).
Performance item 285 is low priority. Full parity remains the long-term
goal; the older counts below are a dated snapshot, not current totals.

Source organization: composer lives under `src/composer/`; administration
data loading, views and handlers now live under `src/admin/`, with central
route registration retained in `src/routes/mod.rs`. These are structural
moves with no intended behavior changes. Categories, groups and review now
also have domain directories, with root aliases preserving existing imports.
The second batch added `src/tags/`, `src/themes/` and `src/notifications/`: 14
implementation files moved, with only module declarations and four relative
screenshot include paths adjusted. Combined validation passed all-target
workspace clippy, 21 domain unit tests, 25 integration tests and three dropdown
browser tests. Routes and runtime behavior are unchanged.

The third batch groups badges and invitations under `src/badges/` and
`src/invites/`, and collects search views and logs under `src/search/`.
Twelve implementations moved, with compatibility aliases and one relative
SVG include path adjusted. Validation passed all-target workspace clippy,
33 focused unit tests, 15 integration tests and 313 badge/invite write
parity cases. Use nextest's parity profile for those cases: concurrent
`cargo test --test writes` cases share the process-wide clock and can
interfere with session rotation and notification timing.

The fourth batch groups topic queries, permissions, lists, timers and
tracking under `src/topics/`; post operations and rendering under
`src/posts/`; and private-message lists, invitations, archives and tracking
under `src/messages/`. All 22 moved source files are byte-identical, with
root aliases preserving imports and central route registration unchanged.
Validation passed all-target workspace clippy, 34 focused unit tests,
67 integration tests and 592 isolated write parity cases covering messages,
posts, topics, post actions and topic timers.

User code now lives under `src/users/`: profile data and helpers at the
domain root, rendering under `views/`, and account updates, renaming,
merging, anonymization and deletion under `operations/`. All 21 moved
implementations are byte-identical; public user types and existing root
module imports are preserved. Routes and administration stay unchanged.
Validation passed all-target workspace clippy, 21 user unit tests,
60 integration tests and 422 isolated write parity cases covering users,
admin users, custom fields, avatars, user API keys and user search.

Plugin infrastructure has also advanced past the original milestone
summary: the compile-time Plugin trait and Wasmtime host are on main.
Bundled plugin ports remain Rust in src/plugins/. See docs/plugin-wasm.md
and the latest plugin audit entries for implementation and remaining gaps.

## Historical snapshot, 2026-10-20

On main at 16353d29. Open: 125 items, 42 S, 2 S/M, 63 M, 18 L (phase 8:
1, item 90; phase 9: 63; phase 10: 16, 257 and 258 found by the demo
smoke test; phase 11: 38; phase 12: 7, new in this sweep). Open inside them: 90g7a, 164b-164h, 188i3-188i5, 194c, 194d, 194e1, 194f, 228a-228c, 258a, 260a, 261b, 262a, 263a, 264a, 265a, 266a, 267a, 268a, 269a, 270b, 271a, 272a, 273a, 274a, 276a, 277a, 280a, 283a. Phases
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
  Playwright. A watched word score has no context when
  NewPostManager queues the post, so its words are found from the
  payload as Rails does (fix/watched-word-reason-guess; the queue had
  answered 500 on every real watched-word hold). Still refused: user reviewables (must_approve_users, 148);
  editing a queued post and the silence and suspend client actions are
  done (194e), a queued topic's tags field too (194e1)
- [x] The admin's site settings answered 500 saving a list setting such as
  authorized_extensions (164), so an admin couldn't allow video or audio
  uploads; the demo sites have them set in the database
  (docs/demo-servers.md): feat/list-settings (164a), done: every list
  type saves with Rails' validations, and the page's list editors open
  and save them.
- [x] A member couldn't start a private message from a profile, the
  messages page or /new-message (the rehearsal's step 10):
  feat/message-composers (282), done.
- [x] A member couldn't start a chat DM: the drawer's "Start a
  conversation", the sidebar's "Start DM or group chat"
  (/chat/new-message answered 405) and the profile's Chat button did
  nothing (the rehearsal's step 15): feat/message-composers (222), done.
- [x] Chat's emoji picker opened partly above the viewport: it took the
  first placement that fit the window, else its default unclamped.
  feat/message-composers, done: emoji-picker.js places it with float.js
  as DMenu's updatePosition does (offset, flip through
  FLOAT_UI_PLACEMENTS within the viewport less the header, shift on both
  axes); the reaction picker in the drawer lands where Ember's does at
  1280x900 and 1280x560 (rs-ref-d).

Found by the demo rehearsal (scripts/demo-rehearsal), the newcomer path:
fix/newcomer-path.

- [x] Every newcomer's first post was silenced as a fast typer however
  slowly it was typed: composer.js sent no typing_duration_msecs or
  composer_open_duration_msecs, so NewPostManager read 0. Done: the
  composer keeps Ember's typing time (composer-body's keyUp, a 100ms
  throttle adding 100ms) and open time (composerOpened while the compose
  state is open, composerTotalOpened across minimizing and fullscreen),
  sends both with a new post (_create_serializer; an edit sends neither,
  as _update_serializer), saves them in the draft (composerTime,
  typingTime) and takes them back from it as loadDraft does.
- [x] /review answered 500 for a silenced or suspended author ("authors
  under a penalty"), and approving a silenced newcomer's held post
  refused on the unsilence. Done: ReviewableSerializer's author_penalties
  (ReviewableAuthorPenaltySerializer: kind, forever, expires_at, the
  reason, applied_at and applied_by from the latest silence or suspend
  record, automatic, from_this_target, can_lift), FlaggedUserSerializer's
  silenced_till, silence_reason, suspended_till and suspend_reason, a
  flagged post's penalty_effect on each action and its unsilence_user
  action, and penalty actions offered only for a penalty not already in
  place. The card draws the penalty icons with their titles, the
  timeline's "Author silenced/suspended" event for a penalty from this
  reviewable or post, each action's effect after its description, and
  Ember's toast with Undo when an action leaves the author silenced.
  Approving a queued post unsilences its author (UserSilencer.unsilence
  with the reviewable). Recorded on rs-ref-c: review/penalized_authors_list,
  review/queued_approve_silenced_author. Still refused: performing a
  flag's unsilence_user (with the flag actions that delete or restore),
  a reason with markup (sanitize_staff_reason).
- [x] Saving a draft holding an upload answered 500 ("drafts with uploads
  (UploadReference)"). Done: Draft.set ends with
  UploadReference.ensure_exist! over Upload.extract_upload_ids(data) (the
  `/<40 hex>` sha1s and every `/<base62>` segment decoded), and the
  draft's references go with it when a post advances its sequence
  (DraftSequence.next!'s destroy_all), as Draft.clear already did.
  Recorded on rs-ref-c: drafts/draft_with_uploads,
  posts/reply_clears_draft_with_uploads.
- [x] A reply past max_consecutive_replies answered 500 instead of Rails'
  message. Done: PostValidator#force_edit_last_validator's error (422,
  "No more than %{count} consecutive replies are allowed...") when the
  member could edit the topic's last post, with the topic owner and
  category group moderators exempt and deleted posts out of
  ordered_posts. Recorded on rs-ref-c: posts/max_consecutive_replies,
  _one and _topic_owner.
- [x] The newcomer journey walked in Chromium on a copy of the demo
  database (enable_local_logins_via_code off): signup by password, the
  admin's activation, a first reply typed at a person's pace with an
  upload in its draft (created, not held, author not silenced), a first
  reply pasted in at once (held, author silenced, /review drawing the
  silence, approval lifting it), a watched word's held topic approved
  from /review (rehearsal steps 13, 16, 19), and a fourth reply in a row
  showing Rails' message in the composer. Step 19's check looked for an
  "Approved" text Ember doesn't draw; it now looks for the approved
  status badge ("Post approved").

Rehearsal findings (scripts/demo-rehearsal, 2026-10-24),
feat/flag-and-penalty-modals:

- [x] Flagging a post did nothing: the post menu's flag button and the
  topic footer's had no modal (step 17). The flag modal ported
  (static/js/flag-modal.js, src/flag_modal_view.rs): the flags the viewer
  may use with their descriptions, the message and its length count,
  the illegal confirmation, take action for staff (hide, or suspend and
  silence through the penalty modal with the flag made first) and Delete
  Spammer; the post drawn again after, its flag button gone, and staff's
  count of a reviewed post's flags (which opens /review) as Ember draws
  it. The backend's flags that send a message and topic flags are ported
  with it (post_actions/flag_notify_*, flag_illegal_with_message,
  flag_spam_with_message, flag_topic_*). ~~Still refused, saying so in
  the modal: official warnings, queueing for review, flags in messages
  (147).~~ (done in `feat/flag-modal-rest`: all three, Delete Spammer's
  email lookup and the penalty modal over a chat message; ~~queueing a
  topic's first post~~ (done in `feat/review-hidden-posts`, with
  restoring a queued post from /review: disagree_and_restore); left,
  named: for a chat message official warnings and queueing for review)
- [x] Admin silence and suspend answered "Not ported yet: the silence modal"
  on the admin user page (step 23): PenalizeUser ported
  (static/js/penalize-user.js, src/penalize_view.rs) with DFutureDateInput's
  shortcuts and custom date and time, the reason presets, the message,
  the post actions when it comes from a post, the similar users and the
  penalty history; the penalty times it sends ("YYYY-MM-DD HH:mmZ") are
  parsed (admin_users/silence_from_penalize_modal,
  suspend_from_penalize_modal). The member gets the "Account temporarily
  on hold" message; unsilence and unsuspend work from the page. Still
  refused: the delete posts decider after "delete all posts". The
  review queue's silence and suspend open it too (194e).
- [x] /admin/reports to a single report raised a script error: the list's
  filter ran on the report page after a page swap (step 21).
  admin-reports.js now acts only where the index template's AdminReports
  controls are, as Ember's index and show routes render different
  components under the same wrapper.
- [x] The chat read marker sent message_id=NaN for the just-sent staged
  message, answered 400 (step 14): staged messages no longer count as the
  last visible one, as firstVisibleMessageId's id never matches a
  persisted message in Ember.
- [x] Ticking a checklist box sometimes answered 422 "The checkbox could not
  be updated." (step 8): the unique post check stood in for Rails' Redis
  key with any recent post of the user whose current raw matched, so a
  tick whose new raw equalled another post's ticked raw was refused as a
  duplicate, and a body re-posted after the first copy was edited wasn't.
  Ported as Rails keeps it: PostCreator stores `unique-post-<user>:<sha1>`
  for unique_posts_mins in discourse_rs.cached_values and the validator
  reads it (checklist/toggle_to_a_revised_posts_raw). The rehearsal's
  topic body carries its label, as Rails refuses a second run's copy.
- [x] The moderator's header avatar had no count for the pending flag
  (step 18): the header drew only the unread notifications count, and
  NotifyReviewable didn't publish. Done: 194g, the avatar's badges by
  Ember's precedence, drawn with the page and kept current live.

Visible if clicked, avoid them in a scripted demo:

- [x] Polls draw a "0 voters" stub and can't be voted on (228):
  feat/poll-ui-vote (228a), done: the widget votes, changes and removes
  votes; its results say they are not ported yet (228b), as do ranked
  choice polls and the actions menu (228b, 228c). Both done since:
  the results (228b), close, open and the breakdown (228c); export
  waits for data explorer (202).
- [x] No Build Poll (229): feat/poll-builder-widgets, done: Build poll
  draws its groups, automatic close and results with Discourse's
  GroupChooser, DDateTimeInput and ComboBox, and shows in a PM with
  discobot for members who may not create polls.
- [x] A post with an uploaded video or audio file is refused (138, 141):
  feat/post-video-audio, done: such posts are created and edited, the
  video drawn as its placeholder with the play button and the audio as
  its player; the composer doesn't make a video's thumbnail yet (139).
- [x] Checklists can't be checked (242): feat/checklist-toggle, done,
  boxes toggle with their batching, retries and optimistic drawing. User
  status can't be set (130): done in 130a, 130b, 130c and 130c2.
- [x] The review queue's claims and notes are refused (194), and the topic
  admin wrench isn't drawn (194, 262a): the wrench and the post admin
  menu, feat/topic-admin-menus (194a), and claims and notes,
  feat/review-claims-notes (194b), done; the topic menu's actions
  (feat/topic-admin-actions, 194c) and the banner atop pages
  (feat/site-banner, 194c2) done but the timer modal, multi-select and
  nested replies (194c1); the post menu's actions done
  (feat/post-admin-actions, 194d) but its owner, badge, permanent
  deletion and publish modals (194d1).
- [x] The review queue's client actions (edit, suspend, silence) and
  editing a reviewable (194e): done, feat/review-client-actions, with
  the raw email modal, and the tags field (feat/review-edit-tags,
  194e1); the revise modal was done before (feat/review-queued-posts).
  Not a demo blocker: the reject reason modal (194e2), which only a
  user reviewable opens, and the queue lists none yet.
- [x] The composer's rich editor and its toolbar buttons (196r1 to 196r6,
  done: the core and the composer's toggle, the everyday extensions, the
  toolbar and its menus with UpsertHyperlink, oneboxes and the preview
  toolbar, the plugins' extensions, rich as the default with the
  profile's About me editor, and the options menu's Insert table and
  Change reply to with chat's transcripts).
- [x] The add participant modal on a message (262a): feat/add-pm-participants
  (262b), done.
- [x] The grouped bookmarks menu (274a): fix/profile-polish-rest, done;
  editing a bookmark from it opens BookmarkModal (289).

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

284. ~~`feat/browser-tests`~~ (done) M: nothing runs the client. static/js is about
    67k lines in 112 files and no test executes a line of it:
    demo_smoke.rs drives the endpoints the scripts call, not the scripts,
    and every browser check so far (the playwright comparisons against
    rs-ref-a/b/c, 223b, 223d and 224's harnesses) was a one-off run left
    out of the repo. Add a committed browser suite:
    - Playwright pinned through nix (nixpkgs' playwright-driver and its
      browsers, in shell.nix and the flake), no npm install at test time,
      no network.
    - tests/browser/*.spec.js and a harness that boots the server on a
      TestDb-style throwaway copy on a free port, with the job worker,
      runs the specs, and tears both down; `make browser-test`, kept out
      of `make lint`'s loop, and its own CI job.
    - First specs: the demo script's steps that need the client
      (composer open, write and post; the post menu and its plugin
      buttons; chat send and live arrival in a second session; the chat
      drawer; dark mode; the review queue's actions), then 223b's, 223d's
      and 224's harnesses folded in (resize, back and forward, paging and
      jumps).
    - From then on a branch that changes static/js adds or changes a
      spec; agent briefs say so.
    Comparing against a running Rails reference stays a manual step
    (parity/ does the HTTP side).
    Done: nixpkgs' @playwright/test and playwright-driver's Chromium
    (nix/playwright.nix, in shell.nix and the flake's browser-test
    shell, one Playwright version from one nixpkgs); scripts/browser-test
    builds the server, clones the test database as TestDb does (swept by
    drop-stale-test-dbs if killed), applies tests/browser/setup.sql,
    boots it with the job worker on a free port, runs the specs and
    stops that server and drops that copy on any exit; `make
    browser-test` and CI's browser job. 25 specs in about 45 seconds:
    the login modal and its flash; the composer's reply with the
    preview, the post arriving without a reload and the mention's count
    arriving live in the mentioned member's header, a new topic, the
    discard question; the post menu's show more, copy link, bookmark,
    flag modal and its plugin buttons (reactions' like and picker,
    solved's accept and unaccept with the title status, assign's modal
    and unassign), each checked in the database; chat sent from the
    drawer arriving live in another member's full-page chat, a
    reaction, the drawer kept across a page swap, collapsed and
    expanded, full page and back, closed, "-" and Esc, its resizer and
    the size kept; dark mode from the sidebar's color selector, kept by
    the next page, and auto; a flag, a note and agreeing in /review, a
    watched word's held post approved. 223b's, 223d's and 224's
    harnesses kept as fixture specs over page.route. Found and fixed:
    the collapsed post menu drew the flag button (its double-button
    wrapper's display beat [hidden]), and a refused read marker
    (another session read further) popped an alert and an unhandled
    rejection in chat.js, where Ember shows nothing. Then
    history.spec.js: back and forward on the server's own pages (a
    boosted forum page restored in place, 223d's chat URL opening in the
    drawer, forward to the forum page, full page preferred loading
    full-page chat, the focus on a page swap); and assign redrawing the
    topic in place (title, footer buttons, the drawn posts, from the
    topic page as it is now, as /staff/topic-assignment updates Ember's
    topic) instead of reloading, the topic-level assign spec'd too.
    Found and fixed there: full-page chat loaded directly (not reached
    from the drawer) opened the drawer on /chat, not its channel, from
    its open drawer button, as chat-drawer.js read lastKnownChatURL
    before leaving the page that stores it; Ember reads it in the
    then().

285. `perf/rate-limit-buckets` M, low priority: discourse_rs.rate_limits
    keeps RateLimiter's Redis list as a bigint[] of epoch-second stamps,
    and rate_limit_perform rewrites the whole array, under the row lock,
    on every request. Since fix/rate-limit-trim-by-age stamps more than a
    window old are dropped, so a list holds one window of requests at most
    (and max at most): at Rails' default limits (search's largest is 150
    a minute) that is cheap, but a raised limit pays for every request in
    the window. The bench snapshot's 1,000,000 anonymous search limits
    show it: back-to-back no-match searches go 1,427 / 651 / 486 / ... /
    276 req/s as the minute window fills to about 21,500 stamps, every
    anonymous search queued on the global key's row
    (bench/20261030-rust/summary.md). Store (second, count) pairs instead
    (at most secs + 1 of them, a running total beside), so a push updates
    the newest pair: the same answers (perform's allowed and wait,
    can_perform?, remaining, rollback! popping the newest, seconds_to_wait),
    the table dropped and recreated (unlogged scratch state), the
    MemoryStore and its save/load kept in step, and the trim test plus a
    sustained-load check that stays flat.

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
`now() - interval ...` in setup rather than relying on seed dates. A
branch that changes static/js adds or changes a spec in tests/browser
and runs `make browser-test` (README: Browser tests); a brief for one
says so.

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
    email or plugin fields, unhiding a first post (unsilencing the author
    on approval landed with fix/newcomer-path);
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
    crate::admin::setting_links), the JSON keeping the server's link.
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
    (refused), ~~the do not disturb and user status modals~~ (done: the
    status modal in 130b, DoNotDisturbModal in
    `feat/do-not-disturb-modal`: the profile tab's Pause notifications
    item carries the modal's strings while off and opens it
    (static/js/user-menu.js): a DTapTile per duration (30, 60, 120,
    tomorrow; click or Enter) saved by POST /do-not-disturb.json, the
    error as the modal's flash, the notification schedule button to the
    notifications preferences. The endpoints, the item's on state and
    the header's moon were there. Tests: sessions
    the_user_menu_pauses_notifications, browser do-not-disturb (3).
    Left: the client's other readers of isInDoNotDisturb, not ported:
    the push subscription (186), the notification sound; the title's
    and favicon's counts are 296's, desktop notifications 297's),
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
    chat footer. Left: the footer's ~~DMs~~ (done, 280), search, ~~starred~~
    (done, 162) and ~~threads~~ (done, 280; its threads with 160)
    routes, the channel list options menu (desktop's too), swipe and long
    press on a row, the list's filtered empty states (the unfiltered one done, 162)
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
130. ~~`feat/user-status`~~ (done in 130a, 130b, 130c and 130c2) M: user status (UserStatus, the /user-status
    routes, the status modal) wherever a user is serialized (users.rs:540;
    current_user.rs:304; about.rs:253; bus.rs:476; directory.rs:650;
    topic_view.rs:1766; routes/groups.rs:867; routes/admin_api_keys.rs:74;
    routes/deleted_posts.rs:55; plugins/chat/direct_messages.rs:237;
    plugins/chat/membership.rs:128; plugins/chat/messages.rs:636;
    plugins/chat/channels.rs:642), the preferences' status
    (preferences_view/mod.rs:942) and PUT /u's `status`
    (user_updater.rs's REFUSED). Split:
    130a. ~~`feat/user-status`~~ (done) M: the model and its routes
    (users/status.rs, routes/user_status.rs): GET, PUT and DELETE
    /user-status behind requires_login and enable_user_status (404),
    User#set_status! (UserStatus' validations in order: Emoji.exists?,
    the 100 characters, an end before set_at; the RecordInvalid 422), the
    upsert, #clear_status!, #publish_user_status (trust_level_0, or the
    user and staff for a silenced one, nothing for a hidden profile), an
    ends_at cast as ActiveRecord casts ISO 8601 (other formats refused);
    PUT /u's status (blank clears, a RecordInvalid rolls the whole update
    back, the cleared status still in its response as Rails keeps the
    destroyed record); UserStatusMixin's `status` (has_status?,
    hide_profile, can_see_user_status?) on users#show and its card (the
    badges' sideloaded users too), current_user, group members, the
    posts' user_status and mentioned_users (no scope: Guardian.new) on the
    topic view. about, directory, TopicStats' last poster, API keys and
    deleted posts pass no include_status, so their refusals went. Rails
    has no job clearing ended statuses. Client (user-status.js,
    css/user-status.css): the user menu's item, UserStatusModal with the
    emoji picker, the description (speech_balloon by default), pause
    notifications (do not disturb), the clear-after tiles (one hour, two
    hours, tomorrow, never) and delete; the header bubble; the emoji by a
    poster's name with its tooltip, on mentions in posts and on the user
    card; the account preferences' row saved with the tab; /live redraws
    the bubble and the topic's posts by a user whose status changed.
    Write cases parity/writes/user_status; tests/browser/user-status.spec.js.
    Left, as 130b.
    130b. ~~`feat/user-status-chat`~~ (done) M: the status in chat's
    serializers with Rails' viewers: MessageUserSerializer and the
    mentioned users (Chat::MessageSerializer, so the messages list, the
    pins, search and every Chat::Publisher payload that carries a
    message) and ThreadOriginalMessageSerializer's user and mentioned
    users get include_status without a scope (Guardian.new: a silenced
    user's status shows to no one there); ChatableUserSerializer (DM
    chatables, /chatables) and the membership and members list users
    get it with the viewer's scope; in-reply-to users, reactions,
    pinned_by, thread previews and the publisher's other users get none,
    as in Rails. Client: the emoji by the author's name
    (ChatMessageInfo), on mentions in cooked messages
    (updateUserStatusOnMention's UserStatusMessage), after a one on one
    DM's title (ChatChannelName, in rows, titles and the new message
    list; never with its description, as `this.site` is not injected
    there) and on its sidebar link (ChatSidebarIndicators), in the
    members list and the message creator's user rows with the
    description, and in the chat composer's mention autocomplete
    (initUserStatusHtml, renderUserStatusHtml). discourse-cakeday's lists
    and discourse-gamification's leaderboard pass no include_status in
    Rails, so their refusals went (no status). /live redraws the topic's
    posts mentioning a user whose status changed, as the mentions
    decorator tracks them. Rails' DM messages' available_flags keep a
    nil where a notify flag is left out (`next` inside `map`); so does
    the port now. Write cases parity/writes/chat/user_status*,
    parity/writes/user_status/cakeday_anniversaries;
    tests/chat_threads.rs (the bus, the page),
    tests/plugin_gamification.rs, tests/browser/user-status.spec.js.
    Left, as 130c.
    130c. ~~`feat/user-status-picker`~~ (done) M: the status modal's
    custom date and time tile was refused by name. Its tiles are now
    DTimeShortcutPicker's (289's Discourse.timeShortcutPicker, as
    UserStatusModal mounts it): in one hour, in two, tomorrow, the
    custom tile with DatePickerFuture, the time input and
    DRelativeTimePicker, and never (the none tile's label), no last
    custom tile; the end is the time picked (onTimeSelected). The
    modal's own tile code and its strings went. The core composer's
    mention autocomplete, with its statuses, is done in 130c1, chat's
    live status tracking in 130c2. MessageBus.last_id("/user-status")
    stays 0 with item 100's placeholders. Tests: browser user-status
    "the custom tile ends the status at a date and time"
    130c1. ~~`feat/composer-autocomplete`~~ (done) M: d-editor's
    autocompletes in the core composer, in both modes (docs/rich-editor.md,
    In the composer). static/js/autocomplete.js is the DAutocompleteModifier
    over an AutocompleteHandler (the textarea's, with caret-position's
    placement at the term; ProsemirrorAutocompleteHandler in
    rich-editor/text-manipulation.js, completing a term as its markdown
    parses into a mention, hashtag or emoji node), triggerRule's
    inCodeBlock and inLink, onRender, destroy, and the shared
    lib/user-search (eager completion in a topic or category, the cache
    per url and parameters, the rate limit wait, emails, exclude,
    organizeResults' order), lib/hashtag-autocomplete (the client's
    hashtag types only) and the results' components, which chat now uses
    too. composer.js applies _applyMentionAutocomplete (search_context
    mention, the topic, its category and replyingToUserId, statuses),
    _applyHashtagAutocomplete (topic-composer's order) and
    _applyEmojiAutocomplete (emoji_autocomplete_min_chars, favorites or
    the defaults, emoticon translations, skin tones, denied emojis, "more"
    opening the picker by the caret) to the textarea once and the rich
    editor each time it opens; none triggers in code (the preview's engine
    for the textarea). The rich editor context carries the settings,
    denied emojis and strings. Tests: browser composer-autocomplete (31),
    rich-editor-autocomplete (13) and autocomplete (13: user-search-test.js
    and user-autocomplete-results-test.gjs as written); the endpoints'
    request specs are parity/writes/user_search and parity/writes/hashtags.
    Not here: the other d-editors' autocompletes (About me, review edits,
    admin texts). ComposerMessages, which the mention warnings feed, is
    213's.
    130c2. ~~`feat/user-status-rest`~~ (done) M: chat's statuses showed
    as of the page's render (each message as of its live redraw). Now a
    /user-status change redraws them in the page
    (user.statusManager.trackStatus): the page's stream (routes/live.rs)
    sends a `user-status` event with each changed user's id, username
    and status (null once cleared) to whoever /user-status' audience
    lets hear it, and static/js/user-status.js draws them again by a
    message's author (ChatMessageInfo), on mentions in messages
    (updateUserStatusOnMention), after a one on one DM's title
    (ChatChannelName, in titles, rows and the new message list; the
    label names its user in data-status-user-id) and in the members'
    (ChatUserInfo) and the message creator's rows with the description;
    the sidebar's chat sections are drawn again by the server for a
    DM's link (ChatSidebarIndicators). Tests: browser user-status (the
    chat test follows a changed, cleared and new status in each place;
    the message creator's rows)
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
    user's queued post (posting/create.rs:331), flagging a new user's
    linked posts as spam once has_host_spam? refuses a post
    (flag_linked_posts_as_spam, posting/create.rs:634), flag_sockpuppets
    (:681), silencing a TL0 user without keep_posts
    (user_penalties.rs:929, hide_posts; chat's rate limit silence), TL0
    bio links (users.rs:708, strip_links). max_consecutive_replies landed
    with fix/newcomer-path
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
147. `feat/flags-rest` M: ~~official warnings (is_warning, flags.rs:204;
    PostCreator's is_warning and UserWarning)~~ (done in
    `feat/flag-modal-rest`: TopicCreator#create_warning through the
    flag's message, the moderator_warning subtype and the user_warnings
    row; POST /posts still refuses is_warning), undoing flags
    (routes/post_actions.rs), ~~flags in messages (flags.rs:129), staff
    queueing a post for review (:177)~~ (done in `feat/flag-modal-rest`:
    a message's posts flagged as any other; queue_for_review hides a
    reply at once with reason queued_by_staff and its system message;
    a topic's first post, which unlists the topic, in
    `feat/review-hidden-posts`),
    hiding on a trusted user's spam flag (flags.rs), silencing new
    users on spam flags and the scoring behind it (flags.rs;
    guardian.rs:392; current_user.rs:909). Flags that
    send a message (notify user, notify moderators, illegal, spam with a
    message) and flagging topics are ported
    (feat/flag-and-penalty-modals); flags on posts already in review
    (the reviewable's new score, reopening a reviewed one, and
    cannot_flag_again?'s refusal) are ported (fix/flags-on-reviewed-posts).
148. `feat/review-actions-rest` L: review actions that delete, edit,
    restore or penalize (review.rs:107; user_penalties.rs:509), on
    ~~hidden or~~ deleted posts (review/mod.rs), ~~hiding a topic's
    first or last post (:698)~~ (done in `feat/review-hidden-posts`: a
    hidden post's disagree_and_restore, and disagree by its alias, run
    perform_disagree's undo: the flags_disagreed message from human
    staff, Post#unhide! as the post admin menu's, the author unsilenced
    when silenced for the post, before the unhide as the unsilencing
    writes outside the perform's transaction; its ignore is the system
    user's, so AutoQueueHandler expires it and it stays hidden; the
    system user's disagree settles auto_action_flag_types; Post#hide!
    unlists the topic of a first or last visible regular post as the
    system user, and queue_for_review unlists a first post's as the
    flagger first; left: the delete_and_* actions on a hidden post as
    on any, a hidden post in a deleted topic, DSA statements and appeal
    instructions, dsa_reporting_enabled, and a first post's hidden
    styling and its topic's unlisted status drawn live, which /live/post
    leaves to the next page load),
    claims and notes (review_posts.rs:257;
    reviewable_user.rs:195,272,446), the other reviewable types
    (review.rs:247; review_posts.rs:211), user reviewables in
    the list (review_list.rs:594,599: ReviewableUserSerializer, the
    user.gjs card with its reject reason modal, approve_user), queued
    posts approved into closed or
    deleted topics, with tags, an email, plugin fields or polls,
    unsilencing their author, unhiding a first post
    (review_posts.rs:464,476,610,624,636,1262), user reviewables on
    invite-only sites or with an uploaded avatar and their rejection
    email (reviewable_user.rs:52,82,316), score conversations and types,
    reasons whose text the restricted cook would change, watched word
    scores without their words, the date and additional filters
    (review_list.rs:205,208,216,1047,1100,1246,1266), reviewable web hook
    payloads (review_list.rs:602,2196,2233,2278 and `web_hooks::refuse` for
    the reviewable events), the single reviewable page (review.show,
    ReviewablesController#show: GET /review/:id is the permalink route),
    the /reviewable_action publish (Reviewable#notify_users), the page's
    warning of unknown reviewable types (meta has the list). Plugin
    reviewables are 148a's and 148b's
148a. ~~`feat/plugin-reviewables`~~ (done) M: plugin reviewables in the
    queue, chat's first. Chat::ReviewableMessage is stored as Rails stores
    it (type `ReviewableChatMessage`, type_source `chat`;
    `reviewables::types` maps the class names meta and the type filter use
    to the stored names, chat's while chat_enabled), listed and drawn
    (Chat::ReviewableMessageSerializer: the live message's cooked and
    chatmessage_id, the payload's message_cooked, message_uploads and
    transcript_topic_id, the embedded chat_channel; chat's
    reviewable/chat-message.gjs with the channel title linking to the
    message, the author, uploads and the transcript link; "Chat flagged
    as"), with build_combined_actions under chat's translations and
    penalty_effect_for, shared with perform (src/plugins/chat/review.rs,
    delete_and_ignore too); a staff flag's take_action agrees at once
    (agree_and_delete, publish_delete!); update_flag_stats enqueues
    Jobs::TruncateUserFlagStats, ported, for post and chat flags alike;
    the ids filter, which review.js now redraws acted on items with;
    meta's unknown_reviewable_types_and_sources. discourse-post-voting
    creates no reviewables in the port (its comment flag is refused,
    plugins/post_voting/comments.rs). tests/chat_reviewables.rs,
    tests/browser/review-chat.spec.js. The rest is 148b
148b. `feat/plugin-reviewables-rest` M: the companion message a chat
    notify flag with a message sends (plugins/chat/flags.rs) and the score
    conversation its meta_topic_id brings
    (ReviewableScoreSerializer#reviewable_conversation, refused for post
    flags with a message too, review_list.rs "score conversations"), the
    transcript a direct message's flag sends, flags of messages with
    uploads (the payload's message_uploads), queue_for_review, and
    discourse-post-voting's ReviewablePostVotingComment with the comment
    flag that makes it (plugins/post_voting/comments.rs)
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
    current_user.rs:722), PUT /u's custom fields
    (user_updater.rs's REFUSED; the date of birth landed with
    discourse-cakeday, feat/plugin-cakeday), user custom fields with several values
    (current_user.rs:467; routes/groups.rs:1122; review_list.rs:1411),
    the profile page's
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
    (routes/tags.rs:661), ~~tag list filters other than latest, top, hot
    and votes (:286)~~ (done in `feat/tag-unread-and-pm-dismiss`,
    270b), the messages' tags pages (routes/users.rs:85,1214),
    tag and hashtag search over localized tags (hashtags.rs:571;
    tagging.rs:569; routes/tag_search.rs:161; routes/search_menu.rs:380;
    routes/tag_admin.rs:1157, with 168)
160. ~~`feat/chat-threads`~~ (done) L: chat threads end to end
    (plugins/chat/threads.rs, thread_view.rs). The model: a reply in a
    channel with threading (or into a forced thread, or with a
    thread_id) is the thread's (Chat::CreateMessage's thread policies,
    thread_invalid_for_channel and thread_does_not_match_parent; the
    channel's last message and the sender's last read stay; the
    original message's author and the sender join, Chat::Thread#add
    with publish_user_has_threads for a first thread; the reply that
    starts one publishes thread_created), hidden threads as before;
    the mentioned join a shown thread at normal (upsert_user_mentions).
    The API: GET /chat/api/channels/:id/threads (LookupChannelThreads,
    FetchThreads' order, Chat::ThreadListSerializer), GET and PUT
    .../threads/:thread_id (LookupThread, UpdateThread), POST .../threads
    (CreateThread), GET .../threads/:thread_id/messages
    (ListChannelThreadMessages, Thread::Policy::MessageExistence), PUT
    .../read (UpdateUserThreadLastRead with MarkMentionsRead), POST
    .../drafts (UpsertDraft's check_thread_exists, also on the channel's
    drafts route), PUT .../notifications-settings/me, POST
    .../mark-thread-title-prompt-seen/me, GET /chat/api/me/threads
    (LookupUserThreads); Chat::ThreadSerializer with its preview,
    original message and participants (ThreadParticipantQuery),
    BaseThreadMembershipSerializer; the message serializer's thread_id
    for forced threads, thread and thread_title; MessagesQuery's thread
    clauses (a forced thread's replies out of an unthreaded channel, a
    thread reply target left out); ThreadUnreadsQuery for the messages'
    and lists' tracking; me/channels' unread_thread_overview and
    has_threads. The publisher: calculate_publish_targets for sent,
    processed, edit, refresh, restore, reaction and delete (a thread's
    latest message for a reply's delete), new-messages "thread" with the
    preview after, publish_thread_original_message_metadata with the
    NullMessage, user tracking state with unread_thread_overview and
    thread_tracking. Notifications: chat_thread_id and the thread url in
    mention notifications and alerts, NotifyWatching's thread watchers
    with chat_watched_thread notifications consolidated as
    watched_thread_message_plan does, action_data's thread_id. The
    summary email counts watched threads (unread_threads, its emailed
    marker) and names them in the subject (watched_thread(s)); the
    joinable filter drops a whole channel as Rails does.
    Jobs::Chat::MarkAllChannelThreadsRead. The pages: the thread
    indicator (ChatMessageThreadIndicator, participants) under an
    original message, no reply line in a thread, the threads list button
    with its unread count, /chat/c/:slug/:id/t (ChatThreadListHeader,
    ChatThreadList items), /t/:thread_id, /t/:thread_id/:message_id and
    /:message_id/t/:thread_id (the channel route with ChatSidePanel and
    ChatRoutesChannelThread: ThreadHeader's back link, title, tracking
    dropdown, settings button and close button, ChatThread with
    ChatThreadHeading and ChatComposerThread), /chat/threads with the
    member's threads (UserThreads, ThreadPreview on a phone; 280a's
    threads part), /chat redirecting to it for my_threads.
    static/js/chat-thread.js runs the thread pane (its composer and
    staged messages, its draft, its live messages from
    /chat/:id/thread/:id drawn by GET /live/chat/:id/:message_id
    ?context=thread, its read state, the near-message urls replaced),
    chat.js redraws an original message on thread_created and
    update_thread_original_message. Thirty-six write cases recorded on
    rs-ref-a (parity/writes/chat/thread_*, threads_index*,
    create_in_thread*, create_threaded_reply, create_thread_mismatch,
    create_in_forced_thread, channel_messages_threaded,
    channel_messages_forced, me_channels_threads, me_threads_unread,
    edit_in_thread, trash_in_thread, restore_in_thread, react_in_thread,
    watched_thread_reply, mark_all_read_threads, channel_read_threaded,
    summary_email_watched_thread), the reply counts ignored (Rails keeps
    them in Redis, which a recording's rollback leaves behind; the port
    counts live replies, what the warm count holds); me_threads_listed
    matches now; tests in tests/chat_threads.rs. The page markup is
    ported from the components, not compared against the reference.
    Left, split into 160a.
    160a. ~~`feat/chat-threads-rest`~~ (done) M: the thread pane in the
    browser. chat.js's composer takes either textarea (composerParts by
    input): the thread's (ChatComposerThread) sends with thread_id,
    stages in the thread's pane, saves the thread's draft, edits with Up
    and drops an edit on Escape (else closes the pane), has no replies,
    reports /chat-reply/:id/thread/:id, and gets the emoji picker, the
    autocompletes, its own replying indicator and its own uploads box
    (picker, paste, drop on the thread, the dropdown's upload button);
    the messages' actions work in the thread (MESSAGE_CONTEXT_THREAD:
    no reply, copyLink's thread url, the thread's own select mode
    without move), and the channel's Up, "+emoji" and selection keep to
    the channel's messages (they took the thread's beside it). The
    thread route draws ChatModalThreadSettings (the title's click for
    admins and the original message's author, the gear for staff and
    the author; PUT .../threads/:id, the header and the heading drawn
    again) and ChatThreadTitlePrompt's toast on a phone (an untitled
    thread of five replies, show_thread_title_prompts, not seen yet;
    mark-thread-title-prompt-seen, "don't show again" saving the user
    option). static/js/chat-thread.js pages the thread (GET
    /live/chat/:id/messages?thread_id=, Context::thread_messages_page:
    past and future from the first and last shown, around a target or
    from the first message; fillPaneAttempt, the arrow to the latest,
    jumps to a message of the thread in place), sizes the side panel
    and drags its resizer (discourse_chat_side_panel_size_width, 250
    each side), picks the back link from the previous chat route
    (ChatHistory, kept per tab in sessionStorage: the threads list after
    the threads list or the channel on desktop, my threads after my
    threads), and runs the lists' load more (GET
    /live/chat/:id/threads and /live/chat/threads, ten at a time) and
    their live updates (ChatThreadList's /chat/:id: an original message
    deleted or restored or a preview changed draws the loaded pages
    again; UserThreads' channels: a preview drawn again in place).
    Unread threads count in the chrome (plugins/chat/view.rs from
    unread_thread_overview): the sidebar's My threads section with its
    dot (urgent for watched), a channel link's dot and the sidebar sort
    and filter with threads replied to since it was viewed
    (unreadThreadsCountSinceLastViewed, #sidebarUnreadCount,
    #compareDirectMessageChannelsByActivity), the header icon's and the
    phone back button's dot (hasUnreadThreads), the phone footer's My
    threads (UnreadThreadsIndicator) and the channel rows; do not
    disturb hides the navbar's and the thread header's unread counts.
    The thread indicator's last replier is drawn without its link (the
    parser split it out of the indicator's link into stray anchors).
    Tests in tests/chat_threads_rest.rs and
    tests/browser/chat-threads.spec.js. Threads in the drawer stay with
    223c. Threads of deleted users' messages stay refused
    (plugins/chat/threads.rs thread_original_message,
    plugins/chat/create.rs, with 162), as do threads of deleted channels
    (threads.rs thread_channel). Left, as 160b.
    160b. ~~`feat/chat-threads-tracking-live`~~ (done) S: the threads lists
    follow the tracking state as ChatSubscriptionsManager keeps it. Each
    listed thread carries its tracking, its membership and its preview's
    last reply (data-tracking), the load more the viewer (id, ignored
    users, followed channels); static/js/chat-thread.js hears the
    channel's /new-messages (while followed) and
    /chat/user-tracking-state/:id beside the list's own bus.
    _onNewThreadMessage: another member's reply past the last read counts
    as unread (watched when watching) unless the membership is quiet,
    the viewer's own or an ignored user's moves the last read only, so the
    count shows before the preview's update_thread_original_message;
    _updateChannelTrackingData: a thread read elsewhere (or any thread
    tracking published) sets its counts. ChatThreadListItem's classes and
    ThreadUnreadIndicator, and UserThreads' ThreadTitle indicator, drawn
    again in place; ChatThreadList#sortedThreads (watched unread, then
    unread, then the rest, each by the preview's last reply, over the
    loaded threads in the threads manager's id order) on the first draw,
    each page and each change; a thread the channel's list doesn't hold
    comes in as the server lists it. The sidebar's indicators already
    followed the tracking state (/live's chat chrome drawn again on
    /chat/user-tracking-state); now covered by a spec. DModal's
    <:headerPrimaryAction> in static/js/modal.js (the mobile view's
    --has-primary-action header, its Cancel in the close button's place),
    ChatModalThreadSettings' save button there ("cancel" joins the
    cooked strings). The thread pane's date separator click
    (ChatThread#fetchMessagesByDate): the day's first message highlighted
    in the center; a thread's messages are never firstOfResults, so its
    target_date page never loads. ChatHistory is the page's state again
    (empty on a load, kept across page.js's swaps), the threads list's
    item opening its thread as a boosted navigation (the router's
    transition) instead of a load. Tests in tests/chat_threads_rest.rs
    and tests/browser/chat-threads.spec.js. Left, as 160b1.
    160b1. ~~`feat/chat-threads-navbar-live`~~ (done) S: one ChatHistory
    for the drawer and full-page chat, as Ember's service is one:
    static/js/chat-thread.js keeps it (Discourse.chatHistory) and
    chat-drawer.js visits it from the drawer's stateFor (the previous
    route read once the drawer's scripts are loaded), so a thread opened
    in the drawer after full-page My Threads backs to them, and a
    full-page thread after the drawer's threads list backs to the list.
    Its routes as router.recognize gives them: the leaf's name with its
    own params only (a channel's index has none, so two channels in a
    row are one visit), the drawer's an index route's parent
    (#forceParentRouteForIndex), the info routes as chat.channel.info.*
    (they were chat.channel-info.*); visits come from full-page chat only
    (a drawer url in the address bar is not the page's route).
    UserThreads#trackChannel for the pages loaded later: a page's new
    channels heard on /chat/:id (the stream opened again from its last
    event), the threads' tracking over every followed channel's new
    messages as ChatSubscriptionsManager hears them (the first page's
    channels only before). Tests in tests/browser/chat-threads.spec.js
    and tests/browser/chat-drawer-routes.spec.js. Left, as 160b2.
    160b2. ~~`feat/chat-threads-navbar-indicators`~~ (done) S: the
    navbar's thread indicators were drawn once by the server. Now they
    follow the tracking state (ChatThreadsListButton's has-unreads and
    ThreadHeaderUnreadIndicator over threadsManager.unreadThreadCount,
    and ChatThreadHeader#backLink's pick by it): the server draws on
    .c-navbar__threads-list-button and the thread's
    .c-navbar__back-button (thread_view.rs' unread_threads_tracking) the
    channel id, its unread threads' ids (unreadThreadOverview's keys),
    the viewer's id, ignored users and followed channels, do not
    disturb, and js.chat.unread_threads_count's one and other forms
    (the indicator's title). chat-thread.js' setupNavbarTracking keeps
    the ids from /chat/user-tracking-state/:id (the channel's
    unread_thread_overview, _updateChannelTrackingData) and
    /chat/:id/new-messages while followed (a reply in a thread the
    viewer is a member of, threadsManager.find through GET
    /chat/api/channels/:id/threads/:id, marks it unread unless it is
    their own or an ignored user's; a thread type's past the last read
    and not quiet, their own reads it), and draws the button's class and
    count, or the back link (the threads with their count while some
    are unread, else the channel, without a previous chat route). Tests:
    browser chat-threads "the navbar's thread indicators follow the
    tracking state"; tests/chat_threads.rs (the button's attributes)
161. `feat/chat-uploads` M: ~~chat message uploads~~ (done,
    feat/chat-uploads: Chat::CreateMessage and Chat::UpdateMessage take
    upload_ids, the sender's own uploads (user_uploads) while
    chat_allow_uploads, an upload-only message passing the contract and
    the minimum length and never a duplicate; an edit replaces the
    references only when every id given is the editor's (those gone
    deleted, those added made, the rest kept); upload_filename_excerpt as
    the excerpt of a message with a blank cooked, excerpt_for_display and
    cooked_for_excerpt (the summary email) of an upload-only one;
    Chat::MessageSerializer's uploads through UploadSerializer, in the
    messages list, the bus payloads and the last message;
    MessageProcessor#process_thumbnails in Jobs::Chat::ProcessMessage
    (Rails' early return at the first small image kept); the channel
    page's ChatMessageCollapser (plugins/chat/collapser.rs: the uploads
    behind their name or "N files", ChatUpload's image, video, audio and
    attachment, and the cooked's images, image oneboxes and galleries
    behind their links) with the Collapser toggle and the uploads'
    lightbox in chat.js; ChatComposerUploads: the draft's uploads
    restored in the composer and saved with it, the dropdown's "Attach a
    file" (chat-upload-btn), the picker, a paste or a drop on the channel
    sent to /uploads.json (upload_type chat-composer) with each upload's
    progress, removing one, sending (upload_ids[]) and editing with
    them, the staged message drawing them; tests/chat_uploads.rs and the
    collapser's unit tests. Left refused: ~~an upload's optimized video
    (plugins/chat/messages.rs:370; plugins/chat/create.rs:930)~~ (done,
    feat/chat-media-and-search), a nil
    message with upload_ids, a 500 in Rails (create.rs:289; modify.rs:345),
    lazy videos in the collapser (collapser.rs:309, they only come from
    oneboxes)), ~~oneboxes, images and videos in messages: the rest of
    Chat::MessageProcessor (oneboxes, hotlinked images and
    Jobs::Chat::PullHotlinkedImages, lightboxes on cooked images, video
    placeholders; plugins/chat/create.rs:816)~~ (done,
    feat/chat-media-and-search: plugins/chat/processor.rs ports
    Chat::MessageProcessor over crate::onebox::post_process (block and
    inline oneboxes, invalidate_oneboxes), the tracked hotlinked images
    localized and the stale rows swept, process_thumbnails,
    post_process_videos with the optimized videos' references, and
    add_lightbox_to_images; the job's pull-job enqueue and
    skip_notifications as Rails nests it; Jobs::Chat::PullHotlinkedImages
    (downloading refused as for posts); the uploads' optimized_video in
    the serializer; chat.js lightboxing the cooked's marked images;
    tests/chat_processor.rs and tests/browser/chat-media-search.spec.js.
    Left: lazy videos in the collapser (discourse-lazy-videos' LazyVideo
    and LazyIframe client, unported for posts too), chat's own local
    oneboxes of channels and messages (Chat::OneboxHandler, refused in
    onebox/mod.rs), HotlinkedMedia.download),
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
    (plugins/chat/bookmarks.rs:176,183, with 162)), ~~chat transcripts in
    posts~~ (done, feat/chat-uploads: chat-transcript.js's `[chat]` rule
    in crates/markdown (render/chat_transcript.rs) while chat_enabled, in
    posts and chat messages alike: the quote's user, message and time,
    the avatar (lookupAvatar), the message link or the noLink span, the
    channel link or the multiQuote meta line, chained, reactions with
    their emoji, a thread's details summary, the messages cooked with
    chat's rules (customMarkdownCookFn) and its allow list, the tokens
    rendered as renderToken writes them; chat's
    post_chat_transcript_spec cases in tests/pretty_text.rs. Left
    refused: a hashtag inside a transcript outside chat, which the chat
    composer's context would order differently (render/mod.rs:241), a
    multiQuote without a channel id and a reaction without users, which
    Rails writes as "null" or throws on (render/chat_transcript.rs:216,338))
162. `feat/chat-rest` M: messages of deleted users (the notification
    jobs done, feat/chat-rest: NotifyMentioned and NotifyWatching run for
    a deleted author's message and fail, as Rails does on @creator, only
    once a mention or a watcher is to be notified; still refused where
    Rails fails on the nil author too: processing one
    (plugins/chat/create.rs:725,767), editing one (DuplicateMessageValidator's
    user.bot?, plugins/chat/modify.rs:336,341), its summary email line
    (summary_email.rs:459), its reactions' users (messages.rs:955), and a
    reply to one, whose thread fails its original_message_user
    (create.rs:297)), ~~webhook messages and blocks~~ (done,
    feat/chat-rest: Chat::WebhookEventSerializer on messages and the
    messages replied to, Chat::BlockSerializer with its button, text and
    category elements; the channel page draws a webhook's ChatEmojiAvatar,
    its name and the bot indicator, the reply line's emoji avatar, and
    ChatMessageBlocks (actions and informative blocks, buttons and
    categories), chat.js sending a button as POST
    .../messages/:id/interactions (218's endpoint); refused: an event
    whose webhook is gone and blocks or elements Rails or the client
    throws on, messages.rs:139,666; page.rs:1543,1578), ~~bot changes~~
    (done, feat/chat-rest: a bot edits, trashes, restores and reacts as
    any user, joining the channel before an edit, enforce_membership),
    ~~archived channels~~ (done, feat/chat-rest, reads only: the channel
    serializer's archive_failed, archive_completed, archived_messages,
    total_messages and archive_topic_id for staff, and
    ChatChannelArchiveStatus for an admin above an archived channel's
    messages, its retry button posting to 216's archives endpoint) and
    channels of deleted categories or other chatables (left: a category
    channel outlives its category only trashed, which these reads hide,
    and core chat has no other chatable; refused where met,
    plugins/chat/channels.rs:563,740,745,776; membership.rs:70;
    publisher.rs:34; bookmarks.rs:176,183), anonymous access
    (plugins/chat/mod.rs:134; publisher.rs:66; channels.rs:461),
    ~~slug changes~~ (done, feat/chat-rest: UpdateChannel's slug, nil when
    blank so a category channel takes its title's (generate_auto_slug,
    blank when taken), ensure_slug_ok encoding a changed one, and
    enqueue_channel_hashtag_remap's remap_hashtag job unless only the case
    changed; an invalid or taken slug is the record_invalid refusal 216
    replaces, dm_groups.rs), the separate sidebar (view.rs:295) and
    empty states (the mobile channels list's done, feat/chat-rest:
    DEmptyState offering the browser; left: the filtered lists' empty
    states, whose show all bypass belongs with 216's filter toggle,
    view.rs:362,676,777,917), the mobile footer's search route (with 220's
    search) and ~~starred route~~ (done, feat/chat-rest: GET
    /chat/starred-channels, ChatStarredChannelsRoute: to /chat on desktop,
    to the channels without a starred one, else the navbar,
    ChannelsListStarred without its divider and the footer; the drawer's
    starred route shares the list, and an unfiltered empty one says
    starred_channels_empty), chat's notification sounds (the preferences'
    preview done, feat/chat-rest: chat-audio-manager's sounds synthesized
    with Web Audio in static/js/preferences.js, played on choosing one and
    by the preview button; ~~playing them on a mention's or a message's
    alert~~ (done in `feat/chat-notify-client-rest`, 225b); left:
    ChatNewMessageAnnouncer's for an active reader, 225b). Tests:
    tests/chat_rest.rs
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
    viewer isn't in (routes/users.rs:1045), the ~~dismiss and~~ bulk
    select buttons (33b; the user topic lists' too, 193c; the dismiss
    buttons done in `feat/tag-unread-and-pm-dismiss`, 270b)
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
    topic_converter.rs:236; a group's message inbox in ~~pm-reset-new
    and~~ bulk archiving, ~~filter=unread over a message inbox~~ (both
    done in `feat/tag-unread-and-pm-dismiss`, 270b), bulk deleting
    one's own topics or as a non-staff user, the nested view toggle
    while nested replies are on, routes/topics_bulk.rs:188;
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
    refreshes the staff groups (the modals are 291, 292 and 294). Left:
    ~~the avatar picker while signing up, which says it is not ported~~
    (done in `feat/account-leftovers`: AvatarSelectorModal with
    deferSave from static/js/avatar-selector.js, which the preferences
    share, the upload or selectable avatar saved once the account
    exists); ~~the username step's availability check with its avatar
    preview~~ (done in `feat/signup-username-check`: CodeLoginForm's
    checkUsernameAvailability in static/js/invite.js, the debounced
    GET /u/check_username with the email, its errors or
    code_login.username_unavailable and username_taken, the answer's
    avatar_template as the preview, Continue disabled until the name
    is available, the username instructions, and the username
    suggested from the email on the verify answer); the username
    step's random usernames (enable_random_usernames: the dice, GET
    /u/random-username, generated usernames) and can_edit_avatar;
    invites past the first page (DLoadMore)
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
    topic lists' bulk select button is drawn without its actions (183;
    the discovery lists' works, `feat/topic-list-bulk-select`)
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
    with a passkey (155), ~~the ForgotPassword modal a reset code opens
    (196)~~ (done in `feat/account-leftovers`), the MCP authorizations
    list
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
    194c. ~~`feat/topic-admin-actions`~~ (done) M: the topic admin menu's
    actions. Pinning until a time (Topic#update_pinned: the time parsed
    as Time.parse reads the pin form's, InvalidParameters for what has
    none, the unpin job scheduled and cancelled; Jobs::UnpinTopic), for
    PUT /t/:id/status and the bulk pin. The FeatureTopic modal
    (static/js/topic-admin-modals.js): GET /topics/feature_stats, pinning
    in the category and globally through PinOptionSection's form (the
    date required and to come, a confirmation past four global pins),
    unpinning, and banners (PUT make-banner and remove-banner,
    src/topics/banner.rs, with Jobs::RemoveBanner and
    Topic.ensure_consistency!'s expired banners, no longer refused).
    Recovering a topic (PUT /t/:id/recover, PostDestroyer#recover for a
    first post: staff_recovered, Topic#recover! with the category's and
    its tags' counts, both topic_recovered events, the user actions and
    the staff log). ChangeTimestamp (PUT /t/:id/change-timestamp,
    src/topics/timestamp.rs: TopicTimestampChanger with each post saved
    as validated, Ruby's float arithmetic for the moved times, the log),
    reset bump date (PUT /t/:id/reset-bump-date(/:post_id)), EditSlowMode
    and ConvertToPublicTopic. penalize-user.js's DFutureDateInput and
    ComboBox are shared as Discourse.futureDateInput (custom shortcuts,
    label classes), its config drawn for whoever has the menu. Recorded
    on rs-ref-a (parity/writes/topic_admin, 28 cases); browser specs in
    tests/browser/topic-admin-actions.spec.js. The rest is 194c1 and
    194c2
    194c1. `feat/topic-admin-actions-rest` M: what the topic admin menu
    still refuses: ~~the EditTopicTimer modal~~ (done,
    `feat/edit-topic-timer-modal`: all its types, the pickers from
    bookmark-modal.js, the preview; browser topic-timer-modal (4). TopicTimerInfo
    under the posts is `feat/topic-timer-info`, rendered by the server
    with viewer_dates; browser topic-timer-info (3). SlowModeInfo beside
    it is `feat/slow-mode-info` (viewer_dates::humanize_seconds, the
    button for who may manage topics; browser slow-mode-info (2)). Left
    of it: the keyboard shortcuts pause, the notices' live redraw),
    selecting posts (the mode, its
    controls, SelectedPosts, change owner and delete with all selected
    are `feat/select-posts`, static/js/select-posts.js; DELETE
    /posts/destroy_many, PUT /posts/merge_posts and GET
    /posts/:id/reply-ids are ported, `feat/posts-bulk-endpoints`, and
    wired to delete selected, merge selected posts and select +replies,
    `feat/select-posts-actions`; MoveToTopic's modal with ChooseTopic
    and ChooseMessage is `feat/move-to-topic-modal`: new topic,
    existing topic, new and existing message, merge-topic with every
    post selected; browser move-to-topic (8). Left of it: Shift+B,
    TagChooser (the composer's MiniTagChooser stands in, existing tags
    only), TopicStatus and emoji in ChooseTopic's rows, the modal's
    plugin outlets and value transformers) and the nested replies
    toggle (PUT
    /n/:slug/:id/toggle)
    194c2. ~~`feat/site-banner`~~ (done) S: the banner topic drawn where
    Ember draws DiscourseBanner: atop the discovery layout (the lists and
    the categories page), the tags index and the topic (in a container of
    its own, only while the first post is loaded). The banner is
    ApplicationLayoutPreloader#banner_json (src/topics/banner.rs: the
    first banner topic's Topic#banner, nothing for a read restricted
    category or a visitor while login is required; a translated banner
    refused), in the chrome (Chrome::site_banner) and at GET /site/banner.
    static/js/site-banner.js draws it with the staff edit link and the
    close button: dismissing it PUTs the member's dismissed_banner_key
    (User#dismissBanner), a visitor's goes to localStorage
    (discourse_dismissed_banner_key). make_banner! and remove_banner!
    publish `/site/banner` (the banner, or null), which /live sends as a
    `banner` event to everyone; the categories page, the tags index and
    every list now keep a stream for a visitor too. record_writes.rb
    clears the preloader's banner cache per case. Recorded on rs-ref-a
    (parity/writes/site_banner, 10 cases); browser specs in
    tests/browser/topic-admin-actions.spec.js (drawn, dismissed by a
    member and a visitor, live)
    194d. ~~`feat/post-admin-actions`~~ (done) M: the post admin menu's
    actions (static/js/topic-admin-menu.js, on the post its template's
    data-post describes, the post drawn again after through GET
    /live/post/:id): the staff color (PUT /posts/:id/post_type: staff,
    PostRevisor.valid_post_type?, no whisper for a first post,
    invalid_whisper_access) and wiki (PUT /posts/:id/wiki, can_wiki?),
    both through PostRevisor, which now takes wiki and post_type (the wiki
    param always revises, as its string never equals the column; the new
    version and its revision, a change undone within the grace period
    giving the version back and advancing the draft sequence of who
    edited before, the wiki first post's bump and its watchers'
    notify_post_revision; a moderator action or whisper may have its type
    or wiki changed, and the revisions page shows wiki_changes and
    post_type_changes). Unhiding (PUT /posts/:id/unhide, Post#unhide! in
    src/posts/admin.rs: a first post hidden by its flags lists its topic
    again; the review queue's approve_and_unhide uses it too, first posts
    included), locking (PUT /posts/:id/locked, PostLocker and its staff
    log), the post notice (ChangePostNoticeModal, PUT /posts/:id/notice:
    the notice cooked without oneboxes, save_custom_fields with
    ActiveSupport's JSON, log_post_staff_note; the topic view serializes
    custom notices and, for staff, notice_created_by_user, drawn with who
    added it, no longer refused) and rebuilding the HTML (PUT
    /posts/:id/rebake, Post#rebake! with invalidate_broken_images and
    invalidate_oneboxes, which leaves the :rebaked message to the post
    processor). Recorded on rs-ref-a (parity/writes/post_admin, 38
    cases); browser specs in tests/browser/post-admin-actions.spec.js. The
    rest is 194d1
    194d1. `feat/post-admin-modals` M: what the post admin menu still
    refuses: the ChangePostOwner modal (POST /t/:id/change-owner is
    ported), the GrantBadgeModal, permanent deletion (force_destroy) and
    the PublishPageModal (the published pages endpoints)
    194e. ~~`feat/review-client-actions`~~ (done) M: the review queue's
    client actions: suspend and silence open the penalty modal
    (penalize-user.js, now on /review) for the author with the post and
    its raw, performing the action as its `before` (a flag's
    agree_and_silence and agree_and_suspend, now performed as
    agree_and_keep when built; a queued or reviewed post's
    reject_and_silence and reject_and_suspend), the automatic claim
    released once the modal closes; edit (a flag's agree_and_edit)
    opens the post in the composer when it may be edited (PostsController
    #show, GET /posts/:id, now ported beside by_number), staying on the
    page (skipJumpOnSave) and performing once saved (onSaved).
    Editing a reviewable: ReviewablesController#update (PUT
    /review/:id, src/review/edit.rs) with editable_for, the params
    checked field by field, update_fields (the payload merged, the
    category cast, the version, the edited history with its changes,
    the raw's upload references, notify_reviewable), JSON bodies
    wrapped under `reviewable` as ParamsWrapper does, and the answer
    with the cooked raw and escaped title; review.js draws the editable
    fields in place (the title's input, the category in the composer's
    CategoryChooser, the raw in a DEditor with the toolbar and preview)
    with Save and Cancel. A queued post by email links to RawEmailModal
    for those in view_raw_email_allowed_groups. Recorded on rs-ref-a
    (parity/writes/review: update_*, review_agree_and_edit*,
    review_agree_and_silence*, review_agree_and_suspend*;
    posts_by_number/show*); browser specs in
    tests/browser/review-client-actions.spec.js. The rest is 194e1 and
    194e2
    194e1. ~~`feat/review-edit-tags`~~ (done) S: a queued topic's tags
    field (ReviewableFieldTags): the composer's MiniTagChooser made
    reusable (Discourse.miniTagChooser in static/js/composer.js, the
    composer's own tags drawn through it) and drawn in place over a
    copy of the composer's chooser, with the categoryId option
    (tagCategoryId: the category being picked, else the reviewable's),
    no minimum and no recent tags first; the tags it changes to are
    sent as MiniTagChooser#_onChange's items ({id, name}, isNew for a
    new tag, the one just chosen as /tags/filter/search returned it),
    and the update goes as JSON as ReviewableAdapter's jsonMode sends
    it (it went form encoded, which loses an empty list and the tag
    objects' shape). Without the composer's chooser (a viewer who may
    not tag topics) the tags are drawn and changing them still says
    so. Browser spec in tests/browser/review-client-actions.spec.js
    (add, remove, saved and shown after a reload)
    194e2. `feat/review-reject-reason` S: the reject reason modal
    (RejectReasonReviewableModal), refused (review.js): only a user
    reviewable's reject opens it, so it waits on user reviewables in
    the queue (148)
    194f. `feat/review-pages-rest` M: the /review/topics, /review/settings
    and /review/:id pages (the user menu's review items link to the
    last), the insights tab (IpLookup), the pin toggle and loading past
    the first ten
    194g. ~~`feat/header-avatar-badges`~~ (done) S: the header avatar's
    badge as Header::UserDropdown::Notifications draws it
    (src/header_badge.rs): the do not disturb moon, else the first of
    new message notifications (envelope, new-pms), new reviewables (flag,
    new-reviewables, staff's unseen_reviewable_count) and the unread
    notifications count, with Ember's titles, labels and classes. Drawn
    with the page and redrawn over the live stream from the member's
    /notification (all_unread_notifications_count,
    new_personal_messages_notifications_count), /reviewable_counts and
    /do-not-disturb messages, the toggle button's contents swapped. Jobs
    NotifyReviewable (admins, then moderators for what they may review,
    with the `updates` of the reviewables an action resolved) and
    RefreshUsersReviewableCounts are ported, so a flag made or handled
    reaches staff's badges and the sidebar's review count, now kept
    current live too. Compared on rs-ref-d as admin and moderator: a
    flag shows the badge live, agreeing clears it, a new message shows
    its badge. Still refused: category group moderators' counts
    (enable_category_group_moderation). ~~Not drawn: the moon going away
    on its own when do not disturb ends (the notifications service's
    timer; a reload or the next message redraws it)~~ (done in
    `feat/do-not-disturb-modal`: the live stream redraws the badge as
    the timing ends). Not drawn: the first
    notification user tip, and the perform result's fast-tracked counts
    (the job's message follows within the worker's poll)
    194h. `feat/user-menu-avatars` S: show_user_menu_avatars, the user
    menu items' avatars (routes/user_menu.rs:51, refused; 193b's
    notifications page likewise)
195. `feat/rss` M: the .rss feeds of the lists, categories, tags, topics,
    a user's posts and activity, topic voting's votes feed, and the
    bookmarks .ics calendar (session/user_api_key.rs:117; 84b). Only the
    badge feed exists (85b)
196. `feat/client-rest` L: the client's leftovers named under done
    items: the rich editor (ProseMirror, split into 196r1 to 196r6 below)
    and its toggle, the composer's
    actions menu (196a) and toolbar menus (196r3), the insert-date
    modal (49, 96), the desktop header's search menu (94; the advanced
    filters panel was 143i), ~~the do not disturb and user status modals
    (95)~~ (done, 130b and `feat/do-not-disturb-modal`),
    the drafts and post menus as sheets, the jump to post prompt (98),
    the topic map's menus, the footer's flag, mark unread and pinned
    buttons, the sidebar's header actions and Customize (milestone 4.5),
    the login, signup and logout dialogs (79c), the Edit Directory
    Columns modal (74), ~~BookmarkModal (editing a bookmark from the
    bookmarks tab, 193c)~~ (done, 289), ~~the ForgotPassword modal a
    password reset code opens (from the security tab, 193e)~~ (done in
    `feat/account-leftovers`: login.js's modal at its code step, from
    the change password button and ConfirmSession), discourse-poll's
    rich editor node view (PollNodeView and the builder's Edit poll mode
    over it, from 229). Split when started
    196a. ~~`feat/composer-actions-menu`~~ (done): the composer's actions
    menu switches between replies, linked topics and private messages,
    with whisper, do-not-bump and unlisted toggles and post voting topics.
    Posts#create handles those flags with the viewer's permissions and
    keeps whispers out of public topic counts and bumps. The toolbar's
    poll builder inserts regular, multiple, number and ranked-choice
    polls. Tests: composer_actions.rs and browser/composer-actions.spec.js.
    Rich text and the remaining client work stay in 196; the poll
    builder's follow-ups were 229.
    The rich editor (docs/rich-editor.md), in six steps. The owner's
    decisions: markdown stays the default composition mode until the
    round-trip tests pass, and the rich editor is reached through the
    member's composition_mode and the composer's toggle; the editor parses
    with the port's markdown crate (its WebAssembly build) through a node
    tree export, never with JS markdown-it; ProseMirror is vendored ES
    modules behind an import map, with no bundler, build step or CDN.
    196r1. ~~`feat/rich-editor-core`~~ (done) L: the core: vendoring, the
    import map, the parser over the markdown crate's node tree, the
    serializer, the toggle on composition_mode, the round-trip tests.
    Phase A (no Rust): every prosemirror-* package the editor imports and
    their dependencies at the reference's pnpm-lock.yaml versions, checked
    against its sha512 integrity (static/vendor/prosemirror, the README);
    the loader and its import map (static/js/rich-editor.js, markdown-it
    mapped to a stub since prosemirror-markdown imports it at load); the
    schema, serializer, keymap, input rules, plugin utils, core extensions,
    to-markdown and the editor itself (static/js/rich-editor/); the parser's
    mapping layer from the node tree to markdown-it's tokens and the tree's
    contract (docs/rich-editor.md); the core CSS and the toggle switch's;
    tests/browser/rich-editor.spec.js with the reference's
    prosemirror-markdown, strikethrough and underline cases. Phase B: the
    crate's node tree (render::rich_editor_tree on render::parse, which
    render() now shares; render/tree.rs rebuilds tight lists' hidden
    paragraphs, the inline tokens and the JS tokens Elements stand for,
    recorded by their rules in element::JsToken; typographer, onebox,
    watched words, censored and the upload pass left out) and
    markdown.wasm's rich_editor_tree export, with Rust tests over the
    spec's cases; assets::rich_editor_urls for the loader's data-assets;
    the loader's tag in the head of a member's pages before lightbox.js;
    the toggle (ComposerToggleSwitch), data-composition-mode and the labels
    in the composer; composer.js switching editors by the toggle, Ctrl+M
    or composition mode 1, keeping the textarea as the markdown's holder
    (setValue wherever it writes it), saving composition_mode after 1s and
    going back to markdown with composer.unsupported_token on a refusal.
    Round trips: all 18 of the spec's round-trip cases pass through
    markdown.wasm (rich-editor.spec.js 66 of 66); composer.spec.js (with
    four rich editor cases: switching and Ctrl+M, opening in mode 1 and
    keeping a saved switch, the refusal, posting from rich mode) and
    composer-actions.spec.js pass. Opening in the stored
    composition_mode is held behind composer::OPEN_IN_STORED_MODE, off
    (a documented, temporary deviation: Discourse's default_composition_mode
    1 would put the seed's, imported and new members in the rich editor
    before its extensions); the toggle and Ctrl+M still switch and save
    196r2. ~~`feat/rich-editor-extensions`~~ (done) L: the everyday
    extensions, each from the reference's extensions/ as plain ES modules
    (static/js/rich-editor/extensions/, node-views/, table/) registered in
    register-default.ts's order: emoji (with the only-emoji decoration and
    the emoticon input rule), mention, hashtag, link (marks by markup,
    attachments, upload links, linkify as the member types or pastes over
    a selection) and the link toolbar (floating, or in the button bar's
    place without room), quote (its node view and title selection), code
    block (prosemirror-highlightjs over the page's highlight.js, the
    language select, Tab indenting, formatCode, pasted line wrappers), html
    inline (the engine's allow list, the outline, small text) and html
    block, trailing inline space, table (lib/table: grips, menus, append
    bars, editing, paste, repair), image (the media node view with its
    toolbar, alt text, scale, lightbox and grid moves, short url lookup,
    data: image uploads), video, upload placeholder, grid, override drag
    ghost, markdown paste and word paste (also in to-markdown). The node
    tree writes the JS tokens of mentions, raw hashtags, quotes and grids
    (JsToken) and applies upload-protocol's client half; the engine
    (rich-editor/engine.js over markdown.wasm, render/editor.rs) answers
    linkify, the allow list, emoji images, emoticons and emojiUnescape.
    The composer gives the editor its context (src/composer/rich_editor.rs:
    settings, strings, hashtag order; appEvents, replaceToolbar), puts
    uploads in as placeholder nodes in rich mode, and GET
    /composer/mentions (ComposerController#mentions) answers the mention
    extension. 196r1's parser gained the reference's closing of inline
    nodes at a block token, the keymap its Enter in a table cell, and
    float.js now takes a padding of 0 as given. Tests: browser
    rich-editor-extensions (107), rich-editor-table (98),
    rich-editor-media (28), rich-editor-to-markdown (41) and
    rich-editor-word-paste (23), the reference's QUnit bodies run as
    written where they are not table-driven; tree.rs and editor.rs;
    tests/composer_mentions.rs. Still refused then: 196r4's constructs
    and [wrap]. Left to 196r3: Edit link (UpsertHyperlink), Load preview
    (onebox), Tab nesting list items (text-manipulation). Not ported here:
    A11y's live regions (table and video announcements are skipped),
    quote avatars in the composer (as its preview). The mention warnings
    show through ComposerMessages since 213
    196r3. ~~`feat/rich-editor-toolbar`~~ (done) M: the toolbar and its
    menus wired to the rich editor. rich-editor/text-manipulation.js is
    text-manipulation.ts (createEditor's textManipulation, its state after
    each transaction); static/js/editor-toolbar.js is toolbar.ts's
    Toolbar, d-editor's newToolbarEvent and keymap and the textarea's
    toolbar half, over whichever editor is open, for the composer and the
    About me: bold, italic, the heading menu, link, blockquote, code,
    upload, the list menu (with checklist's item), emoji (the picker,
    emojiSelected), their active states, icons and shortcuts, Tab and
    Shift+Tab nesting list items; the server draws the menus' triggers
    (composer::view::editor_toolbar). UpsertHyperlink
    (static/js/upsert-hyperlink.js) for the link button, mod+K and the
    link toolbar's Edit. The options menu acts on the toolbar event:
    details, footnote, spoiler's command, graphviz, local dates, math,
    the poll and policy builders, toggle-wrap and the touch view's
    format-code, with active items and the shortcuts in the rich editor.
    The onebox extension and its toolbar (oneboxes.js answering promises)
    with the link toolbar's Load preview; the preview block, its toolbar
    and discourse-graphviz's extension over the crate's graphviz token
    (tree.rs). The serializer writes a selection's inline content with its
    marks and a bare link's following bracket unescaped. Tests: browser
    rich-editor-toolbar (the reference's text-manipulation, onebox,
    preview node view and graphviz QUnit tests, the plugins'
    text-manipulation tests, the system specs' toolbar, keymap, links,
    onebox, spoiler and wrap cases, composer-hyperlink-test), the
    extensions' "Tab still nests list items" and Edit; tree.rs. Not
    ported: Insert table and Change reply to in the options menu (still
    "not supported yet"; 196r6, since done), the About me's emoji button (its page has no
    picker template), spoiler's composer:apply-surround app event (no app
    events service)
    196r4. ~~`feat/rich-editor-plugins`~~ (done) L: the plugins'
    extensions, each from its plugin's lib/rich-editor-extension.js as
    plain ES modules (static/js/rich-editor/plugins/), registered by
    plugins.js for the plugins the composer's context lists as on
    (src/composer/rich_editor.rs `plugins`): checklist (its click, Enter,
    Backspace and list commands), details, spoiler (blurred until the
    selection is in it), footnotes (the note in an editor of its own),
    math (typeset by discourse-math.js, now with a forced render, and its
    Edit math modal, Discourse.mathEdit), local dates (chips drawn by
    local-dates.js's formatLocalDate, moment loaded before the editor, the
    insert date modal editing a date with initialValues), policy (its node
    view, the builder over a node's values: GET
    /live/policy-builder?edit=1) and poll (PollNodeView with PollInfo, the
    builder's Edit poll mode: show(config, composer, { poll, onSave }) with
    pollAttrsFromSettings' originalAttrs); and core's [wrap], its extension
    with insertWrap, updateWrap and removeWrap, the wrap node view and
    WrapAttributesModal (wrap-modal.js), in register-default.ts's order.
    The node tree writes the plugins' JS tokens (details' bbcode,
    wrap_bbcode with `bare` for spoiler, policy and excerpt, wrap, check,
    footnote_ref and footnote_tail's tokens with their meta, math with its
    mathType, local dates' span, the poll's tokens with `fields`
    poll_attrs; element::JsTree, JsWrap, JsFields, JsBare), the cooked HTML
    unchanged; plugin-utils.js has buildBBCodeAttrs, serializeBBCodeAttr
    and parseBBCodeTag. The composer's context gives the editor a modal
    service for the node views' modals, and the editor leaves nothing behind
    when the crate refuses the markdown. Tests: browser
    rich-editor-plugins (93: the plugins' rich-editor-extension tests, the
    [wrap] registry case and the system specs' typing), composer-actions'
    builder editing cases and composer.spec.js's plugins in the composer;
    tree.rs; tests/policy.rs. Still refused: discourse-events' [event] and
    the calendar's blocks (the crate refuses them, the plugins unported,
    202), [graphviz] (a preview node, 196r3's preview-source, since
    done) and chat's [chat] transcripts (chat's extension; 196r6, since
    done), both raw HTML in the tree; [excerpt] as in the reference (no extension). Not ported here:
    the checklist's and footnote's text-manipulation tests and the
    system specs' toolbar cases (196r3)
    196r5. ~~`feat/rich-editor-default`~~ (done) S: once 196r2 and 196r4
    land, turn composer::OPEN_IN_STORED_MODE (src/composer/load.rs) on and
    remove it, so the composer opens in the stored composition_mode as
    Ember does; the profile's About me editor in rich mode, then rich as
    the default composition mode
    (default_composition_mode 1, as Discourse ships) once the round-trip
    tests pass. The switch is gone (load.rs opens_rich: mode 1). The
    round-trip tests, markdown through rich_editor_tree and the editor to
    the markdown an edit writes: rich-editor.spec.js' round-trips cases
    and the testMarkdown and setupRichEditor cases of
    rich-editor-extensions, -plugins, -table and -media (all seven
    rich-editor specs 458 of 458 with to-markdown and word-paste);
    bbcode-inline's [b], [i], [code], [url], [email] and [img] had none,
    and their new round-trips case passes.
    [img] was refused: the crate wrote raw HTML where the reference
    pushes an image token. It is an image node now, cooked as before
    (pretty_text_spec.rb's and pretty-text-test.js' cases), and the
    editor writes ![](src) for it. The
    default needed nothing else: the vendored setting, the column and
    signup (option_defaults) already give 1, and the seed's members have
    it. The About me (preferences/profile.gjs' DEditor, no mode forced)
    opens in the viewer's mode with the toggle first in its bar, through
    composer.js' Discourse.composerRichEditor (the composer's engine,
    extensions and context, its own replaceToolbar slot, the mode's save,
    a closed composer following a switch); its textarea stays hidden
    behind it for Save Changes. What it refuses falls back to markdown
    with the alert in the composer, an edit and the About me, saving mode
    0 as toggleRichEditor does. Still refused: [event] (202), [graphviz]
    (196r3, since done), [chat] transcripts (chat's extension; 196r6, since
    done, less an archive's reactions), [excerpt] as in the reference. The browser suite's members stay on markdown
    (setup.sql), as its specs drive the textarea; demo-rehearsal switches
    a rich composer to markdown before typing. Tests: load.rs,
    preferences_pages (the bio's mode), sessions (the composer in mode 1),
    new_member_smoke (a signup's mode and composer), tree.rs (the
    transcript's raw HTML, [img]'s image token and its cook), pretty_text
    ([img] cooked as Rails); browser composer.spec.js (opening in mode 1,
    an edit falling back, the About me's mode, save, Ctrl+M and fallback),
    rich-editor.spec.js (the bbcode round trips), rich-editor-plugins
    ([chat] refused)
    196r6. ~~`feat/rich-editor-leftovers`~~ (done) M: what 196r3 to 196r5
    left. Insert table (`toggle-spreadsheet`) is ComposerService#insertTable
    as the reference has it now (the spreadsheet editor and jspreadsheet
    are gone there): a four-column table with composer.table.default_heading
    and an empty row put in as a block, markdown in the textarea and a
    table with its editing controls in the rich editor. Change reply to
    (`change-reply-to`) is ChangeReplyTo: the edit composer loads the post
    (can_edit, reply_to_post_number, reply_to_user), ComposerActionTitle's
    reply indicator is a button on the desktop for one who may edit it, and
    the options menu offers "Reply to another post" past the first post;
    the modal (static/js/change-reply-to.js) shows the topic's fullscreen
    timeline (GET /live/timeline/:topic_id, topic.js'
    Discourse.topicTimeline) and picks the post the scrubber lands on, an
    earlier one settable, a current target removable; the draft keeps a
    change and the edit sends it only when changed. posts#update takes
    `reply_to_post_number` and PostRevisor resolves it (an earlier post of
    the topic the editor can see, else "Reply to post number ..."), sets
    reply_to_user_id, records it in the revision and drops the old
    target's PostReply and reply count unless the post still quotes it;
    PostRevisionSerializer shows reply_to_post_number_changes (each side's
    post and author, null for none or one the viewer can't see). chat's
    rich editor extension (rich-editor/plugins/chat.js) is the chat node
    over chat-transcript.js' tokens, which the crate writes into the node
    tree one by one with their nesting (a thread's open and close across
    one another; parser.js takes a childless node's `nesting`), headed by
    group-chat-transcripts.js' buildTranscriptHeader; the rule now writes
    a transcript's `source` and `sourceType` as the reference's does. A
    post with a transcript opens rich and round-trips byte for byte.
    Strings the vendored locale predates (composer.table.default_heading,
    chat.quote.transcript_label, composer.link_toolbar.preview_limit_reached)
    are rich_editor.rs' NEWER_STRINGS until a re-vendor. Still refused: an
    archive's transcript with reactions (its bare tokens, as the reference
    refuses them), [event] (202), [excerpt] as in the reference. Not
    ported here: the nested view's reply stats a reparent moves (145),
    posts#revert and the history modal's reply target rows (neither is
    ported), group-chat-transcripts.js on cooked posts (chat.css predates
    the grouped transcript). Tests: browser composer-insert-table
    (composer-insert-table-test.js and composer_insert_table_spec.rb),
    composer-reply-to (composer-reply-to-test.js and the picker's effect
    on the reply), rich-editor-chat (the plugin's
    rich-editor-extension-test.gjs as written and
    rich_editor_extension_transcript_spec.rb), 100 of 100 with
    --repeat-each=5; rich-editor-plugins' and composer.spec.js' refusals
    now a transcript with reactions; tests/post_reply_to.rs
    (post_revisor_spec.rb's "revising reply_to_post_number" less its
    nested stats cases, posts_controller_spec.rb's and
    post_revision_serializer_spec.rb's reply target cases); tree.rs
197. `feat/filter-route` M: /filter (ListController#filter, not routed)
    with topic voting's votes filters and discourse-solved's
    status:solved and status:unsolved; the topic_ids and state list
    filters (routes/list.rs:54; topic_query.rs:368)
198. ~~`feat/posts-show`~~ (done) S: GET /posts/:id.json (PostsController#show),
    ported with 194e for the review queue's edit action; the `not_yet` case
    solved/post_as_admin matches and lost its mark (5716e3a3)
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
    homepage (and custom_homepage_crawler_route) and `?format=` other
    than json or html (Rails 406s rss; markdown not ported);
    finish_installation while has_login_hint is served since
    feat/setup-command
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
    266a. `feat/docked-header-topic-rest` S: ~~a member's pin toggle in
    the statuses (TopicStatus' pin-toggle-button links and
    togglePinnedForUser's PUT /t/:id/clear-pin and re-pin) is drawn as a
    span here as on the title and lists
    (topic_list_view.rs:1299,1305)~~ (done in `feat/topic-pin-toggle`,
    see 264a; the docked copy takes the title's state as it docks); ~~a
    group participant's card is refused
    (static/js/header-topic-info.js:225)~~ (done in `feat/group-card`:
    GroupCardContents on the user card's script (static/js/user-card.js,
    Discourse.showGroupCard), opened by a group mention in a post
    (a.mention-group), a `[data-group-card]` name and the docked
    header's group, drawn by GET /group-card/:name from the group's
    document and its first ten members: flair, names, bio excerpt,
    member avatars and +N, GroupMembershipButton's Join and Leave in
    place and Message to the composer (composer.open's hasGroups);
    left: Request goes to the group's page, since
    RequestGroupMembershipForm is ported nowhere (the group page's
    Request and Message buttons do nothing, src/groups/view.rs:94,580),
    Leave's confirmation is the browser's where dialog.js is not loaded,
    and the docked-card class and the composer preview's mentions
    (d-editor:preview-click-group-card) are not ported); the small
    logo's dark variant
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
    262a. `feat/pm-topic-view-rest` S: ~~the browse more line is drawn once with the
    page's own message read through the posts shown, not kept live from
    the pmTopicTrackingState bus messages~~ (done in
    `feat/pm-browse-more-live`: on a message's page GET /live hears the
    member's own and their groups' with messages
    /private-message-topic-tracking-state channels, as
    pmTopicTrackingState#startTracking does, and after a message that
    changes its states (a new message not their own, a reply, a read;
    not a group's archiving) counts the line again from their report and
    swaps it in as a `more-topics` event, the reader's own reading coming
    back as the read their timings publish. Counted from the database's
    report each time, not from states merged in the page as Ember keeps
    them; a dismissal on a message list shows when the page is next
    drawn, as it publishes nothing); ~~the title's edit pencil and
    title editing (TopicTitleEditor, the edit-title glyph's styles) are
    not drawn for any topic~~ (done in `feat/topic-title-editor`: for
    details.can_edit the pencil in the title's link and
    static/js/topic-title-editor.js, the title's click swapping it for
    #edit-title, the composer's category and tag choosers copied and the
    save and cancel buttons; Escape cancels, Ctrl+Enter saves, the
    buffer's changes go as Topic.update sends them (PUT /t/:slug/:id,
    JSON, keep_existing_draft), a refusal is popupAjaxError's dialog; the
    title, category, tags, body classes, document title, address slug
    and docked header are drawn again, and a live redraw of the title
    waits for the open editor. Left: the edit-topic plugin outlets, the
    featured link's removal, localization editing, MiniTagChooser's
    useHeaderFilter, the print stylesheet's rule) (the admin menu's
    wrench is 194a); the header's topic title once scrolled past, with
    its glyph, is 266's
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
    264a. `feat/categories-tracking-rest` S: ~~TopicStatus' pin toggle
    for a member (`<a class="pin-toggle-button">` with clearPin and
    rePin, PUT /t/:id/clear-pin and /re-pin, unported) is drawn as the
    visitor's span on every list (topic_list_view.rs:1264)~~ (done in
    `feat/topic-pin-toggle`: TopicsController#clear_pin and #re_pin
    (routes/topic_pin.rs) through TopicUser.change's cleared_pinned_at;
    topic_statuses draws canAct's link on the lists, the categories
    page's latest column, the topic's title and the docked header, and
    the span under disableActions (MobileCategoryTopic and the post
    list no longer link the bookmark either); static/js/topic-status.js
    is togglePinnedForUser, the status, the row's `pinned` class and
    its pinned excerpt redrawn in place, the order kept until the list
    loads again, pinned again when the request fails. Left: the
    reviewable's topic link draws the toggle but names no topic id to
    send; a pinned Uncategorized topic's badge comes back only on a
    reload; the last-visit line stays where it was); the category
    badges don't follow live tracking updates, which refresh
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
    265a. `feat/new-list-subset-toggle` S: ~~/new's All / Topics /
    Replies toggle (NewListHeaderControlsWrapper, with unified new, on
    for everyone) with the member's new and unread counts within the
    category or tag, kept live~~ (done in `feat/new-list-subset-toggle`:
    html::new_list_controls above the topics of /new, /c/:slug/:id/l/new
    and a tag's new list, tags#show_new, which was refused and is now
    served to members, its dismiss buttons sending Topic.resetNew's
    tag_name; All never counts, Topics and Replies only past zero
    (filters.new.all_with_count has no caller upstream); a button
    replaces the URL's `subset`, the other discovery query params kept
    in Ember's order (discovery-topic-list.js); the page's stream
    recounts the two labels in the list's category and tag, the
    `new_list` live param, routes/live.rs list_state), and its bulk
    select variant in the list header (templates/latest.html:40):
    upstream has none any more, DiscoveryTopics only hides the toggle
    while bulk select is on (renderNewListHeaderControls), done in
    `feat/topic-list-bulk-select`; EmptyTopicFilter's
    education in a member's empty list footer is done (270)
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
    269a. `feat/new-topic-route-rest` S: creating a shared draft
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
    268a. `feat/small-action-buttons-rest` S: ~~staff deleting a small
    action with replies get Ember's reply-ids dialog (delete replies or
    not, GET /posts/:id/reply-ids, not routed): refused~~ (done in
    `feat/delete-post-replies-dialog`: the topic controller's
    deletePost for staff and a post with replies, small action or
    not: topic.js asks reply-ids, then the dialog (direct replies,
    all of them when some are deeper, just the post, cancel) and
    Post.deleteMany, the posts on the page drawn again from the
    server once it answers rather than marked deleted before; a
    failed reply-ids is logged, a refused destroy_many (a flagged
    post, Unsupported) shown in the dialog; permanently deleting
    (force_destroy) stays 194d1's); ~~after an
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
    (static/js/composer.js:2390). Left from the save jump: ~~the topic's
    pending posts (TopicPendingPostSerializer, the stream's pending post
    with its Delete) are not drawn, on load or after a queued reply~~
    (done in `feat/topic-pending-posts`: the viewer's queued replies
    under the posts once the last one is in, before SlowModeInfo
    (post_view::pending_posts: awaiting approval, age, avatar, name, the
    raw cooked by the server where Ember's DCookText cooks it in the
    browser), then for staff has-pending-posts with queued_posts_count
    and the queue's link; Delete is deletePending, DELETE /review/:id
    with no confirmation, the author's alone; after PostEnqueuedModal
    the page of the topic replied to draws its pending posts again from
    the server (topic.js pendingPostAdded), where Ember pushes
    pending_post onto whichever topic is shown. Not done: plugin
    NewPostManager handlers, which in Rails turn queued_posts_enabled on
    for every topic with poll or post-voting installed, so the port
    draws pending posts only under a setting, watched word or category
    review mode that queues); a
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
    271a. `feat/list-query-filters-rest` S: ~~tag lists take only page,
    per_page, order, ascending, state, subset and the tags (TagsController
    merges the rest raw, blank or not, and match_all_tags=false makes an
    any tag list; routes/list.rs:113,118,123)~~ (done in
    `feat/tag-list-params`: the tag lists build their options as
    TagsController#build_topic_list_options does, on ListController's
    parser (build_options_for and next_url_for, routes/list.rs).
    TopicQueryParams' part is the same: every public option, present and
    valid, else the 400 naming it, so before, bumped_before, f, subset,
    group_name, exclude_tag, no_tags and the plugins' filter the tag
    lists too. Over it order, ascending, min_posts, max_posts, status,
    filter, state, search and q are the request's even when blank: a
    blank order is "default" (bumped_at, nothing pinned, no category
    sort), a blank filter hides `f`, the rest filter nothing, and all of
    them stay in the next-page URL (`status=`). match_all_tags=false
    stays, an any tag list. set_category's `category` param off the
    /tags/c routes: Category.query_category's top-level slug or id,
    blank or not, a 404 when none or unreadable, the list scoped to it,
    the HTML redirects under /tags/c/ and the next page carrying its
    slug_for_url; the page stays the tag's own. The next-page URL is
    TagsController's page_params (no username, groupname or period) with
    per_page clamped. A `tags` hash is a 400 on the id routes, where no
    tag_name makes it an array. Derived from the Rails source, no
    recording: tests in tests/tag_pages.rs and tests/html_pages.rs.
    Left: a `category` param that is not a string
    (routes/tags.rs:265), a `tag_name` query param on the id routes
    (TopicQueryParams puts it in front of `tags`), set_category's
    permalink lookup for the param's miss, and the intersection routes,
    273a's);
    (discourse-assign's `assigned` is done, feat/plugin-assign);
    discourse-events' sort by event start date for a category
    (topics/query.rs:541); `search` with search_ignore_accents
    (topics/query.rs:501); a before or bumped_before reaching past year 1
    (topics/query.rs:977); page_params' username or group name under
    unicode_usernames, or not a string, and plugin params given as hashes
    (routes/list.rs:487,493,501); list params Rack rejects, given as both
    a value and an array or hash, or arrays of hashes (Rails' 400 Bad
    Request; routes/list.rs:170,176,180), and tag_name that is not a
    string (routes/list.rs:196,203)
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
    270b. `feat/topic-list-load-more` S: (loading more on scroll is done
    in 283) ~~tag unread lists (new is 265a's), and so their dismiss
    buttons, are refused (src/routes/tags.rs:293); the messages lists'
    dismiss buttons are drawn without actions
    (src/messages_view.rs:353, 183)~~ (done in
    `feat/tag-unread-and-pm-dismiss`: tags#show_unread, and unseen,
    read, posted and bookmarks, the rest of Discourse.filters' loop,
    served to members under /tag and /tags/c; the unread list's
    Dismiss… sends tag_name, the tag's name where Ember now sends the
    tag model's numeric id, which the endpoint matches against no
    tag's name. The message lists' buttons are topic-dismiss.js' too,
    the row carrying the route's inbox and group: new is
    Topic.pmResetNew, unread DismissRead with private_message_inbox,
    stop tracking included. PUT /topics/pm-reset-new and /topics/bulk
    take TopicsController#private_message_filter's group inbox and
    filter_private_messages_unread through pm_lists' new_ids,
    unread_ids and ids_among, which also give the user and all
    inboxes the default scope's `visible` and group_with_messages_ids).
    Left: a tag within a category draws no navigation, so no top
    dismiss row (src/routes/tags.rs:578); a bulk operation's `group`
    (src/routes/topics_bulk.rs:188); the message lists' bulk select
    (183). Done in
    `feat/topic-list-bulk-select`: the discovery lists' bulk select
    (BulkSelectHelper, static/js/topic-list-bulk-select.js): the
    toggle for canBulkSelect (topic managers; members on new and
    unread lists with topics, category and tag ones too), the checkbox
    cells, row click and shift range selection, Select All and Clear
    All, rows loaded later, the subset toggle hidden in the mode, the
    dismiss buttons' selected variants ("Dismiss (n)", topic_ids), and
    for topic managers BulkSelectTopicsDropdown with BulkTopicActions
    over PUT /topics/bulk: dismiss, update category, update
    notifications, reset bump dates, mark unread, close (with its
    note), archive, un-pin, unlist, relist, make personal message and
    delete, a failure listed in the modal. Done in
    `feat/bulk-tags-and-pin`: Manage tags (ManageTagsForm: remove,
    remove all, add and replacement rows over one manage_tags
    operation sent as JSON, its TagChoosers copies of the composer's
    chooser through Discourse.miniTagChooser.copy, existing tags only,
    tag_category_errors listed) and Pin (BulkPinOptions over
    FeatureTopic's PinOptionSection, the date required, feature_stats'
    counts for the selection's sole category). Left, refused by name:
    the nested replies items (NestedTopic::Toggle); a replacement
    row's mobilePlacement; the endpoint refuses bulk deleting one's
    own topics (src/topics/bulk.rs:992); the user and message topic
    lists' toggles (183)
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
    ~~Rails' `primary-group-<name>` for a member with a primary group
    (src/html.rs:800)~~ (done in `feat/tag-category-chrome`:
    ApplicationHelper#body_classes' class, the group's name lowercased,
    first among the chrome's classes and after a category page's
    `category-<slug>` as Rails' layout has them); ~~a tag in a category
    draws no breadcrumbs or pills (src/routes/tags.rs:568)~~ (done in
    `feat/tag-category-chrome`: /tags/c/... draws the category's
    navigation, category_breadcrumbs with the tag in TagDrop (`no tags`
    on the untagged list) and category_nav_items at NavItem.pathFor's
    `/tags/c/<category>[/none]/<tag slug>/<id>/l/<filter>`, counted in
    the category and the tag, `latest` never; the port's placeholder
    headings go. A plain tag list with a `category` param stays the
    tag's own page, as Rails redirects only a wrong slug or a name to
    the /tags/c form. Left: the category and tag edit buttons
    (TagCategoryAdminDropdown) and the composer's tag there); the
    welcome banner and its classes on a
    category's or a tag's list under welcome_banner_page_visibility
    discovery or all_pages (only latest, the filters and categories ask
    for it, src/html.rs:1914); ~~/tags/intersection/:tag/*additional_tags
    (`tags-intersection`) is not routed~~ (done in
    `feat/tag-category-chrome`: tags#show on the name route's list path,
    the other tags after tag_params', so the next page is
    `/tag/:tag_name?...&tags[]=`; a repeated tag or the first again is
    url_for(params)'s 302, a synonym first its target's 301, one tag
    alone `/tags/*tag_name`'s redirect; the page has `tags-intersection`
    and no pills, as DNavigation draws none. Left: BreadCrumbs'
    TagsIntersectionChooser (the page names its tags in the port's tag
    heading instead), the category drop from its `category` param, the
    tagIntersection search context and the .rss feed); /filter's
    `navigation-filter` comes with item 197
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
    274a. ~~`fix/profile-polish-rest`~~ (done) S: the footer's grouped
    bookmarks menu (TopicBookmarksMenu with a post's bookmark among the
    member's in the topic) drew its trigger and refused the click. Its
    DDropdownMenu is now the page's template.topic-bookmarks-menu-template
    (post_view::topic_bookmarks_menu) that static/js/topic-bookmarks-menu.js
    opens as a DMenu (a bottom sheet on a phone): a row per post bookmark
    by post number with its name, each opening TopicBookmarkPostSubmenu by
    it (right-start, closing the menu with it) to jump to the post
    (routeTo) or delete its bookmark; the topic's bookmark to delete, or
    Bookmark topic (POST /bookmarks with BookmarkFormData's saveData as
    jQuery sends it, the user's auto delete preference, an empty name
    stored empty as Rails does, where the port stored null); Delete all
    bookmarks with several, after dialog.yesNoConfirm, through the newly
    ported PUT /t/:topic_id/remove_bookmarks
    (TopicsController#remove_bookmarks, BookmarkManager#destroy_for_topic:
    each bookmark and its dependent reminder notifications, the topic
    user's flag; 403 anonymous, 404 not visible). Each shows Ember's toast,
    then the footer's button and the posts concerned are drawn again from
    the server, as is the button after a post's bookmark toggles from its
    own menu. The post menu's and the footer's toggles send the same
    saveData. A single bookmark with a reminder titles the footer's and
    the post's buttons created_with_reminder_generic with
    formattedReminderTime, uncapitalized as BookmarkFormData#formattedReminder
    has it, in the member's timezone (data-tz-zone) or the browser's
    (viewer_dates' new `@reminder-time`), and the post's title carries the
    bookmark's name, which it dropped. GET /g/:name/members as a page
    redirects to the group's page at its lowercased name (group.members'
    beforeModel, the group route's serialize), the 404 page for a group
    the viewer can't see. Five write cases in parity/writes/bookmarks
    (topic_remove_bookmarks*, bookmark_topic_from_menu), tests in
    tests/sessions.rs and tests/groups_pages.rs, a browser spec
    (topic-bookmarks-menu.spec.js). Editing a bookmark, the topic's or a
    post's, opens BookmarkModal, which stays refused with 196.
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
    280a. `fix/chat-dm-thread-pages-rest` S: ~~the threads list with
    threads~~ (done with 160: UserThreads' rows and ThreadPreview; its
    load more is 160a's), the DM list's filtered empty state
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
282. ~~`feat/message-composers`~~ (done) M: a member couldn't start a
    private message (the demo rehearsal's step 10): the profile's Message
    button and the messages page's New Message had no handler, and
    /new-message refused "the message composer" (269a, 196). Now the
    composer's privateMessage action (static/js/composer.js): the
    composer-action-private-message and private-message classes, the
    title with its own placeholder and no category (tags only with
    site.can_tag_pms), the recipients above it (PmUserSelector:
    EmailGroupUserChooser over /u/search/users with the messageable
    groups and, for can_send_private_email_messages, an email, as a
    multi-select: typing on the header opens it, rows ranked as
    organizeResults, the chosen ones excluded and listed as removable
    choices, backspace highlighting then removing the last, Escape and
    clicking outside closing; static/js/email-group-user-chooser.js,
    whose helpers add-pm-participants.js now shares), the staff's
    official warning toggle for one user, "Create message" and "Send
    message" with the envelope, focus on the recipients while there are
    none, its draft (new_private_message_<time>, action privateMessage
    with title, recipients and archetypeId) and the post with title,
    archetype and target_recipients, going to the new message. Every way
    Ember opens it: the profile's Message button (composePrivateMessage
    to the profile's user), the messages page's New Message (to no one),
    the user card's Message button (to its user, with the post's link and
    "RE: <title>" when the card was opened on a post), /new-message with
    username, groupname (after Group.messageable), title, body and tags
    on a page load or a link, and resuming a message draft from the
    drafts menu and the drafts tab, which opened it as a new topic or
    refused. A reply in a message now says Message and saves as "Send
    message" with the envelope (saveLabel, saveIcon). The reply area's
    with-category and with-tags follow the mode, with compose.scss's
    user-selector, add-warning and private-message rules. Compared with
    Playwright against rs-ref-d as user1: the composer opened from the
    profile and the messages page (markup, focus, screenshot), the
    chooser's search, rows, choices and body position, the draft and
    the POST it sends; same but the composer actions caret and the rich
    editor (196). Recorded as messages/compose_new_message (rs-ref-d);
    tests/messages_pages.rs. Refused as before: sending the warning
    (is_warning, posts#create), a message to an email address
    (PostCreator), the composer actions menu (196).
283. ~~`feat/infinite-loading`~~ (done) M: found by the demo rehearsal: a topic over
    one chunk (/t/118, 100 posts) drew its first 20 posts and scrolling
    loaded nothing, so its footer and Reply never showed and posts 21-100
    couldn't be read by scrolling; the topic lists ended with DLoadMore's
    sentinel and a "Next page" link. The server side is done: GET
    /t/:topic_id/posts(.json) (TopicsController#posts,
    TopicViewPostsSerializer): the posts by `post_ids[]` (or one id),
    those after or before a `post_number` with `asc`
    (filter_posts_by_post_number, the megatopics' window), the related
    messages, suggested topics and group name only with
    include_suggested=true, and the topic's id; a missing topic is a 404,
    a message for a visitor a 403 not_logged_in, a topic the viewer can't
    see a 404 unless detailed_404 is on
    (hide_inaccessible_topic_when_detailed_404_disabled). TopicView takes
    post_number, then post_ids, then filter_post_number, as filter_posts
    does, and topics#show now leaves out the related messages too while
    there is a next page (perform_show_response turns off both). Refused:
    filter, username_filters and include_raw. Ten golden cases recorded
    from rs-ref-d: topic 35 by ids, with its suggested topics, as user1,
    after and before a post number, the default chunk; message 42 for a
    visitor, a stranger and a participant (its related messages and
    group name); a missing topic.
    The topic's post stream loads more as Ember's does
    (static/js/post-stream.js; PostStream#appendMore and #prependMore,
    post-stream.gjs, PostLoadMoreAccessible, DLoadMore): #posts carries
    the stream's ids and chunk size, and whether the view had its
    suggested topics; while the first post isn't drawn the stream starts
    with a load more sentinel and its screen reader heading ("Load more
    posts above"), while the last isn't it ends with one ("... below")
    where the bottom boundary was, and the posts wrapper has
    PostStream's `<span>` and its two spinners. A sentinel coming into
    view (its observer made again after each chunk, so one still in view
    goes on) asks for the next 20 ids of the stream, or the 20 before,
    with Ember's params (post_ids[], include_suggested=true until a chunk
    brought the suggested topics) from /live/posts/:id, which draws them
    as the page does (html::topic_page on the same view; Ember draws the
    posts.json in the browser). While a chunk loads its sentinel goes,
    the spinner beside the stream shows and, below, a placeholder stands
    in for each post; posts above keep the first post on screen where it
    was. Once the last post is in (loadedAllPosts) what Ember draws only
    then comes out of templates: the bottom topic map, the reply where,
    the footer buttons with Reply, the more topics (the chunk's suggested
    topics when the page had none) and the page footer
    (Discourse.showFooter, page.js, for Chrome::defer_footer). After
    each chunk the page's live stream connects again from its last event
    with the posts drawn, with the tail once at the end, and the server
    no longer sends changes to posts outside the page's range
    (PostStream#triggerChangedPost finds only loaded posts), which also
    ends the console errors a revision of an undrawn post caused. The
    timeline: a click on the track or a date and the scroller's drag
    (d-pointer-drag: pointer capture, `--dragging`, the body's
    `dragging`, data-pointer-drag on the desktop scroller too) set the
    percentage under the pointer and commit as the container does,
    jumping to the index's post, or past the end to the last post; a
    post the page doesn't draw is looked up by id with GET
    /t/:id/posts.json, then the topic is routed to its number, which
    loads the posts around it (TopicController#_jumpToPostId). The URL
    keeps following the post at the eyeline and screen-track times the
    posts loaded. Ember's posts wrapper before and after a load
    and the request it sends, as Playwright saw them on rs-ref-d with
    topic 35 grown to 35 posts, are what the page and post-stream.js draw
    and send (tests/html_pages.rs). Checked in Chromium
    on a copy of the demo database: /t/118 scrolled to post 100 with the
    footer, Reply and five suggested topics; /t/118/50 loaded up to post
    1 and down to 100; the timeline's click and drag to unloaded posts;
    the rehearsal's steps 1, 2, 6 and 7 pass (step 6 now scrolls to the
    end of the topic before Reply, as a presenter does). The discovery
    lists load their next page the same way (static/js/discovery-topic-list.js;
    DiscoveryTopics#loadMore, TopicList#loadMore, DLoadMore with
    LOAD_MORE_ROOT_MARGIN, 600px below the window): the sentinel carries
    more_topics_url in place of the "Next page" link, the page is asked
    for (its HTML, as an XHR, so no page view is counted), its rows not
    already listed are appended with the first highlighted, the sentinel
    takes its more_topics_url, the footer's spinner shows meanwhile, and
    with room left in the window it goes on; with nothing more the footer
    is the last page's (its dismiss buttons) and the page footer shows.
    Checked in Chromium: /latest loaded page after page as it scrolled,
    /top to its end with the dismiss row and the page footer.
    Left, split into 283a.
    283a. `feat/infinite-loading-rest` S: what Ember draws across chunks:
    the time gap before a chunk's first post and the visited line after
    the last read post when a chunk lands next to it are decided within
    the chunk (post_view::stream); the lists' last visit separator and
    the bottom dismiss buttons' five topic rule are the loaded page's,
    not the whole list's; the a11y announcements while loading
    (a11y.announce); the posts' cloaking (post-stream--cloaked) is not
    ported, so every loaded post stays drawn; megatopics (isMegaTopic,
    lastId, fetchNextWindow over post_number and asc, which GET
    /t/:id/posts.json answers) are
    not, the topic view always carries the stream; keyboard navigation
    (j/k, keyboard:move-selection loading at a boundary) waits for the
    keyboard shortcuts, which the port doesn't have; GET
    /t/:id/posts.json's filter, username_filters and include_raw, and GET
    /t/:id/post_ids (TopicsController#post_ids) are not ported. The
    messages lists draw DLoadMore's sentinel without loading more
    (src/messages_view.rs:403).

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
213. ~~`feat/composer-messages`~~ (done) M: GET /composer_messages
    (ComposerMessagesFinder) was not routed, with discourse-solved's
    check_topic_is_solved education message and
    disable_solved_education_message. Now ComposerMessagesController
    answers it and /composer_messages/user_not_seen_in_a_while
    (routes/composer_messages.rs). The finder (composer/messages.rs) runs
    the education message, new user many replies, dominating topic, get a
    room and don't feed the trolls checks in their definition order, then
    the plugins' (plugins::composer_messages: solved's
    check_topic_is_solved, plugins/solved/composer_message.rs), nothing
    while editing; dominating topic and get a room log
    notified_about_dominating_topic and notified_about_get_a_room in
    user_histories and show once a topic. The index carries
    TopicLink.duplicate_lookup for a topic the member can see, internal
    links to what they cannot see left out. The client is
    static/js/composer-messages.js (composer-messages.gjs,
    composer-message.gjs and its templates, ComposerTipCloseButton) in
    #reply-control's .composer-popup-container: the messages asked for
    as the composer opens (cached per action, topic and post), those
    waiting for typing shown on a key's release, the education message
    once until a post is saved, hide_if_whisper, the message to oneself
    and recipients not seen in a while, get a room's switch to a message,
    dismissing by the close button or Esc (hitEsc; with none left Esc
    minimizes the open composer and leaves fullscreen, composer-body's
    keyDown), hidden while the composer is minimized, none in the mobile
    view; composer-editor's
    preview half (lib/link-mentions: the preview's mentions linked from
    /composer/mentions, then the group mentioned, cannot see and @here
    warnings) and the composer service's duplicate link check
    (lib/link-lookup, afterRefresh); the rich editor's mention warnings
    reach it through composer-messages:create. The popups' rules are in
    composer.css (compose.scss:881-995), and the browser suite turns
    education off as rails_helper does (setup.sql). Tests:
    tests/composer_messages.rs (23: composer_messages_finder_spec,
    composer_messages_controller_spec and the solved plugin's finder spec
    as written, stubbed counts as the user's stats), browser
    composer-messages (11: composer-messages-test.js, composer-test.js'
    duplicate link and group mentioned notices,
    dont_feed_the_trolls_popup_spec.rb, and Esc), composer's "Esc in the
    composer" (5) and the rich editor's Esc, and
    rich-editor-autocomplete's "with composer messages". Left: the
    rest of the ShareTopic modal (287, 288). The dominating topic's Share and
    the topic footer's share open ShareTopic (static/js/share-topic.js:
    the link, its copy button and the share_links sources; browser
    share-topic, 5, and composer-messages' dominating topic Share)
286. ~~`feat/similar-topics`~~ (done) M: GET /similar_topics was not routed, so
    ComposerMessages' similar topics popup was never drawn. Now GET
    /topics/similar_to and /similar_topics (routes/similar_topics.rs:
    SimilarTopicsController, title required, nothing under
    min_title_similar_length or until the site has more than
    minimum_topics_similar topics) answer with Topic.similar_to
    (topics/similar.rs: the title weighted A and the cooked, scrubbed
    start of the body weighted B as one OR tsquery over topic_search_data,
    unaccented under search_ignore_accents; visible listable topics the
    viewer may see, without category definitions, categories search
    ignores and the member's muted categories; the ten best by ts_rank,
    then the three most alike by trigram similarity over 0.2) as
    SimilarTopicSerializer does: the blurb of the first post, the url,
    and each topic side-loaded as a TopicListItemSerializer without the
    viewer's topic data (topic_list Mode::Bare) with its posters' users
    and groups. The popup: one second after the last key released in a
    new topic's composer (composer-body keyUp, composer:find-similar)
    static/js/composer-messages.js asks GET /composer/similar-topics for
    the rows drawn as the search menu's topic results with their links
    (search/menu_view.rs similar_topic), not for a category with form
    templates (the chooser row's data-form-template-ids), not twice for
    the same title and text, hidden again when nothing is alike. Measured
    against Rails (parity/writes/similar_topics: a visitor, a member with
    tags, staff with a restricted and a muted category, a site under the
    minimum). Tests: tests/similar_topics.rs (17: topic_spec's
    .similar_to and similar_topics_controller_spec), browser
    composer-messages "Similar topics" (6). Refused: Chinese and Japanese
    segmentation of the title. Not ported: the
    similar_topic_candidate_ids modifier (discourse-ai's embeddings)
287. ~~`feat/share-topic-rest`~~ (done) M: ShareTopic
    (components/modal/share-topic.gjs, static/js/share-topic.js) drew
    only the topic's link and sources. Now the new topic button
    (replyAsNewTopic for can_reply_as_new_topic: composer.js'
    replyAsNewTopic opens a new topic in the topic's category, or a
    message to the message's recipients, under the topic's draft key
    with post.continue_discussion linking the post or the topic's
    first), the restricted category's warning (DModal's @flash,
    #modal-alert, from GET /c/:id/visible_groups, which was routed
    already: topic.share.restricted_groups with the groups' names), and
    the modal for a post (post.share.title and instructions, the post's
    shareUrl, longDateNoYear of its shown date as Discourse.modal's new
    subtitle): from the post date link (post/meta-data/date.gjs;
    topic.js takes the click in the capture phase, ahead of the link's
    boosted navigation) and the post menu's share (PostMenuShareButton,
    drawn where post_menu lists it, posts/view.rs; the platform's share
    where pwa-utils' canNativeShare has one, the modal otherwise or
    should it fail). Tests: browser share-topic (10: share-topic-test.js'
    new topic, post date link and restricted category cases, a
    message's new message, the post menu's share), tests/sessions.rs'
    the_post_menu_draws_share_where_the_setting_lists_it. The quote
    toolbar's Quote where the viewer may not reply (a closed topic) was
    refused by name; it starts the new topic the same way with the
    quote after the link (TopicController#selectText; browser
    quote-button, 2, the toolbar's first). Left: 288
288. `feat/share-topic-invite` S: what 287 left of ShareTopic: ~~the
    invite button~~ (done, 291), the composer's Share for a topic other
    than the page's (composer-messages.gjs' shareModalData is the
    composer's topic; the modal reads the page's #quote-button-config,
    which only a topic page carries), and the share-topic-sources plugin
    outlet
289. ~~`feat/bookmark-modal`~~ (done) M: editing a bookmark was refused
    by name in the topic footer's bookmarks menu (the topic's and a
    post's) and the bookmarks tab. Now BookmarkModal
    (components/modal/bookmark.gjs, static/js/bookmark-modal.js) opens
    there for the bookmark: its name, what happens after the reminder
    (auto_delete_preference's ComboBox), the reminder there is
    (formattedReminderTime), and DTimeShortcutPicker
    (ui-kit/d-time-shortcut-picker.gjs with lib/time-shortcut.js and
    lib/time-utils.js, as Discourse.timeShortcutPicker for the pickers
    still to come): defaultTimeShortcuts in the member's timezone with
    hideDynamicTimeShortcuts (suggest_weekends_in_date_pickers), the
    bookmarked post's local date, the last custom time
    (keyValueStore), none hidden without a reminder and labelled
    remove_reminder_keep_bookmark with one, and the custom tile's
    DatePickerFuture (Pikaday), time input and DRelativeTimePicker. A
    tile saves and closes, the custom and relative times wait for Save,
    a click outside saves (Discourse.modal's beforeClose is told
    "click-outside"), Cancel and the close button do not, the trash
    deletes after asking when there is a reminder; ItsATrap's sequences
    (enter, d d, l t, n d, n b w, n m, c r, n r). PUT and DELETE
    /bookmarks/:id, then the footer and the post, or the tab's list,
    drawn again. The modal's config is the script tag's
    (bookmarks::modal_script_attrs) for every member's page; the tab's
    rows and the menu's bookmarks carry BookmarkFormData's fields.
    Tests: browser bookmark-modal (4) and topic-bookmarks-menu's
    "editing a bookmark opens BookmarkModal". An existing reminder is
    kept when only the name is saved. Left: the
    bookmark-modal-after-name plugin outlet; the user status modal's
    tiles are this picker's since 130c, and BookmarkMenu is 290
290. ~~`feat/bookmark-menu`~~ (done) M: a post's bookmark button, and
    the footer's while the topic's is the only bookmark, toggled the
    bookmark. Now BookmarkMenu (components/bookmark-menu.gjs, in
    static/js/topic-bookmarks-menu.js) opens on them, a DMenu without an
    arrow: showing it on what is not bookmarked bookmarks it at once
    (onBookmark, POST /bookmarks with the button's saveData) and offers
    a reminder under "Bookmarked!": in two hours, tomorrow, in three
    days (PUT /bookmarks/:id with the time) or more options
    (BookmarkModal for the bookmark just made); on a bookmark it offers
    Edit (the modal), Clear reminder when there is one, and Delete, with
    their toasts. The post and the footer are drawn again as it closes.
    The buttons carry the bookmark's BookmarkFormData fields
    (data-bookmark, posts/view.rs); the click is taken ahead of the
    button's htmx toggle, which is what is left without the scripts.
    Tests: browser bookmark-modal's two BookmarkMenu tests, post-menu and
    topic-bookmarks-menu through the menu
291. ~~`feat/create-invite-modal`~~ (done) M: the sidebar's Invite
    link went to /new-invite, which was not routed; the invites tab's
    Invite button was refused by name; ShareTopic had no invite button.
    Now CreateInvite (components/modal/create-invite.gjs,
    static/js/create-invite-modal.js) for a new invite: the simple
    mode's Create link (createLink: the default redemptions and
    invite_expiry_days) with the link to copy or share, the footer gone,
    under link_validity_MF's line (rendered for the defaults,
    message_format.rs); Edit link options turns to the form (FormKit's
    fields): description (at most 100), the email or domain it is
    restricted to (emailValid, hostnameValid), the redemptions allowed
    (not for an email, capped at one), expires after (1, 7, 30, 90 days,
    invite_expiry_days, never), the groups joined (GroupChooser, for
    staff and group owners) and, for an email with allow_email_invites,
    the custom message and Create link and send email. POST /invites,
    then PUT /invites/:id for the invite made (Invite#save);
    InviteModalAlert under the header carries the result and the link.
    A link to /new-invite followed in the page opens it where the member
    is, a page load lands on the homepage with it (routes/new-invite.js;
    a visitor on the login); the invites tab's button draws the list
    again once it closes; ShareTopic's invite button (allowInvites) from
    the footer's share. The config is the script tag's
    (invites::modal::script_attrs) for who can invite. Fixed with it:
    chat's channel filter toggle also took the invites tab's empty state
    button (the same empty-channels-list identifier), saving a chat
    preference and loading the page again. Tests: browser
    create-invite-modal (5). Left: CreateInviteWithRoles, bulk invites (the
    topic arrived at is 293). Done after, `feat/cannot-invite-dialog`:
    a member who may not invite has the script with the refusal alone
    (invites::modal::refused_attrs) and /new-invite, followed or loaded,
    tells them so in a dialog (cannot_invite_to_forum) where it did
    nothing
292. ~~`feat/edit-invite-modal`~~ (done) S: an invite's Edit on the
    invites tab was refused by name, and the options of a link just made
    still offered the days until it expires under Create link. Now the
    row carries the invite (data-invite, for who may remove it) and Edit
    opens CreateInvite with it (editInvite: `editing`, `invite`): Edit
    invite for a title, InviteModalAlert with the link to copy from the
    start, the invite's values in the form, Update and Update and send
    email, saved with PUT /invites/:id (the topic it has sent along),
    the list drawn again once it closes. An invite that exists has
    expiresAt for a field: DFutureDateInput without its shortcuts
    (@noRelativeOptions), a date (DatePickerFuture, Pikaday) and a time
    in the browser's zone, as
    Discourse.timeShortcutPicker.futureDateInput in bookmark-modal.js;
    no date disables the time and is refused as required.
    parse_time_param reads what it sends (YYYY-MM-DD HH:mmZ), which was
    an Unsupported error. Tests: browser create-invite-modal (6), a unit
    test for the formats. Left: penalize-user.js and
    topic-admin-modals.js keep their own copies of the date input
293. ~~`feat/invite-topic-chooser`~~ (done) S: the form had no field for
    the topic an invite arrives at. Now, for staff where members need no
    approval (canArriveAtTopic), Arrive at topic: TopicChooser, a
    ComboBox over GET /search/query with "status:public" (the public
    topics for no term; a term searches regular topics by title, url or
    id), the topic chosen in its header with a button that clears it.
    The form sends topic_id each time, blank for none as jQuery's param
    does, so an update takes the topic away; invite_to_topic goes with
    an email that is sent. Tests: browser create-invite-modal (7). Left:
    the rows' topic statuses and category
294. ~~`feat/bulk-invite-modal`~~ (done) S: an admin's Bulk Invite on
    the invites tab was refused by name, with POST /invites/upload_csv
    (192e) already there. Now CreateInviteBulk
    (components/modal/create-invite-bulk.gjs, in user-invited.js): the
    locale's instructions, a CSV file input (another kind of file is
    refused in a dialog, csvOnly), the upload button disabled until a
    file is chosen and counting the progress while it is sent, then the
    word that a message follows and only Close. Tests: browser
    bulk-invite-modal (1, the invite the job makes checked in the
    database)
295. ~~`feat/group-request-membership`~~ (done) S: a group page's
    Request and Message buttons had no handler (the endpoint was 82e's).
    Now groups.js: Request opens RequestGroupMembershipForm
    (components/modal/request-group-membership-form.gjs: the title with
    the group's name, the reason starting as the group's
    membership_request_template within 5000 characters in
    DExpandingTextArea, Submit disabled while it is empty or the request
    is out), POSTs /groups/:name/request_membership.json and goes to the
    message it answers with, errors in popupAjaxError's dialog (loaded
    with its strings on a page without one); a visitor's goes to the
    login. Also on the directory's cards, and exported as
    Discourse.groupMembership.request({ name,
    membership_request_template }) for the group card. Message is
    messageGroup: the composer for a message to the group. The endpoint
    now refuses a silenced member as GroupMembershipRequester does.
    Tests: browser group-request-membership (5), groups_pages. Left:
    the group card's two buttons, the composer marking the recipient as
    a group (hasGroups), and Join and Leave still reporting errors with
    window.alert
296. ~~`feat/document-title-counts`~~ (done) S: a member's tab title
    never changed with what was new. Now static/js/document-title.js,
    Ember's document-title service with d-document's listeners: by the
    title_count_mode option, the notifications count
    (all_unread_notifications_count and unseen_reviewable_count, taken
    as onNotification changes them while the tab is hidden, 0 once it
    shows again) or the contextual one (the latest list's banner count;
    the posts others make in the topic viewed in a hidden tab, cleared
    on showing it; 0 with each page), as "(N) title", or with
    dynamic_favicon drawn on the favicon instead (update-tab-count's
    Favcount canvas), and no title count during do not disturb. The
    numbers ride the header redraw (Badge::count_attrs on the avatar),
    the banner's data-count and a marker ending the `post` event for a
    post by someone else (sent off the topic's last page too). The
    prefix goes back on whatever writes <title> (each page, the title
    editor) and readers ask Discourse.documentTitle.getTitle()
    (local-dates). Tests: browser document-title-counts (6), live_page.
    Left: ~~chat's addDocumentTitleCounter and its forced updates~~
    (done in `feat/chat-notify-client-rest`, 225b: nothing in core or
    chat passes forced, the option is ported), /favicon/proxied for a
    favicon on another origin, the banner on lists other than the front
    page's latest, and
    discourse:focus-changed's other listeners
297. ~~`feat/desktop-notifications`~~ (done) M: a member's alerts
    (/notification-alert/:id) only filled the header's
    #notification-alert, and the preferences' PushNotificationSelect
    refused every level but none. Now static/js/desktop-notifications.js
    (lib/desktop-notifications, the desktop-notifications service's
    browser half, subscribe-user-notifications' onAlert): the stream
    sends the alert's payload as a `notification-alert` JSON event
    beside the header's (none in do not disturb, as before), shown with
    the Notification API when the device isn't mobile, the permission
    wasn't denied as the page loaded, the tab is the primary one (in
    sight at load or focused since, no other tab focused after: the
    keyValueStore's focus-tracker; a focused tab shows them too, as
    Ember's) and notifications-disabled isn't "disabled": the title
    translated_title or notifications.popup.<type> (username, topic,
    site_title, group_name), the excerpt as body, the small logo else
    the logo as icon, the tag discourse-notification-<title>-<topic>,
    its click routing to the post and closing it. The select enables
    (Notification.requestPermission, confirmNotification closed after
    10s, the store "enabled", the level saved) and disables (the store
    "disabled", the level none), shows the saved level only while the
    browser is subscribed, and its denied and not supported buttons
    follow Notification alone. NotificationConsentBanner
    (push_notifications_prompt, an installed app, permission not yet
    answered, not dismissed) above the global notice. No service worker
    controls the page, so isPushNotificationsSupported is false and
    every browser takes the desktop branch; with 186 a push-capable
    browser under enable_desktop_push_notifications subscribes to push
    instead and desktop is disabled for it. Tests: browser
    desktop-notifications (8), sessions
    members_hear_their_notifications_on_every_page and
    the_user_menu_pauses_notifications. Left: ~~the handler registry
    (registerDesktopNotificationHandler, 225b)~~ (done in
    `feat/chat-notify-client-rest`), the
    desktop-notification-opened app event, Mobile.mobileForced in
    isMobileDevice, isEnabledPush and the push half (186); the
    header's #notification-alert, which Ember has not, still drawn
214. `feat/csp` M: the content_security_policy settings and header with a
    per-request nonce (only the problem check reads the setting,
    problem_checks.rs:227), and discourse-lti's resubmit page script
    taking that nonce (plugins/lti.rs:54; routes/omniauth.rs:676-693) so
    its auto-submit runs under strict-dynamic
215. ~~`feat/chat-channel-settings`~~ (done) M: a channel's info pages
    404ed and what they edit had no endpoints. Now /chat/c/:slug/:id/info
    replaces itself with /info/settings (ChatChannelInfoIndexRoute),
    /info/settings and /info/members are drawn (plugins/chat/info.rs:
    ChatRoutesChannelInfo's navbar back to the channel with the title and
    star, ChatChannelStatus, ChannelInfoNav, withChatChannel's redirects
    to the channel's own slug or /chat, the members route sending a
    channel that isn't open to the settings), and
    static/js/chat-channel-info.js acts on them: the name, slug and emoji
    modal (with /slugs.json and the emoji picker), the description modal,
    mute and the notification level (PUT notifications-settings/me), auto
    join after its confirmation, channel-wide mentions and threading, the
    close and open modal (PUT .../status: Chat::UpdateChannelStatus, the
    chat_channel_status_change staff log, /chat/channel-status), the
    delete modal (DELETE /chat/api/channels/:id: Chat::TrashChannel with
    the moved slug, its hashtag remap and the chat_channel_delete log;
    Jobs::Chat::ChannelDelete clears webhooks, drafts, memberships and
    the messages' rows and trashes them), leaving or joining, and the
    members list (the filter, more as it scrolls, a profile, removal).
    UpdateChannel changes the slug (generate_auto_slug, ensure_slug_ok,
    the remap) and answers its validation errors as record_invalid
    (dm_groups.rs). POST .../invites (Chat::InviteUsersToChannel,
    chat_invitation notifications, its rate limit) and PUT
    /chat/user_chat_enabled/:user_id are routed. The live page hears
    ChatSubscriptionsManager's global channels (new channel, status,
    edits, metadata, each followed channel's kick; routes/live.rs; a
    channel joined or left ends the stream, and the page reconnects to
    what the member follows now), and
    the channel page redraws on its status or edits and leaves on a kick
    (static/js/chat.js). Deleting a category destroys its empty live
    channel (category_writes.rs), and can_delete and cannot_delete_reason
    read live channels and messages only. Editing, trashing or restoring a
    message of a trashed channel fails with a 500 as Rails' NoMethodError
    does. Tests: tests/chat_channel_settings.rs. Left: the settings'
    archive row (216), adding members to a DM from its members (222a),
    the composer's mention warnings that send invites (218), the drawer's
    info route, info/about and info/search (404 as Ember has no route for
    them).
216. ~~`feat/chat-channel-create`~~ (done, stacked on
    feat/chat-channel-settings for its slug checks and record_invalid
    answers) M: nothing created a channel, and the list menus and the
    sidebar's hover menu did nothing. Now POST /chat/api/channels
    (plugins/chat/channel_create.rs: Chat::CreateCategoryChannel's
    policies, contract, the existing name's InvalidParameters, the
    channel's auto slug and validations as record_invalid, the creator's
    membership, auto join), GET category-chatables/:id/permissions
    (staff, the restricted category's posting groups visible to the
    viewer with their member count, else everyone), and
    static/js/chat-channel-menus.js on every page a member chats on:
    ChatModalCreateChannel (name, slug from /slugs.json, description, the
    category with its security hint and the auto join warning, emoji,
    auto join, threading) behind the browse page's New channel button
    and the menu's, ChatChannelListSidebarMenu from the lists' options
    buttons and the sidebar headers (new channel, browse, new DM and
    group chat through chat-new-message.js, the filter and sort submenus
    saved as the member's chat_channel_list_* options), the filter
    toggle, ChatSidebarChannelListFilterEmptyState (plugins/chat/view.rs,
    refused before), and ChatChannelSidebarContextMenu from a sidebar
    link's hover button (notification levels and mute, settings, star,
    leave). CreateDirectMessageChannel's name errors answer
    record_invalid with the DM and channel kept, as Rails' service has no
    transaction (dm_create.rs). Tests: tests/chat_channel_create.rs.
    Deviations: the category chooser is a native select over /site.json's
    categories (CategoryChooser is select-kit), the empty state's Show
    all saves the filter as all (Ember bypasses it for the session), the
    context menu reads the channel from GET /chat/api/channels/:id.
    Left, as 216a.
    216a. ~~`feat/chat-channel-archive`~~ (done) M: archiving answered
    Unsupported. Now POST /chat/api/channels/:id/archives is
    ChannelsArchivesController#create (src/plugins/chat/archive.rs: a
    failed archive's retry under ensure_can_change_channel_status!, else
    chat_allow_archiving_channels, params.require(:archive) and its type,
    the destination checks, can_change_channel_status?(:read_only), then
    create_archive_process: TopicCreator#valid? for a new topic as 400
    errors, the channel made read only (logged, published), the
    chat_channel_archives row, the job). ChannelArchiveService#execute:
    the new topic by the system user with the archiver's guardian (its
    first post, no excerpt), chat_archive_destination_topic_status for a
    new topic only, find_in_batches over the live messages with threads
    kept whole or split with the original message repeated and
    calculate_thread_ranges, each batch a post (PostCreator in import
    mode, without validations) then trashed and counted, the members
    unfollowed, the channel archived, notify_archiver (archive_error, the
    complete, failed and failed_no_topic system messages with the
    channel's hashtag, /chat/channel-archive-status); Jobs::Chat::
    ChannelArchive (a complete archive marked archived, the channel's
    advisory lock, retry: false). transcript.rs is now the whole
    TranscriptService: the messages' text and uploads (218 left them
    out), noLink, reactions, threads (their replies' blocks, threadId,
    threadTitle with its range), a lone original message expanded.
    PostCreator gained import_mode (src/posting/create.rs) and TopicLink
    recognizes /chat/ paths. The page: the settings' archive row and
    ChatModalArchiveChannel with ChatToTopicSelector
    (chat-channel-info.js), the channel page redrawn on
    /chat/channel-archive-status (chat.js). Tests: tests/chat_archive.rs,
    tests/browser/chat-archive.spec.js. Deviations: each batch's post
    and its trashing are two transactions (PostCreator commits its own);
    the category chooser is a plain select, the tag chooser a comma
    separated field, ChooseTopic a plain radio list over
    /search/query.json; tag watched words match the first post's text
    too (the topic is made with it); a post PostCreator refuses fails
    the archive where Rails trashes the batch anyway. Left: nothing.
217. ~~`feat/chat-moderation`~~ (done, but the queue entries) L: chat
    messages could not be flagged, selected, deleted together or moved.
    Now src/plugins/chat/flags.rs ports POST .../messages/:id/flags
    (four a minute, Chat::FlagMessage's contract and policy with
    can_flag_chat_message? and can_flag_message_as?, then
    Chat::ReviewQueue#flag_message: notify flags refused in DMs as
    invalid parameters, can_flag_again? as message_already_handled, a
    notify_user flag without a message doing nothing,
    Chat::ReviewableMessage.needs_review! with its payload and
    target_created_by, add_score, enforce_auto_silence_threshold over
    sensitivity_score(chat_silence_user_sensitivity, 0.6) for
    chat_auto_silence_from_flags_duration minutes after the commit, and
    publish_flag!: self_flagged to the flagger, flag to staff); messages
    in DMs offer no notify flags. src/plugins/chat/review.rs is
    Chat::ReviewableMessage#perform behind PUT /review/:id/perform
    (src/review.rs hands it the type): build_combined_actions checking
    the action and its aliases, agree_and_keep_message, _keep_deleted,
    _restore, _delete (and its suspend, silence and delete_and_agree
    aliases), disagree and disagree_and_restore lifting a silence this
    reviewable brought, ignore, unsilence_user, the version, the
    transition, the scores, the flag stats and the recalculated score.
    DELETE .../messages is Chat::TrashMessages (modify.rs: up to 200,
    can_delete_chat? for each, notifications, last message ids, read
    states, one bulk_delete publish of the ids as given) and POST
    .../messages/moves Chat::MessageMover (mover.rs: public channels
    only, the copies with their reactions, uploads, mentions,
    revisions, webhook events and hotlinked media, the originals
    trashed and published, replies cut, read states reset, the system
    user's messages_moved placeholder). The page: the flag menu entry
    opens core's flag modal (static/js/flag-modal.js, loaded on demand,
    with ChatMessageFlag: the message's available_flags, chat's
    descriptions, POST .../flags; the chat page carries the modal's
    config), the flag marks drawn from self_flagged and flag, and select
    mode (the checkboxes, shift-click ranges, ChatSelectionManager in
    the composer's place with its delete count, DeleteMessagesConfirm,
    ChatModalMoveMessageToChannel over the member's other public
    channels, as a plain select, then the first moved message).
    tests/chat_moderation.rs. The review queue's entries and page for
    chat reviewables and take_action are 148a's. Refused: notify flags
    with a message (the companion PM), the transcript a DM flag sends,
    queue_for_review, flagging messages with uploads (148b), moving
    threads. Select mode's quote and copy are 218's
218. ~~`feat/chat-message-actions`~~ (done, stacked on
    feat/chat-moderation) M: the message actions left after 217. Now
    POST /chat/:id/quote is ChatController#quote_messages over
    Chat::TranscriptService#generate_markdown (src/plugins/chat/
    transcript.rs: a `[chat]` block per run of one author with quote,
    channel by serialize_attr's marks, channelId, multiQuote and
    chained; at this reference the blocks carry no messages), behind
    select mode's quote (the composer through /new-topic with the body
    and a category channel's category) and copy (the clipboard and
    chat.quote.copy_success). src/plugins/chat/interact.rs: GET
    /chat/:id/:message_id/reactions-users (MessageReactionUsersQuery,
    every reaction whatever the emoji as the query leaves its filter
    out, Chat::ReactionUserSerializer, page and limit) under a hover
    popover (UsersPopup's markup, a tab per reaction, the next page on
    scroll), DELETE .../messages/:id/streaming (Chat::StopMessageStreaming:
    the member's membership, admins, the author or the replied-to author,
    publish_edit!) with the stop-streaming button on streaming messages,
    POST .../messages/:id/interactions (Chat::CreateMessageInteraction:
    the block element by action_id, the interaction saved,
    Chat::MessageInteractionSerializer; :chat_message_interaction has
    no listener until 202), and GET /chat/api/mentions/groups
    (HintsController: five every ten seconds, the visible groups by
    lowercased name, unreachable, over the limit and the rest as invalid)
    with ChatComposerWarningsTracker and ChatMentionWarnings in chat.js
    (a second's debounce, too many mentions, @here and @all where
    disallowed, the group hints; too_many_members_MF rendered server
    side for one, two and more groups). Copy text and the mobile
    long-press sheet (ChatMessageActionsMobile: the message, the
    secondary actions, quick reactions, the picker and reply), select
    hidden from the menu while selecting, the author's name first under
    display_name_on_posts and enable_names without
    prioritize_username_in_ux, the DuplicateMessageValidator's
    exemptions for bots and DMs of two or fewer members (upload-only
    messages can't happen while uploads are refused), target dates in
    any spelling String#to_time reads (Date._parse, numeric zones) and an
    edit too large to diff as Rails' 500. tests/chat_message_actions.rs.
    Left: the popover on mobile (a held reaction), the sheet's bookmark
    button (no chat bookmarks are made from the page), mention parsing
    from the cooked message (a word match here). Refused: transcripts of
    threaded channels, target dates in named zones or with fractions of
    a second
219. ~~`feat/chat-pins`~~ (done) S: nothing pinned: the pin routes were
    unrouted, /chat/c/:slug/:id/pins 404'd and the menu's Pin and Unpin
    did nothing. Now src/plugins/chat/pins.rs ports
    Chat::Api::ChannelPinsController behind chat_pinned_messages (404
    while off): GET .../pins (Chat::ListChannelPins, joinable or 403,
    Chat::ChannelPinsSerializer: the pins in timeline order with
    pinned_at, the excerpt keeping links, the core BasicUserSerializer
    pinned_by and the message, and the member's
    UserChannelMembershipSerializer or null), PUT .../pins/read
    (MarkPinsAsRead: last_viewed_pins_at, 404 without a membership),
    POST and DELETE .../messages/:id/pin (PinMessage and UnpinMessage:
    can_manage_chat_channel_pins? with chat_pinning_messages_allowed_groups
    outside DMs, the 20 pin limit, already pinned and not pinned as 422
    failed_json errors, publish_pin! and publish_unpin! with the
    authoritative count). The pins route draws the channel with
    ChatSidePanel (has-side-panel-expanded) holding
    ChatPinnedMessagesListHeader and ChatPinnedMessagesList: each pin
    as a non-interactive ChatMessage with "Pinned by" and the unseen
    thumbtack, linking to the message, the unpin button for those who
    manage pins, the empty state; chat.js marks the pins read as the
    list opens, unpins from it, acts on the menu's Pin and Unpin, and
    redraws the page on the pin and unpin bus messages (the message's
    mark, the navbar's pinned button). tests/chat_pins.rs. Left: the
    pinned message bar (ChatPinnedMessageBar with its viewport anchor,
    jump, indicator and per-device dismissal) with the list's dismiss and
    show footer and the navbar button's showing only while the bar is
    dismissed (it shows whenever there are pins); the pinned excerpt of a
    message of links only (urls_as_links) is refused
220. ~~`feat/chat-search`~~ (done) S: GET /chat/api/search had no
    route though posting/search_index.rs:631 wrote
    chat_message_search_data. Now src/plugins/chat/search.rs ports
    Chat::SearchMessage behind Chat::Api::SearchController (404 while
    chat_search_enabled is off): the contract (query present, limit
    1..40, offset >= 0, sort relevance or latest, with Rails' messages),
    the channel_id scope (missing or not joinable: 404), else the
    member's secured channels; ProcessSearchQuery's @username (@me) and
    #channel words as filters (the first ten; an unknown user or a
    channel the member can't preview matches nothing), the rest through
    Search.prepare_data and Search.ts_query (prefix match, the site's
    ts config); exclude_threads keeping thread originals; latest by
    created_at desc, relevance unordered as Rails leaves it; one extra
    row for has_more; Chat::MessageSerializer with include_channel (the
    channel after blocks) and meta {has_more, limit, offset}.
    tests/chat_search.rs. Refused: CJK segmentation and
    search_ignore_accents, as the post search refuses them. The
    /chat/search page stays with 162
221. ~~`feat/chat-admin`~~ (done) M: chat's admin plugin page refused
    (its configuration nav), and incoming webhooks, their admin and the
    messages export had no routes. Now /admin/plugins/chat draws its
    settings with the nav (Settings, Incoming webhooks; plugins/mod.rs)
    and ChatAdminPluginActions' Create export in the header of every
    chat admin page (admin_plugins_view.rs takes a plugin's header
    actions); POST /chat/hooks/:key and /:key/slack
    (routes/chat_admin.rs, plugins/chat/incoming_webhooks.rs:
    validate_payload, debug_payload, Chat::SlackCompatibility's text and
    legacy attachments, the 10 a minute limit, the system user following
    the channel, Chat::CreateMessage as it with the webhook event); the
    six /admin/plugins/chat/hooks routes as JSON
    (AdminChatIndexSerializer, IncomingWebhookSerializer,
    AdminChatWebhookShowSerializer, create's key and validations, update,
    destroy) and as pages (plugins/chat/admin_view.rs: the list, new and
    edit forms; static/js/chat-admin.js saves, deletes and picks the
    emoji); POST /chat/admin/export/messages (Jobs::ExportCsvFile for
    chat_message and the entity_export log, a 204); the category editor's
    Chat tab (EditCategoryChat: the category's and subcategories' channels
    with the subcategories' toggle, or the empty state). Tests:
    tests/chat_admin.rs. Left: the ExportCsvFile job itself
    (Chat::MessagesExporter) with 200, drawing webhook messages with 162
    (plugins/chat/messages.rs refuses them); ChatChannelChooser and
    EmojiPicker's trigger are drawn as a native select and a plain
    button
222. ~~`feat/chat-new-message`~~ (done, as feat/message-composers) M:
    /chat/new-message answered 405 and the drawer's "Start a
    conversation", the sidebar's "Start DM or group chat" and the
    profile's and user card's Chat buttons did nothing (the demo
    rehearsal's step 15). Now the new message modal
    (static/js/chat-new-message.js: ChatModalNewMessage, DModal
    --quick-palette without its header, over MessageCreator): the search
    mode lists "New group chat" (chatGuardian.canUseGroupChat) and the
    member's channels as ChatablesLoader preloads them (from
    /chat/api/me/channels: followed public channels by slug, then DMs by
    activity, starred first, a one-person DM as its user), searches GET
    /chat/api/chatables as typed (INPUT_DELAY), sorts as sortChatables
    and draws ChatableUser, ChatableGroup and Channel with their unread
    indicators and disabled warnings; arrows, Enter, Shift+Enter or
    Shift+click (a new group with the item), hover and the cancel button
    as ListHandler and List do. A user opens the DM with them
    (upsertDmChannel: POST /chat/api/direct-message-channels with
    target_usernames[], an empty target_groups and name, upsert), a
    channel opens, a group or "New group chat" goes to the new group
    mode: its name, the members counter against
    chat_max_direct_message_users, MembersSelector's search without
    channels, chips with Backspace, arrow and Enter handling, the max
    members warning, Cancel back to the search, and Create group chat
    (createDmChannel). The channel opens through chat-drawer.js's
    transitionTo, in the drawer when it is preferred. Ways in: the DMs
    list's empty state (drawer and full page), the browse page's empty
    state, a link to /chat/new-message followed in the app (the
    sidebar's), Ctrl/Cmd+K outside a field but the chat composer, and the
    profile's and the user card's Chat buttons (ChatDirectMessageButton,
    the card closed). GET /chat/new-message (chat_pages.rs) goes as the
    route leaves a page load: to /chat, or to the channel_id or channel
    asked for (or /chat), a read. With recipients it serves the chat
    page with the route's empty outlet and writes nothing (it found or
    created the DM behind the GET at first, so a link, prefetch or
    crawler could create channels, fix/chat-new-message-post):
    chat-new-message.js does the route's upsertDmChannel, POST
    /chat/api/direct-message-channels with the recipients, and opens the
    DM, or leaves popupAjaxError's dialog over the page. Compared with
    Playwright against rs-ref-d as user1: the modal from the drawer, the
    search, the DM it opens, the new group (markup and screenshot
    identical), Ctrl+K and Escape, the page loads of /chat/new-message
    and the profile's Chat button, with the same requests. Recorded as
    chat/dm_create_from_new_message_modal (rs-ref-d); tests/plugin_pages.rs.
    Left, split into 222a.
    222a. `feat/chat-new-message-rest` S: the channel list options menu
    (ChatChannelListSidebarMenu: Start DM and Start group chat among its
    items; the drawer's and full-page lists draw its button without a
    menu), the add members mode (MODES.add_members, from a channel's
    members, refused with the info route), and in the modal's rows the
    presence dot (is-online), avatar flair, unread thread
    counts (threads, 99), emoji in channel titles; /chat/new-message with
    a message (#seedDraft, a draft seeded in the browser, refused)
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
    223b. ~~`feat/chat-drawer-resize`~~ (done) S: chat-drawer.js's
    resizer, chatResizableNode on `.chat-drawer-resizer`: a mouse or
    touch drag of the drawer's top left corner (top right in RTL) sets
    its width and height (and the left and top its static position
    ignores) while each stays over the drawer's computed min-width and
    min-height, and didResize writes the size to
    discourse_chat_drawer_size_* with ChatDrawerSize's 250 by 300
    minimums, then checks the composer offset; the drawer's style is
    computed when it is inserted and toggled (computeDrawerStyle), not
    on every route, so a drag lasts until then. The composer's live
    resize: composer.js fires composer:resize-started, div-resizing and
    resize-ended (bubbling from #reply-control) around its grippie's
    drag and arrow keys, and the open drawer turns the outlet's
    transitions off (clear-transitions) between start and end and
    checks its offset once a frame while it is dragged. Driven in
    headless Chromium over a fixture page (the drag, the minimums, the
    toggle's restyle, the composer events). Nothing refused.
    223c. ~~`feat/chat-drawer-routes`~~ (done) M: the drawer's other
    routes, drawn by plugins/chat/drawer.rs as ROUTES maps them (each
    with its model's channel in x-chat-drawer-channel, the routes
    visited in x-chat-drawer-routes, and while collapsed the navbar
    alone): chat.channel.info.settings and .members
    (ChatDrawerRoutesSettings and ChatDrawerRoutesMembers: the back
    button to the channel, ChatNavbarChannelTitle with the star, then
    ChannelInfoNav and the tab as full-page chat draws them, info.rs;
    members not sent to the settings for a channel that isn't open, as
    the drawer doesn't), reached from the channel title, the nav, and
    the browse cards' members links; chat.channel.pins
    (ChatDrawerRoutesChannelPins: "Pinned messages - <title>" with its
    emoji, then ChatPinnedMessagesList, the side panel's list now
    page.rs pinned_messages_list; refused while chat_pinned_messages is
    off, the API's 404); chat.channel.threads (ChatThreadList, through
    thread_view.rs channel_threads_page_in) and chat.channel.thread and
    .thread.near-message (ChatThread through thread_page_in, the
    drawer's navbar in place of ThreadHeader; the back button to the
    threads list or my threads after them (chatHistory.previousRoute),
    else the channel; a thread that isn't the channel's or doesn't show
    refused, as Ember pops the request's error over an empty route);
    chat.channel.near-message-with-thread, the channel around the
    message (the drawer has no side panel); chat.threads
    (ChatDrawerRoutesThreads: UserThreads and the footer), also
    ROUTES.chat's redirect for my_threads; chat.search
    (ChatDrawerRoutesSearch: ChatSearch's filter and sort menu,
    search_view.rs, and the footer). ChatFooter gained its threads item
    (includeThreads: chat_threads_enabled and the member's viewable
    threads, UnreadThreadsIndicator), in the drawer and the mobile view.
    GET /live/chat/search draws a page of ChatSearch's results (LIMIT
    20 over Chat::SearchMessage, x-chat-search-has-more): each entry
    with its accessible label, the channel's title unless scoped,
    ChatThreadHeading, and the message not interactive in the long date
    mode (dates.js draws it in the viewer's zone), its url the thread's
    for a thread's message. static/js/chat-search.js runs it: the query
    after INPUT_DELAY (chatSearchQuery's, kept across routes, the sort
    back to relevance as the component is new), the sort menu, the next
    page as the list's end shows, the alert when nothing is found, a
    result opening its message in the drawer. chat-drawer.js loads
    chat-thread.js, chat-channel-info.js and chat-search.js with
    chat.js and mounts and unmounts each with the route (a drawer
    thread's composer takes the focus when drawn, not on a page swap;
    its Escape goes back to the channel; its near-message URL isn't
    written to the address bar); chat-channel-info.js redraws the
    drawer's route after a change (chatDrawer.reload) and watches the
    drawer's members list; chat.js marks the drawer's pins read each
    time they are drawn. Fixed on the way: a pin's message waits in a
    template chat.js puts in the pin's link (its own links, the time
    above all, closed the link as the page was parsed, leaving it empty
    and the pin's layout broken, in the full page's side panel too). Back and forward and the back buttons follow
    state.history and previous as for the other routes. Tests in
    tests/chat_drawer_routes.rs; driven against the server in
    tests/browser/chat-drawer-routes.spec.js (each route from the
    drawer's UI, its content, a setting saved, a thread reply sent from
    the drawer, the back buttons, search sorting and opening a result,
    back to a thread's URL opening it in the drawer and forward;
    tests/browser/setup.sql turns on chat_threads_enabled, hidden from
    the admin). Not compared against the reference. Left, split into
    223e.
    223e. ~~`feat/chat-drawer-routes-rest`~~ (done) S: the drawer's pins
    list follows the channel's bus itself (ChatPinnedMessagesList's
    subscribe modifier; full page, the channel beside it still draws the
    page again): chat.js opens /chat/:id for a drawer list without its
    channel, an unpin filters its row out (the empty state once the last
    goes, its label in the list's data-label-empty), a pin of a message
    not shown loads the pins again (#loadPins: the drawer's route
    fetched for its list, the latest load winning, the rows unpinned in
    flight left out, the rows already shown keeping the unseen mark they
    were drawn with as isUnseen's snapshot does), and the pins are marked
    read when the route goes with the list (the destructor's
    #markPinsAsRead; not on a page swap, which keeps the drawer). The
    list's own unpin, full page and drawer: the row out at once, the
    focus on the unpin button now at its index, else the last, else the
    empty state (#focusAfterRemoval), the request after, the row back in
    timeline order if it fails (#restorePin). ChatSearch: DFilterInput's
    clear button (btn-small btn-transparent filter-input-clear-btn,
    "Clear filter") while there is a query, drawn by the server for the
    full page's q and by chat-search.js from a template, clearing the
    query to "" (the full page's address to `?q=`, as Ember prunes only
    the undefined default) and focusing the input; the container
    is-focused with the input; dTabToSibling on each result (Tab and
    Shift+Tab to the next or previous focusable sibling).
    DConditionalLoadingSpinner and DLoadMore are in the ref's frontend
    (vendor/discourse lacks only the copy, not the commit): the
    spinner's loading-container gains visible, data-loading="" and
    `<div class="spinner ">` while loading; DLoadMore's sentinel is
    `<div aria-hidden="true" class="load-more-sentinel">` (it was a
    load-more div), kept while there are more results rather than drawn
    again each page, observed with threshold 0 and the document as its
    root, its action debounced 100ms. The chat-thread-navbar-actions
    plugin outlet: plugin_api::Outlet::ChatThreadNavbarActions (thread
    id, the channel's JSON, context "full-page" or "drawer"), first in
    ChatThreadHeader's actions and before the drawer's buttons; no
    bundled plugin connects to it (voice's connector is unported with
    voice) and a WASM guest can't declare it. "chat.deleted_user": in
    neither core's nor chat's locale at the ref (Ember would draw
    "[<locale>.chat.deleted_user]"), and unreachable, Chat::MessageSerializer
    always sending a user (Chat::NullUser for a deleted one): refused
    (Unsupported, "a chat search result without its user"). Tests in
    tests/chat_drawer_routes.rs, tests/chat_search.rs and
    tests/plugin_api.rs (the outlet full page, in the drawer expanded and
    collapsed, gone with the plugin off); driven in
    tests/browser/chat-drawer-routes.spec.js (live pin and unpins to the
    empty state, read on leaving, the list's unpin and its focus, the
    clear button full page and in the drawer, Tab between results, the
    spinner and the sentinel over a second page). Not compared against
    the reference. Left: the unpin's a11y.announce
    ("chat.pinned_messages.message_unpinned"; core's A11yLiveRegions
    isn't ported), the full-page side panel list's read on leaving, and
    the list's dismiss and show footer with the pinned bar (219). Was:
    the drawer's pins list's own
    live pin and unpin (ChatPinnedMessagesList subscribes to the
    channel's bus; the drawer's list follows only a refresh), the
    search's clear button and dTabToSibling, DLoadMore's and
    DConditionalLoadingSpinner's markup (core's frontend isn't in the
    vendored ref: drawn as a sentinel div and loading-container), the
    "chat.deleted_user" label core's locale lacks, the thread pane's
    plugin outlet (chat-thread-navbar-actions), and ~~the full-page
    /chat/search route (162)~~ (done, feat/chat-media-and-search:
    ChatSearchRoute and ChatRoutesSearch in chat_pages::search and
    view.rs search_route, ChatSearch with enableQueryParams, its `q`
    param kept in the address by chat-search.js, a result opening its
    message full page, the sidebar link active, chat_search_enabled off
    going to /chat; tests/chat_search.rs and
    tests/browser/chat-media-search.spec.js)
    223d. ~~`fix/chat-drawer-history`~~ (done) S: the browser's back and
    forward run the chat route's beforeModel, activate and deactivate in
    chat-drawer.js on htmx:historyCacheMiss (the restore skips
    htmx:beforeRequest): a chat URL the drawer takes while it is
    preferred, from a page that isn't full-page chat, cancels the
    restore and opens in the drawer, the address bar left on the chat
    URL as Ember leaves it after aborting a URL transition (and
    router.currentURL, which storeAppURL reads, still the page shown);
    any other chat URL closes the drawer and keeps the forum URL left,
    and leaving full-page chat keeps its URL as the last chat URL. A
    forum page swapped in takes the focus from the drawer's composer.
    Driven in headless Chromium over a fixture app with htmx (back to
    the chat URL asks for the drawer's route only, forward restores the
    forum page, full page preferred loads full-page chat, the focus).
    Where the reference's focus lands after the route change (core's
    frontend isn't in the vendored ref) wasn't determined: the port
    blurs to the body.
224. ~~`feat/chat-pane`~~ (done) M: the channel pane's paging and
    jumps. The server draws pages of a channel's messages for the viewer
    (GET /live/chat/:channel_id/messages, Context::messages_page over
    ListChannelMessages: a page past or future from a message, the page
    around one, or the latest, with the meta's can_load_more_past and
    _future in x-chat-can-load-more-* headers, null for the direction
    not asked, as the API answers; a past page draws its first message
    of the page shown again, its reply line following the message now
    above it; each page's first is firstOfResults; the "last visit" line
    only when the page shown has none). chat.js's pane (ChatChannel,
    ChatMessagesLoader, ChatScrollableList, ChatPaneState): older pages
    at the top or scrolling up under 40% from it (not on iOS), newer at
    the bottom (the last message shown kept at the bottom), loadMore
    stopping only on false, fillPaneAttempt while the first message's
    top shows (under 200 messages), the "all loaded" line; the
    near-message url's target scrolled to the center and highlighted
    for two seconds and the url replaced with the channel's
    (ChatChannelNearMessage), else the last read message centered; a
    link to a message of the channel (the in-reply-to line) jumps in
    place when loaded (expanding a collapsed deleted run), else loads
    the page around it (highlightOrFetchMessage); the scroll to bottom
    arrow (hasPendingContentBelow: newer to load, over 250px away, or
    messages that came while away, which keep the view where it was)
    scrolls to the bottom or loads the latest page. A target that can't
    load draws the unloaded channel with ChatSkeleton and pops the 404's
    error (popupAjaxError) instead of refusing. The engine's /chat/
    channel/:id(/:title(/info...)) redirects (301 to /chat/c/..., the
    title "-" without one); /chat/message/:id stays a 404 on a page load
    as in Rails (only Ember's router has it). A channel without a slug
    takes its title slugified. A reaction without an emoji image draws
    nothing (it refused). The retention reminder's dismiss (POST
    /chat/dismiss-retention-reminder, the reminder gone). In the drawer
    a link to a message reloads its route around it (chat-drawer.js's
    transition), scrolled and highlighted the same way. Tests in
    tests/chat_pane.rs; chat.js driven in headless Chromium over a
    fixture channel (the target, paging both ways to the first message,
    the arrow both ways, jumps loaded and not, the 404's error). Left,
    split into 224a.
    224a. ~~`feat/chat-pane-rest`~~ (done) S: the rest of the channel
    pane. The channel draws its extras for chat.js as a template's data
    (Context::pane_extras: the notices', filter's and arrow's strings,
    the member's unread count in the channel, the mobile emoji label,
    chat_show_copy_button_on_codeblocks). Chat notices: the bus's notice
    (text, or mention_without_membership with its invite: POST
    .../invites with the user ids and, as Ember's notice.data.messageId
    is undefined, no message id; "Invitation(s) sent" for three seconds),
    cleared by their button (ChatNotice has no id, so clearing one clears
    them all) or when the channel is left. The navbar's filter button
    toggles ChatChannelFilter (GET /chat/api/search latest first after
    INPUT_DELAY, the position, previous and next wrapping round, no
    results, Esc and Done). The "+" menu's local dates (the insert date
    modal, its markup at the selection) and the mobile emoji button (the
    picker, the emoji at the selection), in chat-setup.js's order; GIFs
    stay out, core's GIF picker (GifsModal) isn't ported. The stop
    streaming button's refusal shows nothing, as Ember leaves it
    unhandled. Spoilers (spoiler-alert.js's, under spoiler_enabled),
    hashtag icons (hashtags.js's decorate) and code block buttons
    (codeblock-buttons.js's attach, under chat's own setting) on each
    message once, loaded and live; the deferred decorators now also run
    once the page has loaded, which highlight and local dates missed on
    a page load. The channels lists for no member (no channels, no
    browse button, no divider) and a member without chat options (the
    columns' defaults); the mobile list no longer draws the filter's
    empty state under the empty one. The mobile arrow's "N new messages"
    (another member's message past the last read counts, the read sets
    it from the page drawn again, the latest message read clears it).
    The date separator's click (fetchMessagesByDate): the day's first
    message highlighted in the center, or, when it was the first of the
    page that loaded it with older ones left, the page around the day's
    start (GET /live/chat/:id/messages?target_date, query_by_date; the
    meta has no target for a date, so the page shows from its bottom).
    The saved scroll position per channel (chatChannelScrollPositions:
    the bottom-most message in view as the scroll rests away from the
    bottom, forgotten at the bottom, a channel opened again without a
    target highlighted there at the end, chat-channel--saved-scroll-
    position). Browser attention (ChatPaneState): only an active reader
    (present in the last minute, focused, visible) marks read; a message
    at the live edge (10px) follows for an active reader, the member's
    own, or a visible unfocused pane that was at the edge, else keeps the
    place and counts pending; coming back reads what shows. Tests in
    tests/chat_pane_rest.rs; tests/browser/chat-pane-rest.spec.js on the
    real server (the notice and invite, the filter, the separator's jump,
    the saved position across htmx navigation, the decorators, stop
    streaming, local dates, the mobile emoji and count, a hidden tab by
    emulated page visibility). Left, 224b.
    224b. `feat/chat-pane-gifs` S: the "+" menu's GIFs (enable_gifs,
    GifsModal with buildGifPickHandler), once core's GIF picker is
    ported; the DM and starred lists' refusals without a member or chat
    options (unreachable past chat's gate, as the channels list's were)
225. `feat/chat-notify-client` S: desktop notifications for chat, the new
    message sound (ChatAudioManager), the title counter,
    ChatNewMessageAnnouncer under chat_announce_new_messages, the chat
    keyboard shortcuts (channel switching, composer formatting, Shift+Esc
    to mark all read) and their section in the shortcuts help. Split
    into the slices below.
    225a. ~~`feat/chat-notify-client`~~ (done) S: the keyboard shortcuts
    chat-keyboard-shortcuts.js adds that 222 and 223a didn't, in
    static/js/chat-drawer.js (on every page with chat): Alt+Up and
    Alt+Down switch to the previous or next channel in the sidebar's
    order, starred then channels then DMs, wrapping around
    (switchChannelUpOrDown), and with Shift among those with unread
    activity, the active one kept in its place
    (#getOrderedChannelsWithActivity), through the drawer's
    transitionTo; Ctrl/Cmd+B, I and E surround the chat composer's
    selection with `**`, `_` and a backtick, or insert composer.*_text
    selected (modifySelection's applySurround, each line of a
    multi-line selection on its own; the example texts on the
    composer's data-surround); Shift+Esc marks every channel read while
    full-page chat or the drawer is open (PUT /chat/api/channels/read).
    The channel order is read off the sidebar's chat links (their
    `channel-<id>` class and unread badge), so without the sidebar in
    the page (the mobile view) the switch does nothing. Tests in
    tests/chat_client.rs; the keys themselves are browser behavior,
    not compared against the reference.
    225b. `feat/chat-notify-client-rest` M: what waited on core's
    client. ~~Chat's desktop notifications and their sound~~ (done in
    `feat/chat-notify-client-rest`: the stream passes
    /chat/notification-alert on as a `chat-notification-alert` event
    to pages with chat, none in do not disturb; in
    static/js/chat-drawer.js ChatNotificationManager's onMessage, which
    claims an alert for the channel in sight and hands the others to
    core's onNotification, off mobile devices; the chat-audio
    initializer's handler on Discourse.desktopNotifications'
    registerDesktopNotificationHandler, playing the member's chat_sound
    for a chat_mention or chat_message alert unless in do not disturb,
    the indicator preference is never, only_mentions and it is no
    mention, or dm_and_mentions and it is neither; ChatAudioManager's
    play with its incoming and mention sequences, its 3s throttle and
    a suspended context's refusal, which browsers give until the page
    is interacted with; chat-alert-dedup's claims in localStorage. No
    service worker answers isPrimaryTab, so every tab is asked and the
    claim decides, as Ember's without one) and ~~the title counter~~
    (done in `feat/chat-notify-client-rest`: addDocumentTitleCounter
    and notificationsChanged, with forced, on Discourse.documentTitle;
    getDocumentTitleCount's allChannelUrgentCount in the notifications
    mode, whatever the indicator preference, read off
    `#chat-tracking-state`, the totals the header icon is drawn from,
    kept current by the stream; the contextual mode's pending count
    never shows in Ember either, the sum being the notifications
    mode's). Tests: browser chat-notify-client (5), sessions
    chat_alerts_and_totals_reach_the_page. Left: registerChatSound (a
    theme's own sounds), the help modal's chat section (the keyboard
    shortcuts help is refused, static/js/sidebar.js:144), and
    Ctrl/Cmd+L's insert link modal; and in chat.js the new message
    sound (ChatNewMessageAnnouncer under chat_new_message_sound for an
    active reader, with chatAlertPlayed) and the announcement of new
    messages under chat_announce_new_messages (a11y.announce's live
    region)
226. ~~`feat/chat-site-integration`~~ (done) M: chat's keys on core.
    /site.json's markdown_additional_options.chat (MARKDOWN_FEATURES,
    MARKDOWN_IT_RULES and the hashtag contexts, always there as Rails
    sets it in after_initialize), hashtag_configurations from core's and
    chat's type priorities (chat-composer, the channel type in
    topic-composer) and hashtag_icons from the enabled data sources
    (the channel source under enable_public_channels), both only while
    chat is on (src/plugins/chat/site_integration.rs, src/site.rs). GET
    /notifications/totals (UserNotificationTotalSerializer: unread
    notifications less new messages, unread_personal_messages, staff's
    unseen_reviewables, report_totals, group_inboxes) with chat's
    chat_notifications (unreads_total: mentions of followed open
    channels plus followed DMs' unread). chat_quoted notifications from
    a new post quoting messages (PostNotificationHandler after
    after_save_post: `[chat quote="name;...` authors less the notified
    and the author, not for whispers or messages; chat_quoted joins the
    types a reply notification suppresses), and chat's post stripper
    (`.chat-transcript .mention`) in the analysis of post creation,
    revision, the queue and post alerts. The chat_summary unsubscriber
    (its frequency select in the unsubscribe_options outlet and its
    save), registered only while chat is on. The chat:create_message
    API key scope (already recorded) listed, shown and permitting only
    while chat is on, as solved's while solved is (a filtered
    register). Chat's keys dropped from every parity ignore list
    (site.json, users#show, current.json, user_option), the goldens
    matching. Tests in tests/chat_site.rs. Not ported: the four stats
    nothing reads (only Plugin::Instance.stats, which nothing in the
    reference calls); Chat::Seeder's work is in the clean seed
    snapshot (both channels, needs_chat_seeded false). Left, split into
    226a.
    226a. `feat/chat-site-integration-rest` S: what waits on core: the
    remap_hashtag job with chat's MessageHashtagStore (core's
    HashtagRemapper and its stores are unported, the job has no
    handler; category_writes.rs:2461; tag_writes.rs:274),
    secure_uploads' change handler and the chat-composer public upload
    type (secure_uploads can't be turned on, site_setting_update.rs
    CUSTOM_VALIDATIONS, and uploads refuse it, uploads.rs), and
    Chat::Seeder at start for a database still needing it (with
    category channel creation)
227. ~~`feat/chat-pages-rest`~~ (done) S: the small refusals no item
    names. A DM participant's repeated custom field is an array of its
    rows in id order (FieldDescriptor#append_field). A DM whose other
    participants are all deleted is titled by Chat::NullUser ("deleted"),
    as Rails maps them before its never-reached "all users deleted"
    branch, so `dm_title` returns a string and the list, the channel
    page, bookmarks, notifications, the summary email and channel edits
    stop refusing it. What Rails raises on is now a plain 500 rather
    than a refusal: a DM channel without its direct_message_channels row
    (the nil chatable), a title with no viewer and no other participant
    (`acting_user.username` on nil), a mention job's membership of a
    deleted user (`membership.user.id`), the invited-users notice on a
    site without its system user (`Discourse.system_user.guardian`, the
    transaction rolled back). A web hook's message and a DM target
    vanishing mid-request (read in the same request; Rails holds the
    records) are internal errors. Tests in tests/chat_pages_rest.rs.
228. ~~`feat/poll-ui`~~ (done, 228a-c) L: poll has no client: nothing
    in static/js or the templates draws a poll, so a post shows its
    cooked list with a "0 voters" stub. Split into the slices below.
    Export results waits for data explorer (202).
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
    228c. ~~`feat/poll-ui-breakdown`~~ (done) S: the actions dropdown's
    close and open and its breakdown, where 228a and 228b refused them
    (static/js/poll.js). toggleStatus: nothing for an automatically
    closed poll, else Discourse.dialog.yesNoConfirm with
    poll.close.confirm or poll.open.confirm, PUT /polls/toggle_status,
    the results shown while an on_close or always poll is closed (kept as
    the poll object's showResultsToggle), the widget drawn again from the
    server, which has an on_close poll's votes once it is closed as
    Ember's route refresh brings them, popupAjaxError else
    poll.error_while_toggling_status. The breakdown is PollBreakdownModal:
    DModal.poll-breakdown.has-tabs with the Percentage and Count tabs in
    its header (static/js/modal.js takes a headerBelowTitle; the has-tabs
    rules of modal-overrides.scss in static/css/modal.css), the sidebar
    (the poll's title HTML, else the topic's title; the votes line drawn
    by the server; a PollBreakdownOption per option with getColors'
    swatch and I18n.toNumber's share, or the count, its HTML run through
    the cooked decorators), the group ComboBox (poll-builder.js's, loaded
    with it as Discourse.pollBuilder.comboBox) over
    poll_groupable_user_fields labelled as groupableUserFields classifies
    them, fetching /polls/grouped_poll_results.json again on a change,
    and a PollBreakdownChart per group (its label, a Chart.js pie over the
    options with votes, aspectRatio 1.1, chartjs-plugin-datalabels'
    share or count, onHover highlighting the slice's option). The
    widget's state carries the modal's model whenever there are groupable
    fields (plugins/poll/view.rs, breakdown_model; Widget.topic_title
    from the topic page and /live/poll, none for a quoted post's poll),
    so an open modal follows the poll as Ember's tracked poll object
    does. chartjs-plugin-datalabels 2.2.0 is vendored as
    frontend/discourse/package.json pins it (static/vendor/README) and
    loaded after Chart.js as loadChartJSDatalabels does. Measured with
    playwright against rs-ref-a on a three option poll with four voters
    in two cities and none: the modal's normalized DOM equal in both tabs
    but for attribute order and the ComboBox's class order
    (poll-builder.js's, as in 229). Ember throws drawing a highlighted
    PollBreakdownOption (Chart.helpers.getHoverColor, and chart.js/auto's
    Chart has no helpers), after the row's background and before its
    swatch, and renders nothing more: the port draws the row's highlight,
    keeps the swatch's color and leaves the pies as they are, as Ember
    does, but goes on rendering (the highlight follows the pointer, the
    tabs and the group chooser keep working). A failed grouped fetch
    shows its error and keeps the pies (Ember goes on to read
    grouped_results of undefined and throws). Tests in
    tests/plugin_pages.rs, plugins::poll::view and
    tests/browser/poll.spec.js. Left: export (202).
229. ~~`feat/poll-builder`~~ (done, as feat/poll-builder-widgets) M: the
    markdown builder shipped in 196a: types, title, options and
    validation, min/max/step, results visibility, public default, chart
    type, groups, automatic close, dynamic options and BBCode insertion.
    Done here, in static/js/poll-builder.js: the advanced fields are
    Discourse's shared widgets instead of native controls (the port had
    no shared one to reuse; the choosers elsewhere are page-local).
    GroupChooser (MultiSelect over site.groups but everyone): the filter,
    the chosen groups as removable choices, rows without them up to
    displayedGroupsLimit with FilterForMore's tip, the results count and
    no content, Enter and Backspace, open across a choice (closeOnChange
    false). DDateTimeInput, clearable, in the browser's timezone:
    DDateInput's native date input (Pikaday and moment loaded where there
    is none), DTimeInput's ComboBox of quarter hours with allowAny's
    typed time read as onChangeTime reads it ("h:mm am" or "HH:MM"), a
    time without a date today's, a new date at the time there was. The
    results are a ComboBox too. static/js/modal.js gives Escape to an
    open select-kit before the modal, as select-kit's keydown does. The
    options menu's item follows add-poll-ui-builder's condition: a member
    who may not create polls gets it when the composer's topic (the one
    replied in, or edited) is pm_with_non_human_user. Tests:
    composer_actions.rs (the item's condition per viewer and setting) and
    browser/composer-actions.spec.js (the three widgets' choices in the
    markdown they build; the item in a PM with discobot, not with a
    member or in a new topic). The rich editor's node view waits on 196.
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
232. ~~`feat/reactions-ui`~~ (done, the rest split to 232a) M: the
    allow-any-emoji picker button, infinite scroll on the Reactions tab
    and the reactions received page past the first 20, reactions
    received's acting_username and include_likes. The picker's data
    (plugins/reactions/view.rs) carries EmojiPicker's DMenu trigger after
    the reactions when discourse_reactions_allow_any_emoji is on, counted
    as a cell by optimalColsCount, and the topic page loads
    static/js/emoji-picker.js with its template then.
    discourse-reactions.js opens it with the discourse-reactions context;
    while it is open the picker neither collapses on pointer out nor on a
    click outside (preventCollapse, disableClickOutside), and a picked
    emoji is toggled as onSelectEmoji does. The lists: each item carries
    its reaction user's id and reaction, the sentinel the route's
    acting_username and include_likes (as findReactions sends them: when
    not blank), the main reaction and canLoadMore (a full page);
    discourse-reactions.js, loaded on the user pages that draw one, is
    UserActivityReactions#loadMore under PostList's DLoadMore (100ms
    debounce, #updateBeforeIds with likes and other reactions apart under
    include_likes), asking the page again as an XHR, which alone reads
    the cursors (routes/reactions.rs list_page_params); a page that brings
    nothing ends the list and shows the footer a full first page
    deferred. Reactions received reads acting_username and include_likes
    on a page load, so a notification's link lists what it names.
    reactions_received's blank before_reaction_user_id now stands as
    Rails leaves it (id < 0) unless before_post_id replaces it. The admin
    emoji list's locked like reaction was already on main
    (admin/site_settings_view.rs emoji_list). Measured by
    parity/writes/reactions/received_paged_with_likes (both cursors,
    acting_username, the blank cursor and before_post_id), tests in
    tests/plugin_pages.rs, browser specs tests/browser/post-menu.spec.js
    (the emoji picker) and tests/browser/reactions-lists.spec.js.
    232a. `feat/reactions-ui-rest` M: the `l` shortcut (after core's
    keyboard post selection, none in static/js), touch long press,
    vibration and the toggle animations (static/js/discourse-reactions.js:8),
    the mobile users modal and stylesheet, the user nav links once core's
    user nav panel exists
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
240. `feat/lazy-videos-ui` S: ~~discourse-lazy-videos' client:
    LazyVideo's thumbnail button, title bar, dominant color, provider
    icon and play_video label, LazyIframe's YouTube (list, start time),
    Vimeo and TikTok embeds, the per-provider settings, lazy-videos.scss
    (src/assets.rs)~~ (done in `feat/lazy-videos-ui`:
    static/js/lazy-videos.js, a decorateCooked decorator on the stream's
    posts, loaded while lazy_videos_enabled; the provider settings and
    the label come in #cooked-config; css/lazy-videos.css is in
    discourse.css whatever the setting, as the other plugins' are; no
    preventCloak, the port cloaks no posts); the email reduce_cooked
    turning each video into a paragraph link
    (email/notification.rs:913-945)
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
- [x] Fresh install: `discourse-rs setup` loads the vendored schema and
      what db:seed creates (seed/clean_install.sql, `scripts/snapshot-dv
      --clean`) into an empty database in one transaction, with the
      site-dependent and random parts of db:seed made for the site
      (VAPID keys and push_api_secret_key, also made at start when
      missing, notification_email, the system users' passwords, the login
      hint); `discourse-rs admin create` (rake admin:create); the NixOS
      module runs both, the admin's password from a systemd credential.
      GlobalSetting's generated secret_key_base is kept in the database
      as Rails keeps it in Redis. The `discourse-rs-fresh-site` stopgap is
      gone
- [x] The finish installation page (FinishInstallationController: index,
      the front page while has_login_hint, register with the developer
      emails, confirm-email, resend-email), make_developer_admin at login
      and clear_global_notice_if_needed. Not ported: its Discourse ID
      setup (DISCOURSE_SKIP_EMAIL_SETUP)
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
      undoing flags, deleting, editing,
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

- [x] Runtime: bundled plugins are Rust in src/plugins/ on lifecycle
      phases. Compile-time plugins use plugin_api::Plugin; third-party
      guests run in the merged Wasmtime host (docs/plugin-wasm.md).
- [ ] The bundled plugins: 31 started (crate::plugins::STARTED), with
      parity gaps still tracked per plugin. The original parity audit
      (210, docs/plugin-audits.md, 2026-10-17), extended as ports land,
      lists every gap of chat, poll, reactions, solved, topic voting,
      presence, local dates, lazy videos, checklist, details, footnote,
      spoiler, LTI, OAuth2 basic, OpenID Connect and Apple against the
      reference plugin; the uncovered ones are 211-249, the rest earlier
      items (chat 160-162, solved 163, voting and reactions core's 143,
      150, 188i4, 195, 197, the settings 164). Templates and narrative-bot
      now have ports; their remaining gaps and other plugins stay in 202
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
