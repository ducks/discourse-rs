//! GET /t/:topic_id/live: the topic page's live updates as server-sent
//! events of HTML for htmx. Each change on `/topic/<id>` that is about a
//! post becomes a `post` event carrying that post rendered for this
//! viewer as an out-of-band swap: appended when created, replaced when
//! changed, removed when deleted or no longer visible to them.
//!
//! The stream starts after the Last-Event-ID header (the browser
//! reconnecting), else `position` (the page's `bus-position`), else now;
//! each event's id is its bus position. Who may hear what is decided by
//! the viewer's audience tags, as for /bus/events, and the post itself is
//! checked against the viewer's guardian before it is rendered.

use std::convert::Infallible;
use std::time::Duration;

use askama::Template;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use futures_util::stream;
use pg_bus::{Filter, Item, Message, Subscription};
use serde::Deserialize;

use crate::guardian::Guardian;
use crate::html::{PostFragment, post_item};
use crate::posting::revisions::find_post_with_deleted;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::url::Urls;
use crate::{AppError, AppState};

/// The stream ends after about this long and the browser reconnects with
/// its last event id, which checks the viewer's session again.
const MAX_LIFETIME: Duration = Duration::from_secs(600);

#[derive(Deserialize, Default)]
pub struct Params {
    position: Option<String>,
}

/// GET /t/:topic_id/live
pub async fn topic(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    Query(params): Query<Params>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let Ok(topic_id) = id.parse::<i32>() else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    // A topic the viewer cannot see has no live updates for them.
    let visible =
        match crate::topic_guardian::TopicCtx::load(&mut conn, &settings, &guardian, topic_id)
            .await?
        {
            Some(topic) => {
                let secure = guardian.secure_category_ids(&mut conn, &settings).await?;
                guardian.can_see_topic(&settings, &topic, true, &secure)?
            }
            None => false,
        };
    if !visible {
        return Ok(StatusCode::NOT_FOUND.into_response());
    }
    let tags = crate::bus::tags(&mut conn, guardian.user_id()).await?;
    drop(conn);
    let from = match pg_bus::sse::last_event_id(&headers) {
        Some(position) => position,
        None => match params.position.as_deref().filter(|p| !p.is_empty()) {
            Some(raw) => match raw.parse() {
                Ok(position) => position,
                Err(_) => return Ok((StatusCode::BAD_REQUEST, "invalid position").into_response()),
            },
            None => state.bus.now().await?,
        },
    };
    let subscription = state.bus.subscribe(
        from,
        Filter {
            channels: vec![crate::bus::topic_channel(topic_id)],
            tags,
        },
    );
    Ok(events(state, guardian, settings, subscription))
}

struct Live {
    state: AppState,
    guardian: Guardian,
    settings: SiteSettings,
    subscription: Subscription,
    deadline: tokio::time::Instant,
}

fn events(
    state: AppState,
    guardian: Guardian,
    settings: SiteSettings,
    subscription: Subscription,
) -> Response {
    let live = Live {
        state,
        guardian,
        settings,
        subscription,
        deadline: tokio::time::Instant::now() + MAX_LIFETIME,
    };
    let stream = stream::unfold(live, |mut live| async move {
        loop {
            // next is cancel-safe: ending here loses nothing.
            let item = tokio::time::timeout_at(live.deadline, live.subscription.next())
                .await
                .ok()?;
            let message = match item {
                Ok(Item::Message(message)) => message,
                // Trimmed before this viewer read it: the page is stale, and
                // a reload is the honest fix. Not sent yet; the client has
                // nothing to do with it.
                Ok(Item::Gap) => continue,
                Err(e) => {
                    tracing::warn!("live topic: {e}");
                    return None;
                }
            };
            match fragment(&live, &message).await {
                Ok(Some(html)) => {
                    let event = Event::default()
                        .event("post")
                        .id(message.position.to_string())
                        .data(html);
                    return Some((Ok::<_, Infallible>(event), live));
                }
                Ok(None) => continue,
                Err(e) => {
                    tracing::warn!("live topic: rendering a post: {e}");
                    return None;
                }
            }
        }
    });
    let mut response = Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(25)))
        .into_response();
    // nginx buffers proxied responses unless told not to.
    response
        .headers_mut()
        .insert("x-accel-buffering", HeaderValue::from_static("no"));
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    response
}

/// The HTML for one message: the post as this viewer sees it now, or its
/// removal; `None` for messages that are not about a post (the topic's
/// stats, the viewer's notification level).
async fn fragment(live: &Live, message: &Message) -> Result<Option<String>, AppError> {
    let data = &message.data;
    let Some(kind) = data["type"].as_str() else {
        return Ok(None);
    };
    if kind == "stats" {
        return Ok(None);
    }
    let (Some(post_id), Some(post_number)) = (data["id"].as_i64(), data["post_number"].as_i64())
    else {
        return Ok(None);
    };
    let removal = || format!(r#"<div id="post_{post_number}" hx-swap-oob="delete"></div>"#);
    let state = &live.state;
    let host = crate::pretty_text::Host::from_state(state);
    let ctx = crate::posting::Ctx {
        host: &host,
        settings: &live.settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    let mut conn = state.pool.acquire().await?;
    let access = find_post_with_deleted(&mut conn, &ctx, &live.guardian, post_id as i32).await?;
    let visible = access.as_ref().is_some_and(|a| {
        !a.post.trashed()
            && live
                .guardian
                .can_see_post(&live.settings, &a.post, a.can_see_topic)
                .unwrap_or(false)
    });
    if !visible {
        return Ok(Some(removal()));
    }
    let urls = Urls {
        config: &state.config,
        settings: &live.settings,
    };
    let mut view = crate::topic_view::TopicView {
        conn: &mut conn,
        settings: &live.settings,
        i18n: &state.i18n,
        guardian: &live.guardian,
        urls: &urls,
        options: crate::topic_view::Options {
            page: 0,
            post_number: None,
        },
        post_types: Vec::new(),
    };
    let post = view
        .serialize_single_post(post_id as i32, false, false)
        .await?;
    let base = state.config.globals.relative_url_root();
    let topic_url = format!(
        "{base}/t/{}/{}",
        post["topic_slug"].as_str().unwrap_or_default(),
        post["topic_id"]
    );
    let html = PostFragment {
        post: post_item(&state.i18n, base, &topic_url, &post),
        append: kind == "created",
    }
    .render()?;
    Ok(Some(html))
}

#[derive(Deserialize, Default)]
pub struct ListParams {
    position: Option<String>,
    /// The list the page shows (`latest`, `new`, `unread`): the latest
    /// list also gets the new-or-updated banner.
    filter: Option<String>,
    /// When the page was rendered, in milliseconds: topics bumped after it
    /// are the new or updated ones.
    since: Option<i64>,
}

/// GET /live/lists: what a topic list page keeps current, as `list`
/// events of htmx out-of-band HTML: the "N new or updated topics" banner
/// on the latest list, and a logged-in viewer's unread and new counts in
/// the nav. Each is the viewer's own list query run again (their muting,
/// categories, permissions), on connect and after any tracking message
/// they may hear; a burst of messages is one recount, and nothing is sent
/// when nothing changed.
pub async fn lists(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Query(params): Query<ListParams>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let tags = crate::bus::tags(&mut conn, guardian.user_id()).await?;
    drop(conn);
    let from = match pg_bus::sse::last_event_id(&headers) {
        Some(position) => position,
        None => match params.position.as_deref().filter(|p| !p.is_empty()) {
            Some(raw) => match raw.parse() {
                Ok(position) => position,
                Err(_) => return Ok((StatusCode::BAD_REQUEST, "invalid position").into_response()),
            },
            None => state.bus.now().await?,
        },
    };
    let mut channels: Vec<String> = ["/latest", "/new", "/unread", "/delete", "/recover"]
        .into_iter()
        .map(str::to_string)
        .collect();
    if let Some(user_id) = guardian.user_id() {
        channels.push(format!("/unread/{user_id}"));
    }
    let since = match params.filter.as_deref() {
        Some("latest") => params
            .since
            .and_then(chrono::DateTime::from_timestamp_millis)
            .map(|t| t.naive_utc()),
        _ => None,
    };
    let subscription = state.bus.subscribe(from, Filter { channels, tags });
    let live = ListLive {
        state,
        guardian,
        settings,
        subscription,
        since,
        sent: None,
        deadline: tokio::time::Instant::now() + MAX_LIFETIME,
    };
    let stream = stream::unfold(live, |mut live| async move {
        // First the state as it is now, then again after each burst.
        let mut id = None;
        loop {
            if live.sent.is_some() {
                let item = tokio::time::timeout_at(live.deadline, live.subscription.next())
                    .await
                    .ok()?;
                match item {
                    Ok(Item::Message(m)) => id = Some(m.position),
                    Ok(Item::Gap) => {}
                    Err(e) => {
                        tracing::warn!("live lists: {e}");
                        return None;
                    }
                }
                // Whatever else is already due is part of the same recount.
                while let Ok(item) =
                    tokio::time::timeout(Duration::ZERO, live.subscription.next()).await
                {
                    match item {
                        Ok(Item::Message(m)) => id = Some(m.position),
                        Ok(Item::Gap) => {}
                        Err(e) => {
                            tracing::warn!("live lists: {e}");
                            return None;
                        }
                    }
                }
            }
            let html = match list_state(&live).await {
                Ok(html) => html,
                Err(e) => {
                    tracing::warn!("live lists: counting: {e}");
                    return None;
                }
            };
            if live.sent.as_ref() == Some(&html) {
                continue;
            }
            live.sent = Some(html.clone());
            if html.is_empty() {
                continue;
            }
            let mut event = Event::default().event("list").data(html);
            if let Some(position) = id {
                event = event.id(position.to_string());
            }
            return Some((Ok::<_, Infallible>(event), live));
        }
    });
    let mut response = Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(25)))
        .into_response();
    response
        .headers_mut()
        .insert("x-accel-buffering", HeaderValue::from_static("no"));
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    Ok(response)
}

struct ListLive {
    state: AppState,
    guardian: Guardian,
    settings: SiteSettings,
    subscription: Subscription,
    since: Option<chrono::NaiveDateTime>,
    /// The HTML last computed, sent or (when empty) not.
    sent: Option<String>,
    deadline: tokio::time::Instant,
}

/// The most a count shows before "99+".
const COUNT_CAP: i64 = 100;

async fn list_state(live: &ListLive) -> Result<String, AppError> {
    use crate::topic_query::{Filter as ListFilter, Options};
    let mut conn = live.state.pool.acquire().await?;
    let mut out = String::new();
    if let Some(since) = live.since {
        // The front page's latest list: no category definitions.
        let list = viewer_list(
            live,
            &mut conn,
            ListFilter::Latest,
            Options {
                no_definitions: true,
                ..Default::default()
            },
        )
        .await?;
        let n = list.topics.iter().filter(|t| t.bumped_at > since).count();
        let banner = match n {
            0 => String::new(),
            1 => r#"<a href="">1 new or updated topic. Show</a>"#.to_string(),
            n => format!(r#"<a href="">{n} new or updated topics. Show</a>"#),
        };
        out.push_str(&format!(
            r#"<div id="list-updates" class="list-updates" hx-swap-oob="true">{banner}</div>"#
        ));
    }
    if live.guardian.user_id().is_some() {
        for (filter, id) in [
            (ListFilter::Unread, "unread-count"),
            (ListFilter::New, "new-count"),
        ] {
            let list = viewer_list(
                live,
                &mut conn,
                filter,
                Options {
                    per_page: Some(COUNT_CAP),
                    ..Default::default()
                },
            )
            .await?;
            let n = list.topics.len() as i64;
            let count = match n {
                0 => String::new(),
                n if n >= COUNT_CAP => " (99+)".to_string(),
                n => format!(" ({n})"),
            };
            out.push_str(&format!(
                r#"<span id="{id}" hx-swap-oob="true">{count}</span>"#
            ));
        }
    }
    Ok(out)
}

/// The viewer's own list: `TopicQuery#list_<filter>` with these options.
async fn viewer_list(
    live: &ListLive,
    conn: &mut sqlx::PgConnection,
    filter: crate::topic_query::Filter,
    options: crate::topic_query::Options,
) -> Result<crate::topic_query::TopicList, AppError> {
    Ok(crate::topic_query::TopicQuery {
        conn,
        settings: &live.settings,
        guardian: &live.guardian,
        options,
        category: Default::default(),
        tags: Default::default(),
        filter: Default::default(),
        user: Default::default(),
    }
    .list(filter)
    .await?)
}
