//! CategoriesController's reads of one category: show (`/c/:id/show`),
//! find_by_slug (`/c/*slug/find_by_slug`) and visible_groups
//! (`/c/:id/visible_groups`), dispatched from the `/c/*path` route.
//!
//! Refused: their HTML (the Ember app's category editor, also
//! `/c/*slug/edit(/:tab)` and `/new-category`).

use axum::Json;
use axum::http::{HeaderMap, header};
use axum::response::{IntoResponse, Response};
use serde_json::json;
use sqlx::PgConnection;

use crate::category_serializer::{self as cs, GuardedCategory, Preloaded};
use crate::guardian::Guardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

fn not_found(state: &AppState) -> Response {
    super::topics::not_found_response(state)
}

fn wants_json(headers: &HeaderMap, json_suffix: bool) -> bool {
    json_suffix
        || headers
            .get(header::ACCEPT)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|a| a.contains("application/json"))
}

/// The category actions under `/c/`: `Some` when `path` (without
/// `.json`) names one.
pub(super) async fn dispatch(
    state: &AppState,
    guardian: &Guardian,
    headers: &HeaderMap,
    path: &str,
    json_suffix: bool,
) -> Result<Option<Response>, AppError> {
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let json = wants_json(headers, json_suffix);
    match segments.as_slice() {
        [id, "show"] => {
            if !json {
                return Err(Unsupported("the category editor page (Ember)").into());
            }
            Ok(Some(show(state, guardian, id).await?))
        }
        [id, "visible_groups"] => Ok(Some(visible_groups(state, guardian, id).await?)),
        [slugs @ .., "find_by_slug"] if !slugs.is_empty() => {
            Ok(Some(find_by_slug(state, guardian, slugs).await?))
        }
        [slugs @ .., "edit"] | [slugs @ .., "edit", _] if !slugs.is_empty() && !json => {
            Err(Unsupported("the category editor page (Ember)").into())
        }
        _ => Ok(None),
    }
}

/// `fetch_category`: a top-level category by slug, else by `id.to_i`.
pub(crate) async fn fetch_category(
    conn: &mut PgConnection,
    s: &SiteSettings,
    id: &str,
) -> Result<Option<GuardedCategory>, AppError> {
    let max_nesting = s.get("max_category_nesting")?.to_i();
    let by_slug =
        crate::category::Category::find_by_slug_path(&mut *conn, &[id], max_nesting).await?;
    let id = match by_slug {
        Some(c) => c.id,
        None => {
            let id = crate::ruby::to_i(id);
            match i32::try_from(id) {
                Ok(id) => id,
                Err(_) => return Ok(None),
            }
        }
    };
    Ok(GuardedCategory::load(conn, id).await?)
}

/// GET /c/:id/show(.json): categories#show.
async fn show(state: &AppState, guardian: &Guardian, id: &str) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let s = SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let Some(c) = fetch_category(&mut conn, &s, id).await? else {
        return Ok(not_found(state));
    };
    if !cs::can_see(&mut conn, &s, guardian, &c).await? {
        return Ok(super::search::invalid_access(state));
    }
    let pre = Preloaded {
        permission: cs::topic_create_allowed(&mut conn, &s, guardian, c.id)
            .await?
            .then_some(cs::FULL),
        ..Default::default()
    };
    let json = cs::serialize(&mut conn, &cs::Cx::of(state), &s, guardian, c.id, &pre).await?;
    Ok(Json(json!({ "category": json })).into_response())
}

/// GET /c/*slug/find_by_slug(.json)
async fn find_by_slug(
    state: &AppState,
    guardian: &Guardian,
    slugs: &[&str],
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let s = SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let max_nesting = s.get("max_category_nesting")?.to_i();
    let Some(found) =
        crate::category::Category::find_by_slug_path(&mut conn, slugs, max_nesting).await?
    else {
        return super::topics::topic_not_found(state, guardian, "").await;
    };
    let Some(c) = GuardedCategory::load(&mut conn, found.id).await? else {
        return super::topics::topic_not_found(state, guardian, "").await;
    };
    if !cs::can_see(&mut conn, &s, guardian, &c).await? {
        // detailed_404 with a group that grants access answers
        // InvalidAccess naming the group.
        if s.get("detailed_404")?.truthy() {
            let via_group: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM groups g JOIN category_groups cg ON cg.group_id = g.id \
                 WHERE cg.category_id = $1 AND (g.public_admission OR g.allow_membership_requests))",
            )
            .bind(c.id)
            .fetch_one(&mut *conn)
            .await?;
            if via_group {
                return Err(Unsupported("detailed_404").into());
            }
        }
        return super::topics::topic_not_found(state, guardian, "").await;
    }
    let mut pre = cs::preload_user_fields(&mut conn, &s, &state.i18n, guardian, c.id).await?;
    // `topic_create_allowed` sets full first; preload_user_fields only
    // adds it.
    if cs::topic_create_allowed(&mut conn, &s, guardian, c.id).await? {
        pre.permission = Some(cs::FULL);
    }
    pre.category_setting = true;
    let json = cs::serialize(&mut conn, &cs::Cx::of(state), &s, guardian, c.id, &pre).await?;
    Ok(Json(json!({ "category": json })).into_response())
}

/// GET /c/:id/visible_groups(.json): the category's groups the viewer may
/// see, none when everyone has access.
async fn visible_groups(
    state: &AppState,
    guardian: &Guardian,
    id: &str,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let s = SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let Some(c) = fetch_category(&mut conn, &s, id).await? else {
        return Ok(not_found(state));
    };
    if !cs::can_see(&mut conn, &s, guardian, &c).await? {
        return Ok(super::search::invalid_access(state));
    }
    let everyone: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM category_groups WHERE category_id = $1 AND group_id = 0)",
    )
    .bind(c.id)
    .fetch_one(&mut *conn)
    .await?;
    let groups: Vec<String> = if everyone {
        Vec::new()
    } else {
        let visible = crate::groups::visible_groups_scope(guardian, "g", false, false);
        sqlx::query_scalar(&format!(
            "SELECT g.name FROM category_groups cg JOIN groups g ON g.id = cg.group_id \
             WHERE cg.category_id = $1 AND {visible} ORDER BY g.name ASC"
        ))
        .bind(c.id)
        .fetch_all(&mut *conn)
        .await?
    };
    Ok(Json(json!({ "success": "OK", "groups": groups })).into_response())
}
