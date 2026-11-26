//! /tag_groups: TagGroupsController (staff, as StaffConstraint routes it:
//! everyone else 404s), and /tag_groups/filter/search for anyone.
//!
//! A page load of the list, new or a group is its page
//! (crate::tags::groups_view).

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use crate::guardian::Guardian;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::tag_groups::{GroupParams, ParentChoice, TagsChoice};
use crate::{AppError, AppState, Unsupported};

fn not_found(state: &AppState) -> Response {
    super::topics::not_found_response(state)
}

fn wants_json(headers: &HeaderMap, path: &str) -> bool {
    path.ends_with(".json")
        || headers
            .get(header::ACCEPT)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|a| a.contains("application/json"))
}

fn form_pairs(p: &Map<String, Value>) -> Vec<(String, String)> {
    p.iter()
        .filter_map(|(k, v)| crate::params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

/// StaffConstraint, then a staff member.
fn staff(state: &AppState, guardian: &Guardian) -> Option<Response> {
    (!guardian.is_staff()).then(|| not_found(state))
}

fn id_of(segment: &str) -> Option<i32> {
    segment
        .strip_suffix(".json")
        .unwrap_or(segment)
        .parse()
        .ok()
}

enum Which {
    Index,
    New,
    Edit(Option<i32>),
}

/// A page load of /tag_groups(/new|/:id): the tagGroups route with its
/// index, new or edit outlet (src/tags/groups_view.rs).
async fn page(
    state: &AppState,
    guardian: &Guardian,
    headers: &HeaderMap,
    which: Which,
) -> Result<Response, AppError> {
    use crate::tags::groups_view::{Outlet, Page, render};
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let tagging_enabled = settings.get("tagging_enabled")?.truthy();
    let base_path = state.config.globals.relative_url_root().to_string();
    // tagGroups.new's beforeModel: back to the list while tagging is off.
    if matches!(which, Which::New) && !tagging_enabled {
        return Ok(
            axum::response::Redirect::to(&format!("{base_path}/tag_groups")).into_response(),
        );
    }
    // tag_groups#show skips check_xhr, so fetch_tag_group's RecordNotFound
    // is the 404 page for a group that isn't there.
    let edited = match which {
        Which::Edit(id) => {
            let exists = match id {
                Some(id) => {
                    sqlx::query_scalar::<_, i32>("SELECT id FROM tag_groups WHERE id = $1")
                        .bind(id)
                        .fetch_optional(&mut *conn)
                        .await?
                }
                None => None,
            };
            match exists {
                Some(id) => Some(crate::tag_groups::serialize(&mut conn, id).await?),
                None => return Ok(not_found(state)),
            }
        }
        _ => None,
    };
    let groups: Vec<(i32, String)> =
        sqlx::query_as("SELECT id, name FROM tag_groups ORDER BY name ASC")
            .fetch_all(&mut *conn)
            .await?;
    // Site#groups less everyone, as TagGroupsForm's allGroups.
    let site_groups: Vec<(i32, String)> = sqlx::query_as(&format!(
        "SELECT id, name FROM groups WHERE id <> 0 AND {} ORDER BY name ASC",
        crate::groups::visible_groups_where(guardian, "groups")
    ))
    .fetch_all(&mut *conn)
    .await?;
    let t = |key: &str| {
        state
            .i18n
            .t(&format!("js.{key}"))
            .unwrap_or_default()
            .to_string()
    };
    let chooser_strings = json!({
        "default_header": t("select_kit.default_header_text"),
        "filter_placeholder": t("select_kit.filter_placeholder"),
        "tags_placeholder": t("tagging.groups.tags_placeholder"),
        "results_one": t("select_kit.results_count.one").replace("%{count}", "1"),
        "results_other": t("select_kit.results_count.other"),
        "delete_item": t("select_kit.delete_item"),
        "filter_by": t("select_kit.filter_by"),
        "max_tag_search_results": settings.get("max_tag_search_results")?.to_i(),
        "max_tag_length": settings.get("max_tag_length")?.to_i(),
        "force_lowercase_tags": settings.get("force_lowercase_tags")?.truthy(),
        "tags_sort_alphabetically": settings.get("tags_sort_alphabetically")?.truthy(),
        "tag_style": settings.get("tag_style")?.to_s(),
    });
    let outlet = match &edited {
        Some(group) => Outlet::Edit(group),
        None if matches!(which, Which::New) => Outlet::New,
        None => Outlet::Index,
    };
    let main = render(&Page {
        i18n: &state.i18n,
        base_path: &base_path,
        tagging_enabled,
        groups: &groups,
        site_groups: &site_groups,
        outlet,
        chooser_strings,
    }) + &format!(
        "<script type=\"application/json\" id=\"dialog-strings\">{}</script>",
        crate::html::dialog_strings(&state.i18n)
    );
    let bus_position = crate::bus::page_position(&state.bus).await?;
    super::user_preferences::app_page(
        state,
        &mut conn,
        &settings,
        guardian,
        headers,
        bus_position,
        t("tagging.groups.title"),
        main,
    )
    .await
}

/// GET /tag_groups(.json)
pub async fn index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if let Some(r) = staff(&state, &guardian) {
        return Ok(r);
    }
    if !wants_json(&headers, uri.path()) {
        return page(&state, &guardian, &headers, Which::Index).await;
    }
    let mut conn = state.pool.acquire().await?;
    let ids: Vec<i32> = sqlx::query_scalar("SELECT id FROM tag_groups ORDER BY name ASC")
        .fetch_all(&mut *conn)
        .await?;
    let mut groups = Vec::new();
    for id in ids {
        groups.push(crate::tag_groups::serialize(&mut conn, id).await?);
    }
    Ok(Json(json!({ "tag_groups": groups })).into_response())
}

/// GET /tag_groups/:id(.json), and /tag_groups/new (a page).
pub async fn show(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if let Some(r) = staff(&state, &guardian) {
        return Ok(r);
    }
    if !wants_json(&headers, uri.path()) {
        let which = if id == "new" {
            Which::New
        } else {
            Which::Edit(id_of(&id))
        };
        return page(&state, &guardian, &headers, which).await;
    }
    if id == "new" {
        return Err(Unsupported("tag_groups#new's JSON").into());
    }
    let mut conn = state.pool.acquire().await?;
    let exists = match id_of(&id) {
        Some(id) => {
            sqlx::query_scalar::<_, i32>("SELECT id FROM tag_groups WHERE id = $1")
                .bind(id)
                .fetch_optional(&mut *conn)
                .await?
        }
        None => None,
    };
    let Some(id) = exists else {
        return Ok(not_found(&state));
    };
    Ok(
        Json(json!({ "tag_group": crate::tag_groups::serialize(&mut conn, id).await? }))
            .into_response(),
    )
}

/// `tag_groups_params`: the tag_group hash merged over the params.
async fn group_params(
    conn: &mut sqlx::PgConnection,
    s: &SiteSettings,
    i18n: &crate::i18n::I18n,
    p: &Map<String, Value>,
) -> Result<GroupParams, AppError> {
    let mut merged = p.clone();
    if let Some(Value::Object(group)) = p.get("tag_group") {
        for (k, v) in group {
            merged.insert(k.clone(), v.clone());
        }
    }
    let list = |v: Option<&Value>| -> Vec<Value> {
        match v {
            Some(Value::Array(items)) => items.clone(),
            Some(Value::Object(map)) => map.values().cloned().collect(),
            _ => Vec::new(),
        }
    };
    let tags = list(merged.get("tags"));
    let tags = if !tags.is_empty() {
        let mut ids: Vec<i64> = Vec::new();
        let mut new_names: Vec<String> = Vec::new();
        for t in &tags {
            match t
                .get("id")
                .and_then(crate::params::scalar)
                .filter(|i| !crate::ruby::is_blank(i))
            {
                Some(id) => ids.push(crate::ruby::to_i(&id)),
                None => {
                    if let Some(name) = t.get("name").and_then(crate::params::scalar) {
                        new_names.push(name);
                    }
                }
            }
        }
        if !new_names.is_empty() {
            let system = crate::web_hooks::system_guardian(conn).await?;
            match crate::tagging::find_or_create_tags(conn, s, i18n, &system, &new_names).await? {
                Ok(found) => ids.extend(found.into_iter().map(|(id, _)| i64::from(id))),
                Err(_) => {
                    return Err(Unsupported("a new tag Tag.create! refuses, in a tag group").into());
                }
            }
        }
        TagsChoice::Ids(ids)
    } else {
        TagsChoice::Names(
            list(merged.get("tag_names"))
                .iter()
                .filter_map(crate::params::scalar)
                .collect(),
        )
    };
    let parent_list = list(merged.get("parent_tag"));
    let parent = match parent_list.first() {
        Some(first) => {
            match first
                .get("id")
                .and_then(crate::params::scalar)
                .filter(|i| !crate::ruby::is_blank(i))
            {
                Some(id) => ParentChoice::Id(crate::ruby::to_i(&id)),
                None => match first
                    .get("name")
                    .and_then(crate::params::scalar)
                    .filter(|n| !crate::ruby::is_blank(n))
                {
                    Some(name) => ParentChoice::Names(vec![name]),
                    None => ParentChoice::Names(Vec::new()),
                },
            }
        }
        None => ParentChoice::Names(
            list(merged.get("parent_tag_name"))
                .iter()
                .filter_map(crate::params::scalar)
                .collect(),
        ),
    };
    let one_per_topic = matches!(merged.get("one_per_topic"), Some(Value::Bool(true)))
        || merged.get("one_per_topic").and_then(Value::as_str) == Some("true");
    let permissions = match merged.get("permissions") {
        Some(Value::Object(map)) => {
            let mut out = Vec::new();
            for (key, value) in map {
                let group = crate::tag_groups::group_id_from_param(conn, key).await?;
                let permission = match value {
                    Value::Number(n) => n.as_i64().unwrap_or(0) as i32,
                    Value::String(s) => match s.as_str() {
                        "full" => crate::tagging::PERMISSION_FULL,
                        "readonly" => crate::tagging::PERMISSION_READONLY,
                        _ => {
                            return Err(
                                Unsupported("a tag group permission of another kind").into()
                            );
                        }
                    },
                    _ => return Err(Unsupported("a tag group permission of another shape").into()),
                };
                out.push((group, permission));
            }
            Some(out)
        }
        Some(Value::String(s)) if s.is_empty() => Some(Vec::new()),
        None | Some(Value::Null) => None,
        Some(_) => return Err(Unsupported("tag group permissions of another shape").into()),
    };
    Ok(GroupParams {
        name: merged.get("name").and_then(crate::params::scalar),
        tags,
        parent,
        one_per_topic,
        permissions,
    })
}

fn errors(errors: Vec<String>) -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({ "errors": errors })),
    )
        .into_response()
}

/// POST /tag_groups(.json)
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
    if let Some(r) = staff(&state, &guardian) {
        return Ok(r);
    }
    let user_id = guardian.user_id().ok_or(Unsupported("anonymous staff"))?;
    let mut conn = state.pool.acquire().await?;
    let s = SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    if !s.get("tagging_enabled")?.truthy() {
        return Ok(super::search::invalid_access(&state));
    }
    let params = group_params(&mut conn, &s, &state.i18n, &p).await?;
    drop(conn);
    let mut tx = state.pool.begin().await?;
    match crate::tag_groups::create(&mut tx, &s, &state.i18n, &params).await? {
        Err(e) => {
            tx.commit().await?;
            Ok(errors(e))
        }
        Ok(id) => {
            let json = crate::tag_groups::serialize(&mut tx, id).await?;
            let name = json["name"].as_str().unwrap_or_default().to_string();
            crate::tag_groups::log(
                &mut tx,
                &state.i18n,
                user_id,
                "tag_group_create",
                &name,
                None,
                Some(&crate::web_hooks::to_json(&json)),
            )
            .await?;
            tx.commit().await?;
            Ok(Json(json!({ "tag_group": json })).into_response())
        }
    }
}

/// PUT /tag_groups/:id(.json)
pub async fn update(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = crate::params::parse(uri.query(), &headers, &body);
    if !super::session::csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "PUT") {
        return Ok(super::session::bad_csrf());
    }
    if let Some(r) = staff(&state, &guardian) {
        return Ok(r);
    }
    let user_id = guardian.user_id().ok_or(Unsupported("anonymous staff"))?;
    let mut conn = state.pool.acquire().await?;
    let s = SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let exists = match id_of(&id) {
        Some(id) => {
            sqlx::query_scalar::<_, i32>("SELECT id FROM tag_groups WHERE id = $1")
                .bind(id)
                .fetch_optional(&mut *conn)
                .await?
        }
        None => None,
    };
    let Some(id) = exists else {
        return Ok(not_found(&state));
    };
    if !s.get("tagging_enabled")?.truthy() {
        return Ok(super::search::invalid_access(&state));
    }
    let old = crate::web_hooks::to_json(&crate::tag_groups::serialize(&mut conn, id).await?);
    let params = group_params(&mut conn, &s, &state.i18n, &p).await?;
    drop(conn);
    let mut tx = state.pool.begin().await?;
    match crate::tag_groups::update(&mut tx, &s, &state.i18n, id, &params).await? {
        Err(e) => {
            tx.rollback().await?;
            Ok(errors(e))
        }
        Ok(()) => {
            let json = crate::tag_groups::serialize(&mut tx, id).await?;
            let name = json["name"].as_str().unwrap_or_default().to_string();
            crate::tag_groups::log(
                &mut tx,
                &state.i18n,
                user_id,
                "tag_group_change",
                &name,
                Some(&old),
                Some(&crate::web_hooks::to_json(&json)),
            )
            .await?;
            tx.commit().await?;
            Ok(Json(json!({ "success": "OK", "tag_group": json })).into_response())
        }
    }
}

/// DELETE /tag_groups/:id(.json)
pub async fn destroy(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = crate::params::parse(uri.query(), &headers, &body);
    if !super::session::csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "DELETE") {
        return Ok(super::session::bad_csrf());
    }
    if let Some(r) = staff(&state, &guardian) {
        return Ok(r);
    }
    let user_id = guardian.user_id().ok_or(Unsupported("anonymous staff"))?;
    let mut tx = state.pool.begin().await?;
    let s = SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let exists = match id_of(&id) {
        Some(id) => {
            sqlx::query_scalar::<_, i32>("SELECT id FROM tag_groups WHERE id = $1")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?
        }
        None => None,
    };
    let Some(id) = exists else {
        return Ok(not_found(&state));
    };
    if !s.get("tagging_enabled")?.truthy() {
        return Ok(super::search::invalid_access(&state));
    }
    let json = crate::tag_groups::serialize(&mut tx, id).await?;
    let name = json["name"].as_str().unwrap_or_default().to_string();
    crate::tag_groups::log(
        &mut tx,
        &state.i18n,
        user_id,
        "tag_group_destroy",
        &name,
        Some(&crate::web_hooks::to_json(&json)),
        None,
    )
    .await?;
    crate::tag_groups::destroy(&mut tx, id).await?;
    tx.commit().await?;
    Ok(Json(json!({"success": "OK"})).into_response())
}

/// GET /tag_groups/filter/search(.json): the visible groups whose name
/// has `q` or is among `names`, by name, with their visible base tags.
pub async fn search(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    uri: Uri,
) -> Result<Response, AppError> {
    let p = crate::params::parse_query(uri.query().unwrap_or(""));
    let mut conn = state.pool.acquire().await?;
    let s = SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let max = s.get("max_tag_search_results")?.to_i();
    // fetch_limit_from_params(default: max_tag_search_results, max: 1000)
    let limit = match p.get("limit") {
        None => max,
        Some(v) => match crate::params::scalar(v).and_then(|v| crate::ruby::integer(&v)) {
            Some(n) if (0..=1000).contains(&n) => n,
            _ => return Ok(super::search::invalid_parameters(&state, "limit")),
        },
    };
    let mut wheres: Vec<String> = Vec::new();
    if !guardian.is_admin() {
        wheres.push(crate::tags::visible_tag_groups_where(&guardian, &s)?);
    }
    let q = crate::params::string(&p, "q").filter(|q| !q.trim().is_empty());
    if q.is_some() {
        wheres.push("lower(name) ILIKE $1".into());
    }
    let names: Vec<String> = match p.get("names") {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(crate::params::scalar)
            .map(|n| n.to_lowercase())
            .collect(),
        _ => Vec::new(),
    };
    if !names.is_empty() {
        wheres.push("lower(name) = ANY($2)".into());
    }
    let where_sql = if wheres.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", wheres.join(" AND "))
    };
    let groups: Vec<(i32, String)> = sqlx::query_as(&format!(
        "SELECT id, name FROM tag_groups {where_sql} ORDER BY name LIMIT $3"
    ))
    .bind(format!("%{}%", q.as_deref().unwrap_or("").trim()))
    .bind(&names)
    .bind(limit)
    .fetch_all(&mut *conn)
    .await?;
    let visible = crate::tags::visible_tags_where(&guardian, &s)?;
    let mut results = Vec::new();
    for (id, name) in groups {
        let tags: Vec<(i32, String, String)> = sqlx::query_as(&format!(
            "SELECT tags.id, tags.name, tags.slug FROM tags \
             JOIN tag_group_memberships tgm ON tgm.tag_id = tags.id \
             WHERE tgm.tag_group_id = $1 AND tags.target_tag_id IS NULL AND {visible} ORDER BY tgm.ctid"
        ))
        .bind(id)
        .fetch_all(&mut *conn)
        .await?;
        results.push(json!({
            "name": name,
            "tags": tags.iter().map(|(id, name, slug)| json!({"id": id, "name": name, "slug": slug})).collect::<Vec<_>>(),
        }));
    }
    Ok(Json(json!({ "results": results })).into_response())
}
