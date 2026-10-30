//! Full-page chat's routes (Chat::ChatController#respond), rendered here
//! as the Ember routes would draw them: /chat and /chat/channels redirect
//! as ChatIndexRoute and ChatChannelsRoute do, the browse tabs, a
//! channel (and a message in it), and the disabled page.

use askama::Template;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{Html, IntoResponse, Response};

use crate::html::{Chrome, Crawler, Viewer};
use crate::plugins::chat::channels::{Context, Found};
use crate::plugins::chat::page::{self, BROWSE_TABS};
use crate::session::current::AuthGuardian;
use crate::sidebar::Active;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

#[derive(Template)]
#[template(path = "chat.html")]
pub struct ChatPage {
    pub site_title: String,
    pub lang: String,
    pub base_path: String,
    pub crawler: Crawler,
    pub viewer: Option<Viewer>,
    pub bus_position: String,
    pub chrome: Chrome,
    /// The route's title tokens before the site's ("#General - Chat - ").
    pub title: String,
    pub main: String,
}

fn redirect(to: String) -> Response {
    (StatusCode::FOUND, [(header::LOCATION, to)]).into_response()
}

/// `discoveryHomepageRoute` for a member: the first top menu item.
fn homepage(settings: &SiteSettings, base_path: &str) -> Result<String, AppError> {
    let top_menu = settings.get("top_menu")?.to_s().to_string();
    let first = top_menu
        .split('|')
        .next()
        .and_then(|i| i.split(',').next())
        .unwrap_or("latest");
    Ok(format!("{base_path}/{first}"))
}

/// requires_plugin, then the chat route's beforeModel: chat turned off in
/// the member's preferences goes to /chat/disabled, and anyone who can't
/// chat to the homepage.
async fn gate(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    guardian: &crate::guardian::Guardian,
    disabled_route: bool,
) -> Result<Result<SiteSettings, Response>, AppError> {
    let settings =
        SiteSettings::load(&mut *conn, &state.site_setting_defs, &state.config.globals).await?;
    let base_path = state.config.globals.relative_url_root();
    if !crate::plugins::chat::enabled(&settings)? {
        return Ok(Err(super::topics::not_found_response(state)));
    }
    let can_chat = crate::plugins::chat::can_chat(&mut *conn, &settings, guardian).await?;
    let chat_enabled = match guardian.user_id() {
        Some(id) => crate::plugins::chat::options(&mut *conn, id)
            .await?
            .is_some_and(|o| o.chat_enabled),
        None => false,
    };
    // chatDisabledInPreferences
    if can_chat && !chat_enabled {
        if disabled_route {
            return Ok(Ok(settings));
        }
        return Ok(Err(redirect(format!("{base_path}/chat/disabled"))));
    }
    if !(can_chat && chat_enabled) {
        return Ok(Err(redirect(homepage(&settings, base_path)?)));
    }
    if disabled_route {
        // chat.disabled for a member with chat on: nothing redirects them.
        return Ok(Ok(settings));
    }
    Ok(Ok(settings))
}

/// The page around a route's content.
#[allow(clippy::too_many_arguments)]
async fn render(
    state: &AppState,
    headers: &HeaderMap,
    conn: &mut sqlx::PgConnection,
    settings: &SiteSettings,
    guardian: &crate::guardian::Guardian,
    active: Active,
    title: String,
    content: &str,
    chat_view: bool,
) -> Result<Response, AppError> {
    let base_path = state.config.globals.relative_url_root();
    let bus_position = crate::bus::page_position(&state.bus).await?;
    let vs = super::session::viewer_state(state, headers, settings, guardian)?;
    let mut site = crate::html::Site::from_settings(settings, base_path)?;
    site.viewer = vs.viewer.clone();
    site.load_chrome(&mut *conn, state, settings, guardian, active)
        .await?;
    site.bus_position = bus_position;
    // templates/chat.gjs: bodyClass and htmlClass.
    site.chrome
        .body_classes
        .push_str(" has-chat has-full-page-chat");
    site.chrome
        .html_classes
        .push_str(" has-chat has-full-page-chat");
    // The chat routes leave out the application footer.
    site.chrome.show_footer = false;
    // ChatHeader in the mobile view: back to the forum (the last forum
    // page in the app, previousURL, which chat-drawer.js puts in) and the
    // Chat heading in the logo's place.
    if crate::mobile::mobile_view() {
        let t = |key: &str| {
            crate::topic_list_view::escape(state.i18n.t(&format!("js.{key}")).unwrap_or_default())
        };
        let forum = t("chat.back_to_forum");
        site.chrome.chat_header = format!(
            "<div class=\"c-header\"><a class=\"btn-flat back-to-forum\" href=\"{base_path}/\" title=\"{forum}\">{}{forum}</a><a class=\"c-heading\" href=\"{base_path}/chat\">{}</a></div>",
            crate::topic_list_view::icon("arrow-left", None),
            t("chat.heading")
        );
    }
    let hide_presence: bool = match guardian.user_id() {
        Some(id) => sqlx::query_scalar("SELECT hide_presence FROM user_options WHERE user_id = $1")
            .bind(id)
            .fetch_optional(&mut *conn)
            .await?
            .unwrap_or(false),
        None => false,
    };
    let content = content.replace(page::BACK_INDICATOR, &site.chrome.chat_back_indicator);
    let content = content.as_str();
    let page = ChatPage {
        site_title: site.site_title,
        lang: site.lang,
        base_path: site.base_path,
        crawler: Crawler::default(),
        viewer: site.viewer,
        bus_position: site.bus_position,
        chrome: site.chrome,
        title,
        main: Context::full_page(content, base_path, chat_view, hide_presence),
    };
    let response = Html(page.render().map_err(crate::html::HtmlError::from)?).into_response();
    Ok(crate::html::with_viewer_headers(response, &vs))
}

fn context<'a>(
    state: &'a AppState,
    conn: &'a mut sqlx::PgConnection,
    settings: &'a SiteSettings,
    guardian: &'a crate::guardian::Guardian,
) -> Context<'a> {
    Context {
        conn,
        settings,
        i18n: &state.i18n,
        guardian,
        base_path: state.config.globals.relative_url_root(),
        config: &state.config,
    }
}

/// `chat.title_capitalized`, the chat route's title token.
fn chat_title(state: &AppState) -> String {
    state
        .i18n
        .t("js.chat.title_capitalized")
        .unwrap_or("Chat")
        .to_string()
}

/// GET /chat and /chat/channels: ChatIndexRoute#redirect by
/// chat_preferred_index (the DMs page, or the browser when channels aren't
/// the index), then ChatChannelsRoute: in the mobile view the channels
/// list, on desktop the member's last channel, else the browser's open
/// tab.
pub async fn index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings = match gate(&state, &mut conn, &guardian, false).await? {
        Ok(settings) => settings,
        Err(response) => return Ok(response),
    };
    let base_path = state.config.globals.relative_url_root();
    let preferred = settings.get("chat_preferred_index")?.to_s().to_string();
    let public = settings.get("enable_public_channels")?.truthy();
    if !uri.path().ends_with("/chat/channels") {
        let mut cx = context(&state, &mut conn, &settings, &guardian);
        // hasThreads: the member has threads to list.
        if preferred == "my_threads"
            && settings.get("chat_threads_enabled")?.truthy()
            && cx.has_viewable_threads().await?
        {
            return Ok(redirect(format!("{base_path}/chat/threads")));
        }
        // chat.userCanAccessDirectMessages
        let dms =
            crate::plugins::chat::user_can_direct_message(&mut *cx.conn, &settings, &guardian)
                .await?
                || !cx.followed_dm_channels(false).await?.is_empty();
        let channels = preferred == "channels" && public;
        if dms && (preferred == "direct_messages" || !public) {
            return Ok(redirect(format!("{base_path}/chat/direct-messages")));
        }
        if !channels {
            return Ok(redirect(format!("{base_path}/chat/browse/open")));
        }
    } else if !public {
        return Err(crate::Unsupported("chat's channels list without public channels").into());
    }
    if crate::mobile::mobile_view() {
        // ChatIndexRoute replaces /chat with chat.channels.
        if !uri.path().ends_with("/chat/channels") {
            return Ok(redirect(format!("{base_path}/chat/channels")));
        }
        let content = context(&state, &mut conn, &settings, &guardian)
            .channels_route()
            .await?;
        let title = format!("{} - ", chat_title(&state));
        return render(
            &state,
            &headers,
            &mut conn,
            &settings,
            &guardian,
            Active::Chat,
            title,
            &content,
            true,
        )
        .await;
    }
    let last: Option<Option<String>> = sqlx::query_scalar(
        "SELECT value FROM user_custom_fields WHERE user_id = $1 AND name = $2 ORDER BY id LIMIT 1",
    )
    .bind(guardian.user_id())
    .bind(crate::plugins::chat::messages::LAST_CHAT_CHANNEL_ID)
    .fetch_optional(&mut *conn)
    .await?;
    if let Some(id) = last
        .flatten()
        .map(|v| crate::ruby::to_i(&v))
        .filter(|id| *id > 0)
    {
        let mut cx = context(&state, &mut conn, &settings, &guardian);
        // chatChannelsManager.find: a channel the member can't open
        // leaves them on /chat in Rails (the request fails); here, on the
        // browser.
        if let Found::Channel(channel) = cx.find_joinable(&id.to_string()).await? {
            if channel.chatable_type == "Category"
                && let Some(slug) = channel.slug.as_deref().filter(|s| !s.is_empty())
            {
                return Ok(redirect(format!(
                    "{base_path}/chat/c/{slug}/{}",
                    channel.id
                )));
            }
            // A DM's slug is its title for the viewer, which the channel
            // route redirects to.
            if channel.chatable_type == "DirectMessage" {
                return Ok(redirect(format!("{base_path}/chat/c/-/{}", channel.id)));
            }
        }
    }
    Ok(redirect(format!("{base_path}/chat/browse/open")))
}

/// GET /chat/browse: to the open tab (ChatBrowseIndexRoute), or the
/// homepage without public channels.
pub async fn browse_index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings = match gate(&state, &mut conn, &guardian, false).await? {
        Ok(settings) => settings,
        Err(response) => return Ok(response),
    };
    let base_path = state.config.globals.relative_url_root();
    if !settings.get("enable_public_channels")?.truthy() {
        return Ok(redirect(homepage(&settings, base_path)?));
    }
    Ok(redirect(format!("{base_path}/chat/browse/open")))
}

/// The tab a browse path names; archived only while archiving is allowed
/// (ChatBrowseArchivedRoute sends it back to the browser).
fn browse_tab(settings: &SiteSettings, tab: &str) -> Result<Option<&'static str>, AppError> {
    let archiving = settings.get("chat_allow_archiving_channels")?.truthy();
    Ok(BROWSE_TABS
        .iter()
        .copied()
        .find(|t| *t == tab)
        .filter(|t| *t != "archived" || archiving))
}

/// GET /chat/browse/:tab
pub async fn browse(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    Path(tab): Path<String>,
    uri: Uri,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings = match gate(&state, &mut conn, &guardian, false).await? {
        Ok(settings) => settings,
        Err(response) => return Ok(response),
    };
    let base_path = state.config.globals.relative_url_root();
    if !settings.get("enable_public_channels")?.truthy() {
        return Ok(redirect(homepage(&settings, base_path)?));
    }
    let Some(tab) = browse_tab(&settings, &tab)? else {
        return Ok(redirect(format!("{base_path}/chat/browse")));
    };
    let filter = filter_param(&uri);
    let content = context(&state, &mut conn, &settings, &guardian)
        .browse_page(tab, &filter, None)
        .await?;
    let title = format!("{} - ", chat_title(&state));
    render(
        &state,
        &headers,
        &mut conn,
        &settings,
        &guardian,
        Active::Chat,
        title,
        &content,
        false,
    )
    .await
}

fn filter_param(uri: &Uri) -> String {
    uri.query()
        .map(crate::params::parse_query)
        .and_then(|p| p.get("filter").and_then(crate::params::scalar))
        .unwrap_or_default()
}

/// GET /chat/browse/:tab/page: the next cards of a browse list.
pub async fn browse_more(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(tab): Path<String>,
    uri: Uri,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings = match gate(&state, &mut conn, &guardian, false).await? {
        Ok(settings) => settings,
        Err(response) => return Ok(response),
    };
    let Some(tab) = browse_tab(&settings, &tab)? else {
        return Ok(super::topics::not_found_response(&state));
    };
    let offset = uri
        .query()
        .map(crate::params::parse_query)
        .and_then(|p| p.get("offset").and_then(crate::params::scalar))
        .map_or(0, |o| crate::ruby::to_i(&o).max(0));
    let cards = context(&state, &mut conn, &settings, &guardian)
        .browse_cards(tab, &filter_param(&uri), offset)
        .await?;
    Ok(Html(cards).into_response())
}

/// GET /chat/c/:slug/:id and /chat/c/:slug/:id/:message_id: the channel
/// (withChatChannel: one the member can't open goes back to /chat, a
/// wrong slug to the channel's own).
/// The channel's slugifiedTitle (its route's slug) and, for a DM, its
/// title for the viewer: a DM has no slug, so its title is slugified (and
/// titles the page without the channel's "#").
async fn slugified_title(
    cx: &mut Context<'_>,
    channel: &crate::plugins::chat::channels::ChannelRow,
) -> Result<(String, Option<String>), AppError> {
    let dm_title = if channel.chatable_type == "DirectMessage" {
        let title = match &channel.name {
            Some(name) => name.clone(),
            None => {
                let dm = cx.direct_message(channel.chatable_id).await?;
                cx.dm_title(channel, &dm)?
            }
        };
        Some(title)
    } else {
        None
    };
    let own_slug = match &dm_title {
        Some(title) => super::user_menu::slugify_channel(
            channel.slug.as_deref(),
            &crate::topic_list_view::escape(title),
        ),
        None => match channel.slug.clone().filter(|s| !s.is_empty()) {
            Some(slug) => slug,
            // A channel without a slug: its title slugified
            // (CategoryChannel#title: its name, else its category's).
            None => {
                let title = match channel.name.clone().filter(|n| !n.is_empty()) {
                    Some(name) => name,
                    None => {
                        sqlx::query_scalar::<_, String>("SELECT name FROM categories WHERE id = $1")
                            .bind(channel.chatable_id)
                            .fetch_optional(&mut *cx.conn)
                            .await?
                            .ok_or(crate::Unsupported("chat channels of deleted categories"))?
                    }
                };
                super::user_menu::slugify_channel(None, &crate::topic_list_view::escape(&title))
            }
        },
    };
    Ok((own_slug, dm_title))
}

/// GET /chat/new-message: ChatNewMessageRoute's beforeModel on a page
/// load. With `channel_id` or `channel` (a slug: the member's channel,
/// else a public one) it goes to that channel, or to /chat when there is
/// none, a read the redirect does where the route's transition ends; with
/// neither nor `recipients` it goes to /chat, as a page load has no page
/// to open the new message modal over (static/js/chat-new-message.js
/// opens it for a link followed in the app). With `recipients` the page
/// is served with the route's empty outlet and nothing is written here:
/// the route's chat.upsertDmChannel is chat-new-message.js's, which finds
/// or creates the direct message with POST
/// /chat/api/direct-message-channels and goes to it, or leaves the error
/// over this page (popupAjaxError). The query is read as the router
/// reads it.
pub async fn new_message(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings = match gate(&state, &mut conn, &guardian, false).await? {
        Ok(settings) => settings,
        Err(response) => return Ok(response),
    };
    let base_path = state.config.globals.relative_url_root();
    let query: std::collections::HashMap<String, String> =
        url::form_urlencoded::parse(uri.query().unwrap_or("").as_bytes())
            .into_owned()
            .collect();
    let param = |name: &str| query.get(name).filter(|v| !v.is_empty()).cloned();
    // #seedDraft: the message goes into the channel's draft in the
    // browser, never saved.
    if param("message").is_some() {
        return Err(crate::Unsupported(
            "chat's /chat/new-message with a message (a draft seeded in the browser)",
        )
        .into());
    }
    let to_channel = |slug: String, id: i64| format!("{base_path}/chat/c/{slug}/{id}");
    if param("channel_id").is_some() || param("channel").is_some() {
        let mut cx = context(&state, &mut conn, &settings, &guardian);
        // #findChannel: by id, else by slug.
        let id = match param("channel_id") {
            Some(id) => Some(id),
            None => sqlx::query_scalar::<_, i64>(
                "SELECT id FROM chat_channels WHERE slug = $1 AND deleted_at IS NULL ORDER BY id LIMIT 1",
            )
            .bind(param("channel"))
            .fetch_optional(&mut *cx.conn)
            .await?
            .map(|id| id.to_string()),
        };
        let found = match id {
            Some(id) => cx.find_joinable(&id).await?,
            None => Found::NotFound,
        };
        return Ok(match found {
            Found::Channel(channel) => {
                let (slug, _) = slugified_title(&mut cx, &channel).await?;
                redirect(to_channel(slug, channel.id))
            }
            Found::NotFound | Found::Forbidden => redirect(format!("{base_path}/chat")),
        });
    }
    // `params.recipients?.split(",")`: an empty one is still recipients.
    if !query.contains_key("recipients") {
        return Ok(redirect(format!("{base_path}/chat")));
    }
    // The new-message route has no template or title token of its own.
    let title = format!("{} - ", chat_title(&state));
    render(
        &state,
        &headers,
        &mut conn,
        &settings,
        &guardian,
        Active::Chat,
        title,
        "",
        true,
    )
    .await
}

pub async fn channel(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    Path(params): Path<std::collections::HashMap<String, String>>,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings = match gate(&state, &mut conn, &guardian, false).await? {
        Ok(settings) => settings,
        Err(response) => return Ok(response),
    };
    let base_path = state.config.globals.relative_url_root();
    let get = |name: &str| params.get(name).cloned();
    let slug = get("slug").unwrap_or_default();
    let id = get("id").unwrap_or_default();
    let message_id = get("message_id");
    let mut cx = context(&state, &mut conn, &settings, &guardian);
    let channel = match cx.find_joinable(&id).await? {
        Found::Channel(channel) => channel,
        Found::NotFound | Found::Forbidden => return Ok(redirect(format!("{base_path}/chat"))),
    };
    let (own_slug, dm_title) = slugified_title(&mut cx, &channel).await?;
    let target = match &message_id {
        None => None,
        Some(m) => match m.parse::<i64>() {
            Ok(m) => Some(m),
            Err(_) => return Ok(super::topics::not_found_response(&state)),
        },
    };
    if slug != own_slug {
        let tail = target.map(|m| format!("/{m}")).unwrap_or_default();
        return Ok(redirect(format!(
            "{base_path}/chat/c/{own_slug}/{}{tail}",
            channel.id
        )));
    }
    let content = cx.channel_page(&channel, target, None).await?;
    let channel_title = channel.name.clone().filter(|n| !n.is_empty());
    let title = match (dm_title, channel_title) {
        (Some(dm), _) => format!(
            "{} - {} - ",
            settings.emojis().gsub_to_unicode(&dm),
            chat_title(&state)
        ),
        (None, Some(name)) => format!("#{name} - {} - ", chat_title(&state)),
        (None, None) => format!("{} - ", chat_title(&state)),
    };
    render(
        &state,
        &headers,
        &mut conn,
        &settings,
        &guardian,
        Active::ChatChannel(channel.id),
        title,
        &content,
        true,
    )
    .await
}

/// GET /chat/c/:slug/:id/t, /chat/c/:slug/:id/t/:thread_id and
/// .../t/:thread_id/:message_id: the channel route with the channel's
/// threads (ChatChannelThreads) or a thread (ChatChannelThread, around the
/// message) in its side panel. A channel without threading goes to the
/// channel, as does a thread that isn't the channel's or doesn't show.
pub async fn channel_thread(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    Path(params): Path<std::collections::HashMap<String, String>>,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings = match gate(&state, &mut conn, &guardian, false).await? {
        Ok(settings) => settings,
        Err(response) => return Ok(response),
    };
    let base_path = state.config.globals.relative_url_root();
    let get = |name: &str| params.get(name).cloned();
    let slug = get("slug").unwrap_or_default();
    let id = get("id").unwrap_or_default();
    let thread_id = get("thread_id");
    let message_id = get("message_id");
    let channel_message_id = get("channel_message_id");
    let mut cx = context(&state, &mut conn, &settings, &guardian);
    let channel = match cx.find_joinable(&id).await? {
        Found::Channel(channel) => channel,
        Found::NotFound | Found::Forbidden => return Ok(redirect(format!("{base_path}/chat"))),
    };
    let (own_slug, dm_title) = slugified_title(&mut cx, &channel).await?;
    let channel_url = format!("{base_path}/chat/c/{own_slug}/{}", channel.id);
    let parse = |v: &Option<String>| -> Result<Option<i64>, ()> {
        match v {
            None => Ok(None),
            Some(v) => v.parse::<i64>().map(Some).map_err(|_| ()),
        }
    };
    let (Ok(thread_id), Ok(target), Ok(channel_target)) = (
        parse(&thread_id),
        parse(&message_id),
        parse(&channel_message_id),
    ) else {
        return Ok(super::topics::not_found_response(&state));
    };
    if slug != own_slug {
        let tail = match (thread_id, target, channel_target) {
            (Some(t), _, Some(c)) => format!("/{c}/t/{t}"),
            (Some(t), Some(m), None) => format!("/t/{t}/{m}"),
            (Some(t), None, None) => format!("/t/{t}"),
            _ => "/t".to_string(),
        };
        return Ok(redirect(format!("{channel_url}{tail}")));
    }
    // ChatChannelThreads: members only; a channel without threading goes
    // back to the channel.
    // ChatChannelNearMessageWithThread: the channel around its message,
    // and the thread beside it on desktop only.
    let mobile = crate::mobile::mobile_view();
    let panel = match thread_id {
        Some(_) if channel_target.is_some() && mobile => String::new(),
        None => {
            if guardian.is_anonymous() {
                return Ok(redirect(format!("{base_path}/chat/channels")));
            }
            if !channel.threading_enabled {
                return Ok(redirect(channel_url));
            }
            cx.channel_threads_page(&channel).await?
        }
        Some(thread_id) => match cx.shown_thread(&channel, thread_id).await? {
            Some(thread) => cx.thread_page(&channel, &thread, target).await?,
            None => return Ok(redirect(channel_url)),
        },
    };
    let mut content = cx
        .channel_page_beside(&channel, channel_target, None, !panel.is_empty())
        .await?;
    // ChatSidePanel, with its resizer on desktop.
    if !panel.is_empty() {
        content.push_str(&format!(
            "<div class=\"chat-side-panel\">{panel}{}</div>",
            if mobile {
                ""
            } else {
                "<div class=\"chat-side-panel-resizer\"></div>"
            }
        ));
    }
    let channel_title = channel.name.clone().filter(|n| !n.is_empty());
    let title = match (dm_title, channel_title) {
        (Some(dm), _) => format!(
            "{} - {} - ",
            settings.emojis().gsub_to_unicode(&dm),
            chat_title(&state)
        ),
        (None, Some(name)) => format!("#{name} - {} - ", chat_title(&state)),
        (None, None) => format!("{} - ", chat_title(&state)),
    };
    render(
        &state,
        &headers,
        &mut conn,
        &settings,
        &guardian,
        Active::ChatChannel(channel.id),
        title,
        &content,
        true,
    )
    .await
}

/// GET /chat/c/:slug/:id/:message_id/t/:thread_id
/// (ChatChannelNearMessageWithThread): channel_thread with the channel's
/// target message.
pub async fn channel_message_thread(
    state: State<AppState>,
    guardian: AuthGuardian,
    headers: HeaderMap,
    Path(mut params): Path<std::collections::HashMap<String, String>>,
) -> Result<Response, AppError> {
    if let Some(message_id) = params.remove("message_id") {
        params.insert("channel_message_id".into(), message_id);
    }
    channel_thread(state, guardian, headers, Path(params)).await
}

/// GET /chat/direct-messages: ChatDirectMessagesRoute. Its beforeModel
/// sends a desktop member with DMs on to their last channel, but on a page
/// load the reference stays on the list: directMessageChannels is still
/// empty when it runs.
pub async fn direct_messages(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings = match gate(&state, &mut conn, &guardian, false).await? {
        Ok(settings) => settings,
        Err(response) => return Ok(response),
    };
    let content = context(&state, &mut conn, &settings, &guardian)
        .direct_messages_route(None)
        .await?;
    // The route adds no title token of its own.
    let title = format!("{} - ", chat_title(&state));
    render(
        &state,
        &headers,
        &mut conn,
        &settings,
        &guardian,
        Active::Chat,
        title,
        &content,
        true,
    )
    .await
}

/// GET /chat/starred-channels: ChatStarredChannelsRoute. The desktop view
/// goes to /chat (no channel is active on a page load); the mobile view
/// draws the member's starred channels, or goes to the channels without
/// any.
pub async fn starred_channels(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings = match gate(&state, &mut conn, &guardian, false).await? {
        Ok(settings) => settings,
        Err(response) => return Ok(response),
    };
    let base_path = state.config.globals.relative_url_root();
    if !crate::mobile::mobile_view() {
        return Ok(redirect(format!("{base_path}/chat")));
    }
    let Some(content) = context(&state, &mut conn, &settings, &guardian)
        .starred_channels_route()
        .await?
    else {
        return Ok(redirect(format!("{base_path}/chat/channels")));
    };
    let title = format!("{} - ", chat_title(&state));
    render(
        &state,
        &headers,
        &mut conn,
        &settings,
        &guardian,
        Active::Chat,
        title,
        &content,
        true,
    )
    .await
}

/// GET /chat/threads: the member's threads (ChatChannelThreads route,
/// UserThreads).
pub async fn threads(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings = match gate(&state, &mut conn, &guardian, false).await? {
        Ok(settings) => settings,
        Err(response) => return Ok(response),
    };
    let content = context(&state, &mut conn, &settings, &guardian)
        .threads_route()
        .await?;
    let title = format!("{} - ", chat_title(&state));
    render(
        &state,
        &headers,
        &mut conn,
        &settings,
        &guardian,
        Active::Chat,
        title,
        &content,
        true,
    )
    .await
}

/// GET /chat/disabled
pub async fn disabled(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings = match gate(&state, &mut conn, &guardian, true).await? {
        Ok(settings) => settings,
        Err(response) => return Ok(response),
    };
    let content = page::disabled_page(&state.i18n, state.config.globals.relative_url_root());
    let title = format!("{} - ", chat_title(&state));
    render(
        &state,
        &headers,
        &mut conn,
        &settings,
        &guardian,
        Active::Chat,
        title,
        &content,
        true,
    )
    .await
}

#[derive(serde::Deserialize, Default)]
pub struct DrawerParams {
    /// The chat URL the drawer opens, without the base path.
    url: Option<String>,
    /// "false" while the drawer is collapsed.
    expanded: Option<String>,
    /// ChatHistory's previous route name.
    previous: Option<String>,
}

/// GET /live/chat/drawer: the drawer's route for a chat URL, as
/// static/js/chat-drawer.js shows it (crate::plugins::chat::drawer): its
/// markup, with the routes it visited (x-chat-drawer-routes) and the
/// channel it shows (x-chat-drawer-channel). Not found where the drawer
/// doesn't handle the URL, for a viewer who can't chat, and in the mobile
/// view, which has no drawer.
pub async fn drawer(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    axum::extract::Query(params): axum::extract::Query<DrawerParams>,
) -> Result<Response, AppError> {
    if crate::mobile::mobile_view() {
        return Ok(StatusCode::NOT_FOUND.into_response());
    }
    let mut conn = state.pool.acquire().await?;
    let settings = match gate(&state, &mut conn, &guardian, false).await? {
        Ok(settings) => settings,
        Err(_) => return Ok(StatusCode::NOT_FOUND.into_response()),
    };
    let url = params.url.unwrap_or_default();
    let path = url.split(['?', '#']).next().unwrap_or_default();
    let expanded = params.expanded.as_deref() != Some("false");
    let drawn = crate::plugins::chat::drawer::draw(
        &mut context(&state, &mut conn, &settings, &guardian),
        path,
        expanded,
        params.previous.as_deref(),
    )
    .await?;
    let Some(drawn) = drawn else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };
    let mut response = (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache, no-store"),
        ],
        drawn.html,
    )
        .into_response();
    let headers = response.headers_mut();
    if let Ok(v) = drawn.routes.join(" ").parse() {
        headers.insert("x-chat-drawer-routes", v);
    }
    if let Some(id) = drawn.channel_id {
        headers.insert("x-chat-drawer-channel", id.into());
    }
    Ok(response)
}

#[derive(serde::Deserialize, Default)]
pub struct LiveMessageParams {
    previous: Option<String>,
    /// `thread`: as a thread's pane draws it.
    context: Option<String>,
}

/// GET /live/chat/:channel_id/:message_id: a message as the viewer's
/// channel page draws it, after `previous` (a message id, or `staged`: the
/// viewer's own not yet sent), for chat.js to swap in when the channel's
/// bus tells of it.
pub async fn live_message(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path((channel_id, message_id)): Path<(String, String)>,
    axum::extract::Query(params): axum::extract::Query<LiveMessageParams>,
) -> Result<Response, AppError> {
    let (Ok(channel_id), Ok(message_id)) = (channel_id.parse::<i64>(), message_id.parse::<i64>())
    else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };
    let mut conn = state.pool.acquire().await?;
    let settings = match gate(&state, &mut conn, &guardian, false).await? {
        Ok(settings) => settings,
        Err(_) => return Ok(StatusCode::NOT_FOUND.into_response()),
    };
    let staged = params.previous.as_deref() == Some("staged");
    let previous = params.previous.and_then(|p| p.parse::<i64>().ok());
    let html = context(&state, &mut conn, &settings, &guardian)
        .message_fragment_in(
            channel_id,
            message_id,
            previous,
            staged,
            params.context.as_deref() == Some("thread"),
        )
        .await?;
    Ok(match html {
        Some(html) => (
            [
                (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                (header::CACHE_CONTROL, "no-cache, no-store"),
            ],
            html,
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    })
}

#[derive(serde::Deserialize, Default)]
pub struct LiveMessagesParams {
    target_message_id: Option<String>,
    direction: Option<String>,
    /// "true": the viewer's page has no "last visit" line.
    newest: Option<String>,
}

/// GET /live/chat/:channel_id/messages: a page of the channel's messages
/// as the viewer's channel page draws them (Context::messages_page), for
/// chat.js to page through, jump to a message or go back to the latest
/// (ChatMessagesLoader over GET /chat/api/channels/:id/messages), with
/// whether more load either way (x-chat-can-load-more-past and -future:
/// true, false, or null when the page went the other way).
/// Not found where the API is (a channel the viewer can't see, a target
/// that's gone).
pub async fn live_messages(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(channel_id): Path<String>,
    axum::extract::Query(params): axum::extract::Query<LiveMessagesParams>,
) -> Result<Response, AppError> {
    let Ok(channel_id) = channel_id.parse::<i64>() else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };
    let target = match params.target_message_id.as_deref() {
        None | Some("") => None,
        Some(id) => match id.parse::<i64>() {
            Ok(id) => Some(id),
            Err(_) => return Ok(StatusCode::NOT_FOUND.into_response()),
        },
    };
    let mut conn = state.pool.acquire().await?;
    let settings = match gate(&state, &mut conn, &guardian, false).await? {
        Ok(settings) => settings,
        Err(_) => return Ok(StatusCode::NOT_FOUND.into_response()),
    };
    let page = context(&state, &mut conn, &settings, &guardian)
        .messages_page(
            channel_id,
            target,
            params.direction.as_deref(),
            params.newest.as_deref() == Some("true"),
        )
        .await?;
    let Some(page) = page else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };
    Ok((
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache, no-store"),
        ],
        [
            (
                "x-chat-can-load-more-past",
                page.can_load_more_past.to_string(),
            ),
            (
                "x-chat-can-load-more-future",
                page.can_load_more_future.to_string(),
            ),
        ],
        page.html,
    )
        .into_response())
}

/// Journey::Router::Utils.escape_path: a path keeps the unreserved
/// characters, the sub-delimiters, ":", "@" and "/" unescaped.
const PATH: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~')
    .remove(b'!')
    .remove(b'$')
    .remove(b'&')
    .remove(b'\'')
    .remove(b'(')
    .remove(b')')
    .remove(b'*')
    .remove(b'+')
    .remove(b',')
    .remove(b';')
    .remove(b'=')
    .remove(b':')
    .remove(b'@')
    .remove(b'/');

/// GET /chat/channel/:channel_id, /chat/channel/:channel_id/:channel_title
/// and its info routes: the `/channel -> /c` redirects (a 301; the title
/// "-" without one), the channel's own route then correcting the slug.
/// Routed whether or not chat is on, as the engine's redirects are.
pub async fn channel_legacy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(params): Path<std::collections::HashMap<String, String>>,
) -> Response {
    let escape = |v: &str| percent_encoding::utf8_percent_encode(v, PATH).to_string();
    let id = escape(params.get("channel_id").map_or("", String::as_str));
    let title = escape(params.get("channel_title").map_or("-", String::as_str));
    let tail = match params.get("route") {
        Some(route) => format!("/{route}"),
        None => String::new(),
    };
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost");
    (
        StatusCode::MOVED_PERMANENTLY,
        [(
            header::LOCATION,
            format!(
                "http://{host}{}/chat/c/{title}/{id}{tail}",
                state.config.globals.relative_url_root()
            ),
        )],
    )
        .into_response()
}

/// The info routes /chat/channel/:id/:title redirects with their own.
pub async fn channel_legacy_info(
    state: State<AppState>,
    headers: HeaderMap,
    Path(mut params): Path<std::collections::HashMap<String, String>>,
    uri: Uri,
) -> Response {
    // The route after the title: info, info/about, info/members, ...
    let route = ["about", "members", "settings", "search"]
        .iter()
        .find(|r| uri.path().ends_with(&format!("/info/{r}")))
        .map_or_else(|| "info".to_string(), |r| format!("info/{r}"));
    params.insert("route".into(), route);
    channel_legacy(state, headers, Path(params)).await
}
