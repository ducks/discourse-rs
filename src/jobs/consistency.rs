//! `Jobs::EnsureDbConsistency` (every 12 hours): each model's
//! ensure_consistency!, the denormalized counts and links put back in step
//! with the rows they summarize.
//!
//! Each step runs on its own; one that fails is logged and the next runs,
//! as Rails rescues each. What the port refuses is checked first, so a
//! refused run writes nothing.

use chrono::{Duration, NaiveDateTime};
use rand::Rng;
use sqlx::PgConnection;

use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

/// `Jobs::EnsureDbConsistency`, every 12 hours.
pub const ENSURE_DB_CONSISTENCY: &str = "ensure_db_consistency";

/// `UserStat::UPDATE_UNREAD_USERS_LIMIT`, and GroupUser's.
const UNREAD_USERS_LIMIT: i64 = 10_000;

/// `Jobs::EnsureDbConsistency#execute`
pub async fn ensure_db_consistency(state: &AppState) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    let s = SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let now = crate::clock::now_naive();
    let last_seen = now - Duration::hours(13);
    let ten_minutes_ago = now - Duration::minutes(10);
    preflight(&mut conn, &s, &state.i18n, now).await?;

    let c = &mut *conn;
    logged(ENSURE, "UserVisit", user_visit(c).await)?;
    logged(ENSURE, "Group", group(c, &state.i18n).await)?;
    logged(ENSURE, "Notification", notification(c).await)?;
    logged(
        ENSURE,
        "TopicFeaturedUsers",
        super::handlers::featured_users(c, None).await,
    )?;
    logged(ENSURE, "PostRevision", post_revision(c).await)?;
    logged(ENSURE, "Topic", topic(c, now).await)?;
    logged(ENSURE, "Badge", badge(c).await)?;
    logged(ENSURE, "CategoryUser", category_user(c).await)?;
    logged(ENSURE, "UserOption", user_option(c, &s).await)?;
    logged(ENSURE, "Tag", tag(c).await)?;
    logged(ENSURE, "CategoryTagStat", category_tag_stat(c).await)?;
    logged(ENSURE, "User", user(c).await)?;
    logged(ENSURE, "UserAvatar", user_avatar(c).await)?;
    logged(ENSURE, "UserEmail", user_email(c, &s).await)?;
    logged(ENSURE, "Category", category(c).await)?;
    logged(ENSURE, "TopicThumbnail", topic_thumbnail(c).await)?;
    logged(ENSURE, "UserAction", user_action(c, last_seen).await)?;
    logged(
        ENSURE,
        "UserStat",
        user_stat(c, &s, last_seen, ten_minutes_ago).await,
    )?;
    logged(
        ENSURE,
        "GroupUser",
        group_user(c, &s, last_seen, ten_minutes_ago).await,
    )?;
    Ok(())
}

const ENSURE: &str = "ensure_db_consistency";

/// Rails' per-step `rescue StandardError`: logged, and the job goes on;
/// what the port refuses still stops it.
fn logged(job: &str, step: &str, result: Result<(), AppError>) -> Result<(), AppError> {
    match result {
        Err(e) if !e.is_unsupported() => {
            tracing::error!(job, step, error = %e, "ensure_consistency! failed");
            Ok(())
        }
        other => other,
    }
}

/// The branches the port refuses, checked before any step writes.
async fn preflight(
    conn: &mut PgConnection,
    s: &SiteSettings,
    i18n: &crate::i18n::I18n,
    now: NaiveDateTime,
) -> Result<(), AppError> {
    crate::roles::check_automatic_groups(conn, i18n).await?;
    let refuse = |found: bool, reason: &'static str| -> Result<(), AppError> {
        if found {
            return Err(Unsupported(reason).into());
        }
        Ok(())
    };
    // Topic: expired banners (Topic#remove_banner!).
    refuse(
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM topics WHERE bannered_until < $1)")
            .bind(now)
            .fetch_one(&mut *conn)
            .await?,
        "removing an expired banner (Topic#remove_banner!)",
    )?;
    // Category: a definition without its first post is destroyed.
    refuse(
        sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM topics t JOIN categories c ON c.topic_id = t.id \
             LEFT JOIN posts p ON p.topic_id = t.id AND p.post_number = 1 WHERE p.id IS NULL)",
        )
        .fetch_one(&mut *conn)
        .await?,
        "destroying a category definition without its first post (Topic#destroy!)",
    )?;
    // TopicThumbnail: the sizes themes add.
    refuse(
        sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM theme_modifier_sets \
             WHERE topic_thumbnail_sizes IS NOT NULL AND topic_thumbnail_sizes <> '{}')",
        )
        .fetch_one(&mut *conn)
        .await?,
        "theme topic thumbnail sizes",
    )?;
    // UserAvatar: unused avatar thumbnails, destroyed with their files.
    let sizes: Vec<i32> = s
        .get("avatar_sizes")?
        .to_s()
        .split('|')
        .filter_map(|v| v.trim().parse().ok())
        .collect();
    refuse(
        sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM ( \
               SELECT custom_upload_id FROM user_avatars \
               EXCEPT SELECT upload_id FROM upload_references \
                 WHERE target_type <> 'UserAvatar' AND upload_id IS NOT NULL) AS a \
             JOIN optimized_images oi ON oi.upload_id = a.custom_upload_id \
             WHERE NOT (oi.width = ANY($1)) AND NOT (oi.height = ANY($1)))",
        )
        .bind(&sizes)
        .fetch_one(&mut *conn)
        .await?,
        "removing unused avatar thumbnails (OptimizedImage#destroy)",
    )?;
    // Categories left without a live definition topic (after the step's
    // own UPDATE) get one created (create_category_definition), unless
    // they skip it or are the uncategorized category.
    let needs_definition: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM categories c \
           LEFT JOIN topics t ON t.id = c.topic_id AND t.deleted_at IS NULL \
           WHERE t.id IS NULL AND c.id <> $1 \
             AND NOT EXISTS (SELECT 1 FROM category_custom_fields f WHERE f.category_id = c.id \
                               AND f.name = 'skip_category_definition' AND f.value IN ('t', 'true')))",
    )
    .bind(s.get("uncategorized_category_id")?.to_i() as i32)
    .fetch_one(&mut *conn)
    .await?;
    if needs_definition {
        return Err(Unsupported("creating a category's definition topic").into());
    }
    Ok(())
}

async fn exec(conn: &mut PgConnection, sql: &str) -> Result<(), AppError> {
    sqlx::query(sql).execute(conn).await?;
    Ok(())
}

/// `UserVisit.ensure_consistency!`: days_visited from the visits.
async fn user_visit(conn: &mut PgConnection) -> Result<(), AppError> {
    exec(
        conn,
        "UPDATE user_stats u SET days_visited = \
           (SELECT COUNT(*) FROM user_visits v WHERE v.user_id = u.user_id) \
         WHERE days_visited <> (SELECT COUNT(*) FROM user_visits v WHERE v.user_id = u.user_id)",
    )
    .await
}

/// `Group.ensure_consistency!`: every group's count, the automatic
/// groups' members, and has_messages cleared for groups without messages.
async fn group(conn: &mut PgConnection, i18n: &crate::i18n::I18n) -> Result<(), AppError> {
    crate::roles::reset_groups_user_count(&mut *conn, None).await?;
    crate::roles::refresh_automatic_groups(&mut *conn, i18n).await?;
    exec(
        conn,
        "UPDATE groups g SET has_messages = false \
         WHERE NOT EXISTS (SELECT tg.id FROM topic_allowed_groups tg INNER JOIN topics t ON t.id = tg.topic_id \
                           WHERE tg.group_id = g.id AND t.deleted_at IS NULL) \
         AND g.has_messages = true",
    )
    .await
}

/// `Notification.ensure_consistency!`: high priority notifications of
/// posts that are gone.
async fn notification(conn: &mut PgConnection) -> Result<(), AppError> {
    exec(
        conn,
        "DELETE FROM notifications n WHERE high_priority AND n.topic_id IS NOT NULL \
         AND NOT EXISTS (SELECT 1 FROM posts p JOIN topics t ON t.id = p.topic_id \
                         WHERE p.deleted_at IS NULL AND t.deleted_at IS NULL \
                           AND p.post_number = n.post_number AND t.id = n.topic_id)",
    )
    .await
}

/// `PostRevision.ensure_consistency!`: revisions numbered from 2 in order,
/// and posts' versions from their revisions.
async fn post_revision(conn: &mut PgConnection) -> Result<(), AppError> {
    exec(
        &mut *conn,
        "UPDATE post_revisions SET number = pr.rank \
         FROM (SELECT id, 1 + ROW_NUMBER() OVER (PARTITION BY post_id ORDER BY number, created_at, updated_at) AS rank \
               FROM post_revisions) AS pr \
         WHERE post_revisions.id = pr.id AND post_revisions.number <> pr.rank",
    )
    .await?;
    exec(
        conn,
        "UPDATE posts SET version = 1 + (SELECT COUNT(*) FROM post_revisions WHERE post_id = posts.id), \
           public_version = 1 + (SELECT COUNT(*) FROM post_revisions pr WHERE post_id = posts.id AND pr.hidden = 'f') \
         WHERE version <> 1 + (SELECT COUNT(*) FROM post_revisions WHERE post_id = posts.id) \
            OR public_version <> 1 + (SELECT COUNT(*) FROM post_revisions pr WHERE post_id = posts.id AND pr.hidden = 'f')",
    )
    .await
}

/// `Topic.ensure_consistency!`: topics pinned until a time past unpinned
/// (expired banners are refused up front).
async fn topic(conn: &mut PgConnection, now: NaiveDateTime) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE topics SET pinned_at = NULL, pinned_globally = false, pinned_until = NULL \
         WHERE pinned_until < $1",
    )
    .bind(now)
    .execute(conn)
    .await?;
    Ok(())
}

/// `Badge.ensure_consistency!`: badges of users that are gone, and grant
/// counts (a badge nobody holds keeps its count, as in Rails).
async fn badge(conn: &mut PgConnection) -> Result<(), AppError> {
    exec(
        &mut *conn,
        "DELETE FROM user_badges USING user_badges ub LEFT JOIN users u ON u.id = ub.user_id \
         WHERE u.id IS NULL AND user_badges.id = ub.id",
    )
    .await?;
    exec(
        conn,
        "WITH X AS (SELECT badge_id, COUNT(user_id) users FROM user_badges GROUP BY badge_id) \
         UPDATE badges SET grant_count = X.users FROM X WHERE id = X.badge_id AND grant_count <> X.users",
    )
    .await
}

/// `CategoryUser.ensure_consistency!`
async fn category_user(conn: &mut PgConnection) -> Result<(), AppError> {
    exec(
        conn,
        "DELETE FROM category_users WHERE user_id IN ( \
           SELECT cu.user_id FROM category_users cu LEFT JOIN users u ON u.id = cu.user_id WHERE u.id IS NULL)",
    )
    .await
}

/// `UserOption.ensure_consistency!`: options created for users without
/// them (UserOption.create: defaults, then the tracked topics updated).
async fn user_option(conn: &mut PgConnection, s: &SiteSettings) -> Result<(), AppError> {
    let ids: Vec<i32> = sqlx::query_scalar(
        "SELECT u.id FROM users u LEFT JOIN user_options o ON o.user_id = u.id WHERE o.user_id IS NULL",
    )
    .fetch_all(&mut *conn)
    .await?;
    for id in ids {
        crate::signup::create_user_option(&mut *conn, s, id).await?;
        // after_save update_tracked_topics: the column has no default, so
        // a new row always changes it.
        let threshold = s.get("default_other_auto_track_topics_after_msecs")?.to_i();
        crate::user_updater::tracked_topics_updater(&mut *conn, id, threshold).await?;
    }
    Ok(())
}

/// `Tag.ensure_consistency!` (update_topic_counts): staff, public and PM
/// topic counts.
async fn tag(conn: &mut PgConnection) -> Result<(), AppError> {
    exec(
        &mut *conn,
        "UPDATE tags t SET staff_topic_count = x.topic_count FROM ( \
           SELECT COUNT(topics.id) AS topic_count, tags.id AS tag_id FROM tags \
           LEFT JOIN topic_tags ON tags.id = topic_tags.tag_id \
           LEFT JOIN topics ON topics.id = topic_tags.topic_id AND topics.deleted_at IS NULL \
             AND topics.archetype != 'private_message' \
           GROUP BY tags.id) x \
         WHERE x.tag_id = t.id AND x.topic_count <> t.staff_topic_count",
    )
    .await?;
    exec(
        &mut *conn,
        "UPDATE tags t SET public_topic_count = x.topic_count FROM ( \
           WITH tags_with_public_topics AS ( \
             SELECT COUNT(topics.id) AS topic_count, tags.id AS tag_id FROM tags \
             INNER JOIN topic_tags ON tags.id = topic_tags.tag_id \
             INNER JOIN topics ON topics.id = topic_tags.topic_id AND topics.deleted_at IS NULL \
               AND topics.archetype != 'private_message' \
             INNER JOIN categories ON categories.id = topics.category_id AND NOT categories.read_restricted \
             GROUP BY tags.id) \
           SELECT COALESCE(tags_with_public_topics.topic_count, 0) AS topic_count, tags.id AS tag_id \
           FROM tags LEFT JOIN tags_with_public_topics ON tags_with_public_topics.tag_id = tags.id) x \
         WHERE x.tag_id = t.id AND x.topic_count <> t.public_topic_count",
    )
    .await?;
    exec(
        conn,
        "UPDATE tags t SET pm_topic_count = x.pm_topic_count FROM ( \
           SELECT COUNT(topics.id) AS pm_topic_count, tags.id AS tag_id FROM tags \
           LEFT JOIN topic_tags ON tags.id = topic_tags.tag_id \
           LEFT JOIN topics ON topics.id = topic_tags.topic_id AND topics.deleted_at IS NULL \
             AND topics.archetype = 'private_message' \
           GROUP BY tags.id) x \
         WHERE x.tag_id = t.id AND x.pm_topic_count <> t.pm_topic_count",
    )
    .await
}

/// `CategoryTagStat.ensure_consistency!` (update_topic_counts): the
/// per-category tag counts upserted (each drawing a sequence value, as in
/// Rails), stale ones deleted.
async fn category_tag_stat(conn: &mut PgConnection) -> Result<(), AppError> {
    exec(
        &mut *conn,
        "WITH stats AS ( \
           SELECT topics.category_id as category_id, tags.id AS tag_id, COUNT(topics.id) AS topic_count \
           FROM tags INNER JOIN topic_tags ON tags.id = topic_tags.tag_id \
           INNER JOIN topics ON topics.id = topic_tags.topic_id AND topics.deleted_at IS NULL \
             AND topics.category_id IS NOT NULL \
           GROUP BY topics.category_id, tags.id) \
         INSERT INTO category_tag_stats(category_id, tag_id, topic_count) \
         SELECT category_id, tag_id, topic_count FROM stats \
         ON CONFLICT (category_id, tag_id) DO UPDATE SET topic_count = EXCLUDED.topic_count",
    )
    .await?;
    exec(
        conn,
        "DELETE FROM category_tag_stats WHERE (category_id, tag_id) NOT IN ( \
           SELECT topics.category_id as category_id, tags.id AS tag_id FROM tags \
           INNER JOIN topic_tags ON tags.id = topic_tags.tag_id \
           INNER JOIN topics ON topics.id = topic_tags.topic_id AND topics.deleted_at IS NULL \
             AND topics.category_id IS NOT NULL \
           GROUP BY topics.category_id, tags.id)",
    )
    .await
}

/// `User.ensure_consistency!`: avatars pointing at missing uploads.
async fn user(conn: &mut PgConnection) -> Result<(), AppError> {
    exec(
        conn,
        "UPDATE users SET uploaded_avatar_id = NULL WHERE uploaded_avatar_id IN ( \
           SELECT u1.uploaded_avatar_id FROM users u1 LEFT JOIN uploads up ON u1.uploaded_avatar_id = up.id \
           WHERE u1.uploaded_avatar_id IS NOT NULL AND up.id IS NULL)",
    )
    .await
}

/// `UserAvatar.ensure_consistency!`: avatars of missing users, and upload
/// ids pointing at missing uploads (unused thumbnails refused up front).
async fn user_avatar(conn: &mut PgConnection) -> Result<(), AppError> {
    exec(
        &mut *conn,
        "DELETE FROM user_avatars USING user_avatars ua LEFT JOIN users u ON ua.user_id = u.id \
         WHERE user_avatars.id = ua.id AND u.id IS NULL",
    )
    .await?;
    for column in ["gravatar_upload_id", "custom_upload_id"] {
        exec(
            &mut *conn,
            &format!(
                "UPDATE user_avatars SET {column} = NULL WHERE {column} IN ( \
                   SELECT u1.{column} FROM user_avatars u1 LEFT JOIN uploads up ON u1.{column} = up.id \
                   WHERE u1.{column} IS NOT NULL AND up.id IS NULL)"
            ),
        )
        .await?;
    }
    Ok(())
}

/// `UserEmail.ensure_consistency!`: a random primary email for users
/// without one. A domain the site's lists refuse fails the step, as
/// create! raising does.
async fn user_email(conn: &mut PgConnection, s: &SiteSettings) -> Result<(), AppError> {
    const DOMAIN: &str = "missing-primary-email.invalid";
    let ids: Vec<i32> = sqlx::query_scalar(
        "SELECT u.id FROM users u LEFT JOIN user_emails ue ON u.id = ue.user_id AND ue.primary = true \
         WHERE ue.id IS NULL",
    )
    .fetch_all(&mut *conn)
    .await?;
    if ids.is_empty() {
        return Ok(());
    }
    let list = |name: &str| -> Result<Vec<String>, AppError> {
        Ok(s.get(name)?
            .to_s()
            .split('|')
            .filter(|d| !d.is_empty())
            .map(str::to_lowercase)
            .collect())
    };
    let allowed = list("allowed_email_domains")?;
    let blocked = list("blocked_email_domains")?;
    let covered = |domains: &[String]| {
        domains
            .iter()
            .any(|d| DOMAIN == d || DOMAIN.ends_with(&format!(".{d}")))
    };
    if (!allowed.is_empty() && !covered(&allowed)) || covered(&blocked) {
        return Err(std::io::Error::other("the placeholder email's domain is not allowed").into());
    }
    for id in ids {
        // SecureRandom.alphanumeric(64), downcased by the model.
        let local: String = rand::thread_rng()
            .sample_iter(rand::distributions::Alphanumeric)
            .take(64)
            .map(|c| (c as char).to_ascii_lowercase())
            .collect();
        let email = format!("{local}@{DOMAIN}");
        sqlx::query(
            "INSERT INTO user_emails (user_id, email, normalized_email, \"primary\", created_at, updated_at) \
             VALUES ($1, $2, $2, TRUE, clock_timestamp(), clock_timestamp())",
        )
        .bind(id)
        .bind(&email)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// `Category.ensure_consistency!`: categories whose definition topic is
/// gone lose the link (destroying and creating definitions is refused up
/// front).
async fn category(conn: &mut PgConnection) -> Result<(), AppError> {
    exec(
        conn,
        "UPDATE categories c SET topic_id = NULL WHERE c.id IN ( \
           SELECT c2.id FROM categories c2 LEFT JOIN topics t ON t.id = c2.topic_id AND t.deleted_at IS NULL \
           WHERE t.id IS NULL AND c2.topic_id IS NOT NULL)",
    )
    .await
}

/// `TopicThumbnail.ensure_consistency!`: thumbnails of missing uploads or
/// optimized images, and of sizes no longer made (Topic.thumbnail_sizes,
/// themes' own refused up front).
async fn topic_thumbnail(conn: &mut PgConnection) -> Result<(), AppError> {
    exec(
        &mut *conn,
        "DELETE FROM topic_thumbnails WHERE id IN ( \
           SELECT topic_thumbnails.id FROM topic_thumbnails \
           LEFT JOIN uploads on upload_id = uploads.id \
           LEFT JOIN optimized_images on optimized_image_id = optimized_images.id \
           WHERE (optimized_image_id IS NOT NULL AND optimized_images IS NULL) OR uploads IS NULL)",
    )
    .await?;
    exec(
        conn,
        "DELETE FROM topic_thumbnails WHERE NOT ((max_width = 1024 AND max_height = 1024))",
    )
    .await
}

/// `UserAction.ensure_consistency!(13.hours.ago)`
/// (synchronize_target_topic_ids): duplicates of recently acted-on posts
/// removed, and their target topics corrected.
async fn user_action(conn: &mut PgConnection, since: NaiveDateTime) -> Result<(), AppError> {
    sqlx::query(
        "DELETE FROM user_actions USING user_actions ua2 \
         WHERE (user_actions.action_type = ua2.action_type AND user_actions.user_id = ua2.user_id \
           AND user_actions.acting_user_id = ua2.acting_user_id \
           AND user_actions.target_post_id = ua2.target_post_id \
           AND user_actions.target_post_id > 0 AND user_actions.id > ua2.id) \
         AND (user_actions.target_post_id IN (SELECT target_post_id FROM user_actions WHERE created_at > $1))",
    )
    .bind(since)
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "UPDATE user_actions SET target_topic_id = (select topic_id from posts where posts.id = target_post_id) \
         WHERE (target_topic_id <> (select topic_id from posts where posts.id = target_post_id)) \
         AND (target_post_id IN (SELECT target_post_id FROM user_actions WHERE created_at > $1))",
    )
    .bind(since)
    .execute(conn)
    .await?;
    Ok(())
}

/// The whisperer groups as SQL ids, none when whispers_allowed_groups is
/// empty.
fn whisperers(s: &SiteSettings) -> Result<Option<String>, AppError> {
    let ids = s.group_ids("whispers_allowed_groups")?;
    Ok((!ids.is_empty()).then(|| {
        ids.iter()
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    }))
}

/// `UserStat.ensure_consistency!(13.hours.ago)`
async fn user_stat(
    conn: &mut PgConnection,
    s: &SiteSettings,
    last_seen: NaiveDateTime,
    now: NaiveDateTime,
) -> Result<(), AppError> {
    // reset_bounce_scores
    exec(
        &mut *conn,
        "UPDATE user_stats SET bounce_score = 0 WHERE (reset_bounce_score_after < now()) AND (bounce_score > 0)",
    )
    .await?;
    // update_distinct_badge_count
    exec(
        &mut *conn,
        "UPDATE user_stats SET distinct_badge_count = x.distinct_badge_count FROM ( \
           SELECT users.id user_id, COUNT(distinct user_badges.badge_id) distinct_badge_count FROM users \
           LEFT JOIN user_badges ON user_badges.user_id = users.id \
             AND (user_badges.badge_id IN (SELECT id FROM badges WHERE enabled)) \
           GROUP BY users.id) x \
         WHERE user_stats.user_id = x.user_id AND user_stats.distinct_badge_count <> x.distinct_badge_count",
    )
    .await?;
    // update_view_counts
    sqlx::query(
        "UPDATE user_stats SET topics_entered = X.c FROM ( \
           SELECT v.user_id, COUNT(topic_id) AS c FROM topic_views AS v \
           WHERE v.user_id IN (SELECT u1.id FROM users u1 where u1.last_seen_at > $1) \
           GROUP BY v.user_id) AS X \
         WHERE X.user_id = user_stats.user_id AND X.c <> topics_entered",
    )
    .bind(last_seen)
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "WITH filtered_users AS ( \
           SELECT id FROM users u JOIN user_stats ON user_id = u.id \
           WHERE last_seen_at > $1 AND posts_read_count < 10000) \
         UPDATE user_stats SET posts_read_count = X.c FROM ( \
           SELECT pt.user_id, COUNT(*) as c FROM filtered_users AS u \
           JOIN post_timings AS pt ON pt.user_id = u.id JOIN topics t ON t.id = pt.topic_id \
           WHERE t.archetype = 'regular' AND t.deleted_at IS NULL GROUP BY pt.user_id) AS X \
         WHERE X.user_id = user_stats.user_id AND X.c <> posts_read_count",
    )
    .bind(last_seen)
    .execute(&mut *conn)
    .await?;
    // update_first_unread
    let seen = "SELECT id FROM users WHERE last_seen_at IS NOT NULL AND last_seen_at > $1 \
                ORDER BY last_seen_at DESC LIMIT $2";
    sqlx::query(&format!(
        "UPDATE user_stats us SET first_unread_at = COALESCE(Y.min_date, $3) FROM ( \
           SELECT u1.id user_id, X.min min_date FROM users u1 LEFT JOIN ( \
             SELECT u.id AS user_id, min(topics.updated_at) min FROM users u \
             LEFT JOIN topic_users tu ON tu.user_id = u.id \
             LEFT JOIN topics ON tu.topic_id = topics.id \
             JOIN user_stats AS us ON us.user_id = u.id \
             JOIN user_options AS uo ON uo.user_id = u.id \
             JOIN categories c ON c.id = topics.category_id \
             WHERE u.id IN ({seen}) \
               AND topics.archetype <> 'private_message' \
               AND ((\"topics\".\"deleted_at\" IS NULL \
                     AND tu.last_read_post_number < CASE WHEN u.admin OR u.moderator \
                       THEN topics.highest_staff_post_number ELSE topics.highest_post_number END \
                     AND COALESCE(tu.notification_level, 1) >= 2) OR (1=0)) \
               AND (topics.visible OR u.admin OR u.moderator) \
               AND topics.deleted_at IS NULL \
               AND (NOT c.read_restricted OR u.admin OR category_id IN ( \
                 SELECT c2.id FROM categories c2 JOIN category_groups cg ON cg.category_id = c2.id \
                 JOIN group_users gu ON gu.user_id = u.id AND cg.group_id = gu.group_id \
                 WHERE c2.read_restricted)) \
               AND NOT EXISTS (SELECT 1 FROM category_users cu WHERE last_read_post_number IS NULL \
                 AND cu.user_id = u.id AND cu.category_id = topics.category_id AND cu.notification_level = 0) \
             GROUP BY u.id, u.username) AS X ON X.user_id = u1.id \
           WHERE u1.id IN ({seen})) Y \
         WHERE Y.user_id = us.user_id"
    ))
    .bind(last_seen)
    .bind(UNREAD_USERS_LIMIT)
    .bind(now)
    .execute(&mut *conn)
    .await?;
    // update_first_unread_pm
    let whisper = whisperers(s)?;
    let (join, whisperer) = match &whisper {
        Some(ids) => (
            format!("LEFT JOIN group_users gu ON gu.group_id IN ({ids}) AND gu.user_id = u.id"),
            "OR gu.id IS NOT NULL".to_string(),
        ),
        None => (String::new(), String::new()),
    };
    sqlx::query(&format!(
        "UPDATE user_stats us SET first_unread_pm_at = COALESCE(Z.min_date, $3) FROM ( \
           SELECT u1.id user_id, X.min_date FROM users u1 LEFT JOIN ( \
             SELECT tau.user_id, MIN(t.updated_at) min_date FROM topic_allowed_users tau \
             INNER JOIN topics t ON t.id = tau.topic_id \
             INNER JOIN users u ON u.id = tau.user_id \
             LEFT JOIN topic_users tu ON t.id = tu.topic_id AND tu.user_id = tau.user_id \
             {join} \
             WHERE t.deleted_at IS NULL AND t.archetype = 'private_message' \
               AND tu.last_read_post_number < CASE WHEN u.admin OR u.moderator {whisperer} \
                 THEN t.highest_staff_post_number ELSE t.highest_post_number END \
               AND (COALESCE(tu.notification_level, 1) >= 2) \
               AND tau.user_id IN ({seen}) \
             GROUP BY tau.user_id) AS X ON X.user_id = u1.id \
           WHERE u1.id IN ({seen})) AS Z \
         WHERE us.user_id = Z.user_id"
    ))
    .bind(last_seen)
    .bind(UNREAD_USERS_LIMIT)
    .bind(now)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// `GroupUser.ensure_consistency!(13.hours.ago)` (update_first_unread_pm):
/// each recently seen member's oldest unread group message, else ten
/// minutes ago.
async fn group_user(
    conn: &mut PgConnection,
    s: &SiteSettings,
    last_seen: NaiveDateTime,
    now: NaiveDateTime,
) -> Result<(), AppError> {
    let whisperer = match whisperers(s)? {
        Some(ids) => format!("OR gu2.group_id IN ({ids})"),
        None => String::new(),
    };
    sqlx::query(&format!(
        "UPDATE group_users gu SET first_unread_pm_at = Y.min_date FROM ( \
           SELECT X.group_id, X.user_id, X.min_date FROM ( \
             SELECT gu.group_id, gu.user_id, COALESCE(Z.min_date, $3) min_date FROM group_users gu \
             LEFT JOIN ( \
               SELECT gu2.group_id, gu2.user_id, MIN(t.updated_at) min_date FROM group_users gu2 \
               INNER JOIN topic_allowed_groups tag ON tag.group_id = gu2.group_id \
               INNER JOIN topics t ON t.id = tag.topic_id \
               INNER JOIN users u ON u.id = gu2.user_id \
               LEFT JOIN topic_users tu ON t.id = tu.topic_id AND tu.user_id = gu2.user_id \
               WHERE t.deleted_at IS NULL AND t.archetype = 'private_message' \
                 AND tu.last_read_post_number < CASE WHEN u.admin OR u.moderator {whisperer} \
                   THEN t.highest_staff_post_number ELSE t.highest_post_number END \
                 AND (COALESCE(tu.notification_level, 1) >= 2) \
               GROUP BY gu2.user_id, gu2.group_id) AS Z ON Z.user_id = gu.user_id AND Z.group_id = gu.group_id \
           ) AS X \
           WHERE X.user_id IN (SELECT id FROM users WHERE last_seen_at IS NOT NULL AND last_seen_at > $1 \
                               ORDER BY last_seen_at DESC LIMIT $2)) Y \
         WHERE gu.user_id = Y.user_id AND gu.group_id = Y.group_id"
    ))
    .bind(last_seen)
    .bind(UNREAD_USERS_LIMIT)
    .bind(now)
    .execute(conn)
    .await?;
    Ok(())
}
