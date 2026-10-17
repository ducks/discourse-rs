//! Admin::McpClientsController under /admin/mcp/clients (AdminConstraint):
//! the clients' page, a pre-registered client's creation, one client,
//! blocking, and a metadata client's refresh (crate::mcp_clients).

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

use super::admin_mcp::{acting, cast_bool, page_params, param_missing, path_id, request, strings};
use crate::mcp_clients::{self, NewClient, Refused};
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, params};

/// GET and POST /admin/mcp/clients
pub async fn index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    method: axum::http::Method,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let create = method == axum::http::Method::POST;
    let p = match request(
        &state,
        &guardian,
        &headers,
        &uri,
        &body,
        if create { "POST" } else { "GET" },
    ) {
        Ok(p) => p,
        Err(r) => return Ok(*r),
    };
    if !create {
        let (limit, cursor, filter) = match page_params(&state, &p) {
            Ok(v) => v,
            Err(r) => return Ok(*r),
        };
        let mut conn = state.pool.acquire().await?;
        return Ok(
            Json(mcp_clients::page(&mut conn, limit, cursor, &filter).await?).into_response(),
        );
    }
    // `params.require(:client).permit(:client_id, :name, redirect_uris: [])`
    let Some(Value::Object(client)) = p.get("client").filter(|v| match v {
        Value::Object(m) => !m.is_empty(),
        _ => false,
    }) else {
        return Ok(param_missing("client"));
    };
    let text = |key: &str| client.get(key).and_then(params::scalar).unwrap_or_default();
    let fields = NewClient {
        client_id: text("client_id"),
        name: text("name"),
        redirect_uris: match client.get("redirect_uris") {
            Some(Value::Array(_)) => strings(client.get("redirect_uris")),
            _ => Vec::new(),
        },
    };
    let actor = acting(&guardian)?;
    let mut tx = state.pool.begin().await?;
    match mcp_clients::create(&mut tx, actor, &fields).await? {
        Ok(created) => {
            tx.commit().await?;
            Ok((
                StatusCode::CREATED,
                Json(json!({ "client": mcp_clients::serialize(&created) })),
            )
                .into_response())
        }
        // ActiveRecord::RecordInvalid
        Err(errors) => Ok((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "errors": errors, "error_type": "record_invalid" })),
        )
            .into_response()),
    }
}

/// The client the path names, or the not found response.
async fn client(
    conn: &mut sqlx::PgConnection,
    state: &AppState,
    raw: &str,
) -> Result<Result<mcp_clients::Client, Response>, AppError> {
    let found = match crate::ruby::integer(&path_id(raw)) {
        Some(id) => mcp_clients::find(conn, id).await?,
        None => None,
    };
    Ok(found.ok_or_else(|| super::topics::not_found_response(state)))
}

/// GET /admin/mcp/clients/:id
pub async fn show(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(raw): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if let Err(r) = request(&state, &guardian, &headers, &uri, &body, "GET") {
        return Ok(*r);
    }
    let mut conn = state.pool.acquire().await?;
    Ok(match client(&mut conn, &state, &raw).await? {
        Ok(c) => Json(json!({ "client": mcp_clients::serialize(&c) })).into_response(),
        Err(r) => r,
    })
}

/// PUT /admin/mcp/clients/:id/block
pub async fn block(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(raw): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = match request(&state, &guardian, &headers, &uri, &body, "PUT") {
        Ok(p) => p,
        Err(r) => return Ok(*r),
    };
    // `Boolean.cast(params.fetch(:blocked, true))`: nil is not blocked.
    let blocked = p
        .get("blocked")
        .is_none_or(|v| cast_bool(v).unwrap_or(false));
    let actor = acting(&guardian)?;
    let mut tx = state.pool.begin().await?;
    let c = match client(&mut tx, &state, &raw).await? {
        Ok(c) => c,
        Err(r) => return Ok(r),
    };
    let updated = mcp_clients::block(&mut tx, actor, &c, blocked).await?;
    tx.commit().await?;
    Ok(Json(json!({ "client": mcp_clients::serialize(&updated) })).into_response())
}

/// POST /admin/mcp/clients/:id/refresh
pub async fn refresh(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(raw): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if let Err(r) = request(&state, &guardian, &headers, &uri, &body, "POST") {
        return Ok(*r);
    }
    let actor = acting(&guardian)?;
    let mut tx = state.pool.begin().await?;
    let c = match client(&mut tx, &state, &raw).await? {
        Ok(c) => c,
        Err(r) => return Ok(r),
    };
    // `raise Discourse::InvalidParameters` with no parameter named.
    if c.registration_type != "cimd" {
        return Ok(super::search::invalid_parameters(
            &state,
            "Discourse::InvalidParameters",
        ));
    }
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let cx = mcp_clients::ResolveCx {
        pool: &state.pool,
        rate_limits: state.config.rate_limits,
        fetcher: crate::final_destination::Fetcher {
            settings: &settings,
            urls: crate::url::Urls {
                config: &state.config,
                settings: &settings,
            },
        },
        user: crate::rate_limiter::LimitUser {
            id: actor,
            staff: true,
        },
    };
    let outcome = mcp_clients::refresh(&mut tx, &cx, &c).await?;
    match outcome {
        Ok(refreshed) => {
            tx.commit().await?;
            Ok(Json(json!({ "client": mcp_clients::serialize(&refreshed) })).into_response())
        }
        // An unapproved client's update stands; Rails raises after saving
        // it (its transaction already committed).
        Err(Refused::InvalidAccess) => {
            tx.commit().await?;
            Ok(super::search::invalid_access(&state))
        }
        Err(Refused::RateLimited(limit)) => Err(limit.into()),
    }
}

pub(super) fn routes(mut router: axum::Router<AppState>) -> axum::Router<AppState> {
    use axum::routing::{get, post, put};
    for suffix in ["", ".json"] {
        router = router
            .route(
                &format!("/admin/mcp/clients{suffix}"),
                get(index).post(index),
            )
            .route(
                &format!("/admin/mcp/clients/{{id}}/block{suffix}"),
                put(block),
            )
            .route(
                &format!("/admin/mcp/clients/{{id}}/refresh{suffix}"),
                post(refresh),
            );
    }
    router.route("/admin/mcp/clients/{id}", get(show))
}
