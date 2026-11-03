//! The search menu (components/search-menu.gjs and search-menu/*) as the
//! Ember client renders its results area for a state: the initial options
//! (a random quick tip, the recent searches, the page's search context),
//! the assistant's suggestions for `#`, `@` and the `in:`/`status:`/
//! `order:` shortcuts, and the results of a search (crate::search, as
//! /search/query returns them) by type. static/js/search-menu.js keeps the
//! state and asks GET /search-menu for this markup (routes::search_menu).

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::topic_list_view::{
    ListCategory, ListContext, category_badge_html, emoji_unescape, escape, icon,
    relative_age_tiny, t, t_with, topic_statuses_with,
};

/// The route's `search.searchContext`, as the page knows it.
#[derive(Debug, Clone, PartialEq)]
pub enum PageContext {
    Topic(i64),
    Category(i64),
    Tag(String),
    User(String),
    PrivateMessages(String),
}

/// `SearchMenu#matchesSuggestions`: what a term ending in `#`, `@` or a
/// shortcut keyword asks the assistant for.
#[derive(Debug, Clone, PartialEq)]
pub enum Suggestion {
    /// CATEGORY_SLUG_REGEXP's match, without its `#`.
    Category(String),
    /// USERNAME_REGEXP's match, without its `@`.
    Username(String),
    /// SUGGESTIONS_REGEXP's first match (the suggestion keyword).
    Shortcut(String),
}

/// The term's suggestion, unless the menu searches topics already.
pub fn matches_suggestions(term: &str) -> Option<Suggestion> {
    let term = term.trim();
    // CATEGORY_SLUG_REGEXP = /(\#[a-zA-Z0-9\-:]*)$/gi
    if let Some(m) = trailing(term, '#', |c| {
        c.is_ascii_alphanumeric() || c == '-' || c == ':'
    }) {
        return Some(Suggestion::Category(m[1..].to_string()));
    }
    // USERNAME_REGEXP = /(\@[a-zA-Z0-9\-\_]*)$/gi
    if let Some(m) = trailing(term, '@', |c| {
        c.is_ascii_alphanumeric() || c == '-' || c == '_'
    }) {
        return Some(Suggestion::Username(m[1..].to_string()));
    }
    // SUGGESTIONS_REGEXP = /(in:|status:|order:|:)([a-zA-Z]*)$/gi: the
    // earliest start that matches to the end.
    let lower = term.to_ascii_lowercase();
    for (start, _) in term.char_indices() {
        let rest = &lower[start..];
        for keyword in ["in:", "status:", "order:", ":"] {
            if let Some(after) = rest.strip_prefix(keyword)
                && after.chars().all(|c| c.is_ascii_alphabetic())
            {
                return Some(Suggestion::Shortcut(term[start..].to_string()));
            }
        }
    }
    None
}

/// The trailing `<sigil><allowed chars>*` of a term, the regex's match.
fn trailing(term: &str, sigil: char, allowed: impl Fn(char) -> bool) -> Option<&str> {
    let tail_len: usize = term
        .chars()
        .rev()
        .take_while(|c| allowed(*c))
        .map(char::len_utf8)
        .sum();
    let before = &term[..term.len() - tail_len];
    before
        .ends_with(sigil)
        .then(|| &term[before.len() - sigil.len_utf8()..])
}

/// MODIFIER_REGEXP = /.*(\#|\@|:).*$/gi: a term with any of them.
pub fn term_has_modifier(term: &str) -> bool {
    term.contains(['#', '@', ':'])
}

/// The menu's state for one render.
pub struct Menu<'a> {
    pub cx: &'a ListContext<'a>,
    pub mobile: bool,
    pub member: bool,
    pub log_search_queries: bool,
    pub tagging: bool,
    /// `search.activeGlobalSearchTerm`
    pub term: &'a str,
    pub context: Option<&'a PageContext>,
    /// `search.inTopicContext`
    pub in_topic: bool,
    /// `inPMInboxContext`: the inbox's context, not cleared.
    pub in_pm_inbox: bool,
    /// `includesTopics` as the results render: topic results, or a type
    /// filter other than exclude_topics.
    pub search_topics: bool,
    pub invalid_term: bool,
    pub no_results: bool,
    /// The recent searches (RecentSearches), newest first.
    pub recent: &'a [String],
    /// RandomQuickTip's pick: its index in `quick_tips`.
    pub quick_tip: usize,
    /// Whether the shortcut tip is drawn (a keyboard), and on Apple
    /// platforms.
    pub keyboard: bool,
    pub apple: bool,
}

/// What the assistant shows (`suggestionKeyword` and its results).
pub enum Assist<'a> {
    Categories {
        categories: Vec<&'a ListCategory>,
        tags: Vec<String>,
    },
    Users(Vec<Value>),
    Shortcuts {
        keyword: String,
        shortcuts: Vec<String>,
    },
}

/// One AssistantItem's arguments.
#[derive(Default)]
struct Item<'a> {
    type_class: &'a str,
    usage: Option<&'a str>,
    href: Option<String>,
    icon: Option<&'a str>,
    /// `@suggestionKeyword` (None is JS's undefined).
    keyword: Option<&'a str>,
    with_in_label: bool,
    category: Option<&'a ListCategory>,
    tag: Option<String>,
    user: Option<&'a Value>,
    label: Option<String>,
    suffix: Option<String>,
    extra_hint: bool,
    slug: Option<String>,
    concat_slug: bool,
    search_all_topics: bool,
}

impl Menu<'_> {
    fn t(&self, key: &str) -> String {
        t(self.cx, key)
    }

    /// The whole `.results` area inside DConditionalLoadingSection.
    pub fn results(&self, assist: Option<&Assist>, results: Option<&Value>) -> String {
        let mut out =
            String::from("<div class=\"conditional-loading-section\">\n<div class=\"results\">\n");
        out.push_str(&self.results_body(assist, results));
        out.push_str("\n</div>\n</div>");
        out
    }

    fn results_body(&self, assist: Option<&Assist>, results: Option<&Value>) -> String {
        // Results#inTopicContext: nothing until the topic is searched.
        if self.in_topic && !self.search_topics {
            return String::new();
        }
        if let Some(assist) = assist {
            return self.assistant(assist);
        }
        if self.search_topics && self.invalid_term {
            return format!(
                "<div class=\"no-results\">{}</div>",
                escape(&self.t("search.too_short"))
            );
        }
        if self.search_topics && self.no_results {
            return format!(
                "<div class=\"no-results\">{}</div>",
                escape(&self.t("search.no_results"))
            );
        }
        let render_initial = self.term.is_empty() && !self.in_pm_inbox;
        if render_initial {
            return self.initial_options();
        }
        let mut out = String::new();
        if !self.search_topics && !self.in_pm_inbox {
            out.push_str(&self.initial_options());
        }
        let types = results
            .map(|r| result_types(r, self.topic_context()))
            .unwrap_or_default();
        if self.search_topics && !types.is_empty() {
            out.push_str(&self.types(results.unwrap_or(&Value::Null), &types, true, false));
            out.push_str(&self.more_link(&types));
        } else if !self.in_pm_inbox && !self.search_topics && !types.is_empty() {
            out.push_str(&self.types(results.unwrap_or(&Value::Null), &types, false, true));
        }
        out
    }

    fn topic_context(&self) -> bool {
        self.in_topic && matches!(self.context, Some(PageContext::Topic(_)))
    }

    /// InitialOptions
    fn initial_options(&self) -> String {
        let mut out = String::from("<ul class=\"search-menu-initial-options\">\n");
        // The context's keyword, slug, label, suffix and results
        // (setAttributesForSearchContextType).
        let (keyword, slug, label, suffix, in_label): (
            Option<&str>,
            Option<String>,
            Option<String>,
            Option<String>,
            bool,
        ) = match self.context {
            Some(PageContext::Topic(_)) => (
                None,
                None,
                None,
                Some(self.t("search.in_this_topic")),
                false,
            ),
            Some(PageContext::PrivateMessages(_)) => (
                None,
                Some("in:messages".into()),
                Some("in:messages".into()),
                None,
                false,
            ),
            Some(PageContext::Category(id)) => {
                let slug = self.cx.categories.get(id).map(|c| self.full_slug(c));
                (Some("#"), slug, None, None, true)
            }
            Some(PageContext::Tag(name)) => (Some("#"), Some(format!("#{name}")), None, None, true),
            Some(PageContext::User(username)) => (
                Some("@"),
                Some(format!("@{username}")),
                None,
                Some(t_with(
                    self.cx,
                    "search.in_posts_by",
                    &[("username", username)],
                )),
                false,
            ),
            None => (None, None, None, None, false),
        };
        if term_has_modifier(self.term) {
            // termMatchesContextTypeKeyword
            out.push_str(&self.assistant_item(&Item {
                extra_hint: true,
                slug: slug.clone(),
                keyword,
                ..Default::default()
            }));
        } else if !self.term.is_empty() || self.context.is_some() {
            if !self.term.is_empty() {
                out.push_str(&self.assistant_item(&Item {
                    extra_hint: true,
                    search_all_topics: true,
                    suffix: Some(self.t("search.in_topics_posts")),
                    keyword,
                    ..Default::default()
                }));
            }
            if let Some(context) = self.context {
                match context {
                    PageContext::Category(id) => {
                        if let Some(category) = self.cx.categories.get(id) {
                            out.push_str(&self.category_or_tag_assistant(
                                &[category],
                                &[],
                                slug.as_deref(),
                                in_label,
                            ));
                        }
                    }
                    PageContext::Tag(name) => {
                        out.push_str(&self.category_or_tag_assistant(
                            &[],
                            std::slice::from_ref(name),
                            slug.as_deref(),
                            in_label,
                        ));
                    }
                    _ => out.push_str(&self.assistant_item(&Item {
                        label,
                        slug: slug.clone(),
                        suffix,
                        keyword,
                        with_in_label: in_label,
                        ..Default::default()
                    })),
                }
                // displayInitialOptions: no term, a context with options.
                let display_initial =
                    self.term.is_empty() && !matches!(context, PageContext::PrivateMessages(_));
                if self.member && self.log_search_queries && display_initial {
                    out.push_str(&self.recent_searches());
                }
            }
        } else {
            out.push_str(&self.random_quick_tip());
            if self.member && self.log_search_queries {
                out.push_str(&self.recent_searches());
            }
        }
        out.push_str("\n</ul>");
        out
    }

    /// The category's `#slug`, with its parent's (`#parent:child`).
    fn full_slug(&self, category: &ListCategory) -> String {
        match category
            .parent_id
            .and_then(|id| self.cx.categories.get(&id))
        {
            Some(parent) => format!("#{}:{}", parent.slug, category.slug),
            None => format!("#{}", category.slug),
        }
    }

    /// RecentSearches: the terms, then Clear Recent Searches.
    fn recent_searches(&self) -> String {
        if self.recent.is_empty() {
            return String::new();
        }
        let mut out = String::from("<div class=\"search-menu-recent\">\n");
        for term in self.recent {
            out.push_str(&self.assistant_item(&Item {
                concat_slug: true,
                icon: Some("magnifying-glass"),
                label: Some(term.clone()),
                slug: Some(term.clone()),
                usage: Some("recent-search"),
                ..Default::default()
            }));
        }
        out.push_str(&format!(
            "\n<button class=\"btn btn-transparent btn-small clear-recent-searches\" type=\"button\">\n<span class=\"d-button-label\">{}</span>\n\n</button>\n</div>\n",
            escape(&self.t("search.clear_recent"))
        ));
        out
    }

    /// RandomQuickTip's tips (DEFAULT_QUICK_TIPS), the shortcut one only
    /// with a keyboard: label, description, clickable, shortcut.
    pub fn quick_tips(&self) -> Vec<(&'static str, String, bool, bool)> {
        let mut tips = vec![
            ("#", self.t("search.tips.category_tag"), true, false),
            ("@", self.t("search.tips.author"), true, false),
            ("in:", self.t("search.tips.in"), true, false),
            ("status:", self.t("search.tips.status"), true, false),
            ("mod+enter", self.t("search.tips.full_search"), false, true),
            ("@me", self.t("search.tips.me"), false, false),
        ];
        if !self.keyboard {
            tips.retain(|tip| !tip.3);
        }
        tips
    }

    fn random_quick_tip(&self) -> String {
        let tips = self.quick_tips();
        let Some((label, description, clickable, shortcut)) =
            tips.get(self.quick_tip % tips.len().max(1))
        else {
            return String::new();
        };
        let label = if *shortcut {
            self.shortcut_keys()
        } else {
            escape(label)
        };
        format!(
            "<li class=\"search-random-quick-tip\">\n<button aria-describedby=\"tip-description\" class=\"tip-label{}\">\n{label}\n</button>\n\n<span id=\"tip-description\">\n{}\n</span>\n</li>\n",
            if *clickable { " tip-clickable" } else { "" },
            escape(description)
        )
    }

    /// DShortcut for `mod+enter`: Ctrl (a ⌘ glyph on Apple platforms)
    /// and Enter as keycaps.
    fn shortcut_keys(&self) -> String {
        let key = |label: &str, name: &str, glyph: bool| {
            let class = if glyph {
                "d-shortcut__key d-shortcut__key--glyph"
            } else {
                "d-shortcut__key"
            };
            if label == name {
                format!("<kbd class=\"{class}\">\n{}\n</kbd>", escape(label))
            } else {
                format!(
                    "<kbd class=\"{class}\">\n<span aria-hidden=\"true\">{}</span>\n<span class=\"sr-only\">{}</span>\n</kbd>",
                    escape(label),
                    escape(name)
                )
            }
        };
        let modifier = if self.apple {
            key("\u{2318}", &self.t("shortcut_modifier_key.command"), true)
        } else {
            let ctrl = self.t("shortcut_modifier_key.ctrl");
            key(&ctrl, &ctrl, false)
        };
        let enter = self.t("shortcut_modifier_key.enter");
        format!(
            "<kbd class=\"d-shortcut\" dir=\"ltr\">\n{modifier}\n{}\n</kbd>",
            key(&enter, &enter, false)
        )
    }

    /// AssistantItem#prefix
    fn item_prefix(&self, keyword: Option<&str>) -> String {
        if keyword == Some("+") {
            return self.term.to_string();
        }
        // An undefined keyword is looked up as the string "undefined".
        let keyword = keyword.unwrap_or("undefined");
        let term = self.term;
        let prefix = match js_last_index_of(term, keyword) {
            Some(0) => String::new(),
            Some(i) => term[..i].trim().to_string(),
            None => term.trim().to_string(),
        };
        if prefix.is_empty() {
            prefix
        } else {
            format!("{prefix} ")
        }
    }

    /// AssistantItem
    fn assistant_item(&self, item: &Item) -> String {
        let classes = |base: &str| {
            if item.type_class.is_empty() {
                base.to_string()
            } else {
                format!("{} {base}", item.type_class)
            }
        };
        let prefix = self.item_prefix(item.keyword);
        // itemSelected's updatedTerm, and whether it searches every topic,
        // for search-menu.js.
        let updated = match &item.slug {
            Some(slug) if item.keyword.is_some() || item.concat_slug => format!("{prefix}{slug}"),
            _ => prefix.trim().to_string(),
        };
        let mut out = format!(
            "<li class=\"{}\"{} data-term=\"{}\"{}>\n<a class=\"{}\" href=\"{}\">\n<span aria-label=\"{}\" class=\"search-icon-wrapper\">\n{}\n</span>\n<span class=\"search-item-wrapper\">\n",
            classes("search-menu-assistant-item"),
            item.usage
                .map(|u| format!(" data-usage=\"{}\"", escape(u)))
                .unwrap_or_default(),
            escape(&updated),
            if item.search_all_topics {
                " data-all-topics"
            } else {
                ""
            },
            classes("search-link"),
            escape(item.href.as_deref().unwrap_or("#")),
            escape(&self.t("search.title")),
            icon(item.icon.unwrap_or("magnifying-glass"), None),
        );
        if !prefix.is_empty() {
            out.push_str(&format!(
                "<span class=\"search-item-prefix\">\n{}\n</span>\n",
                escape(&prefix)
            ));
        }
        if item.with_in_label {
            out.push_str(&format!(
                "<span class=\"label-suffix\">{}</span>\n",
                escape(&self.t("search.in"))
            ));
        }
        if let Some(category) = item.category {
            out.push_str(&category_badge_html(self.cx, category, false, false));
            out.push('\n');
        } else if let Some(tag) = &item.tag {
            out.push_str(&format!(
                "<span class=\"search-item-tag\">\n{}\n</span>\n",
                self.tag(tag)
            ));
        } else if let Some(user) = item.user {
            out.push_str(&format!(
                "<span class=\"search-item-user\">\n{}\n</span>\n",
                self.user(user, false)
            ));
        }
        if item.label.is_some() || item.suffix.is_some() {
            out.push_str("<span class=\"search-item-slug\">\n");
            if let Some(suffix) = &item.suffix {
                out.push_str(&format!(
                    "<span class=\"label-suffix\">{}</span>\n",
                    escape(suffix)
                ));
            }
            if let Some(label) = &item.label {
                out.push_str(&escape(label));
                out.push('\n');
            }
            out.push_str("</span>\n");
        }
        if item.extra_hint {
            let key = if self.mobile {
                "search.mobile_enter_hint"
            } else {
                "search.enter_hint"
            };
            out.push_str(&format!(
                "<span class=\"extra-hint\">\n{}\n</span>\n",
                escape(&self.t(key))
            ));
        }
        out.push_str("</span>\n</a>\n</li>\n");
        out
    }

    /// Assistant: the suggestions for the term's keyword.
    fn assistant(&self, assist: &Assist) -> String {
        match assist {
            Assist::Categories { categories, tags } => {
                self.category_or_tag_assistant(categories, tags, None, false)
            }
            Assist::Users(users) => {
                let mut out = String::from("<ul class=\"search-menu-assistant\">\n");
                // userMatchesInTopic: one user on a topic page offers the
                // topic and every topic.
                let in_topic_page = matches!(self.context, Some(PageContext::Topic(_)));
                if users.len() == 1 && in_topic_page {
                    let user = &users[0];
                    let slug = format!("@{}", user["username"].as_str().unwrap_or_default());
                    out.push_str(&self.assistant_item(&Item {
                        type_class: "user",
                        extra_hint: true,
                        search_all_topics: true,
                        slug: Some(slug.clone()),
                        suffix: Some(self.t("search.in_topics_posts")),
                        keyword: Some("@"),
                        user: Some(user),
                        ..Default::default()
                    }));
                    out.push_str(&self.assistant_item(&Item {
                        type_class: "user",
                        slug: Some(slug),
                        suffix: Some(self.t("search.in_this_topic")),
                        keyword: Some("@"),
                        user: Some(user),
                        ..Default::default()
                    }));
                } else {
                    for user in users {
                        out.push_str(&self.assistant_item(&Item {
                            type_class: "user",
                            search_all_topics: true,
                            slug: Some(format!(
                                "@{}",
                                user["username"].as_str().unwrap_or_default()
                            )),
                            keyword: Some("@"),
                            user: Some(user),
                            ..Default::default()
                        }));
                    }
                }
                out.push_str("</ul>");
                out
            }
            Assist::Shortcuts { keyword, shortcuts } => {
                let mut out = String::from("<ul class=\"search-menu-assistant\">\n");
                for shortcut in shortcuts {
                    out.push_str(&self.assistant_item(&Item {
                        type_class: "shortcut",
                        label: Some(shortcut.clone()),
                        search_all_topics: true,
                        slug: Some(shortcut.clone()),
                        keyword: Some(keyword),
                        ..Default::default()
                    }));
                }
                out.push_str("</ul>");
                out
            }
        }
    }

    /// Assistant's categoryOrTag branch.
    fn category_or_tag_assistant(
        &self,
        categories: &[&ListCategory],
        tags: &[String],
        slug: Option<&str>,
        with_in_label: bool,
    ) -> String {
        let mut out = String::from("<ul class=\"search-menu-assistant\">\n");
        // Assistant#prefix: what the slug has before its `#`.
        let prefix = slug
            .map(|s| s.split('#').next().unwrap_or("").trim())
            .filter(|p| !p.is_empty())
            .map(|p| format!("{p} "))
            .unwrap_or_default();
        for category in categories {
            out.push_str(&self.assistant_item(&Item {
                type_class: "category",
                category: Some(category),
                href: Some(self.category_url(category)),
                search_all_topics: true,
                keyword: Some("#"),
                with_in_label,
                slug: Some(format!("{prefix}{}", self.full_slug(category))),
                ..Default::default()
            }));
        }
        for tag in tags {
            out.push_str(&self.assistant_item(&Item {
                type_class: "tag",
                tag: Some(tag.clone()),
                search_all_topics: true,
                keyword: Some("#"),
                with_in_label,
                slug: Some(format!("{prefix}#{tag}")),
                ..Default::default()
            }));
        }
        out.push_str("</ul>");
        out
    }

    /// Category#url
    fn category_url(&self, category: &ListCategory) -> String {
        let mut path = category.slug.clone();
        if let Some(parent) = category
            .parent_id
            .and_then(|id| self.cx.categories.get(&id))
        {
            path = format!("{}/{path}", parent.slug);
        }
        format!("{}/c/{path}/{}", self.cx.base_path, category.id)
    }

    /// The tag type: its icon and the tag as a span (dDiscourseTag).
    fn tag(&self, name: &str) -> String {
        format!("{}\n{}", icon("tag", None), self.discourse_tag(name))
    }

    fn discourse_tag(&self, name: &str) -> String {
        let mut classes = String::from("discourse-tag");
        if !self.cx.settings.tag_style.is_empty() {
            classes.push(' ');
            classes.push_str(&self.cx.settings.tag_style);
        }
        format!(
            "<span data-tag-name=\"{}\" class=\"{classes}\">{}</span>",
            escape(&name.to_lowercase()),
            escape(name)
        )
    }

    /// The user type: the small avatar and the names.
    fn user(&self, user: &Value, display_name_with_user: bool) -> String {
        let username = user["username"].as_str().unwrap_or_default();
        let template = user["avatar_template"].as_str().unwrap_or_default();
        let size = self.cx.settings.avatar_size_24;
        let src = template.replace("{size}", &size.to_string());
        let prefix = if src.starts_with('/') && !src.starts_with("//") {
            self.cx.base_path
        } else {
            ""
        };
        let mut out = format!(
            "<img alt=\"\" width=\"24\" height=\"24\" src=\"{prefix}{}\" class=\"avatar\" title=\"{}\">\n<div class=\"user-titles\">\n",
            escape(&src),
            escape(username)
        );
        if display_name_with_user
            && let Some(name) = user["name"].as_str().filter(|n| !n.is_empty())
        {
            out.push_str(&format!("<span class=\"name\">{}</span>\n", escape(name)));
        }
        out.push_str(&format!(
            "<span class=\"username\">\n{}\n</span>\n",
            escape(username)
        ));
        for row in user["custom_data"].as_array().into_iter().flatten() {
            out.push_str(&format!(
                "<span class=\"custom-field\">{}: {}</span>\n",
                escape(row["name"].as_str().unwrap_or_default()),
                escape(row["value"].as_str().unwrap_or_default())
            ));
        }
        out.push_str("</div>");
        out
    }

    /// Types: each result type's list.
    fn types(
        &self,
        results: &Value,
        types: &[ResultType],
        topic_results_only: bool,
        display_name_with_user: bool,
    ) -> String {
        let mut out = String::new();
        for kind in types {
            if topic_results_only != (kind.kind == "topic") {
                continue;
            }
            out.push_str(&format!(
                "<div class=\"search-result-{}\">\n<ul aria-label=\"{} {}\" class=\"list\">\n",
                kind.component,
                escape(&self.t("search.results")),
                kind.kind
            ));
            for item in results[kind.list].as_array().into_iter().flatten() {
                let (href, body, id) = match kind.component {
                    "topic" | "post" => {
                        let topic = results["topics"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .find(|t| t["id"] == item["topic_id"])
                            .cloned()
                            .unwrap_or(Value::Null);
                        let body = if kind.component == "topic" {
                            self.topic_result(item, &topic)
                        } else {
                            self.post_result(item)
                        };
                        (
                            post_url(self.cx.base_path, item, &topic),
                            body,
                            item["id"].to_string(),
                        )
                    }
                    "user" => (
                        format!(
                            "{}/u/{}",
                            self.cx.base_path,
                            item["username"].as_str().unwrap_or_default().to_lowercase()
                        ),
                        self.user(item, display_name_with_user),
                        item["id"].to_string(),
                    ),
                    "category" => {
                        let category = item["id"]
                            .as_i64()
                            .and_then(|id| self.cx.categories.get(&id));
                        match category {
                            Some(category) => (
                                self.category_url(category),
                                category_badge_html(self.cx, category, false, false),
                                category.id.to_string(),
                            ),
                            None => continue,
                        }
                    }
                    "tag" => {
                        let name = item["name"].as_str().unwrap_or_default();
                        let id = item["id"].as_i64().unwrap_or_default();
                        let slug = item["slug"]
                            .as_str()
                            .filter(|s| !s.is_empty())
                            .map(str::to_string)
                            .unwrap_or_else(|| format!("{id}-tag"));
                        (
                            format!("{}/tag/{slug}/{id}", self.cx.base_path),
                            self.tag(name),
                            name.to_string(),
                        )
                    }
                    _ => {
                        let name = item["name"].as_str().unwrap_or_default();
                        (
                            format!("{}/g/{}", self.cx.base_path, name),
                            self.group(item),
                            item["id"].to_string(),
                        )
                    }
                };
                // The result's id and type ride along for the click log
                // (logSearchLinkClick).
                out.push_str(&format!(
                    "\n<li class=\"item\" data-result-id=\"{}\" data-result-type=\"{}\">\n<a class=\"search-link\" href=\"{}\">\n{body}\n</a>\n</li>\n",
                    escape(&id),
                    kind.kind,
                    escape(&href)
                ));
            }
            out.push_str("</ul>\n</div>\n");
        }
        out
    }

    /// The topic type (search-menu/results/type/topic).
    fn topic_result(&self, post: &Value, topic: &Value) -> String {
        let title = emoji_unescape(
            topic["fancy_title"].as_str().unwrap_or_default(),
            &self.cx.settings.emojis,
            self.cx.base_path,
        );
        let mut out = format!(
            "<span class=\"topic\">\n<span class=\"first-line\">\n<span class=\"topic-statuses\">{}</span>\n<span class=\"topic-title\" data-topic-id=\"{}\">\n<span>\n{title}\n</span>\n</span>\n</span>\n<span class=\"second-line\">\n",
            topic_statuses_with(self.cx, topic, true),
            topic["id"]
        );
        if let Some(category) = topic["category_id"]
            .as_i64()
            .and_then(|id| self.cx.categories.get(&id))
        {
            out.push_str(&category_badge_html(self.cx, category, false, false));
        }
        out.push('\n');
        if self.tagging {
            out.push_str(&self.tags(topic));
        }
        out.push_str("\n</span>\n</span>\n");
        out.push_str(&self.blurb(post));
        out
    }

    /// `renderTags` with `tagName: "span"`.
    fn tags(&self, topic: &Value) -> String {
        let tags: Vec<String> = topic["tags"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|t| {
                t.as_str()
                    .map(str::to_string)
                    .or_else(|| t["name"].as_str().map(str::to_string))
            })
            .collect();
        if tags.is_empty() {
            return String::new();
        }
        let separator = "<span class=\"discourse-tags__tag-separator\">,</span>";
        let mut out = format!(
            "<ul class=\"discourse-tags\" aria-label=\"{}\">",
            escape(&self.t("tagging.tags"))
        );
        for (i, tag) in tags.iter().enumerate() {
            out.push_str("<li>");
            out.push_str(&self.discourse_tag(tag));
            if i + 1 < tags.len() {
                out.push_str(separator);
            }
            out.push_str("</li>");
        }
        out.push_str("</ul>");
        out
    }

    /// The post type, a topic's search results: `#N by user` and the blurb.
    fn post_result(&self, post: &Value) -> String {
        let number = post["post_number"].to_string();
        let username = post["username"].as_str().unwrap_or_default();
        format!(
            "{}\n{}",
            escape(&t_with(
                self.cx,
                "search.post_format",
                &[("post_number", &number), ("username", username)]
            )),
            self.blurb(post)
        )
    }

    /// Blurb: the date and the blurb, highlighted by search-menu.js.
    fn blurb(&self, post: &Value) -> String {
        let mut out = String::from("<span class=\"blurb\">\n");
        if let Some(at) = post["created_at"]
            .as_str()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|d| d.with_timezone(&Utc))
        {
            out.push_str(&format!(
                "<span class=\"relative-date\"{} data-time=\"{}\" data-format=\"tiny\">{}</span>\n",
                crate::viewer_dates::long_title(self.cx, at),
                at.timestamp_millis(),
                relative_age_tiny(self.cx, at)
            ));
        }
        let blurb = emoji_unescape(
            post["blurb"].as_str().unwrap_or_default(),
            &self.cx.settings.emojis,
            self.cx.base_path,
        );
        out.push_str(&format!(
            "<span class=\"blurb__separator\"> - </span>\n<span class=\"blurb__text\">\n<span>\n{blurb}\n</span>\n</span>\n</span>"
        ));
        out
    }

    /// The group type.
    fn group(&self, group: &Value) -> String {
        let name = group["name"].as_str().unwrap_or_default();
        let full_name = group["full_name"]
            .as_str()
            .or_else(|| group["display_name"].as_str())
            .filter(|n| !n.is_empty());
        let mut out = format!(
            "<div class=\"group-result \">\n{}\n<div class=\"group-names {}\">\n<span class=\"name\">{}</span>\n",
            icon("users", None),
            if full_name.is_some() {
                "--group-with-slug"
            } else {
                ""
            },
            escape(full_name.unwrap_or(name))
        );
        if full_name.is_some() {
            out.push_str(&format!("<div class=\"slug\">{}</div>\n", escape(name)));
        }
        out.push_str("</div>\n</div>");
        out
    }

    /// MoreLink: the full page's url when there are more topics.
    fn more_link(&self, types: &[ResultType]) -> String {
        let Some(topics) = types.iter().find(|t| t.kind == "topic") else {
            return String::new();
        };
        let mut out = String::from("<div class=\"search-menu__show-more\">\n");
        let more = escape(&self.t("more"));
        if topics.component == "topic" && topics.more {
            out.push_str(&format!(
                "<a class=\"filter search-link\" href=\"{}\">\n{more}...\n</a>\n",
                escape(&self.full_search_url(false))
            ));
        } else if topics.more {
            out.push_str(&format!(
                "<a class=\"filter search-link\">\n{more}...\n</a>\n"
            ));
        }
        out.push_str("</div>\n");
        out
    }

    /// SearchMenu#fullSearchUrl
    pub fn full_search_url(&self, expanded: bool) -> String {
        let mut params: Vec<(String, String)> = Vec::new();
        if !self.term.is_empty() {
            let mut q = self.term.to_string();
            match self.context {
                Some(PageContext::Topic(id)) if self.in_topic => {
                    q.push_str(&format!(" topic:{id}"))
                }
                Some(PageContext::PrivateMessages(_)) if self.in_pm_inbox => {
                    q.push_str(" in:messages")
                }
                _ => {}
            }
            params.push(("q".into(), q));
        }
        if expanded {
            params.push(("expanded".into(), "true".into()));
        }
        let mut url = format!("{}/search", self.cx.base_path);
        if !params.is_empty() {
            url.push('?');
            url.push_str(
                &form_urlencoded::Serializer::new(String::new())
                    .extend_pairs(params)
                    .finish(),
            );
        }
        url
    }
}

/// `String#lastIndexOf` in UTF-16 terms, as a byte offset.
fn js_last_index_of(haystack: &str, needle: &str) -> Option<usize> {
    haystack.rfind(needle)
}

/// One of translateGroupedSearchResults' result types.
#[derive(Debug)]
pub struct ResultType {
    /// topic, category, tag, user, group
    pub kind: &'static str,
    /// The component: the kind, or `post` for a topic's search.
    pub component: &'static str,
    /// The results' key in the document.
    pub list: &'static str,
    pub more: bool,
}

/// translateGroupedSearchResults: the types with results, in display order.
pub fn result_types(results: &Value, topic_context: bool) -> Vec<ResultType> {
    let grouped = &results["grouped_search_result"];
    if grouped.is_null() {
        return Vec::new();
    }
    [
        ("topic", "posts"),
        ("category", "categories"),
        ("tag", "tags"),
        ("user", "users"),
        ("group", "groups"),
    ]
    .into_iter()
    .filter(|(_, list)| results[*list].as_array().is_some_and(|a| !a.is_empty()))
    .map(|(kind, list)| ResultType {
        kind,
        component: if topic_context && kind == "topic" {
            "post"
        } else {
            kind
        },
        list,
        more: grouped[format!("more_{list}")] == true,
    })
    .collect()
}

/// Post#url: the topic's url, with the post number past the first.
fn post_url(base_path: &str, post: &Value, topic: &Value) -> String {
    let slug = match topic["slug"].as_str().map(str::trim) {
        Some(slug) if !slug.is_empty() => slug.to_string(),
        _ => "topic".into(),
    };
    let mut url = format!("{base_path}/t/{slug}/{}", post["topic_id"]);
    if let Some(n) = post["post_number"].as_i64().filter(|n| *n > 1) {
        url.push_str(&format!("/{n}"));
    }
    url
}

/// The suggestion shortcuts (Assistant's `suggestionShortcuts`): the
/// defaults, the bundled plugins' (poll, solved, topic voting), a
/// member's, and the tagging ones; the ones containing the term's last
/// word, eight at most.
pub fn suggestion_shortcuts(
    term: &str,
    member: bool,
    tagging: bool,
    plugins: &[&str],
) -> Vec<String> {
    let mut all: Vec<&str> = vec![
        "in:title",
        "in:pinned",
        "status:open",
        "status:closed",
        "status:public",
        "status:noreplies",
        "order:latest",
        "order:views",
        "order:likes",
        "order:latest_topic",
    ];
    let add = |s: &'static str, all: &mut Vec<&str>| {
        if !all.contains(&s) {
            all.push(s);
        }
    };
    for plugin in plugins {
        match *plugin {
            "poll" => add("in:polls", &mut all),
            "solved" => {
                add("status:solved", &mut all);
                add("status:unsolved", &mut all);
            }
            "voting" => add("order:votes", &mut all),
            _ => {}
        }
    }
    if member {
        for s in [
            "in:likes",
            "in:bookmarks",
            "in:mine",
            "in:messages",
            "in:seen",
            "in:tracking",
            "in:unseen",
            "in:watching",
        ] {
            add(s, &mut all);
        }
    }
    if tagging {
        add("in:tagged", &mut all);
        add("in:untagged", &mut all);
    }
    let last = term.split(' ').next_back().unwrap_or_default();
    all.into_iter()
        .filter(|s| s.contains(last))
        .take(8)
        .map(str::to_string)
        .collect()
}

/// `organizeResults` for the menu's `@` suggestions: the users and groups
/// ranked (exact username, exact group name, partial username, partial
/// group name, exact name, partial name, group full names, then the users
/// matched by other data), cut to the limit, and the users of what is left.
pub fn organize_users(
    users: Vec<Value>,
    groups: &[Value],
    term: Option<&str>,
    limit: usize,
) -> Vec<Value> {
    enum Entry {
        User(usize),
        Group,
    }
    let term = term
        .and_then(|t| t.trim().split(char::is_whitespace).next())
        .filter(|t| !t.is_empty())
        .map(str::to_lowercase);
    let includes = |s: Option<&str>| -> bool {
        match (&term, s) {
            (Some(term), Some(s)) if !s.is_empty() => s.to_lowercase().contains(term.as_str()),
            _ => false,
        }
    };
    let name = |v: &Value, key: &str| v[key].as_str().map(str::to_string);
    let metadata: Vec<bool> = users
        .iter()
        .map(|u| {
            term.is_some() && !includes(u["username"].as_str()) && !includes(u["name"].as_str())
        })
        .collect();
    let mut seen_users: Vec<usize> = Vec::new();
    let mut seen_groups: Vec<usize> = Vec::new();
    let mut order: Vec<Entry> = Vec::new();
    let eq = |s: Option<String>| -> bool {
        match (&term, s) {
            (Some(term), Some(s)) => s.to_lowercase() == *term,
            _ => false,
        }
    };
    // 1. exact usernames
    for (i, u) in users.iter().enumerate() {
        if eq(name(u, "username")) {
            seen_users.push(i);
            order.push(Entry::User(i));
        }
    }
    // 2. exact group names
    for (i, g) in groups.iter().enumerate() {
        if eq(name(g, "name")) {
            seen_groups.push(i);
            order.push(Entry::Group);
        }
    }
    // 3. partial usernames
    for (i, u) in users.iter().enumerate() {
        if seen_users.contains(&i)
            || metadata[i]
            || (term.is_some() && !includes(u["username"].as_str()))
        {
            continue;
        }
        seen_users.push(i);
        order.push(Entry::User(i));
    }
    // 4. partial group names
    for (i, g) in groups.iter().enumerate() {
        if seen_groups.contains(&i) || (term.is_some() && !includes(g["name"].as_str())) {
            continue;
        }
        seen_groups.push(i);
        order.push(Entry::Group);
    }
    // 5. exact names
    for (i, u) in users.iter().enumerate() {
        if seen_users.contains(&i) || metadata[i] {
            continue;
        }
        let exact = match (&term, u["name"].as_str()) {
            (Some(term), Some(n)) => n.to_lowercase() == *term,
            (None, None) => true,
            _ => false,
        };
        if !exact {
            continue;
        }
        seen_users.push(i);
        order.push(Entry::User(i));
    }
    // 6. partial names
    for (i, u) in users.iter().enumerate() {
        if seen_users.contains(&i) || !includes(u["name"].as_str()) || metadata[i] {
            continue;
        }
        seen_users.push(i);
        order.push(Entry::User(i));
    }
    // 7. and 8. group full names
    for exact in [true, false] {
        for (i, g) in groups.iter().enumerate() {
            let Some(full) = g["full_name"].as_str().filter(|f| !f.is_empty()) else {
                continue;
            };
            if seen_groups.contains(&i) {
                continue;
            }
            let hit = if exact {
                term.as_ref().is_some_and(|t| full.to_lowercase() == *t)
            } else {
                includes(Some(full))
            };
            if hit {
                seen_groups.push(i);
                order.push(Entry::Group);
            }
        }
    }
    // 9. metadata matches
    for (i, m) in metadata.iter().enumerate() {
        if *m {
            order.push(Entry::User(i));
        }
    }
    order
        .into_iter()
        .take(limit)
        .filter_map(|e| match e {
            Entry::User(i) => Some(users[i].clone()),
            Entry::Group => None,
        })
        .collect()
}

/// lib/user-search's skipSearch for the menu (no emails, not the last
/// seen users): a term with `@`, punctuation, a double space or only a
/// space, or a `+` without a later `@`.
pub fn skip_user_search(term: &str) -> bool {
    if term.contains('@') {
        return true;
    }
    let punctuation = |c: char| {
        ('\u{2000}'..='\u{206F}').contains(&c)
            || ('\u{2E00}'..='\u{2E7F}').contains(&c)
            || "\\'!\"#$%&()*,/:;<=>?[]^`{|}~".contains(c)
    };
    if term.chars().any(punctuation) || term.contains("  ") || term == " " {
        return true;
    }
    // ^[^+]*\+[^@]*$
    if let Some(i) = term.find('+') {
        return !term[i + 1..].contains('@');
    }
    false
}

/// `Category.search(term, { limit })` over the site's categories in
/// `sortedCategories` order (by topic count, each parent followed by its
/// children): with no term the top-level ones; else the ones whose name or
/// slug starts with it, then the ones containing it; the read-restricted
/// ones last.
pub fn category_search<'a>(
    sorted: &[&'a ListCategory],
    term: &str,
    limit: usize,
) -> Vec<&'a ListCategory> {
    let empty = term.is_empty();
    let normalize = |s: &str| remove_accents(&s.to_lowercase());
    let (term, slug_term) = if empty {
        (String::new(), String::new())
    } else {
        let t = normalize(term);
        (t.replace('-', " "), t)
    };
    let mut data: Vec<&ListCategory> = Vec::new();
    for c in sorted {
        if data.len() == limit {
            break;
        }
        let hit = if empty {
            c.parent_id.is_none()
        } else {
            normalize(&c.name).starts_with(&term) || normalize(&c.slug).starts_with(&slug_term)
        };
        if hit {
            data.push(c);
        }
    }
    if data.len() != limit {
        for c in sorted {
            if data.len() == limit {
                break;
            }
            let index_after = |hay: &str, needle: &str| hay.find(needle).is_some_and(|i| i > 0);
            let hit = (!empty && index_after(&normalize(&c.name), &term))
                || index_after(&normalize(&c.slug), &slug_term);
            if hit && !data.iter().any(|d| d.id == c.id) {
                data.push(c);
            }
        }
    }
    // A stable sort by read_restricted, false first.
    data.sort_by_key(|c| c.read_restricted);
    data
}

/// lib/utilities' removeAccents: NFD, then the combining marks out.
fn remove_accents(s: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    s.nfd()
        .filter(|c| !('\u{0300}'..='\u{036f}').contains(c))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggestions_follow_the_regexps() {
        assert_eq!(
            matches_suggestions("foo #gen"),
            Some(Suggestion::Category("gen".into()))
        );
        assert_eq!(
            matches_suggestions("#"),
            Some(Suggestion::Category(String::new()))
        );
        assert_eq!(
            matches_suggestions("@us"),
            Some(Suggestion::Username("us".into()))
        );
        assert_eq!(
            matches_suggestions("x in:"),
            Some(Suggestion::Shortcut("in:".into()))
        );
        assert_eq!(
            matches_suggestions("x status:op"),
            Some(Suggestion::Shortcut("status:op".into()))
        );
        assert_eq!(
            matches_suggestions("x :"),
            Some(Suggestion::Shortcut(":".into()))
        );
        assert_eq!(matches_suggestions("fixture"), None);
        assert_eq!(matches_suggestions("a#b c"), None);
    }

    #[test]
    fn shortcuts_filter_on_the_last_word() {
        let s = suggestion_shortcuts("x in:", true, true, &["poll", "solved", "voting"]);
        assert_eq!(
            s,
            [
                "in:title",
                "in:pinned",
                "in:polls",
                "in:likes",
                "in:bookmarks",
                "in:mine",
                "in:messages",
                "in:seen"
            ]
        );
        let s = suggestion_shortcuts("x order:", false, false, &["voting"]);
        assert_eq!(
            s,
            [
                "order:latest",
                "order:views",
                "order:likes",
                "order:latest_topic",
                "order:votes"
            ]
        );
    }

    #[test]
    fn user_search_skips_punctuation() {
        assert!(skip_user_search("a@b"));
        assert!(skip_user_search("a  b"));
        assert!(skip_user_search("a+b"));
        assert!(!skip_user_search("us"));
        assert!(!skip_user_search(""));
    }

    #[test]
    fn users_rank_exact_then_partial_then_metadata() {
        let users = vec![
            serde_json::json!({"username": "zed", "name": "Us Person"}),
            serde_json::json!({"username": "user1", "name": null}),
            serde_json::json!({"username": "us", "name": null}),
            serde_json::json!({"username": "other", "name": "Other"}),
        ];
        let ranked = organize_users(users, &[], Some("us"), 6);
        let names: Vec<&str> = ranked
            .iter()
            .map(|u| u["username"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["us", "user1", "zed", "other"]);
    }
}
