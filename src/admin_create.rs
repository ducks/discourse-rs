//! `discourse-rs admin create`: lib/tasks/admin.rake's `admin:create`
//! without its prompts. The email names the account: an existing one gets
//! the new password (the task's default answer to "reset the password?"),
//! otherwise a new one is made with UserNameSuggester's username for the
//! email. Either way it is activated and granted admin (the default answer
//! to "grant Admin privileges?"): `grant_admin!`, trust level 1 at least,
//! its email tokens confirmed and `activate`. Nothing is mailed.
//!
//! The password must meet min_admin_password_length; the task's own save
//! checks min_password_length unless the email is a developer's, since the
//! account is not an admin yet when it is saved.

use sqlx::{Acquire, PgConnection};

use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

/// What the operator gives.
pub struct NewAdmin<'a> {
    pub email: &'a str,
    pub password: &'a str,
    /// The full name, asked when full_name_requirement is
    /// required_at_signup and the account has none.
    pub name: Option<&'a str>,
}

#[derive(Debug, PartialEq)]
pub enum Outcome {
    /// "Account created successfully with username ..."
    Created { username: String },
    /// "Account updated successfully!"
    Updated { username: String },
}

#[derive(Debug)]
pub enum AdminCreateError {
    /// What `admin.save` refused, as `errors.full_messages`.
    Invalid(Vec<String>),
    App(AppError),
}

impl std::fmt::Display for AdminCreateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AdminCreateError::Invalid(messages) => f.write_str(&messages.join("\n")),
            AdminCreateError::App(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for AdminCreateError {}

impl From<AppError> for AdminCreateError {
    fn from(e: AppError) -> Self {
        AdminCreateError::App(e)
    }
}

impl From<sqlx::Error> for AdminCreateError {
    fn from(e: sqlx::Error) -> Self {
        AdminCreateError::App(e.into())
    }
}

impl From<crate::site_settings::SettingError> for AdminCreateError {
    fn from(e: crate::site_settings::SettingError) -> Self {
        AdminCreateError::App(e.into())
    }
}

/// `User.find_by_email`: by any of the user's addresses, downcased.
async fn find_by_email(
    conn: &mut PgConnection,
    email: &str,
) -> Result<Option<(i32, String)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT u.id, u.username FROM users u JOIN user_emails e ON e.user_id = u.id \
         WHERE lower(e.email) = $1 ORDER BY e.\"primary\" DESC LIMIT 1",
    )
    .bind(email.trim().to_lowercase())
    .fetch_optional(&mut *conn)
    .await
}

/// `password` attribute's full message, as `errors.full_messages` has it.
fn password_message(
    state: &AppState,
    settings: &SiteSettings,
    key: &str,
    password: &str,
) -> Result<String, AppError> {
    let count = crate::accounts::too_short_count(settings, true, password)?;
    let label = state
        .i18n
        .t("activerecord.attributes.user.password")
        .unwrap_or("Password");
    Ok(format!(
        "{label} {}",
        crate::accounts::password_error_message(&state.i18n, key, count)
    ))
}

/// Creates or updates the admin, in one transaction.
pub async fn create(state: &AppState, admin: &NewAdmin<'_>) -> Result<Outcome, AdminCreateError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let mut tx = conn.begin().await?;
    let email = admin.email.trim().to_lowercase();
    let existing = find_by_email(&mut tx, &email).await?;

    let name_required = settings.get("full_name_requirement")?.to_s() == "required_at_signup";
    let given_name = admin.name.map(str::trim).filter(|n| !n.is_empty());
    let mut errors = Vec::new();

    let (user_id, username, created) = match existing {
        Some((user_id, username)) => {
            let current_name: Option<String> =
                sqlx::query_scalar("SELECT name FROM users WHERE id = $1")
                    .bind(user_id)
                    .fetch_one(&mut *tx)
                    .await?;
            let name = given_name.map(str::to_string).or(current_name.clone());
            if name_required && current_name.as_deref().is_none_or(|n| n.trim().is_empty()) {
                match given_name {
                    Some(n) => {
                        sqlx::query(
                            "UPDATE users SET name = $2, updated_at = clock_timestamp() WHERE id = $1",
                        )
                        .bind(user_id)
                        .bind(n)
                        .execute(&mut *tx)
                        .await?;
                    }
                    None => errors.push(name_blank(state)),
                }
            }
            let current: Option<(String, String, String)> = sqlx::query_as(
                "SELECT password_hash, password_salt, password_algorithm FROM user_passwords WHERE user_id = $1",
            )
            .bind(user_id)
            .fetch_optional(&mut *tx)
            .await?;
            if let Some(key) = crate::accounts::password_error(
                &settings,
                admin.password,
                &crate::accounts::PasswordOwner {
                    admin: true,
                    username: &username,
                    name: name.as_deref(),
                    email: Some(&email),
                    current: current
                        .as_ref()
                        .map(|(h, s, a)| (h.as_str(), s.as_str(), a.as_str())),
                },
            )? {
                errors.push(password_message(state, &settings, key, admin.password)?);
            }
            if !errors.is_empty() {
                return Err(AdminCreateError::Invalid(errors));
            }
            crate::accounts::set_password(&mut tx, user_id, admin.password).await?;
            // expire_tokens_if_password_changed
            sqlx::query("DELETE FROM user_auth_tokens WHERE user_id = $1")
                .bind(user_id)
                .execute(&mut *tx)
                .await?;
            (user_id, username, false)
        }
        None => {
            let username = crate::user_name_suggester::suggest(
                &mut tx,
                &settings,
                &state.i18n,
                &[Some(&email)],
                None,
                true,
            )
            .await?
            .ok_or_else(|| {
                AppError::from(Unsupported("an email no username can be suggested for"))
            })?;
            if !crate::accounts::valid_email(&email) {
                let label = state
                    .i18n
                    .t("activerecord.attributes.user.primary_email")
                    .unwrap_or("Primary email");
                let invalid = state.i18n.t("user.email.invalid").unwrap_or("is invalid");
                errors.push(format!("{label} {invalid}"));
            }
            if name_required && given_name.is_none() {
                errors.push(name_blank(state));
            }
            if let Some(key) = crate::accounts::password_error(
                &settings,
                admin.password,
                &crate::accounts::PasswordOwner {
                    admin: true,
                    username: &username,
                    name: given_name,
                    email: Some(&email),
                    current: None,
                },
            )? {
                errors.push(password_message(state, &settings, key, admin.password)?);
            }
            if !errors.is_empty() {
                return Err(AdminCreateError::Invalid(errors));
            }
            let payloads = crate::web_hooks::Payloads {
                settings: &settings,
                i18n: &state.i18n,
                config: &state.config,
            };
            let user_id = crate::signup::insert_user(
                &mut tx,
                &settings,
                &payloads,
                &crate::signup::NewUser {
                    username: &username,
                    name: given_name,
                    email: &email,
                    password: Some(admin.password),
                    trust_level: settings.get("default_trust_level")?.to_i() as i32,
                    active: false,
                    approved: false,
                    moderator: false,
                    staged: false,
                    locale: None,
                    ip: None,
                    custom_fields: &[],
                },
            )
            .await?;
            (user_id, username, true)
        }
    };

    // "Ensuring account is active!": admin.active = true; admin.save
    sqlx::query("UPDATE users SET active = TRUE, updated_at = clock_timestamp() WHERE id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;

    // admin.grant_admin!
    let (is_admin, trust_level): (bool, i32) =
        sqlx::query_as("SELECT admin, trust_level FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await?;
    if !is_admin {
        crate::roles::grant(
            &mut tx,
            &state.i18n,
            user_id,
            crate::roles::Permission::Admin,
        )
        .await?;
    } else {
        crate::setup::clear_global_notice_if_needed(&mut tx, user_id).await?;
    }
    // admin.change_trust_level!(1) if admin.trust_level < 1
    let host = crate::pretty_text::Host::from_state(state);
    let promotion = crate::promotion::Ctx {
        settings: &settings,
        i18n: &state.i18n,
        host: &host,
        config: &state.config,
    };
    if trust_level < 1
        && let Err(message) =
            crate::promotion::change_trust_level(&mut tx, &promotion, user_id, 1, None).await?
    {
        return Err(AdminCreateError::Invalid(vec![message]));
    }
    // admin.email_tokens.update_all confirmed: true
    sqlx::query("UPDATE email_tokens SET confirmed = TRUE WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    // admin.activate: a signup token, confirmed.
    let token = crate::accounts::create_email_token(
        &mut tx,
        user_id,
        &email,
        crate::accounts::token_scopes::SIGNUP,
    )
    .await?;
    crate::routes::accounts::confirm_email_token(
        state,
        &mut tx,
        &settings,
        &token,
        crate::accounts::token_scopes::SIGNUP,
    )
    .await?;
    tx.commit().await?;
    Ok(if created {
        Outcome::Created { username }
    } else {
        Outcome::Updated { username }
    })
}

/// The name's blank error, as `errors.full_messages` has it.
fn name_blank(state: &AppState) -> String {
    let label = state
        .i18n
        .t("activerecord.attributes.user.name")
        .unwrap_or("Name");
    let blank = state
        .i18n
        .t("errors.messages.blank")
        .unwrap_or("can't be blank");
    format!("{label} {blank}")
}
