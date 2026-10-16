//! InvitesController's writes: creating invites (one, or many for an
//! admin), editing, trashing and resending them, and looking one up by
//! email.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use super::session::{bad_csrf, csrf_ok};
use crate::guardian::Guardian;
use crate::invites::{self, Generate, Generated, Invite, Inviter, Limits, SerializeOpts};
use crate::params;
use crate::rate_limiter::RateLimiter;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

/// `render_json_error(message)`: 422 unless given.
fn json_error(status: StatusCode, message: String) -> Response {
    (status, Json(json!({"errors": [message]}))).into_response()
}

fn success() -> Response {
    Json(json!({"success": "OK"})).into_response()
}

/// Ruby truthiness of a param.
fn truthy(v: Option<&Value>) -> bool {
    !matches!(v, None | Some(Value::Null) | Some(Value::Bool(false)))
}

/// `params[k].present?`
fn present(p: &Map<String, Value>, key: &str) -> bool {
    match p.get(key) {
        None | Some(Value::Null) => false,
        Some(Value::String(s)) => !crate::ruby::is_blank(s),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(_)) => true,
    }
}

/// A string param, as Rails hands it on (nil when absent).
fn string(p: &Map<String, Value>, key: &str) -> Option<String> {
    p.get(key).and_then(params::scalar)
}

struct Req {
    p: Map<String, Value>,
    settings: SiteSettings,
}

async fn begin(
    state: &AppState,
    headers: &HeaderMap,
    uri: &Uri,
    body: &Bytes,
    method: &str,
) -> Result<Result<Req, Response>, AppError> {
    let p = params::parse(uri.query(), headers, body);
    if !csrf_ok(state, headers, &form_pairs(&p), uri.path(), method) {
        return Ok(Err(bad_csrf()));
    }
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    Ok(Ok(Req { p, settings }))
}

fn base_url(state: &AppState, s: &SiteSettings) -> Result<String, AppError> {
    Ok(crate::url::Urls {
        config: &state.config,
        settings: s,
    }
    .base_url()?)
}

async fn render(
    conn: &mut PgConnection,
    state: &AppState,
    s: &SiteSettings,
    guardian: &Guardian,
    invite: &Invite,
    show_emails: bool,
) -> Result<Value, AppError> {
    let base = base_url(state, s)?;
    invites::serialize(
        conn,
        s,
        &state.i18n,
        guardian,
        &base,
        invite,
        SerializeOpts { show_emails },
    )
    .await
}

/// `skip_email_param`
fn skip_email(s: &SiteSettings, p: &Map<String, Value>) -> Result<bool, AppError> {
    Ok(!s.get("allow_email_invites")?.truthy() || truthy(p.get("skip_email")))
}

/// The topic and groups a create names, checked: `Err` is the response
/// that refuses them.
async fn topic_and_groups(
    conn: &mut PgConnection,
    state: &AppState,
    s: &SiteSettings,
    guardian: &Guardian,
    p: &Map<String, Value>,
) -> Result<Result<(Option<i32>, Option<Vec<i32>>), Response>, AppError> {
    let mut topic_id = None;
    if present(p, "topic_id") {
        let id = string(p, "topic_id")
            .map(|v| crate::ruby::to_i(&v) as i32)
            .unwrap_or(0);
        let Some((topic, can_see)) = invites::topic_for(&mut *conn, s, guardian, id).await? else {
            return Ok(Err(super::search::invalid_parameters(state, "topic_id")));
        };
        // ensure_can_invite_to!
        if !guardian.can_invite_to(s, &topic, can_see)? {
            return Ok(Err(super::search::invalid_access(state)));
        }
        topic_id = Some(topic.id);
    }
    let groups = if present(p, "group_ids") || present(p, "group_names") {
        invites::lookup_groups(&mut *conn, p.get("group_ids"), p.get("group_names")).await?
    } else {
        None
    };
    // ensure_can_invite_to_forum!(groups)
    if !invites::can_invite_to_forum(&mut *conn, s, guardian, groups.as_deref()).await? {
        return Ok(Err(super::search::invalid_access(state)));
    }
    let group_ids: Option<Vec<i32>> = groups.map(|g| g.iter().map(|g| g.id).collect());
    if !invites::groups_can_see_topic(&mut *conn, group_ids.as_deref().unwrap_or(&[]), topic_id)
        .await?
    {
        let message = invites::requires_groups_message(
            &mut *conn,
            s,
            &state.i18n,
            guardian,
            topic_id.unwrap_or(0),
        )
        .await?;
        return Ok(Err(json_error(StatusCode::UNPROCESSABLE_ENTITY, message)));
    }
    Ok(Ok((topic_id, group_ids)))
}

fn generate_opts(
    s: &SiteSettings,
    p: &Map<String, Value>,
    email: Option<String>,
) -> Result<Generate, AppError> {
    let expires = string(p, "expires_at");
    Ok(Generate {
        email,
        description: string(p, "description"),
        domain: string(p, "domain"),
        skip_email: skip_email(s, p)?,
        custom_message: string(p, "custom_message"),
        max_redemptions_allowed: p
            .get("max_redemptions_allowed")
            .and_then(invites::integer_param),
        expires_at: match &expires {
            Some(v) => invites::parse_time_param(v)?,
            None => None,
        },
        expires_at_blank: expires.as_deref().is_some_and(crate::ruby::is_blank)
            && expires.is_some(),
        invite_to_topic: p.get("invite_to_topic").cloned().unwrap_or(Value::Null),
        ..Generate::default()
    })
}

async fn inviter(conn: &mut PgConnection, guardian: &Guardian) -> Result<Inviter, AppError> {
    let id = guardian
        .user_id()
        .ok_or(Unsupported("inviting anonymously"))?;
    Ok(Inviter::load(conn, id)
        .await?
        .ok_or(Unsupported("an inviter who is gone"))?)
}

/// POST /invites(.json)
pub async fn create(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let Req { p, settings: s } = match begin(&state, &headers, &uri, &body, "POST").await? {
        Ok(r) => r,
        Err(r) => return Ok(r),
    };
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    let mut tx = state.pool.begin().await?;
    let admin_invite = string(&p, "is_admin").as_deref() == Some("true");
    let moderator_invite = string(&p, "is_moderator").as_deref() == Some("true");
    let staff_invite = admin_invite || moderator_invite;
    let limits = Limits {
        pool: &state.pool,
        enabled: state.config.rate_limits,
    };
    let inviter = inviter(&mut tx, &guardian).await?;
    if staff_invite {
        // ensure_can_create_admin_invite!
        let can = guardian.is_admin()
            && guardian
                .upcoming_change_enabled(&mut tx, &s, "enable_invite_modal_with_roles")
                .await?;
        if !can {
            return Ok(super::search::invalid_access(&state));
        }
        if present(&p, "topic_id") || present(&p, "group_ids") || present(&p, "group_names") {
            let param = if admin_invite {
                "is_admin"
            } else {
                "is_moderator"
            };
            return Ok(super::search::invalid_parameters(&state, param));
        }
        let limiter = RateLimiter::new(
            Some(crate::rate_limiter::LimitUser {
                id: inviter.id,
                staff: inviter.staff(),
            }),
            "admin-invites-per-day",
            10,
            86_400,
        )
        .apply_limit_to_staff();
        if let Err(refused) = limiter.performed(limits.pool, limits.enabled).await? {
            return Ok(refused.into_response(&state.i18n));
        }
    }
    let (topic_id, group_ids) = match topic_and_groups(&mut tx, &state, &s, &guardian, &p).await? {
        Ok(v) => v,
        Err(refused) => return Ok(refused),
    };
    let mut opts = generate_opts(&s, &p, string(&p, "email"))?;
    if staff_invite {
        opts.max_redemptions_allowed = None;
    }
    opts.admin = admin_invite;
    opts.moderator = moderator_invite;
    opts.topic_id = topic_id;
    opts.group_ids = group_ids.unwrap_or_default();
    let generated = invites::generate(&mut tx, &s, &state.i18n, &limits, &inviter, &opts).await?;
    let response = match generated {
        Generated::Invite(invite) => {
            let doc = render(
                &mut tx,
                &state,
                &s,
                &guardian,
                &invite,
                p.contains_key("email"),
            )
            .await?;
            Json(doc).into_response()
        }
        Generated::UserExists(message) | Generated::Invalid(message) => {
            json_error(StatusCode::UNPROCESSABLE_ENTITY, message)
        }
        Generated::RateLimited(refused) => {
            // Raised out of the create: its transaction rolls back.
            return Ok(refused.into_response(&state.i18n));
        }
    };
    tx.commit().await?;
    Ok(response)
}

/// POST /invites/create-multiple: InvitesController#create_multiple, an
/// admin inviting a list of emails.
pub async fn create_multiple(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let Req { p, settings: s } = match begin(&state, &headers, &uri, &body, "POST").await? {
        Ok(r) => r,
        Err(r) => return Ok(r),
    };
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    // ensure_can_bulk_invite_to_forum!
    if !guardian.is_admin() {
        return Ok(super::search::invalid_access(&state));
    }
    let mut tx = state.pool.begin().await?;
    let (topic_id, group_ids) = match topic_and_groups(&mut tx, &state, &s, &guardian, &p).await? {
        Ok(v) => v,
        Err(refused) => return Ok(refused),
    };
    let emails: Vec<String> = match p.get("email") {
        Some(Value::Array(items)) => items.iter().filter_map(params::scalar).collect(),
        Some(Value::String(one)) => vec![one.clone()],
        _ => return Err(Unsupported("create-multiple without emails (a 500 in Rails)").into()),
    };
    let max = s.get("max_api_invites")?.to_i();
    if emails.len() as i64 > max {
        let max = max.to_string();
        return Ok(json_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            state
                .i18n
                .t_with("invite.max_invite_emails_limit_exceeded", &[("max", &max)])
                .unwrap_or_default(),
        ));
    }
    let limits = Limits {
        pool: &state.pool,
        enabled: state.config.rate_limits,
    };
    let inviter = inviter(&mut tx, &guardian).await?;
    let mut success = Vec::new();
    let mut failed = Vec::new();
    for email in emails {
        let mut opts = generate_opts(&s, &p, Some(email.clone()))?;
        opts.topic_id = topic_id;
        opts.group_ids = group_ids.clone().unwrap_or_default();
        match invites::generate(&mut tx, &s, &state.i18n, &limits, &inviter, &opts).await? {
            Generated::Invite(invite) => success.push(invite),
            Generated::UserExists(error) | Generated::Invalid(error) => {
                failed.push(json!({"email": email, "error": error}))
            }
            Generated::RateLimited(refused) => {
                tx.commit().await?;
                return Ok(refused.into_response(&state.i18n));
            }
        }
    }
    let mut serialized = Vec::new();
    for invite in &success {
        // InviteSerializer.new(invite): its root kept, no show_emails.
        serialized
            .push(json!({"invite": render(&mut tx, &state, &s, &guardian, invite, false).await?}));
    }
    tx.commit().await?;
    Ok(Json(json!({
        "num_successfully_created_invitations": success.len(),
        "num_failed_invitations": failed.len(),
        "failed_invitations": failed,
        "successful_invitations": serialized,
    }))
    .into_response())
}

/// GET /invites/retrieve: the caller's invite to an email.
pub async fn retrieve(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    uri: Uri,
) -> Result<Response, AppError> {
    let p = params::parse_query(uri.query().unwrap_or(""));
    let Some(user_id) = guardian.user_id() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    let Some(email) = string(&p, "email").filter(|e| !e.is_empty()) else {
        return Ok(super::accounts::param_missing("email"));
    };
    let mut conn = state.pool.acquire().await?;
    let s = SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let Some(invite) = Invite::find_by_inviter_and_email(&mut conn, user_id, &email).await? else {
        return Ok(super::search::invalid_parameters(&state, "email"));
    };
    if !invites::can_invite_to_forum(&mut conn, &s, &guardian, None).await? {
        return Ok(super::search::invalid_access(&state));
    }
    let doc = render(&mut conn, &state, &s, &guardian, &invite, true).await?;
    Ok(Json(doc).into_response())
}

/// PUT /invites/:id
pub async fn update(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let Req { p, settings: s } = match begin(&state, &headers, &uri, &body, "PUT").await? {
        Ok(r) => r,
        Err(r) => return Ok(r),
    };
    let Some(user_id) = guardian.user_id() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    let id = crate::ruby::to_i(id.strip_suffix(".json").unwrap_or(&id)) as i32;
    let mut tx = state.pool.begin().await?;
    let invite: Option<Invite> = sqlx::query_as(&format!(
        "SELECT {} FROM invites WHERE invited_by_id = $1 AND id = $2 AND deleted_at IS NULL",
        invites::INVITE_COLUMNS
    ))
    .bind(user_id)
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(invite) = invite else {
        return Ok(super::search::invalid_parameters(&state, "id"));
    };
    if invite.admin {
        for param in ["topic_id", "group_ids", "group_names"] {
            if present(&p, param) {
                return Ok(super::search::invalid_parameters(&state, param));
            }
        }
    }
    let mut topic_id = None;
    if present(&p, "topic_id") {
        let tid = string(&p, "topic_id")
            .map(|v| crate::ruby::to_i(&v) as i32)
            .unwrap_or(0);
        let Some((topic, can_see)) = invites::topic_for(&mut tx, &s, &guardian, tid).await? else {
            return Ok(super::search::invalid_parameters(&state, "topic_id"));
        };
        if !guardian.can_invite_to(&s, &topic, can_see)? {
            return Ok(super::search::invalid_access(&state));
        }
        topic_id = Some(topic.id);
    }
    if !p.contains_key("topic_id") {
        for tid in invite.topic_ids(&mut tx).await? {
            let allowed = match invites::topic_for(&mut tx, &s, &guardian, tid).await? {
                Some((topic, can_see)) => guardian.can_invite_to(&s, &topic, can_see)?,
                None => false,
            };
            if !allowed {
                return Ok(super::search::invalid_access(&state));
            }
        }
    }
    let groups = if present(&p, "group_ids") || present(&p, "group_names") {
        invites::lookup_groups(&mut tx, p.get("group_ids"), p.get("group_names")).await?
    } else {
        None
    };
    if !invites::can_invite_to_forum(&mut tx, &s, &guardian, groups.as_deref()).await? {
        return Ok(super::search::invalid_access(&state));
    }
    // Invite.transaction: a response returned from inside it commits
    // what was written before.
    if p.contains_key("topic_id") {
        sqlx::query("DELETE FROM topic_invites WHERE invite_id = $1")
            .bind(invite.id)
            .execute(&mut *tx)
            .await?;
        if let Some(topic_id) = topic_id {
            invites::add_topic(&mut tx, invite.id, topic_id).await?;
        }
    }
    if p.contains_key("group_ids") || p.contains_key("group_names") {
        sqlx::query("DELETE FROM invited_groups WHERE invite_id = $1")
            .bind(invite.id)
            .execute(&mut *tx)
            .await?;
        for group in groups.iter().flatten() {
            invites::add_group(&mut tx, invite.id, group.id).await?;
        }
    }
    let first_topic = invite.topic_ids(&mut tx).await?.first().copied();
    let invite_groups = invite.group_ids(&mut tx).await?;
    if !invites::groups_can_see_topic(&mut tx, &invite_groups, first_topic).await? {
        let message = invites::requires_groups_message(
            &mut tx,
            &s,
            &state.i18n,
            &guardian,
            first_topic.unwrap_or(0),
        )
        .await?;
        tx.commit().await?;
        return Ok(json_error(StatusCode::UNPROCESSABLE_ENTITY, message));
    }
    let mut email = invite.email.clone();
    let mut domain = invite.domain.clone();
    let mut status = invite.emailed_status;
    if p.contains_key("email") {
        let old_email = invite.email().map(str::to_string);
        let new_email = string(&p, "email").filter(|e| !crate::ruby::is_blank(e));
        if let Some(new_email) = &new_email {
            let other: Option<Invite> = sqlx::query_as(&format!(
                "SELECT {} FROM invites WHERE id <> $1 AND email = $2 AND invited_by_id = $3 \
                 AND deleted_at IS NULL ORDER BY id LIMIT 1",
                invites::INVITE_COLUMNS
            ))
            .bind(invite.id)
            .bind(new_email.to_lowercase())
            .bind(user_id)
            .fetch_optional(&mut *tx)
            .await?;
            if let Some(other) = other
                && other.redeemable(&mut tx).await?
            {
                let escaped = crate::email::notification::html_escape(new_email);
                tx.commit().await?;
                return Ok(json_error(
                    StatusCode::CONFLICT,
                    state
                        .i18n
                        .t_with("invite.invite_exists", &[("email", &escaped)])
                        .unwrap_or_default(),
                ));
            }
        }
        if old_email != new_email {
            status = Some(if new_email.is_some() && !truthy(p.get("skip_email")) {
                invites::emailed_status::PENDING
            } else {
                invites::emailed_status::NOT_REQUIRED
            });
        }
        // The invite's email before the update assigns the new one.
        if invite.email().is_some() {
            domain = None;
        }
    }
    if p.contains_key("domain") {
        domain = string(&p, "domain");
        if domain.as_deref().is_some_and(|d| !crate::ruby::is_blank(d)) {
            email = None;
            status = Some(invites::emailed_status::NOT_REQUIRED);
        }
    }
    if truthy(p.get("send_email")) {
        if !s.get("allow_email_invites")?.truthy() {
            tx.commit().await?;
            return Ok(json_error(
                StatusCode::UNPROCESSABLE_ENTITY,
                state
                    .i18n
                    .t("invite.email_invites_disabled")
                    .unwrap_or_default()
                    .to_string(),
            ));
        }
        if status != Some(invites::emailed_status::PENDING) {
            let limiter = RateLimiter::new(
                Some(crate::rate_limiter::LimitUser {
                    id: user_id,
                    staff: guardian.is_staff(),
                }),
                "resend-invite-per-hour",
                10,
                3600,
            );
            if limiter
                .performed(&state.pool, state.config.rate_limits)
                .await?
                .is_err()
            {
                tx.commit().await?;
                return Ok(crate::rate_limiter::slow_down(&state.i18n));
            }
        }
        status = Some(invites::emailed_status::PENDING);
    }
    // invite.update!(params.permit(:email, :description, :custom_message,
    // :max_redemptions_allowed, :expires_at))
    if p.contains_key("email") {
        email = string(&p, "email");
    }
    let description = if p.contains_key("description") {
        string(&p, "description")
    } else {
        invite.description.clone()
    };
    let custom_message = if p.contains_key("custom_message") {
        string(&p, "custom_message")
    } else {
        invite.custom_message.clone()
    };
    let max = if p.contains_key("max_redemptions_allowed") {
        p.get("max_redemptions_allowed")
            .and_then(invites::integer_param)
    } else {
        Some(i64::from(invite.max_redemptions_allowed))
    };
    let expires_at = if p.contains_key("expires_at") {
        match string(&p, "expires_at") {
            Some(v) => invites::parse_time_param(&v)?,
            None => None,
        }
    } else {
        Some(invite.expires_at)
    };
    let downcased = email.as_deref().map(invites::email_downcase);
    let mut draft = invites::Draft {
        id: Some(invite.id),
        email: downcased.clone(),
        domain: domain.clone(),
        custom_message: custom_message.clone(),
        description: description.clone(),
        max_redemptions_allowed: max,
        redemption_count: i64::from(invite.redemption_count),
        admin: invite.admin,
        moderator: invite.moderator,
        email_changed: downcased != invite.email,
        domain_changed: domain != invite.domain,
        admin_changed: false,
        moderator_changed: false,
    };
    let inviter = inviter(&mut tx, &guardian).await?;
    let errors = invites::validate(&mut tx, &s, &state.i18n, &inviter, &mut draft).await?;
    if let Some(first) = errors.into_iter().next() {
        tx.commit().await?;
        return Ok(json_error(StatusCode::UNPROCESSABLE_ENTITY, first));
    }
    let Some(expires_at) = expires_at else {
        return Err(Unsupported("clearing an invite's expires_at (NOT NULL)").into());
    };
    let max = draft.max_redemptions_allowed.unwrap_or(1) as i32;
    let changed = draft.email != invite.email
        || draft.domain != invite.domain
        || draft.custom_message != invite.custom_message
        || draft.description != invite.description
        || max != invite.max_redemptions_allowed
        || expires_at != invite.expires_at
        || status != invite.emailed_status;
    if changed {
        // before_save: a new email gets a new token.
        let email_token = if draft.email_changed {
            draft
                .email
                .as_deref()
                .filter(|e| !crate::ruby::is_blank(e))
                .map(|_| crate::accounts::random_hex())
        } else {
            invite.email_token.clone()
        };
        sqlx::query(
            "UPDATE invites SET email = $2, domain = $3, custom_message = $4, description = $5, \
             max_redemptions_allowed = $6, expires_at = $7, emailed_status = $8, email_token = $9, \
             updated_at = clock_timestamp() WHERE id = $1",
        )
        .bind(invite.id)
        .bind(&draft.email)
        .bind(&draft.domain)
        .bind(&draft.custom_message)
        .bind(&draft.description)
        .bind(max)
        .bind(expires_at)
        .bind(status)
        .bind(email_token)
        .execute(&mut *tx)
        .await?;
    }
    if status == Some(invites::emailed_status::PENDING) {
        sqlx::query("UPDATE invites SET emailed_status = $2 WHERE id = $1")
            .bind(invite.id)
            .bind(invites::emailed_status::SENDING)
            .execute(&mut *tx)
            .await?;
        crate::jobs::enqueue(
            &mut tx,
            crate::invite_mailer::INVITE_EMAIL,
            json!({
                "invite_id": invite.id,
                "invite_to_topic": p.get("invite_to_topic").cloned().unwrap_or(Value::Null),
            }),
        )
        .await?;
    }
    let invite = Invite::find_with_deleted(&mut tx, invite.id)
        .await?
        .ok_or(Unsupported("an invite gone while it was updated"))?;
    let doc = render(
        &mut tx,
        &state,
        &s,
        &guardian,
        &invite,
        p.contains_key("email"),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(doc).into_response())
}

/// DELETE /invites(/:id): InvitesController#destroy, trashing one.
pub async fn destroy(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    path_id: Option<Path<String>>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let Req { mut p, .. } = match begin(&state, &headers, &uri, &body, "DELETE").await? {
        Ok(r) => r,
        Err(r) => return Ok(r),
    };
    if let Some(Path(id)) = path_id {
        let id = id.strip_suffix(".json").unwrap_or(&id).to_string();
        p.insert("id".into(), Value::String(id));
    }
    let Some(user_id) = guardian.user_id() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    let Some(id) = string(&p, "id").filter(|v| !v.is_empty()) else {
        return Ok(super::accounts::param_missing("id"));
    };
    let mut tx = state.pool.begin().await?;
    let Some(invite) = Invite::find(&mut tx, crate::ruby::to_i(&id) as i32).await? else {
        return Ok(super::search::invalid_parameters(&state, "id"));
    };
    // ensure_can_destroy_invite!
    if !(guardian.is_admin() || invite.invited_by_id == user_id) {
        return Ok(super::search::invalid_access(&state));
    }
    invite.trash(&mut tx, Some(user_id)).await?;
    tx.commit().await?;
    Ok(success())
}

/// POST /invites/destroy-all-expired: the expired invites of a user
/// (one's own, or anyone's for an admin) trashed.
pub async fn destroy_all_expired(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let Req { p, .. } = match begin(&state, &headers, &uri, &body, "POST").await? {
        Ok(r) => r,
        Err(r) => return Ok(r),
    };
    let Some(actor) = guardian.user_id() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    let mut tx = state.pool.begin().await?;
    // fetch_user_from_params
    let Some(user_id) = fetch_user_from_params(&mut tx, &guardian, &p).await? else {
        return Ok(super::topics::not_found_response(&state));
    };
    // ensure_can_destroy_all_invites!
    if !(guardian.is_staff() && (guardian.is_admin() || guardian.is_me(user_id))) {
        return Ok(super::search::invalid_access(&state));
    }
    let ids: Vec<i32> = sqlx::query_scalar(
        "SELECT id FROM invites WHERE invited_by_id = $1 AND expires_at < $2 AND deleted_at IS NULL ORDER BY id",
    )
    .bind(user_id)
    .bind(crate::clock::now_naive())
    .fetch_all(&mut *tx)
    .await?;
    for id in ids {
        sqlx::query(
            "UPDATE invites SET deleted_at = clock_timestamp(), deleted_by_id = $2 WHERE id = $1",
        )
        .bind(id)
        .bind(actor)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(success())
}

/// `fetch_user_from_params` by `username`: None is NotFound.
async fn fetch_user_from_params(
    conn: &mut PgConnection,
    guardian: &Guardian,
    p: &Map<String, Value>,
) -> Result<Option<i32>, AppError> {
    let Some(username) = string(p, "username") else {
        if p.contains_key("external_id") {
            return Err(Unsupported("finding users by external_id").into());
        }
        return Ok(None);
    };
    let lower = username.to_lowercase();
    let lower = lower.strip_suffix(".json").unwrap_or(&lower);
    if let Some(u) = guardian.user()
        && u.username.to_lowercase() == lower
    {
        return Ok(Some(u.id));
    }
    let active_only = !guardian.is_staff();
    Ok(sqlx::query_scalar(
        "SELECT id FROM users WHERE username_lower = $1 AND (active OR NOT $2) ORDER BY id LIMIT 1",
    )
    .bind(lower)
    .bind(active_only)
    .fetch_optional(conn)
    .await?)
}

/// `Invite#resend_invite`
async fn resend(conn: &mut PgConnection, s: &SiteSettings, invite_id: i32) -> Result<(), AppError> {
    let days = s.get("invite_expiry_days")?.to_i();
    sqlx::query(
        "UPDATE invites SET updated_at = $2, invalidated_at = NULL, expires_at = $3 WHERE id = $1",
    )
    .bind(invite_id)
    .bind(crate::clock::now_naive())
    .bind(crate::clock::now_naive() + chrono::Duration::days(days))
    .execute(&mut *conn)
    .await?;
    crate::jobs::enqueue(
        conn,
        crate::invite_mailer::INVITE_EMAIL,
        json!({"invite_id": invite_id}),
    )
    .await?;
    Ok(())
}

fn email_invites_disabled(state: &AppState) -> Response {
    json_error(
        StatusCode::UNPROCESSABLE_ENTITY,
        state
            .i18n
            .t("invite.email_invites_disabled")
            .unwrap_or_default()
            .to_string(),
    )
}

/// POST /invites/reinvite: InvitesController#resend_invite.
pub async fn resend_invite(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let Req { p, settings: s } = match begin(&state, &headers, &uri, &body, "POST").await? {
        Ok(r) => r,
        Err(r) => return Ok(r),
    };
    let Some(user_id) = guardian.user_id() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    if !s.get("allow_email_invites")?.truthy() {
        return Ok(email_invites_disabled(&state));
    }
    let Some(email) = string(&p, "email").filter(|e| !e.is_empty()) else {
        return Ok(super::accounts::param_missing("email"));
    };
    let limiter = RateLimiter::new(
        Some(crate::rate_limiter::LimitUser {
            id: user_id,
            staff: guardian.is_staff(),
        }),
        "resend-invite-per-hour",
        10,
        3600,
    );
    if limiter
        .performed(&state.pool, state.config.rate_limits)
        .await?
        .is_err()
    {
        return Ok(crate::rate_limiter::slow_down(&state.i18n));
    }
    let mut tx = state.pool.begin().await?;
    let Some(invite) = Invite::find_by_inviter_and_email(&mut tx, user_id, &email).await? else {
        return Ok(super::search::invalid_parameters(&state, "email"));
    };
    resend(&mut tx, &s, invite.id).await?;
    tx.commit().await?;
    Ok(success())
}

/// POST /invites/reinvite-all: InvitesController#resend_all_invites, the
/// caller's pending email invites sent again.
pub async fn resend_all_invites(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let Req { settings: s, .. } = match begin(&state, &headers, &uri, &body, "POST").await? {
        Ok(r) => r,
        Err(r) => return Ok(r),
    };
    let Some(user_id) = guardian.user_id() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    if !s.get("allow_email_invites")?.truthy() {
        return Ok(email_invites_disabled(&state));
    }
    // ensure_can_resend_all_invites!
    if !guardian.is_staff() {
        return Ok(super::search::invalid_access(&state));
    }
    let limiter = RateLimiter::new(
        Some(crate::rate_limiter::LimitUser {
            id: user_id,
            staff: true,
        }),
        "bulk-reinvite-per-day",
        1,
        86_400,
    )
    .apply_limit_to_staff();
    if limiter
        .performed(&state.pool, state.config.rate_limits)
        .await?
        .is_err()
    {
        return Ok(crate::rate_limiter::slow_down(&state.i18n));
    }
    let mut tx = state.pool.begin().await?;
    // Invite.pending(current_user).where.not(email: nil).find_each
    let ids: Vec<i32> = sqlx::query_scalar(
        "SELECT DISTINCT invites.id FROM invites \
         LEFT JOIN invited_users ON invites.id = invited_users.invite_id \
         LEFT JOIN users ON invited_users.user_id = users.id \
         WHERE invites.deleted_at IS NULL AND invites.invited_by_id = $1 \
         AND redemption_count < max_redemptions_allowed AND expires_at > $2 \
         AND invites.email IS NOT NULL ORDER BY invites.id",
    )
    .bind(user_id)
    .bind(crate::clock::now_naive())
    .fetch_all(&mut *tx)
    .await?;
    for id in ids {
        resend(&mut tx, &s, id).await?;
    }
    tx.commit().await?;
    Ok(success())
}

/// GET /u/:username/invited(/:filter).json: UsersController#invited, a
/// page of the user's pending, expired or redeemed invites with the
/// counts of each.
pub async fn invited_json(
    state: &AppState,
    guardian: &Guardian,
    username: &str,
    path_filter: Option<&str>,
    uri: &Uri,
) -> Result<Response, AppError> {
    let p = params::parse_query(uri.query().unwrap_or(""));
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(state));
    }
    let mut conn = state.pool.acquire().await?;
    let s = SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    if !guardian.can_invite_to_forum(&s)? {
        if guardian.is_staff() {
            // render_invite_error
            let error = if s.get("enable_discourse_connect")?.truthy() {
                Value::String(
                    state
                        .i18n
                        .t("invite.disabled_errors.discourse_connect_enabled")
                        .unwrap_or_default()
                        .to_string(),
                )
            } else {
                Value::Null
            };
            return Ok(Json(
                json!({"invites": [], "can_see_invite_details": false, "error": error}),
            )
            .into_response());
        }
        return Ok(json_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            state
                .i18n
                .t("invite.disabled_errors.invalid_access")
                .unwrap_or_default()
                .to_string(),
        ));
    }
    let filter = path_filter
        .map(str::to_string)
        .or_else(|| string(&p, "filter"))
        .unwrap_or_else(|| "redeemed".into());
    // fetch_user_from_params(include_inactive: staff? || show_inactive_accounts)
    let include_inactive = guardian.is_staff() || s.get("show_inactive_accounts")?.truthy();
    let lower = username.to_lowercase();
    let user = if include_inactive
        || guardian
            .user()
            .is_some_and(|u| u.username.to_lowercase() == lower)
    {
        crate::users::User::find_any(&mut conn, &lower).await?
    } else {
        crate::users::User::find_active(&mut conn, &lower).await?
    };
    let Some(inviter) = user else {
        return Ok(super::topics::not_found_response(state));
    };
    let details = guardian.is_staff() || guardian.is_me(inviter.id);
    let show_emails = details;
    let offset = string(&p, "offset")
        .map(|o| crate::ruby::to_i(&o))
        .unwrap_or(0);
    let limit = s.get("invites_per_page")?.to_i();
    let search_param = string(&p, "search").filter(|v| !crate::ruby::is_blank(v));
    let base = base_url(state, &s)?;
    let list = match filter.as_str() {
        "pending" if details => Some(invites::List::Pending),
        "expired" if details => Some(invites::List::Expired),
        "redeemed" => Some(invites::List::Redeemed),
        _ => None,
    };
    let mut items = Vec::new();
    match list {
        Some(invites::List::Redeemed) => {
            // `invites.present?` before the search narrows them.
            let mut rows =
                invites::redeemed_page(&mut conn, inviter.id, offset, limit, None).await?;
            if let Some(search) = &search_param
                && !rows.is_empty()
            {
                rows = invites::redeemed_page(
                    &mut conn,
                    inviter.id,
                    offset,
                    limit,
                    Some((search, show_emails)),
                )
                .await?;
            }
            let urls = crate::url::Urls {
                config: &state.config,
                settings: &s,
            };
            let logo_small = crate::admin_users::logo_small_url(&mut conn, &s).await?;
            for row in rows {
                let user = match row.user_id {
                    Some(id) => crate::users::User::find_by_id(&mut conn, id).await?,
                    None => None,
                };
                let Some(user) = user else {
                    return Err(Unsupported("a redeemed invite whose user is gone").into());
                };
                let mut u = Map::new();
                u.insert("id".into(), json!(user.id));
                u.insert("username".into(), json!(user.username));
                if s.get("enable_names")?.truthy() {
                    u.insert("name".into(), json!(user.name));
                }
                u.insert(
                    "avatar_template".into(),
                    json!(crate::avatar::avatar_template(
                        &urls,
                        user.id,
                        &user.username,
                        user.uploaded_avatar_id,
                        logo_small.as_deref(),
                    )?),
                );
                // can_see_invite_details?(invite.invited_by)
                let sees = guardian.is_staff() || guardian.is_me(row.invited_by_id);
                if sees {
                    u.insert("topics_entered".into(), json!(user.topics_entered));
                    u.insert("posts_read_count".into(), json!(user.posts_read_count));
                }
                if user.visible_to(&s, guardian)? {
                    u.insert(
                        "last_seen_at".into(),
                        json!(user.last_seen_at.map(crate::themes::json_time)),
                    );
                }
                if sees {
                    u.insert("time_read".into(), json!(user.time_read));
                    u.insert("days_visited".into(), json!(user.days_visited));
                    let secs = (crate::clock::now_naive() - user.created_at).num_milliseconds()
                        as f64
                        / 1000.0;
                    u.insert(
                        "days_since_created".into(),
                        json!((secs / 60.0 / 60.0 / 24.0).ceil() as i64),
                    );
                }
                let source = if row
                    .invite_email
                    .as_deref()
                    .is_none_or(crate::ruby::is_blank)
                {
                    "link"
                } else {
                    "email"
                };
                items.push(json!({
                    "id": row.invite_id,
                    "redeemed_at": row.redeemed_at.map(crate::themes::json_time),
                    "user": Value::Object(u),
                    "invite_source": source,
                }));
            }
        }
        Some(list) => {
            let mut rows =
                invites::invites_page(&mut conn, inviter.id, list, offset, limit, None).await?;
            if let Some(search) = &search_param
                && !rows.is_empty()
            {
                rows = invites::invites_page(
                    &mut conn,
                    inviter.id,
                    list,
                    offset,
                    limit,
                    Some((search, show_emails)),
                )
                .await?;
            }
            for invite in rows {
                items.push(
                    invites::serialize(
                        &mut conn,
                        &s,
                        &state.i18n,
                        guardian,
                        &base,
                        &invite,
                        SerializeOpts { show_emails },
                    )
                    .await?,
                );
            }
        }
        None => {}
    }
    let pending = if details {
        invites::count(&mut conn, inviter.id, invites::List::Pending).await?
    } else {
        0
    };
    let expired = if details {
        invites::count(&mut conn, inviter.id, invites::List::Expired).await?
    } else {
        0
    };
    let redeemed = invites::count(&mut conn, inviter.id, invites::List::Redeemed).await?;
    Ok(Json(json!({
        "invites": items,
        "can_see_invite_details": details,
        "counts": {
            "pending": pending,
            "expired": expired,
            "redeemed": redeemed,
            "total": pending + expired,
        },
    }))
    .into_response())
}
