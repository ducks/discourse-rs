# Bundled plugin parity audit

ROADMAP 210, 2026-10-17, on main at 33e06c4. Every bundled plugin the
port has started (`crate::plugins::STARTED`, src/plugins/mod.rs:285)
compared with the reference plugin, area by area: plugin.rb (settings,
serializer fields, events, modifiers, custom fields, topic list and
query additions, user and post actions, site and current user
additions), config/routes.rb and every controller action, serializers,
jobs, models and migrations, site settings and their change handlers,
locales, the Ember UI (components, plugin outlet connectors, composer
and toolbar buttons, admin pages, user preferences), markdown and
cooking, emails and notifications.

The rule: a plugin is done only when it has every feature of the
reference plugin. Its markdown, some routes or most of its pages make it
started. By that rule none of the sixteen is done.

Rails paths are relative to the Discourse source root (`plugins/...` for
the plugin, `lib/...` and `app/...` for core). Port paths are relative
to the repository root at the commit above. `[item NNN]` marks a gap an
earlier ROADMAP item already covers; the gaps without one are ROADMAP
211-249 (Phase 11), listed under each plugin's Items. This is a source
audit: nothing was run against the reference, so behavior only a
running Rails settles is under each plugin's "Couldn't determine".

## Summary

| Plugin | Verdict | Gaps | Biggest gaps | New items | Rough size to done |
|---|---|---|---|---|---|
| chat | started | 78 server, 63 UI (overlapping where a UI action needs an endpoint) | threads (160 done, its rest listed there), uploads (161), channel creation, settings, deletion and archiving, flags and review, pins, message search, incoming webhooks, the drawer, scroll paging | 215-227, 212 | several L: 160-162 plus 2 L and 9 M and S new |
| poll | started | 27 | no client at all: voting, results, pie and ranked choice views, breakdown; the builder; emails | 228-230, 211 | L plus 2 M |
| discourse-reactions | started | 23 | deleted users' reactions kept, the regular like sync, logged-out reacting, op_reactions_data on lists, picker and paging UI | 231-232, 211, 212 | 2 M |
| discourse-solved | started | 30 | unaccepting on post deletion, revisions and moves, auto bump, category custom fields on site and categories JSON, composer education message | 233-235, 213, 212 | 2 M, 2 S plus 163 |
| discourse-topic-voting | started | 16 | voting after login, the docked header vote box, Most Votes search order | 236-237, 235, 211, 212 | M plus S, with 197 and 195 |
| discourse-captcha | started | 5 | the login code signup gate and its code-login connector (core's code signup unported), widget placement without user fields, non-English strings (167), CSP (214) | none | M with core's code signup |
| discourse-cakeday | started | 5 | onceoffs never enqueued (core has no EnqueueOnceoffs), non-English strings (167), "today" read from the user's timezone not the browser's | none | S |
| discourse-post-voting | started | 11 | flagging comments (plugin reviewables in the queue), the composer's post voting toggle and category defaults, voting after login | none yet | M (reviewables, shared with chat) plus S |
| discourse-presence | started | 3 | the translate composer state, whisper state | 238 | S |
| discourse-local-dates | started | 5 | refused date forms Rails cooks | 239, 244 | S |
| discourse-lazy-videos | started | 5 | no client player, no email links, no crawler JSON-LD | 240-241 | 2 S |
| checklist | started | 6 | checking a box (PUT /checklist/toggle and the client) | 242 | M |
| discourse-details | started | 2 | email handler not gated on details_enabled | 244 | S |
| footnote | started | 1 | the rich editor (196); per-post ids done (243) | 243 | 196 |
| spoiler-alert | started | 2 | emails show spoilers in the clear | 244 | S |
| discourse-lti | started | 3 | cannot be enabled through the port (164), CSP nonce | 214 | 164 plus M |
| discourse-oauth2-basic | started | 13 | cannot be enabled (164), debug logging, timeout and authorize options, redirects, admin tab, signup buttons | 245-247 | 164 plus 3 S |
| discourse-openid-connect | started | 10 | cannot be enabled (164), mTLS, verbose logging, admin tab | 248-249, 245-246 | 164 plus M and S |
| discourse-login-with-amazon | started | 2 | cannot be enabled through the admin (164), non-English strings (167) | none | 164 |
| discourse-apple-auth | started | 2 | cannot be enabled (164), aria-label | 247 | 164 plus S |
| discourse-microsoft-auth | started | 2 | cannot be enabled through the admin (164), non-English strings (167) | none | 164 |
| discourse-policy | started | 7 | the rich editor node (196), the wiki toggle check (no wiki route), select-kit group chooser in the builder | none | 196 plus S |
| discourse-affiliate | started | 2 | localized posts (no content localization), the enable-if-installed migration | none | S, with content localization |
| discourse-gamification | started | 8 | the workflows node (discourse-workflows unported), the seed fixture, plugin columns on the directory page, the minimal leaderboard, native choosers in the edit form | none | S per gap; the directory page with core |
| discourse-graphviz | started | 3 | the rich editor node and its preview (196), DAsyncContent's loading state, the menu item's place | none | 196 plus S |
| discourse-narrative-bot | started | 8 | the flag step (core refuses flags in messages), the recover step (core refuses authors recovering their own posts), locales other than English and French, image URLs counted only once oneboxed | none | 2 core M (flags in messages, user_recovered) plus S |
| discourse-templates | started | 8 | the chat thread composer's variables (the composer exists since 160), the chat dropdown's core items, the keyboard shortcuts help entry, the canned-replies rake tasks | none | S |
| styleguide | started | 7 | 43 of 50 sections showcase Ember components the port does not have; the sidebar panel, the color mode toggle, code samples | none | L (follows the port's own components) |
| discourse-math | started | 5 | the rich editor node and edit modal (196) | none | S plus 196 |

Findings that cross plugins:

- Core pieces several plugins wait on, which no item named: anonymous
  actions (poll, reactions, topic voting; 211), an MCP server (chat,
  reactions, solved, topic voting; 212), composer messages (solved; 213)
  and a content security policy (LTI; 214).
- Rails runs a plugin's event handlers only while the plugin is enabled
  (lib/plugin/instance.rb:692). The port's details email handler, the
  spoiler excerpt removal and local dates' excerpt suffix run regardless
  (244).
- Item 164 (setting types) blocks the four auth plugins outright: each
  enable setting carries AuthProviderCredentialsValidator, which
  src/site_setting_update.rs:181 refuses, so an admin cannot turn them on
  through the port.
- Stale ROADMAP text corrected in the same commit: 163 and milestone 5
  listed solved's web hooks, auto close timers and search filters and
  voting's web hooks, search filters and category edits as left (done in
  143d, 143e, 190c-190f, src/web_hooks/mod.rs:469,504); milestone 5 said
  reactions in messages are refused (no refusal is left). The module
  comments saying the same (src/plugins/solved/answers.rs:8,
  src/plugins/topic_voting/votes.rs:5, src/plugins/reactions/toggle.rs:11)
  are left for 233, 237 and 231, as this branch changes docs only.

## Chat, server (`plugins/chat`)

Verdict: started (78 gaps)

Scope: server side only (plugin.rb, routes and every controller action, services, serializers, jobs, models, settings and handlers, server locales, emails, notifications, cooking, bookmarks, search, hashtags, web hooks, reviewables, user deletion, MCP/API). The Ember UI is audited separately; markdown-it rules are included here.

### What the port has
- Guard of every API action (requires_plugin, ensure_logged_in, ensure_can_chat): src/routes/chat.rs:20.
- Routes ported (src/routes/mod.rs:284-380, 2492): POST /chat/:id (create message), GET /chat/api/channels, /me/channels, /chatables, GET and PUT /channels/:id, memberships index/create/destroy, memberships/me POST/PUT/DELETE and /me/follows DELETE, PUT /channels/read and /channels/:id/read, POST drafts, GET messages, PUT and DELETE messages/:id, PUT messages/:id/restore, POST direct-message-channels, GET /chat/direct_messages, PUT /chat/:id/react/:message_id, PUT /chat/:id/:message_id/rebake, POST /chat/dismiss-retention-reminder; full pages /chat, /chat/channels, /chat/disabled, /chat/browse/*, /chat/c/:slug/:id(/:message_id), the /chat/channel/... redirects (src/routes/chat_pages.rs); the pane's pages of messages for chat.js, GET /live/chat/:channel_id/messages (plugins/chat/page.rs messages_page).
- Services: CreateMessage with ProcessMessage, mentions (direct, group, here, all) and Notifier (src/plugins/chat/create.rs, mentions.rs, notify.rs), UpdateMessage, TrashMessage, RestoreMessage, RebakeMessage (modify.rs), UpsertDraft, memberships, follows, read tracking (membership.rs), CreateDirectMessageChannel and DM group edits (dm_create.rs, dm_groups.rs), UpdateChannel with its slug (dm_groups.rs), SearchChatable (chatables.rs), channel listing and fetcher (channels.rs, view.rs), MessageRateLimiter with auto silence (routes/chat.rs:539-603), Chat::Publisher (publisher.rs), auto join and leave (auto_join.rs, scheduled.rs).
- Jobs: ProcessMessage, NotifyMentioned, NotifyWatching, UpdateThreadReplyCount, UpdateChannelUserCount, DeleteUserMessages, KickUsersFromChannel, scheduled EmailNotifications, PeriodicalUpdates, AutoJoinUsers, DeleteOldMessages (src/jobs/handlers.rs:28-141; src/jobs/mod.rs:231-299,587-599).
- plugin.rb: current_user keys (can_chat, can_direct_message, has_chat_enabled, chat_sound, retention reminders, has_joinable_public_channels, chat_drafts) and last_chat_channel_id custom field (src/plugins/chat/mod.rs:409-479; src/current_user.rs:446-476), user_option and current_user_option keys (mod.rs:~330-405; src/user_private.rs:919), UserUpdater option attrs (src/user_updater.rs:215-249), can_chat_user on user card and hidden profile (mod.rs:483; src/users.rs:500,672), category has_chat_enabled / has_chat_channels (src/category_serializer.rs:37,192), retention site_setting_changed handler (src/plugins/mod.rs:310-337), user_added_to_group / user_removed_from_group / group_destroyed / category_updated auto join and leave (src/plugins/mod.rs:342-380; category_writes), user deletion callbacks (src/plugins/mod.rs:237,272), upload_in_use (mod.rs:575), search index of messages (src/posting/search_index.rs:624-660; src/jobs/search_jobs.rs:153), outgoing chat_message_* web hooks (src/plugins/chat/web_hooks.rs), about stats chat_messages/chat_users/chat_channels (src/about.rs:167-187), presence prefixes chat and chat-reply (src/plugins/chat/presence.rs), hashtag channel data source (src/hashtags.rs:82,334,655), email dark-mode styles (src/email/styles.rs:374), flag applies_to Chat::Message (src/admin_flags.rs:20), reviewable sti name (src/reviewables.rs:20), notification types (src/notifications.rs:44-55).
- Emails: chat summary (src/plugins/chat/summary_email.rs; src/jobs/user_email.rs:88).
- Cooking: chat engine with MARKDOWN_IT_RULES and features, chat-html-inline (crates/markdown/src/render/chat_html.rs; mod.rs:190,282), slash commands /me /shrug /tableflip (src/plugins/chat/cook.rs), parity corpus parity/chat_cook.

### Gaps

**Routes and controllers**
- POST /chat/api/channels (create a category channel, Chat::CreateCategoryChannel, auto_join limiter). Rails: plugins/chat/config/routes.rb:9; plugins/chat/app/controllers/chat/api/channels_controller.rb:63. Port: absent (src/routes/mod.rs:308 GET only).
- DELETE /chat/api/channels/:channel_id (Chat::TrashChannel, Jobs::Chat::ChannelDelete). Rails: routes.rb:16; channels_controller.rb:51. Port: absent (src/routes/mod.rs:321 GET/PUT only).
- PUT /chat/api/channels/:channel_id/status (open, closed, read_only, archived; Chat::UpdateChannelStatus). Rails: routes.rb:19; app/services/chat/update_channel_status.rb. Port: absent.
- POST /chat/api/channels/:channel_id/archives (ChannelArchiveService to a new or existing topic, Jobs::Chat::ChannelArchive, retry). Rails: routes.rb:27; lib/chat/channel_archive_service.rb:16. Port: absent.
- POST /chat/api/channels/:channel_id/invites (InviteUsersToChannel, chat_invitation notifications). Rails: routes.rb:26; app/services/chat/invite_users_to_channel.rb. Port: absent.
- PUT /chat/api/channels/:channel_id/notifications-settings/me (muted, notification_level). Rails: routes.rb:36; channels_current_user_notifications_settings_controller.rb:6. Port: absent.
- POST /chat/api/channels/:channel_id/messages/:message_id/flags (Chat::FlagMessage, ReviewQueue). Rails: routes.rb:12; app/services/chat/flag_message.rb; lib/chat/review_queue.rb:12. Port: absent.
- POST /chat/api/channels/:channel_id/messages/:message_id/interactions (message block buttons, Chat::CreateMessageInteraction, :chat_message_interaction event). Rails: routes.rb:14; app/services/chat/create_message_interaction.rb. Port: absent.
- POST /chat/api/channels/:channel_id/messages/moves (Chat::MessageMover). Rails: routes.rb:23; lib/chat/message_mover.rb:30. Port: absent.
- DELETE /chat/api/channels/:channel_id/messages/:message_id/streaming (Chat::StopMessageStreaming). Rails: routes.rb:24. Port: absent.
- DELETE /chat/api/channels/:channel_id/messages (bulk_destroy, Chat::TrashMessages). Rails: routes.rb:67; channel_messages_controller.rb:36. Port: absent.
- Pins: GET /channels/:id/pins, PUT /pins/read, POST and DELETE /messages/:id/pin (ListChannelPins, PinMessage, UnpinMessage, MarkPinsAsRead). Rails: routes.rb:39-42; app/controllers/chat/api/channel_pins_controller.rb. Port: absent (the chat_pinned_messages setting and pin counts are read, src/plugins/chat/channels.rs:720, but nothing pins).
- GET /chat/api/search (Chat::SearchMessage over chat_message_search_data). Rails: routes.rb:21; app/services/chat/search_message.rb. Port: absent (the index is written, src/posting/search_index.rs:631, never read).
- GET /chat/api/category-chatables/:id/permissions (staff). Rails: routes.rb:45; category_chatables_controller.rb:6. Port: absent.
- GET /chat/api/mentions/groups (HintsController#check_group_mentions). Rails: routes.rb:50; hints_controller.rb:8. Port: absent.
- GET /chat/:chat_channel_id/:message_id/reactions-users (MessageReactionUsersQuery). Rails: routes.rb:111; chat_controller.rb:25. Port: absent.
- POST /chat/:chat_channel_id/quote (TranscriptService). Rails: routes.rb:113; chat_controller.rb:89; lib/chat/transcript_service.rb:18. Port: absent.
- PUT /chat/user_chat_enabled/:user_id (set_user_chat_status). Rails: routes.rb:114; chat_controller.rb:63. Port: absent.
- POST /chat/admin/export/messages (ExportCsvFile entity chat_message via Chat::MessagesExporter, log_entity_export). Rails: routes.rb:71; app/controllers/chat/admin/export_controller.rb:8; lib/chat/messages_exporter.rb. Port: absent. [item 200]
- Incoming webhooks: POST /chat/hooks/:key and /chat/hooks/:key/slack (SlackCompatibility, rate limit, chat_debug_webhook_payloads). Rails: routes.rb:78,87; app/controllers/chat/incoming_webhooks_controller.rb:15,25,90,118; lib/chat/slack_compatibility.rb:19. Port: absent (incoming_chat_webhooks is never read).
- Admin incoming webhooks: GET/POST /admin/plugins/chat/hooks, GET new, GET :id/edit, PUT and DELETE :id. Rails: plugin.rb:547-562; app/controllers/chat/admin/incoming_webhooks_controller.rb:8-74. Port: absent; /admin/plugins/chat itself refuses (src/routes/admin_plugins.rs:110 "a plugin's own admin pages").
- ~~Thread routes~~ (done, 160): GET /me/threads, the 8 /threads routes, mark-thread-title-prompt-seen and thread drafts (src/plugins/chat/threads.rs; src/routes/chat.rs).
- Respond pages not routed: /chat/new-message, /chat/c/:title/:id/info, info/about, info/members, info/settings, info/search (these 404 today: "info" fails the message_id parse, src/routes/chat_pages.rs:410-412), and the /chat/channel/:channel_id(/:channel_title(/info...)) redirects to /chat/c. Rails: routes.rb:99,121-138. Port: absent. (/chat/search, /chat/starred-channels: [item 162]; /chat/direct-messages, /chat/threads, /chat/c/.../t/...: [item 160])

**plugin.rb**
- Chat::ReviewableMessage behavior: actions (agree_and_keep_message, agree_and_delete, agree_and_restore, agree_and_suspend/silence, disagree, ignore), auto silence from flags (chat_auto_silence_from_flags_duration, chat_silence_user_sensitivity), review queue listing of chat reviewables. Rails: plugin.rb:125; app/models/chat/reviewable_message.rb:4; lib/chat/review_queue.rb:12. Port: only the sti name (src/reviewables.rs:20); the queue refuses other types (src/review_list.rs:570). [item 148 for the queue's other types; chat's actions are not covered]
- :post_alerter_after_save_post, Chat::PostNotificationHandler: chat_quoted notifications for users whose chat messages a post quotes with [chat quote=...]. Rails: plugin.rb:436-439; lib/chat/post_notification_handler.rb:16-37. Port: absent (chat_quoted is only a type id and a menu icon).
- register_post_stripper removing `.chat-transcript .mention`. Rails: plugin.rb:630-632. Port: absent; transcripts cook (crates/markdown/src/render/chat_transcript.rs) but their mentions stay in stripped text.
- Site.markdown_additional_options["chat"] (limited_pretty_text_features, rules, hashtag_configurations). Rails: plugin.rb:634-638. Port: src/site.rs:452-453 emits `{}`; parity ignores it (parity/cases:12-15).
- Hashtag contexts on /site.json: chat-composer (channel 200, category 100, tag 50) and channel in topic-composer (10), and hashtag_icons.channel. Rails: plugin.rb:623-628 (register_hashtag_type_priority_for_context, data source icon). Port: src/site.rs:1022-1037 core types only (hashtag cooking does use the chat order, src/pretty_text/mod.rs:291).
- register_hashtag_content_store(Chat::MessageHashtagStore): remapping #category/#tag refs in chat messages on rename. Rails: plugin.rb:624; lib/chat/message_hashtag_store.rb. Port: absent; the remap_hashtag job itself has no handler (enqueued at src/category_writes.rs:2461, src/tag_writes.rs:274, fails in src/jobs/handlers.rs' fallback).
- user_notification_total :chat_notifications (Chat::ChannelFetcher.unreads_total). Rails: plugin.rb:296-304. Port: absent; core GET /notifications/totals isn't routed either (src/routes/mod.rs:240-267).
- site_setting_changed: secure_uploads turned on runs Chat::SecureUploadsCompatibility.update_settings; UploadSecurity.register_custom_public_type("chat-composer"). Rails: plugin.rb:427-429,652; lib/chat/secure_uploads_compatibility.rb:4. Port: absent (src/plugins/mod.rs:310-337 handles retention only).
- site_setting_changed: chat_allowed_groups enqueues Jobs::Chat::AutoJoinUsers. Rails: plugin.rb:431-433. Port: absent, the list setting update is refused (src/plugins/mod.rs:306-309). [item 164]
- register_email_unsubscriber("chat_summary", ChatSummaryUnsubscriber): the unsubscribe page's chat email frequency select and its save. Rails: plugin.rb:609; lib/email_controller_helper/chat_summary_unsubscriber.rb:5-30. Port: src/email/unsubscribe.rs:108 (refusal "unsubscribe keys of plugins' email types"), the outlet at :526 shows nothing.
- register_push_notification_filter (chat_only push level). Rails: plugin.rb:117-123. Port: absent. [item 186]
- Oneboxer and InlineOneboxer local handlers for chat channels and messages. Rails: plugin.rb:184-190; lib/chat/onebox_handler.rb, inline_onebox_handler.rb. Port: src/onebox/mod.rs:250 (refusal "oneboxes of chat channels and messages"). [item 142]
- register_bookmarkable(Chat::MessageBookmarkable). Rails: plugin.rb:648. Port: src/bookmark_manager.rs:175; src/bookmarks.rs:266,743. [item 161]
- ~~register_notification_consolidation_plan(watched_thread_message_plan)~~ (done, 160: src/plugins/chat/notify.rs consolidate_watched_thread).
- Category deletion with a chat channel (cannot_delete_reason has_chat_channels, deletable_for_chat?, dependent destroy). Rails: lib/chat/category_extension.rb:8-27. Port: src/category_writes.rs:2876 (refusal "destroying a category's chat channel").
- register_mcp_tool chat_channel_list, discourse_get_chat_messages, chat_message_create. Rails: plugin.rb:660-753; lib/chat/mcp_tools.rb. Port: absent (no core MCP server either).
- add_api_key_scope :chat create_message (chat/api/channel_messages#create, path param chat_channel_id). Rails: plugin.rb:593-601. Port: absent from src/api_key_scopes.rs.
- add_automation_scriptable send_chat_message. Rails: plugin.rb:565-591. Port: absent. [item 202]
- discourse-workflows chat approval node and :chat_message_interaction resume, Jobs::Chat::ResumeWorkflowApproval(Interaction). Rails: plugin.rb:127-164; app/jobs/regular/chat/resume_workflow_approval*.rb. Port: absent. [item 202]
- Chat::Seeder (needs_chat_seeded: staff and general category channels on a fresh site). Rails: lib/chat/seeder.rb:5; db/fixtures/600_chat_channels.rb:3. Port: absent (no db:seed).
- allow_chat_in_anonymous_mode (anonymous mode chats as the master account). Rails: lib/chat/guardian_extensions.rb; app/validators/chat/allow_chat_in_anonymous_mode_validator.rb. Port: src/plugins/chat/mod.rs:83 (refusal "chat in anonymous mode"); src/plugins/chat/dm_groups.rs:438. [item 133]
- register_stat chat_channel_messages, chat_direct_messages, chat_open_channels_with_threads_enabled, chat_threaded_messages (not exposed via api). Rails: plugin.rb:615-620; lib/chat/statistics.rb. Port: absent (only the three exposed stats, src/about.rs:167-187).

**Services and model behavior**
- Chat::DuplicateMessageValidator exemptions: bots, DMs with two or fewer users and upload-only messages skip the check. Rails: lib/chat/duplicate_message_validator.rb:14-23. Port: upload-only messages skip it (161); src/plugins/chat/create.rs and modify.rs check bots and small DMs too.
- Channel edits that fail validation (record_invalid, e.g. name too long) and DM names that fail. Rails: app/services/chat/update_channel.rb:58; create_direct_message_channel.rb. Port: src/plugins/chat/dm_groups.rs:498; src/plugins/chat/dm_create.rs:285 (refusals "... (record_invalid)").
- Slug changes: ported (src/plugins/chat/dm_groups.rs, channel_slug); an invalid or taken slug is the record_invalid refusal. [item 162]
- Editing a message of a deleted channel. Rails: app/services/chat/update_message.rb. Port: src/plugins/chat/modify.rs:107 (refusal "messages of deleted chat channels").
- Edits too large to diff for the grace period. Rails: app/services/chat/update_message.rb (edit grace period). Port: src/plugins/chat/modify.rs:431 (refusal "chat edits too large to diff").
- target_date formats other than ISO 8601 (Rails DateTime.parse). Rails: app/services/chat/list_channel_messages.rb:30. Port: src/plugins/chat/messages.rs:1027.
- Deleted or gone users in DMs and titles: src/plugins/chat/direct_messages.rs:132,393; src/plugins/chat/notify.rs:409; src/plugins/chat/create.rs:568; src/routes/chat_pages.rs:388 (refusals "a direct message title without a viewer", "...participants are all deleted", "...user that is gone"). Rails: app/models/chat/direct_message.rb (chatable_url, title). Port: refusals as listed. [item 162 covers deleted users' messages; these lines are not listed there]
- Direct message channels without their direct_message row, multi-valued user custom fields in DM user serialization. Port: src/plugins/chat/direct_messages.rs:63,319.
- The channels list without a membership or without user options (src/plugins/chat/view.rs:319,322); a chat page for a message it can't load (src/plugins/chat/page.rs:126); reactions whose emoji has no image (src/plugins/chat/page.rs:1478); a web hook for a message gone before it fires (src/plugins/chat/web_hooks.rs:37); a site without its system user (src/plugins/chat/dm_groups.rs:323). Rails answers each (empty list, redirect, fallback). Port: refusals as listed.
- Messages of deleted users (processing, editing, the summary line, reactions, replies; the notification jobs are ported), anonymous access, the separate sidebar, filtered list empty states. Webhook messages, blocks, bot changes, archive reads, slug changes, the mobile starred route and the sound preview are ported. [item 162]
- Oneboxes, hotlinked images, lightboxes and video placeholders in messages (the rest of Chat::MessageProcessor), Jobs::Chat::PullHotlinkedImages. Rails: lib/chat/message_processor.rb; app/jobs/regular/chat/pull_hotlinked_images.rb. Port: src/plugins/chat/create.rs:816 (refusal). Message uploads are ported (create.rs, modify.rs, messages.rs, collapser.rs). [item 161]
- ~~Threads everywhere~~ (done, 160): creation, memberships, tracking, publisher targets, notifications, the summary email.
- User status in chat serializers. [item 130]

**Jobs**
- ~~Jobs::Chat::MarkAllChannelThreadsRead~~ (done, 160: src/plugins/chat/threads.rs).
- Jobs::Chat::ChannelArchive. Rails: app/jobs/regular/chat/channel_archive.rb. Port: absent.
- Jobs::Chat::ChannelDelete. Rails: app/jobs/regular/chat/channel_delete.rb. Port: absent.
- Jobs::Chat::ResumeWorkflowApproval, ResumeWorkflowApprovalInteraction. Port: absent. [item 202]
- Jobs::Chat::PullHotlinkedImages. Port: absent. [item 161]

**Models**
- incoming_chat_webhooks (with chat_webhook_events writes for incoming hooks): unused. Rails: app/models/chat/incoming_webhook.rb. Port: absent.
- chat_channel_archives written by archiving: only read (the channel serializer's archive status, src/plugins/chat/channels.rs).
- chat_message_interactions: only deleted by the destroyer (src/plugins/chat/scheduled.rs).
- chat_channel_custom_fields, chat_message_custom_fields, chat_thread_custom_fields (HasCustomFields on the models): unused. Rails: app/models/chat/channel.rb, message.rb, thread.rb.

**Settings**
- Settings with no reader in the port: chat_auto_silence_from_flags_duration, chat_silence_user_sensitivity (flags), chat_archive_destination_topic_status (archiving), chat_debug_webhook_payloads (incoming hooks), needs_chat_seeded (seeder), allow_chat_in_anonymous_mode ([item 133]); chat_show_copy_button_on_codeblocks and enable_new_chat_reactions_popup are client-only (UI audit). Rails: plugins/chat/config/settings.yml.
- Change handlers for chat_allowed_groups [item 164] and secure_uploads (above).

**Locales**
- Server strings load from the vendored plugin_server.en.yml (src/i18n.rs:27); other locales refused. [item 167]

**Markdown**
- [chat quote=...] transcripts in posts (chat-transcript.js) refused. Rails: plugins/chat/assets/javascripts/lib/discourse-markdown/chat-transcript.js. Port: crates/markdown/src/render/bbcode.rs:268. [item 161]
- Oneboxes in cooked chat messages (Oneboxer cache). Port: src/plugins/chat/create.rs:747; src/plugins/chat/cook.rs header. [item 161]

**Emails and notifications**
- chat_invitation notifications (invites route above). Rails: app/services/chat/invite_users_to_channel.rb. Port: absent.
- chat_quoted notifications (post handler above).
- ~~chat_watched_thread notifications, threads in the summary email~~ (done, 160).
- Push notifications for chat mentions and messages (refused at src/plugins/chat/notify.rs:283,448 docs; jobs/post_alert.rs). [item 186]
- chat_summary unsubscribe strategy (above).

**Parity**
- parity/cases ignores every chat key on /session/current.json, /u/:username.json and /site.json (parity/cases:12-15,97-116,163-166,247-249,321-327): the port emits most of them (src/plugins/chat/mod.rs:409-479) but none is compared, and markdown_additional_options / hashtag_configurations / hashtag_icons are wrong (above).

### Items

New (ROADMAP Phase 11): 215 `feat/chat-channel-settings` M, 216 `feat/chat-channel-create` M, 217 `feat/chat-moderation` L, 218 `feat/chat-message-actions` M, 219 `feat/chat-pins` S, 220 `feat/chat-search` S, 221 `feat/chat-admin` M, 226 `feat/chat-site-integration` M, 227 `feat/chat-pages-rest` S, and chat's three MCP tools in 212 `feat/mcp-server` L. Each chat item carries the server and UI halves of its feature together. Earlier items: 160 (threads, MarkAllChannelThreadsRead), 161 (uploads, oneboxes, bookmarks, transcripts' cooking), 162 (deleted users, webhook messages, bots, archived and other chatables, anonymous access, slugs, the separate sidebar, /chat/search and starred), 130, 133, 142, 148, 164, 167, 186, 200, 202.

### Couldn't determine
- Whether ChannelsController#show, ChannelMessagesController#index and the DM/chatables actions match Rails in every key and edge without running Rails; only the params were compared here (they match the service contracts, less context_topic_id/context_post_ids, which only feed the :chat_message_created event for other plugins).
- How the port's user merge handles chat rows (Rails relies on UserDestroyer's dependent destroys after the merge); not traced.
- Where Rails reads the four unexposed register_stat values (Stat.all_stats has no core caller); possibly only discourse-hub telemetry.
- Whether the guardian extensions the port inlines (can_rebake, can_restore_other_chats, can_delete_own/other_chats, can_see_chat_message) match Rails in every branch; names differ, only the ported actions use them.

## Chat, UI (`plugins/chat`)

Scope: the client side only (assets/javascripts/** and admin/assets/javascripts/**). Server internals are under Chat, server; endpoints are named here only where a UI action needs one the port doesn't route (src/routes/mod.rs:284-380). Rails paths below are relative to the source root, `plugins/chat/assets/javascripts/discourse/` abbreviated as `c/`.

Verdict: started (63 gaps)

### What the port has
- Full-page routes: /chat and /chat/channels (ChatIndexRoute/ChatChannelsRoute redirects, mobile channels list), /chat/disabled, /chat/browse and /chat/browse/:tab with the name filter, the joined dropdown and htmx paging, /chat/c/:slug/:id and /:message_id with slug fix-up and the DM title (src/routes/chat_pages.rs:184-500; src/routes/mod.rs:286-295)
- The channel pane: navbar (title, star, pins link, search and drawer buttons drawn), channel status, retention reminder, messages with date separators, last-visit line, collapsed deleted runs and expand, reply-to line, edited label, reactions with screen reader text, bookmark and pin marks in the info line, empty states, the preview card for non-members, the "all loaded" line (src/plugins/chat/page.rs:98-1590)
- Composer: placeholder rules, draft load and save (2s), send shortcut (Enter, Ctrl/Cmd+Enter), edit last message with ArrowUp, Escape, edit and reply details bar, emptied edit deletes, "+:emoji:" reacts to the last message, staged messages with the send error and retry, max length alert, @/:/# autocompletes (static/js/chat.js:399-975, 2056-2580; page.rs:711-856)
- Message actions on desktop: quick reactions (custom, frequent, defaults with skin tone), react picker, reply, copy link with toast, edit, delete, restore, rebake (src/plugins/chat/actions_view.rs; chat.js:980-1565)
- Live updates over the bus: sent, processed, edit, refresh, restore, reaction, delete, bulk_delete (chat.js:1566-1876; GET /live/chat/:channel_id/:message_id, chat_pages.rs:470); mark read on scroll (chat.js:356-398); /chat/online presence and avatars; /chat-reply typing presence and the replying indicator (chat.js:1878-2055)
- Join, leave, star from the browse cards and navbar; DM auto-follow on open (chat.js:316-354, 2583-2600)
- Chrome: the header icon with its unread indicator (header indicator preference), the sidebar sections (search link, starred, channels, DMs with the 50 limit and "start a new DM" link) in "never" mode, the mobile chat header and footer, the mobile back button indicator (src/plugins/chat/view.rs:245-1047; live updates in src/routes/live.rs:146-166)
- User menu chat tab and the chat notification renderers (src/user_menu_view.rs:184-195, 1043+); About page chat_messages activity (src/about_view.rs:343); preferences chat tab (src/preferences_view/security.rs:859), chat email frequency, chat notifications (chat_sound, header indicator, ignore_channel_wide_mention, src/preferences_view/notifications.rs); user card and profile Chat buttons drawn (src/user_card_view.rs:337; src/user_profile_view.rs:484)
- Channel hashtags in the topic composer (static/js/hashtags.js:76,102; composer.js:741); local dates and highlight in chat messages (chat.js:170-188)
- Stylesheets: common, desktop and mobile bundles vendored (static/css/chat.css, the chat_mobile part of static/css/mobile.css); client strings through js.chat.* (state.i18n)

### Gaps

**UI: routes**
- /chat/direct-messages and /chat/threads index routes, and chat_preferred_index other than channels. Rails: c/chat-route-map.js:19,22; c/routes/chat/direct-messages.js; c/routes/chat/threads.js. Port: src/routes/chat_pages.rs:200 (refusal "chat's direct messages and threads indexes"). [item 160]
- ~~Thread routes /chat/c/:slug/:id/t, /t/:thread_id, /t/:thread_id/:message_id and /:message_id/t/:thread_id~~ (done, 160: src/routes/chat_pages.rs channel_thread; src/plugins/chat/thread_view.rs).
- /chat/starred-channels and /chat/search routes (the mobile footer's and the sidebar's search link go there). Rails: c/chat-route-map.js:5,21; c/routes/chat/search.js; c/components/chat/routes/search.gjs. Port: absent (view.rs:430,462 and :569 link to them). [item 162]
- /chat/new-message route (the sidebar's "start a new DM" link, view.rs:681). Rails: c/chat-route-map.js:24; c/routes/chat/new-message.js. Port: absent.
- /chat/c/:slug/:id/info, /info/members, /info/settings (the navbar title links to it, page.rs:544; browse cards link members, page.rs:1791). Rails: c/chat-route-map.js:26-33; c/components/chat/routes/channel-info*.gjs; config/routes.rb:121-123. Port: absent (404).
- /chat/c/:slug/:id/pins (the navbar's pinned button, page.rs:581). Rails: c/chat-route-map.js:15; c/components/chat/routes/channel-pins.gjs; c/components/chat-pinned-messages-list.gjs. Port: absent (chat_pages.rs:412 404s "pins").
- /chat/message/:messageId (ChatMessageRoute, finds the channel and redirects). Rails: c/chat-route-map.js:41; c/routes/chat-channel-legacy.js. Port: absent.
- The /chat/channel/:channel_id and /chat/channel/:id/:title(/info...) legacy redirects. Rails: config/routes.rb:126,134-138. Port: absent.
- A channel without a slug refuses the channel page. Rails: c/routes/chat/channel.js (slugifyChannel). Port: src/routes/chat_pages.rs:406 (refusal "chat channels without a slug").
- A link to a message the list can't load refuses the page; Ember opens the channel and reports it. Rails: c/components/chat-channel.gjs:457 (highlightOrFetchMessage). Port: src/plugins/chat/page.rs:126 (refusal "a chat page for a message it can't load").

**UI: chat drawer**
- The drawer (ChatDrawer in the below-footer outlet) and all its routes (browse, channel, channel info members and settings, pins, threads, thread, DMs, starred, search), the drawer size and resizer, chat-state-manager's full page vs drawer preference, the header icon toggling the drawer, the navbar's open-drawer, full-page, toggle and close buttons (drawn inert at page.rs:572,1630). Rails: c/connectors/below-footer/chat-drawer-outlet.gjs; c/components/chat-drawer.gjs; c/components/chat/drawer-routes/*.gjs; c/services/chat-drawer-router.js; c/services/chat-drawer-size.js; c/services/chat-state-manager.js; c/components/chat/header/icon.gjs. Port: absent (CSS only).
- ChatHistory and the chat header's back-to-forum link to the last forum URL; the port always links "/". Rails: c/initializers/chat-setup.js:78; c/services/chat-history.js; c/components/chat-header.gjs. Port: src/routes/chat_pages.rs:118-124.

**UI: channel pane**
- Loading past and future messages on scroll (fetchMoreMessages), jumping to and highlighting a target message, the scroll-to-bottom arrow (drawn inert, page.rs:256). Rails: c/components/chat-channel.gjs:286,457,543,583; c/components/chat/scroll-to-bottom-arrow.gjs. Port: absent.
- The navbar filter button (in-channel search, ChatNavbarFilter) is inert. Rails: c/components/chat/navbar/filter.gjs; c/components/chat-search.gjs. Port: page.rs:565, no handler. [item 162 for the route; the in-channel filter is not covered]
- The retention reminder's dismiss button is inert (the endpoint exists). Rails: c/components/chat-retention-reminder.gjs. Port: page.rs:510, no handler in chat.js.
- The pinned message bar. Rails: c/components/chat/pinned-message-bar.gjs; c/components/chat-channel.gjs:819; c/lib/chat-pinned-bar-dismissal.js. Port: absent.
- Bus message types self_flagged, flag, notice, pin, unpin (thread_created and update_thread_original_message done, 160). Rails: c/lib/chat-channel-subscription-manager.js:68-86. Port: static/js/chat.js:1805 (onBus handles only sent, processed, edit, refresh, restore, reaction, delete, bulk_delete).
- The global chat subscriptions: new channels, channel status, channel edits, archive status, membership kicks, the user's new-channel and notices channels. Rails: c/services/chat-subscriptions-manager.js:105-509; c/services/chat-channel-notices-manager.js. Port: src/routes/live.rs:146-166 subscribes only new-messages, new-mentions and user-tracking-state. [archive status: item 162]
- Chat notices (mention_without_membership with its "invite" action, POST /chat/api/channels/:id/invites unrouted). Rails: c/components/chat-notices.gjs; c/components/chat/notices/mention_without_membership.gjs. Port: page.rs:173 draws an empty .chat-notices, no notice rendering.
- Composer mention warnings (unreachable, too many, channel-wide disabled, group hints via GET /chat/api/mentions/groups, unrouted). Rails: c/components/chat-mention-warnings.gjs; c/services/chat-composer-warnings-tracker.js. Port: absent.
- The composer dropdown (its "+" trigger is drawn inert, page.rs:811): upload, GIFs (enable_gifs, GifsModal), local dates, the mobile emoji button. Rails: c/initializers/chat-setup.js:88,118,132,156; c/components/chat-composer-dropdown.gjs. Port: absent. [uploads: item 161]
- Message decorators missing from chat.js: spoiler, hashtag icons, codeblock buttons, GitHub onebox body and scrolling, lightbox, transcript dates. Rails: c/initializers/chat-decorators.js:18-90; c/initializers/chat-plugin-decorators.js:24. Port: static/js/chat.js:170 (links, highlight, local dates only). [lightbox, oneboxes and transcripts: item 161]
- The author line shows the username always; Ember shows the name under prioritize_username_in_ux off / enable_names. Rails: c/components/chat/message/info.gjs (get name). Port: src/plugins/chat/page.rs:1449.
- The flag mark in the info line (you flagged, or a link to the reviewable). Rails: c/components/chat/message/info.gjs:183-193. Port: absent (data-user-flag-status is set at page.rs:1228, nothing draws it).
- Webhook and bot messages (bot indicator, message blocks and their interactions, POST .../interactions unrouted). Rails: c/components/chat/message/info.gjs:115-128; c/components/chat-message/blocks/*.gjs. Port: absent. [item 162]
- The stop-streaming button on a streaming message (DELETE .../messages/:id/streaming, unrouted). Rails: c/components/chat-message.gjs:586,787. Port: absent.
- Reactions without an emoji image refuse the page. Rails: c/components/chat-message-reaction.gjs. Port: src/plugins/chat/page.rs:1478 (refusal "chat reactions without an emoji image").
- The reaction users popover (ChatMessageReactionsUsers, GET /chat/:channel_id/:message_id/reactions-users, unrouted). Rails: c/components/chat-message-reaction.gjs:40; c/components/chat-message-reactions-users.gjs:88. Port: absent.
- User status in the author line and DM avatars. Rails: c/components/chat/message/info.gjs:152. Port: src/plugins/chat/channels.rs:654 (refusal "user status"). [item 130]
- Threads in the pane: the thread indicator, the thread panel, the thread lists and the threads list button are done (160); left: the thread title prompt, the thread settings modal, message actions in the thread pane, the side panel resizer, the lists' paging and live updates (ROADMAP 160).
- Uploads in the composer (picker, paste, drop zone, upload previews) and in messages (images, video, audio, collapser). Rails: c/components/chat-composer-uploads.gjs; c/components/chat-upload-drop-zone.gjs; c/components/chat-message-collapser.gjs. Port: page.rs:847 (the hidden field has no handler), page.rs:880,1315 (refusals). [item 161]
- Archived channel status with progress and retry (ChatChannelArchiveStatus). Rails: c/components/chat-channel-archive-status.gjs. Port: absent. [item 162]

**UI: message actions**
- "Select" is listed but does nothing: the selection mode with its checkboxes, shift-click bulk select, and the selection bar's quote to topic (ChatToTopicSelector and the composer, POST /chat/:id/quote unrouted), copy, move to channel (MoveMessageToChannel modal, POST .../messages/moves unrouted) and delete (DeleteMessagesConfirm, DELETE .../messages unrouted). Rails: c/lib/chat-message-interactor.js:257-275; c/components/chat/selection-manager.gjs:75-224; c/components/chat/modal/move-message-to-channel.gjs; c/components/chat/modal/delete-messages-confirm.gjs. Port: static/js/chat.js:1047 (listed), :1468 (no case).
- "Pin" and "Unpin" are listed but do nothing (POST/DELETE .../messages/:id/pin unrouted). Rails: c/lib/chat-message-interactor.js:427,443. Port: static/js/chat.js:1049-1052, :1468 (no case).
- "Flag" is listed but does nothing: the flag modal through ChatMessageFlag (POST .../messages/:id/flags unrouted). Rails: c/lib/chat-message-interactor.js:392; c/lib/chat-message-flag.js. Port: static/js/chat.js:1055, :1468 (no case).
- The bookmark button is drawn with its states but has no click (BookmarkModal on a chat message). Rails: c/lib/chat-message-interactor.js:357. Port: src/plugins/chat/actions_view.rs:90; no handler in chat.js. [item 161]
- "Copy text" in the mobile view. Rails: c/lib/chat-message-interactor.js:181,277. Port: absent from static/js/chat.js:1037.
- Mobile message actions: the long-press bottom sheet (ChatMessageActionsMobile) with its quick reactions and secondary actions; the desktop toolbar is hover-only. Rails: c/components/chat-message-actions-mobile.gjs; c/modifiers/chat/on-long-press.js. Port: absent (chat.js:1283 mouseover only).

**UI: channel info and settings**
- Channel settings (ChatRoutesChannelInfoSettings): edit name, slug and description modals, emoji, auto join, channel-wide mentions, threading toggle, desktop and mobile notification levels and mute (PUT .../notifications-settings/me unrouted), toggle open/closed (ToggleChannelStatus, PUT .../status unrouted), archive (ArchiveChannel, POST .../archives unrouted), delete (DeleteChannel, DELETE /chat/api/channels/:id unrouted), leave. Rails: c/components/chat/routes/channel-info-settings.gjs; c/components/chat/modal/edit-channel-name.gjs; c/components/chat/modal/edit-channel-description.gjs; c/components/chat/modal/toggle-channel-status.gjs; c/components/chat/modal/delete-channel.gjs; c/components/chat/modal/archive-channel.gjs. Port: absent. [archiving: item 162]
- Channel members (ChatRoutesChannelInfoMembers): the list with its filter and paging, and add members through the message creator (the endpoints exist). Rails: c/components/chat/routes/channel-info-members.gjs; c/components/chat/message-creator/add-members.gjs. Port: absent.
- Create channel modal (the browse page's New channel button is drawn inert, page.rs:1624): category chooser, permissions hint (GET /chat/api/category-chatables/:id/permissions unrouted), name, slug, description, auto join, threading (POST /chat/api/channels unrouted). Rails: c/components/chat/modal/create-channel.gjs; c/components/chat/navbar/new-channel-button.gjs. Port: absent.

**UI: new message and DMs**
- The new message modal and quick palette (Cmd/Ctrl+K): the message creator's search over chatables, new group with members, opening or creating a DM. Rails: c/components/chat/modal/new-message.gjs; c/components/chat/message-creator/*.gjs; c/initializers/chat-keyboard-shortcuts.js:156. Port: absent.
- The user card and profile Chat buttons are drawn without an action (ChatDirectMessageButton: create or open the DM, then route to it; the endpoint exists). Rails: c/connectors/user-card-below-message-button/chat-button.gjs; c/connectors/user-profile-controls/chat-button.gjs; c/components/chat/direct-message-button.gjs. Port: src/user_card_view.rs:338; src/user_profile_view.rs:485; no handler in static/js.
- The browse page's empty state CTA (start a DM) is inert. Rails: c/components/browse-channels.gjs. Port: src/plugins/chat/page.rs:1705.

**UI: sidebar and lists**
- The sidebar's section header buttons are inert: channel list options (ChatChannelListSidebarMenu: create channel, browse, new DM, new group, filter and sort choices saved to the user options) and the empty-filter toggle (toggleChannelFilter). Rails: c/initializers/chat-sidebar.js:245-279; c/components/chat-channel-list-sidebar-menu.gjs:145-237. Port: src/plugins/chat/view.rs:732,739; no handler in static/js. (named as left in done item 99)
- The mobile channels list's options button is inert. Rails: c/components/chat/navbar/channel-list-options-button.gjs. Port: src/plugins/chat/view.rs:350. (named as left in done item 99)
- A channel link's hover menu (ChatChannelSidebarContextMenu: notification settings submenu, channel settings, star/unstar, leave) is drawn without an action. Rails: c/initializers/chat-sidebar.js:436-470,665-685,965-981; c/components/chat-channel-sidebar-context-menu.gjs:122-151. Port: src/plugins/chat/view.rs:788; no handler.
- Swipe and long press on a channel row in the mobile list. Rails: c/components/chat-channel-row.gjs. Port: absent. (named as left in done item 99)
- The separate sidebar modes (always, fullscreen), the chat panel and its switch buttons. Rails: c/initializers/chat-sidebar.js:298-317. Port: src/plugins/chat/view.rs:276 (refusal "chat's separate sidebar"). [item 162]
- Empty states of the channels list (none, and none after the filter). Rails: c/components/chat/list/empty-state.gjs; c/components/chat-sidebar-channel-list-filter-empty-state.gjs. Port: src/plugins/chat/view.rs:341,345 (refusals). [item 162]
- The channels list for a member without chat options refuses. Rails: c/components/channels-list-public.gjs. Port: src/plugins/chat/view.rs:319,322 (refusals "chat's channels list without a member", "... without chat options").
- Anonymous viewing of public chat (chat-enabled body class, card listener, the sidebar). Rails: c/initializers/chat-setup.js:65-75; c/initializers/chat-sidebar.js:289. Port: absent. [item 162]

**UI: notifications, sounds, title, shortcuts**
- Desktop notifications for chat (registerDesktopNotificationHandler) and the new message sound (chat_sound, ChatAudioManager), with chat_new_message_sound in the channel. Rails: c/initializers/chat-audio.js:28; c/services/chat-audio-manager.js; c/services/chat-notification-manager.js. Port: absent. [the preferences' preview only: item 162]
- The document title counter (unread chat count in the tab title). Rails: c/initializers/chat-setup.js:210,234. Port: absent.
- The screen reader announcement of new messages (chat_announce_new_messages, ChatNewMessageAnnouncer). Rails: c/services/chat-new-message-announcer.js; c/lib/chat-message-announcement.js. Port: absent (the option saves, nothing reads it in the browser).
- Keyboard shortcuts: Alt+Up/Down and Alt+Shift+Up/Down to switch channels, Cmd/Ctrl+B, I, E and L in the composer, "-" to toggle the drawer, Esc to close it, Shift+Esc to mark all channels read, and the chat section of the keyboard shortcuts help. Rails: c/initializers/chat-keyboard-shortcuts.js:156-300. Port: absent (only the composer's Enter, Escape and ArrowUp, chat.js:908-960).

**UI: admin and category**
- The admin plugin page for chat with its configuration nav (settings, incoming webhooks). Rails: plugin.rb:35; admin/assets/javascripts/discourse/initializers/chat-admin-plugin-configuration-nav.js. Port: src/routes/admin_plugins.rs:110 (refusal "a plugin's own admin pages (its configuration nav)").
- Incoming webhooks admin: list, new, edit (name, description, channel, username, emoji or icon), delete, and the endpoints /admin/plugins/chat/hooks (index, create, update, new, edit, destroy, unrouted). Rails: admin/assets/javascripts/discourse/admin-chat-route-map.js; admin/assets/javascripts/admin/components/admin-chat-incoming-webhooks-list.gjs; admin/assets/javascripts/admin/components/chat-incoming-webhook-edit-form.gjs; plugin.rb:538-551. Port: absent.
- The admin page's "export chat messages" action (POST /chat/api/admin/export/messages, unrouted). Rails: admin/assets/javascripts/admin/components/chat-admin-plugin-actions.gjs; config/routes.rb:71. Port: absent.
- The category editor's Chat tab (the category's channels, include subcategories). Rails: c/initializers/chat-category-tab.js:10; c/components/upsert-category/chat.gjs. Port: absent (src/category_edit_view.rs:369 tab list has no chat tab).
- The review queue's chat message reviewable (ReviewableChatMessage). Rails: c/initializers/chat-setup.js:44; c/components/reviewable/chat-message.gjs. Port: absent. [item 194]

**UI: other**
- The rich editor extension for chat markup. Rails: c/initializers/chat-setup.js:229; assets/javascripts/lib/rich-editor-extension.js. Port: absent. [item 196]
- Chat transcripts in posts and their date decoration. Rails: c/initializers/chat-setup.js:170. Port: absent. [item 161]

### Items

New (ROADMAP Phase 11): 215 `feat/chat-channel-settings` M, 216 `feat/chat-channel-create` M, 217 `feat/chat-moderation` L, 218 `feat/chat-message-actions` M, 219 `feat/chat-pins` S, 221 `feat/chat-admin` M, 222 `feat/chat-new-message` M, 223 `feat/chat-drawer` L, 224 `feat/chat-pane` M, 225 `feat/chat-notify-client` S. The channel list options menu and the row swipe and long press, which done item 99 left without an owner, are in 216 and 218. Earlier items: 160, 161, 162, 130, 194, 196.

### Couldn't determine
- Whether the GitHub onebox and lightbox decorators belong to 161 (oneboxes and images in messages are refused there, so they can't show yet) or need their own work once 161 lands; listed under 161.
- Whether 162's "the mobile footer's search and starred routes" means the full chat search (ChatSearch, GET /chat/api/search, results and filters) or only the two routes; listed under 162 for the routes, with the in-channel filter button under feat/chat-pane.
- Whether the styleguide sections (c/components/styleguide/*.gjs, api.addStyleguideSection) matter: they need core's styleguide, which the port doesn't seem to have.
- Whether the chat.css bundle matches every selector of the reference scss: it says it was compiled from common and desktop. Confirming that means diffing against Rails' compiled CSS, which needs a running Rails.
- Live behavior (the drawer, scroll paging, the bus) can only be compared by running both, and this is a source audit.

## Poll (`plugins/poll`)

Verdict: started (27 gaps)

### What the port has
- Cooking: `[poll]` BBCode with all 13 ALLOWED_ATTRIBUTES, title heading, generated number-poll options, md5 option ids, nested polls left plain, poll-info stub, allow list (crates/markdown/src/render/poll.rs:26,117-229,231); `poll_enabled`, `poll_maximum_options` and the `js.poll.voters` label fed in (src/pretty_text/mod.rs:547-550).
- Extract and validate on create: `Poll.extract` (outside blockquotes), PollsValidator (arguments, numbers, unique names and options, blank and missing options, maximum, multiple-choice bounds), PostValidator incl. `pm_with_non_human_user?` (src/plugins/poll/mod.rs:100-375; src/posting/create.rs:247-249,619,629).
- Write on create: `polls`, `poll_options`, `has_polls` custom field, `schedule_jobs`, `/polls/<topic_id>` publish for replies (src/plugins/poll/mod.rs:490-619; src/posting/create.rs:966-968,1335).
- Edits: PollsUpdater (name diffs, attribute updates, dynamic vs replace options keeping anonymous_votes, edit window error, has_polls upkeep, anonymous publish) (src/plugins/poll/updater.rs; src/posting/revise.rs:378-474).
- PollsController, all five actions and their params incl. `page`/`limit`/`option_id`, `.json` variants, requires_plugin, ensure_logged_in except voters and grouped results, ensure_can_see_post (src/routes/polls.rs; src/routes/mod.rs:455-470; src/plugins/poll/vote.rs: vote with ranked choice, multiple, offset cleanup, `chosen`/`rank`; remove_vote; toggle_status with closed_by/closed_at and `poll_closed`/`poll_opened` staff log; voters; grouped_poll_results).
- PollSerializer and PollOptionSerializer keys incl. `preloaded_voters`, `ranked_choice_outcome` (RankedChoice ported with tests), `closed_by` skipping the system user; post `polls` and `polls_votes` in the topic view and single-post serializers (src/plugins/poll/serializer.rs:206-310,453-570; src/topic_view.rs:1577,1968).
- `can_create_poll` on the current user (src/current_user.rs:889-891; src/plugins/poll/mod.rs:622).
- Jobs: ClosePoll as the system user, quiet refusals (src/jobs/handlers.rs:63,1209).
- Events: `merging_users` (src/user_merger.rs:700-709), User `has_many :poll_votes, dependent: :delete_all` (src/plugins/mod.rs:239), `reduce_excerpt`, gated by `poll_enabled` (src/excerpt.rs:238-246), `in:polls` advanced filter and its search-menu suggestion (src/search/advanced.rs:185; src/search_menu_view.rs:1067).
- Settings: all nine from plugin_settings.yml; none has a change handler in Rails. Admin plugin icon `chart-bar` (src/admin_view.rs:175). English server and client locales loaded (src/i18n.rs:25-27,75-76).
- Composer options menu lists "Build Poll" for staff or poll_create_allowed_groups (src/composer_view.rs:280-285; src/html.rs:1118), but as unported (see UI).

### Gaps
**plugin.rb**
- `on(:reduce_cooked)`: emails replace each poll with its option list (option ids stripped) plus a "link_to_poll" link to the post, and drop polls entirely for a nil or trashed post; the port's format_for_email has no poll handling, so raw poll markup (data-poll-option-id, the "0 voters" stub) goes into emails. Rails: plugins/poll/plugin.rb:94-114. Port: src/email/notification.rs:821-856 absent.
- `register_anonymous_action("vote_poll")`: an anonymous click on a single-choice poll is queued and replayed after login; core's AnonymousAction (`/anonymous-actions`, consume on login) is not ported, the login refuses when the cookie is present. Rails: plugins/poll/plugin.rb:265-273. Port: src/session/login.rs:25-27 (refusal "replaying an anonymous action after a login"), no route.
- `register_modifier(:clean_up_inactive_users_query)`: users with poll votes are spared by CleanUpInactiveUsers; the port's query lacks the `NOT EXISTS (poll_votes)` clause, so it destroys poll voters (and their votes). Rails: plugins/poll/plugin.rb:276-278. Port: src/jobs/user_cleanup.rs:106-119 absent.
- `NewPostManager.add_handler(1)` / `allow_new_queued_post_payload_attribute("is_poll")`: a post that would be queued is poll-validated first and queued with `is_poll`; the port refuses enqueueing any post with polls. Rails: plugins/poll/plugin.rb:42,75-88. Port: src/posting/enqueue.rs:96-97 (refusal "enqueueing a post with polls"). [item 138]
- `on(:approved_post)`: approving a queued `is_poll` post creates its polls. Rails: plugins/poll/plugin.rb:90-92. Port: src/review_posts.rs:623-624 (refusal "approving a queued post with polls"). [item 148]
- `on(:post_moved)` for a copied post: delete the copy's auto-created empty polls, move the originals (with votes), fix both posts' has_polls. Rails: plugins/poll/plugin.rb:145-163. Port: src/post_mover.rs:811-817 (refusal "moving a copied post's polls"). [item 188i4]
- `reduce_excerpt` replacement text is hard-coded "poll" instead of `I18n.t("poll.poll")` (so site text overrides and other locales are ignored) and the URL is HTML-escaped rather than `UrlHelper.normalized_encode`d. Rails: plugins/poll/plugin.rb:116-126. Port: src/excerpt.rs:240-245.
- `Poll.extract` cooks with the post's `topic_id` and `user_id`; the port's create path cooks with default MarkdownOptions, so stored option HTML can differ where cooking depends on them. Rails: plugins/poll/lib/poll.rb:455. Port: src/plugins/poll/mod.rs:143-147.

**Routes and controllers**
- `vote` with a ranked-choice rank that is not an integer: Rails casts it into the integer column; the port refuses. Rails: plugins/poll/lib/poll.rb:72-77. Port: src/plugins/poll/vote.rs:237-245 (refusal "a ranked choice vote with a rank that is not an integer").
- Logged-in voting while in anonymous posting mode (shadow account). Port: src/plugins/poll/vote.rs:263,485 (refusal "voting anonymously"). [item 133]
- grouped_poll_results titleizing a non-ASCII user field value. Rails: plugins/poll/lib/poll.rb:391. Port: src/plugins/poll/vote.rs:740-742 (refusal "titleizing a non-ASCII user field value"). [item 153]

**Serializers**
- parity/cases still ignores `/current_user/can_create_poll` although the port now emits it; the key is unverified against Rails. Rails: plugins/poll/plugin.rb:206-208. Port: parity/cases:163-166 (ignore list), src/current_user.rs:889.

**Jobs**
- None missing (ClosePoll ported).

**Models**
- None missing (the port reads the reference tables; the data migrations are Rails' own).

**Settings**
- None missing on the server; `poll_default_public`, `poll_export_*_data_explorer_query_id` are only read by the missing UI below.

**Locales**
- Every locale but English for `poll.*` and `js.poll.*`. Rails: plugins/poll/config/locales/*.yml. Port: src/i18n.rs:25-27 (English only). [item 167]

**UI**
- The poll widget on topic pages: no static/js counterpart at all, so a post's poll renders as its static cooked list with a "0 voters" stub; nothing replaces it (attachPolls, decorateCookedElement, quoted-post polls via `.expanded-quote`), no options as radio/checkbox buttons, no single-click vote, Cast Votes / update votes for multiple, Remove vote, Change vote / keep vote (amending), Show / Hide results, the groups restriction notice, the closed-by line. Rails: plugins/poll/assets/javascripts/discourse/initializers/extend-for-poll.gjs:8-58; components/poll.gjs:530-640,781-856; components/poll-options.gjs; components/poll-voted-choices.gjs; components/poll-info.gjs. Port: absent.
- poll-info: voters / total votes / average rating, multiple-choice help (x, at least, up to, between), results-visibility hints (on vote, on close, staff only), public hint, automatic close countdown and age, dynamic hint, "vote recorded". Rails: plugins/poll/assets/javascripts/discourse/components/poll-info.gjs. Port: absent.
- Results views: standard bars with even-round percentages and tally/percentage toggle, pie chart (Chart.js, chart-colors legend), ranked choice outcome and rounds (winner, tied, majority, eliminated) with Votes/Outcome tabs, number-poll results. Rails: components/poll-results-standard.gjs, poll-results-pie.gjs, poll-results-ranked-choice.gjs, poll-results-tabs.gjs; lib/even-round.js; lib/chart-colors.js. Port: absent.
- Ranked choice voting UI: per-option rank dropdown with Abstain. Rails: components/poll-option-ranked-choice.gjs, poll-option-ranked-choice-dropdown.gjs. Port: absent.
- Voters lists: avatars per option for public polls with load-more paging via /polls/voters.json, and ranked choice voters by rank. Rails: components/poll-voters.gjs, poll-voters-ranked-choice.gjs; poll.gjs:704-779. Port: absent (the endpoint exists, no caller).
- Poll actions dropdown: Close / Open poll with confirm (staff or OP, not archived), Show tally / Show percentage, Breakdown, Export results. Rails: components/poll-buttons-dropdown.gjs:11-50,95-120; poll.gjs:810-856. Port: absent.
- Breakdown modal: group results by a `poll_groupable_user_fields` field from /polls/grouped_poll_results, one pie per group, option list with highlight. Rails: components/modal/poll-breakdown.gjs; poll-breakdown-chart.gjs; poll-breakdown-option.gjs. Port: absent (endpoint exists, no caller).
- Export results to CSV through discourse-data-explorer's `/admin/plugins/discourse-data-explorer/queries/:id/run.csv` with `poll_export_data_explorer_query_id` / `poll_export_ranked_choice_data_explorer_query_id` (admin, data explorer on, voters > 0). Rails: components/poll.gjs:868-900. Port: absent; data explorer is unported. [item 202]
- Live updates: the topic controller subscribes to `/polls/<topic_id>` and swaps the post's polls; the port publishes but nothing subscribes. Rails: initializers/extend-for-poll.gjs:63-92. Port: absent.
- Anonymous click: deferAnonymousAction("vote_poll") for single choice, login modal for multiple/ranked. Rails: components/poll.gjs:595-609. Port: absent (see plugin.rb gap).
- Poll builder modal from the composer's options menu: type tabs (regular, multiple, number, ranked choice), title, options editor with add/remove and min/max count help, min/max/step with validation help, results visibility (always, on vote, on close, staff), public toggle defaulting to `poll_default_public`, chart type bar/pie, groups chooser, automatic close date-time, dynamic toggle, Insert (BBCode into the composer). The menu item is marked unported, so the options menu treats it as not ported; its condition also lacks `composer.model.topic.pm_with_non_human_user`. Rails: components/modal/poll-ui-builder.gjs (762 lines); initializers/add-poll-ui-builder.js:6-22. Port: src/composer_view.rs:280-285 (PopupOption with `ported` false), src/html.rs:1118-1120.
- Rich editor (ProseMirror) poll extension and node view (edit an existing poll in the builder). Rails: lib/rich-editor-extension.js; components/poll-node-view.gjs. Port: absent. [item 196]
- Poll stylesheets: poll.scss, poll-ui-builder.scss (common and desktop), poll-breakdown.scss. Rails: plugins/poll/assets/stylesheets/*. Port: static/css/cooked.css:109 (one emoji rule only).

**Markdown**
- A poll whose content does not start with a list: Rails leaves an opened `div.poll` with its data attributes and nothing in it (the sanitizer closes it); the port refuses the cook. Rails: plugins/poll/assets/javascripts/lib/discourse-markdown/poll.js:196-200. Port: crates/markdown/src/render/poll.rs:200-203 (refusal "a poll whose content does not start with a list").

**Emails and notifications**
- Poll rendering in emails (the reduce_cooked gap above). The plugin has no notification types.

### Items

New (ROADMAP Phase 11): 228 `feat/poll-ui` L (228a `feat/poll-ui-vote` M, 228b `feat/poll-ui-results` M, 228c `feat/poll-ui-breakdown` S), 229 `feat/poll-builder` M, 230 `feat/poll-rest` M, and `vote_poll` in 211 `feat/anonymous-actions` M. Earlier items: 133, 138, 148, 153, 167, 188i4, 196, 202 (results export through data explorer).

### Couldn't determine
- Whether Rails' `options.select!` on a non-ranked vote whose `options` param is a hash (not an array) yields a 500 or a vote; the port refuses ("poll votes not given as a list", src/plugins/poll/vote.rs:303). Needs a Rails run.
- How much option HTML actually differs when `Poll.extract` cooks without topic_id/user_id (the gap above may be invisible in practice). Needs Rails.
- Whether `reduce_cooked` also reaches other port email paths (digest, group SMTP) that do not go through format_for_email.

## Discourse Reactions (`plugins/discourse-reactions`)

Verdict: started (23 gaps)

### What the port has
- Settings model (`Reaction.valid_reactions`, `main_reaction_id`, `reactions_excluded_from_like`, `reactions_counting_as_like`, `valid?` incl. allow_any_emoji with custom emoji and skin tones): src/plugins/reactions/mod.rs:36-131.
- Post serializer keys `reactions`, `current_user_reaction`, `reaction_users_count`, `current_user_used_main_reaction` (preload_post_reactions with ignored users): src/plugins/reactions/mod.rs:165-358, wired in src/topic_view.rs:1532-1564,1954-1956 (so /t, /posts/:id and the post web hook payload get them); topic view `valid_reactions`: src/topic_view.rs:609, src/topic_view/web_hook.rs:582.
- All five controller actions routed (src/routes/mod.rs:413-452): toggle (src/routes/reactions.rs:29, src/plugins/reactions/toggle.rs:109, ReactionManager with shadow likes, rate limits, undo window, notification create/delete/refresh, `/topic/:id/reactions` and `acted` publishes), reactions_given and reactions_received with before_reaction_user_id, before_post_id fallback, before_like_id, acting_username, include_likes (src/routes/reactions.rs:245-354, src/plugins/reactions/users.rs:471,551), reactions_users_list (PostReactionsQuery, page/limit clamps) and post_reactions_users (src/plugins/reactions/users.rs:104,303). `custom-reactions` index 404s like Rails (action missing in the controller), parity case parity/writes/reactions/custom_reactions_index.case.json.
- UserReactionSerializer with GroupPostSerializer post/user: src/plugins/reactions/users.rs:662.
- ScheduledLikeSynchronizer (hourly) with the full ReactionLikeSynchronizer SQL incl. TopicUser.update_post_action_cache: src/plugins/reactions/like_sync.rs:22, scheduled in src/jobs/mod.rs:298,596.
- Notification type `reaction` (25): src/site.rs:53, src/notifications.rs:40; reacted_by_two_users and consolidated_reactions plans: src/notification_create.rs:219-384; PostAlerterExtension should_notify_previous: src/jobs/post_alert.rs:815; user menu renderer (title, href to reactions-received/likes-received, icon, 2-user/multi-user labels): src/user_menu_view.rs:965-1011; reactions in the Likes tab of the user menu: src/user_menu_view.rs:155.
- `reactions` admin report: src/reports/tables.rs:1407, src/reports/mod.rs:176.
- user_action_stream_builder modifier (shadow likes hidden from likes streams): src/users.rs:1806, src/plugins/reactions/mod.rs:363.
- ReactionForLikeSiteSettingEnum values: src/setting_enums.rs:91. Admin plugin entry with the discourse-emojis icon: src/admin_view.rs:765.
- "First Reaction" badge seeded: seed/fresh_install.sql:749.
- UI: post menu reaction button in the like slot, picker, counter + reactions list, users menu with per-reaction filter tabs and paging over reactions-users-list, hidden-post suppression, desaturated panel, like icon: src/plugins/reactions/view.rs, src/post_view.rs:947-1041, static/js/discourse-reactions.js, static/css/discourse-reactions.css. Activity tab "Reactions" (user-activity-bottom connector) and its PostList with the reaction emoji: src/user_profile_view.rs:802,1327, src/routes/users.rs:733. Notifications tab "Reactions" (user-notifications-bottom connector) and reactions-received page: src/user_notifications_view.rs:143,279, src/routes/user_notifications.rs:138.
- Locales: English client and server yml loaded through vendor/discourse/config/locales/plugin_{client,server}.en.yml (src/i18n.rs:22-28); other locales are refused port-wide.
- Parity: 55 cases under parity/writes/reactions/, incl. messages (react_pm), consolidation and ignoring.

### Gaps

**plugin.rb**
- `add_model_callback(User, :before_destroy)` deleting the user's ReactionUser rows: user deletion leaves them behind. Rails: plugins/discourse-reactions/plugin.rb:182. Port: src/plugins/mod.rs:200 (user_destroying has no reactions arm) | absent.
- `on(:first_post_moved)` copying reactions and reaction users to the copied first post. Rails: plugins/discourse-reactions/plugin.rb:401. Port: src/post_mover.rs:996 (refusal "copying a first post's reactions (discourse-reactions)"). [item 188i4]
- `register_anonymous_action("react_to_post")` plus the client's deferAnonymousAction: an anonymous reader's picked reaction is saved and replayed after login. Rails: plugins/discourse-reactions/plugin.rb:66, plugins/discourse-reactions/assets/javascripts/discourse/components/discourse-reactions-actions.gjs:334,536. Port: src/session/login.rs:26 (refusal "replaying an anonymous action after a login"); the button just links to /login (src/plugins/reactions/view.rs:181); core POST /anonymous-action not routed.
- `TopicList.on_preload` and `op_reactions_data` on topic_list_item and suggested_topic, behind the `include_discourse_reactions_data_on_topic_list` / `_on_suggested_topics` modifiers. Rails: plugins/discourse-reactions/plugin.rb:94,154,167, lib/discourse_reactions/reactions_serializer_helpers.rb:297. Port: absent (no modifier hook, no key).
- `register_modifier(:post_action_users_list)` filtering likers who also reacted out of the "who liked" avatars. Rails: plugins/discourse-reactions/plugin.rb:387 (applied in app/controllers/post_action_users_controller.rb:30). Port: absent (core GET /post_action_users is not routed either).
- `on(:site_setting_changed)` for `discourse_reactions_excluded_from_like`: cancel and re-enqueue LikeSynchronizer 5 minutes out. Rails: plugins/discourse-reactions/plugin.rb:429. Port: src/plugins/mod.rs:310 (no reactions arm; the setting itself can't be saved, src/site_setting_update.rs:212). [item 164]
- MCP tool `discourse_reactions_post_reaction_set` (scope `discourse-reactions:write`). Rails: plugins/discourse-reactions/plugin.rb:440, plugins/discourse-reactions/lib/discourse_reactions/mcp_tools.rb:5. Port: absent (no MCP server in the port).
- Dev rake task `reactions:generate`. Rails: plugins/discourse-reactions/lib/tasks/reactions.rake:128. Port: absent.

**Routes and controllers**
- reactions_given / reactions_received refuse under content localization. Rails: plugins/discourse-reactions/app/controllers/discourse_reactions/custom_reactions_controller.rb:65,150. Port: src/plugins/reactions/users.rs:665 (refusal "reactions with content localization"). [item 168]
- reactions_given / reactions_received refuse topics without a stored slug. Rails: same, GroupPostSerializer url. Port: src/plugins/reactions/users.rs:703 (refusal "topics without a stored slug (Slug.for)"). [item 120]

**Serializers**
- Topic view as a viewer who ignores a reactor (core TopicView hides ignored users' posts). Rails: plugins/discourse-reactions/lib/discourse_reactions/reactions_serializer_helpers.rb:14. Port: parity/writes/reactions/topic_view_ignoring_a_reactor.case.json:35 (`not_yet`). [item 124]
- `op_reactions_data` on topic list items and suggested topics: counted under plugin.rb above.

**Jobs**
- Regular `Jobs::DiscourseReactions::LikeSynchronizer` (the one the setting handler enqueues) is not registered; only the scheduled one is. Rails: plugins/discourse-reactions/app/jobs/regular/discourse_reactions/like_synchronizer.rb:5. Port: src/jobs/mod.rs:596 (only `ScheduledLikeSynchronizer`; the regular name falls to the "not ported" arm) | absent.
- The sync's pre-run and post-run MigrationReport logging. Rails: plugins/discourse-reactions/app/services/discourse_reactions/reaction_like_synchronizer.rb:16,58, plugins/discourse-reactions/lib/discourse_reactions/migration_report.rb:6. Port: src/plugins/reactions/like_sync.rs:22 | absent.

**Models**
- None: the port reads and writes the Rails-created `discourse_reactions_reactions` and `discourse_reactions_reaction_users` tables.

**Settings**
- Saving `discourse_reactions_enabled_reactions`, `discourse_reactions_excluded_from_like` (emoji_list), `discourse_reactions_like_icon` (icon) is refused, and `discourse_reactions_reaction_for_like` (enum class). Rails: plugins/discourse-reactions/config/settings.yml:27-48. Port: src/site_setting_update.rs:212 (refusal "setting types other than string, number, bool and enum"), :560 (refusal "enum settings with an enum class"). [item 164]
- ReactionsExcludedFromLikeSiteSettingValidator (excluded emojis must be enabled or default-excluded, never the like reaction; error `site_settings.errors.invalid_excluded_emoji`). Rails: plugins/discourse-reactions/lib/reactions_excluded_from_like_site_setting_validator.rb:8. Port: absent. [item 164]

**Locales**
- None plugin-specific (English loaded; other locales are a port-wide refusal).

**UI**
- Allow-any-emoji button in the picker (full emoji picker, custom emoji). Rails: plugins/discourse-reactions/assets/javascripts/discourse/components/discourse-reactions-picker.gjs:196. Port: src/plugins/reactions/view.rs:190 ("is not ported").
- Infinite scroll on the Reactions activity tab and the reactions-received page (loadMore with before_reaction_user_id / before_like_id, hideApplicationFooter while more): only the first 20 render. Rails: plugins/discourse-reactions/assets/javascripts/discourse/controllers/user-activity/reactions.js:17, templates/user-activity/reactions.gjs:6. Port: src/routes/users.rs:757, src/routes/user_notifications.rs:145, sentinel without data-next-page at src/user_profile_view.rs:1418.
- reactions-received page ignores `acting_username` and `include_likes` query params, which the reaction notification link always sends. Rails: plugins/discourse-reactions/assets/javascripts/discourse/routes/user-notifications/reactions-received.js:11. Port: src/routes/user_notifications.rs:148-153 (hard-coded None/false; link built at src/user_menu_view.rs:975).
- User nav sidebar links `activity-reactions` and `notifications-reactions` (addUserNavSidebarLink). Rails: plugins/discourse-reactions/assets/javascripts/discourse/initializers/discourse-reactions.gjs:18. Port: absent (core's user nav sidebar panel is absent too).
- Keyboard shortcut `l` toggling the selected post's reaction button. Rails: plugins/discourse-reactions/assets/javascripts/discourse/initializers/discourse-reactions.gjs:39. Port: absent.
- Admin emoji-value-list override for `discourse_reactions_enabled_reactions`: the like reaction prepended when absent and locked from editing. Rails: plugins/discourse-reactions/assets/javascripts/discourse/initializers/discourse-reactions.gjs:60. Port: src/admin_site_settings_view.rs:1486 | absent.
- Touch long-press to open the picker, vibration, and the reaction animations. Rails: plugins/discourse-reactions/assets/javascripts/discourse/components/discourse-reactions-actions.gjs:234-300,345,566. Port: static/js/discourse-reactions.js:7 ("not ported").
- Mobile: users menu as a modal (`modalForMobile`) and the mobile stylesheet. Rails: plugins/discourse-reactions/assets/javascripts/discourse/components/discourse-reactions-counter.gjs:73, plugins/discourse-reactions/assets/stylesheets/mobile/discourse-reactions.scss. Port: static/css/discourse-reactions.css:1 (common and desktop only) | absent.

**Markdown**
- None (the plugin does no cooking).

**Emails and notifications**
- None found: `reaction` has no EmailUser method and is not in PostAlerter::NOTIFIABLE_TYPES, so Rails sends no email or push for it; the port matches.

### Items

New (ROADMAP Phase 11): 231 `feat/reactions-lifecycle` S/M, 232 `feat/reactions-ui` M, `react_to_post` in 211 `feat/anonymous-actions` M, the MCP tool in 212 `feat/mcp-server` L. Earlier items: 120, 124, 164, 168, 188i4. The ROADMAP note that reactions in messages are refused was stale (no refusal is left in plugins/reactions/toggle.rs; the react_pm cases cover them); milestone 5's bullet is corrected and 231 drops the module comment's claim.

### Couldn't determine
- Whether any Rails code path enables the `include_discourse_reactions_data_on_topic_list` / `_on_suggested_topics` modifiers by default (none in ref core or bundled plugins; a theme or third-party plugin could). If nothing does, `op_reactions_data` is never serialized on a stock site.
- `enable_new_post_reply_count_position` and `enable_discourse_reactions_by_default` are permanent upcoming-change settings with no reader in ref; whether the port's admin upcoming-changes page needs them listed would take running Rails.
- Stale docs: src/plugins/reactions/toggle.rs:11 and ROADMAP.md:2835 say reactions in messages are refused, but parity/writes/reactions/react_pm*.case.json exercise them and no refusal remains in toggle.rs. Worth correcting in the ROADMAP pass.
- src/plugins/reactions/toggle.rs:119 ("reacting anonymously", listed under item 133) looks unreachable from the route, which already answers anonymous viewers with not_logged_in (src/routes/reactions.rs:48).

## discourse-solved (`plugins/discourse-solved`)

Verdict: started (30 gaps)

### What the port has
- Enabled check, topic list data (answered topics, answerers, the category fields the list reads), TopicPostersSummaryExtension's answerer rewrite: src/plugins/solved/mod.rs:22-140; plugins/mod.rs:122-150.
- GuardianExtensions: allow_accepted_answers?, solved_enabled_for_category? (all topics, solved tags, enable_accepted_answers), can_accept_answer?, can_unaccept_answer?, category group moderators, can_create_shared_issue?, shared_issue_visible? with the upcoming change: src/plugins/solved/mod.rs:179-504.
- AcceptAnswer and UnacceptAnswer: revoke previous, SOLVED user action, solved row with the silent auto-close timer (category hours capped at MAX_AUTO_CLOSE_HOURS, then the site setting), topic answer, notify answerer / topic owner (notify_on_staff_accept_solved) / tracking users with notify_on_solved and UserCommScreener, notification removal on unaccept, the accepted_solution and unaccepted_solution web hooks: src/plugins/solved/answers.rs:245-537; src/web_hooks/mod.rs:469-502.
- Routes: POST /solution/accept, /solution/unaccept (rate limits, contract 400, 403/404/422, publish on /topic/:id), GET /solution/by_user (SolvedPostSerializer), POST /solution/shared_issue (rate limit, toggle, tracking, bus): src/routes/solved.rs; src/routes/mod.rs:383-390; src/plugins/solved/by_user.rs; src/plugins/solved/shared_issue.rs.
- API key scope solved:answer (vendored api_key_scopes.json:877).
- Serializer keys: post (can_accept_answer, can_unaccept_answer, accepted_answer, topic_accepted_answer, mod.rs:383-409), topic view (accepted_answers, has_accepted_answer, shared issue keys; topic_view.rs:2018-2120, mod.rs:414-478), TopicAnswerMixin on list item / listable / suggested / search / category list / user summary topics (topic_list.rs:381,402-440; users.rs:1526), user card accepted_answers (users.rs:890), user summary solved_count (users.rs:1628), user_option notify_on_solved (user_private.rs:922; user_updater.rs:216), web hook post payloads (topic_view/web_hook.rs:573).
- Search: status:solved / status:unsolved, prioritize_solved_topics_in_search, the eager load (search/advanced.rs:186; search/filters.rs:272; search/posts_query.rs:440,566,823), the menu's suggestions and the advanced panel's statuses (search_menu_view.rs:1068; search_advanced_view.rs:71).
- Report accepted_solutions with category_ids filter and related items data (reports/counts.rs:1067-1232), the dashboard highlight KPI (admin_dashboard.rs:527-557; jobs/dashboard.rs:47), directory_items solutions column query (directory.rs:138-160).
- Support category type (configure/unconfigure, schema): category_types.rs:132,234-310; category_type_config.rs:95,247-314. User destroy deletes shared issues (plugins/mod.rs:240).
- UI: post menu Solved button, accepted answers accordion, shared issue "Me too" button, NoAnswer popup, topic list/title statuses and status-solved class, user activity Solved tab and empty states, summary stat, user card metadata, notify_on_solved preference, admin plugin icon, live bus handling of accepted_solution / unaccepted_solution / shared_issue: plugins/solved/view.rs; post_view.rs:530,922,1659; topic_list_view.rs:778,1299-1311; user_profile_view.rs:821-876,1236,1587; user_card_view.rs:496; preferences_view/notifications.rs:254,344; admin_view.rs:783; routes/live.rs:411,621-660; static/js/discourse-solved.js.
- Locales: plugin client and server yml merged into vendor/discourse/config/locales/plugin_{client,server}.en.yml and loaded (i18n.rs:26-28). Badges Solved 1-4 are rows in seed/fresh_install.sql and run through the generic badge granter.

### Gaps

**plugin.rb**
- `on(:post_destroyed)`: deleting an accepted answer post does not unaccept it (topic stays solved, SOLVED user action and notifications stay). Rails: plugins/discourse-solved/plugin.rb:364. Port: absent (post_destroyer.rs has no solved hook; plugins/mod.rs has none).
- `on(:before_post_publish_changes)`: a category or tag change that moves a topic out of solved does not unaccept its answers, and neither direction sets refresh_stream. Rails: plugins/discourse-solved/plugin.rb:389. Port: absent (posting/revise_topic.rs:958 calls plugins::topic_category_revised, which is voting's only).
- `on(:filter_auto_bump_topics)`: auto bump picks solved topics. Rails: plugins/discourse-solved/plugin.rb:373. Port: src/jobs/periodical.rs:251 (query without the NOT EXISTS solved filter).
- PostMoverExtension: moving accepted answers to another topic neither re-homes the topic answers (target allows answers and has room) nor unaccepts them, and the emptied solved row is not destroyed. Rails: plugins/discourse-solved/lib/discourse_solved/post_mover_extension.rb:8. Port: absent (src/post_mover.rs has no solved handling).
- ComposerMessagesFinder#check_topic_is_solved (education message on replying to a solved topic, disable_solved_education_message). Rails: plugins/discourse-solved/plugin.rb:453. Port: absent (core GET /composer_messages is not routed at all; src/routes/mod.rs).
- Crawler and page schema markup: topic_crawler_container_schema / main_entity_schema / post_schema / skip_post modifiers, the server:topic-main-entity-meta-crawler and server:topic-show-crawler-post-end builders, the QAPage JSON-LD in before-head-close and before-head-close-crawler (BuildSchemaMarkup, QuestionSchemaSerializer, AnswerSchemaSerializer, SchemaUtils), solved_add_schema_markup. Rails: plugins/discourse-solved/plugin.rb:113-164; app/services/discourse_solved/build_schema_markup.rb:1. Port: absent (no schema output in routes/topics.rs:675-699 or templates). [item 163]
- Admin dashboard Support section (register_admin_dashboard_section "support", AdminDashboardSupport.build, its categories setting). Rails: plugins/discourse-solved/plugin.rb:295; lib/discourse_solved/admin_dashboard_support.rb:21. Port: src/admin_dashboard.rs:290 (refusal "discourse-solved's support section settings"), :1350 (refusal "discourse-solved's support section"). [item 163]
- accepted_solutions related items computed without a viewer. Rails: plugins/discourse-solved/plugin.rb:218. Port: src/reports/counts.rs:1144 (refusal "accepted solutions' related items without a viewer"). [item 163]
- SeedAdminDashboardReports: the accepted_solutions AdminDashboardReport row seeded once solved is in use, and discourse_solved_admin_dashboard_seeded. Rails: plugins/discourse-solved/db/fixtures/700_admin_dashboard_reports.rb:1; lib/discourse_solved/seed_admin_dashboard_reports.rb:7. Port: absent.
- register_custom_filter_by_status solved/unsolved and the topics_filter_options entries (status:solved, status:unsolved) for /filter. Rails: plugins/discourse-solved/lib/discourse_solved/register_filters.rb:46-61. Port: absent (/filter not routed). [item 197, not named there]
- TopicQuery custom filter `solved=yes|no` on the topic lists. Rails: plugins/discourse-solved/lib/discourse_solved/register_filters.rb:66. Port: absent (src/routes/list.rs, src/topic_query.rs). [item 163]
- register_preloaded_category_custom_fields (enable_accepted_answers, notify_on_staff_accept_solved, empty_box_on_unsolved, enable_shared_issues) on /site.json and /categories.json categories. Rails: plugins/discourse-solved/plugin.rb:101-106. Port: src/categories.rs:311 (comment "only present when plugins register category fields"); parity/cases:16,44,201,222-224,242-243 ignore `/categories/*/custom_fields`.
- add_directory_column "solutions" drawn on the users page. Rails: plugins/discourse-solved/plugin.rb:451. Port: src/routes/directory.rs:200 (refusal "plugin columns in the user directory"; the JSON query exists in directory.rs:138).
- MCP tool discourse_solved_solution_set. Rails: plugins/discourse-solved/plugin.rb:491; lib/discourse_solved/mcp_tools.rb:5. Port: absent (no core MCP server).
- discourse-automation first_accepted_solution triggerable and the discourse-workflows SolutionChanged node; DiscourseEvent :accepted_solution / :unaccepted_solution are never triggered and an automation or workflow listening is not refused. Rails: plugins/discourse-solved/lib/discourse_automation/entry_point.rb:6; plugin.rb:46; app/services/discourse_solved/accept_answer.rb:189. Port: absent. [item 202]

**Routes and controllers**
- AcceptAnswer#publish_topic_reload: `{reload_topic: true}` on /topic/:id when the new solved row got an auto-close timer. Rails: plugins/discourse-solved/app/services/discourse_solved/accept_answer.rb:34,172. Port: absent (src/routes/solved.rs:167-177 publishes only the accepted answers).
- Accepting a post whose user is gone. Rails: plugins/discourse-solved/app/services/discourse_solved/accept_answer.rb:68. Port: src/plugins/solved/answers.rs:266 (refusal "accepting a post without a user").
- Accepting without a user behind the guardian. Port: src/plugins/solved/answers.rs:264 (refusal "accepting anonymously"). [item 133]
- by_user with a negative limit. Rails: plugins/discourse-solved/app/controllers/discourse_solved/solved_topics_controller.rb:145. Port: src/routes/solved.rs:243 (refusal "a negative by_user limit").
- by_user under content localization or non-ascii slugs. Port: src/plugins/solved/by_user.rs:47 [item 168], :50 [item 120].

**Settings**
- allow_solved_in_groups (group_list) and enable_solved_tags (tag_list) cannot be changed through the settings update. Rails: plugins/discourse-solved/config/settings.yml:15,61. Port: list types refused (site_setting_update.rs). [item 164]
- show_filter_by_solved_status, solved_add_schema_markup, disable_solved_education_message are read nowhere (see the filter, schema and composer message gaps).

**UI**
- bread-crumbs-right SolvedStatusFilter combo box (All / Solved / Unsolved, show_filter_by_solved_status, addDiscoveryQueryParam "solved"). Rails: plugins/discourse-solved/assets/javascripts/discourse/connectors/bread-crumbs-right/solved-status-filter.gjs:19; initializers/extend-for-solved-button.gjs:38. Port: absent. [item 163]
- Support dashboard components (support, response-time, topic-outcomes, whos-answering, support-headline). Rails: plugins/discourse-solved/admin/assets/javascripts/admin/components/dashboard/support.gjs:1. Port: absent. [item 163]
- SolvedAdminReportRelatedItems and SolvedAdminReportTableSummaryItem (the related solved topics under the accepted_solutions report). Rails: plugins/discourse-solved/assets/javascripts/discourse/components/solved-admin-report-related-items.gjs:1; initializers/solved-admin-report-related-items.gjs:1. Port: static/js/admin-report.js:1607 (refusal "discourse-solved's related solved topics").
- show_who_marked_solved tooltip on the post's Solved button. Rails: plugins/discourse-solved/assets/javascripts/discourse/components/solved-unaccept-answer-button.gjs:1. Port: src/plugins/solved/view.rs:9 (not drawn).
- Confetti on solution toggle (NoAnswer) and on Me too. Rails: plugins/discourse-solved/assets/javascripts/discourse/connectors/topic-navigation/no-answer.gjs:84; components/solved-shared-issue-button.gjs. Port: static/js/discourse-solved.js:7 (not ported).
- replaceIcon notification.solved.accepted_notification and notification.solved.topic_solved_notification to square-check in the user menu and notifications page. Rails: plugins/discourse-solved/assets/javascripts/discourse/initializers/extend-for-solved-button.gjs:180-187. Port: src/user_menu_view.rs:580-613 (no entry; the custom item's icon stays `notification.solved.*`, user_menu_view.rs:749).
- notifications-tracking-description transformer (solved's tracking and watching descriptions on topics with solved enabled). Rails: plugins/discourse-solved/assets/javascripts/discourse/initializers/extend-for-solved-button.gjs:52. Port: absent (named as left in done item 98).
- SolvedRemovalConfirmation modal on a topic edit that takes the topic out of solved, its "don't show again" key, and the post stream refresh after a solved state change. Rails: plugins/discourse-solved/assets/javascripts/discourse/initializers/solved-category-change-confirmation.gjs:30; components/modal/solved-removal-confirmation.gjs. Port: absent.
- MoveSolutionConfirmation modal before MoveToTopic with an accepted answer selected. Rails: plugins/discourse-solved/assets/javascripts/discourse/pre-initializers/solved-move-confirmation.js:12; components/modal/move-solution-confirmation.gjs. Port: absent (the port has no move-to-topic modal either).

**Locales**
- No gap in loading; server `education.topic_is_solved` is unused until the composer message exists.

**Markdown**
- None in the reference plugin.

**Emails and notifications**
- Icon and tracking description gaps above; the custom notifications themselves are created and drawn.

**Jobs, Models, Serializers**
- No jobs in the reference plugin. Tables and columns used are the Rails ones. Serializer gaps are the preloaded category custom fields above.

### Items

New (ROADMAP Phase 11): 233 `feat/solved-lifecycle` M, 234 `feat/solved-ui-rest` M, 235 `feat/solved-category-fields` S (shared with topic voting), the education message in 213 `feat/composer-messages` M, the MCP tool in 212 `feat/mcp-server` L. Earlier items: 163 (schema markup, the support section, lists by `solved=`, related items without a viewer; trimmed of the web hooks, auto close timers and search filters, which are done), 197 (now naming status:solved and status:unsolved), 120, 133, 164, 168, 202.

### Couldn't determine
- user_action_stream_builder modifier (t.deleted_at IS NULL, no PMs): the port's UserAction.stream always has both conditions (users.rs:1774-1778); whether core without solved differs for any viewer needs Rails.
- The routes/solved.rs:6 divergence: an anonymous accept is a 500 in Rails (limit_accepts on nil) and a 403 here; whether parity wants the 500 is a call for Jake.
- Whether the Solved badges' PostRevision trigger fires for accept/unaccept in Rails (they are granted by backfill), and so whether the port's granter misses grants between backfills; needs Rails.
- DiscourseDev::DiscourseSolved.populate is development seeding only; not counted.

## discourse-topic-voting (`plugins/discourse-topic-voting`)

Verdict: started (16 gaps)

### What the port has
- Enabled check, Category.can_vote? (category settings rows, definition topic excluded), list data (counts, current_user_voted), list item keys vote_count / can_vote / user_voted, topic view keys, first post can_vote, basic category can_vote, current user votes_exceeded / votes_count / votes_left / vote_limit, UserExtension limits and alert_low_votes?: src/plugins/topic_voting/mod.rs; topic_list.rs:381-400; topic_view.rs:616,1964,2003; categories.rs:400; current_user.rs:879.
- Votes::Cast and Votes::Remove (visibility, votable, already voted, out of votes 403 with alert, count refresh, BackfillBadges, topic_upvote / topic_unvote web hooks), VotesController#who with the limit clamp: src/plugins/topic_voting/votes.rs; src/routes/topic_voting.rs:33-158; src/web_hooks/mod.rs:504-537; routes/mod.rs:472-477.
- ListController#voted_by JSON and its not-found HTML page (routes/topic_voting.rs:163-263), the votes list and order=votes / state=my_votes, list_hot by trending score in voting categories (topic_query.rs:636-805; routes/list.rs:939; routes/tags.rs:283), votes in top menu / filters (site.rs:126; setting_enums.rs:170).
- Search min_vote_count: and order:votes (143d, 143e; search/posts_query.rs:399), the menu suggestion (search_menu_view.rs:1072).
- Events and jobs: topic_status_updated, topic_trashed, post_edited category change, VoteRelease (votes_released notifications unless trashed), VoteReclaim, BackfillBadges, badge_granter_suppress_notification for the voting badges, user destroy, UserMerger: lifecycle.rs; plugins/mod.rs:154-195,238; jobs/handlers.rs:114-160,480-530; badge_granter.rs:221; user_merger.rs:715-750; topic_recovered on a recovered definition (user_destroyer.rs:1234).
- Ideas category type (configure/unconfigure, archive and unarchive votes), category destroy archiving votes: category_types.rs:133,318-345; category_type_config.rs:94,213-301 (190d, 190e).
- UI: VoteBox beside the title (button, menu with votes left, remove, watch toggle, locked and closed states, voters menu, anonymous login link), list vote count tag, voting-post class, hot nav help title, votes and my_votes nav items, user activity Votes tab and empty states, votes_released notification renderer: plugins/topic_voting/view.rs; html.rs:1475-1481,2043-2060; post_view.rs:459; topic_list_view.rs:483; user_profile_view.rs:650,832; user_menu_view.rs:1031-1040; static/js/topic-voting.js.
- Locales: plugin yml merged and loaded (i18n.rs:26-28). Voting badges are rows in seed/fresh_install.sql.

### Gaps

**plugin.rb**
- `on(:topic_merged)` TopicMerger (move a closed source's votes, drop duplicates, recount, backfill badges, append votes_moved to the split_topic post). Rails: plugins/discourse-topic-voting/lib/discourse_topic_voting/topic_merger.rb:13. Port: src/post_mover.rs:2083 (refusal "topic voting's merge of a topic's votes"). [item 188i4]
- `on(:topic_recovered)` VoteReclaim on a recovered open topic. Rails: plugins/discourse-topic-voting/plugin.rb:208. Port: src/post_destroyer.rs:1281 (refusal "recovering topics"; lifecycle::topic_recovered is wired only for user_destroyer.rs:1236). [item 150]
- add_filter_custom_filter votes-min, votes-max, order:votes and the topics_filter_options entries. Rails: plugins/discourse-topic-voting/plugin.rb:129-184; lib/discourse_topic_voting/topic_votes_filter.rb:6. Port: absent (/filter not routed). [item 197]
- GET c/*category_slug_path_with_id/l/votes.rss (ListController#votes_feed). Rails: plugins/discourse-topic-voting/plugin.rb:233; lib/discourse_topic_voting/list_controller_extension.rb:23. Port: absent. [item 195]
- register_anonymous_action "vote_topic" (a vote cast after login) and the button's deferAnonymousAction; an anonymous click on a closed or archived topic does nothing in Ember. Rails: plugins/discourse-topic-voting/plugin.rb:247; assets/javascripts/discourse/components/vote-button.gjs:79-86. Port: src/session/login.rs:26 (refusal "replaying an anonymous action after a login"); src/plugins/topic_voting/view.rs:84-90 always links to login.
- discourse-workflows TopicReceivedVote node on topic_voting_vote_created. Rails: plugins/discourse-topic-voting/plugin.rb:37; app/services/discourse_topic_voting/votes/cast.rb:79. Port: src/plugins/topic_voting/votes.rs:275 (refusal "discourse-workflows triggers on votes"). [item 202]
- MCP tool discourse_topic_voting_vote_set. Rails: plugins/discourse-topic-voting/plugin.rb:257; lib/discourse_topic_voting/mcp_tools.rb. Port: absent (no core MCP server).
- register_preloaded_category_custom_fields enable_topic_voting on /site.json and /categories.json. Rails: plugins/discourse-topic-voting/plugin.rb:94. Port: src/categories.rs:311; parity/cases:16,222-224 ignore `/categories/*/custom_fields`.
- Voting through an anonymous-posting shadow. Port: src/plugins/topic_voting/votes.rs:126,152,175,218 (refusal "voting anonymously"). [item 133]

**Routes and controllers**
- Votes::Cast / Remove contract: a non-integer topic_id (e.g. `abc`) is a 400 with "Topic can't be blank" in Rails; the port takes to_i and answers 404. Rails: plugins/discourse-topic-voting/app/services/discourse_topic_voting/votes/cast.rb:8. Port: src/routes/topic_voting.rs:86-90 (no integer check, unlike routes/solved.rs:97-101).
- voted_by with `period`. Rails: plugins/discourse-topic-voting/lib/discourse_topic_voting/list_controller_extension.rb:11. Port: src/routes/topic_voting.rs:210 (refusal "period on the voted_by list").

**UI**
- header-topic-info__before connector: the vote box in the docked topic header. Rails: plugins/discourse-topic-voting/assets/javascripts/discourse/connectors/header-topic-info__before/header-topic-voting.gjs:1. Port: absent (core's docked header topic info is not ported either; no item names it, only the milestone 5 bullet).
- Full-page search sort order "Most Votes" (sortOrders push, term order:votes). Rails: plugins/discourse-topic-voting/assets/javascripts/discourse/initializers/discourse-topic-voting.js:57-64. Port: src/search_view.rs:45 and static/js/search-page.js:16 (core's six orders only).
- category-available-views "votes" and addCategorySortCriteria("votes") in the category editor. Rails: plugins/discourse-topic-voting/assets/javascripts/discourse/initializers/discourse-topic-voting.js:95; pre-initializers/extend-category-for-voting.js:72. Port: src/category_edit_view.rs:307-308 (sort_order and default_view are free text inputs, no choices).
- replaceIcon("topic_voting.voting_closed", "lock"): the port maps it to far-envelope. Rails: plugins/discourse-topic-voting/assets/javascripts/discourse/initializers/discourse-topic-voting.js:19. Port: src/user_menu_view.rs:611.
- VoteCount's voters list as a modal on mobile (`@modalForMobile`). Rails: plugins/discourse-topic-voting/assets/javascripts/discourse/components/vote-count.gjs:89. Port: static/js/topic-voting.js (menu only).

**Settings**
- No gap found: every setting is read; depends_on/depends_behavior are handled by the admin settings code (admin_site_settings.rs).

**Jobs, Models, Serializers, Locales, Markdown, Emails and notifications**
- Jobs ported (VoteRelease, VoteReclaim, BackfillBadges). Tables are the Rails ones. Serializer gap is the preloaded custom field above. Locales loaded. No markdown in the reference plugin. votes_released is drawn; no emails in the reference plugin.

### Items

New (ROADMAP Phase 11): 236 `feat/topic-voting-ui-rest` M, 237 `feat/topic-voting-rest` S, `vote_topic` in 211 `feat/anonymous-actions` M, enable_topic_voting in 235 `feat/solved-category-fields` S, the MCP tool in 212 `feat/mcp-server` L. Earlier items: 133, 150, 188i4, 195, 197, 202. Milestone 5's voting list was stale (web hooks, min_vote_count, order:votes, the category toggle and edits are done, 143d, 143e, 190c-190f) and is replaced by a pointer here.

### Couldn't determine
- The voting_closed icon: the port's far-envelope may come from an older vendored Discourse; vendor/discourse/DISCOURSE_REF vs the reference checkout would settle it.
- Whether Rails emails votes_released notifications (no plugin code does; core's per-type email rules would need a run).
- prev_topics_url on voted_by: Rails sets it on the list (list_controller_extension.rb:15); whether TopicListSerializer emits it for this list needs a Rails response to compare.

## discourse-post-voting (`plugins/discourse-post-voting`)

Verdict: started (11 gaps). Ported 2026-10-28 on `feat/plugin-post-voting`.

### What the port has
- Enabled check, `Topic#is_post_voting?` (the `question_answer` subtype, not in a nested view), `Post#is_post_voting_topic?` (refused for a trashed topic, as below): src/plugins/post_voting/mod.rs.
- The topic view: the custom default scope (answers only; without `filter=activity` no whispers or small actions, ordered by votes after the question), applied to the stream, the pages, the gaps and the posts by number; `filter_posts_near` by row in the vote order; `next_page` and `highest_post_number` from the last post of the order; `filter=activity` on topics#show and topics#posts; posts by id left alone. The before query phase is `plugins::topic_view_post_scope` (src/plugins/mod.rs), applied by src/topic_view.rs (PostFilter's scope).
- `TopicView.on_preload` (each post's first five comments, comment counts, the viewer's votes on posts and comments, the posts voted on) and PostSerializerExtension's keys (`post_voting_vote_count`, `post_voting_user_voted_direction`, `post_voting_has_votes`, `comments`, `comments_count`; outside a topic view and on web hook posts the bare keys; a trashed topic's posts refused while the plugin is on, as `topic.is_post_voting?` on its nil topic raises at the reference); TopicViewSerializerExtension's keys (`is_post_voting`, `last_answered_at`, `last_commented_on`, `answer_count`, `last_answer_post_number`, `last_answerer`, a deleted last answerer a 404 as `User.find` raises); TopicListItemSerializer's `is_post_voting`; the user card's `vote_count`; the current user's `can_flag_post_voting_comments`; BasicCategorySerializer's `create_as_post_voting_default` and `only_post_voting_in_this_category`.
- VoteManager and VotesController: POST and DELETE /post_voting/vote, GET /post_voting/voters, POST and DELETE /post_voting/vote/comment, with ensure_can_vote's messages, the undo window, PostVotingVote's validations (422 record_invalid), `post_voting_post_voted` on the topic's channel, the destroy's any-vote-in-the-topic check and a second comment upvote failing the unique index: src/plugins/post_voting/votes.rs, src/routes/post_voting.rs.
- PostVotingComment and CommentsController: GET, POST, PUT and DELETE /post_voting/comments (load more, create through CommentCreator, edit, trash), the model's validations in order (presence, ensure_can_comment, stripped length, TextSentinel, watched words), cooking with only emphasis, backticks, linkify and link plus emoji and censored (render_post_voting_comment in crates/markdown, pretty_text::cook_post_voting_comment), the raw's newlines run together, `post_voting_post_commented`, `_comment_edited` and `_comment_trashed`, the question_answer_user_commented notification with its live alert and push check: src/plugins/post_voting/comments.rs.
- Posting: `create_as_post_voting` (the NewPostManager handler) on posts#create, `subtype` in a queued post's payload and on its approval, PostExtension's `ensure_only_replies` on create, the emailed reply to the question made an answer: src/routes/posts.rs, src/posting/create.rs, src/posting/enqueue.rs, src/review_posts.rs, src/plugins/post_voting/lifecycle.rs.
- The user destroyer's content deletion callback (comments, their reviewables, `bulk_remove_votes_by`), while the plugin is on: src/plugins/mod.rs user_content_deleting.
- PUT /post_voting/comments/flag up to the reviewable: login, the 4 a minute rate limit, the comment, `flag_type_id`, `can_flag_post_voting_comment?` and `_as?`.
- UI: PostVotingVoteControls after the avatar (optimistic votes, the voters menu split up and down), PostVotingAnswerHeader before the stream's first answer (answer count, Votes and Activity), PostVotingComments after the post menu (each comment's upvote, edit, delete, the comments menu's Add a comment with its composer and Show more), the Answer button in Reply's place and no likes on answers unless post_voting_enable_likes_on_answers, the `post-voting-topic` and `--sort-by-activity` body classes, the notification's renderer (comment icon, the comment's anchor) and alert verb, the category editor's Post Voting section (src/category_edit_view.rs, static/js/category-edit.js sending the custom fields whole), the last unread link into a post voting topic (src/topic_list_view.rs), the stylesheets: src/plugins/post_voting/view.rs, src/post_view.rs, src/html.rs, src/user_menu_view.rs, static/js/post-voting.js, static/css/post-voting.css. A live redraw of a post keeps the comments loaded in the page and the answers header.
- Settings and locales: the vendored plugin settings and locale files (already loaded).

### Gaps

**Server**
- Flagging a comment: CommentReviewQueue#flag_comment (ReviewablePostVotingComment.needs_review!, add_score with the queued-by-staff reason and force_review, the companion message for notify_user and notify_moderators, take_action's agree_and_delete, the auto silence on chat_auto_silence_from_flags_duration). Rails: plugins/discourse-post-voting/lib/post_voting/comment_review_queue.rb:4. Port: src/plugins/post_voting/comments.rs flag (refusal "flagging post voting comments (plugin reviewables in the review queue)").
- ReviewablePostVotingComment in the review queue: its serializer, its action bundles and every perform (agree_and_keep_comment, agree_and_restore, agree_and_delete, disagree, disagree_and_restore, ignore, delete_and_ignore, and the aliases: agree_and_keep_hidden, agree_and_silence, agree_and_suspend and delete_and_agree to agree_and_delete, so silencing or suspending from it only deletes the comment at the reference). Rails: plugins/discourse-post-voting/app/models/reviewable_post_voting_comment.rb. Port: src/review_list.rs (refusal "plugin reviewables in the queue"), src/review.rs (refusal "reviewable types other than flagged posts"); chat's ReviewableChatMessage waits on the same core piece.
- `register_anonymous_action "vote_post"`: a vote cast after login. Rails: plugins/discourse-post-voting/plugin.rb:275. Port: core has no anonymous actions (src/session/login.rs, item 211); static/js/post-voting.js sends a visitor to log in.
- `topic_embed_import_create_args` modifier (an embedded topic in a post voting category). Rails: plugin.rb:229. Port: TopicEmbed import isn't ported.
- TopicExtension's `ensure_regular_topic` and `ensure_no_post_voting_subtype`: unreachable in the port, which takes no `subtype` param (only the handler sets it, while the plugin is on) and changes no topic's subtype. Rails: plugins/discourse-post-voting/extensions/topic_extension.rb:79-96.
- Post `has_many :post_voting_comments, dependent: :destroy` and the votes' `delete_all` on a post's destroy: the port has no permanent post deletion yet (src/post_destroyer.rs).

**UI**
- The composer: CREATE_TOPIC's "Toggle post voting" composer action, `createAsPostVoting` defaulting from the category (create_as_post_voting_default, only_post_voting_in_this_category) and serialized on create, the action title and save label ("Create post voting topic", "Answer"). Rails: plugins/discourse-post-voting/assets/javascripts/discourse/initializers/extend-composer-actions.js. Port: static/js/composer.js has no composer actions menu yet.
- The flag modal on a comment (PostVotingFlag). Rails: lib/post-voting-flag.js, components/post-voting-comment-actions.gjs. Port: the button is drawn, static/js/post-voting.js sends nothing.
- The reviewable's component (components/reviewable/post-voting-comment.gjs, `post-voting-reviewables` initializer): waits on the queue above.
- A post redrawn live while the page is sorted by activity: the redraw cannot know the page's sort (the header is kept as drawn).

**Settings, Locales, Markdown, Emails**
- No gap found: the settings are read (post_voting_comment_max_raw_length and min_post_length by the composer), the locales load, the comment cooking is ported, and the plugin sends no email of its own (the notification's email is core's).

**Crawler**
- `server:topic-show-crawler-post-end` (crawler_post.html.erb: comments with schema.org Comment markup, the answer count): the port serves crawlers the server-rendered topic page, which draws the comments, without that markup.

### Items

None yet: the reviewables (with chat's ReviewableChatMessage) and the composer actions are core pieces to name in ROADMAP.

### Couldn't determine
- Read at the vendored bf55a44c (the extract in tmp/rails-bf55a44c), after a first pass on the reference checkout's HEAD (167479b1). The plugin differs there in its reviewable (no penalty args or DSA attributes, silence and suspend aliased to delete) and in `Post#is_post_voting_topic?` (`topic.is_post_voting?` without the safe navigation, so a trashed topic's posts raise). Rails raises for those whether or not the plugin is on; the port refuses them only while it is on, so a site without post voting keeps showing staff its trashed topics.
- Whether `highest_post_number` (the last post of the vote order, from post voting's `next_page`) is what Rails serializes when topics#show is cut short by `replies_to_post_number` and its kin, which the port doesn't take.

## The small plugins: shared context

A note on event gating that applies to several plugins below: Rails' `Plugin::Instance#on` wraps every handler in `if enabled?` (lib/plugin/instance.rb:692-693), so a plugin's `reduce_cooked`, `reduce_excerpt` and `before_post_process_cooked` handlers only run while its `enabled_site_setting` is on. Presence channel prefixes are a filtered register too (lib/discourse_plugin_registry.rb:110). Core registers no presence prefixes of its own: `PresenceChannel.register_prefix` is called only through plugins (discourse-presence and chat).

## discourse-presence (`plugins/discourse-presence`)

Verdict: started (3 gaps)

### What the port has
- Channel configs for all four prefixes (`reply`, `whisper` with the shared-draft branch, `edit` with locked/whisper/wiki/edit_all/category moderation, `translate`), matching plugin.rb's rules: src/plugins/presence/mod.rs:147-480.
- Registered under the `discourse-presence` prefix and gated by `presence_enabled`: src/presence.rs:98-100.
- ComposerPresenceDisplay (`before-composer-controls` outlet), TopicPresenceDisplay (`topic-above-footer-buttons` outlet, members only, `--avatar-min-height: 24px`) and ComposerPresenceManager (enter while replyDirty, KEEP_ALIVE 10s, hide_presence respected): static/js/topic-presence.js; outlets in src/composer_view.rs:476-482 and src/post_view.rs:2046-2050; composer state from static/js/composer.js:538-552.
- Settings `presence_enabled`, `presence_max_users_shown` (slice in the composer display) and the strings: src/html.rs:1070-1092.
- Admin plugin entry with the `eye` icon: src/admin_view.rs:757-763.
- Core presence client and /presence/get, /presence/update: static/js/presence.js (SPEC_COVERAGE lists both controller actions covered).

### Gaps
**UI**
- The `whisper` composer state: a whisperer writing a whisper should announce on `/discourse-presence/whisper/<topic>` and read the reply and whisper channels; the port's composer has no whisper toggle, so a whisper is announced on the reply channel. Rails: plugins/discourse-presence/assets/javascripts/discourse/components/composer-presence-display.gjs:109-120,129-134. Port: static/js/topic-presence.js:139 ("the port's composer has no whisper toggle"). [item 196]
- The `translate` state: ComposerPresenceDisplay subscribes `/discourse-presence/translate/<post>` while the composer's action is `Composer.ADD_TRANSLATION`, shows "translating" and enters that channel while dirty. The port's composer has no add-translation action and composerState only knows `edit` and `reply`, so the server-side translate channel is never used. Rails: plugins/discourse-presence/assets/javascripts/discourse/components/composer-presence-display.gjs:68-81,103-120,163-167. Port: static/js/topic-presence.js:132-141,171-179 | absent.

**Locales**
- `js.presence.translating.one/other` are not passed to the client (the composer data carries replying, editing and replying_to_topic only). Rails: plugins/discourse-presence/config/locales/client.en.yml (presence.translating). Port: src/html.rs:1078-1084 | absent.

### Items

New (ROADMAP Phase 11): 238 `feat/presence-translate` S, after 168. The whisper state follows 196.

### Couldn't determine
- Nothing beyond the above; plugin.rb's channel rules were compared line by line.

## discourse-local-dates (`plugins/discourse-local-dates`)

Verdict: started (5 gaps)

### What the port has
- Markdown `[date]`, `[date-range]` with timezone filtering, IST/KST/JST links, `data-email-preview` from `discourse_local_dates_email_format` and `_email_timezone`: crates/markdown/src/render/local_dates.rs.
- `before_post_process_cooked`: the `local_dates` post custom field (dates outside asides, escaped, "undefined" dropped, save_custom_fields semantics), gated by the enabled setting: src/plugins/local_dates.rs:84-150, called from src/jobs/handlers.rs:840-843.
- `reduce_cooked` for emails (text becomes data-email-preview), gated: src/plugins/local_dates.rs:158-216, src/email/notification.rs:852-856.
- `reduce_excerpt` " (UTC)", gated: src/excerpt.rs:247-254.
- Client: applyLocalDates in posts, composer preview and chat, ranges and same-day, tooltip previews, Add to calendar with ics and the download-calendar modal, data-title default: static/js/local-dates.js.
- Insert date modal (from/to, Pikaday, timezone combo, advanced mode with recurrence, timezones, format list from `discourse_local_dates_default_formats`, preview) and Ctrl/Cmd+Shift+. current-date shortcut: static/js/local-dates-create.js; options menu item with `far-clock` and always-visible shortcut: src/composer_view.rs:264-271; static/js/composer.js:1612-1628.
- Admin plugin entry with `far-clock`: src/admin_view.rs:674-679.

### Gaps
**Markdown**
- Dates the port refuses to cook that Rails cooks: times that are not `HH:mm[:ss]` (e.g. `9:00`, read by moment through Date) (crates/markdown/src/render/local_dates.rs:134,141), offsets moment would not read (:164), a year past 9999 (:67), a `[date]` with no date at all (:585), era tokens (`N`, `y`) in a format (:528). Rails: plugins/discourse-local-dates/assets/javascripts/lib/discourse-markdown/discourse-local-dates.js:4-60. Port: refusals as cited | absent.
- Email preview refusals: a `discourse_local_dates_email_timezone` that chrono-tz does not know (:590) and a zone without an abbreviation (:601). Rails: plugins/discourse-local-dates/assets/javascripts/lib/discourse-markdown/discourse-local-dates.js:43-48. Port: crates/markdown/src/render/local_dates.rs:590,601 (refusals).

**UI**
- The rich editor extension and LocalDateNodeView (dates as nodes in ProseMirror, editing a date from the rich editor). Rails: plugins/discourse-local-dates/assets/javascripts/lib/rich-editor-extension.js, lib/local-date-node-view.js. Port: absent. [item 196]
- The `apply-local-dates` behavior transformer that other plugins call (`discourse/lib/local-dates`). Rails: plugins/discourse-local-dates/assets/javascripts/initializers/discourse-local-dates.js:135-137. Port: static/js/local-dates.js exposes `Discourse.applyLocalDates` for posts, preview and chat; no other caller exists in the port yet, so this is only a gap once discourse-calendar or similar arrive. [item 202]

**Locales**
- Moment locales other than English for the request (Accept-Language), and the plugin's non-English client strings. Rails: core JsLocaleHelper with plugins/discourse-local-dates/config/locales/client.*.yml. Port: English only (src/i18n.rs:22-31). [item 167]

### Items

New (ROADMAP Phase 11): 239 `feat/local-dates-cook-rest` S; the excerpt's " (UTC)" gate in 244 `feat/spoiler-email` S. Earlier items: 167, 196.

### Couldn't determine
- Whether server-side cooking in Rails uses a moment locale other than English when `default_locale` is not `en` (the port cooks in English); needs a Rails run.
- Migration 20250529021721_add_utc_to_setting appends " z" to a stored `discourse_local_dates_email_format`; restored databases already carry it, but whether the port's own schema setup ever needs it was not checked.

## discourse-lazy-videos (`plugins/discourse-lazy-videos`)

Verdict: started (5 gaps)

### What the port has
- The YouTube, Vimeo and TikTok onebox overrides (`to_html` with the maxresdefault thumbnail check and fallback, unlisted Vimeo ids from the iframe, TikTok's embed_product_id), gated by `lazy_videos_enabled` and the per-provider settings: src/plugins/lazy_videos.rs:9-49; src/onebox/embed_engines.rs:1324-1460; src/onebox/engines.rs:45-71.
- Admin plugin entry with the `bed` icon: src/admin_view.rs:667-672.

### Gaps
**UI**
- The cooked decorator and its components: each `.lazy-video-container` in the stream (not the composer preview) becomes LazyVideo: a thumbnail button (role=button, tabindex, `aria-label` "Play %{title}", dominant color background, provider play icon) and a title bar link; click, Enter or Space swaps in LazyIframe (`youtube.com/embed/<id>?autoplay=1&rel=0` with `list` and `start` converted from `1h2m3s`, `player.vimeo.com/video/<id>?autoplay=1`, `tiktok.com/embed/v2/<id>`), only when `lazy_<provider>_enabled`, and calls preventCloak for the post. Rails: plugins/discourse-lazy-videos/assets/javascripts/initializers/lazy-videos.gjs:5-40; discourse/components/lazy-video.gjs; discourse/components/lazy-iframe.gjs; lib/lazy-video-attributes.js. Port: absent (no static/js file handles `.lazy-video-container`; the thumbnail is a plain link to the provider).
- The stylesheet. Rails: plugins/discourse-lazy-videos/assets/stylesheets/lazy-videos.scss. Port: absent (src/assets.rs:67-73 lists the other plugins' css, no lazy-videos.css).

**Emails and notifications**
- `reduce_cooked`: each `.lazy-video-container` replaced by `<p><a href=video link>title</a></p>` in emails. Rails: plugins/discourse-lazy-videos/plugin.rb:27-39. Port: src/email/notification.rs:820-856 runs details' and local dates' handlers only | absent.

**plugin.rb**
- `register_html_builder("server:topic-show-crawler-post-end")`: CrawlerPostEnd's schema.org `VideoObject` JSON-LD per video (name, embedUrl, url, uploadDate, description from a 200-char excerpt, thumbnailUrl, contentUrl, `</script` escaped). Rails: plugins/discourse-lazy-videos/plugin.rb:23-25; lib/discourse_lazy_videos/crawler_post_end.rb. Port: absent, and the core crawler topic view with post bodies it hangs off is not ported either (no crawler post markup in src/ or templates/).

**Locales**
- `js.lazy_videos.play_video` is unused (comes with the component above). Rails: plugins/discourse-lazy-videos/config/locales/client.en.yml. Port: absent.

### Items

New (ROADMAP Phase 11): 240 `feat/lazy-videos-ui` S, 241 `feat/lazy-videos-crawler` S.

### Couldn't determine
- Migration 20230317194217_rebake_lazy_yt_posts only resets baked_version for old lazyYT posts; whether any restored data still needs it is a data question, not a code one.
- Chat's collapser for lazy videos (`.chat-message-collapser-lazy-video`) belongs to chat's audit.

## checklist (`plugins/checklist`)

Verdict: started (6 gaps)

### What the port has
- Markdown: `[ ]`, `[x]`, `[X]`, `[]` as `span.chcklst-box` with classes and `data-chk-src="line:nth"`, permanent `[X]`, raw markup guard, allow list, gated by `checklist_enabled`: crates/markdown/src/render/checklist.rs; src/pretty_text/mod.rs:545.
- Stylesheet: static/css/checklist.css (src/assets.rs:69).

### Gaps
**Routes and controllers**
- `PUT /checklist/toggle` (checkboxes#toggle): requires_plugin and login, 60 per minute rate limit, the ToggleCheckbox contract (post_id, up to 50 toggles each with checkbox_index, checkbox_count or checkbox_source `\d+:\d+`, checked; expected_raw up to max_post_length, expected_updated_at, mutation_id up to 100; unique targets), policies (checklist_enabled, can_edit, post_unchanged by raw and millisecond updated_at, counts unchanged, boxes found, toggleable), the revise with bypass_bump, expected_raw and preserve_cooked_token, and the responses (200 with cooked, raw, revised, updated_at, version, last_editor_id; 400 contract errors; 404; 403; 409 `checklist.checkboxes_changed` with raw, updated_at and `retryable`; 422 `checklist.checkbox_locked` / `checklist.revision_failed`). Rails: plugins/checklist/config/routes.rb:3; app/controllers/checklist/checkboxes_controller.rb:11-77; app/services/checklist/toggle_checkbox.rb; lib/checklist/checkbox_locator.rb, source_map.rb, checkbox.rb. Port: absent (no route in src/routes/mod.rs).
- PostRevisor's `preserve_cooked_token` option, which the toggle passes so the client that already drew the change keeps its cooked. Rails: lib/post_revisor.rb:287,894-895. Port: absent (no `preserve_cooked_token` in src/).

**UI**
- The `checklistSyntax` stream decorator: boxes editable for `can_edit` (is-interactive, role checkbox, aria-checked, aria-readonly for permanent ones, keyboard activation), list classes (addUlClasses), boxes inside sourced quotes left alone, debounced batched saves (150ms, 50 per batch, 30s timeout), conflict handling with up to two retries and a fresh `/posts/<id>` baseline, optimistic post updates (registerOptimisticPostUpdate) and popupAjaxError. Rails: plugins/checklist/assets/javascripts/discourse/initializers/checklist.js:52-57,92-900. Port: absent (no static/js file mentions chcklst).
- The composer's list menu item `list-checklist` (`list-check`, `checklist.composer.checklist`, applyList "- [ ] "). Rails: plugins/checklist/assets/javascripts/discourse/initializers/checklist.js:59-73. Port: absent; the toolbar's list menu is not ported. [item 196]
- The rich editor extension (toggleChecklist, inCheckList). Rails: plugins/checklist/assets/javascripts/lib/rich-editor-extension.js. Port: absent. [item 196]

**Locales**
- Server `checklist.checkboxes_changed`, `checklist.checkbox_locked`, `checklist.revision_failed` and client `checklist.item` (the box's fallback label) are unused. Rails: plugins/checklist/config/locales/server.en.yml, client.en.yml. Port: absent.

### Items

New (ROADMAP Phase 11): 242 `feat/checklist-toggle` M. Earlier items: 196 (the list menu item, the rich editor).

### Couldn't determine
- Nothing further from source.

## discourse-details (`plugins/discourse-details`)

Verdict: started (2 gaps)

### What the port has
- Markdown `[details]` with `open`, summary, the allow list: crates/markdown/src/render/bbcode.rs:261 (cooks byte-equal on the corpus per ROADMAP milestone 4).
- Email::Styles plugin style (elided details removed, details replaced by their summary as `p`): src/email/styles.rs:378-404.
- `reduce_cooked` (summary text plus "(click for more details)" link to the post or the site): src/email/notification.rs:833-851.
- Options menu item (`angle-right`, `details.title`, Shift+D, applySurround with `composer.details_title` and `details_text`): src/composer_view.rs:250-256; static/js/composer.js:1598-1605.
- Stylesheet: static/css/discourse-details.css.

### Gaps
**Emails and notifications**
- `reduce_cooked` runs whether or not `details_enabled` is on; Rails' `on` skips it when the plugin is off (the Email::Styles hook is ungated in Rails too, so that one is right). Rails: plugins/discourse-details/plugin.rb:33-43; lib/plugin/instance.rb:692-693. Port: src/email/notification.rs:833 (ungated).

**UI**
- The rich editor extension (details as a ProseMirror node). Rails: plugins/discourse-details/assets/javascripts/lib/rich-editor-extension.js; initializers/apply-details.js:21. Port: absent. [item 196]

### Items

New (ROADMAP Phase 11): the email handler's enabled gate in 244 `feat/spoiler-email` S. Earlier items: 196.

### Couldn't determine
- Whether `[details]` stops cooking when `details_enabled` is off: the markdown file registers no feature option (details.js:23-35), so it depends on whether PrettyText loads a disabled plugin's markdown; needs a Rails run. The port always cooks it outside chat (bbcode.rs:261).

## footnote (`plugins/footnote`)

Verdict: started (1 gap, left to 196)

### What the port has
- markdown-it-footnote port: inline `^[...]`, labelled refs, the list, allow list with the `fn`/`fnref` id rule, gated by `enable_markdown_footnotes`: crates/markdown/src/render/footnotes.rs; src/pretty_text/mod.rs:546.
- `before_post_process_cooked` (243): back references, references and list items renamed to `footnote-ref-<post>-<n>` / `footnote-<post>-<n>` with the plugin's slicing and `to_i`, gated by `enable_markdown_footnotes`, run inside the post processor on its document: src/plugins/footnote.rs; src/pretty_text/cooked_post_processor.rs. Recorded against Rails: parity/writes/footnote/edit_post_scoped_ids.
- Inline footnotes (DTooltip `inline-footnote`, `.inline-footnotes`), loaded when the plugin and `display_footnotes_inline` are on: static/js/inline-footnotes.js; src/html.rs:743-744.
- Options menu item (`asterisk`, `footnote.add`, inserts `^[` + `footnote.title` + `]`, unnamed): src/composer_view.rs:257-263,632; static/js/composer.js:1606-1608.
- Admin plugin entry with `asterisk`: src/admin_view.rs:646-651. Stylesheet: static/css/footnote.css.

### Gaps
**UI**
- The rich editor extension (footnotes as nodes). Rails: plugins/footnote/assets/javascripts/lib/rich-editor-extension.js; initializers/composer.js:10. Port: absent. [item 196]

### Items

New (ROADMAP Phase 11): 243 `feat/footnote-post-process` S (done). Earlier items: 196.

### Couldn't determine
- The popup option's `group: "insertions"` (initializers/composer.js:16): whether the reference's options menu renders groups differently from the port's flat list was not settled from source.

## spoiler-alert (`plugins/spoiler-alert`)

Verdict: started (2 gaps)

### What the port has
- Markdown `[spoiler]` inline and block, gated by `spoiler_enabled`: crates/markdown/src/render/bbcode.rs:262,548,849.
- The decorator (`.spoiler` to `.spoiled`, blurred state, aria attributes, `spoiler.label.show`, click and Enter toggling, interactive-element and selection checks): static/js/spoiler-alert.js.
- Options menu item (`wand-magic`, `spoiler.title`, show-active icon, applySurround in block mode with `composer.spoiler_text`): src/composer_view.rs:272-279; static/js/composer.js:1609-1611.
- `reduce_excerpt` removes `.spoiler`, gated by `spoiler_enabled`: src/excerpt.rs:233-237. Stylesheet: static/css/spoiler-alert.css.

### Gaps
**Emails and notifications**
- `reduce_cooked`: each `.spoiler`'s content replaced by a link to the post reading "spoiler" (`spoiler_alert.excerpt_spoiler`). The port's emails carry the spoiler's text in the clear. Rails: plugins/spoiler-alert/plugin.rb:13-22. Port: src/email/notification.rs:820-856 | absent.

**UI**
- The rich editor extension, `toggleSpoiler` command and the menu item's active state (`state.inSpoiler`). Rails: plugins/spoiler-alert/assets/javascripts/initializers/spoiler-alert.js:16-33; lib/rich-editor-extension.js. Port: absent. [item 196]

### Items

New (ROADMAP Phase 11): 244 `feat/spoiler-email` S. Earlier items: 196.

### Couldn't determine
- Whether Rails' `post.url` in the spoiler handler (no `&.`) raises for digests or other callers without a post; needs a Rails run.

## Auth plugins: shared context

Shared context. ROADMAP item 191 (`feat/external-auth`) and all its sub-items 191a-191g are
marked done: 191b ported the OmniAuth request/callback phases, /auth/failure, the confirmation
page, oauth2-basic and openid-connect, connect/revoke, the login page buttons and OIDC's
RP-initiated logout; 191f ported apple, lti and patreon; 191g the expired-token refresh, admin IP
allowlist, linked groups and the associate page. No open 191 sub-item remains, so none of the gaps
below is tagged [item 191]. Item 164 (`feat/site-settings-types`) covers custom validators
(src/site_setting_update.rs:181, refusal "settings with custom validations") and list/objects
setting types (src/site_setting_update.rs:212, refusal "setting types other than string, number,
bool and enum"). Every one of these plugins' enable settings carries
`validator: "AuthProviderCredentialsValidator"`, so an admin cannot turn any of the four on through
the port today; those gaps are tagged [item 164].

Locales: the port loads every bundled plugin's client and server en.yml
(src/i18n.rs:25-28, vendor/discourse/config/locales/plugin_{client,server}.en.yml), so the
plugins' `js.login.<name>.*`, site setting descriptions and `login.*` strings resolve. The
`login.omniauth_error.openid_connect_*` keys are unused in Rails too (Users::OmniauthCallbacksController
ALLOWED_FAILURE_ERRORS, app/controllers/users/omniauth_callbacks_controller.rb:109), and the port's
/auth/failure matches that (src/routes/omniauth.rs:494).

Refusals that stand in for upstream exceptions (Rails answers a 500 or a NoMethodError, so these
are parity, not gaps): src/plugins/lti.rs:160 (unreadable platform key), :277 (error alone,
re-raised by on_failure), :321 (several audiences, `client_ids.include` NoMethodError at
plugins/discourse-lti/lib/discourse_lti/lti_omniauth_strategy.rb:155); src/routes/omniauth.rs:1258
and :1233 (blank sub), :1570 (ShouldReconnect raised twice); src/plugins/apple.rs:44 (unreadable EC
key), :256 (JWT::DecodeError is not rescued by the Apple strategy); src/plugins/oauth2_basic.rs:219,
:222 (walk_path TypeError/NoMethodError), :360 (user JSON parse error), :451 (blank uid);
src/plugins/openid_connect.rs:628 (unparsable end_session_endpoint, `uri` nil at
plugins/discourse-openid-connect/plugin.rb:58).

## discourse-lti (`plugins/discourse-lti`)

Verdict: started (3 gaps)

### What the port has
- Authenticator registry entry: name `lti`, enable `lti_enabled`, required settings, can_connect
  and can_revoke true, no icon (src/plugins/auth_providers.rs:51-58,130-142).
- `/auth/lti` request phase answers `third_party_only` failure (src/routes/omniauth.rs:235-239).
- `/auth/:provider/initiate` GET/POST (src/routes/mod.rs:2112; src/routes/omniauth.rs:697-753):
  404 unless LTI is enabled, cross-site POST resubmitted as a samesite form
  (src/plugins/lti.rs:34-56), iss/login_hint/target_link_uri required, issuer check, client_id
  checks (invalid, missing with several ids, default first), state and nonce in the session,
  target_link_uri as the server session destination, authorize redirect with sorted `to_query`
  params and optional lti_message_hint (src/plugins/lti.rs:88-137).
- `/plugins/discourse-lti/javascripts/submit-on-load-lti.js` served (src/routes/omniauth.rs:667;
  static/js/submit-on-load-lti.js), script URL made absolute against the CDN
  (src/routes/omniauth.rs:686).
- Callback: cross-site POST resubmit of error/state/id_token (src/routes/omniauth.rs:1451), error
  with CallbackError message, state_missing, id_token_missing, state_mismatch, RS256 signature
  against the PEM or bare Base64 key, exp/nbf/iss/aud, token_invalid, nonce_mismatch, auth hash
  info/extra (raw_info, id_token) (src/plugins/lti.rs:146-346).
- after_authenticate with `lti_email_verified` as primary_email_verified, match by email
  (src/routes/omniauth.rs:1249-1272).
- Both `after_auth` hooks: invite link from the custom claim (both keys, host and invites#show
  check) as destination for new users, and ShouldReconnect when a `_t` cookie is present, completed
  as a reconnect (src/plugins/lti.rs:350-384; src/routes/omniauth.rs:1536-1582).
- UI: LTI login button hidden on the login page (src/routes/login_required.rs:291-294); connect
  button hidden on the account preferences (src/preferences_view/mod.rs:457-458); revoke allowed.
- Admin plugin entry with the graduation-cap icon (src/admin_view.rs:694-700); STARTED
  (src/plugins/mod.rs:292).
- Parity case parity/writes/external_auth/lti_flow.case.json.

### Gaps
**Settings**
- `lti_enabled` cannot be turned on from the admin: its AuthProviderCredentialsValidator is a
  custom validation. Rails: plugins/discourse-lti/config/settings.yml:2-4. Port:
  src/site_setting_update.rs:181 (refusal "settings with custom validations"). [item 164]
- `lti_client_ids` is a `type: list` setting and cannot be saved. Rails:
  plugins/discourse-lti/config/settings.yml:7-9. Port: src/site_setting_update.rs:212 (refusal
  "setting types other than string, number, bool and enum"). [item 164]

**Routes and controllers**
- The resubmit page's script tag gets an empty `nonce=""` and no Content-Security-Policy header is
  sent; Rails puts `ContentSecurityPolicy.nonce_placeholder` there so the CSP middleware fills the
  nonce and the header. The port has no CSP at all (no `content-security-policy` anywhere in src/).
  Rails: plugins/discourse-lti/lib/discourse_lti/lti_omniauth_strategy.rb:199. Port:
  src/plugins/lti.rs:54 | absent.

### Items

New (ROADMAP Phase 11): the resubmit page's nonce in 214 `feat/csp` M, with core's content security policy, which no item had. Earlier items: 164 (the enable validator, lti_client_ids).

### Couldn't determine
- Whether ruby-jwt in the reference's lockfile applies a default leeway to exp/nbf (the ref
  checkout has no Gemfile.lock); the port uses none (src/plugins/lti.rs:228-237).

## discourse-oauth2-basic (`plugins/discourse-oauth2-basic`)

Verdict: started (13 gaps)

### What the port has
- Registry entry: `oauth2_basic`, title_setting `oauth2_button_title`, required settings,
  can_connect and can_revoke from `oauth2_allow_association_change`
  (src/plugins/auth_providers.rs:23-25,67-73,156-168).
- Client from settings: authorize URL with the signup URL when `signup` is present, token URL,
  `oauth2_token_url_method` GET/POST/PUT, auth scheme from `oauth2_send_auth_header` and
  `oauth2_send_auth_body` (both: body auth plus Basic header), FinalDestination, 10s timeout
  (src/plugins/oauth2_basic.rs:26-67).
- `oauth2_scope` through `oauth2_authorize_options` (scope only) (src/plugins/oauth2_basic.rs:71-81);
  `oauth2_disable_csrf` as provider_ignores_state (:84-86; src/routes/omniauth.rs:991).
- uid and info from `oauth2_callback_user_id_path` / `oauth2_callback_user_info_paths`
  (src/plugins/oauth2_basic.rs:90-146).
- User JSON fetch with `oauth2_user_json_url` (`:token`, `:id`), GET/POST
  `oauth2_user_json_url_method`, Bearer and Accept headers; parse_segments, walk_path, json_walk for
  user_id, username, name, email, email_verified, avatar; groups from `oauth2_json_groups_path`;
  `oauth2_user_field_mappings`; failure message `login.authenticator_error_fetch_user_details`
  (src/plugins/oauth2_basic.rs:159-447).
- primary_email_verified (`oauth2_email_verified` or the info's email_verified),
  `oauth2_overrides_email`, provides_groups, associated groups and user field values on the result
  (src/plugins/oauth2_basic.rs:385-489; src/auth/managed.rs:901).
- Token request failures (timeout and connection errors) as `oauth2_basic_request_failed`
  (src/routes/omniauth.rs:1118-1123); expired token refresh (:1129-1138).
- Admin plugin entry with the key icon (src/admin_view.rs:680-686).
- Parity cases parity/writes/external_auth/oauth2_*.case.json (new user, existing user, expired
  token, linked groups, override email, overrides/groups/fields, suspended user, IP allowlist).

### Gaps
**plugin.rb**
- The `oauth2_basic_additional_json_paths` and `oauth2_basic_required_json_paths` registries
  (extra keys stored in the auth hash extra; required path values that fail the login with a
  plugin-given message) do not exist. Rails: plugins/discourse-oauth2-basic/plugin.rb:23,35;
  lib/oauth2_basic_authenticator.rb:253-256,304-315. Port: absent (src/plugins/oauth2_basic.rs:7-9
  says so in a comment).
- `GlobalSetting.oauth2_request_timeout_seconds` (DISCOURSE_OAUTH2_REQUEST_TIMEOUT_SECONDS) is not
  configurable; the port hardcodes 10s. Rails: plugins/discourse-oauth2-basic/plugin.rb:16;
  lib/oauth2_basic_authenticator.rb:335-337. Port: src/plugins/oauth2_basic.rs:19.
- `register_site_setting_area("oauth2")` and `register_admin_config_login_route("oauth2")`: no
  /admin/config/login-and-authentication/oauth2 tab, no `admin_config_login_routes` entry, no
  admin sidebar link (`admin_login_oauth2`), so the `oauth2` area's settings have no page.
  Rails: plugins/discourse-oauth2-basic/plugin.rb:18-19; config/locales/client.en.yml
  (admin.config.login.sub_pages.oauth2); frontend/discourse/app/lib/sidebar/admin-sidebar.js:424.
  Port: src/site.rs:505-506 (always `[]`), src/routes/admin_config.rs:669-707 (comment: not here).

**Routes and controllers**
- `oauth2_debug_auth` is never read: no OAuth2FaradayFormatter request/response logs with
  OAuth2LogRedactor (headers, URL query, userinfo and fragment redaction), and none of the
  authenticator's `log` lines (after_authenticate, user_json request and response, parsed, groups
  path not an array / not found). Rails: plugins/discourse-oauth2-basic/lib/oauth2_faraday_formatter.rb;
  lib/oauth2_basic_authenticator.rb:82-87,148,151,199-201,215-219,232-237,242,279-289. Port: absent.
- The always-on warnings are missing: "OAuth2 Basic: token request timed out/failed" and
  "OAuth2 Basic: user_json request to <redacted url> ... failed". Rails:
  plugins/discourse-oauth2-basic/lib/omniauth/strategies/oauth2_basic.rb:31-38;
  lib/oauth2_basic_authenticator.rb:223-229. Port: src/plugins/oauth2_basic.rs:349-355 (silent),
  src/routes/omniauth.rs:1118-1123.
- `oauth2_authorize_options` only forwards `scope`; Rails' `options_for("authorize")` sends every
  listed strategy option that has a value (e.g. `client_id`, `name`, `provider_ignores_state` if
  listed). Rails: plugins/discourse-oauth2-basic/lib/oauth2_basic_authenticator.rb:49-52
  (omniauth-oauth2 options_for). Port: src/plugins/oauth2_basic.rs:75-79.
- `recurse` over the access token only knows token, refresh_token, expires_at, params; Rails sends
  any method the AccessToken (or a nested Hash) responds to, e.g. `expires_in`, `expires?`,
  `response`. Rails: plugins/discourse-oauth2-basic/lib/omniauth/strategies/oauth2_basic.rb:46-51.
  Port: src/plugins/oauth2_basic.rs:90-112.
- A token endpoint answering a redirect is refused; the oauth2 gem follows it. Rails:
  plugins/discourse-oauth2-basic/lib/oauth2_basic_authenticator.rb:82-91 (oauth2 client
  get_token). Port: src/auth/oauth2.rs:567 (refusal "a redirected OAuth token request").

**Settings**
- `oauth2_enabled` cannot be turned on (AuthProviderCredentialsValidator). Rails:
  plugins/discourse-oauth2-basic/config/settings.yml:2-6. Port: src/site_setting_update.rs:181.
  [item 164]
- `oauth2_fetch_user_details` cannot be changed (Oauth2FetchUserDetailsValidator, error
  `site_settings.errors.oauth2_fetch_user_details`). Rails:
  plugins/discourse-oauth2-basic/config/settings.yml:38-41;
  lib/validators/oauth2_basic/oauth2_fetch_user_details_validator.rb. Port:
  src/site_setting_update.rs:181. [item 164]
- List settings `oauth2_callback_user_info_paths`, `oauth2_authorize_options` cannot be saved.
  Rails: plugins/discourse-oauth2-basic/config/settings.yml:34-37,101-104. Port:
  src/site_setting_update.rs:212. [item 164]
- Objects setting `oauth2_user_field_mappings` (schema path/user_field_id) cannot be saved or
  edited. Rails: plugins/discourse-oauth2-basic/config/settings.yml:73-85. Port:
  src/site_setting_update.rs:212. [item 164]

**UI**
- `oauth2_authorize_signup_url` is unreachable from the UI: the port's /signup page has no
  external login buttons and no button ever posts `signup=true` (Ember's create-account modal and
  the single-external-login /signup route call `doLogin({signup: true})`). Rails:
  frontend/discourse/app/models/login-method.js:61-77; controllers/signup.js:153,606;
  routes/signup.js:95. Port: templates/signup.html (no buttons), templates/login_required.html:40
  (no signup param).

### Items

New (ROADMAP Phase 11): 245 `feat/oauth2-basic-rest` S, 246 `feat/oauth2-oidc-admin-login-tabs` S, 247 `feat/auth-login-buttons` S. Earlier items: 164 (validators, list and objects settings), 203-206 (the JSON path registries).

### Couldn't determine
- Whether `OAuth2::Client` in the reference's gem version uses the request body for PUT token
  requests; the port sends params in the query for anything but POST (src/auth/oauth2.rs:536-537),
  which matches oauth2 2.x as far as I know, but the lockfile is not in ref.

## discourse-openid-connect (`plugins/discourse-openid-connect`)

Verdict: started (10 gaps)

### What the port has
- Registry entry: `oidc`, required settings with the secret dropped under PKCE or mTLS,
  can_connect and can_revoke from `openid_connect_allow_association_change`
  (src/plugins/auth_providers.rs:27-29,75-93,169-181).
- Discovery document via FinalDestination, cached ten minutes under
  `openid-connect-discovery-<url>` (nil cached too), error logs for a missing URL and fetch
  failures (src/plugins/openid_connect.rs:28-68).
- `discover!`: authorization/token endpoints and issuer required (openid_connect_discovery_error),
  userinfo endpoint optional, client_secret_post detection, PKCE without secret uses the body
  (src/plugins/openid_connect.rs:91-149).
- Client with `openid_connect_token_scope` token param (:152-171); authorize params: scope
  `openid_connect_authorize_scope`, passthrough `openid_connect_authorize_parameters`,
  `openid_connect_claims`, nonce in session, `p` kept for the token request (:183-216;
  src/routes/omniauth.rs:255-276,1077-1080).
- PKCE (`openid_connect_use_pkce`): S256 challenge and verifier (src/routes/omniauth.rs:299-308,
  1081-1084).
- `openid_connect_error_redirects` error handler (src/plugins/openid_connect.rs:220-230;
  src/routes/omniauth.rs:993-999).
- ID token: unverified decode, exp, nbf, iss, aud, nonce (jwt_decode_failed,
  jwt_nonce_verify_failed) (src/plugins/openid_connect.rs:233-301); userinfo fetch with sub
  check (openid_connect_sub_mismatch); no-userinfo token request (build_access_token) (:321-472);
  request failures as openid_connect_request_failed (src/routes/omniauth.rs:1112-1116).
- after_authenticate: email_verified rule, `openid_connect_match_by_email`,
  `openid_connect_overrides_email`, groups from `openid_connect_groups_claim`,
  `openid_connect_user_field_mappings` (src/plugins/openid_connect.rs:476-588).
- RP-initiated logout on session destroy with id_token_hint, optional client_id,
  post_logout_redirect_uri and state (src/plugins/openid_connect.rs:593-656;
  src/routes/session.rs:709-729).
- Admin plugin entry with the id-card icon (src/admin_view.rs:722-728).
- Parity case parity/writes/external_auth/oidc_nonce_mismatch.case.json.

### Gaps
**plugin.rb**
- `GlobalSetting.openid_connect_request_timeout_seconds` is not configurable; hardcoded 10s.
  Rails: plugins/discourse-openid-connect/plugin.rb:18;
  lib/openid_connect_authenticator.rb:240-242. Port: src/plugins/openid_connect.rs:18.
- `register_site_setting_area("oidc")` / `register_admin_config_login_route("oidc")`: no OIDC tab,
  no `admin_config_login_routes` entry, no sidebar link. Rails:
  plugins/discourse-openid-connect/plugin.rb:20-21; config/locales/client.en.yml
  (admin.config.login.sub_pages.oidc). Port: src/site.rs:505-506, src/routes/admin_config.rs:669.

**Routes and controllers**
- mTLS client authentication (`openid_connect_mtls_client_cert`, `_key`, `_key_passcode`): the
  `tls_client_auth` scheme and the client certificate on the token/userinfo connection are refused,
  and the cert/key parse failure log is absent. Rails:
  plugins/discourse-openid-connect/lib/openid_connect_authenticator.rb:192-196,219-238. Port:
  src/auth/oauth2.rs:532-533 (refusal "mutual TLS client authentication"),
  src/plugins/openid_connect.rs:7,126-127.
- `openid_connect_verbose_logging` is never read: no OIDCFaradayFormatter request/response logs,
  no `verbose_log` lines (error handled redirect, loaded JWT, verified JWT, userinfo response), no
  non-error `oidc_log` lines (fetching, from cache, the document itself, groups claim not found,
  logout: no record, no id_token, redirecting). Rails:
  plugins/discourse-openid-connect/lib/openid_connect_faraday_formatter.rb;
  lib/omniauth_open_id_connect.rb:129,168,182,192; lib/openid_connect_authenticator.rb:62,119,
  131-132,137-143,157,198-203; plugin.rb:32,38,56. Port: absent.
- Error-level logs missing: "groups claim '<claim>' is not an array" and
  "OIDC Log: request timed out/failed" on a Faraday error. Rails:
  plugins/discourse-openid-connect/lib/openid_connect_authenticator.rb:59-60;
  lib/omniauth_open_id_connect.rb:150-156. Port: src/plugins/openid_connect.rs:572-584,
  src/routes/omniauth.rs:1112-1116 | absent.
- A redirected token response is refused (shared with oauth2-basic). Rails: oauth2 gem via
  plugins/discourse-openid-connect/lib/omniauth_open_id_connect.rb:136. Port:
  src/auth/oauth2.rs:567.
- Groups claim fallback: Rails falls back to id_token_info when raw_info's claim is nil or false
  (`||`); the port only on null. Rails:
  plugins/discourse-openid-connect/lib/openid_connect_authenticator.rb:55. Port:
  src/plugins/openid_connect.rs:566-571.

**Settings**
- `openid_connect_enabled` cannot be turned on (AuthProviderCredentialsValidator). Rails:
  plugins/discourse-openid-connect/config/settings.yml:2-5. Port: src/site_setting_update.rs:181.
  [item 164]
- List settings `openid_connect_error_redirects` (list_type secret, masked in the admin) and
  `openid_connect_authorize_parameters` (compact) cannot be saved. Rails:
  plugins/discourse-openid-connect/config/settings.yml:43-52. Port: src/site_setting_update.rs:212.
  [item 164]
- Objects setting `openid_connect_user_field_mappings` cannot be saved. Rails:
  plugins/discourse-openid-connect/config/settings.yml:63-75. Port: src/site_setting_update.rs:212.
  [item 164]

### Items

New (ROADMAP Phase 11): 248 `feat/openid-connect-mtls` M, 249 `feat/openid-connect-logging` S, the redirected token response in 245 `feat/oauth2-basic-rest` S, the admin tab in 246 `feat/oauth2-oidc-admin-login-tabs` S. Earlier items: 164.

### Couldn't determine
- PKCE verifier format: the plugin passes `pkce_options[:code_verifier]` (32 random bytes,
  urlsafe Base64) but whether the reference's omniauth-oauth2 version calls it or uses its own
  `SecureRandom.hex(64)` depends on the gem version, and ref has no Gemfile.lock. The port uses
  hex(64) (src/routes/omniauth.rs:300; src/plugins/openid_connect.rs:658-659). Settle by running
  Rails or reading the vendored gem.

## discourse-login-with-amazon (`plugins/discourse-login-with-amazon`)

Verdict: started (2 gaps). Ported on `feat/plugin-login-with-amazon` (2026-10-28) against the
vendored bf55a44c (tmp/rails-bf55a44c): the strategy and authenticator moved out of core's
src/auth/builtin.rs into src/plugins/login_with_amazon.rs; tests in
tests/plugin_login_with_amazon.rs port spec/system/associated_account_prefs_spec.rb on the
server drawn preferences and cover the login against Amazon's endpoints stubbed on loopback.

### What the port has
- Registry entry: `amazon`, enable `enable_login_with_amazon`, required settings client id and
  secret, can_connect `login_with_amazon_user_can_connect`, can_revoke
  `login_with_amazon_user_can_revoke`, icon fab-amazon (src/plugins/auth_providers.rs); in
  /site.json's auth_providers and the login buttons only while enabled.
- Settings from the vendored plugin_settings.yml (client id and secret, scope `profile`,
  overrides email, hidden email verified `true`, user can connect and revoke), area
  authenticators; "discourse-login-with-amazon" in `plugins::STARTED`; admin plugin entry
  (src/admin_view.rs).
- OmniAuth::Strategies::Amazon: client on https://www.amazon.com/ with /ap/oa and
  https://api.amazon.com/auth/o2/token, credentials in a Basic header; the scope setting as the
  only authorize param (it replaces the strategy's `profile postal_code`); `callback_url` from
  REDIRECT_URL_ORIGIN when set; `build_access_token` with the callback without its query and the
  client's id and secret in the form (src/plugins/login_with_amazon.rs).
- Auth hash from GET /ap/user/profile with the token in the query: uid `Profile.CustomerId`,
  email `PrimaryEmail`, name `Name`, extra `postal_code` (keys as the SnakyHash reads them).
- Auth::LoginWithAmazonAuthenticator: email verified, always update user email and can connect
  from the settings (`behaviour`; src/routes/omniauth.rs after_authenticate).
- The account preferences' associated accounts list a login only when the user has it or may
  connect it (authProviders' filter), with the revoke button only while users may revoke
  (src/preferences_view/mod.rs).
- Parity case parity/writes/external_auth/amazon_new_user.case.json.

### Gaps
**Settings**
- `enable_login_with_amazon` cannot be turned on through the admin: AuthProviderCredentialsValidator
  is not ported. Rails: plugins/discourse-login-with-amazon/config/settings.yml:2-5. Port:
  src/site_setting_update.rs ("site settings with a validator class not ported"). [item 164]

**Locales**
- Every locale but English for `js.login.amazon.*`, the settings category and descriptions.
  [item 167]

### Items

None new: 164 and 167.

### Couldn't determine
- omniauth-oauth2 and the oauth2 gem are not vendored: the Basic header besides the form's
  credentials, the token in the profile request's query (`access_token_options mode: :query`
  plus the explicit param) and the snake_cased profile keys come from the plugin's code, the
  recorded parity case and the gems' documented behaviour. The endpoints follow the plugin's
  constants and Amazon's Login with Amazon documentation.
- The preferences filter (an account or can_connect) follows the system spec; core's Ember
  component is not in the extract.

## discourse-apple-auth (`plugins/discourse-apple-auth`)

Verdict: started (2 gaps)

### What the port has
- Registry entry: `apple`, enable `sign_in_with_apple_enabled`, required settings, can_connect and
  can_revoke true (ManagedAuthenticator defaults), provider_url https://account.apple.com, icon
  fab-apple (src/plugins/auth_providers.rs:31-42,104-116).
- Client secret: ES256 JWT (iss team id, aud appleid, sub client id, iat, exp +60s, kid header)
  from a PKCS8 or SEC1 PEM (src/plugins/apple.rs:33-63); client with request-body auth, Apple's
  site/authorize/token URLs, `response_mode=form_post` and `scope=email name`
  (src/plugins/apple.rs:66-88).
- Callback POST redirected back as a GET with code, state and user, no session cookie
  (src/plugins/apple.rs:92-103; src/routes/omniauth.rs:1455-1461).
- JWKS fetched (not through FinalDestination) and cached a day as `sign-in-with-apple-jwks`, error
  logs on failure (src/plugins/apple.rs:107-134); ID token RS256 verified by kid, exp, nbf, iss, aud
  (:146-232).
- Auth hash: email from the token, first/last name from the `user` param, name joined, extra
  raw_info with id_token_info and id_token (src/plugins/apple.rs:236-301); every email verified,
  match by email (src/routes/omniauth.rs:1223-1247).
- Settings area `authenticators` shown on the login config's Authenticators tab
  (src/routes/admin_config.rs:691-697); admin plugin entry with fab-apple (src/admin_view.rs:582-587).
- Parity case parity/writes/external_auth/apple_login.case.json.

### Gaps
**Settings**
- `sign_in_with_apple_enabled` cannot be turned on (AuthProviderCredentialsValidator). Rails:
  plugins/discourse-apple-auth/config/settings.yml:2-5. Port: src/site_setting_update.rs:181.
  [item 164]

**UI**
- The login button's `aria-label` (LoginMethod#screenReaderTitle from `js.login.apple.sr_title`
  "Sign in with Apple") is missing; the port's buttons have no aria-label. Rails:
  plugins/discourse-apple-auth/config/locales/client.en.yml (sr_title);
  frontend/discourse/app/components/login-buttons.gjs:79; models/login-method.js:49-52. Port:
  templates/login_required.html:42.

### Items

New (ROADMAP Phase 11): the aria-label in 247 `feat/auth-login-buttons` S. Earlier items: 164 (the enable validator).

### Couldn't determine
- Whether OmniAuth reuses the first generated `client_secret` across requests
  (`options[:client_secret] ||= client_secret` on a possibly shared options hash,
  plugins/discourse-apple-auth/lib/omniauth_apple.rb:98-100); the port makes a fresh one per
  request (src/plugins/apple.rs:66-69). Only observable after 60 seconds; needs Rails to settle.

## discourse-microsoft-auth (`plugins/discourse-microsoft-auth`)

Verdict: started (2 gaps). Ported on `feat/plugin-microsoft-auth` (2026-10-28) against the
vendored bf55a44c (tmp/rails-bf55a44c): the strategy and authenticator moved out of core's
src/auth/builtin.rs into src/plugins/microsoft_auth.rs; tests in tests/plugin_microsoft_auth.rs
port spec/integration/microsoft_auth_spec.rb and spec/lib/microsoft_auth_revoker_spec.rb.

### What the port has
- Registry entry: `microsoft_office365`, enable `microsoft_auth_enabled`, required settings client
  id and secret, can_connect and can_revoke true, icon fab-microsoft (src/plugins/auth_providers.rs);
  in /site.json's auth_providers and the login buttons only while enabled.
- Settings from the vendored plugin_settings.yml (`microsoft_auth_enabled`, `_client_id`,
  `_client_secret` secret, `_email_verified`, `_tenant_id` default `common`), area authenticators;
  "discourse-microsoft-auth" in `plugins::STARTED`; admin plugin entry with fab-microsoft
  (src/admin_view.rs).
- OmniAuth::Strategies::MicrosoftOffice365: client on https://login.microsoftonline.com with the
  tenant's `/{tenant}/oauth2/v2.0/authorize` and `/token` paths, credentials in a Basic header;
  authorize params `display`, `score`, `auth_type` from the request, then the default scope
  (src/plugins/microsoft_auth.rs `client`, `authorize_params`); callback url
  `full_host + script_name + callback_path`.
- Auth hash from Graph's /me with the bearer token: uid `id`, name `displayName` or
  `userPrincipalName`, email `mail` or `userPrincipalName`, extra raw_info as the SnakyHash keeps
  it (snake_case keys) (`auth_hash`).
- MicrosoftAuthenticator: ManagedAuthenticator with `primary_email_verified?` from
  `microsoft_auth_email_verified` (matching an account by email only then), connect and revoke
  (`behaviour`; src/routes/omniauth.rs after_authenticate).
- MicrosoftAuthRevoker and its rake tasks: `discourse-rs microsoft-auth revoke` (deactivate, log
  out, revoke user API keys and the API keys they created, delete the associations) and
  `discourse-rs microsoft-auth log-out-users` (`revoke`, `log_out_users`; src/cli.rs).
- Migrations: the four 2021 data migrations (office365 settings copied and removed,
  oauth2_user_infos rows copied into user_associated_accounts and deleted) are recorded as run in
  the vendored schema; a database from before them goes through Rails' migrations before the port
  reads it.
- Parity case parity/writes/external_auth/microsoft_login.case.json.

### Gaps
**Settings**
- `microsoft_auth_enabled` cannot be turned on through the admin: AuthProviderCredentialsValidator
  is not ported, and the update refuses settings with a validator class it lacks. Rails:
  plugins/discourse-microsoft-auth/config/settings.yml:2-6;
  lib/validators/auth_provider_credentials_validator.rb. Port: src/site_setting_update.rs
  ("site settings with a validator class not ported"). [item 164]

**Locales**
- Every locale but English for `js.login.microsoft_office365.*` and the setting descriptions.
  [item 167]

### Items

None new: 164 and 167.

### Couldn't determine
- omniauth-oauth2 and the oauth2 gem are not vendored: the Basic header (oauth2's default
  `auth_scheme: :basic_auth`), reading a form encoded or JSON token body, and SnakyHash's
  snake_cased /me keys come from the recorded parity case and the gems' documented behaviour,
  not their source. Graph's /me fields and the v2.0 endpoint paths follow Microsoft's identity
  platform documentation.

## discourse-assign (`plugins/discourse-assign`)

Verdict: started (ported 2026-10-28 on `feat/plugin-assign`, against bf55a44c read from the
extract at tmp/rails-bf55a44c; the plugin changed a lot after it, among others a `search`
action and per-user `assign_notification` jobs that bf55a44c does not have).

### What the port has
- Enabled check, settings, AssignmentPermissions (global groups, category additional groups,
  `allowed_user_ids_for_target`, `assignable_user_ids_for_user`, `allowed_group_ids_for_target`,
  `assign_allowed_groups_for_*`), GuardianExtensions `can_assign?` / `can_assign_globally?`,
  `Group.assignable`: src/plugins/assign/mod.rs.
- Assigner: assign (every forbidden reason, details changes with their small actions, the 5 per
  topic limit, max_assigned_topics, silenced assignments, the assignee's tracking level, live
  updates on /staff/topic-assignment, assigned web hooks), unassign (deactivate, silent), auto
  assign by mention (assigns_by_staff_mention and the regex settings), failure messages, the
  resolver: src/plugins/assign/assigner.rs.
- AssignController: PUT assign, PUT unassign, GET suggestions, GET assigned, GET members/:group;
  ListController messages_assigned, group_topics_assigned, private_messages_assigned:
  src/routes/assign.rs, src/routes/messages.rs, src/pm_lists.rs.
- Jobs: assign_notification (CreateNotification: the alert and the high priority notification),
  unassign_notification, remind_user (PendingAssignsReminder's system message), the daily
  enqueue_reminders: src/plugins/assign/jobs.rs, reminder.rs.
- Events: post_created / post_edited (auto assign), topic_status_updated (unassign_on_close,
  reassign_on_open), post_destroyed (deactivation, linked small actions destroyed),
  post_recovered, move_to_inbox / archive_message (unassign_on_group_archive),
  user_added_to_group, user_removed_from_group, group_destroyed, post_moved, solved's
  accepted_solution / unaccepted_solution, Group before_update / before_destroy on
  assign_allowed_on_groups: src/plugins/assign/lifecycle.rs.
- Serializer keys: current user (can_assign, can_assign_globally, never_auto_track_topics, the
  reminder frequency custom field), user_option and current_user_option
  notification_level_when_assigned (and UserUpdater), topic view, post, topic list item,
  suggested topic, search result, topic list assigned_messages_count, group_show,
  notification topic_bumped_at, bookmarks: src/plugins/assign/keys.rs.
- Filters: TopicQuery `assigned=`, search in:assigned, in:unassigned, assigned:, group param
  assignable_level, bulk operations assign and unassign.
- UI: the assignees under the first post and under posts, the post menu's assign button, the
  footer's Assign button and reassign menu, the assign modal, assign small actions on whispers
  with their icons and private-assign class, the topic list's assignee tags, the user
  activity Assigned tab, the tracking preference: src/plugins/assign/view.rs,
  static/js/discourse-assign.js, static/css/discourse-assign.css.

### Gaps
- invite_on_assign invitations (refused when an assignee would be invited), the assignment
  email (assign_mailer, refused when it would send), discourse-workflows triggers (refused).
- API key scope `assign` (the scope table is the recorded one, without the plugin).
- remind_assigns_frequency preference (editable user custom fields are refused by the update).
- Reviewable `assigned_to` filter and the above-review-filters connector, FlaggedTopic keys.
- /filter `assigned:` custom filter and topics/posts_filter_options (no /filter route).
- discourse-automation random_assign scriptable.
- UI: group Assigned tab and its member sidebar (group tabs other than members are not
  ported), user messages Assigned tab page, the user menu's assigns tab, category settings
  (additional groups, unassigned filter), the group interaction assignable_level field, the
  bulk assign modal, the topic list's assign column actions, the advanced search
  "assigned to" field, the unassigned navigation item, keyboard shortcut g a, the
  never_auto_track_topics notification reason.
- Rails joins the assignments for `assigned=` lists, repeating a topic with several; the port
  lists each topic once.

## discourse-captcha (`plugins/discourse-captcha`)

Verdict: started (5 gaps). Ported on `feat/plugin-captcha` (2026-10-28) against the vendored
bf55a44c (tmp/rails-bf55a44c); tests in tests/plugin_captcha.rs port
spec/lib/discourse_captcha/create_users_controller_patch_spec.rb, the hCaptcha and reCAPTCHA
controller and provider specs, the two problem check specs and the server side of
spec/system/signup_with_captcha_spec.rb, against the providers' siteverify endpoints stubbed on
loopback.

### What the port has
- Settings from the vendored plugin_settings.yml (`discourse_captcha_enabled`, the
  `discourse_captcha_provider` enum hcaptcha/recaptcha/none, both providers' site and secret
  keys); "discourse-captcha" in `plugins::STARTED` (it has no enable validator, so admins can
  turn it on); admin plugin entry with the hand icon (src/admin_view.rs).
- CaptchaController, HcaptchaController, RecaptchaController: POST /captcha/hcaptcha/create and
  /captcha/recaptcha/create (with `.json`): the CSRF check, requires_plugin and another provider
  selected as 404s, a blank token as InvalidParameters (400), the token kept in the server session
  under `hcaptcha_token` / `recaptcha_token` for two minutes, `{"success":"OK"}`; open on login
  required sites (src/plugins/captcha.rs `create`, src/routes/captcha.rs,
  src/routes/login_required.rs).
- CaptchaVerification and CreateUsersControllerPatch: users#create, after the honeypot, refuses
  with `fail_with` (`{"success":false,"message":...}`): `captcha_not_configured` for a selected
  provider without both keys, `captcha_verification_failed` for a missing token, a provider
  answer of 500 or more, a body that is not JSON, or `success` nil or false, and any request
  failure (logged, as Rails rescues it). The token is taken out of the server session whatever
  the outcome. The secret and the token are posted as a form through FinalDestination's SSRF
  check (src/plugins/captcha.rs `verification_error`; hook `plugins::users_create_refusal`
  called from src/routes/accounts.rs).
- ProblemCheck::HcaptchaConfiguration and RecaptchaConfiguration, high priority, registered
  while the plugin is enabled and listed before core's checks (`plugins::problem_checks`,
  src/problem_checks.rs `checks`).
- UI: the create-account-after-user-fields connector at the end of the create-account form
  (also in the form an external login's signup draws), loading until the provider's script
  (render=explicit) is in, then the `#h-captcha-field` / `#g-recaptcha` container rendered with
  the site key; the token and expiry/error callbacks; the "create-account" behavior transformer
  that refuses with `missing_token` before a solved challenge, posts the token, refuses with
  `verification_failed` when that fails, and resets the widget after the attempt; the input tip
  under the widget; the error alert when the script cannot load (static/js/discourse-captcha.js;
  `applyBehaviorTransformer` in static/js/login.js; src/plugins/captcha.rs `signup_connector`).
- Migration 20260422000000 (discourse_hcaptcha_enabled to the provider settings): recorded as
  run in the vendored schema.

### Gaps
**SessionControllerPatch**
- The CAPTCHA gate on login code signups (process_verified_login_code with a username,
  complete_verified_login_code_signup and the signup proof's `captcha_verified`): the port's
  login codes only redeem invites, which the plugin never gates, and refuses the rest ("logging
  in with an emailed code", "completing a login code signup"). Rails:
  plugins/discourse-captcha/lib/discourse_captcha/session_controller_patch.rb. Port:
  src/routes/invite_redeem.rs (verify_login_code). Lands with the code signup in core.

**UI**
- The code-login-after-code connector and the `code-login-auto-create-account` value
  transformer (no account created straight from a verified code while a captcha is on): the
  code signup's later steps are not ported. Rails:
  assets/javascripts/discourse/connectors/code-login-after-code/captcha-fields-connector.gjs;
  pre-initializers/h-captcha.js:22.
- The widget is not under user fields: the port's signup page refuses sites with signup user
  fields, so the connector follows the form's last field.

**Locales**
- Every locale but English for `js.discourse_captcha.*`, the dashboard problems and the setting
  descriptions. [item 167]

**Content security policy**
- Rails' CSP would need the providers' script hosts; the port sends no CSP yet. [item 214]

### Items

None new: the login code signup is core's; 167 and 214.

### Couldn't determine
- The providers' siteverify protocol (a form POST of `secret` and `response`, a JSON answer with
  `success`) and their explicit-render JavaScript APIs (`render`, `reset`, the `callback`,
  `expired-callback` and `error-callback` options, the onload callback) come from the plugin's
  code and hCaptcha's and Google's documentation; no provider was called.
- Whether Rails sends the session cookie with a captcha refusal from users#create; the port sets
  it, as for the honeypot's answer.

## discourse-cakeday (`plugins/discourse-cakeday`)

Verdict: started (5 gaps). Ported on `feat/plugin-cakeday` (2026-10-28), after the
audit above; tests in tests/plugin_cakeday.rs port spec/integration/cakeday_spec.rb,
spec/serializers/{post,user}_serializer_spec.rb, spec/jobs/migrate_date_of_birth_to_users_table_spec.rb
and spec/system/cakeday_user_summary_spec.rb.

### What the port has
- Settings `cakeday_enabled` (the enabled setting), `cakeday_emoji`, `cakeday_birthday_enabled`,
  `cakeday_birthday_emoji`, from the vendored plugin_settings.yml; "discourse-cakeday" in
  `plugins::STARTED`; admin plugin entry with `cake-candles` (src/admin_view.rs); the icon is in
  the recorded sprite's plugin icons.
- Serializer keys: `cakedate` and `birthdate` on UserSerializer and UserCardSerializer (after
  chat's, before discourse-solved's), `user_cakedate` and `user_birthdate` on PostSerializer
  (before discourse-reactions'), each with plugin.rb's include conditions (a viewer; hide_profile
  unless it is the viewer's own; birthdays on; a date of birth for the post key), the cakedate in
  the viewer's `user_option.timezone`: src/plugins/cakeday/mod.rs (`user_keys`, `PostsData`),
  called from src/users.rs (show) and src/topic_view.rs (serialize_posts).
- AnniversariesController#index and BirthdaysController#index, GET /cakeday/anniversaries(/:filter)
  and /cakeday/birthdays(/:filter) as JSON: requires_plugin, ensure_logged_in, the birthday
  setting, `setup_params`, `cakedays_by` (today, tomorrow, upcoming, month; leap day as March
  1st; the viewer's zone with Rails' zone names mapped, Nuku'alofa included; unknown zones as
  UTC; at least a year old for anniversaries; the prioritize_username_in_ux tie breaker; pages
  of 48), CakedayUserSerializer, `total_rows_*` and `load_more_*`: src/plugins/cakeday/lists.rs,
  src/routes/cakeday.rs.
- The date of birth: UsersController#update's `date_of_birth` cast as ActiveModel casts a date
  (src/user_updater.rs, `cakeday::cast_date`), and UserDateOfBirthInput on the profile
  preferences (user-custom-preferences outlet) with static/js/cakeday.js composing "1904-M-D".
- Jobs FixInvalidDateOfBirth and MigrateDateOfBirthToUsersTable with Jobs::Onceoff's
  onceoff_logs gate and `force`: src/plugins/cakeday/jobs.rs, src/jobs/handlers.rs.
- UI, server side: the poster icons (`addPosterIcon`, anniversary then birthday, emoji or the
  cake icon, own or other's title, before the names in the mobile view) in src/post_view.rs
  (poster_name), live post renders included; UserCardCakeday (user-card-post-names) in
  src/user_card_view.rs; UserCakeday (user-post-names) on the profile in
  src/user_profile_view.rs; the community section's secondary Anniversaries and Birthdays links
  (`addCommunitySectionLink`) in src/sidebar.rs; the cakeday, anniversaries and birthdays
  templates (nav pills, today/tomorrow/upcoming titles, the All page's month chooser,
  UserInfoList with each date, empty messages, DLoadMore as an htmx sentinel) in
  src/plugins/cakeday/view.rs; the index routes redirect to today; stylesheets
  static/css/cakeday.css.
- Locales: English client and server strings through the vendored plugin locales.

### Gaps
**Jobs**
- The onceoffs are never enqueued: Rails' Jobs::EnqueueOnceoffs (every 10 minutes,
  app/jobs/scheduled/enqueue_onceoffs.rb) is not in the port's SCHEDULED list, so the two jobs
  only run when queued by hand. Sites restored from Rails already ran them (onceoff_logs).
  Rails: app/jobs/onceoff/onceoff.rb:43. Port: src/jobs/mod.rs (SCHEDULED) | absent.
- Date.parse of a custom field naming only part of a date (Ruby completes it from today) is
  refused, as is a two digit year in the profile's date cast (comp false). Port:
  src/plugins/cakeday/jobs.rs (date_parse), src/plugins/cakeday/mod.rs (cast_date).

**UI**
- "Today" for the icons and the list titles is the viewer's date in their `user_option.timezone`
  (UTC without one), where the client reads the browser's clock. Rails:
  plugins/discourse-cakeday/assets/javascripts/discourse/lib/cakeday.js:612-623.
- The date of birth combo boxes have a none row rather than select-kit's clear button
  (`clearable`, `autoInsertNoneItem: false`). Rails:
  plugins/discourse-cakeday/assets/javascripts/discourse/connectors/user-custom-preferences/user-date-of-birth-input.gjs:186-204.

**Locales**
- Every locale but English for `js.cakeday.*`, `js.anniversaries.*`, `js.birthdays.*`,
  `js.user.date_of_birth.*` and the site setting descriptions. [item 167]

### Items

None new: 167 covers the locales; the onceoff scheduler is core's (Jobs::EnqueueOnceoffs).

### Couldn't determine
- The UserSerializer key order around plugin keys: the port puts cakedate and birthdate after
  core's and chat's attributes and before accepted_answers, by plugin load order (directory
  names sorted); a Rails response would settle it.
- Serializing the cakedate for a viewer whose stored timezone Rails can't read: Rails'
  `in_time_zone` raises (a 500); the port formats it in UTC.
- The anniversaries' `cakedate` is the shifted local time serialized as UTC ("Z"), as Rails' select
  gives it; the page shows its date part, where moment would shift it again into the browser's
  zone.
- GET /cakeday on its own: the engine has no root route; whether Rails answers the Ember shell
  (the cakeday route with an empty outlet) or a 404 needs a Rails run. The port has no route.
## discourse-policy (`plugins/discourse-policy`)

Verdict: started (7 gaps). Added on `feat/plugin-policy`, after this
audit's commit, against the plugin at the vendored bf55a44c (it differs
from the checkout's HEAD 167479b1 only in JavaScript import paths and
tooling files).

### What the port has
- Markdown: `[policy]` as `div.policy` with sorted `data-*` attributes and version 1 by default, allow list, gated by `policy_enabled`: crates/markdown/src/render/bbcode.rs (`policy`); tests/pretty_text.rs.
- `on(:post_process_cooked)`: the first `.policy` outside a blockquote, for an author in `create_policy_allowed_groups`, synced to `post_policies` and `post_policy_groups` (groups, renew days or interval, renew start and next renewal, version with the add-to group's accepters removed on a bump, reminder, private, the add-to group when the author may edit it, `last_bumped_at`), the `HasPolicy` custom field, and the policy dropped when no group exists: src/plugins/policy/lifecycle.rs, called from src/jobs/handlers.rs (process_post).
- `validate(:post, :validate_policy)` (PostValidator): wiki posts refused, policy changes refused unless editor and author are allowed and the editor may manage a new add-to group; on create (all_topics_wiki included) and revise: src/plugins/policy/lifecycle.rs; src/posting/create.rs; src/posting/revise.rs.
- PolicyController (`PUT /policy/accept`, `/policy/unaccept`, `GET /policy/accepted`, `/policy/not-accepted` with offset paging), its errors and the `policy_change` post message, the add-to group's membership: src/routes/policy.rs.
- PostSerializer keys (`policy_can_accept` ... `policy_stats`, private and members visibility, easy revoke, inactive and suspended users left out), CurrentUser `can_create_policy`, UserOption `policy_email_frequency` and its UserUpdater attribute: src/plugins/policy/mod.rs; src/topic_view.rs; src/current_user.rs; src/user_private.rs; src/user_updater.rs.
- CheckPolicy every 6 hours (reminder notifications, the policy email for those who asked, renewals by days and intervals, expiry): src/plugins/policy/check.rs; src/jobs/mod.rs.
- UserNotifications#policy_email with its views and unsubscribe key, user_email's path for it, PolicyEmailUnsubscriber and the unsubscribe_options connector: src/plugins/policy/email.rs; src/jobs/user_email.rs; src/email/unsubscribe.rs.
- Report `unaccepted-policies`: src/plugins/policy/mod.rs (`report`); src/reports/mod.rs.
- UI: the PostPolicy component in the post (accept and revoke, the user lists and their toggle, load more, settings button), the composer preview's header, the builder modal (insert from the options menu, edit from the settings button), the email preferences connector, the stylesheets: src/plugins/policy/view.rs; static/js/policy.js; static/css/discourse-policy.css; src/composer_view.rs; src/preferences_view/emails.rs. Locales are the vendored plugin yml.
- Tests: tests/policy.rs.

### Gaps
**plugin.rb**
- `deprecate_setting("policy_restrict_to_staff_posts", ...)`: the deprecation warning when the old setting is read. Port: absent (nothing reads it).
- `has_one :post_policy, dependent: :destroy`: the port never destroys posts (permanent deletion is refused), so nothing to do yet; to add with permanent deletion.

**Models and validation**
- The wiki check runs on revisions of wiki posts only: `PUT /posts/:id/wiki` is not routed, so making a policy post a wiki (policy_controller_spec "wiki policy posts") cannot be exercised.
- `Date.parse` of `renew-start` completes partial dates from today; the port refuses anything but a full date (src/plugins/policy/lifecycle.rs `parse_date`).

**UI**
- The builder's group fields are native multiple selects, not select-kit's GroupChooser; the FormKit tooltips need admin-ui.js to open.
- Accept and revoke redraw the component from the server after the request instead of Ember's optimistic update first.
- The rich editor extension (the ProseMirror policy node and its edit button). Rails: assets/javascripts/discourse/initializers/rich-editor-extension.js. [item 196]

### Items

No new item; the rich editor waits on 196.

### Couldn't determine
- The policy item's place in the composer's options menu (after poll here) and `can_create_policy`'s place among the current user's keys (with chat's, before the `user_option` association) need a Rails response.
- The policy email's exact HTML and text after Email::Renderer and Styles were not compared with a Rails send.

## discourse-affiliate (`plugins/discourse-affiliate`)

Verdict: started (2 gaps). Ported on `feat/plugin-affiliate` (2026-10-29), against bf55a44c
read from the extract at tmp/rails-bf55a44c; tests: the unit tests in
src/plugins/affiliate.rs (every case of spec/lib/affiliate_processor_spec.rb, plus Ruby's
URI and query quirks) and tests/pretty_text.rs
`affiliate_codes_reach_the_cooked_column` (its post processor case).

### What the port has
- `on(:post_process_cooked)`: AffiliateProcessor.apply_to_doc on every `a[href]`, at the end
  of the post processor (after the nofollow pass, as Rails triggers the event), while
  `affiliate_enabled` is on: src/plugins/affiliate.rs, called from
  src/pretty_text/cooked_post_processor.rs.
- The rules: amazon.<postfix>, www. and smile. for every postfix of the list, amzn.com,
  amzn.to and a.co on the .com code, amzn.eu on the .eu code, ldlc.com and www.ldlc.com with
  the code as the fragment; a blank code or a setting that does not exist (amazon.to,
  amazon.co, which raise in Rails) keeps the url. The query is replaced by `tag` plus `k`,
  `ref` (with `k` only) and `node`, written with URI.encode_www_form; the url is parsed and
  rebuilt as Ruby's URI does (host case kept, scheme downcased, default port dropped,
  unparsable urls and bad percent escapes kept as they are).
- Settings and the admin plugin entry with `handshake`: vendored plugin_settings.yml,
  src/admin_view.rs (already there).

### Gaps
- `on(:post_process_localized_cooked)`: localized posts are not ported (no
  PostLocalization or LocalizedCookedPostProcessor), so
  spec/integration/content_localization_spec.rb has nothing to run against.
- Migration 20250722101702_enable_affiliate_if_already_installed: data-only, for
  databases that had codes before the plugin was off by default; a restored database
  already carries its result.

### Couldn't determine
- Nothing beyond the above; the processor was compared line by line.
## discourse-gamification (`plugins/discourse-gamification`)

Verdict: started (8 gaps). Added on `feat/plugin-gamification`, after
this audit's commit, against the plugin at the vendored bf55a44c. At that
ref the plugin keeps its daily scores per leaderboard
(`gamification_leaderboard_scores`, migration 20260402141912); the older
global `gamification_scores` table is only the migration's backfill
source and is not read by the reference, nor by the port.

### What the port has
- Settings: `discourse_gamification_enabled`, `scorable_categories`, the fifteen
  `<source>_score_value` weights and `score_ranking_strategy`, from the vendored
  plugin_settings.yml; "discourse-gamification" in `plugins::STARTED`; the admin plugin entry
  with `trophy` (src/admin_view.rs).
- Score sources (lib/discourse_gamification/scorables): day visited, flag created, like given
  and received, post created, post read, solutions, time read, topic created, user invited,
  reaction given and received (discourse-reactions on), chat reaction given and received and
  chat message created (chat on), each with its weight (leaderboard `score_overrides`, else the
  setting), its enabled check (weight above 0 and its plugin on) and the category filter
  (leaderboard `scorable_category_ids`, else `scorable_categories`); the SQL is the plugin's,
  `:since` inlined as ActiveRecord quotes it: src/plugins/gamification/scorables.rs.
- GamificationLeaderboardScore.calculate_scores (one source or all, with the score events, active
  and unsuspended users, upsert per user and day) and calculate_all (leaderboards not over):
  src/plugins/gamification/scorables.rs.
- LeaderboardCachedView: a materialized view per leaderboard and period, the plugin's query text
  (eligible users: primary email not anonymized, not staged, active, not suspended, not
  anonymous, included and excluded groups; the date range and period window), the unique user
  index, the query signature as the view's comment so Rails-built views stay valid, create,
  refresh concurrently, delete (gone leaderboards too), stale detection and purge, update_all and
  regenerate_all in transactions, `scores` with NotReady for a missing view (current schema
  only), the three ranking strategies: src/plugins/gamification/cached_view.rs.
- Jobs: GenerateLeaderboardPositions, RefreshLeaderboardPositions, DeleteLeaderboardPositions,
  RegenerateLeaderboardPositions, UpdateStaleLeaderboardPositions, RecalculateLeaderboardScores,
  RecalculateScores (with the /recalculate_scores message and the remaining count), the
  scheduled UpdateScoresForToday (hourly) and UpdateScoresForTenDays (daily), both no-ops while
  the plugin is off: src/plugins/gamification/jobs.rs, src/jobs/mod.rs, src/jobs/handlers.rs.
- GamificationLeaderboardController#respond (GET /leaderboard(/:id)(.json)): requires_plugin,
  the first leaderboard by default, can_see_leaderboard?, `period` (`all` as all_time, else the
  default period, else all_time), `page`, `user_limit`, LeaderboardViewSerializer (personal, the
  leaderboard, UserScoreSerializer users), 202 with the reason and the generate job while not
  ready, the minute's anonymous cache: src/routes/gamification.rs, src/plugins/gamification/mod.rs.
- AdminGamificationLeaderboardController: index (GET /admin/plugins/gamification.json and
  /admin/plugins/discourse-gamification/leaderboards.json, with the recalculations left and the
  groups), show, create (name and created_by_id required, the reserved name, the recalculate
  job), update (every field as the controller casts it, the score overrides' validations, the
  recalculate job when scoring changed else the refresh job, updated_at only on a change),
  destroy (scores with it, the delete job), recalculate_scores (5 a day, Date.parse, no future
  date); the StaffConstraint, ensure_admin and requires_plugin order: src/routes/gamification.rs.
- AdminGamificationScoreEventController: show (id, user_id, date filters, 100 at most), create
  (required params, the user must exist), update: src/routes/gamification.rs.
- Serializer keys: `gamification_score` on UserCardSerializer and UserSerializer (the default
  leaderboard's all time total, 0 and the generate job while not ready; after cakeday's keys),
  `default_gamification_leaderboard_id` on SiteSerializer: src/plugins/gamification/mod.rs,
  src/users.rs, src/site.rs.
- Events: `site_setting_changed` for score_ranking_strategy enqueues the regenerate job
  (src/plugins/mod.rs); `merging_users` sums the scores (src/user_merger.rs, already there);
  the user's scores go with them on destroy (src/plugins/mod.rs `user_destroying`).
- The directory column `gamification_score` (DirectoryIntegration.query) in the directory
  refresh: src/directory.rs (already there).
- UI, server side: the leaderboard page (title, the info button and its modal, the period
  chooser, disabled by `period_filter_disabled`, the staff settings link, the not ready state
  with Refresh, the podium, the viewer's row, the ranking paged by an htmx DLoadMore sentinel)
  in src/plugins/gamification/view.rs and static/js/gamification.js; the user card connector
  (user-card-metadata) and the profile connector (user-profile-secondary) linking the score to
  the default leaderboard (src/user_card_view.rs, src/user_profile_view.rs); the admin plugin
  page's configuration nav (Settings, Leaderboards), the leaderboards list with New, the create
  form, Recalculate with its modal (ranges, custom date, the result through the message bus),
  delete with its confirmation, the empty state, and the edit form (name, dates, period filter,
  default period, groups, scorable categories, the fifteen weights) in
  src/plugins/gamification/admin_view.rs, src/admin_plugins_view.rs (`config_page`),
  src/routes/admin_plugins.rs; the stylesheets in static/css/gamification.css.
- Locales: English client and server strings through the vendored plugin locales.
- Tests: tests/plugin_gamification.rs.

### Gaps
**plugin.rb**
- `register_discourse_workflows_node` (the "Award score" node of discourse-workflows,
  lib/discourse_workflows/nodes/gamification_score/v1.rb): discourse-workflows is not ported.
- The leaderboard fixture (db/fixtures/001_gamification_leaderboards.rb, a "Global
  Leaderboard" seeded when none exists): the port runs no seeds; restored sites have it.
- `add_directory_column`'s `before_directory_refresh` find_or_create of the column row: the
  port relies on the row a Rails boot created, and the users directory page refuses plugin
  columns once an admin enables one (src/routes/directory.rs), so the column shows only in
  /directory_items.json.

**Jobs**
- GenerateLeaderboardPositions' DistributedMutex: not taken (view creation is idempotent and
  the port runs one worker per site).

**UI**
- MinimalGamificationLeaderboard (used by themes, e.g. right sidebar blocks): not drawn.
- The period chooser loads the page for its period (`?period=`) where Ember swaps the users
  without changing the URL; a direct `?page=N` URL shows that page's rows alone.
- The group and category choosers of the edit form are native multiple selects, not
  select-kit's GroupChooser and CategorySelector; the default period is a native select, not
  PeriodInput.
- The recalculate modal's ranges are computed on the server in UTC, not with moment in the
  browser; the custom date is a native date input, not DatePickerPast.

### Items

No new item.

### Couldn't determine
- Plugin outlets' wrappers: the user-profile-secondary connector is drawn as a bare
  `<div><dt>..</dt><dd>..</dd></div>` inside the secondary list, and the classic connector on
  the card as its own `div`; a Rails render would settle both.
- The admin plugin show route for a plugin with a configuration nav: the port draws the
  settings with the nav as header tabs (Settings first, labelled
  `admin.plugins.change_settings_short`); Ember's exact tab markup was not available.
- The directory column's header label (`gamification_score` at the root of the client
  strings, "directory column workaround") needs the core directory table's code.
## discourse-graphviz (`plugins/discourse-graphviz`)

Verdict: started (3 gaps). Ported on `feat/plugin-graphviz` (2026-10-29), against bf55a44c
read from the extract at tmp/rails-bf55a44c (the frontend-only plugin: the server no
longer renders SVG, migration 20260610075829); tests in tests/pretty_text.rs
(`graphviz_cooks_as_rails_when_enabled`, spec/integration/graphviz_cooking_spec.rb) and
tests/plugin_graphviz.rs.

### What the port has
- Markdown: the `[graphviz]` block bbcode's `replace`, a `div.graphviz.is-loading` with
  `data-engine` (dot, neato, circo, fdp, osage or twopi, else dot) and the escaped source,
  on the allow list, gated by `discourse_graphviz_enabled` and left out of chat; off, the
  tag is text again instead of a refusal: crates/markdown/src/render/bbcode.rs.
- Client: the cooked decorator replacing each placeholder with a `.graphviz-wrapper`
  (GraphvizInline: the fullscreen button with `discourse-expand`, GraphvizDiagram),
  viz.js loaded once from Rails' path, sanitizeGraphvizSvg (scripts, foreignObject, `on*`
  and `javascript:` attributes, links other than http(s) and relative unwrapped), the
  graph error, GraphvizFullscreen as a modal with click to zoom; on cooked posts (and what
  htmx swaps in) and the composer preview: static/js/graphviz.js.
- viz-3.0.1.js, byte-identical, at /plugins/discourse-graphviz/javascripts/viz-3.0.1.js:
  static/vendor/viz, src/assets.rs.
- The composer's Insert Graphviz graph (`diagram-project`, the sample graph around the
  selection); the options menu no longer refuses to open while the plugin is on:
  src/composer_view.rs, static/js/composer.js.
- Stylesheet: static/css/graphviz.css. Admin plugin entry and icon: src/admin_view.rs
  (already there).

### Gaps
- The rich editor extension (the graphviz node with its live preview, GraphvizPreview).
  [item 196]
- DAsyncContent's loading state is a small spinner here; its exact markup was not ported.
- The options menu item's place among the plugins' items is not known (after local dates
  here).

### Couldn't determine
- Whether Rails' block bbcode `replace` hands the content over exactly as the port's
  `[code]` does (applyBBCode is core JS, not in the extract); the graphviz block takes the
  same path, and the spec checks only substrings.
## discourse-narrative-bot (`plugins/discourse-narrative-bot`)

Verdict: started (8 gaps). Ported on `feat/plugin-narrative-bot` (2026-10-29) against
bf55a44c (tmp/rails-bf55a44c); tests in tests/narrative_bot.rs port new_user_narrative_spec,
advanced_user_narrative_spec, track_selector_spec, user_spec, jobs/bot_input_spec,
jobs/send_default_welcome_message_spec, requests/discobot_certificate_spec,
requests/discobot_welcome_post_spec and lib/certificate_generator_spec (the last as unit tests
in src/plugins/narrative_bot/certificate.rs).

### What the port has
- Settings from the vendored plugin_settings.yml (`discourse_narrative_bot_enabled` and the six
  others, `send_welcome_message` taken over), "discourse-narrative-bot" in `plugins::STARTED`,
  the admin plugin entry with its `robot` icon (src/admin_view.rs), discobot (user -2, its email,
  options, profile and bio) and the Certified and Licensed badges from the clean seed, the
  stylesheet (static/css/discourse-narrative-bot.css).
- The Store (PluginStore rows under the plugin's name, keyed by user id):
  src/plugins/narrative_bot/store.rs.
- Base#input: the transition tables of both narratives with prerequisites, the skip setting,
  `skip`, state data, clean up and init hooks, the end (end reply, timeout cancelled, badge, final
  state), the DistributedMutex per user and track (an advisory lock), the state rolled back on an
  error; every action of NewUserNarrative (hello, bookmark, onebox, emoji, mention, formatting,
  quote, images with its like, likes, flag, search with the hidden answer revised into the first
  post and reverted after) and AdvancedUserNarrative (start, edit with the user's post, delete,
  recover with the user's deleted post, category hashtag, notification level, poll, details);
  `reset_bot`, `notify_timeout`, the certificate iframe: src/plugins/narrative_bot/narrative.rs.
- Actions: discobot's replies (`reply_to` without validations, emails skipped unless asked),
  `reset_rate_limits` (the create limiters given back twice per topic and state), the timeout job
  enqueued and cancelled, `bot_mentioned?`, `reply_to_bot_post?`, `pm_to_bot?`, `fake_delay` in
  production: src/plugins/narrative_bot/bot.rs.
- TrackSelector: resets (the exact match under 200 characters, the cooked trigger otherwise),
  skip, thanks ending the track with a like, generic replies (two, then silence for 15 minutes),
  `@discobot` commands (roll with its limits, quote, fortune, display help, the hint rationed in
  public topics), `discourse_narrative_bot_disable_public_replies`, the disabled-tips answer,
  topic actions (delete, notification level): src/plugins/narrative_bot/track_selector.rs.
- Jobs BotInput, NarrativeInit, NarrativeTimeout, SendDefaultWelcomeMessage (closed,
  welcome_user or welcome_invite), in the user's effective locale:
  src/plugins/narrative_bot/jobs.rs, src/jobs/handlers.rs.
- plugin.rb's events and callbacks, each guarded by the enabled setting: post_created (with
  `skip_bot`, small actions included), post_edited (revisions and topic revisions),
  post_destroyed, post_recovered, PostAction created (like, inappropriate flag, other flags as
  replies), Bookmark created (posts and topics), topic_notification_level_changed (TopicUser.change
  and update_last_read), user_created, user_first_logged_in (the login paths that log on a user),
  user_unstaged (the password login's unstage), User after_destroy, system_message_sent (the TL2
  invitation, moved here from src/system_message.rs), site_setting_changed for default_locale
  (the bio), `enqueue_bot_welcome_post` and `enqueue_narrative_bot_job?`:
  src/plugins/narrative_bot/mod.rs.
- The `pretty_text_allowed_iframes` modifier (the certificate in opt_input and in cooking) and
  PostGuardianExtension's `can_create_post?` (replies to discobot's system messages):
  src/pretty_text/mod.rs, src/topic_guardian.rs.
- CertificatesController#generate (GET /discobot/certificate(.svg)): requires_plugin, login,
  params, the two rate limits, the SVG templates (vendored) with the avatar, logo, font, date,
  name and titles, cached for a day; the plugin's public images and font under
  /plugins/discourse-narrative-bot: src/routes/narrative_bot.rs,
  src/plugins/narrative_bot/certificate.rs.
- Locales: English through the vendored plugin locales, French (the plugin's server.fr.yml,
  vendored beside core's French).

### Gaps
**Tutorials**
- The flag step cannot be done: core refuses flags in messages (src/flags.rs), so a user can
  only skip it. The bot's side (reply_to_flag, missing_flag removing wrong flags) is ported.
- The recover step cannot be done: core refuses authors recovering their own posts
  (`user_recovered`, src/post_destroyer.rs `recover_post`), so a user can only skip it.
- init_tutorial_recover with `delete_removed_posts_after` below 1 is refused (discobot's
  notify_moderators flag in the message and the reviewable's ignore_and_do_nothing).
- `post_analyzer.cook` in the images step cooks the raw without Oneboxer.apply of cached
  oneboxes: an image URL on its own line counts in Rails once its onebox is cached, not here.
  Rails: plugins/discourse-narrative-bot/lib/discourse_narrative_bot/new_user_narrative.rb:358.

**Events**
- user_unstaged from DiscourseConnect, invite redemption, signup of a staged email and email
  verification: those paths refuse unstaging in core. user_first_logged_in from the user API key
  login (core sends no user_logged_in there either).

**Certificate**
- `Date.parse` reads the forms the plugin's links carry and ISO dates; anything else Ruby reads
  (it is lenient) falls back to today here.

**Locales**
- Every locale but English and French for the plugin's server strings (the port carries core in
  those two only). [item 167]
- The bio after a default_locale change is written as a plain paragraph, refused when the
  translation has markup (the settings update has no cooker at hand).

### Items

None new: the flag and recover steps wait on core (flags in messages, user_recovered), the
locales on 167.

### Couldn't determine
- The order of `bot_input` among a request's jobs: the recorder lists the event handlers' jobs
  before the transaction's after-commit ones; tests/writes.rs compares them apart
  (`plugin_jobs`, sorted). Rails' Sidekiq enqueue is outside the database transaction, so a
  message PostCreator then refuses (writes case posts/pm_group_too_large) still leaves the
  creator's topic_notification_level_changed input; the port's queue rolls back with the
  refusal, and that case ignores `plugin_jobs`.
- The body Rails answers a logged-out or disabled /discobot/certificate.svg with (the spec only
  checks the 404); the port sends its JSON not-found.
- The Cache-Control of the plugin's public files (nginx serves them in production); the port
  sends a year, public, immutable.
- Rails url helpers under a subfolder (`user_url`, `guidelines_url`, `certificate_path`): the
  port puts `Discourse.base_url` (which has the subfolder) in front of the plain paths.
## discourse-templates (`plugins/discourse-templates`)

Verdict: started (8 gaps). Added on `feat/plugin-templates`, against the
plugin at the vendored bf55a44c.

### What the port has
- Settings (`discourse_templates_enabled`, `_categories`, `_max_replies_fetched`, `_enable_private_templates`, `_groups_allowed_private_templates`, `_private_templates_tags`) from the vendored plugin yml; enabling allowed (`STARTED`): src/plugins/mod.rs.
- GuardianExtension and UserExtension (`can_use_templates?`, `can_use_category_templates?` with its first-0-or-missing refusal, `can_use_private_templates?` with staff and everyone): src/plugins/templates/mod.rs.
- TopicExtension: `template?(user)` (templates categories and their subcategories, or the author's own message tagged with a private templates tag while tagging and private templates are on), `template_item_usage_count`, `increment_template_item_usage_count!` on `discourse_templates_usage_count`: src/plugins/templates/mod.rs.
- TopicQueryExtension (`list_category_templates`: allowed categories, no definitions, visible and not archived, by title; `list_private_templates`: the member's own messages, not archived, joined to the tags; both with `prioritize_pinned_topics` and the per_page limit) and TemplatesSerializer (id, title, slug, content, visible tags sorted while tagging is on, usages): src/plugins/templates/mod.rs.
- TemplatesController (`GET /discourse_templates`, `POST /discourse_templates/:id/use` with its 422s, 403s and the rows-written body; requires_plugin, ensure_logged_in, CSRF): src/routes/discourse_templates.rs; src/routes/mod.rs.
- `add_to_serializer(:current_user, :can_use_templates)` and `(:topic_view, :is_template)`: src/current_user.rs; src/topic_view.rs.
- Replacement variables (replace-variables.js with JS replacement-string semantics, the composer and chat channel extractors, `my_name` as Ember's displayName), computed on the server from the insert's context (GET /live/templates): src/plugins/templates/variables.rs.
- UI: the composer options menu item; the insert template modal (its own container beside other modals) and the composer preview panel on desktop with the preview shown; the filterable list (keyword filter scoring title over content, the TagDrop with no-tags and remove-filter, relevance, usage and title ordering, the selected tag kept in the keyValueStore and dropped when gone) refreshed through htmx; the items (apply, source link, cooked preview); the `mod+shift+i` shortcut (composer, chat composer, any textarea); the chat composer dropdown button (through a generic `Discourse.chatComposerButtons` registry in chat.js); the topic's Copy and New topic from template controls (the options header's category and tags read on the server); the stylesheet: src/plugins/templates/view.rs; static/js/templates.js; static/css/discourse-templates.css; static/js/composer.js; static/js/chat.js; src/html.rs; templates/layout.html.
- Tests: tests/plugin_templates.rs; unit tests in src/plugins/templates/.

### Gaps
**plugin.rb and models**
- `has_one :template_item_usage, dependent: :destroy`: the port never destroys topics, so nothing to do yet.
- `lib/tasks/discourse-templates.rake` (migrate from and purge discourse-canned-replies) and the `RenameCategorySetting` data migration: absent; the port has no rake tasks.

**UI**
- The chat thread composer's variables (extractVariablesFromChatThread, `#thread-composer`): the thread composer exists since 160, its variables are not wired.
- The chat composer dropdown shows only the plugins' buttons: core's upload, GIFs, local dates and mobile emoji items are not ported (Chat, UI).
- The keyboard shortcuts help modal's "Templates (within a textarea)" entry: the help modal is not ported.
- The editor-preview connector's empty `div.d-templates` is created when the panel first opens, not drawn with the composer.
- The modal does not close on page change (`page:changed`).
- The tag drop's filter is matched in the browser over the rows the server drew, without select-kit's keyboard navigation.

### Items

None new; the thread composer exists since 160.

### Couldn't determine
- The core Ember sources are not in the extracted reference: select-kit TagDrop's shortcut rows order and its header label for no-tags, DCookText's wrapper markup, `User#displayName` and whether `composer.openNewTopic` falls back to the default category were written from memory.
- `last_poster_username` comes from the topic's last poster here; Ember reads it off the client's topic model, which a topic loaded directly (not from a list) may not carry.
- The template item's place in the composer options menu (before core's wrap toggle here) needs a Rails response.
## styleguide (`plugins/styleguide`)

Verdict: started (7 gaps). Ported on `feat/plugin-styleguide` (2026-10-29), against
bf55a44c read from the extract at tmp/rails-bf55a44c; tests in tests/plugin_styleguide.rs
(spec/requests/styleguide/styleguide_controller_spec.rb, spec/integration/access_spec.rb,
spec/integration/assets_spec.rb's page checks) and the section order's unit test in
src/plugins/styleguide/mod.rs.

The plugin is Ember all the way down: Rails renders an empty page at /styleguide and the
client draws ~50 sections of live Ember components. The port keeps the routes, the guard
and the section structure, and shows its own server-rendered markup where it has an
equivalent.

### What the port has
- Routes `/styleguide` and `/styleguide/:category/:section` (StyleguideController#index):
  404 while `styleguide_enabled` is off (requires_plugin) and for whoever
  `can_see_styleguide?` refuses (`in_any_groups?` over `styleguide_allowed_groups`,
  admins by default, with the anonymous and everyone group rules): src/routes/styleguide.rs,
  src/plugins/styleguide/mod.rs.
- `add_to_serializer(:site, :can_see_styleguide)`, present only while the plugin is on:
  src/site.rs.
- lib/styleguide.js's categories and sections in its order (priority, then id),
  `sectionById` by id whatever the category, an unknown section's 404, the page header
  with its breadcrumbs (Styleguide, then the section) and title, the index's welcome, the
  navigation by category: src/plugins/styleguide/view.rs.
- Sections drawn with the port's markup: Typography (h1 to h6, p), Font scale (its text,
  the SCSS samples and the 13 steps), Buttons (every DButton size and state card and
  DToggleSwitch, with the port's button markup), Colors (every swatch card), Icons (the
  site sprite's symbols), Forms (Input, Toggle, Textarea, Select, CheckboxGroup, Alert with
  the port's FormKit markup, src/admin_form_kit.rs), Spinners.
- The plugin's stylesheet, on its pages only (the asset filter's effect, with
  `data-target="styleguide"`): static/css/styleguide.css.
- Setting `styleguide_admin_only` (hidden) and the migration to
  `styleguide_allowed_groups` are data the vendored settings and a restored database carry.

### Gaps
- What can't map: the other 43 sections showcase Ember components and services the
  server-rendered pages do not have, or have only as page-specific markup: BEM (syntax),
  OTP, Date/time inputs, Dropdowns (select-kit's combo boxes, choosers, notifications
  dropdowns), Topic link status, Shortcut, Segmented control, Bread crumbs, Categories,
  Char counter, Combo button, Empty state, Navigation bar, Navigation stacked, Post menu,
  Roving focus, Tooltips, Menus, Context menu, Multi select, Toasts, Dialog, Drag and drop,
  Signup CTA, Topic list item, Topic notifications, Topic timer info, Virtual list, Tabs,
  Overflow controls, Post, Post list, Post oneboxes, Topic map, Topic footer buttons, Topic
  list, Basic topic list, Categories list, Docked composer, Modal, Navigation, Site header,
  More topics. Each shows a note instead; the faker-backed dummy data (lib/dummy-data.js)
  they render from is not ported.
- The Forms section's FormKit examples beyond the six above (Question, Composer, Code,
  Image, Icon, TagChooser, Menu, RadioGroup, the Color variants, Section, InputGroup,
  Collection, Row/Col, Multiline, validation) and the inputs' before/after affixes.
- The styleguide sidebar panel taking over the main sidebar (styleguide-sidebar service,
  its filter): the port draws the same categories and links beside the contents.
- ToggleColorMode in the breadcrumb row, the groups and `?group=` sub-navigation
  (styleguide-groups, styleguide-subnav), the example cards' code toggle with the Ember
  source, `@description`, `@tryThis`, `@note` and `@kind`.
- The Icons section lists the sprite's symbols without the icon library's REPLACEMENTS
  aliases.
- `addSection` and the `styleguide` plugin API (pre-initializers/styleguide-plugin-api.js)
  for other plugins' sections.
- The keyboard shortcut and the "Open styleguide" admin link.

### Couldn't determine
- An unknown section: Rails answers 200 with the empty page and the client then shows the
  404 route; the server-rendered port answers the 404 page directly.
- Button markup is the port's own DButton rendering; its exact classes for the link
  display were not checked against a running Ember.
## discourse-math (`plugins/discourse-math`)

Verdict: started (5 gaps). Ported on `feat/plugin-math` (2026-10-28), against bf55a44c read
from the extract at tmp/rails-bf55a44c; tests in tests/pretty_text.rs
(`math_cooks_as_rails_when_enabled`, every case of spec/pretty_text_spec.rb) and
tests/plugin_math.rs.

### What the port has
- Markdown: `$...$` with its boundary rules (whitespace, ASCII and Unicode punctuation and
  symbols, the CJK, full-width, Arabic and Thai marks), `\(...\)` ahead of the escape rule,
  `%...%` asciimath (with `discourse_math_enable_asciimath` and the MathJax provider only),
  single and multi-line `$$` and `\[` blocks interrupting paragraphs, `span.math`,
  `span.asciimath`, `div.math` on the allow list, gated by `discourse_math_enabled` and
  `discourse_math_enable_latex_delimiters`; the chat engine takes the same rules
  (plugin.rb's `enable_markdown_feature`): crates/markdown/src/render/math.rs;
  src/pretty_text/mod.rs.
- Client: lib/math-renderer.js as static/js/discourse-math.js: the MathJax config
  (`ui/safe`, html or svg output, menu, accessibility `enrich`, zoom 175% on click,
  `input/asciimath`), the hidden wrappers, the preview's 200ms delay, KaTeX with mhchem and
  copy-tex, its `trust` and `\eqref`/`\ref`/`\label` macros; run on cooked posts (and what
  htmx swaps in, through cooked.js), the composer preview and chat messages. The settings
  ride on the script tag: src/plugins/math.rs, templates/layout.html.
- The composer's Insert Math (`square-root-variable`, Shift+M), block mode at a line's start,
  the modal with its block toggle and required field: src/composer_view.rs,
  static/js/composer.js, static/js/discourse-math.js. The options menu no longer refuses
  to open while the plugin is on.
- Stylesheets (common and chat): static/css/discourse-math.css.
- Admin plugin entry with its icon: src/admin_view.rs (already there).
- The renderers: the discourse_math_bundle gem 1.0.1 (MathJax 4.1.0, KaTeX 0.16.27)
  vendored verbatim in static/vendor/discourse-math-bundle, embedded by build.rs and served
  at BundlePaths' /plugins/discourse-math/1.0.1/{mathjax,katex}/... with nginx.sample.conf's
  `^/plugins/` caching (a year, public, immutable, any origin), plugin on or off;
  `chtml.fontURL` at the bundle's woff-v2. `add_to_serializer(:site,
  :discourse_math_bundle_url)` while the plugin is on, after core's keys (plugin load order
  puts it after discourse-gamification's and before styleguide's on main). The client takes
  the same url, under the base path, from the script tag. Rails:
  lib/discourse_math/bundle_paths.rb, lib/math-bundle-paths.js, lib/math-renderer.js. Port:
  src/plugins/math.rs, src/site.rs, tests/plugin_math.rs.

### Gaps
- MathJax's `loader.paths.fonts` is pinned to the bundle; Rails leaves MathJax's default, a
  third-party CDN, where the browser fetches the font's dynamic glyph ranges the gem does not ship.
  Here those ranges 404 instead.
- The rich editor extension and its math node, the edit modal (modal/math-edit.gjs).
  [item 196]
- The insert modal's form is drawn by hand, not FormKit; "Required" is its only
  validation message.
- The options menu item's place among the plugins' items is not known (after local dates
  here).
- Non-English client strings. [item 167]

### Couldn't determine
- The exact cooked bytes: the spec compares with match_html and the reference has the
  plugin off, so no recorded corpus entry has math. The port's sanitizer writes the rule's
  single-quoted class double quoted, as it does every attribute.
