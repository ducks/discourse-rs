//! discourse-captcha: hCaptcha or reCAPTCHA on local signups. The browser
//! solves the provider's widget on the create-account form and hands its
//! token to POST /captcha/:provider/create, which keeps it in the server
//! session for two minutes; users#create (CreateUsersControllerPatch's
//! `check_captcha`, after the honeypot) takes it back out and asks the
//! provider's siteverify before the account is made, refusing the signup
//! with `fail_with` otherwise. Two problem checks report a selected
//! provider without its keys.
//!
//! Not ported: SessionControllerPatch's gate on login code signups, as
//! the port's login codes only redeem invites (which it never gates).

use serde_json::{Value, json};
use sqlx::PgConnection;

use crate::auth::oauth2::Http;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

pub const PLUGIN_NAME: &str = "discourse-captcha";

/// `CaptchaController::TOKEN_TTL`
pub const TOKEN_TTL: i64 = 120;
/// Net::HTTP's open and read timeouts.
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// The captcha providers (`CaptchaProvider::HCAPTCHA`, `RECAPTCHA`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Hcaptcha,
    Recaptcha,
}

impl Provider {
    /// `discourse_captcha_provider`'s value, and the route's segment.
    pub fn name(self) -> &'static str {
        match self {
            Provider::Hcaptcha => "hcaptcha",
            Provider::Recaptcha => "recaptcha",
        }
    }

    pub fn from_name(name: &str) -> Option<Provider> {
        match name {
            "hcaptcha" => Some(Provider::Hcaptcha),
            "recaptcha" => Some(Provider::Recaptcha),
            _ => None,
        }
    }

    /// The controller's `token_key`, the server session key.
    pub fn token_key(self) -> &'static str {
        match self {
            Provider::Hcaptcha => "hcaptcha_token",
            Provider::Recaptcha => "recaptcha_token",
        }
    }

    /// `CAPTCHA_VERIFICATION_URL`
    pub fn verification_url(self) -> &'static str {
        match self {
            Provider::Hcaptcha => "https://hcaptcha.com/siteverify",
            Provider::Recaptcha => "https://www.google.com/recaptcha/api/siteverify",
        }
    }

    fn site_key_setting(self) -> &'static str {
        match self {
            Provider::Hcaptcha => "hcaptcha_site_key",
            Provider::Recaptcha => "recaptcha_site_key",
        }
    }

    fn secret_key_setting(self) -> &'static str {
        match self {
            Provider::Hcaptcha => "hcaptcha_secret_key",
            Provider::Recaptcha => "recaptcha_secret_key",
        }
    }

    /// Both keys set (`site_key.present? && secret_key.present?`).
    fn configured(self, s: &SiteSettings) -> Result<bool, AppError> {
        Ok(s.get(self.site_key_setting())?.presence().is_some()
            && s.get(self.secret_key_setting())?.presence().is_some())
    }
}

/// The enabled setting.
pub fn enabled(s: &SiteSettings) -> Result<bool, AppError> {
    Ok(s.get("discourse_captcha_enabled")?.truthy())
}

/// The selected provider; none for `none`.
pub fn selected(s: &SiteSettings) -> Result<Option<Provider>, AppError> {
    Ok(Provider::from_name(
        &s.get("discourse_captcha_provider")?.to_s(),
    ))
}

/// The plugin on with a provider selected (`captcha_enabled?`, the
/// client's `captchaSelector`).
pub fn active(s: &SiteSettings) -> Result<Option<Provider>, AppError> {
    if !enabled(s)? {
        return Ok(None);
    }
    selected(s)
}

fn http<'a>(state: &'a AppState, settings: &'a SiteSettings) -> Http<'a> {
    Http {
        fetcher: crate::final_destination::Fetcher {
            settings,
            urls: crate::url::Urls {
                config: &state.config,
                settings,
            },
        },
        user_agent: crate::schema::user_agent(),
    }
}

/// `fetch_captcha_token`: the kept token, taken out of the server session.
async fn fetch_token(
    conn: &mut PgConnection,
    session_id: &str,
    provider: Provider,
) -> Result<Option<String>, AppError> {
    let token = crate::session::server::get(&mut *conn, session_id, provider.token_key()).await?;
    crate::session::server::delete(&mut *conn, session_id, provider.token_key()).await?;
    Ok(match token {
        Some(Value::String(s)) => Some(s),
        Some(Value::Null) | None => None,
        Some(other) => Some(other.to_string()),
    })
}

/// `send_verification`: the secret and the token posted as a form, through
/// FinalDestination's SSRF check. Whether the provider accepted it
/// (`validate_captcha_response`: below 500, JSON, `success` neither nil
/// nor false); every failure is a log line, as Rails rescues them.
async fn verify(
    http: &Http<'_>,
    s: &SiteSettings,
    provider: Provider,
    token: &str,
) -> Result<bool, AppError> {
    let body = form_urlencoded::Serializer::new(String::new())
        .append_pair("secret", &s.get(provider.secret_key_setting())?.to_s())
        .append_pair("response", token)
        .finish();
    let fetched = http
        .request(
            true,
            TIMEOUT,
            reqwest::Method::POST,
            provider.verification_url(),
            &[(
                "Content-Type".into(),
                "application/x-www-form-urlencoded".into(),
            )],
            Some(body),
        )
        .await?;
    let fetched = match fetched {
        Ok(f) => f,
        Err(f) => {
            tracing::warn!("Captcha verification error: {} - {}", f.kind, f.message);
            return Ok(false);
        }
    };
    if fetched.status >= 500 {
        tracing::warn!("Captcha verification error: Discourse::InvalidAccess - status");
        return Ok(false);
    }
    let Ok(parsed) = serde_json::from_str::<Value>(&fetched.body) else {
        tracing::warn!("Captcha verification error: JSON::ParserError");
        return Ok(false);
    };
    let accepted = !matches!(
        parsed.get("success"),
        None | Some(Value::Null) | Some(Value::Bool(false))
    );
    if !accepted {
        tracing::warn!("Captcha verification error: Discourse::InvalidAccess");
    }
    Ok(accepted)
}

/// `captcha_verification_error`: none while the plugin is off or no
/// provider is selected, else the locale key of the refusal.
pub async fn verification_error(
    conn: &mut PgConnection,
    state: &AppState,
    s: &SiteSettings,
    session_id: &str,
) -> Result<Option<&'static str>, AppError> {
    let Some(provider) = active(s)? else {
        return Ok(None);
    };
    if !provider.configured(s)? {
        return Ok(Some("captcha_not_configured"));
    }
    let token = fetch_token(conn, session_id, provider).await?;
    let Some(token) = token.filter(|t| !crate::ruby::is_blank(t)) else {
        return Ok(Some("captcha_verification_failed"));
    };
    if !verify(&http(state, s), s, provider, &token).await? {
        return Ok(Some("captcha_verification_failed"));
    }
    Ok(None)
}

/// What POST /captcha/:provider/create answers, after CSRF.
pub enum Created {
    /// requires_plugin, or another provider selected: a 404.
    NotFound,
    /// No token: Discourse::InvalidParameters.new(:token).
    MissingToken,
    Kept,
}

/// CaptchaController#create: `ensure_config`, then the token kept.
pub async fn create(
    conn: &mut PgConnection,
    s: &SiteSettings,
    session_id: &str,
    route: Provider,
    token: Option<&str>,
) -> Result<Created, AppError> {
    if !enabled(s)? || selected(s)? != Some(route) {
        return Ok(Created::NotFound);
    }
    let Some(token) = token.filter(|t| !crate::ruby::is_blank(t)) else {
        return Ok(Created::MissingToken);
    };
    crate::session::server::set(
        conn,
        session_id,
        route.token_key(),
        &json!(token),
        TOKEN_TTL,
    )
    .await?;
    Ok(Created::Kept)
}

/// ProblemCheck::HcaptchaConfiguration and RecaptchaConfiguration, both
/// realtime and high priority.
pub const PROBLEM_CHECKS: [crate::problem_checks::Check; 2] = [
    crate::problem_checks::Check::new("hcaptcha_configuration", "high"),
    crate::problem_checks::Check::new("recaptcha_configuration", "high"),
];

/// A check's `call`: a problem when the plugin is on with the check's
/// provider selected and its keys missing.
pub fn problem_check(s: &SiteSettings, identifier: &str) -> Result<bool, AppError> {
    let provider = match identifier {
        "hcaptcha_configuration" => Provider::Hcaptcha,
        _ => Provider::Recaptcha,
    };
    Ok(enabled(s)? && selected(s)? == Some(provider) && !provider.configured(s)?)
}

/// The create-account-after-user-fields connector: the selected
/// provider's widget, loading until static/js/discourse-captcha.js has
/// the provider's script (BaseCaptcha's DAsyncContent). Empty while the
/// plugin is off or no provider is selected.
pub fn signup_connector(s: &SiteSettings, t: &dyn Fn(&str) -> String) -> Result<String, AppError> {
    let Some(provider) = active(s)? else {
        return Ok(String::new());
    };
    let strings = json!({
        "missing_token": t("discourse_captcha.missing_token"),
        "verification_failed": t("discourse_captcha.verification_failed"),
        "contact_system_administrator": t("discourse_captcha.contact_system_administrator"),
    });
    let esc = crate::topic_list_view::escape;
    Ok(format!(
        "<div class=\"create-account-after-user-fields-outlet captcha-fields-connector\" data-provider=\"{}\" data-site-key=\"{}\" data-strings=\"{}\"><div class=\"input-group\"><div class=\"captcha-container captcha-loading\">{}</div></div></div>",
        provider.name(),
        esc(&s.get(provider.site_key_setting())?.to_s()),
        esc(&strings.to_string()),
        esc(&t("loading")),
    ))
}

/// The signup page head's script for the connector.
pub fn signup_script(s: &SiteSettings, base_path: &str) -> Result<String, AppError> {
    if active(s)?.is_none() {
        return Ok(String::new());
    }
    Ok(format!(
        "<script src=\"{}\" defer></script>",
        crate::assets::url(base_path, "discourse-captcha.js")
    ))
}
