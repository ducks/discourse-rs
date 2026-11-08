//! What the rich editor's extensions read through getContext
//! (static/js/rich-editor/editor.js), as #reply-control's
//! data-rich-editor-context: the client site settings they look at, the
//! strings they show (with a plural key's forms under `key.one` and so on,
//! picked by count in composer.js), the current user's staff flag, the
//! site's hashtag order for the composer and the bundled plugins on, whose
//! extensions the editor registers (static/js/rich-editor/plugins.js).

use serde_json::{Map, Value, json};

use crate::i18n::I18n;
use crate::site_settings::{SettingError, SiteSettings};

/// The client site settings the extensions read.
const SITE_SETTINGS: &[&str] = &[
    "default_code_lang",
    "here_mention",
    "unicode_usernames",
    "max_image_width",
    "max_image_height",
    "simultaneous_uploads",
    "enable_emoji_shortcuts",
    "authorized_extensions",
    "authorized_extensions_for_staff",
    "avatar_sizes",
];

/// The client strings the extensions show (client.en.yml, less `js.`).
const LABELS: &[&str] = &[
    "cancel",
    "composer.image",
    "composer.table.default_heading",
    "invalid_video_url",
    "cannot_render_video",
    "play_video",
    "composer.remove_grid",
    "composer.grid_mode_title",
    "composer.grid_mode_carousel",
    "composer.grid_mode_grid",
    "composer.heading_level_small_text",
    "upload_selector.default_image_alt_text",
    "composer.link_toolbar.edit",
    "composer.link_toolbar.copy",
    "composer.link_toolbar.remove",
    "composer.link_toolbar.visit",
    "composer.link_toolbar.link_copied",
    "composer.link_toolbar.load_preview",
    "composer.image_alt_text.aria_label",
    "composer.image_alt_text.title",
    "composer.image_toolbar.alt_text",
    "composer.image_toolbar.zoom_out",
    "composer.image_toolbar.zoom_in",
    "composer.image_toolbar.remove",
    "composer.image_toolbar.add_to_grid",
    "composer.image_toolbar.move_outside_grid",
    "composer.here_mention",
    "composer.group_mentioned_limit",
    "composer.group_mentioned",
    "composer.larger_group_mentioned",
    "composer.cannot_see_mention.category",
    "composer.cannot_see_mention.private",
    "composer.cannot_see_mention.muted_topic",
    "composer.cannot_see_mention.not_allowed",
    "composer.cannot_see_group_mention.not_mentionable",
    "composer.cannot_see_group_mention.some_not_allowed",
    "composer.cannot_see_group_mention.not_allowed",
    "composer.table.add_row",
    "composer.table.add_column",
    "composer.table.insert_row_above",
    "composer.table.insert_header_above",
    "composer.table.insert_row_below",
    "composer.table.insert_column_left",
    "composer.table.insert_column_right",
    "composer.table.duplicate_row",
    "composer.table.duplicate_column",
    "composer.table.move_row_up",
    "composer.table.move_row_down",
    "composer.table.move_column_left",
    "composer.table.move_column_right",
    "composer.table.clear_contents",
    "composer.table.delete_row",
    "composer.table.delete_column",
    "composer.table.delete_table",
    "composer.table.align_left",
    "composer.table.align_center",
    "composer.table.align_right",
    "composer.table.alignment_left",
    "composer.table.alignment_center",
    "composer.table.alignment_right",
    "composer.table.alignment_changed",
    "composer.table.row_actions",
    "composer.table.column_actions",
    "composer.table.row_inserted",
    "composer.table.header_inserted",
    "composer.table.column_inserted",
    "composer.table.row_duplicated",
    "composer.table.column_duplicated",
    "composer.table.row_moved_up",
    "composer.table.row_moved_down",
    "composer.table.column_moved_left",
    "composer.table.column_moved_right",
    "composer.table.contents_cleared",
    "composer.table.row_deleted",
    "composer.table.column_deleted",
    "composer.table.table_deleted",
    "composer.table.rows_added",
    "composer.table.columns_added",
    "composer.table.rows_removed",
    "composer.table.columns_removed",
    "composer.wrap_text",
    "composer.wrap_modal.title",
    "composer.wrap_modal.name_label",
    "composer.wrap_modal.attributes_label",
    "composer.wrap_modal.add_attribute",
    "composer.wrap_modal.attribute_key",
    "composer.wrap_modal.attribute_value",
    "composer.wrap_modal.remove_attribute",
    "composer.wrap_modal.apply",
    "composer.wrap_modal.unwrap",
    "composer.wrap_modal.no_attributes",
    "form_kit.errors.required",
    // the toolbar's (static/js/editor-toolbar.js): example texts, the
    // heading and list menus, their shortcuts
    "composer.bold_text",
    "composer.italic_text",
    "composer.heading_text",
    "composer.link_description",
    "composer.blockquote_text",
    "composer.code_text",
    "composer.code_title",
    "composer.paste_code_text",
    "composer.list_item",
    "composer.heading_level_n",
    "composer.heading_level_n_title",
    "composer.heading_level_paragraph",
    "composer.heading_level_paragraph_title",
    "composer.heading_level_small",
    "composer.heading_level_small_title",
    "composer.ulist_title",
    "composer.olist_title",
    // the options menu's and the list menu's plugin items
    "composer.details_title",
    "composer.details_text",
    "composer.graphviz_sample",
    "footnote.title",
    "checklist.composer.checklist",
    // UpsertHyperlink
    "composer.link_dialog_title",
    "composer.link_edit_title",
    "composer.link_url_label",
    "composer.link_url_placeholder",
    "composer.link_text_label",
    "composer.link_optional_text",
    "composer.link_dialog_action",
    "composer.link_edit_action",
    "composer.cancel",
    "shortcut_modifier_key.ctrl",
    "shortcut_modifier_key.alt",
    "shortcut_modifier_key.shift",
    // the plugins' extensions
    "composer.spoiler_text",
    "discourse_math.edit_math",
    "discourse_local_dates.edit_date",
    "discourse_policy.title",
    "discourse_policy.builder.edit",
    "discourse_policy.builder.groups.label",
    "discourse_policy.builder.version.label",
    "discourse_policy.builder.renew.label",
    "discourse_policy.builder.renew-start.label",
    "discourse_policy.builder.reminder.label",
    "discourse_policy.builder.accept.label",
    "discourse_policy.builder.revoke.label",
    "discourse_policy.builder.add-users-to-group.label",
    "discourse_policy.builder.private.label",
    "poll.voters",
    "poll.dynamic.enabled_hint",
    "poll.multiple.help.x_options",
    "poll.multiple.help.between_min_and_max_options",
    "poll.multiple.help.at_least_min_options",
    "poll.multiple.help.up_to_max_options",
    "poll.automatic_close.closes_in",
    "poll.automatic_close.age",
    "poll.results.vote.title",
    "poll.results.closed.title",
    "poll.results.staff.title",
    "poll.public.title",
    "poll.ui_builder.edit",
    "poll.ui_builder.poll_title.placeholder",
];

/// The bundled plugins with a rich editor extension, in their load order,
/// by the setting each is enabled by (`enabled_site_setting`; checklist's
/// initializer checks its own).
const PLUGINS: &[(&str, &str)] = &[
    ("checklist", "checklist_enabled"),
    ("details", "details_enabled"),
    ("local-dates", "discourse_local_dates_enabled"),
    ("math", "discourse_math_enabled"),
    ("policy", "policy_enabled"),
    ("footnote", "enable_markdown_footnotes"),
    ("poll", "poll_enabled"),
    ("spoiler", "spoiler_enabled"),
];

const PLURAL_FORMS: &[&str] = &["zero", "one", "two", "few", "many", "other"];

fn labels(i18n: &I18n) -> Map<String, Value> {
    let mut out = Map::new();
    for key in LABELS {
        if let Some(text) = i18n.t(&format!("js.{key}")) {
            out.insert((*key).to_string(), json!(text));
        }
        for form in PLURAL_FORMS {
            if let Some(text) = i18n.t(&format!("js.{key}.{form}")) {
                out.insert(format!("{key}.{form}"), json!(text));
            }
        }
    }
    out
}

/// The context JSON. `hashtag_order` is
/// `site.hashtag_configurations["topic-composer"]`.
pub fn context(
    i18n: &I18n,
    settings: &SiteSettings,
    staff: bool,
    hashtag_order: &[String],
) -> Result<Value, SettingError> {
    let mut site_settings = Map::new();
    for name in SITE_SETTINGS {
        site_settings.insert((*name).to_string(), json!(settings.get(name)?));
    }
    let mut plugins = Vec::new();
    for (name, setting) in PLUGINS {
        if settings.get(setting)?.truthy() {
            plugins.push(*name);
        }
    }
    Ok(json!({
        "siteSettings": site_settings,
        "labels": labels(i18n),
        "currentUser": { "staff": staff },
        "site": { "hashtag_configurations": { "topic-composer": hashtag_order } },
        "plugins": plugins,
    }))
}
