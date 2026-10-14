//! The admin dashboard (the dashboard_improvements upcoming change): its
//! section configuration (AdminDashboardSectionConfiguration), the
//! sections' builders (AdminDashboardHighlights, AdminDashboardEngagement,
//! AdminDashboardSiteTraffic, AdminDashboardSearch, AdminDashboardSystem,
//! AdminDashboard::Reports::Section), the mountable reports
//! (AdminDashboard::Reports' core provider, listing, layout and bulk
//! fetch), the dashboard data caches (StatsCacheable) and the version
//! check (DiscourseUpdates.check_version) as the port knows it: the
//! vendored Discourse's, never checked against meta.

use chrono::{Duration, NaiveDate, NaiveDateTime};
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::reports::{self, Cx, Opts, Viewer};
use crate::{AppError, Unsupported};

/// `AdminDashboardSectionConfiguration::KNOWN_SECTIONS`
pub const KNOWN_SECTIONS: [&str; 6] = [
    "highlights",
    "reports",
    "traffic",
    "engagement",
    "search",
    "system",
];

/// `Report::SCHEMA_VERSION`, in the stats cache keys.
const SCHEMA_VERSION: i32 = 5;

/// `builtin_section_ids`: system only while version checks are on.
fn builtin_section_ids(cx: &Cx<'_>) -> Result<Vec<String>, AppError> {
    let checks = cx.settings.get("version_checks")?.truthy();
    Ok(KNOWN_SECTIONS
        .iter()
        .filter(|s| checks || **s != "system")
        .map(|s| s.to_string())
        .collect())
}

/// discourse-solved's support section is there when solved support
/// categories exist (DiscourseSolved::AdminDashboardSupport.available?).
async fn solved_support_available(conn: &mut PgConnection, cx: &Cx<'_>) -> Result<bool, AppError> {
    if !cx.settings.get("solved_enabled")?.truthy() {
        return Ok(false);
    }
    if cx.settings.get("allow_solved_on_all_topics")?.truthy() {
        return Ok(true);
    }
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM category_custom_fields \
         WHERE name = 'enable_accepted_answers' AND value = 'true')",
    )
    .fetch_one(conn)
    .await?)
}

/// `all_known_section_ids`: the built-in sections and the plugins'.
async fn all_known_section_ids(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
) -> Result<Vec<String>, AppError> {
    let mut ids = builtin_section_ids(cx)?;
    if solved_support_available(conn, cx).await? {
        ids.push("support".into());
    }
    Ok(ids)
}

/// `AdminDashboardSectionConfiguration.sections`: the stored ones by
/// position, then the known ones not stored, visible.
pub async fn sections(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
) -> Result<Vec<(String, bool)>, AppError> {
    let known = all_known_section_ids(conn, cx).await?;
    let stored: Vec<(String, bool)> = sqlx::query_as(
        "SELECT section_id, visible FROM admin_dashboard_sections \
         WHERE section_id = ANY($1) ORDER BY position",
    )
    .bind(&known)
    .fetch_all(&mut *conn)
    .await?;
    let mut out = stored.clone();
    for id in known {
        if !stored.iter().any(|(s, _)| *s == id) {
            out.push((id, true));
        }
    }
    Ok(out)
}

pub fn sections_json(sections: &[(String, bool)]) -> Value {
    Value::Array(
        sections
            .iter()
            .map(|(id, visible)| json!({"id": id, "visible": visible}))
            .collect(),
    )
}

/// `AdminDashboardSectionConfiguration.update(sections, actor:)`
pub async fn update_configuration(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    input: &[Value],
    actor_id: i32,
) -> Result<(), AppError> {
    let known = all_known_section_ids(conn, cx).await?;
    let mut sanitized: Vec<(String, bool)> = Vec::new();
    for s in input {
        let Some(map) = s.as_object() else {
            return Err(Unsupported("dashboard sections that aren't hashes").into());
        };
        let id = map
            .get("id")
            .and_then(crate::params::scalar)
            .unwrap_or_default();
        if !known.contains(&id) || sanitized.iter().any(|(s, _)| *s == id) {
            continue;
        }
        // ActiveModel::Type::Boolean.cast, nil counting as hidden.
        let visible = crate::ruby::cast_boolean(
            map.get("visible")
                .and_then(crate::params::scalar)
                .as_deref(),
        )
        .unwrap_or(false);
        sanitized.push((id, visible));
    }
    let current = sections(conn, cx).await?;
    let mut ordered = sanitized.clone();
    for (id, visible) in current {
        if !sanitized.iter().any(|(s, _)| *s == id) {
            ordered.push((id, visible));
        }
    }
    for (position, (id, visible)) in ordered.iter().enumerate() {
        sqlx::query(
            "INSERT INTO admin_dashboard_sections (section_id, position, visible, created_at, updated_at) \
             SELECT $1, $2, $3, at, at FROM clock_timestamp() AS at \
             ON CONFLICT (section_id) DO UPDATE SET position = EXCLUDED.position, \
               visible = EXCLUDED.visible, created_at = EXCLUDED.created_at, updated_at = EXCLUDED.updated_at",
        )
        .bind(id)
        .bind(position as i32)
        .bind(visible)
        .execute(&mut *conn)
        .await?;
    }
    let layout = ordered
        .iter()
        .map(|(id, v)| format!("{id}:{}", if *v { "visible" } else { "hidden" }))
        .collect::<Vec<_>>()
        .join(", ");
    // StaffActionLogger#log_custom (custom_staff, admin only).
    sqlx::query(
        "INSERT INTO user_histories (action, acting_user_id, custom_type, details, admin_only, \
                                     created_at, updated_at) \
         SELECT 23, $1, 'update_dashboard_sections', $2, TRUE, at, at FROM clock_timestamp() AS at",
    )
    .bind(actor_id)
    .bind(format!("layout: {layout}"))
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Why a section setting was refused: the InvalidParameters key.
pub struct InvalidSetting(pub &'static str);

/// `Integer(id, exception: false)` over a list param.
fn parse_ids(v: Option<&Value>, key: &'static str) -> Result<Vec<Option<i64>>, InvalidSetting> {
    match v {
        Some(Value::Array(items)) => Ok(items
            .iter()
            .map(|i| match i {
                Value::Number(n) => n.as_i64(),
                Value::String(s) => crate::ruby::integer(s),
                _ => None,
            })
            .collect()),
        _ => Err(InvalidSetting(key)),
    }
}

/// The category ids a setting names: at most `max`, unique, existing.
async fn valid_category_ids(
    conn: &mut PgConnection,
    v: Option<&Value>,
    max: usize,
) -> Result<Result<Vec<i64>, InvalidSetting>, AppError> {
    let parsed = match parse_ids(v, "category_ids") {
        Ok(p) => p,
        Err(e) => return Ok(Err(e)),
    };
    if parsed.len() > max || parsed.iter().any(Option::is_none) {
        return Ok(Err(InvalidSetting("category_ids")));
    }
    let ids: Vec<i64> = parsed.into_iter().flatten().collect();
    let mut unique = ids.clone();
    unique.sort_unstable();
    unique.dedup();
    if unique.len() != ids.len() {
        return Ok(Err(InvalidSetting("category_ids")));
    }
    if !ids.is_empty() {
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM categories WHERE id = ANY($1)")
            .bind(&ids)
            .fetch_one(&mut *conn)
            .await?;
        if n != ids.len() as i64 {
            return Ok(Err(InvalidSetting("category_ids")));
        }
    }
    Ok(Ok(ids))
}

/// `Report.valid_group_token?(token)`
async fn valid_group_token(conn: &mut PgConnection, token: &str) -> Result<bool, AppError> {
    if ["new_members", "returning", "staff"].contains(&token) {
        return Ok(true);
    }
    let Some(digits) = token.strip_prefix("group:") else {
        return Ok(false);
    };
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return Ok(false);
    }
    let Ok(id) = digits.parse::<i64>() else {
        return Ok(false);
    };
    if [0, 4, 5, 3].contains(&id) {
        return Ok(false);
    }
    Ok(
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM groups WHERE id = $1)")
            .bind(id as i32)
            .fetch_one(conn)
            .await?,
    )
}

/// `AdminDashboardSectionConfiguration.update_setting`: the setting
/// validated and deep-merged into the section's settings.
pub async fn update_setting(
    conn: &mut PgConnection,
    section_id: &str,
    key: &str,
    params: &Map<String, Value>,
) -> Result<Result<(), InvalidSetting>, AppError> {
    let value = match (section_id, key) {
        ("engagement", "activity_by_category") => {
            match valid_category_ids(conn, params.get("category_ids"), 10).await? {
                Ok(ids) => json!({"category_ids": ids}),
                Err(e) => return Ok(Err(e)),
            }
        }
        ("engagement", "whos_posting") => {
            let ids = match valid_category_ids(conn, params.get("category_ids"), 10).await? {
                Ok(ids) => ids,
                Err(e) => return Ok(Err(e)),
            };
            let mut out = Map::new();
            out.insert("category_ids".into(), json!(ids));
            if let Some(groups) = params.get("groups") {
                let Value::Array(items) = groups else {
                    return Ok(Err(InvalidSetting("groups")));
                };
                let groups: Vec<String> = items
                    .iter()
                    .map(|g| crate::params::scalar(g).unwrap_or_default())
                    .collect();
                let mut unique = groups.clone();
                unique.sort();
                unique.dedup();
                if groups.is_empty() || groups.len() > 10 || unique.len() != groups.len() {
                    return Ok(Err(InvalidSetting("groups")));
                }
                for g in &groups {
                    if !valid_group_token(conn, g).await? {
                        return Ok(Err(InvalidSetting("groups")));
                    }
                }
                out.insert("groups".into(), json!(groups));
            }
            Value::Object(out)
        }
        ("support", "categories") => {
            return Err(Unsupported("discourse-solved's support section settings").into());
        }
        _ => return Ok(Err(InvalidSetting("setting_key"))),
    };
    // find_or_create_by!(section_id:), positioned last and visible.
    let existing: Option<i64> =
        sqlx::query_scalar("SELECT id FROM admin_dashboard_sections WHERE section_id = $1")
            .bind(section_id)
            .fetch_optional(&mut *conn)
            .await?;
    let id = match existing {
        Some(id) => id,
        None => {
            sqlx::query_scalar(
                "INSERT INTO admin_dashboard_sections (section_id, position, visible, settings, created_at, updated_at) \
                 SELECT $1, COALESCE((SELECT max(position) FROM admin_dashboard_sections), -1) + 1, TRUE, '{}'::jsonb, at, at \
                 FROM clock_timestamp() AS at RETURNING id",
            )
            .bind(section_id)
            .fetch_one(&mut *conn)
            .await?
        }
    };
    let current: Value = sqlx::query_scalar(
        "SELECT settings FROM admin_dashboard_sections WHERE id = $1 FOR UPDATE",
    )
    .bind(id)
    .fetch_one(&mut *conn)
    .await?;
    let mut merged = current;
    deep_merge(&mut merged, &json!({ key: value }));
    sqlx::query(
        "UPDATE admin_dashboard_sections SET settings = $2, updated_at = clock_timestamp() WHERE id = $1",
    )
    .bind(id)
    .bind(&merged)
    .execute(&mut *conn)
    .await?;
    Ok(Ok(()))
}

/// `Hash#deep_merge`
fn deep_merge(into: &mut Value, other: &Value) {
    match (into, other) {
        (Value::Object(a), Value::Object(b)) => {
            for (k, v) in b {
                match a.get_mut(k) {
                    Some(existing) if existing.is_object() && v.is_object() => {
                        deep_merge(existing, v)
                    }
                    _ => {
                        a.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (slot, v) => *slot = v.clone(),
    }
}

/// `AdminDashboardSectionConfiguration.settings_for(section_id)`
async fn settings_for(conn: &mut PgConnection, section_id: &str) -> Result<Value, AppError> {
    let s: Option<Value> =
        sqlx::query_scalar("SELECT settings FROM admin_dashboard_sections WHERE section_id = $1")
            .bind(section_id)
            .fetch_optional(conn)
            .await?;
    Ok(s.unwrap_or_else(|| json!({})))
}

/// `Time.zone.parse(value)&.beginning_of_day` for the dashboard's dates:
/// None when blank or not a date.
pub fn zone_parse(value: Option<&str>) -> Result<Option<NaiveDate>, AppError> {
    let Some(v) = value.filter(|v| !crate::ruby::is_blank(v)) else {
        return Ok(None);
    };
    let h = crate::ruby_date::parse(v);
    if h.year.is_none() && h.mon.is_none() && h.mday.is_none() && h.hour.is_none() {
        return Ok(None);
    }
    if let Some(zone) = &h.zone {
        let utc = regex::Regex::new(r"(?i)\A(?:[-+]00:?00|[-+]00|UTC|Z|UT|GMT)\z")
            .expect("a literal pattern");
        if !utc.is_match(zone) {
            return Err(Unsupported("dashboard dates with a zone of their own").into());
        }
    }
    match crate::onebox::ruby_time::parse(v)? {
        crate::onebox::ruby_time::Parsed::Time(f) => Ok(NaiveDate::from_ymd_opt(
            f.year as i32,
            f.month as u32,
            f.day as u32,
        )),
        crate::onebox::ruby_time::Parsed::Raised => Ok(None),
    }
}

/// A dashboard section's date window: its defaults, and back to them when
/// the start is after the end.
fn window(
    start: Option<&str>,
    end: Option<&str>,
    default_days_back: i64,
    swap_check: bool,
) -> Result<(NaiveDateTime, NaiveDateTime), AppError> {
    let now = crate::clock::now_naive();
    let default_start = reports::beginning_of_day((now - Duration::days(default_days_back)).date());
    let default_end = reports::end_of_day(now.date());
    let mut s = zone_parse(start)?
        .map(reports::beginning_of_day)
        .unwrap_or(default_start);
    let mut e = zone_parse(end)?
        .map(reports::end_of_day)
        .unwrap_or(default_end);
    if swap_check && s.date() > e.date() {
        s = default_start;
        e = default_end;
    }
    Ok((s, e))
}

/// `Report.find_cached(type, opts) || Report.find(type, opts)` with the
/// cache filled, as the dashboard's builders do: the JSON, or None.
async fn cached_report(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    viewer: &Viewer<'_>,
    kind: &str,
    opts: &Opts,
    cache_errors: bool,
) -> Result<Option<Value>, AppError> {
    if let Some(hit) = reports::find_cached(conn, cx, viewer, kind, opts).await? {
        return Ok(Some(hit));
    }
    let Some(report) = reports::find(conn, cx, viewer, kind, opts).await? else {
        return Ok(None);
    };
    if cache_errors || report.error.is_none() {
        reports::cache(conn, cx, &report).await?;
    }
    Ok(Some(report.as_json(cx)))
}

fn has_error(report: &Value) -> bool {
    !matches!(report.get("error"), None | Some(Value::Null))
}

/// `period_value(data, average:)`
fn period_value(data: &Value, average: bool) -> Option<Value> {
    let points = data.as_array()?;
    if points.is_empty() {
        return None;
    }
    let ys: Vec<&Value> = points.iter().map(|p| &p["y"]).collect();
    if average {
        let sum: f64 = ys.iter().map(|y| to_f(y)).sum();
        return Some(reports::float_json(reports::round_to(
            sum / ys.len() as f64,
            1,
        )));
    }
    Some(json!(ys.iter().map(|y| to_i(y)).sum::<i64>()))
}

/// Ruby's `to_f` of a JSON value.
fn to_f(v: &Value) -> f64 {
    match v {
        Value::Number(n) => n.as_f64().unwrap_or(0.0),
        Value::String(s) => crate::ruby::to_f(s),
        _ => 0.0,
    }
}

/// Ruby's `to_i` of a JSON value.
fn to_i(v: &Value) -> i64 {
    match v {
        Value::Number(n) => n
            .as_i64()
            .unwrap_or_else(|| n.as_f64().unwrap_or(0.0) as i64),
        Value::String(s) => crate::ruby::to_i(s),
        _ => 0,
    }
}

/// `compute_percent_change(current, previous)`
fn percent_change(current: &Option<Value>, previous: &Value) -> Value {
    let Some(current) = current else {
        return Value::Null;
    };
    if previous.is_null() || to_f(previous) == 0.0 {
        return Value::Null;
    }
    let prev = to_f(previous);
    reports::float_json(reports::round_to((to_f(current) - prev) / prev * 100.0, 2))
}

/// `AdminDashboardKpis#build_kpi`
async fn build_kpi(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    kind: &str,
    report_name: &str,
    start: NaiveDateTime,
    end: NaiveDateTime,
) -> Result<Option<Value>, AppError> {
    let opts = Opts {
        start_date: Some(start),
        end_date: Some(end),
        facets: Some(vec!["prev_period".into()]),
        ..Opts::default()
    };
    let viewer = Viewer { guardian: None };
    let Some(report) = cached_report(conn, cx, &viewer, report_name, &opts, false).await? else {
        return Ok(None);
    };
    if has_error(&report) || report["data"].is_null() {
        return Ok(None);
    }
    let average = report["average"] == json!(true);
    let current = period_value(&report["data"], average);
    let previous = report.get("prev_period").cloned().unwrap_or(Value::Null);
    let change = percent_change(&current, &previous);
    Ok(Some(json!({
        "type": kind,
        "value": current,
        "previous_value": previous,
        "percent_change": change,
        "report_type": report_name,
        "report_query": {
            "start_date": start.date().format("%Y-%m-%d").to_string(),
            "end_date": end.date().format("%Y-%m-%d").to_string(),
        },
    })))
}

/// discourse-solved's accepted solutions KPI, while solved is in use.
pub(crate) async fn solved_kpi_on(conn: &mut PgConnection, cx: &Cx<'_>) -> Result<bool, AppError> {
    if !cx.settings.get("solved_enabled")?.truthy() {
        return Ok(false);
    }
    if cx.settings.get("allow_solved_on_all_topics")?.truthy() {
        return Ok(true);
    }
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM categories INNER JOIN category_custom_fields \
           ON category_custom_fields.category_id = categories.id \
         WHERE category_custom_fields.name = 'enable_accepted_answers' \
           AND category_custom_fields.value = 'true')",
    )
    .fetch_one(conn)
    .await?)
}

/// `AdminDashboardHighlights.build`
async fn highlights(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    start: Option<&str>,
    end: Option<&str>,
) -> Result<Value, AppError> {
    let (s, e) = window(start, end, 30, false)?;
    let mut kpis = vec![
        ("new_signups", "signups"),
        ("dau_mau", "dau_by_mau"),
        ("new_contributors", "new_contributors"),
    ];
    if solved_kpi_on(conn, cx).await? {
        kpis.push(("accepted_solutions", "accepted_solutions"));
    }
    let mut out = Vec::new();
    for (kind, report) in kpis {
        if let Some(k) = build_kpi(conn, cx, kind, report, s, e).await? {
            out.push(k);
        }
    }
    Ok(json!({ "kpis": out }))
}

/// `Category.secured(guardian).in_order_of(:id, ids).pluck(:id)`
async fn visible_category_ids(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    viewer: &Viewer<'_>,
    ids: &Value,
) -> Result<Value, AppError> {
    let list: Vec<i64> = match ids {
        Value::Array(a) if !a.is_empty() => a.iter().filter_map(Value::as_i64).collect(),
        other => return Ok(other.clone()),
    };
    let secure: Vec<i32> = match viewer.guardian {
        Some(g) => g.secure_category_ids(&mut *conn, cx.settings).await?,
        None => Vec::new(),
    };
    let found: Vec<i64> = sqlx::query_scalar(
        "SELECT id::bigint FROM categories WHERE id = ANY($1) \
           AND (NOT read_restricted OR id = ANY($2)) \
         ORDER BY array_position($1::bigint[], id::bigint)",
    )
    .bind(&list)
    .bind(&secure)
    .fetch_all(conn)
    .await?;
    Ok(json!(found))
}

/// `visible_groups(groups)`: the tokens still valid for the viewer, else
/// the default groups.
async fn visible_groups(
    conn: &mut PgConnection,
    viewer: &Viewer<'_>,
    groups: &Value,
) -> Result<Value, AppError> {
    let defaults = json!(["new_members", "returning", "staff"]);
    let Some(items) = groups.as_array().filter(|a| !a.is_empty()) else {
        return Ok(defaults);
    };
    let mut out = Vec::new();
    for token in items.iter().filter_map(Value::as_str) {
        if ["new_members", "returning", "staff"].contains(&token) {
            out.push(token.to_string());
            continue;
        }
        let Some(id) = token
            .strip_prefix("group:")
            .filter(|d| !d.is_empty() && d.chars().all(|c| c.is_ascii_digit()))
            .and_then(|d| d.parse::<i32>().ok())
        else {
            continue;
        };
        let row: Option<(i32, i32)> = sqlx::query_as(
            "SELECT visibility_level, members_visibility_level FROM groups WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?;
        let Some((level, members)) = row else {
            continue;
        };
        let visible = viewer.is_admin()
            || match viewer.guardian {
                Some(g) => {
                    crate::groups::can_see_group_and_members(&mut *conn, g, id, level, members)
                        .await?
                }
                None => false,
            };
        if visible {
            out.push(token.to_string());
        }
    }
    Ok(if out.is_empty() { defaults } else { json!(out) })
}

/// `AdminDashboardEngagement.build`
async fn engagement(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    viewer: &Viewer<'_>,
    start: Option<&str>,
    end: Option<&str>,
) -> Result<Value, AppError> {
    let (s, e) = window(start, end, 30, false)?;
    let mut kpis = Vec::new();
    for (kind, report) in [
        ("dau_mau", "dau_by_mau"),
        ("daily_engaged_users", "daily_engaged_users"),
        ("new_signups", "signups"),
    ] {
        if let Some(k) = build_kpi(conn, cx, kind, report, s, e).await? {
            kpis.push(k);
        }
    }
    // The trust level pipeline, found without a viewer.
    let opts = Opts {
        start_date: Some(s),
        end_date: Some(e),
        ..Opts::default()
    };
    let none = Viewer { guardian: None };
    let pipeline =
        match cached_report(conn, cx, &none, "trust_level_pipeline", &opts, false).await? {
            Some(r) if !has_error(&r) => json!({
                "rows": r["data"],
                "trend": r.get("prev_period").cloned().unwrap_or(Value::Null),
                "total_members": r.get("total").cloned().unwrap_or(Value::Null),
            }),
            _ => Value::Null,
        };
    let settings = settings_for(conn, "engagement").await?;
    // Who's posting, with the section's categories and groups.
    let whos = &settings["whos_posting"];
    let selected_ids = whos.get("category_ids").cloned().unwrap_or(Value::Null);
    let selected_groups = whos.get("groups").cloned().unwrap_or(Value::Null);
    let mut filters = Map::new();
    if !is_blank(&selected_ids) {
        filters.insert("category_ids".into(), selected_ids.clone());
    }
    if !is_blank(&selected_groups) {
        filters.insert("groups".into(), selected_groups.clone());
    }
    let opts = Opts {
        start_date: Some(s),
        end_date: Some(e),
        filters: (!filters.is_empty()).then_some(filters),
        ..Opts::default()
    };
    let posters =
        match cached_report(conn, cx, viewer, "posters_by_member_type", &opts, false).await? {
            Some(r) if !has_error(&r) => json!({
                "rows": r["data"],
                "total": r.get("total").cloned().unwrap_or(Value::Null),
                "category_ids": visible_category_ids(conn, cx, viewer, &selected_ids).await?,
                "groups": visible_groups(conn, viewer, &selected_groups).await?,
            }),
            _ => Value::Null,
        };
    let selected = settings["activity_by_category"]
        .get("category_ids")
        .cloned()
        .unwrap_or(Value::Null);
    let opts = Opts {
        start_date: Some(s),
        end_date: Some(e),
        filters: (!is_blank(&selected)).then(|| {
            let mut m = Map::new();
            m.insert("category_ids".into(), selected.clone());
            m
        }),
        ..Opts::default()
    };
    let activity =
        match cached_report(conn, cx, viewer, "activity_by_category", &opts, false).await? {
            Some(r) if !has_error(&r) => json!({
                "rows": r["data"],
                "total": r.get("total").cloned().unwrap_or(Value::Null),
                "category_ids": visible_category_ids(conn, cx, viewer, &selected).await?,
            }),
            _ => Value::Null,
        };
    Ok(json!({
        "kpis": kpis,
        "trust_level_pipeline": pipeline,
        "posters": posters,
        "activity_by_category": activity,
    }))
}

fn is_blank(v: &Value) -> bool {
    match v {
        Value::Null | Value::Bool(false) => true,
        Value::String(s) => crate::ruby::is_blank(s),
        Value::Array(a) => a.is_empty(),
        Value::Object(o) => o.is_empty(),
        _ => false,
    }
}

/// Ruby's `Float#round` to an Integer (half away from zero).
fn round_int(x: f64) -> i64 {
    x.round() as i64
}

/// `formatted_change(value)`: a tenth under one, whole above; nil for none.
fn formatted_change(value: f64) -> Option<Value> {
    if value.abs() < 1.0 {
        let r = reports::round_to(value, 1);
        (r != 0.0).then(|| reports::float_json(r))
    } else {
        let r = round_int(value);
        (r != 0).then(|| json!(r))
    }
}

/// `AdminDashboardSearch.build`
async fn search(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    start: Option<&str>,
    end: Option<&str>,
) -> Result<Value, AppError> {
    let (s, e) = window(start, end, 29, true)?;
    if !cx.settings.get("log_search_queries")?.truthy() {
        return Ok(json!({ "logging_enabled": false }));
    }
    let crawler_scorer = reports::crawler_scorer_enabled(cx.settings)?;
    // SearchLog.human_only
    let human = if crawler_scorer {
        "LEFT OUTER JOIN users ON users.id = search_logs.user_id \
         WHERE (users.id IS NULL OR (NOT users.admin AND NOT users.moderator)) \
           AND search_logs.crawler = FALSE AND search_logs.likely_crawler = FALSE"
    } else {
        "INNER JOIN users ON users.id = search_logs.user_id \
         WHERE users.admin = FALSE AND users.moderator = FALSE"
    };
    let lit = reports_lit;
    let in_window = |a: NaiveDateTime, b: NaiveDateTime| {
        format!(
            "FROM search_logs {human} AND search_logs.created_at BETWEEN {} AND {}",
            lit(a),
            lit(b)
        )
    };
    let day_count = (e.date() - s.date()).num_days() + 1;
    let prior_end = reports::end_of_day(s.date() - Duration::days(1));
    let prior_start = reports::beginning_of_day(s.date() - Duration::days(day_count));
    let stats = |from: String| {
        format!(
            "SELECT COALESCE(SUM(searches), 0)::bigint AS total, \
               COALESCE(SUM(CASE WHEN clicks = 0 THEN searches ELSE 0 END), 0)::bigint AS no_match \
             FROM (SELECT COUNT(*) AS searches, \
                     SUM(CASE WHEN search_result_id IS NOT NULL THEN 1 ELSE 0 END) AS clicks \
                   {from} GROUP BY lower(search_logs.term)) term_stats LIMIT 1"
        )
    };
    let (cur_total, cur_none): (i64, i64) = sqlx::query_as(&stats(in_window(s, e)))
        .fetch_one(&mut *conn)
        .await?;
    let (prior_total, prior_none): (i64, i64) =
        sqlx::query_as(&stats(in_window(prior_start, prior_end)))
            .fetch_one(&mut *conn)
            .await?;
    let rate = |none: i64, total: i64| none as f64 * 100.0 / total as f64;
    let mut total_kpi = Map::new();
    total_kpi.insert("value".into(), json!(cur_total));
    total_kpi.insert("previous_value".into(), json!(prior_total));
    if cur_total > 0 && prior_total > 0 {
        let change = (cur_total - prior_total) as f64 * 100.0 / prior_total as f64;
        if let Some(c) = formatted_change(change) {
            total_kpi.insert("percent_change".into(), c);
        }
    }
    let previous_rate = (prior_total > 0).then(|| round_int(rate(prior_none, prior_total)));
    let no_result = if cur_total == 0 {
        json!({"value": null, "previous_value": previous_rate, "exceeds_threshold": false})
    } else {
        let mut m = Map::new();
        m.insert("value".into(), json!(round_int(rate(cur_none, cur_total))));
        m.insert("previous_value".into(), json!(previous_rate));
        m.insert(
            "exceeds_threshold".into(),
            json!(cur_none * 100 > cur_total * 10),
        );
        if prior_total > 0
            && let Some(c) =
                formatted_change(rate(cur_none, cur_total) - rate(prior_none, prior_total))
        {
            m.insert("point_change".into(), c);
        }
        Value::Object(m)
    };
    let trending: Vec<(String, i64)> = sqlx::query_as(&format!(
        "SELECT lower(search_logs.term) AS term, COUNT(*) AS searches {} \
         GROUP BY lower(search_logs.term) \
         ORDER BY searches DESC, SUM(CASE WHEN search_result_id IS NOT NULL THEN 1 ELSE 0 END) DESC, term ASC \
         LIMIT 10",
        in_window(s, e)
    ))
    .fetch_all(&mut *conn)
    .await?;
    let gaps: Vec<(String, i64, i64)> = sqlx::query_as(&format!(
        "SELECT lower(search_logs.term) AS term, COUNT(*) AS searches, \
           SUM(CASE WHEN search_result_id IS NOT NULL THEN 1 ELSE 0 END) AS clicks {} \
         GROUP BY lower(search_logs.term) \
         HAVING SUM(CASE WHEN search_result_id IS NOT NULL THEN 1 ELSE 0 END) * 100 <= COUNT(*) * 20 \
         ORDER BY searches DESC, term ASC LIMIT 10",
        in_window(s, e)
    ))
    .fetch_all(&mut *conn)
    .await?;
    let period = match day_count {
        n if n <= 7 => "weekly",
        n if n <= 31 => "monthly",
        n if n <= 92 => "quarterly",
        n if n <= 366 => "yearly",
        _ => "all",
    };
    Ok(json!({
        "logging_enabled": true,
        "kpis": {"total_searches": total_kpi, "no_result_rate": no_result},
        "trending": trending.into_iter().map(|(t, n)| json!({"term": t, "searches": n})).collect::<Vec<_>>(),
        "trending_period": period,
        "content_gaps": gaps.into_iter().map(|(t, n, c)| json!({
            "term": t, "searches": n, "status": if c == 0 { "no_match" } else { "poor_match" },
        })).collect::<Vec<_>>(),
        "search_type": if crawler_scorer { "human_only" } else { "non_staff_only" },
    }))
}

fn reports_lit(t: NaiveDateTime) -> String {
    format!(
        "'{}'::timestamp",
        reports::bind_time(t).format("%Y-%m-%d %H:%M:%S%.6f")
    )
}

/// The vendored Discourse's `committed_at`, as `Discourse.last_commit_date`
/// (a DateTime) writes it in JSON.
pub fn last_commit_date() -> Option<String> {
    let at = crate::schema::discourse_committed_at()?;
    let at = chrono::DateTime::parse_from_rfc3339(at).ok()?;
    Some(
        at.with_timezone(&chrono::Utc)
            .format("%Y-%m-%dT%H:%M:%S%.3f+00:00")
            .to_string(),
    )
}

/// `DiscourseUpdates.check_version.as_json`: the vendored version, never
/// checked against meta, so the check stays pending (and, as Rails does
/// then, a call_discourse_hub job is enqueued).
pub async fn check_version(conn: &mut PgConnection, cx: &Cx<'_>) -> Result<Value, AppError> {
    let describe = crate::schema::discourse_describe()
        .map(|d| {
            // "-<count>-g<sha>" becomes " +<count>".
            let re = regex::Regex::new(r"-(\d+)-.*").expect("a literal pattern");
            re.replace(d, " +$1").into_owned()
        })
        .unwrap_or_else(|| "unknown".into());
    let mut out = Map::new();
    out.insert(
        "installed_version".into(),
        json!(crate::schema::discourse_version()),
    );
    out.insert(
        "installed_sha".into(),
        json!(crate::schema::discourse_commit()),
    );
    out.insert("installed_describe".into(), json!(describe));
    out.insert("git_branch".into(), json!("unknown"));
    out.insert("updated_at".into(), Value::Null);
    if cx.settings.get("version_checks")?.truthy() {
        crate::jobs::enqueue(conn, "call_discourse_hub", json!({"all_sites": true})).await?;
        out.insert("version_check_pending".into(), json!(true));
        out.insert("stale_data".into(), json!(true));
    }
    Ok(Value::Object(out))
}

/// `StatsCacheable.fetch_cached_stats` for the index and general data:
/// the cached JSON, filled for 35 minutes when missing.
pub async fn cached_stats(
    conn: &mut PgConnection,
    key: &str,
    fresh: impl FnOnce() -> Value,
) -> Result<Value, AppError> {
    if let Some(hit) = crate::owned_schema::cached_get(conn, key).await?
        && let Ok(v) = serde_json::from_str::<Value>(&hit)
    {
        return Ok(v);
    }
    let value = fresh();
    crate::owned_schema::cached_setex(conn, key, 35 * 60, &value.to_string()).await?;
    Ok(value)
}

fn now_json() -> Value {
    json!(
        crate::clock::now_naive()
            .format("%Y-%m-%dT%H:%M:%S%.3fZ")
            .to_string()
    )
}

/// `AdminDashboardIndexData.fetch_cached_stats`
pub async fn index_data(conn: &mut PgConnection) -> Result<Value, AppError> {
    cached_stats(
        conn,
        &format!("index-dashboard-data-{SCHEMA_VERSION}"),
        || json!({ "updated_at": now_json() }),
    )
    .await
}

/// `AdminDashboardGeneralData.fetch_cached_stats`
pub async fn general_data(conn: &mut PgConnection) -> Result<Value, AppError> {
    cached_stats(conn, &format!("general-dashboard-data-{SCHEMA_VERSION}"), || {
        // ((DateTime.now - last_commit_date) / 1.day).to_i: a day count
        // divided by a day's seconds, so 0.
        let before = if last_commit_date().is_some() { "0" } else { "" };
        json!({
            "updated_at": now_json(),
            "discourse_updated_at": last_commit_date(),
            "release_notes_link": format!(
                "https://meta.discourse.org/c/announcements/67?tags=release-notes&before={before}"
            ),
        })
    })
    .await
}

/// `AdminDashboardSystem.build`
async fn system(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    viewer: &Viewer<'_>,
) -> Result<Value, AppError> {
    let version = check_version(conn, cx).await?;
    let none = Viewer { guardian: None };
    let opts = Opts::default();
    let stats = match reports::find_cached(conn, cx, &none, "storage_stats", &opts).await? {
        Some(hit) => Some(hit["data"].clone()),
        None => match reports::find(conn, cx, &none, "storage_stats", &opts).await? {
            Some(r) if r.error.is_none() => {
                reports::cache(conn, cx, &r).await?;
                Some(r.data.clone())
            }
            _ => None,
        },
    };
    let storage = match stats.filter(|s| !is_blank(s)) {
        None => Value::Null,
        Some(Value::Object(mut m)) => {
            if !viewer.is_admin() {
                m.remove("backups");
            }
            let mut out = Map::new();
            for (store, values) in m {
                let v = match values {
                    Value::Object(mut v) => {
                        // Backups and uploads are both local here.
                        v.insert("remote".into(), json!(false));
                        Value::Object(v)
                    }
                    other => other,
                };
                out.insert(store, v);
            }
            Value::Object(out)
        }
        Some(other) => other,
    };
    let updated = index_data(conn).await?;
    Ok(json!({
        "version": version,
        "storage": storage,
        "discourse_updated_at": last_commit_date(),
        "dashboard_updated_at": updated.get("updated_at").cloned().unwrap_or(Value::Null),
    }))
}

/// A mountable report as the core provider resolves it.
fn resolved(entry: &Value) -> Value {
    let kind = entry["type"].as_str().unwrap_or_default();
    json!({
        "source": "core_report",
        "identifier": kind,
        "title": entry["title"],
        "description": entry["description"],
        "label": null,
        "url": format!("/admin/reports/{kind}"),
        "key": format!("core_report:{kind}"),
    })
}

/// `CoreReportProvider.dashboard_entries(guardian)`: the listed reports
/// (none excluded from the dashboard among those the port draws).
fn dashboard_entries(cx: &Cx<'_>, viewer: &Viewer<'_>) -> Result<Vec<Value>, AppError> {
    Ok(reports::list(cx, viewer)?
        .as_array()
        .cloned()
        .unwrap_or_default())
}

/// Refuses report sources the port doesn't run (discourse-data-explorer's).
fn refuse_other_sources(cx: &Cx<'_>) -> Result<(), AppError> {
    if cx
        .settings
        .get("data_explorer_enabled")
        .is_ok_and(|v| v.truthy())
    {
        return Err(Unsupported("discourse-data-explorer's dashboard reports").into());
    }
    Ok(())
}

/// `AdminDashboard::Reports::Section.build(guardian:, search:)`
pub async fn reports_section(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    viewer: &Viewer<'_>,
    search: Option<&str>,
) -> Result<Value, AppError> {
    refuse_other_sources(cx)?;
    let records: Vec<(i64, String, String, i32, i32, i32)> = sqlx::query_as(
        "SELECT id, source, identifier, position, rows, cols FROM admin_dashboard_reports \
         ORDER BY created_at DESC",
    )
    .fetch_all(conn)
    .await?;
    let entries = if viewer.guardian.is_some() {
        dashboard_entries(cx, viewer)?
    } else {
        Vec::new()
    };
    let mut visible: Vec<(i32, Value)> = Vec::new();
    for (_, source, identifier, position, rows, cols) in &records {
        if source != "core_report" {
            continue;
        }
        let Some(entry) = entries.iter().find(|e| e["type"] == json!(identifier)) else {
            continue;
        };
        let mut item = resolved(entry);
        item["rows"] = json!(rows);
        item["cols"] = json!(cols);
        visible.push((*position, item));
        if visible.len() == 10 {
            break;
        }
    }
    visible.sort_by_key(|(p, _)| *p);
    let mut items: Vec<Value> = visible.into_iter().map(|(_, v)| v).collect();
    if let Some(q) = search.filter(|s| !s.trim().is_empty()) {
        let q = q.to_lowercase();
        items.retain(|i| {
            i["title"]
                .as_str()
                .unwrap_or("")
                .to_lowercase()
                .contains(&q)
                || i["description"]
                    .as_str()
                    .unwrap_or("")
                    .to_lowercase()
                    .contains(&q)
        });
    }
    Ok(json!({ "items": items }))
}

/// `AdminDashboard::Reports::Listing.call(cursor:, search:)`
pub fn listing(
    cx: &Cx<'_>,
    admin: &Viewer<'_>,
    cursor: Option<(String, String)>,
    search: Option<&str>,
) -> Result<Value, AppError> {
    refuse_other_sources(cx)?;
    // list_all reads as the system user: an admin.
    let mut entries: Vec<Value> = dashboard_entries(cx, admin)?.iter().map(resolved).collect();
    if let Some(q) = search.filter(|s| !s.is_empty()) {
        let q = q.to_lowercase();
        entries.retain(|e| {
            e["title"]
                .as_str()
                .unwrap_or("")
                .to_lowercase()
                .contains(&q)
                || e["description"]
                    .as_str()
                    .unwrap_or("")
                    .to_lowercase()
                    .contains(&q)
        });
    }
    let sort_key = |e: &Value| {
        (
            e["title"].as_str().unwrap_or("").to_lowercase(),
            e["key"].as_str().unwrap_or("").to_string(),
        )
    };
    entries.sort_by_key(sort_key);
    if let Some((title, key)) = &cursor {
        let threshold = (title.to_lowercase(), key.clone());
        entries.retain(|e| sort_key(e) > threshold);
    }
    entries.truncate(31);
    let has_more = entries.len() > 30;
    entries.truncate(30);
    let cursor = if has_more {
        entries
            .last()
            .map(|e| json!({"title": e["title"], "key": e["key"]}))
            .unwrap_or(Value::Null)
    } else {
        Value::Null
    };
    Ok(json!({
        "providers": [{"source": "core_report", "label": null}],
        "items": entries,
        "has_more": has_more,
        "cursor": cursor,
    }))
}

/// An item of a reports payload, as parse_reports_items_payload checks it.
pub struct Item {
    pub source: String,
    pub identifier: String,
    pub rows: i64,
    pub cols: i64,
}

/// `parse_reports_items_payload`: None for the InvalidParameters(:items).
pub fn parse_items(items: Option<&Value>) -> Option<Vec<Item>> {
    let Some(Value::Array(list)) = items else {
        return None;
    };
    if list.len() > 10 {
        return None;
    }
    let mut out = Vec::new();
    for entry in list {
        let m = entry.as_object()?;
        let get = |k: &str| m.get(k).and_then(crate::params::scalar);
        let source = get("source").filter(|s| !crate::ruby::is_blank(s))?;
        let identifier = get("identifier").filter(|s| !crate::ruby::is_blank(s))?;
        // Integer(x.presence || 1, exception: false)
        let int = |k: &str| -> Option<i64> {
            match get(k).filter(|s| !crate::ruby::is_blank(s)) {
                None => Some(1),
                Some(s) => crate::ruby::integer(&s),
            }
        };
        let rows = int("rows")?;
        if !(1..=4).contains(&rows) {
            return None;
        }
        let cols = int("cols")?;
        if !(1..=2).contains(&cols) {
            return None;
        }
        if rows > 1 && cols != 2 {
            return None;
        }
        out.push(Item {
            source,
            identifier,
            rows,
            cols,
        });
    }
    Some(out)
}

/// What a layout update came to.
pub enum Layout {
    Saved,
    InvalidItems,
    InvalidAccess,
}

/// `AdminDashboard::Reports::LayoutUpdater.call(items:, guardian:)`
pub async fn update_layout(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    viewer: &Viewer<'_>,
    items: &[Item],
) -> Result<Layout, AppError> {
    refuse_other_sources(cx)?;
    let entries = dashboard_entries(cx, viewer)?;
    for item in items {
        if item.source != "core_report" {
            return Ok(Layout::InvalidItems);
        }
    }
    for item in items {
        if !entries.iter().any(|e| e["type"] == json!(item.identifier)) {
            return Ok(Layout::InvalidAccess);
        }
    }
    sqlx::query("DELETE FROM admin_dashboard_reports")
        .execute(&mut *conn)
        .await?;
    if !items.is_empty() {
        let now: NaiveDateTime = sqlx::query_scalar("SELECT clock_timestamp()::timestamp")
            .fetch_one(&mut *conn)
            .await?;
        for (i, item) in items.iter().enumerate() {
            sqlx::query(
                "INSERT INTO admin_dashboard_reports (source, identifier, position, rows, cols, created_at, updated_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $6)",
            )
            .bind(&item.source)
            .bind(&item.identifier)
            .bind(i as i32)
            .bind(item.rows as i32)
            .bind(item.cols as i32)
            .bind(now)
            .execute(&mut *conn)
            .await?;
        }
    }
    Ok(Layout::Saved)
}

/// `AdminDashboard::Reports::BulkFetch.call(items:, filters:, guardian:)`
pub async fn bulk_fetch(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    viewer: &Viewer<'_>,
    items: &[Item],
    filters: &Map<String, Value>,
) -> Result<Value, AppError> {
    refuse_other_sources(cx)?;
    let entries = dashboard_entries(cx, viewer)?;
    let mut opts = Opts::default();
    // build_opts: Time.zone.parse of the filter dates.
    if let Some(s) = filters.get("start_date").filter(|v| !v.is_null()) {
        let s = crate::params::scalar(s);
        opts.start_date = zone_parse(s.as_deref())?.map(reports::beginning_of_day);
    }
    if let Some(e) = filters.get("end_date").filter(|v| !v.is_null()) {
        let e = crate::params::scalar(e);
        opts.end_date = zone_parse(e.as_deref())?.map(reports::end_of_day);
    }
    let mut out = Vec::new();
    for item in items {
        let mut payload = Value::Null;
        if item.source == "core_report"
            && entries.iter().any(|e| e["type"] == json!(item.identifier))
        {
            let hit = reports::find_cached(conn, cx, viewer, &item.identifier, &opts).await?;
            let json = match hit {
                Some(h) => Some(h),
                None => match reports::find(conn, cx, viewer, &item.identifier, &opts).await? {
                    Some(r) => {
                        reports::cache(conn, cx, &r).await?;
                        Some(r.as_json(cx))
                    }
                    None => None,
                },
            };
            if let Some(Value::Object(mut m)) = json {
                let empty = m.get("data").is_none_or(is_blank);
                m.insert("empty".into(), json!(empty));
                payload = Value::Object(m);
            }
        }
        let errored = payload.get("error").is_some_and(|e| !is_blank(e));
        out.push(json!({
            "source": item.source,
            "identifier": item.identifier,
            "key": format!("{}:{}", item.source, item.identifier),
            "data": payload,
            "error": errored,
        }));
    }
    Ok(json!({ "items": out }))
}

/// `AdminDashboardSectionLoader.build`: each visible section's data, or
/// `{data: nil, error: true}` when its builder failed.
pub async fn build_sections(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    viewer: &Viewer<'_>,
    ids: &[String],
    start: Option<&str>,
    end: Option<&str>,
) -> Result<Value, AppError> {
    let mut out = Vec::new();
    for id in ids {
        let data = match id.as_str() {
            "highlights" => highlights(conn, cx, start, end).await,
            "traffic" => traffic(conn, cx, viewer, start, end).await,
            "engagement" => engagement(conn, cx, viewer, start, end).await,
            "reports" => reports_section(conn, cx, viewer, None).await,
            "search" => search(conn, cx, start, end).await,
            "system" => system(conn, cx, viewer).await,
            "support" => Err(Unsupported("discourse-solved's support section").into()),
            _ => Ok(Value::Null),
        };
        match data {
            Ok(d) => out.push(json!({"id": id, "data": d})),
            Err(e) if e.is_unsupported() => return Err(e),
            Err(e) => {
                tracing::warn!(section = %id, error = %e, "failed to build admin dashboard section");
                out.push(json!({"id": id, "data": null, "error": true}));
            }
        }
    }
    Ok(Value::Array(out))
}

/// `AdminDashboardSiteTraffic.build`
async fn traffic(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    viewer: &Viewer<'_>,
    start: Option<&str>,
    end: Option<&str>,
) -> Result<Value, AppError> {
    crate::admin_dashboard_traffic::build(conn, cx, viewer, start, end).await
}
