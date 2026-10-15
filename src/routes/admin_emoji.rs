//! Admin::EmojiController (index, create, destroy) under
//! /admin/config/emoji, admins only (AdminConstraint: 404 for others),
//! and SiteController#emoji (GET /site/emoji).
//!
//! Export, import preview and confirm (crate::custom_emoji_import) too.
//!
//! Refused: the Ember pages (an HTML request).

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
            for_export: false,
            for_theme: false,
            gravatar_origin: None,
            skip_validations: false,
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

fn failed_with(state: &AppState, status: StatusCode, key: &str) -> Response {
    let message = state.i18n.t(key).unwrap_or(key).to_string();
    (
        status,
        Json(json!({ "failed": "FAILED", "errors": [message] })),
    )
        .into_response()
}

/// The request's params with their CSRF check: None when it fails.
fn params_with_csrf(
    state: &AppState,
    headers: &HeaderMap,
    uri: &axum::http::Uri,
    body: &Bytes,
) -> Option<serde_json::Map<String, Value>> {
    let p = crate::params::parse(uri.query(), headers, body);
    let form: Vec<(String, String)> = p
        .iter()
        .filter_map(|(k, v)| crate::params::scalar(v).map(|v| (k.clone(), v)))
        .collect();
    csrf_ok(state, headers, &form, uri.path(), "POST").then_some(p)
}

/// POST /admin/config/emoji/export(.json): CustomEmoji::Export, the zip
/// sent as `emojis.zip`.
pub async fn export(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: axum::http::Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if !guardian.is_admin() {
        return Ok(super::topics::not_found_response(&state));
    }
    let Some(p) = params_with_csrf(&state, &headers, &uri, &body) else {
        return Ok(bad_csrf());
    };
    // `names.to_a.map(&:to_s)`
    let names: Vec<String> = match p.get("names") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(list)) => list
            .iter()
            .map(|v| {
                crate::params::scalar(v).ok_or(Unsupported("export names that are lists or hashes"))
            })
            .collect::<Result<_, _>>()?,
        Some(_) => {
            return Err(
                Unsupported("export names that are not a list (NoMethodError in Rails)").into(),
            );
        }
    };
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let store = crate::file_store::FileStore::for_site(&state.config, &settings)?;
    Ok(
        match crate::custom_emoji_import::export(&mut conn, &store, &names).await? {
            crate::custom_emoji_import::Exported::NoSelection => failed_with(
                &state,
                StatusCode::UNPROCESSABLE_ENTITY,
                "emoji.export.no_selection",
            ),
            crate::custom_emoji_import::Exported::NotFound => {
                (StatusCode::NOT_FOUND, Json(json!({ "failed": "FAILED" }))).into_response()
            }
            crate::custom_emoji_import::Exported::Zip(bytes) => (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, "application/zip"),
                    (
                        header::CONTENT_DISPOSITION,
                        "attachment; filename=\"emojis.zip\"; filename*=UTF-8''emojis.zip",
                    ),
                ],
                bytes,
            )
                .into_response(),
        },
    )
}

/// POST /admin/config/emoji/import_preview(.json): CustomEmoji::PreviewImport
/// over the uploaded zip.
pub async fn import_preview(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    request: Request,
) -> Result<Response, AppError> {
    use crate::custom_emoji_import::{Previewed, preview};
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
    let Some((_, bytes)) = &form.file else {
        return Ok(failed_with(
            &state,
            StatusCode::UNPROCESSABLE_ENTITY,
            "emoji.import.missing_file",
        ));
    };
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let store = crate::file_store::FileStore::for_site(&state.config, &settings)?;
    Ok(
        match preview(
            &mut conn,
            &settings,
            &state.i18n,
            &urls,
            &store,
            admin_id,
            bytes,
        )
        .await?
        {
            Previewed::Refused(e) => {
                failed_with(&state, StatusCode::UNPROCESSABLE_ENTITY, e.message_key())
            }
            Previewed::EmptyManifest => failed_with(
                &state,
                StatusCode::UNPROCESSABLE_ENTITY,
                "emoji.import.empty_manifest",
            ),
            Previewed::Rows { token, rows } => Json(json!({
                "token": token,
                "rows": rows.iter().map(|r| r.to_json()).collect::<Vec<_>>(),
            }))
            .into_response(),
        },
    )
}

/// POST /admin/config/emoji/import_confirm(.json): CustomEmoji::ConfirmImport.
pub async fn import_confirm(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: axum::http::Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    use crate::custom_emoji_import::{Confirmed, confirm};
    if !guardian.is_admin() {
        return Ok(super::topics::not_found_response(&state));
    }
    let Some(p) = params_with_csrf(&state, &headers, &uri, &body) else {
        return Ok(bad_csrf());
    };
    let admin_id = guardian
        .user_id()
        .ok_or(Unsupported("an admin without a user"))?;
    let token = match p.get("token") {
        None | Some(Value::Null) => String::new(),
        Some(v) => {
            crate::params::scalar(v).ok_or(Unsupported("an import token that isn't a string"))?
        }
    };
    if crate::ruby::is_blank(&token) {
        let blank = state
            .i18n
            .t("errors.messages.blank")
            .unwrap_or("can't be blank");
        return Ok(failed(vec![format!("Token {blank}")]));
    }
    let resolutions = match p.get("resolutions") {
        None | Some(Value::Null) => serde_json::Map::new(),
        Some(Value::Object(m)) => m.clone(),
        Some(_) => return Err(Unsupported("import resolutions that are not a hash").into()),
    };
    let mut tx = state.pool.begin().await?;
    Ok(
        match confirm(&mut tx, admin_id, &token, &resolutions).await? {
            Confirmed::Expired => failed_with(
                &state,
                StatusCode::UNPROCESSABLE_ENTITY,
                "emoji.import.session_expired",
            ),
            Confirmed::Report(report) => {
                tx.commit().await?;
                Json(report).into_response()
            }
        },
    )
}

/// GET /site/emoji(.json): `Emoji.custom` as EmojiSerializer gives them.
pub async fn site_emoji(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    Ok(Json(settings.emojis().custom_json()))
}
