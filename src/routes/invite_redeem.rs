//! Redeeming invites: InvitesController#show and
//! #perform_accept_invitation, InviteRedeemer (a signed-in user, or a new
//! one created from the invite), `Invite.redeem_for_existing_user` on
//! activation, and the invite branch of SessionController's login codes
//! (Invite::RequestEmailCode, Invite::RedeemWithEmailCode).

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};
use sqlx::{Acquire, PgConnection};

use crate::accounts::token_scopes;
use crate::guardian::Guardian;
use crate::invites::{self, Invite, emailed_status};
use crate::params;
use crate::rate_limiter::RateLimiter;
use crate::session::current::{AuthGuardian, SessionUser};
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

/// `Notification.types`
const INVITEE_ACCEPTED: i32 = 8;
const INVITED_TO_TOPIC: i32 = 13;

/// What InviteRedeemer is given.
#[derive(Debug, Clone, Default)]
pub(crate) struct RedeemArgs {
    pub email: Option<String>,
    pub username: Option<String>,
    pub name: Option<String>,
    pub password: Option<String>,
    pub user_custom_fields: Option<Value>,
    pub ip: Option<String>,
    pub email_token: Option<String>,
    pub email_verified: bool,
    pub redeeming_user: Option<i32>,
}

/// What redeeming came to.
pub(crate) enum Redeemed {
    /// The user the invite went to, and whether it created them.
    User { id: i32, created: bool },
    /// `nil`: not redeemable, or nothing to redeem.
    Nil,
    /// RecordNotSaved, RecordInvalid or Invite::UserExists: its message.
    Failed(String),
}

fn t(state: &AppState, key: &str) -> String {
    state.i18n.t(key).unwrap_or(key).to_string()
}

/// The user's primary email.
async fn primary_email(conn: &mut PgConnection, user_id: i32) -> Result<String, AppError> {
    let email: Option<String> = sqlx::query_scalar(
        "SELECT email FROM user_emails WHERE user_id = $1 AND \"primary\" LIMIT 1",
    )
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(email.unwrap_or_default())
}

/// `can_redeem_invite?`: false, true, or the RecordNotSaved / UserExists
/// it raises.
async fn can_redeem(
    conn: &mut PgConnection,
    state: &AppState,
    invite: &Invite,
    email: &str,
    args: &RedeemArgs,
) -> Result<Result<bool, String>, AppError> {
    if !invite.redeemable(&mut *conn).await? {
        return Ok(Ok(false));
    }
    if email.trim().is_empty() {
        return Ok(Ok(false));
    }
    if invite.is_email_invite() {
        let any: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM invited_users WHERE invite_id = $1)")
                .bind(invite.id)
                .fetch_one(&mut *conn)
                .await?;
        if any {
            return Ok(Ok(false));
        }
    }
    let email_to_check = if args.email_verified {
        email.to_string()
    } else {
        match args.redeeming_user {
            Some(id) => primary_email(&mut *conn, id).await?,
            None => email.to_string(),
        }
    };
    if invite.email().is_some() && !invite.email_matches(&email_to_check) {
        return Ok(Err(t(state, "invite.not_matching_email")));
    }
    if invite.domain().is_some() && !invite.domain_matches(&email_to_check) {
        return Ok(Err(t(state, "invite.domain_not_allowed")));
    }
    // `redeeming_user ||= ...` assigns a local: always the lookup by email.
    let existing: Option<i32> = sqlx::query_scalar(
        "SELECT u.id FROM users u WHERE NOT u.admin AND NOT u.staged AND u.id IN \
         (SELECT user_id FROM user_emails WHERE lower(email) = lower($1)) ORDER BY u.id LIMIT 1",
    )
    .bind(email)
    .fetch_optional(&mut *conn)
    .await?;
    if let Some(user_id) = existing {
        let redeemed = invite.redeemed_by_user(&mut *conn, user_id).await?;
        if redeemed {
            return Ok(Err(t(state, "invite.existing_user_already_redemeed")));
        }
    }
    Ok(Ok(true))
}

/// `Invite#redeem` (InviteRedeemer#redeem), in a savepoint that a
/// failure rolls back.
pub(crate) async fn redeem(
    conn: &mut PgConnection,
    state: &AppState,
    s: &SiteSettings,
    invite: &Invite,
    args: &RedeemArgs,
) -> Result<Redeemed, AppError> {
    if !invite.redeemable(&mut *conn).await? {
        return Ok(Redeemed::Nil);
    }
    if invite.admin || invite.moderator {
        return Err(Unsupported("redeeming admin and moderator invites").into());
    }
    // ensure_email_is_present!
    let given = args.email.clone().filter(|e| !crate::ruby::is_blank(e));
    let email = match (&given, args.redeeming_user) {
        (None, _) if invite.is_email_invite() => invite.email.clone().unwrap_or_default(),
        (_, Some(id)) if !args.email_verified => primary_email(&mut *conn, id).await?,
        _ => given.clone().unwrap_or_default(),
    };
    if email.trim().is_empty() {
        return Err(Unsupported("an invite redeemed without an email (InvalidParameters)").into());
    }
    let mut sp = conn.begin().await?;
    match redeem_in(&mut sp, state, s, invite, &email, args).await? {
        Ok(redeemed) => {
            sp.commit().await?;
            Ok(redeemed)
        }
        Err(message) => {
            sp.rollback().await?;
            Ok(Redeemed::Failed(message))
        }
    }
}

async fn redeem_in(
    conn: &mut PgConnection,
    state: &AppState,
    s: &SiteSettings,
    invite: &Invite,
    email: &str,
    args: &RedeemArgs,
) -> Result<Result<Redeemed, String>, AppError> {
    match can_redeem(&mut *conn, state, invite, email, args).await? {
        Ok(true) => {}
        Ok(false) => return Ok(Ok(Redeemed::Nil)),
        Err(message) => return Ok(Err(message)),
    }
    // mark_invite_redeemed: the invite locked, checked again.
    sqlx::query("SELECT id FROM invites WHERE id = $1 FOR UPDATE NOWAIT")
        .bind(invite.id)
        .execute(&mut *conn)
        .await?;
    let invite = Invite::find_with_deleted(&mut *conn, invite.id)
        .await?
        .ok_or(Unsupported("an invite gone while it was redeemed"))?;
    match can_redeem(&mut *conn, state, &invite, email, args).await? {
        Ok(true) => {}
        Ok(false) => return Ok(Ok(Redeemed::Nil)),
        Err(message) => return Ok(Err(message)),
    }
    let record_id: i64 = sqlx::query_scalar(
        "INSERT INTO invited_users (invite_id, redeemed_at, created_at, updated_at) \
         VALUES ($1, clock_timestamp(), clock_timestamp(), clock_timestamp()) RETURNING id",
    )
    .bind(invite.id)
    .fetch_one(&mut *conn)
    .await?;
    sqlx::query("UPDATE invites SET redemption_count = redemption_count + 1 WHERE id = $1")
        .bind(invite.id)
        .execute(&mut *conn)
        .await?;
    // delete_duplicate_invites
    if invite.is_email_invite() {
        sqlx::query(
            "DELETE FROM invites WHERE id IN (SELECT invites.id FROM invites \
             LEFT JOIN invited_users ON invites.id = invited_users.invite_id \
             WHERE invites.deleted_at IS NULL AND invites.max_redemptions_allowed = 1 \
             AND invited_users.user_id IS NULL AND invites.email = $1 AND invites.id <> $2)",
        )
        .bind(email)
        .bind(invite.id)
        .execute(&mut *conn)
        .await?;
    }
    // process_invitation: the user first (invited_user).
    let (user_id, created) = match args.redeeming_user {
        Some(id) => (id, false),
        None => match create_user_from_invite(&mut *conn, state, s, &invite, email, args).await? {
            Ok(id) => (id, true),
            Err(message) => return Ok(Err(message)),
        },
    };
    add_to_private_topics_if_invited(&mut *conn, s, email, user_id).await?;
    add_user_to_groups(&mut *conn, state, s, &invite, user_id).await?;
    // send_welcome_message: the record gets its user.
    sqlx::query(
        "UPDATE invited_users SET user_id = $2, updated_at = clock_timestamp() WHERE id = $1",
    )
    .bind(record_id)
    .bind(user_id)
    .execute(&mut *conn)
    .await?;
    // notify_invitee
    let username: String = sqlx::query_scalar("SELECT username FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
    crate::notification_create::create(
        &mut *conn,
        &state.bus,
        s,
        &crate::notification_create::NewNotification::new(
            INVITEE_ACCEPTED,
            invite.invited_by_id,
            json!({"display_username": username}),
        ),
    )
    .await?;
    Ok(Ok(Redeemed::User {
        id: user_id,
        created,
    }))
}

/// `add_to_private_topics_if_invited`: the messages any invite to this
/// email named, for inviters who may still invite to them.
async fn add_to_private_topics_if_invited(
    conn: &mut PgConnection,
    s: &SiteSettings,
    email: &str,
    user_id: i32,
) -> Result<(), AppError> {
    let rows: Vec<(i32, i32)> = sqlx::query_as(
        "SELECT ti.topic_id, i.invited_by_id FROM topic_invites ti \
         JOIN invites i ON i.id = ti.invite_id AND i.deleted_at IS NULL \
         JOIN topics t ON t.id = ti.topic_id AND t.deleted_at IS NULL \
         WHERE t.archetype = 'private_message' AND i.email = $1 ORDER BY ti.id",
    )
    .bind(email)
    .fetch_all(&mut *conn)
    .await?;
    for (topic_id, inviter_id) in rows {
        let Some(inviter) = SessionUser::load(&mut *conn, inviter_id).await? else {
            continue;
        };
        let guardian = Guardian::for_user(&mut *conn, &inviter).await?;
        let allowed = match invites::topic_for(&mut *conn, s, &guardian, topic_id).await? {
            Some((topic, can_see)) => guardian.can_invite_to(s, &topic, can_see)?,
            None => false,
        };
        if !allowed {
            continue;
        }
        sqlx::query(
            "INSERT INTO topic_allowed_users (topic_id, user_id, created_at, updated_at) \
             SELECT $1, $2, clock_timestamp(), clock_timestamp() \
             WHERE NOT EXISTS (SELECT 1 FROM topic_allowed_users WHERE topic_id = $1 AND user_id = $2)",
        )
        .bind(topic_id)
        .bind(user_id)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// `add_user_to_groups`: the invite's groups the user isn't in, those the
/// inviter may edit, logged as the inviter's.
async fn add_user_to_groups(
    conn: &mut PgConnection,
    state: &AppState,
    s: &SiteSettings,
    invite: &Invite,
    user_id: i32,
) -> Result<(), AppError> {
    let new_ids: Vec<i32> = sqlx::query_scalar(
        "SELECT ig.group_id FROM invited_groups ig JOIN groups g ON g.id = ig.group_id \
         WHERE ig.invite_id = $1 AND NOT EXISTS (SELECT 1 FROM group_users gu \
           WHERE gu.group_id = ig.group_id AND gu.user_id = $2) ORDER BY ig.id",
    )
    .bind(invite.id)
    .bind(user_id)
    .fetch_all(&mut *conn)
    .await?;
    if new_ids.is_empty() {
        return Ok(());
    }
    let Some(inviter) = SessionUser::load(&mut *conn, invite.invited_by_id).await? else {
        return Ok(());
    };
    let guardian = Guardian::for_user(&mut *conn, &inviter).await?;
    let host = crate::pretty_text::Host::from_state(state);
    let promotion = crate::promotion::Ctx {
        settings: s,
        i18n: &state.i18n,
        host: &host,
        config: &state.config,
    };
    let cx = crate::group_manager::Cx {
        state,
        promotion: &promotion,
    };
    for group_id in new_ids {
        let group: crate::groups::BasicGroup = sqlx::query_as(&format!(
            "SELECT {} FROM groups g LEFT JOIN uploads u ON u.id = g.flair_upload_id WHERE g.id = $1",
            crate::groups::BASIC_GROUP_COLUMNS
        ))
        .bind(group_id)
        .fetch_one(&mut *conn)
        .await?;
        let membership = crate::groups::membership(&mut *conn, &guardian, group_id).await?;
        if !crate::groups::can_edit_group(&guardian, s, &group, membership)? {
            continue;
        }
        let Some(managed) = crate::group_manager::find(&mut *conn, group_id).await? else {
            continue;
        };
        crate::group_manager::add(&mut *conn, &cx, &managed, user_id, inviter.id).await?;
    }
    Ok(())
}

/// `UserNameSuggester.sanitize_username` for ASCII names.
fn sanitize_username(name: &str) -> Result<String, AppError> {
    if !name.is_ascii() {
        return Err(Unsupported("suggesting usernames from non-ASCII names").into());
    }
    static INVALID: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"[^\w.-]").unwrap());
    static LEADING: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"^[^\w]+").unwrap());
    static CONFUSING: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"\.(js|json|css|htm|html|xml|jpg|jpeg|png|gif|bmp|ico|tif|tiff|woff)$")
            .unwrap()
    });
    static TRAILING: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"[^A-Za-z0-9]+$").unwrap());
    static REPEATED: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"[-_.]{2,}").unwrap());
    let name = INVALID.replace_all(name, "_");
    let name = LEADING.replace_all(&name, "");
    let name = CONFUSING.replace_all(&name, "_");
    let name = TRAILING.replace_all(&name, "");
    Ok(REPEATED.replace_all(&name, "_").into_owned())
}

/// `UserNameSuggester.suggest(email, allow_generic_fallback:)`: a free
/// username from the email's local part (when the site uses emails for
/// suggestions), else the generic fallback.
pub(crate) async fn suggest_from_email(
    conn: &mut PgConnection,
    state: &AppState,
    s: &SiteSettings,
    email: &str,
    allow_generic_fallback: bool,
) -> Result<Option<String>, AppError> {
    const GENERIC: [&str; 11] = [
        "i",
        "me",
        "info",
        "support",
        "admin",
        "webmaster",
        "hello",
        "mail",
        "office",
        "contact",
        "team",
    ];
    static EMAIL: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"([^@]+)@([^\.]+)").unwrap());
    let mut name = String::new();
    if s.get("use_email_for_username_and_name_suggestions")?
        .truthy()
    {
        let parsed = match EMAIL.captures(email) {
            Some(c) => {
                let local = c.get(1).map_or("", |m| m.as_str());
                if GENERIC.contains(&local) {
                    c.get(2).map_or("", |m| m.as_str()).to_string()
                } else {
                    local.to_string()
                }
            }
            None => email.to_string(),
        };
        name = sanitize_username(&parsed)?;
    }
    if name.is_empty() {
        if !allow_generic_fallback {
            return Ok(None);
        }
        name = sanitize_username(&t(state, "fallback_username"))?;
        if name.is_empty() {
            name = "user".into();
        }
    }
    Ok(Some(
        super::groups::suggest_username(&mut *conn, s, &name).await?,
    ))
}

/// `InviteRedeemer.create_user_from_invite`: the user saved, maybe
/// activated; Err is the RecordNotSaved or RecordInvalid message.
async fn create_user_from_invite(
    conn: &mut PgConnection,
    state: &AppState,
    s: &SiteSettings,
    invite: &Invite,
    email: &str,
    args: &RedeemArgs,
) -> Result<Result<i32, String>, AppError> {
    if s.get("site_archived")?.truthy() {
        return Err(Unsupported("site_archived").into());
    }
    let email = email.trim().to_lowercase();
    let username = args.username.clone().filter(|u| !crate::ruby::is_blank(u));
    let available = match &username {
        Some(u) if args.email_verified => u.clone(),
        Some(u)
            if crate::signup::username_format_error(s, &state.i18n, u)?.is_none()
                && super::groups::username_available_for(&mut *conn, s, u, Some(&email), false)
                    .await? =>
        {
            u.clone()
        }
        _ => suggest_from_email(&mut *conn, state, s, &email, true)
            .await?
            .ok_or(Unsupported("an invitee without a username"))?,
    };
    if args.email_verified
        && (crate::signup::username_format_error(s, &state.i18n, &available)?.is_some()
            || !super::groups::username_available(&mut *conn, s, &available).await?)
    {
        return Err(Unsupported("a verified invitee's username that is taken or invalid").into());
    }
    let staged: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM users u JOIN user_emails e ON e.user_id = u.id \
         WHERE u.staged AND lower(e.email) = $1)",
    )
    .bind(&email)
    .fetch_one(&mut *conn)
    .await?;
    if staged {
        return Err(Unsupported("redeeming an invite as a staged user (unstage!)").into());
    }
    let must_approve = s.get("must_approve_users")?.truthy();
    if must_approve && s.get("auto_approve_email_domains")?.presence().is_some() {
        return Err(Unsupported("auto-approved email domains on invite redemption").into());
    }
    let approved = !must_approve && s.get("invite_only")?.truthy();
    // user_custom_fields: only with fields defined.
    let has_fields = match &args.user_custom_fields {
        Some(Value::Object(m)) => !m.is_empty(),
        Some(Value::Null) | None => false,
        Some(_) => true,
    };
    if has_fields {
        let defined: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM user_fields)")
            .fetch_one(&mut *conn)
            .await?;
        if defined {
            return Err(Unsupported("user fields on invite redemption").into());
        }
    }
    let password = args.password.clone().filter(|p| !p.is_empty());
    let name = args
        .name
        .clone()
        .filter(|n| !crate::ruby::is_blank(n))
        .or_else(|| (!args.email_verified).then(|| available.clone()));
    if let Some(p) = &password
        && (p.chars().count() > 200
            || crate::accounts::password_error(
                s,
                p,
                &crate::accounts::PasswordOwner {
                    admin: false,
                    username: &available,
                    name: name.as_deref(),
                    email: Some(&email),
                    current: None,
                },
            )?
            .is_some())
    {
        return Err(Unsupported("invite redemption with a password User refuses").into());
    }
    if !s.get("enable_local_logins")?.truthy() {
        return Ok(Err(t(state, "login.incorrect_username_email_or_password")));
    }
    // user.save!: a taken email is the one validation error ported.
    let normalized = crate::signup::normalize_email(&email);
    let taken: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM user_emails WHERE lower(email) = $1 OR ($3 AND lower(normalized_email) = $2))",
    )
    .bind(&email)
    .bind(&normalized)
    .bind(s.get("normalize_emails")?.truthy())
    .fetch_one(&mut *conn)
    .await?;
    if taken {
        let message = state
            .i18n
            .t_with(
                "activerecord.errors.messages.record_invalid",
                &[("errors", "Primary email has already been taken")],
            )
            .unwrap_or_else(|| "Validation failed: Primary email has already been taken".into());
        return Ok(Err(message));
    }
    if name.as_deref().is_some_and(|n| n.chars().count() > 255) {
        return Err(Unsupported("invite redemption validation errors other than the email").into());
    }
    let ip = args.ip.clone().unwrap_or_default();
    if crate::accounts::ip_blocked(&mut *conn, &ip).await? {
        return Err(Unsupported("invite redemption validation errors other than the email").into());
    }
    let payloads = crate::web_hooks::Payloads {
        settings: s,
        i18n: &state.i18n,
        config: &state.config,
    };
    let user_id = crate::signup::insert_user(
        &mut *conn,
        s,
        &payloads,
        &crate::signup::NewUser {
            username: &available,
            name: name.as_deref(),
            email: &email,
            password: password.as_deref(),
            trust_level: s.get("default_invitee_trust_level")?.to_i() as i32,
            ip: Some(ip.as_str()).filter(|i| !i.is_empty()),
            locale: None,
            active: false,
            approved,
            moderator: false,
            custom_fields: &[],
        },
    )
    .await?;
    let verified_by_token = invite.emailed_status != Some(emailed_status::NOT_REQUIRED)
        && invite.email.as_deref() == Some(email.as_str())
        && invite.email_token.is_some()
        && args.email_token.is_some()
        && args.email_token == invite.email_token;
    if args.email_verified || verified_by_token {
        activate(&mut *conn, state, s, user_id, &email).await?;
    }
    Ok(Ok(user_id))
}

/// `User#activate`: a signup token made and confirmed.
async fn activate(
    conn: &mut PgConnection,
    state: &AppState,
    s: &SiteSettings,
    user_id: i32,
    email: &str,
) -> Result<(), AppError> {
    let token =
        crate::accounts::create_email_token(&mut *conn, user_id, email, token_scopes::SIGNUP)
            .await?;
    Box::pin(super::accounts::confirm_email_token(
        state,
        &mut *conn,
        s,
        &token,
        token_scopes::SIGNUP,
    ))
    .await?;
    Ok(())
}

/// `Invite.redeem_for_existing_user(user)`: the invite to the user's
/// email, redeemed by them when it still can be.
pub(crate) async fn redeem_for_existing_user(
    conn: &mut PgConnection,
    state: &AppState,
    s: &SiteSettings,
    user_id: i32,
) -> Result<(), AppError> {
    let email = invites::email_downcase(&primary_email(&mut *conn, user_id).await?);
    let invite: Option<Invite> = sqlx::query_as(&format!(
        "SELECT {} FROM invites WHERE email = $1 AND deleted_at IS NULL ORDER BY id LIMIT 1",
        invites::INVITE_COLUMNS
    ))
    .bind(&email)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(invite) = invite else {
        return Ok(());
    };
    if !invite.redeemable(&mut *conn).await? {
        return Ok(());
    }
    let args = RedeemArgs {
        redeeming_user: Some(user_id),
        ..RedeemArgs::default()
    };
    match Box::pin(redeem(conn, state, s, &invite, &args)).await? {
        Redeemed::Failed(_) => Err(Unsupported("an invite failing to redeem on activation").into()),
        _ => Ok(()),
    }
}

/// `create_topic_invite_notifications(invite, user)`: invited_to_topic
/// for each of the invite's topics the user can see, unless one came in
/// the last hour.
async fn topic_invite_notifications(
    conn: &mut PgConnection,
    state: &AppState,
    s: &SiteSettings,
    invite: &Invite,
    user_id: i32,
) -> Result<(), AppError> {
    let topic_ids = invite.topic_ids(&mut *conn).await?;
    if topic_ids.is_empty() {
        return Ok(());
    }
    let user = SessionUser::load(&mut *conn, user_id)
        .await?
        .ok_or(Unsupported("an invitee gone"))?;
    let guardian = Guardian::for_user(&mut *conn, &user).await?;
    let inviter = SessionUser::load(&mut *conn, invite.invited_by_id)
        .await?
        .ok_or(Unsupported("an inviter gone"))?;
    for topic_id in topic_ids {
        if !matches!(
            invites::topic_for(&mut *conn, s, &guardian, topic_id).await?,
            Some((_, true))
        ) {
            continue;
        }
        let recent: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM notifications WHERE user_id = $1 AND notification_type = $2 \
             AND topic_id = $3 AND post_number = 1 AND created_at > now() - interval '1 hour')",
        )
        .bind(user_id)
        .bind(INVITED_TO_TOPIC)
        .bind(topic_id)
        .fetch_one(&mut *conn)
        .await?;
        if recent {
            continue;
        }
        // Topic#create_invite_notification!
        let screener = crate::user_comm_screener::UserCommScreener::new(
            &mut *conn,
            crate::user_comm_screener::Actor {
                id: inviter.id,
                staff: inviter.admin || inviter.moderator,
            },
            &[user_id],
        )
        .await?;
        if screener.ignoring_or_muting_actor(user_id)? {
            return Err(Unsupported("an invitee ignoring the inviter (NotAllowed)").into());
        }
        let title: String = sqlx::query_scalar("SELECT title FROM topics WHERE id = $1")
            .bind(topic_id)
            .fetch_one(&mut *conn)
            .await?;
        crate::notification_create::create(
            &mut *conn,
            &state.bus,
            s,
            &crate::notification_create::NewNotification::new(
                INVITED_TO_TOPIC,
                user_id,
                json!({
                    "topic_title": title,
                    "display_username": inviter.username,
                    "original_user_id": inviter.id,
                    "original_username": inviter.username,
                }),
            )
            .topic(topic_id, Some(1)),
        )
        .await?;
    }
    Ok(())
}

/// `log_on_user(user)`: a new auth token, the user's logged_in hooks;
/// the auth cookie.
async fn log_on_user(
    conn: &mut PgConnection,
    state: &AppState,
    s: &SiteSettings,
    headers: &HeaderMap,
    peer: Option<std::net::SocketAddr>,
    uri: &Uri,
    user_id: i32,
) -> Result<String, AppError> {
    if crate::session::current::cookie(headers, "anonymous_action").is_some() {
        return Err(Unsupported("replaying an anonymous action after a login").into());
    }
    let user = SessionUser::load(&mut *conn, user_id)
        .await?
        .ok_or(Unsupported("logging in a user that is gone"))?;
    let ip = crate::remote_ip::remote_ip(headers, peer)?;
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let (_, unhashed) = crate::session::token::generate(
        &mut *conn,
        user_id,
        &state.keys.secret_key_base,
        user_agent.as_deref(),
        &ip,
        Some(uri.path()),
    )
    .await?;
    crate::roles::bootstrap_first_admin(&mut *conn, &state.i18n, user_id).await?;
    crate::session::token::enforce_session_count_limit(
        &mut *conn,
        user_id,
        s.get("maximum_session_age")?.to_i(),
    )
    .await?;
    crate::web_hooks::enqueue_user_hooks(
        &mut *conn,
        &crate::web_hooks::Payloads {
            settings: s,
            i18n: &state.i18n,
            config: &state.config,
        },
        "user_logged_in",
        user_id,
    )
    .await?;
    crate::session::current::auth_cookie(&state.keys, s, &user, &unhashed)
}

/// The user's view of the invite's first topic: its path when they can
/// see it.
async fn visible_topic_path(
    conn: &mut PgConnection,
    state: &AppState,
    s: &SiteSettings,
    invite: &Invite,
    user_id: i32,
) -> Result<Option<String>, AppError> {
    let Some(topic_id) = invite.topic_ids(&mut *conn).await?.first().copied() else {
        return Ok(None);
    };
    let user = SessionUser::load(&mut *conn, user_id)
        .await?
        .ok_or(Unsupported("an invitee gone"))?;
    let guardian = Guardian::for_user(&mut *conn, &user).await?;
    if !matches!(
        invites::topic_for(&mut *conn, s, &guardian, topic_id).await?,
        Some((_, true))
    ) {
        return Ok(None);
    }
    let slug: Option<String> = sqlx::query_scalar("SELECT slug FROM topics WHERE id = $1")
        .bind(topic_id)
        .fetch_one(&mut *conn)
        .await?;
    let slug = slug.ok_or(Unsupported("a topic without its slug"))?;
    Ok(Some(format!(
        "{}/t/{slug}/{topic_id}",
        state.config.globals.relative_url_root()
    )))
}

/// `ensure_invites_allowed`: local logins, DiscourseConnect or an
/// enabled auth provider.
fn invites_allowed(s: &SiteSettings) -> Result<bool, AppError> {
    if s.get("enable_local_logins")?.truthy() || s.get("enable_discourse_connect")?.truthy() {
        return Ok(true);
    }
    Err(Unsupported("invites with only external logins (Discourse.enabled_auth_providers)").into())
}

/// `post_process_invite(user)` and the topic notifications, after a
/// redemption.
async fn after_redeem(
    conn: &mut PgConnection,
    state: &AppState,
    s: &SiteSettings,
    invite: &Invite,
    user_id: i32,
    process: bool,
) -> Result<(), AppError> {
    let (active, staff, username): (bool, bool, String) =
        sqlx::query_as("SELECT active, admin OR moderator, username FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&mut *conn)
            .await?;
    // user.send_welcome_message, which redeeming always sets.
    if s.get("send_welcome_message")?.truthy() {
        crate::jobs::enqueue(
            &mut *conn,
            "send_system_message",
            json!({"user_id": user_id, "message_type": "welcome_invite"}),
        )
        .await?;
    }
    if staff {
        return Err(Unsupported("refreshing the staff groups after an invite").into());
    }
    if process {
        let has_password: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM user_passwords WHERE user_id = $1)")
                .bind(user_id)
                .fetch_one(&mut *conn)
                .await?;
        if has_password {
            if !active {
                let email = primary_email(&mut *conn, user_id).await?;
                let token = crate::accounts::create_email_token(
                    &mut *conn,
                    user_id,
                    &email,
                    token_scopes::SIGNUP,
                )
                .await?;
                crate::jobs::enqueue(
                    &mut *conn,
                    "critical_user_email",
                    json!({"type": "signup", "user_id": user_id, "email_token": token, "to_address": null}),
                )
                .await?;
            }
        } else if !s.get("enable_discourse_connect")?.truthy()
            && s.get("enable_local_logins")?.truthy()
        {
            crate::jobs::enqueue(
                &mut *conn,
                crate::invite_mailer::INVITE_PASSWORD_INSTRUCTIONS_EMAIL,
                json!({"username": username}),
            )
            .await?;
        }
    }
    topic_invite_notifications(conn, state, s, invite, user_id).await
}

fn json_status(status: StatusCode, body: Value) -> Response {
    (status, Json(body)).into_response()
}

fn with_cookies(mut response: Response, cookies: &[String]) -> Result<Response, AppError> {
    for cookie in cookies {
        response
            .headers_mut()
            .append(header::SET_COOKIE, HeaderValue::from_str(cookie)?);
    }
    Ok(response)
}

/// PUT /invites/show/:id(.json): InvitesController#perform_accept_invitation.
pub async fn perform_accept_invitation(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    super::search::Peer(peer): super::search::Peer,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    let pairs: Vec<(String, String)> = p
        .iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect();
    if !super::session::csrf_ok(&state, &headers, &pairs, uri.path(), "PUT") {
        return Ok(super::session::bad_csrf());
    }
    let key = id.strip_suffix(".json").unwrap_or(&id).to_string();
    let mut tx = state.pool.begin().await?;
    let s = SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let not_found = || super::topics::not_found_response(&state);
    // ensure_invites_allowed, ensure_new_registrations_allowed
    if !invites_allowed(&s)? {
        return Ok(not_found());
    }
    if !s.get("allow_new_registrations")?.truthy() && guardian.is_anonymous() {
        return Err(Unsupported("accepting an invite with registrations closed (no_ember)").into());
    }
    if s.get("enable_discourse_connect")?.truthy() {
        return Ok(not_found());
    }
    if invites::email_code_enabled(&mut tx, &s, &guardian).await? && guardian.is_anonymous() {
        // server_session[:authentication] is external auth's (not ported).
        return Ok(not_found());
    }
    let not_found_json = || {
        json_status(
            StatusCode::NOT_FOUND,
            json!({"failed": "FAILED", "message": t(&state, "invite.not_found_json")}),
        )
    };
    let Some(invite) = Invite::find_by_key(&mut tx, &key).await? else {
        return Ok(not_found_json());
    };
    let string = |k: &str| p.get(k).and_then(params::scalar);
    let ip = crate::remote_ip::remote_ip(&headers, peer)?;
    let redeeming_user = guardian.user_id();
    let mut args = RedeemArgs {
        ip: Some(ip),
        redeeming_user,
        ..RedeemArgs::default()
    };
    if redeeming_user.is_none() {
        args.username = string("username");
        args.name = string("name");
        args.password = string("password");
        args.user_custom_fields = p.get("user_custom_fields").cloned();
        if invite.is_invite_link() {
            let Some(email) = string("email").filter(|e| !e.is_empty()) else {
                return Ok(super::accounts::param_missing("email"));
            };
            args.email = Some(email);
        } else {
            args.email = invite.email.clone();
            args.email_token = string("email_token").filter(|t| !crate::ruby::is_blank(t));
        }
    }
    let redeemed = redeem(&mut tx, &state, &s, &invite, &args).await?;
    let (user_id, _) = match redeemed {
        Redeemed::User { id, created } => (id, created),
        Redeemed::Nil => {
            tx.commit().await?;
            return Ok(not_found_json());
        }
        Redeemed::Failed(message) => {
            tx.commit().await?;
            return Ok(json_status(
                StatusCode::PRECONDITION_FAILED,
                json!({"failed": "FAILED", "message": message}),
            ));
        }
    };
    let (active, approved, admin): (bool, bool, bool) =
        sqlx::query_as("SELECT active, approved, admin OR moderator FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await?;
    // user.guardian.can_access_forum?
    let can_access = !s.get("must_approve_users")?.truthy() || approved || admin;
    let mut cookies = Vec::new();
    if redeeming_user.is_none() && active && can_access {
        cookies.push(log_on_user(&mut tx, &state, &s, &headers, peer, &uri, user_id).await?);
    }
    crate::accounts::update_timezone_if_missing(&mut tx, user_id, string("timezone").as_deref())
        .await?;
    after_redeem(&mut tx, &state, &s, &invite, user_id, true).await?;
    let topic_path = visible_topic_path(&mut tx, &state, &s, &invite, user_id).await?;
    let base_path = state.config.globals.relative_url_root();
    let mut body = json!({"success": "OK"});
    if active && can_access {
        if redeeming_user.is_some() {
            body["message"] = json!(t(&state, "invite.existing_user_success"));
        }
        body["redirect_to"] = json!(topic_path.unwrap_or_else(|| format!("{base_path}/")));
    } else {
        body["message"] = json!(if active {
            t(&state, "activation.approval_required")
        } else {
            t(&state, "invite.confirm_email")
        });
        if let Some(path) = topic_path {
            cookies.push(format!(
                "destination_url={}; path=/",
                crate::ruby::cgi_escape(&path)
            ));
        }
    }
    tx.commit().await?;
    with_cookies(Json(body).into_response(), &cookies)
}

/// `Email.obfuscate`
fn obfuscate(email: &str) -> String {
    if !crate::accounts::valid_email(email) {
        return email.to_string();
    }
    let (first, last) = email.rsplit_once('@').unwrap_or((email, ""));
    let part = |p: &str| {
        let n = p.chars().count();
        if n < 3 {
            "*".repeat(n)
        } else if n < 5 {
            format!("{}{}", &p[..1], "*".repeat(n - 1))
        } else {
            format!("{}{}{}", &p[..1], "*".repeat(n - 2), &p[p.len() - 1..])
        }
    };
    let mut labels: Vec<&str> = last.split('.').collect();
    let tld = labels.pop().unwrap_or("");
    let mut obfuscated: Vec<String> = labels.into_iter().map(part).collect();
    obfuscated.push(tld.to_string());
    format!("{}@{}", part(first), obfuscated.join("."))
}

/// What the invite page is given (`invite_info`), or the error page an
/// invite that can't be redeemed shows.
pub(crate) enum Shown {
    Info(Value),
    Redirect(String),
    /// The flash error no_ember shows.
    Error(String),
}

/// InvitesController#show up to the response: the checks, then
/// show_invite's info.
pub(crate) async fn show_info(
    conn: &mut PgConnection,
    state: &AppState,
    s: &SiteSettings,
    guardian: &Guardian,
    session_id: &str,
    key: &str,
    t_param: Option<&str>,
) -> Result<Option<Shown>, AppError> {
    if !invites_allowed(s)? {
        return Ok(None);
    }
    if !s.get("allow_new_registrations")?.truthy() && guardian.is_anonymous() {
        return Ok(Some(Shown::Error(t(
            state,
            "login.new_registrations_disabled",
        ))));
    }
    let urls = crate::url::Urls {
        config: &state.config,
        settings: s,
    };
    let base_url = urls.base_url()?;
    let site_name = s.get("title")?.to_s();
    let invite = Invite::find_by_key(&mut *conn, key).await?;
    let redeemable = match &invite {
        Some(i) => i.redeemable(&mut *conn).await?,
        None => false,
    };
    if !redeemable {
        let error = match &invite {
            None => state
                .i18n
                .t_with("invite.not_found", &[("base_url", &base_url)])
                .unwrap_or_default(),
            Some(i) if i.redeemed(&mut *conn).await? => {
                let key = if i.is_invite_link() {
                    "invite.not_found_template_link"
                } else {
                    "invite.not_found_template"
                };
                state
                    .i18n
                    .t_with(key, &[("site_name", &site_name), ("base_url", &base_url)])
                    .unwrap_or_default()
            }
            Some(i) if i.expired() => state
                .i18n
                .t_with("invite.expired", &[("base_url", &base_url)])
                .unwrap_or_default(),
            // Trashed or invalidated: flash[:error] is nil.
            Some(_) => String::new(),
        };
        return Ok(Some(Shown::Error(error)));
    }
    let invite = invite.ok_or(Unsupported("an invite gone"))?;
    // A signed-in user goes to the topic, unless the invite would add them
    // to a group.
    if let Some(user_id) = guardian.user_id() {
        let new_groups: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM invited_groups ig JOIN groups g ON g.id = ig.group_id \
             WHERE ig.invite_id = $1 AND NOT EXISTS (SELECT 1 FROM group_users gu \
               WHERE gu.group_id = ig.group_id AND gu.user_id = $2))",
        )
        .bind(invite.id)
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
        if !new_groups
            && let Some(topic_id) = invite.topic_ids(&mut *conn).await?.first().copied()
            && matches!(
                invites::topic_for(&mut *conn, s, guardian, topic_id).await?,
                Some((_, true))
            )
        {
            let slug: Option<String> = sqlx::query_scalar("SELECT slug FROM topics WHERE id = $1")
                .bind(topic_id)
                .fetch_one(&mut *conn)
                .await?;
            let slug = slug.ok_or(Unsupported("a topic without its slug"))?;
            return Ok(Some(Shown::Redirect(format!(
                "{}/t/{slug}/{topic_id}",
                state.config.globals.relative_url_root()
            ))));
        }
    }
    // show_invite
    let invite_email = invite.email.clone();
    let mut email = invite_email.as_deref().map(obfuscate);
    let code = invites::email_code_enabled(&mut *conn, s, guardian).await?;
    let verified_by_link =
        !code && invite.email_token.is_some() && t_param == invite.email_token.as_deref();
    if verified_by_link {
        email = invite_email.clone();
    }
    let hidden_email = email != invite_email;
    let username = match &invite_email {
        Some(e)
            if !hidden_email
                && s.get("use_email_for_username_and_name_suggestions")?
                    .truthy() =>
        {
            suggest_from_email(&mut *conn, state, s, e, true)
                .await?
                .unwrap_or_default()
        }
        _ => String::new(),
    };
    let inviter = invites::Inviter::load(&mut *conn, invite.invited_by_id)
        .await?
        .ok_or(Unsupported("an inviter gone"))?;
    let (title, avatar): (Option<String>, Option<i32>) =
        sqlx::query_as("SELECT title, uploaded_avatar_id FROM users WHERE id = $1")
            .bind(inviter.id)
            .fetch_one(&mut *conn)
            .await?;
    let logo_small = crate::admin_users::logo_small_url(&mut *conn, s).await?;
    let mut invited_by = Map::new();
    invited_by.insert("id".into(), json!(inviter.id));
    invited_by.insert("username".into(), json!(inviter.username));
    if s.get("enable_names")?.truthy() {
        invited_by.insert("name".into(), json!(inviter.name));
    }
    invited_by.insert(
        "avatar_template".into(),
        json!(crate::avatar::avatar_template(
            &urls,
            inviter.id,
            &inviter.username,
            avatar,
            logo_small.as_deref()
        )?),
    );
    invited_by.insert("title".into(), json!(title));
    let mut info = Map::new();
    info.insert("invited_by".into(), Value::Object(invited_by));
    info.insert("email".into(), json!(email));
    info.insert("hidden_email".into(), json!(hidden_email));
    info.insert("username".into(), json!(username));
    info.insert("is_invite_link".into(), json!(invite.is_invite_link()));
    info.insert("email_verified_by_link".into(), json!(verified_by_link));
    if verified_by_link {
        let staged: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM users u JOIN user_emails e ON e.user_id = u.id \
             WHERE u.staged AND lower(e.email) = lower($1))",
        )
        .bind(&invite_email)
        .fetch_one(&mut *conn)
        .await?;
        if staged {
            return Err(Unsupported("an invite to a staged user's email").into());
        }
    }
    if let Some(user) = guardian.user() {
        let user_email = primary_email(&mut *conn, user.id).await?;
        let can = invite
            .can_be_redeemed_by(&mut *conn, user.id, &user_email)
            .await?;
        info.insert("existing_user_id".into(), json!(user.id));
        info.insert("existing_user_can_redeem".into(), json!(can));
        let error = if can {
            Value::Null
        } else if invite.redeemed_by_user(&mut *conn, user.id).await? {
            json!(t(state, "invite.existing_user_already_redemeed"))
        } else {
            json!(t(state, "invite.existing_user_cannot_redeem"))
        };
        info.insert("existing_user_can_redeem_error".into(), error);
        info.insert("email".into(), json!(user_email));
        info.insert("username".into(), json!(user.username));
    }
    crate::session::server::set(
        &mut *conn,
        session_id,
        "invite-key",
        &json!(invite.invite_key),
        crate::session::server::EXPIRY_SECONDS,
    )
    .await?;
    Ok(Some(Shown::Info(Value::Object(info))))
}

#[derive(askama::Template)]
#[template(path = "invite_error.html")]
struct InviteErrorPage {
    site_title: String,
    lang: String,
    base_path: String,
    crawler: crate::html::Crawler,
    viewer: Option<crate::html::Viewer>,
    bus_position: String,
    chrome: crate::html::Chrome,
    error: String,
}

/// invites/show.html.erb in the no_ember layout.
async fn error_page(
    state: &AppState,
    s: &SiteSettings,
    guardian: &Guardian,
    uri: &Uri,
    error: String,
) -> Result<Response, AppError> {
    use askama::Template;
    let base_path = state.config.globals.relative_url_root().to_string();
    let mut site = crate::html::Site::from_settings(s, &base_path)?;
    let mut conn = state.pool.acquire().await?;
    site.load_chrome(&mut conn, state, s, guardian, crate::sidebar::Active::None)
        .await?;
    drop(conn);
    let urls = crate::url::Urls {
        config: &state.config,
        settings: s,
    };
    let crawler = crate::html::Crawler::for_request(&urls, uri, None)?;
    let page = InviteErrorPage {
        site_title: site.site_title,
        lang: site.lang,
        base_path: site.base_path,
        crawler,
        viewer: site.viewer,
        bus_position: String::new(),
        chrome: site.chrome,
        error,
    };
    let html = page.render().map_err(crate::html::HtmlError::from)?;
    Ok((
        [(header::CACHE_CONTROL, "no-cache, no-store")],
        axum::response::Html(html),
    )
        .into_response())
}

/// GET /invites/:id: InvitesController#show, the invite's info as JSON
/// or the page that accepts it.
pub async fn show(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    super::search::Peer(peer): super::search::Peer,
    uri: Uri,
) -> Result<Response, AppError> {
    // The route only takes the HTML format.
    if id.contains('.') {
        return Ok(super::topics::not_found_response(&state));
    }
    let json_format = super::wants_json(uri.query(), &headers);
    let mut conn = state.pool.acquire().await?;
    let s = SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let ip = crate::remote_ip::remote_ip(&headers, peer)?;
    let limiter = RateLimiter::new(None, &format!("invites-show-{ip}"), 100, 60);
    if let Err(refused) = limiter
        .performed(&state.pool, state.config.rate_limits)
        .await?
    {
        if json_format {
            return Err(Unsupported("the invite page's rate limit as JSON").into());
        }
        let error = refused.description(&state.i18n);
        return error_page(&state, &s, &guardian, &uri, error).await;
    }
    let mut session = crate::session::forum::load_forum_session(&state, &headers);
    let session_id = session.server_session_id();
    let query = params::parse_query(uri.query().unwrap_or(""));
    let t_param = params::string(&query, "t");
    let mut tx = state.pool.begin().await?;
    let shown = show_info(
        &mut tx,
        &state,
        &s,
        &guardian,
        &session_id,
        &id,
        t_param.as_deref(),
    )
    .await?;
    tx.commit().await?;
    let cookie = session.set_cookie(&state, &s)?;
    let response = match shown {
        None => super::topics::not_found_response(&state),
        Some(Shown::Redirect(path)) => super::user_activity::found(&headers, path),
        Some(Shown::Error(error)) => {
            if json_format {
                return Err(Unsupported("an unredeemable invite as JSON (a 500 in Rails)").into());
            }
            error_page(&state, &s, &guardian, &uri, error).await?
        }
        Some(Shown::Info(info)) => {
            if json_format {
                Json(info).into_response()
            } else {
                return Err(Unsupported("the invite accept page (192d)").into());
            }
        }
    };
    let cookies: Vec<String> = cookie.into_iter().collect();
    with_cookies(response, &cookies)
}

/// `login_code_honeypot_fails?` (never for API requests, not ported).
async fn honeypot_fails(
    conn: &mut PgConnection,
    session_id: &str,
    p: &Map<String, Value>,
) -> Result<bool, AppError> {
    super::accounts::honeypot_or_challenge_fails(conn, session_id, p).await
}

/// ensure_login_code_allowed's checks, for an anonymous caller: Err is
/// the refusal.
async fn login_code_allowed(
    conn: &mut PgConnection,
    state: &AppState,
    s: &SiteSettings,
    guardian: &Guardian,
) -> Result<Result<(), Response>, AppError> {
    if !guardian
        .upcoming_change_enabled(conn, s, "enable_local_logins_via_code")
        .await?
    {
        return Ok(Err(super::topics::not_found_response(state)));
    }
    if !s.get("enable_local_logins_via_email")?.truthy()
        || s.get("enable_discourse_connect")?.truthy()
        || !s.get("enable_local_logins")?.truthy()
    {
        return Ok(Err(super::search::invalid_access(state)));
    }
    if guardian.is_authenticated() {
        return Err(Unsupported("a signed-in user asking for a login code (redirect)").into());
    }
    Ok(Ok(()))
}

/// POST /session/login-code(.json) with an invite_key:
/// SessionController#create_login_code's request_invite_login_code.
pub async fn create_login_code(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    super::search::Peer(peer): super::search::Peer,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    let mut tx = state.pool.begin().await?;
    let s = SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    if let Err(refused) = login_code_allowed(&mut tx, &state, &s, &guardian).await? {
        return Ok(refused);
    }
    let invite_key = params::string(&p, "invite_key").filter(|k| !k.is_empty());
    let ip = crate::remote_ip::remote_ip(&headers, peer)?;
    // rate_limit_login_code_request
    let invite = match &invite_key {
        Some(k) => Invite::find_by_key(&mut tx, k).await?,
        None => None,
    };
    let email = invite
        .as_ref()
        .and_then(|i| i.email().map(str::to_string))
        .or_else(|| params::string(&p, "email"))
        .unwrap_or_default()
        .trim()
        .to_lowercase();
    let mut limiters = vec![
        RateLimiter::new(None, &format!("login-code-hour-{ip}"), 6, 3600),
        RateLimiter::new(None, &format!("login-code-min-{ip}"), 3, 60),
    ];
    if !email.is_empty() {
        let digest = {
            use sha2::Digest;
            format!("{:x}", sha2::Sha256::digest(email.as_bytes()))
        };
        limiters.push(RateLimiter::new(
            None,
            &format!("login-code-email-{digest}"),
            3,
            3600,
        ));
    }
    for limiter in limiters {
        if let Err(refused) = limiter
            .performed(&state.pool, state.config.rate_limits)
            .await?
        {
            return Ok(refused.into_response(&state.i18n));
        }
    }
    let mut session = crate::session::forum::load_forum_session(&state, &headers);
    let session_id = session.server_session_id();
    let success = || Json(json!({"success": "OK"})).into_response();
    let respond = |response: Response, session: &crate::session::forum::ForumSession| {
        let cookie = session.set_cookie(&state, &s)?;
        with_cookies(response, &cookie.into_iter().collect::<Vec<_>>())
    };
    if honeypot_fails(&mut tx, &session_id, &p).await? {
        tx.commit().await?;
        return respond(success(), &session);
    }
    if invite_key.is_none() {
        return Err(Unsupported("logging in with an emailed code").into());
    }
    // Invite::RequestEmailCode: an unknown or spent invite, or an email
    // that can't redeem it, answers success with no code.
    let invite = match invite {
        Some(i) if i.redeemable(&mut tx).await? => i,
        _ => {
            tx.commit().await?;
            return respond(success(), &session);
        }
    };
    let email = invite
        .email()
        .map(str::to_string)
        .unwrap_or_else(|| email.clone());
    let existing: Option<i32> = sqlx::query_scalar(
        "SELECT u.id FROM users u JOIN user_emails e ON e.user_id = u.id \
         WHERE u.id > 0 AND NOT u.staged AND lower(e.email) = $1 ORDER BY u.id LIMIT 1",
    )
    .bind(&email)
    .fetch_optional(&mut *tx)
    .await?;
    let can = crate::accounts::valid_email(&email)
        && (invite.email().is_none() || invite.email_matches(&email))
        && (invite.domain().is_none() || invite.domain_matches(&email));
    let can = can
        && (existing.is_some() || {
            let any: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM user_emails WHERE lower(email) = $1)",
            )
            .bind(&email)
            .fetch_one(&mut *tx)
            .await?;
            !any && crate::signup::email_allowed(&s, &email)?
                && !crate::email_updater::should_block(&mut tx, &s, &email).await?
        });
    if !can {
        tx.commit().await?;
        return respond(success(), &session);
    }
    // can_register_from_ip
    if existing.is_none() && crate::signup::prevent_registration_from_ip(&mut tx, &s, &ip).await? {
        tx.commit().await?;
        let error = t(
            &state,
            "activerecord.errors.models.user.attributes.ip_address.max_new_accounts_per_registration_ip",
        );
        return respond(Json(json!({"error": error})).into_response(), &session);
    }
    let (code_id, code) = crate::accounts::generate_login_code(
        &mut tx,
        &email,
        crate::accounts::code_purposes::LOGIN,
        &state.keys.secret_key_base,
    )
    .await?;
    let _ = code_id;
    crate::jobs::enqueue(
        &mut tx,
        "send_email_login_code",
        json!({"to_address": email, "code": code}),
    )
    .await?;
    tx.commit().await?;
    session.changed = true;
    respond(success(), &session)
}

fn invalid_login_code(state: &AppState) -> Response {
    Json(json!({"error": t(state, "email_login_code.invalid_code")})).into_response()
}

/// POST /session/login-code/verify(.json) with an invite_key:
/// SessionController#verify_login_code's process_verified_invite_login_code.
pub async fn verify_login_code(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    super::search::Peer(peer): super::search::Peer,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    let mut tx = state.pool.begin().await?;
    let s = SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    if let Err(refused) = login_code_allowed(&mut tx, &state, &s, &guardian).await? {
        return Ok(refused);
    }
    let ip = crate::remote_ip::remote_ip(&headers, peer)?;
    for limiter in [
        RateLimiter::new(None, &format!("login-code-verify-hour-{ip}"), 30, 3600),
        RateLimiter::new(None, &format!("login-code-verify-min-{ip}"), 6, 60),
    ] {
        if let Err(refused) = limiter
            .performed(&state.pool, state.config.rate_limits)
            .await?
        {
            return Ok(refused.into_response(&state.i18n));
        }
    }
    if p.contains_key("signup_token") {
        return Err(Unsupported("completing a login code signup (signup_token)").into());
    }
    let string = |k: &str| params::string(&p, k);
    let Some(invite_key) = string("invite_key").filter(|k| !k.is_empty()) else {
        return Err(Unsupported("logging in with an emailed code").into());
    };
    let found = Invite::find_by_key(&mut tx, &invite_key).await?;
    // login_code_verification_params: the invite's email over the param.
    let email = found
        .as_ref()
        .and_then(|i| i.email().map(str::to_string))
        .or_else(|| string("email"))
        .unwrap_or_default()
        .trim()
        .to_lowercase();
    let code = string("code").unwrap_or_default().trim().to_string();
    // EmailLoginCode::Verify's contract
    static CODE: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"^\d{6}$").unwrap());
    if email.is_empty() || !crate::accounts::valid_email(&email) || !CODE.is_match(&code) {
        return Err(Unsupported("a login code verification failing its contract").into());
    }
    // fetch_login_code: an active login code for the email.
    let login_code: Option<(i64, String)> = sqlx::query_as(
        "SELECT id, code_hash FROM email_login_codes WHERE lower(email) = $1 AND purpose = $2 \
         AND consumed_at IS NULL AND expires_at > clock_timestamp() AND attempts < 5 ORDER BY id LIMIT 1",
    )
    .bind(&email)
    .bind(crate::accounts::code_purposes::LOGIN)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((code_id, code_hash)) = login_code else {
        tx.commit().await?;
        return Ok(invalid_login_code(&state));
    };
    // code_matches: an attempt burned, given back on a match.
    let burned: Option<i32> = sqlx::query_scalar(
        "UPDATE email_login_codes SET attempts = attempts + 1 WHERE id = $1 AND attempts < 5 RETURNING attempts",
    )
    .bind(code_id)
    .fetch_optional(&mut *tx)
    .await?;
    let matches = burned.is_some()
        && crate::accounts::hash_login_code(&code, &state.keys.secret_key_base) == code_hash;
    if !matches {
        tx.commit().await?;
        return Ok(invalid_login_code(&state));
    }
    sqlx::query("UPDATE email_login_codes SET attempts = 0 WHERE id = $1")
        .bind(code_id)
        .execute(&mut *tx)
        .await?;
    // model :user (optional): a real, unstaged user with the email.
    let matched: Option<(i32, bool, bool)> = sqlx::query_as(
        "SELECT u.id, u.approved OR u.admin, COALESCE(u.suspended_till > now(), FALSE) \
         FROM users u JOIN user_emails e ON e.user_id = u.id \
         WHERE u.id > 0 AND NOT u.staged AND lower(e.email) = $1 ORDER BY u.id LIMIT 1",
    )
    .bind(&email)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some((id, _, _)) = matched {
        let factors: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM user_second_factors WHERE user_id = $1 AND enabled) \
             OR EXISTS (SELECT 1 FROM user_security_keys WHERE user_id = $1 AND enabled)",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
        if factors {
            return Err(Unsupported("login codes for users with second factors").into());
        }
    }
    // process_verified_invite_login_code
    let must_approve = s.get("must_approve_users")?.truthy();
    if let Some((_, approved, suspended)) = matched {
        if must_approve && !approved {
            tx.commit().await?;
            return Ok(Json(json!({"error": t(&state, "login.not_approved")})).into_response());
        }
        if suspended {
            return Err(Unsupported("a suspended user verifying an invite login code").into());
        }
    }
    let name = string("name")
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty());
    let username = string("username")
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty());
    if matched.is_none() {
        let required_fields: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM user_fields WHERE requirement <> 0 AND show_on_signup)",
        )
        .fetch_one(&mut *tx)
        .await?;
        let fields_missing = !p.get("user_fields").is_some_and(|v| match v {
            Value::Object(m) => !m.is_empty(),
            Value::String(s) => !s.is_empty(),
            Value::Null => false,
            _ => true,
        }) && required_fields;
        let name_required = s.get("full_name_requirement")?.to_s() == "required_at_signup"
            && s.get("enable_names")?.truthy();
        let name_missing = name.is_none() && name_required;
        if fields_missing || name_missing {
            tx.commit().await?;
            return Ok(Json(
                json!({"user_fields_required": fields_missing, "name_required": name_missing}),
            )
            .into_response());
        }
    }
    // Invite::RedeemWithEmailCode
    let invite = match found {
        Some(i) if i.redeemable(&mut tx).await? => i,
        _ => {
            tx.commit().await?;
            return Ok(invalid_login_code(&state));
        }
    };
    if (invite.email().is_some() && !invite.email_matches(&email))
        || (invite.domain().is_some() && !invite.domain_matches(&email))
    {
        tx.commit().await?;
        return Ok(invalid_login_code(&state));
    }
    if matched.is_none() {
        let any: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM user_emails WHERE lower(email) = $1)")
                .bind(&email)
                .fetch_one(&mut *tx)
                .await?;
        if any {
            tx.commit().await?;
            return Ok(invalid_login_code(&state));
        }
        if !s.get("allow_new_registrations")?.truthy()
            || !crate::signup::email_allowed(&s, &email)?
            || crate::email_updater::should_block(&mut tx, &s, &email).await?
        {
            tx.commit().await?;
            return Ok(
                Json(json!({"error": t(&state, "login.new_registrations_disabled")}))
                    .into_response(),
            );
        }
        let Some(username) = &username else {
            // render_signup_username_required for an invite
            let suggested = suggest_from_email(&mut tx, &state, &s, &email, false).await?;
            if suggested.is_some() {
                return Err(Unsupported("suggesting an invitee's username from the email").into());
            }
            let trust_level = s.get("default_invitee_trust_level")?.to_i();
            let mut group_ids: Vec<i64> = vec![10];
            for (level, id) in [(1, 11), (2, 12), (3, 13), (4, 14)] {
                if trust_level >= level {
                    group_ids.push(id);
                }
            }
            group_ids.push(crate::guardian::auto_groups::LOGGED_IN_USERS);
            if !s
                .get("granular_anonymous_and_logged_in_groups_permissions")?
                .truthy()
            {
                group_ids.push(0);
            }
            let invite_groups = invite.group_ids(&mut tx).await?;
            group_ids.extend(invite_groups.into_iter().map(i64::from));
            let allowed: Vec<i64> = s
                .get("uploaded_avatars_allowed_groups")?
                .to_s()
                .split('|')
                .filter_map(|g| g.parse().ok())
                .collect();
            tx.commit().await?;
            return Ok(Json(json!({
                "username_required": true,
                "username": null,
                "generated_username": false,
                "trust_level": trust_level,
                "avatar_template": null,
                "can_upload_avatar": group_ids.iter().any(|g| allowed.contains(g)),
            }))
            .into_response());
        };
        if crate::signup::reserved_username(&s, username)? {
            tx.commit().await?;
            return Ok(
                Json(json!({"username_errors": [t(&state, "login.reserved_username")]}))
                    .into_response(),
            );
        }
    }
    // consume_code
    let consumed = sqlx::query(
        "UPDATE email_login_codes SET consumed_at = clock_timestamp() WHERE id = $1 AND consumed_at IS NULL",
    )
    .bind(code_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if consumed != 1 {
        tx.commit().await?;
        return Ok(invalid_login_code(&state));
    }
    let args = match matched {
        Some((id, _, _)) => RedeemArgs {
            redeeming_user: Some(id),
            email: Some(email.clone()),
            email_verified: true,
            ..RedeemArgs::default()
        },
        None => RedeemArgs {
            email: Some(email.clone()),
            name: name.clone(),
            username: username.clone(),
            user_custom_fields: p.get("user_fields").cloned(),
            ip: Some(ip.clone()),
            email_verified: true,
            ..RedeemArgs::default()
        },
    };
    let user_id = match redeem(&mut tx, &state, &s, &invite, &args).await? {
        Redeemed::User { id, .. } => id,
        Redeemed::Nil => {
            tx.commit().await?;
            return Ok(invalid_login_code(&state));
        }
        Redeemed::Failed(message) => {
            return Err(Unsupported(match message.as_str() {
                _ if message.starts_with("Validation failed") => {
                    "a login code redemption failing User's validations"
                }
                _ => "a login code redemption refused by InviteRedeemer",
            })
            .into());
        }
    };
    let active: bool = sqlx::query_scalar("SELECT active FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await?;
    if !active {
        let user_email = primary_email(&mut tx, user_id).await?;
        activate(&mut tx, &state, &s, user_id, &user_email).await?;
    }
    after_redeem(&mut tx, &state, &s, &invite, user_id, false).await?;
    let redirect_url = visible_topic_path(&mut tx, &state, &s, &invite, user_id)
        .await?
        .unwrap_or_else(|| format!("{}/", state.config.globals.relative_url_root()));
    // login_with_login_code
    if s.get("enable_discourse_connect_provider")?.truthy() {
        return Err(Unsupported("login codes with DiscourseConnect provider handoffs").into());
    }
    let approved: bool = sqlx::query_scalar("SELECT approved OR admin FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await?;
    if must_approve && !approved {
        tx.commit().await?;
        return Ok(Json(json!({"pending_approval": true})).into_response());
    }
    let cookie = log_on_user(&mut tx, &state, &s, &headers, peer, &uri, user_id).await?;
    let mut session = crate::session::forum::load_forum_session(&state, &headers);
    if session.map.remove("activate_user").is_some() {
        session.changed = true;
    }
    let body = if matched.is_none() {
        // account_created: UserSerializer, seen by the new user.
        let user = SessionUser::load(&mut tx, user_id)
            .await?
            .ok_or(Unsupported("a new user gone"))?;
        let viewer = Guardian::for_user(&mut tx, &user).await?;
        let urls = crate::url::Urls {
            config: &state.config,
            settings: &s,
        };
        let profile = crate::users::User::find_active(&mut tx, &user.username)
            .await?
            .ok_or(Unsupported(
                "logging in a user the profile loader cannot see",
            ))?;
        let doc = crate::users::Users {
            conn: &mut tx,
            settings: &s,
            i18n: &state.i18n,
            guardian: &viewer,
            urls: &urls,
            base_path: state.config.globals.relative_url_root(),
            auth_token: None,
        }
        .show(&profile)
        .await?;
        let allowed: Vec<i64> = s
            .get("uploaded_avatars_allowed_groups")?
            .to_s()
            .split('|')
            .filter_map(|g| g.parse().ok())
            .collect();
        let can_upload = viewer.group_ids().iter().any(|g| allowed.contains(g));
        json!({
            "success": "OK",
            "account_created": true,
            "user": doc["user"],
            "can_upload_avatar": can_upload,
            "redirect_url": redirect_url,
        })
    } else {
        json!({"success": "OK", "redirect_url": redirect_url})
    };
    // update_timezone_if_missing: an update_all, after the serializer
    // read the user (Rails' user_option is already loaded, so the account
    // created answer shows no timezone).
    crate::accounts::update_timezone_if_missing(&mut tx, user_id, string("timezone").as_deref())
        .await?;
    tx.commit().await?;
    let mut cookies = vec![cookie];
    if let Some(c) = session.set_cookie(&state, &s)? {
        cookies.push(c);
    }
    with_cookies(Json(body).into_response(), &cookies)
}
