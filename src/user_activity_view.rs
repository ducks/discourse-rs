//! The activity tabs the user stream doesn't draw: the topics a user
//! created and the ones read (user-topics-list.gjs over a TopicList), the
//! drafts (user/stream.gjs over UserDraftsStream, PostList with its drafts
//! actions and bulk select), the pending posts (user-activity/pending.gjs)
//! and the bookmarks (user/bookmarks.gjs, BookmarkList).

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::topic_list_view::{
    ListContext, escape, icon, long_date, relative_age_tiny, t, t_count, t_with,
};
use crate::user_profile_view::age_medium;

/// DLoadMore's tail.
const LOAD_MORE: &str = "<div class=\"loading-container\"></div><div aria-hidden=\"true\" class=\"load-more-sentinel\"></div>";

/// DLoadMore's tail where there is a next page: the sentinel says which
/// (user-activity.js asks for it when it scrolls into view).
fn load_more(next_page: Option<i64>) -> String {
    match next_page {
        Some(page) => format!(
            "<div class=\"loading-container\"></div><div aria-hidden=\"true\" class=\"load-more-sentinel\" data-next-page=\"{page}\"></div>"
        ),
        None => LOAD_MORE.to_string(),
    }
}

/// How a topic list is sorted: the `order` query param, if any, and
/// `ascending`.
pub struct Sort<'a> {
    pub order: Option<&'a str>,
    pub ascending: bool,
}

fn date(v: &Value) -> Option<DateTime<Utc>> {
    v.as_str()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc))
}

fn button(class: &str, icon_name: &str, title: &str) -> String {
    format!(
        "<button class=\"btn no-text btn-icon {class}\" title=\"{}\" type=\"button\">{}<span aria-hidden=\"true\">&#8203;</span></button>",
        escape(title),
        icon(icon_name, None)
    )
}

fn empty_state(title: &str, body: &str) -> String {
    let mut out = format!(
        "<div class=\"empty-state__container --text-only\"><div class=\"empty-state\"><div class=\"empty-state__title\" data-test-title>{}</div>",
        escape(title)
    );
    if !body.is_empty() {
        out.push_str(&format!(
            "<div class=\"empty-state__body\"><p data-test-body>{body}</p></div>"
        ));
    }
    out.push_str("</div></div>");
    out
}

/// user-topics-list.gjs: a topic list without its posters, or the
/// route's empty state (`body` is HTML).
pub fn topic_list(
    cx: &ListContext,
    doc: &Value,
    empty_title: &str,
    empty_body: &str,
    bulk_select: bool,
    sort: &Sort,
    next_page: Option<i64>,
) -> String {
    let topics: &[Value] = doc["topic_list"]["topics"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut out = String::from("<section class=\"user-content\" id=\"user-content\">");
    if topics.is_empty() {
        out.push_str(&empty_state(empty_title, empty_body));
        out.push_str("</section>");
        return out;
    }
    let users: &[Value] = doc["users"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let rows: String = topics
        .iter()
        .map(|topic| crate::topic_list_view::row_without_posters(cx, topic, users))
        .collect();
    // SortableColumn: the column the list is ordered by says so.
    let sortable = |class: &str, key: &str| {
        if sort.order == Some(class) {
            let (aria, chevron) = if sort.ascending {
                ("ascending", "chevron-up")
            } else {
                ("descending", "chevron-down")
            };
            format!(
                "<th aria-sort=\"{aria}\" class=\"topic-list-data {class} sortable sorting num\" data-sort-order=\"{class}\" scope=\"col\"><button aria-pressed=\"\">{} {}</button></th>",
                escape(&t(cx, key)),
                icon(chevron, None)
            )
        } else {
            format!(
                "<th class=\"topic-list-data {class} sortable num\" data-sort-order=\"{class}\" scope=\"col\"><button>{}</button></th>",
                escape(&t(cx, key))
            )
        }
    };
    out.push_str(&format!(
        "<div class=\"paginated-topics-list\"><div class=\"row dismiss-container-top\"></div><div><div class=\"loading-container\">\
         <table class=\"topic-list\"><caption class=\"sr-only\">{}</caption><thead class=\"topic-list-header\"><tr>\
         <th class=\"topic-list-data default\" data-sort-order=\"default\" scope=\"col\">{}<span>{}</span></th>\
         {}{}{}</tr></thead><tbody class=\"topic-list-body\">{rows}</tbody></table></div></div>{}</div></section>",
        escape(&t(cx, "sr_topic_list_caption")),
        if bulk_select {
            format!(
                "<button class=\"btn-transparent bulk-select no-text\" title=\"{}\">{}</button> ",
                escape(&t(cx, "topics.bulk.toggle")),
                icon("list-check", None)
            )
        } else {
            String::new()
        },
        escape(&t(cx, "topic.title")),
        sortable("posts", "replies"),
        sortable("views", "views"),
        sortable("activity", "activity"),
        load_more(next_page),
    ));
    out
}

/// What user-activity.js says: the confirmations, bulk controls and
/// toasts of the drafts, pending and bookmarks tabs, and the dialog's
/// buttons.
pub fn strings(cx: &ListContext) -> String {
    let other = |key: &str| t(cx, &format!("{key}.other"));
    let one = |key: &str| t_count(cx, key, 1, &[("count", "1")]);
    let strings = serde_json::json!({
        "drafts_remove_confirmation": t(cx, "drafts.remove_confirmation"),
        "drafts_bulk_delete_confirmation": t(cx, "drafts.bulk_delete_confirmation"),
        "drafts_bulk_delete_message_one": one("drafts.bulk_delete_message"),
        "drafts_bulk_delete_message_other": other("drafts.bulk_delete_message"),
        "drafts_bulk_delete": t(cx, "drafts.bulk_delete"),
        "drafts_label": t(cx, "drafts.label"),
        "drafts_label_with_count": t(cx, "drafts.label_with_count"),
        "post_list_selected_one": one("post_list.bulk.selected"),
        "post_list_selected_other": other("post_list.bulk.selected"),
        "post_list_clear_all": t(cx, "post_list.bulk.clear_all"),
        "post_list_select_all": t(cx, "post_list.bulk.select_all"),
        "post_list_actions": t(cx, "post_list.bulk.actions"),
        "review_delete_confirm": t(cx, "review.delete_confirm"),
        "bookmarks_confirm_delete": t(cx, "bookmarks.confirm_delete"),
        "bookmarks_selected_one": one("bookmarks.bulk.selected_count"),
        "bookmarks_selected_other": other("bookmarks.bulk.selected_count"),
        "bookmarks_select_all": t(cx, "bookmarks.bulk.select_all"),
        "bookmarks_clear_all": t(cx, "bookmarks.bulk.clear_all"),
        "bookmarks_toggle": t(cx, "bookmarks.bulk.toggle"),
        "topic_title": t(cx, "topic.title"),
        "bookmarks_bulk_title": t(cx, "select_kit.components.bulk_select_bookmarks_dropdown.title"),
        "bookmarks_clear_reminders": t(cx, "bookmark_bulk_actions.clear_reminders.name"),
        "bookmarks_delete_bookmarks": t(cx, "bookmark_bulk_actions.delete_bookmarks.name"),
        "bookmarks_clear_reminders_one": one("bookmark_bulk_actions.clear_reminders.description"),
        "bookmarks_clear_reminders_other": other("bookmark_bulk_actions.clear_reminders.description"),
        "bookmarks_delete_bookmarks_one": one("bookmark_bulk_actions.delete_bookmarks.description"),
        "bookmarks_delete_bookmarks_other": other("bookmark_bulk_actions.delete_bookmarks.description"),
        "bookmarks_reminders_cleared": t(cx, "bookmarks.bulk.reminders_cleared"),
        "bookmarks_delete_completed": t(cx, "bookmarks.bulk.delete_completed"),
        "close": t(cx, "close"),
    });
    let dialog = serde_json::json!({
        "ok_value": t(cx, "ok_value"),
        "cancel": t(cx, "cancel_value"),
        "yes_value": t(cx, "yes_value"),
        "no_value": t(cx, "no_value"),
        "delete": t(cx, "delete"),
        "modal.close": t(cx, "modal.close"),
    });
    format!(
        "<script type=\"application/json\" id=\"user-activity-strings\">{}</script><script type=\"application/json\" id=\"dialog-strings\">{}</script>",
        strings.to_string().replace('<', "\\u003c"),
        dialog.to_string().replace('<', "\\u003c")
    )
}

/// A draft the stream lists: drafts#index's row, its data parsed, the
/// excerpt of its cooked reply (UserDraftsStream#findItems).
pub struct Draft<'a> {
    pub row: &'a Value,
    pub data: Value,
    pub excerpt: String,
}

/// user/stream.gjs over UserDraftsStream: the PostList of drafts with
/// bulk select, resume and remove, or the empty state.
pub fn drafts(cx: &ListContext, drafts: &[Draft], viewer_username: &str) -> String {
    let mut out = String::from("<section class=\"user-content\" id=\"user-content\">");
    if drafts.is_empty() {
        out.push_str(&empty_state(
            &t(cx, "user_activity.no_drafts_title"),
            &escape(&t(cx, "user_activity.no_drafts_body")),
        ));
        out.push_str(&format!(
            "<div><div class=\"post-list user-stream\"><div class=\"post-list__empty-text\">{}</div></div>{LOAD_MORE}</div></section>",
            escape(&t(cx, "post_list.empty"))
        ));
        return out;
    }
    out.push_str("<div><div class=\"post-list post-list--bulk-select user-stream\">");
    for d in drafts {
        let row = d.row;
        let key = row["draft_key"].as_str().unwrap_or_default();
        let draft_icon = if key.starts_with("new_private_message") {
            "envelope"
        } else if key.starts_with("new_topic") {
            "layer-group"
        } else {
            "reply"
        };
        // TopicStatus: a draft's topic's closed and archived icons.
        let mut statuses = String::new();
        if row["closed"] == true {
            statuses.push_str(&format!(
                "<span class=\"topic-status\">{}</span>",
                icon("lock", None)
            ));
        }
        if row["archived"] == true {
            statuses.push_str(&format!(
                "<span class=\"topic-status\">{}</span>",
                icon("lock", None)
            ));
        }
        let post_number = d.data["postId"].as_i64();
        let title_html = match row["topic_id"].as_i64() {
            Some(topic_id) => {
                let slug = row["slug"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .unwrap_or("topic");
                let mut url = format!("{}/t/{slug}/{topic_id}", cx.base_path);
                if let Some(n) = post_number.filter(|n| *n > 1) {
                    url.push_str(&format!("/{n}"));
                }
                let title = row["title"].as_str().unwrap_or_default();
                let aria = post_number
                    .map(|n| {
                        format!(
                            " aria-label=\"{}\"",
                            escape(&t_with(
                                cx,
                                "post_list.aria_post_number",
                                &[("title", title), ("postNumber", &n.to_string())]
                            ))
                        )
                    })
                    .unwrap_or_default();
                format!("<a{aria} href=\"{}\">{}</a>", escape(&url), escape(title))
            }
            None => {
                let title = row["title"]
                    .as_str()
                    .or_else(|| d.data["title"].as_str())
                    .map(str::to_string)
                    .unwrap_or_else(|| t(cx, "drafts.dropdown.untitled"));
                format!(
                    "<button class=\"btn no-text btn-transparent draft-title\" type=\"button\">{}</button>",
                    escape(&title)
                )
            }
        };
        let category = d.data["categoryId"]
            .as_i64()
            .and_then(|id| cx.categories.get(&id))
            .map(|c| {
                format!(
                    "<span class=\"category stream-post-category\">{}</span>",
                    crate::topic_list_view::category_badge(cx, c)
                )
            })
            .unwrap_or_default();
        let time = date(&row["created_at"])
            .map(|at| age_medium(cx, at, true))
            .unwrap_or_default();
        let actions = if row["draft_username"].as_str() == Some(viewer_username) {
            format!(
                "<div class=\"user-stream-item-draft-actions\">{}{}</div>",
                button(
                    "btn-default resume-draft",
                    "pencil",
                    &t(cx, "drafts.resume")
                ),
                button(
                    "btn-danger remove-draft",
                    "trash-can",
                    &t(cx, "drafts.remove")
                ),
            )
        } else {
            String::new()
        };
        out.push_str(&format!(
            "<div class=\"post-list-item user-stream-item\" data-draft-key=\"{}\" data-draft-sequence=\"{}\"{}><div class=\"post-list-item__header info\"><div class=\"post-list-item__bulk-select\"><input class=\"bulk-select-checkbox\" type=\"checkbox\"></div><div class=\"draft-icon\">{}</div><div class=\"post-list-item__details\"><div class=\"stream-topic-title\"><span class=\"topic-statuses\">{statuses}</span><span class=\"title\">{title_html}</span></div><div class=\"post-list-item__metadata\">{category}<span class=\"time\">\n{time}</span></div></div>{actions}</div><div class=\"excerpt\"{} data-user-id=\"{}\"><div class=\"cooked\">{}</div></div></div>",
            escape(key),
            row["sequence"],
            row["topic_id"]
                .as_i64()
                .map(|_| String::new())
                .unwrap_or_else(|| " data-resume=\"composer\"".to_string()),
            icon(draft_icon, Some("icon")),
            row["topic_id"]
                .as_i64()
                .map(|id| format!(" data-topic-id=\"{id}\""))
                .unwrap_or_default(),
            row["user_id"],
            d.excerpt,
        ));
    }
    out.push_str(&format!("</div>{LOAD_MORE}</div></section>"));
    out
}

/// UserDraftsStream's `excerpt(cooked, 300)` (lib/text.js): the cooked
/// reply's text, escaped, its links and emoji images kept, other images
/// `[image]`, cut at `length` with `&hellip;`.
pub fn client_excerpt(cooked: &str, length: usize) -> String {
    use html5ever::tendril::TendrilSink;
    use html5ever::{ParseOpts, QualName, local_name, namespace_url, ns, parse_fragment};
    use markup5ever_rcdom::{Handle, NodeData, RcDom, SerializableHandle};

    fn encode(s: &str) -> String {
        s.replace('<', "&lt;").replace('>', "&gt;")
    }
    fn text_of(handle: &Handle, out: &mut String) {
        match &handle.data {
            NodeData::Text { contents } => out.push_str(&contents.borrow()),
            _ => {
                for child in handle.children.borrow().iter() {
                    text_of(child, out);
                }
            }
        }
    }
    fn outer(handle: &Handle) -> String {
        let mut bytes = Vec::new();
        let node: SerializableHandle = handle.clone().into();
        let _ = html5ever::serialize(
            &mut bytes,
            &node,
            html5ever::serialize::SerializeOpts {
                traversal_scope: html5ever::serialize::TraversalScope::IncludeNode,
                ..Default::default()
            },
        );
        String::from_utf8_lossy(&bytes).into_owned()
    }
    fn walk(handle: &Handle, length: usize, out: &mut String, len: &mut usize) {
        for child in handle.children.borrow().iter() {
            if *len >= length {
                return;
            }
            match &child.data {
                NodeData::Text { contents } => {
                    let text = contents.borrow().to_string();
                    let count = text.encode_utf16().count();
                    if *len + count > length {
                        let keep: String = text.chars().take(length - *len).collect();
                        out.push_str(&encode(&keep));
                        out.push_str("&hellip;");
                        *len += keep.encode_utf16().count();
                    } else {
                        out.push_str(&encode(&text));
                        *len += count;
                    }
                }
                NodeData::Element { name, attrs, .. } if name.local == local_name!("a") => {
                    out.push_str(&outer(child));
                    let mut inner = String::new();
                    text_of(child, &mut inner);
                    *len += inner.encode_utf16().count();
                }
                NodeData::Element { name, attrs, .. } if name.local == local_name!("img") => {
                    let emoji = attrs.borrow().iter().any(|a| {
                        a.name.local == local_name!("class")
                            && a.value.split_whitespace().any(|c| c == "emoji")
                    });
                    if emoji {
                        out.push_str(&outer(child));
                    } else {
                        out.push_str("[image]");
                        *len += "[image]".len();
                    }
                }
                NodeData::Element { .. } => walk(child, length, out, len),
                _ => {}
            }
        }
    }
    let dom = parse_fragment(
        RcDom::default(),
        ParseOpts::default(),
        QualName::new(None, ns!(html), local_name!("div")),
        vec![],
    )
    .one(cooked);
    let mut out = String::new();
    let mut len = 0;
    // The fragment's root is an <html> wrapping the parsed nodes.
    for root in dom.document.children.borrow().iter() {
        walk(root, length, &mut out, &mut len);
    }
    out
}

/// user-activity/pending.gjs: PostList of PendingPostSerializer rows, the
/// delete button on one's own.
pub fn pending(cx: &ListContext, posts: &[(Value, String)], can_delete: bool) -> String {
    let mut out = String::from(
        "<section class=\"user-content\" id=\"user-content\"><ul class=\"user-stream\"><div><div class=\"post-list user-stream\">",
    );
    if posts.is_empty() {
        out.push_str(&format!(
            "<div class=\"post-list__empty-text\">{}</div>",
            escape(&t(cx, "post_list.empty"))
        ));
    }
    for (p, cooked) in posts {
        let topic_url = p["topic_url"].as_str();
        // UserActivityPending#model's emojiUnescape(escapeExpression(title)),
        // which the item then draws as text.
        let title = escape(&crate::topic_list_view::emoji_unescape(
            &escape(p["title"].as_str().unwrap_or_default()),
            &cx.settings.emojis,
            cx.base_path,
        ));
        let title_html = match topic_url {
            Some(url) => format!("<a href=\"{}\">{title}</a>", escape(url)),
            None => title,
        };
        let category = p["category_id"]
            .as_i64()
            .and_then(|id| cx.categories.get(&id))
            .map(|c| {
                format!(
                    "<span class=\"category stream-post-category\">{}</span>",
                    crate::topic_list_view::category_badge(cx, c)
                )
            })
            .unwrap_or_default();
        let time = date(&p["created_at"])
            .map(|at| age_medium(cx, at, true))
            .unwrap_or_default();
        let username = p["username"].as_str().unwrap_or_default();
        let avatar = crate::user_profile_view::avatar(
            cx,
            p["avatar_template"].as_str().unwrap_or_default(),
            48,
        )
        .replace(
            "class=\"avatar\"",
            &format!("class=\"avatar actor\" title=\"{}\"", escape(username)),
        );
        // PendingPost#expandedExcerpt: the raw text cooked on the client.
        out.push_str(&format!(
            "<div class=\"post-list-item user-stream-item\" data-pending-id=\"{id}\"><div class=\"post-list-item__header info\"><a class=\"avatar-link\" data-user-card=\"{u}\" href=\"{}/u/{}\"><div class=\"avatar-wrapper\">{avatar}</div></a><div class=\"post-list-item__details\"><div class=\"stream-topic-title\"><span class=\"topic-statuses\"></span><span class=\"title\">{title_html}</span></div><div class=\"post-list-item__metadata\">{category}<span class=\"time\">\n{time}</span></div></div></div><div class=\"excerpt\" data-post-id=\"{id}\"{}><div class=\"cooked\">{cooked}</div></div>",
            cx.base_path,
            escape(&username.to_lowercase()),
            p["topic_id"]
                .as_i64()
                .map(|id| format!(" data-topic-id=\"{id}\""))
                .unwrap_or_default(),
            id = p["id"],
            u = escape(username),
        ));
        if can_delete {
            out.push_str(&format!(
                "<div class=\"reviewable-actions\"><button class=\"btn btn-icon-text btn-danger\" type=\"button\">{}<span class=\"d-button-label\">{}</span></button></div>",
                icon("trash-can", None),
                escape(&t(cx, "review.delete"))
            ));
        }
        out.push_str("</div>");
    }
    out.push_str(&format!("</div>{LOAD_MORE}</div></ul></section>"));
    out
}

/// Ember's `capitalize`: the first character upper cased.
fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// `formattedReminderTime(reminder_at, timezone)`
fn formatted_reminder(cx: &ListContext, at: DateTime<Utc>, zone: chrono_tz::Tz) -> String {
    use chrono::TimeZone;
    let local = at.with_timezone(&zone);
    let wall = Utc.from_utc_datetime(&local.naive_local());
    let now = cx.now.with_timezone(&zone).date_naive();
    let format = |f: &str| {
        crate::pretty_text::render::local_dates::format_utc(wall, &t(cx, f)).unwrap_or_default()
    };
    if local.date_naive() == now + chrono::Duration::days(1) {
        t_with(
            cx,
            "bookmarks.reminders.tomorrow_with_time",
            &[("time", &format("dates.time"))],
        )
    } else if local.date_naive() == now {
        t_with(
            cx,
            "bookmarks.reminders.today_with_time",
            &[("time", &format("dates.time"))],
        )
    } else {
        t_with(
            cx,
            "bookmarks.reminders.at_time",
            &[("date_time", &format("dates.long_with_year"))],
        )
    }
}

/// What the bookmarks tab shows.
pub enum Bookmarks<'a> {
    /// The list request failed (another member's bookmarks).
    Denied,
    /// No bookmarks at all (userDoesNotHaveBookmarks).
    None,
    /// A search that found none.
    NothingFound,
    List(&'a [Value]),
}

/// user/bookmarks.gjs with BookmarkList.
pub fn bookmarks(
    cx: &ListContext,
    shown: &Bookmarks,
    search: &str,
    zone: chrono_tz::Tz,
    mobile: bool,
    next_page: Option<i64>,
) -> String {
    let mut out = String::from(
        "<section class=\"user-content\" id=\"user-content\"><div class=\"loading-container\">",
    );
    if let Bookmarks::Denied = shown {
        out.push_str(&format!(
            "<div class=\"alert alert-info\">{}</div></div></section>",
            escape(&t(cx, "bookmarks.list_permission_denied"))
        ));
        return out;
    }
    if let Bookmarks::None = shown {
        out.push_str(&empty_state(
            &t(cx, "user.no_bookmarks_title"),
            &t_with(
                cx,
                "user.no_bookmarks_body",
                &[("icon", &icon("bookmark", None))],
            ),
        ));
        out.push_str("</div></section>");
        return out;
    }
    out.push_str(&format!(
        "<div class=\"inline-form full-width bookmark-search-form\"><input id=\"bookmark-search\" class=\"ember-text-field\" autocomplete=\"off\" placeholder=\"{}\" type=\"text\"{}><button class=\"btn no-text btn-icon btn-primary\" type=\"button\">{}<span aria-hidden=\"true\">&#8203;</span></button></div>",
        escape(&t(cx, "bookmarks.search_placeholder")),
        if search.is_empty() {
            String::new()
        } else {
            format!(" value=\"{}\"", escape(search))
        },
        icon("magnifying-glass", None)
    ));
    let list = match shown {
        Bookmarks::NothingFound => {
            out.push_str(&format!(
                "<div class=\"alert alert-info\">{}</div></div></section>",
                escape(&t(cx, "user.no_bookmarks_search"))
            ));
            return out;
        }
        Bookmarks::List(list) => *list,
        Bookmarks::None | Bookmarks::Denied => &[],
    };
    out.push_str("<div class=\"bookmark-list-wrapper\"><div class=\"loading-container\"><div><table class=\"topic-list bookmark-list\"><thead class=\"topic-list-header\"><tr>");
    if !mobile {
        out.push_str(&format!(
            "<th class=\"topic-list-data\">{}{}</th><th class=\"topic-list-data\">&nbsp;</th><th class=\"post-metadata topic-list-data\">{}</th><th class=\"post-metadata topic-list-data\">{}</th><th>&nbsp;</th>",
            button("btn-flat bulk-select", "list-check", &t(cx, "bookmarks.bulk.toggle")),
            escape(&t(cx, "topic.title")),
            escape(&t(cx, "post.bookmarks.updated")),
            escape(&t(cx, "activity"))
        ));
    }
    out.push_str("</tr></thead><tbody class=\"topic-list-body\">");
    for b in list {
        out.push_str(&bookmark_row(cx, b, zone, mobile));
    }
    out.push_str(&format!(
        "</tbody></table></div>{}</div></div></div></section>",
        load_more(next_page)
    ));
    out
}

fn bookmark_row(cx: &ListContext, b: &Value, zone: chrono_tz::Tz, mobile: bool) -> String {
    let reminder = date(&b["reminder_at"]);
    let name = b["name"].as_str().filter(|n| !n.is_empty());
    let excerpt = b["excerpt"].as_str().filter(|e| !e.is_empty());
    let has_metadata = reminder.is_some() || name.is_some();
    let mut classes = String::from("topic-list-item bookmark-list-item");
    if excerpt.is_some() {
        classes.push_str(" excerpt-expanded");
    }
    if has_metadata {
        classes.push_str(" has-metadata");
    }
    let mut main =
        String::from("<td class=\"main-link topic-list-data\"><span class=\"link-top-line\">");
    if has_metadata {
        main.push_str("<div class=\"bookmark-metadata\">");
        if let Some(at) = reminder {
            main.push_str(&format!(
                "<span class=\"bookmark-metadata-item bookmark-reminder{}\">{}{}</span>",
                if at < cx.now {
                    " bookmark-expired-reminder"
                } else {
                    ""
                },
                icon("far-clock", None),
                escape(&capitalize(&formatted_reminder(cx, at, zone)))
            ));
        }
        if let Some(name) = name {
            main.push_str(&format!(
                "<span class=\"bookmark-metadata-item\">{}<span>{}</span></span>",
                icon("circle-info", None),
                crate::topic_list_view::emoji_unescape(
                    &escape(name),
                    &cx.settings.emojis,
                    cx.base_path
                )
            ));
        }
        main.push_str("</div>");
    }
    main.push_str("<div class=\"bookmark-status-with-link\">");
    if b["pinned"] == true {
        main.push_str(&icon("thumbtack", Some("bookmark-pinned")));
    }
    let topic_alike = matches!(b["bookmarkable_type"].as_str(), Some("Post" | "Topic"));
    if topic_alike {
        // UserActivityBookmarksController#transform's topicStatus.
        let status = serde_json::json!({
            "closed": b["closed"],
            "archived": b["archived"],
            "visible": b["invisible"] != true,
        });
        let statuses = crate::topic_list_view::topic_statuses_with(cx, &status, false);
        // dTopicLink(bookmark.topicForList): the linked post, else the
        // last unread one; the topic carries no slug.
        let topic_id = b["topic_id"].as_i64().unwrap_or(0);
        let highest = b["highest_post_number"].as_i64().unwrap_or(0);
        let last_read = b["last_read_post_number"].as_i64().unwrap_or(0);
        let linked = b["linked_post_number"]
            .as_i64()
            .filter(|_| b["bookmarkable_type"] != "Topic")
            .filter(|n| *n != 0);
        let post_number = linked.unwrap_or_else(|| (last_read + 1).min(highest));
        let mut url = format!("{}/t/topic/{topic_id}", cx.base_path);
        if post_number > 0 {
            url.push_str(&format!("/{post_number}"));
        }
        let read_suffix = if last_read >= highest && b["last_read_post_number"].is_i64() {
            format!(
                "<span class=\"sr-only\">&nbsp;{}</span>",
                escape(&t(cx, "topic.sr_read"))
            )
        } else {
            String::new()
        };
        main.push_str(&format!(
            "<span class=\"topic-statuses\">{statuses}</span><a href=\"{url}\" class=\"title\" data-topic-id=\"{topic_id}\">{}{read_suffix}</a>",
            crate::topic_list_view::emoji_unescape(
                b["fancy_title"].as_str().unwrap_or_default(),
                &cx.settings.emojis,
                cx.base_path
            )
        ));
    } else {
        main.push_str(&format!(
            "<a aria-level=\"2\" class=\"title\" href=\"{}\" role=\"heading\">{}</a>",
            escape(b["bookmarkable_url"].as_str().unwrap_or_default()),
            b["fancy_title"].as_str().unwrap_or_default()
        ));
    }
    main.push_str("</div></span>");
    if topic_alike {
        let category = b["category_id"]
            .as_i64()
            .and_then(|id| cx.categories.get(&id))
            .map(|c| crate::topic_list_view::category_badge(cx, c))
            .unwrap_or_default();
        main.push_str(&format!(
            "<div class=\"link-bottom-line\">{category}{}</div>",
            crate::topic_list_view::tags_html(cx, b, "", &[])
        ));
    }
    let user = &b["user"];
    let avatar_link = |size: u32| {
        user["avatar_template"]
            .as_str()
            .map(|template| {
                let u = escape(user["username"].as_str().unwrap_or_default());
                format!(
                    "<a class=\"avatar\" data-user-card=\"{u}\" href=\"{}/u/{}\">{}</a>",
                    cx.base_path,
                    u.to_lowercase(),
                    crate::user_profile_view::avatar(cx, template, size).replace(
                        "class=\"avatar\"",
                        &format!("class=\"avatar\" title=\"{u}\"")
                    ),
                )
            })
            .unwrap_or_default()
    };
    // The mobile avatar names bookmark.bookmarkableUser, which the model
    // lacks: an empty link.
    if mobile && excerpt.is_some() && user["avatar_template"].is_string() {
        main.push_str(&format!(
            "<a class=\"avatar\" data-user-card=\"{}\"></a>",
            escape(user["username"].as_str().unwrap_or_default())
        ));
    }
    if let Some(excerpt) = excerpt {
        main.push_str(&format!("<p class=\"post-excerpt\">{excerpt}</p>"));
    }
    main.push_str("</td>");
    let mut cells = main;
    if !mobile {
        cells.push_str(&format!(
            "<td class=\"author-avatar topic-list-data\">{}</td>",
            avatar_link(48)
        ));
        let updated = date(&b["updated_at"]);
        cells.push_str(&format!(
            "<td class=\"post-metadata topic-list-data updated-at\">{}</td>",
            updated
                .map(|at| format!(
                    "<span class=\"relative-date\" title=\"{}\" data-time=\"{}\" data-format=\"tiny\">{}</span>",
                    escape(&long_date(cx, at)),
                    at.timestamp_millis(),
                    escape(&relative_age_tiny(cx, at))
                ))
                .unwrap_or_default()
        ));
        // Bookmark#lastPostUrl
        let last_post_url = match (b["topic_id"].as_i64(), b["highest_post_number"].as_i64()) {
            (Some(topic_id), Some(n)) if n > 0 => format!("{}/t/{topic_id}/{n}", cx.base_path),
            (Some(topic_id), _) => format!("{}/t/{topic_id}", cx.base_path),
            (None, _) => b["bookmarkable_url"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
        };
        cells.push_str(&crate::topic_list_view::activity_cell(
            cx,
            b,
            " post-metadata",
            &last_post_url,
        ));
    }
    cells.push_str(&format!(
        "<td class=\"topic-list-data\">{}</td>",
        actions_dropdown(cx, b)
    ));
    format!(
        "<tr class=\"{classes}\" data-bookmark-id=\"{}\" data-has-reminder=\"{}\">{cells}</tr>",
        b["id"],
        reminder.is_some()
    )
}

/// BookmarkActionsDropdown: delete, edit, clear the reminder, pin.
fn actions_dropdown(cx: &ListContext, b: &Value) -> String {
    let pin = if b["pinned"] == true { "unpin" } else { "pin" };
    let mut rows = vec![
        ("remove", "trash-can", "delete_bookmark"),
        ("edit", "pencil", "edit_bookmark"),
    ];
    if b["reminder_at"].is_string() {
        rows.push((
            "clear_reminder",
            "clock-rotate-left",
            "clear_bookmark_reminder",
        ));
    }
    let pin_key = format!("{pin}_bookmark");
    rows.push(("pin", "thumbtack", &pin_key));
    let items: String = rows
        .iter()
        .enumerate()
        .map(|(i, (id, icon_name, key))| {
            let name = escape(&t(cx, &format!("post.bookmarks.actions.{key}.name")));
            format!(
                "<li aria-checked=\"false\" role=\"menuitemradio\" data-index=\"{i}\" data-name=\"{name}\" data-value=\"{id}\" title=\"{name}\" tabindex=\"0\" class=\"select-kit-row dropdown-select-box-row\"><div class=\"icons\"><span class=\"selection-indicator\"></span>{}</div><div class=\"texts\"><span class=\"name\">{name}</span><span class=\"desc\">{}</span></div></li>",
                icon(icon_name, None),
                escape(&t(cx, &format!("post.bookmarks.actions.{key}.description")))
            )
        })
        .collect();
    let filter_by = escape(&t_with(cx, "select_kit.filter_by", &[("name", "...")]));
    format!(
        "<details class=\"select-kit single-select dropdown-select-box bookmark-actions-dropdown\"><summary aria-label=\"{filter_by}\" name=\"{filter_by}\" data-name=\"...\" tabindex=\"0\" class=\"select-kit-header single-select-header dropdown-select-box-header btn btn-icon-text btn-default\"><div class=\"select-kit-header-wrapper\"><div class=\"select-kit-selected-name selected-name choice\" data-name=\"...\" title=\"...\"><span class=\"name\">...</span></div>&#8203;</div></summary><div class=\"select-kit-body\"><div class=\"select-kit-filter\"></div><ul aria-live=\"polite\" class=\"select-kit-collection\" role=\"menu\">{items}</ul></div></details>"
    )
}

#[cfg(test)]
mod tests {
    use super::client_excerpt;

    #[test]
    fn the_excerpt_keeps_text_links_and_emoji() {
        assert_eq!(
            client_excerpt("<p>One &lt;two&gt; <a href=\"/x\">three</a></p>", 300),
            "One &lt;two&gt; <a href=\"/x\">three</a>"
        );
        assert_eq!(
            client_excerpt(
                "<p><img src=\"/e.png\" class=\"emoji\" alt=\":smile:\"> and <img src=\"/a.png\"></p>",
                300
            ),
            "<img src=\"/e.png\" class=\"emoji\" alt=\":smile:\"> and [image]"
        );
    }

    #[test]
    fn the_excerpt_is_cut_with_an_ellipsis() {
        assert_eq!(client_excerpt("<p>abcdefghij</p>", 4), "abcd&hellip;");
        assert_eq!(client_excerpt("<p>ab</p><p>cdef</p>", 3), "abc&hellip;");
    }
}
