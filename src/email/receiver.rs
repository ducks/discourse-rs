//! Port of Email::Receiver (lib/email/receiver.rb) and Email::Processor
//! for a reply by a known user to a reply key: the incoming_emails row,
//! the body (Discourse's markers cut, the reply trimmed), and the post
//! through PostCreator.
//!
//! Everything a reply to a reply key does not reach is refused, each with
//! its reason: rejection emails (what Email::Processor sends back for most
//! errors), staged users, bounces, HTML bodies, attachments, forwarded
//! emails, group and category addresses (email_in), likes and
//! notification levels by email, and messages (PMs).

use std::sync::LazyLock;

use regex::Regex;
use sqlx::PgConnection;

use super::incoming::{self, Incoming};
use super::reply_trimmer;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

/// `IncomingEmail.created_via_types`
fn created_via(source: Option<&str>) -> i32 {
    match source {
        Some("handle_mail") => 1,
        Some("pop3_poll") => 2,
        Some("group_smtp") => 4,
        _ => 0,
    }
}

/// `Receiver.formats`
pub mod formats {
    pub const PLAINTEXT: i32 = 1;
    pub const MARKDOWN: i32 = 2;
}

/// A Receiver error that leaves no rejection email: the incoming email
/// keeps its class name in `error`.
#[derive(Debug)]
struct Silent(&'static str);

/// What process_internal ends with.
enum Ended {
    Done,
    /// An error Email::Processor only records.
    Silent(Silent),
}

/// `previous_replies_regex` and `reply_above_line_regex`, from every
/// locale's translation (vendor/discourse/config/email_markers.tsv).
static MARKERS: LazyLock<(Regex, Regex)> = LazyLock::new(|| {
    let source = include_str!("../../vendor/discourse/config/email_markers.tsv");
    let strings = |key: &str| {
        let mut out: Vec<String> = Vec::new();
        for line in source.lines() {
            if let Some((k, v)) = line.split_once('\t') {
                let v = v.replace("\\\"", "\"").replace("\\\\", "\\");
                if k == key && !out.contains(&v) {
                    out.push(v);
                }
            }
        }
        out.iter()
            .map(|s| regex::escape(s))
            .collect::<Vec<_>>()
            .join("|")
    };
    (
        Regex::new(&format!(
            r"(?is)\A--[- ]\n\*(?:{})\*\n",
            strings("previous_discussion")
        ))
        .expect("the previous replies markers"),
        Regex::new(&format!(r"(?is)\n(?:{})\n", strings("reply_above_line")))
            .expect("the reply above line markers"),
    )
});

/// `reply.split(regex)[0]`: Ruby's split drops trailing empty pieces, so
/// a text that is all marker has no first piece.
fn split_first(text: &str, re: &Regex) -> Option<String> {
    let mut pieces: Vec<&str> = re.split(text).collect();
    while pieces.last().is_some_and(|p| p.is_empty()) {
        pieces.pop();
    }
    pieces.first().map(|p| p.to_string())
}

/// `trim_discourse_markers`
fn trim_discourse_markers(reply: &str) -> Result<String, Unsupported> {
    if crate::ruby::is_blank(reply) {
        return Ok(String::new());
    }
    let (previous, above) = &*MARKERS;
    let reply = split_first(reply, previous).ok_or(Unsupported(
        "an email that is only Discourse's previous replies",
    ))?;
    split_first(&reply, above).ok_or(Unsupported("an email that is only the reply-above line"))
}

fn blank(s: &Option<String>) -> bool {
    s.as_deref().is_none_or(crate::ruby::is_blank)
}

/// `select_body`: the reply text, what was elided, and its format; None
/// when the email has neither a text nor an HTML body.
pub fn select_body(
    mail: &Incoming,
    s: &SiteSettings,
    base_url: &str,
) -> Result<Option<(String, String, i32)>, AppError> {
    let mut text = Incoming::fix_charset(mail.text_part())?;
    let html = Incoming::fix_charset(mail.html_part())?;
    if blank(&text) && blank(&html) {
        return Ok(None);
    }
    let mut elided: Option<String> = None;
    if !blank(&text) {
        let trimmed = trim_discourse_markers(text.as_deref().unwrap_or(""))?;
        if s.get("trim_incoming_emails")?.truthy() {
            match reply_trimmer::trim(&trimmed) {
                Some((t, e)) => {
                    text = Some(t);
                    elided = Some(e);
                }
                None => {
                    text = None;
                    elided = None;
                }
            }
        } else {
            text = Some(trimmed);
            elided = Some(String::new());
        }
    }
    if !blank(&html) && (blank(&text) || s.get("incoming_email_prefer_html")?.truthy()) {
        return Err(Unsupported("HTML email bodies (HtmlToMarkdown)").into());
    }
    let format = formats::PLAINTEXT;
    if s.get("strip_incoming_email_lines")?.truthy() && !blank(&text) {
        text = Some(strip_lines(text.as_deref().unwrap_or("")));
    }
    let unsubscribe = Regex::new(&format!("{base_url}/email/unsubscribe/[0-9a-fA-F]{{64}}"))
        .map_err(|_| Unsupported("a base URL that is not a valid regex"))?;
    let strip = |t: Option<String>| {
        unsubscribe
            .replace_all(t.as_deref().unwrap_or(""), "")
            .into_owned()
    };
    Ok(Some((strip(text), strip(elided), format)))
}

/// `strip_incoming_email_lines`: each line stripped, list items and code
/// blocks left alone.
fn strip_lines(text: &str) -> String {
    let mut in_code: Option<&str> = None;
    let mut out = String::new();
    for line in text.split_inclusive('\n') {
        let mut stripped = reply_trimmer::ruby_strip(line).to_string();
        stripped.push('\n');
        let b = stripped.as_bytes();
        if (b[0] == b'*' || b[0] == b'-' || b[0] == b'+') && b.get(1) == Some(&b' ') {
            out.push_str(line);
            continue;
        }
        if in_code.is_none() && stripped.starts_with("```") {
            in_code = Some("```");
        } else if in_code == Some("```") && stripped.starts_with("```") {
            in_code = None;
        } else if in_code.is_none() && stripped.starts_with("[code") {
            in_code = Some("[code]");
        } else if in_code == Some("[code]") && stripped.starts_with("[/code]") {
            in_code = None;
        }
        if in_code.is_some() {
            out.push_str(line);
        } else {
            out.push_str(&stripped);
        }
    }
    out
}

/// `subject`: the mail's subject without NULs, at most 255 characters,
/// or the default one.
pub fn subject(mail: &Incoming, from_email: Option<&str>, i18n: &crate::i18n::I18n) -> String {
    match &mail.subject {
        Some(s) => s.replace('\0', "").chars().take(255).collect(),
        None => i18n
            .t_with(
                "emails.incoming.default_subject",
                &[("email", from_email.unwrap_or(""))],
            )
            .unwrap_or_default(),
    }
}

static AUTO_SUBJECT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\A\s*(Auto:|Automatic reply|Autosvar|Automatisk svar|Automatisch antwoord|Abwesenheitsnotiz|Risposta Non al computer|Automatisch antwoord|Auto Response|Respuesta automática|Fuori sede|Out of Office|Frånvaro|Réponse automatique)",
    )
    .unwrap()
});
static AUTO_FROM: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(mailer[\-_]?daemon|post[\-_]?master|no[\-_]?reply)@").unwrap()
});
static AUTO_HEADERS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)auto[\-_]?(response|submitted|replied|reply|generated|respond)|holidayreply|machinegenerated",
    )
    .unwrap()
});

/// `is_auto_generated?`
pub fn is_auto_generated(
    mail: &Incoming,
    s: &SiteSettings,
    from_email: Option<&str>,
) -> Result<bool, AppError> {
    let allowlist = s.get("auto_generated_allowlist")?.to_s();
    if let Some(from) = from_email {
        if allowlist.split('|').any(|a| a == from) {
            return Ok(false);
        }
    }
    let precedence = mail.root.field("Precedence").unwrap_or("");
    if Regex::new(r"(?i)list|junk|bulk|auto_reply")
        .unwrap()
        .is_match(precedence)
    {
        return Ok(true);
    }
    if AUTO_FROM.is_match(mail.root.field("From").unwrap_or("")) {
        return Ok(true);
    }
    if mail
        .subject
        .as_deref()
        .is_some_and(|s| AUTO_SUBJECT.is_match(s))
    {
        return Ok(true);
    }
    Ok(AUTO_HEADERS.is_match(&mail.header_text("X-Auto-Response-Suppress")))
}

/// `reply_by_email_address_regex(extract_reply_key, include_verp)`
fn reply_address_regex(s: &SiteSettings, include_verp: bool) -> Result<Option<Regex>, AppError> {
    let main = s.get("reply_by_email_address")?.to_s();
    let mut addresses: Vec<String> = vec![main.clone()];
    addresses.extend(
        s.get("alternative_reply_by_email_addresses")?
            .to_s()
            .split('|')
            .map(str::to_string),
    );
    if include_verp && !main.is_empty() && main.contains('+') {
        addresses.push(main.replacen("%{reply_key}", "verp-%{reply_key}", 1));
    }
    let parts: Vec<String> = addresses
        .iter()
        .filter(|a| !a.is_empty())
        .map(|a| {
            regex::escape(a)
                .replace(r"\+", r"\+?")
                .replace(r"%\{reply_key\}", "([0-9a-fA-F]{32})?")
        })
        .collect();
    if parts.is_empty() {
        return Ok(None);
    }
    Ok(Some(Regex::new(&parts.join("|")).map_err(|_| {
        Unsupported("a reply_by_email_address that is not a valid regex")
    })?))
}

/// The Rails class name an error is recorded under.
fn error_class(name: &str) -> String {
    format!("Email::Receiver::{name}")
}

/// `Email::Processor.process!(mail, source:)`
pub async fn process(state: &AppState, raw: &str, source: Option<&str>) -> Result<(), AppError> {
    if raw.trim().is_empty() {
        return Err(Unsupported("rejection emails (EmptyEmailError)").into());
    }
    let mail = incoming::parse(raw)?;
    let mut conn = state.pool.acquire().await?;
    let s = SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;

    // is_blocked?
    let subject_raw = mail.subject.clone().unwrap_or_default();
    let titles = s.get("ignore_by_title")?.to_s();
    if !titles.is_empty() {
        let escaped: Vec<String> = titles.split('|').map(regex::escape).collect();
        if Regex::new(&format!("(?i){}", escaped.join("|"))).is_ok_and(|r| r.is_match(&subject_raw))
        {
            return Ok(());
        }
    }
    if s.get("ignore_by_title_regex")?.presence().is_some() {
        return Err(Unsupported("ignore_by_title_regex (a Ruby regex)").into());
    }
    let existing: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM incoming_emails WHERE message_id = $1)")
            .bind(&mail.message_id)
            .fetch_one(&mut *conn)
            .await?;
    if existing {
        return Ok(());
    }
    // Email::Validator.ensure_valid!
    let Some(date) = mail.date else {
        return Err(Unsupported("rejection emails (an email without a valid Date)").into());
    };

    // parse_from_field: bounces and forwarded emails are refused.
    let bounce_key = Regex::new(r"\+verp-[0-9a-fA-F]{32}@").unwrap();
    if mail.root.mime_type() == "multipart/report"
        || mail
            .all_destinations()
            .iter()
            .any(|d| bounce_key.is_match(d))
    {
        return Err(Unsupported("bounced emails").into());
    }
    let forwarded = Regex::new(r"(?i)\A[\t\p{Zs}]*(fwd?|tr)[\t\p{Zs}]?:").unwrap();
    if forwarded.is_match(&subject_raw) {
        return Err(Unsupported("forwarded emails").into());
    }
    let (from_email, _from_name) = match &mail.from {
        Some((a, n)) => (Some(a.clone()), Some(n.clone())),
        None => (None, None),
    };
    let from_user: Option<(i32, bool, bool, bool)> = match &from_email {
        Some(email) => sqlx::query_as(
            "SELECT u.id, u.active, u.staged, COALESCE(u.silenced_till > now(), FALSE) FROM users u \
             WHERE u.id = (SELECT user_id FROM user_emails WHERE lower(email) = lower($1) LIMIT 1)",
        )
        .bind(email)
        .fetch_optional(&mut *conn)
        .await?,
        None => None,
    };

    // create_incoming_email
    let limit = s.get("raw_email_max_length")?.to_i().max(0) as usize;
    let cleaned = incoming::clean(&mail, limit)?;
    let join = |list: &[String]| {
        list.iter()
            .map(|a| a.to_lowercase())
            .collect::<Vec<_>>()
            .join(";")
    };
    let incoming_id: i32 = sqlx::query_scalar(
        "INSERT INTO incoming_emails (message_id, raw, subject, from_address, to_addresses, cc_addresses, \
                                      created_via, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, clock_timestamp(), clock_timestamp()) RETURNING id",
    )
    .bind(&mail.message_id)
    .bind(&cleaned)
    .bind(subject(&mail, from_email.as_deref(), &state.i18n))
    .bind(&from_email)
    .bind(join(&mail.to))
    .bind(join(&mail.cc))
    .bind(created_via(source))
    .fetch_one(&mut *conn)
    .await?;

    let ended = process_internal(
        state,
        &mut conn,
        &s,
        &mail,
        raw,
        date,
        incoming_id,
        from_email.as_deref(),
        from_user,
    )
    .await;
    match ended {
        Ok(Ended::Done) => Ok(()),
        Ok(Ended::Silent(Silent(name))) => {
            sqlx::query("UPDATE incoming_emails SET error = $2 WHERE id = $1")
                .bind(incoming_id)
                .bind(error_class(name))
                .execute(&mut *conn)
                .await?;
            Ok(())
        }
        Err(e) => Err(e),
    }
}

#[allow(clippy::too_many_arguments)]
async fn process_internal(
    state: &AppState,
    conn: &mut PgConnection,
    s: &SiteSettings,
    mail: &Incoming,
    raw: &str,
    date: chrono::NaiveDateTime,
    incoming_id: i32,
    from_email: Option<&str>,
    from_user: Option<(i32, bool, bool, bool)>,
) -> Result<Ended, AppError> {
    let Some(from_email) = from_email.filter(|f| !f.is_empty()) else {
        return Ok(Ended::Silent(Silent("NoSenderDetectedError")));
    };
    if reply_address_regex(s, false)?.is_some_and(|r| r.is_match(from_email)) {
        return Ok(Ended::Silent(Silent("FromReplyByAddressError")));
    }
    let screened: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM screened_emails)")
        .fetch_one(&mut *conn)
        .await?;
    if screened {
        return Err(Unsupported("screened emails (ScreenedEmail.should_block?)").into());
    }
    let Some((user_id, active, staged, silenced)) = from_user else {
        return Err(Unsupported("emails from unknown senders (staged users)").into());
    };
    // log_and_validate_user
    sqlx::query("UPDATE incoming_emails SET user_id = $2 WHERE id = $1")
        .bind(incoming_id)
        .bind(user_id)
        .execute(&mut *conn)
        .await?;
    if staged {
        return Err(Unsupported("emails from staged users").into());
    }
    if !active {
        return Err(Unsupported("rejection emails (InactiveUserError)").into());
    }
    if silenced {
        return Err(Unsupported("rejection emails (SilencedUserError)").into());
    }
    let mut recipients: Vec<String> = Vec::new();
    for a in mail.to.iter().chain(&mail.cc).chain(&mail.bcc) {
        let a = a.to_lowercase();
        if !recipients.contains(&a) {
            recipients.push(a);
        }
    }
    if recipients.len() as i64 > s.get("maximum_recipients_per_new_group_email")?.to_i() {
        return Err(Unsupported("rejection emails (TooManyRecipientsError)").into());
    }

    let urls = crate::url::Urls {
        config: &state.config,
        settings: s,
    };
    let base_url = urls.base_url()?;
    let (body, elided, _format) = select_body(mail, s, &base_url)?.unwrap_or_default();
    if mail.attachment_count() > 0 {
        return Err(Unsupported("email attachments").into());
    }
    if crate::ruby::is_blank(&body) {
        return Err(Unsupported("rejection emails (NoBodyDetectedError)").into());
    }
    if is_auto_generated(mail, s, Some(from_email))? {
        sqlx::query("UPDATE incoming_emails SET is_auto_generated = TRUE WHERE id = $1")
            .bind(incoming_id)
            .execute(&mut *conn)
            .await?;
        if s.get("block_auto_generated_emails")?.truthy() {
            return Err(Unsupported("rejection emails (AutoGeneratedEmailError)").into());
        }
    }
    // subscription_action_for
    if s.get("unsubscribe_via_email")?.truthy() {
        let said = |t: &str| t.to_lowercase() == "unsubscribe";
        if said(&subject(mail, Some(from_email), &state.i18n)) || said(&body) {
            return Err(Unsupported("unsubscribing by email").into());
        }
    }
    if s.get("email_in")?.truthy() {
        return Err(Unsupported("group and category addresses (email_in)").into());
    }
    if !s.get("find_related_post_with_key")?.truthy() {
        return Err(Unsupported("finding the related post without a key").into());
    }
    // find_related_post(force: true): a reply to a post in a category
    // needs reply by email on.
    if let Some(post_id) = related_post(conn, state, s, mail).await? {
        let in_category: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM posts p JOIN topics t ON t.id = p.topic_id \
             WHERE p.id = $1 AND t.category_id IS NOT NULL)",
        )
        .bind(post_id)
        .fetch_one(&mut *conn)
        .await?;
        if in_category && !s.get("reply_by_email_enabled")?.truthy() {
            return Err(Unsupported("rejection emails (ReplyNotAllowedError)").into());
        }
    }

    // destinations: the reply keys the addresses carry.
    let Some(reply_re) = reply_address_regex(s, false)? else {
        return Err(Unsupported("rejection emails (BadDestinationAddress)").into());
    };
    let mut destination: Option<(i32, i32)> = None;
    for address in mail.all_destinations() {
        let Some(c) = reply_re.captures(&address) else {
            continue;
        };
        for key in c.iter().skip(1).flatten() {
            let found: Option<(i32, i32)> = sqlx::query_as(
                "SELECT post_id, user_id FROM post_reply_keys WHERE reply_key = $1::uuid",
            )
            .bind(key.as_str())
            .fetch_optional(&mut *conn)
            .await?;
            if found.is_some() {
                destination = found;
                break;
            }
        }
        if destination.is_some() {
            break;
        }
    }
    let Some((post_id, key_user_id)) = destination else {
        return Err(Unsupported("rejection emails (BadDestinationAddress)").into());
    };
    if key_user_id != user_id {
        return Err(Unsupported("replies to someone else's reply key (forwarded keys)").into());
    }

    // create_reply
    #[derive(sqlx::FromRow)]
    struct Target {
        topic_id: i32,
        post_number: i32,
        post_deleted: bool,
        topic_deleted: Option<bool>,
        archetype: Option<String>,
        closed: Option<bool>,
    }
    let target: Option<Target> = sqlx::query_as(
        "SELECT p.topic_id, p.post_number, p.deleted_at IS NOT NULL AS post_deleted, \
                t.deleted_at IS NOT NULL AS topic_deleted, t.archetype, t.closed \
         FROM posts p LEFT JOIN topics t ON t.id = p.topic_id WHERE p.id = $1",
    )
    .bind(post_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(target) = target else {
        return Err(Unsupported("reply keys for posts that are gone").into());
    };
    if target.topic_deleted.is_none_or(|d| d) {
        return Err(Unsupported("rejection emails (TopicNotFoundError)").into());
    }
    if target.archetype.as_deref() == Some("private_message") {
        return Err(Unsupported("replies to messages by email").into());
    }
    let trimmed_body = reply_trimmer::ruby_strip(&body).to_lowercase();
    let like_title = state
        .i18n
        .t("post_action_types.like.title")
        .unwrap_or("Like")
        .to_lowercase();
    if ["+1", "<3", "❤", like_title.as_str()].contains(&trimmed_body.as_str()) {
        return Err(Unsupported("likes by email").into());
    }
    if body.len() <= 40 && ["mute", "track", "watch"].contains(&trimmed_body.as_str()) {
        return Err(Unsupported("notification levels by email").into());
    }
    if target.closed == Some(true) {
        return Err(Unsupported("rejection emails (TopicClosedError)").into());
    }
    let mut raw_post = body.clone();
    if !crate::ruby::is_blank(&elided) && s.get("always_show_trimmed_content")?.truthy() {
        raw_post.push_str(&elided_html(&elided, &state.i18n));
    }

    // create_post
    if s.get("email_in_spam_header")?.to_s() != "none" {
        return Err(Unsupported("spam headers on incoming email").into());
    }
    if mail.root.field("Authentication-Results").is_some() {
        return Err(Unsupported("Authentication-Results on incoming email").into());
    }
    let now: chrono::NaiveDateTime = sqlx::query_scalar("SELECT clock_timestamp()::timestamp")
        .fetch_one(&mut *conn)
        .await?;
    let created_at = date.min(now);
    let session_user = crate::session::current::SessionUser::load(&mut *conn, user_id)
        .await?
        .ok_or(Unsupported("a user that vanished"))?;
    let guardian = crate::guardian::Guardian::for_user(&mut *conn, &session_user).await?;
    let host = crate::pretty_text::Host::from_state(state);
    let ctx = crate::posting::Ctx {
        host: &host,
        settings: s,
        config: &state.config,
        i18n: &state.i18n,
    };
    let outcome = crate::posting::create::create(
        &state.pool,
        &ctx,
        &guardian,
        crate::posting::create::NewPost {
            raw: raw_post,
            topic_id: Some(target.topic_id),
            reply_to_post_number: (!target.post_deleted).then_some(target.post_number),
            email: Some(crate::posting::create::EmailOrigin {
                raw_email: raw.to_string(),
                created_at,
                incoming_email_id: incoming_id,
                message_id: mail.message_id.clone(),
            }),
            ..Default::default()
        },
    )
    .await?;
    match outcome {
        crate::posting::create::Outcome::Created { .. } => Ok(Ended::Done),
        crate::posting::create::Outcome::Invalid(_) => {
            Err(Unsupported("rejection emails (InvalidPost or TooShortPost)").into())
        }
        crate::posting::create::Outcome::Forbidden => {
            Err(Unsupported("rejection emails (ReplyNotAllowedError)").into())
        }
        crate::posting::create::Outcome::InvalidParameter(_) => {
            Err(Unsupported("rejection emails (InvalidPost)").into())
        }
    }
}

/// `find_related_post(force: true)`: the latest post the In-Reply-To and
/// References ids name.
async fn related_post(
    conn: &mut PgConnection,
    state: &AppState,
    s: &SiteSettings,
    mail: &Incoming,
) -> Result<Option<i32>, AppError> {
    let mut ids: Vec<String> = Vec::new();
    for id in mail.in_reply_to.iter().chain(&mail.references) {
        if !id.is_empty() && !ids.contains(id) {
            ids.push(id.clone());
        }
    }
    ids.truncate(5);
    if ids.is_empty() {
        return Ok(None);
    }
    let urls = crate::url::Urls {
        config: &state.config,
        settings: s,
    };
    let host = urls.current_hostname()?;
    let generated = Regex::new(&format!(r"discourse/post/(\d+)@{}", regex::escape(&host)))
        .map_err(|_| Unsupported("a hostname that is not a valid regex"))?;
    let mut post_ids: Vec<i32> = ids
        .iter()
        .filter_map(|id| generated.captures(id).and_then(|c| c[1].parse().ok()))
        .collect();
    let found: Vec<Option<i32>> = sqlx::query_scalar(
        "SELECT id FROM posts WHERE outbound_message_id = ANY($1) \
         UNION ALL SELECT post_id FROM email_logs WHERE message_id = ANY($1) \
         UNION ALL SELECT post_id FROM incoming_emails WHERE message_id = ANY($1)",
    )
    .bind(&ids)
    .fetch_all(&mut *conn)
    .await?;
    post_ids.extend(found.into_iter().flatten());
    if post_ids.is_empty() {
        return Ok(None);
    }
    Ok(sqlx::query_scalar(
        "SELECT id FROM posts WHERE id = ANY($1) ORDER BY created_at DESC LIMIT 1",
    )
    .bind(&post_ids)
    .fetch_optional(&mut *conn)
    .await?)
}

/// `Email::Receiver.elided_html(elided)`
fn elided_html(elided: &str, i18n: &crate::i18n::I18n) -> String {
    format!(
        "\n\n<details class='elided'>\n<summary title='{}'>&#183;&#183;&#183;</summary>\n\n{elided}\n\n</details>\n",
        i18n.t("emails.incoming.show_trimmed_content").unwrap_or("")
    )
}
