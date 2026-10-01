//! Port of ListController#private_messages* (`message_route`): the
//! `/topics/private-messages*/:username(.json)` lists.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};

use crate::pm_lists::{GroupView, PmList, PmQuery};
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::topic_list::TopicListSerializer;
use crate::url::Urls;
use crate::{AppError, AppState, Unsupported};

use super::list::{ListParams, build_options};

/// GET /topics/private-messages{-sent,-archive,-unread,-new,-warnings}/{username}(.json)
pub async fn personal(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path((kind, username)): Path<(String, String)>,
    Query(params): Query<ListParams>,
) -> Result<Response, AppError> {
    let username = username.trim_end_matches(".json");
    let list = match kind.as_str() {
        "private-messages" => PmList::Inbox,
        "private-messages-sent" => PmList::Sent,
        "private-messages-archive" => PmList::Archive,
        "private-messages-unread" => PmList::Unread,
        "private-messages-new" => PmList::New,
        "private-messages-warnings" => PmList::Warnings,
        _ => return Ok(super::topics::not_found_response(&state, false)),
    };
    respond(state, guardian, username, list, &params).await
}

/// GET /topics/private-messages-group/{username}/{group}(/archive|/new|/unread).json
pub async fn group(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path((username, rest)): Path<(String, String)>,
    Query(params): Query<ListParams>,
) -> Result<Response, AppError> {
    let rest = rest.trim_end_matches(".json");
    let (group_name, view) = match rest.rsplit_once('/') {
        Some((name, "archive")) => (name, GroupView::Archive),
        Some((name, "new")) => (name, GroupView::New),
        Some((name, "unread")) => (name, GroupView::Unread),
        Some(_) => return Ok(super::topics::not_found_response(&state, false)),
        None => (rest, GroupView::Inbox),
    };
    let mut conn = state.pool.acquire().await?;
    let row: Option<(i32, String)> =
        sqlx::query_as("SELECT id, name FROM groups WHERE LOWER(name) = $1")
            .bind(group_name.to_lowercase())
            .fetch_optional(&mut *conn)
            .await?;
    drop(conn);
    let Some((id, name)) = row else {
        return Ok(super::topics::not_found_response(&state, false));
    };
    respond(
        state,
        guardian,
        &username,
        PmList::Group { id, name, view },
        &params,
    )
    .await
}

async fn respond(
    state: AppState,
    guardian: crate::guardian::Guardian,
    username: &str,
    list: PmList,
    params: &ListParams,
) -> Result<Response, AppError> {
    // ensure_logged_in
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state, "/topics"));
    }
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    // fetch_user_from_params: by username_lower; staff may reach inactive users.
    let target: Option<(i32, bool)> =
        sqlx::query_as("SELECT id, active FROM users WHERE username_lower = $1 LIMIT 1")
            .bind(username.to_lowercase())
            .fetch_optional(&mut *conn)
            .await?;
    let owner = match target {
        Some((id, active)) if active || guardian.is_staff() => id,
        _ => return Ok(super::topics::not_found_response(&state, false)),
    };
    let me = guardian.is_me(owner);
    // The access check per action.
    match &list {
        PmList::Inbox | PmList::Sent | PmList::Archive => {
            // can_see_private_messages?
            if !me && !guardian.is_admin() {
                return Ok(super::search::invalid_access(&state));
            }
        }
        PmList::Unread | PmList::New => {
            if !me {
                return Ok(super::topics::not_found_response(&state, false));
            }
        }
        PmList::Warnings => {
            if !me && !guardian.is_staff() {
                return Ok(super::search::invalid_access(&state));
            }
        }
        PmList::Group { id, view, .. } => {
            if matches!(view, GroupView::New | GroupView::Unread) && !me {
                return Ok(super::topics::not_found_response(&state, false));
            }
            // can_see_group_messages?
            let moderators = crate::guardian::auto_groups::MODERATORS as i32;
            let allowed = guardian.is_admin()
                || (guardian.is_moderator() && *id == moderators)
                || (guardian.in_setting_groups(&settings, "personal_message_enabled_groups")?
                    && sqlx::query_scalar::<_, bool>(
                        "SELECT EXISTS (SELECT 1 FROM group_users WHERE group_id = $1 AND user_id = $2)",
                    )
                    .bind(id)
                    .bind(guardian.user_id())
                    .fetch_one(&mut *conn)
                    .await?);
            if !allowed {
                return Ok(super::topics::not_found_response(&state, false));
            }
        }
    }
    let mut options = match build_options(params, &settings) {
        Ok(o) => o,
        Err((status, message)) => return Ok((status, message).into_response()),
    };
    options.no_definitions = false;
    if params.period.is_some() {
        return Err(Unsupported("period on message lists").into());
    }
    let group_id = match &list {
        PmList::Group { id, .. } => Some(*id),
        _ => None,
    };
    let topics = PmQuery {
        conn: &mut conn,
        settings: &settings,
        guardian: &guardian,
        options: options.clone(),
        owner_id: owner,
    }
    .list(&list)
    .await?;
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    // more_topics_url is only serialized with a full page, which the
    // reference never fills; the path shape is the Rails one.
    let more = format!(
        "{}/topics/{}/{}?page={}",
        state.config.globals.relative_url_root(),
        match &list {
            PmList::Inbox => "private-messages".to_string(),
            PmList::Sent => "private-messages-sent".to_string(),
            PmList::Archive => "private-messages-archive".to_string(),
            PmList::Unread => "private-messages-unread".to_string(),
            PmList::New => "private-messages-new".to_string(),
            PmList::Warnings => "private-messages-warnings".to_string(),
            PmList::Group { .. } => "private-messages-group".to_string(),
        },
        username,
        options.page + 1
    );
    let json = TopicListSerializer {
        conn: &mut conn,
        settings: &settings,
        i18n: &state.i18n,
        guardian: &guardian,
        urls: &urls,
        more_topics_url: Some(more),
        category_id: None,
        group_id,
        prefetched: Default::default(),
    }
    .serialize(&topics)
    .await?;
    Ok((
        StatusCode::OK,
        [(header::CACHE_CONTROL, "no-cache, no-store")],
        Json(json),
    )
        .into_response())
}
