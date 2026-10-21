//! The review queue's page (`/review`, review.index) as the Ember app
//! renders it from ReviewablesController#index's document: the navigation
//! (templates/review/index.gjs), ReviewFilters, and each reviewable as
//! components/reviewable/item.gjs draws it, with the flagged post's body
//! (reviewable/flagged-post.gjs, reviewable/post.gjs), its timeline
//! (reviewable/timeline.gjs) and note form, and the moderator actions
//! (ReviewableBundledAction, ReviewableClaimedTopic).
//!
//! The document is side-loaded as the store reads it (`__rest_serializer`):
//! a reviewable's `*_id` and `*_ids` name rows of the top-level arrays.
//! What the page's script needs of each reviewable (its version, actions,
//! watched words) goes in a JSON block for static/js/review.js.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde_json::{Map, Value, json};

use crate::admin_view::AdminCx;
use crate::topic_list_view::{ListContext, escape, icon, t, t_with};

/// Reviewable statuses (models/reviewable.js).
const PENDING: i64 = 0;
const APPROVED: i64 = 1;
const REJECTED: i64 = 2;
const IGNORED: i64 = 3;
const DELETED: i64 = 4;

/// ReviewableHistory's CLAIMED and UNCLAIMED types.
const CLAIMED: i64 = 3;
const UNCLAIMED: i64 = 4;

/// The controller's filters as setupController takes them from the
/// document's meta, and what the page's links carry.
pub struct Page<'a> {
    pub cx: &'a ListContext<'a>,
    pub acx: &'a AdminCx<'a>,
    pub doc: &'a Value,
    pub viewer_id: i64,
    pub admin: bool,
    /// reviewable_claiming: disabled, optional or required.
    pub claiming: &'a str,
    pub blur_tl0: bool,
    /// The View All link's query (`?sort_order=score`), the route's query
    /// params that differ from their defaults.
    pub view_all_query: &'a str,
    /// reviewable_default_visibility, the priority filter.s default.
    pub default_priority: &'a str,
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}

fn id_key(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn time(v: &Value) -> Option<DateTime<Utc>> {
    crate::admin_ui::time(v)
}

/// The side-loaded records by type and id.
struct Store<'a> {
    tables: HashMap<&'a str, HashMap<String, &'a Value>>,
}

impl<'a> Store<'a> {
    fn new(doc: &'a Value) -> Store<'a> {
        let mut tables = HashMap::new();
        if let Some(map) = doc.as_object() {
            for (name, rows) in map {
                if name == "reviewables" || name == "meta" {
                    continue;
                }
                let Some(rows) = rows.as_array() else {
                    continue;
                };
                let by_id = rows
                    .iter()
                    .filter_map(|r| id_key(&r["id"]).map(|k| (k, r)))
                    .collect();
                tables.insert(name.as_str(), by_id);
            }
        }
        Store { tables }
    }

    fn get(&self, table: &str, id: &Value) -> Option<&'a Value> {
        self.tables.get(table)?.get(&id_key(id)?).copied()
    }

    fn user(&self, id: &Value) -> Option<&'a Value> {
        self.get("users", id)
    }

    fn many(&self, table: &str, ids: &Value) -> Vec<&'a Value> {
        ids.as_array()
            .map(|ids| ids.iter().filter_map(|id| self.get(table, id)).collect())
            .unwrap_or_default()
    }
}

/// A score with its type, flagger and reviewer resolved.
struct Score<'a> {
    row: &'a Value,
    kind: &'a str,
    title: &'a str,
    user: Option<&'a Value>,
    reviewed_by: Option<&'a Value>,
}

fn scores<'a>(store: &Store<'a>, r: &Value) -> Vec<Score<'a>> {
    store
        .many("reviewable_scores", &r["reviewable_score_ids"])
        .into_iter()
        .map(|row| {
            let score_type = store.get("score_types", &row["score_type_id"]);
            Score {
                row,
                kind: score_type.map(|t| s(&t["type"])).unwrap_or_default(),
                title: score_type.map(|t| s(&t["title"])).unwrap_or_default(),
                user: store.user(&row["user_id"]),
                reviewed_by: store.user(&row["reviewed_by_id"]),
            }
        })
        .collect()
}

/// A bundle with its actions.
struct Bundle<'a> {
    row: &'a Value,
    actions: Vec<&'a Value>,
}

fn bundles<'a>(store: &Store<'a>, r: &Value) -> Vec<Bundle<'a>> {
    store
        .many("bundled_actions", &r["bundled_action_ids"])
        .into_iter()
        .map(|row| Bundle {
            row,
            actions: store.many("actions", &row["action_ids"]),
        })
        .collect()
}

/// `dasherize`: underscores and camel case to dashes.
fn dasherize(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 && !out.ends_with('-') {
                out.push('-');
            }
            out.push(c.to_ascii_lowercase());
        } else if c == '_' || c == ' ' {
            out.push('-');
        } else {
            out.push(c);
        }
    }
    out
}

/// `underscore`, then anything but word characters to underscores, as
/// allTypes names a type's translation.
fn type_key(name: &str) -> String {
    let mut out = String::new();
    let mut prev: Option<char> = None;
    for c in name.chars() {
        if c.is_ascii_uppercase() {
            if prev.is_some_and(|p| p.is_ascii_lowercase() || p.is_ascii_digit()) {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
        } else {
            out.push('_');
        }
        prev = Some(c);
    }
    // `::` became `__`; underscore() makes it one `/`, then one `_`.
    out.replace("__", "_")
}

/// DUserLink around its content.
fn user_link(cx: &ListContext, user: &Value, content: &str) -> String {
    let username = s(&user["username"]);
    format!(
        "<a aria-label=\"{}\" data-user-card=\"{}\" href=\"{}/u/{}\" tabindex=\"0\">{content}</a>",
        escape(&t_with(
            cx,
            "user.profile_possessive",
            &[("username", username)]
        )),
        escape(username),
        cx.base_path,
        escape(&username.to_lowercase()),
    )
}

/// `dAvatar user imageSize=...`: 24 pixels for small and tiny here.
fn avatar(user: &Value, size: i64) -> String {
    crate::post_view::avatar_img(
        s(&user["avatar_template"]),
        size,
        &format!(" title=\"{}\"", escape(s(&user["username"]))),
    )
}

/// `dFormatDate format="tiny"`
fn tiny_date(cx: &ListContext, at: DateTime<Utc>) -> String {
    crate::viewer_dates::tiny_span(cx, at)
}

/// `dFormatDate format="medium"`
fn medium_date(cx: &ListContext, at: DateTime<Utc>) -> String {
    format!(
        "<span class=\"relative-date date\"{} data-time=\"{}\" data-format=\"medium\">{}</span>",
        crate::viewer_dates::long_title(cx, at),
        at.timestamp_millis(),
        escape(&crate::viewer_dates::relative_age(cx, at, "medium"))
    )
}

/// DButton without an icon: the translated label, the classes given.
fn text_button(class: &str, label: &str, disabled: bool) -> String {
    format!(
        "<button class=\"btn btn-text {}\"{} type=\"button\"><span class=\"d-button-label\">{}</span></button>",
        escape(class),
        if disabled { " disabled=\"\"" } else { "" },
        escape(label)
    )
}

/// DButton with an icon and a label.
fn icon_button(class: &str, icon_name: &str, label: &str, title: Option<&str>) -> String {
    format!(
        "<button class=\"btn btn-icon-text {}\"{} type=\"button\">{}<span class=\"d-button-label\">{}</span></button>",
        escape(class),
        title
            .map(|t| format!(" title=\"{}\"", escape(t)))
            .unwrap_or_default(),
        icon(icon_name, None),
        escape(label)
    )
}

/// review/index.gjs: the navigation, then ReviewIndex.
pub fn render(p: &Page) -> String {
    let cx = p.cx;
    let mut out = String::from("<div class=\"reviewable\">");
    // DHorizontalOverflowNav with the DNavItems; View All is current.
    out.push_str(
        "<nav aria-label=\"Review\" class=\"horizontal-overflow-nav\"><div class=\"d-overflow-controls --owned-scroller horizontal-overflow-nav__controls\"><ul class=\"nav-pills action-list reviewable-title\" data-d-scroll-axis=\"horizontal\">",
    );
    out.push_str(&format!(
        "<li class=\"active\"><a class=\"active\" href=\"{}/review{}\">{}</a></li>",
        cx.base_path,
        p.view_all_query
            .replace('&', "&amp;")
            .replace('"', "&quot;"),
        escape(&t(cx, "review.view_all"))
    ));
    out.push_str(&format!(
        "<li><a href=\"{}/review/topics\">{}</a></li>",
        cx.base_path,
        escape(&t(cx, "review.grouped_by_topic"))
    ));
    if p.admin {
        out.push_str(&format!(
            "<li><a href=\"{}/review/settings\">{} {}</a></li>",
            cx.base_path,
            icon("wrench", None),
            escape(&t(cx, "review.settings.title"))
        ));
    }
    out.push_str("</ul></div></nav>");
    out.push_str("<div class=\"reviewable-container\">");
    out.push_str(&filters(p));
    out.push_str("<div class=\"reviewable-list\">");
    let store = Store::new(p.doc);
    let reviewables: Vec<&Value> = p.doc["reviewables"]
        .as_array()
        .map(|r| r.iter().collect())
        .unwrap_or_default();
    let mut data = Map::new();
    if reviewables.is_empty() {
        out.push_str(&format!(
            "<div class=\"no-review\">{}</div>",
            escape(&t(cx, "review.none"))
        ));
    } else {
        out.push_str("<div><div class=\"reviewables\">");
        for r in &reviewables {
            out.push_str(&item(p, &store, r));
            if let Some(id) = id_key(&r["id"]) {
                data.insert(id, script_data(&store, r));
            }
        }
        out.push_str(
            "</div><div aria-hidden=\"true\" class=\"load-more-sentinel\"></div></div><div class=\"loading-container\"></div>",
        );
    }
    out.push_str("</div></div></div>");
    out.push_str(&expanded_filters_template(p));
    out.push_str(&format!(
        "<script type=\"application/json\" id=\"review-page-data\">{}</script>",
        json!({
            "reviewables": data,
            "viewer_id": p.viewer_id,
            "default_priority": p.default_priority,
            "filters": { "sort_order": p.doc["meta"]["sort_order"] },
            "strings": strings(p),
        })
        .to_string()
        .replace('<', "\\u003c")
    ));
    // The dialog's strings and admin-ui.js, which review.js uses.
    out.push_str(&crate::admin_ui::strings(
        p.acx,
        "review-strings",
        &["generic_error"],
    ));
    out
}

/// What review.js says on its own: the copy feedback, the toasts' close
/// label, the status names, the options button's chevrons.
fn strings(p: &Page) -> Value {
    let cx = p.cx;
    let mut statuses = Map::new();
    for name in [
        "pending",
        "approved_flag",
        "rejected_flag",
        "ignored",
        "deleted",
        "approved_post",
        "rejected_post",
        "approved_user",
        "rejected_user",
    ] {
        statuses.insert(
            name.into(),
            json!(t(cx, &format!("review.statuses.{name}.title"))),
        );
    }
    json!({
        "copy_link_feedback": t(cx, "review.copy_link_feedback"),
        "statuses": statuses,
        "close": t(cx, "modal.close"),
    })
}

/// select-kit's ComboBox for a filter, its rows in a template; the
/// selected name carries the value as the header does.
fn combo(
    p: &Page,
    class: &str,
    rows: &[(String, String)],
    value: Option<&str>,
    none: Option<&str>,
) -> String {
    let html = crate::admin_ui::combo_box(p.acx, class, rows, value, none)
        .replacen("combo-box  has-selection", "combo-box has-selection", 1)
        .replacen("combo-box \"", "combo-box\"", 1);
    match value.filter(|v| rows.iter().any(|(id, _)| id == v)) {
        Some(v) => html.replacen(
            "class=\"select-kit-selected-name selected-name choice\"",
            &format!(
                "class=\"select-kit-selected-name selected-name choice\" data-value=\"{}\"",
                escape(v)
            ),
            1,
        ),
        None => html,
    }
}

/// The filter's value from the meta, when present.
fn meta_value(p: &Page, key: &str) -> Option<String> {
    match &p.doc["meta"][key] {
        Value::String(v) if !v.is_empty() => Some(v.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn filter(label: &str, extra_class: &str, control: &str) -> String {
    format!(
        "<div class=\"reviewable-filter{}\"><label class=\"filter-label\">{}</label>{control}</div>",
        if extra_class.is_empty() {
            String::new()
        } else {
            format!(" {extra_class}")
        },
        escape(label)
    )
}

/// ReviewFilters, collapsed: status, type, refresh and the Options toggle.
fn filters(p: &Page) -> String {
    let cx = p.cx;
    let statuses: Vec<(String, String)> = [
        "pending", "approved", "rejected", "deleted", "ignored", "reviewed", "all",
    ]
    .iter()
    .map(|id| {
        (
            id.to_string(),
            t(cx, &format!("review.statuses.{id}.title")),
        )
    })
    .collect();
    let types: Vec<(String, String)> = p.doc["meta"]["reviewable_types"]
        .as_array()
        .map(|types| {
            types
                .iter()
                .map(|ty| {
                    let ty = s(ty);
                    (
                        ty.to_string(),
                        crate::admin_view::t(
                            cx.i18n,
                            &format!("review.types.{}.title", type_key(ty)),
                        ),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let mut out = String::from("<div class=\"reviewable-filters --collapsed\">");
    out.push_str(&filter(
        &t(cx, "review.filters.status"),
        "",
        &combo(p, "", &statuses, meta_value(p, "status").as_deref(), None),
    ));
    out.push_str(&filter(
        &t(cx, "review.filters.type.title"),
        "",
        &combo(
            p,
            "",
            &types,
            meta_value(p, "type").as_deref(),
            Some(&t(cx, "review.filters.type.all")),
        ),
    ));
    out.push_str("<div class=\"reviewable-filters-actions\">");
    out.push_str(&icon_button(
        "btn-primary refresh",
        "arrows-rotate",
        &t(cx, "review.filters.refresh"),
        None,
    ));
    out.push_str(&icon_button(
        "btn-default expand-secondary-filters",
        "chevron-down",
        &t(cx, "show_help"),
        None,
    ));
    out.push_str("</div></div>");
    out
}

/// EmailGroupUserChooser for one user (admin-ui.js opens it).
fn user_chooser(p: &Page, extra_class: &str, value: Option<&str>) -> String {
    let none = crate::admin_view::t(p.cx.i18n, "select_kit.default_header_text");
    let u = value.map(escape).unwrap_or_default();
    format!(
        "<details class=\"select-kit multi-select user-chooser email-group-user-chooser full-width-wrap{}{}\" data-value=\"{u}\" data-placeholder=\"{}\" data-caret=\"plus\"{}><summary class=\"select-kit-header multi-select-header\" data-name=\"{u}\" tabindex=\"0\"><div class=\"select-kit-header-wrapper\"><span class=\"formatted-selection\">{}</span>{}</div></summary><div class=\"select-kit-body\"></div></details>",
        if extra_class.is_empty() {
            String::new()
        } else {
            format!(" {extra_class}")
        },
        if value.is_some() {
            " has-selection"
        } else {
            ""
        },
        escape(&crate::admin_view::t(
            p.cx.i18n,
            "select_kit.filter_placeholder"
        )),
        crate::admin_ui::chooser_labels(p.acx),
        if value.is_some() {
            u.clone()
        } else {
            escape(&none)
        },
        icon("plus", Some("angle-icon")),
    )
}

/// The filters ReviewFilters adds while expanded, in a template the
/// Options button stamps in before the actions.
fn expanded_filters_template(p: &Page) -> String {
    let cx = p.cx;
    let mut out = String::from("<template id=\"review-secondary-filters\">");
    if p.claiming != "disabled" {
        out.push_str(&filter(
            &t(cx, "review.filtered_claimed_by"),
            "topic-filter claimed-by",
            &user_chooser(p, "", meta_value(p, "claimed_by").as_deref()),
        ));
    }
    let score_types: Vec<(String, String)> = p.doc["meta"]["score_types"]
        .as_array()
        .map(|types| {
            types
                .iter()
                .filter_map(|ty| Some((id_key(&ty["id"])?, s(&ty["name"]).to_string())))
                .collect()
        })
        .unwrap_or_default();
    // filterScoreType: setupController leaves it unset.
    out.push_str(&filter(
        &t(cx, "review.filters.score_type.title"),
        "",
        &combo(
            p,
            "",
            &score_types,
            None,
            Some(&t(cx, "review.filters.score_type.all")),
        ),
    ));
    let priorities: Vec<(String, String)> = ["any", "low", "medium", "high"]
        .iter()
        .map(|id| {
            (
                id.to_string(),
                t(cx, &format!("review.filters.priority.{id}")),
            )
        })
        .collect();
    out.push_str(&filter(
        &t(cx, "review.filters.priority.title"),
        "",
        &combo(
            p,
            "",
            &priorities,
            meta_value(p, "priority").as_deref(),
            None,
        ),
    ));
    let mut categories: Vec<(String, String)> = cx
        .categories
        .values()
        .map(|c| (c.id.to_string(), c.name.clone()))
        .collect();
    categories.sort_by_key(|c| c.1.to_lowercase());
    out.push_str(&filter(
        &t(cx, "review.filters.category"),
        "",
        &combo(
            p,
            "category-chooser",
            &categories,
            meta_value(p, "category_id").as_deref(),
            Some(&t(cx, "review.filters.all_categories")),
        ),
    ));
    out.push_str(&filter(
        &t(cx, "review.filtered_flagged_by"),
        "topic-filter",
        &user_chooser(p, "", meta_value(p, "flagged_by").as_deref()),
    ));
    out.push_str(&filter(
        &t(cx, "review.filtered_reviewed_by"),
        "topic-filter",
        &user_chooser(p, "", meta_value(p, "reviewed_by").as_deref()),
    ));
    out.push_str(&filter(
        &t(cx, "review.filtered_user"),
        "topic-filter",
        &user_chooser(p, "user-selector", meta_value(p, "username").as_deref()),
    ));
    if meta_value(p, "topic_id").is_some() {
        out.push_str(&filter(
            &t(cx, "review.filtered_topic"),
            "topic-filter",
            &icon_button(
                "btn-default",
                "xmark",
                &t(cx, "review.show_all_topics"),
                None,
            ),
        ));
    }
    let orders: Vec<(String, String)> = ["score", "score_asc", "created_at", "created_at_asc"]
        .iter()
        .map(|id| {
            (
                id.to_string(),
                t(cx, &format!("review.filters.orders.{id}")),
            )
        })
        .collect();
    out.push_str(&filter(
        &t(cx, "review.order_by"),
        "sort-order",
        &combo(p, "", &orders, meta_value(p, "sort_order").as_deref(), None),
    ));
    out.push_str(&filter(
        &t(cx, "review.date_filter"),
        "date-range",
        &crate::admin_logs_view::date_range(
            p.acx,
            meta_value(p, "from_date").as_deref(),
            meta_value(p, "to_date").as_deref(),
        ),
    ));
    out.push_str("</template>");
    out
}

/// What review.js needs of a reviewable: its version and topic, the
/// actions (by id) as performing them reads them, and the watched words
/// its scores highlight.
fn script_data(store: &Store, r: &Value) -> Value {
    let mut actions = Map::new();
    for bundle in bundles(store, r) {
        for a in bundle.actions {
            if let Some(id) = id_key(&a["id"]) {
                actions.insert(id, a.clone());
            }
        }
    }
    let words: Vec<Value> = scores(store, r)
        .iter()
        .filter(|sc| sc.row["reason_type"] == "watched_word")
        .flat_map(|sc| {
            sc.row["reason_data"]
                .as_array()
                .cloned()
                .unwrap_or_default()
        })
        .collect();
    json!({
        "version": r["version"],
        "type": r["type"],
        "topic_id": r["topic_id"],
        "actions": actions,
        "watched_words": words,
    })
}

/// reviewable/item.gjs
fn item(p: &Page, store: &Store, r: &Value) -> String {
    let cx = p.cx;
    let kind = s(&r["type"]);
    let status = r["status"].as_i64().unwrap_or(PENDING);
    let target_created_by = store.user(&r["target_created_by_id"]);
    let scores = scores(store, r);
    let bundles = bundles(store, r);
    let claimed_by = store.get("claimed_bies", &r["claimed_by_id"]);
    let claimed_user = claimed_by.and_then(|c| store.user(&c["user_id"]));

    // customClasses
    let mut classes = dasherize(kind);
    if p.blur_tl0 && r["target_created_by_trust_level"] == 0 {
        classes.push_str(" blur-images");
    }
    if !r["deleted_at"].is_null() {
        classes.push_str(" reviewable-deleted");
    }
    let topic_id = [&r["topic_id"], &r["removed_topic_id"]]
        .into_iter()
        .find(|v| !v.is_null())
        .cloned()
        .unwrap_or(Value::Null);
    let claim_enabled = p.claiming != "disabled" && !topic_id.is_null() && status == PENDING;
    let claim_optional = !claim_enabled || p.claiming == "optional";
    let claim_required = claim_enabled && p.claiming == "required";
    let can_perform = if bundles.is_empty() {
        false
    } else if !claim_enabled {
        true
    } else if let Some(user) = claimed_user {
        user["id"].as_i64() == Some(p.viewer_id)
    } else {
        p.claiming != "required"
    };
    let display_context_question = r["created_from_flag"] == true
        && status == PENDING
        && (claim_optional || (claim_required && claimed_by.is_some()));

    // reviewableTypeLabel
    let label_key = match kind {
        "ReviewableUser" => "review.user_label",
        "ReviewableQueuedPost" if topic_id.is_null() => "review.queued_topic_label",
        "ReviewableQueuedPost" => "review.queued_post_label",
        _ if r["created_from_flag"] == true => "review.post_flagged_as",
        _ => "review.flagged_as",
    };
    // scoreSummary: one badge per score type, counted.
    let mut summary: Vec<(&str, &str, usize)> = Vec::new();
    for sc in &scores {
        match summary.iter_mut().find(|(k, _, _)| *k == sc.kind) {
            Some(entry) => entry.2 += 1,
            None => summary.push((sc.kind, sc.title, 1)),
        }
    }

    let mut out = format!(
        "<div class=\"review-container\"><div class=\"review-item {}\" data-reviewable-id=\"{}\"><div class=\"review-item__primary-content\"><div class=\"review-item__flag-summary\"><div class=\"review-item__header\"><div class=\"review-item__label-badges\"><span class=\"review-item__flag-label\">{}</span><div class=\"review-item__flag-badges\">",
        escape(&classes),
        r["id"],
        escape(&t(cx, label_key))
    );
    for (_, title, count) in &summary {
        out.push_str(&flag_reason(title, *count));
    }
    out.push_str("</div></div>");
    out.push_str(&format!(
        "<button class=\"btn btn-transparent reviewable-permalink-copy\" title=\"{}\" type=\"button\">{}</button>",
        escape(&t(cx, "review.copy_permalink_title")),
        icon("link", None)
    ));
    out.push_str(&status_badge(cx, status, kind));
    out.push_str("<span class=\"reviewable-created-date\">");
    if let Some(at) = time(&r["created_at"]) {
        out.push_str(&tiny_date(cx, at));
    }
    out.push_str("</span></div>");
    out.push_str(&match kind {
        "ReviewableFlaggedPost" => {
            post_body(cx, r, store, target_created_by, "review.flagged_user")
        }
        "ReviewablePost" => post_body(cx, r, store, target_created_by, "review.review_user"),
        // review_list refuses the others before they get here.
        _ => format!(
            "<div class=\"alert alert-error review-item__no-component\">{}</div>",
            escape(&t_with(cx, "review.no_component_found", &[("type", kind)]))
        ),
    });
    out.push_str("</div>");

    // The insights tabs, the timeline current.
    out.push_str(&format!(
        "<div class=\"review-item__insights\"><div class=\"d-nav-submenu\"><nav aria-label=\"Review tabs\" class=\"horizontal-overflow-nav\"><div class=\"d-overflow-controls --owned-scroller horizontal-overflow-nav__controls\"><ul class=\"nav-pills action-list d-nav-submenu__tabs\" data-d-scroll-axis=\"horizontal\"><li class=\"timeline active\"><a class=\"active\" href=\"#\">{}</a></li><li class=\"insights\"><a href=\"#\">{}</a></li></ul></div></nav></div>",
        escape(&t(cx, "review.timeline_and_notes")),
        escape(&t(cx, "review.insights.title"))
    ));
    out.push_str(&timeline(p, store, r, &scores, target_created_by));
    out.push_str("</div></div>");

    // The aside: the moderator actions, the claim.
    out.push_str("<div class=\"review-item__aside\">");
    if r["last_performing_username"].is_null() && can_perform {
        let question = if display_context_question {
            Some(context_question(cx, kind, &scores, &topic_id))
        } else {
            user_context_question(cx, kind, status, &scores, &topic_id)
        };
        out.push_str(&format!(
            "<div class=\"review-item__moderator-actions\"><h3 class=\"review-item__aside-title\">{}</h3>",
            escape(&question.unwrap_or_else(|| t(cx, "review.moderator_actions")))
        ));
        let (secondary, primary): (Vec<&Bundle>, Vec<&Bundle>) =
            bundles.iter().partition(|b| b.row["secondary"] == true);
        for b in &primary {
            out.push_str(&bundled_action(cx, b));
        }
        if !secondary.is_empty() || r["can_edit"] == true {
            out.push_str("<div class=\"review-item__secondary-actions\">");
            for b in &secondary {
                out.push_str(&bundled_action(cx, b));
            }
            if r["can_edit"] == true {
                out.push_str(&text_button(
                    "reviewable-action btn-default edit",
                    &t(cx, "review.edit"),
                    false,
                ));
            }
            out.push_str("</div>");
        }
        out.push_str("</div>");
    }
    if claim_enabled {
        out.push_str("<div class=\"review-item__moderator-actions --extra\">");
        if let Some(user) = claimed_user {
            out.push_str(&format!(
                "<div class=\"review-item__assigned\">{}<div class=\"created-by\">{}</div></div>",
                icon("user-plus", None),
                user_link(
                    cx,
                    user,
                    &format!(
                        "{} <span class=\"username\">{}</span>",
                        avatar(user, 24),
                        escape(s(&user["username"]))
                    )
                )
            ));
        }
        // ReviewableClaimedTopic
        out.push_str("<div class=\"reviewable-claimed-topic\">");
        if claimed_user.is_some() {
            out.push_str(&icon_button(
                "btn-default unclaim",
                "xmark",
                &t(cx, "review.unclaim.help"),
                None,
            ));
        } else {
            out.push_str(&icon_button(
                "btn-default claim",
                "user-plus",
                &t(cx, "review.claim.title"),
                Some(&t(cx, "review.claim_help.optional")),
            ));
        }
        out.push_str("</div></div>");
    }
    out.push_str("</div></div></div>");
    out
}

/// ReviewableFlagReason
fn flag_reason(title: &str, count: usize) -> String {
    let count = if count > 1 {
        format!("<span class=\"review-item__flag-count\">x{count}</span>")
    } else {
        String::new()
    };
    format!(
        "<span class=\"review-item__flag-reason\">{} {count}</span>",
        escape(title)
    )
}

/// newReviewableStatus
fn status_badge(cx: &ListContext, status: i64, kind: &str) -> String {
    let (name, class) = match (status, kind) {
        (PENDING, _) => ("pending", "pending"),
        (APPROVED, "ReviewableQueuedPost") => ("approved_post", "approved"),
        (APPROVED, "ReviewableUser") => ("approved_user", "approved"),
        (APPROVED, _) => ("approved_flag", "approved"),
        (REJECTED, "ReviewableQueuedPost") => ("rejected_post", "rejected"),
        (REJECTED, "ReviewableUser") => ("rejected_user", "rejected"),
        (REJECTED, _) => ("rejected_flag", "rejected"),
        (IGNORED, _) => ("ignored", "ignored"),
        (DELETED, _) => ("deleted", "deleted"),
        _ => return String::new(),
    };
    format!(
        "<div class=\"review-item__status --{class}\">{}</div>",
        escape(&t(cx, &format!("review.statuses.{name}.title")))
    )
}

/// The reviewable's noun (`humanNoun`).
fn human_noun(cx: &ListContext, kind: &str, topic_id: &Value) -> String {
    let resolved = if kind == "ReviewableQueuedPost" && topic_id.is_null() {
        "ReviewableQueuedTopic"
    } else {
        kind
    };
    cx.i18n
        .t(&format!("js.review.types.{}.noun", type_key(resolved)))
        .unwrap_or("reviewable")
        .to_string()
}

/// `flaggedReviewableContextQuestion`
fn context_question(cx: &ListContext, kind: &str, scores: &[Score], topic_id: &Value) -> String {
    let noun = human_noun(cx, kind, topic_id);
    let mut unique: Vec<&Score> = Vec::new();
    for sc in scores {
        if !unique.iter().any(|u| u.kind == sc.kind) {
            unique.push(sc);
        }
    }
    if unique.len() == 1 && unique[0].kind == "notify_moderators" {
        return t_with(
            cx,
            "review.context_question.something_else_wrong",
            &[("reviewable_type", &noun)],
        );
    }
    let mut titles: Vec<String> = Vec::new();
    for sc in &unique {
        let title = sc.title.to_lowercase();
        if !titles.contains(&title) {
            titles.push(title);
        }
    }
    let delimiter = t(cx, "review.context_question.delimiter");
    // I18n.listJoiner
    let joined = match titles.len() {
        0 => String::new(),
        1 => titles[0].clone(),
        2 => format!("{} {delimiter} {}", titles[0], titles[1]),
        _ => {
            let last = titles.pop().unwrap_or_default();
            titles.push(delimiter);
            format!("{} {last}", titles.join(", "))
        }
    };
    t_with(
        cx,
        "review.context_question.is_this_post",
        &[
            ("reviewable_human_score_types", &joined),
            ("reviewable_type", &noun),
        ],
    )
}

/// `userReviewableContextQuestion`
fn user_context_question(
    cx: &ListContext,
    kind: &str,
    status: i64,
    scores: &[Score],
    topic_id: &Value,
) -> Option<String> {
    let key = match kind {
        "ReviewableUser" => {
            if status != PENDING {
                return None;
            }
            if scores
                .iter()
                .any(|sc| sc.row["reason_type"] == "suspect_user")
            {
                "review.context_question.suspect_user"
            } else {
                "review.context_question.approve_user"
            }
        }
        "ReviewableQueuedPost" if topic_id.is_null() => "review.context_question.approve_topic",
        "ReviewableQueuedPost" | "ReviewablePost" => "review.context_question.approve_post",
        _ => return None,
    };
    Some(t(cx, key))
}

/// ReviewableBundledAction: a button for one action, else the
/// DropdownSelectBox of them, its rows in a template for review.js.
fn bundled_action(cx: &ListContext, bundle: &Bundle) -> String {
    let Some(first) = bundle.actions.first() else {
        return String::new();
    };
    let action_class = dasherize(s(&first["action_name"]));
    let button_class = s(&first["button_class"]);
    if bundle.actions.len() == 1 {
        let class = [
            "btn-default",
            "reviewable-action",
            action_class.as_str(),
            button_class,
        ]
        .iter()
        .filter(|c| !c.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(" ");
        return format!(
            "<button class=\"btn btn-text {}\" type=\"button\"><span class=\"d-button-label\">{}</span></button>",
            escape(&class),
            escape(s(&first["label"]))
        );
    }
    let class = [
        "reviewable-action-dropdown",
        "btn-icon-text",
        action_class.as_str(),
        button_class,
    ]
    .iter()
    .filter(|c| !c.is_empty())
    .copied()
    .collect::<Vec<_>>()
    .join(" ");
    let name = escape(s(&bundle.row["label"]));
    let filter_by = escape(&t_with(
        cx,
        "select_kit.filter_by",
        &[("name", s(&bundle.row["label"]))],
    ));
    let mut rows = String::new();
    for (index, a) in bundle.actions.iter().enumerate() {
        let label = escape(s(&a["label"]));
        let icon_html = a["icon"]
            .as_str()
            .map(|name| icon(name, None))
            .unwrap_or_default();
        rows.push_str(&format!(
            "<li aria-checked=\"false\" role=\"menuitemradio\" data-index=\"{index}\" data-name=\"{label}\" data-value=\"{}\" title=\"{label}\" tabindex=\"0\" class=\"select-kit-row dropdown-select-box-row\"><div class=\"icons\"><span class=\"selection-indicator\"></span>{icon_html}</div><div class=\"texts\"><span class=\"name\">{label}</span><span class=\"desc\">{}</span></div></li>",
            escape(s(&a["action_name"])),
            escape(s(&a["description"]))
        ));
    }
    format!(
        "<details class=\"select-kit single-select dropdown-select-box {}\"><summary aria-label=\"{filter_by}\" name=\"{filter_by}\" data-name=\"{name}\" tabindex=\"0\" class=\"select-kit-header single-select-header dropdown-select-box-header btn btn-icon-text btn-default\"><div class=\"select-kit-header-wrapper\"><div class=\"select-kit-selected-name selected-name choice\" data-name=\"{name}\" title=\"{name}\"><span class=\"name\">{name}</span></div>{} &#8203;</div></summary><div class=\"select-kit-body\"></div><template class=\"select-kit-rows\">{rows}</template></details>",
        escape(&class),
        icon("angle-down", Some("angle-icon")),
    )
}

/// TopicStatus with showPrivateMessageIcon, for a member (canAct).
fn topic_status(cx: &ListContext, topic: &Value, topic_url: &str) -> String {
    let flag = |name: &str| topic[name] == true;
    let mut out = String::from("<span class=\"topic-statuses\">");
    if flag("bookmarked") {
        out.push_str(&format!(
            "<a class=\"topic-status --bookmarked\" href=\"{}\" title=\"{}\">{}</a>",
            escape(topic_url),
            escape(&t(cx, "topic_statuses.bookmarked.help")),
            icon("bookmark", None)
        ));
    }
    let closed = match (flag("closed"), flag("archived")) {
        (true, true) => Some(("--closed --archived", "locked_and_archived")),
        (true, false) => Some(("--closed", "locked")),
        (false, true) => Some(("--archived", "archived")),
        _ => None,
    };
    if let Some((class, key)) = closed {
        out.push_str(&format!(
            "<span class=\"topic-status {class}\" title=\"{}\">{}</span>",
            escape(&t(cx, &format!("topic_statuses.{key}.help"))),
            icon("lock", None)
        ));
    }
    if flag("is_warning") {
        out.push_str(&format!(
            "<span class=\"topic-status --warning topic-status-warning\" title=\"{}\">{}</span>",
            escape(&t(cx, "topic_statuses.warning.help")),
            icon("envelope", None)
        ));
    } else if s(&topic["archetype"]) == "private_message" {
        out.push_str(&format!(
            "<span class=\"topic-status --personal-message\" title=\"{}\">{}</span>",
            escape(&t(cx, "topic_statuses.personal_message.help")),
            icon("envelope", None)
        ));
    }
    if flag("pinned") {
        out.push_str(&format!(
            "<a class=\"topic-status --pinned pin-toggle-button\" href=\"\" title=\"{}\">{}</a>",
            escape(&t(cx, "topic_statuses.pinned.help")),
            icon("thumbtack", None)
        ));
    } else if flag("unpinned") {
        out.push_str(&format!(
            "<a class=\"topic-status --unpinned pin-toggle-button\" href=\"\" title=\"{}\">{}</a>",
            escape(&t(cx, "topic_statuses.unpinned.help")),
            icon("thumbtack", Some("unpinned"))
        ));
    }
    // Topic#invisible; the reviewable's topic carries no visibility reason.
    if topic["visible"] == false {
        out.push_str(&format!(
            "<span class=\"topic-status --invisible\" title=\"{}\">{}</span>",
            escape(&t_with(
                cx,
                "topic_statuses.unlisted.help",
                &[("unlistedReason", "")]
            )),
            icon("far-eye-slash", None)
        ));
    }
    // discourse-solved's after-topic-status outlet.
    if flag("has_accepted_answer") {
        out.push_str(&format!(
            "<span class=\"topic-status --solved\" title=\"{}\">{}</span>",
            escape(&t(cx, "topic_statuses.solved.help")),
            icon("far-square-check", None)
        ));
    }
    out.push_str("</span>");
    out
}

/// reviewable/post.gjs: the topic, the author, the post's body.
fn post_body(
    cx: &ListContext,
    r: &Value,
    store: &Store,
    author: Option<&Value>,
    user_label_key: &str,
) -> String {
    let mut out = format!(
        "<div class=\"review-item__meta-content\"><div class=\"review-item__meta-label\">{}</div><div class=\"review-item__meta-topic-title\">",
        escape(&t(cx, "review.posted_in"))
    );
    // ReviewableTopicLink
    out.push_str("<div class=\"reviewable-topic-link\">");
    if let Some(topic) = store.get("topics", &r["topic_id"]) {
        out.push_str(&format!(
            "<div class=\"reviewable-topic-link__title-wrapper\"><div class=\"reviewable-topic-link__title-status\">{}</div><div class=\"reviewable-topic-link__title-link\"><a class=\"title-text\" href=\"{}\">{}</a></div></div>",
            topic_status(cx, topic, s(&r["topic_url"])),
            escape(s(&r["target_url"])),
            crate::topic_list_view::emoji_unescape(
                s(&topic["fancy_title"]),
                &cx.settings.emojis,
                cx.base_path
            ),
        ));
        out.push_str("<div class=\"reviewable-topic-link__details\">");
        if let Some(category) = r["category_id"]
            .as_i64()
            .and_then(|id| cx.categories.get(&id))
        {
            out.push_str(&format!(
                "<div class=\"reviewable-topic-link__details-category-badge\">{}</div>",
                crate::topic_list_view::category_badge_html(cx, category, false, false)
            ));
        }
        out.push_str("<div class=\"reviewable-topic-link__details-tags\">");
        let tags = &r["topic_tags"];
        if tags.as_array().is_some_and(|t| !t.is_empty()) {
            out.push_str(&format!(
                "<div class=\"list-tags\">{}</div>",
                crate::topic_list_view::tags_html(cx, &json!({ "tags": tags }), "", &[])
            ));
        }
        out.push_str("</div></div>");
    } else if !r["removed_topic_id"].is_null() {
        out.push_str(&format!(
            "<span class=\"title-text\">{} <a href=\"{}/t/-/{}\">{}</a></span>",
            escape(&t(cx, "review.topics.deleted")),
            cx.base_path,
            r["removed_topic_id"],
            escape(&t(cx, "review.topics.original"))
        ));
    }
    out.push_str("</div>");
    out.push_str(&post_edits(cx, r));
    out.push_str(&format!(
        "</div><div class=\"review-item__meta-label\">{}</div><div class=\"review-item__meta-flagged-user\">",
        escape(&t(cx, user_label_key))
    ));
    // reviewable/created-by.gjs
    out.push_str("<div class=\"created-by\">");
    match author {
        Some(user) => {
            out.push_str(&user_link(cx, user, &avatar(user, 24)));
            // ReviewableCreatedByName; review_list refuses authors under
            // a penalty, so there are no penalty icons.
            out.push_str(&format!(
                "<div class=\"names\"><span class=\"username\">{}</span></div>",
                user_link(cx, user, &escape(s(&user["username"])))
            ));
        }
        None => out.push_str(&format!(
            "<div class=\"deleted-user\">{}<span class=\"deleted-user-name\">{}</span></div>",
            icon("trash-can", Some("deleted-user-avatar")),
            escape(&t(cx, "review.deleted_user"))
        )),
    }
    out.push_str("</div></div></div>");
    out.push_str(
        "<div class=\"review-item__post\"><div class=\"review-item__post-content-wrapper\">",
    );
    if r["blank_post"] == true {
        out.push_str(&format!(
            "<div class=\"review-item__post-content\"><p>{}</p></div>",
            escape(&t(cx, "review.deleted_post"))
        ));
    } else {
        out.push_str(&format!(
            "<div class=\"review-item__post-content\">{}</div>",
            s(&r["cooked"])
        ));
    }
    out.push_str("<span></span></div></div>");
    out
}

/// ReviewablePostEdits: the edits button past the first version.
fn post_edits(cx: &ListContext, r: &Value) -> String {
    let version = r["post_version"].as_i64().unwrap_or(1);
    if version <= 1 {
        return String::new();
    }
    let updated = time(&r["post_updated_at"]);
    let when = updated
        .map(|at| crate::viewer_dates::long_marker(cx, at))
        .unwrap_or_default();
    let title = t_with(cx, "post.last_edited_on", &[("dateTime", &when)]);
    // historyHeat with the default history_hours_* settings.
    let heat = updated.and_then(|at| {
        let hours = (cx.now - at).num_minutes() as f64 / 60.0;
        if hours < 12.0 {
            Some(" heatmap-high")
        } else if hours < 24.0 {
            Some(" heatmap-med")
        } else if hours < 48.0 {
            Some(" heatmap-low")
        } else {
            None
        }
    });
    format!(
        "<div class=\"post-info edits\"><button class=\"btn btn-icon-text btn-flat{}\"{} type=\"button\">{}<span class=\"d-button-label\">{}</span></button></div>",
        heat.unwrap_or_default(),
        crate::viewer_dates::attr(cx, "title", &title),
        icon("pencil", None),
        version - 1
    )
}

/// One timeline event: its date, icon, translation and what fills it.
struct Event<'a> {
    at: DateTime<Utc>,
    icon: &'static str,
    key: &'static str,
    user: Option<&'a Value>,
    flag_reason: Option<&'a str>,
    description: Option<String>,
}

/// reviewable/timeline.gjs, then the note form.
fn timeline(
    p: &Page,
    store: &Store,
    r: &Value,
    scores: &[Score],
    author: Option<&Value>,
) -> String {
    let cx = p.cx;
    let mut events: Vec<Event> = Vec::new();
    if let Some(at) = time(&r["target_created_at"]) {
        events.push(Event {
            at,
            icon: "pen-to-square",
            key: "review.timeline.target_created_by",
            user: author,
            flag_reason: None,
            description: None,
        });
    }
    if let Some(at) = time(&r["target_deleted_at"]) {
        events.push(Event {
            at,
            icon: "trash-can",
            key: "review.timeline.target_deleted_by",
            user: store.user(&r["target_deleted_by_id"]),
            flag_reason: None,
            description: None,
        });
    }
    let mut reviewed: Vec<String> = Vec::new();
    for sc in scores {
        let mut description = String::new();
        if let Some(reason) = sc.row["reason"].as_str() {
            description.push_str(reason);
        }
        if let Some(context) = sc.row["context"].as_str() {
            description.push_str(&format!("<p>{}</p>", escape(context)));
        }
        if let Some(at) = time(&sc.row["created_at"]) {
            events.push(Event {
                at,
                icon: "flag",
                key: "review.timeline.flagged_as_by",
                user: sc.user,
                flag_reason: Some(sc.title),
                description: (!description.is_empty()).then_some(description),
            });
        }
        if let (Some(by), Some(at)) = (sc.reviewed_by, time(&sc.row["reviewed_at"])) {
            let key = format!("{}-{}", by["id"], s(&sc.row["reviewed_at"]));
            if !reviewed.contains(&key) {
                reviewed.push(key);
                let (icon_name, title_key) = match sc.row["status"].as_i64() {
                    Some(1) => ("check", "review.timeline.approved_by"),
                    Some(2) => ("xmark", "review.timeline.rejected_by"),
                    Some(3) => ("far-eye-slash", "review.timeline.ignored_by"),
                    // Ember's fallback leaves the title key unset.
                    _ => ("check", ""),
                };
                events.push(Event {
                    at,
                    icon: icon_name,
                    key: title_key,
                    user: Some(by),
                    flag_reason: None,
                    description: None,
                });
            }
        }
    }
    for history in store.many("reviewable_histories", &r["reviewable_history_ids"]) {
        let (icon_name, key) = match history["reviewable_history_type"].as_i64() {
            Some(CLAIMED) => ("user-plus", "review.timeline.claimed_by"),
            Some(UNCLAIMED) => ("user-xmark", "review.timeline.unclaimed_by"),
            _ => continue,
        };
        if let Some(at) = time(&history["created_at"]) {
            events.push(Event {
                at,
                icon: icon_name,
                key,
                user: store.user(&history["created_by_id"]),
                flag_reason: None,
                description: None,
            });
        }
    }
    events.sort_by_key(|e| e.at);

    let mut out = String::from("<div class=\"reviewable-timeline\">");
    if events.is_empty() {
        out.push_str(&format!(
            "<div class=\"timeline-empty\"><div class=\"timeline-empty__icon\">{}</div><p class=\"timeline-empty__message\">{}</p></div>",
            icon("clock", None),
            escape(&t(cx, "review.timeline.no_events"))
        ));
    } else {
        out.push_str("<div class=\"timeline-events\">");
        for e in &events {
            out.push_str(&format!(
                "<div class=\"timeline-event\"><div class=\"timeline-event__icon\">{}</div><div class=\"timeline-event__content\"><div class=\"timeline-event__main\"><div class=\"timeline-event__text\"><div class=\"timeline-event__title\">{}</div>",
                icon(e.icon, None),
                interpolated(cx, e)
            ));
            if let Some(description) = &e.description {
                out.push_str(&format!(
                    "<div class=\"timeline-event__description\">{description}</div>"
                ));
            }
            out.push_str("</div></div></div></div>");
        }
        out.push_str("</div>");
    }
    out.push_str(&note_form(cx, r));
    out.push_str("</div>");
    out
}

/// DInterpolatedTranslation: the text between the placeholders, each
/// placeholder's content in a span.
fn interpolated(cx: &ListContext, e: &Event) -> String {
    if e.key.is_empty() {
        return String::new();
    }
    let text = t(cx, e.key);
    let mut out = String::new();
    let mut rest = text.as_str();
    while let Some(start) = rest.find("%{") {
        out.push_str(&escape(&rest[..start]));
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            out.push_str(&escape(&rest[start..]));
            rest = "";
            break;
        };
        let content = match &after[..end] {
            "flagReason" => e.flag_reason.map(|title| flag_reason(title, 0)),
            "username" => e.user.map(|user| {
                format!(
                    "{} {}",
                    avatar(user, 24),
                    user_link(cx, user, &escape(s(&user["username"])))
                )
            }),
            "relativeDate" => Some(medium_date(cx, e.at)),
            _ => None,
        };
        out.push_str(&format!("<span>{}</span>", content.unwrap_or_default()));
        rest = &after[end + 1..];
    }
    out.push_str(&escape(rest));
    out
}

/// ReviewableNoteForm: FormKit's textarea field and its submit.
fn note_form(cx: &ListContext, r: &Value) -> String {
    let id = format!("reviewable-note-{}", r["id"]);
    format!(
        "<div class=\"reviewable-note-form\"><form class=\"form-kit reviewable-note-form__form\" novalidate=\"\"><div class=\"form-kit__container form-kit__field form-kit__field-textarea --full\" data-name=\"content\" id=\"control-content\" data-control-type=\"textarea\"><label for=\"{id}\" class=\"form-kit__container-title\"><span>{}</span></label><div class=\"form-kit__container-content\"><div class=\"reviewable-note-form__textarea-wrapper\"><textarea id=\"{id}\" class=\"form-kit__control-textarea reviewable-note-form__textarea\" name=\"content\" style=\"height: 80px\" placeholder=\"{}\"></textarea></div><div class=\"form-kit__meta\"><span class=\"form-kit__char-counter --insufficient\">0/2000</span></div></div></div><div class=\"form-kit__section form-kit__actions\"><button class=\"btn btn-primary form-kit__button btn-small btn-primary\" type=\"submit\"><span class=\"d-button-label\">{}</span></button></div></form></div>",
        escape(&t(cx, "review.notes.add_note_description")),
        escape(&t(cx, "review.notes.placeholder")),
        escape(&t(cx, "review.notes.add_note_button"))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_keys_follow_ember_underscore() {
        assert_eq!(type_key("ReviewableFlaggedPost"), "reviewable_flagged_post");
        assert_eq!(
            type_key("Chat::ReviewableMessage"),
            "chat_reviewable_message"
        );
    }

    #[test]
    fn dasherize_matches_ember() {
        assert_eq!(
            dasherize("ReviewableFlaggedPost"),
            "reviewable-flagged-post"
        );
        assert_eq!(dasherize("post-agree_and_hide"), "post-agree-and-hide");
    }
}
