# Request-spec coverage

Discourse's request specs (`spec/requests/*_controller_spec.rb` at the
vendored commit, `vendor/discourse/DISCOURSE_REF`) used as the checklist
for every controller action the port routes (src/routes/mod.rs). One row
per `it` (near-identical ones merged, marked (xN)); Line is the `it`'s
line in the spec.

Built against `bf55a44c`, refreshed against `9daeae2a745b` as a delta:
line numbers carried through the diff of each cited spec, rows whose `it`
changed rewritten, `it`s added since inside an inventoried action's
describe block given rows of their own, and rows whose cited `Unsupported`
(or UNPORTED_CREATE_PARAMS entry, or user updater REFUSED entry) has since
been ported moved to recordable ("no case yet: ported in <commit>") unless
a case covers them. Controllers and actions not listed here are counted
in parity/GAP.md (`scripts/parity-gap`), not inventoried. The Summary
table is computed from the rows.

Classes:

- **covered**: a write case (parity/writes/<area>/<name>.case.json) or a golden
  (`golden:` a parity/cases line) exercises the behaviour. `(new, ...)`
  marks a case added from this inventory, with its replay result.
- **recordable**: expressible as a write case with the harness as it is
  (seed data, `settings`, `setup` SQL rows, fixture files); not added yet.
  The note suggests a case name.
- **needs-harness**: needs what the harness can't do: freeze_time, stubs
  and mocks, plugin or modifier registration, several users in one case,
  rate limits, redirect Location headers, MessageBus or log inspection,
  multisite, heavy fabrication.
- **unported**: Rails behaviour the port doesn't have yet: it refuses it
  (an `Unsupported(...)` marker, cited) or doesn't serve that route, param
  or format (RSS, HTML-only). Nothing here is left out on purpose; the
  goal is all of it.

`/filter` (ListController#filter) is not routed yet and not inventoried.

## Summary

Scenarios per action (an `it` marked (xN) counts N).

| Action | covered | recordable | needs-harness | unported |
|---|---|---|---|---|
| PostsController#create | 12 | 45 | 14 | 25 |
| PostsController#update | 5 | 18 | 3 | 12 |
| PostsController#destroy | 3 | 2 | 0 | 5 |
| PostsController#recover | 1 | 2 | 0 | 1 |
| PostsController#revisions (and #latest_revision) | 2 | 14 | 6 | 9 |
| PostsController#markdown_num | 2 | 3 | 0 | 6 |
| TopicsController#status | 14 | 4 | 0 | 11 |
| TopicsController#destroy | 5 | 1 | 0 | 6 |
| TopicsController#timings | 5 | 1 | 1 | 0 |
| TopicsController#show | 107 | 45 | 40 | 22 |
| ListController#private_messages* | 5 | 13 | 0 | 6 |
| PostActionsController#destroy | 5 | 2 | 0 | 0 |
| PostActionsController#create | 9 | 11 | 0 | 0 |
| DraftsController#index | 2 | 4 | 0 | 2 |
| DraftsController#show | 1 | 0 | 0 | 0 |
| DraftsController#create | 10 | 7 | 2 | 0 |
| DraftsController#destroy | 1 | 2 | 0 | 4 |
| BookmarksController#create | 4 | 1 | 1 | 0 |
| BookmarksController#update | 1 | 0 | 0 | 0 |
| BookmarksController#toggle_pin | 1 | 0 | 0 | 0 |
| BookmarksController#destroy | 3 | 1 | 0 | 0 |
| NotificationsController#index | 19 | 8 | 2 | 5 |
| NotificationsController#mark_read | 7 | 0 | 0 | 0 |
| UsersController#bookmarks | 10 | 1 | 1 | 6 |
| UsersController#user_menu_bookmarks | 4 | 5 | 1 | 0 |
| UploadsController#create | 6 | 12 | 3 | 6 |
| UploadsController and Admin::BackupsController direct uploads (ExternalUploadHelpers) | 26 | 20 | 16 | 2 |
| ReviewablesController#index | 4 | 15 | 4 | 5 |
| ReviewablesController#perform | 5 | 11 | 1 | 20 |
| Admin::SiteSettingsController#update | 18 | 5 | 2 | 13 |
| Admin::Config::SiteSettingsController#index | 6 | 1 | 0 | 0 |
| Admin::Config::LogoController | 2 | 1 | 0 | 8 |
| Admin::Config::AboutController | 10 | 1 | 0 | 0 |
| Admin::Config::CategoryManagementController#categories | 4 | 1 | 0 | 0 |
| Admin::Config::UpcomingChangesController | 19 | 0 | 4 | 0 |
| Admin::SearchController | 2 | 0 | 0 | 0 |
| Admin::DashboardController#new_features | 10 | 0 | 2 | 0 |
| Admin::McpController | 13 | 0 | 0 | 0 |
| Admin::McpAuthorizationsController | 4 | 0 | 0 | 0 |
| Admin::McpActivityController | 7 | 0 | 0 | 0 |
| Admin::McpClientsController | 3 | 0 | 1 | 0 |
| Admin::Config::WelcomeBannerController#themes_with_setting | 5 | 2 | 0 | 0 |
| Admin::Config::FontsController | 4 | 0 | 0 | 0 |
| Admin::UsersController#suspend | 6 | 11 | 1 | 0 |
| Admin::UsersController#unsuspend | 1 | 2 | 1 | 0 |
| Admin::UsersController#silence | 4 | 11 | 0 | 0 |
| Admin::UsersController#unsilence | 1 | 5 | 0 | 0 |
| Admin::EmailController#handle_mail | 3 | 4 | 0 | 2 |
| Admin::EmailTemplatesController#update | 9 | 2 | 0 | 0 |
| Admin::EmailTemplatesController#revert | 4 | 2 | 0 | 0 |
| Admin::RobotsTxtController#show | 3 | 1 | 0 | 0 |
| Admin::RobotsTxtController#update | 4 | 1 | 0 | 0 |
| Admin::RobotsTxtController#reset | 3 | 1 | 0 | 0 |
| UsersController#create | 12 | 23 | 18 | 39 |
| UsersController#perform_account_activation | 4 | 6 | 2 | 2 |
| UsersController#email_login | 4 | 1 | 3 | 0 |
| UsersController#password_reset_update (PUT /u/password-reset/:token) | 5 | 0 | 5 | 12 |
| UsersController#update (PUT /u/:username) | 5 | 39 | 5 | 4 |
| UsersController#show (GET /u/:username) | 3 | 4 | 13 | 9 |
| UsersController#summary | 1 | 4 | 6 | 0 |
| UserActionsController#index | 4 | 14 | 2 | 0 |
| SessionController#create (POST /session) | 16 | 23 | 11 | 3 |
| SessionController#destroy (DELETE /session/:username) | 1 | 8 | 3 | 0 |
| SessionController#forgot_password | 3 | 17 | 5 | 0 |
| SessionController#redeem_password_reset_code | 2 | 3 | 6 | 1 |
| SessionController#current | 2 | 1 | 2 | 0 |
| SessionController#csrf, #get_honeypot_value | 0 | 0 | 0 | 0 |
| StaticController#enter (POST /login) | 0 | 1 | 20 | 0 |
| ForumsController#status | 2 | 0 | 4 | 0 |
| ListController#latest (and #index generics) | 8 | 26 | 3 | 20 |
| ListController#top (+ /top/:period redirect) | 3 | 2 | 2 | 5 |
| ListController#hot | 0 | 0 | 0 | 2 |
| ListController#category_default / #category_latest / #category_none_* (incl. /l/<filter>, set_category) | 7 | 27 | 11 | 12 |
| ListController user lists (#unread #new #unseen #read #posted #bookmarks) | 2 | 4 | 0 | 2 |
| CategoriesController#index | 4 | 10 | 5 | 12 |
| SearchController#query | 3 | 15 | 8 | 5 |
| SearchController#show | 8 | 9 | 5 | 4 |
| TagsController#index | 22 | 0 | 1 | 9 |
| TagsController#show (+ show in category) | 20 | 1 | 0 | 9 |
| SiteController#site | 0 | 1 | 0 | 0 |
| SiteController#basic_info | 2 | 0 | 0 | 0 |
| RobotsTxtController#builder | 1 | 1 | 0 | 0 |
| RobotsTxtController#index | 4 | 8 | 2 | 0 |
| SitemapController | 6 | 4 | 2 | 0 |
| UserAvatarsController#show_proxy_letter | 0 | 1 | 1 | 0 |
| StylesheetsController (color definitions) | 0 | 0 | 4 | 10 |
| HashtagsController#search | 4 | 0 | 0 | 0 |
| UsersController#search_users | 37 | 1 | 3 | 0 |
| PresenceController#update | 2 | 1 | 1 | 1 |
| PresenceController#get | 2 | 0 | 1 | 0 |
| Quotes (QuotedPost, PostAlerter, CookedPostProcessor) | 12 | 21 | 1 | 0 |
| PostAlerter (links, mentions, the job's options) | 36 | 12 | 2 | 13 |
| **All (92 sections)** | **671** | **587** | **263** | **350** |

## Cases added from this inventory

100 cases, recorded on the reference and replayed. 74 matched as added:

new_topic_restricted_category, new_topic_no_category,
pm_no_recipients, pm_on_existing_topic, reply_duplicate,
reply_to_archived_topic, edit_too_late, edit_archived_topic,
edit_locked_post, edit_wiki, post_delete_first_post,
post_delete_self_disabled, post_delete_allowed_group,
post_recover_self_disabled, revision_history_private, raw_hidden_post,
topic_status_bad_params, topic_close_enabled_variants,
topic_close_already_closed, topic_unpin_globally_tl4,
topic_relist_with_until, topic_status_tl4_forbidden, topic_delete_own_old,
topic_delete_own_fresh, topic_delete_allowed_group,
topic_delete_pm_outsider, timings_restricted_topic, timings_pm_outsider,
timings_msecs_cap, unlike_after_window, like_pm_not_participant,
flag_hidden_post, draft_new_topic_and_pm, draft_too_long, draft_too_many,
draft_stale_sequence_no_draft, draft_edit_conflict_hidden_post,
bookmark_pm_not_participant, bookmark_user_auto_delete_default,
bookmark_edit_other, notifications_recent_bump, notifications_read_other,
notifications_filtered_badge_and_topic, bookmarks_list_hidden_posts,
upload_too_large, upload_staff_extension, review_unknown_action,
review_already_reviewed, setting_enum, setting_integer_delimited,
user_suspend_self, user_suspend_as_moderator,
user_unsuspend_staff_as_moderator, user_silence_with_message,
user_silence_admin_as_moderator, incoming_deprecated_email_param,
signup_reserved_username, signup_ignores_protected_fields,
signup_logged_in, activate_account_unknown_token, email_login_disabled,
password_reset_associated_accounts, password_reset_code_reuse,
session_login_inactive, session_login_not_approved,
users_update_ignores_protected_fields, users_update_names_disabled,
login_required_lists_anonymous, new_user_spam_host, edit_links_require_trust,
show_deleted_reply_staff, show_deleted_topic_staff, show_deleted_moderator,
show_deleted_topic_detailed_404.

The 26 that differed when they were added have since been fixed, each on
its own branch (`git log --merges --grep fix/`), so all 100 match.

## PostsController#create

`spec/requests/posts_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 1635 | action requires login | covered | reply_anonymous |
| 1642 | #create > no quadratic cooking of 2000 unclosed [quote] tags (< 2s) | needs-harness | Benchmark timing assertion |
| 1660 | regular user publishing a global banner (archetype banner) | unported | Unsupported("archetypes other than private messages") |
| 1691 | #create > unregistered archetype ("unknown") creates a regular topic | unported | Unsupported("archetypes other than private messages") in routes/posts.rs |
| 1707 | with api > memoizes duplicate requests (wpid) | unported | API post memoizer not ported (README) |
| 1742 | with api > valid JSON when enqueued | recordable | no case yet: ported in 81ec0943 (was: Unsupported("the review queue (approve_unless_allowed_groups)")) |
| 1761 | with api > import_mode suppresses notifications | recordable | reply_api_key_import_mode (master key via setup SQL as reply_api_key, run_jobs post_alert; port ignores import_mode, expect a diff) |
| 1813 | with api > external_id | unported | external_id in UNPORTED_CREATE_PARAMS |
| 1833 | with api > email PM recipient with PMs disabled | recordable | no case yet: ported in 89b05bce (was: Unsupported("messages to email addresses")) |
| 1861, 1886, 1912 | with api > email PM recipient ignoring the sender / allowlisting others / muting the sender, 422 (x3) | recordable | pm_to_email_recipient_ignoring, pm_to_email_recipient_allowlist, pm_to_email_recipient_muted (api_keys setup row for a TL4 sender, enable_staged_users, ignored_users / allowed_pm_users + user_options.enable_allowed_pm_users / muted_users rows, target_recipients = recipient email). posting/create.rs add_emails only checks allow_private_messages, suspension and silence, expect diffs |
| 1940, 1959, 1980 | with api > whispers for regular users / staff bool / staff string (x3) | recordable | no case yet: ported in 4a27c9b4 (was: whisper in UNPORTED_CREATE_PARAMS) |
| 2000 | with api > does not advance draft | recordable | new_topic_api_key_keeps_draft (setup: api key 9101 + drafts row for new_topic) |
| 2018 | with api > category does not exist ("invalid") -> 400 | recordable | new_topic_invalid_category (same path via session) |
| 2040 | with api > invalid embed_url | unported | embed_url in UNPORTED_CREATE_PARAMS |
| 2058, 2078 | with api > unlist_topic admin / non-admin (x2) | recordable | no case yet: ported in 4a27c9b4 (was: unlist_topic in UNPORTED_CREATE_PARAMS) |
| 2106 | logged in > scalar tags ("macos") rejected with 400 invalid_params before queuing | unported | Unsupported("tags as a single value") in tags/tagging.rs parse_param |
| 2123 | logged in > queues a topic with blank tags, admin approves it, no tags | needs-harness | user then admin in one test; split it records: enqueue_topic_blank_tags (approve_post_count 1, tags "") and queued_approve_post_blank_tags (setup ReviewableQueuedPost payload "tags": "", admin approve_post) |
| 2155-2264 | logged in > fast typing > queue / no silence / first topic / closed / too long (x6) | recordable | no case yet: ported in 81ec0943 (was: Unsupported("the review queue (fast typers)")) |
| 2276 | category-Y reviewers vs queued reply in X | needs-harness | no case yet: ported in 81ec0943 (was: Unsupported("category posting review modes"); also multi-user) |
| 2304 | auto_silence_first_post_regex | unported | Unsupported("auto_silence_first_post_regex") |
| 2325 | silence watched words | unported | Unsupported("watched words") |
| 2348, 2383 | message to a group (x2) | recordable | no case yet: ported in 0cbb8fc4 (was: Unsupported("messages to groups")) |
| 2408 | nested_post param | unported | nested_post in UNPORTED_CREATE_PARAMS |
| 2423 | protects against dupes | covered | reply_duplicate (new, matches) |
| 2440 | cannot create in a disallowed category -> 403 | covered | new_topic_restricted_category (new, matches) |
| 2454-2541 | tags restricted / disabled / no permission / by name / enabled / over limit (x6) | recordable | no case yet: ported in 569ff98c (was: tags in UNPORTED_CREATE_PARAMS) |
| 2575, 2587 | content localization tags (x2) | recordable | no case yet: ported in 569ff98c (was: tags in UNPORTED_CREATE_PARAMS) |
| 2600 | iframe with encoded userinfo not persisted | recordable | new_topic_iframe_not_allowlisted |
| 2619, 2648, 2680 | oEmbed provider HTML stripped (x3) | needs-harness | stub_request + Jobs.run_immediately (also Unsupported oneboxes) |
| 2709 | logged in > iframe URL kept inside the link href of notification emails | needs-harness | renders UserNotifications.user_replied directly; the harness records the user_email job, not the email HTML. An address with quotes may also hit Unsupported("iframes in emails with an unusual address") (email/styles.rs) |
| 2739 | creates topic and post with right attributes | covered | new_topic |
| 2759 | regular users replying to whispers | recordable | no case yet: ported in 4a27c9b4 (was: whisper param; Unsupported("replies to whispers")) |
| 2804 | posts_controller_create_user modifier | needs-harness | plugin modifier registration |
| 2818 | topic_custom_fields, none permitted -> 400 | unported | topic_custom_fields in UNPORTED_CREATE_PARAMS |
| 2844-2895 | permitted custom fields / staff-only / meta_data (x4) | needs-harness | plugin register_editable_topic_custom_field |
| 2915 | uncategorized topic (category "") | recordable | new_topic_uncategorized (settings allow_uncategorized_topics true, uncategorized_category_id 1; seed has -1) |
| 2935 | reply to a PM post with image_sizes | unported | image_sizes in UNPORTED_CREATE_PARAMS (plain PM reply is pm_reply) |
| 2964 | creates a private post (several recipients, odd usernames) | covered | pm_new_two_recipients (Turkish-dotted username edge not covered) |
| 2993 | target_recipients empty -> no_user_selected | covered | pm_no_recipients (new, matches) |
| 3012 | PM archetype with topic_id -> create_pm_on_existing_topic | covered | pm_on_existing_topic (new, matches) |
| 3030 | with errors > does not succeed (raw "test") | covered | reply_too_short |
| 3036 | spam host threshold for TL0 | covered | new_user_spam_host (new, matches; a user with earlier linked posts, whose posts Rails then flags as spam, is refused: flag_linked_posts_as_spam) |
| 3053 | allow_uncategorized_topics false > no category | covered | new_topic_no_category (new, matches) |
| 3065 | allow_uncategorized_topics false > as staff | recordable | new_topic_no_category_staff (admin) |
| 3080, 3101 | slow mode with auto_track false (x2) | unported | auto_track in UNPORTED_CREATE_PARAMS; Unsupported("slow mode") |
| 3119 | enable_user_status off > no mentioned_users | covered | reply_with_markdown (setting defaults to false) |
| 3135, 3159, 3166 | enable_user_status > mentioned_users with status / empty / unknown user (x3) | recordable | mention_user_status (setting + user_statuses row via setup SQL; port has no mentioned_users, expect a diff) |
| 3179, 3197 | unlist_topic staff / non-staff (x2) | recordable | no case yet: ported in 4a27c9b4 (was: unlist_topic in UNPORTED_CREATE_PARAMS) |
| 3214 | mentionable_groups modifier | needs-harness | plugin modifier (not a request either) |
| 3231-3275 | shared drafts (x4) | unported | shared_draft in UNPORTED_CREATE_PARAMS |
| 3302-3354 | is_warning staff / string / false / normal user (x4) | unported | is_warning in UNPORTED_CREATE_PARAMS |
| 3377, 3392, 3438 | no_bump skip / string / regular user 400 (x7 across contexts) | recordable | reply_no_bump (admin, moderator or TL4 user, no_bump true and "true", topic bumped_at unchanged), reply_no_bump_member (400 invalid_parameters). Was unported: no_bump is no longer in UNPORTED_CREATE_PARAMS, routes/posts.rs checks can_skip_bump |
| 3407 | creates the post and bumps the topic (admin, mod, TL4) | covered | reply (topic bumped_at compared) |
| 3452-3492 | featured links (x4) | unported | featured_link in UNPORTED_CREATE_PARAMS |
| 3508 | #create > unrecognized image size suffix cooks in bounded time (< 2s) | needs-harness | Process.clock_gettime timing assertion |

## PostsController#update

`spec/requests/posts_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 716 | action requires login | recordable | edit_anonymous |
| 735 | TL1 wiki edit does not publish hidden profile onebox | needs-harness | stubs CookedPostProcessor#get_size, run_immediately, sign_out + GET (two identities) |
| 791 | wiki editor changing another's topic title -> 403, body edit ok | recordable | no case yet: ported in 309d22ff (was: Unsupported("posts#update title, image sizes and bypass_bump"); the wiki body edit half is proposed as edit_wiki) |
| 838 | TL1 wiki editor moving restricted topic to public category | recordable | no case yet: ported in 309d22ff (was: Unsupported("posts#update fields beyond raw and edit_reason") (category_id)) |
| 866 | regular > TL0/TL1 after edit time limit -> 422 too_late_to_edit | covered | edit_too_late (new, matches) |
| 880 | regular > TL2 after edit time limit | recordable | edit_too_late_tl2 (user2 post 37, tl2_post_edit_time_limit 8) |
| 894 | regular > passes image sizes through | unported | image_sizes refused; also a mock expectation |
| 899 | regular > passes edit reason through | covered | edit |
| 907 | regular > edit conflict -> 409 | covered | edit_conflict |
| 914 | regular > post param missing -> 400 | recordable | edit_missing_post |
| 921 | regular > cannot see the post (PM) -> 403 | recordable | edit_pm_outsider (user2 PUT /posts/48) |
| 927 | regular > OP removed from their PM -> 403 | recordable | edit_pm_removed_author (setup: delete topic_allowed_users row for user1 on topic 42) |
| 934 | regular > updates raw, trailing whitespace stripped | recordable | edit_trailing_whitespace |
| 942 | regular > extracts links from the new body | recordable | edit_with_link |
| 952 | regular > deleted post not updatable | recordable | edit_deleted_post (user2 PUT /posts/45 -> 404) |
| 964 | staff > edit reason limit on a small action | covered | small_action_edit_reason_limit (new, matches) |
| 997 | staff > posts in deleted topics | unported | Unsupported("editing deleted posts") |
| 1008 | staff > unrelated moderator cannot update a reply in a trashed PM (GET 404, PUT 403, raw kept) | recordable | edit_deleted_pm_reply_moderator (setup: trash the PM topic, a moderator user, PUT the reply); the reply itself isn't deleted so routes/posts.rs skips "editing deleted posts" |
| 1035 | staff > invalid first-post title edit rolls back | unported | title param refused; watched words |
| 1077 | staff > no bump for whispers | needs-harness | freeze_time |
| 1093, 1106, 1119, 1145, 1169 | bypass_bump variants (x5) | recordable | no case yet: ported in 309d22ff (was: Unsupported("posts#update title, image sizes and bypass_bump") (nested one via "fields beyond raw")) |
| 1134 | bumps the topic when bypass_bump not provided | recordable | edit_last_post_bumps (admin edits post 52, the last post of 35) |
| 1199, 1208 | group moderator editing category description (x2) | unported | Unsupported("category group moderation"), Unsupported("editing category descriptions") |
| 1231, 1255 | category group moderator who cannot / can see (x2) | unported | Unsupported("category group moderation") |
| 1263, 1283 | change to disallowed / approval-required category (x2) | unported | category_id refused |
| 1303 | moderator cannot move a topic to a category they cannot see (403) | recordable | edit_first_post_category_private_moderator (moderator user, category_id of a group-restricted category); category_id is handled in routes/posts.rs, cf. edit_first_post_category_forbidden |
| 1321 | links without permission | covered | edit_links_require_trust (new, matches) |
| 1349 | plugin_permitted_update_params | needs-harness | plugin registration |
| 1382, 1397, 1412 | reply_to_post_number reparent / clear / invalid (x3) | unported | reply_to_post_number refused ("fields beyond raw and edit_reason") |

## PostsController#destroy

`spec/requests/posts_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 427 | action requires login | recordable | post_delete_anonymous |
| 432 | cannot see the post (PM) -> 404 | recordable | post_delete_pm_outsider (user2 DELETE /posts/48) |
| 442 | self deletions disabled -> 403 | covered | post_delete_self_disabled (new, matches) |
| 451 | member of delete_all_posts_and_topics_allowed_groups deletes another's post | covered | post_delete_allowed_group (new, matches) |
| 464 | uses a PostDestroyer (moderator) | covered | post_delete_staff (the mock expectation itself is not portable) |
| 480-531 | force_destroy: not yet deleted / time / two users / moderators / log (x5) | unported | Unsupported("permanently deleting posts and topics (force_destroy)"); also time travel and multi-user |

## PostsController#recover

`spec/requests/posts_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 682 | action requires login | recordable | post_recover_anonymous |
| 685 | cannot see the post (PM) -> 404 | recordable | post_recover_pm_outsider (user2 PUT /posts/48/recover) |
| 693 | self deletion/recovery disabled -> 403 | covered | post_recover_self_disabled (new, matches) |
| 702 | author recovers their own deleted post | unported | Unsupported("authors recovering their own posts (user_recovered)"); staff recovery is post_recover |

## PostsController#revisions (and #latest_revision)

`spec/requests/posts_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 3529 | revision < 2 -> 400 | covered | revision_not_found |
| 3534 | coalesced grace-period edits attributed to the right editor | needs-harness | PostRevisor with revised_at (grace period coalescing), heavy fabrication |
| 3596 | id passed as an array does not leak another post's revision | recordable | revision_id_array (GET /posts/36/revisions/2.json?id[]=36&id[]=48 as user2, 404) |
| 3626, 3640, 3654 | adjacent hidden revision not disclosed (x3) | recordable | no case yet: ported in eec314ce (was: Unsupported("hidden post revisions")) |
| 3722, 3730 | diff budget exceeded (x2) | needs-harness | ONPDiff stub |
| 3743 | history private > anonymous 403 | recordable | revision_history_private_anonymous |
| 3748 | history private > regular user 403 | covered | revision_history_private (new, matches) |
| 3754 | history private > staff 200 | recordable | revision_history_private_staff |
| 3760 | history private > poster 200 | recordable | revision_history_private_poster (user1 edits 36 then GETs) |
| 3771 | history private > TL4 403 | recordable | revision_history_private_tl4 (user3) |
| 3786, 3793, 3800 | category group moderator (x3) | unported | Unsupported("category group moderation") |
| 3811, 3827 | hide / show a revision (x2) | unported | other actions (hide_revision/show_revision), routes not ported |
| 3851 | post hidden > users 404 | recordable | revision_hidden_post (setup hides 36 + revision row; user1 as author still 404) |
| 3857 | post hidden > admins 200 | recordable | revision_hidden_post_staff |
| 3867 | public history > anyone (anonymous) | recordable | revision_anonymous |
| 3872, 3903, 3925 | restricted tag names not disclosed (x3) | needs-harness | heavy fabrication (tags, tag groups, private category, tag revisions YAML) |
| 3956 | unseen reply target post numbers omitted | unported | reply_to_post_number edit refused, whispers |
| 3997 | names disabled > no acting_user_name | recordable | revision_names_disabled (enable_names false) |
| 4014, 4028 | deleted post / deleted topic for staff (x2) | unported | Unsupported("deleted posts for staff") |
| 4043 | unlisted revision > next_revision only with include_unlisted | unported | Unsupported("navigating post revisions with unlisted ones") in posting/revisions.rs |
| 4053 | unlisted revision > include_unlisted doesn't grant hidden edit history (403 for revisions/2 and latest) | recordable | revision_unlisted_history_hidden (edit_history_visible_to_public false, post_revisions rows 2 and 3 unlisted via setup); the can_see 403 in posting/revisions.rs comes before the unlisted refusal |
| 4068 | tagged topic latest revision with tagging on/off | recordable | revision_latest_tagged_topic (setup revision row on post 42, topic 38 is tagged) |

## PostsController#markdown_num

`spec/requests/posts_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 4749 | anonymous GET /raw/:topic/1.json | covered | raw_post_anonymous (new) |
| 4758 | whole topic /raw/:topic_id | unported | route without post_number not ported |
| 4774 | hidden post revision > anonymous 403 on ?revision=3, moderator gets the visible raw | unported | Unsupported("raw of a revision") in routes/posts.rs; also anonymous then moderator |
| 4814 | hidden post > logged out 404 | recordable | raw_hidden_post_anonymous |
| 4823 | hidden post > regular user 404 | covered | raw_hidden_post (new, matches) |
| 4832 | hidden post > author sees it | recordable | raw_hidden_post_author |
| 4841 | hidden post > moderator sees it | recordable | raw_hidden_post_staff |
| 4850-4881 | whole topic with a hidden post, per viewer (x4) | unported | /raw/:topic_id not routed |

## TopicsController#status

`spec/requests/topics_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 1632 | needs you to be logged in | recordable | topic_status_anonymous (user null, PUT /t/35/status -> 403 not_logged_in) |
| 1640 | moderator > raises if you can't change it (plain user) | covered | topic_status_not_staff |
| 1646 | moderator > requires the status parameter | covered | topic_status_bad_params (new, matches) |
| 1651 | moderator > requires the enabled parameter | covered | topic_status_bad_params (new, matches) |
| 1656 | moderator > status not in the allowlist -> 400 | covered | topic_status_bad_params (new, matches) |
| 1661 | moderator > reopens a closed topic with an open timer, timer removed | recordable | no case yet: ported in 88084d3b (was: Unsupported("topics with timers"); plain reopen covered by topic_open) |
| 1677 | moderator > enabled truthy variants ("t", "0", true) | covered | topic_close_enabled_variants (new, matches) |
| 1708 | group moderator > close | unported | Unsupported("category group moderation") |
| 1716 | group moderator > reopen | unported | Unsupported("category group moderation") |
| 1728 | group moderator > archive | unported | Unsupported("category group moderation") |
| 1738 | group moderator > unarchive | unported | Unsupported("category group moderation") |
| 1748 | group moderator > pin with until | unported | category group moderation (Unsupported("category group moderation")); pinning until a time is ported (b7cad258) |
| 1761 | group moderator > unpin | unported | Unsupported("category group moderation") |
| 1768 | group moderator > unlist | unported | Unsupported("category group moderation") |
| 1779 | group moderator > relist | unported | Unsupported("category group moderation") |
| 1796 | TL4 > close visible topic | covered | topic_unpin_globally_tl4 (new, matches) |
| 1802 | TL4 > can't close restricted category topic | covered | topic_status_tl4_forbidden (new, matches) |
| 1809 | TL4 > can't close PM | covered | topic_status_tl4_forbidden (new, matches) |
| 1816 | TL4 > can't archive restricted topic | covered | topic_status_tl4_forbidden (new, matches) |
| 1823 | TL4 > can't pin restricted topic | covered | topic_status_tl4_forbidden (new, matches) |
| 1830 | TL4 > can't toggle visibility of restricted topic | covered | topic_status_tl4_forbidden (new, matches) |
| 1856 | API key scoped (category_id) > unpermitted category 403 | recordable | no case yet: ported in 17da04f7 (was: Unsupported("granular API key scopes on this route")) |
| 1874 | API key scoped > without category_id 403 | unported | granular API key scopes |
| 1891 | API key scoped > permitted category 200 | unported | granular API key scopes |
| 1920 | API key scope without param restrictions > updates | unported | granular API key scopes |
| - | (extra) closing an already closed topic: no small action, staff log still written | covered | topic_close_already_closed (new, matches) |
| - | (extra) pinned_globally disabled by TL4 (can_moderate branch) | covered | topic_unpin_globally_tl4 (new, matches) |
| - | (extra) visible with until (until ignored outside pinning) | covered | topic_relist_with_until (new, matches) |
| - | (extra) pinned with until / invalid until -> InvalidParameters | recordable | no case yet: ported in b7cad258 (was: Unsupported("pinning until a time (the unpin job)")) |

## TopicsController#destroy

`spec/requests/topics_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 2203 | anonymous -> 403 | recordable | topic_delete_anonymous (user null, DELETE /t/35.json) |
| 2215 | logged in > without access (own topic, 48h old) -> 422 | covered | topic_delete_own_old (new, matches) |
| 2225 | with permission (moderator) deletes | covered | topic_delete |
| 2243 | member of delete_all_posts_and_topics_allowed_groups deletes | covered | topic_delete_allowed_group (new, matches) |
| 2264 | category group moderator who can't see -> 422 | unported | Unsupported("category group moderation") |
| 2288 | category group moderator who can see deletes | unported | Unsupported("category group moderation") |
| 2305 | force destroy > destroys deleted small actions too | unported | refuse_force_destroy (post_destroy.rs) |
| 2326 | force destroy > logs, cleans sensitive info | unported | refuse_force_destroy |
| 2343 | force destroy > refused if not all posts destroyed | unported | refuse_force_destroy |
| 2353 | force destroy > refused if small actions not deleted | unported | refuse_force_destroy |
| - | (extra) author deletes own fresh single-post topic (mark_for_deletion) | covered | topic_delete_own_fresh (new, matches) |
| - | (extra) outsider deletes a PM -> 422 | covered | topic_delete_pm_outsider (new, matches) |

## TopicsController#timings

`spec/requests/topics_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 7349 | ignores invalid timing values (whisper, 1000, last_read recovery from 999) | recordable | timings_last_read_recovery (setup topic_users.last_read_post_number=999); out-of-range half covered by timings_out_of_range, whisper half needs a whisper post row |
| 7378 | ignores invalid values from staff (whisper counted) | needs-harness | needs a fabricated whisper post (posts row, topic counters); none in seed |
| 7401 | no timings for a topic the user can't see (POST /t/:id/timings) -> 404 | covered | timings_restricted_topic (new, matches) |
| 7423 | records the topic timing | covered | timings_read_rest |
| 7443 | caps msecs at 2^31-1 | covered | timings_msecs_cap (new, matches) |
| - | (extra) PM outsider -> 404 | covered | timings_pm_outsider (new, matches) |
| - | (extra) anonymous | covered | timings_anonymous |

## TopicsController#show

`spec/requests/topics_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 3433 | topic not allowed (detailed_404 on) -> 403 | covered | show_deleted_topic_detailed_404 (new, matches: the staff category's topic as user1) |
| 3456 | allowed to a group > descriptive error with group name | unported | Unsupported("detailed_404 for a topic a group gives access to"): not_in_group with the serialized group |
| 3466 | renders canonicals (HTML, Cache-Control) | needs-harness | header/HTML-structure assertions |
| 3474 | 301 even if slug param does not match (/t/:id.json?slug=x, /t/:slug.json) | covered | show_slug_param_mismatch (GET; port ignores ?slug=) |
| 3494 | shows a topic correctly | covered | golden: GET /t/parity-fixture-replies-and-posters/35.json |
| 3499 | PM tag descriptions hidden from viewer who can't see PM tags | covered | show_pm_tags_hidden (settings tagging/pm_tags_allowed_for_groups, setup topic_tags on 42, user0) |
| 3515 | tags restricted to inaccessible categories hidden | covered | show_restricted_tags (setup category_tags tag->cat 3 on topic 38) |
| 3548 | links from hidden posts not exposed | unported | Unsupported("topic links with clicks"), hidden posts |
| 3589 | link_counts from unlisted topics hidden | recordable | no case yet: the topic view computes link_counts now (src/topics/view/mod.rs) (was: post link counts (README Unsupported)) |
| 3632 | linkbacks from hidden or whisper source posts not exposed to non-staff | recordable | show_linkbacks_hidden_whisper_sources (settings whispers_allowed_groups, setup topic_links reflection rows from a hidden post and a whisper post to post on 35, GET 35.json anon); port link_counts query (src/topics/view/mod.rs:3560) does not filter by source post hidden/post_type, would diverge |
| 3684 | hidden post link counts hidden | recordable | no case yet: the topic view computes link_counts now (src/topics/view/mod.rs) (was: hidden posts, link counts) |
| 3743 | blank-slug topic | unported | Unsupported("topics without a stored slug (Slug.for)") |
| 3752 | over-range page redirects to last page | covered | show_page_over_range (GET 35.json?page=2) |
| 3757 | over-range page to last multi-page page | needs-harness | 25 fabricated posts |
| 3766 | keeps locale param when redirecting an over-range page | needs-harness | 25 fabricated posts and Location assertion (port has no tl param handling) |
| 3786 | viewer-visible count for last page (whispers) | needs-harness | 20+ fabricated posts and a whisper |
| 3802 | slug in id param redirects | covered | show_slug_only (GET /t/parity-fixture-replies-and-posters) |
| 3807 | slug with a number in front | covered | show_numeric_slug (setup UPDATE topics slug) |
| 3816 | /t/:id/summary with id[] array -> 400 | unported | topics#summary route not served |
| 3822 | /t/:id/summary nested id -> 400 | unported | topics#summary route not served |
| 3828 | keeps post_number query param when redirecting | covered | show_slug_redirect_params (port drops ?post_number=) |
| 3833 | keeps page when redirecting | covered | show_slug_redirect_params |
| 3839 | page param as array | covered | show_slug_redirect_params |
| 3845 | scrubs invalid query params | covered | show_slug_redirect_params |
| 3851 | nested_replies_default serves topic route | unported | nested replies view not ported |
| 3860 | crawlers not redirected to nested view | unported | nested replies |
| 3870 | PMs not redirected to nested view | unported | nested replies |
| 3882 | embed_mode on nested topics | unported | nested replies / embed mode |
| 3891 | embed class_name on nested topics | unported | nested replies / embed mode |
| 3900 | invalid slug without id -> 404 | covered | show_not_found_variants (GET /t/nope-nope.json) |
| 3906 | slug and id match nothing -> 404 | covered | show_not_found_variants |
| 3911 | id beyond postgres int -> 404 | covered | show_not_found_variants |
| 3917 | print=false is not print mode | covered | golden: GET /t/.../35.json (port ignores print) |
| 3924 | no N+1 with primary/flair groups | needs-harness | query counting |
| 3969 | no N+1 loading mentioned users | needs-harness | query counting |
| 3994 | content localization > no N+1 | needs-harness | query counting |
| 4045 | localized onebox for reader language | needs-harness | localization fabrication, onebox |
| 4053 | localized onebox omitted when translation off | needs-harness | localization fabrication, onebox |
| 4070 | serialize_post_user_badges | needs-harness | badge fabrication / registered badges |
| 4109 | keeps locale param when redirecting to the correct slug | needs-harness | Location header not recorded; status 301 already in show_slug_redirect_params |
| 4119 | omits page zero when redirecting to the correct slug | needs-harness | Location header not recorded |
| 4125 | encodes the locale param when redirecting to the correct slug | needs-harness | Location header not recorded |
| 4139 | redirect keeps modifier-registered param | needs-harness | plugin modifier |
| 4164 | nil-slug topic exists, unknown id -> 404 | covered | golden: GET /t/999999.json |
| 4208-4264 | detailed_404 off > anonymous (x9, HTML) | covered | golden: GET /t/42.json, /t/999999.json (JSON, not HTML) |
| 4208-4264 | detailed_404 off > anonymous login required (x9, HTML 302) | covered | show_login_required |
| 4208-4264 | detailed_404 off > anonymous login required json (x9, 403) | covered | show_login_required |
| 4208-4264 | detailed_404 off > normal user (x9) | covered | golden: GET /t/about-the-staff-category/2.json as=user1, /t/40 as=user1, /t/43 as=user0 |
| 4208-4264 | detailed_404 off > allowed user (x9) | covered | golden: GET /t/42.json as=user1, /t/2.json as=admin |
| 4208-4264 | detailed_404 off > moderator (x9) | covered | show_deleted_moderator (new, matches: setup makes user3 a moderator; deleted topic 200, staff category 200) |
| 4208-4264 | detailed_404 off > admin (x9) | covered | show_deleted_topic_staff (new, matches), show_deleted_reply_staff (new, matches) |
| 4208-4264 | detailed_404 on > normal user (x9) | covered | show_deleted_topic_detailed_404 (new, matches: deleted topic 410, staff category 403, the page too) |
| 4208-4264 | detailed_404 on > anonymous/allowed/moderator/admin (x36) | recordable | show_detailed_404_anonymous and the like (settings detailed_404; anonymous: deleted 410, secure 403, message 403 not_logged_in) |
| 4208-4264 | detailed_404 on > anonymous login required (x9, 302) | covered | show_login_required (login check precedes detailed_404) |
| 4493 | does not record a topic view | needs-harness | Rails view tracking is deferred (Scheduler::Defer) |
| 4497 | records incoming link for invalid post_number | needs-harness | deferred IncomingLink |
| 4504 | records incoming links | needs-harness | deferred IncomingLink |
| 4511 | print disabled -> 403 | unported | /print not served |
| 4519 | print enabled renders print view | unported | /print not served |
| 4530 | application layout without print param | needs-harness | HTML layout assertions |
| 4555 | print hides restricted tags | unported | /print not served |
| 4563 | referer recorded across redirect | needs-harness | follow_redirect + deferred IncomingLink |
| 4572 | tracks a visit for html requests | needs-harness | deferred visit tracking |
| 4587 | reviews new user for promotion | needs-harness | deferred promotion review |
| 4608 | filters > correct set of posts | needs-harness | TopicView.chunk_size stub |
| 4661 | external permalink on deleted topic, XHR | unported | permalinks not ported |
| 4672 | external permalink, XHR, detailed_404 | unported | permalinks, detailed_404 |
| 4685 | external permalink non-XHR 301 | unported | permalinks not ported |
| 4703 | permalink on nonexistent topic, XHR | unported | permalinks not ported |
| 4714 | permalink on nonexistent topic, 301 | unported | permalinks not ported |
| 4748 | show filters > replies_to_post_number | unported | filter params not ported (ignored) |
| 4790 | show filters > filter_top_level_replies | unported | filter params not ported |
| 4825 | show filters > filter_upwards_post_id | unported | filter params not ported |
| 4841 | show filters > max_reply_history | unported | filter params not ported |
| 4863 | login required > logged in shows topic | covered | show_login_required_member (user0, setting login_required) |
| 4872 | login required > anon browser redirected to login | covered | show_login_required |
| 4877 | login required > anon json 403 | covered | show_login_required |
| 4882 | login required > valid API key shows topic | covered | show_login_required_api_key (setup api_keys row with key_hash) |
| 4889 | login required > invalid API key 403 (json, html) | covered | show_login_required (HTTP_API_KEY: bad header) |
| 4900 | X-Robots-Tag for unlisted | needs-harness | response headers not compared |
| 4906 | no X-Robots-Tag for normal | needs-harness | response headers not compared |
| 4912 | X-Robots-Tag when allow_index_in_robots_txt off | needs-harness | response headers not compared |
| 4937 | topic_noindex modifier > receives core decision and the topic | needs-harness | plugin modifier registration, X-Robots-Tag header |
| 4951 | topic_noindex modifier > plugin noindex on every version of the topic page | needs-harness | plugin modifier, TopicView.chunk_size stub, headers (also /print not served) |
| 4971 | topic_noindex modifier > keeps site-wide nofollow when not indexable | needs-harness | plugin modifier registration, X-Robots-Tag header |
| 4985 | no incoming link without referer | needs-harness | deferred IncomingLink |
| 4990 | very long referer | needs-harness | deferred IncomingLink |
| 5009 | enable_user_status off > no mentions | covered | golden: GET /t/.../35.json (no mentioned_users key) |
| 5021 | enable_user_status > mentions with status | recordable | no case yet: ported in 4936de58 (was: Unsupported("user status on BasicUserSerializer")) |
| 5042 | enable_user_status > empty mentioned_users without mentions | covered | show_user_status_no_mentions (setting only; port omits mentioned_users) |
| 5053 | enable_user_status > unknown user mentioned -> empty | covered | show_user_status_no_mentions |
| 5067 | escaped fragment off > app layout | needs-harness | HTML layout assertions |
| 5083 | escaped fragment on > app layout without param | needs-harness | HTML layout assertions |
| 5092 | escaped fragment on > crawler layout | needs-harness | HTML layout assertions |
| 5111 | clear_notifications via cookie (subfolder) | needs-harness | subfolder + cookie |
| 5129 | clear_notifications via header | covered | show_clear_notifications_header (Discourse-Clear-Notifications header, marks a seeded notification read) |
| 5145 | no read only header by default | needs-harness | response headers not compared |
| 5151 | readonly header when site read only | needs-harness | readonly mode + headers |
| 5181, 5188 | hidden first post > not leaked in crawler meta description (topic page, visible reply /2) (x2) | recordable | show_hidden_first_post_crawler_meta (setup UPDATE posts SET hidden=true on post 1 of 35, topics.excerpt kept; HTML GET /t/.../35 and /35/2 with User-Agent Googlebot, extract meta description); port uses topics.excerpt for description (src/routes/topics.rs:822), likely leaks |
| 5196 | hidden first post > not leaked in regular HTML description for a visible reply | recordable | fold into show_hidden_first_post_crawler_meta (same setup, HTML GET /35/2 without bot UA) |
| 5203 | hidden first post > excerpt included for a viewer who can see hidden posts | recordable | show_hidden_first_post_meta_admin (same setup, user admin, HTML GET, extract meta description) |
| 5236 | hidden reply > crawler metadata keeps first post excerpt, not hidden reply | recordable | show_hidden_reply_crawler_meta (setup hide post 2 of 35, HTML GET /35/2 with Googlebot UA, extract meta description and body text) |
| 5247 | image-only topic meta description | needs-harness | HTML meta assertion, upload fixture |
| 5267 | image cdn url for schema markup | needs-harness | CDN config |
| 5279 | suggested topics only on last chunk | needs-harness | stub_const CHUNK_SIZE |
| 5297 | lazy_load_categories list | needs-harness | stub_const CHUNK_SIZE |

## ListController#private_messages*

`spec/requests/list_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 637 | pm tags > fails for non-staff | unported | /topics/private-messages-tags not routed |
| 644 | pm tags > fails for staff when empty | unported | private-messages-tags not routed |
| 653 | pm tags > succeeds for staff | unported | private-messages-tags not routed |
| 662 | pm tags > unicode tag | unported | private-messages-tags not routed |
| 672 | pm tags > direct link /u/:u/messages/tags | unported | users#show HTML, tags route |
| 681 | pm tags > only visible tagged PMs (+ group 404) | unported | private-messages-tags not routed |
| 710 | group new/unread 404 when PMs disabled for user | recordable | pm_group_new_pm_disabled (settings personal_message_enabled_groups=3, user1 trust_level_2/new) |
| 730 | group PMs shown to admin | recordable | pm_group_list_admin (setup topic_allowed_groups 43->12) |
| 740 | moderators-group PMs for moderator | recordable | pm_group_list_moderators (setup topic_allowed_groups -> group 2, admin) |
| 751 | moderator can't see another's group PMs | recordable | pm_group_list_moderator_other (setup makes user3 moderator-only) |
| 760 | group PMs sorted by posts_count | recordable | pm_group_list_order_posts (setup 42,43,44 allowed to group 12) |
| 792 | not found when user not in group | covered | golden: GET /topics/private-messages-group/user1/staff.json as=user1 |
| 797 | group's PMs listed | recordable | pm_group_list_member (setup topic_allowed_groups -> 12, user1) |
| 817 | unicode group name | recordable | pm_group_unicode_name (setup INSERT group + group_users) |
| 1654 | private_messages 403 for others | covered | golden: GET /topics/private-messages/user1.json as=user0 |
| 1660 | private_messages succeeds | covered | golden: GET /topics/private-messages/user1.json as=user1 |
| 1670 | private_messages order=activity | recordable | pm_list_order_activity (GET ?order=activity as user1) |
| 1703 | sent 403 for others | recordable | pm_sent_outsider (user0 GET private-messages-sent/user1) |
| 1709 | sent succeeds | covered | golden: GET /topics/private-messages-sent/user1.json as=user1 |
| 1728 | unread 404 for others | recordable | pm_unread_outsider (user0 GET private-messages-unread/user1) |
| 1734 | unread succeeds with tracking PM | covered | golden: GET /topics/private-messages-unread/user1.json as=user1 |
| 1774 | warnings 403 for unrelated | recordable | pm_warnings_outsider (user0 GET private-messages-warnings/user1) |
| 1781 | warnings shown to moderators and admins | recordable | pm_warnings_staff (setup INSERT user_warnings topic 43 user 3 by 1, admin) |
| 1792 | warning not listed for authoring moderator | recordable | pm_warnings_staff (second request: warnings/admin) |

## PostActionsController#destroy

`spec/requests/post_actions_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 10 | #destroy > requires you to be logged in | recordable | unlike_anonymous (DELETE /post_actions/52.json anon, 403; not proposed, trivial) |
| 18 | #destroy > logged in > 400 when post_action_type_id is missing | recordable | unlike_missing_type (not proposed, no data change) |
| 23 | #destroy > logged in > 404 when the action doesn't exist for that user | covered | unlike_not_liked (new) |
| 40 | #destroy > with a post_action > returns success | covered | unlike |
| 48 | #destroy > with a post_action > deletes the action | covered | unlike |
| 65 | #destroy > with a post_action > not deleted when the user doesn't have permission (created 1 day ago) | covered | unlike_after_window (new, matches) |
| 76 | #destroy > with a post_action > 204, no body, when the user can no longer see the post | covered | unlike_lost_access (new) |

## PostActionsController#create

`spec/requests/post_actions_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 101 | #create > requires you to be logged in | covered | like_anonymous |
| 106 | #create > forbids illegal flags when DSA reporting is enabled (403, no illegal post action, reviewable not marked potentially_illegal) | recordable | flag_illegal_dsa_enabled (dsa_reporting_enabled + setup off_topic flag/reviewable rows on the post, illegal flag with message, 403). flags.rs has no DSA check for illegal (flag_illegal_with_message passes), expect a diff |
| 130 | #create > does not reveal private post existence (PM post, then max id + 1) | covered | like_pm_not_participant (new, matches) |
| 152 | #create > 404 when flagging a hidden topic (flag_topic=true) | recordable | no case yet: ported in 8ef1061b (was: Unsupported("flagging topics") in routes/post_actions.rs) |
| 172 | #create > notify_user to a user with PMs disabled, 422 | recordable | no case yet: ported in 8ef1061b (was: Unsupported("flags that send a message (notify user, notify moderators, illegal)") in flags.rs) |
| 199, 214 | #create > non-staff > forbids warnings when is_warning is true / "true" (x2) | recordable | flag_notify_user_warning_non_staff (JSON body, is_warning true and "true", 403). Was unported: flags.rs now ports message flags, and can_act refuses notify_user + is_warning for non-staff |
| 230 | #create > non-staff > notifying a user without an official warning, is_warning false / "false" via include_examples at 257, 261 (x2) | recordable | flag_notify_user_warning_false (JSON body, 200, related post not an official warning); flag_notify_user covers it without is_warning. Was unported: message flags are ported in flags.rs |
| 271 | #create > moderator > 400 when id is missing | recordable | like_missing_id (not proposed, no data change) |
| 276 | #create > moderator > 404 when the id is invalid (-1) | covered | like_pm_not_participant (new, matches) |
| 286 | #create > moderator > 400 when post_action_type_id is missing | recordable | like_missing_type (not proposed, no data change) |
| 291 | #create > moderator > does not reveal private post existence | covered | like_pm_not_participant (new, matches) |
| 303 | #create > moderator > creates a post action on a post | covered | like |
| 319 | #create > moderator > passes a list of taken actions through (flag after flag, 403) | covered | flag_twice |
| 335 | #create > moderator > passes the message through (notify_user) | recordable | no case yet: ported in 8ef1061b (was: message-flag Unsupported) |
| 350 | #create > moderator > passes the message through as warning | recordable | no case yet: ported in 8ef1061b (was: message-flag Unsupported) |
| 370 | #create > moderator > passes take_action through | covered | flag_take_action |
| 388 | #create > moderator > doesn't pass take_action through for non-staff | covered | flag_inappropriate (spec sends no take_action param either) |
| - | extra (brief): flag a hidden post, 403 | covered | flag_hidden_post (new, matches) |

## DraftsController#index

`spec/requests/drafts_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 7 | #index > requires you to be logged in | recordable | draft_index_anonymous (not proposed, no data change) |
| 15 | #index > invalid limit params (include_examples, limit > 50 / negative) | recordable | draft_index_bad_limit (not proposed, no data change) |
| 18 | #index > correct stream length after adding a draft | covered | draft_new_topic_and_pm (new, matches) |
| 26 | #index > empty stream after deleting the last draft | recordable | draft_index_after_destroy (draft_destroy setup + DELETE + GET; not proposed) |
| 35 | #index > no topic details when user cannot see topic | covered | draft_new_topic_and_pm (new, matches) |
| 53 | #index > translated topic title | unported | Unsupported("draft titles with content localization") |
| 66 | #index > omits display user names when names are disabled | recordable | draft_index_names_disabled (enable_names=false, setup draft on topic_35; not proposed) |
| 83 | #index > categories when lazy load categories is enabled | unported | Unsupported("the drafts' categories when lazy loaded") |

## DraftsController#show

`spec/requests/drafts_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 100 | #show > returns a draft if requested | covered | draft_show |

## DraftsController#create

`spec/requests/drafts_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 111 | #create > requires you to be logged in | covered | draft_anonymous |
| 116 | #create > saves a draft | covered | draft_create |
| 125 | #create > triggers a draft_saved event | needs-harness | DiscourseEvent.track_events |
| 143 | #create > no draft_saved event when the save fails (409) | needs-harness | DiscourseEvent.track_events; the 409 itself is draft_out_of_sequence |
| 161 | #create > 404 when the key is missing | recordable | draft_missing_key (not proposed, no data change) |
| 167 | #create > checks for a raw conflict on update | covered | draft_edit_conflict |
| 193 | #create > checks for a title conflict on update | recordable | draft_edit_title_conflict (postId 35, exact raw + wrong original_title; topic 35 untagged so the port's tag Unsupported is not hit; not proposed) |
| 213 | #create > checks for a tag conflict on update | recordable | no case yet: ported in 03ed15e2 (was: Unsupported("the edit conflict check on tags")) |
| 237 | #create > hidden tags when checking for tag conflict | recordable | no case yet: ported in 03ed15e2 (was: Unsupported("the edit conflict check on tags")) |
| 270 | #create > tag objects format for tag conflict | recordable | no case yet: ported in 03ed15e2 (was: Unsupported("the edit conflict check on tags")) |
| 296 | #create > can't trivially resolve conflicts (sequence behind, no draft) | covered | draft_stale_sequence_no_draft (new, matches) |
| 313 | #create > clean protocol for ownership handover (owner change bumps sequence) | recordable | draft_owner_handover (not proposed; mostly draft_update plus the owner column) |
| 363 | #create > out-of-sequence draft setting, 409 (x2: seq-1, seq+1) | covered | draft_out_of_sequence |
| 396 | #create > data too big, 400 | covered | draft_too_long (new, matches) |
| 414 | #create > data not proper JSON, 400 | covered | draft_invalid_data |
| 423 | #create > data not a string, 400 | recordable | draft_data_not_string (data[cat]=tomtom; not proposed) |
| 430 | #create > 403 when max drafts per user is reached (existing key still saves) | covered | draft_too_many (new, matches) |
| 469 | #create > does not leak conflict info for posts user cannot see | covered | draft_edit_conflict_hidden_post (new, matches) |

## DraftsController#destroy

`spec/requests/drafts_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 490 | #destroy > destroys drafts when required | covered | draft_destroy |
| 499 | #destroy > denies attempts to destroy unowned draft (admin, username param, not API) | unported | Unsupported("clearing another user's drafts") fires on any username param; Rails answers 403 InvalidAccess here, a cheap gap to close |
| 513 | #destroy > via API > admin targets another user with username | unported | Unsupported("clearing another user's drafts") |
| 531 | #destroy > via API > admin acts on self without username | recordable | draft_destroy_api_key (api_keys row via setup SQL as in reply_api_key; same behaviour as draft_destroy, not proposed) |
| 548 | #destroy > via API > non-admin passing username gets 403 | unported | Unsupported("clearing another user's drafts") |
| 568 | #destroy > via API > non-admin acts on self without username | recordable | draft_destroy_api_key (as 495) |
| 585 | #destroy > via API > 404 when admin targets a nonexistent username | unported | Unsupported("clearing another user's drafts") |

## BookmarksController#create

`spec/requests/bookmarks_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 14 | #create > rate limits creates (429) | needs-harness | rate limits disabled on the Rails side |
| 41 | #create > max bookmark limit reached, 400 too_many | recordable | bookmark_too_many (admin has 2 bookmarks, max_bookmarks_per_user=2; not proposed) |
| 67 | #create > already bookmarked (Post and Topic), 400 | covered | bookmark_duplicate (Post half; the Topic half runs the same query with the type) |
| 96 | #create > 403 when the first post of a topic is hidden | covered | bookmark_topic_hidden_first_post (new) |
| - | extra (brief): bookmark a PM the user can't see | covered | bookmark_pm_not_participant (new, matches) |
| - | extra (brief): auto_delete_preference from the user option when not given | covered | bookmark_user_auto_delete_default (new, matches) |

## BookmarksController#update

`spec/requests/bookmarks_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| - | extra (brief): update another user's bookmark, 403; missing bookmark, 404 | covered | bookmark_edit_other (new, matches) |

## BookmarksController#toggle_pin

`spec/requests/bookmarks_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| - | extra (brief): toggle pin on another user's bookmark, 403 | covered | bookmark_edit_other (new, matches) |

## BookmarksController#destroy

`spec/requests/bookmarks_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 114 | #destroy > destroys the bookmark | covered | bookmark_destroy |
| 119 | #destroy > topic_bookmarked metadata as bookmarks go | covered | bookmark_destroy (topic still bookmarked, true) + bookmark_destroy_topic |
| 140 | #destroy > already destroyed, 404 | recordable | bookmark_destroy_missing (DELETE /bookmarks/9999.json; not proposed, no data change) |
| 151 | #destroy > bookmark of another user, 403 | covered | bookmark_destroy_other |

## NotificationsController#index

`spec/requests/notifications_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 67 | #index > returns recent notifications (recent, not silent) | covered | notifications_recent_bump (new, matches) |
| 72 | #index > returns notification history | covered | golden: GET /notifications.json as=user1 |
| 83 | #index > marks notifications as viewed (recent bumps seen_notification_id) | covered | notifications_recent_bump (new, matches) |
| 94 | #index > silent does not mark viewed | covered | golden: GET /notifications.json?recent=true&silent=true as=user1 |
| 105 | #index > not marked viewed in read-only mode | needs-harness | Discourse.received_redis_readonly! |
| 120 | #index > invalid limit params (recent=true) | recordable | notifications_recent_bad_limit (golden only has non-recent limit=61; not proposed) |
| 128 | #index > limit param and load_more_notifications offset | covered | golden: GET /notifications.json?limit=2&offset=1 as=user1, ?username=user1 as=admin |
| 150 | #index > all filters (read / unread) | covered | golden: filter=read, filter=unread as=user1; notifications_read_one |
| 219 | #index > sidebar > unread (high priority first) at the top | covered | golden: GET /notifications.json?recent=true&silent=true as=user1 (user1 has high-priority unread PM notifications) |
| 235 | #index > sidebar > last-seen reviewable not bumped in read-only mode | needs-harness | redis read-only |
| 248 | #index > sidebar > last-seen reviewable not bumped without review access | covered | notifications_recent_bump (new, matches) |
| 255 | #index > sidebar > last-seen reviewable not bumped with silent (admin) | recordable | no case yet: ported in c4c880a0 (was: admin + pending reviewable: Unsupported("pending_reviewables (Reviewable.user_menu_list_for)")) |
| 269 | #index > sidebar > last-seen reviewable not bumped without bump param (admin) | unported | pending_reviewables Unsupported |
| 278 | #index > sidebar > bumps last_seen_reviewable_id | unported | pending_reviewables Unsupported (and the bump runs in Scheduler::Defer) |
| 296 | #index > sidebar > includes pending reviewables | unported | pending_reviewables Unsupported |
| 311 | #index > sidebar > excludes reviewables claimed by others | unported | pending_reviewables Unsupported |
| 331 | #index > sidebar > no reviewables without review queue access | covered | golden: GET /notifications.json?recent=true&silent=true as=user1 |
| 369 | #index > filter_by_types > filters to the given types | covered | golden: ?recent=true&filter_by_types=liked,mentioned&silent=true as=user1 |
| 381 | #index > filter_by_types > excludes other users' notifications | covered | same golden (other users' liked/mentioned rows exist in the seed) |
| 394 | #index > filter_by_types > respects limit | recordable | notifications_types_limit (GET only; not proposed) |
| 402 | #index > username not found, 404 | recordable | notifications_unknown_username (GET only; not proposed) |
| 437 | #index > inaccessible topics > recent filter | covered | notifications_filtered_badge_and_topic (new, matches) |
| 447 | #index > inaccessible topics > without recent | covered | notifications_filtered_badge_and_topic (new, matches) |
| 470 | #index > disabled badges > recent > filters disabled badge | covered | notifications_filtered_badge_and_topic (new, matches) |
| 481 | #index > disabled badges > recent > enable_badges=false | recordable | notifications_badges_disabled (not proposed) |
| 495 | #index > disabled badges > paged > filters disabled badge | covered | notifications_filtered_badge_and_topic (new, matches) |
| 506 | #index > disabled badges > paged > enable_badges=false | recordable | notifications_badges_disabled (not proposed) |
| 523 | #index > show_user_menu_avatars > acting_user_avatar_template | covered | notifications_acting_user_recent, notifications_acting_user_paged (prioritize_full_name_in_ux) (new, match) |
| 539 | #index > names disabled > no mentioner display_name | recordable | notifications_names_disabled (enable_names=false, user1 has mention 51 with display_name; not proposed) |
| 576 | #index > avatars on, names off > no liker name | covered | notifications_acting_user_no_names (new, match) |
| 617 | #index > localizations > localized fancy_title when enabled | recordable | notifications_localized_title (topic_localizations row + users.locale + settings; port has no localization branch and no Unsupported; not proposed) |
| 629 | #index > localizations > plain fancy_title when disabled | covered | golden: GET /notifications.json as=user1 (default settings) |
| 794 | not logged in > #index requires authentication | covered | golden: GET /notifications.json (anon) |
| 843 | user api keys > allows access to notifications#index | unported | port has no User-Api-Key auth |

## NotificationsController#mark_read

`spec/requests/notifications_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 643 | marks every notification as read | covered | notifications_read_all |
| 648 | can update a single notification | covered | notifications_read_one |
| 660 | updates the read status (unread counts) | covered | notifications_read_all |
| 692 | #mark_read > by id > marks a notification as read | covered | notifications_read_one |
| 700 | #mark_read > by id > doesn't mark another user's notification | covered | notifications_read_other (new, matches) |
| 711 | #mark_read > by type > marks notifications as read | covered | notifications_read_types |
| 744 | #mark_read > by type > doesn't mark other users' notifications | covered | notifications_read_types (row diff covers every user's notifications) |

## UsersController#bookmarks

`spec/requests/users_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 9047 | #bookmarks > list of serialized bookmarks | covered | golden: GET /u/user1/bookmarks.json as=user1 |
| 9056 | #bookmarks > including custom registered bookmarkables | needs-harness | register_test_bookmarkable (plugin registration) |
| 9071 | #bookmarks > .ics file in date order | unported | .ics format not routed (/u/:username/bookmarks only, JSON) |
| 9117 | #bookmarks > .ics excludes topics the user cannot see | unported | .ics not routed |
| 9144 | #bookmarks > .ics excludes reminders older than 3 months | unported | .ics not routed |
| 9159 | #bookmarks > .ics no HTML encoding | unported | .ics not routed |
| 9171 | #bookmarks > .ics URI delimiters | unported | .ics not routed |
| 9185 | #bookmarks > another user's bookmarks, 403 | covered | golden: GET /u/user1/bookmarks.json as=user0 |
| 9191 | #bookmarks > moderators can't view another user's bookmarks | recordable | bookmarks_list_moderator (setup makes user3 a moderator; same is_admin branch as the user0 golden; not proposed) |
| 9208 | #bookmarks > no bookmarks found | covered | golden: GET /u/user0/bookmarks.json as=user0 |
| 9218 | #bookmarks > no bookmarks for the search | covered | golden: GET /u/user1/bookmarks.json?q=zzzz as=user1 |
| 9228 | #bookmarks > invalid limit params | covered | golden: limit=0, limit=50, limit=abc as=user1 |
| 9247 | #bookmarks > localized fancy_title when enabled | unported | Unsupported("content localization on bookmarks") |
| 9260 | #bookmarks > plain fancy_title when disabled | covered | golden: GET /u/user1/bookmarks.json as=user1 |
| 9279 | #bookmarks excerpts > first post of the topic as the excerpt | covered | golden: GET /u/user1/bookmarks.json as=user1 (topic bookmark 2) |
| 9295 | #bookmarks excerpts > no excerpt for hidden post bookmarks | covered | bookmarks_list_hidden_posts (new, matches) |
| 9317 | #bookmarks excerpts > hidden first posts not listed or searched | covered | bookmarks_list_hidden_posts (new, matches) |
| 9369 | #bookmarks excerpts > bookmarkable_url to the first post without the option | covered | golden: GET /u/user1/bookmarks.json as=user1 |

## UsersController#user_menu_bookmarks

`spec/requests/users_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 9351 | #bookmarks excerpts > user menu bookmarkable_url to the first unread post | covered | golden: GET /u/user1/user-menu-bookmarks.json as=user1 (topic bookmark 2 with topic_users read state) |
| 9534 | logged out, 404 | covered | golden: GET /u/user1/user-menu-bookmarks.json (anon) |
| 9543 | another user's list, 403 | covered | golden: GET /u/user1/user-menu-bookmarks.json as=admin |
| 9548 | unread bookmark_reminder notifications only (read one dropped) | recordable | bookmarks_menu_read_reminder (setup a second, read, reminder notification; not proposed) |
| 9568 | bookmarks not tied to unread reminders; read reminder or no bookmark_id brings them back | recordable | bookmarks_menu_read_reminder (setup marks notification 52 read / strips bookmark_id; not proposed) |
| 9610 | fills up USER_MENU_LIST_LIMIT | needs-harness | stub_const |
| 9649 | no unread reminders for bookmarks the user can no longer access | recordable | bookmarks_menu_inaccessible_reminder (setup bookmark + reminder notification on post 2, Staff category; not proposed) |
| 9667 | no unread reminders when the bookmark was deleted | recordable | bookmarks_menu_deleted_bookmark (setup DELETE bookmark 5; not proposed) |
| 9677 | same, when the notification only has bookmark_id | recordable | bookmarks_menu_deleted_bookmark (setup strips bookmarkable_type/id from notification 52) |
| 9700 | show_user_menu_avatars > acting_user_avatar | covered | notifications/user_menu_bookmarks_acting_user (new, match) |

## UploadsController#create

`spec/requests/uploads_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 8 | requires you to be logged in | covered | upload_anonymous |
| 28 | logged in > rate limited > 429 past max_uploads_per_minute | needs-harness | rate limits (and avatar PNG uploads) |
| 49 | logged in > expects upload_type (400 without it) | recordable | upload_missing_type (a fixture file without upload_type, 400 param missing); avatar uploads are ported since e73b020a |
| 66 | logged in > accepts the deprecated type param, logs a deprecation | needs-harness | no case yet: ported in 2e8165be (was: Unsupported "the deprecated type param" (also `allow(Discourse).to receive`)) |
| 81 | logged in > is successful with an image (avatar, CreateAvatarThumbnails) | recordable | no case yet: ported in e73b020a (was: Unsupported "avatar uploads"; PNG refused (image_optim)) |
| 88 | logged in > logs unexpected upload errors, generic message | needs-harness | stubs FileStore::LocalStore#store_upload, inspects logs |
| 113 | logged in > returns "raw" url for site settings | unported | Unsupported "site setting uploads"; CDN, PNG |
| 129 | logged in > returns cdn url | unported | PNG refused (image_optim); also needs a CDN config |
| 136 | logged in > is successful with an attachment | covered | upload_text (txt attachment accepted; spec uses `*`) |
| 147 | logged in > is successful with api (url download) | unported | Unsupported "uploads downloaded from a url"; also stub_request |
| 174 | logged in > correctly sets retain_hours for admins | unported | Unsupported "retain_hours" |
| 189 | logged in > requires a file (422 file_missing) | unported | without a fixture the harness sends a urlencoded form: Unsupported "uploads not sent as multipart/form-data" |
| 198 | logged in > properly returns errors (too large, max_attachment_size_kb 1) | covered | upload_too_large (new, matches) |
| 212 | logged in > user must be in uploaded_avatars_allowed_groups for an avatar | covered | upload_avatar_not_allowed (new) |
| 223 | logged in > discourse_connect_overrides_avatar blocks avatars | recordable | upload_avatar_sso_overrides (same controller-level 422 as above) |
| 229 | logged in > auth_overrides_avatar blocks avatars | recordable | upload_avatar_auth_overrides (same controller-level 422) |
| 235 | logged in > staff may upload any file in a PM | covered | upload_staff_any_file_in_pm (new) |
| 252 | logged in > staff upload supported images for site settings | unported | Unsupported "site setting uploads" |
| 272 | logged in > authorized_extensions_for_staff respected for staff | covered | upload_staff_extension (new, matches) |
| 284 | logged in > authorized_extensions_for_staff ignored for non-staff | recordable | upload_staff_extension_not_staff (user2, same settings, 422 unauthorized) |
| 294 | logged in > GIF whose decoded frame exceeds max_image_megapixels (libvips) is a 422 | recordable | upload_gif_frame_over_megapixels (fixture gif_frame_larger_than_logical_screen.gif, new at 9daeae2a so not in vendor yet; max_image_megapixels 5). enable_vips_image_processing is a global setting; uploads.rs TooManyPixels may read the logical screen, expect a possible diff |
| 315 | logged in > could not determine image dimensions (fake.jpg) | recordable | upload_fake_image: needs a generated fixture fake.jpg (non-image bytes). The port answers `not_supported_or_corrupted` when the bytes aren't detected while Rails expects `size_not_found`, worth recording |
| 332 | system user > errors when system_user_max_attachment_size_kb is unset | recordable | upload_system_user_too_large: system via an api_keys setup row + Api-Key/Api-Username headers (as incoming_reply_api_key), large.txt |
| 345 | system user > accepts large files from the system user | recordable | upload_system_user_large (system_user_max_attachment_size_kb above large.txt's size, max_attachment_size_kb 1) |
| 353 | system user > rejects above the system limit; both limits low; above the attachment limit (x3) | recordable | upload_system_user_limits_* (same api key setup; large.txt must be over 10 KB for the "10 KB" variants) |

## UploadsController and Admin::BackupsController direct uploads (ExternalUploadHelpers)

`spec/requests/uploads_controller_spec.rb` (#generate_presigned_put,
#create_multipart, #batch_presign_multipart_parts, #complete_multipart,
#abort_multipart, #complete_external_upload) and
`spec/requests/admin/backups_controller_spec.rb` ("S3 multipart
uploads"). The cases run on MinIO (`s3`, parity/writes/s3_direct_uploads);
what a browser does between the requests (the PUT to a presigned URL,
each part's PUT) is in tests/direct_uploads.rs.

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 1133 | presigned put > errors if the correct params are not provided | recordable | presigned_put_missing_params |
| 1140 | presigned put > generates a presigned URL and creates an external upload stub | covered | presigned_put (new, matches) |
| 1166 | presigned put > includes accepted metadata | covered | presigned_put (sha1-checksum signed and returned, other keys dropped) |
| 1193 | presigned put > s3-accelerate endpoint | needs-harness | AWS's own endpoint; the request host is unit-tested (object_storage::s3) |
| 1216 | presigned put > rate limits | needs-harness | rate limits |
| 1240 | presigned put > 404 when the store is not external | covered | local_store (new, matches) |
| 1254 | presigned put > 404 with direct uploads off | covered | disabled (new, matches) |
| 1279 | create multipart > params missing | recordable | multipart_missing_params |
| 1286 | create multipart > 422 when the create request errors | recordable | multipart_create_error (a wrong secret) |
| 1296 | create multipart > attachment too big | covered | presigned_put (the same validate_file_size) |
| 1306 | create multipart > gif too big | recordable | multipart_gif_too_big |
| 1316 | create multipart > 0 bytes | covered | presigned_put (the same validate_file_size) |
| 1379 | create multipart > creates the upload and a multipart stub | covered | multipart (new, matches) |
| 1404 | create multipart > no backup multipart uploads through /uploads | covered | backup_multipart: at the vendored commit an admin's backup type is let through (200), as the port does |
| 1420 | create multipart > only allowed metadata keys | recordable | multipart_metadata |
| 1447 | create multipart > rate limits | needs-harness | rate limits |
| 1473 | create multipart > 404 when the store is not external | covered | disabled |
| 1539 | batch presign > params missing | recordable | batch_presign_missing_params |
| 1544 | batch presign > part numbers between 1 and 10000 | covered | multipart |
| 1565 | batch presign > 404 when the stub does not exist | covered | multipart (after the abort) |
| 1574 | batch presign > 404 when the stub is another user's | needs-harness | several users in one case |
| 1584 | batch presign > 404 when the multipart upload does not exist | recordable | batch_presign_upload_gone (a stub row with a made-up upload id) |
| 1597 | batch presign > presigned URLs keyed by part number | covered | multipart, backup_multipart |
| 1619 | batch presign > dualstack endpoint by region | needs-harness | AWS's own endpoint; unit-tested |
| 1635 | batch presign > s3-accelerate endpoint | needs-harness | AWS's own endpoint; unit-tested |
| 1653 | batch presign > rate limits | needs-harness | rate limits |
| 1679 | batch presign > 404 when the store is not external | recordable | local_store variant |
| 1747 | complete multipart > params missing | recordable | complete_multipart_missing_params |
| 1752 | complete multipart > part numbers between 1 and 10000 | recordable | complete_multipart_bad_part |
| 1783 | complete multipart > parts with missing values | covered | multipart |
| 1794 | complete multipart > 404 when the stub does not exist | recordable | complete_multipart_no_stub |
| 1803 | complete multipart > 422 when the complete request errors | covered | multipart (a made-up etag) |
| 1817 | complete multipart > 404 when the stub is another user's | needs-harness | several users in one case |
| 1827 | complete multipart > 404 when the multipart upload does not exist | recordable | complete_multipart_upload_gone |
| 1840 | complete multipart > completes, creates and returns the upload | needs-harness | the parts are a browser's PUTs: tests/direct_uploads.rs |
| 1889 | complete multipart > passes site_setting_name | unported | Unsupported "site setting uploads" |
| 1929 | complete multipart > rate limits | needs-harness | rate limits |
| 1951 | complete multipart > 404 when the store is not external | covered | local_store |
| 1993 | abort > params missing | recordable | abort_missing_params |
| 1998 | abort > 200 when the stub is already gone | covered | multipart |
| 2004 | abort > 404 when the stub is another user's | needs-harness | several users in one case |
| 2013 | abort > aborts the upload and deletes the stub | covered | multipart, backup_multipart |
| 2025 | abort > 422 when the abort request errors | needs-harness | stubs an S3 error |
| 2041 | abort > 404 when the store is not external | covered | disabled |
| 2065 | complete external > 404 when the stub does not exist | covered | presigned_put |
| 2070 | complete external > 404 when the stub is another user's | needs-harness | several users in one case |
| 2079 | complete external > 404 when the stub key is outside the temporary upload folder | recordable | complete_external_upload_not_temporary (external_upload_stubs row with key original/1X/logo.png); in_temporary_folder check in routes/external_uploads.rs |
| 2090 | complete external > ChecksumMismatchError | needs-harness | the object needs its sha1-checksum metadata, which `s3.objects` doesn't set |
| 2105 | complete external > SizeMismatchError | covered | complete_external_upload (new, matches) |
| 2120 | complete external > CannotPromoteError | covered | complete_external_upload |
| 2133 | complete external > DownloadFailedError and NotFound | covered | presigned_put (completed before any PUT) |
| 2156 | complete external > a generic upload failure | needs-harness | stubs an exception |
| 2166 | complete external > validation errors on the upload | recordable | complete_external_upload_unauthorized (an object with an extension not authorized) |
| 2177 | complete external > deletes the stub and returns the upload | covered | complete_external_upload |
| 2220 | complete external > stores the validated local bytes, no copy_object | needs-harness | FakeS3 operation_called? and stub_request on the signed URL |
| 2249 | complete external > passes site_setting_name | unported | Unsupported "site setting uploads" |
| 2268 | complete external > no site_setting_name without for_site_setting | recordable | complete_external_upload_for_site_setting_false |
| backups 1096 | backups multipart > 404 for moderators and non-staff (x2) | recordable | backup_multipart_not_admin (the admin constraint) |
| backups 1143 | backups multipart > creates the multipart upload | covered | backup_multipart (new, matches) |
| backups 1169 | backups multipart > in readonly mode | recordable | backup_multipart_readonly |
| backups 1186 | backups multipart > a backup of that name exists | covered | backup_multipart |
| backups 1199 | backups multipart > invalid filename | recordable | backup_multipart_bad_name |
| backups 1212 | backups multipart > invalid extension | covered | backup_multipart |

## ReviewablesController#index

`spec/requests/reviewables_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 5 | anonymous > denies listing | recordable | review_list_anonymous (403) |
| 46 | #index > dsa_classification statement ids and extras, statements loaded in one query | needs-harness | track_sql_queries count; the payload part (review/dsa.rs) is recordable as review_list_dsa_classification (dsa_reporting_enabled, community rules, dsa_statement_of_records rows) |
| 86 | logged in > #index > empty JSON when nothing to review | recordable | review_list_empty (the seed has no reviewables) |
| 93 | #index > loads the acting user behind author penalties in one query | needs-harness | SQL query tracking; penalized authors are also Unsupported |
| 114 | #index > returns JSON with reviewable content | covered | review_list |
| 139 | #index > action ids scoped to their reviewable | covered | review_list_many (two flagged posts, per-reviewable action ids) |
| 189 | #index > trashed topics and posts > shown to staff (moderator) | recordable | no case yet: ported in 37c397e3 (was: Unsupported "the review queue for moderators") |
| 207 | #index > trashed topics and posts > shown to category mods | unported | Unsupported "the review queue for category group moderators" |
| 225 | #index > trashed > excludes inaccessible whispers for category mods | unported | Unsupported "the review queue for category group moderators" |
| 239 | #index > filtering by flag reason (score_type) | recordable | review_list_score_type (setup with two flags of different score types) |
| 264 | #index > filtering by flagged_by | recordable | review_list_flagged_by |
| 289 | #index > filtering by score (min_score=1000) | recordable | review_list_empty with `min_score` (not a controller param; nothing in the queue) |
| 296 | #index > supports offsets | covered | review_list_many_paged |
| 303 | #index > filtering by type (ReviewableUser) | recordable | no case yet: ported in 1d5ca2d6 (was: Unsupported "reviewables other than flagged posts in the queue") |
| 311 | #index > invalid type is a 400 | recordable | review_list_bad_type |
| 316 | #index > filtering by status (ReviewableUser) | covered | review_list_reviewed (status filter; ReviewableUser rows themselves are Unsupported) |
| 341 | #index > invalid status is a 400 | recordable | review_list_bad_status |
| 346 | #index > status=problem_confirmed (rejected queued post, agreed flag) and approval_style | recordable | review_list_problem_confirmed (setup: rejected ReviewableQueuedPost and approved ReviewableFlaggedPost rows); review/list.rs by_outcome and approval_style are ported |
| 360 | #index > filtering by category_id | recordable | review_list_category (category 4 vs another id) |
| 380 | #index > ReviewableUser serializer | recordable | no case yet: ported in 1d5ca2d6 (was: Unsupported "reviewables other than flagged posts in the queue") |
| 396 | #index > ReviewableUser payload name hidden from moderators when enable_names is off, shown to admins | needs-harness | moderator then admin in one test; split it records: review_list_user_names_hidden_moderator / _admin (must_approve_users, enable_names false, ReviewableUser row via setup) |
| 469 | #index > date range > empty, and matching (x2) | unported | Unsupported "the review queue's date filters" |
| 495 | #index > user custom field allowed by a plugin | needs-harness | plugin registration (allow_public_user_custom_field) |
| 521 | #index > filtering by id | recordable | no case yet: ported in b75648d3 (was: Unsupported "the review queue's ids filter") |
| 533 | #index > no N+1 with localizations | unported | Unsupported "content localization in the review queue" |
| 580 | #index > notes > no N+1 | recordable | no case yet: ported in e4551868 (was: Unsupported "claimed topics and notes in the queue") |
| 2016 | #index > PM reviewable hidden from an outside moderator | recordable | no case yet: ported in 37c397e3 (was: Unsupported "the review queue for moderators" (and ReviewablePost)) |

## ReviewablesController#perform

`spec/requests/reviewables_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 10 | anonymous > denies performing | recordable | review_anonymous (403) |
| 422 | #index block > ReviewableUser not found error on second delete_user | unported | Unsupported "reviewable types other than flagged posts" |
| 441 | #index block > reject_reason too long on delete_user | unported | Unsupported "reviewable types other than flagged posts" |
| 1023 | agree_and_edit on a flagged post without saved revision evidence is a 400, stays pending | covered | review/review_agree_and_edit (400, no post_version) |
| 1035 | agree_and_edit with the saved post_version approves | recordable | review_agree_and_edit_post_version (setup: post version 2 + post_revisions row 2, post_version param); edit_saved in review/mod.rs |
| 1053 | agree_and_suspend with reason, suspend_until and post_action delete | unported | Unsupported("deleting a post for a review with a penalty (notify_deletion)") in users/penalties.rs (review penalties pass the reviewable_id) |
| 1078 | reject_and_silence on a queued post with reason and silenced_till | recordable | queued_reject_and_silence_details (queued_reject_and_silence only covers the 422 without details) |
| 1094 | agree_and_silence without penalty details is a 422, stays pending | covered | review/review_agree_and_silence (422) |
| 1108 | category moderator approving a queued post keeps an unrelated silence | unported | Unsupported("the review queue for category group moderators") in routes/review.rs |
| 1124 | #perform > 404 to category moderators for an inaccessible whisper | unported | Unsupported "category group moderation" |
| 1153 | #perform > statuses of other reviewables resolved by delete_user_block | recordable | no case yet: ported in 8268ca35 (was: Unsupported "review actions that delete, edit, restore or penalize") |
| 1175 | #perform > 404 when the reviewable does not exist | covered | review_missing |
| 1180 | #perform > validates the presence of an action (403) | covered | review_unknown_action (new, matches), review_already_reviewed (new, matches) |
| 1185 | #perform > ensures the user can see the reviewable (moderator, not reviewable_by_moderator) | recordable | no case yet: ported in 37c397e3 (was: Unsupported "reviewables for groups") |
| 1191 | #perform > can properly return errors (queued post) | unported | Unsupported "reviewable types other than flagged posts" |
| 1201 | approve_post of a queued topic with scalar tags is a 400 invalid_params, stays pending | unported | Unsupported("tags as a single value") via tagging::parse_param in review/posts.rs |
| 1219 | #perform > requires a version parameter (422) | recordable | review_no_version |
| 1226 | #perform > succeeds for a valid action (approve_user) | unported | Unsupported "reviewable types other than flagged posts" (covered in spirit by review_agree_and_keep) |
| 1246 | #perform > no email when send_email is false | unported | ReviewableUser, Unsupported "reviewable types other than flagged posts" |
| 1256 | #perform > removed likes stay removed after delete_and_agree | recordable | no case yet: ported in 8268ca35 (was: Unsupported "review actions that delete, edit, restore or penalize") |
| 1276 | #perform > releases a deleted topic's claim when agreeing and silencing (now with reason, silenced_till) | unported | Unsupported("reviewing flags on deleted posts") in review/mod.rs (the post is destroyed first); claiming and agree_and_silence with details are ported |
| 1302 | #perform > claims > required, claimed by others, optional (x3) | recordable | no case yet: ported in e4551868 (was: Unsupported "reviewable claiming" (queued posts too)) |
| 1326 | #perform > simultaneous perform > wrong version is a 409 | covered | review_stale_version |
| 1344 | #perform > flagged post deleted > agree_and_keep_deleted works, no 403 (x2) | unported | Unsupported "reviewing flags on deleted posts" |
| 1419 | plugin API reviewable params passed to perform | needs-harness | plugin registration, MessageBus.expects |
| 2059, 2078, 2091, 2104 | DSA classification > classify with the rule and note / unknown rule / string / invalid category (x4) | unported | Unsupported("classifying DSA statements from a review action") in routes/review.rs |
| 2117 | DSA classification > delete_and_agree without classification leaves a pending statement | unported | Unsupported("review actions that delete or restore, and unsilencing") in review/mod.rs |
| 2128 | DSA classification > classification while reporting is disabled is a 403, stays pending | recordable | review_dsa_classification_disabled (dsa_classification param, dsa_reporting_enabled false); routes/review.rs returns invalid_access before the action |
| 2142, 2161 | DSA classification > category moderator classifies in their category / outside it (x2) | unported | Unsupported("the review queue for category group moderators") in routes/review.rs; classifying also refused |
| 2198 | #perform > PM reviewable 404 for an outside moderator | unported | Unsupported "reviewable types other than flagged posts" (ReviewablePost), moderator counts |

## Admin::SiteSettingsController#update

`spec/requests/admin/site_settings_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 248 | admin > sets the value when the param is present (title) | covered | setting_string, setting_title_welcome_topic and setting_title_edited_welcome (new; the welcome topic rewritten unless edited) |
| 254 | admin > bulk updates settings | covered | bulk_update, bulk_update_invalid |
| 271 | admin > error for hard deprecated settings | covered | setting_hard_deprecated (new) |
| 282 | admin > works for soft deprecated settings | needs-harness | stub_deprecated_settings!(override: true); no real soft deprecation in SETTINGS |
| 291 | admin > not a configurable setting (clear_cache!) with update_existing_user | covered | setting_unknown (with update_existing_user the port hits Unsupported "backfilling user preferences") |
| 304 | admin > deprecated enable_personal_messages with override false | covered | setting_unknown (the setting no longer exists at this commit) |
| 317 | admin > value can be a blank string (title) | recordable | setting_blank_string (short_title set via setup SQL, then ""; title is Unsupported) |
| 323 | admin > blank string for selectable_avatars | recordable | no case yet: ported in 8932e28c (was: Unsupported "setting types other than string, number, bool and enum" (uploaded_image_list)) |
| 330 | admin > sanitizes integer values ("1,000") | covered | setting_integer_delimited (new, matches) |
| 337 | admin > sanitizes file_size_restriction values | recordable | no case yet: ported in 8932e28c (was: Unsupported "setting types other than string, number, bool and enum") |
| 344 | admin > sanitizes negative integer values | recordable | setting_negative_integer (pending_users_reminder_delay_minutes "-1", min -1) |
| 358 | admin > default user options > updates, doesn't update, email_digests (x3) | covered | admin_config/fonts_update, fonts_no_backfill, fonts_user_count (new; the digest frequency's backfill with email_digests) |
| 405 | admin > default navigation menu > backfill job or not (x3) | unported | Unsupported "default_* settings" / "backfilling user preferences" |
| 478 | admin > default categories > update or not, MessageBus (x3) | unported | Unsupported "default_* settings" |
| 580 | admin > default tags > update or not, MessageBus (x3) | unported | Unsupported "default_* settings" |
| 636 | admin > upload site settings > remove, reset, update (x3) | covered | admin_config/logo_update, logo_unchanged (new; through LogoController, SiteSetting::Update's upload cast, log and references) |
| 681 | admin > logs the change | covered | setting_string (change_site_setting user_histories row) |
| 692 | admin > does not allow changing hidden settings | covered | setting_hidden |
| 707 | admin > does not allow globally shadowed settings | needs-harness | stubs SiteSetting.shadowed_settings |
| 723 | admin > html_message with linkified validator errors | unported | Unsupported "settings with custom validations" (validator LanguageSwitcherSettingValidator) |
| 740 | admin > plain text exception for non-admin-UI consumers | unported | no request (SiteSetting.set directly) |
| 753 | admin > plugin > configurable and non-configurable (x2) | unported | Unsupported "plugin settings" (also stubs) |
| 774 | admin > fails when a setting does not exist (provider) | covered | setting_unknown |
| 781 | moderator > prevents updates with a 404 | recordable | setting_moderator (user3 made moderator) |
| 781 | non-staff > prevents updates with a 404 | covered | setting_not_admin |
| 817 | non-staff > default categories, default tags not updated (x2) | covered | setting_not_admin (404 before anything) |
| - | (no spec it) secret setting logged as [FILTERED] | covered | setting_secret (new) |
| - | (no spec it) enum with listed choices | covered | setting_enum (new, matches) |

## Admin::Config::SiteSettingsController#index

`spec/requests/admin/config/site_settings_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 9 | not logged in > returns 404 | recordable | config_index_anonymous (no user) |
| 18 | not admin > returns 404 | covered | config_index_member (new) |
| 27 | admin > 400 when no filter_names are provided | covered | config_index_none (new) |
| 32 | admin > 400 when filter_area is invalid | covered | config_index_invalid (new) |
| 37 | admin > includes only certain allowed hidden settings | covered | config_index_names (new) |
| 56 | admin > site settings by exact name | covered | config_index_names (new) |
| 67 | admin > site settings by area | covered | config_index_flags (new) |
| - | (no spec it) by category, by the users area | covered | config_index_groups, config_index_area (new) |

## Admin::Config::LogoController

`spec/requests/admin/config/logo_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 17 | og_image_preview > admin > base64 data URI for the topic | unported | Unsupported "the topic Open Graph image preview (TopicOgImageGenerator)" |
| 26 | og_image_preview > admin > 400 without topic_id | unported | the same refusal |
| 31 | og_image_preview > admin > 404 for a missing topic | unported | the same refusal |
| 36 | og_image_preview > admin > 422 when generation fails | unported | the same refusal |
| 42 | og_image_preview > admin > 422 when login_required | unported | the same refusal |
| 49 | og_image_preview > admin > 422 for personal messages | unported | the same refusal |
| 56 | og_image_preview > admin > 422 for a read-restricted category | unported | the same refusal |
| 64 | og_image_preview > admin > does not persist an Upload | unported | the same refusal |
| 74 | og_image_preview > moderator > denies access | covered | admin_config/fonts_moderator (new) |
| 83 | og_image_preview > regular user > denies access | covered | admin_config/logo_member (new; the update and index) |
| 90 | og_image_preview > not logged in > denies access | recordable | logo_anonymous (no user) |
| - | (no spec it) update: uploads by url, cleared, unchanged | covered | admin_config/logo_update, logo_unchanged (new) |

## Admin::Config::AboutController

`spec/requests/admin/config/about_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 14 | localizations > for the requested locale | covered | admin_config/about_localizations (new) |
| 36 | localizations > no blank localizations | covered | admin_config/about_localizations (new; summary saved blank) |
| 47 | localizations > hyphenated locale names | recordable | about_localizations_hyphen (locale pt-BR supported) |
| 54 | localizations > rejects the default locale | covered | admin_config/about_localizations (new) |
| 62 | localizations > rejects when content localization is disabled | covered | admin_config/about_localizations_disabled (new) |
| 70 | localizations > rejects moderators | covered | admin_config/about_denied (new; AdminConstraint, 404) |
| 80 | update_localizations > saves and logs | covered | admin_config/about_localizations (new) |
| 125 | update_localizations > removes a blank one | covered | admin_config/about_localizations (new) |
| 144 | update_localizations > rejects unsupported locales | covered | admin_config/about_localizations (new; ja, none) |
| 156 | update_localizations > rejects when disabled | covered | admin_config/about_localizations_disabled (new) |
| 170 | update_localizations > rejects moderators | covered | admin_config/about_denied (new) |
| - | (no spec it) update: each section, the welcome topic's rewrite, invalid values unrendered | covered | admin_config/about_general, about_sections (new) |

## Admin::Config::CategoryManagementController#categories

`spec/requests/admin/config/category_management_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 19 | discussion categories with hierarchy data | covered | admin_config/category_management_list (new) |
| 46 | all, discussion-only, and typed categories | covered | admin_config/category_management_list (new; support by its custom field) |
| 108 | filters by hashtag reference | recordable | category_management_slug_ref (filter parent:child) |
| 125 | orders by name and paginates | covered | admin_config/category_management_list (new) |
| 156 | filters by visibility | covered | admin_config/category_management_list (new) |
| - | (no spec it) an unknown type, a moderator | covered | admin_config/category_management_list, category_management_denied (new) |

## Admin::Config::UpcomingChangesController

`spec/requests/admin/config/upcoming_changes_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 24 | index > non-admin gets 404 | covered | admin_config/upcoming_changes_not_staff (new) |
| 33 | index > lists upcoming changes | covered | admin_config/upcoming_changes_list (new) |
| 40 | index > includes the mocked upcoming change | needs-harness | mock_upcoming_change_metadata stubs the declared changes |
| 63 | index > group names when site setting groups are configured | covered | admin_config/upcoming_changes_toggle, upcoming_changes_groups_partial (new) |
| 85 | update_groups > non-admin gets 404 | covered | admin_config/upcoming_changes_not_staff (new) |
| 98 | update_groups > 200 on success | covered | admin_config/upcoming_changes_toggle (new) |
| 109 | update_groups > creates a site setting group record | covered | admin_config/upcoming_changes_toggle (new) |
| 122 | update_groups > updates an existing record | covered | admin_config/upcoming_changes_toggle (new) |
| 136 | update_groups > deletes an existing record | covered | admin_config/upcoming_changes_toggle (new) |
| 151 | update_groups > logs the change | covered | admin_config/upcoming_changes_toggle (new) |
| 166 | update_groups > 400 when setting is missing | covered | admin_config/upcoming_changes_toggle (new) |
| 176 | update_groups > only existing groups | covered | admin_config/upcoming_changes_groups_partial (new) |
| 274 | toggle_change > non-admin gets 404 | covered | admin_config/upcoming_changes_not_staff (new) |
| 283 | toggle_change > 200 on success | covered | admin_config/upcoming_changes_toggle (new) |
| 290 | toggle_change > false to true | covered | admin_config/upcoming_changes_toggle (new) |
| 298 | toggle_change > true to false | covered | admin_config/upcoming_changes_toggle (new) |
| 310 | toggle_change > accepts an empty JSON audience (group_names: []) while disabling, clears the site_setting_groups row | needs-harness | mock_upcoming_change_metadata stubs allow_enabled_for: [specific_groups]; no declared change has a restricted audience (all allow_enabled_for are everyone). An unrestricted change (enable_category_hashtag_cards, setup site_setting_groups row, JSON body) would approximate it as upcoming_changes_toggle_empty_audience |
| 333 | toggle_change > JSON audience (staff, trust_level_2) and enable together, group_ids 3|12 | needs-harness | mock_upcoming_change_metadata (allow_enabled_for staff + specific_groups); port does take group_names on toggle (config_upcoming_changes.rs toggle_change). Approximation with an unrestricted change: upcoming_changes_toggle_with_groups |
| 355 | toggle_change > linked change (group_setting) toggles without touching the linked setting | needs-harness | mock_upcoming_change_metadata with group_setting: about_page_extra_groups; no declared core change uses group_setting |
| 377 | toggle_change > logs the change | covered | admin_config/upcoming_changes_toggle (new) |
| 389 | toggle_change > 400 when setting_name is missing | covered | admin_config/upcoming_changes_toggle (new) |
| 396 | toggle_change > 404 when setting_name is invalid | covered | admin_config/upcoming_changes_toggle (new) |
| 406 | toggle_change > rejects settings that are not upcoming changes | covered | admin_config/upcoming_changes_toggle (new) |
| - | (no spec it) a moderator; groups a change doesn't allow; remove_and_replace_uncategorized's handler (its snapshot, the restore) | covered | admin_config/upcoming_changes_denied, upcoming_changes_toggle, upcoming_changes_uncategorized, upcoming_changes_uncategorized_restore, upcoming_changes_uncategorized_snapshot (new) |

## Admin::SearchController

`spec/requests/admin/search_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 9 | index > settings, themes and components, reports and upcoming changes | covered | admin_search/all (new) |
| 20 | index > default_locale in a general search | covered | admin_search/all, admin_search/filtered (new) |
| - | (no spec it) the contract's filters (names, areas, plugin, categories), no setting left (204), a moderator (403), a non-staff user (404) | covered | admin_search/filtered, admin_search/denied, admin_search/not_staff (new) |

## Admin::DashboardController#new_features

`spec/requests/admin/dashboard_controller_spec.rb` (the meta feed is
parity/writes/files/new_features_feed.json, recorded from
meta.discourse.org/new-features.json and served by the fixtures server)

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 930 | admin > empty by default | covered | admin_dashboard/dashboard_whats_new |
| 937 | admin > invalid JSON | covered | admin_dashboard/whats_new_invalid_feed (new) |
| 945 | admin > includes new features | covered | admin_dashboard/whats_new_feed (new) |
| 958 | admin > force refresh busts the cache | covered | admin_dashboard/whats_new_refresh, whats_new_refresh_failed (new) |
| 990 | admin > unseen feature state | covered | admin_dashboard/whats_new_feed, whats_new_refresh (new) |
| 1001 | admin > bumps the last viewed feature date | needs-harness | a Redis hash the responses don't show (the daily job's notifications read it: scheduled/check_new_features) |
| 1024 | admin > marks new features as seen | covered | admin_dashboard/whats_new_feed (the current user's has_unseen_features after) (new) |
| 1042 | admin > no new features | covered | admin_dashboard/dashboard_whats_new |
| 1077 | admin > a permanent upcoming change with an empty feed | covered | admin_dashboard/dashboard_whats_new |
| 1102 | moderator > includes new features | covered | admin_dashboard/whats_new_moderator (new) |
| 1115 | moderator > no last viewed date | needs-harness | the same Redis hash |
| 1129 | non-staff > 404 | covered | admin_dashboard/whats_new_not_staff (new) |
| - | (no spec it) toggle_feature (a moderator 403, a non-upcoming setting 403, the contract's 400); Jobs::CheckNewFeatures | covered | admin_dashboard/toggle_feature, whats_new_moderator, whats_new_not_staff, scheduled/check_new_features (new) |

## Admin::McpController

`spec/requests/admin/mcp_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 10 | overview > reports approved OAuth client registrations | covered | admin_mcp/capabilities (new) |
| 25 | primitives > the registered OAuth scopes as options | covered | admin_mcp/overview (new) |
| 42 | access > one rule per group, Admins pre-registered | covered | admin_mcp/overview, access (new) |
| 76 | update_access > replaces the scopes for one group | covered | admin_mcp/access (new) |
| 93 | update_access > rejects unknown scopes | covered | admin_mcp/access (new) |
| 102 | update_access > the Admins scopes can be changed | covered | admin_mcp/access (new) |
| 116 | update_access > not for moderators | covered | admin_mcp/denied (new) |
| 128 | destroy_access > removes the scopes for one group | covered | admin_mcp/access (new) |
| 138 | destroy_access > not the Admins rule | covered | admin_mcp/access (new) |
| 149 | emergency_block > blocks and unblocks | covered | admin_mcp/capabilities (new) |
| 164 | emergency_block > fresh consent only when unblocking expands privileged access | covered | admin_mcp/capabilities (new) |
| 202 | update_primitives > fresh consent and revoked tokens for a privileged primitive | covered | admin_mcp/capabilities (new) |
| 255 | update_primitives > no fresh consent for a read-only primitive | covered | admin_mcp/capabilities (new) |
| - | (no spec it) missing params, unknown groups and primitives | covered | admin_mcp/access, capabilities (new) |

## Admin::McpAuthorizationsController

`spec/requests/admin/mcp_authorizations_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 15 | index > the effective authorization status | covered | admin_mcp/authorizations (new; active, access removed, client blocked, revoked, user unavailable) |
| 32 | index > paginates | covered | admin_mcp/authorizations (new) |
| 50 | index > filters before the page limit | covered | admin_mcp/authorizations (new) |
| 61 | index > an authorization whose user was deleted | covered | admin_mcp/authorizations (new) |
| - | (no spec it) destroy, unknown ids, invalid limit and cursor | covered | admin_mcp/authorizations (new) |

## Admin::McpActivityController

`spec/requests/admin/mcp_activity_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 9 | index > activity with the default seven-day metrics | covered | admin_mcp/activity (new) |
| 63 | index > the selected date range | covered | admin_mcp/activity (new) |
| 102 | index > no latency without a timed tool call | covered | admin_mcp/activity (new; the request id filter) |
| 115 | index > filters users, clients, tools, outcomes and request ids | covered | admin_mcp/activity (new) |
| 165 | index > filters before pagination, no metrics with a cursor | covered | admin_mcp/activity (new) |
| 200 | index > rejects invalid filter and date values | covered | admin_mcp/activity (new) |
| 224 | index > not for moderators | covered | admin_mcp/denied (new) |

## Admin::McpClientsController

`spec/requests/admin/mcp_clients_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 13 | index > paginates | covered | admin_mcp/clients (new) |
| 31 | index > filters before the page limit | covered | admin_mcp/clients (new) |
| 43 | create > a non-URL client ID starting with https | covered | admin_mcp/clients (new) |
| 170 | refresh > refreshes the metadata of an approved domain | needs-harness | the document must be served over https from a public address; the harness's remote fixtures are http on loopback |
| - | (no spec it) show, invalid creations, block and unblock with the authorizations revoked, refresh refusals (not a metadata client, policy, blocked, root path, unreachable), moderators | covered | admin_mcp/clients, clients_refresh_refused, clients_refresh_unreachable, clients_denied (new) |

## Admin::Config::WelcomeBannerController#themes_with_setting

`spec/requests/admin/config/welcome_banner_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 19 | admin > user selectable themes and the default theme | covered | admin_config/welcome_banner_save (new; the default theme) |
| 32 | admin > excludes components | recordable | welcome_banner_components (setup: a component theme) |
| 41 | admin > the setting's value when present | covered | admin_config/welcome_banner_save (new; after the theme's PUT) |
| 57 | admin > false when the theme disables it | covered | admin_config/welcome_banner_save (new) |
| 73 | admin > the effective default when not present | covered | admin_config/welcome_banner_save (new; the first request) |
| 86 | moderator > denies access with a 403 | covered | admin_config/fonts_moderator (new) |
| 95 | non-staff > denies access with a 404 | recordable | welcome_banner_member |
| - | (no spec it) index: 204 for JSON | covered | admin_config/welcome_banner_save (new) |

## Admin::Config::FontsController

`spec/requests/admin/config/fonts_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 12 | admin > updates the fonts and text size | covered | admin_config/fonts_update, fonts_no_backfill (new) |
| 26 | admin > validates values | covered | admin_config/fonts_invalid (new) |
| 43 | moderator > denies access with a 403 | covered | admin_config/fonts_moderator (new) |
| 60 | non-staff > denies access with a 404 | covered | admin_config/logo_member (new; LogoController, the same AdminController) |

## Admin::UsersController#suspend

`spec/requests/admin/users_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 541 | admin > suspends user | covered | user_suspend |
| 541 | moderator > suspends user | covered | user_suspend_as_moderator (new, matches) |
| 566 | admin > doesn't allow suspending a staff user | covered | user_suspend_self (new, matches) |
| 566 | moderator > doesn't allow suspending a staff user | recordable | user_suspend_admin_as_moderator |
| 577 | admin, moderator > staff via other_user_ids (x2) | recordable | no case yet: ported in 7eece679 (was: Unsupported "penalizing several users at once (other_user_ids)") |
| 597 | admin > checks if user is suspended (409) | recordable | no case yet: ported in 7eece679 (was: Unsupported "suspending an already suspended user") |
| 628 | admin > webhook > enqueues user_suspended | needs-harness | web hook fabrication and EmitWebHookEvent job args; webhooks not ported |
| 648 | admin > fails if the reason is too long | recordable | user_suspend_reason_too_long (301 chars, 400) |
| 660 | admin > requires suspend_until and reason | covered | user_suspend_without_reason |
| 674 | admin > other_user_ids too big | recordable | no case yet: ported in 7eece679 (was: Unsupported "penalizing several users at once (other_user_ids)") |
| 696 | admin > associated post > have, delete, preserve category topics (x2), delete replies, edit (x6) | recordable | no case yet: ported in 7eece679 (was: Unsupported "acting on a post with a penalty") |
| 773 | admin > can send a message to the user | covered | user_suspend_with_message |
| 794 | admin > also prevents use of any api keys | recordable | user_suspend_blocks_api_key: api_keys setup row for user2, bookmark POST with Api-Key headers, suspend, bookmark again (403). Mixes the admin cookie with key headers in one case |
| 822 | admin > can silence multiple users | recordable | no case yet: ported in 7eece679 (was: Unsupported "penalizing several users at once (other_user_ids)") |
| 842 | moderator > cannot edit a static doc post (now the suspended user's own post) | recordable | user_suspend_moderator_static_doc_edit (tos_topic_id = the post's topic, post_action edit, 200 and raw unchanged). Was unported: users/penalties.rs now ports post_action edit behind can_edit_post, whose TopicCtx knows the static doc topics |
| 863 | non-staff > 404 | covered | user_suspend_not_admin |

## Admin::UsersController#unsuspend

`spec/requests/admin/users_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 891 | admin > webhook > enqueues user_unsuspended | needs-harness | web hook fabrication, job args; webhooks not ported |
| 905 | admin > unsuspends a user granted moderation while suspended | recordable | user_unsuspend_moderator (user3 suspended and made moderator in setup) |
| 926 | moderator > prevents unsuspending a staff user (admin, moderator) | covered | user_unsuspend_staff_as_moderator (new, matches) |
| 940 | moderator > can unsuspend a regular user | recordable | user_unsuspend_as_moderator |

## Admin::UsersController#silence

`spec/requests/admin/users_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 2268 | admin > 404 if the user doesn't exist | covered | user_silence_missing_user (new) |
| 2273 | admin > doesn't allow silencing another admin | recordable | user_silence_self (admin target, 403) |
| 2283 | admin > another admin via other_user_ids | recordable | no case yet: ported in 7eece679 (was: Unsupported "penalizing several users at once (other_user_ids)") |
| 2295 | admin > punishes the user for spamming | covered | user_silence |
| 2307 | admin > can have an associated post (post_action edit) | recordable | no case yet: ported in 7eece679 (was: Unsupported "acting on a post with a penalty") |
| 2335 | admin > sets the provided duration (date only) | recordable | user_silence_date_only (silenced_till "2027-01-01") |
| 2349 | admin > sends the provided message | covered | user_silence_with_message (new, matches) |
| 2364 | admin > checks if user is silenced (409) | recordable | no case yet: ported in 7eece679 (was: Unsupported "silencing a user with a silence on record") |
| 2392 | admin > can silence multiple users | recordable | no case yet: ported in 7eece679 (was: Unsupported "penalizing several users at once (other_user_ids)") |
| 2404 | admin > fails if the reason is too long | recordable | user_silence_reason_too_long |
| 2416 | admin > other_user_ids too big | recordable | no case yet: ported in 7eece679 (was: Unsupported "penalizing several users at once (other_user_ids)") |
| 2441 | moderator > silences user | recordable | user_silence_as_moderator |
| 2454 | moderator > doesn't allow silencing another admin | covered | user_silence_admin_as_moderator (new, matches) |
| 2464 | moderator > another admin via other_user_ids | recordable | no case yet: ported in 7eece679 (was: Unsupported "penalizing several users at once (other_user_ids)") |
| 2481 | non-staff > 404 | recordable | user_silence_not_staff |

## Admin::UsersController#unsilence

`spec/requests/admin/users_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 2496 | admin, moderator > 404 if the user doesn't exist (x2) | recordable | user_unsilence_missing_user |
| 2501 | admin > unsilences the user | covered | user_unsilence |
| 2501 | moderator > unsilences the user | recordable | user_unsilence_as_moderator |
| 2526 | moderator > prevents unsilencing a staff user (admin, moderator) | recordable | user_unsilence_staff_as_moderator (admin silenced in setup) |
| 2543 | non-staff > 404 | recordable | user_unsilence_not_staff |

## Admin::EmailController#handle_mail

`spec/requests/admin/email_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 299 | admin > 400 if neither email parameter is present | recordable | incoming_missing_param |
| 305 | admin > enqueues with the deprecated email param, warns | covered | incoming_deprecated_email_param (new, matches) |
| 320 | admin > decodes email_encoded and enqueues | covered | incoming_reply |
| 338 | admin > cross-group In-Reply-To (group_smtp incoming email of another group's PM) does not grant access | unported | email_in on: Unsupported("group and category addresses (email_in)") in email/receiver.rs; also signs in user, then admin, then user again |
| 397 | admin > normalizes invalid UTF-8 bytes | covered | incoming_invalid_utf8 (new) |
| 406 | admin > enqueues the email job when the site is archived | recordable | incoming_site_archived (site_archived setting, email_encoded cc mail, 200 + process_email job); admin/routes/email.rs handle_mail has no site_archived check, so it should match |
| 418 | admin > bounds the body of a message sent to a group inbox (max_post_length 100) | unported | sender unknown: Unsupported("emails from unknown senders (staged users)"), then Unsupported("group and category addresses (email_in)") (email/receiver.rs); Unsupported("truncating a long incoming email") in email/incoming.rs too |
| 447 | moderator > 404 | recordable | incoming_moderator |
| 447 | non-staff > 404 | recordable | incoming_not_staff |

## Admin::EmailTemplatesController#update

`spec/requests/admin/email_templates_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 153 | admin > unknown template 404 | covered | admin_email/email_templates_update_invalid (new, matches) |
| 170 | admin > invalid: the error messages | covered | admin_email/email_templates_update_invalid (new, matches) |
| 188 | admin > invalid: no overrides created | covered | admin_email/email_templates_update_invalid (new, matches) |
| 202 | admin > invalid: no staff log | covered | admin_email/email_templates_update_invalid (new, matches) |
| 220 | admin > invalid subject, body or both, malformed keys | covered | admin_email/email_templates_update, email_templates_update_invalid (new, match) |
| 307 | admin > valid: the updated template | covered | admin_email/email_templates_update (new, matches) |
| 332 | admin > valid: overrides created | covered | admin_email/email_templates_update (new, matches) |
| 346 | admin > valid: staff log entries | covered | admin_email/email_templates_update (new, matches) |
| 373 | admin > plural keys: the subject left alone | covered | admin_email/email_templates_update_plural (new, matches) |
| 396 | admin > plural keys: validation errors, no overrides | recordable | email_templates_update_plural_invalid |
| 447 | moderator, non-staff > 404 | recordable | email_templates_update_not_admin |

## Admin::EmailTemplatesController#revert

`spec/requests/admin/email_templates_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 483 | admin > unknown template 404 | covered | admin_email/email_templates_revert_unknown |
| 506 | admin > with overrides: the originals restored | covered | admin_email/email_templates_update (new, matches) |
| 516 | admin > with overrides: the restored template | covered | admin_email/email_templates_update (new, matches) |
| 533 | admin > with overrides: staff log entries | covered | admin_email/email_templates_update (new, matches) |
| 555 | moderator > 404 | recordable | email_templates_revert_moderator |
| 555 | non-staff > 404 | recordable | email_templates_revert_not_staff |

## Admin::RobotsTxtController#show

`spec/requests/admin/robots_txt_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 12 | admin > default content without overrides | covered | robots/admin (new, matches) |
| 20 | admin > overridden content | covered | robots/admin (new, matches) |
| 31 | moderator > 404 | covered | robots/moderator (new, matches) |
| 31 | non-staff > 404 | recordable | robots_not_staff |

## Admin::RobotsTxtController#update

`spec/requests/admin/robots_txt_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 56 | admin > overrides the default robots.txt | covered | robots/admin (new, matches) |
| 68 | admin > logs the change | covered | robots/admin (new, matches) |
| 83 | admin > robots_txt param required | covered | robots/admin (new, matches) |
| 91 | moderator > 404 | covered | robots/moderator (new, matches) |
| 91 | non-staff > 404 | recordable | robots_not_staff |

## Admin::RobotsTxtController#reset

`spec/requests/admin/robots_txt_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 117 | admin > resets to the default | covered | robots/admin, robots/no_index (new, match) |
| 133 | admin > logs the reset | covered | robots/admin (new, matches) |
| 150 | moderator > 404 | covered | robots/moderator (new, matches) |
| 150 | non-staff > 404 | recordable | robots_not_staff |

## UsersController#create

`spec/requests/users_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 28 | full account registration flow > handles the honeypot and challenge fields | covered | signup |
| 925 | rejects signup from a logged-in browser session | covered | signup_logged_in (new, matches) |
| 933 | rejects signup from an admin browser session | recordable | signup_logged_in_admin (same as above with user admin) |
| 941 | allows signup with an API key | recordable | signup_api_key (setup SQL api_keys row for user1, Api-Key header); port rejects every authenticated request (accounts.rs:775), Rails skips honeypot for API |
| 952 | anonymous registration > only show_on_signup user fields kept | recordable | signup_hidden_user_field_ignored (setup SQL user_fields rows, one show_on_signup false); port's users/fields.rs signup_values takes every field, so it would save the hidden one |
| 972 | anonymous registration > hidden on_signup-required field skipped | recordable | signup_hidden_required_field (setup SQL user_fields show_on_signup false, requirement on_signup); port's signup_values requires it and fails the signup |
| 981 | regular user API key > only signup fields kept | recordable | signup_api_key_hidden_field (setup SQL api_keys row for user1 + user_fields); port 403s any authenticated signup (routes/accounts.rs:1077), as row 873 |
| 997 | admin API key > hidden fields accepted | recordable | signup_admin_api_key_hidden_field (setup SQL api_keys row for admin + user_fields); same port 403 (routes/accounts.rs:1077) |
| 1014 | email param missing > 400 | recordable | signup_missing_email |
| 1026 | encoded email decoding to invalid email > blocked | unported | signup.rs:278 invalid email Unsupported |
| 1039 | encoded email decoding to valid email > blocked | unported | signup.rs:278 |
| 1052 | sets the user locale to I18n.locale | needs-harness | I18n.stubs(:locale) |
| 1059 | requires invite code when specified | unported | signup.rs:175 invite codes |
| 1077 | timezone guess on signup > sets the timezone | recordable | signup_timezone |
| 1093 | protected profile attributes ignored on unauthenticated signup | covered | signup_ignores_protected_fields (new, matches) |
| 1117 | discourse connect enabled > blocks local registration | recordable | signup_discourse_connect (settings discourse_connect_url, discourse_connect_secret, enable_discourse_connect in that order) |
| 1134 | local logins disabled > blocks without authenticator | unported | signup.rs:224 local logins off |
| 1139 | local logins disabled > blocks with a regular api key | unported | signup.rs:224 |
| 1145 | local logins disabled > works with an admin api key | unported | signup.rs:224 |
| 1183 | external_ids > creates User record | needs-harness | plugin auth provider registration + UserAuthenticator stub |
| 1209 | external_ids > error for unknown provider | needs-harness | plugin auth provider registration |
| 1222 | external_ids > granular users:create API key > 403, staged user untouched | recordable | signup_external_ids_granular_key (setup SQL staged user, api_keys + api_key_scopes users:create for admin); ensure_external_ids_allowed! runs before any provider lookup so no plugin provider is needed; port's blanket 403 for authenticated signups (routes/accounts.rs:1077) likely matches |
| 1253 | non active user > 403 when local logins disabled | unported | signup.rs:224 |
| 1260 | non active user > error when new registrations are disabled | recordable | signup_registrations_disabled (allow_new_registrations false) |
| 1271 | non active user > creates a user correctly | covered | signup |
| 1290 | must approve users > creates a user correctly | unported | signup.rs:215 approval at signup |
| 1318 | normalize_emails + hide_email_address_taken > account_exists email | unported | signup.rs:299 taken email |
| 1345 | email exists > error if hide_email_address_taken disabled | unported | signup.rs:299 |
| 1356 | email exists > success if hide_email_address_taken enabled | unported | signup.rs:299 |
| 1393 | creating as active > does not create the user as active | covered | signup_ignores_protected_fields (new, matches) |
| 1402 | active > regular api key does not activate | recordable | signup_api_key_active (setup SQL api key for user1) |
| 1417 | active > admin api key creates active approved user | unported | must_approve_users at signup, signup.rs:215 |
| 1441 | active > admin api key > reviewable for active unapproved user | unported | signup.rs:215 |
| 1460 | active > admin api key > no reviewable for inactive user | unported | signup.rs:215 |
| 1475 | active > admin api key > developer not active | needs-harness | UsernameCheckerService.expects |
| 1488, 1509 | active > admin api key > developer email with different case / surrounding whitespace not activated (x2) | needs-harness | Rails.configuration.stubs(:developer_emails) |
| 1530 | active > admin api key > does not copy admin locale | recordable | signup_api_admin_locale (setup SQL admin api key, UPDATE users SET locale='fr' WHERE id=1, allow_user_locale) |
| 1546 | active > admin api key > auto approves configured domain | unported | signup.rs:215 auto_approve_email_domains |
| 1569 | creating as staged > does not create the user as staged | covered | signup_ignores_protected_fields (new, matches) |
| 1579 | staged > regular api key does not stage | recordable | signup_api_key_staged (setup SQL api key) |
| 1596 | staged > admin api key creates staged user | recordable | signup_api_admin_staged (setup SQL admin api key) |
| 1608 | staged > developer not staged | needs-harness | UsernameCheckerService.expects |
| 1626 | active user (confirmed email) > enqueues a welcome email | needs-harness | User.any_instance.stubs(:active?) and expects(:enqueue_welcome_message) |
| 1637 | active user > shows the 'active' message | needs-harness | same stubs |
| 1644 | active user > logs in the new user | needs-harness | same stubs |
| 1651 | active user > indicates active in response | needs-harness | same stubs |
| 1658 | active user > fails when new registrations disabled | recordable | signup_registrations_disabled (fail_with runs before the stubbed part) |
| 1694 | auth records > creates Twitter user information | needs-harness | OmniAuth mock |
| 1707 | auth records > error when email changed from validated | needs-harness | OmniAuth mock |
| 1722 | auth records > creates user when email validation required | needs-harness | OmniAuth mock |
| 1736 | auth records > auth_overrides username/name | needs-harness | OmniAuth mock |
| 1776 | no email in auth payload > creates the user | needs-harness | OmniAuth mock |
| 1794 | creates user successfully but doesn't activate | covered | signup |
| 1832 | honeypot value wrong > does not create user | covered | signup_failed_challenge |
| 1848 | challenge answer wrong > does not create user | covered | signup_failed_challenge |
| 1863 | invite only > honeypot-style fake success | covered | invites/signup_invite_only |
| 1888 | password blank > failed signup (x2) | unported | signup.rs:302 password errors |
| 1901 | password too long > failed signup (x2) | recordable | signup_password_too_long (201 chars, fail_with) |
| 1907 | username too long (50000) > rejected without reflection | unported | signup.rs:190 overlong usernames |
| 1920 | password missing > failed signup (x2) | unported | signup.rs:302 |
| 1930 | reserved username > failed signup (x2) | covered | signup_reserved_username (new, matches) |
| 1943 | username matching a user route > failed signup (x2) | recordable | signup_route_username (username account-created) |
| 1949 | missing username > 400 | recordable | signup_missing_username |
| 1967 | Exception raised on save > failed signup (x2) | needs-harness | User.any_instance.stubs(:save).raises |
| 1985 | unknown enum user field value > failed signup (x2) | unported | signup.rs:221 user fields |
| 1998 | custom fields without values > failed signup (x2) | unported | signup.rs:221 |
| 2021 | user fields > multiselect > single value or array | recordable | no case yet: the user updater only refuses custom_fields now (src/users/operations/updater.rs REFUSED) (was: user_updater.rs REFUSED user_fields) |
| 2031 | user fields > multiselect > rejects unregistered values | recordable | no case yet: the user updater only refuses custom_fields now (src/users/operations/updater.rs REFUSED) (was: user_updater.rs REFUSED user_fields) |
| 2037 | user fields > multiselect > filters valid values | unported | same |
| 2043 | user fields > multiselect > allows registered values | unported | same |
| 2049 | user fields > multiselect > required on signup | unported | signup.rs:221 |
| 2063 | user fields > multiselect > required only on sign-up | unported | same |
| 2077 | user fields > multiselect > not required may be empty | unported | same |
| 2108 | user fields > dropdown > rejects unregistered values | unported | same |
| 2114 | user fields > dropdown > allows registered values | unported | same |
| 2120 | user fields > dropdown > required can't be nil | unported | same |
| 2130 | user fields > dropdown > optional may be nil | unported | same |
| 2154 | user fields > creates without optional field | unported | signup.rs:221 |
| 2165 | user fields > creates with optional field | unported | signup.rs:221 |
| 2177 | user fields > trims long fields | unported | signup.rs:221 |
| 2202 | only optional custom fields > creates without values | unported | signup.rs:221 |
| 2223 | staged account > claims it | unported | signup.rs:207 staged takeover |
| 2247 | staged account > works with custom fields | unported | signup.rs:207/221 |

## UsersController#perform_account_activation

`spec/requests/users_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 65 | inexistent token > 422 | covered | activate_account_unknown_token (new, matches) |
| 72 | invalid token (`123%2f%252e`) > 404 | covered | activate_account_bad_token_format (new) |
| 80 | valid token > welcome message enqueued | covered | activate_account (signup then activation of the inactive user) |
| 91 | valid token > already active user gets no welcome message | recordable | activate_account_already_active (setup SQL signup-scope email_tokens row with a known raw token for user0) |
| 100 | valid token > invalid honeypot > 403 | recordable | activate_account_failed_challenge (wrong challenge, any token) |
| 108 | valid token > correctly logs on user | covered | activate_account |
| 131, 142, 153 | valid token > second factor enabled (TOTP, backup codes, security key) > activates without logging in, redirect_to /login (x3) | recordable | activate_account_second_factor (setup SQL user_second_factors / user_security_keys rows plus a known signup token); port always logs on and never redirects to /login (routes/accounts.rs perform_account_activation) |
| 168 | valid token > not approved > pending-approval response | unported | accounts.rs:650 must_approve_users |
| 180 | valid token > already logged in > 404 (GET) | unported | GET is users#activate_account, not routed; the PUT logged-in branch (accounts.rs:860) is recordable as activate_account_logged_in |
| 192 | destination_url cookie with query > redirect_to | needs-harness | request cookie |
| 204 | destination_url cookie > redirect_to | needs-harness | request cookie |
| 226 | invited to topic > redirects to the topic | recordable | activate_account_invited_topic (setup SQL invites, topic_invites, invited_users for user0, plus a known signup token); port always answers redirect_to null |

## UsersController#email_login

`spec/requests/users_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 7566 | enqueues the right email | covered | email_login (by username; spec uses the email, same lookup) |
| 7583 | staff writes only > moderator gets email | needs-harness | readonly mode in Redis |
| 7594 | staff writes only > admin gets email | needs-harness | readonly mode |
| 7605 | staff writes only > regular user 503 | needs-harness | readonly mode |
| 7617 | enable_local_logins_via_email disabled > 404 | covered | email_login_disabled (new, matches) |
| 7624 | invalid username or email > user_found false, no job | recordable | email_login_unknown (hide_email_address_taken false, login `@random`) |
| 7636 | hide_email_address_taken > generic response | covered | email_login (default hide_email_address_taken is true) |
| 7648 | already logged in > redirects to root | covered | email_login_logged_in (new) |

## UsersController#password_reset_update (PUT /u/password-reset/:token)

`spec/requests/users_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 378 | invalid token > proper error message | unported | accounts.rs:433 unknown token |
| 403 | valid token > returns success, auth tokens removed | covered | password_reset |
| 427 | valid token > PM pushes stop to previously authorized browsers | needs-harness | allow(WebPush).to receive + runs Jobs::DeliverPushNotification |
| 456 | valid token > disallows double password reset | unported | second PUT hits accounts.rs:433 |
| 470 | valid token > first admin redirected to wizard | unported | HTML format (redirect_to wizard_path) |
| 481 | valid token > sets the timezone if missing | covered | password_reset_timezone (new) |
| 495 | valid token > deletes user associated accounts | covered | password_reset_associated_accounts (new, matches) |
| 520 | valid token > logs the password change | covered | password_reset |
| 542 | previewed token superseded by forgot_password | unported | accounts.rs:433 |
| 565 | rate limits reset passwords | needs-harness | rate limits + freeze_time |
| 587 | rate limits by username | needs-harness | rate limits + REMOTE_ADDR |
| 619 | TOTP required > invalid token does not change | unported | accounts.rs:443 second factors |
| 645 | TOTP required > valid token changes password | unported | accounts.rs:443 |
| 696 | security key > valid challenge changes password | unported | accounts.rs:443 |
| 710 | security key > fake TOTP token rejected | unported | accounts.rs:443 |
| 723 | security key > authentication fails | unported | accounts.rs:443 |
| 749 | submit change > fails when password blank | unported | accounts.rs:476 error responses |
| 757 | submit change > fails when password too long | unported | accounts.rs:476 |
| 768 | submit change > logs in the user | covered | password_reset |
| 776 | submit change > not approved, not logged in | unported | accounts.rs:650 must_approve_users in EmailToken.confirm |
| 788 | staff writes only > staff can reset | needs-harness | readonly mode |
| 802 | staff writes only > non-staff blocked | needs-harness | readonly mode |

## UsersController#update (PUT /u/:username)

`spec/requests/users_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 3396 | guest > 403 | recordable | users_update_anonymous (user null, PUT /u/user0.json) |
| 3402 | auth_overrides_name > name not updated | recordable | users_update_auth_overrides_name |
| 3418 | username with a period > updates | recordable | users_update_dotted_username (setup SQL renames user0 to user.zero after the harness login) |
| 3430 | staff > uneditable user field and title | recordable | no case yet: the user updater only refuses custom_fields now (src/users/operations/updater.rs REFUSED) (was: user_updater.rs REFUSED user_fields, :295 title) |
| 3463 | allows the update (muted, tags, backgrounds, languages) | unported | tag tracking (:335), backgrounds (REFUSED), array options (:705); the muted part is covered by prefs_muted |
| 3518 | does not update username, email, password | covered | users_update_ignores_protected_fields (new, matches) |
| 3535 | watched tags in everyone tag group | unported | user_updater.rs:335 |
| 3556 | locale differs > updates the locale | recordable | users_update_locale (allow_user_locale true, locale fa_IR) |
| 3562 | locale differs > updates the title | unported | user_updater.rs:295 |
| 3582 | editable user field > updates | recordable | no case yet: the user updater only refuses custom_fields now (src/users/operations/updater.rs REFUSED) (was: REFUSED user_fields) |
| 3595 | editable user field > cannot be blank | recordable | no case yet: the user updater only refuses custom_fields now (src/users/operations/updater.rs REFUSED) (was: REFUSED user_fields) |
| 3608 | editable user field > trims large fields | recordable | no case yet: the user updater only refuses custom_fields now (src/users/operations/updater.rs REFUSED) (was: REFUSED user_fields) |
| 3620 | editable user field > retains existing | recordable | no case yet: the user updater only refuses custom_fields now (src/users/operations/updater.rs REFUSED) (was: REFUSED user_fields) |
| 3652 | notification schedule | recordable | no case yet: the user updater only refuses custom_fields now (src/users/operations/updater.rs REFUSED) (was: REFUSED user_notification_schedule) |
| 3686 | uneditable user field not updated | recordable | no case yet: the user updater only refuses custom_fields now (src/users/operations/updater.rs REFUSED) (was: REFUSED user_fields) |
| 3709 | custom_field > only allowed fields | needs-harness | plugin register_editable_user_custom_field (port also refuses custom_fields) |
| 3725 | custom_field > alongside a user field | needs-harness | plugin registration |
| 3745 | custom_field > alongside a user field during creation (API) | needs-harness | plugin registration |
| 3775 | custom_field > secure with no registered fields | unported | REFUSED custom_fields |
| 3794 | custom_field > staff edits staff-editable fields | needs-harness | plugin registration |
| 3813 | returns user JSON | covered | prefs_text_size (and the other prefs_* cases) |
| 3823 | sidebar > links kept when params absent | covered | prefs_profile (user1 has seeded sidebar links, no sidebar params) |
| 3834 | sidebar > remove all category links | recordable | no case yet: the user updater only refuses custom_fields now (src/users/operations/updater.rs REFUSED) (was: REFUSED sidebar_category_ids) |
| 3844 | sidebar > category links only for accessible categories | recordable | no case yet: the user updater only refuses custom_fields now (src/users/operations/updater.rs REFUSED) (was: REFUSED sidebar_category_ids) |
| 3879 | sidebar > remove all tag links | recordable | no case yet: the user updater only refuses custom_fields now (src/users/operations/updater.rs REFUSED) (was: REFUSED sidebar_tag_names) |
| 3891 | sidebar > tag links rejected when tagging off | recordable | no case yet: the user updater only refuses custom_fields now (src/users/operations/updater.rs REFUSED) (was: REFUSED sidebar_tag_names) |
| 3902 | sidebar > tag links only for browsable tags | recordable | no case yet: the user updater only refuses custom_fields now (src/users/operations/updater.rs REFUSED) (was: REFUSED sidebar_tag_names) |
| 3939 | without permission > forbidden | covered | prefs_other_user |
| 3980 | external_ids > create UserAssociatedAccount | recordable | no case yet: the user updater only refuses custom_fields now (src/users/operations/updater.rs REFUSED) (was: REFUSED external_ids) |
| 3998 | external_ids > destroy UserAssociatedAccount | recordable | no case yet: the user updater only refuses custom_fields now (src/users/operations/updater.rs REFUSED) (was: REFUSED external_ids) |
| 4014 | external_ids > create SingleSignOnRecord | recordable | no case yet: the user updater only refuses custom_fields now (src/users/operations/updater.rs REFUSED) (was: REFUSED external_ids) |
| 4028 | external_ids > update SingleSignOnRecord | recordable | no case yet: the user updater only refuses custom_fields now (src/users/operations/updater.rs REFUSED) (was: REFUSED external_ids) |
| 4046 | external_ids > delete SingleSignOnRecord | recordable | no case yet: the user updater only refuses custom_fields now (src/users/operations/updater.rs REFUSED) (was: REFUSED external_ids) |
| 4064 | external_ids > both in one call | recordable | no case yet: the user updater only refuses custom_fields now (src/users/operations/updater.rs REFUSED) (was: REFUSED external_ids) |
| 4098 | external_ids > unknown provider error | recordable | no case yet: the user updater only refuses custom_fields now (src/users/operations/updater.rs REFUSED) (was: REFUSED external_ids) |
| 4116, 4131 | external_ids > granular users:update API key > can't bind SSO identity / associated account to another account, 403 (x2) | recordable | users_update_external_ids_granular_key (setup SQL api_keys + api_key_scopes users:update for admin; enable_discourse_connect settings for the SSO one, an enabled managed provider for the other since the 403 precedes the provider check); port applies external_ids for any admin API request (routes/user_update.rs:120), no granular-key guard |
| 4155-4323 | user status (sets/updates/clears, other users, disabled, staff) (x14) | recordable | no case yet: the user updater only refuses custom_fields now (src/users/operations/updater.rs REFUSED) (was: REFUSED status) |
| 4341 | plugin users_controller_update_user_params modifier | needs-harness | plugin modifier |
| - | (extra, no spec it) enable_names off > name ignored | covered | users_update_names_disabled (new, matches) |

## UsersController#show (GET /u/:username)

`spec/requests/users_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 6255 | anon > returns success | covered | golden: GET /u/system.json |
| 6266 | anon > returns a hidden profile | recordable | users_show_hidden_profile (setup SQL user_options.hide_profile for user1, ?skip_track_visit=true) |
| 6279 | anon > 403 when profiles hidden from public | recordable | users_show_hidden_from_public (hide_user_profiles_from_public) |
| 6288 | anon > 403 to crawlers | needs-harness | HTML crawler page body, Googlebot UA |
| 6297 | anon > tracks a profile view | needs-harness | UserProfileView.expects; Rails adds the view in Scheduler::Defer |
| 6303 | anon > skips tracking | needs-harness | expects(:add).never |
| 6313 | logged in > returns success | covered | golden: GET /u/user1.json as=user1 |
| 6323 | logged in > unknown username not found | covered | golden: GET /u/nobody.json |
| 6329 | logged in > inactive user not found | recordable | users_show_inactive (setup SQL user2 active=false, user0 views) |
| 6336 | logged in > show_inactive_accounts shows inactive | recordable | users_show_inactive_accounts (port's find_active ignores the setting); also admin viewing an inactive user |
| 6343 | logged in > invalid access 403 | needs-harness | Guardian.any_instance.expects(:can_see?) |
| 6351 | logged in > tracks a signed-in view | needs-harness | expects |
| 6360 | logged in > own profile not tracked | needs-harness | expects |
| 6365 | logged in > skips tracking | needs-harness | expects |
| 6374 | by external_id > matching | unported | /u/by-external not routed: routes/users.rs:92 unknown /u route |
| 6380 | by external_id > not matching | unported | routes/users.rs:92 |
| 6396 | external provider > non-admin 403 | unported | routes/users.rs:92 |
| 6402 | external provider > fetch the user | unported | routes/users.rs:92 |
| 6408 | external provider > disabled provider 404 | unported | routes/users.rs:92 |
| 6414 | external provider > missing user 404 | unported | routes/users.rs:92 |
| 6430 | include_post_count_for > visible posts only | unported | routes/users.rs:191 |
| 6436 | include_post_count_for > no access | unported | routes/users.rs:191 |
| 6445 | include_post_count_for > staff all post types | unported | routes/users.rs:191 |
| 6456 | returns the user (HTML) | needs-harness | HTML profile page body |
| 6463 | private profile not in HTML | needs-harness | HTML profile page body |
| 6476 | username with a period (HTML) | needs-harness | HTML profile page body |
| 6495 | auth token IP > moderator without can_see_ip | needs-harness | DiscourseIpInfo mmdb |
| 6508 | auth token IP > admin sees client_ip | needs-harness | DiscourseIpInfo mmdb |
| 6518 | auth token IP > own tokens | needs-harness | DiscourseIpInfo mmdb |

## UsersController#summary

`spec/requests/users_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 5701 | caches per automatic translation preference | needs-harness | translate cookie, Accept-Language, topic_localizations, Rails summary cache |
| 5717 | generates summary info | covered | golden: GET /u/user1/summary.json |
| 5751 | hidden posts with links excluded | needs-harness | fabricated linked hidden/deleted posts; Rails summary cache not reset between cases |
| 5768 | hide_user_profiles_from_public > 200 for logged in | recordable | users_summary_hidden_from_public_logged_in (port 403s everyone, routes/users.rs:198) |
| 5776 | hide_user_profiles_from_public > 403 anonymous | recordable | users_summary_hidden_from_public |
| 5786 | hide_profile > 404 | recordable | users_summary_hidden_profile (setup SQL hide_profile for user1) |
| 5791 | hide_profile > 200 when allow_users_to_hide_profile off | recordable | users_summary_hide_profile_disallowed (status is safe, body may be cached) |
| 5800 | flair > automatic groups flair | needs-harness | fabricated liker + likes; summary cache |
| 5812 | flair > icon flair | needs-harness | fabricated group + likes; summary cache |
| 5833 | flair > image flair | needs-harness | fabricated group + upload + likes |
| 5864 | content localization > localized titles | needs-harness | I18n.stubs(:locale) |

## UserActionsController#index

`spec/requests/user_actions_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 10 | username missing > 400 | covered | golden: GET /user_actions.json |
| 38 | more than max page size > 100 | needs-harness | 101 fabricated actions |
| 57 | renders list correctly | covered | golden: GET /user_actions.json?username=user1 |
| 64 | hide_user_profiles_from_public > 404 | recordable | user_actions_hidden_from_public |
| 73 | lazy load categories > returns categories | recordable | user_actions_lazy_categories (lazy_load_categories_groups) |
| 91 | acting_username filters results | covered | golden: GET /user_actions.json?username=USER1&acting_username=user1 |
| 106 | hidden profile, hiding disallowed > 200 | recordable | user_actions_hide_profile_disallowed (setup SQL hide_profile) |
| 113 | hidden profile > 404 | recordable | user_actions_hidden_profile |
| 124 | other user > private types omitted without filter | covered | golden: GET /user_actions.json?username=user0 as=user1 |
| 148 | anonymous > private types 404 (xN) | recordable | user_actions_private_anonymous |
| 163 | logged in > private types 404 (xN) | recordable | user_actions_private_other_user |
| 178 | moderator > private types 404 (xN) | recordable | user_actions_private_moderator (setup SQL user3 moderator after login) |
| 193 | admin > private types 200 (xN) | recordable | user_actions_private_admin |
| 219, 228, 237, 246 | bad hash data for filter/username/offset/limit (x4) | recordable | user_actions_bad_params (loop unrolled into four contexts, same assertion) |
| 466 | content localization > translated title and excerpt | recordable | user_actions_localized (setup SQL topic/post localizations, viewer locale ja) |
| 476 | localization disabled > no preloading | needs-harness | query counting |
| 486 | localization > translated excerpt of deleted post to staff | recordable | user_actions_localized_deleted_staff |

## SessionController#create (POST /session)

`spec/requests/session_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 3566, 3590-3595 | JSON password that isn't a string (hash, array, integer, float, true, false) > incorrect password, not logged in (x6) | recordable | session_login_json_non_string_password (JSON body); port's parse_form drops hash/array via params::scalar (routes/session.rs:91), so those answer 400 param missing where Rails answers 200 incorrect password; numbers/booleans are stringified and likely match |
| 3581, 3597-3599 | JSON password nil, {}, [] > 400 (x3) | recordable | session_login_json_missing_password (JSON body); port's blank login/password check (routes/session.rs:291) should answer the same 400 |
| 3608 | read only > regular user blocked | needs-harness | readonly mode |
| 3613 | read only > admin blocked | needs-harness | readonly mode |
| 3626 | staff writes only > admin allowed | needs-harness | readonly mode |
| 3632 | staff writes only > regular blocked | needs-harness | readonly mode |
| 3645 | local login disabled > 403 | recordable | session_login_local_logins_disabled |
| 3657 | SSO enabled > 403 | recordable | session_login_discourse_connect |
| 3666 | local login via email disabled > logs in | recordable | session_login_email_logins_off |
| 3676 | login missing > 400 | covered | golden: POST /session body=login=user1 (same param.require path) |
| 3682 | invalid password | covered | golden: POST /session body=login=user1&password=wrong |
| 3691 | overlong password | recordable | session_login_overlong_password |
| 3707 | suspended > suspension error | unported | session.rs:283 |
| 3725 | suspended forever | unported | session.rs:283 |
| 3743 | deactivated > activation error | covered | session_login_inactive (new, matches) |
| 3755 | success by username and password | covered | session_login_timezone (new) |
| 3782 | timezone param sets user_option timezone | covered | session_login_timezone (new) |
| 3799 | password expired > expired error | recordable | session_login_expired_password (setup SQL user_passwords.password_expired_at) |
| 3832 | security key only > blank params | covered | session_login_security_key_refusals (new, matches) |
| 3852 | security key only > invalid params | covered | session_login_security_key_refusals (new, matches) |
| 3875 | security key only > valid > logs in | needs-harness | the challenge is random; tests/webauthn.rs signs with the reference's recorded credential |
| 3912 | security key disabled in background, TOTP on | covered | session_login_security_key_staged (new, matches: the staged challenge, then TOTP with a key enrolled) |
| 3937 | TOTP > token missing > missing-second-factor error | covered | session_login_second_factor_needed (new, matches) |
| 3949 | TOTP > invalid TOTP token | covered | session_login_totp_invalid (new) |
| 3966 | TOTP > invalid backup code | covered | session_login_backup_code_spent (new, matches) |
| 3985 | TOTP > valid token logs in | covered | session_login_totp, session_login_totp_reused (new, `{{totp:SECRET}}`, match) |
| 4006 | TOTP > valid token > second client replaying the same token rejected | covered | session_login_totp_reused (reuse simulated with last_used set ahead rather than freeze_time + reset!; same invalid_second_factor_code rejection) |
| 4035 | TOTP > valid backup code logs in | covered | session_login_backup_code (new, matches) |
| 4060 | blocked IP | unported | session.rs:291 screened IPs |
| 4078 | strips leading @ | recordable | session_login_at_prefix |
| 4093 | login by email | recordable | session_login_by_email |
| 4105 | strips spaces from username | recordable | session_login_padded_username |
| 4111 | strips spaces from email | recordable | session_login_padded_email |
| 4127 | requires approval > unapproved not logged in | covered | session_login_not_approved (new, matches) |
| 4133 | requires approval > not approved message | covered | session_login_not_approved (new, matches) |
| 4140 | requires approval > unapproved admin logs in | recordable | session_login_not_approved_admin |
| 4158 | admin ip allowlist > admin at allowed IP | recordable | session_login_admin_ip_allowed (use_admin_ip_allowlist; 127.0.0.0/8 allow_admin is in the seed) |
| 4175 | admin ip allowlist > admin elsewhere blocked | needs-harness | REMOTE_ADDR |
| 4191 | admin ip allowlist > non-admin elsewhere ok | needs-harness | REMOTE_ADDR |
| 4214 | email not confirmed > not logged in | recordable | session_login_unconfirmed (setup SQL email_tokens confirmed=false for user1) |
| 4221 | email not confirmed > not activated message | recordable | session_login_unconfirmed |
| 4230 | email not confirmed + must approve > not approved message | recordable | session_login_unconfirmed_not_approved |
| 4241 | rate limits login | needs-harness | rate limits |
| 4259 | rate limits 2FA by IP | needs-harness | rate limits |
| 4285 | rate limits 2FA by login | needs-harness | rate limits |
| 4326 | rate limited > wrong-password attempts don't count toward the per-account 2FA limit | needs-harness | rate limits + REMOTE_ADDR |

## SessionController#destroy (DELETE /session/:username)

`spec/requests/session_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 4370 | removes session and auth token cookies (non-XHR 302) | recordable | session_logout_redirect (headers X-Requested-With "" ; status and the user_auth_tokens delete compare, Location does not) |
| 4379 | XHR returns redirect_url | recordable | session_logout (user0, DELETE /session/user0.json) |
| 4391 | SSO + login_required > /login-required | recordable | session_logout_sso_login_required (port never answers /login-required) |
| 4410 | plugins manipulate redirect URL | needs-harness | DiscourseEvent listener |
| 4425 | before_session_destroy event params | needs-harness | DiscourseEvent listener |
| 4445 | return_url absolute external rejected | recordable | session_logout_external_return_url |
| 4457 | return_url protocol-relative rejected | recordable | session_logout_protocol_relative_return_url |
| 4469 | return_url backslash variant rejected | covered | session_logout_backslash_return_url (new) |
| 4477 | return_url javascript: rejected | recordable | session_logout_javascript_return_url |
| 4489 | external return_url non-XHR redirects to root | needs-harness | Location header not recorded |
| 4497 | valid relative return_url | recordable | session_logout_relative_return_url |
| 4509 | `/` return_url beats logout_redirect | recordable | session_logout_root_over_logout_redirect |

## SessionController#forgot_password

`spec/requests/session_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 4598 | codes on > code to primary for username and secondary email | recordable | forgot_password_secondary_email (setup SQL secondary user_emails row for user1) |
| 4617 | codes on > expires reset and unscoped tokens | recordable | forgot_password_expires_tokens (setup SQL email_tokens rows for user1 and user2) |
| 4634 | codes on > logged-in user resets own password | recordable | forgot_password_own (user user1) |
| 4649 | codes on > staff reset another user > link | recordable | forgot_password_staff_for_user (user admin, login user1) |
| 4665 | codes on > same response for unknown when hidden | covered | forgot_password_unknown |
| 4680 | codes on > rate limits preserved | needs-harness | rate limits |
| 4693 | hide_email_address_taken > denies username | recordable | forgot_password_username_hidden (400) |
| 4700 | hide_email_address_taken > staff may use username | recordable | forgot_password_staff_for_user |
| 4710 | hide_email_address_taken > allows email (link flow) | recordable | forgot_password_link (enable_local_logins_via_code false) |
| 4719 | no login param > 400 | recordable | forgot_password_missing_login |
| 4724 | screens blocked IP | needs-harness | REMOTE_ADDR |
| 4746 | rate limiting | needs-harness | rate limits |
| 4785 | made up username > no token | covered | forgot_password_unknown |
| 4802 | local login disabled > 403 | covered | forgot_password_local_logins_disabled (new) |
| 4814 | SSO enabled > 403 (spec posts /session.json) | recordable | session_login_discourse_connect; forgot_password_discourse_connect for the real action |
| 4824 | local logins disabled > 403 (spec posts /session.json) | recordable | session_login_local_logins_disabled |
| 4830 | via email disabled > still makes a token | recordable | forgot_password_email_logins_off |
| 4837 | existing username > makes a token | recordable | forgot_password_link_by_username (hide false, codes off) |
| 4843 | existing username > enqueues an email | recordable | forgot_password_link_by_username |
| 4852 | system username > no token | recordable | forgot_password_system (hide false, login system) |
| 4858 | system username > no email | recordable | forgot_password_system |
| 4867 | staged > no token | recordable | forgot_password_staged (setup SQL users.staged for user2) |
| 4873 | staged > no email | recordable | forgot_password_staged |
| 4882 | staff writes only > staff allowed | needs-harness | readonly mode |
| 4889 | staff writes only > non-staff 503 | needs-harness | readonly mode |

## SessionController#redeem_password_reset_code

`spec/requests/session_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 4905 | exchanges code, changes password | covered | password_reset |
| 4922 | login and reset codes only for their own flow | needs-harness | SecureRandom stub; login-code endpoints |
| 4954 | existing second factor required | unported | accounts.rs:443 |
| 4977 | code from another browser rejected | needs-harness | second session (reset!) |
| 4994 | previous code invalidated by a new request | needs-harness | SecureRandom stub; only the last job's code is templatable |
| 5009 | latest code usable after rate-limited resend | needs-harness | rate limits |
| 5023 | code redeemed only once | covered | password_reset_code_reuse (new, matches) |
| 5034 | cleared by a later unknown account request | recordable | password_reset_code_after_unknown |
| 5043 | codes disabled > 404 | recordable | password_reset_code_disabled (enable_local_logins_via_code false) |
| 5050 | local logins disabled > 403 | recordable | password_reset_code_local_logins_disabled (literal code; port has no check, answers invalid code 200) |
| 5059 | staff writes only after requesting | needs-harness | readonly mode |
| 5068 | rate limits verification | needs-harness | rate limits |

## SessionController#current

`spec/requests/session_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 5080 | not logged in > 404 | covered | golden: GET /session/current.json |
| 5089 | logged in > user JSON | covered | golden: GET /session/current.json as=user1 |
| 5098 | featured topic the user cannot see omitted | recordable | session_current_hidden_featured_topic (setup SQL user_profiles.featured_topic_id = a staff-category topic for user0) |
| 5118 | anonymous shadow > master suspended stops auth | needs-harness | shadow user sign-in (second user) |
| 5138 | anonymous shadow > master deactivated stops auth | needs-harness | shadow user sign-in |

## SessionController#csrf, #get_honeypot_value

`spec/requests/session_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|

## StaticController#enter (POST /login)

`spec/requests/static_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 545 | no redirect path > root | recordable | login_enter_json (POST /login.json: Rails 302, the port only routes /login) |
| 552 | redirect path | needs-harness | Location header not recorded |
| 559 | full url on this host > path | needs-harness | Location (port also drops it to `/`, session.rs:447) |
| 566 | path with query kept | needs-harness | Location |
| 573 | period forcing a new host > root | needs-harness | Location |
| 580 | external full url > root | needs-harness | Location |
| 587 | javascript: > root | needs-harness | Location |
| 594 | array > root | needs-harness | Location |
| 601 | login page > root | needs-harness | Location |
| 608 | path containing /login kept | needs-harness | Location |
| 615 | invalid path > root | needs-harness | Location |
| 625 | subfolder > root | needs-harness | subfolder install |
| 632 | subfolder > login page > subfolder root | needs-harness | subfolder |
| 639 | subfolder > invalid > subfolder root | needs-harness | subfolder |
| 648 | sso_destination_url cookie > allowed | needs-harness | request cookie |
| 658 | sso_destination_url > wildcard domain | needs-harness | request cookie |
| 667 | sso_destination_url > not in secrets | needs-harness | request cookie |
| 677 | sso_destination_url > empty secrets | needs-harness | request cookie |
| 686 | sso_destination_url > malformed | needs-harness | request cookie |
| 695 | sso_destination_url > provider disabled | needs-harness | request cookie |
| 705 | sso_destination_url cookie deleted | needs-harness | request cookie / response cookies |

## ForumsController#status

`spec/requests/forums_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 5 | read only header > no header by default | covered | golden: GET /srv/status (status only, header not compared) |
| 11 | read only header > readonly header when postgres readonly | needs-harness | Discourse.received_postgres_readonly! (Redis state), header inspection |
| 18 | read only header > staff-writes-only mode header | needs-harness | Discourse.enable_readonly_mode (Redis key) |
| 27 | cluster > 500 when cluster not configured | covered | golden: GET /srv/status?cluster=parity |
| 33 | cluster > 500 when cluster does not match | needs-harness | global_setting(:cluster_name) (GlobalSetting/env) |
| 40 | cluster > 200 when cluster matches | needs-harness | global_setting(:cluster_name) |

## ListController#latest (and #index generics)

`spec/requests/list_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 147 | #index > Klipy key not in preloaded site settings | unported | HTML data-preloaded |
| 163 | invalid params > 400 page negative | covered | list_latest_invalid_params (new) |
| 168 | invalid params > 400 page above max int (x2) | recordable | same branch as page=-1; port 400 plain text |
| 176 | invalid params > 400 before[1]=haxx | recordable | port ignores `before`, answers 200 |
| 181 | invalid params > 400 bumped_before[1]=haxx | recordable | port ignores the param, 200 |
| 186 | invalid params > 400 topic_ids[1]=haxx | covered | list_latest_invalid_params (new) |
| 191-286 | invalid params > 400 for category/order/ascending/min_posts/max_posts/status/filter/state/search/q/f/subset/group_name/tags/user/match_all_tags/no_subcategories/no_tags/exclude_tag non-scalar values (x20) | recordable | only page/per_page/ascending are validated by the port; the rest are dropped, so 200 vs 400 |
| 292 | legit requests return 200 (no_definitions, max_posts, min_posts, page=0/1/1999, search=, topic_ids[], tags[]) | covered | list_latest_invalid_params (new) |
| 327 | anonymous filters (latest, top, hot...) return 200 (xN) | covered | golden: GET /latest.json, /top.json, /hot.json?per_page=3 |
| 335 | filter on a set of topic ids | covered | lists/topic_ids |
| 344 | homepage title with short_site_description | unported | HTML title |
| 355 | structured data in HTML | unported | HTML |
| 365 | no N+1 with tags | needs-harness | SQL query counting |
| 415 | no N+1 with primary groups | needs-harness | SQL query counting |
| 474 | topics with tags > hidden tags not shown | recordable | setup tag_group with staff permission on tag 2, GET /latest.json anon; tag must drop from topic 35/41 |
| 489 | lazy load categories > categories + parents returned | recordable | settings lazy_load_categories_groups=4 (anonymous_users), GET /latest.json |
| 503 | lazy load categories > no categories key when off | covered | golden: GET /latest.json (default "") |
| 521 | login required > homepage shows no topics | covered | login_required_lists_anonymous (new, matches) |
| 546 | categories and X > category latest with no_subcategories=false includes subcategory topics | covered | golden: GET /c/general/4.json (subcategory topic 38 included by default; param is a no-op) |
| 554 | crawler titles > no title for default URL | unported | HTML crawler |
| 564 | crawler titles > title for non-default URLs | unported | HTML crawler |
| 574 | rss feed discovery > advertises feed | unported | HTML + RSS |
| 582 | rss feed discovery > omits feed without route | unported | HTML + RSS |
| 605 | crawler homepage renders for each filter | unported | HTML crawler |
| 615 | crawler homepage falls back when homepage is user-scoped | unported | HTML crawler |
| 929 | RSS > hidden first post of relisted topic not exposed | unported | .rss not routed |
| 956 | RSS > latest RSS | unported | .rss not routed |
| 963 | RSS > localized from Accept-Language for crawler-classified clients | unported | .rss not routed |
| 982 | RSS > topic author cannot forge RSS items | unported | .rss not routed (setup half is a posts.json create) |
| 1008 | RSS > latest RSS with query params | unported | .rss not routed |
| 1015 | RSS > sanitized feed URLs | unported | .rss not routed |
| 1082 | RSS > subfolder links | unported | .rss not routed |
| 1150 | RSS > exclude_tag in latest RSS | unported | .rss not routed |
| 2702 | content localization > crawler title localized | unported | HTML crawler + content localization |
| 2715 | content localization > title as-is without localization | unported | HTML crawler |
| 2742 | content localization > tl=ja crawler title | unported | HTML crawler |
| 2750 | content localization > no N+1 loading localizations | needs-harness | SQL query counting |

## ListController#top (+ /top/:period redirect)

`spec/requests/list_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 1091 | RSS > top RSS | unported | .rss not routed |
| 1097 | RSS > invalid period on top RSS | unported | .rss not routed |
| 1104 | RSS > #{period} top RSS (xN) | unported | .rss not routed |
| 1125 | RSS ignores current user > muted topics in top RSS | unported | .rss not routed |
| 1157 | RSS > exclude_tag in top RSS | unported | .rss not routed |
| 1191 | renders top | covered | golden: GET /top.json |
| 1196 | renders top with a period | covered | golden: GET /top.json?period=weekly |
| 1201 | 400 for invalid period | covered | list_latest_invalid_params (new) |
| 1207 | per page > per_page param used | recordable | `/top.json?per_page=5` |
| 1213 | per page > topics_per_period_in_top_page setting | recordable | settings topics_per_period_in_top_page=32, GET /top.json |
| 1818 | best_periods_for > applicable periods | needs-harness | calls ListController.best_periods_for directly with relative dates (time travel) |
| 1832 | best_periods_for > default period | needs-harness | class-method unit test, no request |

## ListController#hot

`spec/requests/list_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 1132 | RSS ignores current user > muted topics in hot RSS | unported | .rss not routed |
| 1167 | RSS > exclude_tag in hot RSS | unported | .rss not routed |

## ListController#category_default / #category_latest / #category_none_* (incl. /l/<filter>, set_category)

`spec/requests/list_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 1281 | category > without access > 404 | covered | golden: GET /c/staff/3.json (anon) and as=user1 |
| 1288 | category > with access > 200 | covered | golden: GET /c/general/4/l/latest.json?per_page=2 |
| 1299 | category > encoded slug > 200 | recordable | setup category with encoded slug + settings slug_generation_method=encoded |
| 1309 | category > parent/child/id > child list 200 | covered | golden: GET /c/general/sub-general/34.json |
| 1316 | category > invalid parent/child slug > redirects | recordable | `/c/random/another/34/l/latest.json` 301 to canonical |
| 1334 | category > other category named with an id prefix | recordable | setup UPDATE categories SET slug='4-name' on a category |
| 1346 | child category > parent and child requested > 200 | covered | golden: GET /c/general/sub-general/34.json |
| 1353 | child category > wrong parent (no id) > 404 | recordable | `/c/not-the-right-slug/sub-general/l/latest.json` |
| 1361 | feed > RSS | unported | .rss not routed |
| 1367 | feed > RSS in subfolder | unported | .rss not routed |
| 1375 | feed > exclude_tag | unported | .rss not routed |
| 1388 | feed > no route-derived category param in self URL | unported | .rss not routed |
| 1413 | default views > top default view, for_period=default_top_period | covered | list_category_default_view_top (new) |
| 1421 | default views > unsupported default_top_period falls back to site default | recordable | setup UPDATE categories SET default_view='top', default_top_period='bogus' + settings top_page_default_timeframe=monthly |
| 1433 | default views > nil default view | covered | golden: GET /c/general/4.json (seed default_view NULL) |
| 1441 | default views > '' default view | recordable | setup UPDATE categories SET default_view='' |
| 1449 | default views > latest default view | recordable | setup UPDATE categories SET default_view='latest' |
| 1464 | default views > unreachable filter falls back to latest (destroy, categories, unread for anon) | recordable | setup default_view='unread', anon GET /c/general/4.json |
| 1470 | default views > logged-in-only view for logged-in users | recordable | same setup as user0 |
| 1476 | default views > honoured with /none | covered | list_category_default_view_top (new) |
| 1482 | canonical tag > category default view | unported | HTML |
| 1488 | canonical tag > category latest view | unported | HTML |
| 1494 | canonical tag > keeps page number on later pages of a category (page=2, l/latest?page=2, page=0) | recordable | category_canonical_page (HTML GET /c/<slug>/<id>?page=2&order=views, extract link rel=canonical); port builds canonical from category.url only (src/routes/list.rs:1573), drops ?page=, would diverge |
| 1511 | category default view > title | unported | HTML |
| 1517 | category default view > og/twitter description stripped | unported | HTML |
| 1539 | category default view > description escaped once | unported | HTML |
| 1564 | category latest view > title | unported | HTML |
| 1858 | set_category > redirects to updated slug (3 levels) | recordable | settings max_category_nesting=3 needs a 3rd-level category via setup; 2-level form `/c/hello/world/34.json` is simple |
| 1869 | set_category > id-only path redirects, query kept | recordable | `/c/4.json?page=4` 301 |
| 1876 | set_category > correct-case slug redirect | recordable | `/c/General/4.json` 301 |
| 1886 | set_category > no restricted topic titles through permalink fallback | recordable | setup permalink row to topic 4 (staff cat), anon GET /c/old-category/999.json 404 |
| 1900 | set_category > encoded slugs no redirect loop | recordable | setup UPDATE categories SET slug='syst%C3%A8mes' |
| 1910 | set_category > lowercase encoded slugs no loop | recordable | as above, lowercased |
| 1928, 1961-1969 | set_category > encoded non-ASCII slugs > outdated slug redirects to current encoded slug, then 200 (x3) | recordable | set_category_encoded_slug_redirect (settings slug_generation_method=encoded, setup UPDATE categories SET slug to an encoded CJK slug, GET old slug 301 then new path 200); Location not compared |
| 1938, 1961-1969 | set_category > encoded non-ASCII slugs > slug without id redirects to URL with id (x3) | recordable | set_category_encoded_slug_no_id (same setup, GET /c/<encoded slug>.json 301); Location not compared |
| 1945, 1961-1969 | set_category > encoded non-ASCII slugs > keeps filter and query string (x3) | needs-harness | assertion is the Location (/l/latest?page=2 kept); status alone same as 1928 |
| 1952, 1961-1969 | set_category > encoded non-ASCII slugs > keeps the format (x3) | needs-harness | assertion is the Location (.json kept) |
| 1972 | set_category > encoded non-ASCII slugs > outdated parent slug for a subcategory redirects | recordable | set_category_encoded_parent_slug_redirect (settings slug_generation_method=encoded, setup UPDATE parent slug, GET /c/<old parent>/<sub>/<id>/none 301); Location not compared |
| 1986 | set_category > encoded non-ASCII slugs > keeps the subfolder | needs-harness | set_subfolder (global setting), Location |
| 2000 | set_category > subfolder redirect | needs-harness | set_subfolder (global setting) |
| 2008 | set_category > subfolder sub-sub redirect | needs-harness | set_subfolder |
| 2032 | set_category > unsafe redirect error renders 404 | needs-harness | stubs redirect_to |
| 2037 | set_category > unsafe redirect does not log | needs-harness | stubs + logger inspection |
| 2044 | set_category > gibberish slug redirects | recordable | `/c/summit'%22()&%25.../4.json` 301 |
| 2136 | shared drafts > not displayed when disabled | recordable | setup shared_drafts row (topic 35 -> cat 2), admin GET /c/general/4.json |
| 2148 | shared drafts > displayed in both categories | unported | Unsupported("shared_drafts_category in topic lists") src/topic_query.rs:346 |
| 2170 | body class for categories | unported | HTML |
| 2637 | #new unified > category new list | recordable | /c/general/4/l/new.json as user0, settings enable_unified_new (seed topic_users) |
| 2644 | #new unified > category new respects subset | recordable | port ignores `subset` |

## ListController user lists (#unread #new #unseen #read #posted #bookmarks)

`spec/requests/list_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 1177 | RSS > exclude_tag in user topics RSS | unported | .rss not routed (topics_by feed) |
| 1803 | read > not logged in raises (404 HTML) | covered | golden: GET /read (anon) |
| 1809 | read > logged in returns read list | covered | golden: GET /read.json as=user1 |
| 2596 | #new unified > new topics and new replies | recordable | settings enable_unified_new=true, GET /new.json as user0 |
| 2611 | #new unified > subset=topics | recordable | port drops `subset` (not in ListParams) |
| 2624 | #new unified > subset=replies | recordable | as above |
| 2658 | #new unified > tag new list | recordable | no case yet: ported in 5531dad0 (was: Unsupported("tag list filters other than latest, top and hot") src/routes/tags.rs:235) |
| 2665 | #new unified > tag new list subset | unported | same marker |

## CategoriesController#index

`spec/requests/categories_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 10 | crawler view subfolder urls | unported | HTML crawler + subfolder |
| 18 | preloads topic list | unported | HTML data-preloaded |
| 29 | no topic list for crawlers | needs-harness | `.expects(:fetch_topic_list).never` |
| 41 | homepage title | unported | HTML |
| 53 | /category paths redirect to /c | unported | /category/* not routed |
| 59 | /category paths redirect encodes URL-invalid characters | unported | /category/* not routed (only /category/{id}/notifications and /slug in src/routes/mod.rs) |
| 65 | /category permalink before redirect | unported | /category/* not routed |
| 73 | legacy category permalink no title leak | unported | /category/* not routed |
| 88 | normal user response | covered | golden: GET /categories.json as=user1 |
| 103 | omits invisible topics with stale featured rows | recordable | setup UPDATE topics SET visible=false on a featured topic, GET /categories.json?include_topics=true |
| 118 | no subcategories without permission | recordable | include_subcategories ignored by port |
| 135 | private subcategory counts excluded for anon | recordable | setup restricted subcategory rows; anon GET /categories.json |
| 168 | subcategory response with permission | covered | categories_subcategory_params (new) |
| 185 | no subcategories without param | covered | golden: GET /categories.json as=user1 |
| 200 | topics for categories, subs and subsubs | recordable | include_subcategories + include_topics, needs 3rd-level category setup |
| 239 | tag filter > categories | recordable | `?tag=howto&include_topics=true` (port drops `tag`) |
| 257 | tag filter > subcategories | recordable | as above with include_subcategories |
| 278 | tag filter > subsubcategories | recordable | needs 3rd-level category setup |
| 322 | categories_and_latest > default bump order | unported | /categories_and_latest not routed (not this action) |
| 330 | categories_and_latest > no sort in more_topics_url | unported | /categories_and_latest not routed |
| 346 | categories_and_latest > created order | unported | /categories_and_latest not routed |
| 354 | categories_and_latest > sort=created in more_topics_url | unported | /categories_and_latest not routed |
| 366 | subcategories_with_featured_topics style | recordable | no case yet: ported in 8b4db203 (was: Unsupported("subcategory_list (subcategories_with_featured_topics styles)") src/category_list.rs:104) |
| 385 | no extra queries with more categories | needs-harness | SQL query counting |
| 410 | no N+1 with multiple topics | needs-harness | SQL query counting |
| 452 | uncategorized hidden unless allow_uncategorized_topics | recordable | settings desktop_category_page_style=categories_boxes_with_topics, allow_uncategorized_topics=false |
| 467 | parent_category_id lists subcategories | covered | categories_subcategory_params (new) |
| 485 | page > lazy_load_categories pagination | unported | Unsupported("paginated category lists") src/category_list.rs:122 (also stub_const) |
| 497 | page > many categories pagination | needs-harness | stub_const MAX_UNOPTIMIZED_CATEGORIES |
| 509 | page > no pagination by default | needs-harness | stub_const CATEGORIES_PER_PAGE (page=2 -> empty list is recordable without the stub) |
| 519 | page > nested page param | recordable | `/categories.json?page[foo]=2` 200 |

## SearchController#query

`spec/requests/search_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 51 | overloaded > 409 | needs-harness | global_setting + freeze_time + X-Request-Start |
| 81 | null bytes > 400 | recordable | `/search/query.json?term=hello%00hello` |
| 89 | oversized search text (51 words) > 400 on /search.json and /search/query.json | recordable | search_query_oversized_term (Search::MAX_SEARCH_TERM_WORDS 50 is new; port has no term length or word cap in routes/search.rs or src/search, so it searches and answers 200) |
| 99 | can search correctly (pg headlines) | unported | Unsupported("use_pg_headlines_for_excerpt") src/search/mod.rs:537 |
| 132 | advanced filter order:views | recordable | `/search/query.json?term=order:views%20fixture` |
| 148 | type filter topic / user | covered | golden: /search/query.json?term=user&type_filter=user, ?term=fixture&type_filter=topic |
| 193 | category-restricted tags > anon sees none | needs-harness | tag search reads tag_search_data (SearchIndexer rows) plus tag groups |
| 200 | category-restricted tags > unauthorized user sees none | needs-harness | as above |
| 209 | category-restricted tags > authorized group member sees both | needs-harness | as above + custom group membership |
| 223 | category-restricted tags > admin sees both | needs-harness | as above |
| 239 | search by topic id ignores min length | recordable | no case yet: ported in 2fd80508 (was: Unsupported("search_for_id (topic id and URL lookup)") src/routes/search.rs:311) |
| 255 | search by topic id returns topic | unported | same marker |
| 275 | anonymous search disabled > anon 403 not_logged_in | covered | search_anonymous_disabled (new) |
| 282 | anonymous search disabled > logged-in allowed | recordable | settings allow_anonymous_search=false as user0 |
| 291 | logs the search term (search_log_id) | covered | search_query_logs_term (new) |
| 307 | logs pageview session id | recordable | header Discourse-Pageview-Session-Id |
| 323 | no log when disabled | recordable | settings log_search_queries=false |
| 330 | no log for exclude_topics | recordable | `type_filter=exclude_topics`, compare search_logs rows |
| 337 | empty term with search_for_id no 500 | recordable | no case yet: ported in 2fd80508 (was: Unsupported("search_for_id ...") src/routes/search.rs:311) |
| 365 | rate limit anon per user | needs-harness | rate limits + freeze_time |
| 385 | rate limit anon globally | needs-harness | rate limits + freeze_time |
| 405 | rate limit logged in | needs-harness | rate limits |
| 425 | crawler noindex page | unported | HTML crawler |
| 814, 820 | string search_context > 400 on /search.json and /search/query.json (x2) | unported | Unsupported("search_context given as a string") src/routes/search.rs:701; Rails now raises InvalidParameters (400), so the marker can become invalid_parameters(search_context) |
| 826 | search context > invalid context type 400 | recordable | nested search_context[type]=... is silently ignored by the port (only flat keys hit the Unsupported marker) |
| 838 | search context > missing id 400 | recordable | as above |
| 844 | search context > user context not visible 403 | recordable | as above |
| 856 | search context > user context query | recordable | as above |
| 871 | search context > tag does not exist | recordable | as above |
| 884 | search context > tag context query | recordable | as above |

## SearchController#show

`spec/requests/search_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 63 | overloaded > no results and error | needs-harness | global_setting + freeze_time |
| 438 | anon cannot search > HTML redirect to login with destination | needs-harness | HTML request; harness sends xhr JSON |
| 447 | anon cannot search > JSON rejected 403 | covered | search_anonymous_disabled (new) |
| 455 | no term > 200 | covered | golden: GET /search.json |
| 460 | term shorter than min > 400 | covered | golden: GET /search.json?q=ba |
| 465 | term is a hash > 400 | recordable | `/search.json?q[foo]` (port drops it, 200) |
| 470 | null bytes > 400 | recordable | `/search.json?q=hello%00hello` |
| 477 | page string number ok | covered | golden: GET /search.json?q=Parity&page=1 |
| 482 | page integer ok | covered | golden: GET /search.json?q=Parity&page=1 |
| 487 | invalid page > 400 | covered | golden: GET /search.json?q=fixture&page=%203 (same parse-failure branch) |
| 492 | page padded with spaces > 400 | covered | golden: GET /search.json?q=fixture&page=%203 |
| 497 | page above limit > 400 | covered | golden: GET /search.json?q=fixture&page=11 |
| 502 | logs the search term | recordable | `/search.json?q=fixture`, search_logs row |
| 509 | no log when disabled | recordable | settings log_search_queries=false |
| 516 | tag context | recordable | no case yet: ported in 2fd80508 (was: Unsupported("search contexts (user, topic, category, tag)") src/routes/search.rs:213) |
| 545 | restricted tag context > permitted user | unported | same marker |
| 558 | restricted tag context > 403 without permission | unported | same marker |
| 589 | rate limit anon per user | needs-harness | rate limits + freeze_time |
| 607 | rate limit anon globally | needs-harness | rate limits + freeze_time |
| 627 | rate limit logged in | needs-harness | rate limits |
| 667 | lazy loaded categories > extra categories | recordable | settings lazy_load_categories_groups=4, anon `/search.json?q=fixture` |
| 692 | content localization > translated blurb | unported | Unsupported("content_localization_enabled") src/search/mod.rs:556 |
| 708 | content localization > original blurb via cookie | unported | same marker |
| 779 | search priority > empty term status:open | recordable | setup UPDATE categories SET search_priority, `/search.json?q=status:open` |
| 789 | search priority > no order query | recordable | settings category_search_priority_*_weight + setup |
| 802 | search priority > order query ignores priority | recordable | `q=status:open order:latest fixture` |

## TagsController#index

`spec/requests/tags_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 34 | retrieves all tags but synonyms as staff (x2 via shared examples) | covered | parity/writes/tags |
| 68 | no pm_count when user cannot tag PMs (x2) | covered | parity/writes/tags |
| 86 | pm_count when allowed (x2) | covered | parity/writes/tags |
| 114 | tags restricted to unseen category not listed (x2) | covered | parity/writes/tags |
| 128 | non-staff only sees tags used in public topics (x2) | covered | golden: GET /tags.json as=user1 |
| 164 | pm tags enabled > topic and pm counts | covered | parity/writes/tags |
| 177 | pm tags enabled > other users' PM tag list 404 | unported | /u/:username/messages/tags not routed (other action) |
| 191 | pm tags disabled > pm-only tags shown to admins | covered | parity/writes/tags |
| 205 | pm tags disabled > hidden from regular users | covered | parity/writes/tags |
| 222 | listed_by_group > restricted category tags hidden | unported | Unsupported("tags_listed_by_group") src/routes/tags.rs:528 |
| 234 | listed_by_group > pm-only hidden from groups | unported | same marker |
| 245 | listed_by_group > pm-only shown to admins | unported | same marker |
| 259 | listed_by_group > works for tags in groups | unported | same marker |
| 277 | listed_by_group > no N+1 | unported | same marker |
| 324 | not grouped > pm-only hidden from category tag lists | covered | parity/writes/tags |
| 335 | not grouped > pm-only shown in category lists to admins | covered | parity/writes/tags |
| 349 | not grouped > tags and category tags for admin | covered | parity/writes/tags |
| 386 | not grouped > no N+1 with category tags | needs-harness | SQL query counting |
| 453 | hidden tags > returned to admins | covered | tags_show_hidden_as_admin (new) |
| 460 | hidden tags > not returned to anon | covered | parity/writes/tags |
| 466 | hidden tags > not returned to regular user | covered | parity/writes/tags |
| 476 | hidden + restricted to category > admins | covered | parity/writes/tags |
| 485 | hidden + restricted to category > anon | covered | parity/writes/tags |
| 491 | hidden + restricted to category > regular user | covered | parity/writes/tags |
| 502 | hidden + listed by group > admins | unported | Unsupported("tags_listed_by_group") |
| 511 | hidden + listed by group > anon | unported | same marker |
| 517 | hidden + listed by group > regular user | unported | same marker |

## TagsController#show (+ show in category)

`spec/requests/tags_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 532 | returns requested tag in topic list | covered | golden: GET /tag/howto.json |
| 543 | period tag names by id URL and encoded legacy URL | covered | parity/writes/tags |
| 560 | intersections with encoded period names | unported | /tags/intersection not routed |
| 574 | tag info for encoded period name | unported | /tag/:name/info is #info, not routed |
| 583 | invalid tag `/tag/%2ftest%2f` 404 | covered | parity/writes/tags |
| 588 | synonyms redirect (l/top.json?period=daily) | covered | parity/writes/tags |
| 595 | raw query strings preserved in redirects | covered | parity/writes/tags |
| 608 | tag synonym of itself no loop | covered | parity/writes/tags |
| 615 | staff-only tags 404 for anon, 200 for admin | covered | tags_show_hidden_as_admin (new) |
| 630 | additional tags in query params | covered | parity/writes/tags |
| 641 | duplicate tags in query params | covered | parity/writes/tags |
| 652 | tag description in meta description | unported | HTML |
| 663 | default meta description | unported | HTML |
| 673 | keeps page number in canonical URL on later pages of a tag (page=2, page=0) | recordable | tags_show_canonical_page (HTML GET /tag/<slug>/<id>?page=2&order=views, extract link rel=canonical); port canonical is the bare list path (src/routes/tags.rs:842), would diverge |
| 689 | special tag none | covered | parity/writes/tags |
| 698 | numeric tag names via /tag/:tag_name | unported | Unsupported("/tag/:tag_id without .json") src/routes/tags.rs:70 (HTML route) |
| 707 | numeric tag names with filters | covered | parity/writes/tags |
| 716 | missing numeric /tag/:tag_id 404 | covered | golden: GET /tag/99.json |
| 721 | numeric-named slug route redirect | covered | parity/writes/tags |
| 734 | edit path redirect | unported | /tag/:slug/:id/edit not served (unknown /tag/ route marker src/routes/tags.rs:98) |
| 741 | edit tab redirect | unported | same marker |
| 748 | edit page no redirect | unported | same marker |
| 755 | id-only edit redirect | unported | same marker |
| 769 | in category > no restricted name leak via permalink | covered | parity/writes/tags |
| 780 | in category > topic inside category | covered | golden: GET /tags/c/general/4/howto.json |
| 789 | in category > next topic URL | covered | golden: GET /tags/c/general/4/none/howto/1.json?per_page=1 |
| 797 | in category > invalid category path 404 | covered | parity/writes/tags |
| 803 | in category > restricted category 404 | covered | tags_in_staff_category_as_admin (new) |
| 818 | in subcategory > topic inside subcategory | covered | parity/writes/tags |
| 828 | invalid tag parameter ignored | covered | parity/writes/tags |

## SiteController#site

`spec/requests/site_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 66 | anonymous_default_navigation_menu_tags only visible tags | recordable | no case yet: ported in f7cb02f1 (was: Unsupported("anonymous_default_navigation_menu_tags (SidebarTagSerializer)") src/site.rs:1015) |

## SiteController#basic_info

`spec/requests/site_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 100 | visible with login_required, title/description/logos/discover | covered | login_required_lists_anonymous (new, matches) |
| 129 | false values for discover and login_required | covered | golden: GET /site/basic-info.json |

## RobotsTxtController#builder

`spec/requests/robots_txt_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 5 | returns json for building robots.txt | covered | golden: GET /robots-builder.json |
| 13 | includes overridden content | recordable | settings overridden_robots_txt, GET /robots-builder.json |

## RobotsTxtController#index

`spec/requests/robots_txt_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 27 | overridden > not prepended without overrides (admin) | covered | golden: GET /robots.txt (same body for admin with no override) |
| 33 | overridden > header prepended for admin | covered | robots_overridden_as_admin (new) |
| 40 | overridden > not prepended for non-admin | recordable | settings overridden_robots_txt, anon |
| 48 | subfolder prefixes rules | needs-harness | set_subfolder |
| 84 | indexing allowed | covered | golden: GET /robots.txt |
| 95 | allowlist user agents | recordable | settings allowed_crawler_user_agents="Googlebot,Twitterbot" (pipe-separated) |
| 110 | blocklist user agents | recordable | settings blocked_crawler_user_agents |
| 125 | blocklist ignored with allowlist | recordable | both settings |
| 135 | noindex when indexing disallowed | recordable | settings allow_index_in_robots_txt=false |
| 143 | overridden robots.txt returned | recordable | settings overridden_robots_txt, anon |
| 155 | sitemap line when enabled | covered | golden: GET /robots.txt |
| 164 | no sitemap line when disabled | recordable | settings enable_sitemap=false |
| 173 | no sitemap line with login_required | recordable | settings login_required=true (robots.txt is exempt) |
| 192 | plugins add to robots.txt | needs-harness | DiscourseEvent registration |

## SitemapController

`spec/requests/sitemap_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 5 | 404 when sitemap disabled | recordable | settings enable_sitemap=false, GET /sitemap.xml and /news.xml |
| 14 | 404 without format (/news) | recordable | anon GET /news (port routes only /news.xml) |
| 22 | index lists none if not generated | covered | sitemap_index_not_generated (new) |
| 30 | index lists generated sitemaps | covered | golden: GET /sitemap.xml |
| 40 | index skips disabled sitemaps | recordable | setup UPDATE sitemaps SET enabled=false WHERE name='recent' (port's regenerate re-enables it) |
| 51 | page 404 if sitemap missing | covered | golden: GET /sitemap_9.xml |
| 57 | page includes topics | covered | golden: GET /sitemap_1.xml |
| 71 | page refreshes cached page once its newest topic changes | needs-harness | Sitemap.regenerate_sitemaps between two GETs (cache invalidation across requests), update_columns timestamps |
| 88 | page omits topics a sitemap_topics_query modifier drops | needs-harness | plugin modifier registration |
| 110 | recent: topics bumped in last 3 days | covered | golden: GET /sitemap_recent.xml |
| 127 | recent: page numbers from posts_count | recordable | setup UPDATE topics SET bumped_at=now(), posts_count=21 WHERE id=35 |
| 155 | news: topics bumped in last 72h | covered | golden: GET /news.xml |

## UserAvatarsController#show_proxy_letter

`spec/requests/user_avatars_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 40 | 404 when external avatar is elsewhere | recordable | settings external_system_avatars_url=https://somewhere.else.com/avatar.png, GET /letter_avatar_proxy/v4/letter/a/aaaaaa/20.png (port answers an empty 404; Rails xhr gives the JSON not_found body) |
| 46 | returns avatar when proxy allowed | needs-harness | stubs disable_proxy? + stub_request to the avatar CDN |

## StylesheetsController (color definitions)

`spec/requests/stylesheets_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 7 | source map has no username header | unported | /stylesheets/*.map not routed |
| 21 | survives cache miss | unported | /stylesheets/:name not routed; StylesheetCache compile |
| 46 | theme specific css lookup | unported | /stylesheets/:name not routed |
| 105 | plugin css lookup | needs-harness | plugin registration (also route not served) |
| 159 | plugin link tags for staff | needs-harness | plugin registration |
| 169 | no admin link tag for non-staff | needs-harness | plugin registration |
| 179 | no link tags when plugin disabled | needs-harness | plugin registration |
| 191 | ignores Accept header, no Vary | unported | /stylesheets/:name not routed |
| 216 | color_scheme > non-selectable theme for anon | unported | /color-scheme-stylesheet not routed |
| 227 | color_scheme > non-selectable theme for user | unported | /color-scheme-stylesheet not routed |
| 240 | color_scheme > staff uses non-selectable theme | unported | /color-scheme-stylesheet not routed |
| 253 | color_scheme > works | unported | /color-scheme-stylesheet not routed |
| 262 | color_scheme > with theme param | unported | /color-scheme-stylesheet not routed |
| 272 | color_scheme > no duplicate cache entries | unported | route not served; Stylesheet::Manager.cache inspection |


## HashtagsController#search

`spec/requests/hashtags_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 359 | #search > returns the found category and then tag (tag ref gets ::tag) | covered | search_tag_conflict (new, matches) |
| 396 | #search > no hidden and restricted categories/tags for a user | covered | search_staff_hidden, search_tag_hidden (new, match) |
| 409 | #search > admin sees hidden and restricted categories/tags | covered | search_staff_admin, search_tag_hidden_admin (new, match) |
| 455 | #search > not logged in is invalid access | covered | search_anonymous (new, matches) |

## UsersController#search_users

`spec/requests/users_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 6951 | searches when provided the term only | covered | term, default_limit, name (new, match) |
| 6959 | searching usernames > can_review doesn't expose hidden category moderation group members | recordable | user_search_can_review_hidden_group (anon) + user_search_can_review_hidden_group_owner (as the owner); setup SQL owners-visibility group, group_users, category_moderation_groups, enable_category_group_moderation; spec checks two viewers, so two cases |
| 6992 | searching usernames > a list of usernames | covered | usernames (new, matches) |
| 7001 | searching usernames > excludes users hidden by the regular scope | covered | usernames_suspended (new, matches) |
| 7029 | searching usernames > searches groups with include_groups | covered | usernames_groups (new, matches) |
| 7048 | searches when provided the topic only | covered | topic (new, matches) |
| 7055 | searches when provided the term and topic | covered | topic_term (new, matches) |
| 7064 | in a topic > brings first the replied to user | covered | topic_prioritized (new, matches) |
| 7078 | in a topic > respects the term when replying | covered | topic_term (new, matches) |
| 7094 | only users with access to a private topic | covered | topic_allowed (new, matches) |
| 7123 | interprets a blank category id | covered | blank_category (new, matches) |
| 7130 | invalid limit params | covered | bad_limit, big_limit, negative_limit (new, match) |
| 7139 | enable_names true returns names | covered | term (new, matches) |
| 7149 | enable_names false omits names | covered | names_disabled (new, matches) |
| 7200 | groups > sorts on prefix | covered | groups, underscore (new, match) |
| 7209 | groups > none without a term | covered | empty_term (new, matches) |
| 7218 | groups > only visible groups | covered | groups_admin, exact (new, match) |
| 7228 | groups > plugin groups filter | needs-harness | plugin registration |
| 7252 | groups > plugin modifier on the groups filter | needs-harness | modifier registration |
| 7272 | groups > modifier introducing a join | needs-harness | modifier registration |
| 7297 | groups > doesn't search groups without a groups param | covered | default_limit (new, matches) |
| 7309 | groups > messageable groups | covered | messageable (new, matches) |
| 7325 | groups > mentionable groups | covered | mentionable, mentionable_admin (new, match) |
| 7345 | groups > anonymous gets no mentionable or messageable groups | covered | anonymous_mentionable (new, matches) |
| 7381 | by group name > results for a group member | covered | group_members (new, matches) |
| 7389 | by group name > no results for a non-member | covered | group_param (new, matches) |
| 7395 | by group name > member of one of the groups | covered | group_members (new, matches) |
| 7407 | by group name > not a member of the groups | covered | group_param (new, matches) |
| 7417 | by group name > anonymous can't enumerate a hidden group | covered | group_anonymous, group_hidden (new, match) |
| 7431 | by group name > signed-in user filters by a group they can see | covered | group_param (new, matches) |
| 7448 | by group name > missing group gives no results | covered | group_missing (new, matches) |
| 7464 | include_staged_users > not for public callers | covered | staged_user (new, matches) |
| 7473 | include_staged_users > included for staff | covered | staged (new, matches) |
| 7484 | include_staged_users > no last-seen suggestions with staged | covered | staged_last_seen (new, matches) |
| 7493 | include_staged_users > not without the param | covered | inactive, term (new, match) |
| 7499 | include_staged_users > not when false | covered | staged_false (new, matches) |
| 7507 | last_seen_users > results when true | covered | last_seen (new, matches) |
| 7514 | last_seen_users > respects limit | covered | last_seen (new, matches) |
| 7524 | returns avatar_template | covered | term (new, matches) |
| 7535 | status > returned when enabled | covered | status (new, matches) |
| 7550 | status > not returned when disabled | covered | status_disabled (new, matches) |

## PresenceController#update

`spec/requests/presence_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 50 | readonly mode produces 503 | unported | readonly mode isn't ported |
| 64 | rejects 51 leave_channels with 400 invalid_parameters before leaving any | recordable | too_many_leave (like too_many_present, 51 /chat-reply/N leave_channels). routes/presence.rs only caps present_channels, expect a diff |
| 81 | updates presence for the requested channels | covered | online, leave, two_tabs (new, match) |
| 118 | true/false by channel existence and security | covered | unknown, reply_restricted, reply_restricted_admin, not_allowed_to_chat (new, match) |
| 157 | doesn't overwrite the session | needs-harness | session cookie inspection |

## PresenceController#get

`spec/requests/presence_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 176 | returns presence for the requested channels | covered | online, get_duplicates (new, match) |
| 200 | respects existence and security of the channel | covered | unknown, get_anonymous, reply_restricted (new, match) |
| 227 | count_only channels | needs-harness | registers a test channel prefix; no ported prefix is count only |

## Quotes (QuotedPost, PostAlerter, CookedPostProcessor)

No request spec quotes a post; the behaviour is specced on the models
and services a reply with a quote goes through, inventoried here.

| Spec | Line | Scenario | Class | Case / note |
|---|---|---|---|---|
| models/quoted_post_spec.rb | 4 | correctly extracts quotes | covered | posts/quote_reply_same_topic, posts/quote_other_topic |
| models/quoted_post_spec.rb | 46 | doesn't count quotes from the same post | recordable | quote_self (a reply quoting its own post number) |
| models/quoted_post_spec.rb | 68 | correctly handles deltas | covered | posts/edit_add_quote, posts/edit_remove_quote |
| services/post_alerter_spec.rb | 319 | notifies a group member if someone quotes their post | covered | posts/pm_group_alert_quote |
| services/post_alerter_spec.rb | 338 | no notification for a quote inside a whisper | recordable | no case yet: ported in 4a27c9b4 (was: whisper in UNPORTED_CREATE_PARAMS) |
| services/post_alerter_spec.rb | 549 | does not notify muted users | recordable | quote_muted_user (setup: muted_users row) |
| services/post_alerter_spec.rb | 556 | does not notify ignored users | recordable | quote_ignored_user (setup: ignored_users row) |
| services/post_alerter_spec.rb | 563 | not again for a user with a reply notification | covered | posts/quote_reply_same_topic |
| services/post_alerter_spec.rb | 580 | does not collapse quote notifications | recordable | quote_twice (two replies quoting user2) |
| services/post_alerter_spec.rb | 588 | does not notify again on revision | recordable | quote_edit_again (a quote, its post_alert, an edit and post_alert again) |
| services/post_alerter_spec.rb | 595 | doesn't notify the poster | recordable | quote_own_post (user2 quoting post 3) |
| services/post_alerter_spec.rb | 607 | :before_create_notifications_for_users | needs-harness | DiscourseEvent subscription |
| services/post_alerter_spec.rb | 622-659 | display names leading: username: param, nested and several quotes (x4) | recordable | quote_display_names (settings display_name_on_posts, prioritize_username_in_ux false) |
| services/post_alerter_spec.rb | 545 | the quoted user's notification | covered | posts/quote_post_alert, posts/quote_other_topic, posts/quote_unlisted_topic; posts/quote_invisible_post and quote_deleted_post (no notification) |
| lib/cooked_post_processor_spec.rb | 2133-2169 | quote unmodified / modified / missing (x3) | covered | the pretty_text corpus' cooked column (tests/pretty_text.rs) |
| lib/cooked_post_processor_spec.rb | 2234 | full quote of the previous post removed | covered | posts/quote_full_direct_reply |
| lib/cooked_post_processor_spec.rb | 2293-2354 | several quotes / not the first paragraph / setting off / nested (x4) | recordable | quote_full_* variants of quote_full_direct_reply |
| lib/cooked_post_processor_spec.rb | 2369-2580 | the same with full names leading (x6) | recordable | as above with display_name_on_posts |
| jobs/rebake_quoted_posts_for_user | - | posts quoting the user rebaked | covered | avatars/select_avatar_rebake_quoted |
| lib/post_destroyer_spec.rb | - | recovering a reply with quotes | covered | posts/post_recover_with_quote |
| models/post_mover_spec.rb | - | a moved first post that quotes | covered | topics/move_posts_first_post_with_quotes |

## PostAlerter (links, mentions, the job's options)

The rest of what a post's alerts are specced on, beyond quotes:
post_alerter_spec, the consolidation planner and the callers that pass
post_alert options.

| Spec | Line | Scenario | Class | Case / note |
|---|---|---|---|---|
| services/post_alerter_spec.rb | 680 | notifies the linked user | covered | posts/linked_post_alert (new, match: a topic, the system user's topic, a post linked; one notification for the user linked twice) |
| services/post_alerter_spec.rb | 710 | :before_create_notifications_for_users | needs-harness | DiscourseEvent subscription |
| services/post_alerter_spec.rb | 718 | staged user, restricted category allowing strangers | recordable | linked_staged_restricted (setup: a staged user, a category with email_in_allow_strangers) |
| services/post_alerter_spec.rb | 737 | no notification with linked post notifications off | covered | posts/linked_notify_off (new, match) |
| services/post_alerter_spec.rb | 746 | still notifies by default | covered | posts/linked_post_alert |
| services/post_alerter_spec.rb | 2965 | a deleted linked post: the topic's first post | recordable | linked_deleted_post (setup: link_post_id to a deleted post) |
| services/post_alerter_spec.rb | 1979 | email only to users not on the incoming email's CC list | covered | incoming_mail/incoming_reply_cc_skips_email, incoming_mail/incoming_reply_alert (new, match) |
| services/notifications/consolidation_planner_spec.rb | 37 | links below the threshold stay apart | covered | posts/linked_post_alert |
| services/notifications/consolidation_planner_spec.rb | 60 | link notifications consolidate into one | covered | posts/linked_consolidate (new, match: another poster's and an old one left alone) |
| services/notifications/consolidation_planner_spec.rb | 91 | the consolidated link notification updated | covered | posts/linked_consolidated_update (new, match) |
| models/notification_spec.rb | 993 | .populate_acting_user: the acting user and original_name | covered | notifications/notifications_acting_user_* and user_menu_messages_acting_user (new, match: the system user's logo, a username with no user) |
| services/user_silencer_spec.rb | 65 | the silence message's post_alert skips the email | covered | admin_users/silence_system_message_alert (new, match) |
| lib/post_creator_spec.rb | 425 | post_alert_options skip the notification email | covered | admin_users/silence_system_message_alert |
| services/post_alerter_spec.rb | 1125 | an admin mentioned outside the message isn't notified | covered | messages/pm_mention_moderator_plain (new, match) |
| models/topic_spec.rb | 2932-2949 | all_allowed_users: moderators in a flagged message or an official warning, not otherwise (x4) | covered | messages/pm_mention_moderator_flagged, pm_mention_moderator_warning, pm_mention_moderator_plain (new, match) |
| lib/guardian_spec.rb | 671 | moderators see a flagged private message | covered | messages/pm_mention_moderator_flagged (the mentioned moderator's notification passes can_see) |
| services/post_alerter_spec.rb | 765, 774, 796 | @here: unrelated users left out, repliers and staff-colour posters notified, whisperers not for a non-staff author (x3) | covered | posts/here_mention_post_alert (new, match) |
| services/post_alerter_spec.rb | 769 | @here does nothing once a user is named here | recordable | here_mention_user_named_here (setup: a user named here) |
| services/post_alerter_spec.rb | 784 | whisperers notified for a staff author | covered | posts/here_mention_staff_whisper (new, match) |
| services/post_alerter_spec.rb | 805 | only max_here_mentioned users | covered | posts/here_mention_max (new, match) |
| lib/guardian_spec.rb | - | can_mention_here? outside here_mention_allowed_groups | covered | posts/here_mention_not_allowed (new, match) |
| services/post_alerter_spec.rb | 820 | @group mentions: notified, GroupMention rows, not mentionable, members and owners levels | covered | posts/group_mention_post_alert, group_mention_not_mentionable, group_mention_members_level, group_mention_members_level_member (new, match) |
| services/post_alerter_spec.rb | 850 | a user mention takes precedence | covered | posts/group_mention_user_precedence (new, match) |
| services/post_alerter_spec.rb | 890 | :before_create_notifications_for_users | needs-harness | DiscourseEvent subscription |
| services/post_alerter_spec.rb | - | max_users_notified_per_group_mention | covered | posts/group_mention_too_many (new, match) |
| services/post_alerter_spec.rb | 1093, 1140, 1147-1221 | group mentions in messages: only those allowed in it (x9) | covered | messages/pm_group_mention (new, match); the rest recordable as variants |
| services/post_alerter_spec.rb | 2128, 2239, 2283 | nested topics: the OP told of a root reply as replied, bucket 1 in the data, watchers still told | covered | posts/nested_root_reply, posts/nested_by_default (new, match) |
| services/post_alerter_spec.rb | 2170, 2291, 2300 | nested topics: a reply in its parent's bucket, replies in a bucket consolidated | covered | posts/nested_reply_to_post, posts/nested_bucket_consolidate (new, not_yet: only nested_view_post_stats differs, HasNestedReplyStats) |
| services/post_alerter_spec.rb | 2136-2210, 2313-2422 | nested topics: other buckets, whispers, conversion mid-life, flat regression guards (x9) | recordable | variants of the nested_* cases |
| services/post_alerter_spec.rb | 3024-3339 | group SMTP (x13) | unported | jobs/post_alert.rs:555, ROADMAP 166 |
