//! StaticController#show for the guidelines (FAQ), terms of service and
//! privacy pages: GET /faq, /guidelines (and its /rules and /conduct
//! aliases), /tos and /privacy, each also as `.html` (what Ember's
//! StaticPage.find asks for) and `.json`.
//!
//! The order of static#show: the login wall for the guidelines pages,
//! rename_faq_to_guidelines sending /rules, /conduct and (without a
//! faq_url) /faq to /guidelines, the faq_url / tos_url /
//! privacy_policy_url settings sending their page elsewhere (/guidelines
//! and its aliases always show the topic), then the topic the
//! guidelines_topic_id / tos_topic_id / privacy_topic_id setting names,
//! not found without one. An XHR gets show.html.erb alone; a page load
//! gets the Ember route's page around it (templates/faq.gjs, tos.gjs).
//!
//! Refused: plugins' add_topic_static_page pages (no plugin API), the
//! first post's translation under content localization, and a full page
//! load of `.html` / `.json` (Ember has no route for those paths).

use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};

use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::static_page_view::{Labels, StaticPage, fragment};
use crate::url::Urls;
use crate::{AppError, AppState, Unsupported};

/// The routes static#show serves here: (path, `params[:id]`).
pub const PAGES: [(&str, &str); 6] = [
    ("faq", "faq"),
    ("guidelines", "guidelines"),
    ("rules", "guidelines"),
    ("conduct", "guidelines"),
    ("tos", "tos"),
    ("privacy", "privacy"),
];

/// DEFAULT_PAGES: the setting that sends the page elsewhere and the one
/// naming its topic.
fn settings_for(page: &str) -> Option<(&'static str, &'static str)> {
    match page {
        "faq" => Some(("faq_url", "guidelines_topic_id")),
        "tos" => Some(("tos_url", "tos_topic_id")),
        "privacy" => Some(("privacy_policy_url", "privacy_topic_id")),
        _ => None,
    }
}

fn is_xhr(headers: &HeaderMap) -> bool {
    headers
        .get("x-requested-with")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("XMLHttpRequest"))
}

/// `redirect_to`: a path on this host made absolute; a URL with a scheme
/// (or protocol relative) as it is.
fn redirect(urls: &Urls, location: &str) -> Result<Response, AppError> {
    let has_scheme = location.starts_with("//")
        || location.split_once(':').is_some_and(|(scheme, _)| {
            scheme.starts_with(|c: char| c.is_ascii_alphabetic())
                && scheme
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        });
    let location = if has_scheme {
        location.to_string()
    } else {
        format!("{}{location}", urls.base_url_no_prefix()?)
    };
    Ok((
        StatusCode::FOUND,
        [(header::LOCATION, HeaderValue::from_str(&location)?)],
    )
        .into_response())
}

/// The page as Ember draws it.
#[derive(askama::Template)]
#[template(path = "static_page.html")]
pub struct StaticPageHtml {
    pub site_title: String,
    pub lang: String,
    pub base_path: String,
    pub crawler: crate::html::Crawler,
    pub viewer: Option<crate::html::Viewer>,
    pub bus_position: String,
    pub chrome: crate::html::Chrome,
    /// The route's titleToken, `i18n(pageId)`.
    pub page_title: String,
    pub main: String,
    /// WatchRead: a member who has not read the guidelines yet.
    pub watch_read: bool,
}

/// GET /faq, /guidelines, /rules, /conduct, /tos, /privacy (and `.html`,
/// `.json`)
pub async fn show(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let base_path = state.config.globals.relative_url_root();
    let request_path = uri.path();
    let local = request_path
        .strip_prefix(base_path)
        .unwrap_or(request_path)
        .trim_start_matches('/');
    let (route, extension) = match local.rsplit_once('.') {
        Some((route, ext)) => (route, Some(ext)),
        None => (local, None),
    };
    let Some(&(route, id)) = PAGES.iter().find(|(path, _)| *path == route) else {
        return Ok(super::topics::not_found_response(&state));
    };
    let xhr = is_xhr(&headers);

    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };

    let login_wall = settings.get("login_required")?.truthy() && guardian.is_anonymous();
    if login_wall && matches!(id, "faq" | "guidelines") {
        return redirect(&urls, &format!("{base_path}/login"));
    }

    let faq_url = settings.get("faq_url")?.to_s().to_string();
    let rename = guardian
        .upcoming_change_enabled(&mut conn, &settings, "rename_faq_to_guidelines")
        .await?;
    // `request.path`, the base path included.
    if rename
        && (matches!(request_path, "/rules" | "/conduct")
            || (request_path == "/faq" && faq_url.is_empty()))
    {
        return redirect(&urls, &format!("{base_path}/guidelines"));
    }

    if let Some((url_setting, _)) = settings_for(id)
        && let Some(url) = settings.get(url_setting)?.presence()
    {
        return redirect(&urls, &url);
    }

    // The /guidelines route always shows the FAQ topic, faq_url or not.
    let page = if id == "guidelines" { "faq" } else { id };
    let Some((_, topic_setting)) = settings_for(page) else {
        return Ok(super::topics::not_found_response(&state));
    };
    let topic_id = i32::try_from(settings.get(topic_setting)?.to_i()).unwrap_or(0);
    let topic: Option<(i32, Option<String>)> =
        sqlx::query_as("SELECT id, slug FROM topics WHERE id = $1 AND deleted_at IS NULL")
            .bind(topic_id)
            .fetch_optional(&mut *conn)
            .await?;
    let Some((topic_id, slug)) = topic else {
        return Ok(super::topics::not_found_response(&state));
    };
    if settings.get("content_localization_enabled")?.truthy() {
        return Err(Unsupported("a static page's translated post (ContentLocalization)").into());
    }
    // `@topic.posts.first` for the body, `@topic.first_post` for the date.
    let body: Option<String> = sqlx::query_scalar(
        "SELECT cooked FROM posts WHERE topic_id = $1 AND deleted_at IS NULL ORDER BY id LIMIT 1",
    )
    .bind(topic_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(body) = body else {
        return Err(Unsupported("a static page whose topic has no posts").into());
    };
    let reviewed: Option<chrono::NaiveDateTime> = sqlx::query_scalar(
        "SELECT updated_at FROM posts WHERE topic_id = $1 AND post_number = 1 AND deleted_at IS NULL",
    )
    .bind(topic_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(reviewed) = reviewed else {
        return Err(Unsupported("a static page whose topic has no first post").into());
    };

    let edit_url = if guardian.is_staff() {
        // Topic#url
        let Some(slug) = slug.filter(|s| !s.is_empty()) else {
            return Err(Unsupported("a static page topic without a stored slug").into());
        };
        Some(format!(
            "{}{base_path}/t/{slug}/{topic_id}",
            urls.base_url_no_prefix()?
        ))
    } else {
        None
    };
    let tos_url = super::invite_redeem::legal_url(
        &mut conn,
        &settings,
        base_path,
        "tos_url",
        "tos_topic_id",
        "/tos",
    )
    .await?;
    let privacy_policy_url = super::invite_redeem::legal_url(
        &mut conn,
        &settings,
        base_path,
        "privacy_policy_url",
        "privacy_topic_id",
        "/privacy",
    )
    .await?;
    let js = |key: &str| {
        state
            .i18n
            .t(&format!("js.{key}"))
            .unwrap_or(key)
            .to_string()
    };
    let edit_this_page = state
        .i18n
        .t("edit_this_page")
        .unwrap_or("edit_this_page")
        .to_string();
    let labels = (
        js("about.simple_title"),
        js("guidelines"),
        js("faq"),
        js("tos"),
        js("privacy"),
    );
    let last_reviewed = reviewed.date().format("%Y-%m-%d").to_string();
    let html = fragment(&StaticPage {
        base_path,
        page,
        last_reviewed: &last_reviewed,
        login_wall,
        faq_url: &faq_url,
        rename_faq_to_guidelines: rename,
        tos_url: tos_url.as_deref(),
        privacy_policy_url: privacy_policy_url.as_deref(),
        edit_url: edit_url.as_deref(),
        body: &body,
        labels: Labels {
            about: &labels.0,
            guidelines: &labels.1,
            faq: &labels.2,
            tos: &labels.3,
            privacy: &labels.4,
            edit_this_page: &edit_this_page,
        },
    });
    if xhr {
        return Ok((
            [(
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/html; charset=utf-8"),
            )],
            html,
        )
            .into_response());
    }
    if extension.is_some() {
        return Err(Unsupported("a static page's .html or .json path as a page").into());
    }

    // `@title`: the page's name, then the site's.
    let page_name = match page {
        "faq" if rename => "guidelines",
        other => other,
    };
    let site_title = settings.get("title")?.to_s().to_string();
    let title = format!("{} - {site_title}", js(page_name));

    let bus_position = crate::bus::page_position(&state.bus).await?;
    let vs = super::session::viewer_state(&state, &headers, &settings, &guardian)?;
    let mut site = crate::html::Site::from_settings(&settings, base_path)?;
    site.viewer = vs.viewer.clone();
    site.load_chrome(
        &mut conn,
        &state,
        &settings,
        &guardian,
        crate::sidebar::Active::Static(route.to_string()),
    )
    .await?;
    site.bus_position = bus_position;
    // bodyClass (concat "static-" model.path): the route's pageId.
    site.chrome
        .body_classes
        .push_str(&format!(" static-{route}"));
    let faq_template = !matches!(route, "tos" | "privacy");
    let watch_read = match guardian.user_id() {
        Some(user_id) if faq_template => {
            let read: Option<bool> = sqlx::query_scalar(
                "SELECT read_faq IS NOT NULL FROM user_stats WHERE user_id = $1",
            )
            .bind(user_id)
            .fetch_optional(&mut *conn)
            .await?;
            !read.unwrap_or(false)
        }
        _ => false,
    };
    let description = settings.get("site_description")?.to_s().to_string();
    let image = crate::html::site_opengraph_image(&mut conn, &urls).await?;
    let mut crawler = crate::html::Crawler::for_request(&urls, &uri, None)?.with_meta(
        urls.settings.emojis(),
        &title,
        &description,
        image,
    );
    crawler.description = description;
    // templates/faq.gjs and tos.gjs: the above-static and below-static
    // outlets draw nothing.
    let main = format!(
        "<section class=\"container\"><div class=\"contents clearfix body-page\">{html}</div></section>"
    );
    let page = StaticPageHtml {
        site_title: site.site_title,
        lang: site.lang,
        base_path: site.base_path,
        crawler,
        viewer: site.viewer,
        bus_position: site.bus_position,
        chrome: site.chrome,
        page_title: js(route),
        main,
        watch_read,
    };
    use askama::Template;
    let body = page.render().map_err(crate::html::HtmlError::from)?;
    Ok(crate::html::crawler_response(
        body,
        &page.crawler,
        &settings,
        &vs,
    )?)
}

/// A path static#show serves here (the base path left off), which the
/// controller's skip_before_action lets past the login and profile gates.
pub fn is_page(path: &str) -> bool {
    let local = path.trim_start_matches('/');
    let route = local
        .strip_suffix(".html")
        .or_else(|| local.strip_suffix(".json"))
        .unwrap_or(local);
    PAGES.iter().any(|(page, _)| *page == route)
}
