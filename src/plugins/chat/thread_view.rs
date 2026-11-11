//! The thread routes of full-page chat, server-rendered as page.rs draws
//! the channel route: a thread (ChatRoutesChannelThread: ThreadHeader and
//! ChatThread, in the side panel beside its channel), a channel's threads
//! (ChatRoutesChannelThreads: ChatThreadListHeader and ChatThreadList) and
//! the member's threads (UserThreads, for the threads route), and the
//! pages of a thread's messages and of both lists that static/js
//! chat-thread.js loads as the member scrolls or the lists change.
//! chat-thread.js runs a drawn thread (its live messages, its read state,
//! its paging, the settings modal), chat.js its composer and its
//! messages' actions.

use serde_json::Value;

use super::channels::{ChannelRow, Context, Found};
use super::messages::{Listed, ThreadListParams};
use super::page::{MessagesPage, View, js, messages_html, preview_card, s, static_user_avatar};
use super::threads::{Outcome, ThreadRow};
use crate::post_view::d_icon;
use crate::topic_list_view::escape;
use crate::{AppError, Unsupported};

/// `DEFAULT_MESSAGE_PAGE_SIZE`
const PAGE_SIZE: i64 = 50;
/// `THREAD_TITLE_PROMPT_THRESHOLD`: the replies a thread has before its
/// author is asked for a title.
const THREAD_TITLE_PROMPT_THRESHOLD: i64 = 5;

/// Where a thread's back button goes (ChatThreadHeader#backLink): the
/// channel's threads, or the channel.
pub enum ThreadBack {
    Threads,
    Channel,
}

impl Context<'_> {
    /// The channel's unread threads with what chat-thread.js keeps the
    /// navbar's thread indicators current from, as attributes: the
    /// channel, its unread threads' ids (threadsManager.unreadThreadOverview's
    /// keys), the viewer with their ignored users and followed channels,
    /// whether the indicator hides (do not disturb), and
    /// chat.unread_threads_count's forms for its title.
    pub(crate) async fn unread_threads_tracking(
        &mut self,
        channel: &ChannelRow,
        do_not_disturb: bool,
    ) -> Result<(usize, String), AppError> {
        if !channel.threading_enabled || !self.guardian.is_authenticated() {
            return Ok((0, String::new()));
        }
        let overview = self.unread_thread_overview(&[channel.id]).await?;
        let mut ids: Vec<i64> = overview
            .get(&channel.id.to_string())
            .and_then(Value::as_object)
            .map(|o| o.keys().filter_map(|k| k.parse().ok()).collect())
            .unwrap_or_default();
        ids.sort_unstable();
        let title = |form: &str| {
            escape(
                self.i18n
                    .t(&format!("js.chat.unread_threads_count.{form}"))
                    .unwrap_or_default(),
            )
        };
        let attrs = format!(
            " data-channel-id=\"{}\" data-unread-threads=\"{}\" data-title-one=\"{}\" data-title-other=\"{}\"{}{}",
            channel.id,
            escape(&Value::from(ids.clone()).to_string()),
            title("one"),
            title("other"),
            if do_not_disturb {
                " data-do-not-disturb"
            } else {
                ""
            },
            self.list_viewer_attrs().await?,
        );
        Ok((ids.len(), attrs))
    }

    /// A thread of the channel that shows (ChatChannelThread's model and
    /// afterModel), None when the route goes back to the channel.
    pub async fn shown_thread(
        &mut self,
        channel: &ChannelRow,
        thread_id: i64,
    ) -> Result<Option<ThreadRow>, AppError> {
        Ok(self
            .find_thread(thread_id, Some(channel.id))
            .await?
            .filter(|t| super::threads::shows(channel, t)))
    }

    /// The chat-thread-navbar-actions plugin outlet's connectors
    /// (PluginOutlet with thread, channel and context).
    pub(super) async fn thread_navbar_outlet(
        &mut self,
        thread_id: i64,
        channel: &Value,
        context: &str,
    ) -> Result<String, AppError> {
        crate::plugin_api::outlet_html(
            &mut crate::plugin_api::Ctx {
                conn: &mut *self.conn,
                settings: self.settings,
                i18n: self.i18n,
                base_path: self.base_path,
            },
            &crate::plugin_api::Outlet::ChatThreadNavbarActions {
                thread_id,
                channel,
                context,
            },
        )
        .await
    }

    /// The thread route: ThreadHeader, then ChatThread with its messages
    /// (around `target`, else the member's last read one, else from the
    /// first) and its composer, or the channel's preview card for a
    /// category channel the member doesn't follow.
    pub async fn thread_page(
        &mut self,
        channel: &ChannelRow,
        thread: &ThreadRow,
        target: Option<i64>,
    ) -> Result<String, AppError> {
        self.thread_page_in(channel, thread, target, None).await
    }

    /// The thread route, or in the drawer (ChatDrawerRoutesChannelThread)
    /// the drawer's `navbar` in place of ThreadHeader and ChatThread in
    /// the drawer's content.
    pub async fn thread_page_in(
        &mut self,
        channel: &ChannelRow,
        thread: &ThreadRow,
        target: Option<i64>,
        drawer: Option<&str>,
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
        let thread_membership = self
            .thread_memberships(&[thread.id])
            .await?
            .remove(&thread.id);
        // ChatThread#fetchMessages: around the target or the member's last
        // read message, else from the first.
        let target = target.or(thread_membership
            .as_ref()
            .and_then(|m| m.last_read_message_id));
        let params = ThreadListParams {
            page_size: PAGE_SIZE,
            target_message_id: target,
            direction: if target.is_none() {
                Some("future".into())
            } else {
                None
            },
            fetch_from_last_read: false,
            fetch_from_last_message: false,
            fetch_from_first_message: false,
            target_date: None,
        };
        let listed = match self
            .list_thread_messages(channel.id, thread.id, &params)
            .await?
        {
            Listed::Messages(v) => v,
            Listed::NotFound | Listed::Forbidden => {
                return Err(Unsupported("a chat thread page for a message it can't load").into());
            }
        };
        let messages = listed["messages"].as_array().cloned().unwrap_or_default();
        let original_user: Option<i64> =
            sqlx::query_scalar("SELECT user_id::bigint FROM chat_messages WHERE id = $1")
                .bind(thread.original_message_id)
                .fetch_one(&mut *self.conn)
                .await?;
        let do_not_disturb = match self.guardian.user_id() {
            Some(id) => crate::notification_create::do_not_disturb(&mut *self.conn, id).await?,
            None => false,
        };
        let (unread, tracking) = self
            .unread_threads_tracking(channel, do_not_disturb)
            .await?;
        // ChatThreadTitlePrompt#canShowToast: on a phone, for the original
        // message's author or an admin, an untitled thread with five
        // replies or more whose prompt the member hasn't seen, while they
        // want the prompts.
        let viewer = self.guardian.user_id().map(i64::from);
        let title_prompt = match self.guardian.user_id() {
            Some(id)
                if crate::mobile::mobile_view()
                    && (viewer == original_user || self.guardian.is_admin())
                    && thread.title.is_none()
                    && !thread_membership
                        .as_ref()
                        .is_some_and(|m| m.thread_title_prompt_seen) =>
            {
                super::options(&mut *self.conn, id)
                    .await?
                    .is_some_and(|o| o.show_thread_title_prompts)
                    && self.thread_reply_count(thread).await? >= THREAD_TITLE_PROMPT_THRESHOLD
            }
            _ => false,
        };

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
            prioritize_name: super::page::prioritize_name(self.settings)?,
            admin: self.guardian.is_admin(),
            t: &t,
            i18n: self.i18n,
            relative_date_duration: self.settings.get("relative_date_duration")?.to_i(),
            thread_context: true,
        };

        let mut out = String::new();
        if let Some(navbar) = drawer {
            out.push_str(&format!(
                "<div class=\"c-drawer-routes --channel-thread\">{navbar}<div class=\"chat-drawer-content\">"
            ));
        } else {
            out.push_str("<div class=\"c-routes --channel-thread\">");
            let outlet = self
                .thread_navbar_outlet(thread.id, &channel_json, "full-page")
                .await?;
            out.push_str(&self.thread_header(
                &channel_json,
                &outlet,
                thread,
                thread_membership.as_ref().map(|m| m.notification_level),
                original_user,
                // The unread threads, none counted in do not disturb.
                ((!do_not_disturb).then_some(unread), &tracking),
            ));
        }
        // The latest message, which scrollToLatestMessage loads around.
        let pane = format!(
            "{} data-last-message-id=\"{}\"",
            self.pane_attrs(&listed, target, true),
            thread.last_message_id.unwrap_or(0)
        );
        out.push_str(&format!(
            "<div class=\"chat-thread --loaded\" data-id=\"{}\" data-channel-id=\"{}\" data-original-message-id=\"{}\" data-label-today=\"{}\" data-label-yesterday=\"{}\" data-format-time=\"{}\" data-format-tiny=\"{}\" data-format-title=\"{}\" data-format-date=\"{}\" data-last-read=\"{}\" data-following=\"{}\" data-viewer-id=\"{}\" data-staff=\"{}\" data-can-moderate=\"{}\" data-label-deleted-one=\"{}\" data-label-deleted-other=\"{}\" data-title-prompt=\"{title_prompt}\"{pane}>",
            thread.id,
            channel.id,
            thread.original_message_id,
            escape(&t("chat.chat_message_separator.today")),
            escape(&t("chat.chat_message_separator.yesterday")),
            escape(&t("dates.time")),
            escape(&t("dates.time_short")),
            escape(&t("dates.long_with_year")),
            "MMMM D, YYYY",
            thread_membership
                .as_ref()
                .and_then(|m| m.last_read_message_id)
                .unwrap_or(0),
            following,
            viewer_id,
            self.guardian.is_staff(),
            channel_json["meta"]["can_moderate"] == true,
            escape(&t("chat.deleted.one")),
            escape(&t("chat.deleted.other")),
        ));
        out.push_str(
            "<div class=\"chat-messages-scroller\"><div class=\"chat-messages-container\">",
        );
        out.push_str(&messages_html(
            &view,
            &messages,
            0..messages.len(),
            None,
            last_read,
        )?);
        out.push_str("</div>");
        // ChatThreadHeading: a titled thread's title.
        if let Some(title) = thread.title.as_deref().filter(|t| !t.is_empty()) {
            out.push_str(&format!(
                "<div class=\"chat-thread__heading\"><div class=\"chat-thread__heading-icon\">{}</div><h2 class=\"chat-thread__heading-title\">{}</h2></div>",
                d_icon("discourse-threads", None),
                crate::topic_list_view::emoji_unescape(&escape(title), &view.emojis, self.base_path)
            ));
        }
        out.push_str("</div>");
        out.push_str(&format!(
            "<div class=\"chat-scroll-to-bottom\"><button class=\"btn no-text btn-flat chat-scroll-to-bottom__button\" type=\"button\"><span class=\"chat-scroll-to-bottom__arrow\">{}</span></button></div>",
            d_icon("arrow-down", None)
        ));
        if !following && channel.chatable_type != "DirectMessage" {
            out.push_str(&preview_card(&channel_json, &t));
        } else {
            out.push_str(
                &self
                    .composer_for(channel, &channel_json, Some(thread.id))
                    .await?,
            );
        }
        // ChatModalThreadSettings for those who may change it (the title's
        // click and the settings button), and ChatThreadTitlePrompt's
        // toast.
        let editable = viewer.is_some()
            && (self.guardian.is_staff() || self.guardian.is_admin() || viewer == original_user);
        if editable {
            out.push_str(&self.thread_settings_template(thread));
        }
        if title_prompt {
            let username: Option<String> =
                sqlx::query_scalar("SELECT username FROM users WHERE id = $1")
                    .bind(self.guardian.user_id())
                    .fetch_optional(&mut *self.conn)
                    .await?;
            out.push_str(&self.thread_title_toast_template(&username.unwrap_or_default()));
        }
        out.push_str("</div></div>");
        if drawer.is_some() {
            out.push_str("</div>");
        }
        Ok(out)
    }

    /// ChatModalThreadSettings for chat-thread.js to show in a modal: the
    /// header's save button (headerPrimaryAction, which DModal draws on a
    /// phone), the title input with its length, Discourse AI's call to
    /// action for admins, and the footer's save button.
    fn thread_settings_template(&self, thread: &ThreadRow) -> String {
        let i18n = self.i18n;
        let t = move |key: &str| js(i18n, key);
        let title = thread.title.clone().unwrap_or_default();
        let ai = if self.guardian.is_admin() {
            format!(
                "<div class=\"discourse-ai-cta\"><p class=\"discourse-ai-cta__title\">{} {}</p><p class=\"discourse-ai-cta__description\">{}</p></div>",
                d_icon("circle-info", None),
                escape(&t("chat.thread_title_modal.discourse_ai.title")),
                escape(&t("chat.thread_title_modal.discourse_ai.description")).replace(
                    "%{url}",
                    "<a href='https://www.discourse.org/ai' rel='noopener noreferrer' target='_blank'>Discourse AI</a>"
                )
            )
        } else {
            String::new()
        };
        format!(
            "<template class=\"chat-modal-thread-settings-template\" data-title=\"{}\"><div class=\"chat-modal-thread-settings__header-primary-action\"><button class=\"btn btn-text btn-transparent --primary\" type=\"button\"><span class=\"d-button-label\">{}</span></button></div><div class=\"chat-modal-thread-settings__body\"><input class=\"chat-modal-thread-settings__title-input\" maxlength=\"100\" name=\"thread-title\" placeholder=\"{}\" type=\"text\" value=\"{}\"><div class=\"thread-title-length\"><span>{}</span>/100</div>{ai}</div><div class=\"chat-modal-thread-settings__footer\"><button class=\"btn btn-text btn-primary\" type=\"button\"><span class=\"d-button-label\">{}</span></button></div></template>",
            escape(&t("chat.thread_title_modal.title")),
            escape(&t("chat.save")),
            escape(&t("chat.thread_title_modal.input_placeholder")),
            escape(&title),
            title.chars().count(),
            escape(&t("save")),
        )
    }

    /// ChatThreadTitlePrompt's toast (toasts.default, long, with its
    /// progress bar): don't show again, or set the title.
    /// `username`: whose user option "don't show again" turns off.
    fn thread_title_toast_template(&self, username: &str) -> String {
        let i18n = self.i18n;
        let t = move |key: &str| js(i18n, key);
        format!(
            "<template class=\"chat-thread-title-toast-template\" data-username=\"{}\"><output class=\"fk-d-toast thread-toast\" role=\"status\"><div class=\"fk-d-default-toast\"><div class=\"fk-d-default-toast__progress-bar\"></div><div class=\"fk-d-default-toast__main-container\"><div class=\"fk-d-default-toast__texts\"><div class=\"fk-d-default-toast__title\">{}</div><div class=\"fk-d-default-toast__message\">{}</div></div><div class=\"fk-d-default-toast__actions\"><button class=\"btn btn-text btn-link toast-hide\" type=\"button\"><span class=\"d-button-label\">{}</span></button><button class=\"btn btn-text btn-primary toast-action\" type=\"button\"><span class=\"d-button-label\">{}</span></button></div></div><div class=\"fk-d-default-toast__close-container\"><button class=\"btn no-text btn-icon btn-transparent\" type=\"button\">{}<span aria-hidden=\"true\">&#8203;</span></button></div></div></output></template>",
            escape(username),
            escape(&t("chat.thread_title_toast.title")),
            escape(&t("chat.thread_title_toast.message")),
            escape(&t("chat.thread_title_toast.dismiss_action")),
            escape(&t("chat.thread_title_toast.primary_action")),
            d_icon("xmark", None),
        )
    }

    /// A page of a thread's messages as chat-thread.js asks for one, as
    /// Context::messages_page draws the channel's: `direction` past or
    /// future from `target` (the first or last message the viewer has),
    /// the target drawn again after the past ones; else the page around
    /// `target` or from the first message. None when the viewer can't see
    /// the thread or the target is gone.
    pub async fn thread_messages_page(
        &mut self,
        channel_id: i64,
        thread_id: i64,
        target: Option<i64>,
        direction: Option<&str>,
    ) -> Result<Option<MessagesPage>, AppError> {
        let channel = match self.find_joinable(&channel_id.to_string()).await? {
            Found::Channel(channel) => channel,
            Found::NotFound | Found::Forbidden => return Ok(None),
        };
        let Some(thread) = self.shown_thread(&channel, thread_id).await? else {
            return Ok(None);
        };
        let direction = direction.filter(|d| *d == "past" || *d == "future");
        let params = ThreadListParams {
            page_size: PAGE_SIZE,
            target_message_id: target,
            direction: match (direction, target) {
                (Some(d), _) => Some(d.to_string()),
                (None, None) => Some("future".into()),
                (None, Some(_)) => None,
            },
            fetch_from_last_read: false,
            fetch_from_last_message: false,
            // Without a target, from the first message, not the member's
            // last read one.
            fetch_from_first_message: target.is_none(),
            target_date: None,
        };
        let listed = match self
            .list_thread_messages(channel.id, thread.id, &params)
            .await?
        {
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
            prioritize_name: super::page::prioritize_name(self.settings)?,
            admin: self.guardian.is_admin(),
            t: &t,
            i18n: self.i18n,
            relative_date_duration: self.settings.get("relative_date_duration")?.to_i(),
            thread_context: true,
        };
        let mut html = messages_html(&view, &messages, range, None, last_read)?;
        if let Some(index) = redraw {
            html.push_str(&format!(
                "<template class=\"chat-message-anchor\" data-id=\"{}\">{}</template>",
                messages[index]["id"],
                super::page::message_html(&view, &messages, index, None, last_read, false)?
            ));
        }
        Ok(Some(MessagesPage {
            html,
            can_load_more_past: listed["meta"]["can_load_more_past"].clone(),
            can_load_more_future: listed["meta"]["can_load_more_future"].clone(),
        }))
    }

    /// ChatThreadHeader: the back button (to the channel's threads while
    /// some are unread, with their count, else to the channel), the title
    /// (the thread's, else "Thread"; clickable for admins and the original
    /// message's author), the tracking dropdown and settings button for
    /// members, and the close button on desktop. `unread` is None in do
    /// not disturb; `tracking` is unread_threads_tracking's attributes.
    fn thread_header(
        &self,
        c: &Value,
        outlet: &str,
        thread: &ThreadRow,
        level: Option<i32>,
        original_user: Option<i64>,
        (unread, tracking): (Option<usize>, &str),
    ) -> String {
        let i18n = self.i18n;
        let t = move |key: &str| js(i18n, key);
        let slug = super::page::slugified_title(c);
        let channel_url = format!("{}/chat/c/{}/{}", self.base_path, escape(&slug), c["id"]);
        let mut out = String::from("<div class=\"c-navbar-container\"><nav class=\"c-navbar\">");
        if c["threading_enabled"] == true {
            // backLink without a previous route: the threads while some
            // are unread (the member not in do not disturb).
            let back = if unread.is_some_and(|n| n > 0) {
                ThreadBack::Threads
            } else {
                ThreadBack::Channel
            };
            // ChatThreadHeaderUnreadIndicator, hidden in do not disturb.
            let unread_indicator = match unread {
                Some(n) => super::page::thread_header_unread_indicator(self.i18n, n),
                None => String::new(),
            };
            let (href, title, indicator) = match back {
                ThreadBack::Threads => (
                    format!("{channel_url}/t"),
                    t("chat.return_to_threads_list"),
                    unread_indicator.clone(),
                ),
                ThreadBack::Channel => (
                    channel_url.clone(),
                    t("chat.return_to_channel"),
                    String::new(),
                ),
            };
            // What chat-thread.js picks the back link from with the
            // previous chat route (ChatHistory).
            out.push_str(&format!(
                "<a class=\"c-navbar__back-button no-text btn-transparent btn\" title=\"{}\" href=\"{href}\"{tracking} data-channel-href=\"{channel_url}\" data-threads-href=\"{channel_url}/t\" data-my-threads-href=\"{}/chat/threads\" data-label-channel=\"{}\" data-label-threads=\"{}\" data-label-my-threads=\"{}\" data-unread-indicator=\"{}\">{indicator}{}</a>",
                escape(&title),
                self.base_path,
                escape(&t("chat.return_to_channel")),
                escape(&t("chat.return_to_threads_list")),
                escape(&t("chat.my_threads.title")),
                escape(&unread_indicator),
                d_icon("chevron-left", None)
            ));
        }
        let title = thread
            .title
            .clone()
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| t("chat.thread.label"));
        let viewer = self.guardian.user_id().map(i64::from);
        let editable = self.guardian.is_admin() || (viewer.is_some() && viewer == original_user);
        let title_html = crate::topic_list_view::emoji_unescape(
            &escape(&title),
            self.settings.emojis(),
            self.base_path,
        );
        out.push_str(&format!(
            "<div class=\"c-navbar__title{}\" title=\"{}\"{}><span class=\"c-navbar__title-text\">{}</span></div>",
            if editable { " clickable" } else { "" },
            escape(&title_html),
            if editable { " role=\"button\"" } else { "" },
            title_html
        ));
        out.push_str("<nav class=\"c-navbar__actions\">");
        out.push_str(outlet);
        if self.guardian.is_authenticated() {
            out.push_str(&thread_tracking_dropdown(
                self.i18n,
                self.base_path,
                c["id"].as_i64().unwrap_or(0),
                thread.id,
                // threadNotificationLevel: the membership's, else normal.
                level.filter(|l| *l != 0).unwrap_or(super::threads::NORMAL),
            ));
            // ThreadSettingsButton: staff and the original message's author.
            if self.guardian.is_staff() || (viewer.is_some() && viewer == original_user) {
                out.push_str(&format!(
                    "<button class=\"btn no-text btn-icon btn-transparent c-navbar__thread-settings-button\" title=\"{}\" type=\"button\">{}<span aria-hidden=\"true\">&#8203;</span></button>",
                    escape(&t("chat.thread.settings")),
                    d_icon("gear", None)
                ));
            }
        }
        if !crate::mobile::mobile_view() {
            out.push_str(&format!(
                "<a class=\"c-navbar__close-thread-button btn-transparent btn btn-icon no-text\" title=\"{}\" href=\"{channel_url}\">{}</a>",
                escape(&t("chat.thread.close")),
                d_icon("xmark", None)
            ));
        }
        out.push_str("</nav></nav></div>");
        out
    }

    /// The channel threads route: ChatThreadListHeader (the back button on
    /// a phone, the title, the close button on desktop) and
    /// ChatThreadList: the member's threads of the channel as
    /// GET /chat/api/channels/:id/threads lists them, or its empty label.
    pub async fn channel_threads_page(&mut self, channel: &ChannelRow) -> Result<String, AppError> {
        self.channel_threads_page_in(channel, None).await
    }

    /// The channel threads route, or in the drawer
    /// (ChatDrawerRoutesChannelThreads) the drawer's `navbar` in place of
    /// ChatThreadListHeader and ChatThreadList in the drawer's content.
    pub async fn channel_threads_page_in(
        &mut self,
        channel: &ChannelRow,
        drawer: Option<&str>,
    ) -> Result<String, AppError> {
        let membership = self
            .memberships()
            .await?
            .into_iter()
            .find(|m| m.chat_channel_id == channel.id);
        let channel_json = self
            .channel(channel, membership.as_ref(), None, None)
            .await?;
        let i18n = self.i18n;
        let t = move |key: &str| js(i18n, key);
        let slug = super::page::slugified_title(&channel_json);
        let channel_url = format!("{}/chat/c/{}/{}", self.base_path, escape(&slug), channel.id);
        let mobile = crate::mobile::mobile_view();
        let mut out = String::from(
            "<div class=\"c-routes --channel-threads\"><div class=\"c-navbar-container\"><nav class=\"c-navbar\">",
        );
        if mobile {
            out.push_str(&format!(
                "<a class=\"c-navbar__back-button no-text btn-transparent btn\" title=\"{}\" href=\"{channel_url}\">{}</a>",
                escape(&t("chat.return_to_channel")),
                d_icon("chevron-left", None)
            ));
        }
        // ChatThreadListHeader#title: "Threads", with the channel's title
        // on a phone.
        let title = if mobile {
            format!(
                "{} - {}",
                escape(&t("chat.threads.list")),
                crate::topic_list_view::emoji_unescape(
                    &escape(s(&channel_json["title"])),
                    self.settings.emojis(),
                    self.base_path
                )
            )
        } else {
            escape(&t("chat.threads.list"))
        };
        out.push_str(&format!(
            "<div class=\"c-navbar__title\" title=\"{title}\"><span class=\"c-navbar__title-text\">{}{title}</span></div><nav class=\"c-navbar__actions\">",
            d_icon("discourse-threads", None)
        ));
        if !mobile {
            out.push_str(&format!(
                "<a class=\"c-navbar__close-threads-button btn-transparent btn btn-icon no-text\" title=\"{}\" href=\"{channel_url}\">{}</a>",
                escape(&t("chat.thread.close")),
                d_icon("xmark", None)
            ));
        }
        out.push_str("</nav></nav></div>");
        if let Some(navbar) = drawer {
            out = format!(
                "<div class=\"c-drawer-routes --channel-threads\">{navbar}<div class=\"chat-drawer-content\">"
            );
        }
        // The list's bus (/chat/:id) and its pages, for chat-thread.js.
        out.push_str(&format!(
            "<div class=\"chat-thread-list\" data-channel-id=\"{}\"><div class=\"chat-thread-list__items\">",
            channel.id
        ));
        let (items, count) = self.channel_thread_items(channel, &channel_url, 0).await?;
        if count == 0 {
            out.push_str(&format!(
                "<div class=\"chat-thread-list__no-threads\">{}</div>",
                escape(&t("chat.threads.none"))
            ));
        }
        out.push_str(&items);
        let viewer = self.list_viewer_attrs().await?;
        out.push_str(&format!(
            "<div class=\"chat-thread-list__load-more\" data-load-more-url=\"{}/live/chat/{}/threads\" data-offset=\"{count}\" data-done=\"{}\"{viewer}><br></div></div></div></div>",
            self.base_path,
            channel.id,
            count == 0,
        ));
        if drawer.is_some() {
            out.push_str("</div>");
        }
        Ok(out)
    }

    /// A page of the channel's threads from `offset` as ChatThreadList
    /// draws its items (GET /chat/api/channels/:id/threads), with how many.
    pub async fn channel_thread_items(
        &mut self,
        channel: &ChannelRow,
        channel_url: &str,
        offset: i64,
    ) -> Result<(String, usize), AppError> {
        let threads = match self.channel_threads(channel.id, None, Some(offset)).await? {
            Outcome::Done(v) => v,
            _ => serde_json::json!({ "threads": [] }),
        };
        let list = threads["threads"].as_array().cloned().unwrap_or_default();
        let mut out = String::new();
        for thread in &list {
            out.push_str(&self.thread_list_item(channel_url, thread, &threads["tracking"])?);
        }
        Ok((out, list.len()))
    }

    /// GET /live/chat/:channel_id/threads: chat-thread.js's next page of
    /// the channel's threads, None when the viewer can't see them.
    pub async fn channel_threads_more(
        &mut self,
        channel_id: i64,
        offset: i64,
    ) -> Result<Option<(String, usize)>, AppError> {
        let channel = match self.find_joinable(&channel_id.to_string()).await? {
            Found::Channel(channel) if channel.threading_enabled => channel,
            _ => return Ok(None),
        };
        let membership = self
            .memberships()
            .await?
            .into_iter()
            .find(|m| m.chat_channel_id == channel.id);
        let channel_json = self
            .channel(&channel, membership.as_ref(), None, None)
            .await?;
        let slug = super::page::slugified_title(&channel_json);
        let channel_url = format!("{}/chat/c/{}/{}", self.base_path, escape(&slug), channel.id);
        Ok(Some(
            self.channel_thread_items(&channel, &channel_url, offset)
                .await?,
        ))
    }

    /// ChatThreadListItem: the title (the thread's, else its original
    /// message's excerpt) with its unread indicator, the original
    /// message's author and the other participants, and when it was last
    /// replied to; unread and urgent as its tracking says.
    fn thread_list_item(
        &self,
        channel_url: &str,
        thread: &Value,
        tracking: &Value,
    ) -> Result<String, AppError> {
        let i18n = self.i18n;
        let t = move |key: &str| js(i18n, key);
        let id = thread["id"].as_i64().unwrap_or(0);
        let tracked = &tracking[id.to_string()];
        let unread = tracked["unread_count"].as_i64().unwrap_or(0);
        let watched = tracked["watched_threads_unread_count"]
            .as_i64()
            .unwrap_or(0);
        let emojis = self.settings.emojis();
        let title = match thread["title"].as_str().filter(|t| !t.is_empty()) {
            Some(title) => {
                crate::topic_list_view::emoji_unescape(&escape(title), emojis, self.base_path)
            }
            None => crate::topic_list_view::emoji_unescape(
                s(&thread["original_message"]["excerpt"]),
                emojis,
                self.base_path,
            ),
        };
        let original_user = &thread["original_message"]["user"];
        // ChatThreadParticipants without the original message's author.
        let others: Vec<Value> = thread["preview"]["participant_users"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|u| u["id"] != original_user["id"])
            .cloned()
            .collect();
        let participants = if others.is_empty() {
            String::new()
        } else {
            let mut html = String::from(
                "<div class=\"chat-thread-participants chat-thread-list-item__participants\"><div class=\"chat-thread-participants__avatar-group\">",
            );
            for user in &others {
                html.push_str(&static_user_avatar(user, 24, &[]));
            }
            html.push_str("</div>");
            let count = thread["preview"]["participant_count"].as_i64().unwrap_or(0)
                - thread["preview"]["participant_users"]
                    .as_array()
                    .map_or(0, |u| u.len() as i64);
            if count != 0 {
                html.push_str(&format!(
                    "<div class=\"chat-thread-participants__other-count\">{}</div>",
                    escape(
                        &self
                            .i18n
                            .t_count(
                                "js.chat.thread.participants_other_count",
                                count,
                                &[("count", &count.to_string())]
                            )
                            .unwrap_or_default()
                    )
                ));
            }
            html.push_str("</div>");
            html
        };
        let last_reply = thread["preview"]["last_reply_created_at"]
            .as_str()
            .map(|at| {
                super::page::format_date_ago(
                    self.i18n,
                    self.settings
                        .get("relative_date_duration")
                        .map(|v| v.to_i())
                        .unwrap_or(30),
                    at,
                )
            })
            .unwrap_or_default();
        Ok(format!(
            "<div class=\"chat-thread-list-item{}{}\" data-thread-id=\"{id}\"{}><div class=\"chat-thread-list-item__main\"><div class=\"chat-thread-list-item__open-button\" role=\"button\" title=\"{}\" data-href=\"{channel_url}/t/{id}\"><div class=\"chat-thread-list-item__header\"><div class=\"chat-thread-list-item__title\">{title}</div><div class=\"chat-thread-list-item__unread-indicator\">{}</div></div><div class=\"chat-thread-list-item__metadata\"><div class=\"chat-thread-list-item__members\">{}{participants}</div><div class=\"chat-thread-list-item__last-reply-timestamp\">{last_reply}</div></div></div></div></div>",
            if unread > 0 { " -is-unread" } else { "" },
            if watched > 0 { " -is-urgent" } else { "" },
            tracking_attr(thread, tracked),
            escape(&t("chat.thread.view_thread")),
            thread_unread_indicator(tracked),
            static_user_avatar(original_user, 24, &[]),
        ))
    }

    /// UserThreads for the threads route: each of the member's threads
    /// (GET /chat/api/me/threads) with its channel's title and its own; on
    /// desktop the original message's thread indicator, on a phone the
    /// channel's icon and the last reply's preview, linking to the thread.
    /// Without any, the list's empty state.
    pub async fn user_threads_list(&mut self) -> Result<String, AppError> {
        let i18n = self.i18n;
        let t = move |key: &str| js(i18n, key);
        let (items, count) = self.user_thread_items(0).await?;
        if count == 0 {
            return Ok(format!(
                "<div class=\"c-list\"><div class=\"--loaded c-user-threads\"><div class=\"c-list-empty-state\"><div class=\"empty-state-threads\">{}</div></div></div><div><br></div><div class=\"loading-container\"></div></div>",
                escape(&t("chat.empty_state.my_threads"))
            ));
        }
        let viewer = self.list_viewer_attrs().await?;
        Ok(format!(
            "<div class=\"c-list\"><div class=\"--loaded c-user-threads\">{items}</div><div class=\"c-user-threads__load-more\" data-load-more-url=\"{}/live/chat/threads\" data-offset=\"{count}\"{viewer}><br></div><div class=\"loading-container\"></div></div>",
            self.base_path,
        ))
    }

    /// The viewer as ChatSubscriptionsManager's thread tracking reads it,
    /// for chat-thread.js: their id (their own replies leave a thread
    /// read), the users they ignore (whose replies do too), and the
    /// channels it hears new messages on (the followed ones).
    async fn list_viewer_attrs(&mut self) -> Result<String, AppError> {
        let Some(user_id) = self.guardian.user_id() else {
            return Ok(String::new());
        };
        let ignored: Vec<String> = sqlx::query_scalar(
            "SELECT users.username FROM ignored_users INNER JOIN users ON users.id = ignored_users.ignored_user_id \
             WHERE ignored_users.user_id = $1 ORDER BY ignored_users.id",
        )
        .bind(user_id)
        .fetch_all(&mut *self.conn)
        .await?;
        let mut followed: Vec<i64> = self
            .memberships()
            .await?
            .into_iter()
            .filter(|m| m.following)
            .map(|m| m.chat_channel_id)
            .collect();
        followed.sort_unstable();
        Ok(format!(
            " data-viewer-id=\"{user_id}\" data-ignored-users=\"{}\" data-followed=\"{}\"",
            escape(&serde_json::Value::from(ignored).to_string()),
            escape(&serde_json::Value::from(followed).to_string()),
        ))
    }

    /// A page of the member's threads from `offset` as UserThreads draws
    /// them (GET /chat/api/me/threads), with how many: each `.c-user-thread`
    /// carries its channel for chat-thread.js's live updates.
    pub async fn user_thread_items(&mut self, offset: i64) -> Result<(String, usize), AppError> {
        let i18n = self.i18n;
        let t = move |key: &str| js(i18n, key);
        let listed = match self.user_threads(None, Some(offset)).await? {
            Outcome::Done(v) => v,
            _ => serde_json::json!({ "threads": [] }),
        };
        let threads = listed["threads"].as_array().cloned().unwrap_or_default();
        if threads.is_empty() {
            return Ok((String::new(), 0));
        }
        let mobile = crate::mobile::mobile_view();
        let emojis = self.settings.emojis().clone();
        let online = self.online_ids().await?;
        let duration = self.settings.get("relative_date_duration")?.to_i();
        let mut out = String::new();
        for thread in &threads {
            let id = thread["id"].as_i64().unwrap_or(0);
            let c = &thread["channel"];
            let channel_url = format!(
                "{}/chat/c/{}/{}",
                self.base_path,
                escape(&super::page::slugified_title(c)),
                c["id"]
            );
            let tracked = &listed["tracking"][id.to_string()];
            // ChannelTitle (with the channel's icon for a DM) and ThreadTitle.
            let channel_title = super::page::channel_title(c, &emojis, self.base_path, &online);
            let name = match thread["title"].as_str().filter(|t| !t.is_empty()) {
                Some(title) => {
                    crate::topic_list_view::emoji_unescape(&escape(title), &emojis, self.base_path)
                }
                None => crate::topic_list_view::emoji_unescape(
                    s(&thread["original_message"]["excerpt"]),
                    &emojis,
                    self.base_path,
                ),
            };
            let thread_title = format!(
                "<span class=\"chat__thread-title-container\"><span class=\"chat__thread-title\">{}</span>{}</span>",
                if mobile {
                    format!("<span class=\"chat__thread-title__name\">{name}</span>")
                } else {
                    format!(
                        "<a class=\"chat__thread-title__name\" href=\"{channel_url}/t/{id}\">{name}</a>"
                    )
                },
                thread_unread_indicator(tracked)
            );
            let preview = &thread["preview"];
            out.push_str(&format!(
                "<div class=\"c-user-thread\" data-id=\"{id}\" data-channel-id=\"{}\"{}>",
                c["id"],
                tracking_attr(thread, tracked)
            ));
            if mobile {
                // ChannelIcon for a thread: its last replier and the threads flair.
                out.push_str(&format!(
                    "<div class=\"chat-channel-icon --avatar\">{}<div class=\"avatar-flair --threads\">{}</div></div>",
                    super::page::user_avatar_link(&preview["last_reply_user"], 24, self.base_path),
                    d_icon("discourse-threads", None)
                ));
                out.push_str(&format!(
                    "<a class=\"c-user-thread__link\" href=\"{channel_url}/t/{id}\">{channel_title}{thread_title}<span class=\"chat-message-thread-indicator__last-reply-timestamp\">{}</span><span class=\"c-user-thread__excerpt\"><span class=\"c-user-thread__excerpt-poster\">{}</span><span>:</span><span class=\"c-user-thread__excerpt-text\">{}</span></span></a>",
                    super::page::format_date_ago(self.i18n, duration, s(&preview["last_reply_created_at"])),
                    escape(s(&preview["last_reply_user"]["username"])),
                    crate::topic_list_view::emoji_unescape(s(&preview["last_reply_excerpt"]), &emojis, self.base_path)
                ));
            } else {
                out.push_str(&channel_title);
                out.push_str(&thread_title);
                // ThreadIndicator, not interactive (tabindex -1, a static
                // avatar for the last replier).
                let t_count = move |key: &str, count: i64| {
                    i18n.t_count(
                        &format!("js.{key}"),
                        count,
                        &[("count", &count.to_string())],
                    )
                    .unwrap_or_default()
                };
                let participants = super::page::thread_participants_html(preview, &t_count, &|u| {
                    static_user_avatar(u, 24, &[])
                });
                out.push_str(&format!(
                    "<a class=\"chat-message-thread-indicator\" tabindex=\"-1\" title=\"{}\" href=\"{channel_url}/t/{id}/{}\"><div class=\"chat-message-thread-indicator__last-reply-avatar\">{}</div><div class=\"chat-message-thread-indicator__last-reply-info\"><span class=\"chat-message-thread-indicator__last-reply-username\">{}</span><span class=\"chat-message-thread-indicator__last-reply-timestamp\">{}</span></div><div class=\"chat-message-thread-indicator__replies-count\">{}</div>{participants}<div class=\"chat-message-thread-indicator__last-reply-excerpt\">{}</div></a>",
                    escape(&t("chat.threads.open")),
                    preview["last_reply_id"],
                    static_user_avatar(&preview["last_reply_user"], 24, &online),
                    escape(s(&preview["last_reply_user"]["username"])),
                    super::page::format_date_ago(self.i18n, duration, s(&preview["last_reply_created_at"])),
                    escape(&t_count("chat.thread.replies", preview["reply_count"].as_i64().unwrap_or(0))),
                    crate::topic_list_view::emoji_unescape(s(&preview["last_reply_excerpt"]), &emojis, self.base_path)
                ));
            }
            out.push_str("</div>");
        }
        Ok((out, threads.len()))
    }
}

/// What chat-thread.js keeps of a listed thread's tracking, as Ember's
/// models hold it (thread.tracking, thread.currentUserMembership,
/// thread.preview.lastReplyCreatedAt): the counts the lists follow on the
/// bus and the last reply the channel's list sorts by.
fn tracking_attr(thread: &Value, tracked: &Value) -> String {
    let count = |key: &str| tracked[key].as_i64().unwrap_or(0);
    let membership = &thread["current_user_membership"];
    let membership = if membership.is_object() {
        serde_json::json!({
            "notification_level": membership["notification_level"],
            "last_read_message_id": membership["last_read_message_id"],
        })
    } else {
        Value::Null
    };
    let data = serde_json::json!({
        "unread_count": count("unread_count"),
        "mention_count": count("mention_count"),
        "watched_threads_unread_count": count("watched_threads_unread_count"),
        "last_reply_created_at": thread["preview"]["last_reply_created_at"],
        "membership": membership,
    });
    format!(" data-tracking=\"{}\"", escape(&data.to_string()))
}

/// ThreadUnreadIndicator: the urgent count (mentions and watched replies)
/// when any, else the unread one, capped at 99; none without either.
fn thread_unread_indicator(tracked: &Value) -> String {
    let unread = tracked["unread_count"].as_i64().unwrap_or(0);
    let urgent = tracked["mention_count"].as_i64().unwrap_or(0)
        + tracked["watched_threads_unread_count"]
            .as_i64()
            .unwrap_or(0);
    if unread <= 0 && urgent <= 0 {
        return String::new();
    }
    let count = if urgent > 0 { urgent } else { unread };
    format!(
        "<span class=\"chat-thread-list-item-unread-indicator {}\"><span class=\"chat-thread-list-item-unread-indicator__number\">{}</span></span>",
        if urgent > 0 { "-urgent" } else { "" },
        if count > 99 {
            "99+".to_string()
        } else {
            count.to_string()
        }
    )
}

/// ThreadNotificationsTracking (NotificationsTracking with the thread's
/// levels: watching, tracking, normal), each level saved to
/// PUT .../threads/:thread_id/notifications-settings/me.
fn thread_tracking_dropdown(
    i18n: &crate::i18n::I18n,
    base_path: &str,
    channel_id: i64,
    thread_id: i64,
    level: i32,
) -> String {
    const LEVELS: [(i32, &str, &str); 3] = [
        (3, "watching", "d-watching"),
        (2, "tracking", "d-tracking"),
        (1, "regular", "d-regular"),
    ];
    let t = |key: &str| i18n.t(&format!("js.{key}")).unwrap_or_default().to_string();
    let title = |key: &str| t(&format!("chat.thread.notifications.{key}.title"));
    let (_, key, icon_name) = LEVELS
        .iter()
        .find(|(l, _, _)| *l == level)
        .copied()
        .unwrap_or(LEVELS[2]);
    let tooltip = |key: &str| {
        i18n.t_with(
            "js.notifications_tracking.tooltip",
            &[("level", &title(key))],
        )
        .unwrap_or_default()
    };
    let items: String = LEVELS
        .iter()
        .map(|(l, key, icon_name)| {
            format!(
                "<li class=\"dropdown-menu__item\"><button class=\"btn no-text notifications-tracking-btn{}\" data-level-id=\"{l}\" data-level-name=\"{key}\" data-title=\"{}\" data-tooltip=\"{}\" hx-put=\"{base_path}/chat/api/channels/{channel_id}/threads/{thread_id}/notifications-settings/me\" hx-vals='{{\"notification_level\": {l}}}' hx-swap=\"none\" type=\"button\"><div class=\"notifications-tracking-btn__icons\">{}</div><div class=\"notifications-tracking-btn__texts\"><span class=\"notifications-tracking-btn__label\">{}</span><span class=\"notifications-tracking-btn__description\">{}</span></div></button></li>",
                if *l == level { " -selected" } else { "" },
                escape(&title(key)),
                escape(&tooltip(key)),
                d_icon(icon_name, None),
                escape(&title(key)),
                escape(&t(&format!("chat.thread.notifications.{key}.description"))),
            )
        })
        .collect();
    format!(
        "<button class=\"btn no-text fk-d-menu__trigger notifications-tracking-trigger btn-transparent btn-icon notifications-tracking-trigger-btn thread-notifications-tracking c-navbar__thread-tracking-dropdown -persisted\" title=\"{}\" aria-expanded=\"false\" data-identifier=\"notifications-tracking\" data-trigger=\"\" data-level-id=\"{level}\" data-level-name=\"{key}\">{}</button><div class=\"fk-d-menu notifications-tracking-content -animated\" data-content=\"\" data-identifier=\"notifications-tracking\" role=\"dialog\" data-strategy=\"absolute\" data-placement=\"bottom-end\" hidden><div class=\"fk-d-menu__inner-content\"><ul class=\"dropdown-menu\">{items}</ul></div></div>",
        escape(&tooltip(key)),
        d_icon(icon_name, None)
    )
}
