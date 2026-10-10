# Request-spec coverage

Discourse's request specs (`spec/requests/*_controller_spec.rb` at the
vendored commit, `vendor/discourse/DISCOURSE_REF`) used as the checklist
for every controller action the port routes (src/routes/mod.rs). One row
per `it` (near-identical ones merged, marked (xN)); Line is the `it`'s
line in the spec.

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
| PostsController#create | 11 | 9 | 9 | 58 |
| PostsController#update | 3 | 9 | 3 | 21 |
| PostsController#destroy | 3 | 2 | 0 | 5 |
| PostsController#recover | 1 | 2 | 0 | 1 |
| PostsController#revisions (and #latest_revision) | 2 | 10 | 6 | 11 |
| PostsController#markdown_num | 2 | 3 | 0 | 5 |
| TopicsController#status | 14 | 1 | 0 | 14 |
| TopicsController#destroy | 5 | 1 | 0 | 6 |
| TopicsController#timings | 5 | 1 | 1 | 0 |
| TopicsController#show | 79 | 0 | 33 | 89 |
| ListController#private_messages* | 5 | 13 | 0 | 6 |
| PostActionsController#destroy | 5 | 2 | 0 | 0 |
| PostActionsController#create | 9 | 2 | 0 | 8 |
| DraftsController#index | 2 | 4 | 0 | 2 |
| DraftsController#show | 1 | 0 | 0 | 0 |
| DraftsController#create | 10 | 4 | 0 | 3 |
| DraftsController#destroy | 1 | 2 | 0 | 4 |
| BookmarksController#create | 4 | 1 | 1 | 0 |
| BookmarksController#update | 1 | 0 | 0 | 0 |
| BookmarksController#toggle_pin | 1 | 0 | 0 | 0 |
| BookmarksController#destroy | 3 | 1 | 0 | 0 |
| NotificationsController#index | 17 | 7 | 2 | 8 |
| NotificationsController#mark_read | 7 | 0 | 0 | 0 |
| UsersController#bookmarks | 10 | 1 | 1 | 6 |
| UsersController#user_menu_bookmarks | 3 | 5 | 1 | 1 |
| UploadsController#create | 6 | 10 | 2 | 8 |
| ReviewablesController#index | 4 | 8 | 2 | 11 |
| ReviewablesController#perform | 3 | 2 | 1 | 16 |
| Admin::SiteSettingsController#update | 13 | 3 | 2 | 22 |
| Admin::UsersController#suspend | 6 | 3 | 1 | 8 |
| Admin::UsersController#unsuspend | 1 | 2 | 1 | 0 |
| Admin::UsersController#silence | 4 | 5 | 0 | 6 |
| Admin::UsersController#unsilence | 1 | 5 | 0 | 0 |
| Admin::EmailController#handle_mail | 3 | 3 | 0 | 0 |
| UsersController#create | 11 | 17 | 16 | 41 |
| UsersController#perform_account_activation | 4 | 3 | 2 | 2 |
| UsersController#email_login | 4 | 1 | 3 | 0 |
| UsersController#password_reset_update (PUT /u/password-reset/:token) | 5 | 0 | 4 | 12 |
| UsersController#update (PUT /u/:username) | 5 | 4 | 5 | 37 |
| UsersController#show (GET /u/:username) | 3 | 4 | 13 | 9 |
| UsersController#summary | 1 | 4 | 6 | 0 |
| UserActionsController#index | 4 | 14 | 2 | 0 |
| SessionController#create (POST /session) | 8 | 16 | 15 | 3 |
| SessionController#destroy (DELETE /session/:username) | 1 | 8 | 3 | 0 |
| SessionController#forgot_password | 3 | 17 | 5 | 0 |
| SessionController#redeem_password_reset_code | 2 | 3 | 6 | 1 |
| SessionController#current | 2 | 1 | 2 | 0 |
| SessionController#csrf, #get_honeypot_value | 0 | 0 | 0 | 0 |
| StaticController#enter (POST /login) | 0 | 1 | 20 | 0 |
| ForumsController#status | 2 | 0 | 4 | 0 |
| ListController#latest (and #index generics) | 7 | 27 | 3 | 17 |
| ListController#top (+ /top/:period redirect) | 3 | 2 | 2 | 5 |
| ListController#hot | 0 | 0 | 0 | 2 |
| ListController#category_default / #category_latest / #category_none_* (incl. /l/<filter>, set_category) | 7 | 19 | 4 | 12 |
| ListController user lists (#unread #new #unseen #read #posted #bookmarks) | 2 | 3 | 0 | 3 |
| CategoriesController#index | 4 | 9 | 5 | 12 |
| SearchController#query | 3 | 12 | 8 | 5 |
| SearchController#show | 8 | 8 | 5 | 5 |
| TagsController#index | 22 | 0 | 1 | 9 |
| TagsController#show (+ show in category) | 20 | 0 | 0 | 9 |
| SiteController#site | 0 | 0 | 0 | 1 |
| SiteController#basic_info | 2 | 0 | 0 | 0 |
| RobotsTxtController#builder | 1 | 1 | 0 | 0 |
| RobotsTxtController#index | 4 | 8 | 2 | 0 |
| SitemapController | 6 | 4 | 0 | 0 |
| UserAvatarsController#show_proxy_letter | 0 | 1 | 1 | 0 |
| StylesheetsController (color definitions) | 0 | 0 | 4 | 10 |
| **All (67 sections)** | **389** | **308** | **207** | **514** |

## Cases added from this inventory

94 cases, recorded on the reference and replayed. 68 matched as added:

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
login_required_lists_anonymous.

The 26 that differed when they were added have since been fixed, each on
its own branch (`git log --merges --grep fix/`), so all 94 match.

## PostsController#create

`spec/requests/posts_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 1570 | action requires login | covered | reply_anonymous |
| 1577 | regular user publishing a global banner (archetype banner) | unported | Unsupported("archetypes other than private messages") |
| 1609 | with api > memoizes duplicate requests (wpid) | unported | API post memoizer not ported (README) |
| 1644 | with api > valid JSON when enqueued | unported | Unsupported("the review queue (approve_unless_allowed_groups)") |
| 1663 | with api > import_mode suppresses notifications | recordable | reply_api_key_import_mode (master key via setup SQL as reply_api_key, run_jobs post_alert; port ignores import_mode, expect a diff) |
| 1715 | with api > external_id | unported | external_id in UNPORTED_CREATE_PARAMS |
| 1735 | with api > email PM recipient with PMs disabled | unported | Unsupported("messages to email addresses") |
| 1763, 1782, 1803 | with api > whispers for regular users / staff bool / staff string (x3) | unported | whisper in UNPORTED_CREATE_PARAMS |
| 1823 | with api > does not advance draft | recordable | new_topic_api_key_keeps_draft (setup: api key 9101 + drafts row for new_topic) |
| 1841 | with api > category does not exist ("invalid") -> 400 | recordable | new_topic_invalid_category (same path via session) |
| 1863 | with api > invalid embed_url | unported | embed_url in UNPORTED_CREATE_PARAMS |
| 1881, 1901 | with api > unlist_topic admin / non-admin (x2) | unported | unlist_topic in UNPORTED_CREATE_PARAMS |
| 1935-2044 | logged in > fast typing > queue / no silence / first topic / closed / too long (x6) | unported | Unsupported("the review queue (fast typers)") |
| 2056 | category-Y reviewers vs queued reply in X | unported | Unsupported("category posting review modes"); also multi-user |
| 2084 | auto_silence_first_post_regex | unported | Unsupported("auto_silence_first_post_regex") |
| 2105 | silence watched words | unported | Unsupported("watched words") |
| 2128, 2163 | message to a group (x2) | unported | Unsupported("messages to groups") |
| 2188 | nested_post param | unported | nested_post in UNPORTED_CREATE_PARAMS |
| 2203 | protects against dupes | covered | reply_duplicate (new, matches) |
| 2220 | cannot create in a disallowed category -> 403 | covered | new_topic_restricted_category (new, matches) |
| 2234-2321 | tags restricted / disabled / no permission / by name / enabled / over limit (x6) | unported | tags in UNPORTED_CREATE_PARAMS |
| 2355, 2367 | content localization tags (x2) | unported | tags in UNPORTED_CREATE_PARAMS |
| 2380 | iframe with encoded userinfo not persisted | recordable | new_topic_iframe_not_allowlisted |
| 2399, 2428, 2460 | oEmbed provider HTML stripped (x3) | needs-harness | stub_request + Jobs.run_immediately (also Unsupported oneboxes) |
| 2489 | creates topic and post with right attributes | covered | new_topic |
| 2509 | regular users replying to whispers | unported | whisper param; Unsupported("replies to whispers") |
| 2554 | posts_controller_create_user modifier | needs-harness | plugin modifier registration |
| 2568 | topic_custom_fields, none permitted -> 400 | unported | topic_custom_fields in UNPORTED_CREATE_PARAMS |
| 2594-2645 | permitted custom fields / staff-only / meta_data (x4) | needs-harness | plugin register_editable_topic_custom_field |
| 2665 | uncategorized topic (category "") | recordable | new_topic_uncategorized (settings allow_uncategorized_topics true, uncategorized_category_id 1; seed has -1) |
| 2685 | reply to a PM post with image_sizes | unported | image_sizes in UNPORTED_CREATE_PARAMS (plain PM reply is pm_reply) |
| 2714 | creates a private post (several recipients, odd usernames) | covered | pm_new_two_recipients (Turkish-dotted username edge not covered) |
| 2743 | target_recipients empty -> no_user_selected | covered | pm_no_recipients (new, matches) |
| 2762 | PM archetype with topic_id -> create_pm_on_existing_topic | covered | pm_on_existing_topic (new, matches) |
| 2780 | with errors > does not succeed (raw "test") | covered | reply_too_short |
| 2786 | spam host threshold for TL0 | unported | Unsupported("posting as a new user (trust level 0)") |
| 2803 | allow_uncategorized_topics false > no category | covered | new_topic_no_category (new, matches) |
| 2815 | allow_uncategorized_topics false > as staff | recordable | new_topic_no_category_staff (admin) |
| 2830, 2851 | slow mode with auto_track false (x2) | unported | auto_track in UNPORTED_CREATE_PARAMS; Unsupported("slow mode") |
| 2869 | enable_user_status off > no mentioned_users | covered | reply_with_markdown (setting defaults to false) |
| 2885, 2909, 2916 | enable_user_status > mentioned_users with status / empty / unknown user (x3) | recordable | mention_user_status (setting + user_statuses row via setup SQL; port has no mentioned_users, expect a diff) |
| 2929, 2947 | unlist_topic staff / non-staff (x2) | unported | unlist_topic in UNPORTED_CREATE_PARAMS |
| 2964 | mentionable_groups modifier | needs-harness | plugin modifier (not a request either) |
| 2981-3025 | shared drafts (x4) | unported | shared_draft in UNPORTED_CREATE_PARAMS |
| 3052-3104 | is_warning staff / string / false / normal user (x4) | unported | is_warning in UNPORTED_CREATE_PARAMS |
| 3127, 3142, 3189 | no_bump skip / string / regular user 400 (x7 across contexts) | unported | no_bump in UNPORTED_CREATE_PARAMS |
| 3157 | creates the post and bumps the topic (admin, mod, TL4) | covered | reply (topic bumped_at compared) |
| 3204-3244 | featured links (x4) | unported | featured_link in UNPORTED_CREATE_PARAMS |

## PostsController#update

`spec/requests/posts_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 695 | action requires login | recordable | edit_anonymous |
| 714 | TL1 wiki edit does not publish hidden profile onebox | needs-harness | stubs CookedPostProcessor#get_size, run_immediately, sign_out + GET (two identities) |
| 770 | wiki editor changing another's topic title -> 403, body edit ok | unported | Unsupported("posts#update title, image sizes and bypass_bump"); the wiki body edit half is proposed as edit_wiki |
| 817 | TL1 wiki editor moving restricted topic to public category | unported | Unsupported("posts#update fields beyond raw and edit_reason") (category_id) |
| 845 | regular > TL0/TL1 after edit time limit -> 422 too_late_to_edit | covered | edit_too_late (new, matches) |
| 859 | regular > TL2 after edit time limit | recordable | edit_too_late_tl2 (user2 post 37, tl2_post_edit_time_limit 8) |
| 873 | regular > passes image sizes through | unported | image_sizes refused; also a mock expectation |
| 878 | regular > passes edit reason through | covered | edit |
| 886 | regular > edit conflict -> 409 | covered | edit_conflict |
| 893 | regular > post param missing -> 400 | recordable | edit_missing_post |
| 900 | regular > cannot see the post (PM) -> 403 | recordable | edit_pm_outsider (user2 PUT /posts/48) |
| 906 | regular > OP removed from their PM -> 403 | recordable | edit_pm_removed_author (setup: delete topic_allowed_users row for user1 on topic 42) |
| 913 | regular > updates raw, trailing whitespace stripped | recordable | edit_trailing_whitespace |
| 921 | regular > extracts links from the new body | recordable | edit_with_link |
| 931 | regular > deleted post not updatable | recordable | edit_deleted_post (user2 PUT /posts/45 -> 404) |
| 943 | staff > edit reason limit on a small action | unported | Unsupported("editing posts other than regular ones") |
| 976 | staff > posts in deleted topics | unported | Unsupported("editing deleted posts") |
| 987 | staff > invalid first-post title edit rolls back | unported | title param refused; watched words |
| 1029 | staff > no bump for whispers | needs-harness | freeze_time |
| 1045, 1058, 1071, 1097, 1121 | bypass_bump variants (x5) | unported | Unsupported("posts#update title, image sizes and bypass_bump") (nested one via "fields beyond raw") |
| 1086 | bumps the topic when bypass_bump not provided | recordable | edit_last_post_bumps (admin edits post 52, the last post of 35) |
| 1151, 1160 | group moderator editing category description (x2) | unported | Unsupported("category group moderation"), Unsupported("editing category descriptions") |
| 1183, 1207 | category group moderator who cannot / can see (x2) | unported | Unsupported("category group moderation") |
| 1215, 1235 | change to disallowed / approval-required category (x2) | unported | category_id refused |
| 1256 | links without permission | unported | Unsupported("links from users outside post_links_allowed_groups") |
| 1284 | plugin_permitted_update_params | needs-harness | plugin registration |
| 1317, 1332, 1347 | reply_to_post_number reparent / clear / invalid (x3) | unported | reply_to_post_number refused ("fields beyond raw and edit_reason") |

## PostsController#destroy

`spec/requests/posts_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 406 | action requires login | recordable | post_delete_anonymous |
| 411 | cannot see the post (PM) -> 404 | recordable | post_delete_pm_outsider (user2 DELETE /posts/48) |
| 421 | self deletions disabled -> 403 | covered | post_delete_self_disabled (new, matches) |
| 430 | member of delete_all_posts_and_topics_allowed_groups deletes another's post | covered | post_delete_allowed_group (new, matches) |
| 443 | uses a PostDestroyer (moderator) | covered | post_delete_staff (the mock expectation itself is not portable) |
| 459-510 | force_destroy: not yet deleted / time / two users / moderators / log (x5) | unported | Unsupported("permanently deleting posts and topics (force_destroy)"); also time travel and multi-user |

## PostsController#recover

`spec/requests/posts_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 661 | action requires login | recordable | post_recover_anonymous |
| 664 | cannot see the post (PM) -> 404 | recordable | post_recover_pm_outsider (user2 PUT /posts/48/recover) |
| 672 | self deletion/recovery disabled -> 403 | covered | post_recover_self_disabled (new, matches) |
| 681 | author recovers their own deleted post | unported | Unsupported("authors recovering their own posts (user_recovered)"); staff recovery is post_recover |

## PostsController#revisions (and #latest_revision)

`spec/requests/posts_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 3265 | revision < 2 -> 400 | covered | revision_not_found |
| 3270 | coalesced grace-period edits attributed to the right editor | needs-harness | PostRevisor with revised_at (grace period coalescing), heavy fabrication |
| 3332 | id passed as an array does not leak another post's revision | recordable | revision_id_array (GET /posts/36/revisions/2.json?id[]=36&id[]=48 as user2, 404) |
| 3362, 3376, 3390 | adjacent hidden revision not disclosed (x3) | unported | Unsupported("hidden post revisions") |
| 3458, 3466 | diff budget exceeded (x2) | needs-harness | ONPDiff stub |
| 3479 | history private > anonymous 403 | recordable | revision_history_private_anonymous |
| 3484 | history private > regular user 403 | covered | revision_history_private (new, matches) |
| 3490 | history private > staff 200 | recordable | revision_history_private_staff |
| 3496 | history private > poster 200 | recordable | revision_history_private_poster (user1 edits 36 then GETs) |
| 3507 | history private > TL4 403 | recordable | revision_history_private_tl4 (user3) |
| 3522, 3529, 3536 | category group moderator (x3) | unported | Unsupported("category group moderation") |
| 3547, 3563 | hide / show a revision (x2) | unported | other actions (hide_revision/show_revision), routes not ported |
| 3587 | post hidden > users 404 | recordable | revision_hidden_post (setup hides 36 + revision row; user1 as author still 404) |
| 3593 | post hidden > admins 200 | recordable | revision_hidden_post_staff |
| 3603 | public history > anyone (anonymous) | recordable | revision_anonymous |
| 3608, 3639, 3661 | restricted tag names not disclosed (x3) | needs-harness | heavy fabrication (tags, tag groups, private category, tag revisions YAML) |
| 3692 | unseen reply target post numbers omitted | unported | reply_to_post_number edit refused, whispers |
| 3733 | names disabled > no acting_user_name | recordable | revision_names_disabled (enable_names false) |
| 3750, 3764 | deleted post / deleted topic for staff (x2) | unported | Unsupported("deleted posts for staff") |
| 3774 | tagged topic latest revision with tagging on/off | recordable | revision_latest_tagged_topic (setup revision row on post 42, topic 38 is tagged) |

## PostsController#markdown_num

`spec/requests/posts_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 4278 | anonymous GET /raw/:topic/1.json | covered | raw_post_anonymous (new) |
| 4287 | whole topic /raw/:topic_id | unported | route without post_number not ported |
| 4311 | hidden post > logged out 404 | recordable | raw_hidden_post_anonymous |
| 4320 | hidden post > regular user 404 | covered | raw_hidden_post (new, matches) |
| 4329 | hidden post > author sees it | recordable | raw_hidden_post_author |
| 4338 | hidden post > moderator sees it | recordable | raw_hidden_post_staff |
| 4347-4378 | whole topic with a hidden post, per viewer (x4) | unported | /raw/:topic_id not routed |

## TopicsController#status

`spec/requests/topics_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 1550 | needs you to be logged in | recordable | topic_status_anonymous (user null, PUT /t/35/status -> 403 not_logged_in) |
| 1558 | moderator > raises if you can't change it (plain user) | covered | topic_status_not_staff |
| 1564 | moderator > requires the status parameter | covered | topic_status_bad_params (new, matches) |
| 1569 | moderator > requires the enabled parameter | covered | topic_status_bad_params (new, matches) |
| 1574 | moderator > status not in the allowlist -> 400 | covered | topic_status_bad_params (new, matches) |
| 1579 | moderator > reopens a closed topic with an open timer, timer removed | unported | Unsupported("topics with timers"); plain reopen covered by topic_open |
| 1595 | moderator > enabled truthy variants ("t", "0", true) | covered | topic_close_enabled_variants (new, matches) |
| 1626 | group moderator > close | unported | Unsupported("category group moderation") |
| 1634 | group moderator > reopen | unported | Unsupported("category group moderation") |
| 1646 | group moderator > archive | unported | Unsupported("category group moderation") |
| 1656 | group moderator > unarchive | unported | Unsupported("category group moderation") |
| 1666 | group moderator > pin with until | unported | category group moderation; also Unsupported("pinning until a time") |
| 1679 | group moderator > unpin | unported | Unsupported("category group moderation") |
| 1686 | group moderator > unlist | unported | Unsupported("category group moderation") |
| 1697 | group moderator > relist | unported | Unsupported("category group moderation") |
| 1714 | TL4 > close visible topic | covered | topic_unpin_globally_tl4 (new, matches) |
| 1720 | TL4 > can't close restricted category topic | covered | topic_status_tl4_forbidden (new, matches) |
| 1727 | TL4 > can't close PM | covered | topic_status_tl4_forbidden (new, matches) |
| 1734 | TL4 > can't archive restricted topic | covered | topic_status_tl4_forbidden (new, matches) |
| 1741 | TL4 > can't pin restricted topic | covered | topic_status_tl4_forbidden (new, matches) |
| 1748 | TL4 > can't toggle visibility of restricted topic | covered | topic_status_tl4_forbidden (new, matches) |
| 1774 | API key scoped (category_id) > unpermitted category 403 | unported | Unsupported("granular API key scopes on this route") |
| 1792 | API key scoped > without category_id 403 | unported | granular API key scopes |
| 1809 | API key scoped > permitted category 200 | unported | granular API key scopes |
| 1838 | API key scope without param restrictions > updates | unported | granular API key scopes |
| - | (extra) closing an already closed topic: no small action, staff log still written | covered | topic_close_already_closed (new, matches) |
| - | (extra) pinned_globally disabled by TL4 (can_moderate branch) | covered | topic_unpin_globally_tl4 (new, matches) |
| - | (extra) visible with until (until ignored outside pinning) | covered | topic_relist_with_until (new, matches) |
| - | (extra) pinned with until / invalid until -> InvalidParameters | unported | Unsupported("pinning until a time (the unpin job)") |

## TopicsController#destroy

`spec/requests/topics_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 2121 | anonymous -> 403 | recordable | topic_delete_anonymous (user null, DELETE /t/35.json) |
| 2133 | logged in > without access (own topic, 48h old) -> 422 | covered | topic_delete_own_old (new, matches) |
| 2143 | with permission (moderator) deletes | covered | topic_delete |
| 2161 | member of delete_all_posts_and_topics_allowed_groups deletes | covered | topic_delete_allowed_group (new, matches) |
| 2182 | category group moderator who can't see -> 422 | unported | Unsupported("category group moderation") |
| 2206 | category group moderator who can see deletes | unported | Unsupported("category group moderation") |
| 2223 | force destroy > destroys deleted small actions too | unported | refuse_force_destroy (post_destroy.rs) |
| 2244 | force destroy > logs, cleans sensitive info | unported | refuse_force_destroy |
| 2261 | force destroy > refused if not all posts destroyed | unported | refuse_force_destroy |
| 2271 | force destroy > refused if small actions not deleted | unported | refuse_force_destroy |
| - | (extra) author deletes own fresh single-post topic (mark_for_deletion) | covered | topic_delete_own_fresh (new, matches) |
| - | (extra) outsider deletes a PM -> 422 | covered | topic_delete_pm_outsider (new, matches) |

## TopicsController#timings

`spec/requests/topics_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 6768 | ignores invalid timing values (whisper, 1000, last_read recovery from 999) | recordable | timings_last_read_recovery (setup topic_users.last_read_post_number=999); out-of-range half covered by timings_out_of_range, whisper half needs a whisper post row |
| 6797 | ignores invalid values from staff (whisper counted) | needs-harness | needs a fabricated whisper post (posts row, topic counters); none in seed |
| 6820 | no timings for a topic the user can't see (POST /t/:id/timings) -> 404 | covered | timings_restricted_topic (new, matches) |
| 6842 | records the topic timing | covered | timings_read_rest |
| 6862 | caps msecs at 2^31-1 | covered | timings_msecs_cap (new, matches) |
| - | (extra) PM outsider -> 404 | covered | timings_pm_outsider (new, matches) |
| - | (extra) anonymous | covered | timings_anonymous |

## TopicsController#show

`spec/requests/topics_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 3277 | topic not allowed (detailed_404 on) -> 403 | unported | Unsupported("detailed_404") |
| 3300 | allowed to a group > descriptive error with group name | unported | detailed_404 + custom group HTML |
| 3310 | renders canonicals (HTML, Cache-Control) | needs-harness | header/HTML-structure assertions |
| 3318 | 301 even if slug param does not match (/t/:id.json?slug=x, /t/:slug.json) | covered | show_slug_param_mismatch (GET; port ignores ?slug=) |
| 3338 | shows a topic correctly | covered | golden: GET /t/parity-fixture-replies-and-posters/35.json |
| 3343 | PM tag descriptions hidden from viewer who can't see PM tags | covered | show_pm_tags_hidden (settings tagging/pm_tags_allowed_for_groups, setup topic_tags on 42, user0) |
| 3359 | tags restricted to inaccessible categories hidden | covered | show_restricted_tags (setup category_tags tag->cat 3 on topic 38) |
| 3392 | links from hidden posts not exposed | unported | Unsupported("topic links with clicks"), hidden posts |
| 3433 | link_counts from unlisted topics hidden | unported | post link counts (README Unsupported) |
| 3476 | hidden post link counts hidden | unported | hidden posts, link counts |
| 3535 | blank-slug topic | unported | Unsupported("topics without a stored slug (Slug.for)") |
| 3544 | over-range page redirects to last page | covered | show_page_over_range (GET 35.json?page=2) |
| 3549 | over-range page to last multi-page page | needs-harness | 25 fabricated posts |
| 3558 | viewer-visible count for last page (whispers) | needs-harness | 20+ fabricated posts and a whisper |
| 3574 | slug in id param redirects | covered | show_slug_only (GET /t/parity-fixture-replies-and-posters) |
| 3579 | slug with a number in front | covered | show_numeric_slug (setup UPDATE topics slug) |
| 3588 | /t/:id/summary with id[] array -> 400 | unported | topics#summary route not served |
| 3594 | /t/:id/summary nested id -> 400 | unported | topics#summary route not served |
| 3600 | keeps post_number query param when redirecting | covered | show_slug_redirect_params (port drops ?post_number=) |
| 3605 | keeps page when redirecting | covered | show_slug_redirect_params |
| 3611 | page param as array | covered | show_slug_redirect_params |
| 3617 | scrubs invalid query params | covered | show_slug_redirect_params |
| 3623 | nested_replies_default serves topic route | unported | nested replies view not ported |
| 3632 | crawlers not redirected to nested view | unported | nested replies |
| 3642 | PMs not redirected to nested view | unported | nested replies |
| 3654 | embed_mode on nested topics | unported | nested replies / embed mode |
| 3663 | embed class_name on nested topics | unported | nested replies / embed mode |
| 3672 | invalid slug without id -> 404 | covered | show_not_found_variants (GET /t/nope-nope.json) |
| 3678 | slug and id match nothing -> 404 | covered | show_not_found_variants |
| 3683 | id beyond postgres int -> 404 | covered | show_not_found_variants |
| 3689 | print=false is not print mode | covered | golden: GET /t/.../35.json (port ignores print) |
| 3696 | no N+1 with primary/flair groups | needs-harness | query counting |
| 3741 | no N+1 loading mentioned users | needs-harness | query counting |
| 3766 | content localization > no N+1 | needs-harness | query counting |
| 3817 | localized onebox for reader language | needs-harness | localization fabrication, onebox |
| 3825 | localized onebox omitted when translation off | needs-harness | localization fabrication, onebox |
| 3842 | serialize_post_user_badges | needs-harness | badge fabrication / registered badges |
| 3884 | redirect keeps modifier-registered param | needs-harness | plugin modifier |
| 3909 | nil-slug topic exists, unknown id -> 404 | covered | golden: GET /t/999999.json |
| 3954 | detailed_404 off > anonymous (x9, HTML) | covered | golden: GET /t/42.json, /t/999999.json (JSON, not HTML) |
| 3954 | detailed_404 off > anonymous login required (x9, HTML 302) | covered | show_login_required |
| 3954 | detailed_404 off > anonymous login required json (x9, 403) | covered | show_login_required |
| 3954 | detailed_404 off > normal user (x9) | covered | golden: GET /t/about-the-staff-category/2.json as=user1, /t/40 as=user1, /t/43 as=user0 |
| 3954 | detailed_404 off > allowed user (x9) | covered | golden: GET /t/42.json as=user1, /t/2.json as=admin |
| 3954 | detailed_404 off > moderator (x9) | unported | Unsupported("viewing deleted topics as staff") |
| 3954 | detailed_404 off > admin (x9) | unported | Unsupported("viewing deleted topics as staff") |
| 3954 | detailed_404 on > anonymous/normal/allowed/moderator/admin (x45) | unported | Unsupported("detailed_404") |
| 3954 | detailed_404 on > anonymous login required (x9, 302) | covered | show_login_required (login check precedes detailed_404) |
| 4184 | does not record a topic view | needs-harness | Rails view tracking is deferred (Scheduler::Defer) |
| 4188 | records incoming link for invalid post_number | needs-harness | deferred IncomingLink |
| 4195 | records incoming links | needs-harness | deferred IncomingLink |
| 4202 | print disabled -> 403 | unported | /print not served |
| 4210 | print enabled renders print view | unported | /print not served |
| 4221 | application layout without print param | needs-harness | HTML layout assertions |
| 4246 | print hides restricted tags | unported | /print not served |
| 4254 | referer recorded across redirect | needs-harness | follow_redirect + deferred IncomingLink |
| 4263 | tracks a visit for html requests | needs-harness | deferred visit tracking |
| 4278 | reviews new user for promotion | needs-harness | deferred promotion review |
| 4299 | filters > correct set of posts | needs-harness | TopicView.chunk_size stub |
| 4352 | external permalink on deleted topic, XHR | unported | permalinks not ported |
| 4363 | external permalink, XHR, detailed_404 | unported | permalinks, detailed_404 |
| 4376 | external permalink non-XHR 301 | unported | permalinks not ported |
| 4394 | permalink on nonexistent topic, XHR | unported | permalinks not ported |
| 4405 | permalink on nonexistent topic, 301 | unported | permalinks not ported |
| 4439 | show filters > replies_to_post_number | unported | filter params not ported (ignored) |
| 4481 | show filters > filter_top_level_replies | unported | filter params not ported |
| 4516 | show filters > filter_upwards_post_id | unported | filter params not ported |
| 4532 | show filters > max_reply_history | unported | filter params not ported |
| 4554 | login required > logged in shows topic | covered | show_login_required_member (user0, setting login_required) |
| 4563 | login required > anon browser redirected to login | covered | show_login_required |
| 4568 | login required > anon json 403 | covered | show_login_required |
| 4573 | login required > valid API key shows topic | covered | show_login_required_api_key (setup api_keys row with key_hash) |
| 4580 | login required > invalid API key 403 (json, html) | covered | show_login_required (HTTP_API_KEY: bad header) |
| 4591 | X-Robots-Tag for unlisted | needs-harness | response headers not compared |
| 4597 | no X-Robots-Tag for normal | needs-harness | response headers not compared |
| 4603 | X-Robots-Tag when allow_index_in_robots_txt off | needs-harness | response headers not compared |
| 4611 | no incoming link without referer | needs-harness | deferred IncomingLink |
| 4616 | very long referer | needs-harness | deferred IncomingLink |
| 4635 | enable_user_status off > no mentions | covered | golden: GET /t/.../35.json (no mentioned_users key) |
| 4647 | enable_user_status > mentions with status | unported | Unsupported("user status on BasicUserSerializer") |
| 4668 | enable_user_status > empty mentioned_users without mentions | covered | show_user_status_no_mentions (setting only; port omits mentioned_users) |
| 4679 | enable_user_status > unknown user mentioned -> empty | covered | show_user_status_no_mentions |
| 4693 | escaped fragment off > app layout | needs-harness | HTML layout assertions |
| 4709 | escaped fragment on > app layout without param | needs-harness | HTML layout assertions |
| 4718 | escaped fragment on > crawler layout | needs-harness | HTML layout assertions |
| 4737 | clear_notifications via cookie (subfolder) | needs-harness | subfolder + cookie |
| 4755 | clear_notifications via header | covered | show_clear_notifications_header (Discourse-Clear-Notifications header, marks a seeded notification read) |
| 4771 | no read only header by default | needs-harness | response headers not compared |
| 4777 | readonly header when site read only | needs-harness | readonly mode + headers |
| 4786 | image-only topic meta description | needs-harness | HTML meta assertion, upload fixture |
| 4806 | image cdn url for schema markup | needs-harness | CDN config |
| 4818 | suggested topics only on last chunk | needs-harness | stub_const CHUNK_SIZE |
| 4836 | lazy_load_categories list | needs-harness | stub_const CHUNK_SIZE |

## ListController#private_messages*

`spec/requests/list_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 503 | pm tags > fails for non-staff | unported | /topics/private-messages-tags not routed |
| 510 | pm tags > fails for staff when empty | unported | private-messages-tags not routed |
| 519 | pm tags > succeeds for staff | unported | private-messages-tags not routed |
| 528 | pm tags > unicode tag | unported | private-messages-tags not routed |
| 538 | pm tags > direct link /u/:u/messages/tags | unported | users#show HTML, tags route |
| 547 | pm tags > only visible tagged PMs (+ group 404) | unported | private-messages-tags not routed |
| 576 | group new/unread 404 when PMs disabled for user | recordable | pm_group_new_pm_disabled (settings personal_message_enabled_groups=3, user1 trust_level_2/new) |
| 596 | group PMs shown to admin | recordable | pm_group_list_admin (setup topic_allowed_groups 43->12) |
| 606 | moderators-group PMs for moderator | recordable | pm_group_list_moderators (setup topic_allowed_groups -> group 2, admin) |
| 617 | moderator can't see another's group PMs | recordable | pm_group_list_moderator_other (setup makes user3 moderator-only) |
| 626 | group PMs sorted by posts_count | recordable | pm_group_list_order_posts (setup 42,43,44 allowed to group 12) |
| 658 | not found when user not in group | covered | golden: GET /topics/private-messages-group/user1/staff.json as=user1 |
| 663 | group's PMs listed | recordable | pm_group_list_member (setup topic_allowed_groups -> 12, user1) |
| 683 | unicode group name | recordable | pm_group_unicode_name (setup INSERT group + group_users) |
| 1381 | private_messages 403 for others | covered | golden: GET /topics/private-messages/user1.json as=user0 |
| 1387 | private_messages succeeds | covered | golden: GET /topics/private-messages/user1.json as=user1 |
| 1397 | private_messages order=activity | recordable | pm_list_order_activity (GET ?order=activity as user1) |
| 1430 | sent 403 for others | recordable | pm_sent_outsider (user0 GET private-messages-sent/user1) |
| 1436 | sent succeeds | covered | golden: GET /topics/private-messages-sent/user1.json as=user1 |
| 1455 | unread 404 for others | recordable | pm_unread_outsider (user0 GET private-messages-unread/user1) |
| 1461 | unread succeeds with tracking PM | covered | golden: GET /topics/private-messages-unread/user1.json as=user1 |
| 1501 | warnings 403 for unrelated | recordable | pm_warnings_outsider (user0 GET private-messages-warnings/user1) |
| 1508 | warnings shown to moderators and admins | recordable | pm_warnings_staff (setup INSERT user_warnings topic 43 user 3 by 1, admin) |
| 1519 | warning not listed for authoring moderator | recordable | pm_warnings_staff (second request: warnings/admin) |

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
| 106 | #create > does not reveal private post existence (PM post, then max id + 1) | covered | like_pm_not_participant (new, matches) |
| 128 | #create > 404 when flagging a hidden topic (flag_topic=true) | unported | Unsupported("flagging topics") in routes/post_actions.rs |
| 148 | #create > notify_user to a user with PMs disabled, 422 | unported | Unsupported("flags that send a message (notify user, notify moderators, illegal)") in flags.rs |
| 176 | #create > non-staff > forbids warnings when is_warning is true/"true" (x2) | unported | same message-flag Unsupported; the port checks requires_message before post_can_act, so it never reaches the 403 |
| 193 | #create > non-staff > allows notifying a user when is_warning is false/"false" (x2) | unported | message-flag Unsupported |
| 226 | #create > moderator > 400 when id is missing | recordable | like_missing_id (not proposed, no data change) |
| 231 | #create > moderator > 404 when the id is invalid (-1) | covered | like_pm_not_participant (new, matches) |
| 241 | #create > moderator > 400 when post_action_type_id is missing | recordable | like_missing_type (not proposed, no data change) |
| 246 | #create > moderator > does not reveal private post existence | covered | like_pm_not_participant (new, matches) |
| 258 | #create > moderator > creates a post action on a post | covered | like |
| 274 | #create > moderator > passes a list of taken actions through (flag after flag, 403) | covered | flag_twice |
| 290 | #create > moderator > passes the message through (notify_user) | unported | message-flag Unsupported |
| 305 | #create > moderator > passes the message through as warning | unported | message-flag Unsupported |
| 325 | #create > moderator > passes take_action through | covered | flag_take_action |
| 343 | #create > moderator > doesn't pass take_action through for non-staff | covered | flag_inappropriate (spec sends no take_action param either) |
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
| 125 | #create > 404 when the key is missing | recordable | draft_missing_key (not proposed, no data change) |
| 131 | #create > checks for a raw conflict on update | covered | draft_edit_conflict |
| 157 | #create > checks for a title conflict on update | recordable | draft_edit_title_conflict (postId 35, exact raw + wrong original_title; topic 35 untagged so the port's tag Unsupported is not hit; not proposed) |
| 177 | #create > checks for a tag conflict on update | unported | Unsupported("the edit conflict check on tags") |
| 201 | #create > hidden tags when checking for tag conflict | unported | Unsupported("the edit conflict check on tags") |
| 234 | #create > tag objects format for tag conflict | unported | Unsupported("the edit conflict check on tags") |
| 260 | #create > can't trivially resolve conflicts (sequence behind, no draft) | covered | draft_stale_sequence_no_draft (new, matches) |
| 277 | #create > clean protocol for ownership handover (owner change bumps sequence) | recordable | draft_owner_handover (not proposed; mostly draft_update plus the owner column) |
| 327 | #create > out-of-sequence draft setting, 409 (x2: seq-1, seq+1) | covered | draft_out_of_sequence |
| 360 | #create > data too big, 400 | covered | draft_too_long (new, matches) |
| 378 | #create > data not proper JSON, 400 | covered | draft_invalid_data |
| 387 | #create > data not a string, 400 | recordable | draft_data_not_string (data[cat]=tomtom; not proposed) |
| 394 | #create > 403 when max drafts per user is reached (existing key still saves) | covered | draft_too_many (new, matches) |
| 433 | #create > does not leak conflict info for posts user cannot see | covered | draft_edit_conflict_hidden_post (new, matches) |

## DraftsController#destroy

`spec/requests/drafts_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 454 | #destroy > destroys drafts when required | covered | draft_destroy |
| 463 | #destroy > denies attempts to destroy unowned draft (admin, username param, not API) | unported | Unsupported("clearing another user's drafts") fires on any username param; Rails answers 403 InvalidAccess here, a cheap gap to close |
| 477 | #destroy > via API > admin targets another user with username | unported | Unsupported("clearing another user's drafts") |
| 495 | #destroy > via API > admin acts on self without username | recordable | draft_destroy_api_key (api_keys row via setup SQL as in reply_api_key; same behaviour as draft_destroy, not proposed) |
| 512 | #destroy > via API > non-admin passing username gets 403 | unported | Unsupported("clearing another user's drafts") |
| 532 | #destroy > via API > non-admin acts on self without username | recordable | draft_destroy_api_key (as 495) |
| 549 | #destroy > via API > 404 when admin targets a nonexistent username | unported | Unsupported("clearing another user's drafts") |

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
| 41 | #index > returns recent notifications (recent, not silent) | covered | notifications_recent_bump (new, matches) |
| 46 | #index > returns notification history | covered | golden: GET /notifications.json as=user1 |
| 57 | #index > marks notifications as viewed (recent bumps seen_notification_id) | covered | notifications_recent_bump (new, matches) |
| 68 | #index > silent does not mark viewed | covered | golden: GET /notifications.json?recent=true&silent=true as=user1 |
| 79 | #index > not marked viewed in read-only mode | needs-harness | Discourse.received_redis_readonly! |
| 94 | #index > invalid limit params (recent=true) | recordable | notifications_recent_bad_limit (golden only has non-recent limit=61; not proposed) |
| 102 | #index > limit param and load_more_notifications offset | covered | golden: GET /notifications.json?limit=2&offset=1 as=user1, ?username=user1 as=admin |
| 124 | #index > all filters (read / unread) | covered | golden: filter=read, filter=unread as=user1; notifications_read_one |
| 193 | #index > sidebar > unread (high priority first) at the top | covered | golden: GET /notifications.json?recent=true&silent=true as=user1 (user1 has high-priority unread PM notifications) |
| 209 | #index > sidebar > last-seen reviewable not bumped in read-only mode | needs-harness | redis read-only |
| 222 | #index > sidebar > last-seen reviewable not bumped without review access | covered | notifications_recent_bump (new, matches) |
| 229 | #index > sidebar > last-seen reviewable not bumped with silent (admin) | unported | admin + pending reviewable: Unsupported("pending_reviewables (Reviewable.user_menu_list_for)") |
| 243 | #index > sidebar > last-seen reviewable not bumped without bump param (admin) | unported | pending_reviewables Unsupported |
| 252 | #index > sidebar > bumps last_seen_reviewable_id | unported | pending_reviewables Unsupported (and the bump runs in Scheduler::Defer) |
| 270 | #index > sidebar > includes pending reviewables | unported | pending_reviewables Unsupported |
| 285 | #index > sidebar > excludes reviewables claimed by others | unported | pending_reviewables Unsupported |
| 305 | #index > sidebar > no reviewables without review queue access | covered | golden: GET /notifications.json?recent=true&silent=true as=user1 |
| 343 | #index > filter_by_types > filters to the given types | covered | golden: ?recent=true&filter_by_types=liked,mentioned&silent=true as=user1 |
| 355 | #index > filter_by_types > excludes other users' notifications | covered | same golden (other users' liked/mentioned rows exist in the seed) |
| 368 | #index > filter_by_types > respects limit | recordable | notifications_types_limit (GET only; not proposed) |
| 376 | #index > username not found, 404 | recordable | notifications_unknown_username (GET only; not proposed) |
| 411 | #index > inaccessible topics > recent filter | covered | notifications_filtered_badge_and_topic (new, matches) |
| 421 | #index > inaccessible topics > without recent | covered | notifications_filtered_badge_and_topic (new, matches) |
| 444 | #index > disabled badges > recent > filters disabled badge | covered | notifications_filtered_badge_and_topic (new, matches) |
| 455 | #index > disabled badges > recent > enable_badges=false | recordable | notifications_badges_disabled (not proposed) |
| 469 | #index > disabled badges > paged > filters disabled badge | covered | notifications_filtered_badge_and_topic (new, matches) |
| 480 | #index > disabled badges > paged > enable_badges=false | recordable | notifications_badges_disabled (not proposed) |
| 497 | #index > show_user_menu_avatars > acting_user_avatar_template | unported | Unsupported("populate_acting_user on notifications") |
| 513 | #index > names disabled > no mentioner display_name | recordable | notifications_names_disabled (enable_names=false, user1 has mention 51 with display_name; not proposed) |
| 550 | #index > avatars on, names off > no liker name | unported | Unsupported("populate_acting_user on notifications") |
| 591 | #index > localizations > localized fancy_title when enabled | recordable | notifications_localized_title (topic_localizations row + users.locale + settings; port has no localization branch and no Unsupported; not proposed) |
| 603 | #index > localizations > plain fancy_title when disabled | covered | golden: GET /notifications.json as=user1 (default settings) |
| 768 | not logged in > #index requires authentication | covered | golden: GET /notifications.json (anon) |
| 817 | user api keys > allows access to notifications#index | unported | port has no User-Api-Key auth |

## NotificationsController#mark_read

`spec/requests/notifications_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 617 | marks every notification as read | covered | notifications_read_all |
| 622 | can update a single notification | covered | notifications_read_one |
| 634 | updates the read status (unread counts) | covered | notifications_read_all |
| 666 | #mark_read > by id > marks a notification as read | covered | notifications_read_one |
| 674 | #mark_read > by id > doesn't mark another user's notification | covered | notifications_read_other (new, matches) |
| 685 | #mark_read > by type > marks notifications as read | covered | notifications_read_types |
| 718 | #mark_read > by type > doesn't mark other users' notifications | covered | notifications_read_types (row diff covers every user's notifications) |

## UsersController#bookmarks

`spec/requests/users_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 8145 | #bookmarks > list of serialized bookmarks | covered | golden: GET /u/user1/bookmarks.json as=user1 |
| 8154 | #bookmarks > including custom registered bookmarkables | needs-harness | register_test_bookmarkable (plugin registration) |
| 8169 | #bookmarks > .ics file in date order | unported | .ics format not routed (/u/:username/bookmarks only, JSON) |
| 8215 | #bookmarks > .ics excludes topics the user cannot see | unported | .ics not routed |
| 8242 | #bookmarks > .ics excludes reminders older than 3 months | unported | .ics not routed |
| 8257 | #bookmarks > .ics no HTML encoding | unported | .ics not routed |
| 8269 | #bookmarks > .ics URI delimiters | unported | .ics not routed |
| 8283 | #bookmarks > another user's bookmarks, 403 | covered | golden: GET /u/user1/bookmarks.json as=user0 |
| 8289 | #bookmarks > moderators can't view another user's bookmarks | recordable | bookmarks_list_moderator (setup makes user3 a moderator; same is_admin branch as the user0 golden; not proposed) |
| 8306 | #bookmarks > no bookmarks found | covered | golden: GET /u/user0/bookmarks.json as=user0 |
| 8316 | #bookmarks > no bookmarks for the search | covered | golden: GET /u/user1/bookmarks.json?q=zzzz as=user1 |
| 8326 | #bookmarks > invalid limit params | covered | golden: limit=0, limit=50, limit=abc as=user1 |
| 8345 | #bookmarks > localized fancy_title when enabled | unported | Unsupported("content localization on bookmarks") |
| 8358 | #bookmarks > plain fancy_title when disabled | covered | golden: GET /u/user1/bookmarks.json as=user1 |
| 8377 | #bookmarks excerpts > first post of the topic as the excerpt | covered | golden: GET /u/user1/bookmarks.json as=user1 (topic bookmark 2) |
| 8393 | #bookmarks excerpts > no excerpt for hidden post bookmarks | covered | bookmarks_list_hidden_posts (new, matches) |
| 8415 | #bookmarks excerpts > hidden first posts not listed or searched | covered | bookmarks_list_hidden_posts (new, matches) |
| 8467 | #bookmarks excerpts > bookmarkable_url to the first post without the option | covered | golden: GET /u/user1/bookmarks.json as=user1 |

## UsersController#user_menu_bookmarks

`spec/requests/users_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 8449 | #bookmarks excerpts > user menu bookmarkable_url to the first unread post | covered | golden: GET /u/user1/user-menu-bookmarks.json as=user1 (topic bookmark 2 with topic_users read state) |
| 8632 | logged out, 404 | covered | golden: GET /u/user1/user-menu-bookmarks.json (anon) |
| 8641 | another user's list, 403 | covered | golden: GET /u/user1/user-menu-bookmarks.json as=admin |
| 8646 | unread bookmark_reminder notifications only (read one dropped) | recordable | bookmarks_menu_read_reminder (setup a second, read, reminder notification; not proposed) |
| 8666 | bookmarks not tied to unread reminders; read reminder or no bookmark_id brings them back | recordable | bookmarks_menu_read_reminder (setup marks notification 52 read / strips bookmark_id; not proposed) |
| 8708 | fills up USER_MENU_LIST_LIMIT | needs-harness | stub_const |
| 8747 | no unread reminders for bookmarks the user can no longer access | recordable | bookmarks_menu_inaccessible_reminder (setup bookmark + reminder notification on post 2, Staff category; not proposed) |
| 8765 | no unread reminders when the bookmark was deleted | recordable | bookmarks_menu_deleted_bookmark (setup DELETE bookmark 5; not proposed) |
| 8775 | same, when the notification only has bookmark_id | recordable | bookmarks_menu_deleted_bookmark (setup strips bookmarkable_type/id from notification 52) |
| 8798 | show_user_menu_avatars > acting_user_avatar | unported | Unsupported("populate_acting_user on notifications") |

## UploadsController#create

`spec/requests/uploads_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 8 | requires you to be logged in | covered | upload_anonymous |
| 28 | logged in > rate limited > 429 past max_uploads_per_minute | needs-harness | rate limits (and avatar PNG uploads) |
| 49 | logged in > expects upload_type (400 without it) | recordable | upload_missing_type (a fixture file without upload_type, 400 param missing). The avatar uploads that follow are unported (Unsupported "avatar uploads") |
| 66 | logged in > accepts the deprecated type param, logs a deprecation | unported | Unsupported "the deprecated type param" (also `allow(Discourse).to receive`) |
| 81 | logged in > is successful with an image (avatar, CreateAvatarThumbnails) | unported | Unsupported "avatar uploads"; PNG refused (image_optim) |
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
| 294 | logged in > could not determine image dimensions (fake.jpg) | recordable | upload_fake_image: needs a generated fixture fake.jpg (non-image bytes). The port answers `not_supported_or_corrupted` when the bytes aren't detected while Rails expects `size_not_found`, worth recording |
| 311 | system user > errors when system_user_max_attachment_size_kb is unset | recordable | upload_system_user_too_large: system via an api_keys setup row + Api-Key/Api-Username headers (as incoming_reply_api_key), large.txt |
| 324 | system user > accepts large files from the system user | recordable | upload_system_user_large (system_user_max_attachment_size_kb above large.txt's size, max_attachment_size_kb 1) |
| 332 | system user > rejects above the system limit; both limits low; above the attachment limit (x3) | recordable | upload_system_user_limits_* (same api key setup; large.txt must be over 10 KB for the "10 KB" variants) |

## ReviewablesController#index

`spec/requests/reviewables_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 5 | anonymous > denies listing | recordable | review_list_anonymous (403) |
| 46 | logged in > #index > empty JSON when nothing to review | recordable | review_list_empty (the seed has no reviewables) |
| 53 | #index > loads the acting user behind author penalties in one query | needs-harness | SQL query tracking; penalized authors are also Unsupported |
| 74 | #index > returns JSON with reviewable content | covered | review_list |
| 99 | #index > action ids scoped to their reviewable | covered | review_list_many (two flagged posts, per-reviewable action ids) |
| 149 | #index > trashed topics and posts > shown to staff (moderator) | unported | Unsupported "the review queue for moderators" |
| 167 | #index > trashed topics and posts > shown to category mods | unported | Unsupported "the review queue for category group moderators" |
| 185 | #index > trashed > excludes inaccessible whispers for category mods | unported | Unsupported "the review queue for category group moderators" |
| 199 | #index > filtering by flag reason (score_type) | recordable | review_list_score_type (setup with two flags of different score types) |
| 224 | #index > filtering by flagged_by | recordable | review_list_flagged_by |
| 249 | #index > filtering by score (min_score=1000) | recordable | review_list_empty with `min_score` (not a controller param; nothing in the queue) |
| 256 | #index > supports offsets | covered | review_list_many_paged |
| 263 | #index > filtering by type (ReviewableUser) | unported | Unsupported "reviewables other than flagged posts in the queue" |
| 271 | #index > invalid type is a 400 | recordable | review_list_bad_type |
| 276 | #index > filtering by status (ReviewableUser) | covered | review_list_reviewed (status filter; ReviewableUser rows themselves are Unsupported) |
| 301 | #index > invalid status is a 400 | recordable | review_list_bad_status |
| 306 | #index > filtering by category_id | recordable | review_list_category (category 4 vs another id) |
| 326 | #index > ReviewableUser serializer | unported | Unsupported "reviewables other than flagged posts in the queue" |
| 389 | #index > date range > empty, and matching (x2) | unported | Unsupported "the review queue's date filters" |
| 415 | #index > user custom field allowed by a plugin | needs-harness | plugin registration (allow_public_user_custom_field) |
| 441 | #index > filtering by id | unported | Unsupported "the review queue's ids filter" |
| 453 | #index > no N+1 with localizations | unported | Unsupported "content localization in the review queue" |
| 500 | #index > notes > no N+1 | unported | Unsupported "claimed topics and notes in the queue" |
| 1656 | #index > PM reviewable hidden from an outside moderator | unported | Unsupported "the review queue for moderators" (and ReviewablePost) |

## ReviewablesController#perform

`spec/requests/reviewables_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 10 | anonymous > denies performing | recordable | review_anonymous (403) |
| 342 | #index block > ReviewableUser not found error on second delete_user | unported | Unsupported "reviewable types other than flagged posts" |
| 361 | #index block > reject_reason too long on delete_user | unported | Unsupported "reviewable types other than flagged posts" |
| 888 | #perform > 404 to category moderators for an inaccessible whisper | unported | Unsupported "category group moderation" |
| 917 | #perform > statuses of other reviewables resolved by delete_user_block | unported | Unsupported "review actions that delete, edit, restore or penalize" |
| 939 | #perform > 404 when the reviewable does not exist | covered | review_missing |
| 944 | #perform > validates the presence of an action (403) | covered | review_unknown_action (new, matches), review_already_reviewed (new, matches) |
| 949 | #perform > ensures the user can see the reviewable (moderator, not reviewable_by_moderator) | unported | Unsupported "reviewables for groups" |
| 955 | #perform > can properly return errors (queued post) | unported | Unsupported "reviewable types other than flagged posts" |
| 965 | #perform > requires a version parameter (422) | recordable | review_no_version |
| 972 | #perform > succeeds for a valid action (approve_user) | unported | Unsupported "reviewable types other than flagged posts" (covered in spirit by review_agree_and_keep) |
| 992 | #perform > no email when send_email is false | unported | ReviewableUser, Unsupported "reviewable types other than flagged posts" |
| 1002 | #perform > removed likes stay removed after delete_and_agree | unported | Unsupported "review actions that delete, edit, restore or penalize" |
| 1022 | #perform > releases a deleted topic's claim before a silence | unported | Unsupported "reviewable claiming", deleted posts, agree_and_silence |
| 1046 | #perform > claims > required, claimed by others, optional (x3) | unported | Unsupported "reviewable claiming" (queued posts too) |
| 1070 | #perform > simultaneous perform > wrong version is a 409 | covered | review_stale_version |
| 1088 | #perform > flagged post deleted > agree_and_keep_deleted works, no 403 (x2) | unported | Unsupported "reviewing flags on deleted posts" |
| 1163 | plugin API reviewable params passed to perform | needs-harness | plugin registration, MessageBus.expects |
| 1674 | #perform > PM reviewable 404 for an outside moderator | unported | Unsupported "reviewable types other than flagged posts" (ReviewablePost), moderator counts |

## Admin::SiteSettingsController#update

`spec/requests/admin/site_settings_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 248 | admin > sets the value when the param is present (title) | covered | setting_string (title itself is Unsupported "settings whose change handlers write") |
| 254 | admin > bulk updates settings | unported | Unsupported "bulk site setting updates" |
| 271 | admin > error for hard deprecated settings | covered | setting_hard_deprecated (new) |
| 282 | admin > works for soft deprecated settings | needs-harness | stub_deprecated_settings!(override: true); no real soft deprecation in SETTINGS |
| 291 | admin > not a configurable setting (clear_cache!) with update_existing_user | covered | setting_unknown (with update_existing_user the port hits Unsupported "backfilling user preferences") |
| 304 | admin > deprecated enable_personal_messages with override false | covered | setting_unknown (the setting no longer exists at this commit) |
| 317 | admin > value can be a blank string (title) | recordable | setting_blank_string (short_title set via setup SQL, then ""; title is Unsupported) |
| 323 | admin > blank string for selectable_avatars | unported | Unsupported "setting types other than string, number, bool and enum" (uploaded_image_list) |
| 330 | admin > sanitizes integer values ("1,000") | covered | setting_integer_delimited (new, matches) |
| 337 | admin > sanitizes file_size_restriction values | unported | Unsupported "setting types other than string, number, bool and enum" |
| 344 | admin > sanitizes negative integer values | recordable | setting_negative_integer (pending_users_reminder_delay_minutes "-1", min -1) |
| 358 | admin > default user options > updates, doesn't update, email_digests (x3) | unported | Unsupported "default_* settings" |
| 405 | admin > default navigation menu > backfill job or not (x3) | unported | Unsupported "default_* settings" / "backfilling user preferences" |
| 478 | admin > default categories > update or not, MessageBus (x3) | unported | Unsupported "default_* settings" |
| 580 | admin > default tags > update or not, MessageBus (x3) | unported | Unsupported "default_* settings" |
| 636 | admin > upload site settings > remove, reset, update (x3) | unported | Unsupported "setting types other than string, number, bool and enum" (upload) |
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

## Admin::UsersController#suspend

`spec/requests/admin/users_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 522 | admin > suspends user | covered | user_suspend |
| 522 | moderator > suspends user | covered | user_suspend_as_moderator (new, matches) |
| 547 | admin > doesn't allow suspending a staff user | covered | user_suspend_self (new, matches) |
| 547 | moderator > doesn't allow suspending a staff user | recordable | user_suspend_admin_as_moderator |
| 558 | admin, moderator > staff via other_user_ids (x2) | unported | Unsupported "penalizing several users at once (other_user_ids)" |
| 578 | admin > checks if user is suspended (409) | unported | Unsupported "suspending an already suspended user" |
| 609 | admin > webhook > enqueues user_suspended | needs-harness | web hook fabrication and EmitWebHookEvent job args; webhooks not ported |
| 629 | admin > fails if the reason is too long | recordable | user_suspend_reason_too_long (301 chars, 400) |
| 641 | admin > requires suspend_until and reason | covered | user_suspend_without_reason |
| 655 | admin > other_user_ids too big | unported | Unsupported "penalizing several users at once (other_user_ids)" |
| 677 | admin > associated post > have, delete, preserve category topics (x2), delete replies, edit (x6) | unported | Unsupported "acting on a post with a penalty" |
| 754 | admin > can send a message to the user | covered | user_suspend_with_message |
| 775 | admin > also prevents use of any api keys | recordable | user_suspend_blocks_api_key: api_keys setup row for user2, bookmark POST with Api-Key headers, suspend, bookmark again (403). Mixes the admin cookie with key headers in one case |
| 803 | admin > can silence multiple users | unported | Unsupported "penalizing several users at once (other_user_ids)" |
| 823 | moderator > cannot edit an unrelated static doc post | unported | Unsupported "acting on a post with a penalty" |
| 844 | non-staff > 404 | covered | user_suspend_not_admin |

## Admin::UsersController#unsuspend

`spec/requests/admin/users_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 872 | admin > webhook > enqueues user_unsuspended | needs-harness | web hook fabrication, job args; webhooks not ported |
| 886 | admin > unsuspends a user granted moderation while suspended | recordable | user_unsuspend_moderator (user3 suspended and made moderator in setup) |
| 907 | moderator > prevents unsuspending a staff user (admin, moderator) | covered | user_unsuspend_staff_as_moderator (new, matches) |
| 921 | moderator > can unsuspend a regular user | recordable | user_unsuspend_as_moderator |

## Admin::UsersController#silence

`spec/requests/admin/users_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 2181 | admin > 404 if the user doesn't exist | covered | user_silence_missing_user (new) |
| 2186 | admin > doesn't allow silencing another admin | recordable | user_silence_self (admin target, 403) |
| 2196 | admin > another admin via other_user_ids | unported | Unsupported "penalizing several users at once (other_user_ids)" |
| 2208 | admin > punishes the user for spamming | covered | user_silence |
| 2220 | admin > can have an associated post (post_action edit) | unported | Unsupported "acting on a post with a penalty" |
| 2248 | admin > sets the provided duration (date only) | recordable | user_silence_date_only (silenced_till "2027-01-01") |
| 2262 | admin > sends the provided message | covered | user_silence_with_message (new, matches) |
| 2277 | admin > checks if user is silenced (409) | unported | Unsupported "silencing a user with a silence on record" |
| 2305 | admin > can silence multiple users | unported | Unsupported "penalizing several users at once (other_user_ids)" |
| 2317 | admin > fails if the reason is too long | recordable | user_silence_reason_too_long |
| 2329 | admin > other_user_ids too big | unported | Unsupported "penalizing several users at once (other_user_ids)" |
| 2354 | moderator > silences user | recordable | user_silence_as_moderator |
| 2367 | moderator > doesn't allow silencing another admin | covered | user_silence_admin_as_moderator (new, matches) |
| 2377 | moderator > another admin via other_user_ids | unported | Unsupported "penalizing several users at once (other_user_ids)" |
| 2394 | non-staff > 404 | recordable | user_silence_not_staff |

## Admin::UsersController#unsilence

`spec/requests/admin/users_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 2409 | admin, moderator > 404 if the user doesn't exist (x2) | recordable | user_unsilence_missing_user |
| 2414 | admin > unsilences the user | covered | user_unsilence |
| 2414 | moderator > unsilences the user | recordable | user_unsilence_as_moderator |
| 2439 | moderator > prevents unsilencing a staff user (admin, moderator) | recordable | user_unsilence_staff_as_moderator (admin silenced in setup) |
| 2456 | non-staff > 404 | recordable | user_unsilence_not_staff |

## Admin::EmailController#handle_mail

`spec/requests/admin/email_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 299 | admin > 400 if neither email parameter is present | recordable | incoming_missing_param |
| 305 | admin > enqueues with the deprecated email param, warns | covered | incoming_deprecated_email_param (new, matches) |
| 320 | admin > decodes email_encoded and enqueues | covered | incoming_reply |
| 338 | admin > normalizes invalid UTF-8 bytes | covered | incoming_invalid_utf8 (new) |
| 349 | moderator > 404 | recordable | incoming_moderator |
| 349 | non-staff > 404 | recordable | incoming_not_staff |

## UsersController#create

`spec/requests/users_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 28 | full account registration flow > handles the honeypot and challenge fields | covered | signup |
| 857 | rejects signup from a logged-in browser session | covered | signup_logged_in (new, matches) |
| 865 | rejects signup from an admin browser session | recordable | signup_logged_in_admin (same as above with user admin) |
| 873 | allows signup with an API key | recordable | signup_api_key (setup SQL api_keys row for user1, Api-Key header); port rejects every authenticated request (accounts.rs:775), Rails skips honeypot for API |
| 885 | email param missing > 400 | recordable | signup_missing_email |
| 897 | encoded email decoding to invalid email > blocked | unported | signup.rs:278 invalid email Unsupported |
| 910 | encoded email decoding to valid email > blocked | unported | signup.rs:278 |
| 923 | sets the user locale to I18n.locale | needs-harness | I18n.stubs(:locale) |
| 930 | requires invite code when specified | unported | signup.rs:175 invite codes |
| 948 | timezone guess on signup > sets the timezone | recordable | signup_timezone |
| 964 | protected profile attributes ignored on unauthenticated signup | covered | signup_ignores_protected_fields (new, matches) |
| 988 | discourse connect enabled > blocks local registration | recordable | signup_discourse_connect (settings discourse_connect_url, discourse_connect_secret, enable_discourse_connect in that order) |
| 1005 | local logins disabled > blocks without authenticator | unported | signup.rs:224 local logins off |
| 1010 | local logins disabled > blocks with a regular api key | unported | signup.rs:224 |
| 1016 | local logins disabled > works with an admin api key | unported | signup.rs:224 |
| 1054 | external_ids > creates User record | needs-harness | plugin auth provider registration + UserAuthenticator stub |
| 1080 | external_ids > error for unknown provider | needs-harness | plugin auth provider registration |
| 1096 | non active user > 403 when local logins disabled | unported | signup.rs:224 |
| 1103 | non active user > error when new registrations are disabled | recordable | signup_registrations_disabled (allow_new_registrations false) |
| 1114 | non active user > creates a user correctly | covered | signup |
| 1133 | must approve users > creates a user correctly | unported | signup.rs:215 approval at signup |
| 1161 | normalize_emails + hide_email_address_taken > account_exists email | unported | signup.rs:299 taken email |
| 1188 | email exists > error if hide_email_address_taken disabled | unported | signup.rs:299 |
| 1199 | email exists > success if hide_email_address_taken enabled | unported | signup.rs:299 |
| 1236 | creating as active > does not create the user as active | covered | signup_ignores_protected_fields (new, matches) |
| 1245 | active > regular api key does not activate | recordable | signup_api_key_active (setup SQL api key for user1) |
| 1260 | active > admin api key creates active approved user | unported | must_approve_users at signup, signup.rs:215 |
| 1284 | active > admin api key > reviewable for active unapproved user | unported | signup.rs:215 |
| 1303 | active > admin api key > no reviewable for inactive user | unported | signup.rs:215 |
| 1318 | active > admin api key > developer not active | needs-harness | UsernameCheckerService.expects |
| 1330 | active > admin api key > does not copy admin locale | recordable | signup_api_admin_locale (setup SQL admin api key, UPDATE users SET locale='fr' WHERE id=1, allow_user_locale) |
| 1346 | active > admin api key > auto approves configured domain | unported | signup.rs:215 auto_approve_email_domains |
| 1369 | creating as staged > does not create the user as staged | covered | signup_ignores_protected_fields (new, matches) |
| 1379 | staged > regular api key does not stage | recordable | signup_api_key_staged (setup SQL api key) |
| 1396 | staged > admin api key creates staged user | recordable | signup_api_admin_staged (setup SQL admin api key) |
| 1408 | staged > developer not staged | needs-harness | UsernameCheckerService.expects |
| 1426 | active user (confirmed email) > enqueues a welcome email | needs-harness | User.any_instance.stubs(:active?) and expects(:enqueue_welcome_message) |
| 1437 | active user > shows the 'active' message | needs-harness | same stubs |
| 1444 | active user > logs in the new user | needs-harness | same stubs |
| 1451 | active user > indicates active in response | needs-harness | same stubs |
| 1458 | active user > fails when new registrations disabled | recordable | signup_registrations_disabled (fail_with runs before the stubbed part) |
| 1494 | auth records > creates Twitter user information | needs-harness | OmniAuth mock |
| 1507 | auth records > error when email changed from validated | needs-harness | OmniAuth mock |
| 1522 | auth records > creates user when email validation required | needs-harness | OmniAuth mock |
| 1536 | auth records > auth_overrides username/name | needs-harness | OmniAuth mock |
| 1576 | no email in auth payload > creates the user | needs-harness | OmniAuth mock |
| 1594 | creates user successfully but doesn't activate | covered | signup |
| 1632 | honeypot value wrong > does not create user | covered | signup_failed_challenge |
| 1648 | challenge answer wrong > does not create user | covered | signup_failed_challenge |
| 1663 | invite only > honeypot-style fake success | recordable | signup_invite_only (invite_only true, valid honeypot) |
| 1688 | password blank > failed signup (x2) | unported | signup.rs:302 password errors |
| 1701 | password too long > failed signup (x2) | recordable | signup_password_too_long (201 chars, fail_with) |
| 1707 | username too long (50000) > rejected without reflection | unported | signup.rs:190 overlong usernames |
| 1720 | password missing > failed signup (x2) | unported | signup.rs:302 |
| 1730 | reserved username > failed signup (x2) | covered | signup_reserved_username (new, matches) |
| 1743 | username matching a user route > failed signup (x2) | recordable | signup_route_username (username account-created) |
| 1749 | missing username > 400 | recordable | signup_missing_username |
| 1767 | Exception raised on save > failed signup (x2) | needs-harness | User.any_instance.stubs(:save).raises |
| 1785 | unknown enum user field value > failed signup (x2) | unported | signup.rs:221 user fields |
| 1798 | custom fields without values > failed signup (x2) | unported | signup.rs:221 |
| 1821 | user fields > multiselect > single value or array | unported | user_updater.rs REFUSED user_fields |
| 1831 | user fields > multiselect > rejects unregistered values | unported | user_updater.rs REFUSED user_fields |
| 1837 | user fields > multiselect > filters valid values | unported | same |
| 1843 | user fields > multiselect > allows registered values | unported | same |
| 1849 | user fields > multiselect > required on signup | unported | signup.rs:221 |
| 1863 | user fields > multiselect > required only on sign-up | unported | same |
| 1877 | user fields > multiselect > not required may be empty | unported | same |
| 1908 | user fields > dropdown > rejects unregistered values | unported | same |
| 1914 | user fields > dropdown > allows registered values | unported | same |
| 1920 | user fields > dropdown > required can't be nil | unported | same |
| 1930 | user fields > dropdown > optional may be nil | unported | same |
| 1954 | user fields > creates without optional field | unported | signup.rs:221 |
| 1965 | user fields > creates with optional field | unported | signup.rs:221 |
| 1977 | user fields > trims long fields | unported | signup.rs:221 |
| 2002 | only optional custom fields > creates without values | unported | signup.rs:221 |
| 2023 | staged account > claims it | unported | signup.rs:207 staged takeover |
| 2047 | staged account > works with custom fields | unported | signup.rs:207/221 |

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
| 131 | valid token > not approved > pending-approval response | unported | accounts.rs:650 must_approve_users |
| 143 | valid token > already logged in > 404 (GET) | unported | GET is users#activate_account, not routed; the PUT logged-in branch (accounts.rs:860) is recordable as activate_account_logged_in |
| 155 | destination_url cookie with query > redirect_to | needs-harness | request cookie |
| 167 | destination_url cookie > redirect_to | needs-harness | request cookie |
| 189 | invited to topic > redirects to the topic | recordable | activate_account_invited_topic (setup SQL invites, topic_invites, invited_users for user0, plus a known signup token); port always answers redirect_to null |

## UsersController#email_login

`spec/requests/users_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 6671 | enqueues the right email | covered | email_login (by username; spec uses the email, same lookup) |
| 6688 | staff writes only > moderator gets email | needs-harness | readonly mode in Redis |
| 6699 | staff writes only > admin gets email | needs-harness | readonly mode |
| 6710 | staff writes only > regular user 503 | needs-harness | readonly mode |
| 6722 | enable_local_logins_via_email disabled > 404 | covered | email_login_disabled (new, matches) |
| 6729 | invalid username or email > user_found false, no job | recordable | email_login_unknown (hide_email_address_taken false, login `@random`) |
| 6741 | hide_email_address_taken > generic response | covered | email_login (default hide_email_address_taken is true) |
| 6753 | already logged in > redirects to root | covered | email_login_logged_in (new) |

## UsersController#password_reset_update (PUT /u/password-reset/:token)

`spec/requests/users_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 341 | invalid token > proper error message | unported | accounts.rs:433 unknown token |
| 366 | valid token > returns success, auth tokens removed | covered | password_reset |
| 390 | valid token > disallows double password reset | unported | second PUT hits accounts.rs:433 |
| 404 | valid token > first admin redirected to wizard | unported | HTML format (redirect_to wizard_path) |
| 415 | valid token > sets the timezone if missing | covered | password_reset_timezone (new) |
| 427 | valid token > deletes user associated accounts | covered | password_reset_associated_accounts (new, matches) |
| 452 | valid token > logs the password change | covered | password_reset |
| 474 | previewed token superseded by forgot_password | unported | accounts.rs:433 |
| 497 | rate limits reset passwords | needs-harness | rate limits + freeze_time |
| 519 | rate limits by username | needs-harness | rate limits + REMOTE_ADDR |
| 551 | TOTP required > invalid token does not change | unported | accounts.rs:443 second factors |
| 577 | TOTP required > valid token changes password | unported | accounts.rs:443 |
| 628 | security key > valid challenge changes password | unported | accounts.rs:443 |
| 642 | security key > fake TOTP token rejected | unported | accounts.rs:443 |
| 655 | security key > authentication fails | unported | accounts.rs:443 |
| 681 | submit change > fails when password blank | unported | accounts.rs:476 error responses |
| 689 | submit change > fails when password too long | unported | accounts.rs:476 |
| 700 | submit change > logs in the user | covered | password_reset |
| 708 | submit change > not approved, not logged in | unported | accounts.rs:650 must_approve_users in EmailToken.confirm |
| 720 | staff writes only > staff can reset | needs-harness | readonly mode |
| 734 | staff writes only > non-staff blocked | needs-harness | readonly mode |

## UsersController#update (PUT /u/:username)

`spec/requests/users_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 2963 | guest > 403 | recordable | users_update_anonymous (user null, PUT /u/user0.json) |
| 2969 | auth_overrides_name > name not updated | recordable | users_update_auth_overrides_name |
| 2985 | username with a period > updates | recordable | users_update_dotted_username (setup SQL renames user0 to user.zero after the harness login) |
| 2997 | staff > uneditable user field and title | unported | user_updater.rs REFUSED user_fields, :295 title |
| 3030 | allows the update (muted, tags, backgrounds, languages) | unported | tag tracking (:335), backgrounds (REFUSED), array options (:705); the muted part is covered by prefs_muted |
| 3085 | does not update username, email, password | covered | users_update_ignores_protected_fields (new, matches) |
| 3102 | watched tags in everyone tag group | unported | user_updater.rs:335 |
| 3123 | locale differs > updates the locale | recordable | users_update_locale (allow_user_locale true, locale fa_IR) |
| 3129 | locale differs > updates the title | unported | user_updater.rs:295 |
| 3149 | editable user field > updates | unported | REFUSED user_fields |
| 3162 | editable user field > cannot be blank | unported | REFUSED user_fields |
| 3175 | editable user field > trims large fields | unported | REFUSED user_fields |
| 3187 | editable user field > retains existing | unported | REFUSED user_fields |
| 3219 | notification schedule | unported | REFUSED user_notification_schedule |
| 3253 | uneditable user field not updated | unported | REFUSED user_fields |
| 3276 | custom_field > only allowed fields | needs-harness | plugin register_editable_user_custom_field (port also refuses custom_fields) |
| 3292 | custom_field > alongside a user field | needs-harness | plugin registration |
| 3312 | custom_field > alongside a user field during creation (API) | needs-harness | plugin registration |
| 3342 | custom_field > secure with no registered fields | unported | REFUSED custom_fields |
| 3361 | custom_field > staff edits staff-editable fields | needs-harness | plugin registration |
| 3380 | returns user JSON | covered | prefs_text_size (and the other prefs_* cases) |
| 3390 | sidebar > links kept when params absent | covered | prefs_profile (user1 has seeded sidebar links, no sidebar params) |
| 3401 | sidebar > remove all category links | unported | REFUSED sidebar_category_ids |
| 3411 | sidebar > category links only for accessible categories | unported | REFUSED sidebar_category_ids |
| 3446 | sidebar > remove all tag links | unported | REFUSED sidebar_tag_names |
| 3458 | sidebar > tag links rejected when tagging off | unported | REFUSED sidebar_tag_names |
| 3469 | sidebar > tag links only for browsable tags | unported | REFUSED sidebar_tag_names |
| 3506 | without permission > forbidden | covered | prefs_other_user |
| 3547 | external_ids > create UserAssociatedAccount | unported | REFUSED external_ids |
| 3565 | external_ids > destroy UserAssociatedAccount | unported | REFUSED external_ids |
| 3581 | external_ids > create SingleSignOnRecord | unported | REFUSED external_ids |
| 3595 | external_ids > update SingleSignOnRecord | unported | REFUSED external_ids |
| 3613 | external_ids > delete SingleSignOnRecord | unported | REFUSED external_ids |
| 3631 | external_ids > both in one call | unported | REFUSED external_ids |
| 3665 | external_ids > unknown provider error | unported | REFUSED external_ids |
| 3680-3848 | user status (sets/updates/clears, other users, disabled, staff) (x14) | unported | REFUSED status |
| 3866 | plugin users_controller_update_user_params modifier | needs-harness | plugin modifier |
| - | (extra, no spec it) enable_names off > name ignored | covered | users_update_names_disabled (new, matches) |

## UsersController#show (GET /u/:username)

`spec/requests/users_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 5546 | anon > returns success | covered | golden: GET /u/system.json |
| 5557 | anon > returns a hidden profile | recordable | users_show_hidden_profile (setup SQL user_options.hide_profile for user1, ?skip_track_visit=true) |
| 5570 | anon > 403 when profiles hidden from public | recordable | users_show_hidden_from_public (hide_user_profiles_from_public) |
| 5579 | anon > 403 to crawlers | needs-harness | HTML crawler page body, Googlebot UA |
| 5588 | anon > tracks a profile view | needs-harness | UserProfileView.expects; Rails adds the view in Scheduler::Defer |
| 5594 | anon > skips tracking | needs-harness | expects(:add).never |
| 5604 | logged in > returns success | covered | golden: GET /u/user1.json as=user1 |
| 5614 | logged in > unknown username not found | covered | golden: GET /u/nobody.json |
| 5620 | logged in > inactive user not found | recordable | users_show_inactive (setup SQL user2 active=false, user0 views) |
| 5627 | logged in > show_inactive_accounts shows inactive | recordable | users_show_inactive_accounts (port's find_active ignores the setting); also admin viewing an inactive user |
| 5634 | logged in > invalid access 403 | needs-harness | Guardian.any_instance.expects(:can_see?) |
| 5642 | logged in > tracks a signed-in view | needs-harness | expects |
| 5651 | logged in > own profile not tracked | needs-harness | expects |
| 5656 | logged in > skips tracking | needs-harness | expects |
| 5665 | by external_id > matching | unported | /u/by-external not routed: routes/users.rs:92 unknown /u route |
| 5671 | by external_id > not matching | unported | routes/users.rs:92 |
| 5687 | external provider > non-admin 403 | unported | routes/users.rs:92 |
| 5693 | external provider > fetch the user | unported | routes/users.rs:92 |
| 5699 | external provider > disabled provider 404 | unported | routes/users.rs:92 |
| 5705 | external provider > missing user 404 | unported | routes/users.rs:92 |
| 5721 | include_post_count_for > visible posts only | unported | routes/users.rs:191 |
| 5727 | include_post_count_for > no access | unported | routes/users.rs:191 |
| 5736 | include_post_count_for > staff all post types | unported | routes/users.rs:191 |
| 5747 | returns the user (HTML) | needs-harness | HTML profile page body |
| 5754 | private profile not in HTML | needs-harness | HTML profile page body |
| 5767 | username with a period (HTML) | needs-harness | HTML profile page body |
| 5786 | auth token IP > moderator without can_see_ip | needs-harness | DiscourseIpInfo mmdb |
| 5799 | auth token IP > admin sees client_ip | needs-harness | DiscourseIpInfo mmdb |
| 5809 | auth token IP > own tokens | needs-harness | DiscourseIpInfo mmdb |

## UsersController#summary

`spec/requests/users_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 5048 | caches per automatic translation preference | needs-harness | translate cookie, Accept-Language, topic_localizations, Rails summary cache |
| 5064 | generates summary info | covered | golden: GET /u/user1/summary.json |
| 5098 | hidden posts with links excluded | needs-harness | fabricated linked hidden/deleted posts; Rails summary cache not reset between cases |
| 5115 | hide_user_profiles_from_public > 200 for logged in | recordable | users_summary_hidden_from_public_logged_in (port 403s everyone, routes/users.rs:198) |
| 5123 | hide_user_profiles_from_public > 403 anonymous | recordable | users_summary_hidden_from_public |
| 5133 | hide_profile > 404 | recordable | users_summary_hidden_profile (setup SQL hide_profile for user1) |
| 5138 | hide_profile > 200 when allow_users_to_hide_profile off | recordable | users_summary_hide_profile_disallowed (status is safe, body may be cached) |
| 5147 | flair > automatic groups flair | needs-harness | fabricated liker + likes; summary cache |
| 5159 | flair > icon flair | needs-harness | fabricated group + likes; summary cache |
| 5180 | flair > image flair | needs-harness | fabricated group + upload + likes |
| 5211 | content localization > localized titles | needs-harness | I18n.stubs(:locale) |

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
| 147 | anonymous > private types 404 (xN) | recordable | user_actions_private_anonymous |
| 160 | logged in > private types 404 (xN) | recordable | user_actions_private_other_user |
| 173 | moderator > private types 404 (xN) | recordable | user_actions_private_moderator (setup SQL user3 moderator after login) |
| 186 | admin > private types 200 (xN) | recordable | user_actions_private_admin |
| 212 | bad hash data for filter/username/offset/limit (x4) | recordable | user_actions_bad_params |
| 433 | content localization > translated title and excerpt | recordable | user_actions_localized (setup SQL topic/post localizations, viewer locale ja) |
| 443 | localization disabled > no preloading | needs-harness | query counting |
| 453 | localization > translated excerpt of deleted post to staff | recordable | user_actions_localized_deleted_staff |

## SessionController#create (POST /session)

`spec/requests/session_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 3324 | read only > regular user blocked | needs-harness | readonly mode |
| 3329 | read only > admin blocked | needs-harness | readonly mode |
| 3342 | staff writes only > admin allowed | needs-harness | readonly mode |
| 3348 | staff writes only > regular blocked | needs-harness | readonly mode |
| 3361 | local login disabled > 403 | recordable | session_login_local_logins_disabled |
| 3373 | SSO enabled > 403 | recordable | session_login_discourse_connect |
| 3382 | local login via email disabled > logs in | recordable | session_login_email_logins_off |
| 3392 | login missing > 400 | covered | golden: POST /session body=login=user1 (same param.require path) |
| 3398 | invalid password | covered | golden: POST /session body=login=user1&password=wrong |
| 3407 | overlong password | recordable | session_login_overlong_password |
| 3423 | suspended > suspension error | unported | session.rs:283 |
| 3441 | suspended forever | unported | session.rs:283 |
| 3459 | deactivated > activation error | covered | session_login_inactive (new, matches) |
| 3471 | success by username and password | covered | session_login_timezone (new) |
| 3496 | timezone param sets user_option timezone | covered | session_login_timezone (new) |
| 3513 | password expired > expired error | recordable | session_login_expired_password (setup SQL user_passwords.password_expired_at) |
| 3546 | security key only > blank params | needs-harness | webauthn credential + challenge fixtures |
| 3566 | security key only > invalid params | needs-harness | webauthn |
| 3589 | security key only > valid > logs in | needs-harness | webauthn |
| 3626 | security key disabled in background, TOTP on | needs-harness | webauthn |
| 3651 | TOTP > token missing > missing-second-factor error | recordable | session_login_totp_missing (setup SQL user_second_factors) |
| 3663 | TOTP > invalid TOTP token | covered | session_login_totp_invalid (new) |
| 3680 | TOTP > invalid backup code | recordable | session_login_backup_code_invalid (setup SQL totp + backup rows) |
| 3699 | TOTP > valid token logs in | needs-harness | time-based code |
| 3722 | TOTP > valid backup code logs in | needs-harness | hashed backup code fixture |
| 3747 | blocked IP | unported | session.rs:291 screened IPs |
| 3765 | strips leading @ | recordable | session_login_at_prefix |
| 3780 | login by email | recordable | session_login_by_email |
| 3792 | strips spaces from username | recordable | session_login_padded_username |
| 3798 | strips spaces from email | recordable | session_login_padded_email |
| 3814 | requires approval > unapproved not logged in | covered | session_login_not_approved (new, matches) |
| 3820 | requires approval > not approved message | covered | session_login_not_approved (new, matches) |
| 3827 | requires approval > unapproved admin logs in | recordable | session_login_not_approved_admin |
| 3845 | admin ip allowlist > admin at allowed IP | recordable | session_login_admin_ip_allowed (use_admin_ip_allowlist; 127.0.0.0/8 allow_admin is in the seed) |
| 3862 | admin ip allowlist > admin elsewhere blocked | needs-harness | REMOTE_ADDR |
| 3878 | admin ip allowlist > non-admin elsewhere ok | needs-harness | REMOTE_ADDR |
| 3901 | email not confirmed > not logged in | recordable | session_login_unconfirmed (setup SQL email_tokens confirmed=false for user1) |
| 3908 | email not confirmed > not activated message | recordable | session_login_unconfirmed |
| 3917 | email not confirmed + must approve > not approved message | recordable | session_login_unconfirmed_not_approved |
| 3928 | rate limits login | needs-harness | rate limits |
| 3946 | rate limits 2FA by IP | needs-harness | rate limits |
| 3972 | rate limits 2FA by login | needs-harness | rate limits |

## SessionController#destroy (DELETE /session/:username)

`spec/requests/session_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 4016 | removes session and auth token cookies (non-XHR 302) | recordable | session_logout_redirect (headers X-Requested-With "" ; status and the user_auth_tokens delete compare, Location does not) |
| 4025 | XHR returns redirect_url | recordable | session_logout (user0, DELETE /session/user0.json) |
| 4037 | SSO + login_required > /login-required | recordable | session_logout_sso_login_required (port never answers /login-required) |
| 4056 | plugins manipulate redirect URL | needs-harness | DiscourseEvent listener |
| 4071 | before_session_destroy event params | needs-harness | DiscourseEvent listener |
| 4091 | return_url absolute external rejected | recordable | session_logout_external_return_url |
| 4103 | return_url protocol-relative rejected | recordable | session_logout_protocol_relative_return_url |
| 4115 | return_url backslash variant rejected | covered | session_logout_backslash_return_url (new) |
| 4123 | return_url javascript: rejected | recordable | session_logout_javascript_return_url |
| 4135 | external return_url non-XHR redirects to root | needs-harness | Location header not recorded |
| 4143 | valid relative return_url | recordable | session_logout_relative_return_url |
| 4155 | `/` return_url beats logout_redirect | recordable | session_logout_root_over_logout_redirect |

## SessionController#forgot_password

`spec/requests/session_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 4244 | codes on > code to primary for username and secondary email | recordable | forgot_password_secondary_email (setup SQL secondary user_emails row for user1) |
| 4263 | codes on > expires reset and unscoped tokens | recordable | forgot_password_expires_tokens (setup SQL email_tokens rows for user1 and user2) |
| 4280 | codes on > logged-in user resets own password | recordable | forgot_password_own (user user1) |
| 4295 | codes on > staff reset another user > link | recordable | forgot_password_staff_for_user (user admin, login user1) |
| 4311 | codes on > same response for unknown when hidden | covered | forgot_password_unknown |
| 4326 | codes on > rate limits preserved | needs-harness | rate limits |
| 4339 | hide_email_address_taken > denies username | recordable | forgot_password_username_hidden (400) |
| 4346 | hide_email_address_taken > staff may use username | recordable | forgot_password_staff_for_user |
| 4356 | hide_email_address_taken > allows email (link flow) | recordable | forgot_password_link (enable_local_logins_via_code false) |
| 4365 | no login param > 400 | recordable | forgot_password_missing_login |
| 4370 | screens blocked IP | needs-harness | REMOTE_ADDR |
| 4392 | rate limiting | needs-harness | rate limits |
| 4431 | made up username > no token | covered | forgot_password_unknown |
| 4448 | local login disabled > 403 | covered | forgot_password_local_logins_disabled (new) |
| 4460 | SSO enabled > 403 (spec posts /session.json) | recordable | session_login_discourse_connect; forgot_password_discourse_connect for the real action |
| 4470 | local logins disabled > 403 (spec posts /session.json) | recordable | session_login_local_logins_disabled |
| 4476 | via email disabled > still makes a token | recordable | forgot_password_email_logins_off |
| 4483 | existing username > makes a token | recordable | forgot_password_link_by_username (hide false, codes off) |
| 4489 | existing username > enqueues an email | recordable | forgot_password_link_by_username |
| 4498 | system username > no token | recordable | forgot_password_system (hide false, login system) |
| 4504 | system username > no email | recordable | forgot_password_system |
| 4513 | staged > no token | recordable | forgot_password_staged (setup SQL users.staged for user2) |
| 4519 | staged > no email | recordable | forgot_password_staged |
| 4528 | staff writes only > staff allowed | needs-harness | readonly mode |
| 4535 | staff writes only > non-staff 503 | needs-harness | readonly mode |

## SessionController#redeem_password_reset_code

`spec/requests/session_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 4551 | exchanges code, changes password | covered | password_reset |
| 4568 | login and reset codes only for their own flow | needs-harness | SecureRandom stub; login-code endpoints |
| 4600 | existing second factor required | unported | accounts.rs:443 |
| 4623 | code from another browser rejected | needs-harness | second session (reset!) |
| 4640 | previous code invalidated by a new request | needs-harness | SecureRandom stub; only the last job's code is templatable |
| 4655 | latest code usable after rate-limited resend | needs-harness | rate limits |
| 4669 | code redeemed only once | covered | password_reset_code_reuse (new, matches) |
| 4680 | cleared by a later unknown account request | recordable | password_reset_code_after_unknown |
| 4689 | codes disabled > 404 | recordable | password_reset_code_disabled (enable_local_logins_via_code false) |
| 4696 | local logins disabled > 403 | recordable | password_reset_code_local_logins_disabled (literal code; port has no check, answers invalid code 200) |
| 4705 | staff writes only after requesting | needs-harness | readonly mode |
| 4714 | rate limits verification | needs-harness | rate limits |

## SessionController#current

`spec/requests/session_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 4726 | not logged in > 404 | covered | golden: GET /session/current.json |
| 4735 | logged in > user JSON | covered | golden: GET /session/current.json as=user1 |
| 4744 | featured topic the user cannot see omitted | recordable | session_current_hidden_featured_topic (setup SQL user_profiles.featured_topic_id = a staff-category topic for user0) |
| 4764 | anonymous shadow > master suspended stops auth | needs-harness | shadow user sign-in (second user) |
| 4784 | anonymous shadow > master deactivated stops auth | needs-harness | shadow user sign-in |

## SessionController#csrf, #get_honeypot_value

`spec/requests/session_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|

## StaticController#enter (POST /login)

`spec/requests/static_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 475 | no redirect path > root | recordable | login_enter_json (POST /login.json: Rails 302, the port only routes /login) |
| 482 | redirect path | needs-harness | Location header not recorded |
| 489 | full url on this host > path | needs-harness | Location (port also drops it to `/`, session.rs:447) |
| 496 | path with query kept | needs-harness | Location |
| 503 | period forcing a new host > root | needs-harness | Location |
| 510 | external full url > root | needs-harness | Location |
| 517 | javascript: > root | needs-harness | Location |
| 524 | array > root | needs-harness | Location |
| 531 | login page > root | needs-harness | Location |
| 538 | path containing /login kept | needs-harness | Location |
| 545 | invalid path > root | needs-harness | Location |
| 555 | subfolder > root | needs-harness | subfolder install |
| 562 | subfolder > login page > subfolder root | needs-harness | subfolder |
| 569 | subfolder > invalid > subfolder root | needs-harness | subfolder |
| 578 | sso_destination_url cookie > allowed | needs-harness | request cookie |
| 588 | sso_destination_url > wildcard domain | needs-harness | request cookie |
| 597 | sso_destination_url > not in secrets | needs-harness | request cookie |
| 607 | sso_destination_url > empty secrets | needs-harness | request cookie |
| 616 | sso_destination_url > malformed | needs-harness | request cookie |
| 625 | sso_destination_url > provider disabled | needs-harness | request cookie |
| 635 | sso_destination_url cookie deleted | needs-harness | request cookie / response cookies |

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
| 15 | #index > Klipy key not in preloaded site settings | unported | HTML data-preloaded |
| 31 | invalid params > 400 page negative | covered | list_latest_invalid_params (new) |
| 36 | invalid params > 400 page above max int (x2) | recordable | same branch as page=-1; port 400 plain text |
| 44 | invalid params > 400 before[1]=haxx | recordable | port ignores `before`, answers 200 |
| 49 | invalid params > 400 bumped_before[1]=haxx | recordable | port ignores the param, 200 |
| 54 | invalid params > 400 topic_ids[1]=haxx | covered | list_latest_invalid_params (new) |
| 59-154 | invalid params > 400 for category/order/ascending/min_posts/max_posts/status/filter/state/search/q/f/subset/group_name/tags/user/match_all_tags/no_subcategories/no_tags/exclude_tag non-scalar values (x20) | recordable | only page/per_page/ascending are validated by the port; the rest are dropped, so 200 vs 400 |
| 160 | legit requests return 200 (no_definitions, max_posts, min_posts, page=0/1/1999, search=, topic_ids[], tags[]) | covered | list_latest_invalid_params (new) |
| 194 | anonymous filters (latest, top, hot...) return 200 (xN) | covered | golden: GET /latest.json, /top.json, /hot.json?per_page=3 |
| 201 | filter on a set of topic ids | recordable | `/latest.json?topic_ids=35` (port ignores topic_ids, returns full list) |
| 210 | homepage title with short_site_description | unported | HTML title |
| 221 | structured data in HTML | unported | HTML |
| 231 | no N+1 with tags | needs-harness | SQL query counting |
| 281 | no N+1 with primary groups | needs-harness | SQL query counting |
| 340 | topics with tags > hidden tags not shown | recordable | setup tag_group with staff permission on tag 2, GET /latest.json anon; tag must drop from topic 35/41 |
| 355 | lazy load categories > categories + parents returned | recordable | settings lazy_load_categories_groups=4 (anonymous_users), GET /latest.json |
| 369 | lazy load categories > no categories key when off | covered | golden: GET /latest.json (default "") |
| 387 | login required > homepage shows no topics | covered | login_required_lists_anonymous (new, matches) |
| 412 | categories and X > category latest with no_subcategories=false includes subcategory topics | covered | golden: GET /c/general/4.json (subcategory topic 38 included by default; param is a no-op) |
| 420 | crawler titles > no title for default URL | unported | HTML crawler |
| 430 | crawler titles > title for non-default URLs | unported | HTML crawler |
| 440 | rss feed discovery > advertises feed | unported | HTML + RSS |
| 448 | rss feed discovery > omits feed without route | unported | HTML + RSS |
| 471 | crawler homepage renders for each filter | unported | HTML crawler |
| 481 | crawler homepage falls back when homepage is user-scoped | unported | HTML crawler |
| 795 | RSS > latest RSS | unported | .rss not routed |
| 802 | RSS > latest RSS with query params | unported | .rss not routed |
| 809 | RSS > sanitized feed URLs | unported | .rss not routed |
| 876 | RSS > subfolder links | unported | .rss not routed |
| 942 | RSS > exclude_tag in latest RSS | unported | .rss not routed |
| 2263 | content localization > crawler title localized | unported | HTML crawler + content localization |
| 2276 | content localization > title as-is without localization | unported | HTML crawler |
| 2303 | content localization > tl=ja crawler title | unported | HTML crawler |
| 2311 | content localization > no N+1 loading localizations | needs-harness | SQL query counting |

## ListController#top (+ /top/:period redirect)

`spec/requests/list_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 983 | renders top | covered | golden: GET /top.json |
| 988 | renders top with a period | covered | golden: GET /top.json?period=weekly |
| 993 | 400 for invalid period | covered | list_latest_invalid_params (new) |
| 999 | per page > per_page param used | recordable | `/top.json?per_page=5` |
| 1005 | per page > topics_per_period_in_top_page setting | recordable | settings topics_per_period_in_top_page=32, GET /top.json |
| 885 | RSS > top RSS | unported | .rss not routed |
| 891 | RSS > invalid period on top RSS | unported | .rss not routed |
| 897 | RSS > #{period} top RSS (xN) | unported | .rss not routed |
| 917 | RSS ignores current user > muted topics in top RSS | unported | .rss not routed |
| 949 | RSS > exclude_tag in top RSS | unported | .rss not routed |
| 1545 | best_periods_for > applicable periods | needs-harness | calls ListController.best_periods_for directly with relative dates (time travel) |
| 1559 | best_periods_for > default period | needs-harness | class-method unit test, no request |

## ListController#hot

`spec/requests/list_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 924 | RSS ignores current user > muted topics in hot RSS | unported | .rss not routed |
| 959 | RSS > exclude_tag in hot RSS | unported | .rss not routed |

## ListController#category_default / #category_latest / #category_none_* (incl. /l/<filter>, set_category)

`spec/requests/list_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 1021 | category > without access > 404 | covered | golden: GET /c/staff/3.json (anon) and as=user1 |
| 1028 | category > with access > 200 | covered | golden: GET /c/general/4/l/latest.json?per_page=2 |
| 1039 | category > encoded slug > 200 | recordable | setup category with encoded slug + settings slug_generation_method=encoded |
| 1049 | category > parent/child/id > child list 200 | covered | golden: GET /c/general/sub-general/34.json |
| 1056 | category > invalid parent/child slug > redirects | recordable | `/c/random/another/34/l/latest.json` 301 to canonical |
| 1074 | category > other category named with an id prefix | recordable | setup UPDATE categories SET slug='4-name' on a category |
| 1086 | child category > parent and child requested > 200 | covered | golden: GET /c/general/sub-general/34.json |
| 1093 | child category > wrong parent (no id) > 404 | recordable | `/c/not-the-right-slug/sub-general/l/latest.json` |
| 1101 | feed > RSS | unported | .rss not routed |
| 1107 | feed > RSS in subfolder | unported | .rss not routed |
| 1115 | feed > exclude_tag | unported | .rss not routed |
| 1128 | feed > no route-derived category param in self URL | unported | .rss not routed |
| 1153 | default views > top default view, for_period=default_top_period | covered | list_category_default_view_top (new) |
| 1161 | default views > unsupported default_top_period falls back to site default | recordable | setup UPDATE categories SET default_view='top', default_top_period='bogus' + settings top_page_default_timeframe=monthly |
| 1173 | default views > nil default view | covered | golden: GET /c/general/4.json (seed default_view NULL) |
| 1181 | default views > '' default view | recordable | setup UPDATE categories SET default_view='' |
| 1189 | default views > latest default view | recordable | setup UPDATE categories SET default_view='latest' |
| 1204 | default views > unreachable filter falls back to latest (destroy, categories, unread for anon) | recordable | setup default_view='unread', anon GET /c/general/4.json |
| 1210 | default views > logged-in-only view for logged-in users | recordable | same setup as user0 |
| 1216 | default views > honoured with /none | covered | list_category_default_view_top (new) |
| 1222 | canonical tag > category default view | unported | HTML |
| 1228 | canonical tag > category latest view | unported | HTML |
| 1238 | category default view > title | unported | HTML |
| 1244 | category default view > og/twitter description stripped | unported | HTML |
| 1266 | category default view > description escaped once | unported | HTML |
| 1291 | category latest view > title | unported | HTML |
| 1585 | set_category > redirects to updated slug (3 levels) | recordable | settings max_category_nesting=3 needs a 3rd-level category via setup; 2-level form `/c/hello/world/34.json` is simple |
| 1596 | set_category > id-only path redirects, query kept | recordable | `/c/4.json?page=4` 301 |
| 1603 | set_category > correct-case slug redirect | recordable | `/c/General/4.json` 301 |
| 1613 | set_category > no restricted topic titles through permalink fallback | recordable | setup permalink row to topic 4 (staff cat), anon GET /c/old-category/999.json 404 |
| 1627 | set_category > encoded slugs no redirect loop | recordable | setup UPDATE categories SET slug='syst%C3%A8mes' |
| 1637 | set_category > lowercase encoded slugs no loop | recordable | as above, lowercased |
| 1647 | set_category > subfolder redirect | needs-harness | set_subfolder (global setting) |
| 1655 | set_category > subfolder sub-sub redirect | needs-harness | set_subfolder |
| 1679 | set_category > unsafe redirect error renders 404 | needs-harness | stubs redirect_to |
| 1684 | set_category > unsafe redirect does not log | needs-harness | stubs + logger inspection |
| 1691 | set_category > gibberish slug redirects | recordable | `/c/summit'%22()&%25.../4.json` 301 |
| 1709 | shared drafts > not displayed when disabled | recordable | setup shared_drafts row (topic 35 -> cat 2), admin GET /c/general/4.json |
| 1721 | shared drafts > displayed in both categories | unported | Unsupported("shared_drafts_category in topic lists") src/topic_query.rs:346 |
| 1743 | body class for categories | unported | HTML |
| 2198 | #new unified > category new list | recordable | /c/general/4/l/new.json as user0, settings enable_unified_new (seed topic_users) |
| 2205 | #new unified > category new respects subset | recordable | port ignores `subset` |

## ListController user lists (#unread #new #unseen #read #posted #bookmarks)

`spec/requests/list_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 1530 | read > not logged in raises (404 HTML) | covered | golden: GET /read (anon) |
| 1536 | read > logged in returns read list | covered | golden: GET /read.json as=user1 |
| 2157 | #new unified > new topics and new replies | recordable | settings enable_unified_new=true, GET /new.json as user0 |
| 2172 | #new unified > subset=topics | recordable | port drops `subset` (not in ListParams) |
| 2185 | #new unified > subset=replies | recordable | as above |
| 2219 | #new unified > tag new list | unported | Unsupported("tag list filters other than latest, top and hot") src/routes/tags.rs:235 |
| 2226 | #new unified > tag new list subset | unported | same marker |
| 969 | RSS > exclude_tag in user topics RSS | unported | .rss not routed (topics_by feed) |

## CategoriesController#index

`spec/requests/categories_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 10 | crawler view subfolder urls | unported | HTML crawler + subfolder |
| 18 | preloads topic list | unported | HTML data-preloaded |
| 29 | no topic list for crawlers | needs-harness | `.expects(:fetch_topic_list).never` |
| 41 | homepage title | unported | HTML |
| 53 | /category paths redirect to /c | unported | /category/* not routed |
| 59 | /category permalink before redirect | unported | /category/* not routed |
| 67 | legacy category permalink no title leak | unported | /category/* not routed |
| 82 | normal user response | covered | golden: GET /categories.json as=user1 |
| 97 | omits invisible topics with stale featured rows | recordable | setup UPDATE topics SET visible=false on a featured topic, GET /categories.json?include_topics=true |
| 112 | no subcategories without permission | recordable | include_subcategories ignored by port |
| 129 | private subcategory counts excluded for anon | recordable | setup restricted subcategory rows; anon GET /categories.json |
| 162 | subcategory response with permission | covered | categories_subcategory_params (new) |
| 179 | no subcategories without param | covered | golden: GET /categories.json as=user1 |
| 194 | topics for categories, subs and subsubs | recordable | include_subcategories + include_topics, needs 3rd-level category setup |
| 233 | tag filter > categories | recordable | `?tag=howto&include_topics=true` (port drops `tag`) |
| 251 | tag filter > subcategories | recordable | as above with include_subcategories |
| 272 | tag filter > subsubcategories | recordable | needs 3rd-level category setup |
| 316 | categories_and_latest > default bump order | unported | /categories_and_latest not routed (not this action) |
| 324 | categories_and_latest > no sort in more_topics_url | unported | /categories_and_latest not routed |
| 340 | categories_and_latest > created order | unported | /categories_and_latest not routed |
| 348 | categories_and_latest > sort=created in more_topics_url | unported | /categories_and_latest not routed |
| 360 | subcategories_with_featured_topics style | unported | Unsupported("subcategory_list (subcategories_with_featured_topics styles)") src/category_list.rs:104 |
| 379 | no extra queries with more categories | needs-harness | SQL query counting |
| 404 | no N+1 with multiple topics | needs-harness | SQL query counting |
| 446 | uncategorized hidden unless allow_uncategorized_topics | recordable | settings desktop_category_page_style=categories_boxes_with_topics, allow_uncategorized_topics=false |
| 461 | parent_category_id lists subcategories | covered | categories_subcategory_params (new) |
| 479 | page > lazy_load_categories pagination | unported | Unsupported("paginated category lists") src/category_list.rs:122 (also stub_const) |
| 491 | page > many categories pagination | needs-harness | stub_const MAX_UNOPTIMIZED_CATEGORIES |
| 503 | page > no pagination by default | needs-harness | stub_const CATEGORIES_PER_PAGE (page=2 -> empty list is recordable without the stub) |
| 513 | page > nested page param | recordable | `/categories.json?page[foo]=2` 200 |

## SearchController#query

`spec/requests/search_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 51 | overloaded > 409 | needs-harness | global_setting + freeze_time + X-Request-Start |
| 81 | null bytes > 400 | recordable | `/search/query.json?term=hello%00hello` |
| 89 | can search correctly (pg headlines) | unported | Unsupported("use_pg_headlines_for_excerpt") src/search/mod.rs:537 |
| 122 | advanced filter order:views | recordable | `/search/query.json?term=order:views%20fixture` |
| 138 | type filter topic / user | covered | golden: /search/query.json?term=user&type_filter=user, ?term=fixture&type_filter=topic |
| 183 | category-restricted tags > anon sees none | needs-harness | tag search reads tag_search_data (SearchIndexer rows) plus tag groups |
| 190 | category-restricted tags > unauthorized user sees none | needs-harness | as above |
| 199 | category-restricted tags > authorized group member sees both | needs-harness | as above + custom group membership |
| 213 | category-restricted tags > admin sees both | needs-harness | as above |
| 229 | search by topic id ignores min length | unported | Unsupported("search_for_id (topic id and URL lookup)") src/routes/search.rs:311 |
| 245 | search by topic id returns topic | unported | same marker |
| 265 | anonymous search disabled > anon 403 not_logged_in | covered | search_anonymous_disabled (new) |
| 272 | anonymous search disabled > logged-in allowed | recordable | settings allow_anonymous_search=false as user0 |
| 281 | logs the search term (search_log_id) | covered | search_query_logs_term (new) |
| 297 | logs pageview session id | recordable | header Discourse-Pageview-Session-Id |
| 313 | no log when disabled | recordable | settings log_search_queries=false |
| 320 | no log for exclude_topics | recordable | `type_filter=exclude_topics`, compare search_logs rows |
| 327 | empty term with search_for_id no 500 | unported | Unsupported("search_for_id ...") src/routes/search.rs:311 |
| 355 | rate limit anon per user | needs-harness | rate limits + freeze_time |
| 375 | rate limit anon globally | needs-harness | rate limits + freeze_time |
| 395 | rate limit logged in | needs-harness | rate limits |
| 415 | crawler noindex page | unported | HTML crawler |
| 804 | search context > invalid context type 400 | recordable | nested search_context[type]=... is silently ignored by the port (only flat keys hit the Unsupported marker) |
| 816 | search context > missing id 400 | recordable | as above |
| 822 | search context > user context not visible 403 | recordable | as above |
| 834 | search context > user context query | recordable | as above |
| 849 | search context > tag does not exist | recordable | as above |
| 862 | search context > tag context query | recordable | as above |

## SearchController#show

`spec/requests/search_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 63 | overloaded > no results and error | needs-harness | global_setting + freeze_time |
| 428 | anon cannot search > HTML redirect to login with destination | needs-harness | HTML request; harness sends xhr JSON |
| 437 | anon cannot search > JSON rejected 403 | covered | search_anonymous_disabled (new) |
| 445 | no term > 200 | covered | golden: GET /search.json |
| 450 | term shorter than min > 400 | covered | golden: GET /search.json?q=ba |
| 455 | term is a hash > 400 | recordable | `/search.json?q[foo]` (port drops it, 200) |
| 460 | null bytes > 400 | recordable | `/search.json?q=hello%00hello` |
| 467 | page string number ok | covered | golden: GET /search.json?q=Parity&page=1 |
| 472 | page integer ok | covered | golden: GET /search.json?q=Parity&page=1 |
| 477 | invalid page > 400 | covered | golden: GET /search.json?q=fixture&page=%203 (same parse-failure branch) |
| 482 | page padded with spaces > 400 | covered | golden: GET /search.json?q=fixture&page=%203 |
| 487 | page above limit > 400 | covered | golden: GET /search.json?q=fixture&page=11 |
| 492 | logs the search term | recordable | `/search.json?q=fixture`, search_logs row |
| 499 | no log when disabled | recordable | settings log_search_queries=false |
| 506 | tag context | unported | Unsupported("search contexts (user, topic, category, tag)") src/routes/search.rs:213 |
| 535 | restricted tag context > permitted user | unported | same marker |
| 548 | restricted tag context > 403 without permission | unported | same marker |
| 579 | rate limit anon per user | needs-harness | rate limits + freeze_time |
| 597 | rate limit anon globally | needs-harness | rate limits + freeze_time |
| 617 | rate limit logged in | needs-harness | rate limits |
| 657 | lazy loaded categories > extra categories | recordable | settings lazy_load_categories_groups=4, anon `/search.json?q=fixture` |
| 682 | content localization > translated blurb | unported | Unsupported("content_localization_enabled") src/search/mod.rs:556 |
| 698 | content localization > original blurb via cookie | unported | same marker |
| 769 | search priority > empty term status:open | recordable | setup UPDATE categories SET search_priority, `/search.json?q=status:open` |
| 779 | search priority > no order query | recordable | settings category_search_priority_*_weight + setup |
| 792 | search priority > order query ignores priority | recordable | `q=status:open order:latest fixture` |

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
| 673 | special tag none | covered | parity/writes/tags |
| 682 | numeric tag names via /tag/:tag_name | unported | Unsupported("/tag/:tag_id without .json") src/routes/tags.rs:70 (HTML route) |
| 691 | numeric tag names with filters | covered | parity/writes/tags |
| 700 | missing numeric /tag/:tag_id 404 | covered | golden: GET /tag/99.json |
| 705 | numeric-named slug route redirect | covered | parity/writes/tags |
| 718 | edit path redirect | unported | /tag/:slug/:id/edit not served (unknown /tag/ route marker src/routes/tags.rs:98) |
| 725 | edit tab redirect | unported | same marker |
| 732 | edit page no redirect | unported | same marker |
| 739 | id-only edit redirect | unported | same marker |
| 753 | in category > no restricted name leak via permalink | covered | parity/writes/tags |
| 764 | in category > topic inside category | covered | golden: GET /tags/c/general/4/howto.json |
| 773 | in category > next topic URL | covered | golden: GET /tags/c/general/4/none/howto/1.json?per_page=1 |
| 781 | in category > invalid category path 404 | covered | parity/writes/tags |
| 787 | in category > restricted category 404 | covered | tags_in_staff_category_as_admin (new) |
| 802 | in subcategory > topic inside subcategory | covered | parity/writes/tags |
| 812 | invalid tag parameter ignored | covered | parity/writes/tags |

## SiteController#site

`spec/requests/site_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 66 | anonymous_default_navigation_menu_tags only visible tags | unported | Unsupported("anonymous_default_navigation_menu_tags (SidebarTagSerializer)") src/site.rs:1015 |

## SiteController#basic_info

`spec/requests/site_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 80 | visible with login_required, title/description/logos/discover | covered | login_required_lists_anonymous (new, matches) |
| 109 | false values for discover and login_required | covered | golden: GET /site/basic-info.json |

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
| 53 | page 404 if sitemap missing | covered | golden: GET /sitemap_9.xml |
| 59 | page includes topics | covered | golden: GET /sitemap_1.xml |
| 79 | recent: topics bumped in last 3 days | covered | golden: GET /sitemap_recent.xml |
| 96 | recent: page numbers from posts_count | recordable | setup UPDATE topics SET bumped_at=now(), posts_count=21 WHERE id=35 |
| 124 | news: topics bumped in last 72h | covered | golden: GET /news.xml |

## UserAvatarsController#show_proxy_letter

`spec/requests/user_avatars_controller_spec.rb`

| Line | Scenario | Class | Case / note |
|---|---|---|---|
| 5 | 404 when external avatar is elsewhere | recordable | settings external_system_avatars_url=https://somewhere.else.com/avatar.png, GET /letter_avatar_proxy/v4/letter/a/aaaaaa/20.png (port answers an empty 404; Rails xhr gives the JSON not_found body) |
| 11 | returns avatar when proxy allowed | needs-harness | stubs disable_proxy? + stub_request to the avatar CDN |

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

