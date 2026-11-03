//! InviteMailer (app/mailers/invite_mailer.rb) and the job that sends
//! it (`Jobs::InviteEmail`): the email inviting someone to the forum, or
//! to the topic they were invited to.

use serde_json::Value;

use crate::invites::{Invite, emailed_status};
use crate::{AppError, AppState, Unsupported};

pub const INVITE_EMAIL: &str = "invite_email";

/// Ruby truthiness of a job argument.
fn truthy(v: Option<&Value>) -> bool {
    !matches!(v, None | Some(Value::Null) | Some(Value::Bool(false)))
}

/// `ActionView::Base.full_sanitizer.sanitize(message.gsub(/\n+/, " ").strip)`:
/// plain text passes as it is; markup and entities are refused.
fn sanitized_message(message: &str) -> Result<String, AppError> {
    static NEWLINES: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"\n+").unwrap());
    let message = NEWLINES.replace_all(message, " ");
    let message = message.trim();
    if message.contains(['<', '>', '&']) {
        return Err(Unsupported("invite messages with markup (full_sanitizer)").into());
    }
    Ok(message.to_string())
}

/// `Jobs::InviteEmail`
pub async fn invite_email(state: &AppState, args: &Value) -> Result<(), AppError> {
    let Some(invite_id) = args.get("invite_id").and_then(Value::as_i64) else {
        return Err(Unsupported("invite_email without an invite_id (InvalidParameters)").into());
    };
    let mut conn = state.pool.acquire().await?;
    let s = crate::site_settings::SiteSettings::load(
        &mut conn,
        &state.site_setting_defs,
        &state.config.globals,
    )
    .await?;
    let Some(invite) = Invite::find(&mut conn, invite_id as i32).await? else {
        return Ok(());
    };
    if !s.get("allow_email_invites")?.truthy() {
        return Ok(());
    }
    let host = crate::pretty_text::Host::from_state(state);
    let ctx = crate::email::sender::Ctx {
        host: &host,
        settings: &s,
        config: &state.config,
        i18n: &state.i18n,
        mailer: &state.mailer,
    };
    let urls = crate::url::Urls {
        config: &state.config,
        settings: &s,
    };
    // The first topic they were invited to, by when it was created.
    let first_topic: Option<(i32, String, Option<String>)> = sqlx::query_as(
        "SELECT t.id, t.title, t.excerpt FROM topic_invites ti JOIN topics t ON t.id = ti.topic_id \
         WHERE ti.invite_id = $1 AND t.deleted_at IS NULL ORDER BY t.created_at LIMIT 1",
    )
    .bind(invite.id)
    .fetch_optional(&mut *conn)
    .await?;
    let (username, name): (String, Option<String>) =
        sqlx::query_as("SELECT username, name FROM users WHERE id = $1")
            .bind(invite.invited_by_id)
            .fetch_one(&mut *conn)
            .await?;
    let inviter_name = match name.filter(|n| !crate::ruby::is_blank(n)) {
        Some(name) if s.get("enable_names")?.truthy() => format!("{name} ({username})"),
        _ => username,
    };
    let custom = match invite
        .custom_message
        .as_deref()
        .filter(|m| !crate::ruby::is_blank(m))
    {
        Some(m) => Some(sanitized_message(m)?),
        None => None,
    };
    // Invite.email_code_enabled? with no user.
    let code =
        crate::invites::email_code_enabled(&mut conn, &s, &crate::guardian::Guardian::anonymous())
            .await?;
    let invite_link = invite.link(&urls.base_url()?, !code);
    let hostname = urls.current_hostname()?;
    let site_description = s.get("site_description")?.to_s();
    let site_title = s.get("title")?.to_s();
    let to = invite.email.clone().unwrap_or_default();
    let mut extra: Vec<(&str, String)> = vec![
        ("inviter_name", inviter_name),
        ("site_domain_name", hostname),
        ("invite_link", invite_link),
    ];
    let template = match first_topic {
        Some((topic_id, title, excerpt)) if truthy(args.get("invite_to_topic")) => {
            let (title, excerpt) = if s.get("private_email")?.truthy() {
                let id = topic_id.to_string();
                (
                    state
                        .i18n
                        .t_with("system_messages.private_topic_title", &[("id", &id)])
                        .unwrap_or_default(),
                    String::new(),
                )
            } else {
                (title, excerpt.unwrap_or_default().replace('\n', " "))
            };
            extra.push(("topic_title", title));
            extra.push(("topic_excerpt", excerpt));
            if custom.is_some() {
                "custom_invite_mailer"
            } else {
                "invite_mailer"
            }
        }
        _ => {
            if custom.is_some() {
                "custom_invite_forum_mailer"
            } else {
                "invite_forum_mailer"
            }
        }
    };
    extra.push(("site_description", site_description));
    extra.push(("site_title", site_title));
    if let Some(custom) = custom {
        extra.push(("user_custom_message", custom));
    }
    let extra: Vec<(&str, &str)> = extra.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let built = crate::email::account::build_template(&ctx, template, &to, &extra, None).await?;
    let mut tx = state.pool.begin().await?;
    crate::email::sender::send(&mut tx, &ctx, built, "invite", None).await?;
    if invite.emailed_status != Some(emailed_status::NOT_REQUIRED) {
        sqlx::query("UPDATE invites SET emailed_status = $2 WHERE id = $1")
            .bind(invite.id)
            .bind(emailed_status::SENT)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub const INVITE_PASSWORD_INSTRUCTIONS_EMAIL: &str = "invite_password_instructions_email";

/// `Jobs::InvitePasswordInstructionsEmail`: InviteMailer's
/// send_password_instructions, a password reset token in a link to set
/// one.
pub async fn invite_password_instructions_email(
    state: &AppState,
    args: &Value,
) -> Result<(), AppError> {
    let Some(login) = args
        .get("username")
        .and_then(Value::as_str)
        .filter(|u| !crate::ruby::is_blank(u))
    else {
        return Err(Unsupported(
            "invite_password_instructions_email without a username (InvalidParameters)",
        )
        .into());
    };
    let mut conn = state.pool.acquire().await?;
    let s = crate::site_settings::SiteSettings::load(
        &mut conn,
        &state.site_setting_defs,
        &state.config.globals,
    )
    .await?;
    let Some(user_id) = crate::pm_invite::find_by_username_or_email(&mut conn, login).await? else {
        return Err(Unsupported("password instructions for a user that is gone").into());
    };
    let email: String = sqlx::query_scalar(
        "SELECT email FROM user_emails WHERE user_id = $1 AND \"primary\" LIMIT 1",
    )
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?;
    let host = crate::pretty_text::Host::from_state(state);
    let ctx = crate::email::sender::Ctx {
        host: &host,
        settings: &s,
        config: &state.config,
        i18n: &state.i18n,
        mailer: &state.mailer,
    };
    let mut tx = state.pool.begin().await?;
    let token = crate::accounts::create_email_token(
        &mut tx,
        user_id,
        &email,
        crate::accounts::token_scopes::PASSWORD_RESET,
    )
    .await?;
    let built = crate::email::account::build_template(
        &ctx,
        "invite_password_instructions",
        &email,
        &[("email_token", &token)],
        None,
    )
    .await?;
    crate::email::sender::send(&mut tx, &ctx, built, "invite_password_instructions", None).await?;
    tx.commit().await?;
    Ok(())
}
