//! The full-page search (templates/full-page-search.gjs and its
//! controller), rendered as Ember renders it once the route's search came
//! back: the search types, the search bar, the context checkbox the mobile
//! search button asks for, the advanced filters' toggle, the sort order,
//! the result count and the results (SearchResultEntries, or the users
//! type's FPS user items). static/js/search-page.js runs the controls and
//! highlights the terms (HighlightSearch) as the client does.

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::topic_list_view::{
    ListContext, category_badge_html, escape, icon, long_date, relative_age_tiny, t, t_count,
    t_with, tags_html, topic_statuses_with,
};

/// `SEARCH_TYPE_DEFAULT`, `SEARCH_TYPE_CATS_TAGS`, `SEARCH_TYPE_USERS`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SearchType {
    TopicsPosts,
    CategoriesTags,
    Users,
}

impl SearchType {
    /// The `search_type` query param; anything else is the default type
    /// (`usingDefaultSearchType`).
    pub fn parse(value: Option<&str>) -> SearchType {
        match value {
            Some("categories_tags") => SearchType::CategoriesTags,
            Some("users") => SearchType::Users,
            _ => SearchType::TopicsPosts,
        }
    }

    fn id(self) -> &'static str {
        match self {
            SearchType::TopicsPosts => "topics_posts",
            SearchType::CategoriesTags => "categories_tags",
            SearchType::Users => "users",
        }
    }
}

/// The controller's `sortOrders`: id, name key and the term it appends.
const SORT_ORDERS: [(i64, &str, Option<&str>, Option<&str>); 6] = [
    (0, "search.relevance", None, None),
    (1, "search.latest_post", Some("order:latest"), Some("l")),
    (2, "search.most_liked", Some("order:likes"), None),
    (3, "search.most_viewed", Some("order:views"), None),
    (4, "search.latest_topic", Some("order:latest_topic"), None),
    (5, "search.last_read", Some("order:read"), Some("r")),
];

/// `cleanTerm`: the sort order a term asks for (its `order:` word or alias)
/// and the term without it; the default order when it names none.
pub fn clean_term(term: &str, default_order: i64) -> (String, i64) {
    let mut term = term.to_string();
    let mut order = default_order;
    for (id, _, word, alias) in SORT_ORDERS {
        let Some(word) = word else { continue };
        let has = |term: &str, w: &str| term.split_whitespace().any(|part| part == w);
        let matched = if has(&term, word) {
            Some(word)
        } else {
            alias.filter(|a| has(&term, a))
        };
        if let Some(w) = matched {
            order = id;
            // replace(/(^|\s)word($|\s)/g, "$1$2"), then trim.
            let re = regex::Regex::new(&format!(r"(^|\s){}($|\s)", regex::escape(w))).unwrap();
            term = re.replace_all(&term, "$1$2").trim().to_string();
        }
    }
    (term, order)
}

/// `highlightQuery`: the term's words but the `l` alias.
pub fn highlight_query(q: &str) -> String {
    q.split_whitespace()
        .filter(|w| *w != "l")
        .collect::<Vec<_>>()
        .join(" ")
}

/// What the page shows.
pub struct SearchView<'a> {
    /// `q` as the URL has it.
    pub q: &'a str,
    pub search_type: SearchType,
    /// `expanded=true`: the advanced filters open.
    pub expanded: bool,
    /// The route's search context, when the mobile search button passed
    /// one: its type, id and whether the checkbox is on.
    pub context: Option<(&'a str, &'a str, bool)>,
    /// The category's name for a category context.
    pub context_category_name: Option<String>,
    /// `search_default_sort_order`.
    pub default_order: i64,
    pub member: bool,
    pub staff: bool,
    /// `isValidSearchTerm(q)`: a search ran.
    pub search_active: bool,
    pub login_required: bool,
    /// The /search document for the default type, the users for the users
    /// type.
    pub doc: Option<&'a Value>,
    pub users: &'a [Value],
    pub page: i64,
    pub avatar_size: i64,
    /// `${location.protocol}//${location.host}${getURL("/")}`.
    pub site_url: String,
}

/// `search.result_count` with its `+` past a multiple of 50.
fn result_count_label(cx: &ListContext, count: i64, term: &str) -> String {
    let plus = if count % 50 == 0 { "+" } else { "" };
    t_count(
        cx,
        "search.result_count",
        count,
        &[
            ("count", &count.to_string()),
            ("plus", plus),
            ("term", term),
        ],
    )
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}

fn date(v: &Value) -> Option<DateTime<Utc>> {
    v.as_str()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc))
}

/// `avatar(post, {imageSize: "large"})` with its username title.
fn avatar(cx: &ListContext, template: &str, size: i64, title: &str) -> String {
    let src = template.replace("{size}", &size.to_string());
    let prefix = if src.starts_with('/') && !src.starts_with("//") {
        cx.base_path
    } else {
        ""
    };
    format!(
        "<img alt=\"\" width=\"48\" height=\"48\" src=\"{prefix}{}\" class=\"avatar\" title=\"{}\">",
        escape(&src),
        escape(title)
    )
}

/// The whole `section.search-container`.
pub fn render(cx: &ListContext, v: &SearchView) -> String {
    let (search_term, sort_order) = clean_term(v.q, v.default_order);
    let default_type = v.search_type == SearchType::TopicsPosts;
    let mut out = String::from(
        "<section class=\"search-container\"><div class=\"search-header\" role=\"search\">",
    );
    out.push_str(&format!(
        "<h1 class=\"search-page-heading sr-only\">{}</h1>",
        escape(&t(cx, "search.full_page_title"))
    ));
    // DHorizontalOverflowNav with the types.
    out.push_str(&format!(
        "<nav aria-label=\"{}\" class=\"horizontal-overflow-nav\"><div class=\"d-overflow-controls --owned-scroller horizontal-overflow-nav__controls\"><ul class=\"nav-pills action-list search-types\" data-d-scroll-axis=\"horizontal\">",
        escape(&t(cx, "search.type.label"))
    ));
    let types = [
        (SearchType::TopicsPosts, t(cx, "search.type.default")),
        (
            SearchType::CategoriesTags,
            t(cx, "search.type.categories_and_tags"),
        ),
        (SearchType::Users, t(cx, "search.type.users")),
    ];
    for (kind, name) in types {
        out.push_str(&format!(
            "<li><button class=\"search-types__type{}\" data-search-type=\"{}\" type=\"button\">{}</button></li>",
            if kind == v.search_type { " active" } else { "" },
            kind.id(),
            escape(&name)
        ));
    }
    out.push_str("</ul></div></nav>");
    // The search bar.
    let search_label = escape(&t(cx, "search.search_button"));
    out.push_str(&format!(
        "<div class=\"search-bar\"><div class=\"search-bar__field\"><input placeholder=\"{}\" aria-label=\"{}\" enterkeyhint=\"search\" autocapitalize=\"none\" autocorrect=\"off\" autocomplete=\"off\" id=\"search-page-term\" class=\"full-page-search search no-blur search-query\" type=\"search\" value=\"{}\"{}>",
        escape(&t(cx, "search.title")),
        escape(&t(cx, "search.search_term_label")),
        escape(&search_term),
        if v.q.is_empty() { " autofocus" } else { "" }
    ));
    if !search_term.is_empty() {
        let clear = escape(&t(cx, "search.clear_search"));
        out.push_str(&format!(
            "<a aria-label=\"{clear}\" class=\"clear-search\" href=\"\" title=\"{clear}\">{}</a>",
            icon("xmark", None)
        ));
    }
    out.push_str(&format!(
        "</div><button aria-label=\"{search_label}\" class=\"btn btn-icon-text btn-primary search-cta\" type=\"button\">{}<span class=\"d-button-label\">{search_label}</span></button></div>",
        icon("magnifying-glass", None)
    ));
    if default_type {
        if let Some((kind, id, enabled)) = v.context {
            let description = match kind {
                "topic" => t(cx, "search.context.topic"),
                "user" => t_with(cx, "search.context.user", &[("username", id)]),
                "category" => match &v.context_category_name {
                    Some(name) => t_with(cx, "search.context.category", &[("category", name)]),
                    None => String::new(),
                },
                "tag" => t_with(cx, "search.context.tag", &[("tag", id)]),
                "private_messages" => t(cx, "search.context.private_messages"),
                _ => String::new(),
            };
            out.push_str(&format!(
                "<div class=\"search-context\"><label><input name=\"searchContext\" type=\"checkbox\"{}>{}</label></div>",
                if enabled { " checked" } else { "" },
                escape(&description)
            ));
        }
        out.push_str("<div class=\"search-filters\">");
        out.push_str(&advanced_filters(cx, v.expanded));
        out.push_str("</div>");
    }
    out.push_str("<div class=\"search-notice\"></div></div><div class=\"search-advanced\">");

    let posts: Vec<&Value> = if default_type {
        v.doc
            .and_then(|d| d["posts"].as_array())
            .map(|p| p.iter().collect())
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let result_count = posts.len() as i64 + v.users.len() as i64;
    let has_results = result_count > 0;
    let no_sort_q = escape(&clean_term(v.q, v.default_order).0);
    if has_results {
        if default_type {
            out.push_str(&format!(
                "<div aria-label=\"{}\" class=\"search-info\" role=\"region\">",
                escape(&t(cx, "search.sort_or_bulk_actions"))
            ));
            if v.staff {
                out.push_str(&format!(
                    "<button class=\"btn no-text btn-icon btn-default bulk-select\" title=\"{}\" type=\"button\">{}<span aria-hidden=\"true\">\u{200b}</span></button>",
                    escape(&t(cx, "topics.bulk.toggle")),
                    icon("list", None)
                ));
            }
            out.push_str(&format!(
                "<div class=\"sort-by inline-form\"><label>{}</label>{}</div></div>",
                escape(&t(cx, "search.sort_by")),
                sort_chooser(cx, sort_order, v.member)
            ));
        }
        out.push_str(&format!(
            "<h2 aria-live=\"polite\" class=\"result-count\" id=\"search-result-count\">{}</h2>",
            result_count_label(cx, result_count, &no_sort_q)
        ));
    }
    let region = if v.q.is_empty() {
        t(cx, "search.results")
    } else {
        t_with(cx, "search.results_page", &[("term", v.q)])
    };
    out.push_str(&format!(
        "<div aria-label=\"{}\" class=\"search-results\" role=\"region\"><div>",
        escape(&region)
    ));
    let more = v
        .doc
        .is_some_and(|d| d["grouped_search_result"]["more_full_page_results"] == true);
    if default_type {
        out.push_str(&format!(
            "<div class=\"fps-result-entries\" role=\"list\" data-highlight=\"{}\">",
            escape(&highlight_query(v.q))
        ));
        let topics = v.doc.and_then(|d| d["topics"].as_array());
        for post in &posts {
            let topic = topics
                .and_then(|ts| ts.iter().find(|t| t["id"] == post["topic_id"]))
                .cloned()
                .unwrap_or(Value::Null);
            out.push_str(&entry(cx, v, post, &topic));
        }
        out.push_str("</div><div class=\"loading-container\">");
        out.push_str(&footer(cx, v, has_results, more));
        out.push_str("</div>");
    } else {
        out.push_str("<div class=\"loading-container\">");
        if has_results {
            out.push_str("<div class=\"user-items\">");
            for user in v.users {
                out.push_str(&user_item(cx, v, user));
            }
            out.push_str("</div>");
        } else if v.search_active {
            out.push_str(&format!("<h3>{}</h3>", escape(&t(cx, "search.no_results"))));
        }
        out.push_str("</div>");
    }
    // DLoadMore's sentinel, and the next page while there is one
    // (canLoadMore).
    let next = (default_type && more && v.page < 10).then(|| v.page + 1);
    out.push_str(&format!(
        "<div aria-hidden=\"true\" class=\"load-more-sentinel\"{}></div></div></div></div></section>",
        next.map(|p| format!(" data-next-page=\"{p}\""))
            .unwrap_or_default()
    ));
    out
}

/// The search results' footer: the error, no results with the Google
/// form, or whether there is more.
pub fn footer(cx: &ListContext, v: &SearchView, has_results: bool, more: bool) -> String {
    let mut out = String::new();
    if !has_results && v.search_active {
        out.push_str(&format!(
            "<div class=\"no-results-container\"><h3>{}</h3>",
            escape(&t(cx, "search.no_results"))
        ));
        // showSuggestion: canCreateTopic or not login_required. The
        // member's "start a new topic" link needs the composer's
        // createTopic, which the page doesn't open.
        if !v.login_required {
            out.push_str(&format!(
                "<div class=\"no-results-suggestion\">{}{}</div>",
                escape(&t(cx, "search.cant_find")),
                escape(&t(cx, "search.search_google"))
            ));
            out.push_str(&format!(
                "<div class=\"google-search-form\"><form action=\"//google.com/search\" class=\"inline-form\" id=\"google-search\"><input aria-label=\"{}\" name=\"q\" type=\"text\" value=\"{}\"><input name=\"as_sitesearch\" type=\"hidden\" value=\"{}\"><button class=\"btn btn-primary\" type=\"submit\">{}</button></form></div>",
                escape(&t(cx, "search.search_google")),
                escape(&clean_term(v.q, v.default_order).0),
                escape(&v.site_url),
                escape(&t(cx, "search.search_google_button"))
            ));
        }
        out.push_str("</div>");
    }
    if has_results {
        out.push_str("<h3 class=\"search-footer\">");
        if more {
            if v.page == 10 {
                out.push_str(&escape(&t(cx, "search.more_results")));
            }
        } else {
            out.push_str(&escape(&t(cx, "search.no_more_results")));
        }
        out.push_str("</h3>");
    }
    out
}

/// Topic#visited: `last_read_post_number >= highest_post_number` as JS
/// compares them (a null read is 0, a missing one never).
fn visited(topic: &Value) -> bool {
    let Some(read) = topic.get("last_read_post_number") else {
        return false;
    };
    let read = read.as_i64().unwrap_or(0);
    topic["highest_post_number"]
        .as_i64()
        .is_some_and(|highest| read >= highest)
}

/// SearchResultEntry.
pub fn entry(cx: &ListContext, v: &SearchView, post: &Value, topic: &Value) -> String {
    let username = s(&post["username"]);
    let slug = match s(&topic["slug"]) {
        "" => "topic",
        slug => slug,
    };
    let mut url = format!("{}/t/{slug}/{}", cx.base_path, post["topic_id"]);
    if let Some(n) = post["post_number"].as_i64().filter(|n| *n > 1) {
        url.push_str(&format!("/{n}"));
    }
    let mut out = format!(
        "<div role=\"listitem\" class=\"fps-result\"><div class=\"author\"><a data-user-card=\"{u}\" href=\"{}/u/{u}\">{}</a></div>",
        cx.base_path,
        avatar(cx, s(&post["avatar_template"]), v.avatar_size, username),
        u = escape(username),
    );
    out.push_str(&format!(
        "<div class=\"fps-topic\" data-topic-id=\"{}\"><div class=\"topic\"><a aria-level=\"2\" class=\"search-link{}\" href=\"{}\" role=\"heading\"><span class=\"topic-statuses\">{}</span><span class=\"topic-title\"><span>{}</span></span></a>",
        topic["id"],
        if visited(topic) { " visited" } else { "" },
        escape(&url),
        topic_statuses_with(cx, topic, true),
        crate::topic_list_view::emoji_unescape(
            s(&topic["fancy_title"]),
            &cx.settings.emojis,
            cx.base_path
        )
    ));
    out.push_str("<div class=\"search-category\">");
    if let Some(category) = topic["category_id"]
        .as_i64()
        .and_then(|id| cx.categories.get(&id))
    {
        if let Some(parent) = category.parent_id.and_then(|id| cx.categories.get(&id)) {
            out.push_str(&category_badge_html(cx, parent, true, false));
        }
        out.push_str(&category_badge_html(cx, category, true, true));
    }
    if !topic.is_null() {
        out.push_str(&tags_html(cx, topic, s(&topic["title"]), &[]));
    }
    out.push_str("<span></span></div></div>");
    // The blurb.
    let blurb = s(&post["blurb"]);
    out.push_str("<div class=\"blurb container\"><span class=\"date\">");
    if let Some(at) = date(&post["created_at"]) {
        out.push_str(&format!(
            "<span class=\"relative-date\" title=\"{}\" data-time=\"{}\" data-format=\"tiny\">{}</span>",
            long_date(cx, at),
            at.timestamp_millis(),
            relative_age_tiny(cx, at)
        ));
    }
    // The template's whitespace between the date, the separator and
    // the blurb.
    if !blurb.is_empty() {
        out.push_str(" <span class=\"separator\">-</span> ");
    }
    out.push_str("</span>");
    if !blurb.is_empty() {
        out.push_str(&format!(" <span>{blurb}</span>"));
    }
    out.push_str("</div>");
    // showLikeCount: an order:likes search.
    if v.q.contains("order:likes")
        && let Some(likes) = post["like_count"].as_i64().filter(|n| *n > 0)
    {
        out.push_str(&format!(
            "<span class=\"like-count\"><span class=\"value\">{likes}</span>{}</span>",
            icon("heart", None)
        ));
    }
    out.push_str("</div></div>");
    out
}

/// The users type's `DUserLink.fps-user-item`.
fn user_item(cx: &ListContext, v: &SearchView, user: &Value) -> String {
    let username = s(&user["username"]);
    let name = s(&user["name"]);
    let mut out = format!(
        "<a class=\"fps-user-item\" aria-label=\"{}\" data-user-card=\"{u}\" href=\"{}/u/{u}\" tabindex=\"0\">{}<div class=\"user-titles\">",
        escape(&t_with(
            cx,
            "user.profile_possessive",
            &[("username", username)]
        )),
        cx.base_path,
        avatar(cx, s(&user["avatar_template"]), v.avatar_size, username),
        u = escape(username),
    );
    if !name.is_empty() {
        out.push_str(&format!("<span class=\"name\">{}</span>", escape(name)));
    }
    out.push_str(&format!(
        "<span class=\"username\">{}</span></div></a>",
        escape(username)
    ));
    out
}

/// SearchAdvancedOptions' toggle. The filters it opens are not ported:
/// expanded, the toggle says so but the panel stays empty.
fn advanced_filters(cx: &ListContext, expanded: bool) -> String {
    let title = t(cx, "search.advanced.title");
    // DButton translates its @ariaLabel again, which the translated title
    // is not a key of.
    format!(
        "<div class=\"advanced-filters{}\"><button aria-expanded=\"{expanded}\" aria-label=\"[en.{title}]\" class=\"btn no-text advanced-filters__toggle btn-default\" type=\"button\">{title}{}</button></div>",
        if expanded { " --is-expanded" } else { "" },
        icon(
            if expanded {
                "chevron-up"
            } else {
                "chevron-down"
            },
            None
        ),
        title = escape(&title),
    )
}

/// The sort order's ComboBox (`#search-sort-by`), its rows for
/// search-page.js to open; Last Read for a member only.
fn sort_chooser(cx: &ListContext, selected: i64, member: bool) -> String {
    let mut rows = String::new();
    let mut selected_name = String::new();
    for (index, (id, key, _, _)) in SORT_ORDERS
        .iter()
        .filter(|(id, ..)| member || *id != 5)
        .enumerate()
    {
        let name = escape(&t(cx, key));
        let is = *id == selected;
        if is {
            selected_name = name.clone();
        }
        rows.push_str(&format!(
            "<li aria-checked=\"{is}\" role=\"menuitemradio\" data-index=\"{index}\" data-name=\"{name}\" data-value=\"{id}\" title=\"{name}\" tabindex=\"0\" class=\"select-kit-row{}\"><span class=\"name\">{name}</span></li>",
            if is { " is-highlighted is-selected" } else { "" }
        ));
    }
    let filter_by = escape(&t_with(
        cx,
        "select_kit.filter_by",
        &[("name", &selected_name)],
    ));
    format!(
        "<details id=\"search-sort-by\" class=\"select-kit single-select combobox combo-box has-selection\"><summary aria-label=\"{filter_by}\" name=\"{filter_by}\" data-name=\"{selected_name}\" data-value=\"{selected}\" tabindex=\"0\" id=\"search-sort-by-header\" class=\"select-kit-header single-select-header combo-box-header\"><div class=\"select-kit-header-wrapper\"><div class=\"select-kit-selected-name selected-name choice\" data-name=\"{selected_name}\" data-value=\"{selected}\" title=\"{selected_name}\"><span class=\"name\">{selected_name}</span></div>{}</div></summary><div id=\"search-sort-by-body\" class=\"select-kit-body\"><div class=\"select-kit-filter\"></div><ul aria-live=\"polite\" class=\"select-kit-collection\" role=\"menu\">{rows}</ul></div></details>",
        icon("angle-down", Some("angle-icon"))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_term_takes_the_order_out() {
        assert_eq!(clean_term("fixture", 0), ("fixture".into(), 0));
        assert_eq!(clean_term("fixture order:latest", 0), ("fixture".into(), 1));
        assert_eq!(clean_term("l fixture", 0), ("fixture".into(), 1));
        assert_eq!(clean_term("a order:likes b", 0), ("a  b".into(), 2));
        assert_eq!(clean_term("order:latest_topic x", 3), ("x".into(), 4));
    }

    #[test]
    fn the_highlight_leaves_out_the_latest_alias() {
        assert_eq!(highlight_query("l fixture  posts"), "fixture posts");
    }
}
