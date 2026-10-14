//! app/models/about.rb with AboutSerializer, lib/statistics.rb and
//! Stat.api_stats: the about page's document. The stats are cached for 35
//! minutes under `about-stats` (StatsCacheable), as the scheduled job
//! keeps them.

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::guardian::Guardian;
use crate::site_settings::SiteSettings;
use crate::url::Urls;
use crate::{AppError, Unsupported};

/// `About.stats_cache_key`
const STATS_CACHE_KEY: &str = "about-stats";
/// `recalculate_stats_interval` (30 minutes) plus five.
const STATS_TTL: i64 = 35 * 60;

/// `UserAction::USER_ACTED_TYPES`: like, new topic, reply, new message.
const USER_ACTED_TYPES: &str = "1, 4, 5, 12";

/// `Statistics.valid_users`: real, activated, neither suspended nor
/// silenced (and approved when the site approves members).
fn valid_users(settings: &SiteSettings) -> Result<String, AppError> {
    let mut sql = "SELECT id, created_at, last_seen_at FROM users u WHERE u.id > 0 \
         AND NOT EXISTS (SELECT 1 FROM anonymous_users a WHERE a.user_id = u.id) \
         AND u.active \
         AND (u.suspended_till IS NULL OR u.suspended_till <= now()) \
         AND (u.silenced_till IS NULL OR u.silenced_till <= now())"
        .to_string();
    if settings.get("must_approve_users")?.truthy() {
        sql.push_str(" AND u.approved");
    }
    Ok(sql)
}

/// The counts since a day, 7 and 30 days ago of rows a query selects
/// (`created_at` unless named), and their total when asked.
async fn periods(
    conn: &mut PgConnection,
    from: &str,
    column: &str,
    count: bool,
    previous_30_days: bool,
    distinct: Option<&str>,
) -> Result<Vec<(&'static str, i64)>, AppError> {
    let what = match distinct {
        Some(c) => format!("COUNT(DISTINCT {c})"),
        None => "COUNT(*)".to_string(),
    };
    let one = async |conn: &mut PgConnection, filter: &str| -> Result<i64, AppError> {
        let sql = format!("SELECT {what} FROM ({from}) x {filter}");
        Ok(sqlx::query_scalar(&sql).fetch_one(&mut *conn).await?)
    };
    let mut out = vec![
        (
            "last_day",
            one(conn, &format!("WHERE {column} > now() - interval '1 day'")).await?,
        ),
        (
            "7_days",
            one(conn, &format!("WHERE {column} > now() - interval '7 days'")).await?,
        ),
        (
            "30_days",
            one(
                conn,
                &format!("WHERE {column} > now() - interval '30 days'"),
            )
            .await?,
        ),
    ];
    if previous_30_days {
        out.push((
            "previous_30_days",
            one(
                conn,
                &format!(
                    "WHERE {column} BETWEEN now() - interval '60 days' AND now() - interval '30 days'"
                ),
            )
            .await?,
        ));
    }
    if count {
        out.push(("count", one(conn, "").await?));
    }
    Ok(out)
}

fn put(stats: &mut Map<String, Value>, name: &str, values: Vec<(&str, i64)>) {
    for (key, value) in values {
        stats.insert(format!("{name}_{key}"), json!(value));
    }
}

/// `Statistics.participating_users_count(date)`
async fn participating(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    interval: &str,
) -> Result<i64, AppError> {
    let sql = format!(
        "WITH valid_users AS ({}) \
         SELECT COUNT(DISTINCT user_id) FROM ( \
           SELECT DISTINCT user_id FROM user_actions \
             WHERE created_at > now() - interval '{interval}' AND action_type IN ({USER_ACTED_TYPES}) \
           UNION SELECT DISTINCT user_id FROM chat_messages \
             WHERE created_at > now() - interval '{interval}' AND deleted_at IS NULL \
           UNION SELECT DISTINCT user_id FROM chat_message_reactions \
             WHERE created_at > now() - interval '{interval}' \
         ) participating_users JOIN valid_users ON valid_users.id = participating_users.user_id",
        valid_users(settings)?
    );
    Ok(sqlx::query_scalar(&sql).fetch_one(&mut *conn).await?)
}

/// `Stat.api_stats`: the core stats exposed through the API, then the
/// enabled plugins' (chat's).
pub async fn api_stats(
    conn: &mut PgConnection,
    settings: &SiteSettings,
) -> Result<Map<String, Value>, AppError> {
    if settings.get("display_eu_visitor_stats")?.truthy() {
        return Err(Unsupported("visitor stats on the about page (application requests)").into());
    }
    let mut stats = Map::new();
    let topics =
        "SELECT created_at FROM topics WHERE deleted_at IS NULL AND archetype <> 'private_message'";
    put(
        &mut stats,
        "topics",
        periods(conn, topics, "created_at", true, false, None).await?,
    );
    let posts = "SELECT created_at FROM posts WHERE deleted_at IS NULL";
    put(
        &mut stats,
        "posts",
        periods(conn, posts, "created_at", true, false, None).await?,
    );
    let valid = valid_users(settings)?;
    put(
        &mut stats,
        "users",
        periods(conn, &valid, "created_at", true, false, None).await?,
    );
    put(
        &mut stats,
        "active_users",
        periods(conn, &valid, "last_seen_at", false, false, None).await?,
    );
    let likes = "SELECT created_at FROM user_actions WHERE action_type = 1";
    put(
        &mut stats,
        "likes",
        periods(conn, likes, "created_at", true, false, None).await?,
    );
    let mut participating_users = Vec::new();
    for (key, interval) in [
        ("last_day", "1 day"),
        ("7_days", "7 days"),
        ("30_days", "30 days"),
    ] {
        participating_users.push((key, participating(conn, settings, interval).await?));
    }
    put(&mut stats, "participating_users", participating_users);
    if settings.get("chat_enabled")?.truthy() {
        let messages = "SELECT created_at, user_id FROM chat_messages WHERE deleted_at IS NULL";
        put(
            &mut stats,
            "chat_messages",
            periods(conn, messages, "created_at", true, true, None).await?,
        );
        put(
            &mut stats,
            "chat_users",
            periods(conn, messages, "created_at", true, true, Some("user_id")).await?,
        );
        // Chat::Channel.where(status: :open)
        let channels =
            "SELECT created_at FROM chat_channels WHERE deleted_at IS NULL AND status = 0";
        put(
            &mut stats,
            "chat_channels",
            periods(conn, channels, "created_at", true, true, None).await?,
        );
    }
    Ok(stats)
}

/// `About.fetch_cached_stats`: the cached stats, computed and cached when
/// there are none.
pub async fn cached_stats(
    conn: &mut PgConnection,
    settings: &SiteSettings,
) -> Result<Value, AppError> {
    if let Some(cached) = crate::owned_schema::cached_get(conn, STATS_CACHE_KEY).await?
        && let Ok(value) = serde_json::from_str(&cached)
    {
        return Ok(value);
    }
    let stats = Value::Object(api_stats(conn, settings).await?);
    crate::owned_schema::cached_setex(conn, STATS_CACHE_KEY, STATS_TTL, &stats.to_string()).await?;
    Ok(stats)
}

/// `About#moderators` / `#admins`: staff of that kind the viewer may see,
/// most recently seen first, less members of about_page_hidden_groups
/// (bar the other staff group) and hidden profiles for non-staff.
async fn staff(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
    flag: &str,
    ignore_group: i32,
) -> Result<Vec<i32>, AppError> {
    if settings.get("hide_user_profiles_from_public")?.truthy() && guardian.user_id().is_none() {
        return Ok(Vec::new());
    }
    let hidden_groups: Vec<i32> = settings
        .get("about_page_hidden_groups")?
        .to_s()
        .split('|')
        .filter_map(|g| g.trim().parse().ok())
        .filter(|g| *g != ignore_group)
        .collect();
    // No hidden groups: an empty array, which leaves no one out.
    let mut sql = format!(
        "SELECT u.id FROM users u WHERE u.{flag} AND u.id > 0 \
         AND NOT EXISTS (SELECT 1 FROM group_users gu WHERE gu.group_id = ANY($1) AND gu.user_id = u.id)"
    );
    if !guardian.is_staff() && settings.get("allow_users_to_hide_profile")?.truthy() {
        sql.push_str(
            " AND NOT EXISTS (SELECT 1 FROM user_options uo WHERE uo.user_id = u.id AND uo.hide_profile)",
        );
    }
    // order(last_seen_at: :desc): Postgres puts NULLs first.
    sql.push_str(" ORDER BY u.last_seen_at DESC");
    Ok(sqlx::query_scalar(&sql)
        .bind(&hidden_groups)
        .fetch_all(&mut *conn)
        .await?)
}

/// UserAboutSerializer: BasicUserSerializer with title and last_seen_at.
async fn about_user(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    urls: &Urls<'_>,
    id: i32,
) -> Result<Value, AppError> {
    if settings.get("enable_user_status")?.truthy() {
        return Err(Unsupported("user status on BasicUserSerializer").into());
    }
    let (username, name, uploaded_avatar_id, title, last_seen_at): (
        String,
        Option<String>,
        Option<i32>,
        Option<String>,
        Option<chrono::NaiveDateTime>,
    ) = sqlx::query_as(
        "SELECT username, name, uploaded_avatar_id, title, last_seen_at FROM users WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&mut *conn)
    .await?;
    let mut out = Map::new();
    out.insert("id".into(), json!(id));
    out.insert("username".into(), json!(username));
    if settings.get("enable_names")?.truthy() {
        out.insert("name".into(), json!(name));
    }
    out.insert(
        "avatar_template".into(),
        json!(crate::avatar::avatar_template(
            urls,
            id,
            &username,
            uploaded_avatar_id,
            None
        )?),
    );
    out.insert("title".into(), json!(title));
    out.insert(
        "last_seen_at".into(),
        json!(last_seen_at.map(crate::topic_list::time_json)),
    );
    Ok(Value::Object(out))
}

/// `About` serialized by AboutSerializer: `{users, about}`.
pub async fn document(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    urls: &Urls<'_>,
    guardian: &Guardian,
) -> Result<Value, AppError> {
    if settings.get("content_localization_enabled")?.truthy() {
        return Err(Unsupported("localized site settings on the about page").into());
    }
    if !settings.get("about_banner_image")?.is_blank() {
        return Err(Unsupported("the about page's banner image").into());
    }
    let has_category_moderators: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM category_moderation_groups)")
            .fetch_one(&mut *conn)
            .await?;
    if has_category_moderators {
        return Err(Unsupported("category moderators on the about page").into());
    }
    // Group::AUTO_GROUPS: admins 1, moderators 2.
    let moderators = staff(conn, settings, guardian, "moderator", 1).await?;
    let admins = staff(conn, settings, guardian, "admin", 2).await?;
    let mut users = Vec::new();
    let mut seen = Vec::new();
    for id in moderators.iter().chain(admins.iter()) {
        if !seen.contains(id) {
            seen.push(*id);
            users.push(about_user(conn, settings, urls, *id).await?);
        }
    }
    // Discourse.site_creation_date
    let created: Option<chrono::NaiveDateTime> = sqlx::query_scalar(
        "SELECT created_at FROM schema_migration_details ORDER BY created_at LIMIT 1",
    )
    .fetch_optional(&mut *conn)
    .await?;
    let mut about = Map::new();
    // can_see_about_stats? is always true.
    about.insert("stats".into(), cached_stats(conn, settings).await?);
    about.insert(
        "description".into(),
        json!(settings.get("site_description")?),
    );
    about.insert(
        "extended_site_description".into(),
        json!(settings.get("extended_site_description_cooked")?),
    );
    about.insert("banner_image".into(), Value::Null);
    about.insert(
        "site_creation_date".into(),
        json!(created.map(crate::topic_list::time_json)),
    );
    about.insert("title".into(), json!(settings.get("title")?));
    about.insert("locale".into(), json!(settings.get("default_locale")?));
    about.insert("version".into(), json!(crate::schema::discourse_version()));
    about.insert("https".into(), json!(settings.get("force_https")?));
    about.insert("can_see_about_stats".into(), json!(true));
    // can_see_site_contact_details?
    if !settings.get("login_required")?.truthy() || guardian.user_id().is_some() {
        about.insert("contact_url".into(), json!(settings.get("contact_url")?));
        about.insert(
            "contact_email".into(),
            json!(settings.get("contact_email")?),
        );
    }
    about.insert("moderator_ids".into(), json!(moderators));
    about.insert("admin_ids".into(), json!(admins));
    about.insert("category_moderators".into(), json!([]));
    Ok(json!({ "users": users, "about": about }))
}

/// Jobs::AboutStats, every 30 minutes.
pub const JOB: &str = "about_stats";

/// `About.refresh_stats`: computed again and cached.
pub async fn refresh(state: &crate::AppState) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let stats = Value::Object(api_stats(&mut conn, &settings).await?);
    crate::owned_schema::cached_setex(&mut conn, STATS_CACHE_KEY, STATS_TTL, &stats.to_string())
        .await?;
    Ok(())
}
