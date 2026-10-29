//! discourse-templates' UI drawn on the server: the filterable list
//! (d-templates/filterable-list.gjs with its TagDrop and items) that the
//! insert template modal and the composer preview's panel show, filtered
//! by keyword and tag through htmx (GET /live/templates), the topic's
//! template controls (topic-above-posts/discourse-templates-button.gjs)
//! and the composer options menu item. static/js/templates.js opens the
//! list and inserts the template an item's button picks.

use std::collections::HashMap;

use super::Template;
use super::variables::{self, Context};
use crate::i18n::I18n;
use crate::topic_list_view::{escape, icon};

/// select-kit's tag-drop `ALL_TAGS_ID`
pub const ALL_TAGS_ID: &str = "all-tags";
/// select-kit's tag-drop `NO_TAG_ID`
pub const NO_TAG_ID: &str = "no-tags";

/// What the list reads beside the templates.
pub struct Cx<'a> {
    pub i18n: &'a I18n,
    pub base_path: &'a str,
    pub tagging: bool,
    pub tag_style: &'a str,
}

impl Cx<'_> {
    fn t(&self, key: &str) -> String {
        self.i18n.t(&format!("js.{key}")).unwrap_or(key).to_string()
    }
}

/// A tag the templates carry, with how many (the TagDrop's content).
#[derive(Debug, Clone, PartialEq)]
pub struct AvailableTag {
    pub name: String,
    pub count: usize,
}

/// `availableTags`, sorted as the plugin's TagDrop sorts `topTags`: by
/// count descending, then by name.
pub fn available_tags(templates: &[Template]) -> Vec<AvailableTag> {
    let mut out: Vec<AvailableTag> = Vec::new();
    for tag in templates.iter().flat_map(|t| t.tags.iter().flatten()) {
        match out.iter_mut().find(|a| &a.name == tag) {
            Some(a) => a.count += 1,
            None => out.push(AvailableTag {
                name: tag.clone(),
                count: 1,
            }),
        }
    }
    out.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));
    out
}

/// The selected tag the list keeps: a stored one while it is no-tags or
/// among the available tags, else all tags (the stored one is then
/// dropped); always all tags without tagging.
pub fn selected_tag(cx: &Cx, requested: Option<&str>, available: &[AvailableTag]) -> String {
    match requested {
        Some(tag)
            if cx.tagging && (tag == NO_TAG_ID || available.iter().any(|a| a.name == tag)) =>
        {
            tag.to_string()
        }
        _ => ALL_TAGS_ID.to_string(),
    }
}

/// `filteredReplies`: the templates whose title (2) or else content (1)
/// holds the keyword, in the selected tag, by relevance, usage, then
/// title.
pub fn filtered<'t>(templates: &'t [Template], filter: &str, tag: &str) -> Vec<&'t Template> {
    let filter = filter.to_lowercase();
    let mut scored: Vec<(u8, &Template)> = templates
        .iter()
        .filter_map(|t| {
            let score = if t.title.to_lowercase().contains(&filter) {
                2
            } else if t.content.to_lowercase().contains(&filter) {
                1
            } else {
                0
            };
            (score != 0).then_some((score, t))
        })
        .filter(|(_, t)| {
            if tag == ALL_TAGS_ID {
                return true;
            }
            let tags = t.tags.as_deref().unwrap_or_default();
            if tag == NO_TAG_ID && tags.is_empty() {
                return true;
            }
            tags.iter().any(|name| name == tag)
        })
        .collect();
    scored.sort_by(|(sa, a), (sb, b)| {
        sb.cmp(sa)
            .then_with(|| b.usages.cmp(&a.usages))
            .then_with(|| a.title.cmp(&b.title))
    });
    scored.into_iter().map(|(_, t)| t).collect()
}

/// The context's hidden inputs, sent with every filter request.
fn context_inputs(context: &Context) -> String {
    let input = |name: &str, value: String| {
        format!(
            "<input type=\"hidden\" name=\"{name}\" value=\"{}\">",
            escape(&value)
        )
    };
    match context {
        Context::None => String::new(),
        Context::Composer {
            topic_id,
            post_number,
        } => {
            let mut out = input("context", "composer".into());
            if let Some(id) = topic_id {
                out.push_str(&input("topic_id", id.to_string()));
            }
            if let Some(n) = post_number {
                out.push_str(&input("post_number", n.to_string()));
            }
            out
        }
        Context::Chat {
            channel_id,
            reply_to_id,
        } => {
            let mut out = input("context", "chat".into());
            out.push_str(&input("chat_channel_id", channel_id.to_string()));
            if let Some(id) = reply_to_id {
                out.push_str(&input("chat_reply_to_id", id.to_string()));
            }
            out
        }
    }
}

/// dDiscourseTag with a count (the tag drop's rows).
fn discourse_tag(cx: &Cx, name: &str, count: usize) -> String {
    let mut classes = String::from("discourse-tag");
    if !cx.tag_style.is_empty() {
        classes.push(' ');
        classes.push_str(cx.tag_style);
    }
    format!(
        "<span data-tag-name=\"{}\" class=\"{classes}\">{}</span><span class=\"discourse-tag-count\">x{count}</span>",
        escape(&name.to_lowercase()),
        escape(name)
    )
}

/// The plugin's TagDrop (a select-kit combo box): the selection in its
/// header, the shortcuts and the available tags as rows, each picking
/// its tag through htmx.
fn tag_drop(cx: &Cx, available: &[AvailableTag], selected: &str) -> String {
    let url = format!("{}/live/templates", cx.base_path);
    let row = |value: &str, name: &str, inner: String, is_selected: bool| {
        let vals = serde_json::json!({ "tag": value }).to_string();
        format!(
            "<li aria-checked=\"{is_selected}\" role=\"menuitemradio\" data-name=\"{n}\" data-value=\"{v}\" title=\"{n}\" tabindex=\"0\" class=\"select-kit-row tag-row{}\" hx-get=\"{url}\" hx-include=\"closest .templates-filterable-list\" hx-vals=\"{}\" hx-target=\"closest .templates-filterable-list\" hx-swap=\"outerHTML\" hx-push-url=\"false\">{inner}</li>",
            if is_selected {
                " is-highlighted is-selected"
            } else {
                ""
            },
            escape(&vals),
            n = escape(name),
            v = escape(value),
        )
    };
    let mut rows = String::new();
    if selected != ALL_TAGS_ID {
        let label = cx.t("tagging.selector_remove_filter");
        rows.push_str(&row(
            ALL_TAGS_ID,
            &label,
            format!("<span class=\"name\">{}</span>", escape(&label)),
            false,
        ));
    }
    if selected != NO_TAG_ID {
        let label = cx.t("tagging.selector_no_tags");
        rows.push_str(&row(
            NO_TAG_ID,
            &label,
            format!("<span class=\"name\">{}</span>", escape(&label)),
            false,
        ));
    }
    for tag in available {
        rows.push_str(&row(
            &tag.name,
            &tag.name,
            discourse_tag(cx, &tag.name, tag.count),
            tag.name == selected,
        ));
    }
    let label = match selected {
        ALL_TAGS_ID => cx.t("tagging.selector_tags"),
        NO_TAG_ID => cx.t("tagging.selector_no_tags"),
        tag => tag.to_string(),
    };
    let label = escape(&label);
    let aria = escape(
        &cx.i18n
            .t_with("js.select_kit.filter_by", &[("name", &label)])
            .unwrap_or_default(),
    );
    let value = if selected == ALL_TAGS_ID {
        String::new()
    } else {
        format!(" data-value=\"{}\"", escape(selected))
    };
    format!(
        "<details class=\"select-kit single-select combobox combo-box tag-drop{}\"><summary aria-label=\"{aria}\" name=\"{aria}\" data-name=\"{label}\"{value} tabindex=\"0\" class=\"select-kit-header single-select-header combo-box-header tag-drop-header\"><div class=\"select-kit-header-wrapper\"><div class=\"select-kit-selected-name selected-name choice\" data-name=\"{label}\"{value} title=\"{label}\"><span class=\"name\">{label}</span></div>{}</div></summary><div class=\"select-kit-body\"><div class=\"select-kit-filter is-expanded\"><input class=\"filter-input\" autocomplete=\"off\" placeholder=\"{}\" spellcheck=\"false\" type=\"search\"></div><ul aria-live=\"polite\" class=\"select-kit-collection\" role=\"menu\">{rows}</ul></div></details>",
        if selected == ALL_TAGS_ID {
            ""
        } else {
            " has-selection"
        },
        icon("angle-down", Some("angle-icon")),
        escape(&cx.t("select_kit.filter_placeholder")),
    )
}

/// d-templates/item.gjs and item-content.gjs: the template's title with
/// its apply button, and the source link and cooked content it opens to.
/// The button carries the title and content with the variables replaced,
/// for static/js/templates.js to insert.
fn item(
    cx: &Cx,
    template: &Template,
    cooked: &str,
    variables: &HashMap<&'static str, String>,
) -> String {
    let (title, content) =
        variables::replace_variables(&template.title, &template.content, variables);
    format!(
        "<details class=\"template-item\" id=\"template-item-{id}\"><summary class=\"template-item-title\"><div class=\"template-item-title-text\">{}</div><div class=\"actions\"><button class=\"btn no-text btn-icon templates-apply\" type=\"button\" data-template-id=\"{id}\" data-title=\"{}\" data-content=\"{}\">{}<span aria-hidden=\"true\">&#8203;</span></button></div></summary><div class=\"templates-content\"><a href=\"{}/t/{}/{id}\" class=\"template-item-source-link\">{} {}</a><div>{cooked}</div></div></details>",
        escape(&template.title),
        escape(&title),
        escape(&content),
        icon("far-clipboard", None),
        cx.base_path,
        escape(&template.slug),
        icon("crosshairs", None),
        escape(&cx.t("templates.source")),
        id = template.id,
    )
}

/// The list's items, as the filter's swap replaces them.
pub fn items(
    cx: &Cx,
    templates: &[&Template],
    cooked: &HashMap<i32, String>,
    variables: &HashMap<&'static str, String>,
) -> String {
    let mut out = String::from("<div class=\"templates-list\">");
    for t in templates {
        out.push_str(&item(
            cx,
            t,
            cooked.get(&t.id).map(String::as_str).unwrap_or_default(),
            variables,
        ));
    }
    out.push_str("</div>");
    out
}

/// What the filterable list shows.
pub struct List<'a> {
    pub templates: &'a [Template],
    pub filter: &'a str,
    pub requested_tag: Option<&'a str>,
    pub context: &'a Context,
    pub variables: &'a HashMap<&'static str, String>,
    /// The cooked content of the templates shown, by id.
    pub cooked: &'a HashMap<i32, String>,
}

/// d-templates/filterable-list.gjs, loaded: the filter bar (the tag drop
/// while tagging is on, the keyword field) and the list. The keyword
/// field asks for the list again as it changes; `data-selected-tag` is
/// what the keyValueStore keeps (empty to drop it).
pub fn filterable_list(cx: &Cx, list: &List) -> String {
    let available = if cx.tagging {
        available_tags(list.templates)
    } else {
        Vec::new()
    };
    let selected = selected_tag(cx, list.requested_tag, &available);
    let shown = filtered(list.templates, list.filter, &selected);
    let mut out = format!(
        "<div class=\"templates-filterable-list\" data-selected-tag=\"{}\">{}<input type=\"hidden\" name=\"tag\" value=\"{}\"><div class=\"templates-filter-bar\">",
        if selected == ALL_TAGS_ID {
            String::new()
        } else {
            escape(&selected)
        },
        context_inputs(list.context),
        escape(&selected),
    );
    if cx.tagging {
        out.push_str(&tag_drop(cx, &available, &selected));
    }
    out.push_str(&format!(
        "<input class=\"templates-filter\" placeholder=\"{}\" type=\"text\" name=\"filter\" value=\"{}\" autocomplete=\"off\" hx-get=\"{}/live/templates\" hx-trigger=\"input changed\" hx-sync=\"this:replace\" hx-include=\"closest .templates-filterable-list\" hx-target=\"next .templates-list\" hx-select=\".templates-list\" hx-swap=\"outerHTML\" hx-push-url=\"false\"></div>",
        escape(&cx.t("templates.filter_hint")),
        escape(list.filter),
        cx.base_path,
    ));
    out.push_str(&items(cx, &shown, list.cooked, list.variables));
    out.push_str("</div>");
    out
}

/// The templates the list shows, to cook only those.
pub fn shown_ids(cx: &Cx, list: &List) -> Vec<i32> {
    let available = if cx.tagging {
        available_tags(list.templates)
    } else {
        Vec::new()
    };
    let selected = selected_tag(cx, list.requested_tag, &available);
    filtered(list.templates, list.filter, &selected)
        .into_iter()
        .map(|t| t.id)
        .collect()
}

/// The topic's template controls (shown for a template to a member who
/// may create topics): copy the first post's raw, and a new topic from
/// it with the category and tags its options header names.
pub fn topic_controls(
    cx: &Cx,
    topic_id: i64,
    options: &super::TemplateOptions,
    category_id: Option<i32>,
) -> String {
    format!(
        "<div class=\"template-topic-controls\" data-topic-id=\"{topic_id}\" data-body=\"{}\" data-category-id=\"{}\" data-tags=\"{}\"><button class=\"btn btn-icon-text btn-default template-copy\" type=\"button\">{}<span class=\"d-button-label\">{}</span></button><button class=\"btn btn-icon-text btn-default template-new-topic\" type=\"button\">{}<span class=\"d-button-label\">{}</span></button></div>",
        escape(&options.body),
        category_id.map(|id| id.to_string()).unwrap_or_default(),
        escape(options.tags.as_deref().unwrap_or_default()),
        icon("copy", None),
        escape(&cx.t("templates.copy")),
        icon("plus", None),
        escape(&cx.t("templates.new_topic")),
    )
}

/// The topic-above-posts connector for a template's page (the topic
/// view's `is_template`, a member who can_create_topic): the controls,
/// from the first post's raw. Nothing without a first post to read.
pub async fn topic_page_controls(
    conn: &mut sqlx::PgConnection,
    settings: &crate::site_settings::SiteSettings,
    cx: &Cx<'_>,
    guardian: &crate::guardian::Guardian,
    topic_id: i64,
) -> Result<String, crate::plugins::PluginError> {
    let raw: Option<String> = sqlx::query_scalar(
        "SELECT raw FROM posts WHERE topic_id = $1 AND post_number = 1 AND deleted_at IS NULL",
    )
    .bind(i32::try_from(topic_id).unwrap_or(0))
    .fetch_optional(&mut *conn)
    .await?;
    let Some(raw) = raw else {
        return Ok(String::new());
    };
    let options = super::parse_template_options(&raw);
    let category = match options.category_slug.as_deref() {
        Some(slug) => super::find_category(conn, settings, guardian, slug).await?,
        None => None,
    };
    Ok(topic_controls(cx, topic_id, &options, category))
}

/// The composer options menu item (addComposerToolbarPopupMenuOption,
/// unnamed), before core's wrap toggle.
pub fn add_popup_option(options: &mut Vec<crate::composer_view::PopupOption>) {
    let item = crate::composer_view::PopupOption {
        name: "discourse-templates",
        icon: "far-clipboard",
        label: "templates.insert_template",
        shortcut: None,
        always_show_shortcut: false,
        show_active_icon: false,
        when: None,
        ported: true,
        named: false,
    };
    let at = options
        .iter()
        .position(|o| o.name == "toggle-wrap")
        .unwrap_or(options.len());
    options.insert(at, item);
}

/// static/js/templates.js's labels and whether the member can chat (the
/// chat composer's button, addChatIntegration), as its script tag's data
/// attributes.
pub fn script_attrs(i18n: &I18n, can_chat: bool) -> String {
    let t = |key: &str| i18n.t(&format!("js.{key}")).unwrap_or(key).to_string();
    format!(
        " data-label-insert=\"{}\" data-label-close=\"{}\" data-chat=\"{can_chat}\"",
        escape(&t("templates.insert_template")),
        escape(&t("templates.close_templates")),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn template(id: i32, title: &str, content: &str, tags: &[&str], usages: i32) -> Template {
        Template {
            id,
            title: title.into(),
            slug: "s".into(),
            content: content.into(),
            tags: Some(tags.iter().map(|t| t.to_string()).collect()),
            usages,
        }
    }

    fn ids(list: Vec<&Template>) -> Vec<i32> {
        list.into_iter().map(|t| t.id).collect()
    }

    #[test]
    fn filter_scores_title_over_content_then_usage_then_title() {
        let all = vec![
            template(1, "Bravo", "has apple", &[], 0),
            template(2, "Apple pie", "x", &[], 0),
            template(3, "Alpha", "apple too", &[], 5),
            template(4, "Zulu", "nothing", &[], 9),
        ];
        assert_eq!(ids(filtered(&all, "APPLE", ALL_TAGS_ID)), vec![2, 3, 1]);
        assert_eq!(ids(filtered(&all, "", ALL_TAGS_ID)), vec![4, 3, 2, 1]);
    }

    #[test]
    fn filter_by_tag_and_no_tags() {
        // tag_filtering_spec: tag_1, tag_2, both and none.
        let all = vec![
            template(1, "a", "", &["tag1"], 0),
            template(2, "b", "", &["tag2"], 0),
            template(3, "c", "", &["tag1", "tag2"], 0),
            template(4, "d", "", &[], 0),
        ];
        assert_eq!(ids(filtered(&all, "", ALL_TAGS_ID)), vec![1, 2, 3, 4]);
        assert_eq!(ids(filtered(&all, "", "tag1")), vec![1, 3]);
        assert_eq!(ids(filtered(&all, "", "tag2")), vec![2, 3]);
        assert_eq!(ids(filtered(&all, "", NO_TAG_ID)), vec![4]);
        let tags = available_tags(&all);
        assert_eq!(
            tags,
            vec![
                AvailableTag {
                    name: "tag1".into(),
                    count: 2
                },
                AvailableTag {
                    name: "tag2".into(),
                    count: 2
                },
            ]
        );
    }
}
