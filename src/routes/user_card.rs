//! GET /user-card/:username: the user card's HTML (user_card_view) for
//! static/js/user-card.js. It is what the Ember client builds from
//! `/u/:username/card.json?include_post_count_for=<topic>` (users#show_card,
//! whose visit it tracks the same way), so a refusal there is the same
//! refusal here and the script closes the card, as Ember does when the
//! fetch fails.

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, header};
use axum::response::{IntoResponse, Response};

use super::search::Peer;
use super::users::ShowParams;
use crate::site_settings::SiteSettings;
use crate::user_card_view::{CardField, CardSettings, Viewer};
use crate::{AppError, AppState};

/// GET /user-card/{username}
pub async fn show(
    State(state): State<AppState>,
    axum::Extension(incoming): axum::Extension<crate::session::current::Incoming>,
    Path(username): Path<String>,
    Query(params): Query<ShowParams>,
    headers: HeaderMap,
    Peer(peer): Peer,
) -> Result<Response, AppError> {
    let doc = match super::users::card_doc(&state, &incoming, &username, &params, &headers, peer)
        .await?
    {
        Ok(doc) => doc,
        Err(refused) => return Ok(refused),
    };
    let guardian = &incoming.guardian;
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let fields: Vec<(i32, String, bool)> = sqlx::query_as(
        "SELECT id, name, searchable FROM user_fields WHERE show_on_user_card ORDER BY position, id",
    )
    .fetch_all(&mut *conn)
    .await?;
    let fields: Vec<CardField> = fields
        .into_iter()
        .map(|(id, name, searchable)| CardField {
            id: i64::from(id),
            name,
            searchable,
        })
        .collect();
    let topic_id = params
        .include_post_count_for
        .as_deref()
        .map(crate::ruby::to_i)
        .filter(|id| *id != 0)
        .map(|id| id as i32);
    let viewer = Viewer {
        id: guardian.user_id(),
        can_direct_message: crate::plugins::chat::user_can_direct_message(
            &mut conn, &settings, guardian,
        )
        .await?,
        topic_id,
    };
    let card_settings = CardSettings {
        enable_badges: settings.get("enable_badges")?.truthy(),
        allow_profile_backgrounds: settings.get("allow_profile_backgrounds")?.truthy(),
        display_local_time_in_user_card: settings.get("display_local_time_in_user_card")?.truthy(),
        tl3_links_no_follow: settings.get("tl3_links_no_follow")?.truthy(),
    };
    let categories = std::collections::HashMap::new();
    let cx = crate::topic_list_view::ListContext {
        i18n: &state.i18n,
        base_path: state.config.globals.relative_url_root(),
        now: crate::clock::now(),
        categories: &categories,
        expand_all_pinned: false,
        member_trust_level: guardian.user().map(|u| u.trust_level),
        settings: crate::topic_list_view::ListSettings::load(&settings)?,
    };
    let html =
        crate::user_card_view::render(&cx, &card_settings, &viewer, &fields, &doc, &username);
    Ok((
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        html,
    )
        .into_response())
}
