//! The full-page search's advanced filters (SearchAdvancedOptions): what
//! static/js/search-advanced.js draws them from (the categories with their
//! chooser rows and labels, the options, the strings), and the count of
//! filters the term uses, which the toggle's badge shows.

use serde_json::{Value, json};

use crate::topic_list_view::{ListContext, t};

/// A category of the chooser (Category.list, by activity).
pub struct PanelCategory {
    pub id: i64,
    pub slug: String,
    pub parent_slug: Option<String>,
    pub name: String,
    /// CategoryRow's markup.
    pub row: String,
    /// The header's label (categoryBadgeHTML with its ancestors).
    pub label: String,
}

pub struct Panel {
    pub categories: Vec<PanelCategory>,
    pub member: bool,
    pub tagging: bool,
    /// discourse-solved's statuses (addAdvancedSearchOptions).
    pub solved: bool,
    pub mobile: bool,
    pub current_username: String,
    pub max_tag_search_results: i64,
    pub tag_style: String,
    pub tags_sort_alphabetically: bool,
}

/// `inOptionsForUsers` (members) then `inOptionsForAll`: the value and
/// its name's key.
fn in_options(member: bool) -> Vec<(&'static str, &'static str)> {
    let mut out = Vec::new();
    if member {
        for key in [
            "created",
            "posted",
            "likes",
            "bookmarks",
            "seen",
            "unseen",
            "watching",
            "tracking",
            "all",
            "private",
        ] {
            out.push((if key == "private" { "messages" } else { key }, key));
        }
    }
    for key in ["first", "pinned", "wiki", "images", "title"] {
        out.push((key, key));
    }
    out
}

fn status_options(solved: bool) -> Vec<&'static str> {
    let mut out = vec![
        "open",
        "closed",
        "public",
        "archived",
        "noreplies",
        "single_user",
    ];
    if solved {
        out.extend(["solved", "unsolved"]);
    }
    out
}

/// Handlebars' `escapeExpression`.
fn escape_expression(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            '`' => out.push_str("&#x60;"),
            '=' => out.push_str("&#x3D;"),
            c => out.push(c),
        }
    }
    out
}

/// `findSearchTerms`: the escaped term's blocks (REGEXP_BLOCKS, whose
/// quotes the escaping took away).
fn blocks(term: &str) -> Vec<String> {
    escape_expression(term)
        .split([' ', '\t', '\n', '\x0b', '\x0c', '\r'])
        .filter(|b| !b.is_empty())
        .map(str::to_string)
        .collect()
}

/// The block's rest after a case-insensitive prefix.
fn strip_prefix_ci<'a>(block: &'a str, prefixes: &[&str]) -> Option<&'a str> {
    prefixes.iter().find_map(|p| {
        (block.len() >= p.len() && block[..p.len()].eq_ignore_ascii_case(p))
            .then(|| &block[p.len()..])
    })
}

impl Panel {
    /// `activeFilterCount` as `didReceiveAttrs` sets the searched terms
    /// from the term.
    pub fn active_filter_count(&self, term: &str) -> usize {
        let blocks = blocks(term);
        let first = |prefixes: &[&str]| -> Option<String> {
            blocks
                .iter()
                .find_map(|b| strip_prefix_ci(b, prefixes).map(str::to_string))
        };
        let mut count = 0;
        // username
        if first(&["user:", "@"]).is_some_and(|v| !v.is_empty()) {
            count += 1;
        }
        // category: by slug, parent:child
        if let Some(rest) = first(&["category:", "#"]) {
            let parts: Vec<&str> = rest.split(':').collect();
            let found = if parts.len() > 1 {
                self.categories
                    .iter()
                    .any(|c| c.parent_slug.as_deref() == Some(parts[0]) && c.slug == parts[1])
            } else {
                self.categories.iter().any(|c| c.slug == parts[0])
            };
            if found {
                count += 1;
            }
        }
        // tags (and all_tags)
        if self.tagging {
            static TAGS: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
                regex::Regex::new(r"(?i)^(tags?:|#[a-z0-9\-]+::tag)").expect("static pattern")
            });
            if let Some(block) = blocks.iter().find(|b| TAGS.is_match(b)) {
                count += 1;
                if block.contains('+') {
                    count += 1;
                }
            }
        }
        // in: and with:
        let valid: Vec<&str> = in_options(self.member).iter().map(|(v, _)| *v).collect();
        if blocks.iter().any(|b| {
            strip_prefix_ci(b, &["in:", "with:"]).is_some_and(|v| {
                let v = v.to_lowercase();
                let v = if v == "personal" {
                    "messages".to_string()
                } else {
                    v
                };
                valid.contains(&v.as_str())
            })
        }) {
            count += 1;
        }
        // status: matched anywhere in a block, its `status:` prefix taken off
        let statuses = status_options(self.solved).join("|");
        let status = regex::Regex::new(&format!("(?i)status:({statuses})")).expect("built pattern");
        if let Some(block) = blocks.iter().find(|b| status.is_match(b)) {
            let value = strip_prefix_ci(block, &["status:"]).unwrap_or(block);
            if !value.is_empty() {
                count += 1;
            }
        }
        // time
        if first(&["before:", "after:"]).is_some_and(|v| !v.is_empty()) {
            count += 1;
        }
        for prefix in ["min_posts:", "max_posts:", "min_views:", "max_views:"] {
            if first(&[prefix]).is_some_and(|v| !v.is_empty()) {
                count += 1;
            }
        }
        count
    }

    /// What search-advanced.js reads (`#search-advanced-data`).
    pub fn data(&self, cx: &ListContext, count: usize, expanded: bool) -> Value {
        let s = |key: &str| t(cx, key);
        let in_options: Vec<Value> = in_options(self.member)
            .into_iter()
            .map(|(value, key)| json!({"name": s(&format!("search.advanced.filters.{key}")), "value": value}))
            .collect();
        let status_options: Vec<Value> = status_options(self.solved)
            .into_iter()
            .map(|v| json!({"name": s(&format!("search.advanced.statuses.{v}")), "value": v}))
            .collect();
        let time_options: Vec<Value> = ["before", "after"]
            .into_iter()
            .map(|v| json!({"name": s(&format!("search.advanced.post.time.{v}")), "value": v}))
            .collect();
        let categories: Vec<Value> = self
            .categories
            .iter()
            .map(|c| {
                json!({
                    "id": c.id,
                    "slug": c.slug,
                    "parentSlug": c.parent_slug,
                    "name": c.name,
                    "row": c.row,
                    "label": c.label,
                })
            })
            .collect();
        let mut strings = serde_json::Map::new();
        for (name, key) in [
            ("in_category", "search.advanced.in_category.label"),
            ("with_tags", "search.advanced.with_tags.label"),
            ("with_tags_aria", "search.advanced.with_tags.aria_label"),
            ("filters", "search.advanced.filters.label"),
            ("all_tags", "search.advanced.filters.all_tags"),
            ("statuses", "search.advanced.statuses.label"),
            ("posted_by", "search.advanced.posted_by.label"),
            ("posted_by_aria", "search.advanced.posted_by.aria_label"),
            ("post_time", "search.advanced.post.time.label"),
            ("post_time_aria", "search.advanced.post.time.aria_label"),
            ("post_count", "search.advanced.post.count.label"),
            ("post_min_aria", "search.advanced.post.min.aria_label"),
            ("post_min", "search.advanced.post.min.placeholder"),
            ("post_max_aria", "search.advanced.post.max.aria_label"),
            ("post_max", "search.advanced.post.max.placeholder"),
            ("views", "search.advanced.views.label"),
            ("min_views_aria", "search.advanced.min_views.aria_label"),
            ("min_views", "search.advanced.min_views.placeholder"),
            ("max_views_aria", "search.advanced.max_views.aria_label"),
            ("max_views", "search.advanced.max_views.placeholder"),
            ("filter_by", "select_kit.filter_by"),
            ("default_header", "select_kit.default_header_text"),
            ("filter_placeholder", "select_kit.filter_placeholder"),
            ("delete_item", "select_kit.delete_item"),
            ("no_content", "select_kit.no_content"),
            ("clear_input", "clear_input"),
            ("all_categories", "category.all"),
            ("any", "user.locale.any"),
            ("tags_placeholder", "tagging.choose_for_topic"),
            ("date_placeholder", "dates.placeholder"),
            ("search_button", "search.search_button"),
        ] {
            strings.insert(name.into(), json!(s(key)));
        }
        json!({
            "expanded": expanded,
            "count": count,
            "member": self.member,
            "tagging": self.tagging,
            "mobile": self.mobile,
            "currentUsername": self.current_username,
            "maxTagSearchResults": self.max_tag_search_results,
            "tagStyle": self.tag_style,
            "tagsSortAlphabetically": self.tags_sort_alphabetically,
            "strings": strings,
            "categories": categories,
            "inOptions": in_options,
            "statusOptions": status_options,
            "timeOptions": time_options,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panel() -> Panel {
        let cat = |id: i64, slug: &str, parent: Option<&str>| PanelCategory {
            id,
            slug: slug.into(),
            parent_slug: parent.map(str::to_string),
            name: slug.into(),
            row: String::new(),
            label: String::new(),
        };
        Panel {
            categories: vec![
                cat(4, "general", None),
                cat(34, "sub-general", Some("general")),
            ],
            member: true,
            tagging: true,
            solved: true,
            mobile: false,
            current_username: "user1".into(),
            max_tag_search_results: 5,
            tag_style: "simple".into(),
            tags_sort_alphabetically: false,
        }
    }

    #[test]
    fn counts_the_filters_as_activefiltercount_does() {
        let p = panel();
        assert_eq!(
            p.active_filter_count(
                "fixture #general tags:guide in:first status:open @user0 after:2026-01-01 min_posts:2 max_views:9"
            ),
            8
        );
        assert_eq!(p.active_filter_count("#general:sub-general tags:a+b"), 3);
        assert_eq!(p.active_filter_count("category:4 in:polls #=general"), 0);
        assert_eq!(p.active_filter_count("in:first in:likes with:images"), 1);
        assert_eq!(p.active_filter_count("xstatus:solved"), 1);
        assert_eq!(p.active_filter_count("@ before: user:"), 0);
    }
}
