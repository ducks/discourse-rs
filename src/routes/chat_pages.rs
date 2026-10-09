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
    site.chrome.powered_by = false;
    let page = ChatPage {
        site_title: site.site_title,
        lang: site.lang,
        base_path: site.base_path,
        crawler: Crawler::default(),
        viewer: site.viewer,
        bus_position: site.bus_position,
        chrome: site.chrome,
        title,
        main: Context::full_page(content, base_path, chat_view),
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

/// GET /chat and /chat/channels: ChatIndexRoute#redirect with the
/// default preferred index (channels), then ChatChannelsRoute on desktop:
/// the member's last channel, else the browser's open tab.
pub async fn index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings = match gate(&state, &mut conn, &guardian, false).await? {
        Ok(settings) => settings,
        Err(response) => return Ok(response),
    };
    let base_path = state.config.globals.relative_url_root();
    let preferred = settings.get("chat_preferred_index")?.to_s().to_string();
    let public = settings.get("enable_public_channels")?.truthy();
    if preferred != "channels" || !public {
        return Err(crate::Unsupported("chat's direct messages and threads indexes").into());
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
        if let Found::Channel(channel) = cx.find_joinable(&id.to_string()).await?
            && channel.chatable_type == "Category"
            && let Some(slug) = channel.slug.as_deref().filter(|s| !s.is_empty())
        {
            return Ok(redirect(format!(
                "{base_path}/chat/c/{slug}/{}",
                channel.id
            )));
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
        .browse_page(tab, &filter)
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
    let Some(own_slug) = channel.slug.clone().filter(|s| !s.is_empty()) else {
        return Err(crate::Unsupported("chat channels without a slug").into());
    };
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
    let content = cx.channel_page(&channel, target).await?;
    let channel_title = channel.name.clone().filter(|n| !n.is_empty());
    let title = match channel_title {
        Some(name) => format!("#{name} - {} - ", chat_title(&state)),
        None => format!("{} - ", chat_title(&state)),
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
