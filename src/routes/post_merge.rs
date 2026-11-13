//! PostsController#merge_posts: PUT /posts/merge_posts(.json) merges
//! posts of one author in a topic through PostMerger.

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

use super::post_destroy::{Front, front, ids_of};
use crate::post_merger::{self, Outcome};
use crate::posting::Ctx;
use crate::pretty_text::Host;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

/// PUT /posts/merge_posts(.json)
pub async fn merge_posts(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = match front(&state, &guardian, &headers, &uri, &body, "PUT") {
        Front::Params(p) => p,
        Front::Stop(response) => return Ok(response),
    };
    // params.require(:post_ids), which Post.where(id:) casts: a list or
    // a single id.
    let given = match p.get("post_ids") {
        None | Some(Value::Null) => return Ok(super::accounts::param_missing("post_ids")),
        Some(Value::String(s)) if s.is_empty() => {
            return Ok(super::accounts::param_missing("post_ids"));
        }
        Some(Value::Array(list)) if list.is_empty() => {
            return Ok(super::accounts::param_missing("post_ids"));
        }
        Some(Value::Object(_)) => return Err(Unsupported("post_ids as a hash").into()),
        Some(given) => given,
    };
    let wanted = match given {
        Value::Array(list) => ids_of(list, "post_ids holding lists or hashes")?,
        one => ids_of(std::slice::from_ref(one), "post_ids as a hash")?,
    };
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let host = Host::from_state(&state);
    let ctx = Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    // Post.where(id:).order(:id)
    let ids: Vec<i32> = sqlx::query_scalar(
        "SELECT id FROM posts WHERE id = ANY($1) AND deleted_at IS NULL ORDER BY id",
    )
    .bind(&wanted)
    .fetch_all(&mut *conn)
    .await?;
    drop(conn);
    // `posts.pluck(:id) == params[:post_ids]`: only a JSON body's numbers
    // can equal the ids, which Rails then refuses.
    if *given == json!(ids) {
        return Ok(super::search::invalid_parameters(&state, "post_ids"));
    }
    Ok(
        match post_merger::merge(&state.pool, &ctx, &guardian, &ids).await? {
            // render body: nil
            Outcome::Done => StatusCode::OK.into_response(),
            // render_json_error(e.message)
            Outcome::CannotMerge(message) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({ "errors": [message] })),
            )
                .into_response(),
            Outcome::InvalidAccess => super::search::invalid_access(&state),
            Outcome::RateLimited(refused) => refused.into_response(&state.i18n),
        },
    )
}
