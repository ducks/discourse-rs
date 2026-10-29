//! discourse-cakeday's engine, mounted at /cakeday:
//! AnniversariesController#index and BirthdaysController#index (GET
//! /cakeday/anniversaries(/:filter) and /cakeday/birthdays(/:filter)) as
//! JSON, and the cakeday.anniversaries.* and cakeday.birthdays.* pages
//! Ember draws from them.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, Uri};
use axum::response::{IntoResponse, Response};

use crate::guardian::Guardian;
use crate::params;
use crate::plugins::cakeday::lists::{self, Kind, Params};
use crate::plugins::cakeday::view::{Filter, ListPage};
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::url::Urls;
use crate::{AppError, AppState};

/// GET /cakeday/anniversaries(.json)
pub async fn anniversaries(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    serve(&state, &guardian, Kind::Anniversaries, None, &headers, &uri).await
}

/// GET /cakeday/anniversaries/:filter(.json)
pub async fn anniversaries_filter(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(filter): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    serve(
        &state,
        &guardian,
        Kind::Anniversaries,
        Some(filter),
        &headers,
        &uri,
    )
    .await
}

/// GET /cakeday/birthdays(.json)
pub async fn birthdays(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    serve(&state, &guardian, Kind::Birthdays, None, &headers, &uri).await
}

/// GET /cakeday/birthdays/:filter(.json)
pub async fn birthdays_filter(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(filter): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    serve(
        &state,
        &guardian,
        Kind::Birthdays,
        Some(filter),
        &headers,
        &uri,
    )
    .await
}

/// The list's enabled setting: the plugin's (requires_plugin), and for
/// birthdays cakeday_birthday_enabled.
fn kind_enabled(settings: &SiteSettings, kind: Kind) -> Result<bool, AppError> {
    Ok(match kind {
        Kind::Anniversaries => crate::plugins::cakeday::enabled(settings)?,
        Kind::Birthdays => crate::plugins::cakeday::birthday_enabled(settings)?,
    })
}

async fn serve(
    state: &AppState,
    guardian: &Guardian,
    kind: Kind,
    segment: Option<String>,
    headers: &HeaderMap,
    uri: &Uri,
) -> Result<Response, AppError> {
    // The path's format: `:filter(.:format)`.
    let (segment, json_suffix) = match segment {
        Some(s) => match s.strip_suffix(".json") {
            Some(s) => (Some(s.to_string()), true),
            None => (Some(s), false),
        },
        None => (None, uri.path().ends_with(".json")),
    };
    let json = json_suffix || super::wants_json(uri.query(), headers);
    let p = params::parse(uri.query(), headers, b"");
    let param = |key: &str| p.get(key).and_then(params::scalar);
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };

    if json {
        // requires_plugin, ensure_logged_in, then the list's own setting.
        if !crate::plugins::cakeday::enabled(&settings)? {
            return Ok(super::topics::not_found_response(state));
        }
        let Some(viewer_id) = guardian.user_id() else {
            return Ok(super::login_required::not_logged_in(state));
        };
        if !kind_enabled(&settings, kind)? {
            return Ok(super::topics::not_found_response(state));
        }
        // The path's filter over the query's.
        let filter = segment.or_else(|| param("filter"));
        let params = Params::parse(filter, param("page").as_deref(), param("month").as_deref());
        let page = lists::page(&mut conn, &settings, &urls, viewer_id, kind, &params).await?;
        return Ok(Json(lists::document(kind, page)).into_response());
    }

    // The page: an unknown route while its setting is off.
    if !kind_enabled(&settings, kind)? {
        return Ok(super::topics::not_found_response(state));
    }
    let base = state.config.globals.relative_url_root();
    let Some(segment) = segment else {
        // cakeday.<kind>.index replaces itself with today.
        return Ok(
            axum::response::Redirect::to(&format!("{base}/cakeday/{}/today", kind.key()))
                .into_response(),
        );
    };
    let Some(filter) = Filter::from_segment(&segment) else {
        return Ok(super::topics::not_found_response(state));
    };
    let Some(viewer_id) = guardian.user_id() else {
        return Ok(super::login_required::not_logged_in(state));
    };
    let today = crate::plugins::cakeday::viewer_today(&mut conn, Some(viewer_id)).await?;
    // The All page's month: its query param, else this month.
    let month = param("month")
        .map(|m| crate::ruby::to_i(&m).clamp(1, 12))
        .unwrap_or_else(|| i64::from(chrono::Datelike::month(&today)));
    let params = Params {
        filter: filter.param(),
        page: param("page").map_or(0, |v| crate::ruby::to_i(&v).max(0)),
        month,
    };
    let list = lists::page(&mut conn, &settings, &urls, viewer_id, kind, &params).await?;
    let doc = lists::document(kind, list);

    let frame = super::groups::frame(
        state,
        &mut conn,
        &settings,
        guardian,
        headers,
        uri,
        crate::sidebar::Active::Route(format!("cakeday.{}.{}", kind.key(), filter.segment())),
    )
    .await?;
    let cx = crate::topic_list_view::ListContext {
        i18n: &state.i18n,
        base_path: base,
        now: crate::clock::now(),
        categories: &frame.categories,
        expand_all_pinned: false,
        hide_category: false,
        member_trust_level: frame.site.viewer.as_ref().map(|v| v.trust_level),
        settings: crate::topic_list_view::ListSettings::load(&settings)?,
    };
    let main = crate::plugins::cakeday::view::page(
        &cx,
        &ListPage {
            kind,
            filter,
            month,
            page: params.page,
            doc: &doc,
            birthdays: crate::plugins::cakeday::birthday_enabled(&settings)?,
            today,
        },
    );
    // The kind's route's titleToken.
    let page_title = crate::topic_list_view::t(&cx, &format!("{}.title", kind.key()));
    drop(cx);
    super::groups::finish(frame, &settings, page_title, main)
}
