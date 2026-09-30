//! Port of app/controllers/site_controller.rb.

use axum::Json;
use axum::extract::State;
use serde::Serialize;

use crate::guardian::Guardian;
use crate::site::Site;
use crate::site_settings::{SiteSettings, Value};
use crate::url::Urls;
use crate::{AppError, AppState, color_scheme, site_icons};

/// Field order matches the Hash the controller builds; mobile_logo_url is
/// only present when set.
#[derive(Serialize)]
pub struct BasicInfo {
    logo_url: String,
    logo_small_url: String,
    apple_touch_icon_url: String,
    favicon_url: String,
    title: Value,
    description: Value,
    header_primary_color: String,
    header_background_color: String,
    login_required: Value,
    locale: Value,
    include_in_discourse_discover: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    mobile_logo_url: Option<String>,
}

/// GET /site/basic-info. Public even on login_required sites ("this info is
/// always available cause it can be scraped from a 404 page").
pub async fn basic_info(State(state): State<AppState>) -> Result<Json<BasicInfo>, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };

    // `UrlHelper.absolute` over site_*_url, which are already absolute
    // (or ""), so it is a no-op kept for parity with the controller.
    let mut url = async |name: &str| -> Result<String, AppError> {
        let raw = site_icons::site_url(&mut conn, &urls, name).await?;
        Ok(urls.absolute(&raw)?)
    };
    let logo_url = url("logo").await?;
    let logo_small_url = url("logo_small").await?;
    let apple_touch_icon_url = url("apple_touch_icon").await?;
    let favicon_url = url("favicon").await?;
    let mobile_logo_url = url("mobile_logo").await?;

    let header_primary_color = color_scheme::hex_for_name(&mut conn, &settings, "header_primary")
        .await?
        .unwrap_or_else(|| "333333".into());
    let header_background_color =
        color_scheme::hex_for_name(&mut conn, &settings, "header_background")
            .await?
            .unwrap_or_else(|| "ffffff".into());

    Ok(Json(BasicInfo {
        logo_url,
        logo_small_url,
        apple_touch_icon_url,
        favicon_url,
        title: settings.get("title")?.clone(),
        description: settings.get("site_description")?.clone(),
        header_primary_color,
        header_background_color,
        login_required: settings.get("login_required")?.clone(),
        locale: settings.get("default_locale")?.clone(),
        include_in_discourse_discover: settings.get("include_in_discourse_discover")?.clone(),
        mobile_logo_url: (!mobile_logo_url.is_empty()).then_some(mobile_logo_url),
    }))
}

/// GET /site.json: `Site.json_for(guardian)`, anonymous only until sessions
/// are ported.
pub async fn site(State(state): State<AppState>) -> Result<Json<serde_json::Value>, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let mut site = Site {
        conn: &mut conn,
        config: &state.config,
        settings: &settings,
        defs: &state.site_setting_defs,
        i18n: &state.i18n,
        guardian: Guardian::anonymous(),
    };
    Ok(Json(site.json_for().await?))
}
