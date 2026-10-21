//! Stylesheet::Manager, Stylesheet::Importer and the theme side of
//! Stylesheet::Compiler: what a theme's stylesheets (`common_theme`,
//! `desktop_theme`, `mobile_theme`, `embedded_theme`) and the color
//! definitions compile from, compiled with [`super::scss`] and processed by
//! [`super::postcss`], and the link tags the pages draw for them.
//!
//! Compiles are kept by their whole input (source and theme files), as
//! Rails keeps them by digest in its stylesheet cache. The served CSS
//! leaves out Rails' source map annotation (the port serves no maps), and
//! the light-dark() toggles' hash is the port's own input's, as Rails'
//! is its input's (which names the stylesheet's digest).

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use serde_json::Value;
use sqlx::PgConnection;

use super::scss::{self, ThemeFiles};
use crate::config::Config;
use crate::site_settings::SiteSettings;
use crate::{AppError, Unsupported};

/// The targets of a theme's own stylesheets.
pub const THEME_TARGETS: [&str; 4] = [
    "common_theme",
    "desktop_theme",
    "mobile_theme",
    "embedded_theme",
];

/// What a compile reads beside the database.
pub struct Cx<'a> {
    pub config: &'a Config,
    pub settings: &'a SiteSettings,
}

impl Cx<'_> {
    fn urls(&self) -> crate::url::Urls<'_> {
        crate::url::Urls {
            config: self.config,
            settings: self.settings,
        }
    }

    /// `UrlHelper.absolute("#{Discourse.base_path}#{path}")`
    fn absolute(&self, path: &str) -> Result<String, AppError> {
        let base = self.config.globals.relative_url_root();
        Ok(self.urls().absolute(&format!("{base}{path}"))?)
    }
}

/// A theme's columns the stylesheets read.
#[derive(sqlx::FromRow, Debug, Clone)]
struct ThemeRow {
    id: i32,
    name: String,
    component: bool,
    color_scheme_id: Option<i32>,
    remote_theme_id: Option<i32>,
}

async fn theme_row(conn: &mut PgConnection, id: i32) -> Result<Option<ThemeRow>, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, name, component, color_scheme_id, remote_theme_id FROM themes WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(conn)
    .await
}

/// A theme's SCSS fields: the basic ones (scss, embedded_scss,
/// color_definitions of common, desktop and mobile) and extra_scss.
async fn scss_fields(
    conn: &mut PgConnection,
    theme_id: i32,
) -> Result<Vec<crate::themes::Field>, sqlx::Error> {
    Ok(crate::themes::fields(conn, theme_id)
        .await?
        .into_iter()
        .filter(|f| {
            f.type_id == 1
                && (f.target_id == 5
                    || ([0, 1, 2].contains(&f.target_id)
                        && ["scss", "embedded_scss", "color_definitions"]
                            .contains(&f.name.as_str())))
        })
        .collect())
}

/// `Theme#with_scss_load_paths`: the theme's SCSS fields at their export
/// paths, the entry points moved under `_entry_loadpath`.
pub async fn theme_files(conn: &mut PgConnection, theme_id: i32) -> Result<ThemeFiles, AppError> {
    let mut exported = Vec::new();
    for field in scss_fields(conn, theme_id).await? {
        if let Some(path) = crate::themes::field_file_path(conn, &field).await? {
            exported.push((path, field.value));
        }
    }
    Ok(ThemeFiles::from_exported(exported))
}

/// A setting's value as Ruby's `to_s` writes it into SCSS.
fn setting_to_s(type_name: &str, value: &Value) -> Result<String, Unsupported> {
    Ok(match value {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::String(s) => s.clone(),
        Value::Number(n) if type_name == "float" => {
            crate::ruby::float_to_s(n.as_f64().unwrap_or_default())
        }
        Value::Number(n) => n.to_string(),
        Value::Array(_) | Value::Object(_) => {
            return Err(Unsupported("an objects theme setting in SCSS (169a)"));
        }
    })
}

/// `Theme#scss_variables`: the theme's upload variables, then its
/// settings, as Sass variables; None without either.
pub async fn scss_variables(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    theme_id: i32,
) -> Result<Option<String>, AppError> {
    let uploads: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT f.name, u.url FROM theme_fields f LEFT JOIN uploads u ON u.id = f.upload_id \
         WHERE f.theme_id = $1 AND f.type_id = 2 ORDER BY f.id",
    )
    .bind(theme_id)
    .fetch_all(&mut *conn)
    .await?;
    let settings = crate::themes::settings_of(conn, theme_id).await?;
    if uploads.is_empty() && settings.is_empty() {
        return Ok(None);
    }
    let mut out = String::new();
    for (name, url) in uploads {
        let url = match url {
            Some(url) => cx.urls().upload_cdn_path(&url)?,
            None => String::new(),
        };
        out.push_str(&format!("${name}: unquote(\"{url}\");"));
    }
    let stored = crate::themes::stored_settings(conn, theme_id).await?;
    let store = crate::file_store::FileStore::for_site(cx.config, cx.settings)?;
    let vcx = crate::themes::ValueCx::new(&store, cx.settings)?;
    for setting in &settings {
        if [
            "theme_uploads",
            "theme_uploads_local",
            "theme_setting_type_info",
        ]
        .contains(&setting.name.as_str())
        {
            continue;
        }
        let value = crate::themes::setting_value(
            conn,
            &vcx,
            theme_id,
            setting,
            setting.row(&stored),
            crate::themes::Reading::Value,
        )
        .await?;
        out.push_str(&scss::to_scss_variable(
            &setting.name,
            &setting_to_s(setting.type_name, &value)?,
        ));
    }
    Ok(Some(out))
}

/// A color scheme's resolved colors, or the base ones when it is gone.
async fn scheme_colors(
    conn: &mut PgConnection,
    id: Option<i32>,
) -> Result<Vec<(String, String)>, AppError> {
    if let Some(id) = id
        && let Some(scheme) = crate::color_scheme::ColorScheme::find(conn, id).await?
    {
        return Ok(scheme.resolved_colors()?);
    }
    Ok(crate::color_scheme::base_colors().to_vec())
}

/// `Importer#color_variables`' colors for a theme compile: the theme's
/// own palette, a component's the default theme's (or the base colors).
async fn theme_colors(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    theme: Option<&ThemeRow>,
) -> Result<Vec<(String, String)>, AppError> {
    match theme {
        Some(t) if !t.component => scheme_colors(conn, t.color_scheme_id).await,
        _ => {
            let default = i32::try_from(cx.settings.get("default_theme_id")?.to_i()).unwrap_or(0);
            let scheme: Option<Option<i32>> =
                sqlx::query_scalar("SELECT color_scheme_id FROM themes WHERE id = $1")
                    .bind(default)
                    .fetch_optional(&mut *conn)
                    .await?;
            scheme_colors(conn, scheme.flatten()).await
        }
    }
}

fn prepended(colors: &[(String, String)], cx: &Cx<'_>) -> Result<String, AppError> {
    Ok(scss::prepended_scss(colors, &cx.absolute("/images")?))
}

/// Compiles kept by input.
static CACHE: LazyLock<Mutex<HashMap<String, Result<String, String>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Stylesheet::Compiler.compile: Sass, then post_css; the compile's error
/// message when Sass fails. `annotation` is the source map file Rails
/// names in the input post_css hashes.
fn compile(
    source: &str,
    theme: Option<&ThemeFiles>,
    annotation: &str,
) -> Result<Result<String, String>, Unsupported> {
    let mut key = String::from(source);
    if let Some(theme) = theme {
        for (path, text) in &theme.files {
            key.push('\0');
            key.push_str(path);
            key.push('\0');
            key.push_str(text);
        }
    }
    key.push('\0');
    key.push_str(annotation);
    if let Some(hit) = CACHE.lock().expect("the stylesheet cache").get(&key) {
        return Ok(hit.clone());
    }
    let result = match scss::compile(source, theme) {
        Ok(css) => {
            let input = if css.is_empty() {
                format!("\n/*# sourceMappingURL={annotation} */")
            } else {
                format!("{css}\n\n/*# sourceMappingURL={annotation} */")
            };
            Ok(super::postcss::post_css(&css, &input)?)
        }
        Err(e) => Err(e),
    };
    let mut cache = CACHE.lock().expect("the stylesheet cache");
    if cache.len() > 256 {
        cache.clear();
    }
    cache.insert(key, result.clone());
    Ok(result)
}

/// Theme.transform_ids: the theme and its enabled components.
pub async fn theme_ids(conn: &mut PgConnection, theme_id: i32) -> Result<Vec<i32>, AppError> {
    crate::svg_sprite::transform_ids(conn, Some(i64::from(theme_id))).await
}

/// `ThemeField#scss_entrypoint_name`
fn entrypoint(target_id: i32, name: &str) -> Option<&'static str> {
    match (target_id, name) {
        (0, "scss") => Some("common"),
        (1, "scss") => Some("desktop"),
        (2, "scss") => Some("mobile"),
        (0, "color_definitions") => Some("color_definitions"),
        (0, "embedded_scss") => Some("embedded"),
        _ => None,
    }
}

/// The field a theme target compiles (`list_baked_fields`), if the theme
/// has one.
fn target_field(target: &str) -> Option<(i32, &'static str)> {
    match target {
        "common_theme" => Some((0, "scss")),
        "desktop_theme" => Some((1, "scss")),
        "mobile_theme" => Some((2, "scss")),
        "embedded_theme" => Some((0, "embedded_scss")),
        _ => None,
    }
}

/// A theme's stylesheet for a target (`compile_asset` with its
/// variables and entry point), None when the theme has no SCSS for it.
/// A compile error is the comment Rails serves in its place.
pub async fn theme_css(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    theme_id: i32,
    target: &str,
) -> Result<Option<String>, AppError> {
    let Some((target_id, name)) = target_field(target) else {
        return Ok(None);
    };
    let fields = scss_fields(conn, theme_id).await?;
    if !fields
        .iter()
        .any(|f| f.target_id == target_id && f.name == name)
    {
        return Ok(None);
    }
    let theme = theme_row(conn, theme_id).await?;
    let colors = theme_colors(conn, cx, theme.as_ref()).await?;
    let mut source = prepended(&colors, cx)?;
    source.push_str(
        &scss_variables(conn, cx, theme_id)
            .await?
            .unwrap_or_default(),
    );
    for field in fields
        .iter()
        .filter(|f| f.target_id == target_id && f.name == name)
    {
        if let Some(entry) = entrypoint(field.target_id, &field.name) {
            source.push_str(&format!("@import \"theme-entrypoint/{entry}\";\n"));
        }
    }
    let files = theme_files(conn, theme_id).await?;
    let annotation = format!("{target}_{theme_id}.css.map");
    Ok(Some(match compile(&source, Some(&files), &annotation)? {
        Ok(css) => css,
        Err(e) => format!("/* SCSS compilation error: {e} */"),
    }))
}

/// `ThemeField#compile_scss` with the default importer's prepended SCSS:
/// the error a field's save records (`ensure_scss_compiles!`).
pub async fn field_compile_error(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    theme_id: i32,
    target_id: i32,
    name: &str,
) -> Result<Option<String>, AppError> {
    let Some(entry) = entrypoint(target_id, name) else {
        return Ok(None);
    };
    let colors = theme_colors(conn, cx, None).await?;
    let source = format!(
        "{}\n{}\n@import \"theme-entrypoint/{entry}\";\n",
        prepended(&colors, cx)?,
        scss_variables(conn, cx, theme_id)
            .await?
            .unwrap_or_default()
    );
    let files = theme_files(conn, theme_id).await?;
    let target = crate::themes::target_name(target_id).unwrap_or("common");
    Ok(compile(&source, Some(&files), &format!("{target}.css.map"))?.err())
}

/// The color definitions of a scheme (None: the theme's own, else the
/// base palette) under a theme: Discourse's, each plugin's, each
/// color_definitions field of the theme and its components (compiled
/// with the scheme's colors, a failure as Rails' comment), the WCAG
/// overrides for a WCAG palette, and the fonts.
pub async fn color_definitions(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    theme_id: Option<i32>,
    scheme_id: Option<i32>,
) -> Result<String, AppError> {
    let scheme = match scheme_id {
        Some(id) => crate::color_scheme::ColorScheme::find(conn, id).await?,
        None => None,
    };
    let colors = scheme_colors(conn, scheme.as_ref().map(|s| s.id)).await?;
    let prepended_scss = prepended(&colors, cx)?;
    let mut source = format!("{prepended_scss}@import \"color_definitions\";\n");
    source.push_str(&scss::plugin_color_definitions());
    let theme_id = match theme_id {
        Some(id) => id,
        None => i32::try_from(cx.settings.get("default_theme_id")?.to_i()).unwrap_or(0),
    };
    let ids = theme_ids(conn, theme_id).await?;
    for id in &ids {
        let has_field: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM theme_fields WHERE theme_id = $1 AND target_id = 0 \
             AND name = 'color_definitions')",
        )
        .bind(id)
        .fetch_one(&mut *conn)
        .await?;
        if !has_field {
            continue;
        }
        let name: String = sqlx::query_scalar("SELECT name FROM themes WHERE id = $1")
            .bind(id)
            .fetch_one(&mut *conn)
            .await?;
        let field_source = format!(
            "{prepended_scss}\n{}\n@import \"theme-entrypoint/color_definitions\";\n",
            scss_variables(conn, cx, *id).await?.unwrap_or_default()
        );
        let files = theme_files(conn, *id).await?;
        let css = match compile(&field_source, Some(&files), "common.css.map")? {
            Ok(css) => css,
            Err(e) => format!("/* SCSS compilation error: {e} */"),
        };
        source.push_str(&format!(
            "\n\n// Color definitions from {name}\n\n{css}\n\n"
        ));
    }
    if let Some(s) = &scheme
        && matches!(s.base_scheme_id, Some(-9) | Some(-10))
    {
        source.push_str("@import \"wcag\";");
    }
    let setting = |name| {
        cx.settings
            .get(name)
            .map(|v| v.to_s().to_string())
            .unwrap_or_default()
    };
    source.push_str(&super::fonts::css(
        &setting("base_font"),
        &setting("heading_font"),
        &cx.absolute("/fonts")?,
    ));
    let files = theme_files(conn, theme_id).await?;
    let annotation = format!(
        "color_definitions_{}_{theme_id}.css.map",
        scheme.as_ref().map_or(-1, |s| s.id)
    );
    // A failure is logged and served as a comment, as in production.
    Ok(match compile(&source, Some(&files), &annotation)? {
        Ok(css) => css,
        Err(e) => {
            tracing::error!("SCSS compilation error: {e}");
            format!("/* SCSS compilation error: {e} */")
        }
    })
}

/// The theme stylesheets a page links (`discourse_stylesheet_link_tag`
/// for common_theme, mobile_theme and desktop_theme): one per theme of
/// `theme_id` with SCSS for the target.
pub async fn link_tags(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    theme_id: i32,
) -> Result<String, AppError> {
    let ids = theme_ids(conn, theme_id).await?;
    let host = cx.urls().current_hostname()?;
    let root = cx.config.globals.relative_url_root();
    let mut out = Vec::new();
    for (target, media) in [
        ("common_theme", "all"),
        ("mobile_theme", "(width < 40rem)"),
        ("desktop_theme", "(width >= 40rem)"),
    ] {
        let mut sheets = Vec::new();
        for id in &ids {
            let Some(css) = theme_css(conn, cx, *id, target).await? else {
                continue;
            };
            let Some(theme) = theme_row(conn, *id).await? else {
                continue;
            };
            let digest = crate::assets::digest_of(css.as_bytes());
            sheets.push((theme, digest));
        }
        // Remote themes first, the manager's own theme last, then by name.
        sheets.sort_by_key(|(t, _)| {
            (
                t.remote_theme_id.is_none(),
                t.id == theme_id,
                t.name.to_lowercase(),
            )
        });
        for (theme, digest) in sheets {
            out.push(format!(
                "<link href=\"{root}/stylesheets/{target}_{}_{digest}.css?__ws={host}\" media=\"{media}\" rel=\"stylesheet\" data-target=\"{target}\" data-theme-id=\"{}\" data-theme-name=\"{}\"/>",
                theme.id,
                theme.id,
                crate::admin_site_settings::escape_html(&theme.name.to_lowercase())
            ));
        }
    }
    Ok(out.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_write_as_ruby_to_s() {
        assert_eq!(setting_to_s("bool", &Value::Bool(true)).unwrap(), "true");
        assert_eq!(setting_to_s("integer", &serde_json::json!(5)).unwrap(), "5");
        assert_eq!(
            setting_to_s("float", &serde_json::json!(1.0)).unwrap(),
            "1.0"
        );
        assert_eq!(setting_to_s("upload", &Value::Null).unwrap(), "");
        assert!(setting_to_s("objects", &serde_json::json!([{"a": 1}])).is_err());
    }
}
