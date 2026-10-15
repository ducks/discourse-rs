//! TagsController's actions besides the lists: a tag's info, settings and
//! notification level, renaming and redescribing it (update and
//! update_settings), destroying it, its synonyms, and /tags/list,
//! /tags/personal_messages/:username, /tags/upload, /tags/bulk_create and
//! /tags/unused.
//!
//! Refused: content localization (tag localizations in the settings) and
//! CSV rows Rails can't take (an empty row or a blank name: a 500 there).

use axum::Json;
use axum::body::Bytes;
use axum::extract::{FromRequest, Path, Request, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::guardian::Guardian;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::tags::Tag;
use crate::web_hooks::Payloads;
use crate::{AppError, AppState, Unsupported};

fn not_found(state: &AppState) -> Response {
    super::topics::not_found_response(state)
}

fn forbidden(state: &AppState) -> Response {
    super::search::invalid_access(state)
}

fn json_errors(status: StatusCode, errors: Vec<String>) -> Response {
    (status, Json(json!({ "errors": errors }))).into_response()
}

fn success() -> Response {
    Json(json!({"success": "OK"})).into_response()
}

fn failed(status: StatusCode) -> Response {
    (status, Json(json!({"failed": "FAILED"}))).into_response()
}

fn form_pairs(p: &Map<String, Value>) -> Vec<(String, String)> {
    p.iter()
        .filter_map(|(k, v)| crate::params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

/// How a `/tag/...` path names the tag: `:tag_id` (digits) or
/// `:tag_name`.
fn tag_ref(segment: &str) -> &str {
    segment.strip_suffix(".json").unwrap_or(segment)
}

/// `fetch_tag`: by id (then, for a numeric name, by name) or by name;
/// None when there is none or the guardian can't see it.
async fn fetch_tag(
    conn: &mut PgConnection,
    guardian: &Guardian,
    reference: &str,
) -> Result<Option<Tag>, sqlx::Error> {
    let numeric = !reference.is_empty() && reference.bytes().all(|b| b.is_ascii_digit());
    let mut tag = None;
    if numeric && let Ok(id) = reference.parse::<i32>() {
        tag = Tag::find(&mut *conn, id).await?;
    }
    if tag.is_none() {
        tag = Tag::find_by_name(&mut *conn, reference).await?;
    }
    match tag {
        Some(t) if t.visible_to(&mut *conn, guardian).await? => Ok(Some(t)),
        _ => Ok(None),
    }
}

/// `can_edit_tag?`: edit_tags_allowed_groups (the tag seen already).
fn can_edit_tags(guardian: &Guardian, settings: &SiteSettings) -> Result<bool, AppError> {
    Ok(settings.get("tagging_enabled")?.truthy()
        && guardian.user_id().is_some()
        && guardian.in_setting_groups(settings, "edit_tags_allowed_groups")?)
}

/// `can_admin_tags?`
fn can_admin_tags(guardian: &Guardian, settings: &SiteSettings) -> Result<bool, AppError> {
    Ok(guardian.is_staff() && settings.get("tagging_enabled")?.truthy())
}

async fn settings(state: &AppState, conn: &mut PgConnection) -> Result<SiteSettings, AppError> {
    Ok(SiteSettings::load(conn, &state.site_setting_defs, &state.config.globals).await?)
}

fn payloads<'a>(state: &'a AppState, settings: &'a SiteSettings) -> Payloads<'a> {
    Payloads {
        settings,
        i18n: &state.i18n,
        config: &state.config,
    }
}

/// The tag's sub-route under `/tag/` and what it names: `(tag, action,
/// synonym id)`. None for paths that aren't these routes.
fn split(path: &str) -> Option<(String, String, Option<String>)> {
    let path = path.strip_suffix(".json").unwrap_or(path);
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    match segments.as_slice() {
        [tag] => Some((tag.to_string(), String::new(), None)),
        [tag, action] => Some((tag.to_string(), action.to_string(), None)),
        [tag, "synonyms", id] => Some((tag.to_string(), "synonyms".into(), Some(id.to_string()))),
        _ => None,
    }
}

/// GET /tag/:tag/info, /tag/:tag/notifications and /tag/:tag/settings
/// (tags#show takes the rest). None for other paths.
pub async fn get_sub(
    state: &AppState,
    guardian: &Guardian,
    path: &str,
) -> Result<Option<Response>, AppError> {
    let Some((tag, action, None)) = split(path) else {
        return Ok(None);
    };
    if !matches!(action.as_str(), "info" | "notifications" | "settings") {
        return Ok(None);
    }
    let mut conn = state.pool.acquire().await?;
    let s = settings(state, &mut conn).await?;
    if !s.get("tagging_enabled")?.truthy() {
        return Ok(Some(not_found(state)));
    }
    match action.as_str() {
        "settings" => {
            if guardian.is_anonymous() {
                return Ok(Some(super::login_required::not_logged_in(state)));
            }
            let id = tag.parse::<i32>().ok();
            let found = match id {
                Some(id) => Tag::find(&mut conn, id).await?,
                None => None,
            };
            let Some(t) = found else {
                return Ok(Some(not_found(state)));
            };
            if !(can_edit_tags(guardian, &s)? && t.visible_to(&mut conn, guardian).await?) {
                return Ok(Some(not_found(state)));
            }
            Ok(Some(
                Json(settings_json(&mut conn, state, &s, guardian, &t).await?).into_response(),
            ))
        }
        "info" => {
            let Some(t) = fetch_tag(&mut conn, guardian, tag_ref(&tag)).await? else {
                return Ok(Some(not_found(state)));
            };
            Ok(Some(
                Json(info_json(&mut conn, state, &s, guardian, &t).await?).into_response(),
            ))
        }
        _ => {
            if guardian.is_anonymous() {
                return Ok(Some(super::login_required::not_logged_in(state)));
            }
            let Some(t) = fetch_tag(&mut conn, guardian, tag_ref(&tag)).await? else {
                return Ok(Some(not_found(state)));
            };
            let level: Option<i32> = sqlx::query_scalar(
                "SELECT notification_level FROM tag_users WHERE tag_id = $1 AND user_id = $2 ORDER BY id LIMIT 1",
            )
            .bind(t.id)
            .bind(guardian.user_id())
            .fetch_optional(&mut *conn)
            .await?;
            Ok(Some(
                Json(json!({"tag_notification": {
                    "id": t.id,
                    "name": t.name,
                    "notification_level": level.unwrap_or(1),
                }}))
                .into_response(),
            ))
        }
    }
}

/// PUT /tag/{*path}: update, update_notifications, update_settings.
pub async fn put(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(path): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = crate::params::parse(uri.query(), &headers, &body);
    if !super::session::csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "PUT") {
        return Ok(super::session::bad_csrf());
    }
    let Some((tag, action, None)) = split(&path) else {
        return Ok(not_found(&state));
    };
    let mut conn = state.pool.acquire().await?;
    let s = settings(&state, &mut conn).await?;
    if !s.get("tagging_enabled")?.truthy() {
        return Ok(not_found(&state));
    }
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    let user_id = guardian
        .user_id()
        .ok_or(Unsupported("anonymous tag writes"))?;
    match action.as_str() {
        "" => {
            let Some(t) = fetch_tag(&mut conn, &guardian, tag_ref(&tag)).await? else {
                return Ok(not_found(&state));
            };
            let Some(Value::Object(params)) = p.get("tag") else {
                return Err(Unsupported(
                    "tags#update without a tag hash (a NoMethodError in Rails)",
                )
                .into());
            };
            if params.get("id").is_some_and(|v| !v.is_null()) {
                return Ok(json_errors(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    vec![
                        "Updating a tag name by `id` attribute is unsupported. Use the `name` attribute instead."
                            .to_string(),
                    ],
                ));
            }
            if !can_edit_tags(&guardian, &s)? {
                return Ok(forbidden(&state));
            }
            let name = params
                .get("name")
                .and_then(crate::params::scalar)
                .filter(|n| !crate::ruby::is_blank(n));
            let changes = crate::tag_writes::TagChanges {
                name: match name {
                    Some(n) => Some(crate::tagging::clean_tag(&s, &n)?),
                    None => None,
                },
                slug: None,
                description: params
                    .contains_key("description")
                    .then(|| params.get("description").and_then(crate::params::scalar)),
            };
            let mut tx = state.pool.begin().await?;
            let host = crate::pretty_text::Host::from_state(&state);
            match crate::tag_writes::save(&mut tx, &payloads(&state, &s), &host, t.id, &changes)
                .await?
            {
                crate::tag_writes::Saved::Invalid(errors) => {
                    Ok(json_errors(StatusCode::UNPROCESSABLE_ENTITY, errors))
                }
                crate::tag_writes::Saved::Saved { renamed_from } => {
                    let (name, slug, description): (String, String, Option<String>) =
                        sqlx::query_as("SELECT name, slug, description FROM tags WHERE id = $1")
                            .bind(t.id)
                            .fetch_one(&mut *tx)
                            .await?;
                    if let Some(from) = renamed_from {
                        crate::staff_action_logs::log_custom(
                            &mut tx,
                            user_id,
                            "renamed_tag",
                            &[],
                            Some(&from),
                            Some(&name),
                        )
                        .await?;
                    }
                    tx.commit().await?;
                    Ok(Json(json!({"tag": {
                        "id": t.id, "name": name, "slug": slug, "description": description,
                    }}))
                    .into_response())
                }
            }
        }
        "notifications" => {
            let Some(t) = fetch_tag(&mut conn, &guardian, tag_ref(&tag)).await? else {
                return Ok(not_found(&state));
            };
            let level = p
                .get("tag_notification")
                .and_then(|n| n.get("notification_level"))
                .and_then(crate::params::scalar)
                .map(|l| crate::ruby::to_i(&l) as i32)
                .ok_or(Unsupported(
                    "tag notifications without a level (a NoMethodError in Rails)",
                ))?;
            let mut tx = state.pool.begin().await?;
            crate::tag_writes::change_tag_user(&mut tx, user_id, t.id, level).await?;
            let body = user_tag_notifications(&mut tx, user_id).await?;
            tx.commit().await?;
            Ok(Json(body).into_response())
        }
        "settings" => {
            let found = match tag.parse::<i32>() {
                Ok(id) => Tag::find(&mut conn, id).await?,
                Err(_) => None,
            };
            let Some(t) = found else {
                return Ok(not_found(&state));
            };
            if !(can_edit_tags(&guardian, &s)? && t.visible_to(&mut conn, &guardian).await?) {
                return Ok(not_found(&state));
            }
            let Some(Value::Object(params)) = p.get("tag_settings") else {
                return Err(
                    Unsupported("tag settings without tag_settings (ParameterMissing)").into(),
                );
            };
            if params.contains_key("localizations") || params.contains_key("locale") {
                return Err(Unsupported("tag localizations (content localization)").into());
            }
            let name = params
                .get("name")
                .and_then(crate::params::scalar)
                .filter(|n| !crate::ruby::is_blank(n));
            let changes = crate::tag_writes::TagChanges {
                name: match name {
                    Some(n) => Some(crate::tagging::clean_tag(&s, &n)?),
                    None => None,
                },
                slug: params.contains_key("slug").then(|| {
                    params
                        .get("slug")
                        .and_then(crate::params::scalar)
                        .unwrap_or_default()
                }),
                description: params
                    .contains_key("description")
                    .then(|| params.get("description").and_then(crate::params::scalar)),
            };
            let mut tx = state.pool.begin().await?;
            let host = crate::pretty_text::Host::from_state(&state);
            match crate::tag_writes::save(&mut tx, &payloads(&state, &s), &host, t.id, &changes)
                .await?
            {
                crate::tag_writes::Saved::Invalid(errors) => {
                    return Ok(json_errors(StatusCode::UNPROCESSABLE_ENTITY, errors));
                }
                crate::tag_writes::Saved::Saved { renamed_from } => {
                    if let Some(from) = renamed_from {
                        let name: String =
                            sqlx::query_scalar("SELECT name FROM tags WHERE id = $1")
                                .bind(t.id)
                                .fetch_one(&mut *tx)
                                .await?;
                        crate::staff_action_logs::log_custom(
                            &mut tx,
                            user_id,
                            "renamed_tag",
                            &[],
                            Some(&from),
                            Some(&name),
                        )
                        .await?;
                    }
                }
            }
            // remove_synonyms: the editable ones of the tag's synonyms named.
            let removed = int_list(params.get("removed_synonym_ids"));
            if !removed.is_empty() && can_edit_tags(&guardian, &s)? {
                let candidates: Vec<i32> = sqlx::query_scalar(
                    "SELECT id FROM tags WHERE id = ANY($1) AND target_tag_id = $2 ORDER BY ctid",
                )
                .bind(&removed)
                .bind(t.id)
                .fetch_all(&mut *tx)
                .await?;
                let mut editable = Vec::new();
                for id in candidates {
                    if let Some(syn) = Tag::find(&mut tx, id).await?
                        && syn.visible_to(&mut tx, &guardian).await?
                    {
                        editable.push(id);
                    }
                }
                sqlx::query("UPDATE tags SET target_tag_id = NULL WHERE id = ANY($1) AND target_tag_id = $2")
                    .bind(&editable)
                    .bind(t.id)
                    .execute(&mut *tx)
                    .await?;
            }
            // add_synonyms: by id, the editable ones.
            let new_ids: Vec<i32> = match params.get("new_synonyms") {
                Some(Value::Array(items)) => items
                    .iter()
                    .filter_map(|i| i.get("id").and_then(crate::params::scalar))
                    .map(|i| crate::ruby::to_i(&i) as i32)
                    .collect(),
                _ => Vec::new(),
            };
            if !new_ids.is_empty() && can_edit_tags(&guardian, &s)? {
                let mut editable = Vec::new();
                let rows: Vec<i32> =
                    sqlx::query_scalar("SELECT id FROM tags WHERE id = ANY($1) ORDER BY ctid")
                        .bind(&new_ids)
                        .fetch_all(&mut *tx)
                        .await?;
                for id in rows {
                    if let Some(syn) = Tag::find(&mut tx, id).await?
                        && syn.visible_to(&mut tx, &guardian).await?
                    {
                        editable.push(id);
                    }
                }
                if !editable.is_empty() {
                    crate::tag_writes::add_or_create_synonyms(
                        &mut tx,
                        &s,
                        &state.i18n,
                        t.id,
                        &editable,
                        &[],
                    )
                    .await?;
                }
            }
            let updated = Tag::find(&mut tx, t.id)
                .await?
                .ok_or(Unsupported("a tag that vanished"))?;
            let body = settings_json(&mut tx, &state, &s, &guardian, &updated).await?;
            tx.commit().await?;
            Ok(Json(body).into_response())
        }
        _ => Ok(not_found(&state)),
    }
}

/// A list of ids from the params (`[1, "2"]`).
fn int_list(value: Option<&Value>) -> Vec<i32> {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(crate::params::scalar)
            .map(|i| crate::ruby::to_i(&i) as i32)
            .collect(),
        _ => Vec::new(),
    }
}

/// DELETE /tag/{*path}: destroy and destroy_synonym.
pub async fn delete(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(path): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = crate::params::parse(uri.query(), &headers, &body);
    if !super::session::csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "DELETE") {
        return Ok(super::session::bad_csrf());
    }
    let Some((tag, action, synonym)) = split(&path) else {
        return Ok(not_found(&state));
    };
    let mut conn = state.pool.acquire().await?;
    let s = settings(&state, &mut conn).await?;
    if !s.get("tagging_enabled")?.truthy() {
        return Ok(not_found(&state));
    }
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    let user_id = guardian
        .user_id()
        .ok_or(Unsupported("anonymous tag writes"))?;
    let Some(t) = fetch_tag(&mut conn, &guardian, tag_ref(&tag)).await? else {
        return Ok(not_found(&state));
    };
    match (action.as_str(), synonym) {
        ("", None) => {
            if !can_admin_tags(&guardian, &s)? {
                return Ok(forbidden(&state));
            }
            let mut tx = state.pool.begin().await?;
            crate::tag_writes::destroy(&mut tx, &payloads(&state, &s), t.id).await?;
            crate::tag_writes::log_custom(&mut tx, user_id, "deleted_tag", Some(&t.name), "")
                .await?;
            tx.commit().await?;
            Ok(success())
        }
        ("synonyms", Some(id)) => {
            let synonym = match tag_ref(&id).parse::<i32>() {
                Ok(id) => Tag::find(&mut conn, id).await?,
                Err(_) => None,
            };
            let Some(syn) = synonym else {
                return Ok(not_found(&state));
            };
            if !(can_edit_tags(&guardian, &s)? && syn.visible_to(&mut conn, &guardian).await?) {
                return Ok(forbidden(&state));
            }
            if syn.target_tag_id != Some(t.id) {
                return Ok(failed(StatusCode::BAD_REQUEST));
            }
            let mut tx = state.pool.begin().await?;
            sqlx::query(
                "UPDATE tags SET target_tag_id = NULL, updated_at = clock_timestamp() WHERE id = $1",
            )
            .bind(syn.id)
            .execute(&mut *tx)
            .await?;
            crate::tag_writes::updated_hooks(&mut tx, &payloads(&state, &s), syn.id).await?;
            tx.commit().await?;
            Ok(success())
        }
        _ => Ok(not_found(&state)),
    }
}

/// POST /tag/:tag/synonyms: create_synonyms.
pub async fn post(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(path): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = crate::params::parse(uri.query(), &headers, &body);
    if !super::session::csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "POST") {
        return Ok(super::session::bad_csrf());
    }
    let Some((tag, action, None)) = split(&path) else {
        return Ok(not_found(&state));
    };
    if action != "synonyms" {
        return Ok(not_found(&state));
    }
    let mut conn = state.pool.acquire().await?;
    let s = settings(&state, &mut conn).await?;
    if !s.get("tagging_enabled")?.truthy() {
        return Ok(not_found(&state));
    }
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    let Some(t) = fetch_tag(&mut conn, &guardian, tag_ref(&tag)).await? else {
        return Ok(not_found(&state));
    };
    if !can_edit_tags(&guardian, &s)? {
        return Ok(forbidden(&state));
    }
    // params[:tags].try(:values) || params[:tags]
    let tags: Vec<Value> = match p.get("tags") {
        Some(Value::Array(items)) => items.clone(),
        Some(Value::Object(map)) => map.values().cloned().collect(),
        _ => Vec::new(),
    };
    let mut ids: Vec<i32> = Vec::new();
    let mut new_names: Vec<String> = Vec::new();
    for item in &tags {
        let id = item.get("id").and_then(crate::params::scalar);
        match id.as_deref().filter(|i| !crate::ruby::is_blank(i)) {
            Some(i) => ids.push(crate::ruby::to_i(i) as i32),
            None => {
                if let Some(name) = item.get("name").and_then(crate::params::scalar) {
                    new_names.push(name);
                }
            }
        }
    }
    if tags.is_empty()
        && let Some(Value::Array(names)) = p.get("synonyms")
        && !names.is_empty()
    {
        new_names = names.iter().filter_map(crate::params::scalar).collect();
    }
    let mut tx = state.pool.begin().await?;
    let system = crate::web_hooks::system_guardian(&mut tx).await?;
    let mut new_names =
        crate::tagging::tags_for_saving(&mut tx, &s, &system, &new_names, false).await?;
    let lower: Vec<String> = new_names.iter().map(|n| n.to_lowercase()).collect();
    let synonyms: Vec<i32> = sqlx::query_scalar(
        "SELECT id FROM tags WHERE id = ANY($1) OR lower(name) = ANY($2) ORDER BY ctid",
    )
    .bind(&ids)
    .bind(&lower)
    .fetch_all(&mut *tx)
    .await?;
    let mut editable = Vec::new();
    let mut synonym_names = Vec::new();
    for id in synonyms {
        if let Some(syn) = Tag::find(&mut tx, id).await? {
            if syn.visible_to(&mut tx, &guardian).await? {
                editable.push(id);
            }
            synonym_names.push(syn.name);
        }
    }
    new_names.retain(|n| !synonym_names.contains(n));
    let result = crate::tag_writes::add_or_create_synonyms(
        &mut tx,
        &s,
        &state.i18n,
        t.id,
        &editable,
        &new_names,
    )
    .await?;
    tx.commit().await?;
    Ok(match result {
        Some(failed) => {
            let mut tags = Map::new();
            for (name, why) in failed {
                tags.insert(name, json!(why));
            }
            Json(json!({"failed": "FAILED", "failed_tags": tags})).into_response()
        }
        None => success(),
    })
}

/// UserTagNotificationsSerializer: the user's tags by level, as
/// TagUser.notification_levels_for lists them.
async fn user_tag_notifications(conn: &mut PgConnection, user_id: i32) -> Result<Value, AppError> {
    let rows: Vec<(i32, String, String, i32)> = sqlx::query_as(
        "SELECT DISTINCT ON (tag_users.id) tags.id, tags.name, tags.slug, tag_users.notification_level \
         FROM tag_users JOIN tags ON tags.id = tag_users.tag_id \
         LEFT OUTER JOIN tag_group_memberships ON tag_users.tag_id = tag_group_memberships.tag_id \
         LEFT OUTER JOIN tag_group_permissions ON tag_group_memberships.tag_group_id = tag_group_permissions.tag_group_id \
         LEFT OUTER JOIN group_users ON group_users.user_id = tag_users.user_id \
         WHERE tag_users.user_id = $1 \
           AND (tag_group_permissions.group_id IS NULL \
                OR tag_group_permissions.group_id IN (0, group_users.group_id) \
                OR group_users.group_id = 3) \
         ORDER BY tag_users.id",
    )
    .bind(user_id)
    .fetch_all(&mut *conn)
    .await?;
    let level = |l: i32| -> Value {
        Value::Array(
            rows.iter()
                .filter(|r| r.3 == l)
                .map(|(id, name, slug, _)| json!({"id": id, "name": name, "slug": slug}))
                .collect(),
        )
    };
    Ok(json!({
        "watched_tags": level(3),
        "watching_first_post_tags": level(4),
        "tracked_tags": level(2),
        "muted_tags": level(0),
        "regular_tags": level(1),
    }))
}

/// `Tag#all_category_ids`: the categories the tag is restricted to,
/// directly, then through its tag groups.
async fn all_category_ids(conn: &mut PgConnection, tag_id: i32) -> Result<Vec<i32>, sqlx::Error> {
    let mut ids: Vec<i32> =
        sqlx::query_scalar("SELECT category_id FROM category_tags WHERE tag_id = $1 ORDER BY ctid")
            .bind(tag_id)
            .fetch_all(&mut *conn)
            .await?;
    ids.extend(
        sqlx::query_scalar::<_, i32>(
            "SELECT ctg.category_id FROM tag_group_memberships tgm \
             JOIN category_tag_groups ctg ON ctg.tag_group_id = tgm.tag_group_id \
             WHERE tgm.tag_id = $1 ORDER BY tgm.ctid, ctg.ctid",
        )
        .bind(tag_id)
        .fetch_all(&mut *conn)
        .await?,
    );
    Ok(ids)
}

/// `Tag#all_categories(guardian)` with `Category.preload_user_fields!`,
/// as BasicCategorySerializer side-loads them; and their ids.
async fn categories_side(
    conn: &mut PgConnection,
    state: &AppState,
    s: &SiteSettings,
    guardian: &Guardian,
    tag_id: i32,
) -> Result<(Vec<Value>, Vec<i32>), AppError> {
    let wanted = all_category_ids(conn, tag_id).await?;
    let allowed = guardian.allowed_category_ids(&mut *conn, s).await?;
    let ids: Vec<i32> = sqlx::query_scalar(
        "SELECT id FROM categories WHERE id = ANY($1) AND id = ANY($2) ORDER BY ctid",
    )
    .bind(&wanted)
    .bind(&allowed)
    .fetch_all(&mut *conn)
    .await?;
    if ids.is_empty() {
        return Ok((Vec::new(), ids));
    }
    let rows = crate::categories::Categories::load_all(&mut *conn).await?;
    let base_path = state.config.globals.relative_url_root().to_string();
    let mut cats = crate::categories::Categories {
        conn: &mut *conn,
        settings: s,
        i18n: &state.i18n,
        guardian,
        base_path: &base_path,
        topic_url_via_slug: false,
    };
    let levels = cats.notification_levels().await?;
    let default_level = if s.get("mute_all_categories_by_default")?.truthy() {
        0
    } else {
        1
    };
    let can_edit = guardian.is_admin()
        || (s.get("moderators_manage_categories")?.truthy() && guardian.is_moderator());
    let create_allowed = if guardian.is_admin() || guardian.is_anonymous() {
        None
    } else {
        Some(
            guardian
                .topic_create_allowed_category_ids(&mut *cats.conn, s)
                .await?,
        )
    };
    let mut out = Vec::new();
    for id in &ids {
        let Some(row) = rows.iter().find(|r| r.id == *id) else {
            continue;
        };
        let fields = cats.basic_fields(row).await?;
        let children: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM categories WHERE parent_category_id = $1 AND id = ANY($2)",
        )
        .bind(id)
        .bind(&allowed)
        .fetch_one(&mut *cats.conn)
        .await?;
        let mut c = Map::new();
        for (k, v) in fields {
            match k.as_str() {
                "permission" => {
                    let full = guardian.is_admin()
                        || create_allowed.as_ref().is_some_and(|a| a.contains(id));
                    c.insert(k, if full { json!(1) } else { v });
                }
                "notification_level" => {
                    let level = levels
                        .iter()
                        .find(|(cid, _)| *cid == i64::from(*id))
                        .map(|(_, l)| *l)
                        .unwrap_or(default_level);
                    c.insert(k, json!(level));
                    if can_edit {
                        c.insert("can_edit".into(), json!(true));
                    }
                }
                "has_children" => {
                    c.insert(k, json!(children > 0));
                }
                "subcategory_count" => {
                    c.insert(
                        k,
                        if children > 0 {
                            json!(children)
                        } else {
                            Value::Null
                        },
                    );
                }
                _ => {
                    c.insert(k, v);
                }
            }
        }
        cats.uploads(&mut c, row).await?;
        out.push(Value::Object(c));
    }
    Ok((out, ids))
}

/// `TagGroup.visible(guardian)` names of the tag's groups (admins: all).
async fn visible_tag_groups(
    conn: &mut PgConnection,
    s: &SiteSettings,
    guardian: &Guardian,
    tag_id: i32,
) -> Result<Vec<(i32, String)>, AppError> {
    let mut sql = "SELECT tg.id, tg.name FROM tag_groups tg \
                   WHERE tg.id IN (SELECT tag_group_id FROM tag_group_memberships WHERE tag_id = $1)"
        .to_string();
    if !guardian.is_admin() {
        let allowed = guardian.allowed_category_ids_sql(s)?;
        let groups = crate::tagging::permitted_group_ids(guardian)
            .iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(",");
        sql.push_str(&format!(
            " AND (tg.id IN (SELECT tag_group_id FROM category_tag_groups WHERE category_id IN ({allowed})) \
                   OR tg.id NOT IN (SELECT tag_group_id FROM category_tag_groups)) \
              AND tg.id IN (SELECT tag_group_id FROM tag_group_permissions WHERE group_id IN ({groups}))"
        ));
    }
    sql.push_str(" ORDER BY tg.ctid");
    Ok(sqlx::query_as(&sql).bind(tag_id).fetch_all(conn).await?)
}

/// DetailedTagSerializer with the rest serializer's side-loads.
async fn info_json(
    conn: &mut PgConnection,
    state: &AppState,
    s: &SiteSettings,
    guardian: &Guardian,
    t: &Tag,
) -> Result<Value, AppError> {
    let (categories, category_ids) = categories_side(conn, state, s, guardian, t.id).await?;
    let mut info = match t.serialize(&mut *conn, guardian, s).await? {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    // synonyms: filter_visible(object.synonyms), as tag_counts_json lists them.
    let visible = crate::tags::visible_tags_where(guardian, s)?;
    let rows: Vec<crate::tagging::AllowedTag> = sqlx::query_as(&format!(
        "SELECT tags.id, tags.name, tags.slug, tags.{} AS topic_count, tags.pm_topic_count, tags.description, \
                NULL::int AS tgm_id, NULL::int AS tag_group_id, NULL::int AS parent_tag_id, \
                NULL::bool AS one_per_topic, tags.target_tag_id \
         FROM tags WHERE tags.target_tag_id = $1 AND {visible} ORDER BY tags.ctid",
        guardian.tag_count_column(s)?
    ))
    .bind(t.id)
    .fetch_all(&mut *conn)
    .await?;
    info.insert(
        "synonyms".into(),
        Value::Array(super::tag_search::tag_counts_json(conn, s, guardian, &rows).await?),
    );
    if guardian.is_admin() || s.get("tags_listed_by_group")?.truthy() {
        let groups = visible_tag_groups(conn, s, guardian, t.id).await?;
        info.insert(
            "tag_group_names".into(),
            json!(groups.iter().map(|(_, n)| n).collect::<Vec<_>>()),
        );
    }
    let restricted = !all_category_ids(conn, t.id).await?.is_empty();
    info.insert("category_restricted".into(), json!(restricted));
    info.insert("category_ids".into(), json!(category_ids));
    Ok(json!({
        "categories": categories,
        "tag_info": info,
        "__rest_serializer": "1",
    }))
}

/// TagSettingsSerializer with the rest serializer's side-loads.
async fn settings_json(
    conn: &mut PgConnection,
    state: &AppState,
    s: &SiteSettings,
    guardian: &Guardian,
    t: &Tag,
) -> Result<Value, AppError> {
    let (categories, category_ids) = categories_side(conn, state, s, guardian, t.id).await?;
    let (locale, description_cooked): (Option<String>, Option<String>) =
        sqlx::query_as("SELECT locale, description_cooked FROM tags WHERE id = $1")
            .bind(t.id)
            .fetch_one(&mut *conn)
            .await?;
    let synonym_rows: Vec<i32> =
        sqlx::query_scalar("SELECT id FROM tags WHERE target_tag_id = $1 ORDER BY ctid")
            .bind(t.id)
            .fetch_all(&mut *conn)
            .await?;
    let mut synonyms = Vec::new();
    for id in synonym_rows {
        if let Some(syn) = Tag::find(&mut *conn, id).await?
            && syn.visible_to(&mut *conn, guardian).await?
        {
            synonyms.push(json!({"id": syn.id, "name": syn.name}));
        }
    }
    let mut out = Map::new();
    out.insert("id".into(), json!(t.id));
    out.insert("name".into(), json!(t.name));
    out.insert("locale".into(), json!(locale));
    out.insert("slug".into(), json!(t.slug_for_url()));
    out.insert("description".into(), json!(t.description));
    out.insert("description_cooked".into(), json!(description_cooked));
    out.insert("synonyms".into(), Value::Array(synonyms));
    if guardian.is_admin() || s.get("tags_listed_by_group")?.truthy() {
        let groups = visible_tag_groups(conn, s, guardian, t.id).await?;
        out.insert(
            "tag_group_names".into(),
            json!(groups.iter().map(|(_, n)| n).collect::<Vec<_>>()),
        );
        out.insert(
            "tag_groups".into(),
            Value::Array(
                groups
                    .iter()
                    .map(|(id, n)| json!({"id": id, "name": n}))
                    .collect(),
            ),
        );
    }
    let restricted = !all_category_ids(conn, t.id).await?.is_empty();
    out.insert("category_restricted".into(), json!(restricted));
    out.insert(
        "can_edit".into(),
        json!(can_edit_tags(guardian, s)? && t.visible_to(&mut *conn, guardian).await?),
    );
    out.insert("can_admin".into(), json!(can_admin_tags(guardian, s)?));
    out.insert("category_ids".into(), json!(category_ids));
    Ok(json!({
        "categories": categories,
        "tag_settings": out,
        "__rest_serializer": "1",
    }))
}

/// What a tag's page shows around its list (tags/show): the edit button
/// for those who may edit tags, the info button, and the hidden TagInfo
/// panel it opens; for a member, their notification level in the tag.
pub struct TagPageControls {
    pub buttons: String,
    pub info: String,
    pub notifications: String,
}

pub async fn tag_page_controls(
    conn: &mut PgConnection,
    state: &AppState,
    s: &SiteSettings,
    guardian: &Guardian,
    t: &Tag,
) -> Result<TagPageControls, AppError> {
    use crate::post_view::d_icon;
    use crate::topic_list_view::escape;
    let base_path = state.config.globals.relative_url_root().to_string();
    let js = |key: &str| {
        state
            .i18n
            .t(&format!("js.{key}"))
            .unwrap_or_default()
            .to_string()
    };
    let button = |id: &str, icon: &str, label: &str, extra: &str| {
        format!(
            "<button aria-label=\"{l}\"{extra} class=\"btn no-text btn-icon btn-default\" id=\"{id}\" title=\"{l}\" type=\"button\">{}<span aria-hidden=\"true\">&#8203;</span></button>",
            d_icon(icon, None),
            l = escape(label)
        )
    };
    let mut buttons = String::new();
    if can_edit_tags(guardian, s)? {
        // tag-info.js opens the tag's settings page (/tag/:slug/:id/edit).
        buttons.push_str(&button(
            "edit-tag",
            "wrench",
            &js("tagging.edit"),
            &format!(" data-href=\"{}/edit\"", escape(&t.url(&base_path))),
        ));
    }
    buttons.push_str(&button(
        "show-tag-info",
        "circle-info",
        &js("tagging.info"),
        " aria-pressed=\"false\"",
    ));

    // TagInfo, from DetailedTagSerializer's data.
    let info = info_json(conn, state, s, guardian, t).await?;
    let tag_info = &info["tag_info"];
    let categories_map = crate::topic_list_view::categories(&mut *conn).await?;
    let list = crate::topic_list_view::ListContext {
        i18n: &state.i18n,
        base_path: &base_path,
        now: chrono::Utc::now(),
        categories: &categories_map,
        expand_all_pinned: false,
        member_trust_level: guardian.user().map(|u| u.trust_level),
        settings: crate::topic_list_view::ListSettings::load(s)?,
    };
    let style = s.get("tag_style")?.to_s();
    let tag_link = |name: &str, href: &str| {
        format!(
            "<a href=\"{}\" data-tag-name=\"{}\" class=\"discourse-tag {}\">{}</a>",
            escape(href),
            escape(&name.to_lowercase()),
            escape(&style),
            escape(name)
        )
    };
    let tag_list = |tags: &[(String, String)]| {
        let mut out = format!(
            "<ul class='discourse-tags' aria-label={}>",
            escape(&js("tagging.tags"))
        );
        for (i, (name, href)) in tags.iter().enumerate() {
            out.push_str("<li>");
            out.push_str(&tag_link(name, href));
            if i + 1 < tags.len() {
                out.push_str("<span class=\"discourse-tags__tag-separator\">,</span>");
            }
            out.push_str("</li>");
        }
        out.push_str("</ul>");
        out
    };
    let mut html = format!(
        "<section class=\"tag-info discovery-heading \" hidden><div class=\"tag-name\"><div class=\"tag-name-wrapper\">{}</div>",
        tag_link(&t.name, &t.url(&base_path))
    );
    if let Some(cooked) = tag_info["description_cooked"].as_str() {
        html.push_str(&format!(
            "<div class=\"tag-description-wrapper\"><div class=\"cooked\">{cooked}</div></div>"
        ));
    }
    html.push_str("</div><p class=\"tag-associations\">");
    let groups: Vec<String> = tag_info["tag_group_names"]
        .as_array()
        .map(|g| {
            g.iter()
                .filter_map(|n| n.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    if !groups.is_empty() {
        html.push_str(&format!(
            "<span>{}</span>",
            escape(
                &state
                    .i18n
                    .t_count(
                        "js.tagging.tag_groups_info",
                        groups.len() as i64,
                        &[("tag_groups", &groups.join(", "))],
                    )
                    .unwrap_or_default()
            )
        ));
    }
    let category_ids: Vec<i64> = tag_info["category_ids"]
        .as_array()
        .map(|c| c.iter().filter_map(Value::as_i64).collect())
        .unwrap_or_default();
    let restricted = tag_info["category_restricted"].as_bool() == Some(true);
    if !category_ids.is_empty() {
        let badges: Vec<String> = category_ids
            .iter()
            .filter_map(|id| categories_map.get(id))
            .map(|c| crate::topic_list_view::category_badge(&list, c))
            .collect();
        html.push_str(&format!(
            "<span>{}</span>",
            state
                .i18n
                .t_count(
                    "js.tagging.category_restrictions",
                    category_ids.len() as i64,
                    &[("categories", &badges.join(" "))],
                )
                .unwrap_or_default()
        ));
    } else if restricted {
        html.push_str(&format!(
            "<span>{}</span>",
            escape(&js("tagging.category_restricted"))
        ));
    }
    let synonyms: Vec<(String, String)> = tag_info["synonyms"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|syn| {
                    let name = syn["name"].as_str()?.to_string();
                    let href = format!(
                        "{base_path}/tag/{}/{}",
                        syn["slug"].as_str().unwrap_or_default(),
                        syn["id"].as_i64().unwrap_or_default()
                    );
                    Some((name, href))
                })
                .collect()
        })
        .unwrap_or_default();
    if groups.is_empty() && category_ids.is_empty() && synonyms.is_empty() && !restricted {
        html.push_str(&js("tagging.default_info"));
        if guardian.is_staff() {
            html.push_str(
                &state
                    .i18n
                    .t_with("js.tagging.staff_info", &[("basePath", &base_path)])
                    .unwrap_or_default(),
            );
        }
    }
    html.push_str("</p>");
    if !synonyms.is_empty() {
        let base = tag_list(&[(t.name.clone(), t.url(&base_path))]);
        html.push_str(&format!(
            "<div class=\"synonyms-list\">{}</div>",
            state
                .i18n
                .t_count(
                    "js.tagging.synonyms_description",
                    synonyms.len() as i64,
                    &[("base_tag_name", &base), ("synonyms", &tag_list(&synonyms))],
                )
                .unwrap_or_default()
        ));
    }
    html.push_str("</section>");

    let notifications = match guardian.user_id() {
        Some(user_id) => {
            let level: Option<i32> = sqlx::query_scalar(
                "SELECT notification_level FROM tag_users WHERE tag_id = $1 AND user_id = $2 ORDER BY id LIMIT 1",
            )
            .bind(t.id)
            .bind(user_id)
            .fetch_optional(&mut *conn)
            .await?;
            crate::html::tag_notifications(
                &state.i18n,
                &base_path,
                t.id,
                i64::from(level.unwrap_or(1)),
            )
        }
        None => String::new(),
    };
    Ok(TagPageControls {
        buttons,
        info: html,
        notifications,
    })
}

/// GET /tags/list(.json)
pub async fn list(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    uri: Uri,
) -> Result<Response, AppError> {
    let p = crate::params::parse_query(uri.query().unwrap_or(""));
    let mut conn = state.pool.acquire().await?;
    let s = settings(&state, &mut conn).await?;
    if !s.get("tagging_enabled")?.truthy() {
        return Ok(not_found(&state));
    }
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    if s.get("content_localization_enabled")?.truthy() {
        return Err(Unsupported("tag lists over localized tags").into());
    }
    let offset = crate::params::integer(&p, "offset").unwrap_or(0);
    let only = crate::params::string(&p, "only_tags").filter(|o| !o.is_empty());
    let filter = crate::params::string(&p, "filter");
    let exclude = crate::params::string(&p, "exclude_tags");
    let visible = crate::tags::visible_tags_where(&guardian, &s)?;
    let count_column = guardian.tag_count_column(&s)?;
    let mut wheres = vec![visible];
    if only.is_none() {
        wheres.push("tags.target_tag_id IS NULL".into());
    }
    // without_pm_only_tags, unless show_all_tags? (staff admins).
    let show_all = can_admin_tags(&guardian, &s)? && guardian.is_admin();
    if !show_all && !guardian.can_tag_pms(&s)? {
        wheres.push(format!(
            "NOT (tags.pm_topic_count > 0 AND tags.{count_column} = 0)"
        ));
    }
    let mut query = Vec::new();
    query.push(("offset".to_string(), (offset + 1).to_string()));
    if let Some(f) = &filter {
        wheres.push("LOWER(tags.name) ILIKE $1".into());
        query.push(("filter".into(), f.clone()));
    }
    if let Some(o) = &only {
        wheres.push("LOWER(tags.name) = ANY($2)".into());
        query.push(("only_tags".into(), o.clone()));
    }
    if let Some(e) = &exclude {
        wheres.push("NOT (LOWER(tags.name) = ANY($3))".into());
        query.push(("exclude_tags".into(), e.clone()));
    }
    let where_sql = wheres
        .iter()
        .map(|w| format!("({w})"))
        .collect::<Vec<_>>()
        .join(" AND ");
    let pattern = format!("%{}%", filter.as_deref().unwrap_or("").to_lowercase());
    let split = |v: &Option<String>| -> Vec<String> {
        v.as_deref()
            .map(|s| s.split(',').map(str::to_lowercase).collect())
            .unwrap_or_default()
    };
    let total: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM tags WHERE {where_sql}"))
        .bind(&pattern)
        .bind(split(&only))
        .bind(split(&exclude))
        .fetch_one(&mut *conn)
        .await?;
    const LIST_LIMIT: i64 = 51;
    let ids: Vec<i32> = sqlx::query_scalar(&format!(
        "SELECT id FROM tags WHERE {where_sql} ORDER BY LOWER(tags.name) ASC LIMIT {LIST_LIMIT} OFFSET $4"
    ))
    .bind(&pattern)
    .bind(split(&only))
    .bind(split(&exclude))
    .bind(offset * LIST_LIMIT)
    .fetch_all(&mut *conn)
    .await?;
    let mut tags = Vec::new();
    for id in ids {
        if let Some(t) = Tag::find(&mut conn, id).await? {
            tags.push(t.serialize(&mut conn, &guardian, &s).await?);
        }
    }
    let encoded: String = form_urlencoded::Serializer::new(String::new())
        .extend_pairs(query.iter())
        .finish();
    Ok(Json(json!({
        "list_tags": tags,
        "meta": {
            "total_rows_list_tags": total,
            "load_more_list_tags": format!("/tags/list.json?{encoded}"),
        },
    }))
    .into_response())
}

/// GET /tags/personal_messages/:username
pub async fn personal_messages(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(username): Path<String>,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let s = settings(&state, &mut conn).await?;
    if !s.get("tagging_enabled")?.truthy() {
        return Ok(not_found(&state));
    }
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    if !guardian.can_tag_pms(&s)? {
        return Ok(forbidden(&state));
    }
    let Some(user) = crate::user_updater::find_target(&mut conn, &guardian, &username).await?
    else {
        return Ok(not_found(&state));
    };
    if guardian.user_id() != Some(user.id) && !guardian.is_admin() {
        return Ok(not_found(&state));
    }
    let rows: Vec<(i32, String, i64)> = sqlx::query_as(
        "SELECT tags.id, tags.name, COUNT(topics.id) AS count FROM tags \
         JOIN topic_tags ON tags.id = topic_tags.tag_id \
         JOIN topics ON topics.id = topic_tags.topic_id AND topics.deleted_at IS NULL \
           AND topics.archetype = 'private_message' \
         WHERE topic_tags.topic_id IN ( \
           SELECT topic_id FROM topic_allowed_users WHERE user_id = $1 \
           UNION SELECT tg.topic_id FROM topic_allowed_groups tg \
           JOIN group_users gu ON gu.user_id = $1 AND gu.group_id = tg.group_id) \
         GROUP BY tags.id, tags.name ORDER BY count DESC LIMIT 1000",
    )
    .bind(user.id)
    .fetch_all(&mut *conn)
    .await?;
    Ok(Json(json!({
        "tags": rows.iter().map(|(id, name, count)| json!({"id": id, "name": name, "count": count})).collect::<Vec<_>>(),
    }))
    .into_response())
}

/// GET /tags/unused(.json): list_unused.
pub async fn unused(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let s = settings(&state, &mut conn).await?;
    if !s.get("tagging_enabled")?.truthy() {
        return Ok(not_found(&state));
    }
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    if !can_admin_tags(&guardian, &s)? {
        return Ok(forbidden(&state));
    }
    let tags: Vec<(i32, String)> = unused_tags(&mut conn).await?;
    Ok(Json(json!({"tags": tags.iter().map(|(_, n)| n).collect::<Vec<_>>()})).into_response())
}

/// `Tag.unused`: base tags without staff or message topics, outside tag
/// groups.
async fn unused_tags(conn: &mut PgConnection) -> Result<Vec<(i32, String)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT tags.id, tags.name FROM tags \
         LEFT JOIN tag_group_memberships tgm ON tags.id = tgm.tag_id \
         WHERE tags.target_tag_id IS NULL AND tags.staff_topic_count = 0 AND tags.pm_topic_count = 0 \
           AND tgm.tag_id IS NULL ORDER BY tags.ctid",
    )
    .fetch_all(conn)
    .await
}

/// `inspect` of a string, for a logged tag list.
fn inspect(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// DELETE /tags/unused(.json): destroy_unused, logged first.
pub async fn destroy_unused(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = crate::params::parse(uri.query(), &headers, &body);
    if !super::session::csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "DELETE") {
        return Ok(super::session::bad_csrf());
    }
    let mut conn = state.pool.acquire().await?;
    let s = settings(&state, &mut conn).await?;
    if !s.get("tagging_enabled")?.truthy() {
        return Ok(not_found(&state));
    }
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    if !can_admin_tags(&guardian, &s)? {
        return Ok(forbidden(&state));
    }
    let user_id = guardian
        .user_id()
        .ok_or(Unsupported("anonymous tag writes"))?;
    drop(conn);
    let mut tx = state.pool.begin().await?;
    let tags = unused_tags(&mut tx).await?;
    let list = tags
        .iter()
        .map(|(_, n)| inspect(n))
        .collect::<Vec<_>>()
        .join(", ");
    // StaffActionLogger's details, each value cut to 50,000 characters.
    let mut value = format!("[{list}]");
    if value.chars().count() > 50_000 {
        value = format!("{}...", value.chars().take(50_001).collect::<String>());
    }
    crate::tag_writes::log_custom(
        &mut tx,
        user_id,
        "deleted_unused_tags",
        None,
        &format!("tags: {value}"),
    )
    .await?;
    for (id, _) in &tags {
        crate::tag_writes::destroy(&mut tx, &payloads(&state, &s), *id).await?;
    }
    tx.commit().await?;
    Ok(success())
}

/// POST /tags/bulk_create(.json): Tags::BulkCreate.
pub async fn bulk_create(
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
    let mut conn = state.pool.acquire().await?;
    let s = settings(&state, &mut conn).await?;
    if !s.get("tagging_enabled")?.truthy() {
        return Ok(not_found(&state));
    }
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    if !can_admin_tags(&guardian, &s)? {
        return Ok(forbidden(&state));
    }
    let i18n = &state.i18n;
    let raw: Vec<String> = match p.get("tag_names") {
        Some(Value::Array(items)) => items
            .iter()
            .map(|v| crate::params::scalar(v).unwrap_or_default())
            .collect(),
        _ => {
            return Ok(json_errors(
                StatusCode::UNPROCESSABLE_ENTITY,
                vec![format!(
                    "Tag names {}",
                    i18n.t("tags.bulk_create.invalid_params")
                        .unwrap_or_default()
                )],
            ));
        }
    };
    if raw.len() > 100 {
        return Ok(json_errors(
            StatusCode::UNPROCESSABLE_ENTITY,
            vec![format!(
                "Tag names {}",
                i18n.t_count("tags.bulk_create.too_many_tags", 100, &[])
                    .unwrap_or_default()
            )],
        ));
    }
    let max = s.get("max_tag_length")?.to_i();
    let mut created = Vec::new();
    let mut existing = Vec::new();
    let mut failed = Map::new();
    let mut validated: Vec<(String, String)> = Vec::new();
    for name in raw {
        if crate::ruby::is_blank(&name) {
            continue;
        }
        let tag_name = crate::tagging::clean_tag_with(&s, &name, false)?;
        if tag_name.is_empty() {
            failed.insert(
                name,
                json!(i18n.t("tags.bulk_create.invalid_name").unwrap_or_default()),
            );
            continue;
        }
        if tag_name.chars().count() as i64 > max {
            failed.insert(
                name,
                json!(
                    i18n.t_count("tags.bulk_create.tag_too_long", max, &[])
                        .unwrap_or_default()
                ),
            );
            continue;
        }
        validated.push((name, tag_name));
    }
    let names: Vec<String> = validated.iter().map(|(_, n)| n.clone()).collect();
    let already: Vec<String> = sqlx::query_scalar("SELECT name FROM tags WHERE name = ANY($1)")
        .bind(&names)
        .fetch_all(&mut *conn)
        .await?;
    for (raw, tag_name) in validated {
        if already.contains(&tag_name) {
            existing.push(tag_name);
            continue;
        }
        match crate::tagging::create_tag(&mut conn, &s, &tag_name).await? {
            Some(tag) => {
                crate::tagging::enqueue_tag_created(&mut conn, std::slice::from_ref(&tag)).await?;
                created.push(tag_name);
            }
            None => {
                let errors = crate::tagging::tag_name_errors(&mut conn, i18n, &tag_name).await?;
                failed.insert(raw, json!(errors.join(", ")));
            }
        }
    }
    Ok(Json(json!({"created": created, "existing": existing, "failed": failed})).into_response())
}

/// POST /tags/upload(.json): a CSV of tag names, each optionally with a
/// tag group, in one transaction.
pub async fn upload(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    request: Request,
) -> Result<Response, AppError> {
    let path = request.uri().path().to_string();
    let multipart = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("multipart/form-data"));
    let (fields, file) = if multipart {
        let form = super::uploads::read_form(request, &state).await?;
        (form.fields, form.file.map(|(_, b)| b))
    } else {
        let query = request.uri().query().map(str::to_string);
        let body = Bytes::from_request(request, &state)
            .await
            .map_err(|e| std::io::Error::other(e.body_text()))?;
        let p = crate::params::parse(query.as_deref(), &headers, &body);
        (form_pairs(&p), None)
    };
    if !super::session::csrf_ok(&state, &headers, &fields, &path, "POST") {
        return Ok(super::session::bad_csrf());
    }
    let mut conn = state.pool.acquire().await?;
    let s = settings(&state, &mut conn).await?;
    if !s.get("tagging_enabled")?.truthy() {
        return Ok(not_found(&state));
    }
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    if !can_admin_tags(&guardian, &s)? {
        return Ok(forbidden(&state));
    }
    let Some(file) = file else {
        return Err(Unsupported("a tag upload without a file (a NoMethodError in Rails)").into());
    };
    let text = String::from_utf8(file).map_err(|_| Unsupported("a tag CSV that isn't UTF-8"))?;
    let rows = crate::custom_emoji_import::parse_csv(&text)
        .map_err(|_| Unsupported("a malformed tag CSV (CSV::MalformedCSVError)"))?;
    const MAX_CSV_ROWS: usize = 5000;
    drop(conn);
    let mut tx = state.pool.begin().await?;
    let mut created = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        let error = if index + 1 > MAX_CSV_ROWS {
            Some(
                state
                    .i18n
                    .t_count("tags.upload_too_many_rows", MAX_CSV_ROWS as i64, &[])
                    .unwrap_or_default(),
            )
        } else if row.len() > 2 {
            Some(
                state
                    .i18n
                    .t("tags.upload_row_too_long")
                    .unwrap_or_default()
                    .to_string(),
            )
        } else {
            None
        };
        if let Some(error) = error {
            tx.rollback().await?;
            return Ok((
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({"failed": "FAILED", "errors": [error]})),
            )
                .into_response());
        }
        let Some(Some(raw_name)) = row.first() else {
            return Err(
                Unsupported("a tag CSV row without a name (a NoMethodError in Rails)").into(),
            );
        };
        let tag_name = crate::tagging::clean_tag(&s, raw_name)?;
        let tag_id = match Tag::find_by_name(&mut tx, &tag_name).await? {
            Some(t) => t.id,
            None => match crate::tagging::create_tag(&mut tx, &s, &tag_name).await? {
                Some(tag) => {
                    let id = tag.id;
                    created.push(tag);
                    id
                }
                None => {
                    return Err(Unsupported(
                        "a tag CSV name Tag's validations refuse (RecordInvalid)",
                    )
                    .into());
                }
            },
        };
        if let Some(Some(group_name)) = row.get(1) {
            let group: Option<i32> = sqlx::query_scalar(
                "SELECT id FROM tag_groups WHERE lower(name) = lower($1) LIMIT 1",
            )
            .bind(group_name)
            .fetch_optional(&mut *tx)
            .await?;
            let group_id = match group {
                Some(id) => id,
                None if group_name.chars().count() > 100 => {
                    return Err(Unsupported(
                        "a tag group name over 100 characters (RecordInvalid)",
                    )
                    .into());
                }
                None => crate::tag_groups::create_minimal(&mut tx, group_name).await?,
            };
            sqlx::query(
                "INSERT INTO tag_group_memberships (tag_id, tag_group_id, created_at, updated_at) \
                 SELECT $1, $2, clock_timestamp(), clock_timestamp() \
                 WHERE NOT EXISTS (SELECT 1 FROM tag_group_memberships WHERE tag_id = $1 AND tag_group_id = $2)",
            )
            .bind(tag_id)
            .bind(group_id)
            .execute(&mut *tx)
            .await?;
        }
    }
    crate::tagging::enqueue_tag_created(&mut tx, &created).await?;
    tx.commit().await?;
    Ok(success())
}
