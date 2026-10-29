//! preferences/tracking.gjs and preferences/tags.gjs, with the
//! user-preferences Categories and Tags components. The choosers
//! (CategorySelector, TagChooser) are headers here; static/js/
//! prefs-choosers.js opens them, the categories' rows coming from the
//! page's template.

use serde_json::{Map, Value, json};

use super::{Combo, Prefs, checkbox, combo, save_attrs, save_controls, value_string};
use crate::AppError;
use crate::composer_view::{ChooserCategory, chooser_order};
use crate::topic_list_view::{ListContext, escape, icon, t};

/// A multi-select chooser's header (MultiSelectHeader), for a chooser
/// `kind` (category, tag, user) bound to the save attribute `key`.
/// `items` are (value, name).
pub(crate) fn multi_select(
    cx: &ListContext,
    class: &str,
    kind: &str,
    key: &str,
    items: &[(String, String)],
) -> String {
    let names: Vec<&str> = items.iter().map(|(_, n)| n.as_str()).collect();
    let values: Vec<&str> = items.iter().map(|(v, _)| v.as_str()).collect();
    let shown = if names.is_empty() {
        t(cx, "select_kit.default_header_text")
    } else {
        names.join(", ")
    };
    format!(
        "<details class=\"select-kit multi-select {class}{}\" data-chooser=\"{kind}\" data-pref=\"{key}\" data-items=\"{}\"><summary data-name=\"{}\" data-value=\"{}\" tabindex=\"0\" class=\"select-kit-header multi-select-header\"><div class=\"select-kit-header-wrapper\"><span class=\"formatted-selection\">{}</span>{}</div></summary><div class=\"select-kit-body\"></div></details>",
        if items.is_empty() {
            ""
        } else {
            " has-selection"
        },
        escape(&Value::Array(items.iter().map(|(v, n)| json!([v, n])).collect()).to_string()),
        escape(&names.join(",")),
        escape(&values.join(",")),
        escape(&shown),
        icon("plus", Some("angle-icon")),
    )
}

/// The categories the choosers list (Category.list, the viewer's site
/// categories, ordered as categoriesList orders them).
pub struct SiteCategories<'a> {
    pub allowed: &'a [ChooserCategory],
}

/// The rows every CategorySelector filters, in a template.
pub fn category_rows(cx: &ListContext, site: &SiteCategories) -> String {
    let rows: String = chooser_order(cx.categories, site.allowed)
        .into_iter()
        .filter_map(|id| {
            let c = site.allowed.iter().find(|c| c.id == id)?;
            let category = cx.categories.get(&id)?;
            // CategorySelector's rows leave out the description.
            let without_desc = ChooserCategory {
                id: c.id,
                topic_count: c.topic_count,
                description_text: None,
                minimum_required_tags: 0,
            };
            Some(crate::composer_view::row(cx, &without_desc, category))
        })
        .collect();
    format!("<template id=\"prefs-category-rows\">{rows}</template>")
}

fn category_items(cx: &ListContext, ids: &Value) -> Vec<(String, String)> {
    ids.as_array()
        .into_iter()
        .flatten()
        .filter_map(|id| {
            let id = id.as_i64()?;
            let c = cx.categories.get(&id)?;
            Some((id.to_string(), c.name.clone()))
        })
        .collect()
}

fn tag_items(tags: &Value) -> Vec<(String, String)> {
    tags.as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| {
            let name = t.as_str().or_else(|| t["name"].as_str())?;
            Some((name.to_string(), name.to_string()))
        })
        .collect()
}

/// The Categories component.
fn categories(p: &Prefs, can_see: bool) -> Result<String, AppError> {
    let cx = p.cx;
    let mute_all = p.setting("mute_all_categories_by_default")?;
    let homepage = format!("{}/", cx.base_path);
    let show = |state: &str| {
        if can_see {
            format!(
                "<a class=\"show-tracking\" href=\"{}?state={state}\">{}</a>",
                escape(&homepage),
                escape(&t(cx, "user.tracked_topics_link"))
            )
        } else {
            String::new()
        }
    };
    let control = |class: &str, icon_name: &str, label: &str, link: String, key: &str| {
        format!(
            "<div class=\"controls tracking-controls tracking-controls__{class}\"><label>{} {}</label>{link}{}</div>",
            crate::post_view::d_icon(icon_name, None),
            escape(&t(cx, label)),
            multi_select(
                cx,
                "category-selector",
                "category",
                key,
                &category_items(cx, &p.u[key])
            )
        )
    };
    let instructions =
        |key: &str| format!("<div class=\"instructions\">{}</div>", escape(&t(cx, key)));
    let mut out = format!(
        "<div class=\"control-group category-notifications\"><label class=\"control-label\">{}</label>{}{}{}{}{}{}",
        escape(&t(cx, "user.categories_settings")),
        control(
            "watched-categories",
            "d-watching",
            "user.watched_categories",
            show("watching"),
            "watched_category_ids"
        ),
        instructions("user.watched_categories_instructions"),
        control(
            "tracked-categories",
            "d-tracking",
            "user.tracked_categories",
            show("tracking"),
            "tracked_category_ids"
        ),
        instructions("user.tracked_categories_instructions"),
        control(
            "watched-first-categories",
            "d-watching-first",
            "user.watched_first_post_categories",
            String::new(),
            "watched_first_post_category_ids"
        ),
        instructions("user.watched_first_post_categories_instructions"),
    );
    if mute_all {
        out.push_str(&control(
            "regular-categories",
            "d-regular",
            "user.regular_categories",
            String::new(),
            "regular_category_ids",
        ));
        out.push_str(&instructions("user.regular_categories_instructions"));
    } else {
        out.push_str(&control(
            "muted-categories",
            "d-muted",
            "user.muted_categories",
            show("muted"),
            "muted_category_ids",
        ));
        let hide = p.s.get("remove_muted_tags_from_latest")?.to_s() != "never";
        out.push_str(&instructions(if hide {
            "user.muted_categories_instructions"
        } else {
            "user.muted_categories_instructions_dont_hide"
        }));
    }
    out.push_str("</div><span></span><br><span></span>");
    Ok(out)
}

/// The Tags component (empty without tagging).
pub(crate) fn tags(p: &Prefs) -> Result<String, AppError> {
    let cx = p.cx;
    if !p.setting("tagging_enabled")? {
        return Ok(String::new());
    }
    let control = |class: &str, icon_name: &str, icon_class: &str, label: &str, key: &str| {
        format!(
            "<div class=\"controls tracking-controls tracking-controls__{class}\"><label>{} {}</label>{}</div>",
            crate::post_view::d_icon(icon_name, Some(&format!("icon {icon_class}"))),
            escape(&t(cx, label)),
            multi_select(cx, "tag-chooser", "tag", key, &tag_items(&p.u[key]))
        )
    };
    let instructions =
        |key: &str| format!("<div class=\"instructions\">{}</div>", escape(&t(cx, key)));
    Ok(format!(
        "<div class=\"control-group tag-notifications\"><label class=\"control-label\">{}</label>{}{}{}{}{}{}{}{}</div>",
        escape(&t(cx, "user.tag_settings")),
        control(
            "watched-tags",
            "d-watching",
            "watching",
            "user.watched_tags",
            "watched_tags"
        ),
        instructions("user.watched_tags_instructions"),
        control(
            "tracked-tags",
            "d-tracking",
            "tracking",
            "user.tracked_tags",
            "tracked_tags"
        ),
        instructions("user.tracked_tags_instructions"),
        control(
            "watched-first-post-tags",
            "d-watching-first",
            "watching-first-post",
            "user.watched_first_post_tags",
            "watching_first_post_tags"
        ),
        instructions("user.watched_first_post_tags_instructions"),
        control(
            "muted-tags",
            "d-muted",
            "muted",
            "user.muted_tags",
            "muted_tags"
        ),
        instructions("user.muted_tags_instructions"),
    ))
}

/// User#save's data for the tag lists: each a comma-joined string.
fn join_tags(attrs: &mut Map<String, Value>) {
    for key in [
        "muted_tags",
        "tracked_tags",
        "watched_tags",
        "watching_first_post_tags",
    ] {
        if let Some(v) = attrs.get_mut(key) {
            let joined = tag_items(v)
                .into_iter()
                .map(|(n, _)| n)
                .collect::<Vec<_>>()
                .join(",");
            *v = json!(joined);
        }
    }
}

/// preferences/tracking.gjs
pub fn render(
    p: &Prefs,
    site: &SiteCategories,
    viewer_admin: bool,
) -> Result<(String, Map<String, Value>), AppError> {
    let cx = p.cx;
    let can_see = p.viewing_self();
    let combo_row = |id_class: &str,
                     setting: &str,
                     label: &str,
                     key: &str,
                     rows: Vec<(String, String)>| {
        format!(
            "<div class=\"controls controls-dropdown\" data-setting-name=\"{setting}\"><label>{}</label>{}</div>",
            escape(&t(cx, label)),
            combo(
                cx,
                &Combo {
                    id: None,
                    component_class: "",
                    class: id_class,
                    rows,
                    value: value_string(p.option(key)),
                    none: None,
                    key: Some(key),
                    filterable: false,
                }
            )
        )
    };
    let options = |prefix: &str, list: &[(i64, &str)]| -> Vec<(String, String)> {
        list.iter()
            .map(|(v, k)| (v.to_string(), t(cx, &format!("{prefix}.{k}"))))
            .collect()
    };
    // The user-preferences-tracking-topics outlet: discourse-assign's
    // level when assigned, its enum's names as values.
    let assign = p.setting("assign_enabled")?;
    let assign_row = if assign {
        combo_row(
            "",
            "user-notification-level-when-assigned",
            "user.notification_level_when_assigned.label",
            "notification_level_when_assigned",
            ["watch_topic", "track_topic", "do_nothing"]
                .iter()
                .map(|k| {
                    (
                        k.to_string(),
                        t(cx, &format!("user.notification_level_when_assigned.{k}")),
                    )
                })
                .collect(),
        )
    } else {
        String::new()
    };
    let mut out = format!(
        "<div class=\"user-preferences__tracking-topics-wrapper\"><label class=\"control-label\">{}</label><div class=\"user-preferences_tracking-topics-controls\">{}{}{}{}</div></div>",
        escape(&t(cx, "user.topics_settings")),
        combo_row(
            "duration",
            "user-new-topic-duration",
            "user.new_topic_duration.label",
            "new_topic_duration_minutes",
            options(
                "user.new_topic_duration",
                &[
                    (-1, "not_viewed"),
                    (1440, "after_1_day"),
                    (2880, "after_2_days"),
                    (10080, "after_1_week"),
                    (20160, "after_2_weeks"),
                    (-2, "last_here"),
                ]
            )
        ),
        combo_row(
            "",
            "user-auto-track-topics",
            "user.auto_track_topics",
            "auto_track_topics_after_msecs",
            options(
                "user.auto_track_options",
                &[
                    (-1, "never"),
                    (0, "immediately"),
                    (30000, "after_30_seconds"),
                    (60000, "after_1_minute"),
                    (120000, "after_2_minutes"),
                    (180000, "after_3_minutes"),
                    (240000, "after_4_minutes"),
                    (300000, "after_5_minutes"),
                    (600000, "after_10_minutes"),
                ]
            )
        ),
        combo_row(
            "",
            "user-notification-level-when-replying",
            "user.notification_level_when_replying.label",
            "notification_level_when_replying",
            options(
                "user.notification_level_when_replying",
                &[(3, "watch_topic"), (2, "track_topic"), (1, "do_nothing")]
            )
        ),
        assign_row,
    );
    out.push_str(&format!(
        "<div class=\"user-preferences__tracking-categories-tags-wrapper\"><div>{}</div><div>{}</div></div>",
        categories(p, can_see)?,
        tags(p)?
    ));
    // showMutePrecedenceSetting, which prefs-choosers.js keeps as the
    // choosers change.
    let has = |key: &str| p.u[key].as_array().is_some_and(|a| !a.is_empty());
    let precedence = (has("watched_category_ids") && has("muted_tags"))
        || (has("watched_tags") && has("muted_category_ids"));
    {
        out.push_str(&format!(
            "<div class=\"control-group user-preferences__watched-precedence-over-muted\"{}>{}</div>",
            if precedence { "" } else { " style=\"display: none;\"" },
            checkbox(
                "",
                Some("watched-precedence-over-muted"),
                p.option("watched_precedence_over_muted") == true,
                &t(cx, "user.watched_precedence_over_muted"),
                Some("watched_precedence_over_muted")
            )
        ));
    }
    if can_see || viewer_admin {
        out.push_str(&save_controls(cx));
    }
    out.push_str(&category_rows(cx, site));

    let mute_all = p.setting("mute_all_categories_by_default")?;
    let mut fields = vec![
        "new_topic_duration_minutes",
        "auto_track_topics_after_msecs",
        "notification_level_when_replying",
        "watched_precedence_over_muted",
    ];
    let tagging = p.setting("tagging_enabled")?;
    if tagging {
        fields.extend([
            "muted_tags",
            "tracked_tags",
            "watched_tags",
            "watching_first_post_tags",
        ]);
    }
    // addSaveableUserOption("notification_level_when_assigned", tracking).
    let plugin_options: &[&str] = if assign {
        &["notification_level_when_assigned"]
    } else {
        &[]
    };
    fields.extend(plugin_options.iter().copied());
    let mut attrs = save_attrs(p.u, &fields, plugin_options, &[]);
    if tagging {
        for key in [
            "muted_tags",
            "tracked_tags",
            "watched_tags",
            "watching_first_post_tags",
        ] {
            attrs.entry(key).or_insert(json!(""));
        }
        join_tags(&mut attrs);
    }
    // The category levels User#save sends, an empty one as [-1].
    for level in [
        if mute_all { "regular" } else { "muted" },
        "watched",
        "tracked",
        "watched_first_post",
    ] {
        let key = format!("{level}_category_ids");
        let ids: Vec<i64> = category_items(cx, &p.u[&key])
            .iter()
            .filter_map(|(id, _)| id.parse().ok())
            .collect();
        attrs.insert(
            key,
            if ids.is_empty() {
                json!([-1])
            } else {
                json!(ids)
            },
        );
    }
    Ok((out, attrs))
}

/// preferences/tags.gjs: the Tags component and Save Changes.
pub fn render_tags(p: &Prefs) -> Result<(String, Map<String, Value>), AppError> {
    let mut out = tags(p)?;
    out.push_str(&save_controls(p.cx));
    let mut attrs = save_attrs(
        p.u,
        &[
            "muted_tags",
            "tracked_tags",
            "watched_tags",
            "watching_first_post_tags",
        ],
        &[],
        &[],
    );
    for key in [
        "muted_tags",
        "tracked_tags",
        "watched_tags",
        "watching_first_post_tags",
    ] {
        attrs.entry(key).or_insert(json!(""));
    }
    join_tags(&mut attrs);
    Ok((out, attrs))
}
