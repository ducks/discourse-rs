//! `Jobs::UserEmail` (app/jobs/regular/user_email.rb) for notification
//! emails: whether to send at all (each refusal a `skipped_email_logs`
//! row, as Rails keeps them), then the mailer and Email::Sender: the
//! notification types, account emails, chat's summary and digests (whose
//! every attempt is stamped on user_stats).

use serde_json::Value;
use sqlx::PgConnection;

use crate::email::notification::{self, Request};
use crate::email::sender::{self, Ctx, skip, skip_reasons as r};
use crate::guardian::Guardian;
use crate::posting::revisions::find_post;
use crate::session::current::SessionUser;
use crate::{AppError, Unsupported};

/// The types `UserNotifications` answers for a notification.
const NOTIFICATION_TYPES: [&str; 8] = [
    "user_invited_to_private_message",
    "user_replied",
    "user_private_message",
    "user_quoted",
    "user_linked",
    "user_mentioned",
    "user_posted",
    "user_watching_first_post",
];

#[derive(sqlx::FromRow)]
struct Recipient {
    email: Option<String>,
    suspended: bool,
    anonymous: bool,
    seen_recently: bool,
    email_level: i32,
    mailing_list_mode: bool,
    mailing_list_mode_frequency: i32,
    bounce_score: Option<f64>,
}

pub async fn run(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    posting: &crate::posting::Ctx<'_>,
    args: &Value,
    critical: bool,
) -> Result<(), AppError> {
    let quit = !critical && ctx.settings.get("disable_emails")?.to_s() == "yes";
    send_user_email(conn, ctx, posting, args, critical).await?;
    // Every attempt at a digest is recorded, skipped or not.
    if !quit && args.get("type").and_then(Value::as_str) == Some("digest") {
        sqlx::query("UPDATE user_stats SET digest_attempted_at = $2 WHERE user_id = $1")
            .bind(
                args.get("user_id")
                    .and_then(Value::as_i64)
                    .map(|v| v as i32),
            )
            .bind(crate::clock::now_naive())
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

async fn send_user_email(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    posting: &crate::posting::Ctx<'_>,
    args: &Value,
    critical: bool,
) -> Result<(), AppError> {
    let s = ctx.settings;
    let user_id = args
        .get("user_id")
        .and_then(Value::as_i64)
        .map(|v| v as i32);
    let email_type = args.get("type").and_then(Value::as_str).unwrap_or("");
    let (Some(user_id), false) = (user_id, email_type.is_empty()) else {
        return Err(Unsupported("user_email without a user or type (InvalidParameters)").into());
    };
    // critical_user_email ignores disable_emails (quit_email_early?).
    if !critical && s.get("disable_emails")?.to_s() == "yes" {
        return Ok(());
    }
    let account = crate::email::account::template_for(email_type, true).is_some()
        || crate::email::account::plain_template(email_type).is_some();
    // Chat's UserNotifications#chat_summary.
    let chat_summary = email_type == "chat_summary";
    let digest = email_type == "digest";
    if !NOTIFICATION_TYPES.contains(&email_type) && !account && !chat_summary && !digest {
        return Err(Unsupported("this email type").into());
    }
    let given_address = args
        .get("to_address")
        .and_then(Value::as_str)
        .filter(|a| !a.trim().is_empty())
        .map(str::to_string);
    if given_address.is_some() && !chat_summary && !account {
        return Err(Unsupported("user_email with an address").into());
    }
    if !account && args.get("email_token").is_some() {
        return Err(Unsupported("notification emails with a token").into());
    }
    let post_id = args
        .get("post_id")
        .and_then(Value::as_i64)
        .map(|v| v as i32);

    // send_user_email
    let window = s.get("email_time_window_mins")?.to_i();
    let user: Option<Recipient> = sqlx::query_as(
        "SELECT (SELECT email FROM user_emails WHERE user_id = u.id AND \"primary\" LIMIT 1) AS email, \
                COALESCE(u.suspended_till > now(), FALSE) AS suspended, \
                EXISTS (SELECT 1 FROM anonymous_users a WHERE a.user_id = u.id) AS anonymous, \
                COALESCE(u.last_seen_at > now() - make_interval(mins => $2), FALSE) AS seen_recently, \
                o.email_level, o.mailing_list_mode, o.mailing_list_mode_frequency, \
                (SELECT bounce_score::float8 FROM user_stats WHERE user_id = u.id) AS bounce_score \
         FROM users u JOIN user_options o ON o.user_id = u.id WHERE u.id = $1",
    )
    .bind(user_id)
    .bind(window as i32)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(user) = user else {
        return skip(
            conn,
            email_type,
            given_address.as_deref().unwrap_or("no_email_found"),
            Some(user_id),
            post_id,
            r::USER_EMAIL_NO_USER,
        )
        .await;
    };
    let to_address = given_address
        .clone()
        .or_else(|| user.email.clone())
        .unwrap_or_else(|| "no_email_found".into());
    if to_address == "no_email_found" {
        return skip(
            conn,
            email_type,
            &to_address,
            Some(user_id),
            post_id,
            r::USER_EMAIL_NO_EMAIL,
        )
        .await;
    }
    let session_user = SessionUser::load(&mut *conn, user_id)
        .await?
        .ok_or(Unsupported("a user that vanished"))?;
    let guardian = Guardian::for_user(&mut *conn, &session_user).await?;
    if let Some(post_id) = post_id {
        let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM posts WHERE id = $1)")
            .bind(post_id)
            .fetch_one(&mut *conn)
            .await?;
        if !exists {
            return skip(
                conn,
                email_type,
                &to_address,
                Some(user_id),
                Some(post_id),
                r::USER_EMAIL_POST_NOT_FOUND,
            )
            .await;
        }
        if find_post(&mut *conn, posting, &guardian, post_id)
            .await?
            .is_none()
        {
            return skip(
                conn,
                email_type,
                &to_address,
                Some(user_id),
                Some(post_id),
                r::USER_EMAIL_ACCESS_DENIED,
            )
            .await;
        }
    }
    let notification_id = args.get("notification_id").and_then(Value::as_i64);
    let notification_read: Option<bool> = match notification_id {
        Some(id) => {
            sqlx::query_scalar("SELECT read FROM notifications WHERE id = $1")
                .bind(id)
                .fetch_optional(&mut *conn)
                .await?
        }
        None => None,
    };

    // message_for_email
    if user.anonymous {
        return skip(
            conn,
            email_type,
            &to_address,
            Some(user_id),
            post_id,
            r::USER_EMAIL_ANONYMOUS_USER,
        )
        .await;
    }
    if user.suspended {
        return skip(
            conn,
            email_type,
            &to_address,
            Some(user_id),
            post_id,
            r::USER_EMAIL_USER_SUSPENDED_NOT_PM,
        )
        .await;
    }
    if digest {
        return digest_email(conn, ctx, user_id, &to_address, &user).await;
    }
    // email_level always (0) sends even to someone online.
    let always = user.email_level == 0;
    // force_respect_seen_recently (chat's summary) keeps it even then.
    let force = args["force_respect_seen_recently"] == true;
    let seen_recently = user.seen_recently && (force || !always);
    let notification_type = args.get("notification_type").and_then(Value::as_str);
    let has_notification = notification_read.is_some() || notification_type.is_some();
    if (post_id.is_some() || has_notification || force) && seen_recently {
        return skip(
            conn,
            email_type,
            &to_address,
            Some(user_id),
            post_id,
            r::USER_EMAIL_SEEN_RECENTLY,
        )
        .await;
    }
    if account {
        return account_email(conn, ctx, args, email_type, user_id, &to_address, &user).await;
    }
    if chat_summary {
        return chat_summary_email(conn, ctx, email_type, user_id, &to_address, &user).await;
    }
    let Some(notification_type) = notification_type else {
        return Err(Unsupported("notification emails without a notification type").into());
    };
    // UserOption#mailing_list_mode reads false while the site disables it.
    if user.mailing_list_mode
        && !s.get("disable_mailing_list_mode")?.truthy()
        && user.mailing_list_mode_frequency > 0
        && [
            "posted",
            "replied",
            "mentioned",
            "group_mentioned",
            "quoted",
        ]
        .contains(&notification_type)
    {
        return Ok(());
    }
    let post_seen = match post_id {
        Some(post_id) => sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM post_timings pt JOIN posts p ON p.topic_id = pt.topic_id \
               AND p.post_number = pt.post_number WHERE p.id = $1 AND pt.user_id = $2)",
        )
        .bind(post_id)
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?,
        None => false,
    };
    if !always && (notification_read == Some(true) || post_seen) {
        return skip(
            conn,
            email_type,
            &to_address,
            Some(user_id),
            post_id,
            r::USER_EMAIL_NOTIFICATION_ALREADY_READ,
        )
        .await;
    }
    // skip_email_for_post
    let Some(post_id) = post_id else {
        return Err(Unsupported("notification emails without a post").into());
    };
    let (topic_present, author_present, user_deleted): (bool, bool, bool) = sqlx::query_as(
        "SELECT t.id IS NOT NULL, u.id IS NOT NULL, p.user_deleted FROM posts p \
         LEFT JOIN topics t ON t.id = p.topic_id AND t.deleted_at IS NULL \
         LEFT JOIN users u ON u.id = p.user_id WHERE p.id = $1",
    )
    .bind(post_id)
    .fetch_one(&mut *conn)
    .await?;
    if !topic_present {
        return skip(
            conn,
            email_type,
            &to_address,
            Some(user_id),
            Some(post_id),
            r::USER_EMAIL_TOPIC_NIL,
        )
        .await;
    }
    if !author_present {
        return skip(
            conn,
            email_type,
            &to_address,
            Some(user_id),
            Some(post_id),
            r::USER_EMAIL_POST_USER_DELETED,
        )
        .await;
    }
    if user_deleted {
        return skip(
            conn,
            email_type,
            &to_address,
            Some(user_id),
            Some(post_id),
            r::USER_EMAIL_POST_DELETED,
        )
        .await;
    }
    if !always && post_seen {
        return skip(
            conn,
            email_type,
            &to_address,
            Some(user_id),
            Some(post_id),
            r::USER_EMAIL_ALREADY_READ,
        )
        .await;
    }
    // EmailLog.reached_max_emails?, then the bounce score.
    let max = s.get("max_emails_per_day_per_user")?.to_i();
    if max > 0 {
        let sent: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM email_logs WHERE created_at > now() - interval '1 day' AND user_id = $1",
        )
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
        if sent >= max {
            return skip(
                conn,
                email_type,
                &to_address,
                Some(user_id),
                Some(post_id),
                r::EXCEEDED_EMAILS_LIMIT,
            )
            .await;
        }
    }
    let bounce_score = user.bounce_score.unwrap_or(0.0);
    if bounce_score >= s.get("bounce_score_threshold")?.to_f() {
        return skip(
            conn,
            email_type,
            &to_address,
            Some(user_id),
            Some(post_id),
            r::EXCEEDED_BOUNCES_LIMIT,
        )
        .await;
    }
    // EmailLog.unique_email_per_post: one email per post and user.
    let already: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM email_logs WHERE post_id = $1 AND user_id = $2)",
    )
    .bind(post_id)
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?;
    if already {
        return Ok(());
    }
    let data = args
        .get("notification_data_hash")
        .cloned()
        .unwrap_or(Value::Null);
    let built = notification::build(
        &mut *conn,
        ctx,
        &Request {
            email_type,
            notification_type,
            data: &data,
            user_id,
            post_id,
        },
    )
    .await?;
    sender::send(&mut *conn, ctx, built, email_type, Some(user_id)).await?;
    let erode = s.get("bounce_score_erode_on_send")?.to_f();
    if bounce_score > erode {
        sqlx::query("UPDATE user_stats SET bounce_score = bounce_score - $2 WHERE user_id = $1")
            .bind(user_id)
            .bind(erode)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

/// `EmailLog::CRITICAL_EMAIL_TYPES` this port sends.
const CRITICAL_EMAIL_TYPES: [&str; 7] = [
    "signup",
    "forgot_password",
    "confirm_new_email",
    "confirm_old_email",
    "confirm_old_email_add",
    "notify_old_email",
    "notify_old_email_add",
];

/// The rest of message_for_email for an account email: the daily and
/// bounce limits (critical types exempt), then the token template.
async fn account_email(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    args: &Value,
    email_type: &str,
    user_id: i32,
    to_address: &str,
    user: &Recipient,
) -> Result<(), AppError> {
    let s = ctx.settings;
    let plain = crate::email::account::plain_template(email_type);
    let token = args.get("email_token").and_then(Value::as_str);
    if plain.is_none() && token.is_none() {
        return Err(Unsupported("account emails without a token").into());
    }
    let critical = CRITICAL_EMAIL_TYPES.contains(&email_type);
    let max = s.get("max_emails_per_day_per_user")?.to_i();
    if max > 0 && !critical {
        let sent: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM email_logs WHERE created_at > now() - interval '1 day' AND user_id = $1",
        )
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
        if sent >= max {
            return skip(
                conn,
                email_type,
                to_address,
                Some(user_id),
                None,
                r::EXCEEDED_EMAILS_LIMIT,
            )
            .await;
        }
    }
    let bounce_score = user.bounce_score.unwrap_or(0.0);
    if !critical && bounce_score >= s.get("bounce_score_threshold")?.to_f() {
        return skip(
            conn,
            email_type,
            to_address,
            Some(user_id),
            None,
            r::EXCEEDED_BOUNCES_LIMIT,
        )
        .await;
    }
    let has_password: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM user_passwords WHERE user_id = $1)")
            .bind(user_id)
            .fetch_one(&mut *conn)
            .await?;
    let mut built = match (plain, token) {
        (Some((template, job_arg, template_arg)), _) => {
            let value = if job_arg.is_empty() {
                ""
            } else {
                args.get(job_arg).and_then(Value::as_str).unwrap_or("")
            };
            crate::email::account::build_with(
                &mut *conn,
                ctx,
                template,
                user_id,
                template_arg,
                value,
            )
            .await?
        }
        (None, Some(token)) => {
            let mut template = crate::email::account::template_for(email_type, has_password)
                .ok_or(Unsupported("this email type"))?;
            // EmailChangeRequest.find_by_new_token: an admin's request for
            // someone else names the admin.
            if email_type == "confirm_new_email" {
                let by_admin: Option<bool> = sqlx::query_scalar(
                    "SELECT COALESCE(r.admin, FALSE) AND c.requested_by_user_id IS DISTINCT FROM c.user_id \
                     FROM email_change_requests c \
                     JOIN email_tokens t ON t.id = c.new_email_token_id \
                     LEFT JOIN users r ON r.id = c.requested_by_user_id \
                     WHERE t.token_hash = $1 ORDER BY c.id DESC LIMIT 1",
                )
                .bind(crate::accounts::hash_token(token))
                .fetch_optional(&mut *conn)
                .await?;
                if by_admin == Some(true) {
                    template = "user_notifications.confirm_new_email_via_admin";
                }
            }
            crate::email::account::build(&mut *conn, ctx, template, user_id, token).await?
        }
        (None, None) => return Err(Unsupported("account emails without a token").into()),
    };
    // message.to = to_address
    for (name, value) in built.message.headers.iter_mut() {
        if name == "To" {
            *value = to_address.to_string();
        }
    }
    sender::send(&mut *conn, ctx, built, email_type, Some(user_id)).await?;
    let erode = s.get("bounce_score_erode_on_send")?.to_f();
    if bounce_score > erode {
        sqlx::query("UPDATE user_stats SET bounce_score = bounce_score - $2 WHERE user_id = $1")
            .bind(user_id)
            .bind(erode)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

/// The rest of message_for_email for chat's summary: the daily and bounce
/// limits, then the mailer, which may find nothing to send.
async fn chat_summary_email(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    email_type: &str,
    user_id: i32,
    to_address: &str,
    user: &Recipient,
) -> Result<(), AppError> {
    let s = ctx.settings;
    let max = s.get("max_emails_per_day_per_user")?.to_i();
    if max > 0 {
        let sent: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM email_logs WHERE created_at > now() - interval '1 day' AND user_id = $1",
        )
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
        if sent >= max {
            return skip(
                conn,
                email_type,
                to_address,
                Some(user_id),
                None,
                r::EXCEEDED_EMAILS_LIMIT,
            )
            .await;
        }
    }
    let bounce_score = user.bounce_score.unwrap_or(0.0);
    if bounce_score >= s.get("bounce_score_threshold")?.to_f() {
        return skip(
            conn,
            email_type,
            to_address,
            Some(user_id),
            None,
            r::EXCEEDED_BOUNCES_LIMIT,
        )
        .await;
    }
    // A mailer that builds nothing sends nothing, and logs nothing.
    let Some(built) =
        crate::plugins::chat::summary_email::build(&mut *conn, ctx, user_id, to_address).await?
    else {
        return Ok(());
    };
    sender::send(&mut *conn, ctx, built, email_type, Some(user_id)).await?;
    let erode = s.get("bounce_score_erode_on_send")?.to_f();
    if bounce_score > erode {
        sqlx::query("UPDATE user_stats SET bounce_score = bounce_score - $2 WHERE user_id = $1")
            .bind(user_id)
            .bind(erode)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

/// The digest's own checks in message_for_email (those of
/// enqueue_digest_emails again, in case something changed since): each
/// fails with no skipped log at all. Then the daily limit, and the mailer,
/// which builds nothing when there is nothing popular; the bounce score
/// erodes all the same, as the mailer's (empty) message still counts.
async fn digest_email(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    user_id: i32,
    to_address: &str,
    user: &Recipient,
) -> Result<(), AppError> {
    let s = ctx.settings;
    if s.get("disable_digest_emails")?.truthy() || s.get("private_email")?.truthy() {
        return Ok(());
    }
    #[derive(sqlx::FromRow)]
    struct DigestRecipient {
        active: bool,
        staged: bool,
        has_stat: bool,
        has_option: bool,
        email_digests: Option<bool>,
        mailing_list_mode: Option<bool>,
        delay: i64,
        attempted_recently: bool,
        seen_recently: bool,
    }
    let default_delay = s.get("default_email_digest_frequency")?.to_i();
    let now = crate::clock::now_naive();
    let d: DigestRecipient = sqlx::query_as(
        "SELECT u.active, u.staged, s.user_id IS NOT NULL AS has_stat, o.user_id IS NOT NULL AS has_option, \
                o.email_digests, o.mailing_list_mode, \
                COALESCE(o.digest_after_minutes, $2)::int8 AS delay, \
                COALESCE(s.digest_attempted_at > $3 - make_interval(mins => COALESCE(o.digest_after_minutes, $2)::int), FALSE) AS attempted_recently, \
                COALESCE(u.last_seen_at > $3 - make_interval(mins => COALESCE(o.digest_after_minutes, $2)::int), FALSE) AS seen_recently \
         FROM users u LEFT JOIN user_stats s ON s.user_id = u.id \
         LEFT JOIN user_options o ON o.user_id = u.id WHERE u.id = $1",
    )
    .bind(user_id)
    .bind(default_delay as i32)
    .bind(now)
    .fetch_one(&mut *conn)
    .await?;
    // bot, anonymous (skipped before), inactive, suspended (skipped
    // before), staged
    if user_id < 0 || user.anonymous || !d.active || user.suspended || d.staged {
        return Ok(());
    }
    if !d.has_stat {
        return Ok(());
    }
    let bounce_score = user.bounce_score.unwrap_or(0.0);
    if bounce_score >= s.get("bounce_score_threshold")?.to_f() {
        return Ok(());
    }
    // UserOption#mailing_list_mode reads false while the site disables it.
    let mailing_list_mode =
        d.mailing_list_mode == Some(true) && !s.get("disable_mailing_list_mode")?.truthy();
    if !d.has_option || d.email_digests != Some(true) || mailing_list_mode {
        return Ok(());
    }
    if d.delay <= 0 || d.attempted_recently || d.seen_recently {
        return Ok(());
    }
    let max = s.get("max_emails_per_day_per_user")?.to_i();
    if max > 0 {
        let sent: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM email_logs WHERE created_at > now() - interval '1 day' AND user_id = $1",
        )
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
        if sent >= max {
            return skip(
                conn,
                "digest",
                to_address,
                Some(user_id),
                None,
                r::EXCEEDED_EMAILS_LIMIT,
            )
            .await;
        }
    }
    let built = crate::email::digest::build(
        &mut *conn,
        ctx,
        user_id,
        to_address,
        &crate::email::digest::Options::default(),
    )
    .await?;
    if let Some(built) = built {
        sender::send(&mut *conn, ctx, built, "digest", Some(user_id)).await?;
    }
    let erode = s.get("bounce_score_erode_on_send")?.to_f();
    if bounce_score > erode {
        sqlx::query("UPDATE user_stats SET bounce_score = bounce_score - $2 WHERE user_id = $1")
            .bind(user_id)
            .bind(erode)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}
