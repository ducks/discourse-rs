//! Admin::BackupsController (AdminConstraint: everyone but admins gets the
//! routing 404): the backups list and status, emailing a download link
//! and downloading with its token, deleting, uploading in chunks, and the
//! read-only toggle. The admin pages themselves (index and logs as HTML)
//! are Ember's; direct multipart uploads to S3 are refused.

use axum::Json;
use axum::body::{Body, Bytes};
use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use crate::backup_restore;
use crate::backup_restore::store::{self, BackupStore, BackupStoreError, LocalBackupStore};
use crate::guardian::Guardian;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

/// The largest chunk request taken: the uploader sends 5 MB chunks (Rails
/// leaves the limit to the proxy in front of it).
pub const CHUNK_BODY_LIMIT: usize = 64 << 20;

fn pairs(p: &Map<String, Value>) -> Vec<(String, String)> {
    p.iter()
        .filter_map(|(k, v)| crate::params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

/// AdminConstraint: the route does not match for anyone else.
fn admin_only(state: &AppState, guardian: &Guardian) -> Option<Response> {
    (!guardian.is_admin()).then(|| super::topics::not_found_response(state))
}

/// `check_xhr`: a request that is neither XHR nor JSON renders the empty
/// app instead.
fn xhr_or_json(uri: &Uri, headers: &HeaderMap) -> bool {
    headers
        .get("x-requested-with")
        .is_some_and(|v| v == "XMLHttpRequest")
        || uri.path().ends_with(".json")
        || super::wants_json(uri.query(), headers)
}

/// `params.fetch(key)` missing: ActionController::ParameterMissing.
fn param_missing(key: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({
            "errors": [format!("param is missing or the value is empty or invalid: {key}")]
        })),
    )
        .into_response()
}

/// `render status: :unsupported_media_type, plain: I18n.t(key)`
fn unsupported_media_type(state: &AppState, key: &str) -> Response {
    let text = state.i18n.t(key).unwrap_or(key).to_string();
    (
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        text,
    )
        .into_response()
}

/// `rescue BackupStore::StorageError => e; render_json_error(e)`: a 422
/// with the remote store's message; any other error stays one.
fn storage_error(e: AppError) -> Result<Response, AppError> {
    match e.downcast_ref::<BackupStoreError>() {
        Some(BackupStoreError::Storage(message)) => Ok((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "errors": [message] })),
        )
            .into_response()),
        _ => Err(e),
    }
}

async fn settings(state: &AppState) -> Result<SiteSettings, AppError> {
    let mut conn = state.pool.acquire().await?;
    Ok(SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?)
}

/// `ensure_backups_enabled`: Discourse::InvalidAccess unless enable_backups.
fn backups_disabled(
    state: &AppState,
    settings: &SiteSettings,
) -> Result<Option<Response>, AppError> {
    Ok((!settings.get("enable_backups")?.truthy()).then(|| super::search::invalid_access(state)))
}

/// The `:id` of the member routes (RouteFormat.backup, anchored) and the
/// optional format after it; anything else does not route.
fn backup_id(raw: &str) -> Option<(String, Option<String>)> {
    static ID: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?i)^([a-zA-Z0-9._-]+\.(?:sql\.gz|tar\.gz|tgz))(?:\.([^/.?]+))?$")
            .unwrap()
    });
    let c = ID.captures(raw)?;
    Some((c[1].to_string(), c.get(2).map(|m| m.as_str().to_string())))
}

/// GET /admin/backups(.json) and /admin/backups/settings(.json): the
/// store's files (BackupFileSerializer), newest first; as a page the
/// backup files or the settings tab.
pub async fn index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if let Some(r) = admin_only(&state, &guardian) {
        return Ok(r);
    }
    if !uri.path().ends_with(".json") && !super::wants_json(uri.query(), &headers) {
        let tab = if uri.path().trim_end_matches('/').ends_with("/settings") {
            crate::admin_backups_view::Tab::Settings
        } else {
            crate::admin_backups_view::Tab::Files
        };
        return page(&state, &guardian, &headers, &uri, tab).await;
    }
    let settings = settings(&state).await?;
    let store = BackupStore::create(&state.config, &settings)?;
    match store.files().await {
        Ok(files) => Ok(Json(Value::Array(
            files.iter().map(store::BackupFile::json).collect(),
        ))
        .into_response()),
        Err(e) => storage_error(e),
    }
}

/// GET /admin/backups/logs: the logs page, with the operation's status and
/// the logs since it started.
pub async fn logs(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if let Some(r) = admin_only(&state, &guardian) {
        return Ok(r);
    }
    page(
        &state,
        &guardian,
        &headers,
        &uri,
        crate::admin_backups_view::Tab::Logs,
    )
    .await
}

/// The backups pages: what the route preloads (operations_status and the
/// logs), then the tab's content. Every tab has the header's actions,
/// which admin-backups.js handles; the settings tab's editors are
/// site-settings.js's.
async fn page(
    state: &AppState,
    guardian: &Guardian,
    headers: &HeaderMap,
    uri: &Uri,
    tab: crate::admin_backups_view::Tab,
) -> Result<Response, AppError> {
    use crate::admin_backups_view::{Status, Tab};
    let mut conn = state.pool.acquire().await?;
    let shell =
        super::admin_pages::Shell::load(&mut conn, state, guardian, headers, "admin_backups")
            .await?;
    let mut status = Status::from_json(
        &backup_restore::operations_status(state, &mut conn, &shell.settings).await?,
    );
    status.enable_backups = shell.settings.get("enable_backups")?.truthy();
    status.site_archived = shell.settings.get("site_archived")?.truthy();
    status.local_storage = shell.settings.get("backup_location")?.to_s() == "local";
    status.read_only = crate::readonly::mode(&mut conn).await?.readonly;
    let cx = shell.cx(state);
    let content = match tab {
        Tab::Files => {
            let store = BackupStore::create(&state.config, &shell.settings)?;
            let files: Vec<Value> = store
                .files()
                .await?
                .iter()
                .map(store::BackupFile::json)
                .collect();
            crate::admin_backups_view::index(&cx, &status, &files)
        }
        Tab::Logs => {
            let user_id = guardian.user_id().unwrap_or_default();
            let logs = backup_restore::logs(state, &mut conn, user_id).await?;
            crate::admin_backups_view::logs(&cx, &status, &logs)
        }
        Tab::Settings => {
            super::admin_advanced::area_settings(
                &mut conn,
                state,
                &shell,
                uri,
                json!({ "categories": "backups" }),
            )
            .await?
        }
    };
    // The strings first: their #dialog-strings (with Yes and No) is the
    // one dialog.js finds.
    let mut main = format!(
        "{}{}",
        crate::admin_backups_view::strings(&cx),
        crate::admin_backups_view::render(&cx, tab, &status, &content)
    );
    let (classes, title, script) = match tab {
        Tab::Files => ("backups", "admin.backups.title", "admin-backups.js"),
        Tab::Logs => (
            "backups logs",
            "admin.backups.menu.logs",
            "admin-backups.js",
        ),
        Tab::Settings => (
            "backups settings",
            "admin.backups.settings",
            "site-settings.js",
        ),
    };
    if tab == Tab::Settings {
        main.push_str(&format!(
            "<script src=\"{}\"></script>",
            crate::assets::url(&shell.site.base_path, "admin-backups.js")
        ));
    }
    // StartBackup's warning and checkbox, and the toggles' states.
    let s3_uploads = shell.settings.get("enable_s3_uploads")?.truthy();
    let include_s3 = shell
        .settings
        .get("include_s3_uploads_in_backups")?
        .truthy();
    let attrs = format!(
        " data-read-only=\"{}\" data-site-archived=\"{}\" data-s3-warning=\"{}\" data-can-manage-uploads=\"{}\"",
        status.read_only,
        status.site_archived,
        s3_uploads && !include_s3,
        !s3_uploads || include_s3
    );
    let main = crate::admin_ui::contents(classes, &attrs, &main);
    let mut title = crate::admin_view::t(&state.i18n, title);
    if tab != Tab::Files {
        title = format!(
            "{title} - {}",
            crate::admin_view::t(&state.i18n, "admin.backups.title")
        );
    }
    shell.render(state, uri, &title, main, script, StatusCode::OK)
}

/// GET /admin/backups/status(.json): `BackupRestore.operations_status`.
pub async fn status(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if let Some(r) = admin_only(&state, &guardian) {
        return Ok(r);
    }
    if !xhr_or_json(&uri, &headers) {
        return Ok(super::session::render_empty());
    }
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    Ok(
        Json(backup_restore::operations_status(&state, &mut conn, &settings).await?)
            .into_response(),
    )
}

/// GET /admin/backups/:id?token= (#show): the file, once, for the admin
/// the download link was mailed to.
pub async fn show(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(raw): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let (Some((id, _)), None) = (backup_id(&raw), admin_only(&state, &guardian)) else {
        return Ok(super::topics::not_found_response(&state));
    };
    let user_id = guardian.user_id().unwrap_or_default();
    let p = crate::params::parse(uri.query(), &headers, &[]);
    let Some(token) = p.get("token").and_then(crate::params::scalar) else {
        return Ok(param_missing("token"));
    };
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    if !backup_restore::token::compare(&mut conn, user_id, &token).await? {
        // admin/backups/show in the no_ember layout.
        let error = crate::category_badge::html_escape(
            state
                .i18n
                .t("download_backup_mailer.no_token")
                .unwrap_or_default(),
        );
        let html = format!(
            "<div id=\"simple-container\">\n    <div class='alert alert-error'>\n      {error}\n    </div>\n</div>\n"
        );
        let vs = super::session::viewer_state(&state, &headers, &settings, &guardian)?;
        let mut response =
            super::email::render_page(&state, &mut conn, &settings, &uri, &guardian, html, vs)
                .await?;
        *response.status_mut() = StatusCode::UNPROCESSABLE_ENTITY;
        return Ok(response);
    }
    let store = BackupStore::create(&state.config, &settings)?;
    let Some(backup) = store.file(&id, true).await? else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };
    let mut tx = state.pool.begin().await?;
    backup_restore::token::del(&mut tx, user_id).await?;
    backup_restore::log_staff_action(&mut tx, "backup_download", user_id, Some(&backup.filename))
        .await?;
    tx.commit().await?;
    match &backup.source {
        // A remote store: the presigned download.
        Some(store::Source::Url(url)) => Ok((
            StatusCode::FOUND,
            [(header::LOCATION, HeaderValue::from_str(url)?)],
        )
            .into_response()),
        Some(store::Source::File(path)) => send_file(&backup, path).await,
        None => Ok(StatusCode::NOT_FOUND.into_response()),
    }
}

/// `send_file`: the file streamed as an attachment, typed by its
/// extension as Mime::Type knows it (`.gz` only).
async fn send_file(
    backup: &store::BackupFile,
    path: &std::path::Path,
) -> Result<Response, AppError> {
    use futures_util::StreamExt;
    use tokio::io::AsyncReadExt;
    let file = tokio::fs::File::open(path).await?;
    let stream = futures_util::stream::unfold(file, |mut file| async move {
        let mut buf = vec![0u8; 64 * 1024];
        match file.read(&mut buf).await {
            Ok(0) => None,
            Ok(n) => {
                buf.truncate(n);
                Some((Ok::<_, std::io::Error>(Bytes::from(buf)), file))
            }
            Err(e) => Some((Err(e), file)),
        }
    })
    .boxed();
    let content_type = if backup.filename.to_ascii_lowercase().ends_with(".gz") {
        "application/gzip"
    } else {
        "application/octet-stream"
    };
    let name = &backup.filename;
    let mut response = Response::new(Body::from_stream(stream));
    let h = response.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    h.insert(header::CONTENT_LENGTH, HeaderValue::from(backup.size));
    // ActionDispatch::Http::ContentDisposition; the name is already
    // limited to characters that need no escaping.
    h.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!(
            "attachment; filename=\"{name}\"; filename*=UTF-8''{name}"
        ))?,
    );
    h.insert(
        "content-transfer-encoding",
        HeaderValue::from_static("binary"),
    );
    Ok(response)
}

/// The request's own URL for a path (`url_for`), with its scheme, host and
/// the site's subfolder.
fn request_url(state: &AppState, headers: &HeaderMap, path: &str) -> String {
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost");
    let scheme = match headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
    {
        Some(p) if p.eq_ignore_ascii_case("https") => "https",
        _ => "http",
    };
    let root = state.config.globals.relative_url_root();
    format!("{scheme}://{host}{root}{path}")
}

/// PUT /admin/backups/:id (#email): mails the admin a download link.
pub async fn email(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(raw): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let (Some((id, _)), None) = (backup_id(&raw), admin_only(&state, &guardian)) else {
        return Ok(super::topics::not_found_response(&state));
    };
    let p = crate::params::parse(uri.query(), &headers, &body);
    if !super::session::csrf_ok(&state, &headers, &pairs(&p), uri.path(), "PUT") {
        return Ok(super::session::bad_csrf());
    }
    if !xhr_or_json(&uri, &headers) {
        return Ok(super::session::render_empty());
    }
    let settings = settings(&state).await?;
    let store = BackupStore::create(&state.config, &settings)?;
    if store.file(&id, false).await?.is_none() {
        return Ok(StatusCode::NOT_FOUND.into_response());
    }
    let mut conn = state.pool.acquire().await?;
    crate::jobs::enqueue(
        &mut conn,
        backup_restore::jobs::DOWNLOAD_BACKUP_EMAIL,
        json!({
            "user_id": guardian.user_id(),
            "backup_file_path": request_url(&state, &headers, &format!("/admin/backups/{id}")),
        }),
    )
    .await?;
    Ok(StatusCode::OK.into_response())
}

/// DELETE /admin/backups/:id (#destroy): logged as backup_destroy first.
pub async fn destroy(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(raw): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let (Some((id, _)), None) = (backup_id(&raw), admin_only(&state, &guardian)) else {
        return Ok(super::topics::not_found_response(&state));
    };
    let p = crate::params::parse(uri.query(), &headers, &body);
    if !super::session::csrf_ok(&state, &headers, &pairs(&p), uri.path(), "DELETE") {
        return Ok(super::session::bad_csrf());
    }
    if !xhr_or_json(&uri, &headers) {
        return Ok(super::session::render_empty());
    }
    let settings = settings(&state).await?;
    if let Some(r) = backups_disabled(&state, &settings)? {
        return Ok(r);
    }
    let store = BackupStore::create(&state.config, &settings)?;
    let Some(backup) = store.file(&id, false).await? else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };
    let mut conn = state.pool.acquire().await?;
    backup_restore::log_staff_action(
        &mut conn,
        "backup_destroy",
        guardian.user_id().unwrap_or_default(),
        Some(&backup.filename),
    )
    .await?;
    store.delete_file(&backup.filename).await?;
    Ok(StatusCode::OK.into_response())
}

/// GET /admin/backups/upload (#check_backup_chunk): 200 when that chunk
/// is already there whole, else 404.
pub async fn check_backup_chunk(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if let Some(r) = admin_only(&state, &guardian) {
        return Ok(r);
    }
    let settings = settings(&state).await?;
    if let Some(r) = backups_disabled(&state, &settings)? {
        return Ok(r);
    }
    let p = crate::params::parse(uri.query(), &headers, &body);
    let mut fetched = Vec::new();
    for key in [
        "resumableIdentifier",
        "resumableFilename",
        "resumableChunkNumber",
        "resumableCurrentChunkSize",
    ] {
        match p.get(key).and_then(crate::params::scalar) {
            Some(v) => fetched.push(v),
            None => return Ok(param_missing(key)),
        }
    }
    let [identifier, filename, chunk_number, current] = &fetched[..] else {
        unreachable!("four params fetched");
    };
    if !store::valid_filename(identifier) {
        return Ok(super::search::invalid_parameters(
            &state,
            "resumableIdentifier",
        ));
    }
    if !store::valid_filename(filename) {
        return Ok(super::search::invalid_parameters(
            &state,
            "resumableFilename",
        ));
    }
    // LocalBackupStore.chunk_path, whatever backup_location says.
    let store = LocalBackupStore::at(&state.config.public_dir, &state.config.site.db)?;
    let chunk = store.chunk_path(identifier, filename, chunk_number);
    let current = crate::ruby::to_i(current);
    let uploaded = std::fs::metadata(&chunk)
        .ok()
        .is_some_and(|m| m.is_file() && i64::try_from(m.len()).ok() == Some(current));
    Ok(if uploaded {
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    }
    .into_response())
}

/// POST /admin/backups/upload (#upload_backup_chunk): one chunk stored;
/// the last one queues the merge in 5 seconds.
pub async fn upload_backup_chunk(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    request: Request,
) -> Result<Response, AppError> {
    let path = request.uri().path().to_string();
    if let Some(r) = admin_only(&state, &guardian) {
        return Ok(r);
    }
    let form = super::uploads::read_form(request, &state).await?;
    if !super::session::csrf_ok(&state, &headers, &form.fields, &path, "POST") {
        return Ok(super::session::bad_csrf());
    }
    let settings = settings(&state).await?;
    if let Some(r) = backups_disabled(&state, &settings)? {
        return Ok(r);
    }
    let get = |key: &str| form.get(key).map(str::to_string);
    let Some(filename) = get("resumableFilename") else {
        return Ok(param_missing("resumableFilename"));
    };
    let Some(total_size) = get("resumableTotalSize") else {
        return Ok(param_missing("resumableTotalSize"));
    };
    let total_size = crate::ruby::to_i(&total_size);
    let Some(identifier) = get("resumableIdentifier") else {
        return Ok(param_missing("resumableIdentifier"));
    };
    if !store::valid_filename(&identifier) {
        return Ok(super::search::invalid_parameters(
            &state,
            "resumableIdentifier",
        ));
    }
    if !store::valid_extension(&filename) {
        return Ok(unsupported_media_type(
            &state,
            "backup.backup_file_should_be_tar_gz",
        ));
    }
    // LocalBackupStore.chunk_path, whatever backup_location says.
    let store = LocalBackupStore::at(&state.config.public_dir, &state.config.site.db)?;
    let free = store::free_bytes(&state.config.public_dir.join("backups"))?;
    if i64::try_from(free).unwrap_or(i64::MAX) <= total_size {
        return Ok(unsupported_media_type(
            &state,
            "backup.not_enough_space_on_disk",
        ));
    }
    if !store::valid_filename(&filename) {
        return Ok(unsupported_media_type(&state, "backup.invalid_filename"));
    }
    let Some((_, file)) = form.file.as_ref() else {
        return Ok(param_missing("file"));
    };
    let mut numbers = Vec::new();
    for key in [
        "resumableChunkNumber",
        "resumableChunkSize",
        "resumableCurrentChunkSize",
    ] {
        match get(key) {
            Some(v) => numbers.push(crate::ruby::to_i(&v)),
            None => return Ok(param_missing(key)),
        }
    }
    let [chunk_number, chunk_size, current_chunk_size] = numbers[..] else {
        unreachable!("three params fetched");
    };
    let chunk = store.chunk_path(&identifier, &filename, &chunk_number.to_string());
    if let Some(dir) = chunk.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&chunk, file)?;
    if (chunk_number - 1) * chunk_size + current_chunk_size >= total_size {
        let mut conn = state.pool.acquire().await?;
        crate::jobs::enqueue_in(
            &mut conn,
            5,
            backup_restore::jobs::BACKUP_CHUNKS_MERGER,
            json!({"filename": filename, "identifier": identifier, "chunks": chunk_number}),
        )
        .await?;
    }
    Ok(StatusCode::OK.into_response())
}

/// GET /admin/backups/upload_url (#create_upload_url): checked as Rails
/// checks it; a local store has no upload URL (NotImplementedError).
pub async fn create_upload_url(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if let Some(r) = admin_only(&state, &guardian) {
        return Ok(r);
    }
    if !xhr_or_json(&uri, &headers) {
        return Ok(super::session::render_empty());
    }
    let settings = settings(&state).await?;
    if let Some(r) = backups_disabled(&state, &settings)? {
        return Ok(r);
    }
    let p = crate::params::parse(uri.query(), &headers, &body);
    // params.require(:filename): blank is missing too.
    let Some(filename) = p
        .get("filename")
        .and_then(crate::params::scalar)
        .filter(|f| !f.is_empty())
    else {
        return Ok(param_missing("filename"));
    };
    let error = |key: &str| {
        let text = state.i18n.t(key).unwrap_or(key).to_string();
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "errors": [text] })),
        )
            .into_response()
    };
    if !store::valid_extension(&filename) {
        return Ok(error("backup.backup_file_should_be_tar_gz"));
    }
    if !store::valid_filename(&filename) {
        return Ok(error("backup.invalid_filename"));
    }
    let store = BackupStore::create(&state.config, &settings)?;
    match store.generate_upload_url(&filename).await {
        Ok(url) => Ok(Json(json!({ "success": "OK", "url": url })).into_response()),
        Err(e) if matches!(e.downcast_ref(), Some(BackupStoreError::FileExists)) => {
            Ok(error("backup.file_exists"))
        }
        Err(e) => storage_error(e),
    }
}

/// POST /admin/backups/{create,complete,abort}-multipart and
/// batch-presign-multipart-parts: S3 direct uploads.
pub async fn multipart(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
) -> Result<Response, AppError> {
    if let Some(r) = admin_only(&state, &guardian) {
        return Ok(r);
    }
    Err(Unsupported("direct S3 backup uploads").into())
}

/// POST /admin/backups(.json) (#create): a backup started (the port
/// queues what Rails spawns), logged as backup_create. One a minute, for
/// staff too.
pub async fn create(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if let Some(r) = admin_only(&state, &guardian) {
        return Ok(r);
    }
    let p = crate::params::parse(uri.query(), &headers, &body);
    if !super::session::csrf_ok(&state, &headers, &pairs(&p), uri.path(), "POST") {
        return Ok(super::session::bad_csrf());
    }
    if !xhr_or_json(&uri, &headers) {
        return Ok(super::session::render_empty());
    }
    let settings = settings(&state).await?;
    if let Some(r) = backups_disabled(&state, &settings)? {
        return Ok(r);
    }
    let user_id = guardian.user_id().unwrap_or_default();
    let limiter = crate::rate_limiter::RateLimiter::new(
        Some(crate::rate_limiter::LimitUser {
            id: user_id,
            staff: guardian.is_staff(),
        }),
        "max-backups-per-minute",
        1,
        60,
    )
    .apply_limit_to_staff();
    if let Err(refused) = limiter
        .performed(&state.pool, state.config.rate_limits)
        .await?
    {
        return Ok(refused.into_response(&state.i18n));
    }
    let Some(with_uploads) = p.get("with_uploads") else {
        return Ok(param_missing("with_uploads"));
    };
    let with_uploads = crate::params::scalar(with_uploads).as_deref() == Some("true");
    let client_id = p.get("client_id").and_then(crate::params::scalar);
    let mut tx = state.pool.begin().await?;
    crate::jobs::enqueue(
        &mut tx,
        backup_restore::jobs::SPAWN_BACKUP_RESTORE,
        json!({
            "type": "backup",
            "user_id": user_id,
            "opts": {
                "publish_to_message_bus": true,
                "with_uploads": with_uploads,
                "client_id": client_id,
            },
        }),
    )
    .await?;
    backup_restore::log_staff_action(&mut tx, "backup_create", user_id, None).await?;
    tx.commit().await?;
    Ok(Json(json!({"success": "OK"})).into_response())
}

/// POST /admin/backups/:id/restore (#restore): the restore started (the
/// port queues what Rails spawns). Nothing is logged here.
pub async fn restore(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(raw): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let (Some((id, _)), None) = (backup_id(&raw), admin_only(&state, &guardian)) else {
        return Ok(super::topics::not_found_response(&state));
    };
    let p = crate::params::parse(uri.query(), &headers, &body);
    if !super::session::csrf_ok(&state, &headers, &pairs(&p), uri.path(), "POST") {
        return Ok(super::session::bad_csrf());
    }
    if !xhr_or_json(&uri, &headers) {
        return Ok(super::session::render_empty());
    }
    let settings = settings(&state).await?;
    if let Some(r) = backups_disabled(&state, &settings)? {
        return Ok(r);
    }
    let Some(client_id) = p.get("client_id").and_then(crate::params::scalar) else {
        return Ok(param_missing("client_id"));
    };
    let mut conn = state.pool.acquire().await?;
    crate::jobs::enqueue(
        &mut conn,
        backup_restore::jobs::SPAWN_BACKUP_RESTORE,
        json!({
            "type": "restore",
            "user_id": guardian.user_id(),
            "opts": {
                "filename": id,
                "client_id": client_id,
                "publish_to_message_bus": true,
            },
        }),
    )
    .await?;
    Ok(Json(json!({"success": "OK"})).into_response())
}

/// POST /admin/backups/rollback(.json) (#rollback): the tables a restore
/// moved to the `backup` schema moved back, unless an operation runs.
pub async fn rollback(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if let Some(r) = admin_only(&state, &guardian) {
        return Ok(r);
    }
    let p = crate::params::parse(uri.query(), &headers, &body);
    if !super::session::csrf_ok(&state, &headers, &pairs(&p), uri.path(), "POST") {
        return Ok(super::session::bad_csrf());
    }
    if !xhr_or_json(&uri, &headers) {
        return Ok(super::session::render_empty());
    }
    let settings = settings(&state).await?;
    if let Some(r) = backups_disabled(&state, &settings)? {
        return Ok(r);
    }
    let mut conn = state.pool.acquire().await?;
    if backup_restore::is_operation_running(&mut conn).await? {
        // render_error("backup.operation_already_running")
        let message = state
            .i18n
            .t("backup.operation_already_running")
            .unwrap_or_default();
        return Ok(Json(json!({"failed": "FAILED", "message": message})).into_response());
    }
    if backup_restore::can_rollback(&mut conn).await? {
        backup_restore::restorer::move_tables_between_schemas(&state, "backup", "public").await?;
    }
    Ok(Json(json!({"success": "OK"})).into_response())
}

/// DELETE /admin/backups/cancel(.json) (#cancel): the shutdown flag the
/// running operation watches.
pub async fn cancel(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if let Some(r) = admin_only(&state, &guardian) {
        return Ok(r);
    }
    let p = crate::params::parse(uri.query(), &headers, &body);
    if !super::session::csrf_ok(&state, &headers, &pairs(&p), uri.path(), "DELETE") {
        return Ok(super::session::bad_csrf());
    }
    if !xhr_or_json(&uri, &headers) {
        return Ok(super::session::render_empty());
    }
    let settings = settings(&state).await?;
    if let Some(r) = backups_disabled(&state, &settings)? {
        return Ok(r);
    }
    let mut conn = state.pool.acquire().await?;
    backup_restore::set_shutdown_signal(&mut conn).await?;
    Ok(Json(json!({"success": "OK"})).into_response())
}

/// PUT /admin/backups/readonly(.json)?enable= (BackupsController#readonly,
/// AdminConstraint): the user read-only key on or off, logged as
/// change_readonly_mode, an empty 200.
pub async fn readonly(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if !guardian.is_admin() {
        return Ok(super::topics::not_found_response(&state));
    }
    let p = crate::params::parse(uri.query(), &headers, &body);
    if !super::session::csrf_ok(&state, &headers, &pairs(&p), uri.path(), "PUT") {
        return Ok(super::session::bad_csrf());
    }
    // check_xhr: not skipped for this action.
    if !xhr_or_json(&uri, &headers) {
        return Ok(super::session::render_empty());
    }
    if let Some(r) = backups_disabled(&state, &settings(&state).await?)? {
        return Ok(r);
    }
    // params.fetch(:enable): present, even blank.
    let Some(enable) = p.get("enable") else {
        return Ok(param_missing("enable"));
    };
    let enable = crate::params::scalar(enable).as_deref() == Some("true");
    let mut tx = state.pool.begin().await?;
    if enable {
        crate::readonly::enable(&mut tx, &state.bus, crate::readonly::USER_READONLY_MODE_KEY)
            .await?;
    } else {
        crate::readonly::disable(&mut tx, &state.bus, crate::readonly::USER_READONLY_MODE_KEY)
            .await?;
    }
    // StaffActionLogger#log_change_readonly_mode: previous_value !state,
    // new_value state, as the text column stores booleans.
    let as_text = |b: bool| if b { "t" } else { "f" };
    sqlx::query(
        "INSERT INTO user_histories (action, acting_user_id, previous_value, new_value, admin_only, \
                                     created_at, updated_at) \
         VALUES ($1, $2, $3, $4, TRUE, now(), now())",
    )
    .bind(crate::staff_action_logs::action_id("change_readonly_mode").unwrap_or(44))
    .bind(guardian.user_id())
    .bind(as_text(!enable))
    .bind(as_text(enable))
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(StatusCode::OK.into_response())
}

#[cfg(test)]
mod tests {
    use super::backup_id;

    #[test]
    fn member_ids_route_as_rails_does() {
        assert_eq!(
            backup_id("discourse-2026-10-01-033000-v20260925054715.tar.gz"),
            Some((
                "discourse-2026-10-01-033000-v20260925054715.tar.gz".into(),
                None
            ))
        );
        assert_eq!(
            backup_id("x.tar.gz.json"),
            Some(("x.tar.gz".into(), Some("json".into())))
        );
        assert_eq!(backup_id("x.sql.gz"), Some(("x.sql.gz".into(), None)));
        assert_eq!(backup_id("notes.txt"), None);
        assert_eq!(backup_id("bad name.tar.gz"), None);
    }
}
