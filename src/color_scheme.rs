//! Port of ColorScheme.hex_for_name (app/models/color_scheme.rb).

use std::collections::HashMap;
use std::sync::LazyLock;

use sqlx::PgConnection;

use crate::site_settings::{SettingError, SiteSettings};

const BASE_COLORS_SCSS: &str =
    include_str!("../vendor/discourse/app/assets/stylesheets/common/foundation/colors.scss");

/// `ColorScheme.base_colors`: `$name: #hex` pairs from colors.scss, hex kept
/// exactly as written (3 or 6 digits, no `#`).
static BASE_COLORS: LazyLock<HashMap<String, String>> =
    LazyLock::new(|| parse_base_colors(BASE_COLORS_SCSS));

/// Per stripped line, the first match of
/// `/\$([\w]+):\s*#([0-9a-fA-F]{3}|[0-9a-fA-F]{6})(?:[;]|\s)/`.
fn parse_base_colors(scss: &str) -> HashMap<String, String> {
    let mut colors = HashMap::new();
    for line in scss.lines() {
        let line = line.trim();
        let mut rest = line;
        while let Some(pos) = rest.find('$') {
            rest = &rest[pos + 1..];
            if let Some((name, hex)) = match_color(rest) {
                colors.insert(name.to_string(), hex.to_string());
                break;
            }
        }
    }
    colors
}

fn match_color(s: &str) -> Option<(&str, &str)> {
    let name_len = s
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(s.len());
    if name_len == 0 {
        return None;
    }
    let (name, rest) = s.split_at(name_len);
    let rest = rest.strip_prefix(':')?.trim_start().strip_prefix('#')?;
    let hex_len = rest
        .find(|c: char| !c.is_ascii_hexdigit())
        .unwrap_or(rest.len());
    // The regex alternation tries 3 digits first, then 6; each must be
    // followed by `;` or whitespace.
    for len in [3, 6] {
        if hex_len >= len {
            let next = rest[len..].chars().next();
            if matches!(next, Some(c) if c == ';' || c.is_whitespace()) {
                return Some((name, &rest[..len]));
            }
        }
    }
    None
}

/// `ColorScheme.hex_for_name(name)` with no scheme id: the default theme's
/// color scheme (ignoring remote copies), else the base colors. A scheme
/// that lacks the color yields None, it does not fall back to base.
pub async fn hex_for_name(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    name: &str,
) -> Result<Option<String>, HexError> {
    let default_theme_id = settings.get("default_theme_id")?.to_i();
    let scheme_id: Option<i32> = match i32::try_from(default_theme_id) {
        Ok(theme_id) => {
            sqlx::query_scalar(
                "SELECT cs.id FROM themes t \
             JOIN color_schemes cs ON cs.id = t.color_scheme_id AND NOT cs.remote_copy \
             WHERE t.id = $1",
            )
            .bind(theme_id)
            .fetch_optional(&mut *conn)
            .await?
        }
        Err(_) => None,
    };

    match scheme_id {
        Some(id) => Ok(sqlx::query_scalar(
            "SELECT hex FROM color_scheme_colors \
             WHERE color_scheme_id = $1 AND name = $2 ORDER BY id LIMIT 1",
        )
        .bind(id)
        .bind(name)
        .fetch_optional(conn)
        .await?),
        None => Ok(BASE_COLORS.get(name).cloned()),
    }
}

#[derive(Debug)]
pub enum HexError {
    Db(sqlx::Error),
    Setting(SettingError),
}

impl std::fmt::Display for HexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HexError::Db(e) => write!(f, "loading color scheme: {e}"),
            HexError::Setting(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for HexError {}

impl From<sqlx::Error> for HexError {
    fn from(e: sqlx::Error) -> Self {
        HexError::Db(e)
    }
}

impl From<SettingError> for HexError {
    fn from(e: SettingError) -> Self {
        HexError::Setting(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_vendored_base_colors_verbatim() {
        assert_eq!(
            BASE_COLORS.get("header_background").map(String::as_str),
            Some("fff")
        );
        assert_eq!(
            BASE_COLORS.get("header_primary").map(String::as_str),
            Some("333")
        );
        assert_eq!(
            BASE_COLORS.get("highlight").map(String::as_str),
            Some("ffff4d")
        );
        assert_eq!(
            BASE_COLORS.get("quaternary").map(String::as_str),
            Some("e45735")
        );
    }

    #[test]
    fn follows_the_regex_edge_cases() {
        let c = parse_base_colors(
            "$a: #abc;\n$b:#ABCDEF !default;\n$c: #abcd;\n$d: #12;\n  $e: #fff\n$f: red;\n$g: #aabbcc;",
        );
        assert_eq!(c.get("a").map(String::as_str), Some("abc"));
        assert_eq!(c.get("b").map(String::as_str), Some("ABCDEF"));
        assert_eq!(c.get("c"), None);
        assert_eq!(c.get("d"), None);
        // Stripped line ends right after the hex: no `;` or whitespace follows.
        assert_eq!(c.get("e"), None);
        assert_eq!(c.get("f"), None);
        assert_eq!(c.get("g").map(String::as_str), Some("aabbcc"));
    }
}

// ---- ColorScheme records and ColorSchemeSerializer ----------------------

/// `ColorScheme::NAMES_TO_ID_MAP`
const NAMES_TO_ID_MAP: &[(&str, i32)] = &[
    ("Light", -1),
    ("Dark", -2),
    ("Neutral", -3),
    ("Grey Amber", -4),
    ("Shades of Blue", -5),
    ("Latte", -6),
    ("Summer", -7),
    ("Dark Rose", -8),
    ("WCAG", -9),
    ("WCAG Dark", -10),
    ("Dracula", -11),
    ("Solarized Light", -12),
    ("Solarized Dark", -13),
];

/// `ColorScheme::LIGHT_PALETTE_NAME`
const LIGHT_PALETTE_NAME: &str = "Light";

/// `ColorScheme::COLORS_ORDER`
const COLORS_ORDER: &[&str] = &[
    "primary",
    "secondary",
    "tertiary",
    "quaternary",
    "header_background",
    "header_primary",
    "selected",
    "hover",
    "highlight",
    "danger",
    "success",
    "love",
];

/// `ColorScheme::BUILT_IN_SCHEMES`, generated from app/models/color_scheme.rb.
const BUILT_IN_SCHEMES: &[(&str, &[(&str, &str)])] = &[
    (
        "Dark",
        &[
            ("primary", "dddddd"),
            ("secondary", "222222"),
            ("tertiary", "099dd7"),
            ("quaternary", "c14924"),
            ("header_background", "111111"),
            ("header_primary", "dddddd"),
            ("highlight", "a87137"),
            ("selected", "052e3d"),
            ("hover", "313131"),
            ("danger", "e45735"),
            ("success", "1ca551"),
            ("love", "fa6c8d"),
        ],
    ),
    (
        "Neutral",
        &[
            ("primary", "000000"),
            ("secondary", "ffffff"),
            ("tertiary", "51839b"),
            ("quaternary", "b85e48"),
            ("header_background", "333333"),
            ("header_primary", "f3f3f3"),
            ("highlight", "ecec70"),
            ("selected", "e6e6e6"),
            ("hover", "f0f0f0"),
            ("danger", "b85e48"),
            ("success", "518751"),
            ("love", "fa6c8d"),
        ],
    ),
    (
        "Grey Amber",
        &[
            ("primary", "d9d9d9"),
            ("secondary", "3d4147"),
            ("tertiary", "fdd459"),
            ("quaternary", "fdd459"),
            ("header_background", "36393e"),
            ("header_primary", "d9d9d9"),
            ("highlight", "fdd459"),
            ("selected", "272727"),
            ("hover", "2F2F30"),
            ("danger", "e45735"),
            ("success", "fdd459"),
            ("love", "fdd459"),
        ],
    ),
    (
        "Shades of Blue",
        &[
            ("primary", "203243"),
            ("secondary", "eef4f7"),
            ("tertiary", "416376"),
            ("quaternary", "5e99b9"),
            ("header_background", "86bddb"),
            ("header_primary", "203243"),
            ("highlight", "86bddb"),
            ("selected", "bee0f2"),
            ("hover", "d2efff"),
            ("danger", "bf3c3c"),
            ("success", "70db82"),
            ("love", "fc94cb"),
        ],
    ),
    (
        "Latte",
        &[
            ("primary", "f2e5d7"),
            ("secondary", "262322"),
            ("tertiary", "f7f2ed"),
            ("quaternary", "d7c9aa"),
            ("header_background", "d7c9aa"),
            ("header_primary", "262322"),
            ("highlight", "d7c9aa"),
            ("selected", "3e2a14"),
            ("hover", "4c3319"),
            ("danger", "db9584"),
            ("success", "78be78"),
            ("love", "8f6201"),
        ],
    ),
    (
        "Summer",
        &[
            ("primary", "874342"),
            ("secondary", "fffff4"),
            ("tertiary", "fe9896"),
            ("quaternary", "fcc9d0"),
            ("header_background", "96ccbf"),
            ("header_primary", "fff1e7"),
            ("highlight", "f3c07f"),
            ("selected", "f5eaea"),
            ("hover", "f9f3f3"),
            ("danger", "cfebdc"),
            ("success", "fcb4b5"),
            ("love", "f3c07f"),
        ],
    ),
    (
        "Dark Rose",
        &[
            ("primary", "ca9cb2"),
            ("secondary", "3a2a37"),
            ("tertiary", "fdd459"),
            ("quaternary", "7e566a"),
            ("header_background", "a97189"),
            ("header_primary", "d9b2bb"),
            ("highlight", "bd36a3"),
            ("selected", "2a1620"),
            ("hover", "331b27"),
            ("danger", "6c3e63"),
            ("success", "d9b2bb"),
            ("love", "d9b2bb"),
        ],
    ),
    (
        "WCAG",
        &[
            ("primary", "000000"),
            ("primary-medium", "696969"),
            ("primary-low-mid", "909090"),
            ("secondary", "ffffff"),
            ("tertiary", "0033CC"),
            ("quaternary", "3369FF"),
            ("header_background", "ffffff"),
            ("header_primary", "000000"),
            ("highlight", "ffff00"),
            ("highlight-high", "0036E6"),
            ("highlight-medium", "e0e9ff"),
            ("highlight-low", "e0e9ff"),
            ("selected", "E2E9FE"),
            ("hover", "F0F4FE"),
            ("danger", "BB1122"),
            ("success", "3d854d"),
            ("love", "9D256B"),
        ],
    ),
    (
        "WCAG Dark",
        &[
            ("primary", "ffffff"),
            ("primary-medium", "999999"),
            ("primary-low-mid", "888888"),
            ("secondary", "0c0c0c"),
            ("tertiary", "759AFF"),
            ("quaternary", "759AFF"),
            ("header_background", "000000"),
            ("header_primary", "ffffff"),
            ("highlight", "3369FF"),
            ("selected", "0d2569"),
            ("hover", "002382"),
            ("danger", "FF697A"),
            ("success", "70B880"),
            ("love", "9D256B"),
        ],
    ),
    (
        "Dracula",
        &[
            ("primary_very_low", "373A47"),
            ("primary_low", "414350"),
            ("primary_low_mid", "8C8D94"),
            ("primary_medium", "A3A4AA"),
            ("primary_high", "CCCCCF"),
            ("primary", "f2f2f2"),
            ("primary-50", "3F414E"),
            ("primary-100", "535460"),
            ("primary-200", "666972"),
            ("primary-300", "7A7C84"),
            ("primary-400", "8D8F96"),
            ("primary-500", "A2A3A9"),
            ("primary-600", "B6B7BC"),
            ("primary-700", "C7C7C7"),
            ("primary-800", "DEDFE0"),
            ("primary-900", "F5F5F5"),
            ("secondary_low", "CCCCCF"),
            ("secondary_medium", "91939A"),
            ("secondary_high", "6A6C76"),
            ("secondary_very_high", "3D404C"),
            ("secondary", "2d303e"),
            ("tertiary_low", "4A4463"),
            ("tertiary_medium", "6E5D92"),
            ("tertiary", "bd93f9"),
            ("tertiary_high", "9275C1"),
            ("quaternary_low", "6AA8BA"),
            ("quaternary", "8be9fd"),
            ("header_background", "373A47"),
            ("header_primary", "f2f2f2"),
            ("highlight_low", "686D55"),
            ("highlight_medium", "52592B"),
            ("highlight_high", "C0C879"),
            ("selected", "4A4463"),
            ("hover", "61597f"),
            ("danger_low", "957279"),
            ("danger", "ff5555"),
            ("success_low", "386D50"),
            ("success_medium", "44B366"),
            ("success", "50fa7b"),
            ("love_low", "6C4667"),
            ("love", "ff79c6"),
        ],
    ),
    (
        "Solarized Light",
        &[
            ("primary_very_low", "F0ECD7"),
            ("primary_low", "D6D8C7"),
            ("primary_low_mid", "A4AFA5"),
            ("primary_medium", "7E918C"),
            ("primary_high", "4C6869"),
            ("primary", "002B36"),
            ("primary-50", "F0EBDA"),
            ("primary-100", "DAD8CA"),
            ("primary-200", "B2B9B3"),
            ("primary-300", "839496"),
            ("primary-400", "76898C"),
            ("primary-500", "697F83"),
            ("primary-600", "627A7E"),
            ("primary-700", "556F74"),
            ("primary-800", "415F66"),
            ("primary-900", "21454E"),
            ("secondary_low", "325458"),
            ("secondary_medium", "6C8280"),
            ("secondary_high", "97A59D"),
            ("secondary_very_high", "E8E6D3"),
            ("secondary", "FCF6E1"),
            ("tertiary_low", "D6E6DE"),
            ("tertiary_medium", "7EBFD7"),
            ("tertiary", "0088cc"),
            ("tertiary_high", "329ED0"),
            ("quaternary", "e45735"),
            ("header_background", "FCF6E1"),
            ("header_primary", "002B36"),
            ("highlight_low", "FDF9AD"),
            ("highlight_medium", "E3D0A3"),
            ("highlight", "F2F481"),
            ("highlight_high", "BCAA7F"),
            ("selected", "E8E6D3"),
            ("hover", "F0EBDA"),
            ("danger_low", "F8D9C2"),
            ("danger", "e45735"),
            ("success_low", "CFE5B9"),
            ("success_medium", "4CB544"),
            ("success", "009900"),
            ("love_low", "FCDDD2"),
            ("love", "fa6c8d"),
        ],
    ),
    (
        "Solarized Dark",
        &[
            ("primary_very_low", "0D353F"),
            ("primary_low", "193F47"),
            ("primary_low_mid", "798C88"),
            ("primary_medium", "97A59D"),
            ("primary_high", "B5BDB1"),
            ("primary", "FCF6E1"),
            ("primary-50", "21454E"),
            ("primary-100", "415F66"),
            ("primary-200", "556F74"),
            ("primary-300", "627A7E"),
            ("primary-400", "697F83"),
            ("primary-500", "76898C"),
            ("primary-600", "839496"),
            ("primary-700", "B2B9B3"),
            ("primary-800", "DAD8CA"),
            ("primary-900", "F0EBDA"),
            ("secondary_low", "B5BDB1"),
            ("secondary_medium", "81938D"),
            ("secondary_high", "4E6A6B"),
            ("secondary_very_high", "143B44"),
            ("secondary", "002B36"),
            ("tertiary_low", "003E54"),
            ("tertiary_medium", "00557A"),
            ("tertiary", "1a97d5"),
            ("tertiary_high", "006C9F"),
            ("quaternary_low", "944835"),
            ("quaternary", "e45735"),
            ("header_background", "002B36"),
            ("header_primary", "FCF6E1"),
            ("highlight_low", "4D6B3D"),
            ("highlight_medium", "464C33"),
            ("highlight", "F2F481"),
            ("highlight_high", "BFCA47"),
            ("selected", "143B44"),
            ("hover", "21454E"),
            ("danger_low", "443836"),
            ("danger_medium", "944835"),
            ("danger", "e45735"),
            ("success_low", "004C26"),
            ("success_medium", "007313"),
            ("success", "009900"),
            ("love_low", "4B3F50"),
            ("love", "fa6c8d"),
        ],
    ),
];

fn built_in_scheme(name: &str) -> Option<&'static [(&'static str, &'static str)]> {
    BUILT_IN_SCHEMES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, colors)| *colors)
}

/// A `color_schemes` row with what the serializer needs.
#[derive(Debug, Clone)]
pub struct ColorScheme {
    pub id: i32,
    pub name: String,
    pub base_scheme_id: Option<i32>,
    pub theme_id: Option<i32>,
    pub theme_name: Option<String>,
    pub user_selectable: bool,
    /// `color_scheme_colors` ordered by id.
    pub colors: Vec<(String, String)>,
}

impl ColorScheme {
    /// `ColorScheme.find_by_id` (ignoring remote copies) with its colors.
    pub async fn find(
        conn: &mut PgConnection,
        id: i32,
    ) -> Result<Option<ColorScheme>, sqlx::Error> {
        #[derive(sqlx::FromRow)]
        struct Row {
            id: i32,
            name: String,
            base_scheme_id: Option<i32>,
            theme_id: Option<i32>,
            theme_name: Option<String>,
            user_selectable: bool,
        }
        let row: Option<Row> = sqlx::query_as(
            "SELECT cs.id, cs.name, cs.base_scheme_id, cs.theme_id, t.name AS theme_name, cs.user_selectable \
             FROM color_schemes cs LEFT JOIN themes t ON t.id = cs.theme_id \
             WHERE cs.id = $1 AND NOT cs.remote_copy",
        )
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?;
        let Some(Row {
            id,
            name,
            base_scheme_id,
            theme_id,
            theme_name,
            user_selectable,
        }) = row
        else {
            return Ok(None);
        };
        let colors: Vec<(String, String)> = sqlx::query_as(
            "SELECT name, hex FROM color_scheme_colors WHERE color_scheme_id = $1 ORDER BY id",
        )
        .bind(id)
        .fetch_all(conn)
        .await?;
        Ok(Some(ColorScheme {
            id,
            name,
            base_scheme_id,
            theme_id,
            theme_name,
            user_selectable,
            colors,
        }))
    }

    /// `ColorScheme#base_colors`: the built-in palette named by
    /// base_scheme_id, else the DB scheme it points at (not ported), else
    /// nothing (or the scss base colors when there is no base scheme).
    fn base_colors(&self) -> Result<Vec<(String, String)>, crate::Unsupported> {
        let Some(base_id) = self.base_scheme_id else {
            return Ok(BASE_COLORS_ORDERED.clone());
        };
        let light_id = NAMES_TO_ID_MAP
            .iter()
            .find(|(n, _)| *n == LIGHT_PALETTE_NAME)
            .map(|(_, id)| *id);
        if base_id < 0 && Some(base_id) != light_id {
            let name = NAMES_TO_ID_MAP
                .iter()
                .find(|(_, id)| *id == base_id)
                .map(|(n, _)| *n);
            if let Some(colors) = name.and_then(built_in_scheme) {
                return Ok(colors
                    .iter()
                    .map(|(n, h)| (n.to_string(), h.to_string()))
                    .collect());
            }
        }
        if base_id > 0 {
            return Err(crate::Unsupported(
                "color schemes based on another DB scheme",
            ));
        }
        // The Light palette, or an unknown negative id: base_scheme is nil.
        Ok(Vec::new())
    }

    /// `ColorScheme#resolved_colors`: scss base, then the base palette
    /// (minus hover/selected), then DB colors; hover/selected derived when
    /// missing. Insertion order is Ruby's Hash merge order.
    pub fn resolved_colors(&self) -> Result<Vec<(String, String)>, crate::Unsupported> {
        let mut resolved: Vec<(String, String)> = BASE_COLORS_ORDERED.clone();
        let mut merge = |name: &str, hex: &str| match resolved.iter_mut().find(|(n, _)| n == name) {
            Some(entry) => entry.1 = hex.to_string(),
            None => resolved.push((name.to_string(), hex.to_string())),
        };
        for (name, hex) in self.base_colors()? {
            merge(&name, &hex);
        }
        resolved.retain(|(n, _)| n != "hover" && n != "selected");
        let mut merge = |name: &str, hex: &str| match resolved.iter_mut().find(|(n, _)| n == name) {
            Some(entry) => entry.1 = hex.to_string(),
            None => resolved.push((name.to_string(), hex.to_string())),
        };
        for (name, hex) in &self.colors {
            merge(name, hex);
        }

        let lookup = |list: &[(String, String)], name: &str| {
            list.iter().find(|(n, _)| n == name).map(|(_, h)| h.clone())
        };
        let primary = lookup(&resolved, "primary").unwrap_or_default();
        let secondary = lookup(&resolved, "secondary").unwrap_or_default();
        if lookup(&resolved, "hover").is_none() {
            let hover = color_math::dark_light_diff(&primary, &secondary, 0.94, -0.78);
            resolved.push(("hover".into(), hover));
        }
        if lookup(&resolved, "selected").is_none() {
            let selected = color_math::dark_light_diff(&primary, &secondary, 0.9, -0.8);
            resolved.push(("selected".into(), selected));
        }
        Ok(resolved)
    }

    /// `ColorScheme#is_dark?`: nil without DB colors, else whether primary
    /// is brighter than secondary.
    fn is_dark(&self) -> Result<Option<bool>, crate::Unsupported> {
        if self.colors.is_empty() {
            return Ok(None);
        }
        let resolved = self.resolved_colors()?;
        let get = |name: &str| {
            resolved
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, h)| h.clone())
                .unwrap_or_default()
        };
        Ok(Some(
            color_math::brightness(&get("primary")) > color_math::brightness(&get("secondary")),
        ))
    }

    /// ColorSchemeSerializer (embedding ColorSchemeColorSerializer).
    pub fn serialize(&self) -> Result<serde_json::Value, crate::Unsupported> {
        let base = self.base_colors()?;
        let base_hex = |name: &str| base.iter().find(|(n, _)| n == name).map(|(_, h)| h.clone());

        // sort_colors: COLORS_ORDER first, then the rest in resolved order.
        let resolved = self.resolved_colors()?;
        let mut ordered: Vec<&(String, String)> = COLORS_ORDER
            .iter()
            .filter_map(|n| resolved.iter().find(|(name, _)| name == n))
            .collect();
        ordered.extend(
            resolved
                .iter()
                .filter(|(n, _)| !COLORS_ORDER.contains(&n.as_str())),
        );

        let colors: Vec<serde_json::Value> = ordered
            .into_iter()
            .map(|(name, default)| {
                let hex = self
                    .colors
                    .iter()
                    .find(|(n, _)| n == name)
                    .map(|(_, h)| h.clone())
                    .unwrap_or_else(|| default.clone());
                let default_hex = if self.base_scheme_id.is_none() {
                    Some(hex.clone())
                } else {
                    base_hex(name)
                };
                serde_json::json!({
                    "name": name,
                    "hex": hex,
                    "default_hex": default_hex,
                    "is_advanced": !BASE_COLORS.contains_key(name),
                })
            })
            .collect();

        Ok(serde_json::json!({
            "id": self.id,
            "name": self.name,
            "is_base": null,
            "base_scheme_id": self.base_scheme_id,
            "theme_id": self.theme_id,
            "theme_name": self.theme_name,
            "user_selectable": self.user_selectable,
            "is_builtin_default": null,
            "is_dark": self.is_dark()?,
            "colors": colors,
        }))
    }
}

/// `ColorScheme.base_colors` in file order, for merges that keep order.
static BASE_COLORS_ORDERED: LazyLock<Vec<(String, String)>> =
    LazyLock::new(|| parse_base_colors_ordered(BASE_COLORS_SCSS));

fn parse_base_colors_ordered(scss: &str) -> Vec<(String, String)> {
    let mut colors: Vec<(String, String)> = Vec::new();
    for line in scss.lines() {
        let line = line.trim();
        let mut rest = line;
        while let Some(pos) = rest.find('$') {
            rest = &rest[pos + 1..];
            if let Some((name, hex)) = match_color(rest) {
                match colors.iter_mut().find(|(n, _)| n == name) {
                    Some(entry) => entry.1 = hex.to_string(),
                    None => colors.push((name.to_string(), hex.to_string())),
                }
                break;
            }
        }
    }
    colors
}

/// Port of lib/color_math.rb.
pub mod color_math {
    fn hex_to_rgb(color: &str) -> [f64; 3] {
        let expanded: String = if color.len() == 3 {
            color.chars().flat_map(|c| [c, c]).collect()
        } else {
            color.to_string()
        };
        let byte = |i: usize| {
            expanded
                .get(i..i + 2)
                .and_then(|s| u8::from_str_radix(s, 16).ok())
                .unwrap_or(0) as f64
        };
        [byte(0), byte(2), byte(4)]
    }

    fn rgb_to_hex(rgb: [f64; 3]) -> String {
        rgb.iter().map(|c| format!("{:02x}", *c as i64)).collect()
    }

    /// `dc-color-brightness()`
    pub fn brightness(color: &str) -> f64 {
        let [r, g, b] = hex_to_rgb(color);
        (r.trunc() * 299.0 + g.trunc() * 587.0 + b.trunc() * 114.0) / 1000.0
    }

    /// `dark-light-diff()`
    pub fn dark_light_diff(
        adjusted: &str,
        comparison: &str,
        lightness: f64,
        darkness: f64,
    ) -> String {
        if brightness(adjusted) < brightness(comparison) {
            scale_color_lightness(adjusted, lightness)
        } else {
            scale_color_lightness(adjusted, darkness)
        }
    }

    /// `scale_color(color, lightness:)`
    fn scale_color_lightness(color: &str, adjustment: f64) -> String {
        let rgb = hex_to_rgb(color);
        let (h, s, l) = rgb_to_hsl(rgb);
        let l = if adjustment > 0.0 {
            l + (100.0 - l) * adjustment
        } else {
            l + l * adjustment
        };
        rgb_to_hex(hsl_to_rgb(h, s, l))
    }

    fn rgb_to_hsl([r, g, b]: [f64; 3]) -> (f64, f64, f64) {
        let (r, g, b) = (r / 255.0, g / 255.0, b / 255.0);
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let l = (max + min) / 2.0;
        let (mut h, s);
        if max == min {
            h = 0.0;
            s = 0.0;
        } else {
            let d = max - min;
            s = if l >= 0.5 {
                d / (2.0 - max - min)
            } else {
                d / (max + min)
            };
            h = if max == r {
                (g - b) / d + if g < b { 6.0 } else { 0.0 }
            } else if max == g {
                (b - r) / d + 2.0
            } else {
                (r - g) / d + 4.0
            };
            h /= 6.0;
        }
        (
            (h * 360.0).round(),
            (s * 100.0).round(),
            (l * 100.0).round(),
        )
    }

    fn hsl_to_rgb(h: f64, s: f64, l: f64) -> [f64; 3] {
        let (h, s, l) = (h / 360.0, s / 100.0, l / 100.0);
        let (r, g, b) = if s == 0.0 {
            (l, l, l)
        } else {
            let q = if l < 0.5 {
                l * (1.0 + s)
            } else {
                l + s - l * s
            };
            let p = 2.0 * l - q;
            (
                hue_to_rgb(p, q, h + 1.0 / 3.0),
                hue_to_rgb(p, q, h),
                hue_to_rgb(p, q, h - 1.0 / 3.0),
            )
        };
        [
            (r * 255.0).round(),
            (g * 255.0).round(),
            (b * 255.0).round(),
        ]
    }

    fn hue_to_rgb(p: f64, q: f64, mut t: f64) -> f64 {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn brightness_and_hex_roundtrip() {
            assert_eq!(brightness("fff"), 255.0);
            assert_eq!(brightness("000000"), 0.0);
            assert_eq!(rgb_to_hex(hex_to_rgb("08c")), "0088cc");
        }

        #[test]
        fn dark_light_diff_matches_the_sass_helpers() {
            // Light scheme: primary 222 on secondary fff -> lighten 94%
            assert_eq!(dark_light_diff("222", "fff", 0.94, -0.78), "f2f2f2");
            // Dark scheme: primary ddd on secondary 222 -> darken 78%
            assert_eq!(dark_light_diff("dddddd", "222222", 0.94, -0.78), "313131");
        }
    }
}
