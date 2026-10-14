//! UserMerger (app/services/user_merger.rb) and Jobs::MergeUser: one user
//! folded into another. The username rewritten everywhere (the update
//! job inline), the rows of about sixty tables moved to the target where
//! the target has no matching row, likes, timings and visits merged,
//! username site settings repointed, the users' and profiles' columns
//! combined, the plugins' `merging_users` listeners (poll, topic voting,
//! gamification), the stats recounted, then the source destroyed through
//! UserDestroyer (quietly, as system) and the merge logged.
//!
//! Refused: a source with posts (PostOwnerChanger, which revises each
//! post to its new owner, is not ported), and the `/merge_user` progress
//! messages' AdminDetailedUserSerializer payload where the port cannot
//! serialize the user.

use serde_json::{Value, json};
use sqlx::PgConnection;

use crate::guardian::Guardian;
use crate::posting::Ctx;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

/// `Jobs::MergeUser`
pub const MERGE_USER: &str = "merge_user";
/// `UserHistory.actions[:merge_user]`
const MERGE_USER_ACTION: i32 = 65;
/// `Discourse::SYSTEM_USER_ID`
const SYSTEM_USER_ID: i32 = -1;

/// `Guardian#can_merge_user?(source)`: an admin, and the source not staff.
pub async fn can_merge_user(
    conn: &mut PgConnection,
    guardian: &Guardian,
    user_id: i32,
) -> Result<bool, AppError> {
    if !guardian.is_admin() {
        return Ok(false);
    }
    let staff: Option<bool> =
        sqlx::query_scalar("SELECT admin OR moderator FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(conn)
            .await?;
    Ok(staff == Some(false))
}

/// A `/merge_user` progress message to the acting user.
async fn progress(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    acting: Option<i32>,
    key: &str,
) -> Result<(), AppError> {
    let Some(acting) = acting else {
        return Ok(());
    };
    let message = ctx.t(&format!("admin.user.merge_user.{key}"));
    ctx.bus
        .publish(
            &mut *conn,
            "/merge_user",
            &json!({ "message": message }),
            Some(&[crate::bus::user_tag(acting)]),
        )
        .await?;
    Ok(())
}

/// `update_user_id(table, opts)`: the source's rows moved to the target
/// where the target has none matching `conditions`.
async fn update_user_id(
    conn: &mut PgConnection,
    table: &str,
    column: &str,
    conditions: &[&str],
    updates: &[&str],
    source: i32,
    target: i32,
) -> Result<(), AppError> {
    let mut set = vec![format!("{column} = :target_user_id")];
    set.extend(updates.iter().map(|u| u.to_string()));
    let mut wheres = vec![format!("(y.{column} = :target_user_id)")];
    wheres.extend(conditions.iter().map(|c| format!("({c})")));
    let sql = format!(
        "UPDATE {table} AS x SET {} WHERE x.{column} = :source_user_id AND NOT EXISTS(\
           SELECT 1 FROM {table} AS y WHERE {})",
        set.join(", "),
        wheres.join(" AND ")
    );
    exec(conn, &sql, source, target).await
}

/// Runs `sql` with `:source_user_id` and `:target_user_id` bound.
async fn exec(
    conn: &mut PgConnection,
    sql: &str,
    source: i32,
    target: i32,
) -> Result<(), AppError> {
    let sql = sql
        .replace(":source_user_id", "$1")
        .replace(":target_user_id", "$2");
    sqlx::query(&sql)
        .bind(source)
        .bind(target)
        .execute(conn)
        .await?;
    Ok(())
}

/// `UserMerger.new(source, target, acting).merge!`
pub async fn merge(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    source: i32,
    target: i32,
    acting: Option<i32>,
) -> Result<(), AppError> {
    let s = ctx.settings;
    #[derive(sqlx::FromRow)]
    struct U {
        username: String,
        uploaded_avatar_id: Option<i32>,
        email: Option<String>,
    }
    let load = |id: i32| {
        sqlx::query_as::<_, U>(
            "SELECT username, uploaded_avatar_id, \
                    (SELECT email FROM user_emails WHERE user_id = u.id AND \"primary\") AS email \
             FROM users u WHERE id = $1",
        )
        .bind(id)
    };
    let source_user = load(source)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or(Unsupported(
            "merging a missing user (NoMethodError in Rails)",
        ))?;
    let target_user = load(target)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or(Unsupported(
            "merging into a missing user (NoMethodError in Rails)",
        ))?;

    // update_username
    if source_user.username != target_user.username {
        progress(&mut *conn, ctx, acting, "updating_username").await?;
        let urls = crate::url::Urls {
            config: ctx.config,
            settings: s,
        };
        let avatar_template = crate::avatar::avatar_template(
            &urls,
            target,
            &target_user.username,
            target_user.uploaded_avatar_id,
            None,
        )
        .map_err(|e| match e {
            crate::avatar::AvatarError::Unsupported(u) => AppError::from(u),
            other => AppError::from(other),
        })?;
        if target == SYSTEM_USER_ID
            && s.get("use_site_small_logo_as_system_avatar")?.truthy()
            && s.get("logo_small")?.to_i() != 0
        {
            return Err(
                Unsupported("merging into the system user with the site logo avatar").into(),
            );
        }
        crate::username_changer::update_username(
            &mut *conn,
            ctx,
            &json!({
                "user_id": source,
                "old_username": source_user.username,
                "new_username": target_user.username,
                "avatar_template": avatar_template,
            }),
        )
        .await?;
    }

    // move_posts
    let posts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM posts WHERE user_id = $1")
        .bind(source)
        .fetch_one(&mut *conn)
        .await?;
    if posts > 0 {
        return Err(Unsupported("merging a user with posts (PostOwnerChanger)").into());
    }

    update_user_ids(&mut *conn, ctx, acting, source, target).await?;

    // merge_given_daily_likes
    progress(&mut *conn, ctx, acting, "merging_given_daily_likes").await?;
    sqlx::query(
        "INSERT INTO given_daily_likes AS g (user_id, likes_given, given_date, limit_reached) \
           SELECT $2 AS user_id, COUNT(1) AS likes_given, a.created_at::DATE AS given_date, \
                  COUNT(1) >= $3 AS limit_reached \
           FROM post_actions AS a \
           WHERE a.user_id = $2 AND a.deleted_at IS NULL \
             AND EXISTS(SELECT 1 FROM given_daily_likes AS g \
                        WHERE g.user_id = $1 AND a.created_at::DATE = g.given_date) \
           GROUP BY given_date \
         ON CONFLICT (user_id, given_date) DO UPDATE \
           SET likes_given = EXCLUDED.likes_given, limit_reached = EXCLUDED.limit_reached",
    )
    .bind(source)
    .bind(target)
    .bind(s.get("max_likes_per_day")?.to_i())
    .execute(&mut *conn)
    .await?;

    // merge_post_timings
    progress(&mut *conn, ctx, acting, "merging_post_timings").await?;
    update_user_id(
        &mut *conn,
        "post_timings",
        "user_id",
        &["x.topic_id = y.topic_id", "x.post_number = y.post_number"],
        &[],
        source,
        target,
    )
    .await?;
    exec(
        &mut *conn,
        "UPDATE post_timings AS t SET msecs = LEAST(t.msecs::bigint + s.msecs, 2^31 - 1) \
         FROM post_timings AS s \
         WHERE t.user_id = :target_user_id AND s.user_id = :source_user_id \
           AND t.topic_id = s.topic_id AND t.post_number = s.post_number",
        source,
        target,
    )
    .await?;

    // merge_user_visits
    progress(&mut *conn, ctx, acting, "merging_user_visits").await?;
    update_user_id(
        &mut *conn,
        "user_visits",
        "user_id",
        &["x.visited_at = y.visited_at"],
        &[],
        source,
        target,
    )
    .await?;
    exec(
        &mut *conn,
        "UPDATE user_visits AS t SET posts_read = t.posts_read + s.posts_read, \
           mobile = t.mobile OR s.mobile, time_read = t.time_read + s.time_read \
         FROM user_visits AS s \
         WHERE t.user_id = :target_user_id AND s.user_id = :source_user_id \
           AND t.visited_at = s.visited_at",
        source,
        target,
    )
    .await?;

    // update_site_settings
    progress(&mut *conn, ctx, acting, "updating_site_settings").await?;
    update_site_settings(
        &mut *conn,
        ctx,
        &source_user.username,
        &target_user.username,
    )
    .await?;

    // merge_user_attributes
    progress(&mut *conn, ctx, acting, "merging_user_attributes").await?;
    exec(
        &mut *conn,
        "UPDATE users AS t \
         SET created_at = LEAST(t.created_at, s.created_at), \
           updated_at = LEAST(t.updated_at, s.updated_at), \
           seen_notification_id = GREATEST(t.seen_notification_id, s.seen_notification_id), \
           last_posted_at = GREATEST(t.last_seen_at, s.last_seen_at), \
           last_seen_at = GREATEST(t.last_seen_at, s.last_seen_at), \
           admin = t.admin OR s.admin, \
           last_emailed_at = GREATEST(t.last_emailed_at, s.last_emailed_at), \
           trust_level = GREATEST(t.trust_level, s.trust_level), \
           previous_visit_at = GREATEST(t.previous_visit_at, s.previous_visit_at), \
           date_of_birth = COALESCE(t.date_of_birth, s.date_of_birth), \
           ip_address = COALESCE(t.ip_address, s.ip_address), \
           moderator = t.moderator OR s.moderator, \
           title = COALESCE(t.title, s.title), \
           primary_group_id = COALESCE(t.primary_group_id, s.primary_group_id), \
           registration_ip_address = COALESCE(t.registration_ip_address, s.registration_ip_address), \
           first_seen_at = LEAST(t.first_seen_at, s.first_seen_at), \
           manual_locked_trust_level = GREATEST(t.manual_locked_trust_level, s.manual_locked_trust_level) \
         FROM users AS s WHERE t.id = :target_user_id AND s.id = :source_user_id",
        source,
        target,
    )
    .await?;
    exec(
        &mut *conn,
        "UPDATE user_profiles AS t \
         SET location = COALESCE(t.location, s.location), \
           website = COALESCE(t.website, s.website), \
           bio_raw = COALESCE(t.bio_raw, s.bio_raw), \
           bio_cooked = COALESCE(t.bio_cooked, s.bio_cooked), \
           bio_cooked_version = COALESCE(t.bio_cooked_version, s.bio_cooked_version), \
           profile_background_upload_id = COALESCE(t.profile_background_upload_id, s.profile_background_upload_id), \
           dismissed_banner_key = COALESCE(t.dismissed_banner_key, s.dismissed_banner_key), \
           granted_title_badge_id = COALESCE(t.granted_title_badge_id, s.granted_title_badge_id), \
           card_background_upload_id = COALESCE(t.card_background_upload_id, s.card_background_upload_id), \
           views = t.views + s.views \
         FROM user_profiles AS s WHERE t.user_id = :target_user_id AND s.user_id = :source_user_id",
        source,
        target,
    )
    .await?;

    // merge_user_associated_accounts
    progress(&mut *conn, ctx, acting, "merging_user_associated_accounts").await?;
    exec(
        &mut *conn,
        "UPDATE user_associated_accounts SET user_id = CASE \
           WHEN EXISTS (SELECT 1 FROM user_associated_accounts AS conflicts \
                        WHERE (conflicts.user_id = :target_user_id \
                               AND conflicts.provider_name = user_associated_accounts.provider_name)) \
           THEN NULL ELSE :target_user_id END \
         WHERE user_associated_accounts.user_id = :source_user_id",
        source,
        target,
    )
    .await?;

    merging_users(&mut *conn, s, source, target).await?;

    // update_user_stats
    progress(&mut *conn, ctx, acting, "updating_user_stats").await?;
    for sql in [
        "UPDATE user_stats SET topics_entered = (SELECT COUNT(topic_id) FROM topic_views \
           WHERE user_id = :target_user_id) WHERE user_id = :target_user_id",
        "UPDATE user_stats SET time_read = COALESCE(x.time_read, 0), \
           days_visited = COALESCE(x.days_visited, 0) \
         FROM (SELECT SUM(time_read) AS time_read, COUNT(1) AS days_visited \
               FROM user_visits WHERE user_id = :target_user_id) AS x \
         WHERE user_id = :target_user_id",
        "UPDATE user_stats SET posts_read_count = (SELECT COUNT(1) FROM post_timings AS pt \
           WHERE pt.user_id = :target_user_id AND EXISTS(SELECT 1 FROM topics AS t \
             WHERE t.archetype = 'regular' AND t.deleted_at IS NULL)) \
         WHERE user_id = :target_user_id",
        "UPDATE user_stats AS t SET likes_given = t.likes_given + s.likes_given, \
           likes_received = t.likes_received + s.likes_received, \
           new_since = LEAST(t.new_since, s.new_since), \
           read_faq = LEAST(t.read_faq, s.read_faq), \
           first_post_created_at = LEAST(t.first_post_created_at, s.first_post_created_at) \
         FROM user_stats AS s WHERE t.user_id = :target_user_id AND s.user_id = :source_user_id",
    ] {
        exec(&mut *conn, sql, source, target).await?;
    }

    // delete_source_user
    progress(&mut *conn, ctx, acting, "deleting_source_user").await?;
    delete_source_user(&mut *conn, ctx, source).await?;

    // log_merge
    let context = ctx
        .i18n
        .t_with(
            "staff_action_logs.user_merged",
            &[("username", &source_user.username)],
        )
        .ok_or(Unsupported("staff_action_logs.user_merged"))?;
    sqlx::query(
        "INSERT INTO user_histories (action, acting_user_id, target_user_id, context, email, \
                                     admin_only, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, TRUE, clock_timestamp(), clock_timestamp())",
    )
    .bind(MERGE_USER_ACTION)
    .bind(acting.unwrap_or(SYSTEM_USER_ID))
    .bind(target)
    .bind(context)
    .bind(source_user.email.unwrap_or_default())
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// `update_user_ids`
async fn update_user_ids(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    acting: Option<i32>,
    source: i32,
    target: i32,
) -> Result<(), AppError> {
    progress(&mut *conn, ctx, acting, "updating_user_ids").await?;
    // The moves, in Rails' order: a plain `update_all` is a table and a
    // column with no conditions and no NOT EXISTS.
    enum Step {
        All(&'static str, &'static str),
        Merge(&'static str, &'static str, &'static [&'static str]),
    }
    use Step::{All, Merge};
    let mut steps = vec![
        All("categories", "user_id"),
        Merge(
            "category_users",
            "user_id",
            &["x.category_id = y.category_id"],
        ),
        Merge("developers", "user_id", &[]),
        Merge("draft_sequences", "user_id", &["x.draft_key = y.draft_key"]),
        Merge("drafts", "user_id", &["x.draft_key = y.draft_key"]),
        Merge(
            "dismissed_topic_users",
            "user_id",
            &["x.topic_id = y.topic_id"],
        ),
        All("email_logs", "user_id"),
        All("group_histories", "acting_user_id"),
        All("group_histories", "target_user_id"),
        Merge("group_users", "user_id", &["x.group_id = y.group_id"]),
        All("incoming_emails", "user_id"),
        All("incoming_links", "user_id"),
        All("incoming_links", "current_user_id"),
        All("invited_users", "user_id"),
        All("invites", "invited_by_id"),
        All("invites", "deleted_by_id"),
        All("mcp_audit_logs", "user_id"),
        All("mcp_oauth_authorizations", "revoked_by_user_id"),
        Merge(
            "muted_users",
            "user_id",
            &["x.muted_user_id = y.muted_user_id"],
        ),
        Merge("muted_users", "muted_user_id", &["x.user_id = y.user_id"]),
        Merge(
            "ignored_users",
            "user_id",
            &["x.ignored_user_id = y.ignored_user_id"],
        ),
        Merge(
            "ignored_users",
            "ignored_user_id",
            &["x.user_id = y.user_id"],
        ),
        All("notifications", "user_id"),
        Merge(
            "post_actions",
            "user_id",
            &[
                "x.post_id = y.post_id",
                "x.targets_topic = y.targets_topic",
                "x.post_action_type_id = y.post_action_type_id OR (x.post_action_type_id IN (3,4,7,8) AND y.post_action_type_id IN (3,4,7,8))",
            ],
        ),
        All("post_actions", "deleted_by_id"),
        All("post_actions", "deferred_by_id"),
        All("post_actions", "agreed_by_id"),
        All("post_actions", "disagreed_by_id"),
        All("post_revisions", "user_id"),
        All("posts", "deleted_by_id"),
        All("posts", "last_editor_id"),
        All("posts", "locked_by_id"),
        All("posts", "reply_to_user_id"),
        All("reviewables", "created_by_id"),
        All("reviewable_histories", "created_by_id"),
        All("search_logs", "user_id"),
        Merge("tag_users", "user_id", &["x.tag_id = y.tag_id"]),
        All("themes", "user_id"),
        Merge(
            "topic_allowed_users",
            "user_id",
            &["x.topic_id = y.topic_id"],
        ),
        All("topic_embeds", "deleted_by_id"),
        All("topic_links", "user_id"),
        All("topic_link_clicks", "user_id"),
        All("topic_timers", "deleted_by_id"),
        Merge(
            "topic_timers",
            "user_id",
            &[
                "x.status_type = y.status_type",
                "x.timerable_id = y.timerable_id",
                "y.deleted_at IS NULL",
            ],
        ),
        Merge("topic_users", "user_id", &["x.topic_id = y.topic_id"]),
        Merge("topic_views", "user_id", &["x.topic_id = y.topic_id"]),
        All("topics", "deleted_by_id"),
        All("unsubscribe_keys", "user_id"),
        All("uploads", "user_id"),
        Merge(
            "user_archived_messages",
            "user_id",
            &["x.topic_id = y.topic_id"],
        ),
        Merge(
            "user_actions",
            "user_id",
            &[
                "x.action_type = y.action_type",
                "x.target_topic_id IS NOT DISTINCT FROM y.target_topic_id",
                "x.target_post_id IS NOT DISTINCT FROM y.target_post_id",
                "(x.acting_user_id IN (:source_user_id, :target_user_id) OR x.acting_user_id IS NOT DISTINCT FROM y.acting_user_id)",
            ],
        ),
        Merge(
            "user_actions",
            "acting_user_id",
            &[
                "x.action_type = y.action_type",
                "x.user_id = y.user_id",
                "x.target_topic_id IS NOT DISTINCT FROM y.target_topic_id",
                "x.target_post_id IS NOT DISTINCT FROM y.target_post_id",
            ],
        ),
        Merge(
            "user_badges",
            "user_id",
            &[
                "x.badge_id = y.badge_id",
                "x.seq = y.seq",
                "x.post_id IS NOT DISTINCT FROM y.post_id",
            ],
        ),
        Merge(
            "bookmarks",
            "user_id",
            &[
                "x.bookmarkable_type = y.bookmarkable_type",
                "x.bookmarkable_id = y.bookmarkable_id",
            ],
        ),
        All("user_badges", "granted_by_id"),
        Merge("user_custom_fields", "user_id", &["x.name = y.name"]),
    ];
    for step in steps.drain(..) {
        match step {
            All(table, column) => exec(
                &mut *conn,
                &format!(
                    "UPDATE {table} SET {column} = :target_user_id WHERE {column} = :source_user_id"
                ),
                source,
                target,
            )
            .await?,
            Merge(table, column, conditions) => {
                update_user_id(&mut *conn, table, column, conditions, &[], source, target).await?
            }
        }
    }
    // @target_user.human?: the emails, secondary unless the address is
    // the target's already.
    if target > 0 {
        update_user_id(
            &mut *conn,
            "user_emails",
            "user_id",
            &["x.email = y.email OR y.primary = false"],
            &["\"primary\" = false"],
            source,
            target,
        )
        .await?;
    }
    for (table, column) in [
        ("user_exports", "user_id"),
        ("user_histories", "target_user_id"),
        ("user_histories", "acting_user_id"),
        ("user_profile_views", "user_profile_id"),
        ("user_profile_views", "user_id"),
        ("user_warnings", "user_id"),
        ("user_warnings", "created_by_id"),
        ("users", "approved_by_id"),
    ] {
        exec(
            &mut *conn,
            &format!(
                "UPDATE {table} SET {column} = :target_user_id WHERE {column} = :source_user_id"
            ),
            source,
            target,
        )
        .await?;
    }
    Ok(())
}

/// `update_site_settings`: each username setting naming the source names
/// the target, logged by system.
async fn update_site_settings(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    source_username: &str,
    target_username: &str,
) -> Result<(), AppError> {
    let s = ctx.settings;
    // SiteSetting.all_settings(include_hidden: true) measures the upload
    // settings' uploads as it reads them.
    let urls = crate::url::Urls {
        config: ctx.config,
        settings: s,
    };
    crate::admin_site_settings::fix_upload_dimensions(
        &mut *conn,
        &crate::admin_site_settings::Context {
            defs: &ctx.host.site_setting_defs,
            settings: s,
            i18n: ctx.i18n,
            globals: &ctx.config.globals,
            base_path: ctx.config.globals.relative_url_root(),
            urls: &urls,
        },
    )
    .await?;
    for def in ctx.host.site_setting_defs.iter() {
        if def.data_type as i32 != crate::site_settings::DataType::Username as i32 {
            continue;
        }
        let value = s.get(&def.name)?.clone();
        if value.to_s() != source_username {
            continue;
        }
        crate::site_setting_update::set_and_log(
            &mut *conn,
            ctx.bus,
            def,
            &value,
            &crate::site_settings::Value::Str(target_username.to_string()),
            SYSTEM_USER_ID,
        )
        .await?;
    }
    Ok(())
}

/// The bundled plugins' `merging_users` listeners, each when its plugin
/// is enabled.
async fn merging_users(
    conn: &mut PgConnection,
    s: &SiteSettings,
    source: i32,
    target: i32,
) -> Result<(), AppError> {
    if s.get("poll_enabled")?.truthy() {
        // As written: the inner condition compares a column with itself.
        exec(
            &mut *conn,
            "DELETE FROM poll_votes WHERE user_id = :source_user_id AND EXISTS ( \
               SELECT 1 FROM poll_votes WHERE user_id = :target_user_id \
                 AND poll_votes.poll_id = poll_votes.poll_id)",
            source,
            target,
        )
        .await?;
        exec(
            &mut *conn,
            "UPDATE poll_votes SET user_id = :target_user_id WHERE user_id = :source_user_id",
            source,
            target,
        )
        .await?;
    }
    if s.get("topic_voting_enabled")?.truthy() {
        let topics: Vec<i32> = sqlx::query_scalar(
            "SELECT DISTINCT topic_id FROM topic_voting_votes WHERE user_id = $1",
        )
        .bind(source)
        .fetch_all(&mut *conn)
        .await?;
        exec(
            &mut *conn,
            "DELETE FROM topic_voting_votes WHERE user_id = :source_user_id AND EXISTS ( \
               SELECT 1 FROM topic_voting_votes AS target_votes \
               WHERE target_votes.user_id = :target_user_id \
                 AND target_votes.topic_id = topic_voting_votes.topic_id)",
            source,
            target,
        )
        .await?;
        exec(
            &mut *conn,
            "UPDATE topic_voting_votes SET user_id = :target_user_id WHERE user_id = :source_user_id",
            source,
            target,
        )
        .await?;
        if !topics.is_empty() {
            sqlx::query(
                "INSERT INTO topic_voting_topic_vote_count (topic_id, votes_count, created_at, updated_at) \
                 SELECT topic_id, COUNT(*) AS votes_count, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP \
                 FROM topic_voting_votes WHERE topic_id = ANY($1) GROUP BY topic_id \
                 ON CONFLICT (topic_id) DO UPDATE SET votes_count = EXCLUDED.votes_count, \
                   updated_at = CURRENT_TIMESTAMP",
            )
            .bind(&topics)
            .execute(&mut *conn)
            .await?;
        }
    }
    if s.get("discourse_gamification_enabled")?.truthy() {
        sqlx::query(
            "WITH new_scores AS ( \
               SELECT leaderboard_id, $2::bigint AS user_id, date, SUM(score) AS score \
               FROM gamification_leaderboard_scores WHERE user_id IN ($1, $2) GROUP BY 1, 2, 3 \
             ) INSERT INTO gamification_leaderboard_scores (leaderboard_id, user_id, date, score) \
               SELECT leaderboard_id, user_id, date, score FROM new_scores \
               ON CONFLICT (leaderboard_id, user_id, date) DO UPDATE SET score = EXCLUDED.score",
        )
        .bind(i64::from(source))
        .bind(i64::from(target))
        .execute(&mut *conn)
        .await?;
        sqlx::query("DELETE FROM gamification_leaderboard_scores WHERE user_id = $1")
            .bind(i64::from(source))
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

/// `delete_source_user`: the source's email made unreachable (its
/// address's tokens gone), then UserDestroyer as system, quietly.
async fn delete_source_user(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    source: i32,
) -> Result<(), AppError> {
    let (username, admin): (String, bool) =
        sqlx::query_as("SELECT username, admin FROM users WHERE id = $1")
            .bind(source)
            .fetch_one(&mut *conn)
            .await?;
    // @source_user.update(admin: false, email: ...)
    let email = format!(
        "{username}_{}@no-email.invalid",
        crate::accounts::random_hex()
    );
    let old: Option<String> =
        sqlx::query_scalar("SELECT email FROM user_emails WHERE user_id = $1 AND \"primary\"")
            .bind(source)
            .fetch_optional(&mut *conn)
            .await?;
    if let Some(old) = &old {
        sqlx::query("DELETE FROM email_tokens WHERE email = $1")
            .bind(old)
            .execute(&mut *conn)
            .await?;
        // Saved through the user's autosave, unvalidated: normalized_email
        // stays.
        sqlx::query(
            "UPDATE user_emails SET email = $2, updated_at = clock_timestamp() \
             WHERE user_id = $1 AND \"primary\"",
        )
        .bind(source)
        .bind(&email)
        .execute(&mut *conn)
        .await?;
    } else {
        // `email=` builds a primary email when the merge moved it away.
        sqlx::query(
            "INSERT INTO user_emails (user_id, email, \"primary\", created_at, updated_at) \
             VALUES ($1, $2, TRUE, clock_timestamp(), clock_timestamp())",
        )
        .bind(source)
        .bind(&email)
        .execute(&mut *conn)
        .await?;
    }
    if admin {
        sqlx::query("UPDATE users SET admin = FALSE, updated_at = clock_timestamp() WHERE id = $1")
            .bind(source)
            .execute(&mut *conn)
            .await?;
    }
    crate::user_anonymizer::user_save_callbacks(&mut *conn, ctx.settings, source, false).await?;

    let system = crate::session::current::SessionUser::load(&mut *conn, SYSTEM_USER_ID)
        .await?
        .ok_or(Unsupported("a site without its system user"))?;
    let guardian = Guardian::for_user(&mut *conn, &system).await?;
    let opts = crate::user_destroyer::Options {
        quiet: true,
        ..Default::default()
    };
    match crate::user_destroyer::destroy(&mut *conn, ctx, &guardian, source, &opts).await? {
        crate::user_destroyer::Outcome::Destroyed => Ok(()),
        crate::user_destroyer::Outcome::PostsExist => {
            Err(std::io::Error::other("UserDestroyer::PostsExistError").into())
        }
        crate::user_destroyer::Outcome::Forbidden => {
            Err(std::io::Error::other("Discourse::InvalidAccess").into())
        }
    }
}

/// `Jobs::MergeUser#execute`: the merge, then the result to the acting
/// user.
pub async fn merge_user_job(state: &AppState, args: &Value) -> Result<(), AppError> {
    let id = |key: &str| -> Result<Option<i32>, AppError> {
        Ok(match args.get(key) {
            None | Some(Value::Null) => None,
            Some(v) => Some(
                v.as_i64()
                    .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
                    .ok_or(Unsupported("merge_user arguments that are not ids"))?
                    as i32,
            ),
        })
    };
    let (Some(source), Some(target)) = (id("user_id")?, id("target_user_id")?) else {
        return Err(Unsupported("merge_user without its users (NoMethodError in Rails)").into());
    };
    let current = id("current_user_id")?;
    let mut tx = state.pool.begin().await?;
    let s = SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let host = crate::pretty_text::Host::from_state(state);
    let ctx = Ctx {
        host: &host,
        settings: &s,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    let acting: Option<i32> = match current {
        Some(id) => {
            sqlx::query_scalar("SELECT id FROM users WHERE id = $1")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?
        }
        None => None,
    };
    let Some(acting) = acting else {
        // `current_user.id` on nil after the merge.
        return Err(
            Unsupported("merge_user without an acting user (NoMethodError in Rails)").into(),
        );
    };
    merge(&mut tx, &ctx, source, target, Some(acting)).await?;
    // The finished merge's message carries AdminDetailedUserSerializer.
    let urls = crate::url::Urls {
        config: &state.config,
        settings: &s,
    };
    let session = crate::session::current::SessionUser::load(&mut tx, acting)
        .await?
        .ok_or(Unsupported("merge_user's acting user vanished"))?;
    let guardian = Guardian::for_user(&mut tx, &session).await?;
    let cx = crate::admin_user_show::Context {
        settings: &s,
        defs: &state.site_setting_defs,
        i18n: &state.i18n,
        urls: &urls,
        globals: &state.config.globals,
        development: state.config.rails_env == crate::config::RailsEnv::Development,
    };
    let user = crate::admin_user_show::show(
        &mut tx,
        &cx,
        &guardian,
        target,
        crate::admin_user_show::ShowOptions {
            show: false,
            detailed: true,
        },
    )
    .await?
    .ok_or(Unsupported("merge_user's target vanished"))?;
    state
        .bus
        .publish(
            &mut tx,
            "/merge_user",
            &json!({ "success": "OK", "merged": true, "user": user }),
            Some(&[crate::bus::user_tag(acting)]),
        )
        .await?;
    tx.commit().await?;
    Ok(())
}
