//! `BadgeGranter.backfill(badge, post_ids:/user_ids:)`: a SQL badge's
//! query run for some posts or users (or all of them), granting what it
//! returns that isn't granted yet, with a granted_badge notification each
//! and the badge's grant count reset.
//!
//! Badge queries take mini_sql's named parameters (`:backfill`,
//! `:post_ids`, `:user_ids`, `:id`, `:multiple_grant`), which are bound
//! here as literals the way mini_sql expands them.
//!
//! Not ported: notifications in a locale other than English, the
//! user_badge_granted web hooks and automations, each refused when it
//! would apply.

use serde_json::json;
use sqlx::PgConnection;

use crate::site_settings::SiteSettings;
use crate::{AppError, Unsupported};

/// `BadgeGranter::MAX_ITEMS_FOR_DELTA`
const MAX_ITEMS_FOR_DELTA: usize = 200;
/// `Notification.types[:granted_badge]`
const GRANTED_BADGE: i32 = 12;
/// `Badge::Welcome`
const WELCOME: i32 = 5;
/// `Badge::NewUserOfTheMonth`
const NEW_USER_OF_THE_MONTH: i32 = 44;
/// `BadgeGrouping::GettingStarted`
const GETTING_STARTED: i32 = 1;
/// `BadgeType::Bronze`
const BRONZE: i32 = 3;
/// `Discourse::SYSTEM_USER_ID`
const SYSTEM_USER_ID: i32 = -1;

#[derive(Debug, sqlx::FromRow)]
pub struct Badge {
    pub id: i32,
    pub name: String,
    pub badge_type_id: i32,
    pub badge_grouping_id: i32,
    pub allow_title: bool,
    pub multiple_grant: bool,
    pub target_posts: Option<bool>,
    pub query: Option<String>,
    pub enabled: bool,
    pub auto_revoke: bool,
}

impl Badge {
    /// `Badge.find(id)`
    pub async fn find(conn: &mut PgConnection, id: i32) -> Result<Option<Badge>, sqlx::Error> {
        sqlx::query_as(
            "SELECT id, name, badge_type_id, badge_grouping_id, allow_title, multiple_grant, \
                    target_posts, query, enabled, auto_revoke \
             FROM badges WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(conn)
        .await
    }

    pub async fn enabled_named(
        conn: &mut PgConnection,
        names: &[&str],
    ) -> Result<Vec<Badge>, sqlx::Error> {
        sqlx::query_as(
            "SELECT id, name, badge_type_id, badge_grouping_id, allow_title, multiple_grant, \
                    target_posts, query, enabled, auto_revoke \
             FROM badges WHERE enabled AND name = ANY($1) ORDER BY id",
        )
        .bind(names)
        .fetch_all(conn)
        .await
    }

    /// `Badge.display_name(name)`: its translation, else the name.
    fn display_name(&self, i18n: &crate::i18n::I18n) -> String {
        let key = format!("badges.{}.name", self.name.to_lowercase().replace(' ', "_"));
        i18n.t(&key).unwrap_or(&self.name).to_string()
    }

    /// `for_beginners?`
    fn for_beginners(&self) -> bool {
        self.id == WELCOME
            || (self.badge_grouping_id == GETTING_STARTED && self.id != NEW_USER_OF_THE_MONTH)
    }
}

/// Which rows a backfill covers.
pub enum Scope<'a> {
    Posts(&'a [i32]),
    Users(&'a [i32]),
}

/// A mini_sql parameter's literal.
enum Param<'a> {
    Int(i32),
    Bool(bool),
    Ints(&'a [i32]),
}

/// The query with its named parameters bound: `:name` outside `::` casts.
fn bind(sql: &str, params: &[(&str, Param<'_>)]) -> String {
    let mut out = String::with_capacity(sql.len());
    let bytes = sql.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b':' {
            if bytes.get(i + 1) == Some(&b':') {
                out.push_str("::");
                i += 2;
                continue;
            }
            let start = i + 1;
            let mut end = start;
            while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
                end += 1;
            }
            if let Some((_, value)) = params.iter().find(|(n, _)| *n == &sql[start..end]) {
                match value {
                    Param::Int(v) => out.push_str(&v.to_string()),
                    Param::Bool(v) => out.push_str(if *v { "true" } else { "false" }),
                    Param::Ints(v) => {
                        out.push_str(&v.iter().map(i32::to_string).collect::<Vec<_>>().join(","))
                    }
                }
                i = end;
                continue;
            }
        }
        let c = sql[i..].chars().next().unwrap_or(' ');
        out.push(c);
        i += c.len_utf8();
    }
    out
}

/// `BadgeGranter.suppress_notification?` with discourse-topic-voting's
/// modifier: its badges are quiet when granted more than two weeks ago.
fn suppress_notification(
    badge: &Badge,
    granted_at: chrono::NaiveDateTime,
    skip_new_user_tips: bool,
) -> bool {
    let now = crate::clock::now_naive();
    let old_bronze = badge.badge_type_id == BRONZE && granted_at < now - chrono::Duration::days(2);
    let skip_beginner = skip_new_user_tips && badge.for_beginners();
    let voting = crate::plugins::topic_voting::BADGE_NAMES.contains(&badge.name.as_str())
        && granted_at < now - chrono::Duration::weeks(2);
    old_bronze || skip_beginner || voting
}

/// `BadgeGranter.backfill`
pub async fn backfill(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    settings: &SiteSettings,
    i18n: &crate::i18n::I18n,
    badge: &Badge,
    scope: Option<Scope<'_>>,
) -> Result<(), AppError> {
    if !settings.get("enable_badges")?.truthy() || !badge.enabled {
        return Ok(());
    }
    let Some(query) = badge.query.as_deref().filter(|q| !q.trim().is_empty()) else {
        return Ok(());
    };
    let (mut post_ids, mut user_ids) = match scope {
        Some(Scope::Posts(ids)) => (Some(ids), None),
        Some(Scope::Users(ids)) => (None, Some(ids)),
        None => (None, None),
    };
    // A delta of more than 200 is a full backfill.
    if post_ids.is_some_and(|p| p.len() > MAX_ITEMS_FOR_DELTA)
        || user_ids.is_some_and(|u| u.len() > MAX_ITEMS_FOR_DELTA)
    {
        post_ids = None;
        user_ids = None;
    }
    let post_ids = post_ids.filter(|p| !p.is_empty());
    let user_ids = user_ids.filter(|u| !u.is_empty());
    let full_backfill = post_ids.is_none() && user_ids.is_none();
    let target_posts = badge.target_posts.unwrap_or(false);
    let post_clause = if target_posts {
        "AND (q.post_id = ub.post_id OR NOT :multiple_grant)"
    } else {
        ""
    };
    let post_id_field = if target_posts { "q.post_id" } else { "NULL" };

    if badge.auto_revoke && full_backfill {
        return Err(Unsupported("full badge backfills with auto revoke").into());
    }
    // A delta needs the query to take it, else Rails logs and skips.
    if !full_backfill && !query.contains(":backfill") {
        return Ok(());
    }
    if post_ids.is_some() && !query.contains(":post_ids") {
        return Ok(());
    }
    if user_ids.is_some() && !query.contains(":user_ids") {
        return Ok(());
    }

    let sql = format!(
        "WITH w as ( \
           INSERT INTO user_badges(badge_id, user_id, granted_at, granted_by_id, created_at, post_id) \
           SELECT :id, q.user_id, q.granted_at, {SYSTEM_USER_ID}, current_timestamp, {post_id_field} \
             FROM ({query}) q \
             LEFT JOIN user_badges ub ON ub.badge_id = :id AND ub.user_id = q.user_id \
             {post_clause} \
             WHERE ub.badge_id IS NULL AND q.user_id > 0 \
           ON CONFLICT DO NOTHING \
           RETURNING id, user_id, granted_at \
         ) \
         SELECT w.id, w.user_id, w.granted_at, u.username, u.locale, (u.admin OR u.moderator) AS staff, \
                uo.skip_new_user_tips \
           FROM w JOIN users u ON u.id = w.user_id JOIN user_options uo ON uo.user_id = w.user_id"
    );
    let none = [-2];
    let sql = bind(
        &sql,
        &[
            ("id", Param::Int(badge.id)),
            ("multiple_grant", Param::Bool(badge.multiple_grant)),
            ("backfill", Param::Bool(full_backfill)),
            ("post_ids", Param::Ints(post_ids.unwrap_or(&none))),
            ("user_ids", Param::Ints(user_ids.unwrap_or(&none))),
        ],
    );
    #[derive(sqlx::FromRow)]
    struct Granted {
        id: i32,
        user_id: i32,
        granted_at: chrono::NaiveDateTime,
        username: String,
        locale: Option<String>,
        staff: bool,
        skip_new_user_tips: bool,
    }
    let rows: Vec<Granted> = sqlx::query_as(&sql).fetch_all(&mut *conn).await?;

    if !rows.is_empty() {
        user_badge_granted_unsupported(conn).await?;
    }
    for row in &rows {
        if suppress_notification(badge, row.granted_at, row.skip_new_user_tips) {
            continue;
        }
        // awarded_for_trust_level?
        if row.staff && badge.id <= 4 {
            continue;
        }
        let notification_id = send_notification(
            conn,
            bus,
            settings,
            i18n,
            badge,
            row.user_id,
            &row.username,
            row.locale.as_deref(),
        )
        .await?;
        sqlx::query("UPDATE user_badges SET notification_id = $1 WHERE id = $2")
            .bind(notification_id)
            .bind(row.id)
            .execute(&mut *conn)
            .await?;
    }

    // reset_grant_count!
    sqlx::query(
        "UPDATE badges SET grant_count = (SELECT count(*) FROM user_badges WHERE badge_id = $1), \
                updated_at = CASE WHEN grant_count = (SELECT count(*) FROM user_badges WHERE badge_id = $1) \
                                  THEN updated_at ELSE now() END \
         WHERE id = $1",
    )
    .bind(badge.id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// `BadgeGranter.send_notification`: the granted_badge notification in
/// the user's locale (`notification_locale`), English only.
#[allow(clippy::too_many_arguments)]
async fn send_notification(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    settings: &SiteSettings,
    i18n: &crate::i18n::I18n,
    badge: &Badge,
    user_id: i32,
    username: &str,
    user_locale: Option<&str>,
) -> Result<i64, AppError> {
    let locale = match user_locale.filter(|l| !l.is_empty()) {
        Some(l) if settings.get("allow_user_locale")?.truthy() => l.to_string(),
        _ => settings.get("default_locale")?.to_s().to_string(),
    };
    if locale != "en" {
        return Err(Unsupported("badge notifications in locales other than English").into());
    }
    let display_name = badge.display_name(i18n);
    let mut slug = crate::posting::text::slug_for(&display_name, &locale)?;
    // Slug.for(display_name, "-"): its default is a dash.
    if slug == "topic" && !display_name.eq_ignore_ascii_case("topic") {
        slug = "-".to_string();
    }
    let data = json!({
        "badge_id": badge.id,
        "badge_name": display_name,
        "badge_slug": slug,
        "badge_title": badge.allow_title,
        "username": username,
    });
    let notification_id: i64 = sqlx::query_scalar(
        "INSERT INTO notifications (notification_type, user_id, data, read, high_priority, \
                                    created_at, updated_at) \
         VALUES ($1, $2, $3, FALSE, FALSE, clock_timestamp(), clock_timestamp()) RETURNING id",
    )
    .bind(GRANTED_BADGE)
    .bind(user_id)
    .bind(data.to_string())
    .fetch_one(&mut *conn)
    .await?;
    crate::bus::publish_notifications_state(bus, &mut *conn, settings, user_id).await?;
    Ok(notification_id)
}

/// `BadgeGranter.grant(badge, user, post_id:)` by the system user: the
/// badge once (or once per post for a multiple grant), UserBadge's
/// after_create (the grant count, the distinct badge count, the featured
/// ranks), and the granted_badge notification unless suppressed.
#[allow(clippy::too_many_arguments)]
pub async fn grant(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    settings: &SiteSettings,
    i18n: &crate::i18n::I18n,
    badge_id: i32,
    user_id: i32,
    post_id: Option<i32>,
) -> Result<(), AppError> {
    let Some(badge) = Badge::find(&mut *conn, badge_id).await? else {
        return Ok(());
    };
    if !badge.enabled {
        return Ok(());
    }
    #[derive(sqlx::FromRow)]
    struct UserRow {
        username: String,
        locale: Option<String>,
        skip_new_user_tips: Option<bool>,
    }
    let user: Option<UserRow> = sqlx::query_as(
        "SELECT u.username, u.locale, uo.skip_new_user_tips FROM users u \
         LEFT JOIN user_options uo ON uo.user_id = u.id WHERE u.id = $1",
    )
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(user) = user else {
        return Ok(());
    };
    let existing: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM user_badges WHERE badge_id = $1 AND user_id = $2 \
         AND (NOT $3 OR post_id IS NOT DISTINCT FROM $4))",
    )
    .bind(badge.id)
    .bind(user_id)
    .bind(badge.multiple_grant)
    .bind(post_id)
    .fetch_one(&mut *conn)
    .await?;
    if existing && !(badge.multiple_grant && post_id.is_none()) {
        return Ok(());
    }
    let seq: i32 = if badge.multiple_grant {
        let max: Option<i32> = sqlx::query_scalar(
            "SELECT MAX(seq) FROM user_badges WHERE badge_id = $1 AND user_id = $2",
        )
        .bind(badge.id)
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
        max.unwrap_or(-1) + 1
    } else {
        0
    };
    let (user_badge_id, granted_at): (i32, chrono::NaiveDateTime) = sqlx::query_as(
        "INSERT INTO user_badges (badge_id, user_id, granted_by_id, granted_at, post_id, seq, created_at) \
         VALUES ($1, $2, $3, clock_timestamp(), $4, $5, clock_timestamp()) RETURNING id, granted_at",
    )
    .bind(badge.id)
    .bind(user_id)
    .bind(SYSTEM_USER_ID)
    .bind(post_id)
    .bind(seq)
    .fetch_one(&mut *conn)
    .await?;
    // after_create
    sqlx::query("UPDATE badges SET grant_count = grant_count + 1 WHERE id = $1")
        .bind(badge.id)
        .execute(&mut *conn)
        .await?;
    sqlx::query(
        "UPDATE user_stats SET distinct_badge_count = x.distinct_badge_count \
         FROM (SELECT users.id user_id, COUNT(distinct user_badges.badge_id) distinct_badge_count \
               FROM users LEFT JOIN user_badges ON user_badges.user_id = users.id \
                 AND (user_badges.badge_id IN (SELECT id FROM badges WHERE enabled)) \
               GROUP BY users.id) x \
         WHERE user_stats.user_id = x.user_id AND user_stats.distinct_badge_count <> x.distinct_badge_count \
           AND user_stats.user_id IN ($1)",
    )
    .bind(user_id)
    .execute(&mut *conn)
    .await?;
    update_featured_ranks(conn, user_id).await?;
    user_badge_granted_unsupported(conn).await?;
    if !settings.get("enable_badges")?.truthy() {
        return Ok(());
    }
    if !suppress_notification(&badge, granted_at, user.skip_new_user_tips.unwrap_or(false)) {
        let notification_id = send_notification(
            conn,
            bus,
            settings,
            i18n,
            &badge,
            user_id,
            &user.username,
            user.locale.as_deref(),
        )
        .await?;
        sqlx::query("UPDATE user_badges SET notification_id = $1 WHERE id = $2")
            .bind(notification_id)
            .bind(user_badge_id)
            .execute(&mut *conn)
            .await?;
    }
    let favorite: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM user_badges WHERE user_id = $1 AND badge_id = $2 AND is_favorite)",
    )
    .bind(user_id)
    .bind(badge.id)
    .fetch_one(&mut *conn)
    .await?;
    if favorite {
        sqlx::query("UPDATE user_badges SET is_favorite = TRUE WHERE id = $1")
            .bind(user_badge_id)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

/// `UserBadge.update_featured_ranks!([user_id])`
async fn update_featured_ranks(conn: &mut PgConnection, user_id: i32) -> Result<(), sqlx::Error> {
    sqlx::query(
        "WITH featured_tl_badge AS ( \
           SELECT user_id, max(badge_id) as badge_id FROM user_badges \
           WHERE badge_id IN (1,2,3,4) AND user_id IN ($1) GROUP BY user_id \
         ), ranks AS ( \
           SELECT user_badges.user_id, user_badges.badge_id, \
             RANK() OVER ( \
               PARTITION BY user_badges.user_id \
               ORDER BY BOOL_OR(badges.enabled) DESC, \
                        MAX(featured_tl_badge.user_id) NULLS LAST, \
                        BOOL_OR(user_badges.is_favorite) DESC NULLS LAST, \
                        CASE WHEN user_badges.badge_id IN (1,2,3,4) THEN 1 ELSE 0 END ASC, \
                        MAX(badges.badge_type_id) ASC, \
                        MAX(badges.grant_count) ASC, \
                        user_badges.badge_id DESC \
             ) rank_number \
           FROM user_badges \
           INNER JOIN badges ON badges.id = user_badges.badge_id \
           LEFT JOIN featured_tl_badge ON featured_tl_badge.user_id = user_badges.user_id \
             AND featured_tl_badge.badge_id = user_badges.badge_id \
           WHERE user_badges.user_id IN ($1) \
           GROUP BY user_badges.user_id, user_badges.badge_id \
         ) \
         UPDATE user_badges SET featured_rank = rank_number FROM ranks \
         WHERE ranks.badge_id = user_badges.badge_id AND ranks.user_id = user_badges.user_id \
           AND featured_rank IS DISTINCT FROM rank_number",
    )
    .bind(user_id)
    .execute(conn)
    .await?;
    Ok(())
}

/// UserBadge.trigger_user_badge_granted_event: web hooks for
/// user_badge_granted and automations triggered by it aren't ported.
async fn user_badge_granted_unsupported(conn: &mut PgConnection) -> Result<(), AppError> {
    if crate::plugins::web_hooks_active(conn, "user_badge_granted").await? {
        return Err(Unsupported("user_badge_granted web hooks").into());
    }
    let automations: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM discourse_automation_automations \
         WHERE enabled AND trigger = 'user_badge_granted')",
    )
    .fetch_one(conn)
    .await?;
    if automations {
        return Err(Unsupported("automations on user_badge_granted").into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binds_named_parameters_but_not_casts() {
        let sql = bind(
            "SELECT x::int FROM t WHERE (:backfill OR p.id IN (:post_ids)) AND b = :id",
            &[
                ("id", Param::Int(7)),
                ("backfill", Param::Bool(false)),
                ("post_ids", Param::Ints(&[1, 2])),
            ],
        );
        assert_eq!(
            sql,
            "SELECT x::int FROM t WHERE (false OR p.id IN (1,2)) AND b = 7"
        );
    }
}
