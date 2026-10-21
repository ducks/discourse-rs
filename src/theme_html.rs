//! The theme's html fields drawn into pages: `Theme.lookup_field` for
//! head_tag, header, after_header, footer and body_tag, as the layouts
//! (`theme_lookup`) and ApplicationLayoutPreloader's customHTML (the
//! `top` and `footer` Ember draws with DCustomHtml) look them up.
//!
//! The fields are the default theme's and its enabled components'
//! (`Theme.transform_ids`; the port draws the default theme everywhere),
//! each theme's common field then its target's, joined with newlines. The
//! target is the user agent's (`mobile_device?`), not the viewport's.
//!
//! A field not baked yet is baked first (`ensure_baked!`); one baked by
//! another compiler version is drawn as baked, since an html field bakes
//! the same either way but for its inline scripts, which are refused.
//! The CSP nonce placeholder is dropped with its attribute: the port sends
//! no content security policy, so there is no nonce to give.
//!
//! Refused: inline scripts (AssetProcessor, which a restored site's baked
//! html points at in /theme-javascripts; 170), and scripts that would run
//! in after_header or footer, which Ember inserts as HTML where they never
//! run but a server-drawn page would run them.

use sqlx::PgConnection;

use crate::theme_writes::{self, CSP_NONCE_PLACEHOLDER};
use crate::themes;
use crate::{AppError, Unsupported};

/// The html fields of a page, each joined as `Theme.lookup_field` joins
/// them; empty where the theme has none.
#[derive(Clone, Debug, Default)]
pub struct Fields {
    pub head_tag: String,
    pub header: String,
    /// customHTML's `top`.
    pub after_header: String,
    pub footer: String,
    pub body_tag: String,
}

const NAMES: [&str; 5] = ["head_tag", "header", "after_header", "footer", "body_tag"];
/// `Theme.targets`: common, desktop, mobile.
const COMMON: i32 = 0;
const DESKTOP: i32 = 1;
const MOBILE: i32 = 2;
/// `ThemeField.types[:html]`
const HTML: i32 = 0;

#[derive(sqlx::FromRow)]
struct Row {
    id: i32,
    theme_id: i32,
    target_id: i32,
    name: String,
    type_id: i32,
    value: String,
    value_baked: Option<String>,
}

async fn rows(conn: &mut PgConnection, ids: &[i32], target: i32) -> Result<Vec<Row>, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, theme_id, target_id, name, type_id, value, value_baked FROM theme_fields \
         WHERE theme_id = ANY($1) AND target_id IN ($2, $3) AND name = ANY($4) ORDER BY id",
    )
    .bind(ids)
    .bind(COMMON)
    .bind(target)
    .bind(&NAMES[..])
    .fetch_all(conn)
    .await
}

/// `Theme.lookup_field(theme_id, mobile_device? ? :mobile : :desktop, name)`
/// for every html field a page draws.
pub async fn lookup(
    conn: &mut PgConnection,
    cx: &themes::Ctx<'_>,
    theme_id: Option<i64>,
    mobile: bool,
) -> Result<Fields, AppError> {
    let ids = crate::svg_sprite::transform_ids(conn, theme_id).await?;
    if ids.is_empty() {
        return Ok(Fields::default());
    }
    let target = if mobile { MOBILE } else { DESKTOP };
    let mut found = rows(conn, &ids, target).await?;
    if found.is_empty() {
        return Ok(Fields::default());
    }
    // theme_field_values: each field ensure_baked! before it is read.
    let unbaked: Vec<i32> = found
        .iter()
        .filter(|r| r.value_baked.is_none())
        .map(|r| r.id)
        .collect();
    if !unbaked.is_empty() {
        let env = theme_writes::Env::load(conn, cx).await?;
        for id in unbaked {
            theme_writes::ensure_baked(conn, &env, id).await?;
        }
        found = rows(conn, &ids, target).await?;
    }
    for row in &found {
        if row.type_id == HTML && has_script(&row.value) && inline_script(&row.value)? {
            return Err(Unsupported("inline scripts in theme html (AssetProcessor)").into());
        }
    }
    let field = |name: &str| -> Result<String, AppError> {
        let mut parts = Vec::new();
        for id in &ids {
            for t in [COMMON, target] {
                for row in found
                    .iter()
                    .filter(|r| r.theme_id == *id && r.target_id == t && r.name == name)
                {
                    let html = row.value_baked.as_deref().unwrap_or(&row.value);
                    if matches!(name, "after_header" | "footer")
                        && has_script(html)
                        && running_script(html)?
                    {
                        return Err(Unsupported(
                            "scripts in theme after_header or footer html (Ember inserts them without running them)",
                        )
                        .into());
                    }
                    parts.push(html);
                }
            }
        }
        Ok(without_nonce(&parts.join("\n")))
    };
    Ok(Fields {
        head_tag: field("head_tag")?,
        header: field("header")?,
        after_header: field("after_header")?,
        footer: field("footer")?,
        body_tag: field("body_tag")?,
    })
}

/// Whether the html may hold a script element at all, before parsing it.
fn has_script(html: &str) -> bool {
    html.to_ascii_lowercase().contains("<script")
}

/// Any script of the fragment, as Nokogiri's HTML5 parser finds them,
/// matching `is`.
fn any_script(
    html: &str,
    is: impl Fn(&markup5ever_rcdom::Handle) -> bool,
) -> Result<bool, AppError> {
    let mut found = false;
    crate::sanitize::html5_fragment_edit(html, |node| {
        if crate::sanitize::element_name(node).as_deref() == Some("script") && is(node) {
            found = true;
        }
    })
    .map_err(|_| Unsupported("theme html that does not parse"))?;
    Ok(found)
}

/// `inline_javascript?` of any of the fragment's scripts.
fn inline_script(html: &str) -> Result<bool, AppError> {
    any_script(html, theme_writes::inline_javascript)
}

/// A script a browser parsing the page runs: a classic script of a
/// JavaScript MIME type (or none) or a module; the inline ones are
/// refused before this.
fn running_script(html: &str) -> Result<bool, AppError> {
    any_script(html, |node| {
        let kind = crate::sanitize::attr(node, "type").unwrap_or_default();
        let kind = kind
            .trim_matches(|c: char| c.is_ascii_whitespace())
            .to_ascii_lowercase();
        kind.is_empty()
            || kind == "module"
            || matches!(
                kind.as_str(),
                "application/ecmascript"
                    | "application/javascript"
                    | "application/x-ecmascript"
                    | "application/x-javascript"
                    | "text/ecmascript"
                    | "text/javascript"
                    | "text/javascript1.0"
                    | "text/javascript1.1"
                    | "text/javascript1.2"
                    | "text/javascript1.3"
                    | "text/javascript1.4"
                    | "text/javascript1.5"
                    | "text/jscript"
                    | "text/livescript"
                    | "text/x-ecmascript"
                    | "text/x-javascript"
            )
    })
}

/// The baked html less its CSP nonce placeholders: the attribute process_html
/// gives each script, and any other the html spells out.
fn without_nonce(html: &str) -> String {
    html.replace(&format!(" nonce=\"{CSP_NONCE_PLACEHOLDER}\""), "")
        .replace(CSP_NONCE_PLACEHOLDER, "")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_nonce_goes_with_its_attribute() {
        let html =
            format!("<script src=\"/a.js\" nonce=\"{CSP_NONCE_PLACEHOLDER}\"></script>\n<p>x</p>");
        assert_eq!(
            without_nonce(&html),
            "<script src=\"/a.js\"></script>\n<p>x</p>"
        );
    }

    #[test]
    fn scripts_are_told_apart_as_rails_and_browsers_do() {
        assert!(inline_script("<script>alert(1)</script>").unwrap());
        assert!(inline_script("<script type=\"TEXT/JavaScript\">x</script>").unwrap());
        assert!(!inline_script("<script src=\"/a.js\"></script>").unwrap());
        assert!(!inline_script("<script type=\"text/x-handlebars\">x</script>").unwrap());
        assert!(running_script("<script src=\"/a.js\"></script>").unwrap());
        assert!(
            running_script("<div><script type=\"module\" src=\"/a.js\"></script></div>").unwrap()
        );
        assert!(!running_script("<script type=\"application/ld+json\">{}</script>").unwrap());
        assert!(!has_script("<p>no scripts</p>"));
        assert!(has_script("<SCRIPT src=x></SCRIPT>"));
    }
}
