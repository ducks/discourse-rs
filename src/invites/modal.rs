//! What static/js/create-invite-modal.js (CreateInvite,
//! components/modal/create-invite.gjs) reads from its script tag: the
//! form's config for a member who can invite to the forum, the refusal
//! for one who cannot.

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::AppError;
use crate::guardian::Guardian;
use crate::i18n::I18n;
use crate::message_format::Arg;
use crate::site_settings::SiteSettings;

/// CreateInvite's expireAfterOptions: 1, 7, 30 and 90 days with
/// invite_expiry_days among them, then never.
fn expire_after_options(i18n: &I18n, expiry_days: i64) -> Vec<Value> {
    let mut days = vec![1, 7, 30, 90];
    if !days.contains(&expiry_days) {
        days.push(expiry_days);
    }
    days.sort_unstable();
    let mut options: Vec<Value> = days
        .into_iter()
        .map(|d| {
            json!({
                "value": d,
                "text": i18n
                    .t_count("js.dates.medium.x_days", d, &[("count", &d.to_string())])
                    .unwrap_or_default(),
            })
        })
        .collect();
    options.push(json!({
        "value": 999_999,
        "text": i18n.t("js.time_shortcut.never").unwrap_or_default(),
    }));
    options
}

/// The script tag's ` data-config="..."`: the strings, what the form
/// reads of the settings and the viewer (the redemption limits, the
/// default expiry, whether email invites go out, whether they choose
/// the topic arrived at, the groups they may invite to) and the link validity line (link_validity_MF) for the
/// defaults.
pub async fn script_attrs(
    conn: &mut PgConnection,
    i18n: &I18n,
    settings: &SiteSettings,
    guardian: &Guardian,
) -> Result<String, AppError> {
    const KEYS: &[&str] = &[
        "user.invited.invite.new_title",
        "user.invited.invite.edit_title",
        "user.invited.invite.copy_link_and_share_it",
        "user.invited.invite.create_link_to_invite",
        "user.invited.invite.edit_link_options",
        "user.invited.invite.edit_link_options_only",
        "user.invited.invite.description",
        "user.invited.invite.description_help",
        "user.invited.invite.restrict",
        "user.invited.invite.restrict_help",
        "user.invited.invite.email_or_domain_placeholder",
        "user.invited.invite.max_redemptions_allowed",
        "user.invited.invite.expires_after",
        "user.invited.invite.expires_at",
        "user.invited.invite.update_invite",
        "user.invited.invite.update_invite_and_send_email",
        "user.invited.invite.invite_to_topic",
        "user.invited.invite.add_to_groups",
        "user.invited.invite.custom_message",
        "user.invited.invite.custom_message_help",
        "user.invited.invite.custom_message_placeholder",
        "user.invited.invite.create_link",
        "user.invited.invite.create_link_and_send_email",
        "user.invited.invite.cancel",
        "user.invited.invite.copy_link",
        "user.invited.invite.link_copied",
        "user.invited.invite.share_link",
        "user.invited.invite.invite_saved_with_sending_email",
        "user.invited.invite.invite_saved_without_sending_email",
        "form_kit.errors.required",
        "form_kit.errors.too_long",
        "select_kit.default_header_text",
        "select_kit.no_content",
        "choose_topic.title.placeholder",
        "modal.close",
        "generic_error",
    ];
    let strings: Map<String, Value> = KEYS
        .iter()
        .map(|key| {
            let text = i18n.t(&format!("js.{key}")).unwrap_or(key);
            ((*key).to_string(), Value::from(text))
        })
        .collect();
    let staff = guardian.is_staff();
    let limit = settings
        .get(if staff {
            "invite_link_max_redemptions_limit"
        } else {
            "invite_link_max_redemptions_limit_users"
        })?
        .to_i();
    // defaultRedemptionsAllowed
    let default_redemptions = limit.min(if staff { 100 } else { 10 });
    let expiry_days = settings.get("invite_expiry_days")?.to_i();
    let validity = crate::message_format::format(
        i18n.t("js.user.invited.invite.link_validity_MF")
            .unwrap_or_default(),
        &[
            ("user_count", Arg::Num(default_redemptions)),
            ("duration_days", Arg::Num(expiry_days)),
        ],
    )
    .unwrap_or_default();
    // canInviteToGroup: staff, or an owner of a group they see.
    let user_id = guardian.user_id().unwrap_or(0);
    let owner: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM group_users WHERE user_id = $1 AND owner)",
    )
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?;
    // site.groups without the automatic ones.
    let groups: Vec<(i32, String)> = if staff || owner {
        sqlx::query_as(&format!(
            "SELECT g.id, g.name FROM groups g WHERE NOT g.automatic AND {} ORDER BY g.name",
            crate::groups::visible_groups_where(guardian, "g")
        ))
        .fetch_all(&mut *conn)
        .await?
    } else {
        Vec::new()
    };
    let icon = |name: &str| crate::topic_list_view::icon(name, None);
    let config = json!({
        "strings": strings,
        "validity": validity.trim(),
        "maxRedemptionsLimit": limit,
        "defaultRedemptions": default_redemptions,
        "expiryDays": expiry_days,
        "expireAfterOptions": expire_after_options(i18n, expiry_days),
        "allowEmailInvites": settings.get("allow_email_invites")?.truthy(),
        // canArriveAtTopic
        "canArriveAtTopic": staff && !settings.get("must_approve_users")?.truthy(),
        "canInviteToGroup": staff || owner,
        "groups": groups
            .iter()
            .map(|(id, name)| json!({ "id": id, "name": name }))
            .collect::<Vec<_>>(),
        "icons": {
            "copy": icon("copy"),
            "check": icon("check"),
            "share": icon("share"),
            "xmark": icon("xmark"),
            "angle-down": icon("angle-down"),
            "angle-up": icon("angle-up"),
        },
    });
    Ok(format!(
        " data-config=\"{}\"",
        crate::topic_list_view::escape(&config.to_string())
    ))
}

/// The script tag's attribute for a member who may not invite
/// (currentUser.can_invite_to_forum false): /new-invite tells them so in
/// a dialog (routes/new-invite.js).
pub fn refused_attrs(i18n: &I18n) -> String {
    let config = json!({
        "cannotInvite": true,
        "strings": {
            "user.invited.cannot_invite_to_forum": i18n
                .t("js.user.invited.cannot_invite_to_forum")
                .unwrap_or_default(),
        },
    });
    format!(
        " data-config=\"{}\"",
        crate::topic_list_view::escape(&config.to_string())
    )
}
