//! GET /live: a page's live updates as server-sent events of HTML for
//! htmx, one stream per page. Every event is htmx out-of-band swaps into
//! the page, rendered for this viewer:
//!
//! - `post`, on a topic page (`topic=<id>`): each change on the topic's
//!   channel that is about a post, the post appended when created,
//!   replaced when changed, removed when deleted or not visible to them.
//! - `list`, on list pages and for members everywhere: the latest list's
//!   "N new or updated topics" banner (`filter=latest`, `since` the page's
//!   render time), the viewer's own list query run again, and a member's
//!   new and unread counts from their TopicTrackingState report, in the
//!   nav pills (`nav`, the active pill) and on the sidebar's links
//!   (`sidebar`, its Active key); on connect and after the tracking
//!   messages they may hear.
//! - `header`, for members: the unread notification count, on connect and
//!   from their notification state, and the alert for a new notification.
//!
//! The stream starts after the Last-Event-ID header (the browser
//! reconnecting), else `position` (the page's `bus-position`), else now;
//! an event's id is the bus position it brings the page up to. Who may hear
//! what is decided by the viewer's audience tags, as for /bus/events.
//! Messages that arrive together are handled as one batch: the lists are
//! counted once for it, and nothing is sent when nothing changed.

use std::collections::VecDeque;
use std::convert::Infallible;
use std::time::Duration;

use askama::Template;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use chrono::NaiveDateTime;
use futures_util::stream;
use pg_bus::{Filter, Item, Message, Position, Subscription};
use serde::Deserialize;
use serde_json::Value;

use crate::guardian::Guardian;
use crate::html::PostFragment;
use crate::posting::revisions::find_post_with_deleted;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::url::Urls;
use crate::{AppError, AppState};

/// The stream ends after about this long and the browser reconnects with
/// its last event id, which checks the viewer's session again.
const MAX_LIFETIME: Duration = Duration::from_secs(600);

/// TopicTrackingState's channels; a member's own `/unread/<id>` too.
const TRACKING_CHANNELS: [&str; 5] = ["/latest", "/new", "/unread", "/delete", "/recover"];

#[derive(Deserialize, Default)]
pub struct Params {
    position: Option<String>,
    /// The topic the page shows.
    topic: Option<String>,
    /// The page is the topic's last, where new posts are appended.
    tail: Option<String>,
    /// The list the page shows (`latest`, `new`, `unread`).
    filter: Option<String>,
    /// When the page was rendered, in milliseconds: on the latest list,
    /// topics bumped after it are the new or updated ones.
    since: Option<i64>,
    /// The page's active nav pill: a member's counts in the pills.
    nav: Option<String>,
    /// The page's sidebar (`Active::key`): a member's counts on its links.
    sidebar: Option<String>,
}

/// GET /live
pub async fn page(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Query(params): Query<Params>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let topic_id = match params.topic.as_deref().filter(|t| !t.is_empty()) {
        Some(raw) => {
            let Ok(id) = raw.parse::<i32>() else {
                return Ok(StatusCode::NOT_FOUND.into_response());
            };
            // A topic the viewer cannot see has no live updates for them.
            let visible =
                match crate::topic_guardian::TopicCtx::load(&mut conn, &settings, &guardian, id)
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
            Some(id)
        }
        None => None,
    };
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
    let since = match params.filter.as_deref() {
        Some("latest") => params
            .since
            .and_then(chrono::DateTime::from_timestamp_millis)
            .map(|t| t.naive_utc()),
        _ => None,
    };
    let user_id = guardian.user_id();
    // Lists are kept current for the banner, and for a member's counts.
    let lists = since.is_some() || user_id.is_some();

    let mut channels = Vec::new();
    if let Some(id) = topic_id {
        channels.push(crate::bus::topic_channel(id));
    }
    if lists {
        channels.extend(TRACKING_CHANNELS.iter().map(|c| c.to_string()));
    }
    if let Some(id) = user_id {
        channels.push(format!("/unread/{id}"));
        channels.push(crate::bus::notification_channel(id));
        channels.push(format!("/notification-alert/{id}"));
    }
    let subscription = state.bus.subscribe(from, Filter { channels, tags });
    let live = Live {
        state,
        guardian,
        settings,
        subscription,
        topic_channel: topic_id.map(crate::bus::topic_channel),
        tail: params.tail.as_deref() == Some("1"),
        user_id,
        lists,
        since,
        nav: params.nav.filter(|_| user_id.is_some()),
        sidebar: params
            .sidebar
            .filter(|_| user_id.is_some())
            .map(|key| crate::sidebar::Active::parse(&key)),
        sent_lists: None,
        started: false,
        pending: VecDeque::new(),
        deadline: tokio::time::Instant::now() + MAX_LIFETIME,
    };
    let stream = stream::unfold(live, |mut live| async move {
        loop {
            if let Some(event) = live.pending.pop_front() {
                return Some((Ok::<_, Infallible>(event), live));
            }
            let result = if live.started {
                next_batch(&mut live).await?
            } else {
                live.started = true;
                initial(&mut live).await
            };
            if let Err(e) = result {
                tracing::warn!("live: {e}");
                return None;
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
    Ok(response)
}

struct Live {
    state: AppState,
    guardian: Guardian,
    settings: SiteSettings,
    subscription: Subscription,
    topic_channel: Option<String>,
    /// New posts are appended (the topic's last page).
    tail: bool,
    user_id: Option<i32>,
    lists: bool,
    since: Option<NaiveDateTime>,
    /// The nav pill and sidebar a member's counts are kept on.
    nav: Option<String>,
    sidebar: Option<crate::sidebar::Active>,
    /// The list HTML last computed, sent or (when empty) not.
    sent_lists: Option<String>,
    started: bool,
    /// Events rendered and not yet sent, in bus order.
    pending: VecDeque<Event>,
    deadline: tokio::time::Instant,
}

/// What the page needs as soon as it connects: the member's notification
/// count, and the lists as they are now.
async fn initial(live: &mut Live) -> Result<(), AppError> {
    if let Some(user_id) = live.user_id {
        let mut conn = live.state.pool.acquire().await?;
        let count = crate::bus::all_unread_notifications_count(&mut conn, user_id).await?;
        live.pending.push_back(
            Event::default()
                .event("header")
                .data(notification_count(count)),
        );
    }
    if live.lists {
        recount_lists(live, None).await?;
    }
    Ok(())
}

/// Waits for the next message and takes whatever else is already due with
/// it; `None` when the stream's lifetime is up.
async fn next_batch(live: &mut Live) -> Option<Result<(), AppError>> {
    let first = tokio::time::timeout_at(live.deadline, live.subscription.next())
        .await
        .ok()?;
    let mut batch = vec![first];
    while let Ok(item) = tokio::time::timeout(Duration::ZERO, live.subscription.next()).await {
        batch.push(item);
    }
    Some(handle_batch(live, batch).await)
}

async fn handle_batch(
    live: &mut Live,
    batch: Vec<Result<Item, pg_bus::Error>>,
) -> Result<(), AppError> {
    let notification = live.user_id.map(crate::bus::notification_channel);
    let alert = live.user_id.map(|id| format!("/notification-alert/{id}"));
    let mut tracking: Option<Position> = None;
    let mut last: Option<Position> = None;
    for item in batch {
        let message = match item? {
            Item::Message(message) => message,
            // Trimmed before this viewer read it: the page is stale, and a
            // reload is the honest fix. Not sent yet.
            Item::Gap => continue,
        };
        last = Some(message.position);
        let id = message.position.to_string();
        if live.topic_channel.as_ref() == Some(&message.channel) {
            if let Some(html) = post_fragment(live, &message).await? {
                live.pending
                    .push_back(Event::default().event("post").id(id).data(html));
            }
        } else if notification.as_ref() == Some(&message.channel) {
            let count = message.data["all_unread_notifications_count"]
                .as_i64()
                .unwrap_or(0);
            live.pending.push_back(
                Event::default()
                    .event("header")
                    .id(id)
                    .data(notification_count(count)),
            );
        } else if alert.as_ref() == Some(&message.channel) {
            let base = live.state.config.globals.relative_url_root();
            live.pending.push_back(
                Event::default()
                    .event("header")
                    .id(id)
                    .data(notification_alert(base, &message.data)),
            );
        } else {
            tracking = Some(message.position);
        }
    }
    if tracking.is_some() && live.lists {
        // Carries the batch's last position, so the ids stay in bus order.
        recount_lists(live, last).await?;
    }
    Ok(())
}

/// The lists counted again; queued only when they changed.
async fn recount_lists(live: &mut Live, id: Option<Position>) -> Result<(), AppError> {
    let html = list_state(live).await?;
    if live.sent_lists.as_ref() == Some(&html) {
        return Ok(());
    }
    live.sent_lists = Some(html.clone());
    if html.is_empty() {
        return Ok(());
    }
    let mut event = Event::default().event("list").data(html);
    if let Some(position) = id {
        event = event.id(position.to_string());
    }
    live.pending.push_back(event);
    Ok(())
}

/// The unread notification count, the badge on a member's avatar
/// (empty, and so hidden, at none).
fn notification_count(count: i64) -> String {
    let shown = if count > 0 {
        count.to_string()
    } else {
        String::new()
    };
    format!(
        r#"<span id="notification-count" class="badge-notification unread-notifications" hx-swap-oob="true">{shown}</span>"#
    )
}

/// The alert PostAlerter publishes, shown in the header: who, what and
/// where, linking to the post. Everything in it is escaped: titles,
/// usernames and excerpts are user content.
fn notification_alert(base_path: &str, data: &Value) -> String {
    use html_escape::{encode_double_quoted_attribute as attr, encode_text as text};
    let s = |key: &str| data[key].as_str().unwrap_or_default();
    let verb = crate::html::notification_verb(data["notification_type"].as_i64().unwrap_or(0));
    format!(
        r#"<div id="notification-alert" class="notification-alert" role="status" aria-live="polite" hx-swap-oob="true"><a href="{}{}"><strong>{}</strong> {verb} <em>{}</em>: {}</a></div>"#,
        attr(base_path),
        attr(s("post_url")),
        text(s("username")),
        text(s("topic_title")),
        text(s("excerpt")),
    )
}

/// The HTML for one change on the topic's channel: the post as this
/// viewer sees it now, or its removal; `None` for messages that are not
/// about a post (the topic's stats, the viewer's notification level).
async fn post_fragment(live: &Live, message: &Message) -> Result<Option<String>, AppError> {
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
    // A new post belongs at the end of the topic, not on an earlier page.
    if kind == "created" && !live.tail {
        return Ok(None);
    }
    let html = render_post(
        &live.state,
        &live.settings,
        &live.guardian,
        post_id as i32,
        kind == "created",
    )
    .await?;
    // Deleted, or no longer visible to them: it goes from the page, its
    // wrapper with it.
    Ok(Some(html.unwrap_or_else(|| {
        format!(r#"<div hx-swap-oob="delete:#posts > [data-post-number='{post_number}']"></div>"#)
    })))
}

/// One post as this viewer sees it now, as an out-of-band swap: appended
/// to the posts (`append`) or replacing the post in place. `None` when it
/// is deleted or not visible to them.
async fn render_post(
    state: &AppState,
    settings: &SiteSettings,
    guardian: &Guardian,
    post_id: i32,
    append: bool,
) -> Result<Option<String>, AppError> {
    let host = crate::pretty_text::Host::from_state(state);
    let ctx = crate::posting::Ctx {
        host: &host,
        settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    let mut conn = state.pool.acquire().await?;
    let access = find_post_with_deleted(&mut conn, &ctx, guardian, post_id).await?;
    let visible = access.as_ref().is_some_and(|a| {
        !a.post.trashed()
            && guardian
                .can_see_post(settings, &a.post, a.can_see_topic)
                .unwrap_or(false)
    });
    if !visible {
        return Ok(None);
    }
    let urls = Urls {
        config: &state.config,
        settings,
    };
    let mut view = crate::topic_view::TopicView {
        conn: &mut conn,
        settings,
        i18n: &state.i18n,
        guardian,
        urls: &urls,
        options: crate::topic_view::Options {
            page: 0,
            post_number: None,
        },
        post_types: Vec::new(),
    };
    let post = view
        .serialize_single_post(post_id, false, false, true)
        .await?;
    let base = state.config.globals.relative_url_root();
    let topic_id = post["topic_id"].as_i64().unwrap_or(0) as i32;
    let Some(topic_ctx) =
        crate::topic_guardian::TopicCtx::load(&mut conn, settings, guardian, topic_id).await?
    else {
        return Ok(None);
    };
    // details.can_create_post, as the topic view computes it.
    let can_create_post = guardian.is_authenticated() && {
        let anywhere = guardian
            .can_create_post_anywhere(&mut conn, settings)
            .await?;
        guardian.can_create_post_on_topic(settings, &topic_ctx, anywhere)?
    };
    let topic = crate::post_view::TopicInfo {
        id: i64::from(topic_id),
        slug: post["topic_slug"].as_str().unwrap_or_default().to_string(),
        created_by_id: topic_ctx.user_id.map(i64::from),
        archived: topic_ctx.archived,
        can_create_post,
        deleted: topic_ctx.deleted_at.is_some(),
        can_delete: guardian.can_delete_topic(settings, &topic_ctx)?,
        can_recover: guardian.can_recover_topic(&topic_ctx),
        defer_to: String::new(),
        op_map: String::new(),
    };
    // The post shown above it, for its reply-to tab and time gap.
    let number = post["post_number"].as_i64().unwrap_or(0) as i32;
    let prev: Option<(i32, chrono::NaiveDateTime)> = sqlx::query_as(
        "SELECT post_number, created_at FROM posts WHERE topic_id = $1 AND post_number < $2 \
         AND deleted_at IS NULL ORDER BY post_number DESC LIMIT 1",
    )
    .bind(topic_id)
    .bind(number)
    .fetch_optional(&mut *conn)
    .await?;
    let prev = prev.map(|(post_number, created_at)| crate::post_view::Prev {
        post_number: i64::from(post_number),
        created_at: created_at.and_utc(),
    });
    let categories = crate::topic_list_view::categories(&mut conn).await?;
    let list = crate::topic_list_view::ListContext {
        i18n: &state.i18n,
        base_path: base,
        now: chrono::Utc::now(),
        categories: &categories,
        expand_all_pinned: false,
        member_trust_level: guardian.user().map(|u| u.trust_level),
        settings: crate::topic_list_view::ListSettings::load(settings)?,
    };
    let post_settings = crate::post_view::PostSettings::load(settings)?;
    let cx = crate::post_view::PostContext {
        list: &list,
        settings: &post_settings,
        topic: &topic,
        viewer: guardian.user().map(|u| u.username.as_str()),
        staff: guardian.is_staff(),
        can_send_pms: guardian.is_authenticated()
            && guardian.can_send_private_messages(settings)?,
    };
    let html = if append {
        crate::post_view::post(&cx, &post, prev)
    } else if number == 1 && post["post_type"].as_i64() != Some(3) {
        // The first post's main row only: its topic map, which needs the
        // whole topic view, stays as the page rendered it.
        crate::post_view::main_row(&cx, &post, prev).replacen(
            "<div class=\"post__row row\">",
            "<div class=\"post__row row\" hx-swap-oob=\"outerHTML:#post_1 > .post__row:has(> .post__body)\">",
            1,
        )
    } else {
        // Replaces the post where it is shown: its wrapper carries the
        // out-of-band swap.
        crate::post_view::post_body(&cx, &post, prev).replacen(
            &format!("data-post-number=\"{number}\""),
            &format!(
                "data-post-number=\"{number}\" hx-swap-oob=\"outerHTML:#posts > [data-post-number='{number}']\""
            ),
            1,
        )
    };
    let html = PostFragment { html, append }.render()?;
    Ok(Some(html))
}

/// The latest list's banner, and a member's counts from their tracking
/// state: the nav pills' labels (`nav`) and the sidebar's counted links
/// (`sidebar`), each only where the page has them.
async fn list_state(live: &Live) -> Result<String, AppError> {
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
    if live.nav.is_none() && live.sidebar.is_none() {
        return Ok(out);
    }
    let tracking =
        crate::topic_tracking_report::load(&mut conn, &live.settings, &live.guardian).await?;
    if tracking.is_none() {
        return Ok(out);
    }
    let base_path = live.state.config.globals.relative_url_root();
    if let Some(nav) = &live.nav {
        // The pills' labels: only new and unread count.
        for item in crate::html::nav_items(
            &live.state.i18n,
            &live.settings,
            base_path,
            nav,
            tracking.as_ref(),
        )? {
            if item.name == "new" || item.name == "unread" {
                out.push_str(&format!(
                    r#"<a hx-swap-oob="innerHTML:#navigation-bar > li.nav-item_{} > a">{}</a>"#,
                    item.name,
                    crate::topic_list_view::escape(&item.label)
                ));
            }
        }
    }
    if let Some(active) = &live.sidebar {
        let (site, member) = crate::html::sidebar_inputs(
            &mut conn,
            &live.state,
            &live.settings,
            &live.guardian,
            tracking,
        )
        .await?;
        let emoji_set = live.settings.get("emoji_set")?.to_s().to_string();
        let links = crate::sidebar::tracked_links(
            &site,
            &crate::sidebar::Context {
                i18n: &live.state.i18n,
                settings: &live.settings,
                base_path,
                active,
                member: member.as_ref(),
                emoji_set: &emoji_set,
            },
        )?;
        out.push_str(&links.concat());
    }
    Ok(out)
}

/// The viewer's own list: `TopicQuery#list_<filter>` with these options.
async fn viewer_list(
    live: &Live,
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

/// GET /live/post/:id: one post as the viewer sees it now, as an
/// out-of-band swap replacing it on the page. For changes that are the
/// viewer's alone, such as a bookmark, which nothing publishes: the page
/// fetches the post again once the change is made. 404 for a post they
/// cannot see.
pub async fn post(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Response, AppError> {
    let Ok(post_id) = id.parse::<i32>() else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    drop(conn);
    match render_post(&state, &settings, &guardian, post_id, false).await? {
        Some(html) => Ok((
            [
                (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                (header::CACHE_CONTROL, "no-cache, no-store"),
            ],
            html,
        )
            .into_response()),
        None => Ok(StatusCode::NOT_FOUND.into_response()),
    }
}
