//! The reports that count rows per day: signups, topics, posts, likes,
//! flags, emails, bookmarks, visits, profile views, engaged users, new
//! contributors, response times, messages and accepted solutions, with
//! Report's helpers (report_about, basic_report_about, add_counts).

#![allow(clippy::type_complexity)]

use chrono::{Duration, NaiveDate, NaiveDateTime};
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use super::{Cx, Report, Viewer, date_json, float_json, points, real_users, round_to};
use crate::AppError;

/// A time as Rails quotes it into SQL: microseconds, as a timestamp.
pub fn lit(t: NaiveDateTime) -> String {
    format!(
        "'{}'::timestamp",
        super::bind_time(t).format("%Y-%m-%d %H:%M:%S%.6f")
    )
}

/// `"(1, 2, 3)"` for an IN list.
pub fn in_list(ids: &[i64]) -> String {
    if ids.is_empty() {
        return "(NULL)".into();
    }
    format!(
        "({})",
        ids.iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// `SELECT date(col), count(*) ... GROUP BY 1 ORDER BY 1`: a `{date =>
/// count}` hash.
async fn per_day(
    conn: &mut PgConnection,
    sql: &str,
    group: Option<&str>,
) -> Result<Vec<(NaiveDate, i64)>, AppError> {
    let mut q = sqlx::query_as::<_, (Option<NaiveDate>, i64)>(sql);
    if let Some(g) = group {
        q = q.bind(g);
    }
    Ok(q.fetch_all(conn)
        .await?
        .into_iter()
        .filter_map(|(d, n)| d.map(|d| (d, n)))
        .collect())
}

async fn count(conn: &mut PgConnection, sql: &str) -> Result<i64, AppError> {
    Ok(sqlx::query_scalar(sql).fetch_one(conn).await?)
}

/// A countable relation: `SELECT ... FROM <from> WHERE <conditions>`.
pub struct Subject {
    pub from: String,
    pub conditions: String,
}

impl Subject {
    fn count_sql(&self, extra: &str) -> String {
        let mut conds = self.conditions.clone();
        if !extra.is_empty() {
            if conds.is_empty() {
                conds = extra.to_string();
            } else {
                conds = format!("({conds}) AND ({extra})");
            }
        }
        if conds.is_empty() {
            format!("SELECT count(*) FROM {}", self.from)
        } else {
            format!("SELECT count(*) FROM {} WHERE {conds}", self.from)
        }
    }
}

/// `add_counts(report, subject_class, query_column, prev_period_count:)`
async fn add_counts(
    conn: &mut PgConnection,
    r: &mut Report,
    subject: &Subject,
    column: &str,
    prev_period_count: Option<i64>,
) -> Result<(), AppError> {
    if r.has_facet("prev_period") {
        let n = match prev_period_count {
            Some(n) => n,
            None => {
                let sql = subject.count_sql(&format!(
                    "{column} >= {} and {column} < {}",
                    lit(r.prev_start_date()),
                    lit(r.prev_end_date())
                ));
                count(conn, &sql).await?
            }
        };
        r.prev_period = Some(json!(n));
    }
    if r.has_facet("total") {
        r.total = Some(json!(count(conn, &subject.count_sql("")).await?));
    }
    if r.has_facet("prev30Days") {
        let sql = subject.count_sql(&format!(
            "{column} >= {} and {column} < {}",
            lit(r.start_date - Duration::days(30)),
            lit(r.start_date)
        ));
        r.prev30_days = Some(json!(count(conn, &sql).await?));
    }
    Ok(())
}

/// `report_about(report, subject_class, report_method)`: the per day
/// counts from the previous period's start when asked, split there.
async fn report_about(
    conn: &mut PgConnection,
    r: &mut Report,
    per_day_sql: impl Fn(NaiveDateTime, NaiveDateTime) -> String,
    subject: &Subject,
    column: &str,
) -> Result<(), AppError> {
    let data_start = if r.has_facet("prev_period") {
        r.prev_start_date()
    } else {
        r.start_date
    };
    let counts = per_day(conn, &per_day_sql(data_start, r.end_date), None).await?;
    let (counts, prev_count) = if r.has_facet("prev_period") {
        let (current, prev) = super::split_date_counts(counts, r.start_date);
        let n: i64 = prev.iter().map(|(_, n)| n).sum();
        (current, Some(n))
    } else {
        (counts, None)
    };
    r.data = points(&counts);
    add_counts(conn, r, subject, column, prev_count).await
}

/// The group filter (`filters.dig(:group)`), registered as a filter.
fn group_filter(r: &mut Report) -> Option<String> {
    let group = r.filter("group").cloned();
    r.add_filter("group", json!({"type": "group", "default": group}));
    group.as_ref().and_then(crate::params::scalar)
}

/// BasicUserSerializer from a row's columns.
fn basic_user(
    cx: &Cx<'_>,
    id: i32,
    username: &str,
    name: Option<String>,
    avatar: Option<i32>,
) -> Result<Value, AppError> {
    let mut m = Map::new();
    m.insert("id".into(), json!(id));
    m.insert("username".into(), json!(username));
    if cx.settings.get("enable_names")?.truthy() {
        m.insert("name".into(), json!(name));
    }
    m.insert(
        "avatar_template".into(),
        json!(cx.avatar_template(id, username, avatar)?),
    );
    Ok(Value::Object(m))
}

/// The filter's group as SQL: the value cast to an integer as Postgres
/// casts a quoted literal (bad input fails the report).
const GROUP_PARAM: &str = "$1::text::integer";

/// `report_signups`
pub async fn signups(conn: &mut PgConnection, cx: &Cx<'_>, r: &mut Report) -> Result<(), AppError> {
    r.icon = Some("user-plus".into());
    let group = group_filter(r);
    let real = real_users("users");
    if let Some(g) = &group {
        // basic_report_about starts the data before it queries.
        r.data = json!([]);
        let sql = format!(
            "SELECT date(users.created_at), count(*) FROM users \
             INNER JOIN group_users ON group_users.user_id = users.id \
             WHERE {real} AND users.created_at >= {} AND users.created_at <= {} \
               AND group_users.group_id = {GROUP_PARAM} \
             GROUP BY date(users.created_at) ORDER BY date(users.created_at)",
            lit(r.start_date),
            lit(r.end_date)
        );
        r.data = points(&per_day(conn, &sql, Some(g)).await?);
        let subject = Subject {
            from: "users".into(),
            conditions: real.clone(),
        };
        add_counts(conn, r, &subject, "users.created_at", None).await?;
    } else {
        let subject = Subject {
            from: "users".into(),
            conditions: real.clone(),
        };
        let real2 = real.clone();
        report_about(
            conn,
            r,
            move |s, e| {
                format!(
                    "SELECT date(users.created_at), count(*) FROM users \
                     WHERE {real2} AND users.created_at >= {} AND users.created_at <= {} \
                     GROUP BY date(users.created_at) ORDER BY date(users.created_at)",
                    lit(s),
                    lit(e)
                )
            },
            &subject,
            "created_at",
        )
        .await?;
    }
    if !r.include_related_items {
        return Ok(());
    }
    let (join, cond) = match &group {
        Some(_) => (
            "INNER JOIN group_users ON group_users.user_id = users.id",
            format!("AND group_users.group_id = {GROUP_PARAM}"),
        ),
        None => ("", String::new()),
    };
    let base = format!(
        "FROM users {join} WHERE {real} AND users.created_at BETWEEN {} AND {} {cond}",
        lit(r.start_date),
        lit(r.end_date)
    );
    let count_sql = format!("SELECT count(*) {base}");
    let mut q = sqlx::query_scalar::<_, i64>(&count_sql);
    if let Some(g) = &group {
        q = q.bind(g);
    }
    let total = q.fetch_one(&mut *conn).await?;
    let limit = r.limit.unwrap_or(super::RELATED_ITEMS_LIMIT);
    let sql = format!(
        "SELECT users.id, users.username, users.name, users.uploaded_avatar_id, users.created_at \
         {base} ORDER BY users.created_at DESC LIMIT {limit}"
    );
    let mut q =
        sqlx::query_as::<_, (i32, String, Option<String>, Option<i32>, NaiveDateTime)>(&sql);
    if let Some(g) = &group {
        q = q.bind(g);
    }
    let rows = q.fetch_all(&mut *conn).await?;
    let mut users = Vec::new();
    for (id, username, name, avatar, created_at) in rows {
        users.push(json!({
            "user": basic_user(cx, id, &username, name, avatar)?,
            "timestamp": super::iso8601(created_at),
        }));
    }
    r.related_items_totals = Some(json!({"users": total}));
    r.related_items = Some(json!({"users": users}));
    Ok(())
}

/// `Topic.listable_count_per_day` and the listable topics.
pub async fn topics(conn: &mut PgConnection, cx: &Cx<'_>, r: &mut Report) -> Result<(), AppError> {
    let filter = r.add_category_filter();
    let ids = super::category_ids(conn, cx, filter).await?;
    let mut cond =
        "topics.deleted_at IS NULL AND topics.archetype <> 'private_message'".to_string();
    if let Some(ids) = &ids {
        cond.push_str(&category_condition("topics.category_id", ids));
    }
    let sql = format!(
        "SELECT date(topics.created_at), count(*) FROM topics WHERE {cond} \
           AND topics.created_at >= {} AND topics.created_at <= {} \
         GROUP BY date(topics.created_at) ORDER BY date(topics.created_at)",
        lit(r.start_date),
        lit(r.end_date)
    );
    r.data = points(&per_day(conn, &sql, None).await?);
    let subject = Subject {
        from: "topics".into(),
        conditions: cond,
    };
    add_counts(conn, r, &subject, "topics.created_at", None).await
}

/// `where(category_id: ids)`: `=` for one, `IN` for several.
fn category_condition(column: &str, ids: &[i64]) -> String {
    if ids.len() == 1 {
        format!(" AND {column} = {}", ids[0])
    } else {
        format!(" AND {column} IN {}", in_list(ids))
    }
}

/// `Post.public_posts_count_per_day` and the public regular posts.
pub async fn posts(conn: &mut PgConnection, cx: &Cx<'_>, r: &mut Report) -> Result<(), AppError> {
    let filter = r.add_category_filter();
    let ids = super::category_ids(conn, cx, filter).await?;
    let mut cond = "posts.deleted_at IS NULL AND topics.archetype <> 'private_message' \
                    AND posts.post_type = 1"
        .to_string();
    if let Some(ids) = &ids {
        cond.push_str(&format!(" AND topics.category_id IN {}", in_list(ids)));
    }
    let from =
        "posts INNER JOIN topics ON topics.id = posts.topic_id AND topics.deleted_at IS NULL";
    let sql = format!(
        "SELECT date(posts.created_at), count(*) FROM {from} WHERE {cond} \
           AND posts.created_at >= {} AND posts.created_at <= {} \
         GROUP BY date(posts.created_at) ORDER BY date(posts.created_at)",
        lit(r.start_date),
        lit(r.end_date)
    );
    r.data = points(&per_day(conn, &sql, None).await?);
    let subject = Subject {
        from: from.into(),
        conditions: cond,
    };
    add_counts(conn, r, &subject, "posts.created_at", None).await
}

/// `post_action_report(report, post_action_type)` (likes).
pub async fn post_action_report(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    r: &mut Report,
    action_type: i32,
    icon: &str,
) -> Result<(), AppError> {
    r.icon = Some(icon.into());
    let filter = r.add_category_filter();
    let mut from = "post_actions".to_string();
    let mut cond = format!("post_actions.post_action_type_id = {action_type}");
    if let Some((id, include)) = filter {
        from.push_str(
            " INNER JOIN posts ON posts.id = post_actions.post_id AND posts.deleted_at IS NULL \
             INNER JOIN topics ON topics.id = posts.topic_id AND topics.deleted_at IS NULL",
        );
        cond.push_str(&in_category(conn, cx, "topics.category_id", id, include).await?);
    }
    let sql = format!(
        "SELECT date(post_actions.created_at), count(*) FROM {from} WHERE {cond} \
           AND post_actions.created_at >= {} AND post_actions.created_at <= {} \
         GROUP BY date(post_actions.created_at) ORDER BY date(post_actions.created_at)",
        lit(r.start_date),
        lit(r.end_date)
    );
    r.data = points(&per_day(conn, &sql, None).await?);
    let subject = Subject {
        from,
        conditions: cond,
    };
    add_counts(conn, r, &subject, "post_actions.created_at", None).await
}

/// `report_flags`: ReviewableFlaggedPost's scores by day.
pub async fn flags(conn: &mut PgConnection, cx: &Cx<'_>, r: &mut Report) -> Result<(), AppError> {
    let filter = r.add_category_filter();
    r.icon = Some("flag".into());
    r.higher_is_better = false;
    let from = "reviewable_scores \
                INNER JOIN reviewables ON reviewables.id = reviewable_scores.reviewable_id \
                INNER JOIN topics ON topics.id = reviewables.topic_id AND topics.deleted_at IS NULL";
    let mut cond = "reviewables.type = 'ReviewableFlaggedPost'".to_string();
    if let Some((id, include)) = filter {
        cond.push_str(&in_category(conn, cx, "topics.category_id", id, include).await?);
    }
    let sql = format!(
        "SELECT date(reviewable_scores.created_at), count(*) FROM {from} WHERE {cond} \
           AND reviewable_scores.created_at BETWEEN {} AND {} \
         GROUP BY date(reviewable_scores.created_at) ORDER BY date(reviewable_scores.created_at)",
        lit(r.start_date),
        lit(r.end_date)
    );
    r.data = points(&per_day(conn, &sql, None).await?);
    let subject = Subject {
        from: from.into(),
        conditions: cond,
    };
    add_counts(conn, r, &subject, "reviewable_scores.created_at", None).await
}

/// ` AND column = id`, or ` AND column IN (...)` over the category and
/// its subcategories (Category.subcategory_ids).
pub async fn in_category(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    column: &str,
    id: i64,
    include_subcategories: bool,
) -> Result<String, AppError> {
    Ok(if include_subcategories {
        let ids = super::subcategory_ids(conn, cx.settings, id).await?;
        format!(" AND {column} IN {}", in_list(&ids))
    } else {
        format!(" AND {column} = {id}")
    })
}

/// `column = id` for one category, `column IN (...)` for its tree.
fn category_where(column: &str, ids: &[i64]) -> String {
    format!("{column} IN {}", in_list(ids))
}

/// `report_emails`
pub async fn emails(conn: &mut PgConnection, r: &mut Report) -> Result<(), AppError> {
    let subject = Subject {
        from: "email_logs".into(),
        conditions: String::new(),
    };
    report_about(
        conn,
        r,
        |s, e| {
            format!(
                "SELECT DATE(created_at), count(*) FROM email_logs \
                 WHERE created_at BETWEEN {} AND {} GROUP BY DATE(created_at) ORDER BY DATE(created_at)",
                lit(s),
                lit(e)
            )
        },
        &subject,
        "created_at",
    )
    .await
}

/// `report_bookmarks`
pub async fn bookmarks(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    r: &mut Report,
) -> Result<(), AppError> {
    r.icon = Some("bookmark".into());
    let category = r.filter("category").cloned();
    r.add_filter("category", json!({"type": "category", "default": category}));
    let mut from = "bookmarks".to_string();
    let mut cond = format!(
        "bookmarks.created_at >= {} AND bookmarks.created_at <= {}",
        lit(r.start_date),
        lit(r.end_date)
    );
    if let Some(c) = category.as_ref().filter(|v| super::truthy(v)) {
        let id = crate::ruby::to_i(&crate::params::scalar(c).unwrap_or_default());
        from.push_str(
            " LEFT JOIN posts ON posts.id = bookmarks.bookmarkable_id AND bookmarks.bookmarkable_type = 'Post' \
             LEFT JOIN topics ON (topics.id = bookmarks.bookmarkable_id AND bookmarks.bookmarkable_type = 'Topic') \
               OR (topics.id = posts.topic_id)",
        );
        let ids = super::subcategory_ids(conn, cx.settings, id).await?;
        cond.push_str(&format!(
            " AND topics.deleted_at IS NULL AND posts.deleted_at IS NULL AND topics.category_id IN {}",
            in_list(&ids)
        ));
    }
    let sql = format!(
        "SELECT date(bookmarks.created_at), count(*) FROM {from} WHERE {cond} \
         GROUP BY date(bookmarks.created_at) ORDER BY date(bookmarks.created_at)"
    );
    r.data = points(&per_day(conn, &sql, None).await?);
    let subject = Subject {
        from: "bookmarks".into(),
        conditions: String::new(),
    };
    add_counts(conn, r, &subject, "bookmarks.created_at", None).await
}

/// `report_visits`: desktop and mobile visits per day.
pub async fn visits(conn: &mut PgConnection, cx: &Cx<'_>, r: &mut Report) -> Result<(), AppError> {
    let group = group_filter(r);
    r.icon = Some("user".into());
    r.modes = vec!["stacked_chart".into()];
    r.default_group_by = Some("weekly".into());
    // `group_id.to_i`
    let group_id = group.as_deref().map(crate::ruby::to_i);
    let (join, cond) = match group_id {
        Some(id) => (
            "INNER JOIN group_users ON group_users.user_id = user_visits.user_id",
            format!("AND group_users.group_id = {id}"),
        ),
        None => ("", String::new()),
    };
    let sql = format!(
        "SELECT visited_at, mobile, COUNT(*) AS visit_count, (SUM(COUNT(*)) OVER ())::text AS total \
         FROM user_visits {join} \
         WHERE visited_at >= {} AND visited_at <= {} {cond} \
         GROUP BY visited_at, mobile ORDER BY visited_at",
        lit(r.start_date),
        lit(r.end_date)
    );
    let rows: Vec<(NaiveDate, Option<bool>, i64, Option<String>)> =
        sqlx::query_as(&sql).fetch_all(&mut *conn).await?;
    let mut desktop = Vec::new();
    let mut mobile = Vec::new();
    for (date, is_mobile, n, _) in &rows {
        let point = json!({"x": date.format("%Y-%m-%d").to_string(), "y": n});
        if is_mobile.unwrap_or(false) {
            mobile.push(point);
        } else {
            desktop.push(point);
        }
    }
    r.data = if rows.is_empty() {
        json!([])
    } else {
        json!([
            {"req": "desktop", "label": cx.t("reports.visits.xaxis.desktop")?, "color": super::colors::TURQUOISE, "data": desktop},
            {"req": "mobile", "label": cx.t("reports.visits.xaxis.mobile")?, "color": super::colors::LIME, "data": mobile},
        ])
    };
    r.total = Some(match rows.first().and_then(|r| r.3.as_deref()) {
        Some(t) => super::decimal_json(t),
        None => json!(0),
    });
    if r.has_facet("prev30Days") {
        let n = count(
            conn,
            &format!(
                "SELECT count(*) FROM user_visits WHERE visited_at >= {} AND visited_at < {}",
                lit(r.start_date - Duration::days(30)),
                lit(r.start_date)
            ),
        )
        .await?;
        r.prev30_days = Some(json!(n));
    }
    Ok(())
}

/// `report_mobile_visits`
pub async fn mobile_visits(conn: &mut PgConnection, r: &mut Report) -> Result<(), AppError> {
    let sql = format!(
        "SELECT visited_at, count(*) FROM user_visits \
         WHERE visited_at >= '{}' AND visited_at <= '{}' AND mobile \
         GROUP BY visited_at ORDER BY visited_at",
        r.start_date.date(),
        r.end_date.date()
    );
    r.data = points(&per_day(conn, &sql, None).await?);
    r.total = Some(json!(
        count(conn, "SELECT count(*) FROM user_visits WHERE mobile").await?
    ));
    r.prev30_days = Some(json!(
        count(
            conn,
            &format!(
                "SELECT count(*) FROM user_visits WHERE mobile AND visited_at >= {} and visited_at < {}",
                lit(r.start_date - Duration::days(30)),
                lit(r.start_date)
            )
        )
        .await?
    ));
    Ok(())
}

/// `report_profile_views`
pub async fn profile_views(conn: &mut PgConnection, r: &mut Report) -> Result<(), AppError> {
    let group = group_filter(r);
    r.data = json!([]);
    let (join, cond) = match &group {
        Some(_) => (
            "INNER JOIN users ON users.id = user_profile_views.user_id \
             INNER JOIN group_users ON group_users.user_id = users.id",
            format!("AND group_users.group_id = {GROUP_PARAM}"),
        ),
        None => ("", String::new()),
    };
    let sql = format!(
        "SELECT date(viewed_at), count(*) FROM user_profile_views {join} \
         WHERE viewed_at >= {} AND viewed_at < {} {cond} \
         GROUP BY date(viewed_at) ORDER BY date(viewed_at)",
        lit(r.start_date),
        lit(r.end_date + Duration::days(1))
    );
    r.data = points(&per_day(conn, &sql, group.as_deref()).await?);
    // UserProfile.sum(:views)
    r.total = Some(json!(
        count(
            conn,
            "SELECT COALESCE(SUM(views), 0)::bigint FROM user_profiles"
        )
        .await?
    ));
    r.prev30_days = Some(json!(
        count(
            conn,
            &format!(
                "SELECT count(*) FROM user_profile_views WHERE viewed_at >= {} AND viewed_at < {}",
                lit(r.start_date - Duration::days(30)),
                lit(r.start_date + Duration::seconds(1))
            )
        )
        .await?
    ));
    Ok(())
}

/// `report_dau_by_mau`
pub async fn dau_by_mau(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    r: &mut Report,
) -> Result<(), AppError> {
    r.labels = json!([
        {"type": "date", "property": "x", "title": cx.t("reports.default.labels.day")?},
        {"type": "percent", "property": "y", "title": cx.t("reports.default.labels.percent")?},
    ]);
    r.average = true;
    r.percent = true;
    let data_start = if r.has_facet("prev_period") {
        r.prev_start_date()
    } else {
        r.start_date
    };
    let points_of = |rows: &[(NaiveDate, i64, i64)]| -> Vec<(NaiveDate, Value, f64)> {
        rows.iter()
            .map(|(d, dau, mau)| {
                if *mau == 0 {
                    (*d, json!(0), 0.0)
                } else {
                    let v = super::ceil_to(*dau as f64 / *mau as f64 * 100.0, 2);
                    (*d, float_json(v), v)
                }
            })
            .collect()
    };
    // dau_avg: a Float, nil for no points.
    let avg = |pts: &[(NaiveDate, Value, f64)]| -> Option<Value> {
        if pts.is_empty() {
            return None;
        }
        let sum: f64 = pts.iter().map(|p| p.2).sum();
        Some(float_json(super::ceil_to(sum / pts.len() as f64, 2)))
    };
    let rows = rollups(conn, data_start, r.end_date).await?;
    let mut pts = points_of(&rows);
    if r.has_facet("prev_period") {
        let start = r.start_date.date();
        let (current, prev): (Vec<_>, Vec<_>) = pts.into_iter().partition(|p| p.0 >= start);
        r.prev_period = avg(&prev);
        pts = current;
    }
    r.data = Value::Array(
        pts.iter()
            .map(|(d, y, _)| json!({"x": date_json(*d), "y": y}))
            .collect(),
    );
    if r.has_facet("prev30Days") {
        let rows = rollups(conn, r.start_date - Duration::days(30), r.start_date).await?;
        r.prev30_days = Some(avg(&points_of(&rows)).unwrap_or(Value::Null));
    }
    Ok(())
}

/// `UserVisitDailyRollup.fetch(start_date:, end_date:)`
async fn rollups(
    conn: &mut PgConnection,
    start: NaiveDateTime,
    end: NaiveDateTime,
) -> Result<Vec<(NaiveDate, i64, i64)>, AppError> {
    Ok(sqlx::query_as(
        "SELECT date, dau, mau FROM user_visit_daily_rollups \
         WHERE date BETWEEN $1 AND $2 ORDER BY date",
    )
    .bind(start.date())
    .bind(end.date())
    .fetch_all(conn)
    .await?)
}

/// `UserAction::USER_ACTED_TYPES`: like, new topic, reply, new message.
const USER_ACTED_TYPES: &str = "(1, 4, 5, 12)";

/// `UserAction.count_daily_engaged_users(start_date, end_date)`
async fn engaged_per_day(
    conn: &mut PgConnection,
    start: NaiveDateTime,
    end: NaiveDateTime,
) -> Result<Vec<(NaiveDate, i64)>, AppError> {
    per_day(
        conn,
        &format!(
            "SELECT day, COUNT(*) AS count FROM ( \
               SELECT DISTINCT date(created_at) AS day, user_id FROM user_actions \
               WHERE action_type IN {USER_ACTED_TYPES} AND created_at > {} AND created_at < {} \
             ) distinct_daily_users GROUP BY day ORDER BY day",
            lit(start),
            lit(end)
        ),
        None,
    )
    .await
}

/// `report_daily_engaged_users`
pub async fn daily_engaged_users(conn: &mut PgConnection, r: &mut Report) -> Result<(), AppError> {
    r.average = true;
    let data_start = if r.has_facet("prev_period") {
        r.prev_start_date()
    } else {
        r.start_date
    };
    let mut data = engaged_per_day(conn, data_start, r.end_date).await?;
    if r.has_facet("prev30Days") {
        let prev = engaged_per_day(conn, r.start_date - Duration::days(30), r.start_date).await?;
        r.prev30_days = Some(json!(prev.iter().map(|(_, n)| n).sum::<i64>()));
    }
    if r.has_facet("total") {
        r.total = Some(json!(
            count(
                conn,
                &format!(
                    "SELECT COUNT(DISTINCT user_id) FROM user_actions \
                     WHERE action_type IN {USER_ACTED_TYPES} AND user_id IS NOT NULL"
                )
            )
            .await?
        ));
    }
    if r.has_facet("prev_period") {
        let (current, prev) = super::split_date_counts(data, r.start_date);
        let sum: i64 = prev.iter().map(|(_, n)| n).sum();
        r.prev_period = Some(if sum == 0 {
            json!(0)
        } else {
            float_json(round_to(sum as f64 / prev.len() as f64, 1))
        });
        data = current;
    }
    r.data = points(&data);
    Ok(())
}

/// `User.real.count_by_first_post(start_date, end_date)`
async fn first_posts_per_day(
    conn: &mut PgConnection,
    start: NaiveDateTime,
    end: NaiveDateTime,
) -> Result<Vec<(NaiveDate, i64)>, AppError> {
    per_day(
        conn,
        &format!(
            "SELECT date(us.first_post_created_at), count(*) FROM users \
             INNER JOIN user_stats AS us ON us.user_id = users.id \
             WHERE {} AND us.first_post_created_at > {} AND us.first_post_created_at < {} \
             GROUP BY date(us.first_post_created_at) ORDER BY date(us.first_post_created_at)",
            real_users("users"),
            lit(start),
            lit(end)
        ),
        None,
    )
    .await
}

/// `report_new_contributors`
pub async fn new_contributors(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    r: &mut Report,
) -> Result<(), AppError> {
    let data_start = if r.has_facet("prev_period") {
        r.prev_start_date()
    } else {
        r.start_date
    };
    let mut data = first_posts_per_day(conn, data_start, r.end_date).await?;
    if r.has_facet("prev30Days") {
        let prev =
            first_posts_per_day(conn, r.start_date - Duration::days(30), r.start_date).await?;
        r.prev30_days = Some(json!(prev.iter().map(|(_, n)| n).sum::<i64>()));
    }
    if r.has_facet("total") {
        r.total = Some(json!(
            count(
                conn,
                &format!(
                    "SELECT count(*) FROM users INNER JOIN user_stats AS us ON us.user_id = users.id WHERE {}",
                    real_users("users")
                )
            )
            .await?
        ));
    }
    if r.has_facet("prev_period") {
        let (current, prev) = super::split_date_counts(data, r.start_date);
        r.prev_period = Some(json!(prev.iter().map(|(_, n)| n).sum::<i64>()));
        data = current;
    }
    r.data = points(&data);
    if !r.include_related_items {
        return Ok(());
    }
    let base = format!(
        "FROM users INNER JOIN user_stats ON user_stats.user_id = users.id \
         WHERE {} AND user_stats.first_post_created_at > {} AND user_stats.first_post_created_at < {}",
        real_users("users"),
        lit(r.start_date),
        lit(r.end_date)
    );
    let total = count(conn, &format!("SELECT count(*) {base}")).await?;
    let limit = r.limit.unwrap_or(super::RELATED_ITEMS_LIMIT);
    let rows: Vec<(i32, String, Option<String>, Option<i32>, NaiveDateTime)> =
        sqlx::query_as(&format!(
            "SELECT users.id, users.username, users.name, users.uploaded_avatar_id, \
                user_stats.first_post_created_at {base} \
         ORDER BY user_stats.first_post_created_at DESC LIMIT {limit}"
        ))
        .fetch_all(&mut *conn)
        .await?;
    let mut users = Vec::new();
    for (id, username, name, avatar, at) in rows {
        users.push(json!({
            "user": basic_user(cx, id, &username, name, avatar)?,
            "timestamp": super::iso8601(at),
        }));
    }
    r.related_items_totals = Some(json!({"users": total}));
    r.related_items = Some(json!({"users": users}));
    Ok(())
}

/// Topic.time_to_first_response's conditions.
fn first_response_where(
    start: Option<NaiveDateTime>,
    end: Option<NaiveDateTime>,
    category: Option<&[i64]>,
) -> String {
    let mut w: Vec<String> = Vec::new();
    if let Some(s) = start {
        w.push(format!("t.created_at >= {}", lit(s)));
    }
    if let Some(e) = end {
        w.push(format!("t.created_at < {}", lit(e)));
    }
    if let Some(ids) = category {
        w.push(category_where("t.category_id", ids));
    }
    w.push("t.archetype <> 'private_message'".into());
    w.push("t.deleted_at IS NULL".into());
    w.push("p.deleted_at IS NULL".into());
    w.push("p.post_number > 1".into());
    w.push("p.user_id != t.user_id".into());
    w.push("p.post_type = 1".into());
    w.push("EXTRACT(EPOCH FROM p.created_at - t.created_at) > 0".into());
    w.iter()
        .map(|c| format!("({c})"))
        .collect::<Vec<_>>()
        .join(" AND ")
}

/// `report_time_to_first_response`
pub async fn time_to_first_response(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    r: &mut Report,
) -> Result<(), AppError> {
    let filter = r.add_category_filter();
    r.icon = Some("reply".into());
    r.higher_is_better = false;
    r.y_axis_title = Some(cx.t("reports.time_to_first_response.yaxis")?);
    r.average = true;
    let ids = super::category_ids(conn, cx, filter).await?;
    let filter = ids.as_deref();
    let sql = format!(
        "SELECT AVG(t.hours)::float AS hours, t.created_at AS date FROM ( \
           SELECT t.id, t.created_at::date AS created_at, \
                  EXTRACT(EPOCH FROM MIN(p.created_at) - t.created_at)::float / 3600.0 AS hours \
           FROM topics t LEFT JOIN posts p ON p.topic_id = t.id \
           WHERE {} GROUP BY t.id \
         ) t GROUP BY t.created_at ORDER BY t.created_at",
        first_response_where(Some(r.start_date), Some(r.end_date), filter)
    );
    let rows: Vec<(Option<f64>, NaiveDate)> = sqlx::query_as(&sql).fetch_all(&mut *conn).await?;
    r.data = Value::Array(
        rows.iter()
            .map(|(h, d)| json!({"x": date_json(*d), "y": float_json(round_to(h.unwrap_or(0.0), 2))}))
            .collect(),
    );
    let total: Option<f64> = sqlx::query_scalar(&format!(
        "SELECT AVG(t.hours)::float AS hours FROM ( \
           SELECT t.id, EXTRACT(EPOCH FROM MIN(p.created_at) - t.created_at)::float / 3600.0 AS hours \
           FROM topics t LEFT JOIN posts p ON p.topic_id = t.id \
           WHERE {} GROUP BY t.id \
         ) t",
        first_response_where(
            Some(r.start_date - Duration::days(30)),
            Some(r.start_date),
            filter
        )
    ))
    .fetch_one(&mut *conn)
    .await?;
    r.prev30_days = Some(float_json(round_to(total.unwrap_or(0.0), 2)));
    Ok(())
}

/// Topic.with_no_response's conditions.
fn no_response_where(
    start: Option<NaiveDateTime>,
    end: Option<NaiveDateTime>,
    category: Option<&[i64]>,
) -> String {
    let mut w: Vec<String> = Vec::new();
    if let Some(s) = start {
        w.push(format!("t.created_at >= {}", lit(s)));
    }
    if let Some(e) = end {
        w.push(format!("t.created_at < {}", lit(e)));
    }
    if let Some(ids) = category {
        w.push(category_where("t.category_id", ids));
    }
    w.push("t.archetype <> 'private_message'".into());
    w.push("t.deleted_at IS NULL".into());
    w.iter()
        .map(|c| format!("({c})"))
        .collect::<Vec<_>>()
        .join(" AND ")
}

/// `report_topics_with_no_response`
pub async fn topics_with_no_response(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    r: &mut Report,
) -> Result<(), AppError> {
    let filter = r.add_category_filter();
    let ids = super::category_ids(conn, cx, filter).await?;
    let filter = ids.as_deref();
    let sql = format!(
        "SELECT COUNT(*) as count, tt.created_at AS date FROM ( \
           SELECT t.id, t.created_at::date AS created_at, MIN(p.post_number) first_reply \
           FROM topics t LEFT JOIN posts p ON p.topic_id = t.id AND p.user_id != t.user_id \
             AND p.deleted_at IS NULL AND p.post_type = 1 \
           WHERE {} GROUP BY t.id \
         ) tt WHERE tt.first_reply IS NULL OR tt.first_reply < 2 \
         GROUP BY tt.created_at ORDER BY tt.created_at",
        no_response_where(Some(r.start_date), Some(r.end_date), filter)
    );
    let rows: Vec<(i64, NaiveDate)> = sqlx::query_as(&sql).fetch_all(&mut *conn).await?;
    r.data = Value::Array(
        rows.iter()
            .map(|(n, d)| json!({"x": date_json(*d), "y": n}))
            .collect(),
    );
    // with_no_response_total reads only the category.
    let total = count(
        conn,
        &format!(
            "SELECT COUNT(*) as count FROM ( \
               SELECT t.id, MIN(p.post_number) first_reply \
               FROM topics t LEFT JOIN posts p ON p.topic_id = t.id AND p.user_id != t.user_id \
                 AND p.deleted_at IS NULL AND p.post_type = 1 \
               WHERE {} GROUP BY t.id \
             ) tt WHERE tt.first_reply IS NULL OR tt.first_reply < 2",
            no_response_where(None, None, filter)
        ),
    )
    .await?;
    r.total = Some(json!(total));
    r.prev30_days = Some(json!(total));
    Ok(())
}

/// `private_messages_report(report, topic_subtype)`
pub async fn private_messages(
    conn: &mut PgConnection,
    r: &mut Report,
    subtype: &str,
) -> Result<(), AppError> {
    r.icon = Some("envelope".into());
    let cond = format!(
        "topics.deleted_at IS NULL AND topics.user_id > 0 \
         AND topics.archetype = 'private_message' AND topics.subtype = '{subtype}'"
    );
    let sql = format!(
        "SELECT date(topics.created_at), count(*) FROM topics WHERE {cond} \
           AND topics.created_at >= {} AND topics.created_at <= {} \
         GROUP BY date(topics.created_at) ORDER BY date(topics.created_at)",
        lit(r.start_date),
        lit(r.end_date)
    );
    r.data = points(&per_day(conn, &sql, None).await?);
    let subject = Subject {
        from: "topics".into(),
        conditions: cond,
    };
    add_counts(conn, r, &subject, "topics.created_at", None).await
}

/// `report_user_to_user_private_messages_with_replies`
pub async fn private_messages_with_replies(
    conn: &mut PgConnection,
    r: &mut Report,
) -> Result<(), AppError> {
    r.icon = Some("envelope".into());
    let from =
        "posts INNER JOIN topics ON topics.id = posts.topic_id AND topics.deleted_at IS NULL";
    let cond = "posts.deleted_at IS NULL AND posts.user_id > 0 \
                AND topics.archetype = 'private_message' AND topics.subtype = 'user_to_user'";
    let sql = format!(
        "SELECT date(posts.created_at), count(*) FROM {from} WHERE {cond} \
           AND posts.created_at >= {} AND posts.created_at <= {} \
         GROUP BY date(posts.created_at) ORDER BY date(posts.created_at)",
        lit(r.start_date),
        lit(r.end_date)
    );
    r.data = points(&per_day(conn, &sql, None).await?);
    let subject = Subject {
        from: from.into(),
        conditions: cond.into(),
    };
    add_counts(conn, r, &subject, "posts.created_at", None).await
}

/// The `category_ids` filter as discourse-solved and Who's posting read
/// it: the ids that exist, in the asked order, at most `max`; None when
/// none were asked for.
pub async fn requested_category_ids(
    conn: &mut PgConnection,
    raw: Option<&Value>,
    max: i64,
) -> Result<Option<Vec<i64>>, AppError> {
    let Some(raw) = raw.filter(|v| !super::is_blank(v)) else {
        return Ok(None);
    };
    let parsed = parse_id_list(raw);
    if parsed.is_empty() {
        return Ok(Some(Vec::new()));
    }
    // Category.in_order_of(:id, ids).limit(max).pluck(:id)
    let found: Vec<i64> = sqlx::query_scalar(&format!(
        "SELECT id::bigint FROM categories WHERE id IN {} \
         ORDER BY array_position(ARRAY[{}]::bigint[], id::bigint) LIMIT {max}",
        in_list(&parsed),
        parsed
            .iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(",")
    ))
    .fetch_all(conn)
    .await?;
    Ok(Some(found))
}

/// `Array(raw.is_a?(String) ? raw.split(",") : raw).map(&:to_i)`
pub fn parse_id_list(raw: &Value) -> Vec<i64> {
    match raw {
        Value::String(s) => s.split(',').map(crate::ruby::to_i).collect(),
        Value::Array(items) => items
            .iter()
            .map(|v| crate::ruby::to_i(&crate::params::scalar(v).unwrap_or_default()))
            .collect(),
        Value::Number(n) => vec![n.as_i64().unwrap_or(0)],
        _ => Vec::new(),
    }
}

/// discourse-solved's `accepted_solutions` report.
pub async fn accepted_solutions(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    viewer: &Viewer<'_>,
    r: &mut Report,
) -> Result<(), AppError> {
    r.data = json!([]);
    let filter_requested = r
        .filter("category_ids")
        .is_some_and(|v| !super::is_blank(v));
    let raw = r.filter("category_ids").cloned();
    let requested = requested_category_ids(conn, raw.as_ref(), 50).await?;
    r.add_filter(
        "category_ids",
        json!({"type": "category_list", "default": requested}),
    );
    let mut base = "discourse_solved_solved_topics \
                    INNER JOIN topics ON topics.id = discourse_solved_solved_topics.topic_id \
                      AND topics.deleted_at IS NULL"
        .to_string();
    let mut cond = "topics.archetype != 'private_message'".to_string();
    if filter_requested {
        let ids = requested.clone().unwrap_or_default();
        cond.push_str(&format!(" AND topics.category_id IN {}", in_list(&ids)));
    }
    let current = format!(
        "{cond} AND discourse_solved_solved_topics.created_at >= {} \
         AND discourse_solved_solved_topics.created_at <= {}",
        lit(r.start_date),
        lit(r.end_date)
    );
    let sql = format!(
        "SELECT DATE(discourse_solved_solved_topics.created_at), count(*) FROM {base} WHERE {current} \
         GROUP BY DATE(discourse_solved_solved_topics.created_at) \
         ORDER BY DATE(discourse_solved_solved_topics.created_at)"
    );
    r.data = points(&per_day(conn, &sql, None).await?);
    if r.has_facet("total") {
        r.total = Some(json!(
            count(conn, &format!("SELECT count(*) FROM {base} WHERE {cond}")).await?
        ));
    }
    if r.has_facet("prev30Days") {
        r.prev30_days = Some(json!(
            count(
                conn,
                &format!(
                    "SELECT count(*) FROM {base} WHERE {cond} \
                     AND discourse_solved_solved_topics.created_at >= {} \
                     AND discourse_solved_solved_topics.created_at <= {}",
                    lit(r.start_date - Duration::days(30)),
                    lit(r.start_date)
                )
            )
            .await?
        ));
    }
    if r.has_facet("prev_period") {
        r.prev_period = Some(json!(
            count(
                conn,
                &format!(
                    "SELECT count(*) FROM {base} WHERE {cond} \
                     AND discourse_solved_solved_topics.created_at >= {} \
                     AND discourse_solved_solved_topics.created_at < {}",
                    lit(r.prev_start_date()),
                    lit(r.prev_end_date())
                )
            )
            .await?
        ));
    }
    if !r.include_related_items {
        return Ok(());
    }
    let Some(guardian) = viewer.guardian else {
        return Err(
            crate::Unsupported("accepted solutions' related items without a viewer").into(),
        );
    };
    // Topic.listable_topics.secured(guardian), shared drafts unless seen.
    let secure = guardian
        .secure_category_ids(&mut *conn, cx.settings)
        .await?;
    let secure_cond = if secure.is_empty() {
        "NOT read_restricted".to_string()
    } else {
        format!(
            "NOT read_restricted OR id IN {}",
            in_list(&secure.iter().map(|i| *i as i64).collect::<Vec<_>>())
        )
    };
    let mut related = format!(
        "{current} AND topics.archetype <> 'private_message' \
         AND (topics.category_id IS NULL OR topics.category_id IN (SELECT id FROM categories WHERE {secure_cond}))"
    );
    if !guardian.can_see_shared_draft(cx.settings)? {
        related.push_str(" AND discourse_solved_solved_topics.topic_id NOT IN (SELECT topic_id FROM shared_drafts)");
    }
    base.push_str("");
    let total = count(
        conn,
        &format!("SELECT count(*) FROM {base} WHERE {related}"),
    )
    .await?;
    let limit = r.limit.unwrap_or(super::RELATED_ITEMS_LIMIT);
    let rows: Vec<(i64, i32, String, Option<String>, Option<i32>)> = sqlx::query_as(&format!(
        "SELECT discourse_solved_solved_topics.id, topics.id, topics.title, topics.slug, topics.category_id \
         FROM {base} WHERE {related} \
         ORDER BY discourse_solved_solved_topics.created_at DESC LIMIT {limit}"
    ))
    .fetch_all(&mut *conn)
    .await?;
    let mut items = Vec::new();
    for (solved_id, topic_id, title, slug, category_id) in rows {
        // answer_posts by their creation and id, each user once.
        let answerers: Vec<(i32, String, Option<String>, Option<i32>)> = sqlx::query_as(
            "SELECT u.id, u.username, u.name, u.uploaded_avatar_id FROM discourse_solved_topic_answers a \
             INNER JOIN posts p ON p.id = a.answer_post_id AND p.deleted_at IS NULL \
             INNER JOIN users u ON u.id = p.user_id \
             WHERE a.solved_topic_id = $1 ORDER BY p.created_at, p.id",
        )
        .bind(solved_id)
        .fetch_all(&mut *conn)
        .await?;
        let mut seen = Vec::new();
        let mut users = Vec::new();
        for (id, username, name, avatar) in answerers {
            if seen.contains(&id) {
                continue;
            }
            seen.push(id);
            users.push(basic_user(cx, id, &username, name, avatar)?);
        }
        let slug = slug
            .filter(|s| !s.is_empty())
            .ok_or(crate::Unsupported("a topic without a slug"))?;
        let category = match category_id {
            Some(cid) => {
                let c: Option<(i32, String, String, String, String)> = sqlx::query_as(
                    "SELECT id, name, slug, color, text_color FROM categories WHERE id = $1",
                )
                .bind(cid)
                .fetch_optional(&mut *conn)
                .await?;
                match c {
                    Some((id, name, slug, color, text)) => json!({
                        "id": id, "name": name, "slug": slug, "color": color, "textColor": text,
                    }),
                    None => Value::Null,
                }
            }
            None => Value::Null,
        };
        items.push(json!({
            "topic": {
                "title": title,
                "url": format!("{}/t/{slug}/{topic_id}", cx.base_path()),
            },
            "solved_by_users": users,
            "category": category,
        }));
    }
    r.related_items_totals = Some(json!({"solved_topics": total}));
    r.related_items = Some(json!({"solved_topics": items}));
    Ok(())
}
