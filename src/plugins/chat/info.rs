//! A channel's info routes, server-rendered (ChatRoutesChannelInfo with
//! its nav, ChatRouteChannelInfoSettings and ChatRouteChannelInfoMembers).
//! static/js/chat-channel-info.js acts on them: the edit modals, the
//! toggles, the notification level, closing, opening and deleting the
//! channel, leaving it, the members filter and removing members.

use serde_json::Value;

use super::channels::{ChannelRow, Context};
use crate::post_view::d_icon;
use crate::topic_list_view::escape;
use crate::{AppError, Unsupported};

/// An info route's tab.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Settings,
    Members,
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}

fn js(i18n: &crate::i18n::I18n, key: &str) -> String {
    i18n.t(&format!("js.{key}")).unwrap_or_default().to_string()
}

/// DButton with a label (and an icon when given).
fn button(class: &str, icon: Option<&str>, label: &str, attrs: &str) -> String {
    format!(
        "<button class=\"btn {}{class}\"{attrs} type=\"button\">{}<span class=\"d-button-label\">{}</span></button>",
        if icon.is_some() {
            "btn-icon-text "
        } else {
            "btn-text "
        },
        icon.map(|i| d_icon(i, None)).unwrap_or_default(),
        escape(label)
    )
}

/// DToggleSwitch, its class on the switch.
fn toggle(class: &str, on: bool) -> String {
    format!(
        "<div class=\"d-toggle-switch\"><label class=\"d-toggle-switch__label\"><button aria-checked=\"{on}\" class=\"d-toggle-switch__checkbox {class}\" role=\"switch\" type=\"button\"></button><span class=\"d-toggle-switch__checkbox-slider\">{}</span></label></div>",
        if on {
            d_icon("check", None)
        } else {
            String::new()
        }
    )
}

/// ChatFormSection
fn section(title: Option<&str>, attrs: &str, content: &str) -> String {
    format!(
        "<div class=\"chat-form__section\"{attrs}>{}<div class=\"chat-form__section-content\">{content}</div></div>",
        title
            .map(|t| format!(
                "<div class=\"chat-form__section-title\">{}</div>",
                escape(t)
            ))
            .unwrap_or_default()
    )
}

/// ChatFormRow: its label, its default block, action and description.
fn row(
    label: Option<&str>,
    content: Option<&str>,
    action: Option<&str>,
    description: Option<&str>,
) -> String {
    let mut out =
        String::from("<div class=\"chat-form__row\"><div class=\"chat-form__row-content\">");
    if let Some(label) = label {
        out.push_str(&format!(
            "<span class=\"chat-form__row-label\">{}</span>",
            escape(label)
        ));
    }
    if let Some(content) = content {
        out.push_str(&format!(
            "<span class=\"chat-form__row-label\">{content}</span>"
        ));
    }
    if let Some(action) = action {
        out.push_str(&format!(
            "<div class=\"chat-form__row-action\">{action}</div>"
        ));
    }
    out.push_str("</div>");
    if let Some(description) = description {
        out.push_str(&format!(
            "<div class=\"chat-form__row-description\">{}</div>",
            escape(description)
        ));
    }
    out.push_str("</div>");
    out
}

/// select-kit's ComboBox, closed, its rows in a template for
/// chat-channel-info.js.
fn combo_box(class: &str, rows: &[(&str, String)], selected: &str, filter_by: &str) -> String {
    let name = rows
        .iter()
        .find(|(v, _)| *v == selected)
        .map(|(_, n)| n.clone())
        .unwrap_or_default();
    let mut template = String::new();
    for (index, (value, row)) in rows.iter().enumerate() {
        let is = *value == selected;
        template.push_str(&format!(
            "<li aria-checked=\"{is}\" role=\"menuitemradio\" data-index=\"{index}\" data-name=\"{n}\" data-value=\"{v}\" title=\"{n}\" tabindex=\"0\" class=\"select-kit-row{}\"><span class=\"name\">{n}</span></li>",
            if is { " is-highlighted is-selected" } else { "" },
            v = escape(value),
            n = escape(row)
        ));
    }
    format!(
        "<details class=\"select-kit single-select combobox combo-box {class} has-selection\"><summary aria-label=\"{fb}\" class=\"select-kit-header single-select-header combo-box-header\" data-name=\"{n}\" data-value=\"{v}\" name=\"{fb}\" tabindex=\"0\"><div class=\"select-kit-header-wrapper\"><div class=\"select-kit-selected-name selected-name choice\" data-name=\"{n}\" data-value=\"{v}\" title=\"{n}\"><span class=\"name\">{n}</span></div>{}</div></summary><div class=\"select-kit-body\"></div><template class=\"select-kit-rows\">{template}</template></details>",
        d_icon("angle-down", Some("angle-icon")),
        fb = escape(&filter_by.replace("%{name}", &name)),
        n = escape(&name),
        v = escape(selected),
    )
}

impl Context<'_> {
    /// The channel's info route on its tab: the navbar (back to the
    /// channel, the title and star), the status, the nav and the tab.
    /// The members tab of a channel that isn't open goes to the settings
    /// (ChatChannelInfoMembersRoute), which the caller redirects to.
    pub async fn channel_info_page(
        &mut self,
        channel: &ChannelRow,
        tab: Tab,
    ) -> Result<String, AppError> {
        let membership = self
            .memberships()
            .await?
            .into_iter()
            .find(|m| m.chat_channel_id == channel.id);
        let c = self
            .channel(channel, membership.as_ref(), None, None)
            .await?;
        let i18n = self.i18n;
        let t = move |key: &str| js(i18n, key);
        let slug = super::page::slugified_title(&c);
        let id = channel.id;
        let online = self.online_ids().await?;
        let mobile = crate::mobile::mobile_view();
        let following = c["current_user_membership"]["following"] == true;

        let mut out = String::from(
            "<div class=\"c-routes --channel-info\"><div class=\"c-navbar-container\"><nav class=\"c-navbar\">",
        );
        // ChatNavbarBackButton to the channel; ChatChannelInfoRouteOriginManager
        // has no origin on a page load.
        out.push_str(&format!(
            "<a class=\"c-navbar__back-button no-text btn-transparent btn\" title=\"{}\" href=\"{}/chat/c/{}/{id}\">{}{}</a>",
            escape(&t("chat.channel_info.back_to_channel")),
            self.base_path,
            escape(&slug),
            d_icon("chevron-left", None),
            if mobile { super::page::BACK_INDICATOR } else { "" }
        ));
        out.push_str(&format!(
            "<a class=\"c-navbar__channel-title\" href=\"{}/chat/c/{}/{id}/info/settings\">{}</a>",
            self.base_path,
            escape(&slug),
            super::page::channel_title(&c, self.settings.emojis(), self.base_path, &online)
        ));
        if following {
            let starred = c["current_user_membership"]["starred"] == true;
            out.push_str(&format!(
                "<span aria-expanded=\"false\" class=\"fk-d-tooltip__trigger\" data-identifier=\"star-channel\" data-trigger=\"\" role=\"button\" data-tooltip=\"{}\"><span class=\"fk-d-tooltip__trigger-container\"><button class=\"btn no-text btn-icon btn-transparent c-navbar__star-channel-button{}\" type=\"button\">{}<span aria-hidden=\"true\">&#8203;</span></button></span></span>",
                escape(&t(if starred {
                    "chat.channel_settings.unstar_channel"
                } else {
                    "chat.channel_settings.star_channel"
                })),
                if starred { " --starred" } else { "" },
                d_icon(if starred { "star" } else { "far-star" }, None)
            ));
        }
        out.push_str("</nav></div>");
        out.push_str(&super::page::channel_status(&c, &t));
        out.push_str("<div class=\"c-channel-info\"><div class=\"c-routes --channel-info-nav\">");
        // ChannelInfoNav: desktop, open channels.
        if !mobile && c["status"] == "open" {
            let link = |active: bool, path: &str, label: String| {
                format!(
                    "<li><a{} href=\"{}/chat/c/{}/{id}/info/{path}\">{label}</a></li>",
                    if active { " class=\"active\"" } else { "" },
                    self.base_path,
                    escape(&slug)
                )
            };
            let members_label = if channel.chatable_type == "Category" {
                format!(
                    "{}<span class=\"c-channel-info__member-count\">({})</span>",
                    escape(&t("chat.channel_info.tabs.members")),
                    c["memberships_count"].as_i64().unwrap_or(0)
                )
            } else {
                escape(&t("chat.channel_info.tabs.members"))
            };
            out.push_str(&format!(
                "<nav class=\"c-channel-info__nav\"><ul class=\"nav nav-pills\">{}{}</ul></nav>",
                link(
                    tab == Tab::Settings,
                    "settings",
                    escape(&t("chat.channel_info.tabs.settings"))
                ),
                link(tab == Tab::Members, "members", members_label)
            ));
        }
        out.push_str("</div>");
        match tab {
            Tab::Settings => out.push_str(&self.info_settings(channel, &c).await?),
            Tab::Members => out.push_str(&self.info_members(channel, &c).await?),
        }
        out.push_str("</div></div>");
        // What chat-channel-info.js shows: the toasts, the emoji picker.
        out.push_str(&super::actions_view::toast_template());
        out.push_str(&super::actions_view::picker_template(
            &t,
            self.base_path,
            self.settings.emojis(),
        ));
        out.push_str(&format!(
            "<script src=\"{}\"></script>",
            crate::assets::url(self.base_path, "chat-channel-info.js")
        ));
        Ok(out)
    }

    /// ChatRouteChannelInfoSettings
    async fn info_settings(&mut self, channel: &ChannelRow, c: &Value) -> Result<String, AppError> {
        let i18n = self.i18n;
        let t = move |key: &str| js(i18n, key);
        let category = channel.chatable_type == "Category";
        let group = c["chatable"]["group"] == true;
        let open = c["status"] == "open";
        let staff = self.guardian.is_staff();
        // chatGuardian.canEditChatChannel: chat on for the member, and staff.
        let chat_on = match self.guardian.user_id() {
            Some(id) => super::options(&mut *self.conn, id)
                .await?
                .map(|o| o.chat_enabled)
                .unwrap_or(true),
            None => false,
        };
        let can_edit_chat = chat_on && staff;
        let can_edit = (category && can_edit_chat) || (!category && group);
        let membership = &c["current_user_membership"];
        let following = membership["following"] == true;
        let muted = membership["muted"] == true;
        let slug = super::page::slugified_title(c);
        let id = channel.id;
        let mobile = crate::mobile::mobile_view();

        if can_edit_chat
            && category
            && self.settings.get("chat_allow_archiving_channels")?.truthy()
            && c["status"] != "archived"
            && c["status"] != "read_only"
        {
            return Err(
                Unsupported("archiving chat channels (the settings' archive button)").into(),
            );
        }

        let mut form = String::new();
        // The title section: the name, a category channel's url.
        let mut name = format!(
            "<div class=\"c-channel-settings__name\">{}</div>",
            crate::category_badge::codes_to_img(
                self.settings.emojis(),
                self.base_path,
                &escape(s(&c["title"]))
            )
        );
        if category {
            name.push_str(&format!(
                "<div class=\"c-channel-settings__slug\"><a href=\"{}/chat/c/{}/{id}\">/chat/c/{}/{id}</a></div>",
                self.base_path,
                escape(&slug),
                escape(s(&c["slug"]))
            ));
        }
        let edit = can_edit.then(|| {
            button(
                "edit-name-slug-btn btn-flat",
                None,
                &t("chat.channel_settings.edit"),
                "",
            )
        });
        form.push_str(&section(
            Some(&t("chat.about_view.title")),
            "",
            &row(None, Some(&name), edit.as_deref(), None),
        ));
        if category {
            let description = s(&c["description"]);
            let text = if description.is_empty() {
                escape(&t("chat.channel_edit_description_modal.description"))
            } else {
                escape(description)
            };
            let edit = can_edit.then(|| {
                button(
                    "edit-description-btn btn-flat",
                    None,
                    &t(if description.is_empty() {
                        "chat.channel_settings.add"
                    } else {
                        "chat.channel_settings.edit"
                    }),
                    "",
                )
            });
            form.push_str(&section(
                Some(&t("chat.about_view.description")),
                "",
                &row(None, Some(&text), edit.as_deref(), None),
            ));
        }
        if mobile {
            form.push_str(&section(
                None,
                "",
                &format!(
                    "<a class=\"chat-form__row -link\" href=\"{}/chat/c/{}/{id}/info/members\"><div class=\"chat-form__row-content\">{}{}</div></a>",
                    self.base_path,
                    escape(&slug),
                    escape(&t("chat.channel_info.tabs.members")),
                    d_icon("chevron-right", Some("chat-form__row-icon"))
                ),
            ));
        }
        // The member's settings.
        let threading_dm = !category && open;
        if open && (following || threading_dm) {
            let mut rows = String::new();
            if following {
                rows.push_str(&row(
                    Some(&t("chat.settings.mute")),
                    None,
                    Some(&toggle("c-channel-settings__mute-switch", muted)),
                    None,
                ));
            }
            if following && !muted {
                let levels: Vec<(&str, String)> = ["never", "mention", "always"]
                    .into_iter()
                    .map(|l| (l, t(&format!("chat.notification_levels.{l}"))))
                    .collect();
                rows.push_str(&row(
                    Some(&t("chat.settings.notification_level")),
                    None,
                    Some(&combo_box(
                        "c-channel-settings__selector c-channel-settings__notifications-selector",
                        &levels,
                        s(&membership["notification_level"]),
                        &t("select_kit.filter_by"),
                    )),
                    None,
                ));
            }
            if threading_dm {
                rows.push_str(&row(
                    Some(&t("chat.settings.channel_threading_label")),
                    None,
                    Some(&toggle(
                        "c-channel-settings__threading-switch",
                        c["threading_enabled"] == true,
                    )),
                    Some(&t("chat.settings.channel_threading_description")),
                ));
            }
            form.push_str(&section(
                Some(&t("chat.settings.settings_title")),
                "",
                &rows,
            ));
        }
        // Channel info: the category, the history.
        let mut rows = String::new();
        if category {
            rows.push_str(&row(
                Some(&t("chat.settings.category_label")),
                Some(&self.category_badge(&c["chatable"]).await?),
                None,
                None,
            ));
        }
        rows.push_str(&row(
            Some(&t("chat.settings.history_label")),
            Some(&self.retention_text(category)),
            None,
            None,
        ));
        form.push_str(&section(Some(&t("chat.settings.info_title")), "", &rows));
        // Admin.
        if can_edit && (category && open || category) {
            let mut rows = String::new();
            if self.settings.get("max_chat_auto_joined_users")?.to_i() > 0 && category && open {
                rows.push_str(&row(
                    Some(&t("chat.settings.auto_join_users_label")),
                    None,
                    Some(&toggle(
                        "c-channel-settings__auto-join-switch",
                        c["auto_join_users"] == true,
                    )),
                    None,
                ));
            }
            if category && open {
                rows.push_str(&row(
                    Some(&t("chat.settings.channel_wide_mentions_label")),
                    None,
                    Some(&toggle(
                        "c-channel-settings__channel-wide-mentions",
                        c["allow_channel_wide_mentions"] == true,
                    )),
                    Some(
                        &t("chat.settings.channel_wide_mentions_description")
                            .replace("%{channel}", s(&c["title"])),
                    ),
                ));
                rows.push_str(&row(
                    Some(&t("chat.settings.channel_threading_label")),
                    None,
                    Some(&toggle(
                        "c-channel-settings__threading-switch",
                        c["threading_enabled"] == true,
                    )),
                    Some(&t("chat.settings.channel_threading_description")),
                ));
            }
            if category {
                let status = if open {
                    button(
                        "close-btn chat-form__btn btn-transparent",
                        Some("lock"),
                        &t("chat.channel_settings.close_channel"),
                        "",
                    )
                } else if c["status"] == "closed" {
                    button(
                        "open-btn chat-form__btn btn-transparent",
                        Some("unlock"),
                        &t("chat.channel_settings.open_channel"),
                        "",
                    )
                } else {
                    let reason = t(if c["status"] == "archived" {
                        "chat.channel_settings.open_channel_disabled_archived"
                    } else {
                        "chat.channel_settings.open_channel_disabled_read_only"
                    });
                    format!(
                        "<span aria-expanded=\"false\" class=\"fk-d-tooltip__trigger\" data-identifier=\"channel-open-disabled\" data-trigger=\"\" role=\"button\" data-tooltip=\"{}\"><span class=\"fk-d-tooltip__trigger-container\">{}</span></span>",
                        escape(&reason),
                        button(
                            "open-btn chat-form__btn btn-transparent",
                            Some("unlock"),
                            &t("chat.channel_settings.open_channel"),
                            " disabled=\"\"",
                        )
                    )
                };
                rows.push_str(&row(None, None, Some(&status), None));
                rows.push_str(&row(
                    None,
                    None,
                    Some(&button(
                        "delete-btn chat-form__btn btn-transparent",
                        Some("trash-can"),
                        &t("chat.channel_settings.delete_channel"),
                        "",
                    )),
                    None,
                ));
            }
            form.push_str(&section(
                Some(&t("chat.settings.admin_title")),
                " data-section=\"admin\"",
                &rows,
            ));
        }
        // Leaving (or joining).
        let mut leave = String::new();
        if group {
            leave.push_str(&format!(
                "<div class=\"c-channel-settings__leave-info\">{}{}</div>",
                d_icon("triangle-exclamation", None),
                escape(&t("chat.channel_settings.leave_groupchat_info"))
            ));
        }
        let membership_button = if following {
            let title = t("chat.channel_settings.leave_channel");
            format!(
                "<button class=\"btn btn-icon-text toggle-channel-membership-button -leave btn-default\" title=\"{}\" type=\"button\">{}<span class=\"d-button-label\">{}</span></button>",
                escape(&title),
                d_icon("right-from-bracket", None),
                escape(&title)
            )
        } else {
            let title = t("chat.channel_settings.join_channel");
            format!(
                "<button class=\"btn btn-icon-text toggle-channel-membership-button -join btn-primary\" title=\"{}\" type=\"button\">{}<span class=\"d-button-label\">{}</span></button>",
                escape(&title),
                d_icon("right-to-bracket", None),
                escape(&title)
            )
        };
        leave.push_str(&row(None, None, Some(&membership_button), None));
        form.push_str(&format!(
            "<div class=\"chat-form__section --leave-channel\"><div class=\"chat-form__section-content\">{leave}</div></div>"
        ));
        Ok(format!(
            "<div class=\"c-routes --channel-info-settings\"><div class=\"c-channel-settings\" {}><div class=\"chat-form\">{form}</div></div></div>",
            self.info_data(channel, c)
        ))
    }

    /// What chat-channel-info.js reads: the channel, its title, category
    /// and what its modals and toasts say.
    fn info_data(&self, channel: &ChannelRow, c: &Value) -> String {
        let i18n = self.i18n;
        let t = move |key: &str| js(i18n, key);
        let strings = serde_json::json!({
            "saved": t("saved"),
            "save": t("save"),
            "cancel": t("cancel"),
            "edit_name_title": t("chat.channel_edit_name_slug_modal.title"),
            "name_label": t("chat.channel_edit_name_slug_modal.name"),
            "name_placeholder": t("chat.channel_edit_name_slug_modal.input_placeholder"),
            "slug_label": t("chat.channel_edit_name_slug_modal.slug"),
            "slug_description": t("chat.channel_edit_name_slug_modal.slug_description"),
            "emoji_label": t("chat.channel_edit_name_slug_modal.emoji"),
            "reset_emoji": t("chat.channel_edit_name_slug_modal.reset_emoji"),
            "edit_description_title": t("chat.channel_edit_description_modal.title"),
            "description_label": t("chat.channel_edit_description_modal.description"),
            "description_placeholder": t("chat.channel_edit_description_modal.input_placeholder"),
            "open_title": t("chat.channel_open.title"),
            "open_instructions": t("chat.channel_open.instructions"),
            "close_title": t("chat.channel_close.title"),
            "close_instructions": t("chat.channel_close.instructions"),
            "open_channel": t("chat.channel_settings.open_channel"),
            "close_channel": t("chat.channel_settings.close_channel"),
            "delete_title": t("chat.channel_delete.title"),
            "delete_instructions": t("chat.channel_delete.instructions"),
            "delete_confirm": t("chat.channel_delete.confirm"),
            "delete_confirm_name": t("chat.channel_delete.confirm_channel_name"),
            "delete_started": t("chat.channel_delete.process_started"),
            "auto_join_warning": t("chat.settings.auto_join_users_warning"),
            "generic_error": t("generic_error"),
        });
        format!(
            "data-channel-id=\"{}\" data-title=\"{}\" data-slug=\"{}\" data-emoji=\"{}\" data-description=\"{}\" data-category-name=\"{}\" data-max-title=\"{}\" data-strings=\"{}\"",
            channel.id,
            escape(s(&c["title"])),
            escape(s(&c["slug"])),
            escape(s(&c["emoji"])),
            escape(s(&c["description"])),
            escape(s(&c["chatable"]["name"])),
            self.settings
                .get("max_topic_title_length")
                .map(|v| v.to_i())
                .unwrap_or(255),
            escape(&strings.to_string())
        )
    }

    /// ChatRetentionReminderText, short.
    fn retention_text(&self, category: bool) -> String {
        let i18n = self.i18n;
        let t = move |key: &str| js(i18n, key);
        let days = self
            .settings
            .get(if category {
                "chat_channel_retention_days"
            } else {
                "chat_dm_retention_days"
            })
            .map(|v| v.to_i())
            .unwrap_or(0);
        let text = if days > 0 {
            t(if days == 1 {
                "chat.retention_reminders.short.one"
            } else {
                "chat.retention_reminders.short.other"
            })
            .replace("%{count}", &days.to_string())
        } else {
            t("chat.retention_reminders.indefinitely_short")
        };
        format!("<span class=\"chat-retention-reminder-text\">{text}</span>")
    }

    /// dCategoryBadge(chatable, link: true, allowUncategorized: true)
    async fn category_badge(&mut self, cat: &Value) -> Result<String, AppError> {
        let id = cat["id"].as_i64().unwrap_or(0);
        let parent: Option<(i32, String, String, String)> = match cat["parent_category_id"].as_i64()
        {
            Some(pid) => {
                sqlx::query_as("SELECT id, color, text_color, slug FROM categories WHERE id = $1")
                    .bind(super::channels::category_id(pid))
                    .fetch_optional(&mut *self.conn)
                    .await?
            }
            None => None,
        };
        let mut style = format!(
            "--category-badge-color: #{};--category-badge-text-color: #{};",
            s(&cat["color"]),
            s(&cat["text_color"])
        );
        let mut classes = String::from("badge-category");
        let restricted = cat["read_restricted"] == true;
        if restricted {
            classes.push_str(" restricted");
        }
        let mut data = format!("data-category-id=\"{id}\"");
        let mut path = s(&cat["slug"]).to_string();
        if let Some((pid, color, text_color, pslug)) = &parent {
            style.push_str(&format!(
                "--parent-category-badge-color: #{color};--parent-category-badge-text-color: #{text_color};"
            ));
            classes.push_str(" --has-parent");
            data.push_str(&format!(" data-parent-category-id=\"{pid}\""));
            path = format!("{pslug}/{path}");
        }
        let style_type = cat["style_type"].as_str().unwrap_or("square");
        classes.push_str(&format!(" --style-{style_type}"));
        let mut inner = String::new();
        match style_type {
            "icon" => {
                if let Some(icon) = cat["icon"].as_str() {
                    inner.push_str(&d_icon(icon, None));
                }
            }
            "emoji" => {
                if let Some(emoji) = cat["emoji"].as_str() {
                    inner.push_str(&crate::category_badge::codes_to_img(
                        self.settings.emojis(),
                        self.base_path,
                        &format!(":{emoji}:"),
                    ));
                }
            }
            _ => {}
        }
        if restricted {
            inner.push_str(&d_icon("lock", None));
        }
        let dir = if self.settings.get("support_mixed_text_direction")?.truthy() {
            " dir=\"auto\""
        } else {
            ""
        };
        Ok(format!(
            "<a class=\"badge-category__wrapper \" style=\"{style}\" href=\"{}/c/{}/{id}\"><span {data} data-drop-close=\"true\" class=\"{classes}\">{inner}<span class=\"badge-category__name\"{dir}>{}</span></span></a>",
            self.base_path,
            escape(&path),
            escape(s(&cat["name"]))
        ))
    }

    /// ChatRouteChannelInfoMembers: the filter and the first members
    /// (`listChannelMemberships` with limit 20); chat-channel-info.js
    /// filters and loads more as it scrolls. Adding members is a group
    /// DM's (MessageCreator's add members mode): refused.
    async fn info_members(&mut self, channel: &ChannelRow, c: &Value) -> Result<String, AppError> {
        let i18n = self.i18n;
        let t = move |key: &str| js(i18n, key);
        let dm = channel.chatable_type == "DirectMessage";
        let can_group = self.guardian.is_staff()
            || self.settings.get("chat_max_direct_message_users")?.to_i() > 1;
        if dm
            && can_group
            && (c["chatable"]["group"] == true || c["last_message"]["message"].is_null())
        {
            return Err(
                Unsupported("adding members to a direct message from its members (222a)").into(),
            );
        }
        let members = self.members(channel, 0, 20, None).await?;
        let mut out = String::from("<div class=\"c-routes --channel-info-members\">");
        let slug = super::page::slugified_title(c);
        if crate::mobile::mobile_view() {
            out.push_str(&format!(
                "<a class=\"c-back-button\" href=\"{}/chat/c/{}/{}/info/settings\">{}{}</a>",
                self.base_path,
                escape(&slug),
                channel.id,
                d_icon("chevron-left", None),
                escape(&t("chat.members_view.back_to_settings"))
            ));
        }
        out.push_str(&format!(
            "<div class=\"c-channel-members\" data-channel-id=\"{}\" data-page-url=\"{}/chat/c/{}/{}/info/members/page\">",
            channel.id,
            self.base_path,
            escape(&slug),
            channel.id,
        ));
        // DFilterInput with its right icon.
        out.push_str(&format!(
            "<div class=\"d-filter-input c-channel-members__filter\"><input class=\"d-filter-input__input\" placeholder=\"{}\" type=\"text\" autofocus=\"\">{}</div>",
            escape(&t("chat.members_view.filter_placeholder")),
            d_icon("magnifying-glass", Some("d-filter-input__icon -right"))
        ));
        out.push_str("<ul class=\"c-channel-members__list\">");
        out.push_str(&self.member_items(channel, &members).await?);
        out.push_str("</ul><div class=\"c-channel-members__load-more\"><br></div></div></div>");
        Ok(out)
    }

    /// The members list's items for a page of memberships: each member,
    /// with the remove button when the viewer may remove them
    /// (`canRemoveMember`), or the no results item.
    async fn member_items(
        &mut self,
        channel: &ChannelRow,
        members: &Value,
    ) -> Result<String, AppError> {
        let i18n = self.i18n;
        let t = move |key: &str| js(i18n, key);
        // can_remove_members?: admins, in a category channel or a group DM.
        let can_remove = self.guardian.is_admin()
            && (channel.chatable_type == "Category"
                || (channel.chatable_type == "DirectMessage"
                    && self.direct_message(channel.chatable_id).await?.group));
        let viewer = i64::from(self.guardian.user_id().unwrap_or(0));
        let name_first = !self.settings.get("prioritize_username_in_ux")?.truthy();
        let online = self.online_ids().await?;
        let list = members["memberships"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        if list.is_empty() {
            return Ok(format!(
                "<li class=\"c-channel-members__list-item -no-results alert alert-info\">{}</li>",
                escape(&t("chat.channel.no_memberships_found"))
            ));
        }
        let mut out = String::new();
        for m in &list {
            out.push_str(&member_item(
                &m["user"],
                can_remove && m["user"]["id"].as_i64() != Some(viewer),
                name_first,
                &online,
                &t,
            )?);
        }
        Ok(out)
    }

    /// The members page's next items (GET .../info/members/page).
    pub async fn channel_info_members_items(
        &mut self,
        channel: &ChannelRow,
        offset: i64,
        username: Option<&str>,
    ) -> Result<String, AppError> {
        let members = self.members(channel, offset, 20, username).await?;
        if offset > 0
            && members["memberships"]
                .as_array()
                .is_none_or(|m| m.is_empty())
        {
            return Ok(String::new());
        }
        self.member_items(channel, &members).await
    }
}

/// A member in the list: ChatUserInfo (tiny avatar, not interactive) and
/// the remove button when the viewer may remove them.
pub(crate) fn member_item(
    user: &Value,
    removable: bool,
    name_first: bool,
    online: &[i64],
    t: &dyn Fn(&str) -> String,
) -> Result<String, AppError> {
    let username = s(&user["username"]);
    let name = user["name"].as_str().filter(|n| !n.trim().is_empty());
    let mut info = super::page::static_user_avatar(user, 24, online);
    // ChatUserDisplayName
    info.push_str("<span class=\"chat-user-display-name\">");
    let first = name.filter(|_| name_first);
    if let Some(name) = first {
        info.push_str(&format!(
            "<span class=\"chat-user-display-name__name -first\">{}</span>",
            escape(name)
        ));
    }
    info.push_str(&format!(
        "<span class=\"chat-user-display-name__username{}\">{}</span>",
        if first.is_none() { " -first" } else { "" },
        escape(username)
    ));
    if let Some(name) = name.filter(|_| !name_first) {
        info.push_str(&format!(
            "<span class=\"chat-user-display-name__name\">{}</span>",
            escape(name)
        ));
    }
    info.push_str("</span><div class=\"user-status\"></div>");
    Ok(format!(
        "<li class=\"c-channel-members__list-item -member\" tabindex=\"0\" data-user-id=\"{}\" data-username=\"{}\"><div class=\"c-channel-members__list-item -user-info\" role=\"button\">{info}</div>{}</li>",
        user["id"],
        escape(username),
        if removable {
            button(
                "btn-flat -remove-member",
                None,
                &t("chat.channel_info.remove_member"),
                "",
            )
        } else {
            String::new()
        }
    ))
}
