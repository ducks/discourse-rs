//! The admin design wizard: Admin::Config::DesignWizardController's index
//! (DesignWizardSerializer over the core themes, with
//! DesignWizard::PalettePairs and ThemeScreenshotThumbnails) and update
//! (DesignWizard::Apply: the theme's light and dark palettes, built-in
//! palettes materialized, which palettes members may pick, then the site
//! settings and the theme's themeable settings).

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::color_scheme::ColorScheme;
use crate::site_settings::{Definitions, SiteSettings};
use crate::themes::{self, Theme};
use crate::{AppError, Unsupported};

/// `Theme::CORE_THEMES`' ids.
pub const CORE_THEMES: [i32; 2] = [-1, -2];
const HORIZON: i32 = -2;
/// `DesignWizard::PalettePairs::DARK_NAME_SUFFIX`
const DARK_NAME_SUFFIX: &str = " Dark";
/// `DesignWizard::PalettePairs::BUILT_IN_PAIRS`: key, light, dark.
const BUILT_IN_PAIRS: [(&str, Option<&str>, Option<&str>); 4] = [
    ("default", Some("Light"), Some("Dark")),
    ("wcag", Some("WCAG"), Some("WCAG Dark")),
    ("solarized", Some("Solarized Light"), Some("Solarized Dark")),
    ("dracula", None, Some("Dracula")),
];
/// `ThemeScreenshotThumbnails` sizes.
const THUMBNAIL_WIDTH: i32 = 800;
const THUMBNAIL_HEIGHT: i32 = 450;
/// `ThemeScreenshotThumbnails::RETRY_INTERVAL`
const THUMBNAIL_RETRY_SECONDS: i64 = 3600;

/// What the wizard reads beside the connection.
pub struct Cx<'a> {
    pub themes: &'a themes::Ctx<'a>,
    pub defs: &'a Definitions,
}

impl Cx<'_> {
    fn settings(&self) -> &SiteSettings {
        self.themes.settings
    }
}

/// `SiteSetting.homepage`: default_homepage when it is one of
/// HomepageSiteSetting's choices, else the top menu's first item.
async fn homepage(conn: &mut PgConnection, cx: &Cx<'_>) -> Result<String, AppError> {
    let configured = cx.settings().get("default_homepage")?.to_s().to_string();
    if !crate::ruby::is_blank(&configured)
        && homepage_choices(conn, cx).await?.contains(&configured)
    {
        return Ok(configured);
    }
    let top_menu = cx.settings().get("top_menu")?.to_s().to_string();
    Ok(top_menu
        .split('|')
        .next()
        .and_then(|i| i.split(',').next())
        .unwrap_or("latest")
        .to_string())
}

/// `HomepageSiteSetting.choices`
async fn homepage_choices(conn: &mut PgConnection, cx: &Cx<'_>) -> Result<Vec<String>, AppError> {
    let admin_cx = crate::admin_site_settings::Context {
        defs: cx.defs,
        settings: cx.settings(),
        i18n: cx.themes.i18n,
        globals: &cx.themes.urls.config.globals,
        base_path: cx.themes.urls.config.globals.relative_url_root(),
        urls: cx.themes.urls,
    };
    let (values, _) = crate::setting_enums::runtime(conn, &admin_cx, "HomepageSiteSetting").await?;
    Ok(values
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.get("value").and_then(Value::as_str))
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .collect())
}

/// `ThemeScreenshotThumbnails.url_for(theme, name)`: the screenshot's
/// 800x450 webp when made; otherwise its generation queued (once an hour
/// per theme) and nothing.
async fn thumbnail_url(
    conn: &mut PgConnection,
    theme_id: i32,
    upload_id: i32,
) -> Result<Option<String>, AppError> {
    let existing: Option<String> = sqlx::query_scalar(
        "SELECT url FROM optimized_images WHERE upload_id = $1 AND width = $2 AND height = $3 \
         AND extension = '.webp' ORDER BY id LIMIT 1",
    )
    .bind(upload_id)
    .bind(THUMBNAIL_WIDTH)
    .bind(THUMBNAIL_HEIGHT)
    .fetch_optional(&mut *conn)
    .await?;
    if existing.is_some() {
        return Ok(existing);
    }
    let key = format!("theme_screenshot_thumbnails_{theme_id}");
    if crate::owned_schema::set_once(conn, &key, THUMBNAIL_RETRY_SECONDS).await? {
        crate::jobs::enqueue(
            conn,
            "generate_theme_screenshot_thumbnails",
            json!({ "theme_id": theme_id }),
        )
        .await?;
    }
    Ok(None)
}

/// `{id, name, colors}` of a palette (serialize_scheme).
fn scheme_json(scheme: &ColorScheme) -> Result<Value, AppError> {
    let mut colors = Map::new();
    for (name, hex) in scheme.sorted_resolved_colors()? {
        colors.insert(name, json!(hex));
    }
    Ok(json!({"id": scheme.id, "name": scheme.name, "colors": colors}))
}

fn pair_json(
    key: &str,
    name: &str,
    light: Option<&ColorScheme>,
    dark: Option<&ColorScheme>,
) -> Result<Value, AppError> {
    Ok(json!({
        "key": key,
        "name": name,
        "dark_only": light.is_none(),
        "light": light.map(scheme_json).transpose()?,
        "dark": dark.map(scheme_json).transpose()?,
    }))
}

/// ActiveSupport's `parameterize.underscore` of an ASCII name.
fn parameterize(name: &str) -> String {
    let mut out = String::new();
    for c in name.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').replace('-', "_")
}

/// `ColorScheme.find_by(base_scheme_id:, via_wizard: true)` (no remote
/// copies), the built-in palette materialized.
async fn materialized(conn: &mut PgConnection, base: i32) -> Result<Option<ColorScheme>, AppError> {
    let id: Option<i32> = sqlx::query_scalar(
        "SELECT id FROM color_schemes WHERE base_scheme_id = $1 AND via_wizard AND NOT remote_copy \
         ORDER BY id LIMIT 1",
    )
    .bind(base)
    .fetch_optional(&mut *conn)
    .await?;
    match id {
        Some(id) => Ok(ColorScheme::find(conn, id).await?),
        None => Ok(None),
    }
}

/// `DesignWizard::PalettePairs.for_theme`
async fn palette_pairs(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    theme_id: i32,
) -> Result<Value, AppError> {
    let owned: Vec<i32> = sqlx::query_scalar(
        "SELECT id FROM color_schemes WHERE theme_id = $1 AND NOT remote_copy ORDER BY id",
    )
    .bind(theme_id)
    .fetch_all(&mut *conn)
    .await?;
    let mut out = Vec::new();
    if !owned.is_empty() {
        let mut schemes = Vec::new();
        for id in owned {
            if let Some(s) = ColorScheme::find(conn, id).await? {
                schemes.push(s);
            }
        }
        // group_by the name less " Dark", in order of first appearance.
        let mut groups: Vec<(String, Vec<&ColorScheme>)> = Vec::new();
        for s in &schemes {
            let base = s
                .name
                .strip_suffix(DARK_NAME_SUFFIX)
                .unwrap_or(&s.name)
                .to_string();
            match groups.iter_mut().find(|(b, _)| *b == base) {
                Some((_, members)) => members.push(s),
                None => groups.push((base, vec![s])),
            }
        }
        for (base, members) in groups {
            let light = members
                .iter()
                .find(|s| !s.name.ends_with(DARK_NAME_SUFFIX))
                .copied();
            let dark = members
                .iter()
                .find(|s| s.name.ends_with(DARK_NAME_SUFFIX))
                .copied();
            out.push(pair_json(&parameterize(&base), &base, light, dark)?);
        }
        return Ok(Value::Array(out));
    }
    let bases = ColorScheme::base_color_schemes(cx.themes.i18n);
    for (key, light, dark) in BUILT_IN_PAIRS {
        let mut resolve = async |name: Option<&str>| -> Result<Option<ColorScheme>, AppError> {
            let Some(name) = name else {
                return Ok(None);
            };
            let id = crate::color_scheme::built_in_id(name).ok_or(Unsupported(
                "a built-in palette missing from NAMES_TO_ID_MAP",
            ))?;
            Ok(match materialized(conn, id).await? {
                Some(s) => Some(s),
                None => bases.iter().find(|s| s.base_scheme_id == Some(id)).cloned(),
            })
        };
        let light = resolve(light).await?;
        let dark = resolve(dark).await?;
        let name = cx
            .themes
            .i18n
            .t(&format!("design_wizard.palette_pairs.{key}"))
            .unwrap_or(key)
            .to_string();
        out.push(pair_json(key, &name, light.as_ref(), dark.as_ref())?);
    }
    Ok(Value::Array(out))
}

/// The upload of a theme's screenshot field (`screenshot_upload(name)`).
async fn screenshot_upload(
    conn: &mut PgConnection,
    theme_id: i32,
    name: &str,
) -> Result<Option<(i32, String)>, AppError> {
    Ok(sqlx::query_as(
        "SELECT u.id, u.url FROM theme_fields f JOIN uploads u ON u.id = f.upload_id \
         WHERE f.theme_id = $1 AND f.name = $2 AND f.type_id = 7 ORDER BY f.id LIMIT 1",
    )
    .bind(theme_id)
    .bind(name)
    .fetch_optional(&mut *conn)
    .await?)
}

/// DesignWizardThemeSerializer
async fn theme_json(conn: &mut PgConnection, cx: &Cx<'_>, t: &Theme) -> Result<Value, AppError> {
    let basic = themes::basic(conn, cx.themes, t).await?;
    let default = i64::from(t.id) == cx.settings().get("default_theme_id")?.to_i();
    let mut out = Map::new();
    out.insert("id".into(), json!(t.id));
    out.insert("name".into(), json!(t.name));
    out.insert("description".into(), basic["description"].clone());
    out.insert("default".into(), json!(default));
    out.insert("color_scheme_id".into(), json!(t.color_scheme_id));
    out.insert("dark_color_scheme_id".into(), json!(t.dark_color_scheme_id));
    for (key, name) in [
        ("screenshot_light_url", "screenshot_light"),
        ("screenshot_dark_url", "screenshot_dark"),
    ] {
        let url = match screenshot_upload(conn, t.id, name).await? {
            Some((upload_id, url)) => {
                Some(thumbnail_url(conn, t.id, upload_id).await?.unwrap_or(url))
            }
            None => None,
        };
        out.insert(key.into(), json!(url));
    }
    out.insert("palette_pairs".into(), palette_pairs(conn, cx, t.id).await?);
    Ok(Value::Object(out))
}

/// DesignWizardSerializer over the default theme and the core themes.
pub async fn index(conn: &mut PgConnection, cx: &Cx<'_>) -> Result<Value, AppError> {
    let default_id = cx.settings().get("default_theme_id")?.to_i() as i32;
    let default = Theme::find(conn, default_id).await?;
    let current_theme = match &default {
        Some(t) if !CORE_THEMES.contains(&t.id) => json!({"id": t.id, "name": t.name}),
        _ => Value::Null,
    };
    // preselected_theme: the default when it is a core theme, else Horizon.
    let preselected = match &default {
        Some(t) if CORE_THEMES.contains(&t.id) => Some(t.id),
        _ => Theme::find(conn, HORIZON).await?.map(|t| t.id),
    };
    let palettes_user_selectable = palettes_user_selectable(conn, preselected).await?;
    let core = Theme::select(conn, "WHERE id IN (-1, -2) ORDER BY id").await?;
    let mut list = Vec::new();
    for t in &core {
        list.push(theme_json(conn, cx, t).await?);
    }
    Ok(json!({
        "current_theme": current_theme,
        "base_font": cx.settings().get("base_font")?.to_s(),
        "heading_font": cx.settings().get("heading_font")?.to_s(),
        "homepage": homepage(conn, cx).await?,
        "palettes_user_selectable": palettes_user_selectable,
        "themes": list,
    }))
}

/// The palettes the wizard offers for a theme: its own, else the
/// materialized built-ins (no remote copies either way).
const OFFERED: &str = "(CASE WHEN EXISTS (SELECT 1 FROM color_schemes WHERE theme_id \
     IS NOT DISTINCT FROM $1 AND NOT remote_copy) THEN theme_id IS NOT DISTINCT FROM $1 \
     ELSE via_wizard END) AND NOT remote_copy";

async fn palettes_user_selectable(
    conn: &mut PgConnection,
    theme_id: Option<i32>,
) -> Result<bool, AppError> {
    let (any, all_selectable): (bool, bool) = sqlx::query_as(&format!(
        "SELECT count(*) > 0, bool_and(user_selectable) IS NOT FALSE FROM color_schemes \
         WHERE {OFFERED}"
    ))
    .bind(theme_id)
    .fetch_one(&mut *conn)
    .await?;
    Ok(any && all_selectable)
}

/// DesignWizard::Apply's params, cast as ActiveModel casts them.
#[derive(Debug, Default)]
pub struct Params {
    pub theme_id: Option<i64>,
    pub light_palette_id: Option<i64>,
    pub dark_palette_id: Option<i64>,
    pub palettes_user_selectable: Option<bool>,
    pub base_font: Option<String>,
    pub heading_font: Option<String>,
    pub homepage: Option<String>,
    pub category_page_style: Option<String>,
    pub enable_welcome_banner: Option<bool>,
    pub welcome_banner_location: Option<String>,
    pub search_experience: Option<String>,
}

/// ActiveModel's integer cast: blank or non-numeric is nil.
fn cast_integer(v: Option<&Value>) -> Option<i64> {
    match v? {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f.trunc() as i64)),
        Value::Bool(b) => Some(i64::from(*b)),
        other => {
            let s = crate::params::scalar(other)?;
            let trimmed = s.trim_start();
            let digits = trimmed.strip_prefix(['+', '-']).unwrap_or(trimmed);
            if crate::ruby::is_blank(&s) || !digits.starts_with(|c: char| c.is_ascii_digit()) {
                return None;
            }
            Some(crate::ruby::to_i(&s))
        }
    }
}

fn cast_bool(v: Option<&Value>) -> Option<bool> {
    match v? {
        Value::Bool(b) => Some(*b),
        Value::Null => None,
        Value::Number(n) => Some(n.as_f64() != Some(0.0)),
        other => crate::ruby::cast_boolean(crate::params::scalar(other).as_deref()),
    }
}

fn cast_string(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::Null => None,
        other => crate::params::scalar(other),
    }
}

impl Params {
    pub fn from(p: &Map<String, Value>) -> Params {
        let presence = |s: Option<String>| s.filter(|s| !crate::ruby::is_blank(s));
        let mut params = Params {
            theme_id: cast_integer(p.get("theme_id")),
            light_palette_id: cast_integer(p.get("light_palette_id")),
            dark_palette_id: cast_integer(p.get("dark_palette_id")),
            palettes_user_selectable: if p.contains_key("palettes_user_selectable") {
                cast_bool(p.get("palettes_user_selectable"))
            } else {
                Some(false)
            },
            base_font: cast_string(p.get("base_font")),
            heading_font: cast_string(p.get("heading_font")),
            homepage: cast_string(p.get("homepage")),
            category_page_style: cast_string(p.get("category_page_style")),
            enable_welcome_banner: cast_bool(p.get("enable_welcome_banner")),
            welcome_banner_location: presence(cast_string(p.get("welcome_banner_location"))),
            search_experience: presence(cast_string(p.get("search_experience"))),
        };
        // The built-in light palette is the theme without one.
        if params.light_palette_id == crate::color_scheme::built_in_id("Light").map(i64::from) {
            params.light_palette_id = None;
        }
        params
    }

    /// The contract's errors, as full messages.
    fn errors(&self) -> Vec<String> {
        let mut errors = Vec::new();
        let recorded = |setting: &str, key: &str| -> Vec<String> {
            crate::setting_enums::recorded(setting)
                .and_then(|r| r.get(key))
                .and_then(Value::as_array)
                .map(|vs| {
                    vs.iter()
                        .filter_map(|v| v.get("value").and_then(Value::as_str).map(str::to_string))
                        .collect()
                })
                .unwrap_or_default()
        };
        let not_included = |attr: &str| format!("{attr} is not included in the list");
        match self.theme_id {
            None => {
                errors.push("Theme can't be blank".to_string());
                errors.push(not_included("Theme"));
            }
            Some(id) if !CORE_THEMES.iter().any(|c| i64::from(*c) == id) => {
                errors.push(not_included("Theme"));
            }
            _ => {}
        }
        let fonts = recorded("base_font", "choices");
        let check =
            |errors: &mut Vec<String>, attr: &str, value: &Option<String>, allowed: &[String]| {
                if let Some(v) = value.as_ref().filter(|v| !crate::ruby::is_blank(v))
                    && !allowed.contains(v)
                {
                    errors.push(not_included(attr));
                }
            };
        check(&mut errors, "Base font", &self.base_font, &fonts);
        check(&mut errors, "Heading font", &self.heading_font, &fonts);
        let homepages: Vec<String> = ["latest", "new", "hot", "categories"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        check(&mut errors, "Homepage", &self.homepage, &homepages);
        check(
            &mut errors,
            "Category page style",
            &self.category_page_style,
            &recorded("desktop_category_page_style", "valid_values"),
        );
        check(
            &mut errors,
            "Welcome banner location",
            &self.welcome_banner_location,
            &recorded("welcome_banner_location", "valid_values"),
        );
        check(
            &mut errors,
            "Search experience",
            &self.search_experience,
            &recorded("search_experience", "valid_values"),
        );
        // built_in_palettes_exist
        for (attr, id) in [
            ("Light palette", self.light_palette_id),
            ("Dark palette", self.dark_palette_id),
        ] {
            if let Some(id) = id
                && id <= 0
                && i32::try_from(id)
                    .ok()
                    .and_then(crate::color_scheme::built_in_name)
                    .is_none()
            {
                errors.push(not_included(attr));
            }
        }
        errors
    }
}

/// How an Apply ends.
pub enum Outcome {
    Done,
    /// The contract's errors (400).
    FailedContract(Vec<String>),
    /// `model :theme` found none (404).
    ThemeNotFound,
    /// palettes_available_to_theme (422).
    PaletteUnavailable,
}

/// `DesignWizard::Action::ResolvePalette`: a palette by id, a built-in one
/// materialized as a via_wizard scheme.
async fn resolve_palette(
    conn: &mut PgConnection,
    i18n: &crate::i18n::I18n,
    id: Option<i64>,
) -> Result<Option<i32>, AppError> {
    let Some(id) = id.and_then(|i| i32::try_from(i).ok()) else {
        return Ok(None);
    };
    if id > 0 {
        let found: Option<i32> =
            sqlx::query_scalar("SELECT id FROM color_schemes WHERE id = $1 AND NOT remote_copy")
                .bind(id)
                .fetch_optional(&mut *conn)
                .await?;
        return Ok(found);
    }
    if let Some(existing) = materialized(conn, id).await? {
        return Ok(Some(existing.id));
    }
    // ColorScheme.create_from_base
    let scheme_name = crate::color_scheme::built_in_name(id).ok_or(Unsupported(
        "a built-in palette missing from NAMES_TO_ID_MAP",
    ))?;
    let key = format!(
        "color_schemes.{}_theme_name",
        scheme_name.to_lowercase().replace(' ', "_")
    );
    let name = i18n
        .t(&key)
        .map(str::to_string)
        .unwrap_or_else(|| format!("Translation missing: en.{key}"));
    let colors = crate::color_scheme::built_in_colors(scheme_name)
        .unwrap_or_else(|| crate::color_scheme::base_colors().to_vec());
    let scheme_id: i32 = sqlx::query_scalar(
        "INSERT INTO color_schemes (name, version, via_wizard, base_scheme_id, created_at, \
         updated_at) VALUES ($1, 1, TRUE, $2, now(), now()) RETURNING id",
    )
    .bind(&name)
    .bind(id)
    .fetch_one(&mut *conn)
    .await?;
    for (color, hex) in colors {
        sqlx::query(
            "INSERT INTO color_scheme_colors (name, hex, color_scheme_id, created_at, updated_at) \
             VALUES ($1, $2, $3, now(), now())",
        )
        .bind(color)
        .bind(hex)
        .bind(scheme_id)
        .execute(&mut *conn)
        .await?;
    }
    Ok(Some(scheme_id))
}

/// DesignWizard::Apply's transaction: the palettes. Runs in the caller's
/// transaction; the site settings follow with `apply_settings`.
pub async fn apply_palettes(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    params: &Params,
) -> Result<Result<Theme, Outcome>, AppError> {
    let errors = params.errors();
    if !errors.is_empty() {
        return Ok(Err(Outcome::FailedContract(errors)));
    }
    let theme_id = params
        .theme_id
        .and_then(|i| i32::try_from(i).ok())
        .unwrap_or(0);
    let Some(theme) = Theme::find(conn, theme_id).await? else {
        return Ok(Err(Outcome::ThemeNotFound));
    };
    // palettes_available_to_theme
    for id in [params.light_palette_id, params.dark_palette_id] {
        let Some(id) = id.filter(|i| *i > 0) else {
            continue;
        };
        let available: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM color_schemes WHERE id = $1 AND NOT remote_copy \
             AND (theme_id IS NULL OR theme_id = $2))",
        )
        .bind(i32::try_from(id).unwrap_or(0))
        .bind(theme.id)
        .fetch_one(&mut *conn)
        .await?;
        if !available {
            return Ok(Err(Outcome::PaletteUnavailable));
        }
    }
    let i18n = cx.themes.i18n;
    let light = resolve_palette(conn, i18n, params.light_palette_id).await?;
    let dark = resolve_palette(conn, i18n, params.dark_palette_id).await?;
    // assign_theme_palettes: theme.update! (a system theme may change
    // these), then its after_save.
    if theme.color_scheme_id != light || theme.dark_color_scheme_id != dark {
        sqlx::query(
            "UPDATE themes SET color_scheme_id = $2, dark_color_scheme_id = $3, updated_at = now() \
             WHERE id = $1",
        )
        .bind(theme.id)
        .bind(light)
        .bind(dark)
        .execute(&mut *conn)
        .await?;
    }
    {
        let env = crate::theme_writes::Env::load(conn, cx.themes).await?;
        crate::theme_writes::after_save(conn, &env, theme.id, &[], false).await?;
    }
    let selectable = params.palettes_user_selectable.unwrap_or(false);
    let owns: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM color_schemes WHERE theme_id = $1 AND NOT remote_copy)",
    )
    .bind(theme.id)
    .fetch_one(&mut *conn)
    .await?;
    // offer_built_in_palettes
    if selectable && !owns {
        for (_, light, dark) in BUILT_IN_PAIRS {
            for name in [light, dark].into_iter().flatten() {
                let id = crate::color_scheme::built_in_id(name).map(i64::from);
                resolve_palette(conn, i18n, id).await?;
            }
        }
    }
    // update_palette_selectability: the palettes no longer offered
    // revoked, the offered ones set (update_all: no timestamps).
    sqlx::query(&format!(
        "UPDATE color_schemes SET user_selectable = FALSE WHERE NOT remote_copy \
         AND (theme_id IN (-1, -2) OR via_wizard) AND id NOT IN (SELECT id FROM color_schemes \
         WHERE {OFFERED})"
    ))
    .bind(Some(theme.id))
    .execute(&mut *conn)
    .await?;
    sqlx::query(&format!(
        "UPDATE color_schemes SET user_selectable = $2 WHERE {OFFERED}"
    ))
    .bind(Some(theme.id))
    .bind(selectable)
    .execute(&mut *conn)
    .await?;
    let theme = Theme::find(conn, theme.id)
        .await?
        .ok_or(Unsupported("a theme gone during the design wizard"))?;
    Ok(Ok(theme))
}

/// The site settings DesignWizard::Apply writes through SiteSetting::Update
/// (`allow_changing_hidden: default_theme_id`), in site_settings.yml's
/// order: each one that changed set and logged by the admin.
pub async fn apply_settings(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    cx: &Cx<'_>,
    params: &Params,
    theme: &Theme,
    actor: i32,
) -> Result<(), AppError> {
    use crate::site_settings::Value as Setting;
    let presence = |s: &Option<String>| s.clone().filter(|s| !crate::ruby::is_blank(s));
    let mut wanted: Vec<(&str, Setting)> =
        vec![("default_theme_id", Setting::Int(i64::from(theme.id)))];
    for (name, value) in [
        ("base_font", presence(&params.base_font)),
        ("heading_font", presence(&params.heading_font)),
        ("default_homepage", presence(&params.homepage)),
        (
            "desktop_category_page_style",
            presence(&params.category_page_style),
        ),
        (
            "welcome_banner_location",
            presence(&params.welcome_banner_location),
        ),
    ] {
        if let Some(v) = value {
            wanted.push((name, Setting::Str(v)));
        }
    }
    // The dependency graph's order, the definitions' for these.
    let position = |name: &str| cx.defs.position(name).unwrap_or(usize::MAX);
    wanted.sort_by_key(|(name, _)| position(name));
    for (name, value) in wanted {
        let def = cx.defs.get(name).ok_or(Unsupported(
            "a design wizard setting missing from site_settings.yml",
        ))?;
        let previous = cx.settings().get(name)?.clone();
        if previous == value {
            continue;
        }
        crate::site_setting_update::set_and_log(conn, bus, def, &previous, &value, actor).await?;
    }
    Ok(())
}

/// update_themeable_site_settings: the theme's welcome banner and search
/// experience, each that differs from what the theme has.
pub async fn apply_themeable_settings(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    params: &Params,
    theme: &Theme,
    actor: i32,
) -> Result<(), AppError> {
    let wanted: [(&str, Option<Value>); 2] = [
        (
            "enable_welcome_banner",
            params.enable_welcome_banner.map(Value::Bool),
        ),
        (
            "search_experience",
            params.search_experience.clone().map(Value::String),
        ),
    ];
    for (name, value) in wanted {
        let Some(value) = value else {
            continue;
        };
        // SiteSetting.<name>(theme_id:): the theme's row, else the site's.
        let row: Option<(i32, Option<String>)> = sqlx::query_as(
            "SELECT data_type, value FROM theme_site_settings WHERE theme_id = $1 AND name = $2 \
             LIMIT 1",
        )
        .bind(theme.id)
        .bind(name)
        .fetch_optional(&mut *conn)
        .await?;
        let current = match row {
            Some((data_type, raw)) => {
                crate::site_settings::to_rb_value(cx.defs, name, data_type, raw.as_deref())?
            }
            None => cx.settings().get(name)?.clone(),
        };
        let same = match (&current, &value) {
            (crate::site_settings::Value::Bool(a), Value::Bool(b)) => a == b,
            (other, Value::String(s)) => other.to_s() == *s,
            _ => false,
        };
        if same {
            continue;
        }
        let raw = match &value {
            Value::Bool(b) => b.to_string(),
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        crate::theme_writes::set_theme_site_setting(conn, cx.defs, theme, name, Some(raw), actor)
            .await?
            .map_err(|message| {
                std::io::Error::other(format!("Discourse::InvalidParameters: {message}"))
            })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pair_keys_parameterize_their_names() {
        assert_eq!(parameterize("Horizon"), "horizon");
        assert_eq!(parameterize("Royal Blue"), "royal_blue");
        assert_eq!(parameterize("A & B!"), "a_b");
    }

    #[test]
    fn the_contract_names_its_attributes() {
        let p = Params::from(
            &serde_json::from_str(
                r#"{"theme_id": "5", "light_palette_id": "-99", "homepage": "top"}"#,
            )
            .unwrap(),
        );
        assert_eq!(
            p.errors(),
            vec![
                "Theme is not included in the list",
                "Homepage is not included in the list",
                "Light palette is not included in the list",
            ]
        );
        let none = Params::from(&Map::new());
        assert_eq!(
            none.errors(),
            vec!["Theme can't be blank", "Theme is not included in the list"]
        );
    }
}
