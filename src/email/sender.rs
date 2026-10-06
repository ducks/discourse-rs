//! `Email::Sender#send` (lib/email/sender.rb) and `Email::Renderer`: the
//! HTML styled for mail clients, the threading and list headers, delivery
//! and the `email_logs` row. Group SMTP, reply-by-email keys, bounce
//! addresses, attachments and provider-specific headers are refused.

use sqlx::PgConnection;

use super::Mailer;
use super::styles::Styles;
use crate::config::Config;
use crate::i18n::I18n;
use crate::pretty_text::Host;
use crate::site_settings::SiteSettings;
use crate::url::Urls;
use crate::{AppError, Unsupported};

/// What building and sending mail reads from the application.
pub struct Ctx<'a> {
    pub host: &'a Host,
    pub settings: &'a SiteSettings,
    pub config: &'a Config,
    pub i18n: &'a I18n,
    pub mailer: &'a Mailer,
}

/// Why Email::Sender (or user_email before it) did not send.
pub mod skip_reasons {
    pub const EXCEEDED_EMAILS_LIMIT: i32 = 2;
    pub const EXCEEDED_BOUNCES_LIMIT: i32 = 3;
    pub const USER_EMAIL_NO_USER: i32 = 5;
    pub const USER_EMAIL_POST_NOT_FOUND: i32 = 6;
    pub const USER_EMAIL_ANONYMOUS_USER: i32 = 7;
    pub const USER_EMAIL_USER_SUSPENDED_NOT_PM: i32 = 8;
    pub const USER_EMAIL_SEEN_RECENTLY: i32 = 9;
    pub const USER_EMAIL_NOTIFICATION_ALREADY_READ: i32 = 10;
    pub const USER_EMAIL_TOPIC_NIL: i32 = 11;
    pub const USER_EMAIL_POST_USER_DELETED: i32 = 12;
    pub const USER_EMAIL_POST_DELETED: i32 = 13;
    pub const USER_EMAIL_ALREADY_READ: i32 = 15;
    pub const SENDER_MESSAGE_TO_BLANK: i32 = 17;
    pub const SENDER_TEXT_PART_BODY_BLANK: i32 = 18;
    pub const SENDER_POST_DELETED: i32 = 20;
    pub const SENDER_MESSAGE_TO_INVALID: i32 = 21;
    pub const USER_EMAIL_ACCESS_DENIED: i32 = 22;
    pub const SENDER_TOPIC_DELETED: i32 = 23;
    pub const USER_EMAIL_NO_EMAIL: i32 = 24;
}

/// `SkippedEmailLog.create!`
pub async fn skip(
    conn: &mut PgConnection,
    email_type: &str,
    to_address: &str,
    user_id: Option<i32>,
    post_id: Option<i32>,
    reason_type: i32,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO skipped_email_logs (email_type, to_address, user_id, post_id, reason_type, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, clock_timestamp(), clock_timestamp())",
    )
    .bind(email_type)
    .bind(to_address)
    .bind(user_id)
    .bind(post_id)
    .bind(reason_type)
    .execute(conn)
    .await?;
    Ok(())
}

/// The mail gem's order for the fields it knows; the rest follow in the
/// order they were added.
pub(crate) const FIELD_ORDER: [&str; 28] = [
    "return-path",
    "received",
    "resent-date",
    "resent-from",
    "resent-sender",
    "resent-to",
    "resent-cc",
    "resent-bcc",
    "resent-message-id",
    "date",
    "from",
    "sender",
    "reply-to",
    "to",
    "cc",
    "bcc",
    "message-id",
    "in-reply-to",
    "references",
    "subject",
    "comments",
    "keywords",
    "mime-version",
    "content-type",
    "content-transfer-encoding",
    "content-location",
    "content-disposition",
    "content-description",
];

fn ordered(headers: Vec<(String, String)>) -> Vec<(String, String)> {
    let mut known: Vec<(usize, (String, String))> = Vec::new();
    let mut rest = Vec::new();
    for h in headers {
        match FIELD_ORDER
            .iter()
            .position(|f| f.eq_ignore_ascii_case(&h.0))
        {
            Some(i) => known.push((i, h)),
            None => rest.push(h),
        }
    }
    known.sort_by_key(|(i, _)| *i);
    known.into_iter().map(|(_, h)| h).chain(rest).collect()
}

/// `Email::MessageIdService.generate_or_use_existing(post_ids)`: each
/// post's outbound Message-ID, set on first use, by creation.
async fn message_ids(
    conn: &mut PgConnection,
    post_ids: &[i32],
    host: &str,
) -> Result<Vec<String>, AppError> {
    if post_ids.is_empty() {
        return Ok(Vec::new());
    }
    sqlx::query(
        "UPDATE posts SET outbound_message_id = 'discourse/post/' || posts.id || '@' || $2 \
         WHERE outbound_message_id IS NULL AND posts.id = ANY($1)",
    )
    .bind(post_ids)
    .bind(host)
    .execute(&mut *conn)
    .await?;
    Ok(sqlx::query_scalar(
        "SELECT '<' || outbound_message_id || '>' FROM posts WHERE id = ANY($1) ORDER BY created_at ASC",
    )
    .bind(post_ids)
    .fetch_all(&mut *conn)
    .await?)
}

/// `Email::Sender.new(message, type, user).send`
pub async fn send(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    built: super::notification::Built,
    email_type: &str,
    user_id: Option<i32>,
) -> Result<(), AppError> {
    let s = ctx.settings;
    let mut message = built.message;
    match s.get("disable_emails")?.to_s().as_str() {
        "yes" => return Ok(()),
        "non-staff" => {
            let staff: bool = sqlx::query_scalar(
                "SELECT COALESCE((SELECT admin OR moderator FROM users WHERE id = $1), FALSE)",
            )
            .bind(user_id)
            .fetch_one(&mut *conn)
            .await?;
            if !staff {
                return Ok(());
            }
        }
        _ => {}
    }
    let to_address = message.header("To").unwrap_or("").to_string();
    if to_address.is_empty() {
        return skip(
            conn,
            email_type,
            "no_email_found",
            user_id,
            None,
            skip_reasons::SENDER_MESSAGE_TO_BLANK,
        )
        .await;
    }
    if to_address.ends_with(".invalid") {
        return skip(
            conn,
            email_type,
            &to_address,
            user_id,
            None,
            skip_reasons::SENDER_MESSAGE_TO_INVALID,
        )
        .await;
    }
    if message.text.trim().is_empty() {
        return skip(
            conn,
            email_type,
            &to_address,
            user_id,
            None,
            skip_reasons::SENDER_TEXT_PART_BODY_BLANK,
        )
        .await;
    }
    let urls = Urls {
        config: ctx.config,
        settings: s,
    };
    let base_url = urls.base_url()?;
    let base_url_no_prefix = urls.base_url_no_prefix()?;

    // Email::Renderer#html
    let rendered = {
        let style = Styles::new(&built.html_part);
        style.format_basic(&base_url)?;
        style.format_html(s)?;
        style.to_html(&base_url, &base_url_no_prefix)
    };
    let store = crate::file_store::FileStore::for_site(ctx.config, s)?;
    if message
        .text
        .contains(&format!("{}/", store.relative_base_url()))
    {
        return Err(Unsupported("uploads in email text").into());
    }

    let host = base_url
        .split_once("://")
        .map(|(_, rest)| rest.split(['/', ':']).next().unwrap_or("localhost"))
        .filter(|h| !h.is_empty())
        .unwrap_or("localhost")
        .to_lowercase();
    let post_id: Option<i32> = message
        .header("X-Discourse-Post-Id")
        .and_then(|v| v.parse().ok());
    let topic_id: Option<i32> = message
        .header("X-Discourse-Topic-Id")
        .and_then(|v| v.parse().ok());
    let reply_key = match (user_id, post_id) {
        (Some(user_id), Some(post_id))
            if message
                .header("X-Discourse-Allow-Reply-By-Email")
                .is_some_and(|v| !v.is_empty()) =>
        {
            Some(reply_key_for(&mut *conn, post_id, user_id).await?)
        }
        _ => None,
    };
    let from_address = message.header("From").and_then(|f| {
        f.rsplit_once('<')
            .map(|(_, a)| a.trim_end_matches('>').to_string())
    });
    let smtp_group: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM groups WHERE email_username = $1 AND smtp_enabled)",
    )
    .bind(&from_address)
    .fetch_one(&mut *conn)
    .await?;
    if smtp_group {
        return Err(Unsupported("group SMTP").into());
    }
    message.set_header("Message-ID", Some(format!("<{}@{host}>", random_uuid())));

    if let (Some(topic_id), Some(post_id)) = (topic_id, post_id) {
        let post: Option<(i32, Option<i32>)> = sqlx::query_as(
            "SELECT p.post_number, t.id FROM posts p LEFT JOIN topics t ON t.id = p.topic_id AND t.deleted_at IS NULL \
             WHERE p.id = $1 AND p.topic_id = $2 AND p.deleted_at IS NULL",
        )
        .bind(post_id)
        .bind(topic_id)
        .fetch_optional(&mut *conn)
        .await?;
        let Some((post_number, topic)) = post else {
            return skip(
                conn,
                email_type,
                &to_address,
                user_id,
                None,
                skip_reasons::SENDER_POST_DELETED,
            )
            .await;
        };
        if topic.is_none() {
            return skip(
                conn,
                email_type,
                &to_address,
                user_id,
                None,
                skip_reasons::SENDER_TOPIC_DELETED,
            )
            .await;
        }
        // add_identification_field_headers
        let own = message_ids(&mut *conn, &[post_id], &host).await?;
        message.set_header("Message-ID", own.first().cloned());
        if post_number > 1 {
            let first_post: i32 =
                sqlx::query_scalar("SELECT id FROM posts WHERE topic_id = $1 AND post_number = 1")
                    .bind(topic_id)
                    .fetch_one(&mut *conn)
                    .await?;
            let op = message_ids(&mut *conn, &[first_post], &host).await?;
            let referenced: Vec<i32> = sqlx::query_scalar(
                "SELECT posts.id FROM posts INNER JOIN post_replies ON post_replies.post_id = posts.id \
                 WHERE post_replies.reply_post_id = $1 AND posts.deleted_at IS NULL ORDER BY posts.id DESC",
            )
            .bind(post_id)
            .fetch_all(&mut *conn)
            .await?;
            if referenced.is_empty() {
                message.set_header("In-Reply-To", op.first().cloned());
                message.set_header("References", op.first().cloned());
            } else {
                if referenced.len() > 1 {
                    return Err(Unsupported("emails for posts replying to several posts").into());
                }
                let in_reply_to = message_ids(&mut *conn, &referenced, &host).await?;
                // The reply tree above the replied-to post.
                let parents: Vec<i32> = sqlx::query_scalar(
                    "WITH RECURSIVE cte AS ( \
                       SELECT reply_post_id, post_id FROM post_replies WHERE reply_post_id = $1 \
                       UNION SELECT pr.reply_post_id, pr.post_id FROM post_replies pr \
                       INNER JOIN cte ON cte.post_id = pr.reply_post_id) \
                     SELECT DISTINCT ON (cte.reply_post_id) cte.post_id FROM cte \
                     INNER JOIN posts ON posts.id = cte.reply_post_id \
                     ORDER BY cte.reply_post_id, posts.created_at DESC, cte.post_id DESC",
                )
                .bind(referenced[0])
                .fetch_all(&mut *conn)
                .await?;
                if !parents.is_empty() {
                    return Err(Unsupported("emails for replies deeper than one level").into());
                }
                let mut references: Vec<String> = Vec::new();
                for id in op.iter().chain(in_reply_to.last()) {
                    if !references.contains(id) {
                        references.push(id.clone());
                    }
                }
                message.set_header("In-Reply-To", Some(in_reply_to.join(" ")));
                message.set_header("References", Some(references.join(" ")));
            }
        }
        // List-ID, Precedence, List-Archive
        let title = s.get("title")?.to_s();
        #[derive(sqlx::FromRow)]
        struct TopicCategory {
            category_id: Option<i32>,
            name: Option<String>,
            parent_category_id: Option<i32>,
            slug: Option<String>,
        }
        let row: TopicCategory = sqlx::query_as(
            "SELECT c.id AS category_id, c.name, c.parent_category_id, t.slug FROM topics t \
             LEFT JOIN categories c ON c.id = t.category_id WHERE t.id = $1",
        )
        .bind(topic_id)
        .fetch_one(&mut *conn)
        .await?;
        let category = row
            .category_id
            .zip(row.name)
            .map(|(id, name)| (id, name, row.parent_category_id));
        let slug = row.slug;
        let uncategorized = s.get("uncategorized_category_id")?.to_i() as i32;
        let dashed = |name: &str| name.to_lowercase().replace(' ', "-");
        let list_id = match &category {
            Some((id, name, parent)) if *id != uncategorized => match parent {
                Some(parent) => {
                    let parent_name: String =
                        sqlx::query_scalar("SELECT name FROM categories WHERE id = $1")
                            .bind(parent)
                            .fetch_one(&mut *conn)
                            .await?;
                    format!(
                        "{title} | {parent_name} {name} <{}.{}.{host}>",
                        dashed(name),
                        dashed(&parent_name)
                    )
                }
                None => format!("{title} | {name} <{}.{host}>", dashed(name)),
            },
            _ => format!("{title} <{host}>"),
        };
        message.set_header("Precedence", Some("list".into()));
        message.set_header("List-ID", Some(list_id));
        message.set_header(
            "List-Archive",
            Some(format!(
                "{base_url}/t/{}/{topic_id}",
                slug.unwrap_or_default()
            )),
        );
        let uploads: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM upload_references WHERE target_type = 'Post' AND target_id = $1)",
        )
        .bind(post_id)
        .fetch_one(&mut *conn)
        .await?;
        if uploads && s.get("email_total_attachment_size_limit_kb")?.to_i() > 0 {
            return Err(Unsupported("email attachments").into());
        }
    }
    // A bounceable reply address: a VERP Return-Path with the log's key.
    let reply_address = s.get("reply_by_email_address")?.to_s();
    let bounce_key = if reply_address.contains('+') {
        let key = crate::accounts::random_hex();
        message.set_header(
            "Return-Path",
            Some(reply_address.replacen("%{reply_key}", &format!("verp-{key}"), 1)),
        );
        Some(key)
    } else {
        None
    };
    if let Some(key) = &reply_key {
        let reply_to = message
            .header("Reply-To")
            .map(|v| v.replace("%{reply_key}", key));
        message.set_header("Reply-To", reply_to);
        message.set_header("X-Discourse-Allow-Reply-By-Email", None);
    }
    // Custom headers carrying a reply key get it, or go without one.
    let custom: Vec<String> = s
        .get("email_custom_headers")?
        .to_s()
        .split('|')
        .filter_map(|item| {
            item.split_once(':')
                .map(|(n, _)| n.trim().to_ascii_lowercase())
        })
        .collect();
    message.headers.retain_mut(|(name, value)| {
        if !custom.contains(&name.to_ascii_lowercase()) || !value.contains("%{reply_key}") {
            return true;
        }
        match &reply_key {
            Some(key) => {
                *value = value.replace("%{reply_key}", key);
                true
            }
            None => false,
        }
    });
    let smtp_address = ctx.config.globals.get("smtp_address").unwrap_or("");
    if smtp_address.contains(".mailjet.com")
        || smtp_address == "smtp.mandrillapp.com"
        || smtp_address == "smtp.sparkpostmail.com"
    {
        return Err(Unsupported("provider-specific email headers").into());
    }
    let short = s.get("strip_images_from_short_emails")?.truthy()
        && rendered.len() as i64 <= s.get("short_email_length")?.to_i()
        && rendered.contains("<img");
    message.html = Some({
        let style = Styles::new(&rendered);
        if short {
            style.strip_avatars_and_emojis();
        }
        style.to_s()
    });
    message.headers.push(("MIME-Version".into(), "1.0".into()));
    message.headers.push((
        "Content-Type".into(),
        "multipart/alternative; boundary=--==_mimepart_discourse_rs".into(),
    ));
    message
        .headers
        .push(("Content-Transfer-Encoding".into(), "7bit".into()));
    message.headers = ordered(std::mem::take(&mut message.headers));
    let message_id = message
        .header("Message-ID")
        .unwrap_or("")
        .trim_matches(['<', '>'])
        .to_string();

    let response = ctx.mailer.deliver(&message).await?;
    sqlx::query(
        "INSERT INTO email_logs (email_type, to_address, user_id, post_id, topic_id, message_id, \
                                 smtp_transaction_response, bounce_key, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8::uuid, clock_timestamp(), clock_timestamp())",
    )
    .bind(email_type)
    .bind(&to_address)
    .bind(user_id)
    .bind(post_id)
    .bind(topic_id)
    .bind(&message_id)
    .bind(response)
    .bind(&bounce_key)
    .execute(&mut *conn)
    .await?;
    // EmailLog after_create
    sqlx::query("UPDATE users SET last_emailed_at = now() WHERE id = $1")
        .bind(user_id)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// `SecureRandom.uuid`
fn random_uuid() -> String {
    use rand::RngCore;
    let mut b = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut b);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

/// `PostReplyKey.create_or_find_by!(post_id:, user_id:).reply_key`: the
/// user's key for replying to the post by email, without dashes.
async fn reply_key_for(
    conn: &mut PgConnection,
    post_id: i32,
    user_id: i32,
) -> Result<String, AppError> {
    sqlx::query(
        "INSERT INTO post_reply_keys (user_id, post_id, reply_key, created_at, updated_at) \
         VALUES ($1, $2, $3::uuid, clock_timestamp(), clock_timestamp()) \
         ON CONFLICT (user_id, post_id) DO NOTHING",
    )
    .bind(user_id)
    .bind(post_id)
    .bind(crate::accounts::random_hex())
    .execute(&mut *conn)
    .await?;
    let key: String = sqlx::query_scalar(
        "SELECT reply_key::text FROM post_reply_keys WHERE user_id = $1 AND post_id = $2",
    )
    .bind(user_id)
    .bind(post_id)
    .fetch_one(&mut *conn)
    .await?;
    Ok(key.replace('-', ""))
}
