//! Admin::EmailLogsController's lists: sent and bounced (EmailLog), skipped
//! (SkippedEmailLog), received and rejected (IncomingEmail), 50 a page
//! newest first, with the controller's ILIKE filters, through
//! EmailLogSerializer, SkippedEmailLogSerializer and IncomingEmailSerializer.
//!
//! And the incoming email details (`incoming/:id`,
//! `incoming_from_bounced/:id`): IncomingEmailDetailsSerializer over the
//! raw message, read as the cleaner wrote it.

use chrono::NaiveDateTime;
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::i18n::I18n;
use crate::{AppError, Unsupported};

pub struct Context<'a> {
    pub user_cx: &'a crate::admin_user_show::Context<'a>,
    pub i18n: &'a I18n,
}

/// The filters the lists read, by param name.
#[derive(Default)]
pub struct Filters {
    pub offset: Option<String>,
    pub user: Option<String>,
    pub address: Option<String>,
    pub email_type: Option<String>,
    pub smtp_transaction_response: Option<String>,
    pub reply_key: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub subject: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Copy, PartialEq)]
pub enum List {
    Sent,
    Bounced,
    Skipped,
    Received,
    Rejected,
}

fn present(v: &Option<String>) -> Option<&str> {
    v.as_deref().filter(|s| !crate::ruby::is_blank(s))
}

/// `"%#{value}%"`, as ILIKE reads it.
fn like(v: &str) -> String {
    format!("%{v}%")
}

fn iso(t: NaiveDateTime) -> String {
    t.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

/// `post.url` and `"#{topic.title} ##{post_number}"` for a live post (the
/// default scopes hide deleted posts and topics; a post whose topic is
/// gone links to /404 without a description).
async fn post_fields(
    conn: &mut PgConnection,
    post_id: Option<i32>,
) -> Result<Option<(String, Option<String>)>, AppError> {
    let Some(id) = post_id else {
        return Ok(None);
    };
    #[derive(sqlx::FromRow)]
    struct Row {
        topic_id: i32,
        post_number: i32,
        slug: Option<String>,
        title: Option<String>,
        live_topic: bool,
    }
    let row: Option<Row> = sqlx::query_as(
        "SELECT p.topic_id, p.post_number, t.slug, t.title, t.id IS NOT NULL AS live_topic \
         FROM posts p LEFT JOIN topics t ON t.id = p.topic_id AND t.deleted_at IS NULL \
         WHERE p.id = $1 AND p.deleted_at IS NULL",
    )
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.map(|r| {
        if r.live_topic {
            // Post.url: `"#{slug}/"`, so a nil slug leaves `//`.
            let url = format!(
                "/t/{}/{}/{}",
                r.slug.unwrap_or_default(),
                r.topic_id,
                r.post_number
            );
            (url, r.title.map(|t| format!("{t} #{}", r.post_number)))
        } else {
            ("/404".to_string(), None)
        }
    }))
}

fn split(v: Option<String>) -> Value {
    match v.filter(|s| !crate::ruby::is_blank(s)) {
        Some(s) => json!(s.split(';').collect::<Vec<_>>()),
        None => Value::Null,
    }
}

pub async fn list(
    conn: &mut PgConnection,
    cx: &Context<'_>,
    which: List,
    f: &Filters,
) -> Result<Value, AppError> {
    let offset = f.offset.as_deref().map_or(0, crate::ruby::to_i).max(0);
    let logo = crate::admin_users::logo_small_url(conn, cx.user_cx.settings).await?;
    let mut w = Where::default();
    let (table, base) = match which {
        List::Sent | List::Bounced => ("email_logs", "email_logs"),
        List::Skipped => ("skipped_email_logs", "skipped_email_logs"),
        List::Received | List::Rejected => ("incoming_emails", "incoming_emails"),
    };
    match which {
        List::Received | List::Rejected => {
            if which == List::Rejected {
                w.clauses.push("NOT is_bounce AND error IS NOT NULL".into());
            }
            if let Some(v) = present(&f.from) {
                w.add("from_address ILIKE $n", like(v));
            }
            if let Some(v) = present(&f.to) {
                w.add("(to_addresses ILIKE $n OR cc_addresses ILIKE $n)", like(v));
            }
            if let Some(v) = present(&f.subject) {
                w.add("subject ILIKE $n", like(v));
            }
            if let Some(v) = present(&f.error) {
                w.add("error ILIKE $n", like(v));
            }
        }
        _ => {
            if which == List::Bounced {
                w.clauses.push("email_logs.bounced".into());
            }
            if let Some(v) = present(&f.user) {
                w.add("users.username ILIKE $n", like(v));
            }
            if let Some(v) = present(&f.address) {
                let cc = if table == "email_logs" {
                    format!(" OR {table}.cc_addresses ILIKE $n")
                } else {
                    String::new()
                };
                w.add(&format!("({table}.to_address ILIKE $n{cc})"), like(v));
            }
            if let Some(v) = present(&f.email_type) {
                w.add(&format!("{table}.email_type ILIKE $n"), like(v));
            }
            if table == "email_logs"
                && let Some(v) = present(&f.smtp_transaction_response)
            {
                w.add("email_logs.smtp_transaction_response ILIKE $n", like(v));
            }
            if which == List::Sent
                && let Some(key) = present(&f.reply_key)
            {
                if key.chars().count() == 32 {
                    w.add("post_reply_keys.reply_key = $n::uuid", key.to_string());
                } else {
                    w.add(
                        "replace(post_reply_keys.reply_key::VARCHAR, '-', '') ILIKE $n",
                        like(key),
                    );
                }
            }
        }
    }
    let joins = match which {
        List::Sent => {
            "LEFT JOIN users ON users.id = email_logs.user_id \
             LEFT JOIN post_reply_keys ON post_reply_keys.post_id = email_logs.post_id \
               AND post_reply_keys.user_id = email_logs.user_id"
        }
        List::Bounced | List::Skipped => &format!("LEFT JOIN users ON users.id = {table}.user_id"),
        _ => "",
    };
    let filter = if w.clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", w.clauses.join(" AND "))
    };
    let ids_sql = format!(
        "SELECT {base}.id::int8 FROM {base} {joins} {filter} \
         ORDER BY {base}.created_at DESC OFFSET {offset} LIMIT 50"
    );
    let mut q = sqlx::query_scalar::<_, i64>(&ids_sql);
    for b in &w.binds {
        q = q.bind(b);
    }
    let ids: Vec<i64> = q.fetch_all(&mut *conn).await?;

    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        let item = match which {
            List::Sent | List::Bounced => sent_item(conn, cx, id, logo.as_deref()).await?,
            List::Skipped => skipped_item(conn, cx, id, logo.as_deref()).await?,
            List::Received | List::Rejected => incoming_item(conn, cx, id, logo.as_deref()).await?,
        };
        out.push(item);
    }
    Ok(Value::Array(out))
}

/// EmailLogsMixin's attributes and the BasicUserSerializer user.
async fn mixin(
    conn: &mut PgConnection,
    cx: &Context<'_>,
    o: &mut Map<String, Value>,
    row: (i64, String, String, Option<i32>, NaiveDateTime, Option<i32>),
    logo: Option<&str>,
) -> Result<(), AppError> {
    let (id, to_address, email_type, user_id, created_at, post_id) = row;
    o.insert("id".into(), json!(id));
    o.insert("to_address".into(), json!(to_address));
    o.insert("email_type".into(), json!(email_type));
    o.insert("user_id".into(), json!(user_id));
    o.insert("created_at".into(), json!(iso(created_at)));
    if let Some((url, description)) = post_fields(conn, post_id).await? {
        o.insert("post_url".into(), json!(url));
        if let Some(d) = description {
            o.insert("post_description".into(), json!(d));
        }
    }
    let user = crate::admin_user_show::basic_user(conn, cx.user_cx, user_id, logo).await?;
    o.insert("user".into(), user);
    Ok(())
}

async fn sent_item(
    conn: &mut PgConnection,
    cx: &Context<'_>,
    id: i64,
    logo: Option<&str>,
) -> Result<Value, AppError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        to_address: String,
        email_type: String,
        user_id: Option<i32>,
        created_at: NaiveDateTime,
        post_id: Option<i32>,
        cc_addresses: Option<String>,
        bounced: bool,
        has_bounce_key: bool,
        smtp_transaction_response: Option<String>,
        reply_key: Option<String>,
    }
    let r: Row = sqlx::query_as(
        "SELECT e.to_address, e.email_type, e.user_id, e.created_at, e.post_id, e.cc_addresses, \
                e.bounced, e.bounce_key IS NOT NULL AS has_bounce_key, e.smtp_transaction_response, \
                (SELECT replace(k.reply_key::text, '-', '') FROM post_reply_keys k \
                 WHERE k.post_id = e.post_id AND k.user_id = e.user_id LIMIT 1) AS reply_key \
         FROM email_logs e WHERE e.id = $1",
    )
    .bind(id as i32)
    .fetch_one(&mut *conn)
    .await?;
    let mut o = Map::new();
    mixin(
        conn,
        cx,
        &mut o,
        (
            id,
            r.to_address,
            r.email_type,
            r.user_id,
            r.created_at,
            r.post_id,
        ),
        logo,
    )
    .await?;
    o.insert("cc_addresses".into(), split(r.cc_addresses));
    o.insert("post_id".into(), json!(r.post_id));
    if let Some(key) = r.reply_key {
        o.insert("reply_key".into(), json!(key));
    }
    o.insert("bounced".into(), json!(r.bounced));
    o.insert("has_bounce_key".into(), json!(r.has_bounce_key));
    o.insert(
        "smtp_transaction_response".into(),
        json!(r.smtp_transaction_response),
    );
    Ok(Value::Object(o))
}

/// `SkippedEmailLog.reason_types`
const REASON_TYPES: [&str; 27] = [
    "custom",
    "exceeded_emails_limit",
    "exceeded_bounces_limit",
    "mailing_list_no_echo_mode",
    "user_email_no_user",
    "user_email_post_not_found",
    "user_email_anonymous_user",
    "user_email_user_suspended_not_pm",
    "user_email_seen_recently",
    "user_email_notification_already_read",
    "user_email_topic_nil",
    "user_email_post_user_deleted",
    "user_email_post_deleted",
    "user_email_user_suspended",
    "user_email_already_read",
    "sender_message_blank",
    "sender_message_to_blank",
    "sender_text_part_body_blank",
    "sender_body_blank",
    "sender_post_deleted",
    "sender_message_to_invalid",
    "user_email_access_denied",
    "sender_topic_deleted",
    "user_email_no_email",
    "group_smtp_post_deleted",
    "group_smtp_topic_deleted",
    "group_smtp_disabled_for_group",
];

async fn skipped_item(
    conn: &mut PgConnection,
    cx: &Context<'_>,
    id: i64,
    logo: Option<&str>,
) -> Result<Value, AppError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        to_address: String,
        email_type: String,
        user_id: Option<i32>,
        created_at: NaiveDateTime,
        post_id: Option<i32>,
        reason_type: i32,
        custom_reason: Option<String>,
    }
    let r: Row = sqlx::query_as(
        "SELECT to_address, email_type, user_id, created_at, post_id, reason_type, custom_reason \
         FROM skipped_email_logs WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&mut *conn)
    .await?;
    // SkippedEmailLog#reason
    let reason = if r.reason_type == 1 {
        json!(r.custom_reason)
    } else {
        let name = usize::try_from(r.reason_type - 1)
            .ok()
            .and_then(|i| REASON_TYPES.get(i))
            .copied()
            .unwrap_or_default();
        let (user, post) = (
            r.user_id.map(|u| u.to_string()).unwrap_or_default(),
            r.post_id.map(|p| p.to_string()).unwrap_or_default(),
        );
        json!(cx.i18n.t_with(
            &format!("skipped_email_log.{name}"),
            &[("user_id", &user), ("post_id", &post)]
        ))
    };
    let mut o = Map::new();
    mixin(
        conn,
        cx,
        &mut o,
        (
            id,
            r.to_address,
            r.email_type,
            r.user_id,
            r.created_at,
            r.post_id,
        ),
        logo,
    )
    .await?;
    o.insert("skipped_reason".into(), reason);
    Ok(Value::Object(o))
}

async fn incoming_item(
    conn: &mut PgConnection,
    cx: &Context<'_>,
    id: i64,
    logo: Option<&str>,
) -> Result<Value, AppError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        user_id: Option<i32>,
        created_at: NaiveDateTime,
        from_address: Option<String>,
        to_addresses: Option<String>,
        cc_addresses: Option<String>,
        subject: Option<String>,
        error: Option<String>,
        post_id: Option<i32>,
    }
    let r: Row = sqlx::query_as(
        "SELECT user_id, created_at, from_address, to_addresses, cc_addresses, subject, error, post_id \
         FROM incoming_emails WHERE id = $1",
    )
    .bind(id as i32)
    .fetch_one(&mut *conn)
    .await?;
    let mut o = Map::new();
    o.insert("id".into(), json!(id));
    o.insert("created_at".into(), json!(iso(r.created_at)));
    o.insert("from_address".into(), json!(r.from_address));
    o.insert("to_addresses".into(), split(r.to_addresses));
    o.insert("cc_addresses".into(), split(r.cc_addresses));
    o.insert("subject".into(), json!(r.subject));
    if let Some(error) = r.error {
        let error = if crate::ruby::is_blank(&error) {
            cx.i18n
                .t("emails.incoming.unrecognized_error")
                .unwrap_or_default()
                .to_string()
        } else {
            error
        };
        o.insert("error".into(), json!(error));
    }
    if let Some((url, _)) = post_fields(conn, r.post_id).await? {
        o.insert("post_url".into(), json!(url));
    }
    let user = crate::admin_user_show::basic_user(conn, cx.user_cx, r.user_id, logo).await?;
    o.insert("user".into(), user);
    Ok(Value::Object(o))
}

/// A WHERE clause's conditions and their bound values (`$n` is the next).
#[derive(Default)]
struct Where {
    clauses: Vec<String>,
    binds: Vec<String>,
}

impl Where {
    fn add(&mut self, clause: &str, value: String) {
        self.binds.push(value);
        self.clauses
            .push(clause.replace("$n", &format!("${}", self.binds.len())));
    }
}

/// ActiveSupport's `underscore` for a class name: `BadDestinationAddress`
/// to `bad_destination_address`, `::` to `/`.
fn underscore(name: &str) -> String {
    let chars: Vec<char> = name.replace("::", "/").chars().collect();
    let mut out = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if c.is_ascii_uppercase() {
            let prev = i.checked_sub(1).map(|j| chars[j]);
            let next = chars.get(i + 1).copied();
            let boundary = match prev {
                Some(p) if p.is_ascii_lowercase() || p.is_ascii_digit() => true,
                Some(p) if p.is_ascii_uppercase() => next.is_some_and(|n| n.is_ascii_lowercase()),
                _ => false,
            };
            if boundary {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(if c == '-' { '_' } else { c });
        }
    }
    out
}

/// ActiveSupport's `truncate_words(100, escape: false)`: the text up to
/// the 100th word (Ruby's `\s` runs between words) and `...`.
fn truncate_words(text: &str, count: usize) -> String {
    let ws = |c: char| matches!(c, ' ' | '\t' | '\r' | '\n' | '\u{c}' | '\u{b}');
    let mut words = 0;
    let mut in_word = false;
    for (i, c) in text.char_indices() {
        if ws(c) {
            // A separator after the last word kept (`#{sep}.*`).
            if in_word && words == count {
                return format!("{}...", &text[..i]);
            }
            in_word = false;
        } else if !in_word {
            in_word = true;
            words += 1;
        }
    }
    text.to_string()
}

/// IncomingEmailDetailsSerializer
pub fn incoming_details(
    i18n: &I18n,
    error: Option<String>,
    rejection_message: Option<String>,
    raw: &str,
) -> Result<Value, AppError> {
    const PREFIX: &str = "Email::Receiver::";
    let mail = crate::email::incoming::parse(raw)?;
    let mut out = Map::new();
    if let Some(error) = &error {
        out.insert(
            "error".into(),
            json!(if crate::ruby::is_blank(error) {
                i18n.t("emails.incoming.unrecognized_error")
                    .unwrap_or("Unrecognized Error")
                    .to_string()
            } else {
                error.clone()
            }),
        );
        if error.contains(PREFIX) {
            let name = underscore(&error.replacen(PREFIX, "", 1));
            let key = format!("emails.incoming.errors.{name}");
            out.insert(
                "error_description".into(),
                json!(
                    i18n.t(&key)
                        .map(str::to_string)
                        .unwrap_or_else(|| format!("Translation missing: en.{key}"))
                ),
            );
        }
    }
    out.insert("rejection_message".into(), json!(rejection_message));
    out.insert("headers".into(), json!(mail.header_to_s()?));
    let subject = mail
        .subject
        .clone()
        .filter(|s| !crate::ruby::is_blank(s))
        .unwrap_or_else(|| {
            i18n.t("emails.incoming.no_subject")
                .unwrap_or("(no subject)")
                .to_string()
        });
    out.insert("subject".into(), json!(subject));
    let body = mail.details_body()?;
    let body = if crate::ruby::is_blank(&body) {
        i18n.t("emails.incoming.no_body")
            .unwrap_or("(no body)")
            .to_string()
    } else {
        truncate_words(crate::ruby::strip(&body), 100)
    };
    out.insert("body".into(), json!(body));
    Ok(Value::Object(out))
}

/// `IncomingEmail.find(id)`: the details, None when there is no such row.
pub async fn incoming(
    conn: &mut PgConnection,
    i18n: &I18n,
    id: i64,
) -> Result<Option<Value>, AppError> {
    let row: Option<(Option<String>, Option<String>, Option<String>)> =
        sqlx::query_as("SELECT error, rejection_message, raw FROM incoming_emails WHERE id = $1")
            .bind(id)
            .fetch_optional(&mut *conn)
            .await?;
    let Some((error, rejection_message, raw)) = row else {
        return Ok(None);
    };
    let raw = raw.ok_or(Unsupported("an incoming email without its raw message"))?;
    Ok(Some(incoming_details(
        i18n,
        error,
        rejection_message,
        &raw,
    )?))
}

/// Why incoming_from_bounced found nothing: the exception's message.
pub enum NotBounced {
    InvalidParameters,
    NotFound,
}

/// `incoming_from_bounced`: the bounced log's key (as Rails writes the
/// uuid, without dashes) in the bounce addresses it tries in turn.
pub async fn incoming_from_bounced(
    conn: &mut PgConnection,
    settings: &crate::site_settings::SiteSettings,
    i18n: &I18n,
    id: i64,
) -> Result<Result<Value, NotBounced>, AppError> {
    let key: Option<Option<String>> = sqlx::query_scalar(
        "SELECT replace(bounce_key::text, '-', '') FROM email_logs WHERE id = $1 AND bounced",
    )
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(key) = key.flatten().filter(|k| !k.is_empty()) else {
        return Ok(Err(NotBounced::InvalidParameters));
    };
    let reply = settings.get("reply_by_email_address")?.to_s();
    let mut candidates = Vec::new();
    // bounceable_reply_address?
    if !reply.is_empty() && reply.contains('+') {
        candidates.push(reply.replacen("%{reply_key}", &format!("verp-{key}"), 1));
    }
    let notification = settings.get("notification_email")?.to_s();
    let mut parts = notification.split('@');
    let (local, domain) = (parts.next().unwrap_or(""), parts.next());
    candidates.push(match domain {
        Some(d) => format!("{local}+verp-{key}@{d}"),
        None => format!("{local}+verp-{key}@"),
    });
    // The legacy `subdomain+verp-key@root.ext` form.
    let mut parts = reply.split('@');
    let _ = parts.next();
    let domain_parts: Vec<&str> = parts
        .next()
        .map(|d| d.split('.').collect())
        .unwrap_or_default();
    let get = |i: usize| domain_parts.get(i).copied().unwrap_or("");
    candidates.push(format!("{}+verp-{key}@{}.{}", get(0), get(1), get(2)));
    for address in candidates {
        let row: Option<(Option<String>, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT error, rejection_message, raw FROM incoming_emails WHERE to_addresses = $1 \
             ORDER BY id LIMIT 1",
        )
        .bind(&address)
        .fetch_optional(&mut *conn)
        .await?;
        if let Some((error, rejection_message, raw)) = row {
            let raw = raw.ok_or(Unsupported("an incoming email without its raw message"))?;
            return Ok(Ok(incoming_details(i18n, error, rejection_message, &raw)?));
        }
    }
    Ok(Err(NotBounced::NotFound))
}

#[cfg(test)]
mod details_tests {
    use super::*;

    #[test]
    fn underscores_class_names() {
        assert_eq!(
            underscore("BadDestinationAddress"),
            "bad_destination_address"
        );
        assert_eq!(underscore("EmptyEmailError"), "empty_email_error");
        assert_eq!(underscore("HTMLError"), "html_error");
    }

    #[test]
    fn truncates_after_the_hundredth_word() {
        let words: Vec<String> = (1..=101).map(|i| format!("w{i}")).collect();
        let text = words.join(" ");
        assert!(truncate_words(&text, 100).ends_with("w100..."));
        let exact = words[..100].join("  ");
        assert_eq!(truncate_words(&exact, 100), exact);
    }
}
