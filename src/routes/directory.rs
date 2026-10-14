//! GET /directory_items (directory_items#index): a period's user
//! directory, a page at a time.

use axum::Json;
use axum::extract::State;
use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use crate::directory::{PAGE_LIMIT, PAGE_SIZE, Query, Refused};
use crate::params;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::url::Urls;
use crate::{AppError, AppState};

/// `fetch_int_from_params(key, default:, max:)`, min 0: the value, or the
/// InvalidParameters it raises.
fn int_param(p: &Map<String, Value>, key: &str, default: i64, max: i64) -> Result<i64, ()> {
    match p.get(key).and_then(params::scalar) {
        None => Ok(default),
        Some(v) => match v.trim().parse::<i64>() {
            Ok(n) if (0..=max).contains(&n) => Ok(n),
            _ => Err(()),
        },
    }
}

/// `Hash#to_query`: keys sorted, values form-encoded.
fn to_query(pairs: &mut [(String, String)]) -> String {
    pairs.sort();
    let enc = |s: &str| form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>();
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", enc(k), enc(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// GET /directory_items
pub async fn index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    if !settings.get("enable_user_directory")?.truthy() {
        return Ok(super::search::invalid_access(&state));
    }
    let Some(period) = params::string(&p, "period").filter(|s| !s.is_empty()) else {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({"errors": ["param is missing or the value is empty or invalid: period"]})),
        )
            .into_response());
    };
    if !crate::directory::PERIODS.iter().any(|(n, _)| *n == period) {
        return Ok(super::search::invalid_access(&state));
    }
    let Ok(page) = int_param(&p, "page", 0, PAGE_LIMIT) else {
        return Ok(super::search::invalid_parameters(&state, "page"));
    };
    let Ok(limit) = int_param(&p, "limit", PAGE_SIZE, PAGE_SIZE) else {
        return Ok(super::search::invalid_parameters(&state, "limit"));
    };
    if p.contains_key("user_field_ids") {
        return Err(crate::Unsupported("user fields in the user directory").into());
    }
    let s = |k: &str| params::string(&p, k);
    let query = Query {
        period: period.clone(),
        group: s("group"),
        exclude_groups: s("exclude_groups"),
        exclude_usernames: s("exclude_usernames"),
        order: s("order"),
        ascending: s("asc").is_some_and(|v| !v.trim().is_empty()),
        name: s("name"),
        username: s("username"),
        page,
        limit,
    };
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let (items, total, last_updated_at) =
        match crate::directory::index(&mut conn, &settings, &urls, &guardian, &query).await? {
            Ok(r) => r,
            Err(Refused::GroupNotFound) => {
                return Ok(super::search::invalid_parameters(&state, "group"));
            }
            Err(Refused::InvalidAccess) => return Ok(super::search::invalid_access(&state)),
        };
    let mut more: Vec<(String, String)> =
        ["period", "order", "asc", "group", "user_field_ids", "name"]
            .iter()
            .filter_map(|k| s(k).map(|v| (k.to_string(), v)))
            .collect();
    more.push(("page".into(), (page + 1).to_string()));
    let base = state.config.globals.relative_url_root();
    let load_more = format!("{base}/directory_items.json?{}", to_query(&mut more));
    Ok(Json(json!({
        "directory_items": items,
        "meta": {
            "last_updated_at": last_updated_at,
            "total_rows_directory_items": total,
            "load_more_directory_items": load_more,
        },
    }))
    .into_response())
}
