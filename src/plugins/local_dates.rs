//! discourse-local-dates (plugins/discourse-local-dates/plugin.rb): its
//! server side. The markup is the markdown renderer's
//! (pretty_text::render::local_dates), the excerpt's " (UTC)" is
//! excerpt.rs's; the dates are drawn in the browser
//! (static/js/local-dates.js). Here:
//!
//! - `before_post_process_cooked`: the post's `local_dates` custom field
//!   (`Post#local_dates`), its dates outside quotes;
//! - `reduce_cooked`: in emails, each date's text is its
//!   `data-email-preview`.

use markup5ever_rcdom::{Handle, NodeData, RcDom};
use serde_json::{Map, Value};

use super::PluginError;
use crate::pretty_text::cleanup::{all_elements, element_name, has_class, new_text};
use crate::site_settings::{SettingError, SiteSettings};

/// `DiscourseLocalDates::POST_CUSTOM_FIELD`
pub const POST_CUSTOM_FIELD: &str = "local_dates";

/// `enabled_site_setting :discourse_local_dates_enabled`: the plugin's
/// event handlers run only while it is on.
pub fn enabled(settings: &SiteSettings) -> Result<bool, SettingError> {
    Ok(settings.get("discourse_local_dates_enabled")?.truthy())
}

/// The `data-*` attributes the custom field keeps.
const KEPT: [&str; 4] = ["date", "time", "timezone", "recurring"];

/// What the handler collects from `doc.css("span.discourse-local-date")`:
/// for each date not inside an `aside`, its date, time, timezone and
/// recurring attributes in the element's order, `data-` taken out of the
/// name (`gsub`), values HTML-escaped, "undefined" ones left out.
pub fn dates(dom: &RcDom) -> Vec<Map<String, Value>> {
    let mut out = Vec::new();
    fn walk(node: &Handle, in_aside: bool, out: &mut Vec<Map<String, Value>>) {
        for child in node.children.borrow().iter() {
            let name = element_name(child);
            if name == Some("span") && !in_aside && has_class(child, "discourse-local-date") {
                let mut date = Map::new();
                if let NodeData::Element { attrs, .. } = &child.data {
                    for a in attrs.borrow().iter() {
                        let data_name = a.name.local.replace("data-", "");
                        if KEPT.contains(&data_name.as_str()) && &*a.value != "undefined" {
                            date.insert(
                                data_name,
                                Value::String(crate::discourse_diff::escape_html(&a.value)),
                            );
                        }
                    }
                }
                out.push(date);
            }
            walk(child, in_aside || name == Some("aside"), out);
        }
    }
    walk(
        &crate::pretty_text::cleanup::fragment_root(dom),
        false,
        &mut out,
    );
    out
}

/// `to_json` as ActiveSupport writes it: `<`, `>`, `&` and the line and
/// paragraph separators escaped (escape_html_entities_in_json). They only
/// occur inside strings, so escaping the whole text is the same.
fn active_support_json(value: &Value) -> String {
    let mut out = String::new();
    for c in value.to_string().chars() {
        match c {
            '<' => out.push_str("\\u003c"),
            '>' => out.push_str("\\u003e"),
            '&' => out.push_str("\\u0026"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c => out.push(c),
        }
    }
    out
}

/// `on(:before_post_process_cooked)`: the dates of the post's cooked html
/// into its `local_dates` custom field (a `:json` field), or the field
/// removed when it has none. As `save_custom_fields` does: nothing is
/// written while the stored value parses to the same dates; otherwise the
/// rows that are not the new value go and the new value is inserted.
pub async fn before_post_process_cooked(
    conn: &mut sqlx::PgConnection,
    settings: &SiteSettings,
    post_id: i32,
    cooked: &str,
) -> Result<(), PluginError> {
    if !enabled(settings)? {
        return Ok(());
    }
    let dates: Vec<Value> = dates(&crate::pretty_text::cleanup::parse(cooked))
        .into_iter()
        .map(Value::Object)
        .collect();
    let rows: Vec<(i32, Option<String>)> = sqlx::query_as(
        "SELECT id, value FROM post_custom_fields WHERE post_id = $1 AND name = $2 ORDER BY id",
    )
    .bind(post_id)
    .bind(POST_CUSTOM_FIELD)
    .fetch_all(&mut *conn)
    .await?;
    // refresh_custom_fields_from_db: a value that is not JSON reads as {}.
    let parse = |v: &Option<String>| {
        serde_json::from_str::<Value>(v.as_deref().unwrap_or(""))
            .unwrap_or_else(|_| Value::Object(Map::new()))
    };
    let stored = match rows.as_slice() {
        [] => None,
        [(_, v)] => Some(parse(v)),
        many => Some(Value::Array(many.iter().map(|(_, v)| parse(v)).collect())),
    };
    if dates.is_empty() {
        if stored.is_some() {
            sqlx::query("DELETE FROM post_custom_fields WHERE post_id = $1 AND name = $2")
                .bind(post_id)
                .bind(POST_CUSTOM_FIELD)
                .execute(&mut *conn)
                .await?;
        }
        return Ok(());
    }
    let dates = Value::Array(dates);
    if stored.as_ref() == Some(&dates) {
        return Ok(());
    }
    let value = active_support_json(&dates);
    let kept = rows
        .iter()
        .find(|(_, v)| v.as_deref() == Some(value.as_str()))
        .map(|(id, _)| *id);
    sqlx::query("DELETE FROM post_custom_fields WHERE post_id = $1 AND name = $2 AND id IS DISTINCT FROM $3")
        .bind(post_id)
        .bind(POST_CUSTOM_FIELD)
        .bind(kept)
        .execute(&mut *conn)
        .await?;
    if kept.is_none() {
        sqlx::query(
            "INSERT INTO post_custom_fields (post_id, name, value, created_at, updated_at) \
             VALUES ($1, $2, $3, clock_timestamp(), clock_timestamp())",
        )
        .bind(post_id)
        .bind(POST_CUSTOM_FIELD)
        .bind(&value)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// `on(:reduce_cooked)` (PrettyText.format_for_email): each date's
/// content becomes its `data-email-preview`, where it has one.
pub fn reduce_cooked(dom: &RcDom) {
    for e in all_elements(dom) {
        if !has_class(&e, "discourse-local-date") {
            continue;
        }
        if let Some(preview) = crate::pretty_text::cleanup::attr(&e, "data-email-preview") {
            for child in e.children.borrow().iter() {
                child.parent.set(None);
            }
            e.children.borrow_mut().clear();
            crate::pretty_text::cleanup::append(&e, new_text(&preview));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collected(html: &str) -> String {
        let dates: Vec<Value> = dates(&crate::pretty_text::cleanup::parse(html))
            .into_iter()
            .map(Value::Object)
            .collect();
        active_support_json(&Value::Array(dates))
    }

    #[test]
    fn dates_outside_quotes_in_attribute_order() {
        let html = "<p><span data-date=\"2026-10-07\" data-time=\"10:00:00\" class=\"discourse-local-date\" \
                    data-timezone=\"Europe/Paris\" data-recurring=\"1.weeks\" data-format=\"LL\" \
                    data-email-preview=\"x\">x</span></p>\
                    <aside class=\"quote\"><blockquote><p><span class=\"discourse-local-date\" \
                    data-date=\"2026-10-08\">y</span></p></blockquote></aside>\
                    <p><span class=\"discourse-local-date\" data-timezone=\"a&amp;b<\" data-time=\"undefined\"></span>\
                    <span class=\"cooked-date\" data-date=\"2026-10-09\"></span></p>";
        assert_eq!(
            collected(html),
            "[{\"date\":\"2026-10-07\",\"time\":\"10:00:00\",\"timezone\":\"Europe/Paris\",\
             \"recurring\":\"1.weeks\"},{\"timezone\":\"a\\u0026amp;b\\u0026lt;\"}]"
        );
    }

    #[test]
    fn email_shows_the_preview() {
        let dom = crate::pretty_text::cleanup::parse(
            "<p><span class=\"discourse-local-date\" data-email-preview=\"2026-10-07T10:00:00Z UTC\">\
             <b>2026</b></span> <span class=\"discourse-local-date\">kept</span></p>",
        );
        reduce_cooked(&dom);
        assert_eq!(
            crate::pretty_text::cleanup::to_html(&dom),
            "<p><span class=\"discourse-local-date\" data-email-preview=\"2026-10-07T10:00:00Z UTC\">\
             2026-10-07T10:00:00Z UTC</span> <span class=\"discourse-local-date\">kept</span></p>"
        );
    }
}
