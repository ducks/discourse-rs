//! Admin::EmojiController (index, create, destroy) under
//! /admin/config/emoji, admins only (AdminConstraint: 404 for others),
//! and SiteController#emoji (GET /site/emoji).
//!
//! Refused: the Ember pages (an HTML request), and export/import.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{FromRequest, Path, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

use super::session::{bad_csrf, csrf_ok};
use super::uploads::{Form, read_form};
use crate::custom_emoji::{self, Created};
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::uploads::{NewUpload, Outcome};
use crate::url::Urls;
use crate::{AppError, AppState, Unsupported};

fn failed(messages: Vec<String>) -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({ "failed": "FAILED", "errors": messages })),
    )
        .into_response()
}

/// check_xhr: the admin pages themselves are Ember's.
fn refuse_html(path: &str, headers: &HeaderMap) -> Result<(), AppError> {
    if super::wants_json(None, headers) || path.ends_with(".json") {
        return Ok(());
    }
    Err(Unsupported("the admin emoji pages (Ember)").into())
}

/// GET /admin/config/emoji(.json)
pub async fn index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    uri: axum::http::Uri,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    if !guardian.is_admin() {
        return Ok(super::topics::not_found_response(&state));
    }
    refuse_html(uri.path(), &headers)?;
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    Ok(Json(custom_emoji::admin_index(settings.emojis())).into_response())
}

/// GET /admin/config/emoji/{new,settings,import}: Ember's pages.
pub async fn page(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
) -> Result<Response, AppError> {
    if !guardian.is_admin() {
        return Ok(super::topics::not_found_response(&state));
    }
    Err(Unsupported("the admin emoji pages (Ember)").into())
}

/// The create form: multipart from the admin page, else form or JSON
/// params (with no file).
async fn create_form(
    state: &AppState,
    headers: &HeaderMap,
    request: Request,
) -> Result<Form, AppError> {
    let multipart = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("multipart/form-data"));
    if multipart {
        return read_form(request, state).await;
    }
    let query = request.uri().query().map(str::to_string);
    let body = Bytes::from_request(request, state)
        .await
        .map_err(|e| std::io::Error::other(e.body_text()))?;
    let params = crate::params::parse(query.as_deref(), headers, &body);
    if params.contains_key("file") || params.contains_key("files") {
        return Err(Unsupported("custom emoji files not sent as multipart/form-data").into());
    }
    let mut form = Form::default();
    for (k, v) in &params {
        if let Some(v) = crate::params::scalar(v) {
            form.fields.push((k.clone(), v));
        }
    }
    Ok(form)
}

/// POST /admin/config/emoji(.json): CustomEmoji::Create.
pub async fn create(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    request: Request,
) -> Result<Response, AppError> {
    if !guardian.is_admin() {
        return Ok(super::topics::not_found_response(&state));
    }
    let path = request.uri().path().to_string();
    let form = create_form(&state, &headers, request).await?;
    if !csrf_ok(&state, &headers, &form.fields, &path, "POST") {
        return Ok(bad_csrf());
    }
    let admin_id = guardian
        .user_id()
        .ok_or(Unsupported("an admin without a user"))?;
    // The contract: a file, the name from the given one or the file's.
    let Some((filename, bytes)) = &form.file else {
        let blank = state
            .i18n
            .t("errors.messages.blank")
            .unwrap_or("can't be blank");
        return Ok(failed(vec![format!("File {blank}")]));
    };
    let name = custom_emoji::sanitize_name(custom_emoji::basename_without_extension(
        form.get("name").unwrap_or(filename),
    ));
    let group = form.get("group").map(str::to_lowercase);

    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    // The upload, made before (and kept whatever follows) the record.
    let outcome = crate::uploads::create(
        &mut conn,
        &settings,
        &state.i18n,
        &urls,
        &crate::file_store::FileStore::for_site(&state.config, &settings)?,
        &NewUpload {
            user_id: admin_id,
            staff: true,
            upload_type: "custom_emoji",
            filename,
            bytes,
            pasted: false,
            for_private_message: false,
        },
    )
    .await?;
    let upload_id = match outcome {
        Outcome::Created(upload) => upload["id"]
            .as_i64()
            .and_then(|id| i32::try_from(id).ok())
            .ok_or(Unsupported("an upload without its id"))?,
        Outcome::Invalid(messages) => return Ok(failed(messages)),
    };
    drop(conn);
    let mut tx = state.pool.begin().await?;
    let created = custom_emoji::create(
        &mut tx,
        &state.i18n,
        admin_id,
        &name,
        group.as_deref(),
        upload_id,
    )
    .await?;
    match created {
        Created::Emoji(emoji) => {
            tx.commit().await?;
            Ok(Json(emoji).into_response())
        }
        Created::Invalid(messages) => Ok(failed(messages)),
    }
}

/// DELETE /admin/config/emoji/:id(.json): CustomEmoji::Destroy.
pub async fn destroy(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: axum::http::Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if !guardian.is_admin() {
        return Ok(super::topics::not_found_response(&state));
    }
    let p = crate::params::parse(uri.query(), &headers, &body);
    let form: Vec<(String, String)> = p
        .iter()
        .filter_map(|(k, v)| crate::params::scalar(v).map(|v| (k.clone(), v)))
        .collect();
    if !csrf_ok(&state, &headers, &form, uri.path(), "DELETE") {
        return Ok(bad_csrf());
    }
    let admin_id = guardian
        .user_id()
        .ok_or(Unsupported("an admin without a user"))?;
    let name = id.strip_suffix(".json").unwrap_or(&id);
    let mut tx = state.pool.begin().await?;
    custom_emoji::destroy(&mut tx, admin_id, name).await?;
    tx.commit().await?;
    Ok(Json(json!({ "success": "OK" })).into_response())
}

/// GET /site/emoji(.json): `Emoji.custom` as EmojiSerializer gives them.
pub async fn site_emoji(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    Ok(Json(settings.emojis().custom_json()))
}
