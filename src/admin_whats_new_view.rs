//! What's new (admin/templates/admin/whats-new.gjs): DPageHeader with
//! Check for updates, and DashboardNewFeatures' cards, a month each
//! (`released_at || created_at`), of DashboardNewFeatureItems.

use serde_json::{Map, Value};

use crate::admin_view::{AdminCx, Crumb, PageAction, PageHeader, page_actions, t, t_with};
use crate::topic_list_view::escape;

/// A feature with its description cooked (DCookText).
pub struct Feature<'a> {
    pub item: &'a Map<String, Value>,
    pub cooked: String,
}

/// Ember's `dasherize` (decamelize, then spaces and underscores to
/// dashes), lower-cased: the item's id.
fn dasherize(text: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = text.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        // decamelize: `([a-z\d])([A-Z])` gets an underscore between.
        if c.is_ascii_uppercase()
            && i > 0
            && (chars[i - 1].is_ascii_lowercase() || chars[i - 1].is_ascii_digit())
        {
            out.push('-');
        }
        out.push(match c {
            ' ' | '_' => '-',
            other => *other,
        });
    }
    out.to_lowercase()
}

fn present(item: &Map<String, Value>, key: &str) -> Option<String> {
    match item.get(key) {
        Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
        Some(Value::Number(n)) => Some(n.to_string()),
        _ => None,
    }
}

/// DashboardNewFeatureItem
fn item_html(cx: &AdminCx, feature: &Feature) -> String {
    let item = feature.item;
    let id = match present(item, "upcoming_change_setting_name") {
        Some(name) => Some(format!("upcoming-change-{name}")),
        None => present(item, "title").map(|title| dasherize(&title)),
    };
    let title = present(item, "title").unwrap_or_default();
    let screenshot = present(item, "screenshot_url");
    let emoji = match (present(item, "emoji"), &screenshot) {
        (Some(emoji), None) => format!(
            "<div class=\"admin-new-feature-item__new-feature-emoji\">{}</div>",
            escape(&emoji)
        ),
        _ => String::new(),
    };
    let image = screenshot
        .map(|url| {
            format!(
                "<div class=\"admin-new-feature-item__img-container\"><img alt=\"{}\" class=\"admin-new-feature-item__screenshot\" src=\"{}\"></div>",
                escape(&title),
                escape(&url)
            )
        })
        .unwrap_or_default();
    let learn_more = present(item, "link")
        .map(|link| {
            format!(
                "<a class=\"admin-new-feature-item__learn-more\" href=\"{}\" rel=\"noopener noreferrer\" target=\"_blank\">{}</a>",
                escape(&link),
                escape(&t(cx.i18n, "admin.dashboard.new_features.learn_more"))
            )
        })
        .unwrap_or_default();
    format!(
        "<div class=\"admin-new-feature-item\"{}><div class=\"admin-new-feature-item__content\"><div class=\"admin-new-feature-item__header\">{emoji}<h3>{}</h3></div><div class=\"admin-new-feature-item__body-wrapper\">{image}<div class=\"admin-new-feature-item__body\"><div class=\"admin-new-feature-item__feature-description\"><div>{}</div>{learn_more}</div></div></div></div></div>",
        id.map(|id| format!(" id=\"{}\"", escape(&id)))
            .unwrap_or_default(),
        escape(&title),
        feature.cooked,
    )
}

/// `moment(released_at || created_at)`'s month: (key, "MMMM YYYY").
fn month(item: &Map<String, Value>) -> Option<(String, String)> {
    let at = present(item, "released_at").or_else(|| present(item, "created_at"))?;
    let t = chrono::DateTime::parse_from_rfc3339(&at).ok()?;
    Some((t.format("%Y-%m").to_string(), t.format("%B %Y").to_string()))
}

/// The page's main area: the cards by month in the order the features
/// come, or AdminConfigAreaEmptyList.
pub fn page(cx: &AdminCx, features: &[Feature]) -> String {
    let title = t(cx.i18n, "admin.config.whats_new.title");
    let header = PageHeader {
        crumbs: vec![
            Crumb {
                label: t(cx.i18n, "admin_title"),
                path: "/admin",
            },
            Crumb {
                label: title.clone(),
                path: "/admin/whats-new",
            },
        ],
        title,
        actions: page_actions(
            cx,
            "d-page-header",
            &[PageAction {
                class: "admin-whats-new__check".into(),
                kind: "primary",
                label: t(cx.i18n, "admin.new_features.check_for_updates"),
                title: None,
            }],
        ),
        description: Some(escape(&t(
            cx.i18n,
            "admin.config.whats_new.header_description",
        ))),
        learn_more_url: Some("https://releases.discourse.org/".into()),
        tabs: None,
    }
    .render(cx);
    // reduce into an object keyed by month: the months in the order first
    // seen, each with its features.
    let mut groups: Vec<(String, String, Vec<&Feature>)> = Vec::new();
    for feature in features {
        let Some((key, label)) = month(feature.item) else {
            continue;
        };
        match groups.iter_mut().find(|(k, _, _)| *k == key) {
            Some((_, _, list)) => list.push(feature),
            None => groups.push((key, label, vec![feature])),
        }
    }
    let content = if groups.is_empty() {
        format!(
            "<div class=\"admin-config-area-empty-list\"><span class=\"admin-config-area-empty-list__title\">{}</span></div>",
            t_with(
                cx.i18n,
                "admin.dashboard.new_features.no_new_features_found",
                &[("url", "https://releases.discourse.org/")]
            )
        )
    } else {
        groups
            .iter()
            .map(|(_, label, list)| {
                // The h2 inside the title's h3, as Glimmer builds it: the
                // parser closes the h3 first, admin-whats-new.js moves it in.
                format!(
                    "<section class=\"admin-config-area-card admin-new-features__group\" data-new-features-group=\"{label}\"><div class=\"admin-config-area-card__header-wrapper\"><h3 class=\"admin-config-area-card__title\"></h3><h2>{label}</h2></div><div class=\"admin-config-area-card__content\">{}</div></section>",
                    list.iter().map(|f| item_html(cx, f)).collect::<String>()
                )
            })
            .collect()
    };
    // Each drawn item's month date as epoch milliseconds: moment groups
    // them in the browser's zone, which admin-whats-new.js regroups by.
    let dates: Vec<i64> = groups
        .iter()
        .flat_map(|(_, _, list)| list.iter())
        .filter_map(|f| month_date(f.item))
        .collect();
    format!(
        "{header}<div class=\"admin-container admin-config-page__main-area\"><div class=\"admin-config-area\"><div class=\"admin-config-area__primary-content\"><div class=\"loading-container\">{content}</div></div></div></div><script type=\"application/json\" id=\"admin-whats-new-dates\">{}</script>",
        serde_json::to_string(&dates).unwrap_or_default()
    )
}

/// `released_at || created_at` as epoch milliseconds.
fn month_date(item: &Map<String, Value>) -> Option<i64> {
    let at = present(item, "released_at").or_else(|| present(item, "created_at"))?;
    Some(
        chrono::DateTime::parse_from_rfc3339(&at)
            .ok()?
            .timestamp_millis(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_dasherized_titles() {
        assert_eq!(
            dasherize("What users are asking with Ask AI"),
            "what-users-are-asking-with-ask-ai"
        );
        assert_eq!(
            dasherize("Edit dates, polls, tables, and more in the rich text editor "),
            "edit-dates,-polls,-tables,-and-more-in-the-rich-text-editor-"
        );
        assert_eq!(dasherize("innerHTML x_y"), "inner-html-x-y");
    }
}
