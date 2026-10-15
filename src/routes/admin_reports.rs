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
    super::admin_pages::refuse_page(&uri, &headers, "the admin report page (AdminReport)")?;
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
