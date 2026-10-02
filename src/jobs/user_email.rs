//! `Jobs::UserEmail` (app/jobs/regular/user_email.rb) for notification
//! emails: whether to send at all (each refusal a `skipped_email_logs`
//! row, as Rails keeps them), then the mailer and Email::Sender. Digests,
//! account emails and messages are not ported.

use serde_json::Value;
use sqlx::PgConnection;

use crate::email::notification::{self, Request};
use crate::email::sender::{self, Ctx, skip, skip_reasons as r};
use crate::guardian::Guardian;
use crate::posting::revisions::find_post;
use crate::session::current::SessionUser;
use crate::{AppError, Unsupported};

/// The types `UserNotifications` answers for a notification.
const NOTIFICATION_TYPES: [&str; 6] = [
    "user_replied",
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
    let account = crate::email::account::template_for(email_type, true).is_some();
    if !NOTIFICATION_TYPES.contains(&email_type) && !account {
        return Err(Unsupported("this email type").into());
    }
    if args.get("to_address").is_some_and(|a| !a.is_null()) {
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
            "no_email_found",
            Some(user_id),
            post_id,
            r::USER_EMAIL_NO_USER,
        )
        .await;
    };
    let to_address = user
        .email
        .clone()
        .unwrap_or_else(|| "no_email_found".into());
    if user.email.is_none() {
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
    // email_level always (0) sends even to someone online.
    let always = user.email_level == 0;
    let seen_recently = user.seen_recently && !always;
    let notification_type = args.get("notification_type").and_then(Value::as_str);
    let has_notification = notification_read.is_some() || notification_type.is_some();
    if (post_id.is_some() || has_notification) && seen_recently {
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
    let Some(notification_type) = notification_type else {
        return Err(Unsupported("notification emails without a notification type").into());
    };
    if user.mailing_list_mode
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
    sender::send(&mut *conn, ctx, built, email_type, user_id).await?;
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
const CRITICAL_EMAIL_TYPES: [&str; 2] = ["signup", "forgot_password"];

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
    let Some(token) = args.get("email_token").and_then(Value::as_str) else {
        return Err(Unsupported("account emails without a token").into());
    };
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
    let template = crate::email::account::template_for(email_type, has_password)
        .ok_or(Unsupported("this email type"))?;
    let built = crate::email::account::build(&mut *conn, ctx, template, user_id, token).await?;
    sender::send(&mut *conn, ctx, built, email_type, user_id).await?;
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
