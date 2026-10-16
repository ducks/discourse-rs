//! CategoriesController's reads of one category: show (`/c/:id/show`),
//! find_by_slug (`/c/*slug/find_by_slug`) and visible_groups
//! (`/c/:id/visible_groups`), dispatched from the `/c/*path` route.
//!
//! Refused: their HTML (the Ember app's category editor, also
//! `/c/*slug/edit(/:tab)` and `/new-category`).

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use serde_json::json;
use sqlx::PgConnection;

use crate::category_serializer::{self as cs, GuardedCategory, Preloaded};
use crate::guardian::Guardian;
use crate::session::current::AuthGuardian;
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

fn form_pairs(p: &serde_json::Map<String, serde_json::Value>) -> Vec<(String, String)> {
    p.iter()
        .filter_map(|(k, v)| crate::params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

/// A 422 with the errors.
fn unprocessable(errors: Vec<String>) -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({ "errors": errors })),
    )
        .into_response()
}

/// `params.require(key)`'s 400.
fn param_missing(key: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({
            "errors": [format!("param is missing or the value is empty or invalid: {key}")]
        })),
    )
        .into_response()
}

/// `params.require(:name)`: present and not blank.
fn present(v: Option<&serde_json::Value>) -> bool {
    match v {
        None | Some(serde_json::Value::Null) => false,
        Some(serde_json::Value::String(s)) => !crate::ruby::is_blank(s),
        Some(serde_json::Value::Bool(b)) => *b,
        Some(serde_json::Value::Array(a)) => !a.is_empty(),
        Some(serde_json::Value::Object(o)) => !o.is_empty(),
        Some(serde_json::Value::Number(_)) => true,
    }
}

/// `Categories::Configure.call` for a type: Ok(Err(message)) when the
/// type is not available to the guardian.
#[allow(clippy::too_many_arguments)]
async fn configure_type(
    conn: &mut PgConnection,
    state: &AppState,
    cx: &crate::category_writes::Cx<'_>,
    guardian: &Guardian,
    category_id: i32,
    category_type: &str,
    category_values: &serde_json::Map<String, serde_json::Value>,
    site_setting_values: &serde_json::Map<String, serde_json::Value>,
) -> Result<Result<(), String>, AppError> {
    cs::with_registry(&cs::Cx::of(state), cx.settings, async |r| {
        crate::category_type_config::configure(
            conn,
            cx,
            r,
            guardian,
            category_id,
            category_type,
            category_values,
            site_setting_values,
        )
        .await
    })
    .await
}

/// A hash param's scalar entries.
fn param_map(
    p: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> serde_json::Map<String, serde_json::Value> {
    match p.get(key) {
        Some(serde_json::Value::Object(m)) => m.clone(),
        _ => serde_json::Map::new(),
    }
}

/// The scalar params of a list param (`Array(params[key]).compact_blank`).
fn param_list(p: &serde_json::Map<String, serde_json::Value>, key: &str) -> Vec<String> {
    match p.get(key) {
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .filter_map(crate::params::scalar)
            .filter(|t| !crate::ruby::is_blank(t))
            .collect(),
        Some(v) => crate::params::scalar(v)
            .filter(|t| !crate::ruby::is_blank(t))
            .into_iter()
            .collect(),
        None => Vec::new(),
    }
}

/// `position.to_i`
fn position_to_i(position: &serde_json::Value) -> i64 {
    match position {
        serde_json::Value::Number(n) => n
            .as_i64()
            .unwrap_or_else(|| n.as_f64().unwrap_or(0.0) as i64),
        other => crate::ruby::to_i(&crate::params::scalar(other).unwrap_or_default()),
    }
}

/// POST /categories(.json): categories#create.
pub async fn create(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = crate::params::parse(uri.query(), &headers, &body);
    if !super::session::csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "POST") {
        return Ok(super::session::bad_csrf());
    }
    let Some(user_id) = guardian.user_id() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    let mut conn = state.pool.acquire().await?;
    let s = SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    // ensure_can_create!(Category)
    if !cs::can_create(&s, &guardian)? {
        return Ok(super::search::invalid_access(&state));
    }
    let mut params = crate::category_writes::permit(&p, &s)?;
    let position = params.remove("position");
    if let Some(description) = params.get("description").and_then(|d| d.as_str())
        && !crate::ruby::is_blank(description)
        && description.chars().count() > crate::category_writes::MAX_DESCRIPTION_PARAM_LENGTH
    {
        let message = state
            .i18n
            .t_count(
                "category.errors.description_too_long",
                crate::category_writes::MAX_DESCRIPTION_PARAM_LENGTH as i64,
                &[],
            )
            .unwrap_or_default();
        return Ok(unprocessable(vec![message]));
    }
    if !present(p.get("name")) {
        return Ok(param_missing("name"));
    }
    let category_type = p
        .get("category_type")
        .and_then(crate::params::scalar)
        .filter(|t| !crate::ruby::is_blank(t));
    let category_types = param_list(&p, "category_types");
    drop(conn);

    let host = crate::pretty_text::Host::from_state(&state);
    let cx = crate::category_writes::Cx {
        host: &host,
        bus: &state.bus,
        settings: &s,
    };
    let mut tx = state.pool.begin().await?;
    let mut record = crate::category_writes::Record::new(user_id);
    if let Err(rejected) = record.assign(&mut tx, &params).await? {
        tx.rollback().await?;
        return Ok(match rejected {
            crate::category_writes::Rejected::Argument(m) => unprocessable(vec![m]),
            crate::category_writes::Rejected::Invalid(e) => unprocessable(e),
        });
    }
    let id = match crate::category_writes::save(&mut tx, &cx, &mut record).await? {
        crate::category_writes::Saved::Invalid(errors) => {
            tx.rollback().await?;
            return Ok(unprocessable(errors));
        }
        crate::category_writes::Saved::Done(id) => id,
    };
    // move_to(position.to_i) if position
    if let Some(position) = position.filter(|p| !p.is_null()) {
        crate::category_writes::move_to(&mut tx, id, position_to_i(&position)).await?;
    }
    if let Some(t) = &category_type {
        // [custom_fields, category_type_settings (Events' keys only)]
        let values = params.custom_fields.clone().unwrap_or_default();
        let site_values = param_map(&p, "category_type_site_settings");
        if let Err(message) = configure_type(
            &mut tx,
            &state,
            &cx,
            &guardian,
            id,
            t,
            &values,
            &site_values,
        )
        .await?
        {
            tx.rollback().await?;
            return Ok(unprocessable(vec![message]));
        }
    }
    let additional: Vec<&String> = category_types
        .iter()
        .filter(|t| Some(*t) != category_type.as_ref() && *t != "discussion")
        .collect();
    let none = serde_json::Map::new();
    for t in &additional {
        if let Err(message) =
            configure_type(&mut tx, &state, &cx, &guardian, id, t, &none, &none).await?
        {
            tx.rollback().await?;
            return Ok(unprocessable(vec![message]));
        }
    }
    // `@category.reload` after configuring types: no saved changes, the
    // category_setting association unloaded.
    let reloaded = category_type.is_some() || !additional.is_empty();
    let hooked = crate::category_writes::after_commit(
        &mut tx,
        &cx,
        id,
        "category_created",
        (!reloaded).then_some(record.saved_changes.as_slice()),
        !reloaded,
    )
    .await?;
    crate::category_writes::log_creation(&mut tx, &cx, user_id, id).await?;
    tx.commit().await?;
    let mut conn = state.pool.acquire().await?;
    // The settings as the type configuration left them.
    let s = SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let json = cs::serialize(
        &mut conn,
        &cs::Cx::of(&state),
        &s,
        &guardian,
        id,
        &Preloaded {
            category_setting: !reloaded,
            // The web hook payload asked has_children? (can_delete).
            has_children: hooked.then_some(false),
            ..Default::default()
        },
    )
    .await?;
    Ok(Json(json!({ "category": json })).into_response())
}

/// `Category#category_types`' keys for the category: Discussion always,
/// then each plugin type it matches.
async fn current_types(
    conn: &mut PgConnection,
    state: &AppState,
    s: &SiteSettings,
    id: i32,
) -> Result<Vec<String>, AppError> {
    let types = cs::with_registry(&cs::Cx::of(state), s, async |r| {
        r.for_category(conn, id).await
    })
    .await?;
    Ok(types
        .as_object()
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default())
}

/// `manage_category_types`: the types the request's category_types adds
/// are checked for availability first (Ok(Err(message)) for one that
/// isn't), then the removed ones unconfigured and the added ones
/// configured. Returns the custom field keys the changed types own,
/// which leave the pending custom fields. Site settings given per type
/// (category_type_site_settings) are refused.
async fn manage_category_types(
    conn: &mut PgConnection,
    state: &AppState,
    cx: &crate::category_writes::Cx<'_>,
    guardian: &Guardian,
    p: &serde_json::Map<String, serde_json::Value>,
    id: i32,
) -> Result<Result<Vec<&'static str>, String>, AppError> {
    if p.get("category_type_site_settings")
        .is_some_and(|v| !crate::category_writes::blank_param(v))
    {
        return Err(Unsupported("a category type's site settings").into());
    }
    if !p.contains_key("category_types") {
        return Ok(Ok(Vec::new()));
    }
    let mut wanted = param_list(p, "category_types");
    wanted.push("discussion".into());
    let current = current_types(conn, state, cx.settings, id).await?;
    let removed: Vec<String> = current
        .iter()
        .filter(|t| !wanted.contains(t))
        .cloned()
        .collect();
    let mut added: Vec<String> = Vec::new();
    for t in wanted {
        if !current.contains(&t) && !added.contains(&t) {
            added.push(t);
        }
    }
    let scx = cs::Cx::of(state);
    // Preflight: every added type available before anything changes.
    for t in &added {
        let available = cs::with_registry(&scx, cx.settings, async |r| {
            Ok(match crate::category_type_config::parse(t) {
                Some(ty) => crate::category_type_config::available_for(r, ty, guardian)?,
                None => false,
            })
        })
        .await?;
        if !available {
            let message = cs::with_registry(&scx, cx.settings, async |r| {
                crate::category_type_config::unavailable_error(&state.i18n, r, t)
            })
            .await?;
            return Ok(Err(message));
        }
    }
    let mut dropped: Vec<&'static str> = Vec::new();
    for t in added.iter().chain(removed.iter()) {
        let Some(ty) = crate::category_type_config::parse(t) else {
            continue;
        };
        let keys = cs::with_registry(&scx, cx.settings, async |r| {
            crate::category_type_config::custom_field_keys(conn, r, ty).await
        })
        .await?;
        dropped.extend(keys);
    }
    for t in &removed {
        crate::category_type_config::unconfigure(conn, cx, guardian, id, t).await?;
    }
    let none = serde_json::Map::new();
    for t in &added {
        if let Err(message) = configure_type(conn, state, cx, guardian, id, t, &none, &none).await?
        {
            return Ok(Err(message));
        }
    }
    if p.get("category_type_settings")
        .is_some_and(|v| !crate::category_writes::blank_param(v))
    {
        return Err(Unsupported("a category type's category settings").into());
    }
    Ok(Ok(dropped))
}

/// The id segment of `/categories/:id(.json)`.
fn id_param(id: &str) -> &str {
    id.strip_suffix(".json").unwrap_or(id)
}

/// PUT /categories/:id(.json): categories#update.
pub async fn update(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    axum::extract::Path(id): axum::extract::Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = crate::params::parse(uri.query(), &headers, &body);
    if !super::session::csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "PUT") {
        return Ok(super::session::bad_csrf());
    }
    let Some(user_id) = guardian.user_id() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    let mut conn = state.pool.acquire().await?;
    let s = SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let Some(c) = fetch_category(&mut conn, &s, id_param(&id)).await? else {
        return Ok(not_found(&state));
    };
    if !cs::can_edit(&mut conn, &s, &guardian, &c).await? {
        return Ok(super::search::invalid_access(&state));
    }
    let mut params = crate::category_writes::permit(&p, &s)?;
    let mut old_params = params.clone();
    drop(conn);
    let host = crate::pretty_text::Host::from_state(&state);
    let cx = crate::category_writes::Cx {
        host: &host,
        bus: &state.bus,
        settings: &s,
    };
    let mut tx = state.pool.begin().await?;
    // move_to(category_params[:position].to_i) if category_params[:position]
    if let Some(position) = params.remove("position").filter(|p| !p.is_null()) {
        crate::category_writes::move_to(&mut tx, c.id, position_to_i(&position)).await?;
    }
    let old_custom_fields = crate::category_serializer::custom_fields(&mut tx, c.id).await?;
    let mut pending = params.custom_fields.take();
    match manage_category_types(&mut tx, &state, &cx, &guardian, &p, c.id).await? {
        Err(message) => {
            // Rendered from inside json_result: the move stays.
            tx.commit().await?;
            return Ok(unprocessable(vec![message]));
        }
        Ok(dropped) => {
            // The changed types' own fields leave the pending custom fields
            // (the hash category_params and its copy share).
            for key in dropped {
                if let Some(fields) = pending.as_mut() {
                    fields.remove(key);
                }
                if let Some(fields) = old_params.custom_fields.as_mut() {
                    fields.remove(key);
                }
            }
        }
    }
    // cat.reload
    let mut record = crate::category_writes::Record::load(&mut tx, c.id)
        .await?
        .ok_or(Unsupported("a category that vanished during its update"))?;
    if let Some(pending) = &pending {
        let merged = crate::category_writes::merge_pending_custom_fields(
            record.custom_fields_loaded(),
            pending,
        );
        record.set_custom_fields(merged);
    }
    let old_permissions = {
        let perms = crate::category_writes::permissions_params(&mut tx, c.id).await?;
        if perms.is_empty() {
            vec![("everyone".to_string(), 1)]
        } else {
            perms
        }
    };
    if let Err(rejected) = record.assign(&mut tx, &params).await? {
        return Err(match rejected {
            // An ArgumentError json_result doesn't rescue: Rails' 500.
            crate::category_writes::Rejected::Argument(_) => {
                Unsupported("an invalid enum value in a category update (a 500 in Rails)").into()
            }
            crate::category_writes::Rejected::Invalid(_) => {
                Unsupported("an invalid category assignment").into()
            }
        });
    }
    match crate::category_writes::save(&mut tx, &cx, &mut record).await? {
        crate::category_writes::Saved::Invalid(errors) => {
            // The move (and nothing else) was written outside the save.
            tx.commit().await?;
            return Ok(unprocessable(errors));
        }
        crate::category_writes::Saved::Done(_) => {}
    }
    crate::category_writes::after_commit(
        &mut tx,
        &cx,
        c.id,
        "category_updated",
        Some(record.saved_changes.as_slice()),
        true,
    )
    .await?;
    record.run_deferred(&mut tx).await?;
    let mut pre = cs::preload_user_fields(&mut tx, &s, &state.i18n, &guardian, c.id).await?;
    pre.category_setting = true;
    crate::category_writes::log_settings_change(
        &mut tx,
        &cx,
        user_id,
        c.id,
        &old_params,
        &record.saved_changes,
        &old_permissions,
        &old_custom_fields,
    )
    .await?;
    tx.commit().await?;
    let mut conn = state.pool.acquire().await?;
    // The settings as the type configuration left them.
    let s = SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let json = cs::serialize(&mut conn, &cs::Cx::of(&state), &s, &guardian, c.id, &pre).await?;
    Ok(Json(json!({ "success": "OK", "category": json })).into_response())
}

/// PUT /category/:category_id/slug(.json): categories#update_slug.
pub async fn update_slug(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    axum::extract::Path(id): axum::extract::Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = crate::params::parse(uri.query(), &headers, &body);
    if !super::session::csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "PUT") {
        return Ok(super::session::bad_csrf());
    }
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    let mut conn = state.pool.acquire().await?;
    let s = SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    // Category.find(params[:category_id].to_i)
    let found = match i32::try_from(crate::ruby::to_i(&id)) {
        Ok(id) => GuardedCategory::load(&mut conn, id).await?,
        Err(_) => None,
    };
    let Some(c) = found else {
        return Ok(not_found(&state));
    };
    if !cs::can_edit(&mut conn, &s, &guardian, &c).await? {
        return Ok(super::search::invalid_access(&state));
    }
    let slug = p
        .get("slug")
        .and_then(crate::params::scalar)
        .unwrap_or_default();
    if crate::ruby::is_blank(&slug) {
        let message = format!(
            "Slug {}",
            state
                .i18n
                .t("errors.messages.blank")
                .unwrap_or("can't be blank")
        );
        return Ok(unprocessable(vec![message]));
    }
    drop(conn);
    let host = crate::pretty_text::Host::from_state(&state);
    let cx = crate::category_writes::Cx {
        host: &host,
        bus: &state.bus,
        settings: &s,
    };
    let mut tx = state.pool.begin().await?;
    let mut record = crate::category_writes::Record::load(&mut tx, c.id)
        .await?
        .ok_or(Unsupported("a category that vanished during its update"))?;
    let params = crate::category_writes::Params {
        attrs: vec![("slug", json!(slug))],
        ..Default::default()
    };
    if record.assign(&mut tx, &params).await?.is_err() {
        return Err(Unsupported("an invalid category assignment").into());
    }
    match crate::category_writes::save(&mut tx, &cx, &mut record).await? {
        crate::category_writes::Saved::Invalid(errors) => {
            tx.rollback().await?;
            Ok(unprocessable(errors))
        }
        crate::category_writes::Saved::Done(_) => {
            crate::category_writes::after_commit(
                &mut tx,
                &cx,
                c.id,
                "category_updated",
                Some(record.saved_changes.as_slice()),
                true,
            )
            .await?;
            record.run_deferred(&mut tx).await?;
            tx.commit().await?;
            Ok(Json(json!({ "success": "OK" })).into_response())
        }
    }
}

/// DELETE /categories/:id(.json): categories#destroy.
pub async fn destroy(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    axum::extract::Path(id): axum::extract::Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = crate::params::parse(uri.query(), &headers, &body);
    if !super::session::csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "DELETE") {
        return Ok(super::session::bad_csrf());
    }
    let Some(user_id) = guardian.user_id() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    let mut conn = state.pool.acquire().await?;
    let s = SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let Some(c) = fetch_category(&mut conn, &s, id_param(&id)).await? else {
        return Ok(not_found(&state));
    };
    if !cs::can_delete(&mut conn, &s, &guardian, &c).await? {
        return Ok(super::search::invalid_access(&state));
    }
    drop(conn);
    let host = crate::pretty_text::Host::from_state(&state);
    let cx = crate::category_writes::Cx {
        host: &host,
        bus: &state.bus,
        settings: &s,
    };
    let mut tx = state.pool.begin().await?;
    let deleted = crate::category_writes::Deleted::read(&mut tx, &cx, c.id).await?;
    crate::category_writes::destroy(&mut tx, &cx, c.id).await?;
    deleted.log(&mut tx, user_id, c.id).await?;
    tx.commit().await?;
    Ok(Json(json!({ "success": "OK" })).into_response())
}
