//! The composer (Ember's #reply-control) in its markdown mode, rendered
//! once per page for a member and opened by static/js/composer.js to reply,
//! create a topic or edit a post. The markup follows composer-container,
//! composer-editor, d-editor and the category chooser's select-kit; the
//! script sets the mode, its title and submit label (rendered here into
//! data attributes) and posts through /posts, with the tag chooser
//! (MiniTagChooser) for a member who may tag topics.
//!
//! Not ported yet: the preview (Rust as WASM, next), drafts, uploads,
//! the heading, emoji and options menus, the composer actions menu and the
//! rich text editor.

use std::collections::HashMap;

use crate::topic_list_view::{ListCategory, ListContext, category_badge, escape, icon, t, t_count};

/// What the category chooser lists of a category.
pub struct ChooserCategory {
    pub id: i64,
    pub topic_count: i64,
    pub description_text: Option<String>,
    /// `Category#minimumRequiredTags`: the minimum, or the required tag
    /// groups' counts when they come to more (0 for none).
    pub minimum_required_tags: i64,
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
pub(crate) fn badge_span(list: &ListContext, category: &ListCategory, inside: &str) -> String {
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

pub(crate) fn row(list: &ListContext, c: &ChooserCategory, category: &ListCategory) -> String {
    let parent = category
        .parent_id
        .and_then(|p| list.categories.get(&p))
        .map(|p| badge_span(list, p, ""))
        .unwrap_or_default();
    // categoryBadgeHTML's topicCount: none for an empty category.
    let count = if c.topic_count == 0 {
        String::new()
    } else {
        format!(
            "<span class=\"topic-count\" aria-label=\"{}\">× {}</span>",
            escape(&t_count(
                list,
                "category_row.topic_count",
                c.topic_count,
                &[]
            )),
            c.topic_count
        )
    };
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
        "<div class=\"category-row select-kit-row\" data-name=\"{n}\" data-title=\"{n}\" data-value=\"{}\"{} role=\"menuitemradio\" tabindex=\"-1\" title=\"{n}\"><div class=\"category-status\">{parent}{}</div>{desc}</div>",
        c.id,
        if c.minimum_required_tags > 0 {
            format!(
                " data-minimum-required-tags=\"{}\"",
                c.minimum_required_tags
            )
        } else {
            String::new()
        },
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

/// One of the toolbar's options menu items (Composer#popupMenuOptions):
/// core's and the enabled bundled plugins' `addComposerToolbarPopupMenuOption`s.
pub struct PopupOption {
    pub name: &'static str,
    pub icon: &'static str,
    /// The label's js.* key.
    pub label: &'static str,
    /// The shortcut after the platform's modifier (`Shift+.`).
    pub shortcut: Option<&'static str>,
    pub always_show_shortcut: bool,
    pub show_active_icon: bool,
    /// When the item shows: `touch` (a touch device) or `edit-reply` (an
    /// edit of a reply); always without.
    pub when: Option<&'static str>,
    /// Whether composer.js carries out its action; the others refuse.
    pub ported: bool,
    /// Whether Ember gives the option a `name` (its button's data-name);
    /// footnote's, spoiler's and poll's have none.
    pub named: bool,
}

/// The options menu's items for the site's settings, in Ember's order:
/// core's quote, format code and insert table, change reply-to, the
/// plugins' (details, footnote, local dates, spoiler, poll), then wrap.
/// `can_create_poll` is the viewer's (staff or poll_create_allowed_groups).
pub fn popup_options(
    settings: &crate::site_settings::SiteSettings,
    can_create_poll: bool,
) -> Result<Vec<PopupOption>, crate::site_settings::SettingError> {
    let on = |name: &str| settings.get(name).map(|v| v.truthy());
    let option = |name, icon, label| PopupOption {
        name,
        icon,
        label,
        shortcut: None,
        always_show_shortcut: false,
        show_active_icon: false,
        when: None,
        ported: false,
        named: true,
    };
    let mut out = vec![
        PopupOption {
            ported: true,
            ..option("quote", "far-comment", "composer.quote_post_title")
        },
        PopupOption {
            when: Some("touch"),
            ..option("format-code", "code", "composer.code_title")
        },
        option("toggle-spreadsheet", "table", "composer.insert_table"),
        PopupOption {
            when: Some("edit-reply"),
            ..option(
                "change-reply-to",
                "share",
                "composer.change_reply_to.open_from_menu",
            )
        },
    ];
    if on("details_enabled")? {
        out.push(PopupOption {
            shortcut: Some("Shift+D"),
            ported: true,
            ..option("details", "angle-right", "details.title")
        });
    }
    if on("enable_markdown_footnotes")? {
        out.push(PopupOption {
            ported: true,
            named: false,
            ..option("footnote", "asterisk", "footnote.add")
        });
    }
    if crate::plugins::local_dates::enabled(settings)? {
        out.push(PopupOption {
            shortcut: Some("Shift+."),
            always_show_shortcut: true,
            ported: true,
            ..option("local-dates", "far-clock", "discourse_local_dates.title")
        });
    }
    if on("spoiler_enabled")? {
        out.push(PopupOption {
            show_active_icon: true,
            ported: true,
            named: false,
            ..option("spoiler", "wand-magic", "spoiler.title")
        });
    }
    if on("poll_enabled")? && can_create_poll {
        out.push(PopupOption {
            named: false,
            ..option("poll", "chart-bar", "poll.ui_builder.title")
        });
    }
    out.push(PopupOption {
        show_active_icon: true,
        ..option(
            "toggle-wrap",
            "right-to-bracket",
            "composer.apply_wrap_title",
        )
    });
    Ok(out)
}

/// The plugins whose options menu items aren't ported (their place among
/// the others not known either): the menu refuses to open while one is on.
pub fn unported_popup_plugins(
    settings: &crate::site_settings::SiteSettings,
) -> Result<Vec<&'static str>, crate::site_settings::SettingError> {
    let mut out = Vec::new();
    for (setting, plugin) in [
        ("discourse_math_enabled", "discourse-math"),
        ("discourse_post_event_enabled", "discourse-events"),
        ("policy_enabled", "discourse-policy"),
        ("discourse_graphviz_enabled", "discourse-graphviz"),
    ] {
        if settings.get(setting)?.truthy() {
            out.push(plugin);
        }
    }
    // discourse-templates adds its item for a member who can_use_templates,
    // which takes its categories or its private templates' tags.
    if settings.get("discourse_templates_enabled")?.truthy()
        && (!settings
            .get("discourse_templates_categories")?
            .to_s()
            .is_empty()
            || (settings
                .get("discourse_templates_enable_private_templates")?
                .truthy()
                && !settings
                    .get("discourse_templates_private_templates_tags")?
                    .to_s()
                    .is_empty()))
    {
        out.push("discourse-templates");
    }
    Ok(out)
}

/// The options trigger and what composer.js builds its menu from.
fn options_trigger(list: &ListContext, options: &[PopupOption], unported: &[&str]) -> String {
    let items: Vec<serde_json::Value> = options
        .iter()
        .map(|o| {
            serde_json::json!({
                "name": o.name,
                "icon": o.icon,
                "label": t(list, o.label),
                "shortcut": o.shortcut,
                "alwaysShowShortcut": o.always_show_shortcut,
                "showActiveIcon": o.show_active_icon,
                "when": o.when,
                "ported": o.ported,
                "named": o.named,
            })
        })
        .collect();
    let data = serde_json::json!({ "items": items, "unported": unported });
    format!(
        "<button aria-expanded=\"false\" class=\"btn no-text fk-d-menu__trigger toolbar-menu__options-trigger toolbar__button options toolbar-popup-menu-options\" tabindex=\"-1\" title=\"{}\" data-identifier=\"toolbar-menu__options\" data-trigger=\"\" type=\"button\" data-popup-options=\"{}\">{}</button>",
        escape(&t(list, "composer.options")),
        escape(&data.to_string()),
        icon("circle-plus", None)
    )
}

pub(crate) fn toolbar(
    list: &ListContext,
    upload: Option<&UploadUi>,
    options: Option<(&[PopupOption], &[&str])>,
) -> String {
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
    // The extras group: the options menu (emoji is not ported).
    if let Some((options, unported)) = options {
        out.push_str(&options_trigger(list, options, unported));
    }
    out
}

/// What the tag chooser (MiniTagChooser) needs of the site and the member,
/// for a member who may tag topics (`composer.canEditTags`).
pub struct TagChooser {
    /// `site.can_create_tag`: the chooser offers to create a tag.
    pub can_create: bool,
    /// Staff skip the composer's required tags check.
    pub staff: bool,
    pub max_tags_per_topic: i64,
    pub max_tag_search_results: i64,
    pub max_tag_length: i64,
    pub force_lowercase: bool,
    pub sort_alphabetically: bool,
    pub prioritize_recent: bool,
    /// The tag_style setting (`simple`, `box`, `bullet`).
    pub style: String,
}

/// `site.tags_filter_regexp`: DiscourseTagging::TAGS_FILTER_REGEXP's source.
const TAGS_FILTER_REGEXP: &str = r#"[\/\?#\[\]@!\$&'\(\)\*\+,;=%\\`^\s|\{\}"<>]+"#;

/// The `.tags-input`: MiniTagChooser closed (its body is drawn when it
/// opens, by composer.js) and its validation's tip.
fn tag_chooser(list: &ListContext, tags: &TagChooser, category: Option<i64>) -> String {
    let labels = serde_json::json!({
        "optional": t(list, "tagging.choose_for_topic"),
        "required_one": t(list, "tagging.choose_for_topic_required.one"),
        "required_other": t(list, "tagging.choose_for_topic_required.other"),
        "required_group_one": t(list, "tagging.choose_for_topic_required_group.one"),
        "required_group_other": t(list, "tagging.choose_for_topic_required_group.other"),
        "filter_placeholder": t(list, "select_kit.filter_placeholder"),
        "filter_placeholder_with_any": t(list, "select_kit.filter_placeholder_with_any"),
        "delete_item": t(list, "select_kit.delete_item"),
        "create": t(list, "select_kit.create"),
        "no_content": t(list, "select_kit.no_content"),
        "results_one": t(list, "select_kit.results_count.one"),
        "results_other": t(list, "select_kit.results_count.other"),
        "max_one": t(list, "select_kit.max_content_reached.one"),
        "max_other": t(list, "select_kit.max_content_reached.other"),
        "synonym_hint": t(list, "tagging.synonym_hint"),
        "tags_missing_one": t(list, "composer.error.tags_missing.one"),
        "tags_missing_other": t(list, "composer.error.tags_missing.other"),
    });
    let options = serde_json::json!({
        "allowAny": tags.can_create,
        "staff": tags.staff,
        "maximum": tags.max_tags_per_topic,
        "limit": tags.max_tag_search_results,
        "maxLength": tags.max_tag_length,
        "forceLowercase": tags.force_lowercase,
        "sortAlphabetically": tags.sort_alphabetically,
        "prioritizeRecent": tags.prioritize_recent,
        "filterRegexp": TAGS_FILTER_REGEXP,
        "style": tags.style,
        "labels": labels,
    });
    format!(
        "<div class=\"tags-input\"><details{} class=\"select-kit multi-select mini-tag-chooser no-tags\" data-options=\"{}\"><summary data-name=\"\" data-value=\"\" tabindex=\"0\" class=\"select-kit-header multi-select-header\"><div class=\"select-kit-header-wrapper\"><span class=\"formatted-selection\">{}</span>{}</div></summary><div class=\"select-kit-body\"></div></details><a tabindex=\"0\" class=\"popup-tip good hide\"> {}</a></div>",
        category
            .map(|c| format!(" category-id=\"{c}\""))
            .unwrap_or_default(),
        escape(&options.to_string()),
        escape(&t(list, "tagging.choose_for_topic")),
        icon("plus", Some("angle-icon")),
        icon("circle-xmark", None),
    )
}

/// What the new-topic and new-message routes open the composer with: the
/// title length the composer service takes a title within, and the
/// dialog (dialog.js and its #dialog-strings) their alerts load when the
/// page has none.
pub struct RouteData {
    pub max_title_length: i64,
    pub dialog_src: String,
    pub dialog_strings: String,
    pub pm: PmUi,
}

/// What the message composer (the composer's privateMessage action) needs
/// of the member: the recipients chooser's emails
/// (`can_send_private_email_messages`), the staff's warning toggle
/// (showWarning), the tags a message may take (canEditTags with
/// `site.can_tag_pms`), and who they are (focusTarget).
pub struct PmUi {
    pub username: String,
    pub staff: bool,
    pub allow_emails: bool,
    pub can_tag_pms: bool,
}

/// PmUserSelector: ComposerUserSelector's EmailGroupUserChooser
/// (`#private-message-users`, closed and empty; composer.js runs it with
/// email-group-user-chooser.js) and, for staff, the official warning
/// toggle. Hidden until the composer opens to create a message.
fn user_selector(list: &ListContext, pm: &PmUi) -> String {
    let options = serde_json::json!({
        "none": t(list, "composer.users_placeholder"),
        "placeholder": t(list, "select_kit.filter_placeholder"),
        "noContent": t(list, "select_kit.no_content"),
        "resultsOne": t(list, "select_kit.results_count.one"),
        "resultsOther": t(list, "select_kit.results_count.other"),
        "deleteItem": t(list, "select_kit.delete_item"),
        "allowEmails": pm.allow_emails,
    });
    let warning = if pm.staff {
        format!(
            "<label class=\"add-warning\" hidden><input class=\"ember-checkbox\" type=\"checkbox\"><span>{}</span></label>",
            escape(&t(list, "composer.add_warning"))
        )
    } else {
        String::new()
    };
    format!(
        "<div class=\"user-selector\" hidden><div class=\"users-input\"><details id=\"private-message-users\" class=\"select-kit multi-select user-chooser email-group-user-chooser\" data-options=\"{}\"><summary data-name=\"\" data-value=\"\" tabindex=\"0\" id=\"private-message-users-header\" class=\"select-kit-header multi-select-header\"><div class=\"select-kit-header-wrapper\"><span class=\"formatted-selection\">{}</span>{}</div></summary><div id=\"private-message-users-body\" class=\"select-kit-body\"></div></details></div>{warning}</div>",
        escape(&options.to_string()),
        escape(&t(list, "composer.users_placeholder")),
        icon("plus", Some("angle-icon")),
    )
}

/// The closed composer with its category chooser. `default_category` is
/// the category a new topic starts in (default_composer_category).
/// `preview` is where the preview's WebAssembly and render settings are
/// served, and the preview's onebox limit (max_oneboxes_per_post); None
/// leaves the composer without a preview.
#[allow(clippy::too_many_arguments)]
pub fn render(
    list: &ListContext,
    allowed: &[ChooserCategory],
    default_category: Option<i64>,
    preview: Option<(&str, &str, i64)>,
    allow_preview: bool,
    presence: Option<&serde_json::Value>,
    upload: Option<&UploadUi>,
    options: Option<(&[PopupOption], &[&str])>,
    tags: Option<&TagChooser>,
    route: &RouteData,
) -> String {
    let route_data = format!(
        " data-max-title-length=\"{}\" data-label-cant-send-pm=\"{}\" data-label-create-message-error=\"{}\" data-dialog-src=\"{}\" data-dialog-strings=\"{}\"",
        route.max_title_length,
        escape(&t(list, "composer.cant_send_pm")),
        escape(&t(list, "composer.create_message_error")),
        escape(&route.dialog_src),
        escape(&route.dialog_strings),
    );
    // discourse-presence's before-composer-controls connector, and the
    // strings and settings static/js/topic-presence.js draws it with.
    let (presence_data, presence_outlet) = match presence {
        Some(p) => (
            format!(" data-presence=\"{}\"", escape(&p.to_string())),
            "<div class=\"before-composer-controls-outlet presence\"></div>",
        ),
        None => (String::new(), ""),
    };
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
    // The message composer: its action title and save button (envelope,
    // as a reply in a message saves too), its title placeholder (no
    // featured link for a message), composePrivateMessage's reference
    // title, and who the member is.
    let pm_data = format!(
        " data-action-private-message=\"{}\" data-submit-private-message=\"{}\" data-action-reply-message=\"{}\" data-label-title-message=\"{}\" data-label-reference-title=\"{}\" data-username=\"{}\"{}",
        escape(&action(
            "envelope",
            &t(
                list,
                "composer.composer_actions.create_personal_message.label"
            )
        )),
        escape(&action("envelope", &t(list, "composer.create_pm"))),
        escape(&action(
            "share",
            &t(list, "composer.composer_actions.reply_to_message.trigger")
        )),
        escape(&t(list, "composer.title_placeholder")),
        escape(&t(list, "composer.reference_topic_title")),
        escape(&route.pm.username),
        if route.pm.can_tag_pms && tags.is_some() {
            " data-can-tag-pms=\"true\""
        } else {
            ""
        },
    );
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
    ) + &format!(
        " data-label-hide-toolbar=\"{}\" data-label-show-toolbar=\"{}\"",
        escape(&t(list, "composer.hide_toolbar")),
        escape(&t(list, "composer.show_toolbar")),
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
    // The save and discard buttons: the mobile view's discard is an icon
    // (cancelIcon: the trash can, composer.js swaps in an xmark for an
    // edit), with its plugin outlets' spans and the upload button.
    let mobile = crate::mobile::mobile_view();
    let discard = escape(&t(list, "composer.discard"));
    let save_or_cancel = if mobile {
        format!(
            "<span></span><div class=\"save-or-cancel\"><button class=\"btn btn-icon-text btn-primary create\" type=\"button\"></button><button class=\"btn no-text btn-icon discard-button btn-transparent\" title=\"{discard}\" type=\"button\">{}<span aria-hidden=\"true\">&#8203;</span></button><span></span></div><span></span>{}",
            icon("trash-can", None),
            match upload {
                Some(upload) => format!(
                    "<a aria-label=\"{}\" class=\"btn btn-default no-text mobile-file-upload\" id=\"mobile-file-upload\">{}</a>",
                    escape(&t(list, "composer.upload_title")),
                    icon(upload.icon, None)
                ),
                None => String::new(),
            }
        )
    } else {
        format!(
            "<div class=\"save-or-cancel\"><button class=\"btn btn-icon-text btn-primary create\" type=\"button\"></button><button class=\"btn discard-button btn-transparent\" title=\"{discard}\" type=\"button\"><span class=\"d-button-label\">{discard}</span></button></div>"
        )
    };
    // The options menu's example texts (applySurround's exampleKey,
    // footnote's title) and what an unported item says.
    let popup_data = if options.is_some() {
        format!(
            " data-text-details-title=\"{}\" data-text-details=\"{}\" data-text-footnote=\"{}\" data-text-spoiler=\"{}\" data-label-unported=\"Not supported yet:\"",
            escape(&t(list, "composer.details_title")),
            escape(&t(list, "composer.details_text")),
            escape(&t(list, "footnote.title")),
            escape(&t(list, "composer.spoiler_text")),
        )
    } else {
        String::new()
    };
    let preview_data = match preview {
        Some((wasm, settings, max_oneboxes)) => format!(
            " data-preview-wasm=\"{}\" data-preview-settings=\"{}\" data-max-oneboxes=\"{max_oneboxes}\" data-label-show-preview=\"{}\" data-label-hide-preview=\"{}\"",
            escape(wasm),
            escape(settings),
            escape(&t(list, "composer.show_preview")),
            escape(&t(list, "composer.hide_preview")),
        ),
        None => String::new(),
    };
    // What a save may show instead of the post: the reply-where dialog's
    // title and cancel button, and PostEnqueued's modal for a post sent
    // for review.
    let save_data = format!(
        " data-label-posting-not-on-topic=\"{}\" data-label-cancel=\"{}\" data-approval=\"{}\"",
        escape(&t(list, "composer.posting_not_on_topic")),
        escape(&t(list, "composer.cancel")),
        escape(
            &serde_json::json!({
                "title": t(list, "review.approval.title"),
                "description": t(list, "review.approval.description"),
                "pending_one": t(list, "review.approval.pending_posts.one"),
                "pending_other": t(list, "review.approval.pending_posts.other"),
                "ok": t(list, "review.approval.ok"),
            })
            .to_string()
        ),
    );
    format!(
        "<div id=\"reply-control\" class=\"closed hide-preview\" hx-preserve=\"true\" data-page-attrs=\"data-default-category\" data-allow-preview=\"{allow_preview}\"{preview_data}{draft_data}{upload_data}{presence_data}{popup_data}{route_data}{pm_data}{save_data} data-base-path=\"{}\" data-default-category=\"{}\" data-action-reply=\"{}\" data-action-create-topic=\"{}\" data-action-edit=\"{}\" data-submit-reply=\"{}\" data-submit-create-topic=\"{}\" data-submit-edit=\"{}\" data-label-reply-to-topic=\"{}\" data-label-fullscreen=\"{}\" data-label-exit-fullscreen=\"{}\" data-text-bold=\"{}\" data-text-italic=\"{}\" data-text-link=\"{}\" data-text-blockquote=\"{}\" data-text-code=\"{}\" data-text-list=\"{}\"><div class=\"d-resize-separator grippie\" aria-label=\"{}\" aria-orientation=\"horizontal\" data-pointer-drag=\"none\" data-resize-axis=\"vertical\" role=\"separator\" tabindex=\"0\"></div><div class=\"reply-area{reply_area_tags} with-category\" role=\"dialog\" hidden><div class=\"reply-to\"><div class=\"composer-action-title\"><span aria-level=\"1\" class=\"action-title\" role=\"heading\"><button class=\"btn btn-icon-text composer-actions-trigger btn-flat btn-icon-text composer-actions\" tabindex=\"-1\" type=\"button\"></button></span></div>{presence_outlet}<div class=\"composer-controls\">{}{}{}</div></div><div class=\"toolbar-visible wmd-controls\"><div class=\"d-editor\"><div class=\"d-editor-container --markdown-editor-enabled\"><div class=\"d-editor-textarea-column\"><div class=\"composer-fields\">{user_selector}<div class=\"title-and-category\" hidden><div class=\"title-input\"><input aria-label=\"{title}\" autocomplete=\"off\" id=\"reply-title\" placeholder=\"{title}\" type=\"text\"></div><div class=\"category-input\">{chooser}</div>{tags_input}</div></div><div class=\"d-editor-textarea-wrapper\"><div class=\"d-overflow-controls d-editor-button-bar__wrap\"><div class=\"d-overflow-controls__content d-editor-button-bar\" role=\"toolbar\">{}</div></div><textarea aria-label=\"{body}\" autocomplete=\"off\" class=\"d-editor-input --markdown-monospace\" placeholder=\"{body}\"></textarea></div></div><div class=\"d-editor-preview-wrapper\"><div class=\"d-editor-preview\"></div></div></div></div>{pick_files}</div><div class=\"submit-panel\">{save_or_cancel}{file_uploading}{draft_status}<p class=\"composer-error\" role=\"alert\" hidden></p></div></div></div>",
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
        // ComposerToggles: the mobile view hides and shows the toolbar
        // instead of previewing or going fullscreen.
        if mobile {
            control("toggle-toolbar", "bars", "composer.hide_toolbar")
        } else {
            preview_toggle.clone()
                + &control(
                    "toggle-fullscreen",
                    "discourse-expand",
                    "composer.enter_fullscreen",
                )
        },
        control("toggler toggle-minimize", "minus", "composer.collapse"),
        control(
            "toggler toggle-save-and-close",
            "xmark",
            "composer.save_and_close"
        ),
        toolbar(list, upload, options),
        title = escape(&t(list, "composer.title_or_link_placeholder")),
        body = escape(&t(list, "composer.reply_placeholder")),
        save_or_cancel = save_or_cancel,
        reply_area_tags = if tags.is_some() { " with-tags" } else { "" },
        tags_input = tags
            .map(|t| tag_chooser(list, t, selected.map(|c| c.id)))
            .unwrap_or_default(),
        user_selector = user_selector(list, &route.pm),
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
                minimum_required_tags: 0,
            },
            ChooserCategory {
                id: 3,
                topic_count: 9,
                description_text: None,
                minimum_required_tags: 0,
            },
            ChooserCategory {
                id: 1,
                topic_count: 3,
                description_text: None,
                minimum_required_tags: 0,
            },
        ];
        assert_eq!(chooser_order(&cats, &allowed), [1, 3, 2]);
    }
}
