//! discourse-solved's contribution to the admin dashboard: its "support"
//! section (`register_admin_dashboard_section`, built by
//! DiscourseSolved::AdminDashboardSupport) with the section's categories
//! setting (AdminDashboardSupportCategoriesSetting), and the section's
//! refetch endpoint (AdminDashboardSupportController#index). Its
//! accepted_solutions report and highlight KPI are in reports/counts.rs
//! and admin_dashboard.rs.
//!
//! Rails keeps the built data in Discourse.cache for 30 minutes and the
//! availability for 5; the port computes both on every request.

use chrono::{NaiveDate, NaiveDateTime};
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::guardian::Guardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, Unsupported};

/// The section's id.
pub const SECTION_ID: &str = "support";

/// `AdminDashboardSupportCategoriesSetting::MAX_CATEGORY_IDS`
const MAX_CATEGORY_IDS: usize = 10;

/// `RESPONSE_TIME_BUCKETS`: each bucket's key and upper bound in seconds.
const RESPONSE_TIME_BUCKETS: [(&str, Option<i64>); 4] = [
    ("lt_1h", Some(3600)),
    ("1_4h", Some(4 * 3600)),
    ("4_24h", Some(24 * 3600)),
    ("gt_24h", None),
];

/// `Categories::Types::Support.find_matches`: categories joined to their
/// enable_accepted_answers custom field.
const FIND_MATCHES: &str = "categories INNER JOIN category_custom_fields \
     ON category_custom_fields.category_id = categories.id \
     AND category_custom_fields.name = 'enable_accepted_answers' \
     AND category_custom_fields.value = 'true'";

/// `AdminDashboardSupport.available?`
pub async fn available(conn: &mut PgConnection, settings: &SiteSettings) -> Result<bool, AppError> {
    if !super::enabled(settings)? {
        return Ok(false);
    }
    if settings.get("allow_solved_on_all_topics")?.truthy() {
        return Ok(true);
    }
    Ok(
        sqlx::query_scalar(&format!("SELECT EXISTS (SELECT 1 FROM {FIND_MATCHES})"))
            .fetch_one(conn)
            .await?,
    )
}

/// `AdminDashboardSupportCategoriesSetting.validate` over the permitted
/// params (`[{category_ids: []}]`): the ids, or the InvalidParameters key.
pub async fn validate_categories(
    conn: &mut PgConnection,
    params: &Map<String, Value>,
) -> Result<Result<Value, &'static str>, AppError> {
    // permit drops a category_ids that isn't a list of scalars.
    let Some(Value::Array(items)) = params.get("category_ids") else {
        return Ok(Err("category_ids"));
    };
    let mut parsed = Vec::new();
    for item in items {
        parsed.push(match item {
            // Integer(id, exception: false)
            Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
            Value::String(s) => crate::ruby::integer(s),
            Value::Null | Value::Bool(_) => None,
            // permit drops what isn't a scalar from the list.
            _ => return Err(Unsupported("support categories nested in the list").into()),
        });
    }
    if parsed.len() > MAX_CATEGORY_IDS || parsed.iter().any(Option::is_none) {
        return Ok(Err("category_ids"));
    }
    let ids: Vec<i64> = parsed.into_iter().flatten().collect();
    let mut unique = ids.clone();
    unique.sort_unstable();
    unique.dedup();
    if unique.len() != ids.len() {
        return Ok(Err("category_ids"));
    }
    if !ids.is_empty() {
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM categories WHERE id = ANY($1)")
            .bind(&ids)
            .fetch_one(&mut *conn)
            .await?;
        if n != ids.len() as i64 {
            return Ok(Err("category_ids"));
        }
    }
    Ok(Ok(json!({ "category_ids": ids })))
}

/// The section's stored categories: `AdminDashboardSectionConfiguration
/// .settings_for("support").dig("categories", "category_ids")`.
pub async fn stored_category_ids(conn: &mut PgConnection) -> Result<Value, AppError> {
    let settings: Option<Value> = sqlx::query_scalar(
        "SELECT settings FROM admin_dashboard_sections WHERE section_id = $1 LIMIT 1",
    )
    .bind(SECTION_ID)
    .fetch_optional(conn)
    .await?;
    Ok(settings
        .map(|s| s["categories"]["category_ids"].clone())
        .unwrap_or(Value::Null))
}

/// `Array(category_ids.is_a?(String) ? category_ids.split(",") :
/// category_ids).map(&:to_i)`
fn requested_ids(value: &Value) -> Result<Vec<i64>, AppError> {
    let to_i = |v: &Value| -> Result<i64, AppError> {
        match v {
            Value::Number(n) => Ok(n
                .as_i64()
                .unwrap_or_else(|| n.as_f64().unwrap_or(0.0) as i64)),
            Value::String(s) => Ok(crate::ruby::to_i(s)),
            Value::Null => Ok(0),
            _ => Err(Unsupported("support categories that aren't numbers or strings").into()),
        }
    };
    match value {
        Value::Null => Ok(Vec::new()),
        Value::String(s) => Ok(crate::ruby::split(s, ",")
            .into_iter()
            .map(crate::ruby::to_i)
            .collect()),
        Value::Array(items) => items.iter().map(to_i).collect(),
        Value::Object(_) => Err(Unsupported("support categories as a hash").into()),
        other => Ok(vec![to_i(other)?]),
    }
}

/// The window's bounds and the previous, length-matched one.
struct Window {
    start: NaiveDateTime,
    end: NaiveDateTime,
}

impl Window {
    /// `prev_start_date`: `start_date - (end_date - start_date)`.
    fn previous(&self) -> Window {
        Window {
            start: self.start - (self.end - self.start),
            end: self.start,
        }
    }
}

struct Support {
    window: Window,
    /// `selected_category_ids`
    selected: Vec<i64>,
    /// `effective_category_ids`
    effective: Vec<i64>,
    /// `category_options`
    options: Vec<(i64, String)>,
}

/// `AdminDashboardSupport.build(start_date:, end_date:, current_user:,
/// category_ids:)`: the section's data.
pub async fn build(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
    start: Option<&str>,
    end: Option<&str>,
    category_ids: &Value,
) -> Result<Value, AppError> {
    // parse_date: Time.zone.parse(value)&.beginning_of_day, nil when it
    // raises; the defaults are 30 days ago and the end of today.
    let now = crate::clock::now_naive();
    let start = crate::admin_dashboard::zone_parse(start)?
        .map(crate::reports::beginning_of_day)
        .unwrap_or_else(|| {
            crate::reports::beginning_of_day((now - chrono::Duration::days(30)).date())
        });
    let end = crate::admin_dashboard::zone_parse(end)?
        .map(crate::reports::end_of_day)
        .unwrap_or_else(|| crate::reports::end_of_day(now.date()));
    let requested = requested_ids(category_ids)?;

    // support_categories: find_matches (or every category with
    // allow_solved_on_all_topics), secured for the viewer.
    let secure = guardian.secure_category_ids(&mut *conn, settings).await?;
    let secured = if secure.is_empty() {
        "NOT categories.read_restricted".to_string()
    } else {
        format!(
            "(NOT categories.read_restricted OR categories.id IN ({}))",
            secure
                .iter()
                .map(i32::to_string)
                .collect::<Vec<_>>()
                .join(",")
        )
    };
    let from = if settings.get("allow_solved_on_all_topics")?.truthy() {
        "categories"
    } else {
        FIND_MATCHES
    };
    let all: Vec<i64> = sqlx::query_scalar(&format!(
        "SELECT categories.id::bigint FROM {from} WHERE {secured}"
    ))
    .fetch_all(&mut *conn)
    .await?;
    let options: Vec<(i64, String)> = sqlx::query_as(&format!(
        "SELECT categories.id::bigint, categories.name FROM {from} WHERE {secured} ORDER BY categories.name"
    ))
    .fetch_all(&mut *conn)
    .await?;
    // requested & all: in the requested order, each once.
    let mut selected: Vec<i64> = Vec::new();
    for id in requested {
        if all.contains(&id) && !selected.contains(&id) {
            selected.push(id);
        }
    }
    let effective = if selected.is_empty() {
        all.clone()
    } else {
        selected.clone()
    };
    let s = Support {
        window: Window { start, end },
        selected,
        effective,
        options,
    };
    s.build(conn).await
}

/// `resolution_rate`: the resolved share, a tenth of a percent.
fn resolution_rate(outcomes: &[i64; 3]) -> Value {
    let total: i64 = outcomes.iter().sum();
    if total == 0 {
        return Value::Null;
    }
    crate::reports::float_json(crate::reports::round_to(
        outcomes[0] as f64 / total as f64 * 100.0,
        1,
    ))
}

/// `share`: a whole percent, 0.0 of nothing.
fn share(count: i64, total: i64) -> Value {
    if total == 0 {
        return crate::reports::float_json(0.0);
    }
    json!((count as f64 / total as f64 * 100.0).round() as i64)
}

impl Support {
    async fn build(&self, conn: &mut PgConnection) -> Result<Value, AppError> {
        let current = &self.window;
        let previous = current.previous();
        let outcomes = self.outcomes(conn, current).await?;
        let prev_outcomes = self.outcomes(conn, &previous).await?;
        let replies = self.first_reply_seconds(conn, current).await?;
        let prev_replies = self.first_reply_seconds(conn, &previous).await?;
        let avg_now = average(&replies);
        let avg_prev = average(&prev_replies);

        let mut query = Map::new();
        query.insert("start_date".into(), json!(day(current.start).to_string()));
        query.insert("end_date".into(), json!(day(current.end).to_string()));
        if !self.selected.is_empty() {
            let joined: Vec<String> = self.selected.iter().map(i64::to_string).collect();
            query.insert(
                "filters".into(),
                json!({ "category_ids": joined.join(",") }),
            );
        }
        let kpis = json!({
            "resolution_rate": {
                "value": resolution_rate(&outcomes),
                "previous_value": resolution_rate(&prev_outcomes),
                "report_type": "accepted_solutions",
                "report_query": query,
            },
            "staff_involvement": {
                "value": self.staff_involvement(conn, current, &outcomes).await?,
                "previous_value": self.staff_involvement(conn, &previous, &prev_outcomes).await?,
            },
            "avg_first_reply": {
                "value": avg_now,
                "previous_value": avg_prev,
            },
        });

        let total = replies.len() as i64;
        let mut buckets = Vec::new();
        let mut min = 0;
        for (key, max) in RESPONSE_TIME_BUCKETS {
            let count = replies
                .iter()
                .filter(|s| **s >= min && max.is_none_or(|m| **s < m))
                .count() as i64;
            buckets.push(json!({"key": key, "count": count, "share": share(count, total)}));
            min = max.unwrap_or(min);
        }
        let trend = match (avg_now, avg_prev) {
            (Some(now), Some(prev)) => json!({
                "direction": if now == prev { "flat" } else if now < prev { "faster" } else { "slower" },
                "seconds": (now - prev).abs(),
            }),
            _ => json!({"direction": "flat", "seconds": 0}),
        };

        Ok(json!({
            "category_options": self
                .options
                .iter()
                .map(|(id, name)| json!({"id": id, "name": name}))
                .collect::<Vec<_>>(),
            "category_ids": self.selected,
            "kpis": kpis,
            "topic_outcomes": {
                "resolved": outcomes[0],
                "in_progress": outcomes[1],
                "unanswered": outcomes[2],
            },
            "whos_answering": self.whos_answering(conn).await?,
            "response_time_distribution": {"buckets": buckets, "trend": trend},
        }))
    }

    fn ids(&self) -> String {
        self.effective
            .iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// `compute_outcomes`: resolved, in progress and unanswered topics
    /// created in the window.
    async fn outcomes(&self, conn: &mut PgConnection, w: &Window) -> Result<[i64; 3], AppError> {
        if self.effective.is_empty() {
            return Ok([0, 0, 0]);
        }
        let (resolved, in_progress, unanswered): (i64, i64, i64) = sqlx::query_as(&format!(
            "SELECT \
               COUNT(*) FILTER (WHERE solved) AS resolved, \
               COUNT(*) FILTER (WHERE NOT solved AND has_reply) AS in_progress, \
               COUNT(*) FILTER (WHERE NOT solved AND NOT has_reply) AS unanswered \
             FROM ( \
               SELECT \
                 (st.topic_id IS NOT NULL) AS solved, \
                 EXISTS ( \
                   SELECT 1 FROM posts p \
                   WHERE p.topic_id = t.id AND p.post_number > 1 AND p.post_type = 1 \
                     AND p.deleted_at IS NULL \
                 ) AS has_reply \
               FROM topics t \
               LEFT JOIN discourse_solved_solved_topics st ON st.topic_id = t.id \
               WHERE t.category_id IN ({}) AND t.archetype = 'regular' AND t.deleted_at IS NULL \
                 AND t.created_at >= $1 AND t.created_at <= $2 \
             ) topics_with_state",
            self.ids()
        ))
        .bind(w.start)
        .bind(w.end)
        .fetch_one(conn)
        .await?;
        Ok([resolved, in_progress, unanswered])
    }

    /// `staff_involvement`: the share of the window's topics whose first
    /// reply (by someone other than the author) came from staff.
    async fn staff_involvement(
        &self,
        conn: &mut PgConnection,
        w: &Window,
        outcomes: &[i64; 3],
    ) -> Result<Value, AppError> {
        if self.effective.is_empty() {
            return Ok(Value::Null);
        }
        let total: i64 = outcomes.iter().sum();
        if total == 0 {
            return Ok(Value::Null);
        }
        let staff_first: i64 = sqlx::query_scalar(&format!(
            "SELECT COUNT(*) FROM ( \
               SELECT DISTINCT ON (p.topic_id) p.user_id \
               FROM posts p JOIN topics t ON t.id = p.topic_id \
               WHERE t.category_id IN ({}) AND t.archetype = 'regular' AND t.deleted_at IS NULL \
                 AND t.created_at >= $1 AND t.created_at <= $2 \
                 AND p.post_number > 1 AND p.post_type = 1 AND p.deleted_at IS NULL \
                 AND p.user_id <> t.user_id \
               ORDER BY p.topic_id, p.created_at ASC \
             ) first_replies \
             JOIN users u ON u.id = first_replies.user_id \
             WHERE u.admin OR u.moderator",
            self.ids()
        ))
        .bind(w.start)
        .bind(w.end)
        .fetch_one(conn)
        .await?;
        Ok(crate::reports::float_json(crate::reports::round_to(
            staff_first as f64 / total as f64 * 100.0,
            1,
        )))
    }

    /// `compute_first_reply_seconds`: each topic's wait for its first
    /// reply by someone else, when there was one.
    async fn first_reply_seconds(
        &self,
        conn: &mut PgConnection,
        w: &Window,
    ) -> Result<Vec<i64>, AppError> {
        if self.effective.is_empty() {
            return Ok(Vec::new());
        }
        let seconds: Vec<Option<i32>> = sqlx::query_scalar(&format!(
            "SELECT EXTRACT(EPOCH FROM MIN(p.created_at) - t.created_at)::int AS seconds \
             FROM topics t JOIN posts p ON p.topic_id = t.id \
             WHERE t.category_id IN ({}) AND t.archetype = 'regular' AND t.deleted_at IS NULL \
               AND t.created_at >= $1 AND t.created_at <= $2 \
               AND p.post_number > 1 AND p.post_type = 1 AND p.deleted_at IS NULL \
               AND p.user_id <> t.user_id \
             GROUP BY t.id \
             HAVING EXTRACT(EPOCH FROM MIN(p.created_at) - t.created_at) > 0",
            self.ids()
        ))
        .bind(w.start)
        .bind(w.end)
        .fetch_all(conn)
        .await?;
        Ok(seconds.into_iter().map(|s| s.unwrap_or(0) as i64).collect())
    }

    /// `compute_whos_answering`: the window's replies by their author's
    /// group, staff first, then by trust level.
    async fn whos_answering(&self, conn: &mut PgConnection) -> Result<Value, AppError> {
        if self.effective.is_empty() {
            return Ok(json!({"rows": [], "total": 0}));
        }
        let rows: Vec<(String, i64)> = sqlx::query_as(&format!(
            "SELECT \
               CASE \
                 WHEN u.admin OR u.moderator THEN 'staff' \
                 WHEN u.trust_level >= 4 THEN 'leader' \
                 WHEN u.trust_level = 3 THEN 'regular' \
                 WHEN u.trust_level = 2 THEN 'member' \
                 ELSE 'basic' \
               END AS bucket, \
               COUNT(*) AS count \
             FROM posts p \
             JOIN topics t ON t.id = p.topic_id \
             JOIN users u ON u.id = p.user_id \
             WHERE t.category_id IN ({}) AND t.archetype = 'regular' AND t.deleted_at IS NULL \
               AND p.created_at >= $1 AND p.created_at <= $2 \
               AND p.post_number > 1 AND p.post_type = 1 AND p.deleted_at IS NULL \
               AND p.user_id <> t.user_id AND u.id > 0 \
             GROUP BY bucket",
            self.ids()
        ))
        .bind(self.window.start)
        .bind(self.window.end)
        .fetch_all(conn)
        .await?;
        let total: i64 = rows.iter().map(|(_, c)| c).sum();
        Ok(json!({
            "rows": rows
                .iter()
                .map(|(kind, count)| json!({"type": kind, "count": count, "share": share(*count, total)}))
                .collect::<Vec<_>>(),
            "total": total,
        }))
    }
}

/// `avg_first_reply_seconds`: the mean, rounded; none without replies.
fn average(seconds: &[i64]) -> Option<i64> {
    if seconds.is_empty() {
        return None;
    }
    let sum: i64 = seconds.iter().sum();
    Some((sum as f64 / seconds.len() as f64).round() as i64)
}

fn day(t: NaiveDateTime) -> NaiveDate {
    t.date()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_previous_window_is_as_long_and_ends_at_the_start() {
        let d = |y, m, dd| NaiveDate::from_ymd_opt(y, m, dd).unwrap();
        let w = Window {
            start: crate::reports::beginning_of_day(d(2026, 9, 21)),
            end: crate::reports::end_of_day(d(2026, 10, 20)),
        };
        let p = w.previous();
        // A nanosecond past the day before, which binds as that midnight.
        assert_eq!(p.start.date(), d(2026, 8, 22));
        assert_eq!(p.start.and_utc().timestamp_subsec_micros(), 0);
        assert_eq!(p.end, w.start);
    }

    #[test]
    fn shares_and_rates_round_as_ruby() {
        assert_eq!(share(1, 3), json!(33));
        assert_eq!(share(2, 3), json!(67));
        assert_eq!(share(0, 0), json!(0.0));
        assert_eq!(resolution_rate(&[1, 1, 1]), json!(33.3));
        assert_eq!(resolution_rate(&[0, 0, 0]), Value::Null);
        assert_eq!(average(&[0, 1800]), Some(900));
        assert_eq!(average(&[]), None);
    }

    #[test]
    fn requested_categories_read_as_rails_does() {
        assert_eq!(requested_ids(&json!("4,2")).unwrap(), vec![4, 2]);
        assert_eq!(requested_ids(&json!(["4", "x"])).unwrap(), vec![4, 0]);
        assert_eq!(requested_ids(&json!([4])).unwrap(), vec![4]);
        assert_eq!(requested_ids(&Value::Null).unwrap(), Vec::<i64>::new());
        assert!(requested_ids(&json!({"a": 1})).is_err());
    }
}
