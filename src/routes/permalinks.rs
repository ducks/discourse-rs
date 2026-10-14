//! PermalinksController (the `*url` route and /permalink-check),
//! CategoriesController#redirect (`/category/*path`) and
//! Admin::PermalinksController.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{MatchedPath, Path, Request, State};
use axum::http::{HeaderMap, Method, StatusCode, Uri, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use crate::permalinks::{self, Permalink};
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::url::Urls;
use crate::{AppError, AppState, Unsupported};

/// The `*url` route's PermalinkConstraint: without a permalink for the
/// full path (query string included) there is no route, Rails' routing
/// 404, before any filter runs.
pub async fn constraint(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, AppError> {
    let catch_all = request
        .extensions()
        .get::<MatchedPath>()
        .is_some_and(|m| m.as_str() == CATCH_ALL);
    if !catch_all {
        return Ok(next.run(request).await);
    }
    // `get "*url"`: other methods have no route.
    if request.method() != Method::GET && request.method() != Method::HEAD {
        return Ok(StatusCode::NOT_FOUND.into_response());
    }
    let fullpath = request
        .uri()
        .path_and_query()
        .map(|p| p.as_str().to_string())
        .unwrap_or_default();
    let found = {
        let mut conn = state.pool.acquire().await?;
        let settings =
            SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
        permalinks::find_by_url(&mut conn, &settings, &fullpath)
            .await?
            .is_some()
    };
    if !found {
        return Ok(StatusCode::NOT_FOUND.into_response());
    }
    Ok(next.run(request).await)
}

/// Any other method on the `*url` route: no route (the constraint answers
/// first; this keeps axum from answering 405).
pub async fn no_route() -> StatusCode {
    StatusCode::NOT_FOUND
}

/// The route the constraint guards.
pub const CATCH_ALL: &str = "/{*url}";

/// GET /*url (PermalinksController#show): a 301 to the target, or the
/// not-found error when the viewer may not see it or it is gone.
pub async fn show(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let fullpath = uri
        .path_and_query()
        .map(|p| p.as_str().to_string())
        .unwrap_or_default();
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let base_path = state.config.globals.relative_url_root();
    match permalinks::visible_target(&mut conn, &settings, base_path, &guardian, &fullpath).await? {
        Some(target) => Ok(permalinks::moved_permanently(&headers, &target)),
        None => Ok(super::topics::not_found_response(&state)),
    }
}

/// GET /permalink-check(.json)?path= (PermalinksController#check).
pub async fn check(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    uri: Uri,
) -> Result<Response, AppError> {
    let query = crate::params::parse_query(uri.query().unwrap_or(""));
    let path = crate::params::string(&query, "path").filter(|p| !crate::ruby::is_blank(p));
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let base_path = state.config.globals.relative_url_root();
    let found = match &path {
        Some(path) => match permalinks::find_by_url(&mut conn, &settings, path).await? {
            Some(p) if permalinks::can_see_target(&mut conn, &settings, &guardian, &p).await? => {
                Some(p)
            }
            _ => None,
        },
        None => None,
    };
    let body = match found {
        Some(p) => {
            let target = permalinks::target_url(&mut conn, base_path, &p).await?;
            json!({
                "found": true,
                "internal": p.external_url.as_deref().is_none_or(crate::ruby::is_blank),
                "target_url": target,
            })
        }
        None => {
            let urls = Urls {
                config: &state.config,
                settings: &settings,
            };
            let page = crate::not_found_page::NotFound {
                forbidden: false,
                custom_message: None,
                slug: "",
                guardian: &guardian,
            };
            let html =
                crate::not_found_page::build(&mut conn, &settings, &state.i18n, &urls, &page)
                    .await?;
            json!({ "found": false, "html": html })
        }
    };
    Ok(Json(body).into_response())
}

/// GET /category/*path (CategoriesController#redirect): a permalink for
/// `/category/<path>`, else a 302 to `/c/<path>`.
pub async fn category_redirect(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(path): Path<String>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    // The glob leaves the format out.
    let path = path.strip_suffix(".json").unwrap_or(&path).to_string();
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let base_path = state.config.globals.relative_url_root();
    // handle_permalink: a plain 301.
    if let Some(target) = permalinks::visible_target(
        &mut conn,
        &settings,
        base_path,
        &guardian,
        &format!("/category/{path}"),
    )
    .await?
    {
        return Ok(permalinks::moved_permanently(&headers, &target));
    }
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost");
    Ok((
        StatusCode::FOUND,
        [(
            header::LOCATION,
            format!("http://{host}{base_path}/c/{path}"),
        )],
    )
        .into_response())
}

/// Who reaches Admin::PermalinksController: the /admin/permalinks routes
/// are AdminConstraint (anyone else: no route); /admin/config/permalinks
/// is StaffConstraint, then ensure_admin's InvalidAccess for moderators.
fn refuse(state: &AppState, guardian: &crate::guardian::Guardian, uri: &Uri) -> Option<Response> {
    if guardian.is_admin() {
        return None;
    }
    if guardian.is_staff() && uri.path().starts_with("/admin/config/") {
        return Some(super::search::invalid_access(state));
    }
    Some(super::topics::not_found_response(state))
}

/// check_xhr for the JSON actions.
fn json_request(headers: &HeaderMap, uri: &Uri) -> bool {
    uri.path().ends_with(".json")
        || headers
            .get("x-requested-with")
            .is_some_and(|v| v == "XMLHttpRequest")
        || super::wants_json(uri.query(), headers)
}

fn id_of(raw: &str) -> i32 {
    crate::ruby::to_i(raw.strip_suffix(".json").unwrap_or(raw)) as i32
}

fn pairs(p: &Map<String, Value>) -> Vec<(String, String)> {
    p.iter()
        .filter_map(|(k, v)| crate::params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

/// GET /admin/permalinks(.json) and /admin/config/permalinks(.json):
/// PermalinkSerializer for each, a bare array.
pub async fn index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if let Some(r) = refuse(&state, &guardian, &uri) {
        return Ok(r);
    }
    if !json_request(&headers, &uri) {
        return Err(Unsupported("the admin permalinks page").into());
    }
    let query = crate::params::parse_query(uri.query().unwrap_or(""));
    let filter = crate::params::string(&query, "filter");
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let base_path = state.config.globals.relative_url_root().to_string();
    let mut out = Vec::new();
    for p in permalinks::filter_by(&mut conn, filter.as_deref()).await? {
        out.push(permalinks::serialize(&mut conn, &urls, &base_path, &p).await?);
    }
    Ok(Json(Value::Array(out)).into_response())
}

/// The permalink, rooted, as render_serialized writes one.
async fn rendered(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    p: &Permalink,
) -> Result<Response, AppError> {
    let settings =
        SiteSettings::load(&mut *conn, &state.site_setting_defs, &state.config.globals).await?;
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let base_path = state.config.globals.relative_url_root().to_string();
    let body = permalinks::serialize(conn, &urls, &base_path, p).await?;
    Ok(Json(json!({ "permalink": body })).into_response())
}

/// GET /admin/permalinks/:id(.json)
pub async fn show_admin(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if let Some(r) = refuse(&state, &guardian, &uri) {
        return Ok(r);
    }
    if !json_request(&headers, &uri) {
        return Err(Unsupported("the admin permalinks page").into());
    }
    let mut conn = state.pool.acquire().await?;
    let Some(p) = permalinks::find(&mut conn, id_of(&id)).await? else {
        return Ok(super::topics::not_found_response(&state));
    };
    rendered(&state, &mut conn, &p).await
}

/// `permalink_params`: the url, and only the type's own column set.
struct Params {
    url: Option<String>,
    topic_id: Option<i32>,
    post_id: Option<i32>,
    category_id: Option<i32>,
    tag_name: Option<String>,
    user_id: Option<i32>,
    external_url: Option<String>,
}

/// `params.require(:permalink).permit(...)`; None when it is missing.
fn permalink_params(p: &Map<String, Value>) -> Option<Params> {
    let permalink = p.get("permalink")?.as_object()?;
    if permalink.is_empty() {
        return None;
    }
    let s = |k: &str| permalink.get(k).and_then(crate::params::scalar);
    let kind = s("permalink_type");
    let value = s("permalink_type_value");
    let of = |t: &str| {
        if kind.as_deref() == Some(t) {
            value.clone()
        } else {
            None
        }
    };
    let int = |t: &str| of(t).map(|v| crate::ruby::to_i(&v) as i32);
    Some(Params {
        url: s("url"),
        topic_id: int("topic"),
        post_id: int("post"),
        category_id: int("category"),
        tag_name: of("tag"),
        user_id: int("user"),
        external_url: of("external_url"),
    })
}

/// `render_json_error(e.record.errors.full_messages)`
fn invalid(errors: Vec<String>) -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({ "errors": errors })),
    )
        .into_response()
}

/// `create!` / `update!`: the url normalized, validations run, the row
/// written. Err(response) for a validation failure.
async fn save(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    id: Option<i32>,
    params: Params,
) -> Result<Result<Permalink, Response>, AppError> {
    let settings =
        SiteSettings::load(&mut *conn, &state.site_setting_defs, &state.config.globals).await?;
    let url = match &params.url {
        Some(u) => Some(permalinks::url_for_write(&settings, u)?),
        None => None,
    };
    let Some(url) = url else {
        return Err(Unsupported("a permalink without a url (NOT NULL violation)").into());
    };
    // Tag.where(name:).pluck(:id).first || -1
    let tag_id = match &params.tag_name {
        Some(name) => Some(
            sqlx::query_scalar::<_, i32>("SELECT id FROM tags WHERE name = $1 ORDER BY id LIMIT 1")
                .bind(name)
                .fetch_optional(&mut *conn)
                .await?
                .unwrap_or(-1),
        ),
        None => None,
    };
    let mut errors = Vec::new();
    let taken: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM permalinks WHERE url = $1 AND id IS DISTINCT FROM $2)",
    )
    .bind(&url)
    .bind(id)
    .fetch_one(&mut *conn)
    .await?;
    if taken {
        errors.push("Url has already been taken".to_string());
    }
    let set = [
        params.topic_id.is_some(),
        params.post_id.is_some(),
        params.category_id.is_some(),
        tag_id.is_some(),
        params.user_id.is_some(),
        params.external_url.is_some(),
    ]
    .iter()
    .filter(|b| **b)
    .count();
    if set != 1 {
        errors.push(
            "Exactly one of topic_id, post_id, category_id, tag_id, user_id, or external_url must be set"
                .to_string(),
        );
    }
    if !errors.is_empty() {
        return Ok(Err(invalid(errors)));
    }
    let id: i32 = match id {
        Some(id) => sqlx::query_scalar(
            "UPDATE permalinks SET url = $2, topic_id = $3, post_id = $4, category_id = $5, tag_id = $6, \
               user_id = $7, external_url = $8, updated_at = now() WHERE id = $1 RETURNING id",
        )
        .bind(id),
        None => sqlx::query_scalar(
            "INSERT INTO permalinks (url, topic_id, post_id, category_id, tag_id, user_id, external_url, \
               created_at, updated_at) VALUES ($2, $3, $4, $5, $6, $7, $8, now(), now()) RETURNING id",
        )
        .bind(None::<i32>),
    }
    .bind(&url)
    .bind(params.topic_id)
    .bind(params.post_id)
    .bind(params.category_id)
    .bind(tag_id)
    .bind(params.user_id)
    .bind(&params.external_url)
    .fetch_one(&mut *conn)
    .await?;
    let saved = permalinks::find(&mut *conn, id)
        .await?
        .ok_or(Unsupported("a permalink gone once saved"))?;
    Ok(Ok(saved))
}

/// POST /admin/permalinks(.json), /admin/config/permalinks(.json)
pub async fn create(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    write(state, guardian, headers, uri, body, None, Method::POST).await
}

/// PUT /admin/permalinks/:id(.json)
pub async fn update(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    write(
        state,
        guardian,
        headers,
        uri,
        body,
        Some(id_of(&id)),
        Method::PUT,
    )
    .await
}

async fn write(
    state: AppState,
    guardian: crate::guardian::Guardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
    id: Option<i32>,
    method: Method,
) -> Result<Response, AppError> {
    if let Some(r) = refuse(&state, &guardian, &uri) {
        return Ok(r);
    }
    let p = crate::params::parse(uri.query(), &headers, &body);
    if !super::session::csrf_ok(&state, &headers, &pairs(&p), uri.path(), method.as_str()) {
        return Ok(super::session::bad_csrf());
    }
    if !json_request(&headers, &uri) {
        return Ok(super::session::render_empty());
    }
    let mut tx = state.pool.begin().await?;
    // before_action :fetch_permalink (update)
    if let Some(id) = id
        && permalinks::find(&mut tx, id).await?.is_none()
    {
        return Ok(super::topics::not_found_response(&state));
    }
    let Some(params) = permalink_params(&p) else {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "errors": ["param is missing or the value is empty or invalid: permalink"]
            })),
        )
            .into_response());
    };
    let saved = match save(&state, &mut tx, id, params).await? {
        Ok(saved) => saved,
        Err(response) => return Ok(response),
    };
    let response = rendered(&state, &mut tx, &saved).await?;
    tx.commit().await?;
    Ok(response)
}

/// DELETE /admin/permalinks/:id(.json), /admin/config/permalinks/:id(.json)
pub async fn destroy(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if let Some(r) = refuse(&state, &guardian, &uri) {
        return Ok(r);
    }
    let p = crate::params::parse(uri.query(), &headers, &body);
    if !super::session::csrf_ok(&state, &headers, &pairs(&p), uri.path(), "DELETE") {
        return Ok(super::session::bad_csrf());
    }
    if !json_request(&headers, &uri) {
        return Ok(super::session::render_empty());
    }
    let mut conn = state.pool.acquire().await?;
    let deleted = sqlx::query("DELETE FROM permalinks WHERE id = $1")
        .bind(id_of(&id))
        .execute(&mut *conn)
        .await?;
    if deleted.rows_affected() == 0 {
        return Ok(super::topics::not_found_response(&state));
    }
    Ok(Json(json!({ "success": "OK" })).into_response())
}
