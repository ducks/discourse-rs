//! GET /about.json (about#index): the about page's document.

use axum::Json;
use axum::extract::State;
use serde_json::Value;

use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::url::Urls;
use crate::{AppError, AppState};

/// GET /about.json
pub async fn show(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
) -> Result<Json<Value>, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    Ok(Json(
        crate::about::document(&mut conn, &settings, &urls, &guardian).await?,
    ))
}

/// GET /about/live_post_counts (about#live_post_counts): the public
/// topics, and their replies, outside category definitions; members once
/// every ten minutes, staff as often as they like.
pub async fn live_post_counts(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
) -> Result<axum::response::Response, AppError> {
    use axum::response::IntoResponse;
    let Some(user_id) = guardian.user_id() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    let limiter = crate::rate_limiter::RateLimiter::new(
        Some(crate::rate_limiter::LimitUser {
            id: user_id,
            staff: guardian.is_staff(),
        }),
        "live_post_counts",
        1,
        600,
    );
    if let Err(refused) = limiter
        .performed(&state.pool, state.config.rate_limits)
        .await?
    {
        return Ok(refused.into_response(&state.i18n));
    }
    // Topic.listable_topics.visible.secured(Guardian.new(nil))
    let (topics, posts): (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(SUM(t.posts_count), 0)::bigint FROM topics t \
         WHERE t.deleted_at IS NULL AND t.archetype <> 'private_message' AND t.visible \
           AND (t.category_id IS NULL OR t.category_id IN (SELECT id FROM categories WHERE NOT read_restricted)) \
           AND t.id NOT IN (SELECT topic_id FROM categories WHERE topic_id IS NOT NULL)",
    )
    .fetch_one(&state.pool)
    .await?;
    Ok(Json(serde_json::json!({
        "public_topic_count": topics,
        "public_post_count": posts - topics,
    }))
    .into_response())
}

/// The HTML of /about (layout plus the about route's content).
#[derive(askama::Template)]
#[template(path = "about.html")]
pub struct AboutHtml {
    pub site_title: String,
    pub lang: String,
    pub base_path: String,
    pub crawler: crate::html::Crawler,
    pub viewer: Option<crate::html::Viewer>,
    pub bus_position: String,
    pub chrome: crate::html::Chrome,
    /// `About - <site>`, the route's titleToken before the site's title.
    pub page_title: String,
    pub main: String,
}

/// GET /about (about#index as HTML): Ember's about page from the same
/// document /about.json serves, with crawlable_meta_data for its title
/// and the site description.
pub async fn page(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: axum::http::HeaderMap,
    uri: axum::http::Uri,
) -> Result<axum::response::Response, AppError> {
    let bus_position = crate::bus::page_position(&state.bus).await?;
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let doc = crate::about::document(&mut conn, &settings, &urls, &guardian).await?;
    let base_path = state.config.globals.relative_url_root();
    let vs = super::session::viewer_state(&state, &headers, &settings, &guardian)?;
    let mut site = crate::html::Site::from_settings(&settings, base_path)?;
    site.viewer = vs.viewer.clone();
    site.load_chrome(
        &mut conn,
        &state,
        &settings,
        &guardian,
        crate::sidebar::Active::About,
    )
    .await?;
    site.bus_position = bus_position;
    site.chrome.body_classes.push_str(" about-page");
    let categories = crate::topic_list_view::categories(&mut conn).await?;
    let cx = crate::topic_list_view::ListContext {
        i18n: &state.i18n,
        base_path,
        now: crate::clock::now(),
        categories: &categories,
        expand_all_pinned: false,
        hide_category: false,
        member_trust_level: site.viewer.as_ref().map(|v| v.trust_level),
        settings: crate::topic_list_view::ListSettings::load(&settings)?,
    };
    let faq_url = settings.get("faq_url")?.to_s().to_string();
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
    let main = crate::about_view::render(
        &cx,
        &crate::about_view::AboutPage {
            doc: &doc,
            viewer_admin: guardian.is_admin(),
            faq_url: &faq_url,
            rename_faq_to_guidelines: settings.get("rename_faq_to_guidelines")?.truthy(),
            tos_url: tos_url.as_deref(),
            privacy_policy_url: privacy_policy_url.as_deref(),
            chat_enabled: crate::plugins::chat::enabled(&settings)?,
        },
    )?;
    // @title and @description_meta
    let simple_title = crate::topic_list_view::t(&cx, "about.simple_title");
    let title = format!(
        "{simple_title} - {}",
        doc["about"]["title"].as_str().unwrap_or("")
    );
    let description = doc["about"]["description"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let image = crate::html::site_opengraph_image(&mut conn, &urls).await?;
    let mut crawler = crate::html::Crawler::for_request(&urls, &uri, None)?.with_meta(
        urls.settings.emojis(),
        &title,
        &description,
        image,
    );
    crawler.description = description;
    let page = AboutHtml {
        site_title: site.site_title,
        lang: site.lang,
        base_path: site.base_path,
        crawler,
        viewer: site.viewer,
        bus_position: site.bus_position,
        chrome: site.chrome,
        page_title: simple_title,
        main,
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
