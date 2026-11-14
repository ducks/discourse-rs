//! The composer (Ember's #reply-control) in its markdown mode, rendered
//! once per page for a member and opened by static/js/composer.js to reply,
//! create a topic or edit a post. The markup follows composer-container,
//! composer-editor, d-editor and the category chooser's select-kit; the
//! script sets the mode, its title and submit label (rendered here into
//! data attributes) and posts through /posts, with the tag chooser
//! (MiniTagChooser) for a member who may tag topics.
//!
//! The toggle switches to the rich editor (static/js/rich-editor.js,
//! docs/rich-editor.md) in the member's composition mode.
//!
//! Not ported yet: the heading and emoji menus.

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
    /// `form_template_ids`, for ComposerMessages' similar topics.
    pub form_template_ids: Vec<i64>,
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
    row_with(list, c, category, false)
}

/// The row of the chooser's readOnlyCategoryId: a category the member
/// can't create topics in, listed but not to be chosen (category-row's
/// isReadOnly: the read_only_description as its title, and
/// categoryBadgeHTML's readOnly after the topic count).
pub(crate) fn read_only_row(
    list: &ListContext,
    c: &ChooserCategory,
    category: &ListCategory,
) -> String {
    row_with(list, c, category, true)
}

fn row_with(
    list: &ListContext,
    c: &ChooserCategory,
    category: &ListCategory,
    read_only: bool,
) -> String {
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
    let (title, inside, read_only_data) = if read_only {
        let label = escape(&t(list, "category_row.read_only"));
        (
            escape(&t(list, "category_row.read_only_description")),
            format!("{count}<span class=\"read-only\" aria-label=\"{label}\">{label}</span>"),
            " data-read-only=\"true\"",
        )
    } else {
        (escape(&category.name), count, "")
    };
    format!(
        "<div class=\"category-row select-kit-row\" data-name=\"{n}\" data-title=\"{title}\" data-value=\"{}\" data-url=\"{}\"{}{}{read_only_data} role=\"menuitemradio\" tabindex=\"-1\" title=\"{title}\"><div class=\"category-status\">{parent}{}</div>{desc}</div>",
        c.id,
        // Category#url, for EditTopicTimer's notice (topic-admin-modals.js).
        escape(&crate::topic_list_view::category_url(list, category)),
        if c.minimum_required_tags > 0 {
            format!(
                " data-minimum-required-tags=\"{}\"",
                c.minimum_required_tags
            )
        } else {
            String::new()
        },
        if c.form_template_ids.is_empty() {
            String::new()
        } else {
            format!(
                " data-form-template-ids=\"{}\"",
                c.form_template_ids
                    .iter()
                    .map(|id| id.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            )
        },
        badge_span(list, category, &inside),
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
    /// When the item shows: `touch` (a touch device), `edit-reply` (an
    /// edit of a reply) or `pm-with-non-human-user` (the composer's topic
    /// is `pm_with_non_human_user`); always without.
    pub when: Option<&'static str>,
    /// Whether composer.js carries out its action; the others refuse.
    pub ported: bool,
    /// Whether Ember gives the option a `name` (its button's data-name);
    /// footnote's, spoiler's and poll's have none.
    pub named: bool,
}

/// The options menu's items for the site's settings, in Ember's order:
/// core's quote, format code and insert table, change reply-to, the
/// plugins' (details, footnote, local dates, spoiler, poll, policy), then
/// wrap. `can_create_poll` is the viewer's (staff or
/// poll_create_allowed_groups), `can_create_policy` their
/// `can_create_policy`.
pub fn popup_options(
    settings: &crate::site_settings::SiteSettings,
    can_create_poll: bool,
    can_create_policy: bool,
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
            ported: true,
            ..option("format-code", "code", "composer.code_title")
        },
        PopupOption {
            ported: true,
            ..option("toggle-spreadsheet", "table", "composer.insert_table")
        },
        PopupOption {
            when: Some("edit-reply"),
            ported: true,
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
    // discourse-graphviz's Insert Graphviz graph, which has no name; its
    // place among the plugins' items is not known.
    if crate::plugins::graphviz::enabled(settings)? {
        out.push(PopupOption {
            ported: true,
            named: false,
            ..option("graphviz", "diagram-project", "graphviz.composer_title")
        });
    }
    // discourse-math's Insert Math; its place among the plugins' items is
    // not known.
    if crate::plugins::math::enabled(settings)? {
        out.push(PopupOption {
            shortcut: Some("Shift+M"),
            ported: true,
            ..option(
                "insert-math",
                "square-root-variable",
                "discourse_math.composer.insert_math",
            )
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
    // add-poll-ui-builder's condition: poll_enabled, and the composer's
    // topic a PM with a non-human user or a viewer who may create polls.
    if on("poll_enabled")? {
        out.push(PopupOption {
            when: (!can_create_poll).then_some("pm-with-non-human-user"),
            ported: true,
            named: false,
            ..option("poll", "chart-bar", "poll.ui_builder.title")
        });
    }
    // discourse-policy's builder; its place among the plugins' items is
    // not known.
    if crate::plugins::policy::enabled(settings)? && can_create_policy {
        out.push(PopupOption {
            ported: true,
            named: false,
            ..option(
                "policy",
                "file-signature",
                "discourse_policy.builder.attach",
            )
        });
    }
    out.push(PopupOption {
        show_active_icon: true,
        ported: true,
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
    if settings.get("discourse_post_event_enabled")?.truthy() {
        out.push("discourse-events");
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

/// A popup menu's trigger (ToolbarPopupMenuOptions' DMenu): its options
/// are drawn by static/js/editor-toolbar.js when it opens.
fn menu_trigger(list: &ListContext, id: &str, icon_name: &str, title: &str) -> String {
    format!(
        "<button aria-expanded=\"false\" class=\"btn no-text fk-d-menu__trigger toolbar-menu__{id}-trigger toolbar__button {id} toolbar-popup-menu-options\" data-action=\"{id}\" tabindex=\"-1\" title=\"{}\" data-identifier=\"toolbar-menu__{id}\" data-trigger=\"\" type=\"button\">{}</button>",
        escape(&t(list, title)),
        icon(icon_name, None)
    )
}

fn toolbar_button(list: &ListContext, class: &str, icon_name: &str, title: &str) -> String {
    format!(
        "<button class=\"btn no-text btn-icon toolbar__button {class}\" data-action=\"{class}\" tabindex=\"-1\" title=\"{}\" type=\"button\">{}<span aria-hidden=\"true\">&#8203;</span></button>",
        escape(&t(list, title)),
        icon(icon_name, None)
    )
}

/// The emoji picker the toolbar's emoji button opens (enable-emoji.js):
/// static/js/emoji-picker.js, loaded the first time, and its template.
pub struct EmojiPickerUi {
    pub src: String,
    pub template: String,
}

/// The composer's and the About me's bar (lib/composer/toolbar.ts, the
/// composer's extraButtons and enable-emoji.js' button): fontStyles (bold,
/// italic, the heading menu), insertions (link, blockquote, code, upload)
/// and extras (the list menu, emoji, the options menu), run by
/// static/js/editor-toolbar.js.
pub(crate) fn editor_toolbar(
    list: &ListContext,
    upload: Option<&UploadUi>,
    options: Option<(&[PopupOption], &[&str])>,
    emoji: Option<&EmojiPickerUi>,
) -> String {
    let mut out = String::new();
    out.push_str(&toolbar_button(list, "bold", "bold", "composer.bold_title"));
    out.push_str(&toolbar_button(
        list,
        "italic",
        "italic",
        "composer.italic_title",
    ));
    out.push_str(&menu_trigger(
        list,
        "heading",
        "discourse-text",
        "composer.text_size_title",
    ));
    out.push_str(&toolbar_button(list, "link", "link", "composer.link_title"));
    out.push_str(&toolbar_button(
        list,
        "blockquote",
        "quote-right",
        "composer.blockquote_title",
    ));
    out.push_str(&toolbar_button(list, "code", "code", "composer.code_title"));
    if let Some(upload) = upload {
        out.push_str(&toolbar_button(list, "upload", upload.icon, "upload"));
    }
    out.push_str(&menu_trigger(
        list,
        "list",
        "list-ul",
        "composer.list_title",
    ));
    if let Some(emoji) = emoji {
        out.push_str(&format!(
            "<button class=\"btn no-text btn-icon toolbar__button emoji insert-composer-emoji\" data-action=\"emoji\" data-src=\"{}\" tabindex=\"-1\" title=\"{}\" type=\"button\">{}<span aria-hidden=\"true\">&#8203;</span></button>{}",
            escape(&emoji.src),
            escape(&t(list, "composer.emoji")),
            icon("far-face-smile", None),
            emoji.template
        ));
    }
    if let Some((options, unported)) = options {
        out.push_str(&options_trigger(list, options, unported));
    }
    out
}

/// The markdown bar of the d-editors whose scripts run it themselves
/// (review's edit, the admin's About page).
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

impl TagChooser {
    pub fn load(
        settings: &crate::site_settings::SiteSettings,
        guardian: &crate::guardian::Guardian,
    ) -> Result<Self, crate::site_settings::SettingError> {
        Ok(TagChooser {
            can_create: guardian.can_create_tag(settings)?,
            staff: guardian.is_staff(),
            max_tags_per_topic: settings.get("max_tags_per_topic")?.to_i(),
            max_tag_search_results: settings.get("max_tag_search_results")?.to_i(),
            max_tag_length: settings.get("max_tag_length")?.to_i(),
            force_lowercase: settings.get("force_lowercase_tags")?.truthy(),
            sort_alphabetically: settings.get("tags_sort_alphabetically")?.truthy(),
            prioritize_recent: settings.get("prioritize_recently_used_tags")?.truthy(),
            style: settings.get("tag_style")?.to_s(),
        })
    }

    /// The chooser's `data-options` (mini-tag-chooser.js): select-kit's
    /// options and the strings it draws.
    pub fn options(&self, i18n: &crate::i18n::I18n) -> serde_json::Value {
        let t = |key: &str| i18n.t(&format!("js.{key}")).unwrap_or(key).to_string();
        let labels = serde_json::json!({
            "optional": t("tagging.choose_for_topic"),
            "required_one": t("tagging.choose_for_topic_required.one"),
            "required_other": t("tagging.choose_for_topic_required.other"),
            "required_group_one": t("tagging.choose_for_topic_required_group.one"),
            "required_group_other": t("tagging.choose_for_topic_required_group.other"),
            "filter_placeholder": t("select_kit.filter_placeholder"),
            "filter_placeholder_with_any": t("select_kit.filter_placeholder_with_any"),
            "delete_item": t("select_kit.delete_item"),
            "create": t("select_kit.create"),
            "no_content": t("select_kit.no_content"),
            "results_one": t("select_kit.results_count.one"),
            "results_other": t("select_kit.results_count.other"),
            "max_one": t("select_kit.max_content_reached.one"),
            "max_other": t("select_kit.max_content_reached.other"),
            "synonym_hint": t("tagging.synonym_hint"),
            "tags_missing_one": t("composer.error.tags_missing.one"),
            "tags_missing_other": t("composer.error.tags_missing.other"),
        });
        serde_json::json!({
            "allowAny": self.can_create,
            "staff": self.staff,
            "maximum": self.max_tags_per_topic,
            "limit": self.max_tag_search_results,
            "maxLength": self.max_tag_length,
            "forceLowercase": self.force_lowercase,
            "sortAlphabetically": self.sort_alphabetically,
            "prioritizeRecent": self.prioritize_recent,
            "filterRegexp": TAGS_FILTER_REGEXP,
            "style": self.style,
            "labels": labels,
        })
    }
}

/// The `.tags-input`: MiniTagChooser closed (its body is drawn when it
/// opens, by mini-tag-chooser.js) and its validation's tip.
fn tag_chooser(list: &ListContext, tags: &TagChooser, category: Option<i64>) -> String {
    let options = tags.options(list.i18n);
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
    pub actions: ActionsUi,
    /// discourse-poll's builder, when the options menu offers it.
    pub poll_builder: Option<PollBuilderUi>,
    /// The rich editor's getContext data (rich_editor::context), as JSON.
    pub rich_editor_context: String,
    /// The emoji button's picker, where enable_emoji is on.
    pub emoji_picker: Option<EmojiPickerUi>,
}

/// What static/js/poll-builder.js (PollUiBuilderModal) is drawn with.
pub struct PollBuilderUi {
    pub src: String,
    pub max_options: i64,
    pub default_public: bool,
    pub staff: bool,
    /// `site.groups` but everyone, by name.
    pub groups: Vec<String>,
    /// discourse_local_dates_enabled: mod+Shift+. in an option.
    pub local_dates: bool,
    /// DDateInput's Pikaday, for a browser without a native date input.
    pub moment_src: String,
    pub pikaday_src: String,
}

const POLL_BUILDER_STRINGS: &[&str] = &[
    "poll.ui_builder.title",
    "poll.ui_builder.insert",
    "poll.ui_builder.edit",
    "save",
    "poll.ui_builder.help.options_min_count",
    "poll.ui_builder.help.options_max_count",
    "poll.ui_builder.help.invalid_min_value",
    "poll.ui_builder.help.invalid_max_value",
    "poll.ui_builder.help.invalid_values",
    "poll.ui_builder.help.min_step_value",
    "poll.ui_builder.poll_type.regular",
    "poll.ui_builder.poll_type.multiple",
    "poll.ui_builder.poll_type.ranked_choice",
    "poll.ui_builder.poll_type.number",
    "poll.ui_builder.poll_result.label",
    "poll.ui_builder.poll_result.always",
    "poll.ui_builder.poll_result.vote",
    "poll.ui_builder.poll_result.closed",
    "poll.ui_builder.poll_result.staff",
    "poll.ui_builder.poll_groups.label",
    "poll.ui_builder.poll_chart_type.label",
    "poll.ui_builder.poll_chart_type.bar",
    "poll.ui_builder.poll_chart_type.pie",
    "poll.ui_builder.poll_config.max",
    "poll.ui_builder.poll_config.min",
    "poll.ui_builder.poll_config.step",
    "poll.ui_builder.poll_public.label",
    "poll.ui_builder.poll_dynamic.label",
    "poll.ui_builder.poll_title.label",
    "poll.ui_builder.poll_options.label",
    "poll.ui_builder.poll_options.add",
    "poll.ui_builder.automatic_close.label",
    "advanced_mode_toggle.simple_mode",
    "advanced_mode_toggle.advanced_mode",
    "select_kit.default_header_text",
    "select_kit.filter_by",
    "select_kit.select_to_filter",
    "select_kit.filter_placeholder",
    "select_kit.filter_placeholder_with_any",
    "select_kit.no_content",
    "select_kit.results_count.one",
    "select_kit.results_count.other",
    "select_kit.create",
    "select_kit.delete_item",
    "select_kit.components.filter_for_more",
    "dates.placeholder",
    "dates.time",
    "dates.previous_month",
    "dates.next_month",
    "cancel",
];

/// `data-poll-builder` for composer.js.
fn poll_builder_data(list: &ListContext, builder: &PollBuilderUi) -> String {
    let strings: serde_json::Map<String, serde_json::Value> = POLL_BUILDER_STRINGS
        .iter()
        .map(|key| ((*key).to_string(), t(list, key).into()))
        .collect();
    let data = serde_json::json!({
        "src": builder.src,
        "maxOptions": builder.max_options,
        "defaultPublic": builder.default_public,
        "staff": builder.staff,
        "groups": builder.groups,
        "localDates": builder.local_dates,
        "assets": { "moment": builder.moment_src, "pikaday": builder.pikaday_src },
        "strings": strings,
    });
    format!(" data-poll-builder=\"{}\"", escape(&data.to_string()))
}

/// What the composer actions menu (ComposerActions, a DMenu on the
/// composer's title, with ComposerActionItemBuilder's items) needs of the
/// member and the site, and of discourse-post-voting's composer actions.
pub struct ActionsUi {
    pub can_create_topic: bool,
    pub can_send_private_messages: bool,
    pub staff: bool,
    /// `canToggleNoBump`: staff or trust level 4.
    pub can_skip_bump: bool,
    /// `currentUser.whisperer`
    pub whisperer: bool,
    /// `site.shared_drafts_category_id` is sent to this member.
    pub shared_drafts: bool,
    /// prioritize_username_in_ux, for a post's display name.
    pub prioritize_username: bool,
    /// discourse-post-voting, when enabled: the categories whose new topics
    /// default to post voting, and those that only allow it.
    pub post_voting: Option<(Vec<i64>, Vec<i64>)>,
}

/// The strings composer.js draws the menu, its trigger and the whisper
/// indicator with (js.* keys).
const ACTIONS_STRINGS: &[&str] = &[
    "composer.composer_actions.reply_to_post.label",
    "composer.composer_actions.reply_to_post.desc",
    "composer.composer_actions.reply_as_new_topic.label",
    "composer.composer_actions.reply_as_new_topic.desc",
    "composer.composer_actions.reply_as_new_topic.confirm",
    "composer.composer_actions.reply_as_new_group_message.label",
    "composer.composer_actions.reply_as_new_group_message.desc",
    "composer.composer_actions.reply_to_topic.label",
    "composer.composer_actions.reply_to_topic.desc",
    "composer.composer_actions.reply_to_topic.trigger",
    "composer.composer_actions.reply_to_message.label",
    "composer.composer_actions.reply_to_message.desc",
    "composer.composer_actions.reply_to_message.trigger",
    "composer.composer_actions.toggle_whisper.label",
    "composer.composer_actions.toggle_whisper.desc",
    "composer.composer_actions.create_topic.label",
    "composer.composer_actions.create_topic.desc",
    "composer.composer_actions.create_personal_message.label",
    "composer.composer_actions.create_personal_message.desc",
    "composer.composer_actions.shared_draft.label",
    "composer.composer_actions.shared_draft.desc",
    "composer.composer_actions.toggle_topic_bump.label",
    "composer.composer_actions.toggle_topic_bump.desc",
    "composer.composer_actions.toggle_unlisted.label",
    "composer.composer_actions.toggle_unlisted.desc",
    "composer.composer_actions.no_actions_available",
    "composer.composer_actions.unknown_user",
    "composer.whisper_indicator.public",
    "composer.whisper_indicator.whispering",
    "composer.create_whisper",
    "composer.ok_proceed",
    "composer.unlist",
    "post.continue_discussion",
    "post.post_number",
    // ChangeReplyTo
    "composer.change_reply_to.title",
    "composer.change_reply_to.hint",
    "composer.change_reply_to.current_selection",
    "composer.change_reply_to.invalid_target",
    "composer.change_reply_to.submit",
    "composer.change_reply_to.open",
    "composer.change_reply_to.remove",
    "cancel",
];

/// discourse-post-voting's composer strings.
const POST_VOTING_STRINGS: &[&str] = &[
    "composer.create_post_voting.label",
    "composer.composer_actions.create_as_post_voting.label",
    "composer.composer_actions.create_as_post_voting.desc",
    "composer.composer_actions.remove_as_post_voting.label",
    "composer.composer_actions.remove_as_post_voting.desc",
    "post_voting.topic.answer.label",
];

/// The menu's data for composer.js (`data-composer-actions`).
fn actions_data(list: &ListContext, actions: &ActionsUi) -> String {
    let mut strings = serde_json::Map::new();
    let keys = ACTIONS_STRINGS.iter().chain(
        actions
            .post_voting
            .as_ref()
            .map_or(&[][..], |_| POST_VOTING_STRINGS)
            .iter(),
    );
    for key in keys {
        strings.insert((*key).to_string(), t(list, key).into());
    }
    let data = serde_json::json!({
        "canCreateTopic": actions.can_create_topic,
        "canSendPrivateMessages": actions.can_send_private_messages,
        "staff": actions.staff,
        "canSkipBump": actions.can_skip_bump,
        "whisperer": actions.whisperer,
        "sharedDrafts": actions.shared_drafts,
        "prioritizeUsername": actions.prioritize_username,
        "postVoting": actions.post_voting.as_ref().map(|(default, only)| {
            serde_json::json!({ "defaultCategories": default, "onlyCategories": only })
        }),
        "strings": strings,
    });
    escape(&data.to_string())
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

/// ComposerToggleSwitch, first in d-editor's button bar while no editor
/// mode is forced (showEditorModeToggle): the switch between the markdown
/// and the rich editor, `rich` in composition mode 1. Labelled for a
/// keyboard outside Apple's platforms (formatShortcut("ctrl+m")): composer.js
/// relabels it from #reply-control's data attributes for the platform, the
/// pointer (the `_no_shortcut` forms without a keyboard) and each switch.
pub(crate) fn toggle_switch(list: &ListContext, rich: bool) -> String {
    let ctrl = t(list, "shortcut_modifier_key.ctrl");
    let key = if rich {
        "composer.switch_to_markdown"
    } else {
        "composer.switch_to_rich_text"
    };
    let label = t(list, key).replace("%{keyboardShortcut}", &format!("{ctrl} M"));
    let active = |on: bool| if on { " --active" } else { "" };
    format!(
        "<button aria-checked=\"{rich}\" aria-keyshortcuts=\"{}\" aria-label=\"{label}\" class=\"composer-toggle-switch {}\" data-rich-editor=\"{rich}\" role=\"switch\" title=\"{label}\" type=\"button\"><span class=\"composer-toggle-switch__slider\"><span aria-hidden=\"true\" class=\"composer-toggle-switch__left-icon{}\">{}</span><span aria-hidden=\"true\" class=\"composer-toggle-switch__right-icon{}\">{}</span></span></button>",
        escape(&format!("{ctrl}+M")),
        if rich { "--rte" } else { "--markdown" },
        active(!rich),
        icon("fab-markdown", None),
        active(rich),
        icon("a", None),
        label = escape(&label),
    )
}

/// The rich editor's data on #reply-control for composer.js: its
/// extensions' context (rich_editor::context), the member's composition
/// mode, the toggle's labels (with `%{keyboardShortcut}` left in), the key
/// names it formats the shortcut with, and the alert for a post the rich
/// editor cannot hold.
fn rich_editor_data(list: &ListContext, rich: bool, context: &str) -> String {
    format!(
        " data-composition-mode=\"{}\" data-label-switch-to-markdown=\"{}\" data-label-switch-to-rich-text=\"{}\" data-label-switch-to-markdown-no-shortcut=\"{}\" data-label-switch-to-rich-text-no-shortcut=\"{}\" data-label-key-ctrl=\"{}\" data-label-key-control=\"{}\" data-label-unsupported-token=\"{}\" data-rich-editor-context=\"{}\"",
        if rich { 1 } else { 0 },
        escape(&t(list, "composer.switch_to_markdown")),
        escape(&t(list, "composer.switch_to_rich_text")),
        escape(&t(list, "composer.switch_to_markdown_no_shortcut")),
        escape(&t(list, "composer.switch_to_rich_text_no_shortcut")),
        escape(&t(list, "shortcut_modifier_key.ctrl")),
        escape(&t(list, "shortcut_modifier_key.control")),
        escape(&t(list, "composer.unsupported_token")),
        escape(context),
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
    rich: bool,
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
    // ComposerMessages' popups (static/js/composer-messages.js). Ember has
    // none on mobile; the composer is kept through a redraw in the other
    // layout (page.js), so it is drawn in both and stays empty in the
    // mobile view.
    let popup_container = "<div class=\"composer-popup-container\"></div>";
    let mobile = crate::mobile::mobile_view();
    // The save and discard buttons: the mobile view's discard is an icon
    // (cancelIcon: the trash can, composer.js swaps in an xmark for an
    // edit), with its plugin outlets' spans and the upload button.
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
    // What an unported options menu item says (the items' example texts
    // are the rich editor context's strings), and topic.js for
    // ChangeReplyTo's timeline off a topic page.
    let popup_data = if options.is_some() {
        format!(
            " data-label-unported=\"Not supported yet:\" data-timeline-script=\"{}\"",
            escape(&crate::assets::url(list.base_path, "topic.js"))
        ) + &route
            .poll_builder
            .as_ref()
            .map(|b| poll_builder_data(list, b))
            .unwrap_or_default()
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
        "<div id=\"reply-control\" class=\"closed hide-preview\" hx-preserve=\"true\" data-page-attrs=\"data-default-category\" data-allow-preview=\"{allow_preview}\"{rich_data}{preview_data}{draft_data}{upload_data}{presence_data}{popup_data}{route_data}{pm_data}{save_data} data-base-path=\"{}\" data-default-category=\"{}\" data-action-reply=\"{}\" data-action-create-topic=\"{}\" data-action-edit=\"{}\" data-submit-reply=\"{}\" data-submit-create-topic=\"{}\" data-submit-edit=\"{}\" data-label-reply-to-topic=\"{}\" data-label-fullscreen=\"{}\" data-label-exit-fullscreen=\"{}\" data-text-bold=\"{}\" data-text-italic=\"{}\" data-text-link=\"{}\" data-text-blockquote=\"{}\" data-text-code=\"{}\" data-text-list=\"{}\"><div class=\"d-resize-separator grippie\" aria-label=\"{}\" aria-orientation=\"horizontal\" data-pointer-drag=\"none\" data-resize-axis=\"vertical\" role=\"separator\" tabindex=\"0\"></div>{popup_container}<div class=\"reply-area{reply_area_tags} with-category\" role=\"dialog\" hidden><div class=\"reply-to\"><div class=\"composer-action-title\"><span aria-level=\"1\" class=\"action-title\" role=\"heading\"><button aria-expanded=\"false\" class=\"btn btn-icon-text fk-d-menu__trigger composer-actions-trigger btn-flat btn-icon-text composer-actions\" data-trigger=\"\" type=\"button\" data-composer-actions=\"{actions_data}\"></button></span></div>{presence_outlet}<div class=\"composer-controls\">{}{}{}</div></div><div class=\"toolbar-visible wmd-controls\"><div class=\"d-editor\"><div class=\"d-editor-container {container_mode}\"><div class=\"d-editor-textarea-column\"><div class=\"composer-fields\">{user_selector}<div class=\"title-and-category\" hidden><div class=\"title-input\"><input aria-label=\"{title}\" autocomplete=\"off\" id=\"reply-title\" placeholder=\"{title}\" type=\"text\"></div><div class=\"category-input\">{chooser}</div>{tags_input}</div></div><div class=\"d-editor-textarea-wrapper\"><div class=\"d-overflow-controls d-editor-button-bar__wrap\"><div class=\"d-overflow-controls__content d-editor-button-bar\" role=\"toolbar\">{toggle}{}</div></div><textarea aria-label=\"{body}\" autocomplete=\"off\" class=\"d-editor-input --markdown-monospace\" placeholder=\"{body}\"></textarea></div></div><div class=\"d-editor-preview-wrapper\"><div class=\"d-editor-preview\"></div></div></div></div>{pick_files}</div><div class=\"submit-panel\">{save_or_cancel}{file_uploading}{draft_status}<p class=\"composer-error\" role=\"alert\" hidden></p></div></div></div>",
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
        editor_toolbar(list, upload, options, route.emoji_picker.as_ref()),
        title = escape(&t(list, "composer.title_or_link_placeholder")),
        body = escape(&t(list, "composer.reply_placeholder")),
        save_or_cancel = save_or_cancel,
        reply_area_tags = if tags.is_some() { " with-tags" } else { "" },
        tags_input = tags
            .map(|t| tag_chooser(list, t, selected.map(|c| c.id)))
            .unwrap_or_default(),
        user_selector = user_selector(list, &route.pm),
        actions_data = actions_data(list, &route.actions),
        rich_data = rich_editor_data(list, rich, &route.rich_editor_context),
        toggle = toggle_switch(list, rich),
        container_mode = if rich {
            "--rich-editor-enabled"
        } else {
            "--markdown-editor-enabled"
        },
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
                form_template_ids: Vec::new(),
            },
            ChooserCategory {
                id: 3,
                topic_count: 9,
                description_text: None,
                minimum_required_tags: 0,
                form_template_ids: Vec::new(),
            },
            ChooserCategory {
                id: 1,
                topic_count: 3,
                description_text: None,
                minimum_required_tags: 0,
                form_template_ids: Vec::new(),
            },
        ];
        assert_eq!(chooser_order(&cats, &allowed), [1, 3, 2]);
    }
}
