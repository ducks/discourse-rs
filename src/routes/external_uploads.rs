//! ExternalUploadHelpers and ExternalUploadManager: files a browser puts
//! straight into the external store, for UploadsController (uploads) and
//! Admin::BackupsController (backups). A presigned PUT or a multipart
//! upload at a temporary key, an ExternalUploadStub row to find it by,
//! then completion: the object checked against the stub and made an
//! upload (moved where it belongs, or stored again when processing changed
//! it), or moved into the backups' folder.
//!
//! The stores do the object work (`FileStore::External`,
//! `BackupStore::External`); this module only knows that they are
//! external.

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use super::session::{bad_csrf, csrf_ok};
use crate::backup_restore::store::{BackupStore, BackupStoreError, ExternalBackupStore};
use crate::config::RailsEnv;
use crate::file_store::{ExternalStore, FileStore};
use crate::guardian::Guardian;
use crate::object_storage::{Backend, Part, StorageError};
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

/// The controller the helpers are included in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Controller {
    Uploads,
    Backups,
}

/// `ExternalUploadManager::DOWNLOAD_LIMIT`
const DOWNLOAD_LIMIT: u64 = 100 * 1024 * 1024;
/// `ExternalUploadManager::SIZE_MISMATCH_BAN_MINUTES`
const SIZE_MISMATCH_BAN_SECONDS: i64 = 5 * 60;
const BAN_USER_REDIS_PREFIX: &str = "ban_user_from_external_uploads_";

/// The store a stub's upload type goes to (`store_for_upload_type`).
enum Target {
    Uploads(FileStore),
    Backups(Box<ExternalBackupStore>),
}

impl Target {
    fn backend(&self) -> &Backend {
        match self {
            Target::Uploads(store) => external(store).backend(),
            Target::Backups(store) => store.backend(),
        }
    }

    /// `@store.delete_file(key)`
    async fn delete_file(&self, key: &str) -> Result<(), AppError> {
        self.backend().ensure_bucket().await?;
        Ok(self.backend().delete(key).await?)
    }
}

fn external(store: &FileStore) -> &ExternalStore {
    store
        .as_external()
        .expect("the external store check ran before")
}

/// Why `store_for_upload_type` refused.
struct InvalidAccess;

/// `ExternalUploadManager.store_for_upload_type`: a backup only for an
/// admin, with backups on and kept on S3; anything else the site's store.
fn store_for_upload_type(
    state: &AppState,
    settings: &SiteSettings,
    upload_type: &str,
    is_admin: bool,
) -> Result<Result<Target, InvalidAccess>, AppError> {
    if upload_type == "backup" {
        if !is_admin
            || !settings.get("enable_backups")?.truthy()
            || settings.get("backup_location")?.to_s() != "s3"
        {
            return Ok(Err(InvalidAccess));
        }
        return match BackupStore::create(&state.config, settings)? {
            BackupStore::External(store) => Ok(Ok(Target::Backups(store))),
            BackupStore::Local(_) => Err(Unsupported("a backup store neither local nor S3").into()),
        };
    }
    Ok(Ok(Target::Uploads(FileStore::for_site(
        &state.config,
        settings,
    )?)))
}

/// One ExternalUploadStub.
#[derive(sqlx::FromRow)]
struct Stub {
    id: i64,
    key: String,
    original_filename: String,
    status: i32,
    created_by_id: i32,
    upload_type: String,
    filesize: i64,
    external_upload_identifier: Option<String>,
}

const STUB_COLUMNS: &str = "id, key, original_filename, status, created_by_id, upload_type, \
                            filesize, external_upload_identifier";

/// `ExternalUploadStub.find_by(unique_identifier:, created_by:)`: a value
/// that is no uuid finds nothing, as ActiveRecord casts it to nil.
async fn stub_by_identifier(
    conn: &mut PgConnection,
    unique_identifier: &str,
    user_id: i32,
) -> Result<Option<Stub>, AppError> {
    let uuid_like = unique_identifier.len() == 36
        && unique_identifier
            .chars()
            .all(|c| c.is_ascii_hexdigit() || c == '-');
    if !uuid_like {
        return Ok(None);
    }
    Ok(sqlx::query_as(&format!(
        "SELECT {STUB_COLUMNS} FROM external_upload_stubs \
         WHERE unique_identifier = $1::uuid AND created_by_id = $2 LIMIT 1"
    ))
    .bind(unique_identifier)
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?)
}

async fn destroy_stub(conn: &mut PgConnection, id: i64) -> Result<(), AppError> {
    sqlx::query("DELETE FROM external_upload_stubs WHERE id = $1")
        .bind(id)
        .execute(conn)
        .await?;
    Ok(())
}

/// `render_json_error(message, status:)`
fn json_error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "errors": [message] }))).into_response()
}

fn t(state: &AppState, key: &str) -> String {
    state.i18n.t(key).unwrap_or(key).to_string()
}

/// `debug_upload_error(err, friendly_message)`: the generic failure unless
/// enable_upload_debug_mode, and then the friendly one in development
/// and test or for the API.
fn debug_upload_error(
    state: &AppState,
    settings: &SiteSettings,
    headers: &HeaderMap,
    err: &dyn std::fmt::Display,
    key: &str,
    detail: &str,
) -> Result<String, AppError> {
    if !settings.get("enable_upload_debug_mode")?.truthy() {
        return Ok(t(state, "upload.failed"));
    }
    let friendly = state
        .i18n
        .t_with(key, &[("additional_detail", detail)])
        .unwrap_or_default();
    tracing::warn!("[ExternalUploadError] {friendly}: {err}");
    let api = headers.contains_key(crate::session::api_key::HEADER_API_KEY)
        || headers.contains_key(crate::session::user_api_key::HEADER_USER_API_KEY);
    Ok(
        if matches!(
            state.config.rails_env,
            RailsEnv::Development | RailsEnv::Test
        ) || api
        {
            friendly
        } else {
            t(state, "upload.failed")
        },
    )
}

/// The request: its params, CSRF checked; the settings; the user.
struct Request {
    params: Map<String, Value>,
    settings: SiteSettings,
    user_id: i32,
}

/// The before actions, in the order Rails runs them: the controller's
/// own constraint (backups: admins), external_store_check,
/// direct_s3_uploads_check, can_upload_external? (creating only), then
/// requires_login (uploads) or ensure_backups_enabled (backups).
async fn before(
    state: &AppState,
    guardian: &Guardian,
    controller: Controller,
    creating: bool,
    headers: &HeaderMap,
    uri: &Uri,
    body: &[u8],
) -> Result<Result<Request, Response>, AppError> {
    if controller == Controller::Backups && !guardian.is_admin() {
        return Ok(Err(super::topics::not_found_response(state)));
    }
    let params = crate::params::parse(uri.query(), headers, body);
    let pairs: Vec<(String, String)> = params
        .iter()
        .filter_map(|(k, v)| crate::params::scalar(v).map(|v| (k.clone(), v)))
        .collect();
    if !csrf_ok(state, headers, &pairs, uri.path(), "POST") {
        return Ok(Err(bad_csrf()));
    }
    let settings = {
        let mut conn = state.pool.acquire().await?;
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?
    };
    if FileStore::for_site(&state.config, &settings)?.internal() {
        return Ok(Err(super::topics::not_found_response(state)));
    }
    if !settings.get("enable_direct_s3_uploads")?.truthy() {
        return Ok(Err(super::topics::not_found_response(state)));
    }
    if creating && let Some(user_id) = guardian.user_id() {
        let mut conn = state.pool.acquire().await?;
        let banned = crate::owned_schema::cached_get(
            &mut conn,
            &format!("{BAN_USER_REDIS_PREFIX}{user_id}"),
        )
        .await?;
        if banned.as_deref() == Some("1") {
            return Ok(Err(super::search::invalid_access(state)));
        }
    }
    let Some(user_id) = guardian.user_id() else {
        return Ok(Err(super::login_required::not_logged_in(state)));
    };
    if controller == Controller::Backups && !settings.get("enable_backups")?.truthy() {
        return Ok(Err(super::search::invalid_access(state)));
    }
    Ok(Ok(Request {
        params,
        settings,
        user_id,
    }))
}

/// `params.require(key)`: present and not blank.
fn require(params: &Map<String, Value>, key: &str) -> Option<Value> {
    match params.get(key) {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if s.is_empty() => None,
        Some(Value::Array(a)) if a.is_empty() => None,
        Some(Value::Object(o)) if o.is_empty() => None,
        Some(v) => Some(v.clone()),
    }
}

fn text(value: &Value) -> String {
    crate::params::scalar(value).unwrap_or_default()
}

/// A RateLimiter's `performed!` for the user.
async fn rate_limit(
    state: &AppState,
    guardian: &Guardian,
    settings: &SiteSettings,
    key: &str,
    setting: &str,
) -> Result<Option<Response>, AppError> {
    let limiter = crate::rate_limiter::RateLimiter::new(
        guardian.user_id().map(|id| crate::rate_limiter::LimitUser {
            id,
            staff: guardian.is_staff(),
        }),
        key,
        settings.get(setting)?.to_i(),
        60,
    );
    Ok(limiter
        .performed(&state.pool, state.config.rate_limits)
        .await?
        .err()
        .map(|refused| refused.into_response(&state.i18n)))
}

/// `parse_allowed_metadata`: only `sha1-checksum`.
fn allowed_metadata(params: &Map<String, Value>) -> Vec<(String, String)> {
    params
        .get("metadata")
        .and_then(Value::as_object)
        .and_then(|m| m.get("sha1-checksum"))
        .and_then(crate::params::scalar)
        .map(|v| vec![("sha1-checksum".to_string(), v)])
        .unwrap_or_default()
}

/// `validate_part_number`: 1 to 10000, else InvalidParameters.
fn part_number(value: &Value) -> Option<u16> {
    let n = crate::ruby::to_i(&text(value));
    (1..=10_000).contains(&n).then_some(n as u16)
}

/// The size checks before an upload is created: UploadsController's
/// `validate_file_size`, BackupsController's extension and name.
async fn validate_before_create(
    state: &AppState,
    conn: &mut PgConnection,
    settings: &SiteSettings,
    controller: Controller,
    user_id: i32,
    file_name: &str,
    file_size: i64,
) -> Result<Option<String>, AppError> {
    match controller {
        Controller::Backups => {
            if !crate::backup_restore::store::valid_extension(file_name) {
                return Ok(Some(t(state, "backup.backup_file_should_be_tar_gz")));
            }
            if !crate::backup_restore::store::valid_filename(file_name) {
                return Ok(Some(t(state, "backup.invalid_filename")));
            }
            Ok(None)
        }
        Controller::Uploads => {
            if file_size == 0 {
                return Ok(Some(t(state, "upload.size_zero_failure")));
            }
            let _ = conn;
            let image = crate::uploads::is_supported_image(file_name);
            // max_attachment_size_for_user
            let system_max = settings.get("system_user_max_attachment_size_kb")?.to_i();
            let max_attachment = if user_id == -1 && system_max != 0 {
                system_max
            } else {
                settings.get("max_attachment_size_kb")?.to_i()
            };
            if !image && file_size >= max_attachment * 1024 {
                return Ok(Some(
                    state
                        .i18n
                        .t_with(
                            "upload.attachments.too_large_humanized",
                            &[(
                                "max_size",
                                &crate::uploads::human_size(max_attachment * 1024),
                            )],
                        )
                        .unwrap_or_default(),
                ));
            }
            let max_image = settings.get("max_image_size_kb")?.to_i();
            if image
                && crate::file_store::extname(file_name) == ".gif"
                && file_size >= max_image * 1024
            {
                return Ok(Some(
                    state
                        .i18n
                        .t_with(
                            "upload.images.too_large_humanized",
                            &[("max_size", &crate::uploads::human_size(max_image * 1024))],
                        )
                        .unwrap_or_default(),
                ));
            }
            Ok(None)
        }
    }
}

/// POST /uploads/generate-presigned-put: a temporary key, a PUT presigned
/// for it, and the stub to complete it by.
pub async fn generate_presigned_put(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let req = match before(
        &state,
        &guardian,
        Controller::Uploads,
        true,
        &headers,
        &uri,
        &body,
    )
    .await?
    {
        Ok(req) => req,
        Err(response) => return Ok(response),
    };
    if let Some(r) = rate_limit(
        &state,
        &guardian,
        &req.settings,
        "generate-presigned-put-upload-stub",
        "max_presigned_put_per_minute",
    )
    .await?
    {
        return Ok(r);
    }
    let mut fetched = Vec::new();
    for key in ["file_name", "file_size", "type"] {
        match require(&req.params, key) {
            Some(v) => fetched.push(text(&v)),
            None => return Ok(super::accounts::param_missing(key)),
        }
    }
    let (file_name, file_size, upload_type) = (
        fetched[0].clone(),
        crate::ruby::to_i(&fetched[1]),
        fetched[2].clone(),
    );
    let mut conn = state.pool.acquire().await?;
    if let Some(message) = validate_before_create(
        &state,
        &mut conn,
        &req.settings,
        Controller::Uploads,
        req.user_id,
        &file_name,
        file_size,
    )
    .await?
    {
        return Ok(json_error(StatusCode::UNPROCESSABLE_ENTITY, &message));
    }
    // ExternalUploadManager.create_direct_upload
    let target =
        match store_for_upload_type(&state, &req.settings, &upload_type, guardian.is_admin())? {
            Ok(target) => target,
            Err(InvalidAccess) => return Ok(super::search::invalid_access(&state)),
        };
    let Target::Uploads(store) = &target else {
        // A backup store has no signed_request_for_temporary_upload.
        return Err(Unsupported("presigned PUTs of backups").into());
    };
    let (url, signed_headers) = external(store)
        .signed_request_for_temporary_upload(&file_name, allowed_metadata(&req.params))?;
    // s3_helper.path_from_url(url): the URL's path.
    let key = url
        .split_once("://")
        .map(|(_, rest)| rest)
        .and_then(|rest| rest.find('/').map(|i| &rest[i + 1..]))
        .unwrap_or_default()
        .split('?')
        .next()
        .unwrap_or_default()
        .to_string();
    let unique_identifier: String = sqlx::query_scalar(
        "INSERT INTO external_upload_stubs (key, created_by_id, original_filename, upload_type, \
                                            filesize, status, unique_identifier, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, 1, gen_random_uuid(), clock_timestamp(), clock_timestamp()) \
         RETURNING unique_identifier::text",
    )
    .bind(&key)
    .bind(req.user_id)
    .bind(&file_name)
    .bind(&upload_type)
    .bind(file_size)
    .fetch_one(&mut *conn)
    .await?;
    Ok(Json(json!({
        "url": url,
        "key": key,
        "unique_identifier": unique_identifier,
        "signed_headers": signed_headers,
    }))
    .into_response())
}

/// POST /uploads/complete-external-upload: the stub's object made an
/// upload.
pub async fn complete_external_upload(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let req = match before(
        &state,
        &guardian,
        Controller::Uploads,
        false,
        &headers,
        &uri,
        &body,
    )
    .await?
    {
        Ok(req) => req,
        Err(response) => return Ok(response),
    };
    let Some(identifier) = require(&req.params, "unique_identifier") else {
        return Ok(super::accounts::param_missing("unique_identifier"));
    };
    let mut conn = state.pool.acquire().await?;
    let Some(stub) = stub_by_identifier(&mut conn, &text(&identifier), req.user_id).await? else {
        return Ok(super::topics::not_found_response(&state));
    };
    drop(conn);
    complete_via_manager(&state, &req, &headers, Controller::Uploads, stub).await
}

/// POST /uploads/create-multipart and /admin/backups/create-multipart: a
/// multipart upload at a temporary key, and its stub.
pub async fn create_multipart(
    state: &AppState,
    guardian: &Guardian,
    controller: Controller,
    headers: &HeaderMap,
    uri: &Uri,
    body: &[u8],
) -> Result<Response, AppError> {
    let req = match before(state, guardian, controller, true, headers, uri, body).await? {
        Ok(req) => req,
        Err(response) => return Ok(response),
    };
    if let Some(r) = rate_limit(
        state,
        guardian,
        &req.settings,
        "create-multipart-upload",
        "max_create_multipart_per_minute",
    )
    .await?
    {
        return Ok(r);
    }
    let mut fetched = Vec::new();
    for key in ["file_name", "file_size", "upload_type"] {
        match require(&req.params, key) {
            Some(v) => fetched.push(text(&v)),
            None => return Ok(super::accounts::param_missing(key)),
        }
    }
    let (file_name, file_size, upload_type) = (
        fetched[0].clone(),
        crate::ruby::to_i(&fetched[1]),
        fetched[2].clone(),
    );
    let mut conn = state.pool.acquire().await?;
    if let Some(message) = validate_before_create(
        state,
        &mut conn,
        &req.settings,
        controller,
        req.user_id,
        &file_name,
        file_size,
    )
    .await?
    {
        return Ok(json_error(StatusCode::UNPROCESSABLE_ENTITY, &message));
    }
    // ExternalUploadManager.create_direct_multipart_upload
    let content_type = crate::file_store::mime_type(&file_name);
    let target =
        match store_for_upload_type(state, &req.settings, &upload_type, guardian.is_admin())? {
            Ok(target) => target,
            Err(InvalidAccess) => return Ok(super::search::invalid_access(state)),
        };
    let metadata = allowed_metadata(&req.params);
    let created: Result<(String, String), AppError> = match &target {
        Target::Uploads(store) => external(store)
            .create_multipart(&file_name, content_type, metadata)
            .await
            .map_err(AppError::from),
        Target::Backups(store) => {
            store
                .create_multipart(&file_name, content_type, metadata)
                .await
        }
    };
    let (upload_id, key) = match created {
        Ok(created) => created,
        Err(e) => {
            // create_direct_multipart_upload's rescues.
            if let Some(BackupStoreError::FileExists) = e.downcast_ref::<BackupStoreError>() {
                return Ok(json_error(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    &t(state, "backup.file_exists"),
                ));
            }
            let service = e.downcast_ref::<crate::file_store::StoreError>().is_some()
                || e.downcast_ref::<StorageError>().is_some();
            if service && !e.is_unsupported() {
                let message = debug_upload_error(
                    state,
                    &req.settings,
                    headers,
                    &e,
                    "upload.create_multipart_failure",
                    &e.to_string(),
                )?;
                return Ok(json_error(StatusCode::UNPROCESSABLE_ENTITY, &message));
            }
            return Err(e);
        }
    };
    let unique_identifier: String = sqlx::query_scalar(
        "INSERT INTO external_upload_stubs (key, created_by_id, original_filename, upload_type, \
                                            external_upload_identifier, multipart, filesize, status, \
                                            unique_identifier, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, TRUE, $6, 1, gen_random_uuid(), clock_timestamp(), clock_timestamp()) \
         RETURNING unique_identifier::text",
    )
    .bind(&key)
    .bind(req.user_id)
    .bind(&file_name)
    .bind(&upload_type)
    .bind(&upload_id)
    .bind(file_size)
    .fetch_one(&mut *conn)
    .await?;
    Ok(Json(json!({
        "external_upload_identifier": upload_id,
        "key": key,
        "unique_identifier": unique_identifier,
    }))
    .into_response())
}

/// The stub's store, as its creator's guardian gets it.
async fn stub_target(
    state: &AppState,
    settings: &SiteSettings,
    stub: &Stub,
) -> Result<Result<Target, InvalidAccess>, AppError> {
    let mut conn = state.pool.acquire().await?;
    let admin: bool =
        sqlx::query_scalar("SELECT COALESCE((SELECT admin FROM users WHERE id = $1), FALSE)")
            .bind(stub.created_by_id)
            .fetch_one(&mut *conn)
            .await?;
    store_for_upload_type(state, settings, &stub.upload_type, admin)
}

/// `check_multipart_upload_exists`: the upload's parts listed; gone, a
/// 404 with the (debug) message.
async fn check_multipart_upload_exists(
    state: &AppState,
    req: &Request,
    headers: &HeaderMap,
    target: &Target,
    stub: &Stub,
) -> Result<Option<Response>, AppError> {
    let upload_id = stub.external_upload_identifier.clone().unwrap_or_default();
    match target.backend().list_parts(&stub.key, &upload_id, 1).await {
        Ok(_) => Ok(None),
        Err(StorageError::NoSuchUpload) => {
            let message = debug_upload_error(
                state,
                &req.settings,
                headers,
                &StorageError::NoSuchUpload,
                "upload.external_upload_not_found",
                &format!("path: {}", stub.key),
            )?;
            Ok(Some(json_error(StatusCode::NOT_FOUND, &message)))
        }
        Err(e) => Err(e.into()),
    }
}

/// POST .../batch-presign-multipart-parts: a URL for each part.
pub async fn batch_presign_multipart_parts(
    state: &AppState,
    guardian: &Guardian,
    controller: Controller,
    headers: &HeaderMap,
    uri: &Uri,
    body: &[u8],
) -> Result<Response, AppError> {
    let req = match before(state, guardian, controller, false, headers, uri, body).await? {
        Ok(req) => req,
        Err(response) => return Ok(response),
    };
    let Some(part_numbers) = require(&req.params, "part_numbers") else {
        return Ok(super::accounts::param_missing("part_numbers"));
    };
    let Some(identifier) = require(&req.params, "unique_identifier") else {
        return Ok(super::accounts::param_missing("unique_identifier"));
    };
    if let Some(r) = rate_limit(
        state,
        guardian,
        &req.settings,
        "batch-presign",
        "max_batch_presign_multipart_per_minute",
    )
    .await?
    {
        return Ok(r);
    }
    let raw: Vec<Value> = match part_numbers {
        Value::Array(a) => a,
        Value::Object(o) => o.into_values().collect(),
        other => vec![other],
    };
    let mut numbers = Vec::new();
    for value in &raw {
        match part_number(value) {
            Some(n) => numbers.push(n),
            None => {
                return Ok(super::search::invalid_parameters(
                    state,
                    "Each part number should be between 1 and 10000",
                ));
            }
        }
    }
    let mut conn = state.pool.acquire().await?;
    let Some(stub) = stub_by_identifier(&mut conn, &text(&identifier), req.user_id).await? else {
        return Ok(super::topics::not_found_response(state));
    };
    drop(conn);
    let target = match stub_target(state, &req.settings, &stub).await? {
        Ok(target) => target,
        Err(InvalidAccess) => return Ok(super::search::invalid_access(state)),
    };
    if let Some(r) = check_multipart_upload_exists(state, &req, headers, &target, &stub).await? {
        return Ok(r);
    }
    let upload_id = stub.external_upload_identifier.clone().unwrap_or_default();
    let mut urls = Map::new();
    for n in numbers {
        let url = target.backend().presign_part(
            &stub.key,
            &upload_id,
            n,
            crate::file_store::UPLOAD_URL_EXPIRES_AFTER_SECONDS,
        )?;
        urls.insert(n.to_string(), Value::String(url));
    }
    Ok(Json(json!({ "presigned_urls": urls })).into_response())
}

/// POST .../abort-multipart: the upload abandoned and its stub gone (a
/// stub already gone is fine).
pub async fn abort_multipart(
    state: &AppState,
    guardian: &Guardian,
    controller: Controller,
    headers: &HeaderMap,
    uri: &Uri,
    body: &[u8],
) -> Result<Response, AppError> {
    let req = match before(state, guardian, controller, false, headers, uri, body).await? {
        Ok(req) => req,
        Err(response) => return Ok(response),
    };
    let Some(identifier) = require(&req.params, "external_upload_identifier") else {
        return Ok(super::accounts::param_missing("external_upload_identifier"));
    };
    let mut conn = state.pool.acquire().await?;
    let stub: Option<Stub> = sqlx::query_as(&format!(
        "SELECT {STUB_COLUMNS} FROM external_upload_stubs \
         WHERE external_upload_identifier = $1 LIMIT 1"
    ))
    .bind(text(&identifier))
    .fetch_optional(&mut *conn)
    .await?;
    let Some(stub) = stub else {
        return Ok(Json(json!({ "success": "OK" })).into_response());
    };
    if stub.created_by_id != req.user_id {
        return Ok(super::topics::not_found_response(state));
    }
    let target = match stub_target(state, &req.settings, &stub).await? {
        Ok(target) => target,
        Err(InvalidAccess) => return Ok(super::search::invalid_access(state)),
    };
    let upload_id = stub.external_upload_identifier.clone().unwrap_or_default();
    if let Err(e) = target
        .backend()
        .abort_multipart(&stub.key, &upload_id)
        .await
    {
        if let StorageError::Unsupported(u) = e {
            return Err(u.into());
        }
        let message = debug_upload_error(
            state,
            &req.settings,
            headers,
            &e,
            "upload.abort_multipart_failure",
            &format!("external upload stub id: {}", stub.id),
        )?;
        return Ok(json_error(StatusCode::UNPROCESSABLE_ENTITY, &message));
    }
    destroy_stub(&mut conn, stub.id).await?;
    Ok(Json(json!({ "success": "OK" })).into_response())
}

/// POST .../complete-multipart: the parts put together, then completed
/// as an upload (or a backup).
pub async fn complete_multipart(
    state: &AppState,
    guardian: &Guardian,
    controller: Controller,
    headers: &HeaderMap,
    uri: &Uri,
    body: &[u8],
) -> Result<Response, AppError> {
    let req = match before(state, guardian, controller, false, headers, uri, body).await? {
        Ok(req) => req,
        Err(response) => return Ok(response),
    };
    let Some(identifier) = require(&req.params, "unique_identifier") else {
        return Ok(super::accounts::param_missing("unique_identifier"));
    };
    let Some(parts) = require(&req.params, "parts") else {
        return Ok(super::accounts::param_missing("parts"));
    };
    if let Some(r) = rate_limit(
        state,
        guardian,
        &req.settings,
        "complete-multipart-upload",
        "max_complete_multipart_per_minute",
    )
    .await?
    {
        return Ok(r);
    }
    let mut conn = state.pool.acquire().await?;
    let Some(stub) = stub_by_identifier(&mut conn, &text(&identifier), req.user_id).await? else {
        return Ok(super::topics::not_found_response(state));
    };
    drop(conn);
    let target = match stub_target(state, &req.settings, &stub).await? {
        Ok(target) => target,
        Err(InvalidAccess) => return Ok(super::search::invalid_access(state)),
    };
    if let Some(r) = check_multipart_upload_exists(state, &req, headers, &target, &stub).await? {
        return Ok(r);
    }
    let raw: Vec<Value> = match parts {
        Value::Array(a) => a,
        Value::Object(o) => o.into_values().collect(),
        other => vec![other],
    };
    let mut list = Vec::new();
    for part in &raw {
        let Some(number) = part_number(&part["part_number"]) else {
            return Ok(super::search::invalid_parameters(
                state,
                "Each part number should be between 1 and 10000",
            ));
        };
        let etag = text(&part["etag"]);
        if crate::ruby::is_blank(&etag) {
            return Ok(super::search::invalid_parameters(
                state,
                "All parts must have an etag and a valid part number",
            ));
        }
        list.push(Part { number, etag });
    }
    list.sort_by_key(|p| p.number);
    let upload_id = stub.external_upload_identifier.clone().unwrap_or_default();
    if let Err(e) = target
        .backend()
        .complete_multipart(&stub.key, &upload_id, &list)
        .await
    {
        if let StorageError::Unsupported(u) = e {
            return Err(u.into());
        }
        let message = debug_upload_error(
            state,
            &req.settings,
            headers,
            &e,
            "upload.complete_multipart_failure",
            &format!("external upload stub id: {}", stub.id),
        )?;
        return Ok(json_error(StatusCode::UNPROCESSABLE_ENTITY, &message));
    }
    complete_via_manager(state, &req, headers, controller, stub).await
}

/// How `transform!` failed.
enum Failure {
    CannotPromote,
    SizeMismatch(String),
    ChecksumMismatch,
    DownloadFailed(String),
    Other(AppError),
}

/// `complete_external_upload_via_manager`: ExternalUploadManager's
/// `transform!`, then the upload serialized (`{}` for a backup) and the
/// stub gone; a failure is a 422 (the stub and object gone too, unless
/// enable_upload_debug_mode, which marks the stub failed instead).
async fn complete_via_manager(
    state: &AppState,
    req: &Request,
    headers: &HeaderMap,
    controller: Controller,
    stub: Stub,
) -> Result<Response, AppError> {
    let p = &req.params;
    let flag = |key: &str| p.get(key).map(text).as_deref() == Some("true");
    if flag("for_site_setting") {
        return Err(Unsupported("site setting uploads").into());
    }
    // ExternalUploadManager.new: the store for its creator.
    let target = match stub_target(state, &req.settings, &stub).await? {
        Ok(target) => target,
        Err(InvalidAccess) => return Ok(super::search::invalid_access(state)),
    };
    let result = transform(
        state,
        &req.settings,
        &target,
        &stub,
        flag("for_private_message"),
        flag("pasted"),
    )
    .await;
    let mut conn = state.pool.acquire().await?;
    let failure = match result {
        Ok(Some(crate::uploads::Outcome::Created(upload))) => {
            destroy_stub(&mut conn, stub.id).await?;
            return Ok(Json(upload).into_response());
        }
        // Errors on the upload: rendered, the stub kept.
        Ok(Some(crate::uploads::Outcome::Invalid(errors))) => {
            return Ok((
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({ "errors": errors })),
            )
                .into_response());
        }
        // A backup: BackupsController.serialize_upload is `{}`.
        Ok(None) => {
            destroy_stub(&mut conn, stub.id).await?;
            let _ = controller;
            return Ok(Json(json!({})).into_response());
        }
        Err(Failure::Other(e)) if e.is_unsupported() => return Err(e),
        Err(failure) => failure,
    };
    // transform!'s rescue.
    if req.settings.get("enable_upload_debug_mode")?.truthy() {
        sqlx::query("UPDATE external_upload_stubs SET status = 3, updated_at = clock_timestamp() WHERE id = $1")
            .bind(stub.id)
            .execute(&mut *conn)
            .await?;
    } else {
        target.delete_file(&stub.key).await?;
        destroy_stub(&mut conn, stub.id).await?;
    }
    let (key, detail, err): (&str, String, String) = match &failure {
        Failure::SizeMismatch(d) => ("upload.size_mismatch_failure", d.clone(), d.clone()),
        Failure::ChecksumMismatch => (
            "upload.checksum_mismatch_failure",
            String::new(),
            "ChecksumMismatchError".into(),
        ),
        Failure::CannotPromote => (
            "upload.cannot_promote_failure",
            String::new(),
            "CannotPromoteError".into(),
        ),
        Failure::DownloadFailed(d) => ("upload.download_failure", d.clone(), d.clone()),
        Failure::Other(e) => {
            tracing::error!(
                "Complete external upload failed unexpectedly for user {}: {e}",
                req.user_id
            );
            return Ok(json_error(
                StatusCode::UNPROCESSABLE_ENTITY,
                &t(state, "upload.failed"),
            ));
        }
    };
    let message = debug_upload_error(state, &req.settings, headers, &err, key, &detail)?;
    Ok(json_error(StatusCode::UNPROCESSABLE_ENTITY, &message))
}

/// ExternalUploadManager#transform!: the stub marked uploaded, its
/// object's size checked against the one announced (a mismatch bans the
/// user from external uploads for five minutes), then a backup moved into
/// place, or the file downloaded, its checksum checked and made an
/// upload with its object as the existing key.
async fn transform(
    state: &AppState,
    settings: &SiteSettings,
    target: &Target,
    stub: &Stub,
    for_private_message: bool,
    pasted: bool,
) -> Result<Option<crate::uploads::Outcome>, Failure> {
    let other = |e: AppError| Failure::Other(e);
    if stub.status != 1 {
        return Err(Failure::CannotPromote);
    }
    let mut conn = state.pool.acquire().await.map_err(|e| other(e.into()))?;
    sqlx::query(
        "UPDATE external_upload_stubs SET status = 2, updated_at = clock_timestamp() WHERE id = $1",
    )
    .bind(stub.id)
    .execute(&mut *conn)
    .await
    .map_err(|e| other(e.into()))?;
    target
        .backend()
        .ensure_bucket()
        .await
        .map_err(|e| other(e.into()))?;
    let meta = match target.backend().head(&stub.key).await {
        Ok(Some(meta)) => meta,
        // Aws::S3::Errors::NotFound, rescued as a download failure.
        Ok(None) => return Err(Failure::DownloadFailed(String::new())),
        Err(e) => return Err(other(e.into())),
    };
    let size = meta.size as i64;
    if size != stub.filesize {
        crate::owned_schema::cached_setex(
            &mut conn,
            &format!("{BAN_USER_REDIS_PREFIX}{}", stub.created_by_id),
            SIZE_MISMATCH_BAN_SECONDS,
            "1",
        )
        .await
        .map_err(|e| other(e.into()))?;
        return Err(Failure::SizeMismatch(format!(
            "expected: {}, actual: {size}",
            stub.filesize
        )));
    }
    match target {
        // move_to_final_destination
        Target::Backups(store) => {
            store
                .move_existing_stored_upload(&stub.key, &stub.original_filename)
                .await
                .map_err(other)?;
            Ok(None)
        }
        // promote_to_upload
        Target::Uploads(store) => {
            if meta.size >= DOWNLOAD_LIMIT {
                return Err(other(
                    Unsupported("promoting an external upload of 100 MB or more").into(),
                ));
            }
            // FileHelper.download of the presigned url in Rails; the
            // object itself here.
            let bytes = match target.backend().get(&stub.key).await {
                Ok(Some(bytes)) => bytes,
                Ok(None) => return Err(Failure::DownloadFailed(String::new())),
                Err(e) => return Err(Failure::DownloadFailed(e.to_string())),
            };
            if let Some(expected) = meta.metadata.get("sha1-checksum") {
                use sha1::Digest;
                let actual = format!("{:x}", sha1::Sha1::digest(&bytes));
                if *expected != actual {
                    return Err(Failure::ChecksumMismatch);
                }
            }
            let staff: bool = sqlx::query_scalar(
                "SELECT COALESCE((SELECT admin OR moderator FROM users WHERE id = $1), FALSE)",
            )
            .bind(stub.created_by_id)
            .fetch_one(&mut *conn)
            .await
            .map_err(|e| other(e.into()))?;
            let urls = crate::url::Urls {
                config: &state.config,
                settings,
            };
            let up = crate::uploads::NewUpload {
                user_id: stub.created_by_id,
                staff,
                upload_type: &stub.upload_type,
                filename: &stub.original_filename,
                bytes: &bytes,
                pasted,
                for_private_message,
                for_export: false,
                for_theme: false,
                gravatar_origin: None,
                skip_validations: false,
            };
            crate::uploads::create_with(
                &mut conn,
                settings,
                &state.i18n,
                &urls,
                store,
                &up,
                Some(&stub.key),
            )
            .await
            .map(Some)
            .map_err(other)
        }
    }
}

/// The backups controller's routes: the helpers with backups' checks.
pub mod backups {
    use super::*;

    pub async fn create_multipart(
        State(state): State<AppState>,
        AuthGuardian(guardian): AuthGuardian,
        headers: HeaderMap,
        uri: Uri,
        body: Bytes,
    ) -> Result<Response, AppError> {
        super::create_multipart(
            &state,
            &guardian,
            Controller::Backups,
            &headers,
            &uri,
            &body,
        )
        .await
    }

    pub async fn batch_presign_multipart_parts(
        State(state): State<AppState>,
        AuthGuardian(guardian): AuthGuardian,
        headers: HeaderMap,
        uri: Uri,
        body: Bytes,
    ) -> Result<Response, AppError> {
        super::batch_presign_multipart_parts(
            &state,
            &guardian,
            Controller::Backups,
            &headers,
            &uri,
            &body,
        )
        .await
    }

    pub async fn abort_multipart(
        State(state): State<AppState>,
        AuthGuardian(guardian): AuthGuardian,
        headers: HeaderMap,
        uri: Uri,
        body: Bytes,
    ) -> Result<Response, AppError> {
        super::abort_multipart(
            &state,
            &guardian,
            Controller::Backups,
            &headers,
            &uri,
            &body,
        )
        .await
    }

    pub async fn complete_multipart(
        State(state): State<AppState>,
        AuthGuardian(guardian): AuthGuardian,
        headers: HeaderMap,
        uri: Uri,
        body: Bytes,
    ) -> Result<Response, AppError> {
        super::complete_multipart(
            &state,
            &guardian,
            Controller::Backups,
            &headers,
            &uri,
            &body,
        )
        .await
    }
}

/// The uploads controller's multipart routes.
pub mod uploads {
    use super::*;

    pub async fn create_multipart(
        State(state): State<AppState>,
        AuthGuardian(guardian): AuthGuardian,
        headers: HeaderMap,
        uri: Uri,
        body: Bytes,
    ) -> Result<Response, AppError> {
        super::create_multipart(
            &state,
            &guardian,
            Controller::Uploads,
            &headers,
            &uri,
            &body,
        )
        .await
    }

    pub async fn batch_presign_multipart_parts(
        State(state): State<AppState>,
        AuthGuardian(guardian): AuthGuardian,
        headers: HeaderMap,
        uri: Uri,
        body: Bytes,
    ) -> Result<Response, AppError> {
        super::batch_presign_multipart_parts(
            &state,
            &guardian,
            Controller::Uploads,
            &headers,
            &uri,
            &body,
        )
        .await
    }

    pub async fn abort_multipart(
        State(state): State<AppState>,
        AuthGuardian(guardian): AuthGuardian,
        headers: HeaderMap,
        uri: Uri,
        body: Bytes,
    ) -> Result<Response, AppError> {
        super::abort_multipart(
            &state,
            &guardian,
            Controller::Uploads,
            &headers,
            &uri,
            &body,
        )
        .await
    }

    pub async fn complete_multipart(
        State(state): State<AppState>,
        AuthGuardian(guardian): AuthGuardian,
        headers: HeaderMap,
        uri: Uri,
        body: Bytes,
    ) -> Result<Response, AppError> {
        super::complete_multipart(
            &state,
            &guardian,
            Controller::Uploads,
            &headers,
            &uri,
            &body,
        )
        .await
    }
}
