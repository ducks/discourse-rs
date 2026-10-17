//! The emoji pages (90g6), as Ember draws them: templates/admin-emojis.gjs
//! (DPageHeader with Add emoji, Import and the Settings and Emoji tabs)
//! over AdminConfigAreasEmojisList, EmojisNew with EmojiUploader, and
//! EmojisImport. Their script is static/js/admin-emoji.js.

use serde_json::Value;

use crate::admin_ui::{self, Action, Button};
use crate::admin_view::{AdminCx, Crumb, PageHeader, t, t_with};
use crate::topic_list_view::{escape, icon};

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}

/// The emoji route's outlet.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    List,
    Settings,
    New,
    Import,
}

/// admin-emojis.gjs: the header (hidden with its tabs on the new and
/// import routes) over the outlet.
pub fn page(cx: &AdminCx, tab: Tab, outlet: &str) -> String {
    let header = if matches!(tab, Tab::New | Tab::Import) {
        String::new()
    } else {
        header(cx, tab)
    };
    format!(
        "<div class=\"admin-emoji admin-config-page\">{header}<div class=\"admin-container admin-config-page__main-area\">{outlet}</div></div>"
    )
}

fn header(cx: &AdminCx, tab: Tab) -> String {
    let title = t(cx.i18n, "admin.config.emoji.title");
    let mut crumbs = vec![
        Crumb {
            label: t(cx.i18n, "admin_title"),
            path: "/admin",
        },
        Crumb {
            label: title.clone(),
            path: "/admin/config/emoji",
        },
    ];
    if tab == Tab::Settings {
        // AdminAreaSettings' own crumb.
        crumbs.push(Crumb {
            label: t(cx.i18n, "settings"),
            path: "/admin/config/emoji/settings",
        });
    }
    PageHeader {
        crumbs,
        title,
        actions: admin_ui::actions(
            cx,
            "d-page-header",
            &[
                Action::Primary(Button {
                    label: admin_ui::tr(cx, "admin.emoji.add"),
                    href: Some("/admin/config/emoji/new"),
                    ..Default::default()
                }),
                Action::Default(Button {
                    class: "admin-emoji__import",
                    label: admin_ui::tr(cx, "admin.emoji.import"),
                    href: Some("/admin/config/emoji/import"),
                    ..Default::default()
                }),
            ],
        ),
        description: Some(t(cx.i18n, "admin.config.emoji.header_description")),
        learn_more_url: None,
        tabs: Some(admin_ui::tabs(
            cx,
            &[
                (
                    "admin-emoji-tabs__settings",
                    "/admin/config/emoji/settings",
                    "settings",
                    tab == Tab::Settings,
                ),
                (
                    "admin-emoji-tabs__emoji",
                    "/admin/config/emoji",
                    "admin.emoji.title",
                    tab == Tab::List,
                ),
            ],
        )),
    }
    .render(cx)
}

/// AdminEmojis#emojiGroups: default, then each emoji's group once.
pub fn groups(emojis: &[Value]) -> Vec<String> {
    let mut out = vec!["default".to_string()];
    for e in emojis {
        let g = s(&e["group"]).to_string();
        if !out.contains(&g) {
            out.push(g);
        }
    }
    out
}

/// A custom emoji's row, as the list draws it while not selecting.
fn row(cx: &AdminCx, emoji: &Value) -> String {
    let name = escape(s(&emoji["name"]));
    let label = |key: &str| {
        format!(
            "<div class=\"d-table__mobile-label\">{}</div>",
            escape(&t(cx.i18n, key))
        )
    };
    format!(
        "<tr class=\"d-table__row\" data-emoji-name=\"{name}\" data-emoji-group=\"{group}\"><td class=\"d-table__cell --overview\"><img alt=\"{alt}\" class=\"emoji emoji-custom\" src=\"{url}\" title=\"{name}\"></td><td class=\"d-table__cell --detail\">{name_label}:{name}:</td><td class=\"d-table__cell --detail\">{group_label}{group}</td><td class=\"d-table__cell --detail\">{by_label}{by}</td><td class=\"d-table__cell --controls action\">{delete}</td></tr>",
        alt = escape(&t(cx.i18n, "admin.emoji.alt")),
        url = escape(s(&emoji["url"])),
        name_label = label("admin.emoji.name"),
        group = escape(s(&emoji["group"])),
        group_label = label("admin.emoji.group"),
        by_label = label("admin.emoji.created_by"),
        by = escape(s(&emoji["created_by"])),
        delete = Button {
            class: "btn-default btn-small d-table__cell-action-delete",
            label: admin_ui::tr(cx, "admin.emoji.delete"),
            ..Default::default()
        }
        .render()
    )
}

/// AdminConfigAreasEmojisList: DFilterControls (the name filter, the
/// groups dropdown, Select to export) over the table, or the empty list.
pub fn list(cx: &AdminCx, emojis: &[Value]) -> String {
    if emojis.is_empty() {
        return admin_ui::empty_list(
            cx,
            "admin-emoji__add-emoji",
            "admin.emoji.add",
            "/admin/config/emoji/new",
            "admin.emoji.no_emoji",
        );
    }
    let all = t(cx.i18n, "admin.emoji.all_groups");
    let mut options = format!(
        "<option class=\"d-native-select__option --selected\" value=\"all\" selected>{}</option>",
        escape(&all)
    );
    for g in groups(emojis) {
        options.push_str(&format!(
            "<option class=\"d-native-select__option\" value=\"{g}\">{g}</option>",
            g = escape(&g)
        ));
    }
    let reset = escape(&t(cx.i18n, "filter_controls.reset"));
    let rows: String = emojis.iter().map(|e| row(cx, e)).collect();
    format!(
        "<div class=\"d-filter-controls\"><div class=\"d-filter-controls__inputs\"><div class=\"filter-input-container\">{search}<input autocapitalize=\"none\" autocomplete=\"off\" autocorrect=\"off\" class=\"filter-input d-filter-controls__input\" placeholder=\"{placeholder}\" type=\"text\" value=\"\"></div></div><div class=\"d-filter-controls__dropdowns\"><select aria-label=\"{all}\" class=\"d-filter-controls__dropdown d-native-select\">{options}</select></div><button aria-label=\"{reset}\" class=\"btn no-text btn-icon btn-default d-filter-controls__reset\" disabled=\"\" title=\"{reset}\" type=\"button\">{undo}<span aria-hidden=\"true\">\u{200b}</span></button>{select}</div><table class=\"d-table admin-emoji-list\" id=\"custom_emoji\"><thead class=\"d-table__header\"><tr class=\"d-table__row\"><th class=\"d-table__header-cell\">{image}</th><th class=\"d-table__header-cell\">{name}</th><th class=\"d-table__header-cell\">{group}</th><th class=\"d-table__header-cell\" colspan=\"3\">{by}</th></tr></thead><tbody class=\"d-table__body\">{rows}</tbody></table>",
        search = icon("magnifying-glass", Some("-left")),
        placeholder = escape(&t(cx.i18n, "admin.emoji.filter_placeholder")),
        all = escape(&all),
        undo = icon("arrow-rotate-left", None),
        select = Button {
            class: "btn-default admin-emoji-list__select-to-export",
            icon: Some("download"),
            label: admin_ui::tr(cx, "admin.emoji.select_to_export"),
            ..Default::default()
        }
        .render(),
        image = escape(&t(cx.i18n, "admin.emoji.image")),
        name = escape(&t(cx.i18n, "admin.emoji.name")),
        group = escape(&t(cx.i18n, "admin.emoji.group")),
        by = escape(&t(cx.i18n, "admin.emoji.created_by")),
    )
}

/// EmojisNew: the back button over the card with EmojiUploader (the
/// name, the group ComboBox with allowAny, Choose files).
pub fn new(cx: &AdminCx, groups: &[String]) -> String {
    let filter_by = t_with(cx.i18n, "select_kit.filter_by", &[("name", "default")]);
    let rows: String = groups
        .iter()
        .enumerate()
        .map(|(index, g)| {
            let selected = g == "default";
            format!(
                "<li aria-checked=\"{selected}\" role=\"menuitemradio\" data-index=\"{index}\" data-name=\"{g}\" data-value=\"{g}\" title=\"{g}\" tabindex=\"0\" class=\"select-kit-row{}\"><span class=\"name\">{g}</span></li>",
                if selected {
                    " is-highlighted is-selected"
                } else {
                    ""
                },
                g = escape(g)
            )
        })
        .collect();
    let combo = format!(
        "<details class=\"select-kit single-select combobox combo-box has-selection\" id=\"emoji-group-selector\" data-value=\"default\"><summary aria-label=\"{fb}\" class=\"select-kit-header single-select-header combo-box-header\" data-name=\"default\" data-value=\"default\" id=\"emoji-group-selector-header\" name=\"{fb}\" tabindex=\"0\"><div class=\"select-kit-header-wrapper\"><div class=\"select-kit-selected-name selected-name choice\" data-name=\"default\" data-value=\"default\" title=\"default\"><span class=\"name\">default</span></div>{caret}</div></summary><div class=\"select-kit-body\" id=\"emoji-group-selector-body\"></div><template class=\"select-kit-rows\">{rows}</template></details>",
        fb = escape(&filter_by),
        caret = icon("angle-down", Some("angle-icon")),
    );
    let form = format!(
        "<div class=\"form-kit\"><div class=\"form-kit__container form-kit__field form-kit__field-input-text\"><label class=\"form-kit__container-title\">{name}</label><div class=\"form-kit__container-content --large\"><div class=\"form-kit__control-input-wrapper\"><input class=\"ember-text-field form-kit__control-input\" id=\"emoji-name\" name=\"name\" type=\"text\"></div></div></div><div class=\"form-kit__container form-kit__field form-kit__field-input-combo-box\"><label class=\"form-kit__container-title\">{group}</label><div class=\"form-kit__container-content --large\"><div class=\"form-kit__control-input-wrapper\">{combo}</div></div></div><div class=\"control-group\"><div class=\"input\"><input accept=\".gif,.png,.svg\" class=\"hidden-upload-field\" multiple=\"true\" type=\"file\">{choose}</div></div></div>",
        name = escape(&t(cx.i18n, "admin.emoji.name")),
        group = escape(&t(cx.i18n, "admin.emoji.group")),
        choose = Button {
            class: "btn-primary admin-emoji-uploader__choose",
            label: admin_ui::tr(cx, "admin.emoji.choose_files"),
            ..Default::default()
        }
        .render(),
    );
    format!(
        "{}<div class=\"admin-config-area\"><div class=\"admin-config-area__primary-content admin-emoji-form\"><section class=\"admin-config-area-card\"><div class=\"admin-config-area-card__header-wrapper\"><h3 class=\"admin-config-area-card__title\">{}</h3></div><div class=\"admin-config-area-card__content\">{form}</div></section></div></div>",
        admin_ui::back_button(cx, "/admin/config/emoji", "admin.emoji.back"),
        escape(&t(cx.i18n, "admin.emoji.add"))
    )
}

/// EmojisImport before a zip is chosen: the heading, the description
/// with the sample's link, the file input.
pub fn import(cx: &AdminCx) -> String {
    format!(
        "{}<div class=\"admin-emoji-import\"><h2 class=\"admin-emoji-import__heading\">{}</h2><p class=\"admin-emoji-import__description\">{}<br><a href=\"{}/emoji-import-sample.zip\" rel=\"noopener noreferrer\" target=\"_blank\">{}</a></p><div class=\"inputs\"><input accept=\".zip\" class=\"admin-emoji-import__file-input\" type=\"file\"></div></div>",
        admin_ui::back_button(cx, "/admin/config/emoji", "admin.emoji.back"),
        escape(&t(cx.i18n, "admin.emoji.import_heading")),
        escape(&t(cx.i18n, "admin.emoji.import_description")),
        cx.base_path,
        escape(&t(cx.i18n, "admin.emoji.import_download_sample")),
    )
}

/// The strings admin-emoji.js shows.
pub const STRINGS: &[&str] = &[
    "admin.emoji.delete_confirm",
    "admin.emoji.uploading",
    "admin.emoji.choose_files",
    "admin.emoji.export_count",
    "admin.export_json.button_text",
    "admin.emoji.export_failed",
    "admin.emoji.select_all",
    "admin.emoji.select",
    "admin.emoji.select_to_export",
    "admin.emoji.no_results",
    "admin.emoji.import_preview_heading",
    "admin.emoji.import_summary",
    "admin.emoji.import_category.new",
    "admin.emoji.import_category.conflict",
    "admin.emoji.import_category.invalid",
    "admin.emoji.import_unchanged_count",
    "admin.emoji.import_excluded",
    "admin.emoji.import_will_skip",
    "admin.emoji.import_use_incoming",
    "admin.emoji.import_keep_existing",
    "admin.emoji.import_incoming_alt",
    "admin.emoji.import_existing_alt",
    "admin.emoji.import_existing_group_alt",
    "admin.emoji.import_confirm",
    "admin.emoji.image",
    "admin.emoji.name",
    "admin.emoji.group",
    "admin.emoji.alt",
    "admin.emoji.created_by",
    "admin.emoji.delete",
    "cancel",
    "delete",
    "filter_controls.reset",
    "generic_error",
    "select_kit.filter_placeholder_with_any",
];
