//! discourse-policy's PolicyController, mounted at /policy: PUT accept and
//! unaccept (`post_id`), GET accepted and not-accepted, a page of the
//! users who did and did not accept (`offset`).

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use super::session::{bad_csrf, csrf_ok};
use crate::params;
use crate::plugins::policy::{self, POLICY_USER_DEFAULT_LIMIT, PostPolicy, Users};
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::url::Urls;
use crate::{AppError, AppState};

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

/// `render_json_error(message)`
fn json_error(message: &str) -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({ "errors": [message] })),
    )
        .into_response()
}

#[derive(Clone, Copy, PartialEq)]
enum Action {
    Accept,
    Unaccept,
    Accepted,
    NotAccepted,
}

/// PUT /policy/accept
pub async fn accept(
    state: State<AppState>,
    guardian: AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    run(state, guardian, headers, uri, body, Action::Accept).await
}

/// PUT /policy/unaccept
pub async fn unaccept(
    state: State<AppState>,
    guardian: AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    run(state, guardian, headers, uri, body, Action::Unaccept).await
}

/// GET /policy/accepted
pub async fn accepted(
    state: State<AppState>,
    guardian: AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    run(
        state,
        guardian,
        headers,
        uri,
        Bytes::new(),
        Action::Accepted,
    )
    .await
}

/// GET /policy/not-accepted
pub async fn not_accepted(
    state: State<AppState>,
    guardian: AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    run(
        state,
        guardian,
        headers,
        uri,
        Bytes::new(),
        Action::NotAccepted,
    )
    .await
}

async fn run(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
    action: Action,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    let write = matches!(action, Action::Accept | Action::Unaccept);
    if write && !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "PUT") {
        return Ok(bad_csrf());
    }
    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    // requires_plugin
    if !policy::enabled(&settings)? {
        return Ok(super::topics::not_found_response(&state));
    }
    // ensure_logged_in
    let Some(user_id) = guardian.user_id() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    // set_post
    let Some(post_id) = p
        .get("post_id")
        .and_then(params::scalar)
        .filter(|v| !v.trim().is_empty())
    else {
        return Ok(super::accounts::param_missing("post_id"));
    };
    let post_id = crate::ruby::to_i(&post_id) as i32;
    let post: Option<(Option<i32>,)> =
        sqlx::query_as("SELECT user_id FROM posts WHERE id = $1 AND deleted_at IS NULL")
            .bind(post_id)
            .fetch_optional(&mut *tx)
            .await?;
    let Some((post_user_id,)) = post else {
        return Ok(super::topics::not_found_response(&state));
    };
    let visible = crate::posting::revisions::find_post_with_deleted_in(
        &mut tx, &settings, &guardian, post_id,
    )
    .await?
    .is_some();
    if !visible {
        return Ok(super::topics::not_found_response(&state));
    }
    let t = |key: &str| state.i18n.t(key).unwrap_or(key).to_string();
    let Some(post_policy) = PostPolicy::for_post(&mut tx, post_id).await? else {
        return Ok(json_error(&t("discourse_policy.errors.no_policy")));
    };
    let group_ids = post_policy.group_ids(&mut tx).await?;
    if group_ids.is_empty() {
        return Ok(json_error(&t("discourse_policy.errors.group_not_found")));
    }
    if !policy::user_allowed(&mut tx, &settings, post_user_id).await? {
        return Ok(json_error(&t("discourse_policy.errors.staff_only")));
    }

    if write {
        // ensure_can_accept
        let member: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM group_users WHERE group_id = ANY($1) AND user_id = $2)",
        )
        .bind(&group_ids)
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await?;
        if !member {
            return Ok(json_error(&t("discourse_policy.errors.user_missing")));
        }
        let add_users_group = match post_policy.add_users_to_group {
            Some(id) => crate::group_manager::find(&mut tx, id).await?,
            None => None,
        };
        if let Some(group) = &add_users_group
            && !policy::can_edit_group(&mut tx, &settings, post_user_id, group.id).await?
        {
            return Ok(json_error(&t(
                "discourse_policy.errors.policy_group_inaccessible",
            )));
        }
        if action == Action::Accept {
            policy::accept(&mut tx, &post_policy, user_id).await?;
        } else {
            policy::revoke(&mut tx, &post_policy, user_id).await?;
        }
        let host = crate::pretty_text::Host::from_state(&state);
        let ctx = crate::posting::Ctx {
            host: &host,
            settings: &settings,
            config: &state.config,
            i18n: &state.i18n,
            bus: &state.bus,
        };
        crate::bus::publish_post_change(&ctx, &mut tx, post_id, "policy_change", Map::new(), false)
            .await?;
        if let Some(group) = &add_users_group {
            let promotion = crate::promotion::Ctx {
                settings: &settings,
                i18n: &state.i18n,
                host: &host,
                config: &state.config,
            };
            let cx = crate::group_manager::Cx {
                state: &state,
                promotion: &promotion,
            };
            if action == Action::Accept {
                crate::group_manager::add_many(&mut tx, &cx, group, &[user_id], false).await?;
            } else {
                crate::group_manager::remove_many(&mut tx, &cx, group, &[user_id]).await?;
            }
        }
        tx.commit().await?;
        return Ok((StatusCode::OK, Json(json!({ "success": "OK" }))).into_response());
    }

    // ensure_can_see_policy_users
    if post_policy.private && !guardian.is_admin() {
        return Ok(super::search::invalid_access(&state));
    }
    if !policy::can_see_groups_members(&mut tx, &guardian, &group_ids).await? {
        return Ok(json_error(&t("discourse_policy.error.no_permission")));
    }
    let offset = p
        .get("offset")
        .and_then(params::scalar)
        .map(|o| crate::ruby::to_i(&o).max(0))
        .unwrap_or(0);
    let kind = if action == Action::Accepted {
        Users::Accepted
    } else {
        Users::NotAccepted
    };
    let ids = post_policy
        .users(&mut tx, kind, Some(POLICY_USER_DEFAULT_LIMIT), offset)
        .await?;
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let users = policy::basic_users(&mut tx, &urls, &ids).await?;
    Ok((StatusCode::OK, Json(json!({ "users": users }))).into_response())
}

#[derive(serde::Deserialize, Default)]
pub struct FooterParams {
    /// `showNotAccepted`: the not-accepted list is the one shown.
    not_accepted: Option<String>,
}

/// GET /live/policy/:post_id: the post's PostPolicy component drawn again
/// for the viewer with the list static/js/policy.js toggled to (Ember
/// redraws the component in the browser; here the server does).
pub async fn footer(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    axum::extract::Path(post_id): axum::extract::Path<String>,
    axum::extract::Query(params): axum::extract::Query<FooterParams>,
) -> Result<Response, AppError> {
    let Ok(post_id) = post_id.parse::<i32>() else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    if !policy::enabled(&settings)? {
        return Ok(StatusCode::NOT_FOUND.into_response());
    }
    let deleted: Option<bool> =
        sqlx::query_scalar("SELECT deleted_at IS NOT NULL FROM posts WHERE id = $1")
            .bind(post_id)
            .fetch_optional(&mut *conn)
            .await?;
    let visible = deleted == Some(false)
        && crate::posting::revisions::find_post_with_deleted_in(
            &mut conn, &settings, &guardian, post_id,
        )
        .await?
        .is_some();
    if !visible {
        return Ok(StatusCode::NOT_FOUND.into_response());
    }
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let post = {
        let mut view = crate::topic_view::TopicView {
            conn: &mut conn,
            settings: &settings,
            i18n: &state.i18n,
            guardian: &guardian,
            urls: &urls,
            options: Default::default(),
            post_types: Vec::new(),
            logo_small_url: None,
        };
        view.serialize_single_post(post_id, false, false, true)
            .await?
    };
    let categories = crate::topic_list_view::categories(&mut conn).await?;
    drop(conn);
    let list = crate::topic_list_view::ListContext {
        i18n: &state.i18n,
        base_path: state.config.globals.relative_url_root(),
        now: crate::clock::now(),
        categories: &categories,
        expand_all_pinned: false,
        hide_category: false,
        member_trust_level: guardian.user().map(|u| u.trust_level),
        settings: crate::topic_list_view::ListSettings::load(&settings)?,
    };
    let post_settings = crate::post_view::PostSettings::load(&settings)?;
    let viewer = policy::view::Viewer {
        can_manage: guardian.is_staff() || post["yours"] == true,
        show_not_accepted: params.not_accepted.as_deref() == Some("1"),
        tiny_avatar_size: post_settings.avatar_size_24,
    };
    let Some(html) = policy::view::footer_for(&list, &viewer, &post) else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };
    Ok((
        [
            (axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (axum::http::header::CACHE_CONTROL, "no-cache, no-store"),
        ],
        html,
    )
        .into_response())
}

#[derive(serde::Deserialize, Default)]
pub struct BuilderParams {
    /// The post whose policy the settings button edits; none to insert.
    post_id: Option<String>,
}

/// GET /live/policy-builder: the PolicyBuilder modal's content for
/// static/js/policy.js, empty to insert a policy or filled from a post's
/// to edit it.
pub async fn builder(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    axum::extract::Query(params): axum::extract::Query<BuilderParams>,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    if !policy::enabled(&settings)? || guardian.is_anonymous() {
        return Ok(StatusCode::NOT_FOUND.into_response());
    }
    let cooked = match params.post_id.as_deref().filter(|p| !p.is_empty()) {
        Some(id) => {
            let Ok(post_id) = id.parse::<i32>() else {
                return Ok(StatusCode::NOT_FOUND.into_response());
            };
            let row: Option<(String, i32, i32)> = sqlx::query_as(
                "SELECT cooked, topic_id, post_number FROM posts WHERE id = $1 AND deleted_at IS NULL",
            )
            .bind(post_id)
            .fetch_optional(&mut *conn)
            .await?;
            let visible = row.is_some()
                && crate::posting::revisions::find_post_with_deleted_in(
                    &mut conn, &settings, &guardian, post_id,
                )
                .await?
                .is_some();
            if !visible {
                return Ok(StatusCode::NOT_FOUND.into_response());
            }
            // The raw the builder edits, from posts#by_number (GET
            // /posts/:id is not routed).
            row.map(|(cooked, topic_id, post_number)| {
                (
                    cooked,
                    format!(
                        "{}/posts/by_number/{topic_id}/{post_number}.json",
                        state.config.globals.relative_url_root()
                    ),
                )
            })
        }
        None => None,
    };
    // site.groups
    let rows: Vec<(i32, String, bool)> = sqlx::query_as(&format!(
        "SELECT id, name, automatic FROM groups WHERE {} ORDER BY name ASC",
        crate::groups::visible_groups_where(&guardian, "groups")
    ))
    .fetch_all(&mut *conn)
    .await?;
    let groups: Vec<policy::view::SiteGroup> = rows
        .into_iter()
        .map(|(id, name, automatic)| policy::view::SiteGroup {
            id,
            name,
            automatic,
        })
        .collect();
    let categories = crate::topic_list_view::categories(&mut conn).await?;
    drop(conn);
    let list = crate::topic_list_view::ListContext {
        i18n: &state.i18n,
        base_path: state.config.globals.relative_url_root(),
        now: crate::clock::now(),
        categories: &categories,
        expand_all_pinned: false,
        hide_category: false,
        member_trust_level: guardian.user().map(|u| u.trust_level),
        settings: crate::topic_list_view::ListSettings::load(&settings)?,
    };
    let (values, raw_url) = match cooked {
        Some((cooked, raw_url)) => match policy::view::BuilderValues::from_cooked(&list, &cooked) {
            Some(values) => (values, Some(raw_url)),
            None => return Ok(StatusCode::NOT_FOUND.into_response()),
        },
        None => (policy::view::BuilderValues::new_policy(), None),
    };
    let html = policy::view::builder(&list, &groups, &values, raw_url.as_deref());
    Ok((
        [
            (axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (axum::http::header::CACHE_CONTROL, "no-cache, no-store"),
        ],
        html,
    )
        .into_response())
}
