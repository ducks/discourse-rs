//! The account emails of UserNotifications (`signup`, `forgot_password`,
//! `email_login`, ...): `build_user_email_token_by_template`, which gives
//! Email::MessageBuilder a template and the user's token and nothing else.
//! Without an HTML override, Email::Renderer cooks the text into the
//! layout.

use sqlx::PgConnection;

use super::Message;
use super::notification::{Built, layout, t};
use super::sender::Ctx;
use crate::pretty_text::{self, MarkdownOptions};
use crate::url::Urls;
use crate::{AppError, Unsupported};

/// The email types built from a token template, and their template.
pub fn template_for(email_type: &str, has_password: bool) -> Option<&'static str> {
    Some(match email_type {
        "signup" => "user_notifications.signup",
        "email_login" => "user_notifications.email_login",
        "forgot_password" if has_password => "user_notifications.forgot_password",
        "forgot_password" => "user_notifications.set_password",
        _ => return None,
    })
}

/// `build_user_email_token_by_template(template, user, email_token)`
pub async fn build(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    template: &str,
    user_id: i32,
    email_token: &str,
) -> Result<Built, AppError> {
    let s = ctx.settings;
    let (email, username, locale): (Option<String>, String, Option<String>) = sqlx::query_as(
        "SELECT (SELECT email FROM user_emails WHERE user_id = u.id AND \"primary\" LIMIT 1), \
                u.username, u.locale FROM users u WHERE u.id = $1",
    )
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?;
    let default_locale = s.get("default_locale")?.to_s();
    if locale
        .as_deref()
        .is_some_and(|l| !l.is_empty() && l != default_locale)
        && s.get("allow_user_locale")?.truthy()
    {
        return Err(Unsupported("account emails in a user's own locale").into());
    }
    let to = email.ok_or(Unsupported("a recipient without an email"))?;
    build_template(
        conn,
        ctx,
        template,
        &to,
        &[
            ("email_token", email_token),
            ("recipient_username", &username),
        ],
        Some(user_id),
    )
    .await
}

/// `build_email(to, template:, **args)` for a template with no HTML
/// override: `extra` are the template's own arguments, `user_id` the user
/// the text is cooked for (Email::Sender's user).
pub async fn build_template(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    template: &str,
    to: &str,
    extra: &[(&str, &str)],
    user_id: Option<i32>,
) -> Result<Built, AppError> {
    let s = ctx.settings;
    let default_locale = s.get("default_locale")?.to_s();
    let overridden: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM translation_overrides WHERE locale = $1 AND translation_key LIKE $2)",
    )
    .bind(&default_locale)
    .bind(format!("{template}.%"))
    .fetch_one(&mut *conn)
    .await?;
    if overridden {
        return Err(Unsupported("translation overrides of email templates").into());
    }
    let urls = Urls {
        config: ctx.config,
        settings: s,
    };
    let base_url = urls.base_url()?;
    let site_name = s.get("title")?.to_s();
    let email_prefix = s
        .get("email_prefix")?
        .presence()
        .unwrap_or_else(|| site_name.clone());
    let user_preferences_url = format!("{base_url}/my/preferences");
    let hostname = urls.current_hostname()?;
    let mut args: Vec<(&str, &str)> = vec![
        ("site_name", &site_name),
        ("email_prefix", &email_prefix),
        ("base_url", &base_url),
        ("user_preferences_url", &user_preferences_url),
        ("hostname", &hostname),
        ("optional_re", ""),
        ("optional_pm", ""),
        ("optional_cat", ""),
        ("optional_tags", ""),
    ];
    args.extend_from_slice(extra);
    let preview = ctx
        .i18n
        .t(&format!("{template}.preview"))
        .map(|_| t(ctx, &format!("{template}.preview"), &args))
        .transpose()?;
    let subject_key = if s.get("simple_email_subject")?.truthy()
        && ctx
            .i18n
            .t(&format!("{template}.subject_template_improved"))
            .is_some()
    {
        format!("{template}.subject_template_improved")
    } else {
        format!("{template}.subject_template")
    };
    let subject = t(ctx, &subject_key, &args)?;
    let text = t(ctx, &format!("{template}.text_body_template"), &args)?;

    // Email::Renderer#html without an HTML part: the text, unescaped and
    // cooked, in the layout with the preview.
    let unescaped = html_escape::decode_html_entities(&text).into_owned();
    let cooked = pretty_text::cook(
        ctx.host,
        &unescaped,
        &MarkdownOptions {
            user_id: user_id.map(i64::from),
            ..Default::default()
        },
    )
    .await?;
    let preview_html = match &preview {
        Some(p) if !p.trim().is_empty() => {
            pretty_text::cook(ctx.host, p, &MarkdownOptions::default()).await?
        }
        _ => String::new(),
    };
    let html_part = layout(
        &cooked,
        &default_locale.replacen('_', "-", 1),
        &preview_html,
    );

    // header_args: no unsubscribe, post or topic; Reply-To is the From.
    let notification_email = s.get("notification_email")?.to_s();
    let site_title = s.get("email_site_title")?.presence().unwrap_or(site_name);
    let cleanup = |name: &str| name.replace([':', '<', '>', ',', '"'], "");
    let from = format!("\"{}\" <{notification_email}>", cleanup(&site_title));
    let mut headers = vec![
        ("To".to_string(), to.to_string()),
        ("Subject".to_string(), subject),
        ("From".to_string(), from.clone()),
    ];
    if let Some(p) = preview.filter(|p| !p.trim().is_empty()) {
        headers.push(("X-Discourse-Email-Preview".into(), p));
    }
    headers.push(("X-Auto-Response-Suppress".into(), "All".into()));
    headers.push(("x-ms-reactions".into(), "disallow".into()));
    headers.push(("Reply-To".into(), from));
    for item in s.get("email_custom_headers")?.to_s().split('|') {
        if let Some((name, value)) = item.split_once(':') {
            let (name, value) = (name.trim(), value.trim());
            if !name.is_empty() && !value.is_empty() {
                headers.push((name.to_string(), value.to_string()));
            }
        }
    }
    Ok(Built {
        message: Message {
            headers,
            text,
            html: None,
        },
        html_part,
    })
}
