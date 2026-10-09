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
    let allow_user_locale = settings.get("allow_user_locale")?.truthy();
    let default_locale = settings.get("default_locale")?.to_s();
    for row in &rows {
        if suppress_notification(badge, row.granted_at, row.skip_new_user_tips) {
            continue;
        }
        // awarded_for_trust_level?
        if row.staff && badge.id <= 4 {
            continue;
        }
        // notification_locale
        let locale = match row.locale.as_deref().filter(|l| !l.is_empty()) {
            Some(l) if allow_user_locale => l.to_string(),
            _ => default_locale.clone(),
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
            "username": row.username,
        });
        let notification_id: i64 = sqlx::query_scalar(
            "INSERT INTO notifications (notification_type, user_id, data, read, high_priority, \
                                        created_at, updated_at) \
             VALUES ($1, $2, $3, FALSE, FALSE, clock_timestamp(), clock_timestamp()) RETURNING id",
        )
        .bind(GRANTED_BADGE)
        .bind(row.user_id)
        .bind(data.to_string())
        .fetch_one(&mut *conn)
        .await?;
        crate::bus::publish_notifications_state(bus, &mut *conn, settings, row.user_id).await?;
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
