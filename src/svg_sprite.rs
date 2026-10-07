//! `SvgSprite.raw_svg`: an icon of Discourse's core sprites inlined as an
//! `<svg>`, as the server-rendered pages (the not-found page, category
//! badges) embed them. The page chrome uses the smaller sprite in
//! static/vendor/icons.svg instead.
//!
//! The Font Awesome sprites are vendored from Discourse
//! (vendor/discourse/vendor/assets/svg-icons/fontawesome, CC BY 4.0).
//! Discourse's own sprites (discourse-additional, nested replies) hold
//! nested markup whose Nokogiri serialization is not ported.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::Unsupported;

const SOLID: &str =
    include_str!("../vendor/discourse/vendor/assets/svg-icons/fontawesome/solid.svg");
const REGULAR: &str =
    include_str!("../vendor/discourse/vendor/assets/svg-icons/fontawesome/regular.svg");
const BRANDS: &str =
    include_str!("../vendor/discourse/vendor/assets/svg-icons/fontawesome/brands.svg");
const ADDITIONAL: &str =
    include_str!("../vendor/discourse/vendor/assets/svg-icons/discourse-additional.svg");

/// Icon id to its symbol as `Nokogiri.XML(symbol).children.first` renamed
/// to `svg` prints it.
fn symbols() -> &'static HashMap<String, String> {
    static SYMBOLS: OnceLock<HashMap<String, String>> = OnceLock::new();
    SYMBOLS.get_or_init(|| {
        let mut out = HashMap::new();
        for (sprite, prefix) in [(BRANDS, "fab-"), (REGULAR, "far-"), (SOLID, "")] {
            for (id, svg) in parse(sprite, prefix) {
                out.insert(id, svg);
            }
        }
        out
    })
}

/// The `<symbol>`s of a Font Awesome sprite, each child (self-closing
/// paths, the odd comment) on a line of its own indented two spaces, as
/// Nokogiri re-serializes a NOBLANKS parse.
fn parse(sprite: &str, prefix: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = sprite;
    while let Some(start) = rest.find("<symbol ") {
        rest = &rest[start..];
        let Some(head_end) = rest.find('>') else {
            break;
        };
        let head = &rest[..head_end];
        let Some(close) = rest.find("</symbol>") else {
            break;
        };
        let body = &rest[head_end + 1..close];
        rest = &rest[close + "</symbol>".len()..];
        let Some(id) = attr(head, "id") else {
            continue;
        };
        let attrs = head["<symbol".len()..].trim().replacen(
            &format!("id=\"{id}\""),
            &format!("id=\"{prefix}{id}\""),
            1,
        );
        let mut svg = format!("<svg {attrs}>\n");
        for child in children(body) {
            svg.push_str("  ");
            svg.push_str(child);
            svg.push('\n');
        }
        svg.push_str("</svg>");
        out.push((format!("{prefix}{id}"), svg));
    }
    out
}

fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let key = format!(" {name}=\"");
    let start = tag.find(&key)? + key.len();
    let end = tag[start..].find('"')? + start;
    Some(&tag[start..end])
}

/// The top-level nodes of a symbol's body: comments and self-closing
/// elements; whitespace between them is dropped.
fn children(body: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = body.trim_start();
    while !rest.is_empty() {
        let end = if rest.starts_with("<!--") {
            rest.find("-->").map(|i| i + 3)
        } else if rest.starts_with('<') {
            rest.find("/>").map(|i| i + 2)
        } else {
            None
        };
        let Some(end) = end else {
            break;
        };
        out.push(&rest[..end]);
        rest = rest[end..].trim_start();
    }
    out
}

/// `SvgSprite.raw_svg(name)`: "" for an unknown icon, as Rails answers.
pub fn raw_svg(name: &str) -> Result<String, Unsupported> {
    let name = name.trim();
    match symbols().get(name) {
        Some(symbol) => Ok(format!(
            "<svg class=\"fa d-icon svg-icon svg-node\" aria-hidden=\"true\">{symbol}</svg>\n"
        )),
        None if ADDITIONAL.contains(&format!("<symbol id=\"{name}\""))
            || name.starts_with("discourse-") =>
        {
            Err(Unsupported("inlining Discourse's own sprite icons"))
        }
        None => Ok(String::new()),
    }
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
    fn prefixes_and_unknown_icons() {
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
        assert_eq!(raw_svg("no-such-icon").unwrap(), "");
        // language's symbol carries a comment before its path.
        let language = raw_svg("language").unwrap();
        assert!(language.contains(">\n  <!--!Font Awesome"));
        assert!(language.contains("-->\n  <path d="));
    }
}
