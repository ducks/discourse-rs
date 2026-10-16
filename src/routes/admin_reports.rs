//! /admin/reports: Admin::ReportsController (staff; everyone else 404s,
//! as StaffConstraint has it). The index lists the reports
//! (Reports::ListQuery), show and bulk find them (Report.find), cached
//! for an hour when asked (Report.find_cached / Report.cache, here in
//! discourse_rs.cached_values).

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, Uri};
use axum::response::{IntoResponse, Response};
use chrono::{Duration, NaiveDate};
use serde_json::{Map, Value, json};

use crate::onebox::ruby_time::{self, Parsed};
use crate::reports::{self, Cx, Opts, Viewer};
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

/// `Admin::ReportsController::REPORTS_LIMIT`
const REPORTS_LIMIT: i64 = 50;

fn not_found(state: &AppState) -> Response {
    super::topics::not_found_response(state)
}

/// What the request's params came to: the find options, or the 400.
enum Params {
    Opts(Opts),
    Invalid(String),
}

/// `Time.parse(s).to_date`, or ArgumentError's message.
fn parse_date(s: &str) -> Result<Result<NaiveDate, String>, AppError> {
    let empty = {
        let h = crate::ruby_date::parse(s);
        h.year.is_none()
            && h.mon.is_none()
            && h.mday.is_none()
            && h.yday.is_none()
            && h.hour.is_none()
            && h.min.is_none()
            && h.sec.is_none()
            && !h.sec_fraction
    };
    if empty && s.is_ascii() {
        return Ok(Err(format!(
            "no time information in {}",
            crate::ruby::inspect_string(s)
        )));
    }
    match ruby_time::parse(s)? {
        Parsed::Time(f) => Ok(
            NaiveDate::from_ymd_opt(f.year as i32, f.month as u32, f.day as u32)
                .ok_or_else(|| "argument out of range".to_string()),
        ),
        Parsed::Raised => Ok(Err("argument out of range".into())),
    }
}

/// A param's string, `present?` ones only.
fn present(p: &Map<String, Value>, key: &str) -> Option<String> {
    p.get(key)
        .and_then(crate::params::scalar)
        .filter(|s| !crate::ruby::is_blank(s))
}

/// `parse_params(report_params)`
fn parse_params(p: &Map<String, Value>) -> Result<Params, AppError> {
    let now = crate::clock::now_naive();
    let start = match present(p, "start_date") {
        Some(s) => match parse_date(&s)? {
            Ok(d) => reports::beginning_of_day(d),
            Err(m) => return Ok(Params::Invalid(m)),
        },
        None => reports::beginning_of_day((now - Duration::days(1)).date()),
    };
    let end = match present(p, "end_date") {
        Some(s) => match parse_date(&s)? {
            Ok(d) => reports::end_of_day(d),
            Err(m) => return Ok(Params::Invalid(m)),
        },
        None => reports::end_of_day((start + Duration::days(30)).date()),
    };
    let facets = match p.get("facets") {
        Some(Value::Array(items)) => Some(
            items
                .iter()
                .map(|v| crate::params::scalar(v).unwrap_or_default())
                .collect(),
        ),
        _ => None,
    };
    // fetch_limit_from_params(default: nil, max: REPORTS_LIMIT)
    let limit = match p.get("limit") {
        None => None,
        Some(v) => {
            let n = crate::params::scalar(v).and_then(|s| crate::ruby::integer(&s));
            match n {
                Some(n) if (0..=REPORTS_LIMIT).contains(&n) => Some(n),
                _ => return Ok(Params::Invalid("limit".into())),
            }
        }
    };
    let filters = match p.get("filters") {
        None => None,
        Some(Value::Object(m)) => Some(m.clone()),
        Some(Value::Null) => Some(Map::new()),
        Some(_) => return Err(Unsupported("report filters that aren't a hash").into()),
    };
    let include_related_items = crate::ruby::cast_boolean(
        p.get("include_related_items")
            .and_then(crate::params::scalar)
            .as_deref(),
    )
    .unwrap_or(false);
    Ok(Params::Opts(Opts {
        start_date: Some(start),
        end_date: Some(end),
        facets,
        limit,
        filters,
        include_related_items,
    }))
}

/// Whether a report type is one the controller looks up.
fn valid_type(kind: &str) -> bool {
    !kind.is_empty()
        && kind
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

async fn context<'a>(
    conn: &mut sqlx::PgConnection,
    state: &'a AppState,
    settings: &'a SiteSettings,
    urls: &'a crate::url::Urls<'a>,
) -> Result<Cx<'a>, AppError> {
    reports::context(conn, settings, &state.site_setting_defs, &state.i18n, urls).await
}

async fn cached(
    conn: &mut sqlx::PgConnection,
    cx: &Cx<'_>,
    viewer: &Viewer<'_>,
    kind: &str,
    opts: &Opts,
) -> Result<Option<Value>, AppError> {
    reports::find_cached(conn, cx, viewer, kind, opts).await
}

async fn cache(
    conn: &mut sqlx::PgConnection,
    cx: &Cx<'_>,
    report: &reports::Report,
) -> Result<(), AppError> {
    reports::cache(conn, cx, report).await
}

fn truthy_param(p: &Map<String, Value>, key: &str) -> bool {
    match p.get(key) {
        None | Some(Value::Null) | Some(Value::Bool(false)) => false,
        Some(_) => true,
    }
}

/// GET /admin/reports(.json)
pub async fn index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
) -> Result<Response, AppError> {
    if !guardian.is_staff() {
        return Ok(not_found(&state));
    }
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let list = list_doc(&mut conn, &state, &settings, &guardian).await?;
    Ok(Json(json!({ "reports": list })).into_response())
}

/// The index's reports, as the admin reports page lists them.
pub async fn list_doc(
    conn: &mut sqlx::PgConnection,
    state: &AppState,
    settings: &SiteSettings,
    guardian: &crate::guardian::Guardian,
) -> Result<Value, AppError> {
    let urls = crate::url::Urls {
        config: &state.config,
        settings,
    };
    let cx = context(&mut *conn, state, settings, &urls).await?;
    let viewer = Viewer {
        guardian: Some(guardian),
    };
    Ok(json!(reports::list(&cx, &viewer)?))
}

/// GET /admin/reports/:type(.json)
pub async fn show(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(kind): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if !guardian.is_staff() {
        return Ok(not_found(&state));
    }
    if !super::admin_pages::wants_json(&uri, &headers) {
        return page(&state, &guardian, &kind, &headers, &uri).await;
    }
    let kind = kind.strip_suffix(".json").unwrap_or(&kind).to_string();
    let p = crate::params::parse(uri.query(), &headers, &body);
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let viewer = Viewer {
        guardian: Some(&guardian),
    };
    if !valid_type(&kind) || reports::hidden(&kind, &viewer, &settings)? {
        return Ok(not_found(&state));
    }
    let opts = match parse_params(&p)? {
        Params::Opts(o) => o,
        Params::Invalid(m) => return Ok(super::search::invalid_parameters(&state, &m)),
    };
    let urls = crate::url::Urls {
        config: &state.config,
        settings: &settings,
    };
    let cx = context(&mut conn, &state, &settings, &urls).await?;
    let use_cache = truthy_param(&p, "cache");
    if use_cache && let Some(hit) = cached(&mut conn, &cx, &viewer, &kind, &opts).await? {
        return Ok(Json(json!({ "report": hit })).into_response());
    }
    let Some(report) = reports::find(&mut conn, &cx, &viewer, &kind, &opts).await? else {
        return Ok(not_found(&state));
    };
    if use_cache {
        cache(&mut conn, &cx, &report).await?;
    }
    Ok(Json(json!({ "report": report.as_json(&cx) })).into_response())
}

/// GET /admin/reports/bulk(.json): each of `reports[<type>]`, a report
/// that is hidden or not found carrying `error: "not_found"`.
pub async fn bulk(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if !guardian.is_staff() {
        return Ok(not_found(&state));
    }
    let p = crate::params::parse(uri.query(), &headers, &body);
    let Some(Value::Object(requested)) = p.get("reports") else {
        return Err(Unsupported("bulk reports without a reports hash").into());
    };
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let urls = crate::url::Urls {
        config: &state.config,
        settings: &settings,
    };
    let cx = context(&mut conn, &state, &settings, &urls).await?;
    let viewer = Viewer {
        guardian: Some(&guardian),
    };
    let out = bulk_reports(&mut conn, &state, &cx, &viewer, requested).await?;
    Ok(match out {
        Ok(list) => Json(json!({ "reports": list })).into_response(),
        Err(r) => r,
    })
}

async fn bulk_reports(
    conn: &mut sqlx::PgConnection,
    state: &AppState,
    cx: &Cx<'_>,
    viewer: &Viewer<'_>,
    requested: &Map<String, Value>,
) -> Result<Result<Vec<Value>, Response>, AppError> {
    let mut list = Vec::new();
    for (kind, params) in requested {
        if !valid_type(kind) {
            return Ok(Err(not_found(state)));
        }
        let params = match params {
            Value::Object(m) => m.clone(),
            _ => return Err(Unsupported("bulk report params that aren't a hash").into()),
        };
        let opts = match parse_params(&params)? {
            Params::Opts(o) => o,
            Params::Invalid(m) => return Ok(Err(super::search::invalid_parameters(state, &m))),
        };
        let use_cache = truthy_param(&params, "cache");
        let mut found: Option<Value> = None;
        if use_cache {
            found = cached(conn, cx, viewer, kind, &opts).await?;
        }
        if reports::hidden(kind, viewer, cx.settings)? {
            let mut r = reports::get(cx, viewer, kind, &opts)?;
            r.error = Some("not_found");
            found = Some(r.as_json(cx));
        }
        if let Some(v) = found {
            list.push(v);
            continue;
        }
        let report = reports::find(conn, cx, viewer, kind, &opts).await?;
        match report {
            Some(r) => {
                if use_cache {
                    cache(conn, cx, &r).await?;
                }
                list.push(r.as_json(cx));
            }
            None => {
                let mut r = reports::get(cx, viewer, kind, &opts)?;
                r.error = Some("not_found");
                list.push(r.as_json(cx));
            }
        }
    }
    Ok(Ok(list))
}

/// `$.param`'s strings for a filter value, as the bulk request sends the
/// route's filters.
fn param_value(v: &Value) -> Value {
    match v {
        Value::Null => json!(""),
        Value::Bool(b) => json!(b.to_string()),
        Value::Number(n) => json!(n.to_string()),
        Value::String(_) => v.clone(),
        Value::Array(a) => Value::Array(a.iter().map(param_value).collect()),
        Value::Object(m) => {
            Value::Object(m.iter().map(|(k, v)| (k.clone(), param_value(v))).collect())
        }
    }
}

/// `moment(value, "YYYY-MM-DD")` formatted back, the value itself when
/// it isn't a date.
fn route_date(v: &str) -> String {
    NaiveDate::parse_from_str(v, "%Y-%m-%d")
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|_| v.to_string())
}

/// GET /admin/reports/:type as the page (adminReports.show): the route's
/// model from its query params, the report as AdminReport loads it
/// (ReportLoader's bulk request with `_buildPayload`'s data), drawn by
/// crate::admin_report_view.
async fn page(
    state: &AppState,
    guardian: &crate::guardian::Guardian,
    kind: &str,
    headers: &HeaderMap,
    uri: &Uri,
) -> Result<Response, AppError> {
    let mut query: Map<String, Value> = Map::new();
    for (k, v) in form_urlencoded::parse(uri.query().unwrap_or("").as_bytes()) {
        query.insert(k.into_owned(), Value::String(v.into_owned()));
    }
    let param = |key: &str| {
        query
            .get(key)
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    };
    let mut conn = state.pool.acquire().await?;
    let shell =
        super::admin_pages::Shell::load(&mut conn, state, guardian, headers, "admin_all_reports")
            .await?;
    let mode = param("mode");
    // The route's redirect: site traffic is the legacy page views report
    // while those are used.
    if kind == "site_traffic" && shell.settings.get("use_legacy_pageviews")?.truthy() {
        let mut to = format!(
            "{}/admin/reports/consolidated_page_views",
            shell.site.base_path
        );
        if let Some(m) = &mode {
            to.push_str(&format!(
                "?mode={}",
                form_urlencoded::byte_serialize(m.as_bytes()).collect::<String>()
            ));
        }
        return Ok(axum::response::Redirect::to(&to).into_response());
    }
    let custom_filters = match param("filters") {
        None => None,
        Some(raw) => match serde_json::from_str::<Value>(&raw) {
            Ok(Value::Object(m)) => Some(m),
            Ok(Value::Null) => None,
            _ => {
                return Err(
                    Unsupported("a report page whose filters param isn't a JSON object").into(),
                );
            }
        },
    };
    let chart_grouping = param("chart_grouping");
    let today = crate::clock::now().date_naive();
    let start_date = param("start_date")
        .map(|v| route_date(&v))
        .unwrap_or_else(|| {
            let d = today - Duration::days(1);
            d.checked_sub_months(chrono::Months::new(1))
                .unwrap_or(d)
                .format("%Y-%m-%d")
                .to_string()
        });
    let end_date = param("end_date")
        .map(|v| route_date(&v))
        .unwrap_or_else(|| today.format("%Y-%m-%d").to_string());
    // reportOptions.table.limit
    let limit = if kind == "top_referred_topics" {
        10
    } else {
        50
    };
    let mut payload = Map::new();
    payload.insert("facets".into(), json!(["prev_period"]));
    payload.insert("start_date".into(), json!(start_date));
    payload.insert("end_date".into(), json!(end_date));
    payload.insert("limit".into(), json!(limit.to_string()));
    if let Some(f) = custom_filters.as_ref().filter(|f| !f.is_empty()) {
        payload.insert("filters".into(), param_value(&Value::Object(f.clone())));
    }
    if guardian.is_admin() {
        payload.insert("include_related_items".into(), json!("true"));
    }
    let mut requested = Map::new();
    requested.insert(kind.to_string(), Value::Object(payload));
    let urls = crate::url::Urls {
        config: &state.config,
        settings: &shell.settings,
    };
    let report = {
        let cx = context(&mut conn, state, &shell.settings, &urls).await?;
        let viewer = Viewer {
            guardian: Some(guardian),
        };
        // A failed request (a type the controller doesn't look up, params
        // it refuses) leaves the component without a model.
        match bulk_reports(&mut conn, state, &cx, &viewer, &requested).await? {
            Ok(list) => list.into_iter().find(|r| r["type"] == kind),
            Err(_) => None,
        }
    };
    let filter_types: Vec<String> = report
        .as_ref()
        .and_then(|r| r["available_filters"].as_array())
        .map(|a| {
            a.iter()
                .filter_map(|f| f["type"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let categories = if filter_types
        .iter()
        .any(|t| t == "category" || t == "category_list")
    {
        super::search::advanced_panel(state, &mut conn, &shell.settings, guardian)
            .await?
            .categories
    } else {
        Vec::new()
    };
    let groups: Vec<(i64, String)> = if filter_types.iter().any(|t| t == "group") {
        // Site#groups, without everyone.
        sqlx::query_as::<_, (i32, String)>("SELECT id, name FROM groups WHERE id > 0 ORDER BY name")
            .fetch_all(&mut *conn)
            .await?
            .into_iter()
            .map(|(id, name)| (i64::from(id), name))
            .collect()
    } else {
        Vec::new()
    };
    let hidden = shell
        .settings
        .get("dashboard_hidden_reports")?
        .to_s()
        .split('|')
        .any(|v| !v.is_empty() && v == kind);
    // The route's backLink: the dashboard when the page came from there.
    let base = shell.site.base_path.clone();
    let from_dashboard = headers
        .get("hx-current-url")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split_once("://"))
        .and_then(|(_, rest)| rest.find('/').map(|i| rest[i..].to_string()))
        .map(|rest| {
            let rest = rest.split('#').next().unwrap_or_default().to_string();
            let (path, q) = rest.split_once('?').unwrap_or((&rest, ""));
            (path.to_string(), q.to_string())
        })
        .filter(|(path, _)| {
            let path = path.strip_prefix(base.as_str()).unwrap_or(path);
            path == "/admin" || path == "/admin/"
        });
    let back = match from_dashboard {
        Some((_, q)) => crate::admin_report_view::Back {
            label_key: "admin.reports.back_to_dashboard",
            href: if q.is_empty() {
                "/admin".into()
            } else {
                format!("/admin?{q}")
            },
        },
        None => crate::admin_report_view::Back {
            label_key: "admin.reports.back",
            href: "/admin/reports".into(),
        },
    };
    let all_categories = crate::topic_list_view::categories(&mut conn).await?;
    let list = crate::topic_list_view::ListContext {
        i18n: &state.i18n,
        base_path: &shell.site.base_path,
        now: crate::clock::now(),
        categories: &all_categories,
        expand_all_pinned: false,
        member_trust_level: None,
        settings: crate::topic_list_view::ListSettings::load(&shell.settings)?,
    };
    let cx = shell.cx(state);
    let content = crate::admin_report_view::ReportPage {
        kind,
        report: report.as_ref(),
        mode: mode.as_deref(),
        chart_grouping: chart_grouping.as_deref(),
        start_date: &start_date,
        end_date: &end_date,
        custom_filters: custom_filters.as_ref(),
        hidden,
        admin: guardian.is_admin(),
        back,
        groups: &groups,
        categories: &categories,
        limit,
    }
    .render(&cx, &list);
    let main = crate::admin_view::contents("admin-reports", &content);
    let title = crate::admin_view::t(&state.i18n, "admin.config.reports.title");
    shell.render(
        state,
        uri,
        &title,
        main,
        "admin-report.js",
        axum::http::StatusCode::OK,
    )
}
