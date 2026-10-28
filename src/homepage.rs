//! `current_homepage` (ApplicationController, lib/homepage_helper.rb): the
//! filter the front page shows for this request. Ember reads it from the
//! `discourse_current_homepage` meta tag as `defaultHomepage()`.

use sqlx::PgConnection;

use crate::AppError;
use crate::guardian::Guardian;
use crate::plugin_api;
use crate::site_settings::SiteSettings;

/// `UserOption::HOMEPAGES` (-1 is the custom homepage, resolved below).
const HOMEPAGES: &[(i32, &str)] = &[
    (1, "latest"),
    (2, "categories"),
    (3, "unread"),
    (4, "new"),
    (5, "top"),
    (6, "bookmarks"),
    (7, "unseen"),
];

/// `Discourse.anonymous_filters` with what discourse-topic-voting pushes
/// onto it at load, enabled or not.
const ANONYMOUS_MENU_ITEMS: &[&str] = &["latest", "top", "categories", "hot", "votes"];

/// `current_user&.user_option&.homepage || HomepageHelper.resolve`.
///
/// The theme's `custom_homepage` modifier is read for the default theme and
/// its components, the theme the port draws every page with. The
/// homepage options are the installed plugins' (plugin_api::Homepage); the
/// `custom_homepage_enabled` modifier has nothing registered: no bundled
/// plugin adds it.
pub async fn current(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
) -> Result<String, AppError> {
    let user_id = guardian.user_id();
    if let Some(user_id) = user_id {
        let homepage_id: Option<i32> =
            sqlx::query_scalar("SELECT homepage_id FROM user_options WHERE user_id = $1")
                .bind(user_id)
                .fetch_optional(&mut *conn)
                .await?
                .flatten();
        if let Some(id) = homepage_id {
            // UserOption#homepage
            if let Some((_, name)) = HOMEPAGES.iter().find(|(i, _)| *i == id) {
                return Ok((*name).to_string());
            }
            if id == 8 && top_menu_names(settings)?.iter().any(|n| n == "hot") {
                return Ok("hot".to_string());
            }
        }
    } else if settings.get("login_required")?.truthy() {
        return Ok("blank".to_string());
    }
    // ThemeModifierHelper#custom_homepage
    let theme_ids =
        crate::svg_sprite::transform_ids(conn, Some(settings.get("default_theme_id")?.to_i()))
            .await?;
    let custom: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM theme_modifier_sets \
         WHERE theme_id = ANY($1) AND custom_homepage)",
    )
    .bind(&theme_ids)
    .fetch_one(&mut *conn)
    .await?;
    if custom {
        return Ok("custom".to_string());
    }
    let homepage = if user_id.is_some() {
        homepage(settings)?
    } else {
        anonymous_homepage(settings)?
    };
    if plugin_api::homepage_available(conn, settings, guardian, &homepage).await? {
        return Ok(homepage);
    }
    // top_menu_homepage
    let names = top_menu_names(settings)?;
    Ok(if user_id.is_some() {
        names.into_iter().next()
    } else {
        names
            .into_iter()
            .find(|n| ANONYMOUS_MENU_ITEMS.contains(&n.as_str()))
    }
    .unwrap_or_default())
}

/// `SiteSetting.anonymous_homepage`: the homepage when an anonymous filter
/// or a plugin homepage open to anonymous visitors, else the first
/// anonymous top menu item.
fn anonymous_homepage(settings: &SiteSettings) -> Result<String, AppError> {
    let homepage = homepage(settings)?;
    if ANONYMOUS_MENU_ITEMS.contains(&homepage.as_str())
        || plugin_api::homepages(settings)?
            .iter()
            .any(|(_, h)| h.id == homepage && h.anonymous)
    {
        return Ok(homepage);
    }
    Ok(top_menu_names(settings)?
        .into_iter()
        .find(|n| ANONYMOUS_MENU_ITEMS.contains(&n.as_str()))
        .unwrap_or_default())
}

/// The plugin homepage this visitor's homepage is, if any.
pub fn plugin_homepage(
    settings: &SiteSettings,
    homepage: &str,
) -> Result<Option<plugin_api::HomepageOption>, AppError> {
    Ok(plugin_api::homepages(settings)?
        .into_iter()
        .find(|(_, h)| h.id == homepage))
}

/// `SiteSetting.homepage`: default_homepage when it is one of
/// `HomepageSiteSetting.choices`, else the first top menu item.
fn homepage(settings: &SiteSettings) -> Result<String, AppError> {
    let configured = settings.get("default_homepage")?.to_s();
    if !configured.is_empty() && homepage_choices(settings)?.contains(&configured.as_str()) {
        return Ok(configured);
    }
    Ok(top_menu_names(settings)?
        .into_iter()
        .next()
        .unwrap_or_default())
}

/// `HomepageSiteSetting.choices`: `TopMenu.homepage_choices`
/// (`choices | (Discourse.filters - unread)`, discourse-topic-voting's
/// `votes` among the filters), then the plugins' homepages.
fn homepage_choices(settings: &SiteSettings) -> Result<Vec<&'static str>, AppError> {
    let mut choices = vec![
        "latest",
        "new",
        "unseen",
        "top",
        "categories",
        "read",
        "posted",
        "bookmarks",
        "hot",
        "votes",
    ];
    if !settings.get("enable_unified_new")?.truthy() {
        choices.push("unread");
    }
    choices.extend(plugin_api::homepages(settings)?.iter().map(|(_, h)| h.id));
    Ok(choices)
}

/// `SiteSetting.top_menu_items.map(&:name)`
fn top_menu_names(settings: &SiteSettings) -> Result<Vec<String>, AppError> {
    Ok(settings
        .get("top_menu")?
        .to_s()
        .split('|')
        .filter(|item| !item.is_empty())
        .map(|item| item.split(',').next().unwrap_or_default().to_string())
        .collect())
}
