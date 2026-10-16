//! EmailUpdater (lib/email_updater.rb): a change of the primary email or
//! a new secondary one requested (EmailValidator, the taken and alias
//! checks, the request and its token mailed to the old address or the
//! new one), and a confirmation link followed (the old address's leads
//! to the new one's; the new one's completes the change).

use serde_json::json;
use sqlx::PgConnection;

use crate::accounts::{create_email_token, hash_token, token_scopes};
use crate::guardian::Guardian;
use crate::i18n::I18n;
use crate::site_settings::SiteSettings;
use crate::{AppError, Unsupported};

/// `EmailChangeRequest.states`
pub mod states {
    pub const AUTHORIZING_OLD: i32 = 1;
    pub const AUTHORIZING_NEW: i32 = 2;
    pub const COMPLETE: i32 = 3;
}

/// UserHistory actions EmailUpdater and the email endpoints log.
pub mod actions {
    pub const ADD_EMAIL: i32 = 88;
    pub const UPDATE_EMAIL: i32 = 89;
    pub const DESTROY_EMAIL: i32 = 90;
}

/// The user an email change is for.
pub struct Owner {
    pub id: i32,
    pub email: String,
    pub admin: bool,
    pub moderator: bool,
}

impl Owner {
    pub async fn load(conn: &mut PgConnection, user_id: i32) -> Result<Owner, AppError> {
        let (email, admin, moderator): (Option<String>, bool, bool) = sqlx::query_as(
            "SELECT (SELECT email FROM user_emails WHERE user_id = u.id AND \"primary\" LIMIT 1), \
                    u.admin, u.moderator FROM users u WHERE u.id = $1",
        )
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
        Ok(Owner {
            id: user_id,
            email: email.ok_or(Unsupported("a user without a primary email"))?,
            admin,
            moderator,
        })
    }

    fn staff(&self) -> bool {
        self.admin || self.moderator
    }
}

/// `guardian.can_edit_email?(user)`
pub fn can_edit_email(
    s: &SiteSettings,
    guardian: &Guardian,
    user_id: i32,
) -> Result<bool, AppError> {
    if s.get("auth_overrides_email")?.truthy() || !s.get("email_editable")?.truthy() {
        return Ok(false);
    }
    if guardian.is_admin() {
        return Ok(true);
    }
    if guardian.is_anonymous() || (guardian.is_moderator() && !guardian.is_me(user_id)) {
        return Ok(false);
    }
    Ok(guardian.is_me(user_id) || guardian.is_staff())
}

/// `UserHistory.create!` for an email action: by the user, or the staff
/// member acting for them (StaffActionLogger, with the target).
pub async fn log(
    conn: &mut PgConnection,
    guardian: &Guardian,
    user_id: i32,
    action: i32,
) -> Result<(), sqlx::Error> {
    let actor = guardian.user_id().unwrap_or(user_id);
    let by_staff = guardian.is_staff() && actor != user_id;
    sqlx::query(
        "INSERT INTO user_histories (action, acting_user_id, target_user_id, admin_only, created_at, updated_at) \
         VALUES ($1, $2, $3, TRUE, now(), now())",
    )
    .bind(action)
    .bind(if by_staff { actor } else { user_id })
    .bind(by_staff.then_some(user_id))
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// UserEmail#destroy_email_tokens: every token for the address, whoever's
/// (before an address changes, after a primary flag does, after one is
/// destroyed).
pub async fn destroy_email_tokens(conn: &mut PgConnection, email: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM email_tokens WHERE email = $1")
        .bind(email)
        .execute(conn)
        .await?;
    Ok(())
}

/// `ScreenedEmail.canonical(email)`
fn canonical(email: &str) -> String {
    let (name, domain) = email.split_once('@').unwrap_or((email, ""));
    let mut name = name.split('+').next().unwrap_or("").to_string();
    if ["gmail.com", "googlemail.com"].contains(&domain.to_lowercase().as_str()) {
        name = name.replace('.', "");
    }
    format!("{name}@{domain}").to_lowercase()
}

/// `ScreenedEmail.levenshtein`
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=a.len()).collect();
    for (i, cb) in b.iter().enumerate() {
        let mut row = vec![i + 1; a.len() + 1];
        for (j, ca) in a.iter().enumerate() {
            row[j + 1] = if ca == cb {
                prev[j]
            } else {
                prev[j + 1].min(row[j]).min(prev[j]) + 1
            };
        }
        prev = row;
    }
    prev[a.len()]
}

/// `ScreenedEmail.should_block?(email)`: the latest hundred screened
/// emails within the Levenshtein distance; the match recorded.
pub(crate) async fn should_block(
    conn: &mut PgConnection,
    s: &SiteSettings,
    email: &str,
) -> Result<bool, AppError> {
    let email = canonical(email);
    let screened: Vec<(i32, String, i32)> = sqlx::query_as(
        "SELECT id, email, action_type FROM screened_emails ORDER BY created_at DESC LIMIT 100",
    )
    .fetch_all(&mut *conn)
    .await?;
    let max = s.get("levenshtein_distance_spammer_emails")?.to_i() as usize;
    let near: Vec<&(i32, String, i32)> = screened
        .iter()
        .filter(|(_, e, _)| levenshtein(&e.to_lowercase(), &email) <= max)
        .collect();
    let found = match near.as_slice() {
        [] => return Ok(false),
        [one] => *one,
        // Ruby's `sort` with a one-argument block orders them by chance.
        _ => {
            return Err(
                Unsupported("several screened emails within the Levenshtein distance").into(),
            );
        }
    };
    sqlx::query(
        "UPDATE screened_emails SET match_count = match_count + 1, last_match_at = now(), updated_at = now() WHERE id = $1",
    )
    .bind(found.0)
    .execute(&mut *conn)
    .await?;
    // ScreenedEmail.actions[:block]
    Ok(found.2 == 1)
}

/// `EmailUpdater#change_to(email, add:)`: the error messages, none when
/// the request was made (or a taken address quietly mailed its owner).
#[allow(clippy::too_many_arguments)]
pub async fn change_to(
    conn: &mut PgConnection,
    s: &SiteSettings,
    i18n: &I18n,
    guardian: &Guardian,
    user: &Owner,
    email: &str,
    add: bool,
) -> Result<Vec<String>, AppError> {
    let email = email.trim().to_lowercase();
    let t = |key: &str| i18n.t(key).unwrap_or(key).to_string();
    let mut errors: Vec<String> = Vec::new();
    // EmailValidator, as full messages of :email.
    let mut invalid = false;
    if email.is_empty() {
        errors.push(format!("Email {}", t("user.email.blank")));
        invalid = true;
    } else if !crate::accounts::valid_email(&email) {
        errors.push(format!("Email {}", t("user.email.invalid")));
        invalid = true;
    }
    if !crate::signup::email_allowed(s, &email)? {
        errors.push(format!("Email {}", t("user.email.not_allowed")));
        invalid = true;
    }
    if !invalid && should_block(&mut *conn, s, &email).await? {
        errors.push(format!("Email {}", t("user.email.blocked")));
    }
    if !errors.is_empty() {
        return Ok(errors);
    }

    let mut existing: Option<(i32, bool)> = sqlx::query_as(
        "SELECT u.id, u.staged FROM user_emails e JOIN users u ON u.id = e.user_id \
         WHERE lower(e.email) = $1 LIMIT 1",
    )
    .bind(&email)
    .fetch_optional(&mut *conn)
    .await?;
    let mut alias = false;
    if existing.is_none() && s.get("normalize_emails")?.truthy() {
        existing = sqlx::query_as(
            "SELECT u.id, u.staged FROM user_emails e JOIN users u ON u.id = e.user_id \
             WHERE lower(e.normalized_email) = $1 LIMIT 1",
        )
        .bind(crate::signup::normalize_email(&email))
        .fetch_optional(&mut *conn)
        .await?;
        alias = existing.is_some();
    }
    if let Some((owner, staged)) = existing {
        if s.get("hide_email_address_taken")?.truthy() && owner != user.id {
            crate::jobs::enqueue(
                &mut *conn,
                "critical_user_email",
                json!({"type": "account_exists", "user_id": owner}),
            )
            .await?;
        } else if alias {
            errors.push(t("change_email.error_alias"));
        } else if staged {
            errors.push(t("change_email.error_staged"));
        } else {
            errors.push(t("change_email.error"));
        }
    }
    if add {
        let secondary: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM user_emails WHERE user_id = $1 AND NOT \"primary\"",
        )
        .bind(user.id)
        .fetch_one(&mut *conn)
        .await?;
        if secondary >= s.get("max_allowed_secondary_emails")?.to_i() {
            errors.push(t("change_email.max_secondary_emails_error"));
        }
    }
    if !errors.is_empty() || existing.is_some() {
        return Ok(errors);
    }

    log(&mut *conn, guardian, user.id, actions::ADD_EMAIL).await?;

    // EmailChangeRequest.find_or_initialize_by(user_id:, old_email:, new_email:)
    let old_email = (!add).then(|| user.email.clone());
    let found: Option<(i32, Option<i32>)> = sqlx::query_as(
        "SELECT id, change_state FROM email_change_requests \
         WHERE user_id = $1 AND old_email IS NOT DISTINCT FROM $2 AND new_email = $3 ORDER BY id LIMIT 1",
    )
    .bind(user.id)
    .bind(&old_email)
    .bind(&email)
    .fetch_optional(&mut *conn)
    .await?;
    let mut state = found.and_then(|(_, state)| state);
    if state.is_none() || state == Some(states::COMPLETE) {
        state = Some(
            if s.get("require_change_email_confirmation")?.truthy() || user.staff() {
                states::AUTHORIZING_OLD
            } else {
                states::AUTHORIZING_NEW
            },
        );
    }
    let state = state.unwrap_or(states::AUTHORIZING_NEW);
    let (old_token_id, new_token_id) = if state == states::AUTHORIZING_OLD {
        let token =
            create_email_token(&mut *conn, user.id, &user.email, token_scopes::EMAIL_UPDATE)
                .await?;
        send(
            &mut *conn,
            &user.email,
            if add {
                "confirm_old_email_add"
            } else {
                "confirm_old_email"
            },
            user.id,
            &token,
        )
        .await?;
        (Some(id_of_token(&mut *conn, &token).await?), None)
    } else {
        let token =
            create_email_token(&mut *conn, user.id, &email, token_scopes::EMAIL_UPDATE).await?;
        send(&mut *conn, &email, "confirm_new_email", user.id, &token).await?;
        (None, Some(id_of_token(&mut *conn, &token).await?))
    };
    match found {
        Some((id, _)) => {
            sqlx::query(
                "UPDATE email_change_requests SET change_state = $2, \
                    old_email_token_id = COALESCE($3, old_email_token_id), \
                    new_email_token_id = COALESCE($4, new_email_token_id), updated_at = now() \
                 WHERE id = $1",
            )
            .bind(id)
            .bind(state)
            .bind(old_token_id)
            .bind(new_token_id)
            .execute(&mut *conn)
            .await?;
        }
        None => {
            sqlx::query(
                "INSERT INTO email_change_requests (user_id, old_email, new_email, old_email_token_id, \
                    new_email_token_id, change_state, requested_by_user_id, created_at, updated_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, now(), now())",
            )
            .bind(user.id)
            .bind(&old_email)
            .bind(&email)
            .bind(old_token_id)
            .bind(new_token_id)
            .bind(state)
            .bind(guardian.user_id())
            .execute(&mut *conn)
            .await?;
        }
    }
    Ok(Vec::new())
}

async fn id_of_token(conn: &mut PgConnection, token: &str) -> Result<i32, sqlx::Error> {
    sqlx::query_scalar("SELECT id FROM email_tokens WHERE token_hash = $1")
        .bind(hash_token(token))
        .fetch_one(conn)
        .await
}

/// EmailUpdater#send_email
async fn send(
    conn: &mut PgConnection,
    to: &str,
    kind: &str,
    user_id: i32,
    token: &str,
) -> Result<(), sqlx::Error> {
    crate::jobs::enqueue(
        conn,
        "critical_user_email",
        json!({"to_address": to, "type": kind, "user_id": user_id, "email_token": token}),
    )
    .await
}

/// A change request a confirmation link belongs to.
pub struct Request {
    pub user_id: i32,
    pub old_email: Option<String>,
    pub new_email: String,
}

/// `EmailToken.confirmable(token, scope: :email_update)`: the token's id
/// and user.
pub async fn confirmable(
    conn: &mut PgConnection,
    s: &SiteSettings,
    token: &str,
) -> Result<Option<(i32, i32)>, AppError> {
    if token.trim().is_empty() {
        return Ok(None);
    }
    let valid_hours = s.get("email_token_valid_hours")?.to_i();
    Ok(sqlx::query_as(
        "SELECT t.id, t.user_id FROM email_tokens t JOIN users u ON u.id = t.user_id \
         WHERE t.token_hash = $1 AND NOT t.confirmed AND NOT t.expired \
           AND t.created_at >= now() - make_interval(hours => $2) AND (t.scope = $3 OR t.scope IS NULL) \
         ORDER BY (t.scope = $3) DESC NULLS LAST, t.id LIMIT 1",
    )
    .bind(hash_token(token))
    .bind(valid_hours as i32)
    .bind(token_scopes::EMAIL_UPDATE)
    .fetch_optional(&mut *conn)
    .await?)
}

/// UsersEmailController#load_change_request: the request whose old
/// (`old`) or new token this is, in the state that awaits it.
pub async fn load_request(
    conn: &mut PgConnection,
    token_id: i32,
    user_id: i32,
    old: bool,
) -> Result<Option<Request>, sqlx::Error> {
    let row: Option<(Option<String>, String)> = sqlx::query_as(&format!(
        "SELECT old_email, new_email FROM email_change_requests \
         WHERE user_id = $1 AND {} = $2 AND change_state = $3 ORDER BY id LIMIT 1",
        if old {
            "old_email_token_id"
        } else {
            "new_email_token_id"
        }
    ))
    .bind(user_id)
    .bind(token_id)
    .bind(if old {
        states::AUTHORIZING_OLD
    } else {
        states::AUTHORIZING_NEW
    })
    .fetch_optional(conn)
    .await?;
    Ok(row.map(|(old_email, new_email)| Request {
        user_id,
        old_email,
        new_email,
    }))
}

/// How `EmailUpdater#confirm` ends.
#[derive(PartialEq, Eq, Debug)]
pub enum Confirmed {
    Error,
    AuthorizingNew,
    Complete,
}

/// `EmailUpdater#confirm(token)`: the old address confirmed mails the new
/// one; the new one confirmed changes (or adds) the address.
pub async fn confirm(
    conn: &mut PgConnection,
    s: &SiteSettings,
    token: &str,
) -> Result<Confirmed, AppError> {
    let Some((token_id, user_id)) = confirmable(&mut *conn, s, token).await? else {
        return Ok(Confirmed::Error);
    };
    sqlx::query("UPDATE email_tokens SET confirmed = TRUE, updated_at = now() WHERE id = $1")
        .bind(token_id)
        .execute(&mut *conn)
        .await?;
    let request: Option<(i32, i32, Option<String>, String)> = sqlx::query_as(
        "SELECT id, change_state, old_email, new_email FROM email_change_requests \
         WHERE user_id = $1 AND (old_email_token_id = $2 OR new_email_token_id = $2) ORDER BY id LIMIT 1",
    )
    .bind(user_id)
    .bind(token_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((request_id, state, old_email, new_email)) = request else {
        return Ok(Confirmed::Error);
    };
    match state {
        states::AUTHORIZING_OLD => {
            let new_token =
                create_email_token(&mut *conn, user_id, &new_email, token_scopes::EMAIL_UPDATE)
                    .await?;
            let new_token_id = id_of_token(&mut *conn, &new_token).await?;
            sqlx::query(
                "UPDATE email_change_requests SET change_state = $2, new_email_token_id = $3, updated_at = now() WHERE id = $1",
            )
            .bind(request_id)
            .bind(states::AUTHORIZING_NEW)
            .bind(new_token_id)
            .execute(&mut *conn)
            .await?;
            send(
                &mut *conn,
                &new_email,
                "confirm_new_email",
                user_id,
                &new_token,
            )
            .await?;
            Ok(Confirmed::AuthorizingNew)
        }
        states::AUTHORIZING_NEW => {
            sqlx::query(
                "UPDATE email_change_requests SET change_state = $2, updated_at = now() WHERE id = $1",
            )
            .bind(request_id)
            .bind(states::COMPLETE)
            .execute(&mut *conn)
            .await?;
            let owner = Owner::load(&mut *conn, user_id).await?;
            if !owner.staff() {
                crate::jobs::enqueue(
                    &mut *conn,
                    "critical_user_email",
                    json!({
                        "to_address": owner.email,
                        "type": if old_email.is_some() { "notify_old_email" } else { "notify_old_email_add" },
                        "user_id": user_id,
                        "new_email": new_email,
                    }),
                )
                .await?;
            }
            // update_user_email
            match &old_email {
                Some(old) => {
                    if *old != new_email {
                        destroy_email_tokens(&mut *conn, old).await?;
                    }
                    sqlx::query(
                        "UPDATE user_emails SET email = $3, normalized_email = $4, updated_at = now() \
                         WHERE id = (SELECT id FROM user_emails WHERE user_id = $1 AND email = $2 ORDER BY id LIMIT 1)",
                    )
                    .bind(user_id)
                    .bind(old)
                    .bind(&new_email)
                    .bind(crate::signup::normalize_email(&new_email))
                    .execute(&mut *conn)
                    .await?;
                }
                None => {
                    sqlx::query(
                        "INSERT INTO user_emails (user_id, email, normalized_email, \"primary\", created_at, updated_at) \
                         VALUES ($1, $2, $3, FALSE, now(), now())",
                    )
                    .bind(user_id)
                    .bind(&new_email)
                    .bind(crate::signup::normalize_email(&new_email))
                    .execute(&mut *conn)
                    .await?;
                }
            }
            Ok(Confirmed::Complete)
        }
        _ => Ok(Confirmed::Error),
    }
}

#[cfg(test)]
mod tests {
    use super::{canonical, levenshtein};

    #[test]
    fn canonical_drops_tags_and_gmail_dots() {
        assert_eq!(canonical("A.B+x@Gmail.com"), "ab@gmail.com");
        assert_eq!(canonical("a.b+x@example.com"), "a.b@example.com");
    }

    #[test]
    fn levenshtein_counts_edits() {
        assert_eq!(levenshtein("kitten", "sitting"), 3);
        assert_eq!(levenshtein("same", "same"), 0);
    }
}
