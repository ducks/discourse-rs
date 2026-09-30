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
