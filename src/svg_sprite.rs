//! SvgSprite (lib/svg_sprite.rb): the icons a site's sprite holds
//! (`all_icons`: core's SVG_ICONS, the icons its site settings, badges,
//! groups, plugins and theme name, and the custom sprites' own), the
//! sprite itself (`bundle`) with its version, served as
//! `/svg-sprite/:hostname/svg-:theme_id-:version.js`, the icon picker's
//! search, and `raw_svg` for the server-rendered pages.
//!
//! What SvgSprite reads from files rather than the database is recorded
//! from the reference by scripts/record-svg-sprite into
//! vendor/discourse/config/svg_sprite.json: the core symbols as
//! `symbols_for` serializes them, the bundled plugins' symbols, SVG_ICONS
//! and the icons the plugins register. The Font Awesome icons are CC BY
//! 4.0.
//!
//! Each site's sprite is kept in process per theme (for_site), checked
//! against one fingerprint of what it is built from on each page.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::Value;
use sqlx::PgConnection;

use crate::site_settings::{DataType, Definitions, SiteSettings};
use crate::{AppError, Unsupported};

const RECORDED: &str = include_str!("../vendor/discourse/config/svg_sprite.json");

struct Recorded {
    /// `core_svgs`: icon id to its symbol.
    core: HashMap<String, String>,
    /// `plugin_svgs`, in their order.
    plugin: Vec<(String, String)>,
    /// SVG_ICONS
    svg_icons: Vec<String>,
    /// DiscoursePluginRegistry.svg_icons
    plugin_icons: Vec<String>,
}

fn recorded() -> &'static Recorded {
    static RECORD: OnceLock<Recorded> = OnceLock::new();
    RECORD.get_or_init(|| {
        let json: Value = serde_json::from_str(RECORDED).expect("svg_sprite.json");
        let pairs = |key: &str| -> Vec<(String, String)> {
            json[key]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|p| Some((p[0].as_str()?.to_string(), p[1].as_str()?.to_string())))
                .collect()
        };
        let names = |key: &str| -> Vec<String> {
            json[key]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        };
        Recorded {
            core: pairs("core").into_iter().collect(),
            plugin: pairs("plugin"),
            svg_icons: names("svg_icons"),
            plugin_icons: names("plugin_icons"),
        }
    })
}

/// `svgs_for(theme_id)[name]` with a theme: a custom symbol over core's.
fn symbol(name: &str) -> Option<&'static str> {
    let r = recorded();
    r.plugin
        .iter()
        .rev()
        .find(|(id, _)| id == name)
        .map(|(_, s)| s.as_str())
        .or_else(|| r.core.get(name).map(String::as_str))
}

/// `Nokogiri.XML(symbol).children.first` renamed to `svg`.
fn as_svg(symbol: &str) -> String {
    let body = symbol.strip_prefix("<symbol").unwrap_or(symbol);
    let body = body.strip_suffix("</symbol>").unwrap_or(body);
    format!("<svg{body}</svg>")
}

/// `SvgSprite.raw_svg(name)`: "" for an unknown icon, as Rails answers.
/// It searches the default theme's symbols; a theme's own sprite (an
/// `icons-sprite` upload) is not among them here.
pub fn raw_svg(name: &str) -> Result<String, Unsupported> {
    match symbol(name.trim()) {
        Some(symbol) => Ok(format!(
            "<svg class=\"fa d-icon svg-icon svg-node\" aria-hidden=\"true\">{}</svg>\n",
            as_svg(symbol)
        )),
        None => Ok(String::new()),
    }
}

/// `SvgSprite.search(name)`: the symbol, for /svg-sprite/search.
pub fn search(name: &str) -> Option<&'static str> {
    symbol(name.trim())
}

/// `Theme.transform_ids(id)`: a parent theme with its components (sorted),
/// less those disabled or unsupported; none for a blank id.
pub async fn transform_ids(conn: &mut PgConnection, id: Option<i64>) -> Result<Vec<i32>, AppError> {
    let Some(id) = id.and_then(|id| i32::try_from(id).ok()) else {
        return Ok(Vec::new());
    };
    let parent: Option<bool> = sqlx::query_scalar("SELECT NOT component FROM themes WHERE id = $1")
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?;
    let mut ids = vec![id];
    if parent == Some(true) {
        let mut components: Vec<i32> = sqlx::query_scalar(
            "SELECT child_theme_id FROM child_themes WHERE parent_theme_id = $1",
        )
        .bind(id)
        .fetch_all(&mut *conn)
        .await?;
        components.sort_unstable();
        components.dedup();
        ids.extend(components);
    }
    let mut kept = Vec::with_capacity(ids.len());
    for theme_id in ids {
        let Some(theme) = crate::themes::Theme::find(conn, theme_id).await? else {
            // Not a theme: Rails keeps the id (nothing to disable).
            kept.push(theme_id);
            continue;
        };
        if theme.enabled && crate::themes::supported_theme(conn, &theme).await? {
            kept.push(theme_id);
        }
    }
    Ok(kept)
}

/// `settings_icons`: the values of the settings named `*_icon*` or typed
/// icon, split on `|`, and the icon properties of objects settings.
fn settings_icons(defs: &Definitions, settings: &SiteSettings) -> Vec<String> {
    let mut out = Vec::new();
    for def in defs.iter() {
        let Ok(crate::site_settings::Value::Str(value)) = settings.get(&def.name) else {
            continue;
        };
        if def.name.contains("_icon") || def.data_type == DataType::Icon {
            out.extend(value.split('|').map(str::to_string));
        } else if def.data_type == DataType::Objects && !value.is_empty() {
            let schema = def
                .options
                .as_ref()
                .and_then(|o| o.get("schema"))
                .map(crate::themes::yaml_json);
            if let (Some(schema), Ok(Value::Array(objects))) =
                (schema, serde_json::from_str::<Value>(value))
            {
                out.extend(icon_strings(&schema, &objects));
            }
        }
    }
    out
}

/// `objects_setting_icons`: the icon properties' String values.
fn icon_strings(schema: &Value, objects: &[Value]) -> Vec<String> {
    crate::schema_objects::property_values_of_type(schema, objects, "icon")
        .into_iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect()
}

/// `theme_icons(theme_ids)`: the themes' settings named `*_icon*` or typed
/// icon, their objects settings' icon properties, and their modifiers'
/// svg_icons.
async fn theme_icons(
    conn: &mut PgConnection,
    vcx: &crate::themes::ValueCx<'_>,
    theme_ids: &[i32],
) -> Result<Vec<String>, AppError> {
    use crate::themes::{Reading, setting_value, settings_of, stored_settings};
    let mut out = Vec::new();
    for &theme_id in theme_ids {
        let settings = settings_of(conn, theme_id).await?;
        let stored = stored_settings(conn, theme_id).await?;
        for s in &settings {
            let named = s.name.contains("_icon") || s.type_name == "icon";
            if !named && s.type_name != "objects" {
                continue;
            }
            let value =
                setting_value(conn, vcx, theme_id, s, s.row(&stored), Reading::Value).await?;
            match value {
                Value::String(v) if named => out.extend(v.split('|').map(str::to_string)),
                Value::Array(objects) if s.type_name == "objects" => {
                    out.extend(icon_strings(&s.schema(), &objects));
                }
                _ => {}
            }
        }
    }
    // ThemeModifierHelper#svg_icons: the sets' arrays, flattened.
    let modifiers: Vec<Vec<String>> = sqlx::query_scalar(
        "SELECT svg_icons FROM theme_modifier_sets \
         WHERE theme_id = ANY($1) AND svg_icons IS NOT NULL",
    )
    .bind(theme_ids)
    .fetch_all(&mut *conn)
    .await?;
    out.extend(modifiers.into_iter().flatten());
    Ok(out)
}

/// The site's sprite for a theme (`theme_id` None for none), kept per
/// site and theme as SvgSprite keeps it in its DistributedCache, and
/// built again only when what it is built from has changed
/// (crate::theme_cache). Rails expires that cache
/// (SvgSprite.expire_cache) when a badge, group, theme, theme field,
/// theme modifier set, theme sprite or Boards column is saved, and when an
/// icon setting changes.
pub async fn for_site(
    conn: &mut PgConnection,
    state: &crate::AppState,
    settings: &SiteSettings,
    theme_id: Option<i64>,
) -> Result<Arc<Built>, AppError> {
    let at = crate::theme_cache::read(&mut *conn).await?;
    for_site_at(conn, state, settings, theme_id, &at).await
}

/// [`for_site`] at the fingerprint the request already read.
pub async fn for_site_at(
    conn: &mut PgConnection,
    state: &crate::AppState,
    settings: &SiteSettings,
    theme_id: Option<i64>,
    at: &crate::theme_cache::Fingerprint,
) -> Result<Arc<Built>, AppError> {
    static KEPT: crate::theme_cache::Kept<Option<i64>, Arc<Built>> =
        crate::theme_cache::Kept::new();
    if let Some(built) = KEPT.get(at, theme_id) {
        return Ok(built);
    }
    let store = crate::file_store::FileStore::for_site(&state.config, settings)?;
    let inputs = inputs(conn, &state.site_setting_defs, settings, &store, theme_id).await?;
    let built = build(&inputs);
    KEPT.put(at, theme_id, built.clone());
    Ok(built)
}

/// What a site's sprite is built from for a theme.
pub struct Inputs {
    /// `all_icons(theme_id)`, sorted.
    pub icons: Vec<String>,
    /// Whether a theme was given (`svgs_for` adds the custom symbols then).
    pub themed: bool,
    /// `custom_svgs(theme_id)`: the plugins' symbols, then the themes'.
    pub custom: Vec<(String, String)>,
}

/// `theme_svgs(theme_ids)`: each theme's own sprite (its `icons-sprite`
/// upload, kept in theme_svg_sprites), in the themes' order; one that
/// doesn't parse is left out, as Rails logs and skips it.
async fn theme_svgs(
    conn: &mut PgConnection,
    theme_ids: &[i32],
) -> Result<Vec<(String, String)>, AppError> {
    if theme_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows: Vec<(i32, Vec<u8>)> =
        sqlx::query_as("SELECT theme_id, sprite FROM theme_svg_sprites WHERE theme_id = ANY($1)")
            .bind(theme_ids)
            .fetch_all(&mut *conn)
            .await?;
    let mut out = Vec::new();
    for id in theme_ids {
        // `.to_h`: the last row of a theme wins.
        if let Some((_, sprite)) = rows.iter().rev().find(|(t, _)| t == id) {
            match crate::svg_sprite_xml::symbols(&String::from_utf8_lossy(sprite)) {
                Some(symbols) => out.extend(symbols),
                None => tracing::warn!("Bad XML in custom sprite in theme with ID={id}"),
            }
        }
    }
    Ok(out)
}

/// `a.merge(b)` over (id, symbol) lists: a later id replaces an earlier
/// one in its place.
fn merge(lists: &[&[(String, String)]]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for list in lists {
        for (id, symbol) in list.iter() {
            match out.iter_mut().find(|(i, _)| i == id) {
                Some(slot) => slot.1 = symbol.clone(),
                None => out.push((id.clone(), symbol.clone())),
            }
        }
    }
    out
}

/// `all_icons(theme_id)` for the site, with the custom symbols.
pub async fn inputs(
    conn: &mut PgConnection,
    defs: &Definitions,
    settings: &SiteSettings,
    store: &crate::file_store::FileStore,
    theme_id: Option<i64>,
) -> Result<Inputs, AppError> {
    let theme_ids = transform_ids(conn, theme_id).await?;
    let r = recorded();
    let custom = merge(&[&r.plugin, &theme_svgs(conn, &theme_ids).await?]);
    let mut icons: Vec<String> = settings_icons(defs, settings);
    icons.extend(r.plugin_icons.iter().cloned());
    // plugin_icon_sources: the enabled plugins' blocks, Boards' columns.
    if settings.get("boards_enabled").is_ok_and(|v| v.truthy()) {
        let columns: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT icon FROM discourse_kanban_columns \
             WHERE icon IS NOT NULL AND icon <> ''",
        )
        .fetch_all(&mut *conn)
        .await?;
        icons.extend(columns);
    }
    let named: Vec<Option<String>> =
        sqlx::query_scalar("SELECT icon FROM badges UNION SELECT flair_icon FROM groups")
            .fetch_all(&mut *conn)
            .await?;
    icons.extend(named.into_iter().flatten());
    let vcx = crate::themes::ValueCx::new(store, settings)?;
    icons.extend(theme_icons(conn, &vcx, &theme_ids).await?);
    // custom_icons: the custom symbols' ids.
    icons.extend(custom.iter().map(|(id, _)| id.clone()));
    let mut set: BTreeSet<String> = icons
        .into_iter()
        .filter(|i| !crate::ruby::is_blank(i) && !i.contains('/'))
        .map(|i| crate::ruby::strip(&i).to_string())
        .collect();
    set.extend(r.svg_icons.iter().cloned());
    Ok(Inputs {
        icons: set.into_iter().collect(),
        themed: theme_id.is_some(),
        custom,
    })
}

/// A built sprite.
pub struct Built {
    /// `bundle`
    pub bundle: String,
    /// `version`: the bundle's SHA1.
    pub version: String,
    /// `picker_icon_ids(theme_id, true)` and `(theme_id, false)`.
    pub available: Vec<String>,
    pub all: Vec<String>,
    /// The custom symbols the picker hands out (none without a theme).
    custom: Vec<(String, String)>,
}

type Key = (Vec<String>, bool, Vec<(String, String)>);

/// The sprite for these inputs, built once.
pub fn build(inputs: &Inputs) -> Arc<Built> {
    static BUILT: OnceLock<Mutex<HashMap<Key, Arc<Built>>>> = OnceLock::new();
    let cache = BUILT.get_or_init(Default::default);
    let key = (inputs.icons.clone(), inputs.themed, inputs.custom.clone());
    if let Some(built) = cache.lock().expect("sprite cache").get(&key) {
        return built.clone();
    }
    let r = recorded();
    let mut bundle = String::from(
        "<!--\nDiscourse SVG subset of Font Awesome Free by @fontawesome - https://fontawesome.com\nLicense - https://fontawesome.com/license/free (Icons: CC BY 4.0, Fonts: SIL OFL 1.1, Code: MIT License)\n-->\n<svg xmlns='http://www.w3.org/2000/svg' style='display: none;'>\n",
    );
    // core_svgs.slice(*icons), then the custom symbols.
    for icon in &inputs.icons {
        if let Some(symbol) = r.core.get(icon) {
            bundle.push_str(symbol);
        }
    }
    for (_, symbol) in &inputs.custom {
        bundle.push_str(symbol);
    }
    bundle.push_str("</svg>");
    let version = {
        use sha1::{Digest, Sha1};
        format!("{:x}", Sha1::digest(bundle.as_bytes()))
    };
    // svgs_for: core, with the custom symbols when themed.
    let mut symbols: HashSet<&str> = r.core.keys().map(String::as_str).collect();
    if inputs.themed {
        symbols.extend(inputs.custom.iter().map(|(id, _)| id.as_str()));
    }
    let available: Vec<String> = inputs
        .icons
        .iter()
        .filter(|i| symbols.contains(i.as_str()))
        .cloned()
        .collect();
    let in_sprite: HashSet<&str> = available.iter().map(String::as_str).collect();
    let mut rest: Vec<&str> = symbols.difference(&in_sprite).copied().collect();
    rest.sort_unstable();
    let mut all = available.clone();
    all.extend(rest.into_iter().map(str::to_string));
    let built = Arc::new(Built {
        bundle,
        version,
        available,
        all,
        custom: if inputs.themed {
            inputs.custom.clone()
        } else {
            Vec::new()
        },
    });
    let mut guard = cache.lock().expect("sprite cache");
    if guard.len() >= 32 {
        guard.clear();
    }
    guard.insert(key, built.clone());
    built
}

/// `SvgSprite.path(theme_id)`
pub fn path(hostname: &str, theme_id: Option<i64>, version: &str) -> String {
    format!(
        "/svg-sprite/{hostname}/svg-{}-{version}.js",
        theme_id.map(|t| t.to_string()).unwrap_or_default()
    )
}

/// The sprite's script: `window.__svg_sprite = <bundle.inspect>;`.
pub fn script(built: &Built) -> String {
    format!(
        "window.__svg_sprite = {};",
        crate::ruby::inspect_string(&built.bundle)
    )
}

/// `icon_picker_search(keyword, only_available, page:, per_page:)`.
pub fn picker_search(built: &Built, keyword: &str, only_available: bool, page: usize) -> Value {
    const PER_PAGE: usize = 100;
    let ids = if only_available {
        &built.available
    } else {
        &built.all
    };
    let keyword = keyword.to_lowercase();
    let matching: Vec<&String> = ids
        .iter()
        .filter(|id| keyword.is_empty() || id.to_lowercase().contains(&keyword))
        .skip(page * PER_PAGE)
        .take(PER_PAGE + 1)
        .collect();
    let has_more = matching.len() > PER_PAGE;
    let page_ids = &matching[..matching.len().min(PER_PAGE)];
    let available: HashSet<&str> = built.available.iter().map(String::as_str).collect();
    let icons: Vec<Value> = page_ids
        .iter()
        .map(|id| {
            if only_available || available.contains(id.as_str()) {
                serde_json::json!({ "id": id })
            } else {
                let symbol = built
                    .custom
                    .iter()
                    .find(|(i, _)| i == *id)
                    .map(|(_, s)| s.as_str())
                    .or_else(|| recorded().core.get(id.as_str()).map(String::as_str));
                serde_json::json!({ "id": id, "symbol": symbol })
            }
        })
        .collect();
    serde_json::json!({ "icons": icons, "has_more": has_more })
}

/// `svg_icon`'s document: the symbol as a standalone SVG, filled when a
/// color is given.
pub fn icon_document(symbol: &str, color: Option<&str>) -> String {
    let svg = as_svg(symbol);
    let end = svg.find('>').unwrap_or(svg.len());
    let mut attrs = String::from(" xmlns=\"http://www.w3.org/2000/svg\"");
    if let Some(color) = color {
        // adjust_hex: a 3-digit hex doubled.
        let hex = if color.len() == 3 {
            color.chars().flat_map(|c| [c, c]).collect()
        } else {
            color.to_string()
        };
        attrs.push_str(&format!(" fill=\"#{hex}\""));
    }
    format!(
        "<?xml version=\"1.0\"?>\n{}{attrs}{}\n",
        &svg[..end],
        &svg[end..]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn house_is_inlined_as_rails_prints_it() {
        let svg = raw_svg("house").unwrap();
        assert!(svg.starts_with(
            "<svg class=\"fa d-icon svg-icon svg-node\" aria-hidden=\"true\"><svg id=\"house\" viewBox=\"0 0 576 512\">\n  <path d=\"M575.8 255.5"
        ));
        assert!(svg.ends_with("z\"/>\n</svg></svg>\n"));
    }

    #[test]
    fn prefixes_custom_and_unknown_icons() {
        assert!(
            raw_svg("far-image")
                .unwrap()
                .contains("<svg id=\"far-image\"")
        );
        assert!(
            raw_svg("fab-github")
                .unwrap()
                .contains("<svg id=\"fab-github\"")
        );
        assert!(
            raw_svg("discourse-bell-slash")
                .unwrap()
                .contains("<svg id=\"discourse-bell-slash\"")
        );
        assert!(
            raw_svg("discobot")
                .unwrap()
                .contains("<svg id=\"discobot\"")
        );
        assert_eq!(raw_svg("no-such-icon").unwrap(), "");
    }

    #[test]
    fn the_bundle_is_the_icons_then_the_custom_symbols() {
        let built = build(&Inputs {
            icons: vec!["bars".into(), "discobot".into(), "no-such-icon".into()],
            themed: true,
            custom: merge(&[&recorded().plugin]),
        });
        assert!(built.bundle.starts_with("<!--\nDiscourse SVG subset"));
        let bars = built.bundle.find("<symbol id=\"bars\"").unwrap();
        let discobot = built.bundle.find("<symbol id=\"discobot\"").unwrap();
        assert!(bars < discobot);
        assert!(built.bundle.ends_with("</symbol></svg>"));
        assert_eq!(built.version.len(), 40);
        assert_eq!(built.available, vec!["bars", "discobot"]);
        assert!(built.all.len() > 2000);
        let page = picker_search(&built, "", false, 0);
        assert_eq!(page["icons"][0]["id"], "bars");
        assert!(page["icons"][2]["symbol"].is_string());
        assert_eq!(page["has_more"], true);
    }

    #[test]
    fn a_standalone_icon() {
        let doc = icon_document(search("house").unwrap(), Some("f0a"));
        assert!(doc.starts_with(
            "<?xml version=\"1.0\"?>\n<svg id=\"house\" viewBox=\"0 0 576 512\" xmlns=\"http://www.w3.org/2000/svg\" fill=\"#ff00aa\">"
        ));
    }
}
