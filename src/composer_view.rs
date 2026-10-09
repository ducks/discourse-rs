//! The composer (Ember's #reply-control) in its markdown mode, rendered
//! once per page for a member and opened by static/js/composer.js to reply,
//! create a topic or edit a post. The markup follows composer-container,
//! composer-editor, d-editor and the category chooser's select-kit; the
//! script sets the mode, its title and submit label (rendered here into
//! data attributes) and posts through /posts.
//!
//! Not ported yet: the preview (Rust as WASM, next), drafts, uploads, tags,
//! the heading, emoji and options menus, the composer actions menu and the
//! rich text editor.

use std::collections::HashMap;

use crate::topic_list_view::{ListCategory, ListContext, category_badge, escape, icon, t, t_count};

/// What the category chooser lists of a category.
pub struct ChooserCategory {
    pub id: i64,
    pub topic_count: i64,
    pub description_text: Option<String>,
}

/// The categories the member may create topics in, ordered as
/// categoriesList orders them: by topic count, each parent before its
/// children.
pub fn chooser_order(
    categories: &HashMap<i64, ListCategory>,
    allowed: &[ChooserCategory],
) -> Vec<i64> {
    let mut by_count: Vec<&ChooserCategory> = allowed.iter().collect();
    by_count.sort_by_key(|c| (std::cmp::Reverse(c.topic_count), c.id));
    fn add(
        out: &mut Vec<i64>,
        all: &[&ChooserCategory],
        cats: &HashMap<i64, ListCategory>,
        parent: Option<i64>,
    ) {
        for c in all {
            if cats.get(&c.id).and_then(|l| l.parent_id) == parent {
                out.push(c.id);
                add(out, all, cats, Some(c.id));
            }
        }
    }
    let mut out = Vec::new();
    add(&mut out, &by_count, categories, None);
    out
}

/// A badge as a span (categoryBadgeHTML with link: false), with extra
/// content inside the wrapper.
fn badge_span(list: &ListContext, category: &ListCategory, inside: &str) -> String {
    let link = category_badge(list, category);
    let span = link.replacen("<a ", "<span ", 1);
    // Drop the link's href.
    let span = match (span.find(" href=\""), span.find('>')) {
        (Some(at), Some(end)) if at < end => {
            let close = span[at + 7..]
                .find('"')
                .map(|i| at + 7 + i + 1)
                .unwrap_or(at);
            format!("{}{}", &span[..at], &span[close..])
        }
        _ => span,
    };
    match span.rfind("</a>") {
        Some(at) => format!("{}{inside}</span>", &span[..at]),
        None => span,
    }
}

fn row(list: &ListContext, c: &ChooserCategory, category: &ListCategory) -> String {
    let parent = category
        .parent_id
        .and_then(|p| list.categories.get(&p))
        .map(|p| badge_span(list, p, ""))
        .unwrap_or_default();
    let count = format!(
        "<span class=\"topic-count\" aria-label=\"{}\">× {}</span>",
        escape(&t_count(
            list,
            "category_row.topic_count",
            c.topic_count,
            &[]
        )),
        c.topic_count
    );
    let desc = c
        .description_text
        .as_deref()
        .filter(|d| !d.is_empty())
        .map(|d| {
            format!(
                "<div aria-hidden=\"true\" class=\"category-desc\"><span>{}</span></div>",
                escape(d)
            )
        })
        .unwrap_or_default();
    format!(
        "<div class=\"category-row select-kit-row\" data-name=\"{n}\" data-title=\"{n}\" data-value=\"{}\" role=\"menuitemradio\" tabindex=\"-1\" title=\"{n}\"><div class=\"category-status\">{parent}{}</div>{desc}</div>",
        c.id,
        badge_span(list, category, &count),
        n = escape(&category.name)
    )
}

/// The toolbar's buttons: the ones whose action is a markdown edit.
/// What the composer offers for uploads (`allowUpload`): the toolbar
/// button's icon (`uploadIcon`) and the file picker's accepted extensions,
/// None when every extension is allowed.
pub struct UploadUi {
    pub icon: &'static str,
    pub accept: Option<String>,
}

impl UploadUi {
    /// `authorizedExtensions`, `authorizesAllExtensions` and
    /// `allowsAttachments` from the extension settings, for staff or not;
    /// None when nothing may be uploaded (`authorizesOneOrMoreExtensions`).
    pub fn for_user(authorized: &str, for_staff: &str, staff: bool) -> Option<UploadUi> {
        // extensionsToArray
        let to_array = |exts: &str| -> Vec<String> {
            exts.to_lowercase()
                .chars()
                .filter(|c| !c.is_whitespace() && *c != '.')
                .collect::<String>()
                .split('|')
                .filter(|e| !e.contains('*'))
                .map(str::to_string)
                .collect()
        };
        let all = authorized.contains('*') || (staff && for_staff.contains('*'));
        let mut extensions = to_array(authorized);
        if staff {
            extensions.extend(to_array(for_staff));
        }
        let extensions: Vec<String> = extensions.into_iter().filter(|e| !e.is_empty()).collect();
        if !all && extensions.is_empty() {
            return None;
        }
        let is_image = |e: &str| {
            matches!(
                e,
                "png"
                    | "webp"
                    | "jpg"
                    | "jpeg"
                    | "gif"
                    | "svg"
                    | "ico"
                    | "heic"
                    | "heif"
                    | "avif"
                    | "jxl"
            )
        };
        let images = extensions.iter().filter(|e| is_image(e)).count();
        let allows_attachments = all || extensions.len() > images;
        Some(UploadUi {
            icon: if allows_attachments {
                "upload"
            } else {
                "far-image"
            },
            accept: (!all).then(|| {
                extensions
                    .iter()
                    .map(|e| format!(".{e}"))
                    .collect::<Vec<_>>()
                    .join(",")
            }),
        })
    }
}

fn toolbar(list: &ListContext, upload: Option<&UploadUi>) -> String {
    let mut buttons = vec![
        ("bold", "bold", "composer.bold_title"),
        ("italic", "italic", "composer.italic_title"),
        ("link", "link", "composer.link_title"),
        ("blockquote", "quote-right", "composer.blockquote_title"),
        ("code", "code", "composer.code_title"),
    ];
    // extraButtons: upload at the end of the insertions.
    if let Some(upload) = upload {
        buttons.push(("upload", upload.icon, "upload"));
    }
    buttons.push(("list", "list", "composer.ulist_title"));
    let mut out = String::new();
    for (class, icon_name, title) in buttons {
        out.push_str(&format!(
            "<button class=\"btn no-text btn-icon toolbar__button {class}\" data-action=\"{class}\" tabindex=\"-1\" title=\"{}\" type=\"button\">{}<span aria-hidden=\"true\">&#8203;</span></button>",
            escape(&t(list, title)),
            icon(icon_name, None)
        ));
    }
    out
}

/// The closed composer with its category chooser. `default_category` is
/// the category a new topic starts in (default_composer_category).
/// `preview` is where the preview's WebAssembly and render settings are
/// served; None leaves the composer without a preview.
pub fn render(
    list: &ListContext,
    allowed: &[ChooserCategory],
    default_category: Option<i64>,
    preview: Option<(&str, &str)>,
    allow_preview: bool,
    upload: Option<&UploadUi>,
) -> String {
    let control = |class: &str, icon_name: &str, title: &str| {
        format!(
            "<button class=\"btn no-text btn-icon btn-transparent {class} btn-small\" title=\"{}\" type=\"button\">{}<span aria-hidden=\"true\">&#8203;</span></button>",
            escape(&t(list, title)),
            icon(icon_name, None)
        )
    };
    let action = |icon_name: &str, label: &str| {
        format!(
            "{}<span class=\"d-button-label\">{}</span>",
            icon(icon_name, None),
            escape(label)
        )
    };
    let by_id: HashMap<i64, &ChooserCategory> = allowed.iter().map(|c| (c.id, c)).collect();
    let rows: String = chooser_order(list.categories, allowed)
        .iter()
        .filter_map(|id| Some(row(list, by_id.get(id)?, list.categories.get(id)?)))
        .collect();
    let selected = default_category
        .filter(|id| by_id.contains_key(id))
        .and_then(|id| list.categories.get(&id));
    let header = match selected {
        Some(c) => format!(
            "<div class=\"select-kit-selected-name selected-name choice\" data-name=\"{n}\" data-value=\"{}\" title=\"{n}\"><span class=\"name\">{}</span></div>",
            c.id,
            badge_span(list, c, ""),
            n = escape(&c.name)
        ),
        None => format!(
            "<div class=\"select-kit-selected-name selected-name choice\"><span class=\"name\">{}</span></div>",
            escape(&t(list, "category.choose").replace("&hellip;", "…"))
        ),
    };
    let chooser = format!(
        "<details class=\"select-kit single-select combobox combo-box category-chooser{}\"><summary class=\"select-kit-header single-select-header combo-box-header\" data-value=\"{}\" tabindex=\"0\"><div class=\"select-kit-header-wrapper\">{header}{}</div></summary><div class=\"select-kit-body\"><div class=\"select-kit-filter is-expanded\"><input class=\"filter-input\" autocomplete=\"off\" placeholder=\"{}\" spellcheck=\"false\" type=\"search\">{}</div><ul aria-live=\"polite\" class=\"select-kit-collection\" role=\"menu\">{rows}</ul></div></details>",
        if selected.is_some() {
            " has-selection"
        } else {
            ""
        },
        selected.map(|c| c.id.to_string()).unwrap_or_default(),
        icon("angle-down", Some("angle-icon")),
        escape(&t(list, "select_kit.filter_placeholder")),
        icon("magnifying-glass", Some("filter-icon")),
    );
    // ComposerToggles' preview toggle; composer.js sets its title and
    // `active` from the stored preference.
    let preview_toggle = match preview {
        Some(_) => format!(
            "<button class=\"btn no-text btn-icon btn-transparent btn-mini-toggle toggle-preview\" title=\"{}\" type=\"button\">{}<span aria-hidden=\"true\">&#8203;</span></button>",
            escape(&t(list, "composer.hide_preview")),
            icon("angles-left", None)
        ),
        None => String::new(),
    };
    // The draft's status and the discard confirmation (DiscardDraftModal's
    // message), for composer.js.
    let draft_data = format!(
        " data-label-drafts-offline=\"{}\" data-label-edit-conflict=\"{}\" data-label-discard-confirm=\"{}\" data-label-discard-confirm-edit=\"{}\"",
        escape(&t(list, "composer.drafts_offline")),
        escape(&t(list, "composer.edit_conflict")),
        escape(&t(list, "post.cancel_composer.confirm")),
        escape(&t(list, "post.cancel_composer.confirm_edit")),
    );
    let draft_status = format!(
        "<div id=\"draft-status\" hidden><span class=\"draft-error\" title=\"\">{}</span></div>",
        icon("triangle-exclamation", None)
    );
    // DPickFilesButton's input (`#file-uploader`), the uploads' progress
    // line (`#file-uploading`), and the placeholder texts composer.js
    // writes while a file uploads.
    let (pick_files, file_uploading, upload_data) = match upload {
        Some(upload) => (
            format!(
                "<div class=\"pick-files-button\"><input{} id=\"file-uploader\" multiple type=\"file\"></div>",
                upload
                    .accept
                    .as_deref()
                    .map(|a| format!(" accept=\"{}\"", escape(a)))
                    .unwrap_or_default()
            ),
            format!(
                "<div id=\"file-uploading\" hidden><div class=\"spinner small\"></div><span></span><a href id=\"cancel-file-upload\">{}</a></div>",
                icon("xmark", None)
            ),
            format!(
                " data-label-uploading=\"{}\" data-label-processing=\"{}\" data-label-uploading-filename=\"{}\" data-label-processing-filename=\"{}\" data-label-clipboard=\"{}\" data-label-pasted-image=\"{}\"",
                escape(&t(list, "upload_selector.uploading")),
                escape(&t(list, "upload_selector.processing")),
                escape(&t(list, "uploading_filename")),
                escape(&t(list, "processing_filename")),
                escape(&t(list, "clipboard")),
                escape(&t(list, "upload_selector.default_image_alt_text")),
            ) + &format!(
                // I18n.toHumanSize's units, for an attachment's size.
                " data-size-units=\"{}\"",
                escape(
                    &serde_json::json!({
                        "byte_one": t(list, "number.human.storage_units.units.byte.one"),
                        "byte_other": t(list, "number.human.storage_units.units.byte.other"),
                        "kb": t(list, "number.human.storage_units.units.kb"),
                        "mb": t(list, "number.human.storage_units.units.mb"),
                        "gb": t(list, "number.human.storage_units.units.gb"),
                        "tb": t(list, "number.human.storage_units.units.tb"),
                        "format": t(list, "number.human.storage_units.format"),
                    })
                    .to_string()
                )
            ),
        ),
        None => (String::new(), String::new(), String::new()),
    };
    let preview_data = match preview {
        Some((wasm, settings)) => format!(
            " data-preview-wasm=\"{}\" data-preview-settings=\"{}\" data-label-show-preview=\"{}\" data-label-hide-preview=\"{}\"",
            escape(wasm),
            escape(settings),
            escape(&t(list, "composer.show_preview")),
            escape(&t(list, "composer.hide_preview")),
        ),
        None => String::new(),
    };
    format!(
        "<div id=\"reply-control\" class=\"closed hide-preview\" hx-preserve=\"true\" data-page-attrs=\"data-default-category\" data-allow-preview=\"{allow_preview}\"{preview_data}{draft_data}{upload_data} data-base-path=\"{}\" data-default-category=\"{}\" data-action-reply=\"{}\" data-action-create-topic=\"{}\" data-action-edit=\"{}\" data-submit-reply=\"{}\" data-submit-create-topic=\"{}\" data-submit-edit=\"{}\" data-label-reply-to-topic=\"{}\" data-label-fullscreen=\"{}\" data-label-exit-fullscreen=\"{}\" data-text-bold=\"{}\" data-text-italic=\"{}\" data-text-link=\"{}\" data-text-blockquote=\"{}\" data-text-code=\"{}\" data-text-list=\"{}\"><div class=\"d-resize-separator grippie\" aria-label=\"{}\" aria-orientation=\"horizontal\" role=\"separator\" tabindex=\"0\"></div><div class=\"reply-area with-category\" role=\"dialog\" hidden><div class=\"reply-to\"><div class=\"composer-action-title\"><span aria-level=\"1\" class=\"action-title\" role=\"heading\"><button class=\"btn btn-icon-text composer-actions-trigger btn-flat btn-icon-text composer-actions\" tabindex=\"-1\" type=\"button\"></button></span></div><div class=\"composer-controls\">{}{}{}</div></div><div class=\"toolbar-visible wmd-controls\"><div class=\"d-editor\"><div class=\"d-editor-container --markdown-editor-enabled\"><div class=\"d-editor-textarea-column\"><div class=\"composer-fields\"><div class=\"title-and-category\" hidden><div class=\"title-input\"><input aria-label=\"{title}\" autocomplete=\"off\" id=\"reply-title\" placeholder=\"{title}\" type=\"text\"></div><div class=\"category-input\">{chooser}</div></div></div><div class=\"d-editor-textarea-wrapper\"><div class=\"d-overflow-controls d-editor-button-bar__wrap\"><div class=\"d-overflow-controls__content d-editor-button-bar\" role=\"toolbar\">{}</div></div><textarea aria-label=\"{body}\" autocomplete=\"off\" class=\"d-editor-input --markdown-monospace\" placeholder=\"{body}\"></textarea></div></div><div class=\"d-editor-preview-wrapper\"><div class=\"d-editor-preview\"></div></div></div></div>{pick_files}</div><div class=\"submit-panel\"><div class=\"save-or-cancel\"><button class=\"btn btn-icon-text btn-primary create\" type=\"button\"></button><button class=\"btn discard-button btn-transparent\" title=\"{discard}\" type=\"button\"><span class=\"d-button-label\">{discard}</span></button></div>{file_uploading}{draft_status}<p class=\"composer-error\" role=\"alert\" hidden></p></div></div></div>",
        escape(list.base_path),
        selected.map(|c| c.id.to_string()).unwrap_or_default(),
        escape(&action(
            "share",
            &t(list, "composer.composer_actions.reply_to_topic.trigger")
        )),
        escape(&action(
            "far-pen-to-square",
            &t(list, "composer.composer_actions.create_topic.label")
        )),
        escape(&action(
            "pencil",
            &t(list, "composer.composer_actions.edit")
        )),
        escape(&action("reply", &t(list, "composer.reply"))),
        escape(&action(
            "far-pen-to-square",
            &t(list, "composer.create_topic")
        )),
        escape(&action("pencil", &t(list, "composer.save_edit"))),
        escape(&t(list, "composer.composer_actions.reply_to_topic.trigger")),
        escape(&t(list, "composer.enter_fullscreen")),
        escape(&t(list, "composer.exit_fullscreen")),
        escape(&t(list, "composer.bold_text")),
        escape(&t(list, "composer.italic_text")),
        escape(&t(list, "composer.link_description")),
        escape(&t(list, "composer.blockquote_text")),
        escape(&t(list, "composer.paste_code_text")),
        escape(&t(list, "composer.list_item")),
        escape(&t(list, "composer.resize")),
        preview_toggle.clone()
            + &control(
                "toggle-fullscreen",
                "discourse-expand",
                "composer.enter_fullscreen",
            ),
        control("toggler toggle-minimize", "minus", "composer.collapse"),
        control(
            "toggler toggle-save-and-close",
            "xmark",
            "composer.save_and_close"
        ),
        toolbar(list, upload),
        title = escape(&t(list, "composer.title_or_link_placeholder")),
        body = escape(&t(list, "composer.reply_placeholder")),
        discard = escape(&t(list, "composer.discard")),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn category(id: i64, parent: Option<i64>) -> ListCategory {
        ListCategory {
            id,
            name: format!("c{id}"),
            slug: format!("c{id}"),
            color: "0088CC".into(),
            text_color: "FFFFFF".into(),
            style_type: "square",
            emoji: None,
            icon: None,
            read_restricted: false,
            parent_id: parent,
            navigate_to_first_post_after_read: false,
        }
    }

    #[test]
    fn categories_by_topic_count_with_children_after_parents() {
        let cats: HashMap<i64, ListCategory> = [
            (1, category(1, None)),
            (2, category(2, None)),
            (3, category(3, Some(1))),
        ]
        .into_iter()
        .collect();
        let allowed = [
            ChooserCategory {
                id: 2,
                topic_count: 1,
                description_text: None,
            },
            ChooserCategory {
                id: 3,
                topic_count: 9,
                description_text: None,
            },
            ChooserCategory {
                id: 1,
                topic_count: 3,
                description_text: None,
            },
        ];
        assert_eq!(chooser_order(&cats, &allowed), [1, 3, 2]);
    }
}
