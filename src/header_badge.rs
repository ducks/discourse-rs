//! The header avatar's badge, as Header::UserDropdown::Notifications draws
//! it inside the toggle-current-user button: the do not disturb moon while
//! notifications are paused, else the first of the new message
//! notifications (envelope), the new reviewables (flag) and the unread
//! notifications count. Drawn with the page and again over the live stream
//! (routes/live.rs) from the member's `/notification`,
//! `/reviewable_counts` and `/do-not-disturb` messages, the fields
//! subscribe-user-notifications sets on the current user.

use chrono::NaiveDateTime;
use html_escape::encode_double_quoted_attribute as attr;
use sqlx::PgConnection;

use crate::AppError;
use crate::guardian::Guardian;
use crate::i18n::I18n;
use crate::site_settings::SiteSettings;
use crate::topic_list_view::icon;

/// What the badge reads of the current user.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Badge {
    /// `do_not_disturb_until`, for `isInDoNotDisturb`.
    pub do_not_disturb_until: Option<NaiveDateTime>,
    /// `new_personal_messages_notifications_count`
    pub new_pms: i64,
    /// `unseen_reviewable_count`, staff only (0 for everyone else).
    pub unseen_reviewables: i64,
    /// `all_unread_notifications_count`
    pub all_unread: i64,
}

impl Badge {
    /// The counts as CurrentUserSerializer gives them to the page.
    pub async fn load(
        conn: &mut PgConnection,
        settings: &SiteSettings,
        guardian: &Guardian,
    ) -> Result<Badge, AppError> {
        let Some(user_id) = guardian.user_id() else {
            return Ok(Badge::default());
        };
        let new_pms: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM notifications WHERE user_id = $1 \
               AND id > (SELECT seen_notification_id FROM users WHERE id = $1) \
               AND NOT read AND notification_type = 6",
        )
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
        let unseen_reviewables = if guardian.is_staff() {
            if settings.get("enable_category_group_moderation")?.truthy() {
                return Err(crate::Unsupported(
                    "enable_category_group_moderation (reviewable counts)",
                )
                .into());
            }
            crate::reviewables::staff_counts(&mut *conn, settings, guardian)
                .await?
                .1
        } else {
            0
        };
        Ok(Badge {
            do_not_disturb_until: crate::do_not_disturb::until(&mut *conn, user_id).await?,
            new_pms,
            unseen_reviewables,
            all_unread: crate::bus::all_unread_notifications_count(&mut *conn, user_id).await?,
        })
    }

    /// `currentUser.isInDoNotDisturb()`: an end still ahead.
    pub fn in_do_not_disturb(&self) -> bool {
        self.do_not_disturb_until
            .is_some_and(|until| until >= crate::clock::now_naive())
    }

    /// The badge's markup, empty when nothing applies.
    pub fn html(&self, i18n: &I18n) -> String {
        let t_count = |key: &str, count: i64| {
            i18n.t_count(&format!("js.{key}"), count, &[])
                .unwrap_or_else(|| key.to_string())
        };
        if self.in_do_not_disturb() {
            let title = i18n
                .t("js.notifications.paused")
                .unwrap_or("notifications.paused");
            return format!(
                r#"<div class="do-not-disturb-background" title="{}">{}</div>"#,
                attr(title),
                icon("discourse-dnd", None)
            );
        }
        if self.new_pms > 0 {
            let label = t_count(
                "notifications.tooltip.new_message_notification",
                self.new_pms,
            );
            return format!(
                r##"<a aria-label="{0}" class="badge-notification with-icon new-pms" href="#" title="{0}">{1}</a>"##,
                attr(&label),
                icon("envelope", None)
            );
        }
        if self.unseen_reviewables > 0 {
            let label = t_count(
                "notifications.tooltip.new_reviewable",
                self.unseen_reviewables,
            );
            return format!(
                r##"<a aria-label="{0}" class="badge-notification with-icon new-reviewables" href="#" title="{0}">{1}</a>"##,
                attr(&label),
                icon("flag", None)
            );
        }
        if self.all_unread > 0 {
            // `user.notifications` is given the count but has no plural
            // forms and doesn't show it.
            let label = i18n
                .t("js.user.notifications")
                .unwrap_or("user.notifications");
            let title = t_count("notifications.tooltip.regular", self.all_unread);
            return format!(
                r##"<a aria-label="{}" class="badge-notification unread-notifications" href="#" title="{}">{}</a>"##,
                attr(label),
                attr(&title),
                self.all_unread
            );
        }
        String::new()
    }
}

/// What the toggle-current-user button holds: the member's avatar, then
/// the badge.
pub fn toggle_contents(avatar_url: &str, badge: &str) -> String {
    format!(
        r#"<img alt="" width="48" height="48" src="{}" class="avatar">{badge}"#,
        attr(avatar_url)
    )
}
