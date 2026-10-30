//! Chat's admin plugin pages: the configuration nav (the settings and
//! Incoming webhooks), the header's Create export action
//! (ChatAdminPluginActions), the webhooks list
//! (AdminChatIncomingWebhooksList) and the new and edit forms
//! (ChatIncomingWebhookEditForm). static/js/chat-admin.js acts on them.

use serde_json::Value;

use crate::admin_ui::{Action, Button};
use crate::admin_view::{AdminCx, t};
use crate::topic_list_view::{escape, icon};

pub const PLUGIN_ID: &str = "chat";
pub const SHOW_PATH: &str = "/admin/plugins/chat";
pub const HOOKS_PATH: &str = "/admin/plugins/chat/hooks";

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}

/// addAdminPluginConfigurationNav: the settings, then the webhooks.
pub fn config_nav(cx: &AdminCx, hooks: bool) -> String {
    crate::admin_ui::tabs(
        cx,
        &[
            (
                "",
                &format!("{SHOW_PATH}/settings"),
                "admin.plugins.change_settings_short",
                !hooks,
            ),
            ("", HOOKS_PATH, "chat.incoming_webhooks.title", hooks),
        ],
    )
}

/// ChatAdminPluginActions in the plugin's page header, for admins.
pub fn header_actions(cx: &AdminCx) -> String {
    if !cx.viewer.admin {
        return String::new();
    }
    // With chat-admin.js and its strings, on every chat admin page.
    format!(
        "{}{}<script src=\"{}\"></script>",
        crate::admin_ui::actions(
            cx,
            "d-page-header",
            &[Action::Primary(Button {
                class: "admin-chat-export",
                icon: Some("right-from-bracket"),
                label: Some(t(cx.i18n, "chat.admin.export_messages.create_export")),
                title: Some(t(cx.i18n, "chat.admin.export_messages.create_export")),
                ..Default::default()
            })],
        ),
        strings(cx),
        crate::assets::url(cx.base_path, "chat-admin.js")
    )
}

/// The strings chat-admin.js shows.
pub fn strings(cx: &AdminCx) -> String {
    crate::admin_ui::strings(
        cx,
        "chat-admin-strings",
        &[
            "chat.admin.export_messages.confirm_export",
            "chat.admin.export_messages.export_has_started",
            "chat.incoming_webhooks.confirm_destroy",
            "chat.incoming_webhooks.saved",
            "chat.incoming_webhooks.created",
            "chat.incoming_webhooks.current_emoji",
            "form_kit.errors.required",
            "generic_error",
        ],
    )
}

/// The webhooks route: DPageSubheader with New incoming webhook, then the
/// list (by updated_at, newest first) or None yet.
pub fn hooks_index(cx: &AdminCx, webhooks: &[Value], channel_titles: &[String]) -> String {
    let new = crate::admin_ui::actions(
        cx,
        "d-page-subheader",
        &[Action::Primary(Button {
            class: "admin-incoming-webhooks-new",
            icon: Some("plus"),
            label: Some(t(cx.i18n, "chat.incoming_webhooks.new")),
            title: Some(t(cx.i18n, "chat.incoming_webhooks.new")),
            href: Some(&format!("{HOOKS_PATH}/new")),
            ..Default::default()
        })],
    );
    let mut out = String::from("<div class=\"discourse-chat-incoming-webhooks admin-detail\">");
    out.push_str(&crate::admin_ui::subheader(
        &t(cx.i18n, "chat.incoming_webhooks.title"),
        &new,
        Some(&t(cx.i18n, "chat.incoming_webhooks.instructions")),
    ));
    out.push_str("<div class=\"incoming-chat-webhooks\">");
    if webhooks.is_empty() {
        out.push_str(&escape(&t(cx.i18n, "chat.incoming_webhooks.none")));
    } else {
        let label = |key: &str| escape(&t(cx.i18n, &format!("chat.incoming_webhooks.{key}")));
        out.push_str(&format!(
            "<table class=\"d-table\"><thead class=\"d-table__header\"><th>{}</th><th>{}</th><th>{}</th><th>{}</th><th></th></thead><tbody>",
            label("name"),
            label("emoji"),
            label("username"),
            label("channel")
        ));
        let mut rows: Vec<(usize, &Value)> = webhooks.iter().enumerate().collect();
        rows.sort_by(|a, b| s(&b.1["updated_at"]).cmp(s(&a.1["updated_at"])));
        for (index, w) in rows {
            let emoji = crate::category_badge::codes_to_img(
                cx.settings.emojis(),
                cx.base_path,
                &escape(s(&w["emoji"])),
            );
            out.push_str(&format!(
                "<tr class=\"d-table__row incoming-chat-webhooks-row\" data-webhook-id=\"{id}\"><td class=\"d-table__cell --overview\"><div class=\"d-table__overview-name\">{}</div><div class=\"d-table__overview-about\">{}</div></td><td class=\"d-table__cell --detail\"><div class=\"d-table__mobile-label\">{}</div>{emoji}</td><td class=\"d-table__cell --detail\"><div class=\"d-table__mobile-label\">{}</div>{}</td><td class=\"d-table__cell --detail\"><div class=\"d-table__mobile-label\">{}</div>{}</td><td class=\"d-table__cell --controls incoming-chat-webhooks-row__controls\"><div class=\"d-table__cell-actions\"><a class=\"btn btn-default btn-small admin-chat-incoming-webhooks-edit\" href=\"{}{HOOKS_PATH}/{id}/edit\">{}</a>{}</div></td></tr>",
                escape(s(&w["name"])),
                escape(s(&w["description"])),
                label("emoji"),
                label("username"),
                escape(s(&w["username"])),
                label("channel"),
                channel_titles.get(index).cloned().unwrap_or_default(),
                cx.base_path,
                label("edit"),
                Button {
                    class: "btn-danger btn-small admin-chat-incoming-webhooks-delete",
                    icon: Some("trash-can"),
                    title: Some(t(cx.i18n, "chat.incoming_webhooks.delete")),
                    ..Default::default()
                }
                .render(),
                id = w["id"],
            ));
        }
        out.push_str("</tbody></table>");
    }
    out.push_str("</div></div>");
    out
}

/// The new and edit routes: BackButton and ChatIncomingWebhookEditForm.
/// `channels` the public channels (id, title) ChatChannelChooser lists.
pub fn hooks_form(cx: &AdminCx, webhook: Option<&Value>, channels: &[(i64, String)]) -> String {
    use crate::admin_form_kit::{Field, form, input, submit, textarea};
    let w = webhook.cloned().unwrap_or(Value::Null);
    let label = |key: &str| t(cx.i18n, &format!("chat.incoming_webhooks.{key}"));
    let mut fields = String::new();
    fields.push_str(
        &Field {
            name: "name",
            control: "input",
            title: Some(label("name")),
            inner: input("name", s(&w["name"]), "text", None, false),
            ..Default::default()
        }
        .render(),
    );
    fields.push_str(
        &Field {
            name: "description",
            control: "textarea",
            title: Some(label("description")),
            optional: Some(t(cx.i18n, "form_kit.optional")),
            inner: textarea("description", s(&w["description"]), false),
            ..Default::default()
        }
        .render(),
    );
    fields.push_str(
        &Field {
            name: "username",
            control: "input",
            title: Some(label("username")),
            optional: Some(t(cx.i18n, "form_kit.optional")),
            description: Some(label("username_instructions")),
            inner: input("username", s(&w["username"]), "text", None, false),
            ..Default::default()
        }
        .render(),
    );
    let selected = w["chat_channel"]["id"].as_i64();
    let mut select = format!(
        "<select class=\"d-native-select chat-channel-chooser\" name=\"chat_channel_id\"><option value=\"\">{}</option>",
        escape(&label("channel_placeholder"))
    );
    for (id, title) in channels {
        select.push_str(&format!(
            "<option value=\"{id}\"{}>{}</option>",
            if Some(*id) == selected {
                " selected=\"\""
            } else {
                ""
            },
            escape(title)
        ));
    }
    select.push_str("</select>");
    fields.push_str(
        &Field {
            name: "chat_channel_id",
            control: "custom",
            title: Some(label("post_to")),
            inner: select,
            ..Default::default()
        }
        .render(),
    );
    let emoji = s(&w["emoji"]);
    let current = if emoji.is_empty() {
        String::new()
    } else {
        format!(
            "{}<span class=\"incoming-chat-webhooks-current-emoji\">{}</span>",
            escape(&label("current_emoji")),
            crate::category_badge::codes_to_img(cx.settings.emojis(), cx.base_path, &escape(emoji))
        )
    };
    fields.push_str(
        &Field {
            name: "emoji",
            control: "custom",
            title: Some(label("emoji")),
            optional: Some(t(cx.i18n, "form_kit.optional")),
            description: Some(label("emoji_instructions")),
            inner: format!(
                "<input type=\"hidden\" name=\"emoji\" value=\"{}\">{current}<div class=\"form-kit__row\"><div class=\"form-kit__col --col-2\"><button class=\"btn no-text btn-icon btn-default emoji-picker-trigger\" type=\"button\">{}</button></div><div class=\"form-kit__col --col-6\">{}</div></div>",
                escape(emoji),
                icon("discourse-emojis", None),
                Button {
                    class: "admin-chat-webhooks-clear-emoji",
                    label: Some(label("reset_emoji")),
                    disabled: emoji.is_empty(),
                    ..Default::default()
                }
                .render()
            ),
            ..Default::default()
        }
        .render(),
    );
    if let Some(url) = w["url"].as_str() {
        fields.push_str(&format!(
            "<div class=\"form-kit__container\" data-name=\"url\"><label class=\"form-kit__container-title\"><span>{}</span></label><p class=\"form-kit__container-subtitle\">{}</p><div class=\"form-kit__container-content\"><code>{}</code></div></div>",
            escape(&label("url")),
            escape(&label("url_instructions")),
            escape(url)
        ));
    }
    fields.push_str(&submit("", &t(cx.i18n, "submit")));
    // EmojiPicker: emoji-picker.js with its template.
    let picker_t = |key: &str| t(cx.i18n, key);
    format!(
        "<div class=\"admin-detail discourse-chat-incoming-webhooks\">{}<div class=\"chat-incoming-webhook-form\" data-webhook-id=\"{}\">{}</div></div>{}<script src=\"{}\"></script>",
        crate::admin_ui::back_button(cx, HOOKS_PATH, "chat.incoming_webhooks.back"),
        w["id"]
            .as_i64()
            .map(|id| id.to_string())
            .unwrap_or_default(),
        form("", &fields),
        super::actions_view::picker_template(&picker_t, cx.base_path, cx.settings.emojis()),
        crate::assets::url(cx.base_path, "emoji-picker.js")
    )
}

/// The category editor's Chat tab (EditCategoryChat): the category's and
/// its subcategories' channels by title (GET /chat/api/channels with
/// include_subcategories), the subcategories' toggle when they have any,
/// or the empty state.
pub fn category_tab(
    i18n: &crate::i18n::I18n,
    emojis: &crate::emojis::Emojis,
    base_path: &str,
    category_id: i64,
    channels: &[Value],
) -> String {
    let t = |key: &str| {
        escape(&crate::admin_view::t(
            i18n,
            &format!("chat.edit_category.{key}"),
        ))
    };
    let mut list: Vec<&Value> = channels.iter().collect();
    list.sort_by_key(|c| s(&c["title"]).to_lowercase());
    let mut out = String::from("<section class=\"edit-category-chat\">");
    if list.is_empty() {
        out.push_str(&format!(
            "<div class=\"empty-state__container\"><div class=\"empty-state\"><span class=\"empty-state__title\">{}</span><div class=\"empty-state__body\"><p>{}</p></div></div></div>",
            t("no_channels"),
            crate::admin_view::t(i18n, "chat.edit_category.no_channels_body")
                .replace("%{chatBrowseUrl}", &format!("{base_path}/chat/browse/open"))
        ));
        out.push_str("</section>");
        return out;
    }
    let is_sub = |c: &Value| c["chatable"]["id"].as_i64() != Some(category_id);
    if list.iter().any(|c| is_sub(c)) {
        out.push_str(&format!(
            "<div class=\"edit-category-chat__subcategory-toggle\"><div class=\"d-toggle-switch\"><label class=\"d-toggle-switch__label\"><button aria-checked=\"true\" class=\"d-toggle-switch__checkbox\" role=\"switch\" type=\"button\"></button><span class=\"d-toggle-switch__checkbox-slider\">{}</span></label></div><span>{}</span></div>",
            icon("check", None),
            t("include_subcategories")
        ));
    }
    out.push_str(&format!(
        "<table class=\"d-table edit-category-chat__table\"><thead class=\"d-table__header\"><tr class=\"d-table__row\"><th class=\"d-table__header-cell\">{}</th><th class=\"d-table__header-cell\">{}</th><th class=\"d-table__header-cell\"></th></tr></thead><tbody>",
        t("channel"),
        t("description")
    ));
    for c in list {
        let icon_html = match c["emoji"].as_str() {
            Some(e) => crate::category_badge::codes_to_img(emojis, base_path, &format!(":{e}:")),
            None => icon("d-chat", None),
        };
        let badge = if is_sub(c) {
            format!(
                "<div><span class=\"badge-category__wrapper\" style=\"--category-badge-color: #{};--category-badge-text-color: #{};\"><span data-category-id=\"{}\" data-drop-close=\"true\" class=\"badge-category --style-{}\"><span class=\"badge-category__name\">{}</span></span></span></div>",
                escape(s(&c["chatable"]["color"])),
                escape(s(&c["chatable"]["text_color"])),
                c["chatable"]["id"],
                escape(c["chatable"]["style_type"].as_str().unwrap_or("square")),
                escape(s(&c["chatable"]["name"]))
            )
        } else {
            String::new()
        };
        let description = s(&c["description"]);
        out.push_str(&format!(
            "<tr class=\"d-table__row\" data-channel-id=\"{id}\"{}><td class=\"d-table__cell edit-category-chat__channel-name\"><span class=\"chat-channel-icon --icon\" style=\"color: #{}\">{icon_html}</span><span>{}</span>{badge}</td><td class=\"d-table__cell edit-category-chat__channel-description\">{}</td><td class=\"d-table__cell edit-category-chat__channel-actions d-table__cell-actions\"><a class=\"btn btn-default btn-small\" href=\"{base_path}/chat/c/{}/{id}/info/settings\">{}</a></td></tr>",
            if is_sub(c) { " data-subcategory=\"\"" } else { "" },
            escape(c["chatable"]["color"].as_str().unwrap_or("000")),
            escape(s(&c["title"])),
            if description.is_empty() {
                "-".to_string()
            } else {
                escape(description)
            },
            escape(c["slug"].as_str().filter(|s| !s.is_empty()).unwrap_or("-")),
            t("settings"),
            id = c["id"],
        ));
    }
    out.push_str("</tbody></table>");
    out.push_str(&format!(
        "<script src=\"{}\"></script></section>",
        crate::assets::url(base_path, "chat-admin.js")
    ));
    out
}
