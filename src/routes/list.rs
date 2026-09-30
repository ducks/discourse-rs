//! Port of app/controllers/list_controller.rb (latest) with
//! lib/topic_query_params.rb.

use askama::Template;
use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::Html;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use crate::guardian::Guardian;
use crate::site_settings::SiteSettings;
use crate::topic_list::TopicListSerializer;
use crate::topic_query::{Options, TopicQuery};
use crate::url::Urls;
use crate::{AppError, AppState};

/// The TopicQuery.public_valid_options ported so far; the rest are
/// rejected until they are.
#[derive(Deserialize, Default)]
pub struct ListParams {
    page: Option<String>,
    per_page: Option<String>,
    order: Option<String>,
    ascending: Option<String>,
}

/// `build_topic_list_options` + `TopicQuery.validate?`: a bad value is
/// Discourse::InvalidParameters, a 400.
fn build_options(
    params: &ListParams,
    settings: &SiteSettings,
) -> Result<Options, (StatusCode, String)> {
    let invalid = |name: &str| (StatusCode::BAD_REQUEST, format!("{name} is invalid"));
    let mut options = Options {
        no_definitions: !settings
            .get("show_category_definitions_in_topic_lists")
            .map(|v| v.truthy())
            .unwrap_or(false),
        ..Options::default()
    };
    if let Some(page) = params.page.as_deref().filter(|p| !p.is_empty()) {
        let max = settings
            .get("max_topic_query_page_param")
            .map(|v| v.to_i())
            .unwrap_or(2000);
        let digits = page.strip_prefix('-').unwrap_or(page);
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return Err(invalid("page"));
        }
        let n: i64 = page.parse().map_err(|_| invalid("page"))?;
        if !(0..=max).contains(&n) {
            return Err(invalid("page"));
        }
        options.page = n;
    }
    if let Some(per_page) = params.per_page.as_deref().filter(|p| !p.is_empty()) {
        let n: i64 = per_page.parse().map_err(|_| invalid("per_page"))?;
        if !(1..=100).contains(&n) {
            return Err(invalid("per_page"));
        }
        options.per_page = Some(n);
    }
    if let Some(order) = params.order.as_deref().filter(|o| !o.is_empty()) {
        options.order = Some(order.to_string());
    }
    if let Some(ascending) = params.ascending.as_deref().filter(|a| !a.is_empty()) {
        options.ascending = match ascending {
            "true" => true,
            "false" => false,
            _ => return Err(invalid("ascending")),
        };
    }
    Ok(options)
}

/// `construct_url_with(:next, opts)`: `latest_path(opts + page+1)`, params
/// sorted by name, `.json` dropped.
fn next_url(base_path: &str, params: &ListParams, options: &Options) -> String {
    let mut pairs: Vec<(&str, String)> = Vec::new();
    if let Some(a) = params.ascending.as_deref().filter(|a| !a.is_empty()) {
        pairs.push(("ascending", a.to_string()));
    }
    if options.no_definitions {
        pairs.push(("no_definitions", "true".into()));
    }
    if let Some(o) = &options.order {
        pairs.push(("order", o.clone()));
    }
    pairs.push(("page", (options.page + 1).to_string()));
    if let Some(p) = options.per_page {
        pairs.push(("per_page", p.to_string()));
    }
    pairs.sort_by(|a, b| a.0.cmp(b.0));
    let query: Vec<String> = pairs.into_iter().map(|(k, v)| format!("{k}={v}")).collect();
    format!("{base_path}/latest?{}", query.join("&"))
}

/// The /latest document, shared by the JSON and HTML responses.
async fn latest_document(
    state: &AppState,
    params: &ListParams,
) -> Result<Result<(serde_json::Value, SiteSettings), (StatusCode, String)>, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let options = match build_options(params, &settings) {
        Ok(o) => o,
        Err(e) => return Ok(Err(e)),
    };
    let guardian = Guardian::anonymous();
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };

    let list = TopicQuery {
        conn: &mut conn,
        settings: &settings,
        guardian: &guardian,
        options: options.clone(),
    }
    .list_latest()
    .await?;

    let more = next_url(state.config.globals.relative_url_root(), params, &options);
    let json = TopicListSerializer {
        conn: &mut conn,
        settings: &settings,
        i18n: &state.i18n,
        guardian: &guardian,
        urls: &urls,
        more_topics_url: Some(more),
    }
    .serialize(&list)
    .await?;
    Ok(Ok((json, settings)))
}

/// GET /latest.json
pub async fn latest_json(
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> Result<Response, AppError> {
    match latest_document(&state, &params).await? {
        Ok((json, _)) => Ok(Json(json).into_response()),
        Err((status, message)) => Ok((status, message).into_response()),
    }
}

/// GET / and GET /latest: the server-rendered list. The "more" link
/// points at the HTML page.
pub async fn latest(
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> Result<Response, AppError> {
    let (json, settings) = match latest_document(&state, &params).await? {
        Ok(doc) => doc,
        Err((status, message)) => return Ok((status, message).into_response()),
    };
    let mut conn = state.pool.acquire().await?;
    let base_path = state.config.globals.relative_url_root();
    let site = crate::html::Site::from_settings(&settings, base_path)?;
    let mut page = crate::html::latest_page(&mut conn, site, &json).await?;
    // Strip `no_definitions`, which is what the JSON list carries around
    // but the HTML list applies on its own.
    page.more_url = page.more_url.map(|u| {
        u.replace("no_definitions=true&", "")
            .replace("?no_definitions=true", "")
    });
    Ok(Html(page.render().map_err(crate::html::HtmlError::from)?).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_url_sorts_params_and_increments_page() {
        let p = ListParams {
            per_page: Some("10".into()),
            ..ListParams::default()
        };
        let o = Options {
            page: 2,
            per_page: Some(10),
            no_definitions: true,
            ..Options::default()
        };
        assert_eq!(
            next_url("", &p, &o),
            "/latest?no_definitions=true&page=3&per_page=10"
        );
        let o = Options {
            no_definitions: true,
            ..Options::default()
        };
        assert_eq!(
            next_url("/forum", &ListParams::default(), &o),
            "/forum/latest?no_definitions=true&page=1"
        );
    }
}
