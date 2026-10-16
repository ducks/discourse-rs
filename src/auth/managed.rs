//! Auth::ManagedAuthenticator (lib/auth/managed_authenticator.rb): an
//! external account kept as a `user_associated_accounts` row, matched to
//! a user by the account, the user connecting it, or a verified email;
//! and Auth::Result (lib/auth/result.rb), what a login through it found,
//! as the client sees it and as signing up reads it from the server
//! session.

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::site_settings::SiteSettings;
use crate::{AppError, Unsupported};

/// An `OmniAuth::AuthHash`.
#[derive(Debug, Clone, Default)]
pub struct AuthHash {
    pub provider: String,
    pub uid: String,
    pub info: Map<String, Value>,
    pub credentials: Value,
    pub extra: Map<String, Value>,
}

impl AuthHash {
    /// `auth.to_json`, as persist_auth_token keeps it: InfoHash#to_hash
    /// fills in a missing name (the first and last names, the nickname or
    /// the email).
    pub fn to_json(&self) -> Value {
        let mut info = self.info.clone();
        if !truthy(info.get("name")) {
            let s = |k: &str| match info.get(k) {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Null) | None => String::new(),
                Some(other) => other.to_string(),
            };
            let name = if truthy(info.get("first_name")) || truthy(info.get("last_name")) {
                json!(format!("{} {}", s("first_name"), s("last_name")).trim())
            } else if truthy(info.get("nickname")) {
                info["nickname"].clone()
            } else if truthy(info.get("email")) {
                info["email"].clone()
            } else {
                Value::Null
            };
            info.insert("name".into(), name);
        }
        json!({
            "provider": self.provider,
            "uid": self.uid,
            "info": info,
            "credentials": self.credentials,
            "extra": self.extra,
        })
    }

    pub fn from_json(v: &Value) -> AuthHash {
        let s = |k: &str| match &v[k] {
            Value::String(s) => s.clone(),
            Value::Null => String::new(),
            other => other.to_string(),
        };
        AuthHash {
            provider: s("provider"),
            uid: s("uid"),
            info: v["info"].as_object().cloned().unwrap_or_default(),
            credentials: v.get("credentials").cloned().unwrap_or(Value::Null),
            extra: v["extra"].as_object().cloned().unwrap_or_default(),
        }
    }

    /// An info value as a string (`info[:email]`).
    pub fn info_str(&self, key: &str) -> Option<String> {
        match self.info.get(key) {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s.clone()),
            Some(other) => Some(other.to_string()),
        }
    }
}

/// A value Ruby would call truthy.
pub fn truthy(v: Option<&Value>) -> bool {
    !matches!(v, None | Some(Value::Null) | Some(Value::Bool(false)))
}

/// `true`, or the string "true" in any case: providers that send
/// email_verified as a string.
pub fn true_or_string_true(v: Option<&Value>) -> bool {
    match v {
        Some(Value::Bool(true)) => true,
        Some(Value::String(s)) => s.to_lowercase() == "true",
        _ => false,
    }
}

/// What an authenticator decides from settings and the auth hash.
pub struct Behaviour {
    /// `primary_email_verified?(auth)`
    pub email_verified: bool,
    /// `match_by_email`
    pub match_by_email: bool,
    /// `always_update_user_email?`
    pub overrides_email: bool,
    /// `can_connect_existing_user?`
    pub can_connect: bool,
    /// `find_user_by_email` replaced (GitHub's verified secondaries): the
    /// user it finds.
    pub user_by_email: Option<Option<i32>>,
}

/// An `Auth::Result`.
#[derive(Debug, Clone, Default)]
pub struct AuthResult {
    pub user_id: Option<i32>,
    pub name: Option<String>,
    pub username: Option<String>,
    email: Option<String>,
    pub email_valid: bool,
    /// `extra_data`: the account's provider and uid.
    pub extra_data: Option<(String, String)>,
    pub awaiting_activation: bool,
    pub awaiting_approval: bool,
    pub authenticated: bool,
    pub authenticator_name: Option<String>,
    pub requires_invite: bool,
    pub not_allowed_from_ip_address: bool,
    pub admin_not_allowed_from_ip_address: bool,
    pub skip_email_validation: bool,
    pub destination_url: Option<String>,
    pub omniauth_disallow_totp: bool,
    pub failed: bool,
    pub failed_reason: Option<String>,
    /// `associated_groups`: `{id, name}` each.
    pub associated_groups: Option<Vec<(String, String)>>,
    /// `user_field_values`: field id to value.
    pub user_field_values: Option<Map<String, Value>>,
    pub overrides_email: bool,
    pub overrides_username: bool,
    pub overrides_name: bool,
}

impl AuthResult {
    pub fn failed(reason: impl Into<String>) -> AuthResult {
        AuthResult {
            failed: true,
            failed_reason: Some(reason.into()),
            ..Default::default()
        }
    }

    /// `email`: downcased.
    pub fn email(&self) -> Option<String> {
        self.email.as_ref().map(|e| e.to_lowercase())
    }

    pub fn set_email(&mut self, email: Option<String>) {
        self.email = email;
    }

    /// `session_data`: what signing up reads back.
    pub fn session_data(&self) -> Value {
        json!({
            "email": self.email(),
            "username": self.username,
            "email_valid": self.email_valid,
            "name": self.name,
            "authenticator_name": self.authenticator_name,
            "extra_data": self.extra_data.as_ref().map(|(p, u)| json!({"provider": p, "uid": u})),
            "skip_email_validation": self.skip_email_validation,
            "associated_groups": self.associated_groups.as_ref().map(|groups| {
                groups.iter().map(|(id, name)| json!({"id": id, "name": name})).collect::<Vec<_>>()
            }),
            "user_field_values": self.user_field_values,
            "overrides_email": self.overrides_email,
            "overrides_username": self.overrides_username,
            "overrides_name": self.overrides_name,
        })
    }

    /// `Auth::Result.from_session_data(data, user:)`
    pub fn from_session_data(data: &Value) -> AuthResult {
        let s = |k: &str| match &data[k] {
            Value::String(s) => Some(s.clone()),
            _ => None,
        };
        let b = |k: &str| truthy(data.get(k));
        AuthResult {
            email: s("email"),
            username: s("username"),
            email_valid: b("email_valid"),
            name: s("name"),
            authenticator_name: s("authenticator_name"),
            extra_data: match (&data["extra_data"]["provider"], &data["extra_data"]["uid"]) {
                (Value::String(p), Value::String(u)) => Some((p.clone(), u.clone())),
                (Value::String(p), u) if !u.is_null() => Some((p.clone(), u.to_string())),
                _ => None,
            },
            skip_email_validation: b("skip_email_validation"),
            associated_groups: data["associated_groups"].as_array().map(|groups| {
                groups
                    .iter()
                    .map(|g| {
                        let s = |v: &Value| match v {
                            Value::String(s) => s.clone(),
                            other => other.to_string(),
                        };
                        (s(&g["id"]), s(&g["name"]))
                    })
                    .collect()
            }),
            user_field_values: data["user_field_values"].as_object().cloned(),
            overrides_email: b("overrides_email"),
            overrides_username: b("overrides_username"),
            overrides_name: b("overrides_name"),
            ..Default::default()
        }
    }

    /// `can_edit_name`
    pub fn can_edit_name(&self, s: &SiteSettings) -> Result<bool, AppError> {
        Ok(!(s.get("auth_overrides_name")?.truthy() || self.overrides_name))
    }

    /// `can_edit_username`
    pub fn can_edit_username(&self, s: &SiteSettings) -> Result<bool, AppError> {
        Ok(!(s.get("auth_overrides_username")?.truthy() || self.overrides_username))
    }

    /// `resolve_username`: a staged user's name for their email (when
    /// the provider's nickname is theirs or missing), else a suggestion.
    pub async fn resolve_username(
        &self,
        conn: &mut PgConnection,
        s: &SiteSettings,
        i18n: &crate::i18n::I18n,
        current_username: Option<&str>,
    ) -> Result<Option<String>, AppError> {
        if let Some(email) = self.email().filter(|e| !e.trim().is_empty())
            && self.email_valid
        {
            let staged: Option<String> = sqlx::query_scalar(
                "SELECT u.username FROM users u JOIN user_emails e ON e.user_id = u.id \
                 WHERE u.staged AND lower(e.email) = $1 LIMIT 1",
            )
            .bind(&email)
            .fetch_optional(&mut *conn)
            .await?;
            if let Some(staged) = staged {
                let matches = match self.username.as_deref().filter(|u| !u.trim().is_empty()) {
                    None => true,
                    Some(u) => {
                        crate::user_name_suggester::fix_username(s, i18n, Some(u), true)?.as_deref()
                            == Some(staged.as_str())
                    }
                };
                if matches {
                    return Ok(Some(staged));
                }
            }
        }
        let mut input = vec![self.username.as_deref()];
        if s.get("use_name_for_username_suggestions")?.truthy() {
            input.push(self.name.as_deref());
        }
        let email = self.email();
        if s.get("use_email_for_username_and_name_suggestions")?
            .truthy()
        {
            input.push(email.as_deref());
        }
        crate::user_name_suggester::suggest(conn, s, i18n, &input, current_username, false).await
    }

    /// `to_client_hash`: what the browser's authentication_data cookie
    /// says of the login.
    pub async fn to_client_hash(
        &self,
        conn: &mut PgConnection,
        s: &SiteSettings,
        i18n: &crate::i18n::I18n,
    ) -> Result<Map<String, Value>, AppError> {
        let mut out = Map::new();
        if self.requires_invite {
            out.insert("requires_invite".into(), json!(true));
            return Ok(out);
        }
        if let Some(user_id) = self.user_id {
            let suspended: bool = sqlx::query_scalar(
                "SELECT suspended_till IS NOT NULL AND suspended_till > now() FROM users WHERE id = $1",
            )
            .bind(user_id)
            .fetch_one(&mut *conn)
            .await?;
            if suspended {
                return Err(
                    Unsupported("a suspended user's external login (suspended_message)").into(),
                );
            }
        }
        if self.omniauth_disallow_totp {
            out.insert("omniauth_disallow_totp".into(), json!(true));
            out.insert("email".into(), json!(self.email()));
            return Ok(out);
        }
        if self.user_id.is_some() {
            out.insert("authenticated".into(), json!(self.authenticated));
            out.insert(
                "awaiting_activation".into(),
                json!(self.awaiting_activation),
            );
            out.insert("awaiting_approval".into(), json!(self.awaiting_approval));
            out.insert(
                "not_allowed_from_ip_address".into(),
                json!(self.not_allowed_from_ip_address),
            );
            out.insert(
                "admin_not_allowed_from_ip_address".into(),
                json!(self.admin_not_allowed_from_ip_address),
            );
            if self.authenticated
                && let Some(d) = self
                    .destination_url
                    .as_ref()
                    .filter(|d| !d.trim().is_empty())
            {
                out.insert("destination_url".into(), json!(d));
            }
            return Ok(out);
        }
        out.insert("email".into(), json!(self.email()));
        out.insert(
            "username".into(),
            json!(self.resolve_username(conn, s, i18n, None).await?),
        );
        out.insert("auth_provider".into(), json!(self.authenticator_name));
        out.insert("email_valid".into(), json!(self.email_valid));
        out.insert(
            "can_edit_username".into(),
            json!(self.can_edit_username(s)?),
        );
        out.insert("can_edit_name".into(), json!(self.can_edit_name(s)?));
        if let Some(d) = self
            .destination_url
            .as_ref()
            .filter(|d| !d.trim().is_empty())
        {
            out.insert("destination_url".into(), json!(d));
        }
        if s.get("enable_names")?.truthy() {
            let name = self.name.clone().filter(|n| !n.trim().is_empty());
            out.insert("name".into(), json!(name));
            out.insert("name_from_provider".into(), json!(name.is_some()));
            if name.is_none() && self.can_edit_name(s)? {
                let from = self.username.clone().or_else(|| self.email());
                out.insert(
                    "name".into(),
                    json!(crate::user_name_suggester::suggest_name(
                        from.as_deref().unwrap_or("")
                    )),
                );
            }
        }
        Ok(out)
    }
}

/// `User.find_by_email(email)`: any of a user's addresses.
pub async fn user_by_email(conn: &mut PgConnection, email: &str) -> Result<Option<i32>, AppError> {
    Ok(sqlx::query_scalar(
        "SELECT user_id FROM user_emails WHERE lower(email) = lower($1) ORDER BY \"primary\" DESC, id LIMIT 1",
    )
    .bind(email.trim())
    .fetch_optional(&mut *conn)
    .await?)
}

/// `ManagedAuthenticator#after_authenticate(auth_token, existing_account:)`
pub async fn after_authenticate(
    conn: &mut PgConnection,
    s: &SiteSettings,
    auth: &AuthHash,
    behaviour: &Behaviour,
    existing_account: Option<i32>,
) -> Result<AuthResult, AppError> {
    #[derive(sqlx::FromRow)]
    struct Assoc {
        id: i64,
        user_id: Option<i32>,
    }
    let found: Option<Assoc> = sqlx::query_as(
        "SELECT id, user_id FROM user_associated_accounts WHERE provider_name = $1 AND provider_uid = $2",
    )
    .bind(&auth.provider)
    .bind(&auth.uid)
    .fetch_optional(&mut *conn)
    .await?;
    let mut user_id = found.as_ref().and_then(|a| a.user_id);
    // A user that is gone reads as none (`association.user`).
    if let Some(id) = user_id {
        let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1)")
            .bind(id)
            .fetch_one(&mut *conn)
            .await?;
        if !exists {
            user_id = None;
        }
    }
    // Reconnecting to an existing account.
    if behaviour.can_connect
        && let Some(existing) = existing_account
        && user_id != Some(existing)
    {
        user_id = Some(existing);
    }
    // Matching an account by email.
    if behaviour.match_by_email && user_id.is_none() {
        let by_email = match behaviour.user_by_email {
            Some(found) => found,
            None => match auth.info_str("email") {
                Some(email) if behaviour.email_verified => user_by_email(conn, &email).await?,
                _ => None,
            },
        };
        if let Some(user) = by_email {
            // Destroy existing associations for the new user.
            sqlx::query(
                "DELETE FROM user_associated_accounts WHERE user_id = $1 AND provider_name = $2",
            )
            .bind(user)
            .bind(&auth.provider)
            .execute(&mut *conn)
            .await?;
            user_id = Some(user);
        }
    }
    let info = Value::Object(auth.info.clone());
    let credentials = match &auth.credentials {
        Value::Null => json!({}),
        other => other.clone(),
    };
    let extra = Value::Object(auth.extra.clone());
    let still_there = match &found {
        Some(a) => {
            sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS (SELECT 1 FROM user_associated_accounts WHERE id = $1)",
            )
            .bind(a.id)
            .fetch_one(&mut *conn)
            .await?
        }
        None => false,
    };
    if let (Some(a), true) = (&found, still_there) {
        sqlx::query(
            "UPDATE user_associated_accounts SET user_id = $2, info = $3, credentials = $4, extra = $5, \
                    last_used = now(), updated_at = now() WHERE id = $1",
        )
        .bind(a.id)
        .bind(user_id)
        .bind(&info)
        .bind(&credentials)
        .bind(&extra)
        .execute(&mut *conn)
        .await?;
    } else {
        sqlx::query(
            "INSERT INTO user_associated_accounts (provider_name, provider_uid, user_id, last_used, info, \
                                                   credentials, extra, created_at, updated_at) \
             VALUES ($1, $2, $3, now(), $4, $5, $6, now(), now())",
        )
        .bind(&auth.provider)
        .bind(&auth.uid)
        .bind(user_id)
        .bind(&info)
        .bind(&credentials)
        .bind(&extra)
        .execute(&mut *conn)
        .await?;
    }
    if let Some(user) = user_id {
        retrieve_avatar(conn, s, user, auth.info_str("image").as_deref()).await?;
        retrieve_profile(conn, user, &auth.info).await?;
    }
    let mut result = AuthResult::default();
    result.set_email(auth.info_str("email"));
    let first = auth.info_str("first_name").unwrap_or_default();
    let last = auth.info_str("last_name").unwrap_or_default();
    let full = format!("{first} {last}");
    result.name = if full.trim().is_empty() {
        auth.info_str("name")
    } else {
        Some(full)
    };
    // An email in the name parameter is not shown as one.
    if result.name.as_deref().is_some_and(|n| !n.trim().is_empty()) && result.name == result.email {
        result.name = None;
    }
    result.username = auth.info_str("nickname");
    if result
        .email
        .as_deref()
        .is_some_and(|e| !e.trim().is_empty())
    {
        result.email_valid = behaviour.email_verified;
    }
    result.overrides_email = behaviour.overrides_email;
    result.extra_data = Some((auth.provider.clone(), auth.uid.clone()));
    result.user_id = user_id;
    Ok(result)
}

/// `retrieve_avatar(user, url)`
pub async fn retrieve_avatar(
    conn: &mut PgConnection,
    s: &SiteSettings,
    user_id: i32,
    url: Option<&str>,
) -> Result<(), AppError> {
    let Some(url) = url.filter(|u| !u.trim().is_empty()) else {
        return Ok(());
    };
    let custom: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM user_avatars WHERE user_id = $1 AND custom_upload_id IS NOT NULL)",
    )
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?;
    if custom && !s.get("auth_overrides_avatar")?.truthy() {
        return Ok(());
    }
    crate::jobs::enqueue(
        &mut *conn,
        "download_avatar_from_url",
        json!({"url": url, "user_id": user_id, "override_gravatar": false}),
    )
    .await?;
    Ok(())
}

/// `retrieve_profile(user, info)`: the provider's description and
/// location where the profile has none.
pub async fn retrieve_profile(
    conn: &mut PgConnection,
    user_id: i32,
    info: &Map<String, Value>,
) -> Result<(), AppError> {
    let bio = info.get("description").filter(|v| !v.is_null());
    let location = info.get("location").filter(|v| !v.is_null());
    if bio.is_none() && location.is_none() {
        return Ok(());
    }
    if bio.is_some() {
        return Err(Unsupported("a provider's description as the bio (cooking it)").into());
    }
    if let Some(Value::String(location)) = location {
        sqlx::query(
            "UPDATE user_profiles SET location = $2 WHERE user_id = $1 AND COALESCE(btrim(location), '') = ''",
        )
        .bind(user_id)
        .bind(location)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// `apply_user_attributes!`: the username, email and name the provider
/// overrides on the user (settings or the authenticator), saved when one
/// changed.
pub async fn apply_user_attributes(
    conn: &mut PgConnection,
    cx: &crate::discourse_connect::Ctx<'_>,
    result: &AuthResult,
    user_id: i32,
) -> Result<(), AppError> {
    let s = cx.settings;
    let (username, email, name): (String, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT u.username, (SELECT email FROM user_emails WHERE user_id = u.id AND \"primary\"), u.name \
         FROM users u WHERE id = $1",
    )
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?;
    if s.get("auth_overrides_username")?.truthy() || result.overrides_username {
        let resolved = result
            .resolve_username(conn, s, cx.i18n, Some(&username))
            .await?
            .filter(|u| !u.trim().is_empty());
        if let Some(resolved) = resolved {
            crate::discourse_connect::override_username_of(conn, cx, user_id, &resolved).await?;
        }
    }
    let invalid_email = email.as_deref().is_some_and(|e| e.ends_with(".invalid"));
    let mut new_email = None;
    if (s.get("auth_overrides_email")?.truthy() || result.overrides_email || invalid_email)
        && result.email_valid
        && let Some(given) = result.email().filter(|e| !e.trim().is_empty())
        && email.as_deref() != Some(given.as_str())
    {
        if crate::discourse_connect::email_error(conn, s, &given, Some(user_id))
            .await?
            .is_some()
        {
            return Err(
                Unsupported("an external login's email another user has (RecordInvalid)").into(),
            );
        }
        new_email = Some(given);
    }
    let mut new_name = None;
    if (s.get("auth_overrides_name")?.truthy() || result.overrides_name)
        && let Some(given) = result.name.clone().filter(|n| !n.trim().is_empty())
        && name.as_deref() != Some(given.as_str())
    {
        new_name = Some(given);
    }
    if new_email.is_some() || new_name.is_some() {
        crate::discourse_connect::save_email_and_name(conn, cx, user_id, new_email, new_name)
            .await?;
    }
    Ok(())
}

/// `apply_associated_attributes!`: the provider's groups as the user's
/// associated groups (when it provides them), the user field values it
/// sent, and the trust level groups refreshed (consistent data leaves
/// them as they are).
pub async fn apply_associated_attributes(
    conn: &mut PgConnection,
    s: &SiteSettings,
    result: &AuthResult,
    user_id: i32,
    provides_groups: bool,
) -> Result<(), AppError> {
    if provides_groups && let Some(groups) = &result.associated_groups {
        let provider = result
            .extra_data
            .as_ref()
            .map(|(p, _)| p.clone())
            .unwrap_or_default();
        let mut ids: Vec<i64> = Vec::new();
        let mut seen: Vec<&(String, String)> = Vec::new();
        for g in groups {
            if seen.contains(&g) {
                continue;
            }
            seen.push(g);
            let (provider_id, name) = g;
            let existing: Option<i64> = sqlx::query_scalar(
                "SELECT id FROM associated_groups WHERE name = $1 AND provider_id = $2 AND provider_name = $3",
            )
            .bind(name)
            .bind(provider_id)
            .bind(&provider)
            .fetch_optional(&mut *conn)
            .await?;
            let id = match existing {
                Some(id) => id,
                None => {
                    sqlx::query_scalar(
                        "INSERT INTO associated_groups (name, provider_name, provider_id, created_at, updated_at) \
                         VALUES ($1, $2, $3, now(), now()) RETURNING id",
                    )
                    .bind(name)
                    .bind(&provider)
                    .bind(provider_id)
                    .fetch_one(&mut *conn)
                    .await?
                }
            };
            ids.push(id);
        }
        // user.update(associated_group_ids:): the join rows to match, whose
        // callbacks add to and remove from the linked groups.
        let current: Vec<i64> = sqlx::query_scalar(
            "SELECT associated_group_id FROM user_associated_groups WHERE user_id = $1",
        )
        .bind(user_id)
        .fetch_all(&mut *conn)
        .await?;
        let changed: Vec<i64> = ids
            .iter()
            .filter(|id| !current.contains(id))
            .chain(current.iter().filter(|id| !ids.contains(id)))
            .copied()
            .collect();
        let linked: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM group_associated_groups WHERE associated_group_id = ANY($1))",
        )
        .bind(&changed)
        .fetch_one(&mut *conn)
        .await?;
        if linked {
            return Err(
                Unsupported("groups linked to a provider's groups (add_automatically)").into(),
            );
        }
        sqlx::query(
            "DELETE FROM user_associated_groups WHERE user_id = $1 AND NOT (associated_group_id = ANY($2))",
        )
        .bind(user_id)
        .bind(&ids)
        .execute(&mut *conn)
        .await?;
        for id in &ids {
            if !current.contains(id) {
                sqlx::query(
                    "INSERT INTO user_associated_groups (user_id, associated_group_id, created_at, updated_at) \
                     VALUES ($1, $2, now(), now())",
                )
                .bind(user_id)
                .bind(id)
                .execute(&mut *conn)
                .await?;
            }
        }
        sqlx::query(
            "UPDATE associated_groups SET last_used = CURRENT_TIMESTAMP WHERE id = ANY($1)",
        )
        .bind(&ids)
        .execute(&mut *conn)
        .await?;
        // The update's save: the after_save callbacks.
        let (username, name): (String, Option<String>) =
            sqlx::query_as("SELECT username_lower, name FROM users WHERE id = $1")
                .bind(user_id)
                .fetch_one(&mut *conn)
                .await?;
        crate::user_updater::after_save(conn, s, user_id, false, &username, name.as_deref())
            .await?;
    }
    if let Some(values) = result.user_field_values.as_ref().filter(|v| !v.is_empty()) {
        for (field_id, value) in values {
            let name = format!("user_field_{field_id}");
            let value = match value {
                Value::String(s) => s.clone(),
                Value::Null => String::new(),
                other => other.to_string(),
            };
            sqlx::query("DELETE FROM user_custom_fields WHERE user_id = $1 AND name = $2")
                .bind(user_id)
                .bind(&name)
                .execute(&mut *conn)
                .await?;
            sqlx::query(
                "INSERT INTO user_custom_fields (user_id, name, value, created_at, updated_at) \
                 VALUES ($1, $2, $3, now(), now())",
            )
            .bind(user_id)
            .bind(&name)
            .bind(&value)
            .execute(&mut *conn)
            .await?;
        }
    }
    Ok(())
}
