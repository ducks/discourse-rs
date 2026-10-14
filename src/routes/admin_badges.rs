//! Admin::BadgesController under /admin/badges, admins only
//! (AdminConstraint: a 404 for everyone else): the badges with their
//! groupings and types, badge types, saving the groupings, a badge
//! query's preview, creating, updating and destroying a badge, and the
//! mass award from a CSV file.
//!
//! Refused: the Ember pages (an HTML request).

use axum::Json;
use axum::body::Bytes;
use axum::extract::{FromRequest, Path, Request, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use super::session::{bad_csrf, csrf_ok};
use crate::admin_badges::{self as service, Ctx, PROTECTED_SYSTEM_FIELDS, Record, Saved};
use crate::badges_view::{BADGE_ROW_SELECT, BadgeRow, admin_badge_json, badge_type_json};
use crate::guardian::Guardian;
use crate::params;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::url::Urls;
use crate::{AppError, AppState, Unsupported};

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

/// `render_json_error(errors)`: 422 with the messages.
fn json_errors(errors: &[String]) -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({ "errors": errors })),
    )
        .into_response()
}

fn xhr_or_json(path: &str, headers: &HeaderMap) -> bool {
    path.ends_with(".json")
        || super::wants_json(None, headers)
        || headers
            .get("x-requested-with")
            .is_some_and(|v| v.as_bytes() == b"XMLHttpRequest")
}

/// check_xhr: the admin pages themselves are Ember's.
fn refuse_html(path: &str, headers: &HeaderMap) -> Result<(), AppError> {
    if xhr_or_json(path, headers) {
        Ok(())
    } else {
        Err(Unsupported("the admin badge pages (Ember)").into())
    }
}

/// AdminConstraint: the routes exist for admins only.
fn admin(state: &AppState, guardian: &Guardian) -> Result<i32, Box<Response>> {
    match guardian.user_id() {
        Some(id) if guardian.is_admin() => Ok(id),
        _ => Err(Box::new(super::topics::not_found_response(state))),
    }
}

fn strip_json(s: &str) -> &str {
    s.strip_suffix(".json").unwrap_or(s)
}

/// `Badge.find(params[:id])`'s id: an integer cast of the segment.
fn badge_id(segment: &str) -> Option<i32> {
    let s = strip_json(segment);
    let t = s.trim_start();
    let t2 = t.strip_prefix(['+', '-']).unwrap_or(t);
    if !t2.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    i32::try_from(crate::ruby::to_i(s)).ok()
}

struct Loaded {
    settings: SiteSettings,
}

async fn load(state: &AppState, conn: &mut sqlx::PgConnection) -> Result<Loaded, AppError> {
    Ok(Loaded {
        settings: SiteSettings::load(conn, &state.site_setting_defs, &state.config.globals).await?,
    })
}

/// GET /admin/badges(.json): AdminBadgesSerializer.
pub async fn index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if let Err(r) = admin(&state, &guardian) {
        return Ok(*r);
    }
    refuse_html(uri.path(), &headers)?;
    let mut conn = state.pool.acquire().await?;
    let Loaded { settings } = load(&state, &mut conn).await?;
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let base_path = state.config.globals.relative_url_root();
    // badges.name in the collation Discourse's databases are created
    // with (en_US.UTF-8), whatever this database's default is.
    let rows: Vec<BadgeRow> = sqlx::query_as(&format!(
        "{BADGE_ROW_SELECT} ORDER BY b.enabled DESC, bg.position, b.badge_type_id, b.name COLLATE \"en_US\""
    ))
    .fetch_all(&mut *conn)
    .await?;
    let all_types: Vec<(i32, String)> =
        sqlx::query_as("SELECT id, name FROM badge_types ORDER BY id")
            .fetch_all(&mut *conn)
            .await?;
    let groupings: Vec<(i32, String, Option<String>, i32)> = sqlx::query_as(
        "SELECT id, name, description, position FROM badge_groupings ORDER BY position",
    )
    .fetch_all(&mut *conn)
    .await?;
    let mut badges = Vec::new();
    let mut types: Vec<Value> = Vec::new();
    let mut type_ids: Vec<i32> = Vec::new();
    for b in &rows {
        badges.push(admin_badge_json(
            &state.i18n,
            &settings,
            &urls,
            base_path,
            b,
        )?);
        if !type_ids.contains(&b.badge_type_id) {
            type_ids.push(b.badge_type_id);
            types.push(badge_type_json(b.badge_type_id, &b.badge_type_name));
        }
    }
    for (id, name) in &all_types {
        if !type_ids.contains(id) {
            type_ids.push(*id);
            types.push(badge_type_json(*id, name));
        }
    }
    let grouping_json: Vec<Value> = groupings
        .iter()
        .map(|(id, name, description, position)| {
            json!({"id": id, "name": name, "description": description, "position": position, "system": *id <= 5})
        })
        .collect();
    Ok(Json(json!({
        "badges": badges,
        "badge_types": types,
        "badge_groupings": grouping_json,
        "admin_badges": {
            "protected_system_fields": PROTECTED_SYSTEM_FIELDS,
            "triggers": {"none": 0, "post_action": 1, "post_revision": 2, "trust_level_change": 4, "user_change": 8},
            "badge_ids": rows.iter().map(|b| b.id).collect::<Vec<_>>(),
            "badge_grouping_ids": groupings.iter().map(|g| g.0).collect::<Vec<_>>(),
            "badge_type_ids": all_types.iter().map(|t| t.0).collect::<Vec<_>>(),
        },
    }))
    .into_response())
}

/// GET /admin/badges/types(.json)
pub async fn badge_types(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if let Err(r) = admin(&state, &guardian) {
        return Ok(*r);
    }
    refuse_html(uri.path(), &headers)?;
    // BadgeType.all, in the table's order.
    let types: Vec<(i32, String)> = sqlx::query_as("SELECT id, name FROM badge_types")
        .fetch_all(&state.pool)
        .await?;
    let types: Vec<Value> = types
        .iter()
        .map(|(id, name)| badge_type_json(*id, name))
        .collect();
    Ok(Json(json!({ "badge_types": types })).into_response())
}

/// GET /admin/badges/{new,:id} and /admin/badges/award/:badge_id: empty
/// actions, the Ember page for a browser and no content otherwise.
pub async fn page(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if let Err(r) = admin(&state, &guardian) {
        return Ok(*r);
    }
    refuse_html(uri.path(), &headers)?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// The badge's AdminBadgeSerializer document: its type and itself.
async fn badge_document(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    settings: &SiteSettings,
    id: i32,
) -> Result<Value, AppError> {
    let row = BadgeRow::load(conn, id).await?;
    let urls = Urls {
        config: &state.config,
        settings,
    };
    let badge = admin_badge_json(
        &state.i18n,
        settings,
        &urls,
        state.config.globals.relative_url_root(),
        &row,
    )?;
    Ok(json!({
        "badge_types": [badge_type_json(row.badge_type_id, &row.badge_type_name)],
        "badge": badge,
    }))
}

/// POST /admin/badges(.json)
pub async fn create(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let admin_id = match admin(&state, &guardian) {
        Ok(id) => id,
        Err(r) => return Ok(*r),
    };
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "POST") {
        return Ok(bad_csrf());
    }
    let mut tx = state.pool.begin().await?;
    let Loaded { settings } = load(&state, &mut tx).await?;
    let ctx = Ctx {
        settings: &settings,
        i18n: &state.i18n,
        bus: &state.bus,
        base_path: state.config.globals.relative_url_root(),
    };
    let mut record = Record::new();
    match service::update_from_params(&mut tx, &ctx, &mut record, &p).await? {
        Saved::Invalid(errors) => Ok(json_errors(&errors)),
        Saved::Done { id, .. } => {
            service::log_creation(&mut tx, &ctx, &record, admin_id).await?;
            let doc = badge_document(&state, &mut tx, &settings, id).await?;
            tx.commit().await?;
            Ok(Json(doc).into_response())
        }
    }
}

/// PUT /admin/badges/:id(.json)
pub async fn update(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let admin_id = match admin(&state, &guardian) {
        Ok(id) => id,
        Err(r) => return Ok(*r),
    };
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "PUT") {
        return Ok(bad_csrf());
    }
    let mut tx = state.pool.begin().await?;
    let record = match badge_id(&id) {
        Some(id) => Record::find(&mut tx, id).await?,
        None => None,
    };
    let Some(mut record) = record else {
        return Ok(super::topics::not_found_response(&state));
    };
    let Loaded { settings } = load(&state, &mut tx).await?;
    let ctx = Ctx {
        settings: &settings,
        i18n: &state.i18n,
        bus: &state.bus,
        base_path: state.config.globals.relative_url_root(),
    };
    match service::update_from_params(&mut tx, &ctx, &mut record, &p).await? {
        Saved::Invalid(errors) => Ok(json_errors(&errors)),
        Saved::Done { id, name_changed } => {
            if name_changed {
                service::enqueue_title_update(&mut tx, &state.i18n, &record).await?;
            }
            service::log_change(&mut tx, &record, admin_id).await?;
            let doc = badge_document(&state, &mut tx, &settings, id).await?;
            tx.commit().await?;
            Ok(Json(doc).into_response())
        }
    }
}

/// DELETE /admin/badges/:id(.json)
pub async fn destroy(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let admin_id = match admin(&state, &guardian) {
        Ok(id) => id,
        Err(r) => return Ok(*r),
    };
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "DELETE") {
        return Ok(bad_csrf());
    }
    let mut tx = state.pool.begin().await?;
    let record = match badge_id(&id) {
        Some(id) => Record::find(&mut tx, id).await?,
        None => None,
    };
    let Some(record) = record else {
        return Ok(super::topics::not_found_response(&state));
    };
    if record.system {
        let message = state
            .i18n
            .t("badges.errors.cant_delete_system_badge")
            .unwrap_or("System badges cannot be deleted.")
            .to_string();
        return Ok(json_errors(&[message]));
    }
    let Loaded { settings } = load(&state, &mut tx).await?;
    let ctx = Ctx {
        settings: &settings,
        i18n: &state.i18n,
        bus: &state.bus,
        base_path: state.config.globals.relative_url_root(),
    };
    service::destroy(&mut tx, &ctx, &record, admin_id).await?;
    tx.commit().await?;
    // render body: nil
    Ok(([(header::CONTENT_TYPE, "text/html")], "").into_response())
}

/// POST /admin/badges/badge_groupings(.json)
pub async fn save_badge_groupings(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if let Err(r) = admin(&state, &guardian) {
        return Ok(*r);
    }
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "POST") {
        return Ok(bad_csrf());
    }
    let strings = |k: &str| -> Option<Vec<String>> {
        match p.get(k)? {
            Value::Array(items) => items.iter().map(params::scalar).collect(),
            _ => None,
        }
    };
    let (Some(ids), Some(names)) = (strings("ids"), strings("names")) else {
        return Err(Unsupported("saving badge groupings without ids and names arrays").into());
    };
    // ids.map(&:to_i)
    let ids: Vec<i64> = ids.iter().map(|s| crate::ruby::to_i(s)).collect();
    let mut tx = state.pool.begin().await?;
    service::save_groupings(&mut tx, &ids, &names).await?;
    let groupings: Vec<(i32, String, Option<String>, i32)> = sqlx::query_as(
        "SELECT id, name, description, position FROM badge_groupings ORDER BY position",
    )
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    let groupings: Vec<Value> = groupings
        .iter()
        .map(|(id, name, description, position)| {
            json!({"id": id, "name": name, "description": description, "position": position, "system": *id <= 5})
        })
        .collect();
    Ok(Json(json!({ "badge_groupings": groupings })).into_response())
}

/// POST /admin/badges/preview(.json)
pub async fn preview(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if let Err(r) = admin(&state, &guardian) {
        return Ok(*r);
    }
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "POST") {
        return Ok(bad_csrf());
    }
    let mut conn = state.pool.acquire().await?;
    let Loaded { settings } = load(&state, &mut conn).await?;
    if !settings.get("enable_badge_sql")?.truthy() {
        return Ok((StatusCode::FORBIDDEN, Json(json!("preview not allowed"))).into_response());
    }
    let text = |k: &str| p.get(k).and_then(params::scalar);
    let opts = crate::badge_preview::Opts {
        target_posts: text("target_posts").as_deref() == Some("true"),
        explain: text("explain").as_deref() == Some("true"),
        trigger: text("trigger").map_or(0, |t| crate::ruby::to_i(&t)),
    };
    let sql = match p.get("sql") {
        None | Some(Value::Null) => None,
        Some(v) => {
            Some(params::scalar(v).ok_or(Unsupported("a badge preview sql that isn't a string"))?)
        }
    };
    let doc = crate::badge_preview::preview(&mut conn, sql.as_deref(), &opts).await?;
    Ok(Json(doc).into_response())
}

/// The mass award's params: multipart from the admin page, else the
/// request's params (with no file).
async fn award_form(
    state: &AppState,
    headers: &HeaderMap,
    request: Request,
) -> Result<(Vec<(String, String)>, Option<Vec<u8>>), AppError> {
    let multipart = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("multipart/form-data"));
    if multipart {
        let form = super::uploads::read_form(request, state).await?;
        return Ok((form.fields, form.file.map(|(_, bytes)| bytes)));
    }
    let query = request.uri().query().map(str::to_string);
    let body = Bytes::from_request(request, state)
        .await
        .map_err(|e| std::io::Error::other(e.body_text()))?;
    let p = params::parse(query.as_deref(), headers, &body);
    Ok((form_pairs(&p), None))
}

/// POST /admin/badges/award/:badge_id(.json): the mass award.
pub async fn mass_award(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(badge_param): Path<String>,
    headers: HeaderMap,
    request: Request,
) -> Result<Response, AppError> {
    if let Err(r) = admin(&state, &guardian) {
        return Ok(*r);
    }
    let path = request.uri().path().to_string();
    let (fields, file) = award_form(&state, &headers, request).await?;
    if !csrf_ok(&state, &headers, &fields, &path, "POST") {
        return Ok(bad_csrf());
    }
    let field = |k: &str| fields.iter().find(|(f, _)| f == k).map(|(_, v)| v.as_str());
    let mut tx = state.pool.begin().await?;
    let badge = match badge_id(&badge_param) {
        Some(id) => crate::badge_granter::Badge::find(&mut tx, id).await?,
        None => None,
    };
    let (Some(file), Some(badge)) = (file, badge) else {
        return Ok(super::search::invalid_parameters(
            &state,
            "Discourse::InvalidParameters",
        ));
    };
    let display_name = badge.display_name(&state.i18n);
    if !badge.enabled {
        let message = state
            .i18n
            .t_with(
                "badges.mass_award.errors.badge_disabled",
                &[("badge_name", &display_name)],
            )
            .unwrap_or_else(|| format!("Please enable the {display_name} badge first."));
        return Ok(json_errors(&[message]));
    }
    let replace = field("replace_badge_owners") == Some("true");
    let once = field("grant_existing_holders") != Some("true");
    if !once && !badge.multiple_grant {
        let message = state
            .i18n
            .t_with(
                "badges.mass_award.errors.cant_grant_multiple_times",
                &[("badge_name", &display_name)],
            )
            .unwrap_or_else(|| {
                format!("Can't grant the {display_name} badge multiple times to a single user.")
            });
        return Ok(json_errors(&[message]));
    }
    // render_json_error(error, status: 400)
    let bad_request = |message: String| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({ "errors": [message] })),
        )
            .into_response()
    };
    let entries = match service::read_csv(&file)? {
        service::Csv::Entries(entries) => entries,
        service::Csv::TooMany => {
            let count = service::MAX_CSV_LINES as i64;
            let message = state
                .i18n
                .t_count("badges.mass_award.errors.too_many_csv_entries", count, &[])
                .ok_or(Unsupported("the too_many_csv_entries message"))?;
            return Ok(bad_request(message));
        }
        service::Csv::Malformed(line) => {
            let line = line.to_string();
            let message = state
                .i18n
                .t_with(
                    "badges.mass_award.errors.invalid_csv",
                    &[("line_number", &line)],
                )
                .ok_or(Unsupported("the invalid_csv message"))?;
            return Ok(bad_request(message));
        }
    };
    if replace {
        service::revoke_all(&mut tx, &badge).await?;
    }
    let result = service::enqueue_mass_grant(&mut tx, badge.id, &entries, once).await?;
    tx.commit().await?;
    let first: Vec<&String> = result.unmatched.iter().take(100).collect();
    Ok(Json(json!({
        "unmatched_entries": first,
        "matched_users_count": result.matched_users_count,
        "unmatched_entries_count": result.unmatched.len(),
    }))
    .into_response())
}
