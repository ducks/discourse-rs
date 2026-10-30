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

use super::channels::{ChannelRow, Context, Found};
use super::messages::{ListParams, Listed};
use crate::post_view::d_icon;
use crate::topic_list_view::escape;
use crate::{AppError, Unsupported};

/// `DEFAULT_MESSAGE_PAGE_SIZE`
const PAGE_SIZE: i64 = 50;

/// Where the mobile channel route's back button takes its unread indicator,
/// which the page's chrome knows (routes::chat_pages).
pub const BACK_INDICATOR: &str = "<!--chat-back-indicator-->";
/// The empty state's facepile (MAX_AVATARS).
const MAX_AVATARS: i64 = 5;

pub(super) fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}

/// `renderAvatar(user, { imageSize })`
pub(super) fn avatar(user: &Value, size: u32) -> String {
    format!(
        "<img alt=\"\" width=\"{size}\" height=\"{size}\" src=\"{}\" class=\"avatar\" title=\"{}\">",
        escape(&s(&user["avatar_template"]).replace("{size}", &size.to_string())),
        escape(s(&user["username"]))
    )
}

/// A client string (`js.` keys).
pub(super) fn js(i18n: &crate::i18n::I18n, key: &str) -> String {
    i18n.t(&format!("js.{key}")).unwrap_or(key).to_string()
}

/// A page of messages for chat.js (Context::messages_page), with the
/// meta's can_load_more_past and _future as the API answers them (null
/// for the direction a page wasn't loaded in).
pub struct MessagesPage {
    pub html: String,
    pub can_load_more_past: Value,
    pub can_load_more_future: Value,
}

/// What every message in the page reads.
pub(super) struct View<'a> {
    pub(super) base_path: &'a str,
    pub(super) channel: &'a Value,
    pub(super) viewer_id: i64,
    /// Who is in /chat/online, drawn online (chat.js follows it after).
    pub(super) online: Vec<i64>,
    pub(super) emojis: std::sync::Arc<crate::emojis::Emojis>,
    /// max_image_width and max_image_height, which ChatUpload shrinks to.
    pub(super) max_image: (f64, f64),
    /// The site's base url, which an element's href and src resolve
    /// against in the browser.
    pub(super) base_url: String,
    /// `userCanInteractWithChat`: not silenced.
    pub(super) can_interact: bool,
    /// ChatMessageInfo#prioritizeName: display_name_on_posts with names
    /// shown first (enable_names, not prioritize_username_in_ux).
    pub(super) prioritize_name: bool,
    /// The viewer is an admin (the stop streaming button).
    pub(super) admin: bool,
    pub(super) t: &'a (dyn Fn(&str) -> String + Sync),
    pub(super) i18n: &'a crate::i18n::I18n,
    /// The relative_date_duration site setting, for dFormatDate.
    pub(super) relative_date_duration: i64,
    /// The message context: a thread's pane draws no thread indicators
    /// or reply lines (`MESSAGE_CONTEXT_THREAD`).
    pub(super) thread_context: bool,
}

impl Context<'_> {
    /// The members in /chat/online (PresenceChannel#user_ids).
    pub(crate) async fn online_ids(&mut self) -> Result<Vec<i64>, AppError> {
        Ok(sqlx::query_scalar(
            "SELECT DISTINCT user_id::bigint FROM discourse_rs.presence_clients WHERE channel = '/chat/online'",
        )
        .fetch_all(&mut *self.conn)
        .await?)
    }

    /// The chat page around a route's content (templates/chat.gjs with the
    /// core sidebar).
    /// `chat_view`: mainOutletModifierClasses' chat-view, off for the
    /// browse and channel info routes.
    /// `hide_presence`: the member's choice to stay out of /chat/online,
    /// for chat.js.
    pub fn full_page(
        content: &str,
        base_path: &str,
        chat_view: bool,
        hide_presence: bool,
    ) -> String {
        format!(
            "<div id=\"chat-progress-bar-container\"></div><div class=\"full-page-chat full-page-chat-sidebar-enabled\" data-base-path=\"{}\"{}><div class=\"main-chat-outlet{}{}\" id=\"main-chat-outlet\">{content}</div></div>",
            escape(base_path),
            if hide_presence {
                " data-hide-presence=\"true\""
            } else {
                ""
            },
            // mainOutletModifierClasses: has-side-panel-expanded while the
            // side panel shows (a thread, a channel's threads).
            if content.contains("<div class=\"chat-side-panel\">") {
                " has-side-panel-expanded"
            } else {
                ""
            },
            if chat_view { " chat-view" } else { "" }
        )
    }

    /// The channel route: the navbar and the channel, `target` the message
    /// a near-message url names. In the drawer (`drawer`), its channel
    /// route (ChatDrawerRoutesChannel): the drawer's navbar, and the
    /// channel only while the drawer is expanded.
    pub async fn channel_page(
        &mut self,
        channel: &ChannelRow,
        target: Option<i64>,
        drawer: Option<&super::drawer::Nav<'_>>,
    ) -> Result<String, AppError> {
        self.channel_page_beside(channel, target, drawer, false)
            .await
    }

    /// The channel route, `beside_thread` while a thread or the threads
    /// list shows in the side panel (the threads list button hides).
    pub async fn channel_page_beside(
        &mut self,
        channel: &ChannelRow,
        target: Option<i64>,
        drawer: Option<&super::drawer::Nav<'_>>,
        beside_thread: bool,
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
        if let Some(nav) = drawer.filter(|nav| !nav.expanded) {
            let online = self.online_ids().await?;
            return Ok(format!(
                "<div class=\"c-drawer-routes --channel\">{}</div>",
                self.navbar(&channel_json, following, &online, Some(nav), None)?
            ));
        }

        // ChatChannel#loadMessages: around the target, else from the last
        // read message.
        let params = ListParams {
            page_size: PAGE_SIZE,
            target_message_id: target,
            direction: None,
            fetch_from_last_read: target.is_none(),
            target_date: None,
        };
        // A target that can't load (the request's 404) leaves the channel
        // unloaded: fetchedOnce stays false.
        let listed = match self.list_messages(channel.id, &params).await? {
            Listed::Messages(v) => Some(v),
            Listed::NotFound | Listed::Forbidden => None,
        };
        let loaded = listed.is_some();
        let listed = listed.unwrap_or_default();
        let messages = listed["messages"].as_array().cloned().unwrap_or_default();

        let i18n = self.i18n;
        let t = move |key: &str| js(i18n, key);
        let viewer_id = i64::from(self.guardian.user_id().unwrap_or(0));
        let view = View {
            base_path: self.base_path,
            channel: &channel_json,
            viewer_id,
            online: self.online_ids().await?,
            emojis: self.settings.emojis().clone(),
            max_image: (
                self.settings.get("max_image_width")?.to_f(),
                self.settings.get("max_image_height")?.to_f(),
            ),
            base_url: crate::url::Urls {
                config: self.config,
                settings: self.settings,
            }
            .base_url()?,
            can_interact: channel_json["meta"]["user_silenced"] != true,
            prioritize_name: prioritize_name(self.settings)?,
            admin: self.guardian.is_admin(),
            t: &t,
            i18n: self.i18n,
            relative_date_duration: self.settings.get("relative_date_duration")?.to_i(),
            thread_context: false,
        };

        let mut out = String::from(if drawer.is_some() {
            "<div class=\"c-drawer-routes --channel\">"
        } else {
            "<div class=\"c-routes --channel\">"
        });
        // The channel's unread threads (threadsManager.unreadThreadCount).
        let threads_button = if beside_thread {
            None
        } else {
            Some(self.unread_thread_count(channel).await?)
        };
        out.push_str(&self.navbar(
            &channel_json,
            following,
            &view.online,
            drawer,
            threads_button,
        )?);
        if drawer.is_some() {
            out.push_str("<div class=\"chat-drawer-content\">");
        }

        let mut classes = String::from(if loaded {
            "chat-channel --loaded"
        } else {
            "chat-channel"
        });
        if loaded && messages.is_empty() {
            classes.push_str(" is-empty");
        }
        let pane = self.pane_attrs(&listed, target, loaded);
        out.push_str(&format!(
            "<div class=\"{classes}\" data-id=\"{}\" data-label-today=\"{}\" data-label-yesterday=\"{}\" data-label-last-visit=\"{}\" data-format-time=\"{}\" data-format-tiny=\"{}\" data-format-title=\"{}\" data-format-date=\"{}\" data-last-read=\"{}\" data-following=\"{}\" data-viewer-id=\"{}\" data-staff=\"{}\" data-can-moderate=\"{}\" data-label-deleted-one=\"{}\" data-label-deleted-other=\"{}\" data-direct-message=\"{}\"{pane}>",
            channel.id,
            escape(&t("chat.chat_message_separator.today")),
            escape(&t("chat.chat_message_separator.yesterday")),
            escape(&t("chat.last_visit")),
            escape(&t("dates.time")),
            escape(&t("dates.time_short")),
            escape(&t("dates.long_with_year")),
            // moment's LL, in English.
            "MMMM D, YYYY",
            last_read.unwrap_or(0),
            following,
            viewer_id,
            self.guardian.is_staff(),
            channel_json["meta"]["can_moderate"] == true,
            escape(&t("chat.deleted.one")),
            escape(&t("chat.deleted.other")),
            channel.chatable_type == "DirectMessage",
        ));
        out.push_str(&channel_status(
            &channel_json,
            &t,
            self.guardian.is_admin(),
            self.base_path,
        ));
        out.push_str(&format!(
            "<div class=\"chat-notices\">{}</div>",
            self.retention_reminder(&channel_json).await?
        ));
        out.push_str(
            "<div class=\"chat-messages-scroller\"><div class=\"chat-messages-container\">",
        );
        if !loaded {
            // ChatSkeleton, as fetchedOnce stays false; chat.js pops the
            // request's error over it (popupAjaxError).
            out.push_str(&skeleton());
        } else if messages.is_empty() {
            out.push_str(&self.empty_state(channel, &channel_json, following).await?);
        } else {
            let newest = newest_id(&channel_json, &messages);
            out.push_str(&messages_html(
                &view,
                &messages,
                0..messages.len(),
                newest,
                last_read,
            )?);
        }
        out.push_str("</div>");
        if !messages.is_empty() && listed["meta"]["can_load_more_past"] == false {
            out.push_str(&format!(
                "<div class=\"all-loaded-message\">{}</div>",
                escape(&t("chat.all_loaded"))
            ));
        }
        out.push_str("</div>");
        // ChatMessageActionsDesktop, for members who can interact
        // (userCanInteractWithChat).
        if viewer_id > 0 && view.can_interact {
            let custom = match self.guardian.user_id() {
                Some(id) => super::options(&mut *self.conn, id)
                    .await?
                    .filter(|o| o.chat_quick_reaction_type == 1)
                    .and_then(|o| o.chat_quick_reactions_custom),
                None => None,
            };
            let defaults = self
                .settings
                .get("default_emoji_reactions")?
                .to_s()
                .to_string();
            let pins = self.settings.get("chat_pinned_messages")?.truthy();
            let actions = {
                let toolbar = super::actions_view::Toolbar {
                    t: &t,
                    base_path: self.base_path,
                    emojis: &view.emojis,
                    viewer_id,
                    staff: self.guardian.is_staff(),
                    quick_custom: custom.as_deref(),
                    default_reactions: &defaults,
                    pins,
                };
                super::actions_view::template(&toolbar, &channel_json, following)
            };
            out.push_str(&actions);
            out.push_str(&self.staged_templates(&view).await?);
            // The flag modal ChatMessageFlag opens (static/js/flag-modal.js,
            // which chat.js loads from data-src when a flag is asked for).
            if channel_json["meta"]["can_flag"] == true {
                out.push_str(&format!(
                    "<template class=\"chat-flag-modal\" data-src=\"{}\"></template>",
                    escape(&crate::assets::url(self.base_path, "flag-modal.js"))
                ));
                out.push_str(
                    &crate::flag_modal_view::chat_config(
                        &mut *self.conn,
                        self.i18n,
                        self.settings,
                        self.base_path,
                        self.guardian.is_staff(),
                    )
                    .await?,
                );
            }
            out.push_str(&self.mention_warnings_template(&channel_json)?);
        }
        out.push_str(&format!(
            "<div class=\"chat-scroll-to-bottom\"><button class=\"btn no-text btn-flat chat-scroll-to-bottom__button\" type=\"button\"><span class=\"chat-scroll-to-bottom__arrow\">{}</span></button></div>",
            d_icon("arrow-down", None)
        ));
        // The preview card is a category channel's; a DM always has the
        // composer (chat.js follows it on opening).
        if !following && channel.chatable_type != "DirectMessage" {
            out.push_str(&preview_card(&channel_json, &t));
        } else {
            out.push_str(&self.composer(channel, &channel_json).await?);
        }
        out.push_str("</div></div>");
        if drawer.is_some() {
            out.push_str("</div>");
        }
        Ok(out)
    }

    /// The member's own message as ChatChannel#stageMessage shows it before
    /// the server has it, for chat.js to fill (the cooked text, an id):
    /// with its author line, and without (hideUserInfo, after their own
    /// message), as Ember draws a message by currentUser.
    async fn staged_templates(&mut self, view: &View<'_>) -> Result<String, AppError> {
        let Some(user_id) = self.guardian.user_id() else {
            return Ok(String::new());
        };
        #[derive(sqlx::FromRow)]
        struct Me {
            username: String,
            name: Option<String>,
            uploaded_avatar_id: Option<i32>,
            admin: bool,
            moderator: bool,
            primary_group_name: Option<String>,
        }
        let me: Me = sqlx::query_as(
            "SELECT users.username, users.name, users.uploaded_avatar_id, users.admin, users.moderator, \
               groups.name AS primary_group_name \
             FROM users LEFT JOIN groups ON groups.id = users.primary_group_id WHERE users.id = $1",
        )
        .bind(user_id)
        .fetch_one(&mut *self.conn)
        .await?;
        let logo = crate::admin_users::logo_small_url(&mut *self.conn, self.settings).await?;
        let urls = crate::url::Urls {
            config: self.config,
            settings: self.settings,
        };
        let avatar = crate::avatar::avatar_template(
            &urls,
            user_id,
            &me.username,
            me.uploaded_avatar_id,
            logo.as_deref(),
        )?;
        let now = crate::topic_list::time_json(crate::clock::now_naive());
        let message = |id: i64| {
            serde_json::json!({
                "id": id,
                "message": "",
                "cooked": "",
                "excerpt": "",
                "created_at": now,
                "deleted_at": null,
                "in_reply_to": null,
                "chat_webhook_event": null,
                "user": {
                    "id": user_id,
                    "username": me.username,
                    "name": me.name,
                    "avatar_template": avatar,
                    "admin": me.admin,
                    "moderator": me.moderator,
                    "staff": me.admin || me.moderator,
                    "primary_group_name": me.primary_group_name,
                },
            })
        };
        let alone = [message(0)];
        let after_own = [message(-1), message(0)];
        Ok(format!(
            "<template class=\"chat-staged-template\" data-variant=\"info\" data-wasm=\"{}/assets/markdown.wasm?v={}\" data-settings=\"{}/assets/markdown-settings.json?chat=1\" data-label-network-error=\"{}\" data-label-send-again=\"{}\"><template class=\"chat-retry-icon\">{}</template>{}</template><template class=\"chat-staged-template\" data-variant=\"hidden\">{}</template>",
            self.base_path,
            crate::routes::composer::MARKDOWN_WASM_VERSION,
            self.base_path,
            escape(&(view.t)("chat.retry_staged_message.title")),
            escape(&(view.t)("chat.retry_staged_message.action")),
            d_icon("circle-exclamation", None),
            message_html(view, &alone, 0, None, None, false)?,
            message_html(view, &after_own, 1, None, None, false)?
        ))
    }

    /// One message as the channel page draws it, after `previous` (the
    /// message above it in the viewer's page, which decides what it
    /// repeats of its author and reply): what chat.js swaps in when the
    /// channel's bus says a message came, changed or came back. A deleted
    /// message, for those who see it (its author, staff, moderators), is
    /// drawn expanded. `staged_previous`: the message above is the viewer's
    /// own still staged. None when the viewer can't see it.
    pub async fn message_fragment(
        &mut self,
        channel_id: i64,
        message_id: i64,
        previous: Option<i64>,
        staged_previous: bool,
    ) -> Result<Option<String>, AppError> {
        self.message_fragment_in(channel_id, message_id, previous, staged_previous, false)
            .await
    }

    /// message_fragment in a thread's pane (`thread_context`): without
    /// thread indicators or reply lines.
    pub async fn message_fragment_in(
        &mut self,
        channel_id: i64,
        message_id: i64,
        previous: Option<i64>,
        staged_previous: bool,
        thread_context: bool,
    ) -> Result<Option<String>, AppError> {
        let sql = format!(
            "SELECT {} FROM chat_channels WHERE id = $1 AND deleted_at IS NULL",
            super::channels::CHANNEL_COLUMNS
        );
        let channel: Option<ChannelRow> = sqlx::query_as(&sql)
            .bind(channel_id)
            .fetch_optional(&mut *self.conn)
            .await?;
        let Some(channel) = channel else {
            return Ok(None);
        };
        if !self.can_preview(&channel).await? {
            return Ok(None);
        }
        let found: Option<(Option<i32>, bool)> = sqlx::query_as(
            "SELECT user_id, deleted_at IS NOT NULL FROM chat_messages WHERE id = $1 AND chat_channel_id = $2",
        )
        .bind(message_id)
        .bind(channel.id)
        .fetch_optional(&mut *self.conn)
        .await?;
        let Some((author, deleted)) = found else {
            return Ok(None);
        };
        let viewer = self.guardian.user_id();
        if deleted
            && !(author.is_some() && author == viewer)
            && !self.guardian.is_staff()
            && !self.can_moderate(&channel).await?
        {
            return Ok(None);
        }
        let previous: Option<i64> = match previous {
            Some(id) => {
                sqlx::query_scalar(
                    "SELECT id FROM chat_messages WHERE id = $1 AND chat_channel_id = $2",
                )
                .bind(id)
                .bind(channel.id)
                .fetch_optional(&mut *self.conn)
                .await?
            }
            None => None,
        };
        let membership = self
            .memberships()
            .await?
            .into_iter()
            .find(|m| m.chat_channel_id == channel.id);
        let channel_json = self
            .channel(&channel, membership.as_ref(), None, None)
            .await?;
        let last_read = membership.as_ref().and_then(|m| m.last_read_message_id);
        let ids: Vec<i64> = previous.into_iter().chain([message_id]).collect();
        let serialized = self.serialize_page_messages(&channel, &ids).await?;
        // In the page's order: the previous one first.
        let mut messages: Vec<Value> = ids
            .iter()
            .filter_map(|id| {
                serialized
                    .iter()
                    .find(|m| m["id"].as_i64() == Some(*id))
                    .cloned()
            })
            .collect();
        // After the viewer's staged message: theirs, just now.
        if staged_previous && previous.is_none() {
            messages.insert(
                0,
                serde_json::json!({
                    "id": null,
                    "user": { "id": viewer },
                    "created_at": crate::topic_list::time_json(crate::clock::now_naive()),
                    "deleted_at": null,
                    "in_reply_to": null,
                }),
            );
        }
        let Some(index) = messages
            .iter()
            .position(|m| m["id"].as_i64() == Some(message_id))
        else {
            return Ok(None);
        };
        let i18n = self.i18n;
        let t = move |key: &str| js(i18n, key);
        let view = View {
            base_path: self.base_path,
            channel: &channel_json,
            viewer_id: i64::from(viewer.unwrap_or(0)),
            online: self.online_ids().await?,
            emojis: self.settings.emojis().clone(),
            max_image: (
                self.settings.get("max_image_width")?.to_f(),
                self.settings.get("max_image_height")?.to_f(),
            ),
            base_url: crate::url::Urls {
                config: self.config,
                settings: self.settings,
            }
            .base_url()?,
            can_interact: channel_json["meta"]["user_silenced"] != true,
            prioritize_name: prioritize_name(self.settings)?,
            admin: self.guardian.is_admin(),
            t: &t,
            i18n: self.i18n,
            relative_date_duration: self.settings.get("relative_date_duration")?.to_i(),
            thread_context,
        };
        Ok(Some(message_html(
            &view, &messages, index, None, last_read, deleted,
        )?))
    }

    /// What chat.js pages and jumps with: whether more messages load
    /// either way (ChatMessagesLoader's canLoadMorePast and
    /// canLoadMoreFuture), the message a near-message url targets (scrolled
    /// to and highlighted), the "all loaded" line and the error a page
    /// that can't load pops (its 404's: popupAjaxError).
    fn pane_attrs(&self, listed: &Value, target: Option<i64>, loaded: bool) -> String {
        let mut out = format!(
            " data-can-load-more-past=\"{}\" data-can-load-more-future=\"{}\" data-label-all-loaded=\"{}\" data-label-not-found=\"{}\"",
            listed["meta"]["can_load_more_past"],
            listed["meta"]["can_load_more_future"],
            escape(&js(self.i18n, "chat.all_loaded")),
            escape(
                self.i18n
                    .t("not_found")
                    .unwrap_or("The requested URL or resource could not be found.")
            ),
        );
        if let Some(id) = target.filter(|_| loaded) {
            out.push_str(&format!(" data-target=\"{id}\""));
        }
        out
    }

    /// A page of the channel's messages as chat.js asks for one:
    /// ChatMessagesLoader#loadMore's, `direction` past or future from
    /// `target` (the first or last message the viewer has), drawn as if
    /// before or after it, the target drawn again after the past ones (its
    /// reply line follows the message now above it); else #load's, the
    /// page around `target` or the latest, drawn alone. `newest`: the
    /// viewer's page has no "last visit" line, so this one may carry it
    /// (processMessages' hasNewest). None when the viewer can't see the
    /// channel or the target is gone (the request's 404).
    pub async fn messages_page(
        &mut self,
        channel_id: i64,
        target: Option<i64>,
        direction: Option<&str>,
        newest: bool,
    ) -> Result<Option<MessagesPage>, AppError> {
        let channel = match self.find_joinable(&channel_id.to_string()).await? {
            Found::Channel(channel) => channel,
            Found::NotFound | Found::Forbidden => return Ok(None),
        };
        let direction = direction.filter(|d| *d == "past" || *d == "future");
        let params = ListParams {
            page_size: PAGE_SIZE,
            target_message_id: target,
            direction: direction.map(str::to_string),
            fetch_from_last_read: false,
            target_date: None,
        };
        let listed = match self.list_messages(channel.id, &params).await? {
            Listed::Messages(v) => v,
            Listed::NotFound | Listed::Forbidden => return Ok(None),
        };
        let mut messages = listed["messages"].as_array().cloned().unwrap_or_default();
        let membership = self
            .memberships()
            .await?
            .into_iter()
            .find(|m| m.chat_channel_id == channel.id);
        let channel_json = self
            .channel(&channel, membership.as_ref(), None, None)
            .await?;
        let last_read = membership.as_ref().and_then(|m| m.last_read_message_id);
        let newest = if newest {
            newest_id(&channel_json, &messages)
        } else {
            None
        };
        let anchor = match (direction, target) {
            (Some(_), Some(id)) => self
                .serialize_messages(&channel, &[id])
                .await?
                .into_iter()
                .next(),
            _ => None,
        };
        let count = messages.len();
        let (range, redraw) = match (direction, anchor) {
            (Some("past"), Some(mut anchor)) => {
                anchor["first_of_results"] = Value::Bool(true);
                let redraw = anchor["deleted_at"].is_null();
                messages.push(anchor);
                (0..count, redraw.then_some(count))
            }
            (Some(_), Some(anchor)) => {
                if let Some(first) = messages.first_mut() {
                    first["first_of_results"] = Value::Bool(true);
                }
                messages.insert(0, anchor);
                (1..count + 1, None)
            }
            _ => (0..count, None),
        };
        let i18n = self.i18n;
        let t = move |key: &str| js(i18n, key);
        let view = View {
            base_path: self.base_path,
            channel: &channel_json,
            viewer_id: i64::from(self.guardian.user_id().unwrap_or(0)),
            online: self.online_ids().await?,
            emojis: self.settings.emojis().clone(),
            max_image: (
                self.settings.get("max_image_width")?.to_f(),
                self.settings.get("max_image_height")?.to_f(),
            ),
            base_url: crate::url::Urls {
                config: self.config,
                settings: self.settings,
            }
            .base_url()?,
            can_interact: channel_json["meta"]["user_silenced"] != true,
            prioritize_name: prioritize_name(self.settings)?,
            admin: self.guardian.is_admin(),
            t: &t,
            i18n: self.i18n,
            relative_date_duration: self.settings.get("relative_date_duration")?.to_i(),
            thread_context: false,
        };
        let mut html = messages_html(&view, &messages, range, newest, last_read)?;
        if let Some(index) = redraw {
            html.push_str(&format!(
                "<template class=\"chat-message-anchor\" data-id=\"{}\">{}</template>",
                messages[index]["id"],
                message_html(&view, &messages, index, newest, last_read, false)?
            ));
        }
        Ok(Some(MessagesPage {
            html,
            can_load_more_past: listed["meta"]["can_load_more_past"].clone(),
            can_load_more_future: listed["meta"]["can_load_more_future"].clone(),
        }))
    }

    /// ChatRetentionReminder on a category channel, for staff who haven't
    /// dismissed it while channel messages expire
    /// (`needs_channel_retention_reminder`).
    async fn retention_reminder(&mut self, c: &Value) -> Result<String, AppError> {
        let i18n = self.i18n;
        let t = move |key: &str| js(i18n, key);
        let dm = c["chatable_type"] == "DirectMessage";
        // needs_dm_retention_reminder (anyone) or
        // needs_channel_retention_reminder (staff), with the type's days.
        let days = self
            .settings
            .get(if dm {
                "chat_dm_retention_days"
            } else {
                "chat_channel_retention_days"
            })?
            .to_i();
        let Some(user_id) = self.guardian.user_id() else {
            return Ok(String::new());
        };
        if (!dm && !self.guardian.is_staff()) || days == 0 {
            return Ok(String::new());
        }
        let dismissed = super::options(&mut *self.conn, user_id)
            .await?
            .and_then(|o| {
                if dm {
                    o.dismissed_dm_retention_reminder
                } else {
                    o.dismissed_channel_retention_reminder
                }
            })
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
    /// channel's settings), the star, and the actions. In the drawer, its
    /// navbar: clickable (toggling the drawer), the back button while
    /// expanded, and the drawer's buttons in place of OpenDrawerButton.
    fn navbar(
        &self,
        c: &Value,
        following: bool,
        online: &[i64],
        drawer: Option<&super::drawer::Nav<'_>>,
        threads_button: Option<usize>,
    ) -> Result<String, AppError> {
        let i18n = self.i18n;
        let t = move |key: &str| js(i18n, key);
        let slug = &slugified_title(c);
        let id = c["id"].as_i64().unwrap_or(0);
        let mut out = format!(
            "<div class=\"c-navbar-container{}\"><nav class=\"c-navbar\">",
            if drawer.is_some() { " -clickable" } else { "" }
        );
        let mobile = crate::mobile::mobile_view();
        if let Some(nav) = drawer {
            out.push_str(&nav.back_button(nav.back));
        } else if mobile {
            // ChatNavbarBackButton to the channels (or a DM's list), with
            // the other channels' unread indicator, which the page's
            // chrome puts in place of BACK_INDICATOR.
            let route = if c["chatable_type"] == "DirectMessage" {
                "direct-messages"
            } else {
                "channels"
            };
            out.push_str(&format!(
                "<a class=\"c-navbar__back-button no-text btn-transparent btn\" title=\"{}\" href=\"{}/chat/{route}\">{}{BACK_INDICATOR}</a>",
                escape(&t("chat.browse.back")),
                self.base_path,
                d_icon("chevron-left", None)
            ));
        }
        // The collapsed drawer's navbar: the title without its link, and
        // none of the channel's buttons (isDrawerCollapsed).
        let collapsed = drawer.is_some_and(|nav| !nav.expanded);
        let title = channel_title(c, self.settings.emojis(), self.base_path, online);
        if collapsed {
            out.push_str(&format!(
                "<div class=\"c-navbar__channel-title\">{title}</div>"
            ));
        } else {
            out.push_str(&format!(
                "<a class=\"c-navbar__channel-title\" href=\"{}/chat/c/{}/{id}/info/settings\">{title}</a>",
                self.base_path,
                escape(slug),
            ));
        }
        if following && !collapsed {
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
        if self.settings.get("chat_search_enabled")?.truthy() && !collapsed {
            out.push_str(&format!(
                "<button class=\"btn no-text btn-icon btn-transparent c-navbar__filter\" type=\"button\">{}<span aria-hidden=\"true\">&#8203;</span></button>",
                d_icon("discourse-chat-search", None)
            ));
        }
        // OpenDrawerButton: desktop only, and not in the drawer.
        if !mobile && drawer.is_none() {
            out.push_str(&format!(
                "<button class=\"btn no-text btn-icon c-navbar__open-drawer-button btn-transparent\" title=\"{}\" type=\"button\">{}<span aria-hidden=\"true\">&#8203;</span></button>",
                escape(&t("chat.close_full_page")),
                d_icon("discourse-compress", None)
            ));
        }
        if self.settings.get("chat_pinned_messages")?.truthy()
            && !collapsed
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
        // ChatNavbarThreadsListButton, with ThreadHeaderUnreadIndicator:
        // not on the threads or a thread route (`threads_button` None).
        if let Some(unread) =
            threads_button.filter(|_| c["threading_enabled"] == true && !collapsed)
        {
            out.push_str(&format!(
                "<a class=\"c-navbar__threads-list-button btn no-text btn-transparent{}\" title=\"{}\" href=\"{}/chat/c/{}/{id}/t\">{}{}</a>",
                if unread > 0 { " has-unreads" } else { "" },
                escape(&t("chat.threads.list")),
                self.base_path,
                escape(slug),
                d_icon("discourse-threads", None),
                thread_header_unread_indicator(self.i18n, unread)
            ));
        }
        if let Some(nav) = drawer {
            out.push_str(&nav.buttons());
        }
        out.push_str("</nav></nav></div>");
        Ok(out)
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
        // A one-to-one DM: the other user's face, its own title, no tip
        // or facepile.
        if c["chatable_type"] == "DirectMessage" && c["chatable"]["group"] != true {
            let image = match c["chatable"]["users"].get(0) {
                Some(u) => avatar(u, 96),
                None => String::new(),
            };
            let mut out = String::from(
                "<div class=\"empty-state__container --chat-channel --with-image\"><div class=\"empty-state\">",
            );
            out.push_str(&format!("<div class=\"empty-state__image\">{image}</div>"));
            out.push_str(&format!(
                "<div class=\"empty-state__title\" data-test-title=\"\">{}</div>",
                escape(
                    &t("chat.channel.empty_state.direct_message_title")
                        .replace("%{name}", s(&c["title"]))
                )
            ));
            if let Some(description) = c["description"].as_str() {
                out.push_str(&format!(
                    "<div class=\"empty-state__body\"><p data-test-body=\"\">{}</p></div>",
                    escape(description)
                ));
            }
            out.push_str("<div class=\"empty-state__tip\"></div></div></div>");
            return Ok(out);
        }
        let channel_name = format!("#{}", s(&c["title"]));
        let total = c["memberships_count"].as_i64().unwrap_or(0);
        let count = (if following { total - 1 } else { total }).max(0);
        let icon = match c["emoji"].as_str() {
            Some(emoji) => emoji_img(self.settings.emojis(), self.base_path, emoji),
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
        self.composer_for(channel, c, None).await
    }

    /// The channel's composer (ChatComposerChannel), or a thread's
    /// (ChatComposerThread: its placeholder, draft and ids).
    pub(super) async fn composer_for(
        &mut self,
        channel: &ChannelRow,
        c: &Value,
        thread: Option<i64>,
    ) -> Result<String, AppError> {
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
        } else if thread.is_some() {
            t("chat.placeholder_thread")
        } else if c["chatable_type"] == "DirectMessage" {
            // #messageRecipients for a DM: a group's, the viewer alone, or
            // the others by name (else @username).
            let users = c["chatable"]["users"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            let viewer = i64::from(self.guardian.user_id().unwrap_or(0));
            if c["chatable"]["group"] == true {
                t("chat.placeholder_group")
            } else if users.len() == 1 && users[0]["id"].as_i64() == Some(viewer) {
                t("chat.placeholder_self")
            } else {
                let comma = self.i18n.t("js.word_connector.comma").unwrap_or(", ");
                let names = users
                    .iter()
                    .map(|u| match u["name"].as_str().filter(|n| !n.is_empty()) {
                        Some(name) => name.to_string(),
                        None => format!("@{}", s(&u["username"])),
                    })
                    .collect::<Vec<_>>()
                    .join(comma);
                t("chat.placeholder_users").replace("%{commaSeparatedNames}", &names)
            }
        } else {
            t("chat.placeholder_channel").replace("%{channelName}", &format!("#{}", s(&c["title"])))
        };
        let draft = self.draft_message(channel.id, thread).await?;
        let context = if thread.is_some() {
            "thread"
        } else {
            "channel"
        };
        let disabled_attr = if disabled { " disabled=\"\"" } else { "" };
        // What chat.js sends with: the minimum length (hasContent) and the
        // member's send shortcut.
        let shortcut = match self.guardian.user_id() {
            Some(id) => super::options(&mut *self.conn, id)
                .await?
                .and_then(|o| {
                    super::SEND_SHORTCUTS
                        .get(usize::try_from(o.send_shortcut).unwrap_or(0))
                        .copied()
                })
                .unwrap_or("enter"),
            None => "enter",
        };
        // What the composer's autocompletes read: the site settings that
        // turn them on, the denied emojis, the hashtag types in the
        // chat-composer context's order (ordered_types_for_context) and the
        // "more" item's label.
        let mut hashtag_order = Vec::new();
        if self.settings.get("enable_public_channels")?.truthy() {
            hashtag_order.push("channel");
        }
        hashtag_order.push("category");
        if self.settings.get("tagging_enabled")?.truthy() {
            hashtag_order.push("tag");
        }
        let autocomplete = serde_json::json!({
            "mentions": self.settings.get("enable_mentions")?.truthy(),
            "emoji": self.settings.get("enable_emoji")?.truthy(),
            "emojiMinChars": self.settings.get("emoji_autocomplete_min_chars")?.to_i(),
            "deniedEmojis": self.settings.emojis().denied,
            "hashtagOrder": hashtag_order,
            "moreEmoji": t("composer.more_emoji"),
        });
        // ChatReplyingIndicator's strings, its plural forms whole.
        let replying = serde_json::json!({
            "single_user": t("chat.replying_indicator.single_user"),
            "multiple_users": t("chat.replying_indicator.multiple_users"),
            "many_users_one": t("chat.replying_indicator.many_users.one"),
            "many_users_other": t("chat.replying_indicator.many_users.other"),
            "comma": t("word_connector.comma"),
        });
        // modifySelection's example texts (applySurround's bold_text,
        // italic_text and code_text), for chat-drawer.js's shortcuts.
        let surround = serde_json::json!({
            "bold": t("composer.bold_text"),
            "italic": t("composer.italic_text"),
            "code": t("composer.code_text"),
        });
        let mut out = format!(
            "<div class=\"chat-composer__wrapper\" data-min-length=\"{}\" data-max-length=\"{}\" data-send-shortcut=\"{shortcut}\" data-autocomplete=\"{}\" data-replying=\"{}\" data-surround=\"{}\">",
            self.settings.get("chat_minimum_message_length")?.to_i(),
            self.settings.get("chat_maximum_message_length")?.to_i(),
            escape(&autocomplete.to_string()),
            escape(&replying.to_string()),
            escape(&surround.to_string())
        );
        out.push_str(&draft.details);
        // In the mobile view the toolbar trigger and the send button sit
        // outside the input's container, and there is no emoji button.
        let mobile = crate::mobile::mobile_view();
        out.push_str(&format!(
            "<div aria-label=\"{}\" class=\"chat-composer is-send-disabled {} is-draft-saved\" role=\"region\"><div class=\"chat-composer__outer-container\">",
            escape(&t("chat.aria_roles.composer")),
            if disabled { "is-disabled" } else { "is-enabled" }
        ));
        let trigger = format!(
            "<button aria-expanded=\"false\" class=\"btn no-text btn-icon fk-d-menu__trigger chat-composer-dropdown__menu-trigger chat-composer-dropdown__trigger-btn btn-flat\" title=\"{}\" data-identifier=\"chat-composer-dropdown__menu\" data-trigger=\"\" type=\"button\"{disabled_attr}>{}</button>",
            escape(&t("chat.composer.toggle_toolbar")),
            d_icon("plus", None)
        );
        if mobile {
            out.push_str(&trigger);
            out.push_str("<div class=\"chat-composer__inner-container\">");
        } else {
            out.push_str("<div class=\"chat-composer__inner-container\">");
            out.push_str(&trigger);
        }
        out.push_str(&format!(
            "<div class=\"chat-composer__input-container\"><textarea placeholder=\"{}\" autocorrect=\"on\" autocapitalize=\"sentences\" rows=\"1\" id=\"{context}-composer\" class=\"chat-composer__input\" data-chat-composer-context=\"{context}\" type=\"text\"{disabled_attr}>{}</textarea></div>",
            escape(&placeholder),
            escape(&draft.message)
        ));
        if mobile {
            out.push_str("</div>");
        } else {
            out.push_str(&format!(
                "<button aria-expanded=\"false\" class=\"btn no-text fk-d-menu__trigger emoji-picker-trigger chat-composer-button btn-transparent --emoji\" data-identifier=\"emoji-picker\" data-trigger=\"\" type=\"button\"{disabled_attr}>{}&#8203;</button><div class=\"chat-composer-separator\"></div>",
                d_icon("far-face-smile", None)
            ));
        }
        out.push_str(&format!(
            "<div class=\"chat-composer-button__wrapper\"><button class=\"chat-composer-button -send\" disabled=\"\" tabindex=\"-1\" title=\"{}\" type=\"button\">{}</button></div>",
            escape(&t("chat.composer.send")),
            d_icon("paper-plane", None)
        ));
        out.push_str(if mobile {
            "</div></div>"
        } else {
            "</div></div></div>"
        });
        if self.settings.get("chat_allow_uploads")?.truthy() {
            // ChatComposerUploads: the draft's uploads (existingUploads),
            // done, then the file picker.
            let uploads = if draft.uploads.is_empty() {
                String::new()
            } else {
                format!(
                    "<div class=\"chat-composer-uploads-container\">{}</div>",
                    draft
                        .uploads
                        .iter()
                        .map(|u| composer_upload(u, self.base_path, &t))
                        .collect::<String>()
                )
            };
            out.push_str(&format!(
                "<div class=\"chat-composer-uploads\" data-uploads=\"{}\" data-label-upload=\"{}\" data-label-uploading=\"{}\" data-label-processing=\"{}\" data-label-remove=\"{}\" data-label-uploaded-files=\"{}\" data-max-image=\"{}x{}\">{uploads}<div class=\"pick-files-button\"><input accept=\"{}\" class=\"hidden-upload-field\" id=\"{context}-file-uploader\" multiple=\"\" type=\"file\"></div></div>",
                escape(&Value::Array(draft.uploads.clone()).to_string()),
                escape(&t("chat.upload")),
                escape(&t("uploading")),
                escape(&t("processing")),
                escape(&t("chat.remove_upload")),
                escape(&t("chat.uploaded_files.other")),
                self.settings.get("max_image_width")?.to_i(),
                self.settings.get("max_image_height")?.to_i(),
                escape(&self.upload_accept()?)
            ));
        }
        out.push_str("<div class=\"chat-replying-indicator-container\"><div class=\"chat-replying-indicator is-subscribed\"></div></div></div>");
        Ok(out)
    }

    /// The member's draft for the channel or thread (chat_drafts): its
    /// text, its uploads, and the message it edits or replies to.
    async fn draft_message(
        &mut self,
        channel_id: i64,
        thread: Option<i64>,
    ) -> Result<Draft, AppError> {
        let Some(user_id) = self.guardian.user_id() else {
            return Ok(Draft::default());
        };
        let data: Option<Option<String>> = sqlx::query_scalar(
            "SELECT data FROM chat_drafts WHERE user_id = $1 AND chat_channel_id = $2 AND thread_id IS NOT DISTINCT FROM $3 \
             ORDER BY updated_at DESC LIMIT 1",
        )
        .bind(user_id)
        .bind(channel_id)
        .bind(thread)
        .fetch_optional(&mut *self.conn)
        .await?;
        let Some(Some(data)) = data else {
            return Ok(Draft::default());
        };
        // What UpsertDraft stores is Ruby's inspect of the client's hash,
        // which the client's JSON.parse throws on: such a draft shows as
        // none here.
        let Ok(draft) = serde_json::from_str::<Value>(&data) else {
            return Ok(Draft::default());
        };
        Ok(Draft {
            message: s(&draft["message"]).to_string(),
            details: self.draft_details(&draft).await?,
            uploads: draft["uploads"].as_array().cloned().unwrap_or_default(),
        })
    }

    /// ChatComposerMessageDetails for a draft restored as
    /// ChatMessage.createDraftMessage makes it: an edit (`editing`, the
    /// draft's id and excerpt, the member's own), else a reply
    /// (`replyToMsg`, as it was saved).
    async fn draft_details(&mut self, draft: &Value) -> Result<String, AppError> {
        let i18n = self.i18n;
        let t = move |key: &str| js(i18n, key);
        let (action, id, excerpt, user) = if draft["editing"] == true {
            let Some(user_id) = self.guardian.user_id() else {
                return Ok(String::new());
            };
            let (username, avatar): (String, Option<i32>) =
                sqlx::query_as("SELECT username, uploaded_avatar_id FROM users WHERE id = $1")
                    .bind(user_id)
                    .fetch_one(&mut *self.conn)
                    .await?;
            let logo = crate::admin_users::logo_small_url(&mut *self.conn, self.settings).await?;
            let urls = crate::url::Urls {
                config: self.config,
                settings: self.settings,
            };
            let template =
                crate::avatar::avatar_template(&urls, user_id, &username, avatar, logo.as_deref())?;
            (
                "edit",
                draft["id"].as_i64().unwrap_or(0),
                s(&draft["excerpt"]).to_string(),
                serde_json::json!({"id": user_id, "username": username, "avatar_template": template}),
            )
        } else if draft["replyToMsg"].is_object() {
            let reply = &draft["replyToMsg"];
            (
                "reply",
                reply["id"].as_i64().unwrap_or(0),
                s(&reply["excerpt"]).to_string(),
                reply["user"].clone(),
            )
        } else {
            return Ok(String::new());
        };
        let view = View {
            base_path: self.base_path,
            channel: &Value::Null,
            viewer_id: i64::from(self.guardian.user_id().unwrap_or(0)),
            online: self.online_ids().await?,
            emojis: self.settings.emojis().clone(),
            max_image: (
                self.settings.get("max_image_width")?.to_f(),
                self.settings.get("max_image_height")?.to_f(),
            ),
            base_url: crate::url::Urls {
                config: self.config,
                settings: self.settings,
            }
            .base_url()?,
            can_interact: true,
            prioritize_name: prioritize_name(self.settings)?,
            admin: self.guardian.is_admin(),
            t: &t,
            i18n: self.i18n,
            relative_date_duration: self.settings.get("relative_date_duration")?.to_i(),
            thread_context: false,
        };
        let text = format!(
            "<span class=\"chat-reply__username\">{}</span><span class=\"chat-reply__excerpt\">{}</span>",
            escape(s(&user["username"])),
            crate::topic_list_view::emoji_unescape(&excerpt, &view.emojis, self.base_path)
        );
        let data = format!(
            " data-excerpt=\"{}\" data-user=\"{}\"",
            escape(&excerpt),
            escape(&user.to_string())
        );
        Ok(super::actions_view::details_bar(
            &t,
            action,
            id,
            &data,
            &user_avatar(&user, 24, false, &view),
            &text,
        ))
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

/// ChatChannel#slugifiedTitle: the slug, else the title slugified.
pub(crate) fn slugified_title(c: &Value) -> String {
    crate::routes::user_menu::slugify_channel(c["slug"].as_str(), &escape(s(&c["title"])))
}

/// ChatUserAvatar, not interactive: the avatar in a span, marked online
/// when the user is on /chat/online.
pub(super) fn static_user_avatar(user: &Value, size: u32, online: &[i64]) -> String {
    let is_online = user["id"].as_i64().is_some_and(|id| online.contains(&id));
    format!(
        "<div class=\"chat-user-avatar{}\" data-username=\"{}\"><span class=\"chat-user-avatar__container\">{}</span></div>",
        if is_online { " is-online" } else { "" },
        escape(s(&user["username"])),
        avatar(user, size)
    )
}

/// ChatChannelTitle: the channel's icon (ChannelIcon) and its name.
pub(super) fn channel_title(
    c: &Value,
    emojis: &crate::emojis::Emojis,
    base_path: &str,
    online: &[i64],
) -> String {
    let icon = channel_icon(c, emojis, base_path, online);
    if c["chatable_type"] == "DirectMessage" {
        return format!(
            "<div class=\"chat-channel-title is-dm\">{icon}<div class=\"chat-channel-name\"><div class=\"chat-channel-name__label\">{}</div></div></div>",
            escape(c["unicode_title"].as_str().unwrap_or(s(&c["title"])))
        );
    }
    format!(
        "<div class=\"chat-channel-title is-category\">{icon}<div class=\"chat-channel-name\"><div class=\"chat-channel-name__label\">{}</div></div></div>",
        escape(s(&c["unicode_title"]))
    )
}

/// ChannelIcon: a DM's first user, emoji or member count, a category
/// channel's emoji or chat icon in the category's color.
pub(crate) fn channel_icon(
    c: &Value,
    emojis: &crate::emojis::Emojis,
    base_path: &str,
    online: &[i64],
) -> String {
    if c["chatable_type"] == "DirectMessage" {
        let users = c["chatable"]["users"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let first = users.first();
        let face = |u: Option<&Value>| match u {
            Some(u) => format!(
                "<div class=\"chat-channel-icon --avatar\">{}</div>",
                static_user_avatar(u, 24, online)
            ),
            None => "<div class=\"chat-channel-icon --avatar\"></div>".to_string(),
        };
        return if c["chatable"]["group"] == true {
            if let Some(emoji) = c["emoji"].as_str() {
                format!(
                    "<div class=\"chat-channel-icon --emoji\">{}</div>",
                    emoji_img(emojis, base_path, emoji)
                )
            } else if users.len() == 1 {
                face(first)
            } else {
                format!(
                    "<div class=\"chat-channel-icon --users-count\">{}</div>",
                    c["memberships_count"].as_i64().unwrap_or(0)
                )
            }
        } else {
            face(first)
        };
    }
    let icon = match c["emoji"].as_str() {
        Some(emoji) => emoji_img(emojis, base_path, emoji),
        None => d_icon("d-chat", None),
    };
    let lock = if c["chatable"]["read_restricted"] == true {
        d_icon("lock", Some("chat-channel-icon__restricted-category-icon"))
    } else {
        String::new()
    };
    format!(
        "<div class=\"chat-channel-icon --icon\" style=\"color: #{}\">{icon}{lock}</div>",
        escape(s(&c["chatable"]["color"]))
    )
}

/// dReplaceEmoji(`:emoji:`): the emoji's image (a custom one's upload),
/// nothing for a denied one, the code as written for an unknown one.
fn emoji_img(emojis: &crate::emojis::Emojis, base_path: &str, name: &str) -> String {
    if emojis.enabled && emojis.is_denied(name) {
        return String::new();
    }
    let url = emojis.url(name, base_path).filter(|_| emojis.enabled);
    let Some(url) = url else {
        return escape(&format!(":{name}:"));
    };
    let class = if emojis.is_custom(name) {
        "emoji emoji-custom"
    } else {
        "emoji"
    };
    format!(
        "<img width=\"20\" height=\"20\" src=\"{}\" title=\"{n}\" alt=\"{n}\" class=\"{class}\">",
        escape(&url),
        n = escape(name)
    )
}

/// ChatChannelStatus, long: a closed, read only or archived channel says
/// so above its messages, with ChatChannelArchiveStatus for an admin.
pub(super) fn channel_status(
    c: &Value,
    t: &dyn Fn(&str) -> String,
    admin: bool,
    base_path: &str,
) -> String {
    let status = s(&c["status"]);
    let icon = match status {
        "closed" => "lock",
        "read_only" => "comment-slash",
        "archived" => "box-archive",
        _ => return String::new(),
    };
    format!(
        "<div class=\"chat-channel-status\">{}<span>{}</span>{}</div>",
        d_icon(icon, None),
        escape(&t(&format!("chat.channel_status.{status}_header"))),
        if admin {
            archive_status(c, t, base_path)
        } else {
            String::new()
        }
    )
}

/// ChatChannelArchiveStatus: a failed archive's progress with its retry
/// button, or a completed one's topic (ChatChannel#archive, made when the
/// serializer says either).
fn archive_status(c: &Value, t: &dyn Fn(&str) -> String, base_path: &str) -> String {
    let topic_url = c["archive_topic_id"]
        .as_i64()
        .map(|id| format!("{base_path}/t/-/{id}"))
        .unwrap_or_default();
    let status = s(&c["status"]);
    if c["archive_failed"] == true {
        let key = if c["archive_topic_id"].is_null() {
            "chat.channel_status.archive_failed_no_topic"
        } else {
            "chat.channel_status.archive_failed"
        };
        let message = t(key)
            .replace("%{completed}", &c["archived_messages"].to_string())
            .replace("%{total}", &c["total_messages"].to_string())
            .replace("%{topic_url}", &topic_url);
        format!(
            "<div class=\"alert alert-warn chat-channel-retry-archive {status}\"><div class=\"chat-channel-archive-failed-message\">{message}</div><div class=\"chat-channel-archive-failed-retry\"><button class=\"btn btn-text\" type=\"button\" data-channel-id=\"{}\"><span class=\"d-button-label\">{}</span></button></div></div>",
            c["id"],
            escape(&t("chat.channel_archive.retry"))
        )
    } else if c["archive_completed"] == true {
        format!(
            "<div class=\"chat-channel-archive-status {status}\">{}</div>",
            t("chat.channel_status.archive_completed").replace("%{topic_url}", &topic_url)
        )
    } else {
        String::new()
    }
}

/// ChatChannelPreviewCard for a member who doesn't follow the channel.
pub(super) fn preview_card(c: &Value, t: &dyn Fn(&str) -> String) -> String {
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

/// processMessages: the first message past the membership's last read one
/// is the newest (the "last visit" line). Ember skips it when that message
/// `!==` the channel's lastMessage, but the message it just created is
/// never that object, so the last message gets the line too. Without a
/// membership the last read id is undefined and no message is past it; a
/// null one is.
fn newest_id(channel_json: &Value, messages: &[Value]) -> Option<i64> {
    let membership_json = &channel_json["current_user_membership"];
    if !membership_json.is_object() {
        return None;
    }
    let read = membership_json["last_read_message_id"]
        .as_i64()
        .unwrap_or(0);
    messages
        .iter()
        .find(|m| m["id"].as_i64().unwrap_or(0) > read)
        .and_then(|m| m["id"].as_i64())
}

/// The messages at `range` of `messages` (the others are their
/// neighbours): of a run of deleted messages only the last is drawn,
/// collapsed (shouldRender); expanding it (chat.js) draws the run from
/// each one's expanded form.
fn messages_html(
    view: &View,
    messages: &[Value],
    range: std::ops::Range<usize>,
    newest: Option<i64>,
    last_read: Option<i64>,
) -> Result<String, AppError> {
    let mut out = String::new();
    for index in range {
        if messages[index]["deleted_at"].is_null() {
            out.push_str(&message_html(
                view, messages, index, newest, last_read, false,
            )?);
            continue;
        }
        let next_deleted = messages
            .get(index + 1)
            .is_some_and(|n| !n["deleted_at"].is_null());
        if !next_deleted {
            out.push_str(&message_html(
                view, messages, index, newest, last_read, false,
            )?);
        }
        out.push_str(&format!(
            "<template class=\"chat-message-expanded\" data-id=\"{}\">{}</template>",
            messages[index]["id"],
            message_html(view, messages, index, newest, last_read, true)?
        ));
    }
    Ok(out)
}

/// ChatSkeleton: fifteen placeholder messages, each with an image one
/// time in ten, one to five lines of 20 to 95% and up to three reactions.
fn skeleton() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let mut out = String::from("<div class=\"chat-skeleton --animation\">");
    for _ in 0..15 {
        out.push_str("<div class=\"chat-skeleton__body\"><div class=\"chat-skeleton__message\"><div class=\"chat-skeleton__message-avatar\"></div><div class=\"chat-skeleton__message-poster\"></div><div class=\"chat-skeleton__message-content\">");
        if rng.gen_range(1..=10) == 5 {
            out.push_str("<div class=\"chat-skeleton__message-img\"></div>");
        }
        out.push_str("<div class=\"chat-skeleton__message-text\">");
        for _ in 0..rng.gen_range(1..=5) {
            out.push_str(&format!(
                "<div class=\"chat-skeleton__message-msg\" style=\"width: {}%\"></div>",
                rng.gen_range(20..=95)
            ));
        }
        out.push_str("</div>");
        let reactions = rng.gen_range(0..=3);
        if reactions > 0 {
            out.push_str("<div class=\"chat-skeleton__message-reactions\">");
            for _ in 0..reactions {
                out.push_str("<div class=\"chat-skeleton__message-reaction\"></div>");
            }
            out.push_str("</div>");
        }
        out.push_str("</div></div></div>");
    }
    out.push_str("</div>");
    out
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
pub(super) fn message_html(
    v: &View,
    messages: &[Value],
    index: usize,
    newest: Option<i64>,
    last_read: Option<i64>,
    expanded: bool,
) -> Result<String, AppError> {
    let t = v.t;
    let m = &messages[index];
    let previous = index.checked_sub(1).map(|i| &messages[i]);
    // firstOfResults: the first of the request that loaded it (a page
    // loaded later marks its own).
    let first_of_results = index == 0 || m["first_of_results"] == true;
    let id = m["id"].as_i64().unwrap_or(0);
    let user = &m["user"];
    let hide_info = hide_user_info(m, previous, first_of_results);
    // hideReplyToInfo: a reply to the message right above says nothing.
    // threadingEnabled: the message is in a thread that shows (its
    // thread_id is serialized then), where replies say nothing either.
    let in_thread = m["thread_id"].is_i64();
    let has_reply_line = !m["in_reply_to"].is_null()
        && !v.thread_context
        && !in_thread
        && previous.is_none_or(|p| m["in_reply_to"]["id"] != p["id"]);
    // showThreadIndicator: an original message whose thread has replies.
    let thread_indicator = !v.thread_context
        && in_thread
        && m["thread"]["preview"]["reply_count"].as_i64().unwrap_or(0) > 0;
    let deleted = !m["deleted_at"].is_null();

    let mut classes = vec!["chat-message-container"];
    let streaming = m["streaming"] == true;
    if streaming {
        classes.push("-streaming");
    }
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
    if thread_indicator {
        classes.push("has-thread-indicator");
    }
    if hide_info {
        classes.push("-user-info-hidden");
    }
    if has_reply_line {
        classes.push("has-reply");
    }
    // What the message actions read: its author, text and state.
    let mut data = format!(
        " data-user-id=\"{}\" data-username=\"{}\" data-avatar-template=\"{}\" data-message=\"{}\" data-excerpt=\"{}\"",
        user["id"],
        escape(s(&user["username"])),
        escape(s(&user["avatar_template"])),
        escape(s(&m["message"])),
        escape(s(&m["excerpt"])),
    );
    if let Some(name) = user["name"].as_str() {
        data.push_str(&format!(" data-name=\"{}\"", escape(name)));
    }
    // Its uploads, which an edit starts from.
    if m["uploads"].as_array().is_some_and(|u| !u.is_empty()) {
        data.push_str(&format!(
            " data-uploads=\"{}\"",
            escape(&m["uploads"].to_string())
        ));
    }
    if deleted {
        data.push_str(&format!(" data-deleted-by-id=\"{}\"", m["deleted_by_id"]));
    }
    if let Some(status) = m.get("user_flag_status") {
        data.push_str(&format!(" data-user-flag-status=\"{status}\""));
    }
    // What the flag modal offers (ChatMessageFlag#flagsAvailable).
    if let Some(flags) = m["available_flags"].as_array().filter(|f| !f.is_empty()) {
        let names: Vec<&str> = flags.iter().filter_map(Value::as_str).collect();
        data.push_str(&format!(
            " data-available-flags=\"{}\"",
            escape(&names.join("|"))
        ));
    }
    if m["pinned"] == true {
        data.push_str(" data-pinned=\"\"");
    }
    if let Some(bookmark) = m.get("bookmark") {
        data.push_str(&format!(
            " data-bookmark-name=\"{}\" data-bookmark-reminder-at=\"{}\"",
            escape(s(&bookmark["name"])),
            escape(s(&bookmark["reminder_at"]))
        ));
    }
    let mut out = format!(
        "<div class=\"{}\" data-id=\"{id}\"{} data-created-at=\"{}\"{data}{}{}>",
        classes.join(" "),
        match m["thread_id"].as_i64() {
            Some(thread_id) => format!(" data-thread-id=\"{thread_id}\""),
            None => String::new(),
        },
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

    // A deleted message, unless expanded, is collapsed to a label counting
    // the deleted ones above it.
    if deleted && !expanded {
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
            "<div class=\"chat-message-text -deleted\"><button class=\"btn btn-flat chat-message-expand\" type=\"button\"><span class=\"d-button-label\">{}</span></button></div></div>",
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
            message_avatar(&reply["chat_webhook_event"], &reply["user"], 24, false, v),
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
            message_avatar(&m["chat_webhook_event"], user, 48, true, v)
        ));
    }
    out.push_str("<div class=\"chat-message-content\">");
    if !hide_info {
        out.push_str(&message_info(m, &date, v));
    } else if m["is_action"] != true && user["username"].is_string() {
        out.push_str(&format!(
            "<div class=\"chat-message-info -author-only sr-only\"><button class=\"chat-message-info__username__name\" data-user-card=\"{u}\" type=\"button\">{n}</button></div>",
            u = escape(s(&user["username"])),
            n = escape(author_name(user, v))
        ));
    }
    // ChatMessageText: the collapser when it has uploads, images or image
    // oneboxes.
    let uploads: &[Value] = m["uploads"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let cooked = s(&m["cooked"]);
    if super::collapser::is_collapsible(cooked, uploads) {
        let page_url = format!("{}/chat/", v.base_url);
        let opts = super::collapser::Opts {
            base_path: v.base_path,
            page_url: &page_url,
            max_image: v.max_image,
            t,
        };
        out.push_str(&format!(
            "<div class=\"chat-message-text\">{}",
            super::collapser::collapser_html(cooked, uploads, &opts)?
        ));
    } else {
        out.push_str(&format!(
            "<div class=\"chat-message-text\"><div class=\"chat-cooked\">{cooked}</div>"
        ));
    }
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
    out.push_str("</div>");
    out.push_str(&blocks_html(m, v)?);
    // shouldRenderStopMessageStreamingButton: a streaming message, for an
    // admin or the author of the message it replies to.
    if streaming
        && (v.admin
            || (v.viewer_id > 0 && m["in_reply_to"]["user"]["id"].as_i64() == Some(v.viewer_id)))
    {
        out.push_str(&format!(
            "<div class=\"stop-streaming-btn-container\"><button class=\"btn btn-icon-text stop-streaming-btn\" type=\"button\">{}<span class=\"d-button-label\">{}</span></button></div>",
            d_icon("circle-stop", None),
            escape(&t("cancel"))
        ));
    }
    out.push_str("</div>");
    if thread_indicator {
        out.push_str(&thread_indicator_html(v, m));
    }
    out.push_str("</div></div>");
    Ok(out)
}

/// ThreadHeaderUnreadIndicator: the channel's unread threads, capped at
/// 99, none without.
pub(crate) fn thread_header_unread_indicator(i18n: &crate::i18n::I18n, unread: usize) -> String {
    if unread == 0 {
        return String::new();
    }
    let capped = if unread > 99 {
        "99+".to_string()
    } else {
        unread.to_string()
    };
    format!(
        "<div class=\"chat-thread-header-unread-indicator\" title=\"{}\"><div class=\"chat-thread-header-unread-indicator__number\">{capped}</div></div>",
        escape(
            &i18n
                .t_count(
                    "js.chat.unread_threads_count",
                    unread as i64,
                    &[("count", &capped)]
                )
                .unwrap_or_default()
        )
    )
}

/// ChatUserAvatar linking to the profile, without presence
/// (`@showPresence={{false}}`).
pub(super) fn user_avatar_link(user: &Value, size: u32, base_path: &str) -> String {
    let username = escape(s(&user["username"]));
    format!(
        "<div class=\"chat-user-avatar\" data-username=\"{username}\"><a class=\"chat-user-avatar__container\" data-user-card=\"{username}\" href=\"{base_path}/u/{username}\">{}</a></div>",
        avatar(user, size)
    )
}

/// What dFormatDate reads: the strings and relative_date_duration.
struct ChatDates<'a> {
    i18n: &'a crate::i18n::I18n,
    relative_date_duration: i64,
}

impl crate::viewer_dates::DateContext for ChatDates<'_> {
    fn i18n(&self) -> &crate::i18n::I18n {
        self.i18n
    }
    fn now(&self) -> chrono::DateTime<chrono::Utc> {
        crate::clock::now()
    }
    fn relative_date_duration(&self) -> i64 {
        self.relative_date_duration
    }
}

/// `dFormatDate(date, { leaveAgo: true })`: autoUpdatingRelativeAge in the
/// medium-with-ago format with its title, drawn in the viewer's zone.
pub(crate) fn format_date_ago(
    i18n: &crate::i18n::I18n,
    relative_date_duration: i64,
    at: &str,
) -> String {
    let Ok(at) = chrono::DateTime::parse_from_rfc3339(at).map(|d| d.with_timezone(&chrono::Utc))
    else {
        return String::new();
    };
    let cx = ChatDates {
        i18n,
        relative_date_duration,
    };
    format!(
        "<span class='relative-date date'{} data-time='{}' data-format='medium-with-ago'>{}</span>",
        crate::viewer_dates::long_title(&cx, at),
        at.timestamp_millis(),
        escape(&crate::viewer_dates::relative_age(
            &cx,
            at,
            "medium-with-ago"
        ))
    )
}

/// ChatThreadParticipants: with the original message's author counted,
/// shown for more than one; the first two and the last of more than
/// three, and how many others.
pub(crate) fn thread_participants_html(
    preview: &Value,
    t_count: &dyn Fn(&str, i64) -> String,
    avatar_html: &dyn Fn(&Value) -> String,
) -> String {
    let users = preview["participant_users"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if users.len() <= 1 {
        return String::new();
    }
    let shown: Vec<&Value> = if users.len() > 3 {
        users[..2].iter().chain(users.last()).collect()
    } else {
        users.iter().collect()
    };
    let mut out = String::from(
        "<div class=\"chat-thread-participants\"><div class=\"chat-thread-participants__avatar-group\">",
    );
    for user in &shown {
        out.push_str(&avatar_html(user));
    }
    out.push_str("</div>");
    // otherParticipantCount: participant_count less the users shown by
    // the preview (all of them, not the three drawn).
    let others = preview["participant_count"].as_i64().unwrap_or(0) - users.len() as i64;
    if others != 0 {
        out.push_str(&format!(
            "<div class=\"chat-thread-participants__other-count\">{}</div>",
            escape(&t_count("chat.thread.participants_other_count", others))
        ));
    }
    out.push_str("</div>");
    out
}

/// ChatMessageThreadIndicator: the thread's last reply (its author, when,
/// the excerpt), the reply count and the participants, linking to the
/// last reply in the thread.
fn thread_indicator_html(v: &View, m: &Value) -> String {
    let preview = &m["thread"]["preview"];
    let last_user = &preview["last_reply_user"];
    let i18n = v.i18n;
    let t_count = move |key: &str, count: i64| {
        i18n.t_count(
            &format!("js.{key}"),
            count,
            &[("count", &count.to_string())],
        )
        .unwrap_or_default()
    };
    let participants =
        thread_participants_html(preview, &t_count, &|u| static_user_avatar(u, 24, &[]));
    format!(
        "<a class=\"chat-message-thread-indicator\" tabindex=\"0\" title=\"{}\" href=\"{}/chat/c/{}/{}/t/{}/{}\"><div class=\"chat-message-thread-indicator__last-reply-avatar\">{}</div><div class=\"chat-message-thread-indicator__last-reply-info\"><span class=\"chat-message-thread-indicator__last-reply-username\">{}</span><span class=\"chat-message-thread-indicator__last-reply-timestamp\">{}</span></div><div class=\"chat-message-thread-indicator__replies-count\">{}</div>{participants}<div class=\"chat-message-thread-indicator__last-reply-excerpt\">{}</div></a>",
        escape(&(v.t)("chat.threads.open")),
        v.base_path,
        escape(&slugified_title(v.channel)),
        v.channel["id"],
        m["thread"]["id"],
        preview["last_reply_id"],
        user_avatar(last_user, 24, false, v),
        escape(s(&last_user["username"])),
        format_date_ago(
            v.i18n,
            v.relative_date_duration,
            s(&preview["last_reply_created_at"])
        ),
        escape(&t_count(
            "chat.thread.replies",
            preview["reply_count"].as_i64().unwrap_or(0)
        )),
        crate::topic_list_view::emoji_unescape(
            s(&preview["last_reply_excerpt"]),
            &v.emojis,
            v.base_path
        )
    )
}

/// ChatUserAvatar: the avatar linking to the profile, marked online when
/// its user is in /chat/online.
pub(super) fn user_avatar(user: &Value, size: u32, aria_hidden: bool, v: &View) -> String {
    let username = s(&user["username"]);
    let online = user["id"].as_i64().is_some_and(|id| v.online.contains(&id));
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

/// ChatMessageBlocks: a bot's blocks, each "actions" (its buttons, which
/// chat.js sends as interactions) or "informative" (its elements); the
/// client throws on any other block or element type.
fn blocks_html(m: &Value, v: &View) -> Result<String, AppError> {
    let Some(blocks) = m["blocks"].as_array().filter(|b| !b.is_empty()) else {
        return Ok(String::new());
    };
    let element = |e: &Value| -> Result<String, AppError> {
        match e["type"].as_str() {
            Some("button") => {
                let icon = e["icon"].as_str().filter(|i| !i.is_empty());
                let mut class = format!(
                    "btn {} block__button",
                    if icon.is_some() {
                        "btn-icon-text"
                    } else {
                        "btn-text"
                    }
                );
                if let Some(style) = e["style"].as_str().filter(|s| !s.is_empty()) {
                    class.push_str(&format!(" btn-{style}"));
                }
                let id = match e["action_id"].as_str() {
                    Some(id) => format!(" id=\"{}\"", escape(id)),
                    None => String::new(),
                };
                Ok(format!(
                    "<button class=\"{}\"{id} type=\"button\" data-message-id=\"{}\">{}<span class=\"d-button-label\">{}</span></button>",
                    escape(&class),
                    m["id"],
                    icon.map(|i| d_icon(i, None)).unwrap_or_default(),
                    crate::topic_list_view::emoji_unescape(
                        &escape(s(&e["text"]["text"])),
                        &v.emojis,
                        v.base_path
                    )
                ))
            }
            Some("category") => {
                let mut style = format!("--category-badge-color: #{}", s(&e["color"]));
                let parent_color = e["parent_color"].as_str().filter(|c| !c.is_empty());
                if let Some(pc) = parent_color {
                    style.push_str(&format!("; --parent-category-badge-color: #{pc}"));
                }
                let badge = format!(
                    "<span class=\"badge-category --style-square\"><span class=\"badge-category__name\">{}</span></span>",
                    escape(s(&e["title"]))
                );
                let heading = match e["url"].as_str().filter(|u| !u.is_empty()) {
                    Some(url) => format!(
                        "<a class=\"block__category-link\" href=\"{}\">{badge}</a>",
                        escape(url)
                    ),
                    None => badge,
                };
                let mut out = format!(
                    "<div class=\"block__category\" style=\"{}\"><div class=\"block__category-inner\"><div class=\"block__category-heading\">{heading}</div>",
                    escape(&style)
                );
                if let Some(parent) = e["parent_name"].as_str().filter(|p| !p.is_empty()) {
                    let parent_style = parent_color
                        .map(|pc| format!(" style=\"--category-badge-color: #{}\"", escape(pc)))
                        .unwrap_or_default();
                    out.push_str(&format!(
                        "<div class=\"block__category-parent\"><span class=\"badge-category --style-square\"{parent_style}><span class=\"badge-category__name\">{}</span></span></div>",
                        escape(parent)
                    ));
                }
                if let Some(d) = e["description"].as_str().filter(|d| !d.is_empty()) {
                    out.push_str(&format!(
                        "<div class=\"block__category-description\">{}</div>",
                        escape(d)
                    ));
                }
                out.push_str("</div></div>");
                Ok(out)
            }
            _ => Err(Unsupported("a chat message block element the client can't draw").into()),
        }
    };
    let mut out = String::from(
        "<div class=\"chat-message__blocks-wrapper\"><div class=\"chat-message__blocks\">",
    );
    for block in blocks {
        let elements = block["elements"].as_array().cloned().unwrap_or_default();
        let inner = match block["type"].as_str() {
            Some("actions") => {
                let mut html = String::from(
                    "<div class=\"block__actions-wrapper\"><div class=\"block__actions\">",
                );
                for e in &elements {
                    html.push_str(&format!(
                        "<div class=\"block__action-wrapper\"><div class=\"block__action\">{}</div></div>",
                        element(e)?
                    ));
                }
                html.push_str("</div></div>");
                html
            }
            Some("informative") => {
                let mut html = String::from(
                    "<div class=\"block__informative-wrapper\"><div class=\"block__informative\">",
                );
                for e in &elements {
                    html.push_str(&format!(
                        "<div class=\"block__informative-item\">{}</div>",
                        element(e)?
                    ));
                }
                html.push_str("</div></div>");
                html
            }
            _ => return Err(Unsupported("a chat message block the client can't draw").into()),
        };
        out.push_str(&format!(
            "<div class=\"chat-message__block-wrapper\"><div class=\"chat-message__block\">{inner}</div></div>"
        ));
    }
    out.push_str("</div></div>");
    Ok(out)
}

/// A message's avatar: ChatEmojiAvatar for a webhook's message with an
/// emoji, else the author's ChatUserAvatar.
fn message_avatar(hook: &Value, user: &Value, size: u32, aria_hidden: bool, v: &View) -> String {
    match hook["emoji"].as_str().filter(|e| !e.is_empty()) {
        Some(emoji) => format!(
            "<div class=\"chat-emoji-avatar\"><div class=\"chat-emoji-avatar-container\">{}</div></div>",
            crate::topic_list_view::emoji_unescape(&escape(emoji), &v.emojis, v.base_path)
        ),
        None => user_avatar(user, size, aria_hidden, v),
    }
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
    let text_key = if tiny {
        "dates.time_short"
    } else {
        "dates.time"
    };
    // The same in the viewer's zone before the page paints
    // (crate::viewer_dates); chat.js draws the messages it adds.
    let tz = at
        .map(|at| {
            format!(
                "{}{}",
                crate::viewer_dates::tz_attr(
                    "title",
                    &crate::viewer_dates::marker(at, &(v.t)("dates.long_with_year"))
                ),
                crate::viewer_dates::tz_attr(
                    "text",
                    &crate::viewer_dates::marker(at, &(v.t)(text_key))
                )
            )
        })
        .unwrap_or_default();
    format!(
        "<a title=\"{}\" tabindex=\"-1\" class=\"chat-time\" href=\"{}/chat/c/-/{}/{}\" data-mode=\"{}\"{tz}>{}</a>",
        escape(&format("dates.long_with_year")),
        v.base_path,
        v.channel["id"],
        m["id"],
        if tiny { "tiny" } else { "time" },
        escape(&format(text_key))
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

impl Context<'_> {
    /// What ChatMentionWarnings draws with (chat.js tracks the composer's
    /// mentions as ChatComposerWarningsTracker does): the limits, whether
    /// the channel allows @here and @all, and its strings, the
    /// too_many_members_MF message rendered for one, two and more groups
    /// with the group names (and count) left for chat.js.
    fn mention_warnings_template(&self, c: &Value) -> Result<String, AppError> {
        let settings = self.settings;
        let i18n = self.i18n;
        let t = |key: &str| js(i18n, key);
        let admin = self.guardian.is_admin();
        let max_mentions = settings.get("max_mentions_per_chat_message")?.to_i();
        let limit = settings.get("max_users_notified_per_group_mention")?.to_i();
        let base = self.base_path;
        let mf = t("chat.mention_warning.groups.too_many_members_MF");
        // A count no message has, swapped for chat.js's placeholder.
        const SENTINEL: i64 = 987_654_321;
        let render = |count: i64| -> Result<String, AppError> {
            let url = format!(
                "{base}/admin/site_settings/category/plugins?filter=max_users_notified_per_group_mention"
            );
            let out = crate::message_format::format(
                &mf,
                &[
                    ("groupCount", crate::message_format::Arg::Num(count)),
                    ("isAdmin", crate::message_format::Arg::Bool(admin)),
                    ("siteSettingUrl", crate::message_format::Arg::Str(&url)),
                    ("notificationLimit", crate::message_format::Arg::Num(limit)),
                    ("group1", crate::message_format::Arg::Str("{group1}")),
                    ("group2", crate::message_format::Arg::Str("{group2}")),
                ],
            )?;
            Ok(out.replace(&SENTINEL.to_string(), "{groupCount}"))
        };
        let too_many_key = if admin {
            "chat.mention_warning.too_many_mentions_admin"
        } else {
            "chat.mention_warning.too_many_mentions"
        };
        let too_many = t(&format!(
            "{too_many_key}.{}",
            if max_mentions == 1 { "one" } else { "other" }
        ))
        .replace("%{count}", &max_mentions.to_string())
        .replace(
            "%{siteSettingUrl}",
            &format!(
                "{base}/admin/site_settings/category/plugins?filter=max_mentions_per_chat_message"
            ),
        );
        let attrs = [
            (
                "enabled",
                settings.get("enable_mentions")?.truthy().to_string(),
            ),
            ("max-mentions", max_mentions.to_string()),
            (
                "channel-wide",
                (c["allow_channel_wide_mentions"] == true).to_string(),
            ),
            ("header-all", t("chat.mention_warning.groups.header.all")),
            ("header-some", t("chat.mention_warning.groups.header.some")),
            ("too-many", too_many),
            (
                "channel-wide-disallowed",
                t("chat.mention_warning.channel_wide_mentions_disallowed"),
            ),
            (
                "unreachable-1",
                t("chat.mention_warning.groups.unreachable_1"),
            ),
            (
                "unreachable-2",
                t("chat.mention_warning.groups.unreachable_2"),
            ),
            (
                "unreachable-multiple-one",
                t("chat.mention_warning.groups.unreachable_multiple.one"),
            ),
            (
                "unreachable-multiple-other",
                t("chat.mention_warning.groups.unreachable_multiple.other"),
            ),
            ("over-1", render(1)?),
            ("over-2", render(2)?),
            ("over-other", render(SENTINEL)?),
        ];
        let mut out = String::from("<template class=\"chat-mention-warnings-template\"");
        for (name, value) in attrs {
            out.push_str(&format!(" data-{name}=\"{}\"", escape(&value)));
        }
        out.push_str(&format!(
            "><div class=\"chat-mention-warnings\"><div class=\"chat-mention-warning__icon\">{}</div><div class=\"chat-mention-warning__text\"><div class=\"chat-mention-warning__header\"></div><ul></ul></div></div></template>",
            d_icon("triangle-exclamation", None)
        ));
        Ok(out)
    }
}

/// The settings behind ChatMessageInfo#prioritizeName.
pub(super) fn prioritize_name(settings: &crate::site_settings::SiteSettings) -> Result<bool, AppError> {
    Ok(settings.get("display_name_on_posts")?.truthy()
        && settings.get("enable_names")?.truthy()
        && !settings.get("prioritize_username_in_ux")?.truthy())
}

/// ChatMessageInfo#name: the author's name when it goes first and they
/// have one, else their username.
fn author_name<'a>(user: &'a Value, v: &View) -> &'a str {
    match user["name"].as_str() {
        Some(name) if v.prioritize_name && !name.trim().is_empty() => name,
        _ => s(&user["username"]),
    }
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
    let hook = &m["chat_webhook_event"];
    let mut out = if hook.is_null() {
        classes.push("clickable".into());
        format!(
            "<div class=\"chat-message-info\"><span class=\"{}\"><button class=\"chat-message-info__username__name\" data-user-card=\"{u}\" type=\"button\">{n}</button></span><span class=\"chat-message-info__date\">{date}</span>",
            escape(&classes.join(" ")),
            u = escape(s(&user["username"])),
            n = escape(author_name(user, v))
        )
    } else {
        // A webhook's message: the webhook's name, if it has one, and the
        // bot indicator.
        let name = match hook["username"].as_str().filter(|n| !n.is_empty()) {
            Some(n) => format!(
                "<span class=\"{}\">{}</span>",
                escape(&classes.join(" ")),
                escape(n)
            ),
            None => String::new(),
        };
        format!(
            "<div class=\"chat-message-info\">{name}<span class=\"chat-message-info__bot-indicator\">{}</span><span class=\"chat-message-info__date\">{date}</span>",
            escape(&(v.t)("chat.bot"))
        )
    };
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
    let count = r["count"].as_i64().unwrap_or(0);
    let reacted = r["reacted"] == true;
    // emojiUrlFor: a custom emoji's upload, else the standard image; a
    // reaction without one draws nothing.
    let Some(url) = v
        .emojis
        .url(emoji, v.base_path)
        .filter(|_| v.emojis.enabled)
    else {
        return Ok(String::new());
    };
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
    Some(text.replace(&code, &emoji_img(&v.emojis, v.base_path, emoji)))
}

/// The browse tabs, as `TABS` (archived only with archiving allowed).
pub const BROWSE_TABS: [&str; 4] = ["all", "open", "closed", "archived"];
/// The browse list's page (ChatApi#channels' default limit).
const BROWSE_LIMIT: i64 = 10;

impl Context<'_> {
    /// The browse route (BrowseChannels): the tabs, the filter controls
    /// and the channel cards, ten at a time (the next ten load when the
    /// loading container shows). `filter` narrows by name; the joined
    /// dropdown filters the loaded cards in the browser. In the drawer
    /// (ChatDrawerRoutesBrowse): the drawer's navbar, back to the channels,
    /// and the view only while expanded.
    pub async fn browse_page(
        &mut self,
        tab: &str,
        filter: &str,
        drawer: Option<&super::drawer::Nav<'_>>,
    ) -> Result<String, AppError> {
        let i18n = self.i18n;
        let t = move |key: &str| js(i18n, key);
        let mut out = String::new();
        if let Some(nav) = drawer {
            out.push_str("<div class=\"c-drawer-routes --browse\"><div class=\"c-navbar-container -clickable\"><nav class=\"c-navbar\">");
            out.push_str(&nav.back_button("/chat/channels"));
        } else {
            out.push_str("<div class=\"c-routes --browse\"><div class=\"c-navbar-container\"><nav class=\"c-navbar\">");
            out.push_str(&format!(
                "<a class=\"active c-navbar__back-button no-text btn-transparent btn\" title=\"{}\" href=\"{}/chat\">{}</a>",
                escape(&t("chat.browse.back")),
                self.base_path,
                d_icon("chevron-left", None)
            ));
        }
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
        match drawer {
            Some(nav) => {
                out.push_str(&nav.buttons());
                out.push_str("</nav></nav></div>");
                if !nav.expanded {
                    out.push_str("</div>");
                    return Ok(out);
                }
                out.push_str("<div class=\"chat-drawer-content\">");
            }
            None => out.push_str(&format!(
                "<button class=\"btn no-text btn-icon c-navbar__open-drawer-button btn-transparent\" title=\"{}\" type=\"button\">{}<span aria-hidden=\"true\">&#8203;</span></button></nav></nav></div>",
                escape(&t("chat.close_full_page")),
                d_icon("discourse-compress", None)
            )),
        }

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
        if drawer.is_some() {
            out.push_str("</div>");
        }
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
        let emojis = self.settings.emojis().clone();
        for c in &channels {
            out.push_str(&channel_card(c, &emojis, self.base_path, &t));
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
fn channel_card(
    c: &Value,
    emojis: &crate::emojis::Emojis,
    base_path: &str,
    t: &dyn Fn(&str) -> String,
) -> String {
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
        out.push_str(&emoji_img(emojis, base_path, emoji));
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
pub(crate) const CHAT_ZERO: &str = include_str!("../../../static/svg/chat-zero.svg");

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

/// A member's draft, as the composer shows it.
#[derive(Default)]
struct Draft {
    message: String,
    /// ChatComposerMessageDetails, when it edits or replies.
    details: String,
    /// The uploads it carries, as the client saved them.
    uploads: Vec<Value>,
}

/// ChatComposerUpload for an upload that is done: its preview (the image,
/// or its type's icon), its name and extension unless an image, and the
/// remove button.
fn composer_upload(upload: &Value, base_path: &str, t: &dyn Fn(&str) -> String) -> String {
    let name = s(&upload["original_filename"]);
    let lower = name.to_lowercase();
    let ends = |exts: &[&str]| exts.iter().any(|e| lower.ends_with(&format!(".{e}")));
    let image = ends(&[
        "png", "webp", "jpg", "jpeg", "gif", "svg", "ico", "heic", "heif", "avif", "jxl",
    ]);
    let preview = if image {
        let url = s(&upload["url"]);
        // getURLWithCDN
        let url = if url.starts_with('/')
            && !url.starts_with("//")
            && !url.starts_with(&format!("{base_path}/"))
        {
            format!("{base_path}{url}")
        } else {
            url.to_string()
        };
        format!("<img class=\"preview-img\" src=\"{}\">", escape(&url))
    } else if ends(&["mov", "mp4", "webm", "m4v", "3gp", "ogv", "avi", "mpeg"]) {
        d_icon("file-video", None)
    } else if ends(&[
        "mp3", "ogg", "oga", "opus", "wav", "m4a", "m4b", "m4p", "m4r", "aac", "flac",
    ]) {
        d_icon("file-audio", None)
    } else {
        d_icon("file-lines", None)
    };
    let data = if image {
        "<div class=\"bottom-data\"></div>".to_string()
    } else {
        format!(
            "<div class=\"top-data\"><span class=\"file-name\">{}</span></div><div class=\"bottom-data\"><span class=\"extension-pill\">{}</span></div>",
            escape(name),
            escape(s(&upload["extension"]))
        )
    };
    format!(
        "<div class=\"chat-composer-upload{}\" data-id=\"{}\"><div class=\"preview\">{preview}</div><span class=\"data\">{data}</span><button class=\"btn no-text btn-icon btn-flat chat-composer-upload__remove-btn\" title=\"{}\" type=\"button\">{}<span aria-hidden=\"true\">&#8203;</span></button></div>",
        if image {
            " chat-composer-upload--image"
        } else {
            ""
        },
        upload["id"],
        escape(&t("chat.remove_upload")),
        d_icon("xmark", None)
    )
}
