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

## Ready, in porting order

| Job | Every | Missing without it | Size |
|---|---|---|---|
| unsilence_users | 15m | timed silences never end | S |
| purge_expired_ignored_users | 1d | expiring ignores never expire | S |
| ignored_users_summary | 1d | moderators aren't told of widely ignored users | S |
| purge_old_notifications | 1w | notifications are never capped | S |
| top_refresh_today, top_refresh_older | 1h, 1d | /top reads top_topics nothing writes | M–L |
| update_topic_hot_scores | 10m | /hot reads topic_hot_scores nothing writes | M |
| category_stats | 1d | topics_week/month/year and /categories counts go stale | M |
| periodical_updates, calculate_scores | 15m, 1w | featured topics, post/topic scores, percent_rank, rebakes | L |
| update_heat_settings | 1mo | topic list heat thresholds never adapt | S |
| Chat::PeriodicalUpdates | 15m | channel and thread counts drift | S |
| Chat::AutoJoinUsers | 1h | batch channel auto-join and removals never run | M |
| Chat::DeleteOldMessages | daily | public chat is never pruned (DM half refused) | M |
| DiscourseReactions::ScheduledLikeSynchronizer | 1h | reactions and likes drift apart | M |
| tl3_promotions | daily 4h | TL3 is never granted or revoked | M |
| pending_reviewables_reminder | 15m | no PM about stale flags (last id kept in Redis) | M |
| reviewable_priorities | 1d | review priority thresholds never adapt | S |
| auto_queue_handler | 1d | old flags never auto-ignored (flagged posts only) | M |
| destroy_old_deletion_stubs | 30m | "(post deleted by author)" stubs stay | M |
| destroy_old_hidden_posts | 1d | old hidden posts stay | M |
| activation_reminder_emails | 2h | no activation reminders (template needed) | S–M |
| invalidate_inactive_admins | 1d | inactive admins stay active | S |
| clean_up_api_keys_max_life, clean_up_unused_api_keys | 1d | API keys never auto-revoked | S |
| clean_up_email_tokens, clean_up_email_login_codes, clean_up_email_change_requests, clean_up_unsubscribe_keys, clean_up_post_reply_keys, clean_up_email_logs, delete_rejected_emails | 1d–1w | expired tokens, keys and logs pile up | S |
| clean_dismissed_topic_users, clean_up_drafts, clean_up_bookmarks, clean_up_user_auth_tokens, clean_up_search_logs, clean_up_tags | 1d–1w | housekeeping | S |
| clean_up_unmatched_emails, clean_up_unmatched_ips | 1d | stale screened emails and IPs stay | S/M |
| create_recent_post_search_indexes, reindex_search | 1d, 2h | search's recent-posts offset and stale index rows | S/M |
| update_animated_uploads, backfill_dominant_colors | 1h, 15m | upload metadata never backfilled | S |
| clean_up_uploads, purge_deleted_uploads | 1h, 1d | orphaned uploads stay | L |
| ensure_db_consistency | 12h | denormalized counters drift | L |

## Blocked

| Jobs | Needs |
|---|---|
| badge_grant, ensure_badges_consistency, grant_anniversary_badges, grant_new_user_of_the_month_badges, process_badge_backlog | badges |
| topic_timer_enqueuer | topic timers |
| process_shelved_notifications, process_user_notification_schedules | do not disturb |
| enqueue_digest_emails | digest emails |
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
