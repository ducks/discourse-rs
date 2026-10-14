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
    pub trigger: Option<i32>,
}

const BADGE_COLUMNS: &str = "id, name, badge_type_id, badge_grouping_id, allow_title, multiple_grant, \
     target_posts, query, enabled, auto_revoke, trigger";

impl Badge {
    /// `Badge.find(id)`
    pub async fn find(conn: &mut PgConnection, id: i32) -> Result<Option<Badge>, sqlx::Error> {
        sqlx::query_as(&format!("SELECT {BADGE_COLUMNS} FROM badges WHERE id = $1"))
            .bind(id)
            .fetch_optional(conn)
            .await
    }

    /// `Badge.find_by(name:, enabled: true)`
    pub async fn find_enabled_by_name(
        conn: &mut PgConnection,
        name: &str,
    ) -> Result<Option<Badge>, sqlx::Error> {
        sqlx::query_as(&format!(
            "SELECT {BADGE_COLUMNS} FROM badges WHERE name = $1 AND enabled ORDER BY id LIMIT 1"
        ))
        .bind(name)
        .fetch_optional(conn)
        .await
    }

    pub async fn enabled_named(
        conn: &mut PgConnection,
        names: &[&str],
    ) -> Result<Vec<Badge>, sqlx::Error> {
        sqlx::query_as(&format!(
            "SELECT {BADGE_COLUMNS} FROM badges WHERE enabled AND name = ANY($1) ORDER BY id"
        ))
        .bind(names)
        .fetch_all(conn)
        .await
    }

    /// `Badge.where(trigger:)`, by id.
    pub async fn with_trigger(
        conn: &mut PgConnection,
        trigger: i32,
    ) -> Result<Vec<Badge>, sqlx::Error> {
        sqlx::query_as(&format!(
            "SELECT {BADGE_COLUMNS} FROM badges WHERE trigger = $1 ORDER BY id"
        ))
        .bind(trigger)
        .fetch_all(conn)
        .await
    }

    /// Every enabled badge, by id.
    pub async fn all_enabled(conn: &mut PgConnection) -> Result<Vec<Badge>, sqlx::Error> {
        sqlx::query_as(&format!(
            "SELECT {BADGE_COLUMNS} FROM badges WHERE enabled ORDER BY id"
        ))
        .fetch_all(conn)
        .await
    }

    /// `Badge.i18n_key(name)`: `badges.<name downcased, spaces to _>.name`.
    pub fn translation_key(&self) -> String {
        format!("badges.{}.name", self.name.to_lowercase().replace(' ', "_"))
    }

    /// `Badge.display_name(name)`: its translation, else the name.
    pub fn display_name(&self, i18n: &crate::i18n::I18n) -> String {
        i18n.t(&self.translation_key())
            .unwrap_or(&self.name)
            .to_string()
    }

    /// `awarded_for_trust_level?`
    pub fn awarded_for_trust_level(&self) -> bool {
        self.id <= 4
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
        if row.staff && badge.awarded_for_trust_level() {
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

/// `BadgeGranter.new(badge, user, opts)`'s options.
pub struct GrantOpts {
    /// `granted_by`: the system user unless given.
    pub granted_by: i32,
    pub post_id: Option<i32>,
    pub suppress_notification: bool,
    /// `created_at`: the grant's `granted_at`, now unless given.
    pub created_at: Option<chrono::NaiveDateTime>,
}

impl GrantOpts {
    /// A grant by the system user.
    pub fn system(post_id: Option<i32>) -> Self {
        GrantOpts {
            granted_by: SYSTEM_USER_ID,
            post_id,
            suppress_notification: false,
            created_at: None,
        }
    }
}

/// `Guardian.new(user).can_grant_badges?`: badges on and a staff user.
async fn can_grant_badges(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    user_id: i32,
) -> Result<bool, AppError> {
    if !settings.get("enable_badges")?.truthy() {
        return Ok(false);
    }
    let staff: Option<bool> =
        sqlx::query_scalar("SELECT admin OR moderator FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(&mut *conn)
            .await?;
    Ok(staff.unwrap_or(false))
}

/// `BadgeGranter.grant(badge, user, opts)`: the badge once (or once per
/// post for a multiple grant, or again without a post), UserBadge's
/// after_create (the grant count, the distinct badge count, the featured
/// ranks), the staff log of a grant by someone other than the system
/// user, and the granted_badge notification unless suppressed. The
/// user badge's id, the existing one when already granted; None when the
/// granter can't grant or the badge is disabled.
pub async fn grant(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    settings: &SiteSettings,
    i18n: &crate::i18n::I18n,
    badge: &Badge,
    user_id: i32,
    opts: &GrantOpts,
) -> Result<Option<i32>, AppError> {
    if !can_grant_badges(&mut *conn, settings, opts.granted_by).await? {
        return Ok(None);
    }
    if !badge.enabled {
        return Ok(None);
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
        return Ok(None);
    };
    // find_by(badge_id, user_id[, post_id for a multiple grant])
    let existing: Option<i32> = sqlx::query_scalar(
        "SELECT id FROM user_badges WHERE badge_id = $1 AND user_id = $2 \
         AND (NOT $3 OR post_id IS NOT DISTINCT FROM $4) ORDER BY id LIMIT 1",
    )
    .bind(badge.id)
    .bind(user_id)
    .bind(badge.multiple_grant)
    .bind(opts.post_id)
    .fetch_optional(&mut *conn)
    .await?;
    if let Some(id) = existing
        && !(badge.multiple_grant && opts.post_id.is_none())
    {
        return Ok(Some(id));
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
         VALUES ($1, $2, $3, COALESCE($6, clock_timestamp()), $4, $5, clock_timestamp()) \
         RETURNING id, granted_at",
    )
    .bind(badge.id)
    .bind(user_id)
    .bind(opts.granted_by)
    .bind(opts.post_id)
    .bind(seq)
    .bind(opts.created_at)
    .fetch_one(&mut *conn)
    .await?;
    // after_create
    sqlx::query("UPDATE badges SET grant_count = grant_count + 1 WHERE id = $1")
        .bind(badge.id)
        .execute(&mut *conn)
        .await?;
    update_distinct_badge_count(&mut *conn, Some(&[user_id])).await?;
    update_featured_ranks(&mut *conn, Some(&[user_id])).await?;
    user_badge_granted_unsupported(&mut *conn).await?;
    if opts.granted_by != SYSTEM_USER_ID {
        // StaffActionLogger#log_badge_grant
        user_history(
            &mut *conn,
            &History {
                action: GRANT_BADGE,
                acting_user_id: Some(opts.granted_by),
                target_user_id: user_id,
                details: Some(&badge.name),
                previous_value: None,
                new_value: None,
            },
        )
        .await?;
    }
    if !opts.suppress_notification
        && !suppress_notification(badge, granted_at, user.skip_new_user_tips.unwrap_or(false))
    {
        let notification_id = send_notification(
            conn,
            bus,
            settings,
            i18n,
            badge,
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
    // user.user_badges (enabled badges) where is_favorite
    let favorite: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM user_badges WHERE user_id = $1 AND badge_id = $2 AND is_favorite \
         AND badge_id IN (SELECT id FROM badges WHERE enabled))",
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
    Ok(Some(user_badge_id))
}

/// `BadgeGranter.revoke(user_badge, revoked_by:)`: the grant destroyed
/// (UserBadge's after_destroy, and its notification with it), logged
/// when someone revoked it, and the user's title taken away when it was
/// the badge's name or its custom name.
pub async fn revoke(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    settings: &SiteSettings,
    i18n: &crate::i18n::I18n,
    user_badge_id: i32,
    revoked_by: Option<i32>,
) -> Result<(), AppError> {
    let row: Option<(i32, i32, Option<i64>)> = sqlx::query_as(
        "DELETE FROM user_badges WHERE id = $1 RETURNING user_id, badge_id, notification_id",
    )
    .bind(user_badge_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((user_id, badge_id, notification_id)) = row else {
        return Ok(());
    };
    let badge = Badge::find(&mut *conn, badge_id)
        .await?
        .ok_or(Unsupported("revoking a grant of a missing badge"))?;
    // after_destroy
    sqlx::query("UPDATE badges SET grant_count = grant_count - 1 WHERE id = $1")
        .bind(badge.id)
        .execute(&mut *conn)
        .await?;
    update_distinct_badge_count(&mut *conn, Some(&[user_id])).await?;
    update_featured_ranks(&mut *conn, Some(&[user_id])).await?;
    if crate::plugins::web_hooks_active(&mut *conn, "user_badge_revoked").await? {
        return Err(Unsupported("user_badge_revoked web hooks").into());
    }
    // belongs_to :notification, dependent: :destroy
    if let Some(notification_id) = notification_id {
        let deleted: Option<i32> =
            sqlx::query_scalar("DELETE FROM notifications WHERE id = $1 RETURNING user_id")
                .bind(notification_id)
                .fetch_optional(&mut *conn)
                .await?;
        // after_commit refresh_notification_count
        if let Some(owner) = deleted {
            crate::bus::publish_notifications_state(bus, &mut *conn, settings, owner).await?;
        }
    }
    if let Some(by) = revoked_by {
        // StaffActionLogger#log_badge_revoke
        user_history(
            &mut *conn,
            &History {
                action: REVOKE_BADGE,
                acting_user_id: Some(by),
                target_user_id: user_id,
                details: Some(&badge.name),
                previous_value: None,
                new_value: None,
            },
        )
        .await?;
    }
    // TranslationOverride.find_by(translation_key:), in any locale.
    let custom_name: Option<String> = sqlx::query_scalar(
        "SELECT value FROM translation_overrides WHERE translation_key = $1 ORDER BY id LIMIT 1",
    )
    .bind(badge.translation_key())
    .fetch_optional(&mut *conn)
    .await?;
    let title: Option<String> = sqlx::query_scalar("SELECT title FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
    let matches = title.as_deref() == Some(badge.name.as_str())
        || custom_name
            .as_deref()
            .is_some_and(|c| !c.trim().is_empty() && title.as_deref() == Some(c));
    if matches {
        if let Some(by) = revoked_by {
            // StaffActionLogger#log_title_revoke
            user_history(
                &mut *conn,
                &History {
                    action: REVOKE_TITLE,
                    acting_user_id: Some(by),
                    target_user_id: user_id,
                    details: Some(TITLE_REVOKE_REASON),
                    previous_value: title.as_deref(),
                    new_value: None,
                },
            )
            .await?;
        }
        set_title(&mut *conn, settings, i18n, user_id, None).await?;
    }
    Ok(())
}

/// The revoke reason both title revocations log.
pub const TITLE_REVOKE_REASON: &str =
    "user title was same as revoked badge name or custom badge name";

/// `user.title = title; user.save!`: before_save
/// `check_if_title_is_badged_granted`, the row with its updated_at, and
/// User's after_save.
pub async fn set_title(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    i18n: &crate::i18n::I18n,
    user_id: i32,
    title: Option<&str>,
) -> Result<(), AppError> {
    #[derive(sqlx::FromRow)]
    struct UserRow {
        title: Option<String>,
        admin: bool,
        active: bool,
        username_lower: String,
        name: Option<String>,
    }
    let user: UserRow = sqlx::query_as(
        "SELECT title, admin, active, username_lower, name FROM users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?;
    if user.title.as_deref() == title {
        // Nothing changed: save! writes nothing, after_save still runs.
        return crate::user_updater::after_save(
            conn,
            settings,
            user_id,
            user.admin && user.active,
            &user.username_lower,
            user.name.as_deref(),
        )
        .await;
    }
    check_if_title_is_badged_granted(&mut *conn, i18n, user_id, title).await?;
    sqlx::query("UPDATE users SET title = $2, updated_at = clock_timestamp() WHERE id = $1")
        .bind(user_id)
        .bind(title)
        .execute(&mut *conn)
        .await?;
    crate::user_updater::after_save(
        conn,
        settings,
        user_id,
        user.admin && user.active,
        &user.username_lower,
        user.name.as_deref(),
    )
    .await
}

/// User's before_save `check_if_title_is_badged_granted` for a changed
/// title: the profile's granted_title_badge_id is the badge (of the
/// user's enabled badges) that allows titles and whose display name or
/// name is the title, else NULL.
pub async fn check_if_title_is_badged_granted(
    conn: &mut PgConnection,
    i18n: &crate::i18n::I18n,
    user_id: i32,
    title: Option<&str>,
) -> Result<(), AppError> {
    let mut matching: Option<i32> = None;
    if let Some(title) = title {
        let badges: Vec<Badge> = sqlx::query_as(&format!(
            "SELECT {BADGE_COLUMNS} FROM badges WHERE id IN ( \
               SELECT badge_id FROM user_badges WHERE user_id = $1 \
               AND badge_id IN (SELECT id FROM badges WHERE enabled)) \
             ORDER BY id"
        ))
        .bind(user_id)
        .fetch_all(&mut *conn)
        .await?;
        matching = badges
            .iter()
            .find(|b| b.allow_title && (b.display_name(i18n) == title || b.name == title))
            .map(|b| b.id);
    }
    // user_profile.update!: only a change writes.
    sqlx::query(
        "UPDATE user_profiles SET granted_title_badge_id = $2 \
         WHERE user_id = $1 AND granted_title_badge_id IS DISTINCT FROM $2",
    )
    .bind(user_id)
    .bind(matching)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// `UserHistory.actions`
pub const GRANT_BADGE: i32 = 13;
pub const REVOKE_BADGE: i32 = 14;
pub const REVOKE_TITLE: i32 = 83;
pub const CHANGE_TITLE: i32 = 84;

/// A user_histories row of the badge and title actions.
pub struct History<'a> {
    pub action: i32,
    pub acting_user_id: Option<i32>,
    pub target_user_id: i32,
    pub details: Option<&'a str>,
    pub previous_value: Option<&'a str>,
    pub new_value: Option<&'a str>,
}

/// `UserHistory.create!` (StaffActionLogger's when acting_user_id is set).
pub async fn user_history(conn: &mut PgConnection, h: &History<'_>) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO user_histories (action, acting_user_id, target_user_id, details, \
                                     previous_value, new_value, admin_only, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, FALSE, clock_timestamp(), clock_timestamp())",
    )
    .bind(h.action)
    .bind(h.acting_user_id)
    .bind(h.target_user_id)
    .bind(h.details)
    .bind(h.previous_value)
    .bind(h.new_value)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// `UserStat.update_distinct_badge_count(user_ids)`, everyone's for None.
pub async fn update_distinct_badge_count(
    conn: &mut PgConnection,
    user_ids: Option<&[i32]>,
) -> Result<(), sqlx::Error> {
    let filter = if user_ids.is_some() {
        "AND user_stats.user_id = ANY($1)"
    } else {
        ""
    };
    sqlx::query(&format!(
        "UPDATE user_stats SET distinct_badge_count = x.distinct_badge_count \
         FROM (SELECT users.id user_id, COUNT(distinct user_badges.badge_id) distinct_badge_count \
               FROM users LEFT JOIN user_badges ON user_badges.user_id = users.id \
                 AND (user_badges.badge_id IN (SELECT id FROM badges WHERE enabled)) \
               GROUP BY users.id) x \
         WHERE user_stats.user_id = x.user_id AND user_stats.distinct_badge_count <> x.distinct_badge_count \
           {filter}"
    ))
    .bind(user_ids.unwrap_or(&[]))
    .execute(conn)
    .await?;
    Ok(())
}

/// `UserBadge.update_featured_ranks!(user_ids)`, everyone's for None.
pub async fn update_featured_ranks(
    conn: &mut PgConnection,
    user_ids: Option<&[i32]>,
) -> Result<(), sqlx::Error> {
    let (tl_filter, filter) = if user_ids.is_some() {
        (
            "AND user_id = ANY($1)",
            "WHERE user_badges.user_id = ANY($1)",
        )
    } else {
        ("", "")
    };
    sqlx::query(&format!(
        "WITH featured_tl_badge AS ( \
           SELECT user_id, max(badge_id) as badge_id FROM user_badges \
           WHERE badge_id IN (1,2,3,4) {tl_filter} GROUP BY user_id \
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
           {filter} \
           GROUP BY user_badges.user_id, user_badges.badge_id \
         ) \
         UPDATE user_badges SET featured_rank = rank_number FROM ranks \
         WHERE ranks.badge_id = user_badges.badge_id AND ranks.user_id = user_badges.user_id \
           AND featured_rank IS DISTINCT FROM rank_number"
    ))
    .bind(user_ids.unwrap_or(&[]))
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
