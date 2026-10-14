# Scheduled jobs

Rails' scheduled jobs (`app/jobs/scheduled/`, and those of the bundled
plugins the port ports: chat and discourse-reactions) against the port's
`SCHEDULED` list in src/jobs/mod.rs. 113 jobs: 108 core, 5 plugin.

Classes:

- **ported**: in `SCHEDULED`, with write cases where it writes tables
  (parity/writes/scheduled, through the harness's `enqueue` and
  `run_jobs`).
- **ready**: everything it touches exists in the port; what is missing
  today without it is noted.
- **blocked**: needs a feature the port doesn't have yet.
- **infra**: Rails-internal housekeeping with no counterpart.

## Ported

| Job | Every | Cases |
|---|---|---|
| regenerate_sitemaps | 1h | sitemap tests |
| Chat::EmailNotifications | 5m | chat/summary_email* |
| presence_channel_auto_leave | 60s | tests/presence.rs |
| bookmark_reminder_notifications | 5m | scheduled/bookmark_reminder_* |
| unsilence_users | 15m | scheduled/unsilence_users* |
| purge_expired_ignored_users | 1d | scheduled/purge_expired_ignored_users |
| purge_old_notifications | 1w | scheduled/purge_old_notifications* |
| top_refresh_today, top_refresh_older | 1h, 1d | scheduled/top_refresh_* |
| update_topic_hot_scores | 10m | scheduled/hot_scores* |
| category_stats | 1d | scheduled/category_stats* |
| Chat::PeriodicalUpdates | 15m | scheduled/chat_periodical_updates* |
| Chat::AutoJoinUsers | 1h | scheduled/chat_auto_* |
| Chat::DeleteOldMessages | daily (from the worker's start; Rails: midnight) | scheduled/chat_delete_old_messages* |
| DiscourseReactions::ScheduledLikeSynchronizer | 1h | scheduled/like_sync_* |
| tl3_promotions | 1d (from the worker's start; Rails: 4am) | scheduled/tl3_* |
| reviewable_priorities | 1d | scheduled/reviewable_priorities* |
| auto_queue_handler | 1d (flagged posts; queued posts and users refused) | scheduled/auto_queue_* |
| destroy_old_deletion_stubs | 30m | scheduled/deletion_stubs |
| destroy_old_hidden_posts | 1d (posts in messages refused) | scheduled/hidden_posts* |
| activation_reminder_emails | 2h | scheduled/activation_reminder* |
| invalidate_inactive_admins | 1d (pending reviews, shadows and associated accounts refused) | scheduled/inactive_admins* |
| clean_up_api_keys_max_life, clean_up_unused_api_keys | 1d | scheduled/api_keys_* |
| clean_up_email_tokens, clean_up_email_login_codes, clean_up_email_change_requests, clean_up_unsubscribe_keys, clean_up_post_reply_keys, clean_up_email_logs | 1d | scheduled/{email_tokens,email_login_codes,email_change_requests,unsubscribe_keys,post_reply_keys,email_logs}_* |
| delete_rejected_emails | 1w | scheduled/rejected_emails_purge |
| enqueue_digest_emails | 30m | scheduled/digest_enqueue* |
| clean_dismissed_topic_users | 1d | scheduled/dismissed_topic_users* |
| clean_up_drafts, clean_up_bookmarks, clean_up_user_auth_tokens, clean_up_search_logs | 1w | scheduled/{drafts,bookmarks,user_auth_tokens}_cleanup, scheduled/search_logs_* |
| clean_up_tags | 1d (tags on live topics and synonyms refused) | scheduled/tags_cleanup* |
| clean_up_unmatched_emails, CleanUpUnmatchedIPs (with the roll-up) | 1d | scheduled/unmatched_* |
| update_heat_settings | 1mo (from the worker's start) | scheduled/heat_* |
| reindex_search | 2h | search/reindex_* |
| calculate_scores | 1w | scheduled/scores_* |
| periodical_updates | 15m (tag descriptions with text and auto bumps refused) | scheduled/periodical_* |
| ensure_db_consistency | 12h (expired banners, category definitions and unused avatar thumbnails refused) | scheduled/consistency_* |
| update_animated_uploads, backfill_dominant_colors | 1h, 15m (remote uploads refused) | scheduled/animated_uploads_*, scheduled/dominant_colors_* |
| clean_up_uploads, purge_deleted_uploads | 1h, 1d | scheduled/uploads_cleanup*, tests/jobs.rs (the throttle and the tombstone purge) |
| create_recent_post_search_indexes | 1d (CREATE INDEX CONCURRENTLY can't be recorded in a transaction) | search/recent_index_small_site, tests/jobs.rs |

## Ready, in porting order

None: every job whose pieces the port has is ported. The rest wait on
the features below (ROADMAP.md's queue has them).

## Blocked

| Jobs | Needs |
|---|---|
| badge_grant, ensure_badges_consistency, grant_anniversary_badges, grant_new_user_of_the_month_badges, process_badge_backlog | badges |
| topic_timer_enqueuer | topic timers |
| ignored_users_summary, pending_reviewables_reminder | messages to a group (PostCreator target_group_names; group message alerts are refused) |
| process_shelved_notifications, process_user_notification_schedules | do not disturb |
| directory_refresh | the user directory |
| about_stats | the /about page |
| purge_unactivated, clean_up_inactive_users, clean_up_unused_staged_users | deleting users (UserDestroyer) |
| pending_queued_posts_reminder, pending_users_reminder, enqueue_suspect_users | queued posts, user review |
| fix_user_usernames_and_groups_names_clash | username changes |
| aggregate_web_hooks_events, cleanup_redelivering_web_hook_events, redeliver_web_hook_events, purge_old_web_hook_events, purge_old_web_hook_events_daily_aggregate | web hooks |
| clean_up_unused_user_api_keys, clean_up_user_api_keys_max_life, clean_up_unused_registered_user_api_key_clients | user API keys |
| clear_expired_impersonations | impersonation |
| clean_up_associated_accounts, clean_up_associated_groups | external auth |
| check_out_of_date_themes | themes |
| check_translation_overrides | translation overrides |
| schedule_backup, drop_backup_schema | backups |
| ensure_s3_uploads_existence | S3 |
| clean_up_exports | user exports |
| clean_up_mcp_records | the MCP server |
| create_missing_avatars | gravatar downloads |
| poll_mailbox | POP3 (mail arrives through /admin/email/handle_mail) |
| recalculate_nested_hot_scores, backfill_nested_reply_stats | the nested replies view |
| flush_browser_pageview_events, clean_up_browser_pageview_events, maintain_browser_pageview_rollups, detect_crawler_pageviews, clean_up_crawler_stats | pageview analytics |
| maintain_category_activity_daily_rollups, rebuild_category_activity_daily_rollups, maintain_user_visit_daily_rollups, warm_dashboard_reports | the admin dashboard and reports |
| run_problem_checks, cleanup_problem_check_trackers, notify_admins_of_problems | admin problem checks |

## Infra

purge_old_mini_scheduler_stat, heartbeat, enqueue_onceoffs,
clean_up_stylesheet_cache, call_discourse_hub, check_new_features,
check_upcoming_changes, notify_admins_of_available_upcoming_changes,
migrate_upload_scheme, old_keys_reminder.
