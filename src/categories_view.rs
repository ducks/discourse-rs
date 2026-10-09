//! The categories page as the Ember app renders it for
//! `desktop_category_page_style: categories_and_latest_topics`
//! (components/categories-and-latest-topics.gjs): the categories table
//! (categories-only.gjs, parent-category-row.gjs) beside the latest
//! topics (categories-topic-list.gjs, latest-topic-list-item.gjs).
//!
//! Not drawn yet: category logos, the unread and new counts of
//! CategoryUnread.

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::Unsupported;
use crate::topic_list_view::{
    ListContext, category_badge, category_badge_html, escape, long_date, relative_age_tiny, t,
    t_count, t_with, tags_html, topic_statuses,
};

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}

fn date(v: &Value) -> Option<DateTime<Utc>> {
    v.as_str()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc))
}

/// `number(n)` for the counts here (below a thousand in practice).
fn number(n: i64) -> String {
    n.to_string()
}

/// `Category#notificationLevelString`
fn notification_level(level: i64) -> &'static str {
    match level {
        0 => "muted",
        2 => "tracking",
        3 => "watching",
        4 => "watching_first_post",
        _ => "regular",
    }
}

/// `.categories-and-latest`, from the category list and latest list
/// documents; `muted`, the viewer's muted category ids.
pub fn render(
    cx: &ListContext,
    category_list: &Value,
    topic_list: &Value,
    muted: &[i64],
) -> Result<String, Unsupported> {
    let categories: Vec<&Value> = category_list["category_list"]["categories"]
        .as_array()
        .into_iter()
        .flatten()
        .collect();
    let mut out =
        String::from("<div class=\"categories-and-latest\"><div class=\"column categories\">");
    out.push_str(&categories_only(cx, &categories, muted)?);
    out.push_str("</div><div class=\"column\">");
    out.push_str(&latest_topics(cx, topic_list));
    out.push_str("</div></div>");
    Ok(out)
}

/// CategoryList.list's `statPeriod`: week, else month, when two thirds of
/// the categories have topics in it; else all time.
fn stat_period(categories: &[&Value]) -> &'static str {
    for period in ["week", "month"] {
        let with = categories
            .iter()
            .filter(|c| c[format!("topics_{period}")].as_i64().unwrap_or(0) > 0)
            .count();
        if with as f64 >= categories.len() as f64 * 0.66 {
            return period;
        }
    }
    "all"
}

/// Which list a row is drawn in (ParentCategoryRow's `listType`).
#[derive(Clone, Copy, PartialEq)]
enum ListType {
    Normal,
    Muted,
}

/// The viewer's muted categories, for Category#isMuted, #isHidden and
/// #hasMuted.
struct Muting<'a> {
    cx: &'a ListContext<'a>,
    muted: &'a [i64],
}

impl Muting<'_> {
    fn subs(&self, c: &Value) -> Vec<i64> {
        c["subcategory_ids"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_i64)
            .filter(|id| self.cx.categories.contains_key(id))
            .collect()
    }

    fn is_muted(&self, id: i64) -> bool {
        self.muted.contains(&id)
    }

    /// Muted, and so are its subcategories.
    fn is_hidden(&self, c: &Value) -> bool {
        let id = c["id"].as_i64().unwrap_or(0);
        self.is_muted(id) && self.subs(c).iter().all(|s| self.is_muted(*s))
    }

    fn has_muted(&self, c: &Value) -> bool {
        let id = c["id"].as_i64().unwrap_or(0);
        self.is_muted(id) || self.subs(c).iter().any(|s| self.is_muted(*s))
    }

    /// The muted styling in a list: muted ones in the normal list, the
    /// others in the muted list.
    fn styled_muted(&self, id: i64, list: ListType) -> bool {
        (self.is_muted(id) && list == ListType::Normal)
            || (!self.is_muted(id) && list == ListType::Muted)
    }
}

/// CategoriesOnly: the categories table, then the muted ones behind their
/// toggle.
fn categories_only(
    cx: &ListContext,
    categories: &[&Value],
    muted: &[i64],
) -> Result<String, Unsupported> {
    if categories.is_empty() {
        return Ok(String::new());
    }
    let muting = Muting { cx, muted };
    let period = stat_period(categories);
    let filtered: Vec<&&Value> = categories.iter().filter(|c| !muting.is_hidden(c)).collect();
    let mut out = String::new();
    let rows = |list: ListType| -> Result<String, Unsupported> {
        let mut rows = String::new();
        for c in categories {
            rows.push_str(&row(cx, &muting, c, period, list)?);
        }
        Ok(rows)
    };
    if !filtered.is_empty() {
        out.push_str(&format!(
            "<table class=\"category-list\"><thead class=\"category-list-header\"><tr>\
             <th class=\"category topic-list-data default\"><span aria-level=\"2\" id=\"categories-only-category\" role=\"heading\">{}</span></th>\
             <th class=\"topics topic-list-data num\">{}</th></tr></thead>\
             <tbody aria-labelledby=\"categories-only-category\">{}</tbody></table>",
            escape(&t(cx, "categories.category")),
            escape(&t(cx, "categories.topics")),
            rows(ListType::Normal)?
        ));
    }
    let any_muted = categories[0]["parent_category_id"].is_null()
        && categories.iter().any(|c| muting.has_muted(c));
    if any_muted {
        // showMutedCategories: only when nothing else is listed.
        let show = filtered.is_empty();
        let toggle = if filtered.is_empty() {
            String::new()
        } else {
            crate::topic_list_view::icon("plus", None)
        };
        out.push_str(&format!(
            "<div class=\"muted-categories\"><a class=\"muted-categories-link\" href><h3 class=\"muted-categories-heading\">{}</h3>{toggle}</a>\
             <table class=\"category-list{}\"><thead><tr><th class=\"category\"><span aria-level=\"2\" id=\"categories-only-category-muted\" role=\"heading\">{}</span></th>\
             <th class=\"topics\">{}</th></tr></thead><tbody aria-labelledby=\"categories-only-category-muted\">{}</tbody></table></div>",
            escape(&t(cx, "categories.muted")),
            if show { "" } else { " hidden" },
            escape(&t(cx, "categories.category")),
            escape(&t(cx, "categories.topics")),
            rows(ListType::Muted)?
        ));
    }
    Ok(out)
}

/// ParentCategoryRow on desktop.
fn row(
    cx: &ListContext,
    muting: &Muting,
    c: &Value,
    period: &str,
    list: ListType,
) -> Result<String, Unsupported> {
    let hidden = (muting.is_hidden(c) && list == ListType::Normal)
        || (!muting.has_muted(c) && list == ListType::Muted);
    let Some(category) = c["id"].as_i64().and_then(|id| cx.categories.get(&id)) else {
        return Ok(String::new());
    };
    if hidden {
        return Ok(String::new());
    }
    if c["uploaded_logo"].is_object() {
        return Err(Unsupported("category logos on the categories page"));
    }
    let mut out = String::new();
    let excerpt = c["description_excerpt"].as_str().filter(|e| !e.is_empty());
    out.push_str(&format!(
        "<tr class=\"{} no-logo\" data-category-id=\"{}\" data-notification-level=\"{}\">",
        if excerpt.is_some() {
            "has-description"
        } else {
            "no-description"
        },
        category.id,
        notification_level(c["notification_level"].as_i64().unwrap_or(1))
    ));
    // categoryColorVariable
    out.push_str(&format!(
        "<td class=\"category{}\" style=\"--category-badge-color: #{};\">",
        if muting.styled_muted(category.id, list) {
            " muted"
        } else {
            ""
        },
        category.color
    ));
    // CategoryTitleLink
    let url = format!("{}/c/{}/{}", cx.base_path, category.slug, category.id);
    out.push_str(&format!(
        "<h3><a class=\"category-title-link\" href=\"{url}\"><div class=\"category-text-title\"><span class=\"category-name\"><span>{}</span></span></div></a></h3>",
        category_badge_html(cx, category, false, false)
    ));
    if let Some(e) = excerpt {
        out.push_str(&format!(
            "<div class=\"category-description\"><div><span>{e}</span></div></div>"
        ));
    }
    // SubCategoryItem: not the ones muted for this list.
    let subs: Vec<String> = muting
        .subs(c)
        .into_iter()
        .filter(|id| !muting.styled_muted(*id, list))
        .filter_map(|id| cx.categories.get(&id))
        .map(|sub| {
            format!(
                "<span class=\"subcategory\">{} <span class=\"category__badges\"></span></span>",
                category_badge_html(cx, sub, true, true)
            )
        })
        .collect();
    if !muting.subs(c).is_empty() {
        out.push_str(&format!(
            "<div class=\"subcategories\">{}</div>",
            subs.join(" ")
        ));
    }
    out.push_str("</td>");
    // The topic stat
    let stat = c[format!("topics_{period}")].as_i64().unwrap_or(0);
    let (stat_html, title) = if period != "all" && stat > 0 {
        let unit = t(cx, &format!("categories.topic_stat_unit.{period}"));
        (
            t_count(
                cx,
                "categories.topic_stat",
                stat,
                &[
                    (
                        "number",
                        &format!("<span class=\"value\">{}</span>", number(stat)),
                    ),
                    (
                        "unit",
                        &format!("<span class=\"unit\">{}</span>", escape(&unit)),
                    ),
                ],
            ),
            t_count(
                cx,
                &format!("categories.topic_stat_sentence_{period}"),
                stat,
                &[],
            ),
        )
    } else {
        let all = c["topics_all_time"].as_i64().unwrap_or(0);
        (
            format!("<span class=\"value\">{}</span>", number(all)),
            t_count(cx, "categories.topic_sentence", all, &[]),
        )
    };
    out.push_str(&format!(
        "<td class=\"topics topic-list-data num\"><div title=\"{}\">{stat_html}</div>\
         <div class=\"unread-new\"><div class=\"category__badges unread-new\"></div></div></td></tr>",
        escape(&title)
    ));
    Ok(out)
}

/// CategoriesTopicList for `latest`.
fn latest_topics(cx: &ListContext, doc: &Value) -> String {
    let topics: Vec<&Value> = doc["topic_list"]["topics"]
        .as_array()
        .into_iter()
        .flatten()
        .collect();
    let users: Vec<&Value> = doc["users"].as_array().into_iter().flatten().collect();
    let mut out = format!(
        "<div class=\"latest-topic-list\"><div aria-level=\"2\" class=\"table-heading\" role=\"heading\">{}</div>",
        escape(&t(cx, "filters.latest.title"))
    );
    if topics.is_empty() {
        out.push_str(&format!(
            "<div class=\"no-topics\"><h3>{}</h3></div></div>",
            escape(&t(cx, "topics.none.latest"))
        ));
        return out;
    }
    for topic in topics {
        out.push_str(&latest_item(cx, topic, &users));
    }
    out.push_str(&format!(
        "<div class=\"more-topics\"><a class=\"btn btn-default pull-right\" href=\"{}/latest\">{}</a></div></div>",
        cx.base_path,
        escape(&t(cx, "more"))
    ));
    out
}

/// LatestTopicListItem
fn latest_item(cx: &ListContext, topic: &Value, users: &[&Value]) -> String {
    let base = cx.base_path;
    let id = topic["id"].as_i64().unwrap_or(0);
    let slug = match s(&topic["slug"]).trim() {
        "" => "topic",
        slug => slug,
    };
    let url = format!("{base}/t/{slug}/{id}");
    let flag = |name: &str| topic[name] == true;
    let category = topic["category_id"]
        .as_i64()
        .and_then(|cid| cx.categories.get(&cid));

    let mut classes = vec!["latest-topic-list-item".to_string()];
    for tag in topic["tags"].as_array().into_iter().flatten() {
        let name = tag
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| s(&tag["name"]).to_string());
        classes.push(format!("tag-{name}"));
    }
    if let Some(c) = category {
        // fullSlug: the parent's slug first.
        let full = match c.parent_id.and_then(|p| cx.categories.get(&p)) {
            Some(p) => format!("{}-{}", p.slug, c.slug),
            None => c.slug.clone(),
        };
        classes.push(format!("category-{full}"));
    }
    for (name, class) in [
        ("liked", "liked"),
        ("archived", "archived"),
        ("bookmarked", "bookmarked"),
        ("pinned", "pinned"),
        ("closed", "closed"),
    ] {
        if flag(name) {
            classes.push(class.to_string());
        }
    }
    if topic["last_read_post_number"].is_number() && cx.member_trust_level.is_some() {
        classes.push("visited".to_string());
    }

    // lastPosterUser: the poster marked "latest", else the first.
    let posters: Vec<&Value> = topic["posters"].as_array().into_iter().flatten().collect();
    let last = posters
        .iter()
        .find(|p| s(&p["extras"]).split(' ').any(|e| e == "latest"))
        .or(posters.first())
        .and_then(|p| {
            let uid = p["user_id"].as_i64()?;
            users
                .iter()
                .find(|u| u["id"].as_i64() == Some(uid))
                .copied()
        });
    let poster = last
        .map(|u| {
            let username = s(&u["username"]);
            let template = s(&u["avatar_template"]);
            let src = if template.starts_with('/') && !template.starts_with("//") {
                format!("{base}{}", template.replace("{size}", "48"))
            } else {
                template.replace("{size}", "48")
            };
            format!(
                "<a aria-label=\"{}\" data-user-card=\"{user}\" href=\"{base}/u/{}\" tabindex=\"0\"><img alt=\"\" width=\"48\" height=\"48\" src=\"{}\" class=\"avatar\" title=\"{user}\"></a>",
                escape(&t_with(cx, "user.profile_possessive", &[("username", username)])),
                escape(&username.to_lowercase()),
                escape(&src),
                user = escape(username),
            )
        })
        .unwrap_or_default();

    // dTopicLink
    let title_html =
        crate::topic_list_view::emoji_unescape(s(&topic["fancy_title"]), &cx.settings, base);
    let tags = tags_html(
        cx,
        topic,
        s(&topic["title"]),
        &crate::topic_list_view::tags_callbacks(cx, topic),
    );
    let category_link = category.map(|c| category_badge(cx, c)).unwrap_or_default();

    // ItemRepliesCell
    let posts_count = topic["posts_count"].as_i64().unwrap_or(0);
    let replies = posts_count - 1;
    let highest = topic["highest_post_number"].as_i64().unwrap_or(1);
    // lastPostUrl and bumpedAtTitle
    let created = date(&topic["created_at"]);
    let bumped = date(&topic["bumped_at"]).or(created);
    let mut bumped_title = String::new();
    if let Some(c) = created {
        bumped_title = t_with(cx, "topic.created_at", &[("date", &long_date(cx, c))]);
        let second = |d: DateTime<Utc>| d.format("%Y-%m-%dT%H:%M:%S").to_string();
        if let Some(b) = bumped.filter(|b| second(*b) != second(c)) {
            bumped_title.push('\n');
            bumped_title.push_str(&t_with(
                cx,
                "topic.bumped_at",
                &[("date", &long_date(cx, b))],
            ));
        }
    }
    let age = bumped
        .map(|b| {
            format!(
                "<span class=\"relative-date\" data-time=\"{}\" data-format=\"tiny\">{}</span>",
                b.timestamp_millis(),
                escape(&relative_age_tiny(cx, b))
            )
        })
        .unwrap_or_default();

    format!(
        "<div class=\"{}\" data-topic-id=\"{id}\"><div class=\"topic-poster\">{poster}</div>\
         <div class=\"main-link\"><div class=\"top-row\"><span class=\"topic-statuses\">{}</span> <a href=\"{url}\" class=\"title\" data-topic-id=\"{id}\">{title_html}</a><span class=\"topic-post-badges\"></span></div>\
         <div class=\"bottom-row\">{category_link}{tags}</div></div>\
         <div class=\"topic-stats\"><div class=\"num posts-map posts topic-list-data\"><a aria-label=\"{}\" class=\"badge-posts\" href=\"{url}/1\"><span class=\"number\">{}</span></a></div>\
         <div class=\"topic-last-activity\"><a href=\"{url}/{highest}\" title=\"{}\">{age}</a></div></div></div>",
        classes.join(" "),
        topic_statuses(cx, topic),
        escape(&t_count(cx, "topic.reply_count_link", replies, &[])),
        number(replies),
        escape(&bumped_title),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn stat_period_needs_two_thirds() {
        let a = json!({"topics_week": 3, "topics_month": 3});
        let b = json!({"topics_week": 1, "topics_month": 1});
        let c = json!({"topics_week": 0, "topics_month": 0});
        assert_eq!(stat_period(&[&a, &b, &c]), "week");
        assert_eq!(stat_period(&[&a, &c, &c]), "all");
    }
}
