//! Light and dark: ApplicationHelper's color scheme stylesheets and meta
//! tags (discourse_color_scheme_stylesheets, discourse_theme_color_meta_tags,
//! discourse_color_scheme_meta_tag), from the theme's palettes, the
//! member's color_scheme_id, dark_scheme_id and interface_color_mode, and
//! the `color_scheme_id`, `dark_scheme_id` and `forced_color_mode` cookies
//! (the last written by the interface color selector,
//! static/js/interface-color.js).
//!
//! The theme is the default theme: the port does not resolve a member's
//! own theme (theme_ids, the theme_ids cookie, preview_theme_id) anywhere
//! yet.

use axum::extract::Request;
use axum::http::HeaderMap;
use axum::middleware::Next;
use axum::response::Response;
use sqlx::PgConnection;

use crate::color_scheme::ColorScheme;
use crate::i18n::I18n;
use crate::site_settings::SiteSettings;

/// `UserOption::LIGHT_MODE` and `DARK_MODE` (AUTO_MODE is 1).
pub const LIGHT_MODE: i32 = 2;
pub const DARK_MODE: i32 = 3;

/// The request's cookies the helpers read, as Rack decodes them.
#[derive(Clone, Default, Debug, PartialEq, Eq, Hash)]
pub struct Cookies {
    pub forced_color_mode: Option<String>,
    pub color_scheme_id: Option<String>,
    pub dark_scheme_id: Option<String>,
}

impl Cookies {
    pub fn from_headers(headers: &HeaderMap) -> Cookies {
        let get = |name| {
            crate::session::current::cookie(headers, name).map(|v| {
                percent_encoding::percent_decode_str(&v.replace('+', " "))
                    .decode_utf8_lossy()
                    .into_owned()
            })
        };
        Cookies {
            forced_color_mode: get("forced_color_mode"),
            color_scheme_id: get("color_scheme_id"),
            dark_scheme_id: get("dark_scheme_id"),
        }
    }
}

tokio::task_local! {
    static COOKIES: Cookies;
}

/// The request's cookies, scoped by [`layer`]; none outside a request.
pub fn cookies() -> Cookies {
    COOKIES.try_with(Clone::clone).unwrap_or_default()
}

/// Reads the cookies once per request for the pages it renders.
pub async fn layer(request: Request, next: Next) -> Response {
    let cookies = Cookies::from_headers(request.headers());
    COOKIES.scope(cookies, next.run(request)).await
}

/// The member's user_options columns the helpers read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Options {
    pub color_scheme_id: Option<i32>,
    pub dark_scheme_id: Option<i32>,
    pub interface_color_mode: i32,
}

/// What the head draws: the theme-color and color-scheme meta tags and the
/// color definitions stylesheets.
///
/// Empty for a page drawn without the chrome, whose layout links the
/// theme's light palette.
#[derive(Clone, Debug, Default)]
pub struct Head {
    pub meta: String,
    pub links: String,
    /// The interface color selector's header item under
    /// `interface_color_selector` header, empty otherwise.
    pub header_item: String,
    /// The selector's sidebar footer button under `sidebar_footer`.
    pub footer_button: String,
    /// The session's userColorSchemeId and userDarkSchemeId
    /// (`user_scheme_id || -1`, `user_dark_scheme_id || -1`).
    pub user_scheme_id: i32,
    pub user_dark_scheme_id: i32,
    /// InterfaceColor's selectorAvailable and colorMode.
    pub selector_available: bool,
    pub selector_mode: Option<&'static str>,
}

impl Head {
    fn light_only(base_path: &str) -> Head {
        Head {
            meta: String::new(),
            links: link_tag(
                &format!("{base_path}/assets/color_definitions_light.css"),
                "all",
                "light-scheme",
                None,
            ),
            header_item: String::new(),
            footer_button: String::new(),
            user_scheme_id: -1,
            user_dark_scheme_id: -1,
            selector_available: false,
            selector_mode: None,
        }
    }
}

/// The default theme's columns the helpers read.
struct Theme {
    id: i32,
    color_scheme_id: Option<i32>,
    dark_color_scheme_id: Option<i32>,
    /// `theme.color_scheme`: its color_scheme_id when that scheme exists
    /// (not a remote copy).
    color_scheme: Option<i32>,
}

/// `ColorScheme.exists?(id:, user_selectable: true)` and friends.
async fn scheme_exists(
    conn: &mut PgConnection,
    id: i64,
    user_selectable: bool,
    theme_id: Option<i32>,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM color_schemes WHERE id = $1 AND NOT remote_copy \
           AND ($2 = false OR user_selectable) AND ($3::int IS NULL OR theme_id = $3))",
    )
    .bind(id)
    .bind(user_selectable)
    .bind(theme_id)
    .fetch_one(conn)
    .await
}

/// `ColorScheme.valid_id(id)`: Integer(id) when such a scheme exists.
async fn valid_id(conn: &mut PgConnection, raw: &str) -> Result<Option<i32>, sqlx::Error> {
    let Some(id) = crate::ruby::integer(raw).and_then(|i| i32::try_from(i).ok()) else {
        return Ok(None);
    };
    let found: Option<i32> =
        sqlx::query_scalar("SELECT id FROM color_schemes WHERE id = $1 AND NOT remote_copy")
            .bind(id)
            .fetch_optional(conn)
            .await?;
    Ok(found)
}

/// `user_scheme_id` / `user_dark_scheme_id`: the cookie, else the member's
/// option, when it names a user selectable scheme or the theme's own.
async fn user_scheme(
    conn: &mut PgConnection,
    cookie: Option<&str>,
    option: Option<i32>,
    theme_scheme: Option<i32>,
) -> Result<Option<i32>, sqlx::Error> {
    let raw = match (cookie, option) {
        (Some(c), _) => c.to_string(),
        (None, Some(o)) => o.to_string(),
        (None, None) => return Ok(None),
    };
    // exists?(id:) casts as ActiveModel's integer type does (to_i); the
    // theme's id is compared with to_i.
    let cast = crate::ruby::to_i(&raw);
    let allowed = (!raw.trim().is_empty() && scheme_exists(conn, cast, true, None).await?)
        || theme_scheme.is_some_and(|t| i64::from(t) == cast);
    if !allowed {
        return Ok(None);
    }
    valid_id(conn, &raw).await
}

/// The color definitions stylesheet of a scheme (None: the base palette):
/// the theme's light and dark ones at their fixed URLs, any other scheme
/// by id (crate::routes::stylesheets).
fn href(base_path: &str, theme: &Theme, scheme: Option<i32>) -> String {
    if scheme == theme.color_scheme {
        format!("{base_path}/assets/color_definitions_light.css")
    } else if scheme.is_some() && scheme == theme.dark_color_scheme_id {
        format!("{base_path}/assets/color_definitions_dark.css")
    } else {
        match scheme {
            Some(id) => format!("{base_path}/assets/color_definitions_scheme/{id}.css"),
            None => format!("{base_path}/assets/color_definitions_light.css"),
        }
    }
}

/// `color_scheme_stylesheet_details(id, fallback_to_base: true)` for the
/// default theme: the scheme it serves (the one asked for when it exists,
/// else the theme's own, else the base palette, -1) and its stylesheet.
pub async fn scheme_stylesheet(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    base_path: &str,
    id: Option<i64>,
) -> Result<(i32, String), crate::AppError> {
    let theme_id = i32::try_from(settings.get("default_theme_id")?.to_i()).unwrap_or(0);
    let row: Option<(Option<i32>, Option<i32>, Option<i32>)> = sqlx::query_as(
        "SELECT t.color_scheme_id, t.dark_color_scheme_id, cs.id \
         FROM themes t \
         LEFT JOIN color_schemes cs ON cs.id = t.color_scheme_id AND NOT cs.remote_copy \
         WHERE t.id = $1",
    )
    .bind(theme_id)
    .fetch_optional(&mut *conn)
    .await?;
    let (color_scheme_id, dark_color_scheme_id, color_scheme) = row.unwrap_or_default();
    let theme = Theme {
        id: theme_id,
        color_scheme_id,
        dark_color_scheme_id,
        color_scheme,
    };
    // ColorScheme.find_by(id:)
    let found = match id.and_then(|id| i32::try_from(id).ok()) {
        Some(id) => ColorScheme::find(&mut *conn, id).await?.map(|s| s.id),
        None => None,
    };
    let scheme = found.or(theme.color_scheme);
    Ok((scheme.unwrap_or(-1), href(base_path, &theme, scheme)))
}

/// `color_scheme_stylesheet_link_tag`
fn link_tag(href: &str, media: &str, class: &str, scheme_id: Option<i32>) -> String {
    let id = match scheme_id {
        Some(id) if id != -1 => format!(" data-scheme-id=\"{id}\""),
        _ => String::new(),
    };
    format!(
        "<link href=\"{}\" media=\"{media}\" rel=\"stylesheet\" class=\"{class}\"{id}/>",
        escape(href)
    )
}

/// [`head`] kept per site, cookies and options until the theme tables, the
/// palettes or the site settings change (crate::theme_cache), as
/// Stylesheet::Manager keeps the color scheme stylesheets' details.
pub async fn head_at(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    i18n: &I18n,
    base_path: &str,
    cookies: &Cookies,
    options: Option<&Options>,
    at: &crate::theme_cache::Fingerprint,
) -> Result<Head, crate::AppError> {
    type Key = (String, Cookies, Option<Options>);
    static KEPT: crate::theme_cache::Kept<Key, Head> = crate::theme_cache::Kept::new();
    let key = (base_path.to_string(), cookies.clone(), options.copied());
    if let Some(head) = KEPT.get(at, key.clone()) {
        return Ok(head);
    }
    let head = head(conn, settings, i18n, base_path, cookies, options).await?;
    KEPT.put(at, key, head.clone());
    Ok(head)
}

/// The helpers' answers for the request: the member's options (None for a
/// visitor) and the request's cookies.
pub async fn head(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    i18n: &I18n,
    base_path: &str,
    cookies: &Cookies,
    options: Option<&Options>,
) -> Result<Head, crate::AppError> {
    let Ok(theme_id) = i32::try_from(settings.get("default_theme_id")?.to_i()) else {
        return Ok(Head::light_only(base_path));
    };
    // A theme that is gone has no palettes of its own.
    let row: Option<(Option<i32>, Option<i32>, Option<i32>)> = sqlx::query_as(
        "SELECT t.color_scheme_id, t.dark_color_scheme_id, cs.id \
         FROM themes t \
         LEFT JOIN color_schemes cs ON cs.id = t.color_scheme_id AND NOT cs.remote_copy \
         WHERE t.id = $1",
    )
    .bind(theme_id)
    .fetch_optional(&mut *conn)
    .await?;
    let (color_scheme_id, dark_color_scheme_id, color_scheme) = row.unwrap_or_default();
    let theme = Theme {
        id: theme_id,
        color_scheme_id,
        dark_color_scheme_id,
        color_scheme,
    };
    // theme_limits_color_schemes?
    let limits: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM theme_modifier_sets \
           WHERE theme_id = $1 AND only_theme_color_schemes)",
    )
    .bind(theme.id)
    .fetch_one(&mut *conn)
    .await?;

    // scheme_id
    let user_light = user_scheme(
        &mut *conn,
        cookies.color_scheme_id.as_deref(),
        options.and_then(|o| o.color_scheme_id),
        theme.color_scheme_id,
    )
    .await?;
    let mut scheme_id = theme.color_scheme_id;
    if let Some(user) = user_light
        && (!limits || scheme_exists(&mut *conn, user.into(), false, Some(theme.id)).await?)
    {
        scheme_id = Some(user);
    }

    // dark_scheme_id: None for -1, Some(None) for nil (a theme that limits
    // the schemes and has no light one).
    let user_dark = user_scheme(
        &mut *conn,
        cookies.dark_scheme_id.as_deref(),
        options.and_then(|o| o.dark_scheme_id),
        theme.dark_color_scheme_id,
    )
    .await?;
    let dark_scheme_id: Option<Option<i32>> = match user_dark {
        Some(user)
            if !limits || scheme_exists(&mut *conn, user.into(), false, Some(theme.id)).await? =>
        {
            Some(Some(user))
        }
        _ => match theme.dark_color_scheme_id {
            Some(id) => Some(Some(id)),
            None if limits => Some(theme.color_scheme_id),
            None => None,
        },
    };
    let dark_scheme_id = dark_scheme_id.filter(|id| *id != Some(-1));

    // dark_color_scheme?
    let light_dark = match scheme_id {
        Some(id) => match ColorScheme::find(&mut *conn, id).await? {
            Some(scheme) => scheme.is_dark()?.unwrap_or(false),
            None => false,
        },
        None => false,
    };
    let forced_cookie = cookies
        .forced_color_mode
        .as_deref()
        .filter(|c| !crate::ruby::is_blank(c));
    let mode = options.map(|o| o.interface_color_mode);
    // forced_light_mode? and forced_dark_mode?
    let forced_light = !light_dark
        && match forced_cookie {
            Some(c) => c == "light",
            None => mode == Some(LIGHT_MODE),
        };
    let forced_dark = dark_scheme_id.is_some()
        && match forced_cookie {
            Some(c) => c == "dark",
            None => mode == Some(DARK_MODE),
        };
    let (light_media, dark_media) = if forced_light {
        ("all", "none")
    } else if forced_dark {
        ("none", "all")
    } else {
        (
            "(prefers-color-scheme: light)",
            "(prefers-color-scheme: dark)",
        )
    };

    // discourse_color_scheme_stylesheets: the light one falls back to the
    // theme's palette and then the base one, the dark one to nothing.
    let light_scheme = match scheme_id {
        Some(id) if ColorScheme::find(&mut *conn, id).await?.is_some() => Some(id),
        _ => theme.color_scheme,
    };
    let dark_scheme = match dark_scheme_id.flatten() {
        Some(id) => ColorScheme::find(&mut *conn, id).await?.map(|s| s.id),
        None => None,
    };
    let light_href = href(base_path, &theme, light_scheme);
    let links = match dark_scheme {
        Some(dark) if Some(dark) != light_scheme => {
            link_tag(&light_href, light_media, "light-scheme", scheme_id)
                + &link_tag(
                    &href(base_path, &theme, Some(dark)),
                    dark_media,
                    "dark-scheme",
                    dark_scheme_id.flatten(),
                )
        }
        _ => link_tag(&light_href, "all", "light-scheme", scheme_id),
    };

    // discourse_theme_color_meta_tags and discourse_color_scheme_meta_tag
    let hex = |h: Option<String>| h.unwrap_or_default();
    let light_hex = hex(crate::color_scheme::hex_for_name_in(
        &mut *conn,
        settings,
        "header_background",
        scheme_id,
    )
    .await?);
    let mut meta = String::new();
    match dark_scheme_id {
        Some(dark) => {
            let dark_hex = hex(crate::color_scheme::hex_for_name_in(
                &mut *conn,
                settings,
                "header_background",
                dark,
            )
            .await?);
            meta.push_str(&format!(
                "<meta name=\"theme-color\" media=\"{light_media}\" content=\"#{light_hex}\">\n\
                 <meta name=\"theme-color\" media=\"{dark_media}\" content=\"#{dark_hex}\">\n"
            ));
            meta.push_str("<meta name=\"color-scheme\" content=\"light dark\">\n");
        }
        None => {
            meta.push_str(&format!(
                "<meta name=\"theme-color\" media=\"all\" content=\"#{light_hex}\">\n"
            ));
            let scheme = if light_dark { "dark" } else { "light" };
            meta.push_str(&format!(
                "<meta name=\"color-scheme\" content=\"{scheme}\">\n"
            ));
        }
    }

    // The client's InterfaceColor service: the selector shows when the page
    // has a dark stylesheet and the light palette is not itself dark.
    let selector_available = links.contains("class=\"dark-scheme\"") && !light_dark;
    let (mut header_item, mut footer_button) = (String::new(), String::new());
    if selector_available {
        let button = selector_button(i18n, selector_mode(cookies, mode));
        match settings.get("interface_color_selector")?.to_s().as_ref() {
            "header" => {
                header_item = format!(
                    "<li class=\"header-dropdown-toggle header-color-scheme-toggle\">{button}</li>"
                )
            }
            "sidebar_footer" => footer_button = button,
            _ => {}
        }
    }
    Ok(Head {
        meta,
        links,
        header_item,
        footer_button,
        user_scheme_id: user_light.unwrap_or(-1),
        user_dark_scheme_id: user_dark.unwrap_or(-1),
        selector_available,
        selector_mode: if selector_available {
            selector_mode(cookies, mode)
        } else {
            None
        },
    })
}

/// InterfaceColor#ensureCorrectMode's mode: the cookie's `auto`, `light`
/// or `dark`, else the member's light or dark option. currentUser's
/// prefersAutoColor compares with INTERFACE_COLOR_MODES.AUTODARK, which is
/// undefined, so the auto option chooses nothing (None).
fn selector_mode(cookies: &Cookies, mode: Option<i32>) -> Option<&'static str> {
    match cookies.forced_color_mode.as_deref() {
        Some("auto") => Some("auto"),
        Some("light") => Some("light"),
        Some("dark") => Some("dark"),
        _ => match mode {
            Some(LIGHT_MODE) => Some("light"),
            Some(DARK_MODE) => Some("dark"),
            _ => None,
        },
    }
}

/// InterfaceColorSelector's DMenu trigger: the forced mode's icon, else
/// the half circle. Without a mode the label keeps I18n.js's placeholder
/// for the missing value.
fn selector_button(i18n: &I18n, mode: Option<&str>) -> String {
    let t = |key: &str| {
        let key = format!("js.sidebar.footer.interface_color_selector.{key}");
        i18n.t(&key).map(str::to_string).unwrap_or(key)
    };
    let label = t("aria_label").replace("%{mode}", mode.unwrap_or("[missing %{mode} value]"));
    let icon = match mode {
        Some("light") => "sun",
        Some("dark") => "moon",
        _ => "circle-half-stroke",
    };
    let current = mode
        .map(|m| format!(" data-current-mode=\"{m}\""))
        .unwrap_or_default();
    format!(
        "<button aria-expanded=\"false\" aria-label=\"{}\" class=\"btn no-text btn-icon fk-d-menu__trigger interface-color-selector-trigger btn-flat sidebar-footer-actions-button interface-color-selector icon\" title=\"{}\" data-identifier=\"interface-color-selector\" data-trigger=\"\" type=\"button\"{current}>{}</button>",
        escape(&label),
        escape(&t("title")),
        crate::topic_list_view::icon(icon, None)
    )
}

fn escape(s: &str) -> String {
    askama::filters::escape(s, askama::filters::Html)
        .map(|e| e.to_string())
        .unwrap_or_default()
}
