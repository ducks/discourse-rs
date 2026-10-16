//! External logins: `Discourse.auth_providers` (lib/discourse.rb's
//! BUILTIN_AUTH, then the providers bundled plugins register), each with
//! its authenticator (lib/auth/*_authenticator.rb): the setting that turns
//! it on, the settings it needs, whether users may connect and revoke it,
//! and what /site.json's AuthProviderSerializer shows of it.
//!
//! Every authenticator here is a ManagedAuthenticator (its accounts are
//! `user_associated_accounts` rows).

use serde_json::{Value, json};

pub mod builtin;
pub mod managed;
pub mod oauth2;
use sqlx::PgConnection;

use crate::AppError;
use crate::site_settings::SettingError;
use crate::site_settings::SiteSettings;

/// An authenticator's answer that depends on settings.
pub type Check = fn(&SiteSettings) -> Result<bool, SettingError>;

/// An `Auth::AuthProvider` with its authenticator.
pub struct Provider {
    /// `authenticator.name`
    pub name: &'static str,
    /// `display_name`: the name in messages (the name when not set).
    pub display_name: Option<&'static str>,
    /// `enable_setting`
    pub enable_setting: &'static str,
    /// `required_settings`
    pub required_settings: fn(&SiteSettings) -> Result<Vec<&'static str>, SettingError>,
    /// `can_connect_existing_user?`
    pub can_connect: Check,
    /// `can_revoke?`
    pub can_revoke: Check,
    /// `provider_url`
    pub provider_url: fn(&SiteSettings) -> Result<Option<String>, SettingError>,
    pub icon: Option<&'static str>,
    pub frame_width: Option<i64>,
    pub frame_height: Option<i64>,
    pub title_setting: Option<&'static str>,
}

pub fn yes(_: &SiteSettings) -> Result<bool, SettingError> {
    Ok(true)
}

pub fn no_url(_: &SiteSettings) -> Result<Option<String>, SettingError> {
    Ok(None)
}

fn discourse_id_url(s: &SiteSettings) -> Result<Option<String>, SettingError> {
    Ok(Some(
        s.get("discourse_id_provider_url")?
            .presence()
            .unwrap_or_else(|| "https://id.discourse.com".to_string()),
    ))
}

macro_rules! required {
    ($($s:literal),*) => {{
        fn f(_: &SiteSettings) -> Result<Vec<&'static str>, SettingError> {
            Ok(vec![$($s),*])
        }
        f
    }};
}

macro_rules! url {
    ($u:literal) => {{
        fn f(_: &SiteSettings) -> Result<Option<String>, SettingError> {
            Ok(Some($u.to_string()))
        }
        f
    }};
}

/// lib/discourse.rb's BUILTIN_AUTH, in its order.
pub const BUILTIN: [Provider; 7] = [
    Provider {
        name: "discourse_id",
        display_name: Some("Discourse ID"),
        enable_setting: "enable_discourse_id",
        required_settings: required!("discourse_id_client_id", "discourse_id_client_secret"),
        can_connect: yes,
        can_revoke: yes,
        provider_url: discourse_id_url,
        icon: Some("fab-discourse"),
        frame_width: None,
        frame_height: None,
        title_setting: None,
    },
    Provider {
        name: "facebook",
        display_name: Some("Facebook"),
        enable_setting: "enable_facebook_logins",
        required_settings: required!("facebook_app_id", "facebook_app_secret"),
        can_connect: yes,
        can_revoke: yes,
        provider_url: url!("https://www.facebook.com"),
        icon: Some("fab-facebook"),
        frame_width: Some(580),
        frame_height: Some(400),
        title_setting: None,
    },
    Provider {
        name: "google_oauth2",
        display_name: Some("Google"),
        enable_setting: "enable_google_oauth2_logins",
        required_settings: required!("google_oauth2_client_id", "google_oauth2_client_secret"),
        can_connect: yes,
        can_revoke: yes,
        provider_url: url!("https://accounts.google.com"),
        icon: None,
        frame_width: Some(850),
        frame_height: Some(500),
        title_setting: None,
    },
    Provider {
        name: "github",
        display_name: Some("GitHub"),
        enable_setting: "enable_github_logins",
        required_settings: required!("github_client_id", "github_client_secret"),
        can_connect: yes,
        can_revoke: yes,
        provider_url: url!("https://github.com"),
        icon: Some("fab-github"),
        frame_width: None,
        frame_height: None,
        title_setting: None,
    },
    Provider {
        name: "twitter",
        display_name: Some("X / Twitter"),
        enable_setting: "enable_twitter_logins",
        required_settings: required!("twitter_consumer_key", "twitter_consumer_secret"),
        can_connect: yes,
        can_revoke: yes,
        provider_url: url!("https://x.com"),
        icon: Some("fab-x-twitter"),
        frame_width: None,
        frame_height: None,
        title_setting: None,
    },
    Provider {
        name: "discord",
        display_name: Some("Discord"),
        enable_setting: "enable_discord_logins",
        required_settings: required!("discord_client_id", "discord_secret"),
        can_connect: yes,
        can_revoke: yes,
        provider_url: url!("https://discord.com"),
        icon: Some("fab-discord"),
        frame_width: None,
        frame_height: None,
        title_setting: None,
    },
    Provider {
        name: "linkedin_oidc",
        display_name: Some("LinkedIn"),
        enable_setting: "enable_linkedin_oidc_logins",
        required_settings: required!("linkedin_oidc_client_id", "linkedin_oidc_client_secret"),
        can_connect: yes,
        can_revoke: yes,
        provider_url: url!("https://www.linkedin.com"),
        icon: Some("fab-linkedin-in"),
        frame_width: None,
        frame_height: None,
        title_setting: None,
    },
];

/// `Discourse.auth_providers`: the builtin ones, then the plugins'.
pub fn providers() -> impl Iterator<Item = &'static Provider> {
    BUILTIN
        .iter()
        .chain(crate::plugins::auth_providers::PROVIDERS.iter())
}

pub fn find(name: &str) -> Option<&'static Provider> {
    providers().find(|p| p.name == name)
}

impl Provider {
    /// `missing_settings`: the required settings that are blank.
    pub fn missing_settings(&self, s: &SiteSettings) -> Result<Vec<&'static str>, SettingError> {
        let mut out = Vec::new();
        for name in (self.required_settings)(s)? {
            if s.get(name)?.presence().is_none() {
                out.push(name);
            }
        }
        Ok(out)
    }

    /// `enabled?`: the setting on and nothing it needs missing.
    pub fn enabled(&self, s: &SiteSettings) -> Result<bool, SettingError> {
        Ok(s.get(self.enable_setting)?.truthy() && self.missing_settings(s)?.is_empty())
    }

    pub fn display_name(&self) -> &'static str {
        self.display_name.unwrap_or(self.name)
    }

    /// AuthProviderSerializer
    pub fn serialize(&self, s: &SiteSettings) -> Result<Value, SettingError> {
        let title = match self.title_setting {
            Some(setting) => json!(s.get(setting)?.to_s()),
            None => Value::Null,
        };
        Ok(json!({
            "can_connect": (self.can_connect)(s)?,
            "can_revoke": (self.can_revoke)(s)?,
            "custom_url": null,
            "frame_height": self.frame_height,
            "frame_width": self.frame_width,
            "icon_override": self.icon,
            "name": self.name,
            "pretty_name_override": null,
            "provider_url": (self.provider_url)(s)?,
            "title_override": title,
        }))
    }
}

/// `Discourse.enabled_authenticators`, in order.
pub fn enabled(s: &SiteSettings) -> Result<Vec<&'static Provider>, SettingError> {
    let mut out = Vec::new();
    for p in providers() {
        if p.enabled(s)? {
            out.push(p);
        }
    }
    Ok(out)
}

/// An enabled authenticator by name that keeps its accounts as
/// UserAssociatedAccount rows (`is_managed?`: all of them).
pub fn enabled_managed(s: &SiteSettings, name: &str) -> Result<bool, SettingError> {
    Ok(match find(name) {
        Some(p) => p.enabled(s)?,
        None => false,
    })
}

/// `User#associated_accounts`: each authenticator's description of the
/// user's account with it (ManagedAuthenticator#description_for_user),
/// for the ones they have.
pub async fn associated_accounts(
    conn: &mut PgConnection,
    i18n: &crate::i18n::I18n,
    user_id: i32,
) -> Result<Value, AppError> {
    let rows: Vec<(String, Value)> = sqlx::query_as(
        "SELECT DISTINCT ON (provider_name) provider_name, info FROM user_associated_accounts \
         WHERE user_id = $1 ORDER BY provider_name, id",
    )
    .bind(user_id)
    .fetch_all(&mut *conn)
    .await?;
    let mut out = Vec::new();
    for p in providers() {
        let Some((_, info)) = rows.iter().find(|(name, _)| name == p.name) else {
            continue;
        };
        let description = description(i18n, info);
        if !description.is_empty() {
            out.push(json!({"name": p.name, "description": description}));
        }
    }
    Ok(Value::Array(out))
}

/// `description_for_auth_hash(info) || I18n.t("associated_accounts.connected")`:
/// the info's email, nickname or name, the first that is set.
pub fn description(i18n: &crate::i18n::I18n, info: &Value) -> String {
    let field = |k: &str| match &info[k] {
        Value::Null | Value::Bool(false) => None,
        Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    };
    if info.is_null() {
        return i18n
            .t("associated_accounts.connected")
            .unwrap_or("(connected)")
            .to_string();
    }
    field("email")
        .or_else(|| field("nickname"))
        .or_else(|| field("name"))
        .unwrap_or_else(|| {
            i18n.t("associated_accounts.connected")
                .unwrap_or("(connected)")
                .to_string()
        })
}
