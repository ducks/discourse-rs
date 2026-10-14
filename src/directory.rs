//! The user directory: app/models/directory_item.rb's refresh (with the
//! solved and gamification plugins' column queries, which Rails runs
//! whether or not the plugins are enabled), Jobs::DirectoryRefresh, and
//! DirectoryItemsQuery with DirectoryItemSerializer for
//! /directory_items.

use std::collections::HashMap;

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::guardian::Guardian;
use crate::site_settings::SiteSettings;
use crate::url::Urls;
use crate::{AppError, Unsupported};

/// `DirectoryItem.period_types`, in Enum order.
pub const PERIODS: [(&str, i32); 6] = [
    ("all", 1),
    ("yearly", 2),
    ("monthly", 3),
    ("weekly", 4),
    ("daily", 5),
    ("quarterly", 6),
];

/// `DirectoryItemsQuery::PAGE_SIZE` and `PAGE_LIMIT`.
pub const PAGE_SIZE: i64 = 50;
pub const PAGE_LIMIT: i64 = 10;

/// `DirectoryColumn.automatic_column_names`
const AUTOMATIC_COLUMNS: [&str; 7] = [
    "likes_received",
    "likes_given",
    "topics_entered",
    "topic_count",
    "post_count",
    "posts_read",
    "days_visited",
];

/// The plugins' columns in the order their queries run (plugin load
/// order: discourse-gamification, discourse-solved).
const PLUGIN_COLUMNS: [&str; 2] = ["gamification_score", "solutions"];

/// A key without expiry, as Redis' SET leaves it: a century.
const FOREVER: i64 = 100 * 365 * 86_400;

fn period_type(name: &str) -> Option<i32> {
    PERIODS.iter().find(|(n, _)| *n == name).map(|(_, t)| *t)
}

/// `period_query_args`' `since`, as SQL.
fn since(period: &str) -> &'static str {
    match period {
        "daily" => "now() - interval '1 day'",
        "weekly" => "now() - interval '1 week'",
        "monthly" => "now() - interval '1 month'",
        "quarterly" => "now() - interval '3 months'",
        "yearly" => "now() - interval '1 year'",
        _ => "now() - interval '1000 years'",
    }
}

/// `with_prepare_refresh_period_cte`, around the final statement.
fn refresh_cte(since: &str, final_query: &str) -> String {
    format!(
        "WITH eligible_users AS ( \
           SELECT id FROM users WHERE id > 0 AND active AND silenced_till IS NULL \
         ), recent_user_actions AS ( \
           WITH visible_topics AS ( \
             SELECT t.id FROM topics t \
             WHERE t.deleted_at IS NULL AND t.visible AND t.archetype = 'regular' \
           ), visible_posts AS ( \
             SELECT p.id FROM posts p JOIN visible_topics vt ON vt.id = p.topic_id \
             WHERE p.deleted_at IS NULL AND p.post_type = 1 AND NOT p.hidden \
           ) \
           SELECT ua.user_id, \
             SUM(CASE WHEN ua.action_type = 2 AND vp.id IS NOT NULL THEN 1 ELSE 0 END) AS likes_received, \
             SUM(CASE WHEN ua.action_type = 1 AND vp.id IS NOT NULL THEN 1 ELSE 0 END) AS likes_given, \
             SUM(CASE WHEN ua.action_type = 5 AND vp.id IS NOT NULL THEN 1 ELSE 0 END) AS post_count, \
             SUM(CASE WHEN ua.action_type = 4 AND vt.id IS NOT NULL THEN 1 ELSE 0 END) AS topic_count \
           FROM user_actions ua \
           LEFT JOIN visible_posts vp ON vp.id = ua.target_post_id \
           LEFT JOIN visible_topics vt ON vt.id = ua.target_topic_id \
           WHERE ua.created_at > {since} \
           GROUP BY ua.user_id \
         ), viewed_topics AS ( \
           SELECT user_id, COUNT(*) AS topics_entered FROM topic_views \
           WHERE topic_views.viewed_at > {since} GROUP BY topic_views.user_id \
         ), visited_topics AS ( \
           SELECT user_visits.user_id, COUNT(*) AS days_visited, \
             COALESCE(SUM(user_visits.posts_read), 0) AS posts_read \
           FROM user_visits WHERE user_visits.visited_at > {since} GROUP BY user_visits.user_id \
         ), directory_cte AS ( \
           SELECT h.id AS user_id, \
             COALESCE(rua.likes_received, 0) AS likes_received, \
             COALESCE(rua.likes_given, 0) AS likes_given, \
             COALESCE(tv.topics_entered, 0) AS topics_entered, \
             COALESCE(vi.days_visited, 0) AS days_visited, \
             COALESCE(vi.posts_read, 0) AS posts_read, \
             COALESCE(rua.topic_count, 0) AS topic_count, \
             COALESCE(rua.post_count, 0) AS post_count \
           FROM eligible_users h \
           LEFT JOIN recent_user_actions rua ON rua.user_id = h.id \
           LEFT JOIN viewed_topics tv ON tv.user_id = h.id \
           LEFT JOIN visited_topics vi ON vi.user_id = h.id \
         ) {final_query}"
    )
}

/// discourse-gamification's DirectoryIntegration.query.
fn gamification_query(since: &str) -> String {
    format!(
        "WITH default_leaderboard AS ( \
           SELECT id, from_date, to_date FROM gamification_leaderboards ORDER BY id ASC LIMIT 1 \
         ), total_score AS ( \
           SELECT gls.user_id, SUM(gls.score) AS score \
           FROM gamification_leaderboard_scores gls \
           INNER JOIN default_leaderboard lb ON gls.leaderboard_id = lb.id \
           WHERE gls.date >= {since} \
             AND ((lb.from_date IS NULL OR gls.date >= lb.from_date) \
               AND (lb.to_date IS NULL OR gls.date <= lb.to_date)) \
           GROUP BY 1 \
         ), scored_directory AS ( \
           SELECT directory_items.user_id, COALESCE(total_score.score, 0) AS score \
           FROM directory_items LEFT JOIN total_score ON total_score.user_id = directory_items.user_id \
           WHERE directory_items.period_type = $1 \
         ) \
         UPDATE directory_items SET gamification_score = scored_directory.score \
         FROM scored_directory \
         WHERE scored_directory.user_id = directory_items.user_id \
           AND directory_items.period_type = $1 \
           AND scored_directory.score != directory_items.gamification_score"
    )
}

/// discourse-solved's `solutions` column query, after its first
/// statement (`SOLVED_RESET`).
const SOLVED_RESET: &str = "UPDATE directory_items di SET solutions = 0 WHERE di.period_type = $1 AND di.solutions IS NOT NULL";

fn solved_query(since: &str) -> String {
    format!(
        "WITH x AS ( \
           SELECT p.user_id, COUNT(DISTINCT sta.id) AS solutions \
           FROM discourse_solved_solved_topics AS st \
           JOIN discourse_solved_topic_answers AS sta ON sta.solved_topic_id = st.id \
             AND COALESCE(sta.created_at, {since}) > {since} \
           JOIN posts AS p ON p.id = sta.answer_post_id AND p.deleted_at IS NULL \
           JOIN topics AS t ON t.id = st.topic_id \
             AND t.archetype <> 'private_message' AND t.deleted_at IS NULL \
           JOIN users AS u ON u.id = p.user_id \
           WHERE u.id > 0 AND u.active AND u.silenced_till IS NULL AND u.suspended_till IS NULL \
           GROUP BY p.user_id \
         ) \
         UPDATE directory_items di SET solutions = x.solutions \
         FROM x WHERE x.user_id = di.user_id AND di.period_type = $1"
    )
}

/// `DirectoryItem.refresh_period!(period_type, force:)`
pub async fn refresh_period(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    period: &str,
    force: bool,
) -> Result<(), AppError> {
    let ptype = period_type(period).expect("a known period");
    crate::owned_schema::cached_setex(
        conn,
        &format!("directory_{ptype}"),
        FOREVER,
        &crate::clock::now().timestamp().to_string(),
    )
    .await?;
    if !settings.get("enable_user_directory")?.truthy() && !force {
        return Ok(());
    }
    let since = since(period);
    let mut tx = sqlx::Connection::begin(&mut *conn).await?;
    // Records of users who are gone, inactive or silenced.
    sqlx::query(
        "DELETE FROM directory_items USING directory_items di \
         LEFT JOIN users u ON (u.id = user_id AND u.active AND u.silenced_till IS NULL AND u.id > 0) \
         WHERE di.id = directory_items.id AND u.id IS NULL AND di.period_type = $1",
    )
    .bind(ptype)
    .execute(&mut *tx)
    .await?;
    add_missing_users(&mut tx, settings, ptype).await?;
    let update = refresh_cte(
        since,
        "UPDATE directory_items di SET \
           likes_received = directory_cte.likes_received, \
           likes_given = directory_cte.likes_given, \
           topics_entered = directory_cte.topics_entered, \
           days_visited = directory_cte.days_visited, \
           posts_read = directory_cte.posts_read, \
           topic_count = directory_cte.topic_count, \
           post_count = directory_cte.post_count \
         FROM directory_cte \
         WHERE directory_cte.user_id = di.user_id AND di.period_type = $1 AND ( \
           di.likes_received <> directory_cte.likes_received OR \
           di.likes_given <> directory_cte.likes_given OR \
           di.topics_entered <> directory_cte.topics_entered OR \
           di.days_visited <> directory_cte.days_visited OR \
           di.posts_read <> directory_cte.posts_read OR \
           di.topic_count <> directory_cte.topic_count OR \
           di.post_count <> directory_cte.post_count)",
    );
    sqlx::query(&update).bind(ptype).execute(&mut *tx).await?;
    for query in [
        gamification_query(since),
        SOLVED_RESET.to_string(),
        solved_query(since),
    ] {
        sqlx::query(&query).bind(ptype).execute(&mut *tx).await?;
    }
    if period == "all" {
        sqlx::query(
            "UPDATE user_stats s SET likes_given = d.likes_given, likes_received = d.likes_received, \
               topic_count = d.topic_count, post_count = d.post_count \
             FROM directory_items d \
             WHERE s.user_id = d.user_id AND d.period_type = 1 AND ( \
               s.likes_given <> d.likes_given OR s.likes_received <> d.likes_received OR \
               s.topic_count <> d.topic_count OR s.post_count <> d.post_count)",
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// `DirectoryItem.add_missing_users(period_type)`
async fn add_missing_users(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    ptype: i32,
) -> Result<(), AppError> {
    let columns: Vec<&str> = AUTOMATIC_COLUMNS
        .iter()
        .chain(PLUGIN_COLUMNS.iter())
        .copied()
        .collect();
    let zeros = vec!["0"; columns.len()].join(", ");
    let approved = if settings.get("must_approve_users")?.truthy() {
        "AND u.approved"
    } else {
        ""
    };
    let sql = format!(
        "INSERT INTO directory_items(period_type, user_id, {}) \
         SELECT $1, u.id, {zeros} FROM users u \
         LEFT JOIN directory_items di ON di.user_id = u.id AND di.period_type = $1 \
         WHERE di.id IS NULL AND u.id > 0 AND u.silenced_till IS NULL AND u.active \
           AND NOT EXISTS (SELECT 1 FROM anonymous_users WHERE anonymous_users.user_id = u.id) \
           {approved}",
        columns.join(", ")
    );
    sqlx::query(&sql).bind(ptype).execute(&mut *conn).await?;
    Ok(())
}

/// Jobs::DirectoryRefresh, hourly.
pub const JOB: &str = "directory_refresh";
const OLDER_PERIODS_REFRESH_KEY: &str = "directory_older_periods_last_refresh";

/// `Jobs::DirectoryRefresh#execute`: the daily period, and the others
/// on a small site or once a day.
pub async fn refresh_job(state: &crate::AppState) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    refresh_period(&mut conn, &settings, "daily", false).await?;
    let limit = settings.get("directory_hourly_refresh_max_users")?.to_i();
    let small_site = limit > 0 && {
        let humans: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE id > 0")
            .fetch_one(&mut *conn)
            .await?;
        humans <= limit
    };
    let due = match crate::owned_schema::cached_get(&mut conn, OLDER_PERIODS_REFRESH_KEY).await? {
        None => true,
        Some(last) => {
            last.parse::<i64>().unwrap_or(0) < crate::clock::now().timestamp() - 23 * 3600
        }
    };
    if small_site || due {
        for (period, _) in PERIODS.iter().filter(|(p, _)| *p != "daily") {
            refresh_period(&mut conn, &settings, period, false).await?;
        }
        crate::owned_schema::cached_setex(
            &mut conn,
            OLDER_PERIODS_REFRESH_KEY,
            FOREVER,
            &crate::clock::now().timestamp().to_string(),
        )
        .await?;
    }
    Ok(())
}

/// What /directory_items asks DirectoryItemsQuery for.
pub struct Query {
    pub period: String,
    pub group: Option<String>,
    pub exclude_groups: Option<String>,
    pub exclude_usernames: Option<String>,
    pub order: Option<String>,
    pub ascending: bool,
    pub name: Option<String>,
    pub username: Option<String>,
    pub page: i64,
    pub limit: i64,
}

/// Refusals the controller turns into errors.
pub enum Refused {
    /// `Discourse::InvalidParameters.new(:group)`
    GroupNotFound,
    /// `ensure_can_see_group_and_members!`
    InvalidAccess,
}

fn present(s: &Option<String>) -> Option<&str> {
    s.as_deref().filter(|v| !v.trim().is_empty())
}

/// DirectoryItemsQuery#call with prioritize_user, serialized: the items,
/// the total, and when the period was last refreshed.
pub async fn index(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    urls: &Urls<'_>,
    guardian: &Guardian,
    q: &Query,
) -> Result<Result<(Vec<Value>, i64, Value), Refused>, AppError> {
    let ptype = period_type(&q.period).expect("checked by the controller");
    if present(&q.name).is_some() {
        return Err(Unsupported("searching the user directory by name (UserSearch)").into());
    }
    let mut filters = vec![format!("di.period_type = {ptype}")];
    if let Some(name) = present(&q.group) {
        let group: Option<(i32, i32, i32)> = sqlx::query_as(
            "SELECT id, visibility_level, members_visibility_level FROM groups WHERE name = $1",
        )
        .bind(name)
        .fetch_optional(&mut *conn)
        .await?;
        let Some((id, visibility, members)) = group else {
            return Ok(Err(Refused::GroupNotFound));
        };
        if !crate::groups::can_see_group_and_members(conn, guardian, id, visibility, members)
            .await?
        {
            return Ok(Err(Refused::InvalidAccess));
        }
        filters.push(format!(
            "di.user_id IN (SELECT user_id FROM group_users WHERE group_id = {id})"
        ));
    }
    let mut excluded_user_ids: Vec<i32> = Vec::new();
    if let Some(names) = present(&q.exclude_groups) {
        let names: Vec<String> = names.split('|').map(str::to_string).collect();
        let groups: Vec<(i32, i32, i32)> = sqlx::query_as(
            "SELECT id, visibility_level, members_visibility_level FROM groups WHERE name = ANY($1)",
        )
        .bind(&names)
        .fetch_all(&mut *conn)
        .await?;
        let mut visible = Vec::new();
        for (id, v, m) in groups {
            if crate::groups::can_see_group_and_members(conn, guardian, id, v, m).await? {
                visible.push(id);
            }
        }
        excluded_user_ids =
            sqlx::query_scalar("SELECT user_id FROM group_users WHERE group_id = ANY($1)")
                .bind(&visible)
                .fetch_all(&mut *conn)
                .await?;
        filters.push(format!(
            "NOT (di.user_id = ANY('{{{}}}'::int[]))",
            excluded_user_ids
                .iter()
                .map(i32::to_string)
                .collect::<Vec<_>>()
                .join(",")
        ));
    }
    if let Some(names) = present(&q.exclude_usernames) {
        let lower: Vec<String> = names.split(',').map(str::to_lowercase).collect();
        let ids: Vec<i32> =
            sqlx::query_scalar("SELECT id FROM users WHERE username_lower = ANY($1)")
                .bind(&lower)
                .fetch_all(&mut *conn)
                .await?;
        filters.push(format!(
            "NOT (di.user_id = ANY('{{{}}}'::int[]))",
            ids.iter().map(i32::to_string).collect::<Vec<_>>().join(",")
        ));
    }
    if let Some(username) = present(&q.username) {
        let id: Option<i32> = sqlx::query_scalar("SELECT id FROM users WHERE username_lower = $1")
            .bind(username.to_lowercase())
            .fetch_optional(&mut *conn)
            .await?;
        filters.push(match id {
            Some(id) => format!("di.user_id = {id}"),
            None => "false".into(),
        });
    }
    let active = active_column_names(conn).await?;
    let direction = if q.ascending { "ASC" } else { "DESC" };
    let order = q
        .order
        .as_deref()
        .filter(|o| !o.is_empty())
        .unwrap_or(AUTOMATIC_COLUMNS[0]);
    let (join, order_by) = if active.iter().any(|c| c == order) {
        (String::new(), format!("di.{order} {direction}, di.id ASC"))
    } else if order == "username" {
        (
            "JOIN users ou ON ou.id = di.user_id".to_string(),
            format!("ou.username {direction}, di.id ASC"),
        )
    } else {
        let field: Option<i32> = sqlx::query_scalar(if guardian.is_staff() {
            "SELECT id FROM user_fields WHERE name = $1 LIMIT 1"
        } else {
            "SELECT id FROM user_fields WHERE name = $1 AND (show_on_profile OR show_on_user_card) LIMIT 1"
        })
        .bind(order)
        .fetch_optional(&mut *conn)
        .await?;
        if field.is_some() {
            return Err(Unsupported("ordering the user directory by a user field").into());
        }
        (
            String::new(),
            format!("di.{} {direction}, di.id ASC", AUTOMATIC_COLUMNS[0]),
        )
    };
    let filter = filters.join(" AND ");
    let total: i64 = sqlx::query_scalar(&format!(
        "SELECT COUNT(*) FROM directory_items di {join} WHERE {filter}"
    ))
    .fetch_one(&mut *conn)
    .await?;
    let mut ids: Vec<(i32, i32)> = sqlx::query_as(&format!(
        "SELECT di.id, di.user_id FROM directory_items di {join} WHERE {filter} \
         ORDER BY {order_by} LIMIT {} OFFSET {}",
        q.limit,
        q.limit * q.page
    ))
    .fetch_all(&mut *conn)
    .await?;
    // prioritize_user!: the viewer first on the first page when they are
    // not among its first ten.
    if let Some(me) = guardian.user_id()
        && !ids.is_empty()
        && q.page == 0
        && present(&q.group).is_none()
    {
        let position = ids.iter().position(|(_, u)| *u == me);
        if position.unwrap_or(10) >= 10 && !excluded_user_ids.contains(&me) {
            let mine: Option<i32> = sqlx::query_scalar(
                "SELECT id FROM directory_items WHERE period_type = $1 AND user_id = $2 LIMIT 1",
            )
            .bind(ptype)
            .bind(me)
            .fetch_optional(&mut *conn)
            .await?;
            if let Some(id) = mine {
                ids.insert(0, (id, me));
            }
        }
    }
    let mut items = Vec::new();
    for (id, _) in &ids {
        items.push(serialize(conn, settings, urls, &active, *id).await?);
    }
    let last_updated_at = match crate::owned_schema::cached_get(conn, &format!("directory_{ptype}"))
        .await?
        .and_then(|v| v.parse::<i64>().ok())
        .and_then(|t| chrono::DateTime::from_timestamp(t, 0))
    {
        Some(at) => json!(crate::topic_list::time_json(at.naive_utc())),
        None => Value::Null,
    };
    Ok(Ok((items, total, last_updated_at)))
}

/// `DirectoryColumn.active_column_names`
async fn active_column_names(conn: &mut PgConnection) -> Result<Vec<String>, AppError> {
    // type: automatic 0, plugin 2; pluck without ORDER BY reads the heap.
    Ok(
        sqlx::query_scalar("SELECT name FROM directory_columns WHERE type IN (0, 2) AND enabled")
            .fetch_all(&mut *conn)
            .await?,
    )
}

/// DirectoryItemSerializer: the user (UserNameSerializer with
/// UserPrimaryGroupMixin), the id, the active columns, and time_read for
/// the all-time period.
async fn serialize(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    urls: &Urls<'_>,
    active: &[String],
    item_id: i32,
) -> Result<Value, AppError> {
    if settings.get("enable_user_status")?.truthy() {
        return Err(Unsupported("user status on BasicUserSerializer").into());
    }
    let row: (i32, i32) =
        sqlx::query_as("SELECT user_id, period_type FROM directory_items WHERE id = $1")
            .bind(item_id)
            .fetch_one(&mut *conn)
            .await?;
    let (user_id, ptype) = row;
    #[allow(clippy::type_complexity)]
    let user: (
        String,
        Option<String>,
        Option<i32>,
        Option<String>,
        i32,
        bool,
        bool,
        Option<i32>,
        Option<i32>,
    ) = sqlx::query_as(
        "SELECT username, name, uploaded_avatar_id, title, trust_level, admin, moderator, \
           primary_group_id, flair_group_id FROM users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?;
    let (username, name, avatar, title, trust_level, admin, moderator, primary, flair) = user;
    let mut u = Map::new();
    u.insert("id".into(), json!(user_id));
    u.insert("username".into(), json!(username));
    if settings.get("enable_names")?.truthy() {
        u.insert("name".into(), json!(name));
    }
    u.insert(
        "avatar_template".into(),
        json!(crate::avatar::avatar_template(
            urls, user_id, &username, avatar, None
        )?),
    );
    u.insert("title".into(), json!(title));
    let group_ids: Vec<i32> = primary.iter().chain(flair.iter()).copied().collect();
    let groups: HashMap<i32, crate::groups::Group> = if group_ids.is_empty() {
        HashMap::new()
    } else {
        crate::groups::load(&mut *conn, &group_ids).await?
    };
    crate::groups::user_group_fields(&mut u, &groups, primary, flair)?;
    if admin {
        u.insert("admin".into(), json!(true));
    }
    if moderator {
        u.insert("moderator".into(), json!(true));
    }
    u.insert("trust_level".into(), json!(trust_level));
    let mut out = Map::new();
    out.insert("id".into(), json!(user_id));
    for column in active {
        let value: Option<i32> = sqlx::query_scalar(&format!(
            "SELECT {column} FROM directory_items WHERE id = $1"
        ))
        .bind(item_id)
        .fetch_one(&mut *conn)
        .await?;
        out.insert(column.clone(), json!(value));
    }
    if ptype == 1 {
        let time_read: Option<i32> =
            sqlx::query_scalar("SELECT time_read FROM user_stats WHERE user_id = $1")
                .bind(user_id)
                .fetch_optional(&mut *conn)
                .await?;
        out.insert("time_read".into(), json!(time_read));
    }
    out.insert("user".into(), Value::Object(u));
    Ok(Value::Object(out))
}
