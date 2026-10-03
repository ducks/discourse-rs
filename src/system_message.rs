//! `SystemMessage.create(recipient, type, params)`: a private message to
//! the user from the site contact (the system user unless
//! site_contact_username names someone), its title and body from
//! `system_messages.<type>` with the recipient's details interpolated,
//! created through PostCreator without validations, then archived for the
//! sender.
//!
//! Refused: a site contact group (site_contact_group_name).

use serde_json::Value;
use sqlx::{PgConnection, PgPool};

use crate::guardian::Guardian;
use crate::posting::Ctx;
use crate::posting::create::{self, NewPost, Outcome};
use crate::session::current::{SESSION_USER_COLUMNS, SessionUser};
use crate::url::Urls;
use crate::{AppError, Unsupported};

/// The recipient as the template defaults read them.
#[derive(sqlx::FromRow)]
struct Recipient {
    username: String,
    name: Option<String>,
}

/// `defaults.merge(params)`, then the type's preview from them.
fn interpolations(
    ctx: &Ctx<'_>,
    recipient: &Recipient,
    message_type: &str,
    given: &[(String, String)],
) -> Result<Vec<(String, String)>, AppError> {
    let urls = Urls {
        config: ctx.config,
        settings: ctx.settings,
    };
    let base_url = urls.base_url()?;
    let name = recipient.name.clone().unwrap_or_default();
    let name_or_username = if name.trim().is_empty() {
        recipient.username.clone()
    } else {
        name.clone()
    };
    let username_encoded =
        form_urlencoded::byte_serialize(recipient.username.as_bytes()).collect::<String>();
    let mut params = vec![
        ("site_name".to_string(), ctx.settings.get("title")?.to_s()),
        ("username".to_string(), recipient.username.clone()),
        ("name".to_string(), name),
        ("name_or_username".to_string(), name_or_username),
        (
            "user_preferences_url".to_string(),
            format!("{base_url}/u/{username_encoded}/preferences"),
        ),
        (
            "new_user_tips".to_string(),
            ctx.i18n
                .t_with(
                    "system_messages.usage_tips.text_body_template",
                    &[("base_url", &base_url)],
                )
                .unwrap_or_default(),
        ),
        ("site_password".to_string(), String::new()),
        ("base_url".to_string(), base_url),
        ("email_preview".to_string(), String::new()),
    ];
    for (key, value) in given {
        params.retain(|(k, _)| k != key);
        params.push((key.clone(), value.clone()));
    }
    let args: Vec<(&str, &str)> = params
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let preview = ctx
        .i18n
        .t_with(&format!("system_messages.{message_type}.preview"), &args);
    if let Some(preview) = preview {
        params.retain(|(k, _)| k != "email_preview");
        params.push(("email_preview".to_string(), preview));
    }
    Ok(params)
}

/// The site contact: site_contact_username's user, else the system user.
async fn site_contact(conn: &mut PgConnection, ctx: &Ctx<'_>) -> Result<SessionUser, AppError> {
    let username = ctx.settings.get("site_contact_username")?.to_s();
    let sql = format!(
        "SELECT {SESSION_USER_COLUMNS} FROM users WHERE {}",
        if username.trim().is_empty() {
            "users.id = -1"
        } else {
            "users.username_lower = lower($1)"
        }
    );
    let user: Option<SessionUser> = sqlx::query_as(&sql)
        .bind(username.trim())
        .fetch_optional(&mut *conn)
        .await?;
    match user {
        Some(u) => Ok(u),
        None => Err(Unsupported("a site contact that does not exist").into()),
    }
}

/// `SystemMessage.create(recipient, type, params)`: `params` interpolate
/// into the templates over the defaults. Returns the post's id.
pub async fn create(
    pool: &PgPool,
    ctx: &Ctx<'_>,
    recipient_id: i32,
    message_type: &str,
    params: &[(String, String)],
    post_alert_options: Option<Value>,
) -> Result<i32, AppError> {
    if !ctx
        .settings
        .get("site_contact_group_name")?
        .to_s()
        .trim()
        .is_empty()
    {
        return Err(Unsupported("system messages copied to a site contact group").into());
    }
    let mut conn = pool.acquire().await?;
    let recipient: Recipient = sqlx::query_as("SELECT username, name FROM users WHERE id = $1")
        .bind(recipient_id)
        .fetch_one(&mut *conn)
        .await?;
    let params = interpolations(ctx, &recipient, message_type, params)?;
    let args: Vec<(&str, &str)> = params
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let template = |part: &str| {
        let key = format!("system_messages.{message_type}.{part}");
        ctx.i18n
            .t_with(&key, &args)
            .ok_or(Unsupported("system messages without a translation"))
    };
    let title = template("subject_template")?;
    let raw = template("text_body_template")?;

    let sender = site_contact(&mut conn, ctx).await?;
    let guardian = Guardian::for_user(&mut conn, &sender).await?;
    drop(conn);
    let outcome = create::create(
        pool,
        ctx,
        &guardian,
        NewPost {
            raw,
            title: Some(title),
            pm_recipients: Some(vec![recipient.username.to_lowercase()]),
            skip_validations: true,
            subtype: Some("system_message".to_string()),
            post_alert_options,
            ..Default::default()
        },
    )
    .await?;
    let post_id = match outcome {
        Outcome::Created { post_id } => post_id,
        // raise StandardError, creator.errors.full_messages
        _ => return Err(Unsupported("a system message PostCreator refused").into()),
    };
    // UserArchivedMessage.create!(user: site_contact_user, topic:)
    sqlx::query(
        "INSERT INTO user_archived_messages (user_id, topic_id, created_at, updated_at) \
         SELECT $1, topic_id, clock_timestamp(), clock_timestamp() FROM posts WHERE id = $2",
    )
    .bind(sender.id)
    .bind(post_id)
    .execute(pool)
    .await?;
    Ok(post_id)
}
