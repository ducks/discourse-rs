//! Invites (app/models/invite.rb): `Invite.generate` with the model's
//! validations, trashing, the inviter's pending, expired and redeemed
//! lists, and InviteSerializer.

use chrono::NaiveDateTime;
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::guardian::Guardian;
use crate::i18n::I18n;
use crate::rate_limiter::{LimitExceeded, LimitUser, RateLimiter};
use crate::site_settings::SiteSettings;
use crate::{AppError, Unsupported};

/// `Invite.emailed_status_types`
pub mod emailed_status {
    pub const NOT_REQUIRED: i32 = 0;
    pub const PENDING: i32 = 1;
    pub const BULK_PENDING: i32 = 2;
    pub const SENDING: i32 = 3;
    pub const SENT: i32 = 4;
}

/// `Invite::DESCRIPTION_MAX_LENGTH`
const DESCRIPTION_MAX_LENGTH: usize = 100;

pub const INVITE_COLUMNS: &str = "invites.id, invites.invite_key, invites.email, invites.invited_by_id, \
    invites.created_at, invites.updated_at, invites.deleted_at, invites.invalidated_at, invites.moderator, \
    invites.custom_message, invites.emailed_status, invites.max_redemptions_allowed, \
    invites.redemption_count, invites.expires_at, invites.email_token, invites.domain, \
    invites.description, invites.admin";

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Invite {
    pub id: i32,
    pub invite_key: String,
    pub email: Option<String>,
    pub invited_by_id: i32,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub deleted_at: Option<NaiveDateTime>,
    pub invalidated_at: Option<NaiveDateTime>,
    pub moderator: bool,
    pub custom_message: Option<String>,
    pub emailed_status: Option<i32>,
    pub max_redemptions_allowed: i32,
    pub redemption_count: i32,
    pub expires_at: NaiveDateTime,
    pub email_token: Option<String>,
    pub domain: Option<String>,
    pub description: Option<String>,
    pub admin: bool,
}

fn present(s: &Option<String>) -> Option<&str> {
    s.as_deref().filter(|s| !crate::ruby::is_blank(s))
}

impl Invite {
    /// `Invite.find_by(id:)`: the default scope leaves out trashed ones.
    pub async fn find(conn: &mut PgConnection, id: i32) -> Result<Option<Invite>, sqlx::Error> {
        sqlx::query_as(&format!(
            "SELECT {INVITE_COLUMNS} FROM invites WHERE id = $1 AND deleted_at IS NULL"
        ))
        .bind(id)
        .fetch_optional(conn)
        .await
    }

    /// `Invite.find_by(invite_key:)`
    pub async fn find_by_key(
        conn: &mut PgConnection,
        key: &str,
    ) -> Result<Option<Invite>, sqlx::Error> {
        sqlx::query_as(&format!(
            "SELECT {INVITE_COLUMNS} FROM invites WHERE invite_key = $1 AND deleted_at IS NULL LIMIT 1"
        ))
        .bind(key)
        .fetch_optional(conn)
        .await
    }

    /// `Invite.find_by(invited_by:, email:)`: the first in heap order,
    /// taken here as the oldest.
    pub async fn find_by_inviter_and_email(
        conn: &mut PgConnection,
        inviter_id: i32,
        email: &str,
    ) -> Result<Option<Invite>, sqlx::Error> {
        sqlx::query_as(&format!(
            "SELECT {INVITE_COLUMNS} FROM invites WHERE invited_by_id = $1 AND email = $2 \
             AND deleted_at IS NULL ORDER BY id LIMIT 1"
        ))
        .bind(inviter_id)
        .bind(email)
        .fetch_optional(conn)
        .await
    }

    pub fn email(&self) -> Option<&str> {
        present(&self.email)
    }

    pub fn domain(&self) -> Option<&str> {
        present(&self.domain)
    }

    /// `is_invite_link?`: no email, though maybe a domain.
    pub fn is_invite_link(&self) -> bool {
        self.email().is_none()
    }

    pub fn is_email_invite(&self) -> bool {
        !self.is_invite_link()
    }

    pub fn expired(&self) -> bool {
        self.expires_at < crate::clock::now_naive()
    }

    pub fn link_valid(&self) -> bool {
        self.invalidated_at.is_none()
    }

    /// `redeemed?`
    pub async fn redeemed(&self, conn: &mut PgConnection) -> Result<bool, sqlx::Error> {
        if self.is_invite_link() {
            return Ok(self.redemption_count >= self.max_redemptions_allowed);
        }
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM invited_users WHERE invite_id = $1)")
            .bind(self.id)
            .fetch_one(conn)
            .await
    }

    /// `redeemable?`
    pub async fn redeemable(&self, conn: &mut PgConnection) -> Result<bool, sqlx::Error> {
        Ok(!self.redeemed(conn).await?
            && !self.expired()
            && self.deleted_at.is_none()
            && self.link_valid())
    }

    /// `redeemed_by_user?(user)`
    pub async fn redeemed_by_user(
        &self,
        conn: &mut PgConnection,
        user_id: i32,
    ) -> Result<bool, sqlx::Error> {
        sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM invited_users WHERE invite_id = $1 AND user_id = $2)",
        )
        .bind(self.id)
        .bind(user_id)
        .fetch_one(conn)
        .await
    }

    /// `email_matches?(email)`
    pub fn email_matches(&self, email: &str) -> bool {
        self.email
            .as_deref()
            .is_some_and(|e| e.to_lowercase() == email.to_lowercase())
    }

    /// `domain_matches?(email)`: the part after the first `@`.
    pub fn domain_matches(&self, email: &str) -> bool {
        let domain = email.split('@').nth(1);
        self.domain.as_deref() == domain
    }

    /// `can_be_redeemed_by?(user)`, given the user's primary email.
    pub async fn can_be_redeemed_by(
        &self,
        conn: &mut PgConnection,
        user_id: i32,
        user_email: &str,
    ) -> Result<bool, sqlx::Error> {
        if !self.redeemable(&mut *conn).await? {
            return Ok(false);
        }
        if self.redeemed_by_user(&mut *conn, user_id).await? {
            return Ok(false);
        }
        if self.domain().is_none() && self.email().is_none() {
            return Ok(true);
        }
        if self.email().is_some() && self.email_matches(user_email) {
            return Ok(true);
        }
        Ok(self.domain().is_some() && self.domain_matches(user_email))
    }

    /// `link(with_email_token:)`
    pub fn link(&self, base_url: &str, with_email_token: bool) -> String {
        match (&self.email_token, with_email_token) {
            (Some(token), true) => format!("{base_url}/invites/{}?t={token}", self.invite_key),
            (None, true) => format!("{base_url}/invites/{}?t=", self.invite_key),
            _ => format!("{base_url}/invites/{}", self.invite_key),
        }
    }

    /// The ids of `invite.topics` (topic_invites, through to live
    /// topics), in the join's order.
    pub async fn topic_ids(&self, conn: &mut PgConnection) -> Result<Vec<i32>, sqlx::Error> {
        sqlx::query_scalar(
            "SELECT t.id FROM topic_invites ti JOIN topics t ON t.id = ti.topic_id AND t.deleted_at IS NULL \
             WHERE ti.invite_id = $1 ORDER BY ti.id",
        )
        .bind(self.id)
        .fetch_all(conn)
        .await
    }

    /// The ids of `invite.groups`, in the join's order.
    pub async fn group_ids(&self, conn: &mut PgConnection) -> Result<Vec<i32>, sqlx::Error> {
        sqlx::query_scalar(
            "SELECT g.id FROM invited_groups ig JOIN groups g ON g.id = ig.group_id \
             WHERE ig.invite_id = $1 ORDER BY ig.id",
        )
        .bind(self.id)
        .fetch_all(conn)
        .await
    }

    /// `trash!(user)`: update_columns, so updated_at stays.
    pub async fn trash(&self, conn: &mut PgConnection, by: Option<i32>) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE invites SET deleted_at = clock_timestamp(), deleted_by_id = $2 WHERE id = $1",
        )
        .bind(self.id)
        .bind(by)
        .execute(conn)
        .await?;
        Ok(())
    }
}

/// `Invite.find_user_by_email`: a user other than a staged one with the
/// address among their emails.
pub async fn find_user_by_email(
    conn: &mut PgConnection,
    email: &str,
) -> Result<Option<i32>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT u.id FROM users u JOIN user_emails e ON e.user_id = u.id \
         WHERE lower(e.email) = lower($1) AND NOT u.staged ORDER BY u.id LIMIT 1",
    )
    .bind(email.trim())
    .fetch_optional(conn)
    .await
}

/// `Email.downcase`: valid addresses lowercased, anything else as given.
pub fn email_downcase(email: &str) -> String {
    if crate::accounts::valid_email(email) {
        email.to_lowercase()
    } else {
        email.to_string()
    }
}

/// `user_exists_error_msg(email)`
pub fn user_exists_error_msg(
    s: &SiteSettings,
    i18n: &I18n,
    email: &str,
) -> Result<String, AppError> {
    let key = if s.get("hide_email_address_taken")?.truthy() {
        "invite.generic_error_response"
    } else {
        "invite.user_exists"
    };
    let escaped = crate::email::notification::html_escape(email);
    Ok(i18n
        .t_with(key, &[("email", &escaped)])
        .unwrap_or_else(|| key.to_string()))
}

/// `Invite.email_code_enabled?(user)`
pub async fn email_code_enabled(
    conn: &mut PgConnection,
    s: &SiteSettings,
    guardian: &Guardian,
) -> Result<bool, AppError> {
    Ok(guardian
        .upcoming_change_enabled(conn, s, "enable_local_logins_via_code")
        .await?
        && s.get("enable_local_logins")?.truthy()
        && s.get("enable_local_logins_via_email")?.truthy()
        && !s.get("enable_discourse_connect")?.truthy())
}

/// What generating and validating read of the inviter.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Inviter {
    pub id: i32,
    pub username: String,
    pub name: Option<String>,
    pub admin: bool,
    pub moderator: bool,
    pub timezone: Option<String>,
}

impl Inviter {
    pub async fn load(conn: &mut PgConnection, id: i32) -> Result<Option<Inviter>, sqlx::Error> {
        sqlx::query_as(
            "SELECT u.id, u.username, u.name, u.admin, u.moderator, o.timezone \
             FROM users u LEFT JOIN user_options o ON o.user_id = u.id WHERE u.id = $1",
        )
        .bind(id)
        .fetch_optional(conn)
        .await
    }

    pub fn staff(&self) -> bool {
        self.admin || self.moderator
    }

    fn limit_user(&self) -> Option<LimitUser> {
        Some(LimitUser {
            id: self.id,
            staff: self.staff(),
        })
    }
}

/// `time_zone.now + SiteSetting.invite_expiry_days.days`: days added on
/// the inviter's wall clock (Time.find_zone, UTC when unknown).
pub fn default_expiry(s: &SiteSettings, timezone: Option<&str>) -> Result<NaiveDateTime, AppError> {
    let days = s.get("invite_expiry_days")?.to_i();
    let now = crate::clock::now_naive();
    let tz: Option<chrono_tz::Tz> = timezone.and_then(|t| t.parse().ok());
    Ok(match tz {
        Some(tz) => {
            use chrono::TimeZone;
            let local = tz.from_utc_datetime(&now).naive_local() + chrono::Duration::days(days);
            tz.from_local_datetime(&local)
                .earliest()
                .map(|t| t.naive_utc())
                .unwrap_or(now + chrono::Duration::days(days))
        }
        None => now + chrono::Duration::days(days),
    })
}

/// A datetime param as ActiveRecord casts it (Time.zone is UTC): None
/// when blank; an unparsable one is refused.
pub fn parse_time_param(value: &str) -> Result<Option<NaiveDateTime>, AppError> {
    let v = value.trim();
    if v.is_empty() {
        return Ok(None);
    }
    if let Ok(t) = chrono::DateTime::parse_from_rfc3339(v) {
        return Ok(Some(t.naive_utc()));
    }
    for format in [
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M",
    ] {
        if let Ok(t) = NaiveDateTime::parse_from_str(v, format) {
            return Ok(Some(t));
        }
    }
    if let Ok(t) = chrono::DateTime::parse_from_str(v, "%Y-%m-%d %H:%M:%S %z") {
        return Ok(Some(t.naive_utc()));
    }
    if let Ok(d) = chrono::NaiveDate::parse_from_str(v, "%Y-%m-%d") {
        return Ok(d.and_hms_opt(0, 0, 0));
    }
    Err(Unsupported("an expires_at Time.zone.parse reads differently").into())
}

/// An integer param as ActiveRecord casts it: None when blank, else
/// Ruby's `to_i`.
pub fn integer_param(value: &Value) -> Option<i64> {
    match value {
        Value::Null => None,
        Value::Bool(true) => Some(1),
        Value::Bool(false) => Some(0),
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Value::String(s) if crate::ruby::is_blank(s) => None,
        Value::String(s) => Some(crate::ruby::to_i(s)),
        _ => None,
    }
}

/// The record as the validations see it, with what the save changes.
#[derive(Debug, Clone, Default)]
pub struct Draft {
    pub id: Option<i32>,
    pub email: Option<String>,
    pub domain: Option<String>,
    pub custom_message: Option<String>,
    pub description: Option<String>,
    pub max_redemptions_allowed: Option<i64>,
    pub redemption_count: i64,
    pub admin: bool,
    pub moderator: bool,
    pub email_changed: bool,
    pub domain_changed: bool,
    pub admin_changed: bool,
    pub moderator_changed: bool,
}

impl Draft {
    fn new_record(&self) -> bool {
        self.id.is_none()
    }
}

fn humanize(attr: &str) -> String {
    let mut s = attr.replace('_', " ");
    if let Some(first) = s.get(0..1) {
        s.replace_range(0..1, &first.to_uppercase());
    }
    s
}

/// The model's validations (and its before_validation and the
/// validations' own normalizing: the email downcased, the domain too, a
/// missing max_redemptions_allowed set to 1), as full messages in the
/// order Rails adds them.
pub async fn validate(
    conn: &mut PgConnection,
    s: &SiteSettings,
    i18n: &I18n,
    inviter: &Inviter,
    d: &mut Draft,
) -> Result<Vec<String>, AppError> {
    let t = |key: &str| i18n.t(key).unwrap_or(key).to_string();
    let t_with = |key: &str, args: &[(&str, &str)]| {
        i18n.t_with(key, args).unwrap_or_else(|| key.to_string())
    };
    let attr = |name: &str, message: &str| format!("{} {message}", humanize(name));
    let mut errors = Vec::new();
    // before_validation: Email.downcase
    if let Some(email) = &d.email {
        d.email = Some(email_downcase(email));
    }
    // validates :email, email: true, allow_blank: true
    if let Some(email) = d.email.clone().filter(|e| !crate::ruby::is_blank(e)) {
        let mut invalid = false;
        if !crate::accounts::valid_email(&email) {
            let escaped = crate::email::notification::html_escape(&email);
            errors.push(t_with("invite.invalid_email", &[("email", &escaped)]));
            invalid = true;
        }
        if !crate::signup::email_allowed(s, &email)? {
            errors.push(attr("email", &t("user.email.not_allowed")));
            invalid = true;
        }
        if !invalid && crate::email_updater::should_block(&mut *conn, s, &email).await? {
            errors.push(attr("email", &t("user.email.blocked")));
        }
    }
    let too_long = |count: usize| {
        let n = count.to_string();
        t_with("errors.messages.too_long.other", &[("count", &n)])
    };
    if d.custom_message
        .as_deref()
        .is_some_and(|m| m.chars().count() > 1000)
    {
        errors.push(attr("custom_message", &too_long(1000)));
    }
    if d.domain.as_deref().is_some_and(|m| m.chars().count() > 500) {
        errors.push(attr("domain", &too_long(500)));
    }
    if d.description
        .as_deref()
        .is_some_and(|m| m.chars().count() > DESCRIPTION_MAX_LENGTH)
    {
        errors.push(attr("description", &too_long(DESCRIPTION_MAX_LENGTH)));
    }
    // ensure_max_redemptions_allowed
    match d.max_redemptions_allowed {
        None => d.max_redemptions_allowed = Some(1),
        Some(max) => {
            let limit = if inviter.staff() {
                s.get("invite_link_max_redemptions_limit")?.to_i()
            } else {
                s.get("invite_link_max_redemptions_limit_users")?.to_i()
            };
            let has_email = d
                .email
                .as_deref()
                .is_some_and(|e| !crate::ruby::is_blank(e));
            if (d.admin || d.moderator) && max != 1 {
                errors.push(attr(
                    "max_redemptions_allowed",
                    &t("invite.max_redemptions_allowed_one_staff"),
                ));
            } else if has_email && max != 1 {
                errors.push(attr(
                    "max_redemptions_allowed",
                    &t("invite.max_redemptions_allowed_one"),
                ));
            } else if !(1..=limit).contains(&max) {
                let limit = limit.to_string();
                errors.push(attr(
                    "max_redemptions_allowed",
                    &t_with(
                        "invite_link.max_redemptions_limit",
                        &[("max_limit", &limit)],
                    ),
                ));
            }
        }
    }
    let max = d.max_redemptions_allowed.unwrap_or(1);
    // valid_redemption_count
    if d.redemption_count > max {
        let max = max.to_string();
        errors.push(attr(
            "redemption_count",
            &t_with(
                "invite.redemption_count_less_than_max",
                &[("max_redemptions_allowed", &max)],
            ),
        ));
    }
    // valid_domain, if will_save_change_to_domain?
    if d.domain_changed
        && let Some(domain) = d.domain.clone().filter(|v| !crate::ruby::is_blank(v))
    {
        let domain = domain.to_lowercase();
        static DOMAIN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
            regex::Regex::new(
                r"\A(([a-zA-Z0-9]|[a-zA-Z0-9][a-zA-Z0-9\-]*[a-zA-Z0-9])\.)+([A-Za-z0-9]|[A-Za-z0-9][A-Za-z0-9\-]*[A-Za-z0-9])\z",
            )
            .unwrap()
        });
        if !DOMAIN.is_match(&domain) {
            errors.push(t("invite.domain_not_allowed_admin"));
        }
        d.domain = Some(domain);
    }
    // user_doesnt_already_exist, if will_save_change_to_email?
    if d.email_changed
        && let Some(email) = d.email.clone().filter(|e| !crate::ruby::is_blank(e))
        && let Some(user_id) = find_user_by_email(&mut *conn, &email).await?
    {
        let first_redeemer: Option<i32> = match d.id {
            Some(id) => sqlx::query_scalar(
                "SELECT user_id FROM invited_users WHERE invite_id = $1 ORDER BY id LIMIT 1",
            )
            .bind(id)
            .fetch_optional(&mut *conn)
            .await?
            .flatten(),
            None => None,
        };
        if first_redeemer != Some(user_id) {
            errors.push(user_exists_error_msg(s, i18n, &email)?);
        }
    }
    // email_xor_domain
    if d.email
        .as_deref()
        .is_some_and(|e| !crate::ruby::is_blank(e))
        && d.domain
            .as_deref()
            .is_some_and(|e| !crate::ruby::is_blank(e))
    {
        errors.push(t("invite.email_xor_domain"));
    }
    let reach_changed = d.email_changed || d.domain_changed;
    if d.admin && (d.new_record() || d.admin_changed || reach_changed) && !inviter.admin {
        errors.push(t("invite.admin_invite_requires_admin_inviter"));
    }
    if d.moderator && (d.new_record() || d.moderator_changed || reach_changed) && !inviter.staff() {
        errors.push(t("invite.moderator_invite_requires_staff_inviter"));
    }
    Ok(errors)
}

/// `SecureRandom.base58(10)`
pub fn random_key() -> String {
    use rand::Rng;
    const ALPHABET: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    let mut rng = rand::thread_rng();
    (0..10)
        .map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())] as char)
        .collect()
}

/// What `Invite.generate` is asked for (its `opts`).
#[derive(Debug, Clone, Default)]
pub struct Generate {
    pub email: Option<String>,
    pub description: Option<String>,
    pub domain: Option<String>,
    pub skip_email: bool,
    pub custom_message: Option<String>,
    pub max_redemptions_allowed: Option<i64>,
    pub admin: bool,
    pub moderator: bool,
    pub topic_id: Option<i32>,
    pub group_ids: Vec<i32>,
    pub expires_at: Option<NaiveDateTime>,
    /// `opts[:expires_at]` given but blank: truthy in Ruby, so it is what
    /// the create is given (cast to nil, then before_create's default).
    pub expires_at_blank: bool,
    /// `invite_to_topic`, passed through to the job as given.
    pub invite_to_topic: Value,
    pub emailed_status: Option<i32>,
}

pub enum Generated {
    Invite(Box<Invite>),
    /// `Invite::UserExists`: its message.
    UserExists(String),
    /// `ActiveRecord::RecordInvalid`: the first full message.
    Invalid(String),
    RateLimited(LimitExceeded),
}

/// The rate limiter and the enabled flag `generate` performs with.
pub struct Limits<'a> {
    pub pool: &'a sqlx::PgPool,
    pub enabled: bool,
}

/// `Invite.generate(invited_by, opts)`
pub async fn generate(
    conn: &mut PgConnection,
    s: &SiteSettings,
    i18n: &I18n,
    limits: &Limits<'_>,
    inviter: &Inviter,
    opts: &Generate,
) -> Result<Generated, AppError> {
    let email = opts
        .email
        .as_deref()
        .filter(|e| !crate::ruby::is_blank(e))
        .map(email_downcase);
    if let Some(email) = &email
        && find_user_by_email(&mut *conn, email).await?.is_some()
    {
        return Ok(Generated::UserExists(user_exists_error_msg(
            s, i18n, email,
        )?));
    }
    let mut existing: Option<Invite> = None;
    if let Some(email) = &email {
        // with_deleted, newest first
        let found: Option<Invite> = sqlx::query_as(&format!(
            "SELECT {INVITE_COLUMNS} FROM invites WHERE email = $1 AND invited_by_id = $2 \
             ORDER BY created_at DESC LIMIT 1"
        ))
        .bind(email)
        .bind(inviter.id)
        .fetch_optional(&mut *conn)
        .await?;
        if let Some(invite) = found {
            if invite.expired() || invite.deleted_at.is_some() {
                // destroy: the joins stay (no dependent).
                sqlx::query("DELETE FROM invites WHERE id = $1")
                    .bind(invite.id)
                    .execute(&mut *conn)
                    .await?;
                // after_destroy: the invites-per-day limiter gives one
                // back.
                invites_per_day(s, inviter)?
                    .rollback(limits.pool, limits.enabled)
                    .await?;
            } else {
                existing = Some(invite);
            }
        }
        let digest = {
            use sha2::Digest;
            format!("{:x}", sha2::Sha256::digest(email.as_bytes()))
        };
        let limiter = RateLimiter::new(
            inviter.limit_user(),
            &format!("reinvites-per-day-{digest}"),
            3,
            86_400,
        );
        if let Err(refused) = limiter.performed(limits.pool, limits.enabled).await? {
            return Ok(Generated::RateLimited(refused));
        }
    }
    let status = if opts.skip_email
        || existing
            .as_ref()
            .is_some_and(|i| i.emailed_status == Some(emailed_status::NOT_REQUIRED))
    {
        emailed_status::NOT_REQUIRED
    } else if let Some(status) = opts.emailed_status {
        status
    } else if email.is_some() {
        emailed_status::PENDING
    } else {
        emailed_status::NOT_REQUIRED
    };
    let expires_at = match opts.expires_at {
        Some(t) => t,
        None => default_expiry(s, inviter.timezone.as_deref())?,
    };
    let invite_id = match existing {
        Some(invite) => {
            sqlx::query(
                "UPDATE invites SET created_at = $2, updated_at = $2, expires_at = $3, emailed_status = $4, \
                 admin = $5, moderator = $6 WHERE id = $1",
            )
            .bind(invite.id)
            .bind(crate::clock::now_naive())
            .bind(expires_at)
            .bind(status)
            .bind(opts.admin && inviter.admin)
            .bind(opts.moderator && inviter.staff())
            .execute(&mut *conn)
            .await?;
            invite.id
        }
        None => {
            let mut draft = Draft {
                email: email.clone(),
                domain: opts.domain.clone(),
                custom_message: opts.custom_message.clone(),
                description: opts.description.clone(),
                max_redemptions_allowed: opts.max_redemptions_allowed,
                admin: opts.admin,
                moderator: opts.moderator,
                email_changed: email.is_some(),
                domain_changed: opts.domain.is_some(),
                admin_changed: opts.admin,
                moderator_changed: opts.moderator,
                ..Draft::default()
            };
            let errors = validate(&mut *conn, s, i18n, inviter, &mut draft).await?;
            if let Some(first) = errors.into_iter().next() {
                return Ok(Generated::Invalid(first));
            }
            // A blank expires_at casts to nil, which before_create fills
            // with the default from now (in UTC).
            let expires_at = if opts.expires_at.is_none() && opts.expires_at_blank {
                crate::clock::now_naive()
                    + chrono::Duration::days(s.get("invite_expiry_days")?.to_i())
            } else {
                expires_at
            };
            let email_token = draft
                .email
                .as_deref()
                .filter(|e| !crate::ruby::is_blank(e))
                .map(|_| crate::accounts::random_hex());
            let id: i32 = sqlx::query_scalar(
                "INSERT INTO invites (invite_key, email, invited_by_id, created_at, updated_at, moderator, \
                 custom_message, emailed_status, max_redemptions_allowed, redemption_count, expires_at, \
                 email_token, domain, description, admin) \
                 VALUES ($1, $2, $3, $4, $4, $5, $6, $7, $8, 0, $9, $10, $11, $12, $13) RETURNING id",
            )
            .bind(random_key())
            .bind(&draft.email)
            .bind(inviter.id)
            .bind(crate::clock::now_naive())
            .bind(draft.moderator)
            .bind(&draft.custom_message)
            .bind(status)
            .bind(draft.max_redemptions_allowed.unwrap_or(1) as i32)
            .bind(expires_at)
            .bind(email_token)
            .bind(&draft.domain)
            .bind(&draft.description)
            .bind(draft.admin)
            .fetch_one(&mut *conn)
            .await?;
            // after_create: rate_limit :limit_invites_per_day
            if let Err(refused) = invites_per_day(s, inviter)?
                .performed(limits.pool, limits.enabled)
                .await?
            {
                return Ok(Generated::RateLimited(refused));
            }
            id
        }
    };
    let invite = Invite::find_with_deleted(&mut *conn, invite_id)
        .await?
        .ok_or(Unsupported("an invite gone while it was generated"))?;
    if invite.admin || invite.moderator {
        // staff invites never carry topic or group associations
        sqlx::query("DELETE FROM topic_invites WHERE invite_id = $1")
            .bind(invite.id)
            .execute(&mut *conn)
            .await?;
        sqlx::query("DELETE FROM invited_groups WHERE invite_id = $1")
            .bind(invite.id)
            .execute(&mut *conn)
            .await?;
    } else {
        if let Some(topic_id) = opts.topic_id {
            add_topic(&mut *conn, invite.id, topic_id).await?;
        }
        for group_id in &opts.group_ids {
            add_group(&mut *conn, invite.id, *group_id).await?;
        }
    }
    if status == emailed_status::PENDING {
        sqlx::query("UPDATE invites SET emailed_status = $2 WHERE id = $1")
            .bind(invite.id)
            .bind(emailed_status::SENDING)
            .execute(&mut *conn)
            .await?;
        crate::jobs::enqueue(
            &mut *conn,
            "invite_email",
            json!({"invite_id": invite.id, "invite_to_topic": opts.invite_to_topic}),
        )
        .await?;
    }
    let invite = Invite::find_with_deleted(&mut *conn, invite_id)
        .await?
        .ok_or(Unsupported("an invite gone while it was generated"))?;
    Ok(Generated::Invite(Box::new(invite)))
}

impl Invite {
    /// `Invite.with_deleted.find(id)`
    pub async fn find_with_deleted(
        conn: &mut PgConnection,
        id: i32,
    ) -> Result<Option<Invite>, sqlx::Error> {
        sqlx::query_as(&format!(
            "SELECT {INVITE_COLUMNS} FROM invites WHERE id = $1"
        ))
        .bind(id)
        .fetch_optional(conn)
        .await
    }
}

/// `limit_invites_per_day`
fn invites_per_day(s: &SiteSettings, inviter: &Inviter) -> Result<RateLimiter, AppError> {
    Ok(RateLimiter::new(
        inviter.limit_user(),
        "invites-per-day",
        s.get("max_invites_per_day")?.to_i(),
        86_400,
    ))
}

/// `invite.topic_invites.find_or_create_by!(topic_id:)`
pub async fn add_topic(
    conn: &mut PgConnection,
    invite_id: i32,
    topic_id: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO topic_invites (topic_id, invite_id, created_at, updated_at) \
         SELECT $1, $2, clock_timestamp(), clock_timestamp() \
         WHERE NOT EXISTS (SELECT 1 FROM topic_invites WHERE topic_id = $1 AND invite_id = $2)",
    )
    .bind(topic_id)
    .bind(invite_id)
    .execute(conn)
    .await?;
    Ok(())
}

/// `invite.invited_groups.find_or_create_by!(group_id:)`
pub async fn add_group(
    conn: &mut PgConnection,
    invite_id: i32,
    group_id: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO invited_groups (group_id, invite_id, created_at, updated_at) \
         SELECT $1, $2, clock_timestamp(), clock_timestamp() \
         WHERE NOT EXISTS (SELECT 1 FROM invited_groups WHERE group_id = $1 AND invite_id = $2)",
    )
    .bind(group_id)
    .bind(invite_id)
    .execute(conn)
    .await?;
    Ok(())
}

/// `Group.lookup_groups(group_ids:, group_names:)`: the groups with those
/// ids, narrowed to those names; None when neither was given.
pub async fn lookup_groups(
    conn: &mut PgConnection,
    group_ids: Option<&Value>,
    group_names: Option<&Value>,
) -> Result<Option<Vec<crate::groups::BasicGroup>>, sqlx::Error> {
    let ids: Option<Vec<i32>> = group_ids.and_then(|v| match v {
        Value::Array(items) => {
            let ids: Vec<i32> = items
                .iter()
                .filter_map(crate::params::scalar)
                .map(|s| crate::ruby::to_i(&s) as i32)
                .collect();
            (!items.is_empty()).then_some(ids)
        }
        other => crate::params::scalar(other)
            .filter(|s| !crate::ruby::is_blank(s))
            .map(|s| {
                s.split(',')
                    .map(|p| crate::ruby::to_i(p) as i32)
                    .collect::<Vec<_>>()
            }),
    });
    let names: Option<Vec<String>> = group_names.and_then(|v| {
        crate::params::scalar(v)
            .filter(|s| !crate::ruby::is_blank(s))
            .map(|s| s.split(',').map(str::to_string).collect::<Vec<_>>())
    });
    if ids.is_none() && names.is_none() {
        return Ok(None);
    }
    let mut clauses = Vec::new();
    if ids.is_some() {
        clauses.push("g.id = ANY($1)");
    }
    if names.is_some() {
        clauses.push("g.name = ANY($2)");
    }
    let groups = sqlx::query_as(&format!(
        "SELECT {} FROM groups g LEFT JOIN uploads u ON u.id = g.flair_upload_id WHERE {} ORDER BY g.id",
        crate::groups::BASIC_GROUP_COLUMNS,
        clauses.join(" AND ")
    ))
    .bind(ids.unwrap_or_default())
    .bind(names.unwrap_or_default())
    .fetch_all(conn)
    .await?;
    Ok(Some(groups))
}

/// `guardian.can_edit_group?(group)` for each, as
/// `can_invite_to_forum?(groups)` asks.
pub async fn can_edit_groups(
    conn: &mut PgConnection,
    s: &SiteSettings,
    guardian: &Guardian,
    groups: &[crate::groups::BasicGroup],
) -> Result<bool, AppError> {
    for group in groups {
        let membership = crate::groups::membership(&mut *conn, guardian, group.id).await?;
        if !crate::groups::can_edit_group(guardian, s, group, membership)? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// `can_invite_to_forum?(groups)`
pub async fn can_invite_to_forum(
    conn: &mut PgConnection,
    s: &SiteSettings,
    guardian: &Guardian,
    groups: Option<&[crate::groups::BasicGroup]>,
) -> Result<bool, AppError> {
    if !guardian.can_invite_to_forum(s)? {
        return Ok(false);
    }
    match groups {
        Some(groups) if !groups.is_empty() => can_edit_groups(conn, s, guardian, groups).await,
        _ => Ok(true),
    }
}

/// A topic as the invite checks read it: its context and whether the
/// guardian can see it (`can_see?`).
pub async fn topic_for(
    conn: &mut PgConnection,
    s: &SiteSettings,
    guardian: &Guardian,
    topic_id: i32,
) -> Result<Option<(crate::topic_guardian::TopicCtx, bool)>, AppError> {
    let Some(topic) =
        crate::topic_guardian::TopicCtx::load(&mut *conn, s, guardian, topic_id).await?
    else {
        return Ok(None);
    };
    if topic.deleted_at.is_some() {
        // Topic.find_by: the default scope leaves out trashed topics.
        return Ok(None);
    }
    let secure = guardian.secure_category_ids(&mut *conn, s).await?;
    let can_see = guardian.can_see_topic(s, &topic, true, &secure)?;
    Ok(Some((topic, can_see)))
}

/// `groups_can_see_topic?(groups, topic)`: a topic in a restricted
/// category needs one of its groups among the invite's.
pub async fn groups_can_see_topic(
    conn: &mut PgConnection,
    group_ids: &[i32],
    topic_id: Option<i32>,
) -> Result<bool, sqlx::Error> {
    let Some(topic_id) = topic_id else {
        return Ok(true);
    };
    let restricted: Option<bool> = sqlx::query_scalar(
        "SELECT c.read_restricted FROM topics t JOIN categories c ON c.id = t.category_id WHERE t.id = $1",
    )
    .bind(topic_id)
    .fetch_optional(&mut *conn)
    .await?;
    if restricted != Some(true) {
        return Ok(true);
    }
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM category_groups cg JOIN topics t ON t.category_id = cg.category_id \
                        WHERE t.id = $1 AND cg.group_id = ANY($2))",
    )
    .bind(topic_id)
    .bind(group_ids)
    .fetch_one(conn)
    .await
}

/// `I18n.t("invite.requires_groups", groups:)`: the topic category's
/// groups the guardian may edit, by name.
pub async fn requires_groups_message(
    conn: &mut PgConnection,
    s: &SiteSettings,
    i18n: &I18n,
    guardian: &Guardian,
    topic_id: i32,
) -> Result<String, AppError> {
    let groups: Vec<crate::groups::BasicGroup> = sqlx::query_as(&format!(
        "SELECT {} FROM groups g LEFT JOIN uploads u ON u.id = g.flair_upload_id \
         JOIN category_groups cg ON cg.group_id = g.id JOIN topics t ON t.category_id = cg.category_id \
         WHERE t.id = $1 ORDER BY cg.id",
        crate::groups::BASIC_GROUP_COLUMNS
    ))
    .bind(topic_id)
    .fetch_all(&mut *conn)
    .await?;
    let mut names = Vec::new();
    for group in &groups {
        let membership = crate::groups::membership(&mut *conn, guardian, group.id).await?;
        if crate::groups::can_edit_group(guardian, s, group, membership)? {
            names.push(group.name.clone());
        }
    }
    Ok(i18n
        .t_with("invite.requires_groups", &[("groups", &names.join(", "))])
        .unwrap_or_default())
}

/// InviteSerializer's options.
#[derive(Debug, Clone, Copy, Default)]
pub struct SerializeOpts {
    pub show_emails: bool,
}

/// InviteSerializer
pub async fn serialize(
    conn: &mut PgConnection,
    s: &SiteSettings,
    i18n: &I18n,
    guardian: &Guardian,
    base_url: &str,
    invite: &Invite,
    opts: SerializeOpts,
) -> Result<Value, AppError> {
    // can_see_invite_details? / can_see_invite_emails?: staff or the
    // inviter.
    let details = guardian.is_staff() || guardian.is_me(invite.invited_by_id);
    let has_email = invite.email().is_some();
    let mut out = Map::new();
    out.insert("id".into(), json!(invite.id));
    if details {
        out.insert("invite_key".into(), json!(invite.invite_key));
        out.insert("link".into(), json!(invite.link(base_url, false)));
        out.insert("description".into(), json!(invite.description));
    }
    if opts.show_emails && details && !invite.redeemed(&mut *conn).await? {
        out.insert("email".into(), json!(invite.email));
    }
    if details {
        out.insert("domain".into(), json!(invite.domain));
    }
    if has_email && details {
        out.insert(
            "emailed".into(),
            json!(invite.emailed_status != Some(emailed_status::NOT_REQUIRED)),
        );
    }
    // can_destroy_invite?
    out.insert(
        "can_delete_invite".into(),
        json!(guardian.is_admin() || guardian.is_me(invite.invited_by_id)),
    );
    if !has_email && details {
        out.insert(
            "max_redemptions_allowed".into(),
            json!(invite.max_redemptions_allowed),
        );
        out.insert("redemption_count".into(), json!(invite.redemption_count));
    }
    if has_email && details {
        out.insert("custom_message".into(), json!(invite.custom_message));
    }
    out.insert(
        "created_at".into(),
        json!(crate::themes::json_time(invite.created_at)),
    );
    out.insert(
        "updated_at".into(),
        json!(crate::themes::json_time(invite.updated_at)),
    );
    out.insert(
        "expires_at".into(),
        json!(crate::themes::json_time(invite.expires_at)),
    );
    out.insert("expired".into(), json!(invite.expired()));
    if details {
        out.insert("grants_admin".into(), json!(invite.admin));
        out.insert("grants_moderator".into(), json!(invite.moderator));
        let mut topics = Vec::new();
        for topic_id in invite.topic_ids(&mut *conn).await? {
            if let Some((_, true)) = topic_for(&mut *conn, s, guardian, topic_id).await? {
                topics.push(basic_topic(&mut *conn, s, topic_id).await?);
            }
        }
        out.insert("topics".into(), Value::Array(topics));
        let mut groups = Vec::new();
        for group_id in invite.group_ids(&mut *conn).await? {
            let group: crate::groups::BasicGroup = sqlx::query_as(&format!(
                "SELECT {} FROM groups g LEFT JOIN uploads u ON u.id = g.flair_upload_id WHERE g.id = $1",
                crate::groups::BASIC_GROUP_COLUMNS
            ))
            .bind(group_id)
            .fetch_one(&mut *conn)
            .await?;
            let membership: Option<bool> =
                crate::groups::membership(&mut *conn, guardian, group_id).await?;
            let mut json = group.json(i18n, guardian, s, membership.map(|o| (true, o)))?;
            // Serialized without owner_group_ids: bio_raw is staff's only.
            if !guardian.is_staff()
                && let Some(map) = json.as_object_mut()
            {
                map.remove("bio_raw");
            }
            groups.push(json);
        }
        out.insert("groups".into(), Value::Array(groups));
    }
    Ok(Value::Object(out))
}

/// BasicTopicSerializer: id, title, fancy_title, slug, posts_count.
pub async fn basic_topic(
    conn: &mut PgConnection,
    s: &SiteSettings,
    topic_id: i32,
) -> Result<Value, AppError> {
    let (title, fancy, slug, posts_count): (String, Option<String>, Option<String>, i32) =
        sqlx::query_as("SELECT title, fancy_title, slug, posts_count FROM topics WHERE id = $1")
            .bind(topic_id)
            .fetch_one(&mut *conn)
            .await?;
    let fancy =
        crate::topic_query::fancy_title(&mut *conn, s, topic_id, &title, fancy.as_deref()).await?;
    let slug = slug.ok_or(Unsupported("a topic without its slug"))?;
    Ok(json!({
        "id": topic_id,
        "title": title,
        "fancy_title": fancy,
        "slug": slug,
        "posts_count": posts_count,
    }))
}

/// The inviter's lists (`Invite.pending`, `.expired`, `.redeemed_users`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum List {
    Pending,
    Expired,
    Redeemed,
}

/// The FROM and WHERE of `Invite.pending(inviter)` or `.expired`: invites
/// with users joined through their redemptions.
fn invite_list_sql(list: List) -> &'static str {
    match list {
        List::Pending => {
            "FROM invites LEFT JOIN invited_users ON invites.id = invited_users.invite_id \
             LEFT JOIN users ON invited_users.user_id = users.id \
             WHERE invites.deleted_at IS NULL AND invites.invited_by_id = $1 \
             AND redemption_count < max_redemptions_allowed AND expires_at > $2"
        }
        List::Expired | List::Redeemed => {
            "FROM invites LEFT JOIN invited_users ON invites.id = invited_users.invite_id \
             LEFT JOIN users ON invited_users.user_id = users.id \
             WHERE invites.deleted_at IS NULL AND invites.invited_by_id = $1 \
             AND redemption_count < max_redemptions_allowed AND expires_at < $2"
        }
    }
}

/// `Invite.redeemed_users(inviter)`'s FROM and WHERE.
const REDEEMED_SQL: &str = "FROM invited_users LEFT JOIN invites ON invites.id = invited_users.invite_id \
    LEFT OUTER JOIN users ON users.id = invited_users.user_id \
    LEFT OUTER JOIN user_stats ON user_stats.user_id = users.id \
    WHERE invited_users.user_id IS NOT NULL AND invites.invited_by_id = $1";

/// `.reorder(nil).count` of a list.
pub async fn count(
    conn: &mut PgConnection,
    inviter_id: i32,
    list: List,
) -> Result<i64, sqlx::Error> {
    let now = crate::clock::now_naive();
    match list {
        List::Redeemed => {
            sqlx::query_scalar(&format!("SELECT COUNT(*) {REDEEMED_SQL}"))
                .bind(inviter_id)
                .fetch_one(conn)
                .await
        }
        _ => {
            sqlx::query_scalar(&format!(
                "SELECT COUNT(DISTINCT invites.id) {}",
                invite_list_sql(list)
            ))
            .bind(inviter_id)
            .bind(now)
            .fetch_one(conn)
            .await
        }
    }
}

/// A page of the pending or expired invites, optionally searched.
pub async fn invites_page(
    conn: &mut PgConnection,
    inviter_id: i32,
    list: List,
    offset: i64,
    limit: i64,
    search: Option<(&str, bool)>,
) -> Result<Vec<Invite>, sqlx::Error> {
    let now = crate::clock::now_naive();
    let filter = match search {
        Some((_, true)) => {
            " AND ((LOWER(invites.email) LIKE $5) or (LOWER(users.username) LIKE $5))"
        }
        Some((_, false)) => " AND ((LOWER(users.username) LIKE $5))",
        None => "",
    };
    let order = match list {
        List::Pending => "invites.updated_at DESC",
        _ => "invites.expires_at ASC",
    };
    let pattern = search
        .map(|(s, _)| format!("%{}%", s.to_lowercase()))
        .unwrap_or_default();
    sqlx::query_as(&format!(
        "SELECT DISTINCT {INVITE_COLUMNS} {}{filter} ORDER BY {order} LIMIT $3 OFFSET $4",
        invite_list_sql(list)
    ))
    .bind(inviter_id)
    .bind(now)
    .bind(limit)
    .bind(offset)
    .bind(pattern)
    .fetch_all(conn)
    .await
}

/// A redeemed invitation: the invite, its redeemer and when.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Redeemed {
    pub invite_id: i32,
    pub invite_email: Option<String>,
    pub invited_by_id: i32,
    pub redeemed_at: Option<NaiveDateTime>,
    pub user_id: Option<i32>,
}

/// A page of `Invite.redeemed_users(inviter)`, optionally searched.
pub async fn redeemed_page(
    conn: &mut PgConnection,
    inviter_id: i32,
    offset: i64,
    limit: i64,
    search: Option<(&str, bool)>,
) -> Result<Vec<Redeemed>, sqlx::Error> {
    let filter = match search {
        Some((_, true)) => {
            " AND ((LOWER(invites.email) LIKE $4) or (LOWER(users.username) LIKE $4))"
        }
        Some((_, false)) => " AND ((LOWER(users.username) LIKE $4))",
        None => "",
    };
    let pattern = search
        .map(|(s, _)| format!("%{}%", s.to_lowercase()))
        .unwrap_or_default();
    sqlx::query_as(&format!(
        "SELECT invited_users.invite_id, invites.email AS invite_email, invites.invited_by_id, \
                invited_users.redeemed_at, invited_users.user_id \
         {REDEEMED_SQL}{filter} ORDER BY invited_users.redeemed_at DESC LIMIT $2 OFFSET $3"
    ))
    .bind(inviter_id)
    .bind(limit)
    .bind(offset)
    .bind(pattern)
    .fetch_all(conn)
    .await
}
