//! Full-page chat, server-rendered: the chat template's layout
//! (full-page-chat, main-chat-outlet) around the channel route
//! (ChatRoutesChannel: the navbar, then ChatChannel with its messages and
//! composer). What the Ember components compute in the browser is
//! computed here from the API's own payloads (the channel serializer,
//! Chat::ListChannelMessages); what depends on the browser's clock and
//! zone (the date separators, the messages' times in the member's zone)
//! is redrawn by static/js/chat.js from each message's data-created-at.
//!
//! The message actions (react, reply, the menus), the composer's
//! sending and the emoji picker are drawn as their controls; they act in
//! slice 2.

use serde_json::Value;

use super::channels::{ChannelRow, Context};
use super::messages::{ListParams, Listed};
use crate::post_view::d_icon;
use crate::topic_list_view::escape;
use crate::{AppError, Unsupported};

/// `DEFAULT_MESSAGE_PAGE_SIZE`
const PAGE_SIZE: i64 = 50;
/// The empty state's facepile (MAX_AVATARS).
const MAX_AVATARS: i64 = 5;

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}

/// `renderAvatar(user, { imageSize })`
fn avatar(user: &Value, size: u32) -> String {
    format!(
        "<img alt=\"\" width=\"{size}\" height=\"{size}\" src=\"{}\" class=\"avatar\" title=\"{}\">",
        escape(&s(&user["avatar_template"]).replace("{size}", &size.to_string())),
        escape(s(&user["username"]))
    )
}

/// A client string (`js.` keys).
fn js(i18n: &crate::i18n::I18n, key: &str) -> String {
    i18n.t(&format!("js.{key}")).unwrap_or(key).to_string()
}

/// What every message in the page reads.
struct View<'a> {
    base_path: &'a str,
    channel: &'a Value,
    viewer_id: i64,
    emoji_set: String,
    /// `userCanInteractWithChat`: not silenced.
    can_interact: bool,
    t: &'a (dyn Fn(&str) -> String + Sync),
}

impl Context<'_> {
    /// The chat page around a route's content (templates/chat.gjs with the
    /// core sidebar).
    pub fn full_page(content: &str) -> String {
        format!(
            "<div id=\"chat-progress-bar-container\"></div><div class=\"full-page-chat full-page-chat-sidebar-enabled\"><div class=\"main-chat-outlet chat-view\" id=\"main-chat-outlet\">{content}</div></div>"
        )
    }

    /// The channel route: the navbar and the channel, `target` the message
    /// a near-message url names.
    pub async fn channel_page(
        &mut self,
        channel: &ChannelRow,
        target: Option<i64>,
    ) -> Result<String, AppError> {
        let membership = self
            .memberships()
            .await?
            .into_iter()
            .find(|m| m.chat_channel_id == channel.id);
        let channel_json = self
            .channel(channel, membership.as_ref(), None, None)
            .await?;
        let following = membership.as_ref().is_some_and(|m| m.following);
        let last_read = membership.as_ref().and_then(|m| m.last_read_message_id);

        // ChatChannel#loadMessages: around the target, else from the last
        // read message.
        let params = ListParams {
            page_size: PAGE_SIZE,
            target_message_id: target,
            direction: None,
            fetch_from_last_read: target.is_none(),
            target_date: None,
        };
        let listed = match self.list_messages(channel.id, &params).await? {
            Listed::Messages(v) => v,
            Listed::NotFound | Listed::Forbidden => {
                return Err(Unsupported("a chat page for a message it can't load").into());
            }
        };
        let messages = listed["messages"].as_array().cloned().unwrap_or_default();

        let i18n = self.i18n;
        let t = move |key: &str| js(i18n, key);
        let viewer_id = i64::from(self.guardian.user_id().unwrap_or(0));
        let view = View {
            base_path: self.base_path,
            channel: &channel_json,
            viewer_id,
            emoji_set: self.settings.get("emoji_set")?.to_s().to_string(),
            can_interact: channel_json["meta"]["user_silenced"] != true,
            t: &t,
        };

        let mut out = String::from("<div class=\"c-routes --channel\">");
        out.push_str(&self.navbar(&channel_json, following)?);

        let mut classes = String::from("chat-channel --loaded");
        if messages.is_empty() {
            classes.push_str(" is-empty");
        }
        out.push_str(&format!(
            "<div class=\"{classes}\" data-id=\"{}\" data-label-today=\"{}\" data-label-yesterday=\"{}\" data-label-last-visit=\"{}\" data-format-time=\"{}\" data-format-tiny=\"{}\" data-format-title=\"{}\" data-format-date=\"{}\">",
            channel.id,
            escape(&t("chat.chat_message_separator.today")),
            escape(&t("chat.chat_message_separator.yesterday")),
            escape(&t("chat.last_visit")),
            escape(&t("dates.time")),
            escape(&t("dates.time_short")),
            escape(&t("dates.long_with_year")),
            // moment's LL, in English.
            "MMMM D, YYYY",
        ));
        out.push_str(&channel_status(&channel_json, &t));
        out.push_str(&format!(
            "<div class=\"chat-notices\">{}</div>",
            self.retention_reminder().await?
        ));
        out.push_str(
            "<div class=\"chat-messages-scroller\"><div class=\"chat-messages-container\">",
        );
        if messages.is_empty() {
            out.push_str(&self.empty_state(channel, &channel_json, following).await?);
        } else {
            // processMessages: the first message past the last read one is
            // the newest (the "last visit" line), unless it is the
            // channel's last message.
            let newest = messages
                .iter()
                .find(|m| m["id"].as_i64().unwrap_or(0) > last_read.unwrap_or(0))
                .and_then(|m| m["id"].as_i64())
                .filter(|id| Some(*id) != channel_json["last_message"]["id"].as_i64());
            for index in 0..messages.len() {
                out.push_str(&message_html(&view, &messages, index, newest, last_read)?);
            }
        }
        out.push_str("</div>");
        if !messages.is_empty() && listed["meta"]["can_load_more_past"] == false {
            out.push_str(&format!(
                "<div class=\"all-loaded-message\">{}</div>",
                escape(&t("chat.all_loaded"))
            ));
        }
        out.push_str("</div>");
        out.push_str(&format!(
            "<div class=\"chat-scroll-to-bottom\"><button class=\"btn no-text btn-flat chat-scroll-to-bottom__button\" type=\"button\"><span class=\"chat-scroll-to-bottom__arrow\">{}</span></button></div>",
            d_icon("arrow-down", None)
        ));
        if !following {
            out.push_str(&preview_card(&channel_json, &t));
        } else {
            out.push_str(&self.composer(channel, &channel_json).await?);
        }
        out.push_str("</div></div>");
        Ok(out)
    }

    /// ChatRetentionReminder on a category channel, for staff who haven't
    /// dismissed it while channel messages expire
    /// (`needs_channel_retention_reminder`).
    async fn retention_reminder(&mut self) -> Result<String, AppError> {
        let i18n = self.i18n;
        let t = move |key: &str| js(i18n, key);
        let days = self.settings.get("chat_channel_retention_days")?.to_i();
        let Some(user_id) = self.guardian.user_id() else {
            return Ok(String::new());
        };
        if !self.guardian.is_staff() || days == 0 {
            return Ok(String::new());
        }
        let dismissed = super::options(&mut *self.conn, user_id)
            .await?
            .and_then(|o| o.dismissed_channel_retention_reminder)
            .unwrap_or(false);
        if dismissed {
            return Ok(String::new());
        }
        let label = escape(&t("chat.retention_reminders.chat_settings"));
        let link = if self.guardian.is_admin() {
            format!(
                "<a href=\"{}/admin/site_settings/category/chat\">{label}</a>",
                self.base_path
            )
        } else {
            label
        };
        let key = if days == 1 {
            "chat.retention_reminders.long.one"
        } else {
            "chat.retention_reminders.long.other"
        };
        Ok(format!(
            "<div class=\"chat-retention-reminder\"><span class=\"chat-retention-reminder-text\">{}</span><button class=\"btn no-text btn-icon btn-transparent dismiss-btn\" type=\"button\">{}<span aria-hidden=\"true\">&#8203;</span></button></div>",
            t(key)
                .replace("%{chatSettingsLink}", &link)
                .replace("%{count}", &days.to_string()),
            d_icon("xmark", None)
        ))
    }

    /// ChatNavbar on the channel route: the title (linking to the
    /// channel's settings), the star, and the actions.
    fn navbar(&self, c: &Value, following: bool) -> Result<String, AppError> {
        let i18n = self.i18n;
        let t = move |key: &str| js(i18n, key);
        let slug = s(&c["slug"]);
        let id = c["id"].as_i64().unwrap_or(0);
        let mut out = String::from("<div class=\"c-navbar-container\"><nav class=\"c-navbar\">");
        out.push_str(&format!(
            "<a class=\"c-navbar__channel-title\" href=\"{}/chat/c/{}/{id}/info/settings\">{}</a>",
            self.base_path,
            escape(slug),
            channel_title(c, &self.emoji_set()?, self.base_path)
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
        out.push_str("<nav class=\"c-navbar__actions\">");
        if self.settings.get("chat_search_enabled")?.truthy() {
            out.push_str(&format!(
                "<button class=\"btn no-text btn-icon btn-transparent c-navbar__filter\" type=\"button\">{}<span aria-hidden=\"true\">&#8203;</span></button>",
                d_icon("discourse-chat-search", None)
            ));
        }
        out.push_str(&format!(
            "<button class=\"btn no-text btn-icon c-navbar__open-drawer-button btn-transparent\" title=\"{}\" type=\"button\">{}<span aria-hidden=\"true\">&#8203;</span></button>",
            escape(&t("chat.close_full_page")),
            d_icon("discourse-compress", None)
        ));
        if self.settings.get("chat_pinned_messages")?.truthy()
            && c["pinned_messages_count"].as_i64().unwrap_or(0) > 0
        {
            out.push_str(&format!(
                "<a class=\"c-navbar__pinned-messages-btn btn no-text btn-transparent\" title=\"{}\" href=\"{}/chat/c/{}/{id}/pins\">{}</a>",
                escape(&t("chat.pinned_messages.title")),
                self.base_path,
                escape(slug),
                d_icon("thumbtack", None)
            ));
        }
        if c["threading_enabled"] == true {
            return Err(Unsupported("chat threads (the threads list button)").into());
        }
        out.push_str("</nav></nav></div>");
        Ok(out)
    }

    fn emoji_set(&self) -> Result<String, AppError> {
        Ok(self.settings.get("emoji_set")?.to_s().to_string())
    }

    /// ChatChannelEmptyState for a member: the channel's icon, the title,
    /// the tip and the other members' faces.
    async fn empty_state(
        &mut self,
        channel: &ChannelRow,
        c: &Value,
        following: bool,
    ) -> Result<String, AppError> {
        let i18n = self.i18n;
        let t = move |key: &str| js(i18n, key);
        let channel_name = format!("#{}", s(&c["title"]));
        let total = c["memberships_count"].as_i64().unwrap_or(0);
        let count = (if following { total - 1 } else { total }).max(0);
        let icon = match c["emoji"].as_str() {
            Some(emoji) => emoji_img(&self.emoji_set()?, self.base_path, emoji),
            None => d_icon("d-chat", None),
        };
        let icon = match c["chatable"]["color"].as_str() {
            Some(color) => format!("<span style=\"color: #{}\">{icon}</span>", escape(color)),
            None => icon,
        };
        let (title, tip) = if following {
            (
                "chat.channel.empty_state.joined_title",
                "chat.channel.empty_state.joined_tip",
            )
        } else {
            (
                "chat.channel.empty_state.title",
                "chat.channel.empty_state.guest_tip",
            )
        };
        let mut out = String::from(
            "<div class=\"empty-state__container --chat-channel --with-image\"><div class=\"empty-state\">",
        );
        out.push_str(&format!("<div class=\"empty-state__image\">{icon}</div>"));
        out.push_str(&format!(
            "<div class=\"empty-state__title\" data-test-title=\"\">{}</div>",
            escape(&t(title).replace("%{channelName}", &channel_name))
        ));
        if let Some(description) = c["description"].as_str() {
            out.push_str(&format!(
                "<div class=\"empty-state__body\"><p data-test-body=\"\">{}</p></div>",
                escape(description)
            ));
        }
        // The first five memberships less the viewer's.
        let others: Vec<Value> = if count > 0 {
            let viewer = i64::from(self.guardian.user_id().unwrap_or(0));
            self.members(channel, 0, MAX_AVATARS, None).await?["memberships"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|m| m["user"]["id"].as_i64() != Some(viewer))
                .collect()
        } else {
            Vec::new()
        };
        out.push_str(&format!(
            "<div class=\"empty-state__tip\"><p class=\"empty-state__tip-text\">{}</p><div class=\"empty-state__members-facepile{}\">",
            escape(&t(tip)),
            if others.is_empty() { "" } else { " --with-avatars" }
        ));
        if !others.is_empty() {
            out.push_str("<div class=\"empty-state__members-avatars\">");
            for m in &others {
                out.push_str(&avatar(&m["user"], 24));
            }
            out.push_str("</div>");
        }
        let members = if count > 0 {
            let key = if count == 1 {
                "chat.channel.empty_state.members_here.one"
            } else {
                "chat.channel.empty_state.members_here.other"
            };
            t(key).replace("%{count}", &count.to_string())
        } else {
            escape(&t("chat.channel.empty_state.no_other_members"))
        };
        out.push_str(&format!(
            "<span class=\"empty-state__members-count\">{members}</span></div></div></div></div>"
        ));
        Ok(out)
    }

    /// ChatComposerChannel: the draft's text, the toolbar trigger, the
    /// emoji and send buttons, the upload field and the replying line.
    async fn composer(&mut self, channel: &ChannelRow, c: &Value) -> Result<String, AppError> {
        let i18n = self.i18n;
        let t = move |key: &str| js(i18n, key);
        let status = s(&c["status"]);
        let staff = self.guardian.is_staff();
        // canModifyMessages
        let can_modify = status == "open" || (staff && status == "closed");
        let can_interact = c["meta"]["user_silenced"] != true;
        let disabled = !can_interact || !can_modify;
        let placeholder = if !can_modify {
            t(&format!("chat.placeholder_new_message_disallowed.{status}"))
        } else if !can_interact {
            t("chat.placeholder_silenced")
        } else {
            t("chat.placeholder_channel").replace("%{channelName}", &format!("#{}", s(&c["title"])))
        };
        let draft = self.draft_message(channel.id).await?;
        let disabled_attr = if disabled { " disabled=\"\"" } else { "" };
        let mut out = String::from("<div class=\"chat-composer__wrapper\">");
        out.push_str(&format!(
            "<div aria-label=\"{}\" class=\"chat-composer is-send-disabled {} is-draft-saved\" role=\"region\"><div class=\"chat-composer__outer-container\"><div class=\"chat-composer__inner-container\">",
            escape(&t("chat.aria_roles.composer")),
            if disabled { "is-disabled" } else { "is-enabled" }
        ));
        out.push_str(&format!(
            "<button aria-expanded=\"false\" class=\"btn no-text btn-icon fk-d-menu__trigger chat-composer-dropdown__menu-trigger chat-composer-dropdown__trigger-btn btn-flat\" title=\"{}\" data-identifier=\"chat-composer-dropdown__menu\" data-trigger=\"\" type=\"button\"{disabled_attr}>{}</button>",
            escape(&t("chat.composer.toggle_toolbar")),
            d_icon("plus", None)
        ));
        out.push_str(&format!(
            "<div class=\"chat-composer__input-container\"><textarea placeholder=\"{}\" autocorrect=\"on\" autocapitalize=\"sentences\" rows=\"1\" id=\"channel-composer\" class=\"chat-composer__input\" data-chat-composer-context=\"channel\" type=\"text\"{disabled_attr}>{}</textarea></div>",
            escape(&placeholder),
            escape(&draft)
        ));
        out.push_str(&format!(
            "<button aria-expanded=\"false\" class=\"btn no-text fk-d-menu__trigger emoji-picker-trigger chat-composer-button btn-transparent --emoji\" data-identifier=\"emoji-picker\" data-trigger=\"\" type=\"button\"{disabled_attr}>{}&#8203;</button><div class=\"chat-composer-separator\"></div>",
            d_icon("far-face-smile", None)
        ));
        out.push_str(&format!(
            "<div class=\"chat-composer-button__wrapper\"><button class=\"chat-composer-button -send\" disabled=\"\" tabindex=\"-1\" title=\"{}\" type=\"button\">{}</button></div>",
            escape(&t("chat.composer.send")),
            d_icon("paper-plane", None)
        ));
        out.push_str("</div></div></div>");
        if self.settings.get("chat_allow_uploads")?.truthy() {
            out.push_str(&format!(
                "<div class=\"chat-composer-uploads\"><div class=\"pick-files-button\"><input accept=\"{}\" class=\"hidden-upload-field\" id=\"channel-file-uploader\" multiple=\"\" type=\"file\"></div></div>",
                escape(&self.upload_accept()?)
            ));
        }
        out.push_str("<div class=\"chat-replying-indicator-container\"><div class=\"chat-replying-indicator is-subscribed\"></div></div></div>");
        Ok(out)
    }

    /// The member's draft for the channel (chat_drafts): its text. A draft
    /// carrying uploads, a reply or an edit is refused until composing is
    /// ported.
    async fn draft_message(&mut self, channel_id: i64) -> Result<String, AppError> {
        let Some(user_id) = self.guardian.user_id() else {
            return Ok(String::new());
        };
        let data: Option<Option<String>> = sqlx::query_scalar(
            "SELECT data FROM chat_drafts WHERE user_id = $1 AND chat_channel_id = $2 AND thread_id IS NULL \
             ORDER BY updated_at DESC LIMIT 1",
        )
        .bind(user_id)
        .bind(channel_id)
        .fetch_optional(&mut *self.conn)
        .await?;
        let Some(Some(data)) = data else {
            return Ok(String::new());
        };
        let draft: Value = serde_json::from_str(&data).unwrap_or(Value::Null);
        let only_text = draft.as_object().is_some_and(|o| {
            o.iter().all(|(k, v)| match k.as_str() {
                "message" => true,
                "uploads" => v.as_array().is_none_or(Vec::is_empty),
                _ => v.is_null(),
            })
        });
        if !only_text {
            return Err(Unsupported("chat drafts with uploads, replies or edits").into());
        }
        Ok(draft["message"].as_str().unwrap_or_default().to_string())
    }

    /// The upload field's accepted types: the authorized extensions for
    /// chat (images only unless more are authorized).
    fn upload_accept(&self) -> Result<String, AppError> {
        let extensions = self
            .settings
            .get("authorized_extensions")?
            .to_s()
            .to_string();
        if extensions.split('|').any(|e| e.trim() == "*") {
            return Ok(String::new());
        }
        Ok(extensions
            .split('|')
            .map(str::trim)
            .filter(|e| !e.is_empty())
            .map(|e| format!(".{}", e.trim_start_matches('.')))
            .collect::<Vec<_>>()
            .join(","))
    }
}

/// ChatChannelTitle: the channel's icon (its emoji, else the chat icon in
/// the category's color, locked for a restricted category) and its name.
fn channel_title(c: &Value, emoji_set: &str, base_path: &str) -> String {
    let icon = match c["emoji"].as_str() {
        Some(emoji) => emoji_img(emoji_set, base_path, emoji),
        None => d_icon("d-chat", None),
    };
    let lock = if c["chatable"]["read_restricted"] == true {
        d_icon("lock", Some("chat-channel-icon__restricted-category-icon"))
    } else {
        String::new()
    };
    format!(
        "<div class=\"chat-channel-title is-category\"><div class=\"chat-channel-icon --icon\" style=\"color: #{}\">{icon}{lock}</div><div class=\"chat-channel-name\"><div class=\"chat-channel-name__label\">{}</div></div></div>",
        escape(s(&c["chatable"]["color"])),
        escape(s(&c["unicode_title"]))
    )
}

/// dReplaceEmoji(`:emoji:`) for a standard emoji.
fn emoji_img(set: &str, base_path: &str, name: &str) -> String {
    format!(
        "<img width=\"20\" height=\"20\" src=\"{base_path}/images/emoji/{set}/{}.png?v={}\" title=\"{n}\" alt=\"{n}\" class=\"emoji\">",
        escape(name),
        crate::emoji::image_version(),
        n = escape(name)
    )
}

/// ChatChannelStatus, long: a closed, read only or archived channel says
/// so above its messages.
fn channel_status(c: &Value, t: &dyn Fn(&str) -> String) -> String {
    let status = s(&c["status"]);
    let icon = match status {
        "closed" => "lock",
        "read_only" => "comment-slash",
        "archived" => "box-archive",
        _ => return String::new(),
    };
    format!(
        "<div class=\"chat-channel-status\">{}<span>{}</span></div>",
        d_icon(icon, None),
        escape(&t(&format!("chat.channel_status.{status}_header")))
    )
}

/// ChatChannelPreviewCard for a member who doesn't follow the channel.
fn preview_card(c: &Value, t: &dyn Fn(&str) -> String) -> String {
    let channel_name = format!("#{}", s(&c["title"]));
    if c["status"] == "open" && c["meta"]["can_join_chat_channel"] == true {
        format!(
            "<div class=\"chat-channel-preview-card --logged-in\"><div class=\"chat-channel-preview-card__icon\">{}</div><div class=\"chat-channel-preview-card__title\">{}</div><div class=\"chat-channel-preview-card__body\">{}</div><div class=\"chat-channel-preview-card__actions\"><button class=\"btn btn-text toggle-channel-membership-button -join btn-primary\" title=\"{}\" type=\"button\"><span class=\"d-button-label\">{}</span></button></div></div>",
            d_icon("lock", None),
            escape(
                &t("chat.channel.preview_card.guest_title")
                    .replace("%{channelName}", &channel_name)
            ),
            escape(&t("chat.channel.preview_card.join_body")),
            escape(&t("chat.channel_settings.join_channel")),
            escape(&t("chat.channel_settings.join"))
        )
    } else {
        format!(
            "<div class=\"chat-channel-preview-card --logged-in --no-access\"><div class=\"chat-channel-preview-card__icon\">{}</div><div class=\"chat-channel-preview-card__title\">{}</div><div class=\"chat-channel-preview-card__body\">{}</div></div>",
            d_icon("lock", None),
            escape(
                &t("chat.channel.preview_card.no_access_title")
                    .replace("%{channelName}", &channel_name)
            ),
            escape(&t("chat.channel.preview_card.no_access_body"))
        )
    }
}

/// ChatMessage#hideUserInfo
fn hide_user_info(m: &Value, previous: Option<&Value>, first_of_results: bool) -> bool {
    if m["is_action"] == true {
        return true;
    }
    if m["pinned"] == true {
        return false;
    }
    let Some(p) = previous else {
        return false;
    };
    if first_of_results || !m["chat_webhook_event"].is_null() {
        return false;
    }
    if !p["deleted_at"].is_null() || p["is_action"] == true {
        return false;
    }
    let at = |v: &Value| {
        chrono::DateTime::parse_from_rfc3339(s(&v["created_at"]))
            .map(|d| d.timestamp_millis())
            .unwrap_or(0)
    };
    if (at(m) - at(p)).abs() > 300_000 {
        return false;
    }
    let same_user = m["user"]["id"] == p["user"]["id"];
    if !m["in_reply_to"].is_null() {
        return m["in_reply_to"]["id"] == p["id"] && same_user;
    }
    same_user
}

/// The Message component for one message of the channel.
fn message_html(
    v: &View,
    messages: &[Value],
    index: usize,
    newest: Option<i64>,
    last_read: Option<i64>,
) -> Result<String, AppError> {
    let t = v.t;
    let m = &messages[index];
    let previous = index.checked_sub(1).map(|i| &messages[i]);
    let first_of_results = index == 0;
    let id = m["id"].as_i64().unwrap_or(0);
    let user = &m["user"];
    let hide_info = hide_user_info(m, previous, first_of_results);
    // hideReplyToInfo: a reply to the message right above says nothing.
    let has_reply_line =
        !m["in_reply_to"].is_null() && previous.is_none_or(|p| m["in_reply_to"]["id"] != p["id"]);
    let deleted = !m["deleted_at"].is_null();

    let mut classes = vec!["chat-message-container"];
    if user["id"].as_i64().is_some_and(|id| id < 0) {
        classes.push("is-bot");
    }
    if user["id"].as_i64() == Some(v.viewer_id) {
        classes.push("is-by-current-user");
    }
    if Some(id) == last_read {
        classes.push("-last-read");
    }
    classes.push("-persisted");
    classes.push("-processed");
    if m.get("bookmark").is_some() {
        classes.push("-bookmarked");
    }
    if deleted {
        classes.push("-deleted");
    }
    if hide_info {
        classes.push("-user-info-hidden");
    }
    if has_reply_line {
        classes.push("has-reply");
    }
    let mut out = format!(
        "<div class=\"{}\" data-id=\"{id}\" data-created-at=\"{}\"{}{}>",
        classes.join(" "),
        escape(s(&m["created_at"])),
        if Some(id) == newest {
            " data-newest=\"\""
        } else {
            ""
        },
        if hide_info {
            " data-user-info-hidden=\"\""
        } else {
            ""
        }
    );

    // A deleted message is collapsed to a label counting the deleted ones
    // above it.
    if deleted {
        let count = 1 + messages[..index]
            .iter()
            .rev()
            .take_while(|p| !p["deleted_at"].is_null())
            .count();
        let key = if count == 1 {
            "chat.deleted.one"
        } else {
            "chat.deleted.other"
        };
        out.push_str(&format!(
            "<div class=\"chat-message-text -deleted\"><button class=\"btn btn-text btn-flat chat-message-expand\" type=\"button\"><span class=\"d-button-label\">{}</span></button></div></div>",
            escape(&t(key).replace("%{count}", &count.to_string()))
        ));
        return Ok(out);
    }

    out.push_str("<div class=\"chat-message\">");
    if has_reply_line {
        let reply = &m["in_reply_to"];
        let slug = s(&v.channel["slug"]);
        out.push_str(&format!(
            "<a class=\"chat-reply is-direct-reply\" href=\"{}/chat/c/{}/{}/{}\"><span class=\"svg-icon-title\" title=\"{}\">{}</span>{}<span class=\"chat-reply__excerpt\">{}</span></a>",
            v.base_path,
            escape(slug),
            v.channel["id"],
            reply["id"],
            escape(&t("chat.in_reply_to")),
            d_icon("share", None),
            user_avatar(&reply["user"], 24, false, v),
            s(&reply["excerpt"])
        ));
    }
    let date = chat_date(v, m, hide_info);
    if hide_info {
        out.push_str(&format!(
            "<div class=\"chat-message-left-gutter\"><span class=\"chat-message-left-gutter__date\">{date}</span>{}</div>",
            bookmark_icon(m, t, "chat-message-left-gutter__bookmark")
        ));
    } else {
        out.push_str(&format!(
            "<div class=\"chat-message-avatar\">{}</div>",
            user_avatar(user, 48, true, v)
        ));
    }
    out.push_str("<div class=\"chat-message-content\">");
    if !hide_info {
        out.push_str(&message_info(m, &date, v));
    } else if m["is_action"] != true && user["username"].is_string() {
        out.push_str(&format!(
            "<div class=\"chat-message-info -author-only sr-only\"><button class=\"chat-message-info__username__name\" data-user-card=\"{u}\" type=\"button\">{u}</button></div>",
            u = escape(s(&user["username"]))
        ));
    }
    if m["uploads"].as_array().is_some_and(|u| !u.is_empty()) {
        return Err(Unsupported("chat message uploads").into());
    }
    out.push_str(&format!(
        "<div class=\"chat-message-text\"><div class=\"chat-cooked\">{}</div>",
        s(&m["cooked"])
    ));
    if m["edited"] == true {
        out.push_str(&format!(
            "<span class=\"chat-message-edited\">({})</span>",
            escape(&t("chat.edited"))
        ));
    }
    if let Some(reactions) = m["reactions"].as_array().filter(|r| !r.is_empty()) {
        out.push_str("<div class=\"chat-message-reaction-list\">");
        for (index, r) in reactions.iter().enumerate() {
            out.push_str(&reaction_html(v, id, index, r)?);
        }
        if v.can_interact {
            out.push_str(&format!(
                "<button aria-expanded=\"false\" class=\"btn no-text fk-d-menu__trigger emoji-picker-trigger btn-flat react-btn chat-message-react-btn\" data-identifier=\"emoji-picker\" data-trigger=\"\" type=\"button\">{}&#8203;</button>",
                d_icon("far-face-smile", None)
            ));
        }
        out.push_str("</div>");
    }
    out.push_str("</div></div></div></div>");
    Ok(out)
}

/// ChatUserAvatar: the avatar linking to the profile, the viewer's own
/// marked online (they are in chat, so in its presence channel).
fn user_avatar(user: &Value, size: u32, aria_hidden: bool, v: &View) -> String {
    let username = s(&user["username"]);
    let online = user["id"].as_i64() == Some(v.viewer_id);
    let img = avatar(user, size);
    let hidden = if aria_hidden {
        " aria-hidden=\"true\""
    } else {
        ""
    };
    let tabindex = if aria_hidden { " tabindex=\"-1\"" } else { "" };
    format!(
        "<div class=\"chat-user-avatar{}\" data-username=\"{u}\"><a{hidden} class=\"chat-user-avatar__container\" data-user-card=\"{u}\" href=\"{}/u/{u}\"{tabindex}>{img}</a></div>",
        if online { " is-online" } else { "" },
        v.base_path,
        u = escape(username)
    )
}

/// formatChatDate: the time linking to the message, drawn in UTC here
/// and in the member's zone by chat.js.
fn chat_date(v: &View, m: &Value, tiny: bool) -> String {
    let at = chrono::DateTime::parse_from_rfc3339(s(&m["created_at"]))
        .map(|d| d.with_timezone(&chrono::Utc))
        .ok();
    let format = |key: &str| {
        at.and_then(|at| crate::pretty_text::render::local_dates::format_utc(at, &(v.t)(key)).ok())
            .unwrap_or_default()
    };
    format!(
        "<a title=\"{}\" tabindex=\"-1\" class=\"chat-time\" href=\"{}/chat/c/-/{}/{}\" data-mode=\"{}\">{}</a>",
        escape(&format("dates.long_with_year")),
        v.base_path,
        v.channel["id"],
        m["id"],
        if tiny { "tiny" } else { "time" },
        escape(&format(if tiny {
            "dates.time_short"
        } else {
            "dates.time"
        }))
    )
}

/// BookmarkIcon for a bookmarked message.
fn bookmark_icon(m: &Value, t: &dyn Fn(&str) -> String, class: &str) -> String {
    let Some(bookmark) = m.get("bookmark") else {
        return String::new();
    };
    let name = s(&bookmark["name"]);
    let title = if name.is_empty() {
        t("bookmarks.created")
    } else {
        t("bookmarks.created_with_name").replace("%{name}", name)
    };
    format!(
        "<span class=\"{class}\"><span class=\"svg-icon-title\" title=\"{}\">{}</span></span>",
        escape(&title),
        d_icon("bookmark", Some("bookmark-icon bookmark-icon__bookmarked"))
    )
}

/// ChatMessageInfo: the author (with their roles as classes), the time,
/// the bookmark and the pin.
fn message_info(m: &Value, date: &str, v: &View) -> String {
    let user = &m["user"];
    let mut classes = vec![
        "chat-message-info__username".to_string(),
        "is-username".into(),
    ];
    if user["staff"] == true {
        classes.push("is-staff".into());
    }
    if user["admin"] == true {
        classes.push("is-admin".into());
    }
    if user["moderator"] == true {
        classes.push("is-moderator".into());
    }
    if user["new_user"] == true {
        classes.push("is-new-user".into());
    }
    if let Some(group) = user["primary_group_name"].as_str() {
        classes.push(format!("group--{group}"));
    }
    classes.push("clickable".into());
    let mut out = format!(
        "<div class=\"chat-message-info\"><span class=\"{}\"><button class=\"chat-message-info__username__name\" data-user-card=\"{u}\" type=\"button\">{u}</button></span><span class=\"chat-message-info__date\">{date}</span>",
        escape(&classes.join(" ")),
        u = escape(s(&user["username"]))
    );
    out.push_str(&bookmark_icon(m, v.t, "chat-message-info__bookmark"));
    if m["pinned"] == true {
        out.push_str(&format!(
            "<span class=\"chat-message-info__pinned\" title=\"{}\">{}</span>",
            escape(&(v.t)("chat.pinned")),
            d_icon("thumbtack", None)
        ));
    }
    out.push_str("</div>");
    out
}

/// ChatMessageReaction, with its screen reader description
/// (getReactionText).
fn reaction_html(v: &View, message_id: i64, index: usize, r: &Value) -> Result<String, AppError> {
    let t = v.t;
    let emoji = s(&r["emoji"]);
    if !crate::emoji::DATA.exists(emoji.trim_matches(':')) {
        return Err(Unsupported("custom emoji chat reactions").into());
    }
    let count = r["count"].as_i64().unwrap_or(0);
    let reacted = r["reacted"] == true;
    let url = format!(
        "{}/images/emoji/{}/{}.png?v={}",
        v.base_path,
        v.emoji_set,
        emoji.replace(":t", "/"),
        crate::emoji::image_version()
    );
    let description_id = format!("chat-message-reaction-description-{message_id}-{index}");
    let label = if count > 0 {
        let key = if count == 1 {
            "chat.reactions.counted.one"
        } else {
            "chat.reactions.counted.other"
        };
        t(key)
            .replace("%{emoji}", emoji)
            .replace("%{count}", &count.to_string())
    } else if reacted {
        t("chat.reactions.remove").replace("%{emoji}", emoji)
    } else {
        t("chat.reactions.add").replace("%{emoji}", emoji)
    };
    let description = reaction_text(v, r);
    let mut out = format!(
        "<button{} aria-label=\"{}\" aria-pressed=\"{}\" class=\"chat-message-reaction{}\" data-emoji-name=\"{}\" tabindex=\"0\" title=\":{e}:\" type=\"button\"><img alt=\":{e}:\" class=\"emoji\" height=\"20\" loading=\"lazy\" src=\"{}\" width=\"20\">",
        if description.is_some() {
            format!(" aria-describedby=\"{description_id}\"")
        } else {
            String::new()
        },
        escape(&label),
        reacted,
        if reacted { " reacted" } else { "" },
        escape(emoji),
        escape(&url),
        e = escape(emoji)
    );
    if count > 0 {
        out.push_str(&format!("<span class=\"count\">{count}</span>"));
    }
    out.push_str("</button>");
    if let Some(text) = description {
        out.push_str(&format!(
            "<span class=\"sr-only\" id=\"{description_id}\">{text}</span>"
        ));
    }
    Ok(out)
}

/// getReactionText, with the emoji code turned to its image.
fn reaction_text(v: &View, r: &Value) -> Option<String> {
    let t = v.t;
    let count = r["count"].as_i64().unwrap_or(0);
    let users = r["users"].as_array()?;
    if count == 0 || users.is_empty() {
        return None;
    }
    let emoji = s(&r["emoji"]);
    let mut names: Vec<String> = users
        .iter()
        .filter(|u| u["id"].as_i64() != Some(v.viewer_id))
        .take(15)
        .map(|u| escape(s(&u["username"])))
        .collect();
    let comma = t("word_connector.comma");
    let plural = |key: &str, n: i64| {
        if n == 1 {
            format!("{key}.one")
        } else {
            format!("{key}.other")
        }
    };
    let fill = |key: &str, username: &str, names: &[String], n: i64| {
        t(key)
            .replace("%{emoji}", emoji)
            .replace("%{username}", username)
            .replace("%{commaSeparatedUsernames}", &names.join(&comma))
            .replace("%{count}", &n.to_string())
    };
    let named = names.len() as i64;
    let text = if r["reacted"] == true {
        if count == 1 {
            fill("chat.reactions.only_you", "", &[], 0)
        } else if count == 2 {
            let username = names.pop().unwrap_or_default();
            fill("chat.reactions.you_and_single_user", &username, &[], 0)
        } else if count - named - 1 > 0 {
            let unnamed = count - named - 1;
            fill(
                &plural("chat.reactions.you_multiple_users_and_more", unnamed),
                "",
                &names,
                unnamed,
            )
        } else {
            let username = names.pop().unwrap_or_default();
            fill(
                "chat.reactions.you_and_multiple_users",
                &username,
                &names,
                0,
            )
        }
    } else if count == 1 {
        let username = names.pop().unwrap_or_default();
        fill("chat.reactions.single_user", &username, &[], 0)
    } else if count - named > 0 {
        let unnamed = count - named;
        fill(
            &plural("chat.reactions.multiple_users_and_more", unnamed),
            "",
            &names,
            unnamed,
        )
    } else {
        let username = names.pop().unwrap_or_default();
        fill("chat.reactions.multiple_users", &username, &names, 0)
    };
    // emojiUnescape of the `:emoji:` the text ends with.
    let code = format!(":{emoji}:");
    Some(text.replace(
        &code,
        &format!(
            "<img width=\"20\" height=\"20\" src=\"{}/images/emoji/{}/{}.png?v={}\" title=\"{e}\" alt=\"{e}\" class=\"emoji\">",
            v.base_path,
            v.emoji_set,
            emoji.replace(":t", "/"),
            crate::emoji::image_version(),
            e = escape(emoji)
        ),
    ))
}

/// The browse tabs, as `TABS` (archived only with archiving allowed).
pub const BROWSE_TABS: [&str; 4] = ["all", "open", "closed", "archived"];
/// The browse list's page (ChatApi#channels' default limit).
const BROWSE_LIMIT: i64 = 10;

impl Context<'_> {
    /// The browse route (BrowseChannels): the tabs, the filter controls
    /// and the channel cards, ten at a time (the next ten load when the
    /// loading container shows). `filter` narrows by name; the joined
    /// dropdown filters the loaded cards in the browser.
    pub async fn browse_page(&mut self, tab: &str, filter: &str) -> Result<String, AppError> {
        let i18n = self.i18n;
        let t = move |key: &str| js(i18n, key);
        let mut out = String::from(
            "<div class=\"c-routes --browse\"><div class=\"c-navbar-container\"><nav class=\"c-navbar\">",
        );
        out.push_str(&format!(
            "<a class=\"active c-navbar__back-button no-text btn-transparent btn\" title=\"{}\" href=\"{}/chat\">{}</a>",
            escape(&t("chat.browse.back")),
            self.base_path,
            d_icon("chevron-left", None)
        ));
        let title = t("chat.browse.title");
        out.push_str(&format!(
            "<div class=\"c-navbar__title\" title=\"{t}\"><span class=\"c-navbar__title-text\">{t}</span></div><nav class=\"c-navbar__actions\">",
            t = escape(&title)
        ));
        if self.guardian.is_staff() {
            out.push_str(&format!(
                "<button class=\"btn btn-icon-text c-navbar__new-channel-button btn-transparent\" type=\"button\">{}<span class=\"d-button-label\">{}</span></button>",
                d_icon("plus", None),
                escape(&t("chat.create_channel.title"))
            ));
        }
        out.push_str(&format!(
            "<button class=\"btn no-text btn-icon c-navbar__open-drawer-button btn-transparent\" title=\"{}\" type=\"button\">{}<span aria-hidden=\"true\">&#8203;</span></button></nav></nav></div>",
            escape(&t("chat.close_full_page")),
            d_icon("discourse-compress", None)
        ));

        let archiving = self.settings.get("chat_allow_archiving_channels")?.truthy();
        out.push_str("<div class=\"chat-browse-view\"><div class=\"chat-browse-view__actions\"><nav><ul class=\"nav-pills chat-browse-view__filters\">");
        for each in BROWSE_TABS
            .iter()
            .filter(|x| archiving || **x != "archived")
        {
            out.push_str(&format!(
                "<li class=\"chat-browse-view__filter -{each}\"><a class=\"{}chat-browse-view__filter-link -{each}\" href=\"{}/chat/browse/{each}\">{}</a></li>",
                if *each == tab { "active " } else { "" },
                self.base_path,
                escape(&t(&format!("chat.browse.filter_{each}")))
            ));
        }
        let joined = [
            ("all", t("chat.browse.filter_joined_all")),
            ("joined", t("chat.browse.filter_joined_joined")),
            ("not-joined", t("chat.browse.filter_joined_not_joined")),
        ];
        out.push_str(&format!(
            "</ul></nav><div class=\"d-filter-controls\"><div class=\"d-filter-controls__inputs\"><div class=\"filter-input-container\">{}<input autocapitalize=\"none\" autocomplete=\"off\" autocorrect=\"off\" class=\"filter-input d-filter-controls__input\" name=\"filter\" value=\"{}\" placeholder=\"{}\" type=\"text\" hx-get=\"{}/chat/browse/{tab}\" hx-trigger=\"input changed delay:400ms\" hx-target=\".chat-browse-view__content\" hx-select=\".chat-browse-view__content\" hx-swap=\"outerHTML\" hx-push-url=\"false\"></div></div>",
            d_icon("magnifying-glass", Some("-left")),
            escape(filter),
            escape(&t("chat.browse.filter_input_placeholder")),
            self.base_path
        ));
        out.push_str(&format!(
            "<div class=\"d-filter-controls__dropdowns\"><select aria-label=\"{}\" class=\"d-filter-controls__dropdown d-native-select\">",
            escape(&joined[0].1)
        ));
        for (index, (value, label)) in joined.iter().enumerate() {
            out.push_str(&format!(
                "<option class=\"d-native-select__option{}\" value=\"{value}\">{}</option>",
                if index == 0 { " --selected" } else { "" },
                escape(label)
            ));
        }
        out.push_str("</select></div></div></div>");
        out.push_str("<div class=\"chat-browse-view__content_wrapper\"><div class=\"chat-browse-view__content\"><div class=\"c-list\">");
        out.push_str(&self.browse_cards(tab, filter, 0).await?);
        out.push_str("</div></div></div></div></div>");
        Ok(out)
    }

    /// A page of the browse list: its cards (or the empty state for the
    /// first), and the container that loads the next page when shown.
    pub async fn browse_cards(
        &mut self,
        tab: &str,
        filter: &str,
        offset: i64,
    ) -> Result<String, AppError> {
        let i18n = self.i18n;
        let t = move |key: &str| js(i18n, key);
        let search = super::channels::Search {
            filter: Some(filter.to_string()),
            status: super::channels::STATUSES
                .iter()
                .position(|x| *x == tab)
                .map(|i| i as i32),
            limit: Some(BROWSE_LIMIT),
            offset,
            ..Default::default()
        };
        let channels = self.index(&search).await?;
        let mut out = String::new();
        if offset == 0 {
            out.push_str("<div class=\"--loaded chat-browse-view__cards\">");
        }
        if channels.is_empty() && offset == 0 {
            out.push_str(&format!(
                "<div class=\"empty-state__container --text-only\"><div class=\"empty-state\"><div class=\"empty-state__title\" data-test-title=\"\">{}</div><div class=\"empty-state__body\"><p data-test-body=\"\">{}</p></div><div class=\"empty-state__cta\"><button class=\"btn btn-text btn-primary\" type=\"button\"><span class=\"d-button-label\">{}</span></button></div></div></div>",
                escape(&t("chat.empty_state.title")),
                escape(&t("chat.empty_state.direct_message")),
                escape(&t("chat.empty_state.direct_message_cta"))
            ));
        }
        let emoji_set = self.emoji_set()?;
        for c in &channels {
            out.push_str(&channel_card(c, &emoji_set, self.base_path, &t));
        }
        if offset == 0 {
            out.push_str("</div><div><br></div>");
        }
        if channels.len() as i64 == BROWSE_LIMIT {
            out.push_str(&format!(
                "<div class=\"loading-container\" hx-get=\"{}/chat/browse/{tab}/page?filter={}&offset={}\" hx-trigger=\"revealed\" hx-swap=\"outerHTML\"></div>",
                self.base_path,
                crate::ruby::cgi_escape(filter),
                offset + BROWSE_LIMIT
            ));
        } else if offset == 0 {
            out.push_str("<div class=\"loading-container\"></div>");
        }
        Ok(out)
    }
}

/// ChatChannelCard
fn channel_card(c: &Value, emoji_set: &str, base_path: &str, t: &dyn Fn(&str) -> String) -> String {
    let id = c["id"].as_i64().unwrap_or(0);
    let slug = escape(s(&c["slug"]));
    let status = s(&c["status"]);
    let following = c["current_user_membership"]["following"] == true;
    let mut out = format!(
        "<div class=\"chat-channel-card{}{}\" data-channel-id=\"{id}\" data-following=\"{following}\" style=\"--chat-channel-card-border: #{}\"><div class=\"chat-channel-card__header\"><a class=\"chat-channel-card__name-container\" href=\"{base_path}/chat/c/{slug}/{id}\"><span class=\"chat-channel-card__name\">",
        if status == "closed" { " --closed" } else { "" },
        if status == "archived" {
            " --archived"
        } else {
            ""
        },
        escape(s(&c["chatable"]["color"]))
    );
    if let Some(emoji) = c["emoji"].as_str() {
        out.push_str(&emoji_img(emoji_set, base_path, emoji));
    }
    out.push_str(&escape(s(&c["title"])));
    out.push_str("</span>");
    if c["chatable"]["read_restricted"] == true {
        out.push_str(&d_icon("lock", Some("chat-channel-card__read-restricted")));
    }
    if c["current_user_membership"]["muted"] == true {
        out.push_str(&format!(
            "<span aria-label=\"{m}\" class=\"chat-channel-card__muted\" title=\"{m}\">{}</span>",
            d_icon("d-muted", None),
            m = escape(&t("chat.muted"))
        ));
    }
    out.push_str("</a></div><div class=\"chat-channel-card__cta\">");
    if following {
        out.push_str(&format!(
            "<button class=\"btn toggle-channel-membership-button -leave btn-default btn-small chat-channel-card__leave-btn\" title=\"{}\" type=\"button\"><span class=\"d-button-label\">{}</span></button>",
            escape(&t("chat.channel_settings.leave_channel")),
            escape(&t("chat.channel_settings.leave"))
        ));
    } else if status == "open" {
        out.push_str(&format!(
            "<button class=\"btn toggle-channel-membership-button -join btn-primary btn-small chat-channel-card__join-btn\" title=\"{}\" type=\"button\"><span class=\"d-button-label\">{}</span></button>",
            escape(&t("chat.channel_settings.join_channel")),
            escape(&t("chat.channel_settings.join"))
        ));
    }
    out.push_str("</div>");
    let members = c["memberships_count"].as_i64().unwrap_or(0);
    if members > 0 {
        let key = if members == 1 {
            "chat.channel.memberships_count.one"
        } else {
            "chat.channel.memberships_count.other"
        };
        out.push_str(&format!(
            "<a class=\"chat-channel-card__members\" href=\"{base_path}/chat/c/{slug}/{id}/info/members\">{}</a>",
            escape(&t(key).replace("%{count}", &members.to_string()))
        ));
    }
    if let Some(description) = c["description"].as_str() {
        out.push_str(&format!(
            "<div class=\"chat-channel-card__description\">{}</div>",
            escape(description)
        ));
    }
    out.push_str("</div>");
    out
}

/// ChatZero, the empty chat illustration.
const CHAT_ZERO: &str = include_str!("../../../static/svg/chat-zero.svg");

/// The chat.disabled route (ChatDisabled): chat turned off in the
/// member's preferences, with the way back on.
pub fn disabled_page(i18n: &crate::i18n::I18n, base_path: &str) -> String {
    format!(
        "<div class=\"chat-disabled\"><div class=\"empty-state__container --chat-disabled --with-image\"><div class=\"empty-state\"><div class=\"empty-state__image\">{CHAT_ZERO}</div><div class=\"empty-state__title\" data-test-title=\"\">{}</div><div class=\"empty-state__body\"><p data-test-body=\"\">{}</p></div><div class=\"empty-state__cta\"><a class=\"btn btn-icon-text btn-primary\" href=\"{base_path}/my/preferences/chat\">{}<span class=\"d-button-label\">{}</span></a></div></div></div></div>",
        escape(&js(i18n, "chat.disabled.title")),
        escape(&js(i18n, "chat.disabled.body")),
        d_icon("gear", None),
        escape(&js(i18n, "chat.disabled.cta"))
    )
}
