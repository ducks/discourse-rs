//! The login providers bundled plugins register (`auth_provider` in their
//! plugin.rb), in the order Discourse loads the plugins: Sign in with
//! Apple, Login with Amazon, LTI, Microsoft, OAuth2 Basic, OpenID Connect
//! and Patreon. Registered whether or not their plugin is enabled, as
//! DiscoursePluginRegistry.auth_providers is.

use crate::auth::{Provider, no_url, yes};
use crate::site_settings::SettingError;
use crate::site_settings::SiteSettings;

fn setting(s: &SiteSettings, name: &str) -> Result<bool, SettingError> {
    Ok(s.get(name)?.truthy())
}

fn amazon_connect(s: &SiteSettings) -> Result<bool, SettingError> {
    setting(s, "login_with_amazon_user_can_connect")
}

fn amazon_revoke(s: &SiteSettings) -> Result<bool, SettingError> {
    setting(s, "login_with_amazon_user_can_revoke")
}

fn oauth2_association_change(s: &SiteSettings) -> Result<bool, SettingError> {
    setting(s, "oauth2_allow_association_change")
}

fn oidc_association_change(s: &SiteSettings) -> Result<bool, SettingError> {
    setting(s, "openid_connect_allow_association_change")
}

fn apple_required(_: &SiteSettings) -> Result<Vec<&'static str>, SettingError> {
    Ok(vec![
        "apple_client_id",
        "apple_team_id",
        "apple_key_id",
        "apple_pem",
    ])
}

fn apple_url(_: &SiteSettings) -> Result<Option<String>, SettingError> {
    Ok(Some("https://account.apple.com".to_string()))
}

fn amazon_required(_: &SiteSettings) -> Result<Vec<&'static str>, SettingError> {
    Ok(vec![
        "login_with_amazon_client_id",
        "login_with_amazon_client_secret",
    ])
}

fn lti_required(_: &SiteSettings) -> Result<Vec<&'static str>, SettingError> {
    Ok(vec![
        "lti_client_ids",
        "lti_authorization_endpoint",
        "lti_platform_issuer_id",
        "lti_platform_public_key",
    ])
}

fn microsoft_required(_: &SiteSettings) -> Result<Vec<&'static str>, SettingError> {
    Ok(vec![
        "microsoft_auth_client_id",
        "microsoft_auth_client_secret",
    ])
}

fn oauth2_required(_: &SiteSettings) -> Result<Vec<&'static str>, SettingError> {
    Ok(vec![
        "oauth2_client_id",
        "oauth2_authorize_url",
        "oauth2_token_url",
    ])
}

/// OpenIDConnectAuthenticator#required_settings: the secret unless PKCE or
/// mTLS client authentication replaces it.
fn oidc_required(s: &SiteSettings) -> Result<Vec<&'static str>, SettingError> {
    let mut out = vec![
        "openid_connect_discovery_document",
        "openid_connect_client_id",
    ];
    let mtls = s
        .get("openid_connect_mtls_client_cert")?
        .presence()
        .is_some()
        && s.get("openid_connect_mtls_client_key")?
            .presence()
            .is_some();
    if !s.get("openid_connect_use_pkce")?.truthy() && !mtls {
        out.push("openid_connect_client_secret");
    }
    Ok(out)
}

fn patreon_required(_: &SiteSettings) -> Result<Vec<&'static str>, SettingError> {
    Ok(vec![
        "patreon_client_id",
        "patreon_client_secret",
        "patreon_creator_discourse_username",
    ])
}

pub const PROVIDERS: [Provider; 7] = [
    Provider {
        name: "apple",
        display_name: None,
        enable_setting: "sign_in_with_apple_enabled",
        required_settings: apple_required,
        can_connect: yes,
        can_revoke: yes,
        provider_url: apple_url,
        icon: Some("fab-apple"),
        frame_width: None,
        frame_height: None,
        title_setting: None,
    },
    Provider {
        name: "amazon",
        display_name: None,
        enable_setting: "enable_login_with_amazon",
        required_settings: amazon_required,
        can_connect: amazon_connect,
        can_revoke: amazon_revoke,
        provider_url: no_url,
        icon: Some("fab-amazon"),
        frame_width: None,
        frame_height: None,
        title_setting: None,
    },
    Provider {
        name: "lti",
        display_name: None,
        enable_setting: "lti_enabled",
        required_settings: lti_required,
        can_connect: yes,
        can_revoke: yes,
        provider_url: no_url,
        icon: None,
        frame_width: None,
        frame_height: None,
        title_setting: None,
    },
    Provider {
        name: "microsoft_office365",
        display_name: None,
        enable_setting: "microsoft_auth_enabled",
        required_settings: microsoft_required,
        can_connect: yes,
        can_revoke: yes,
        provider_url: no_url,
        icon: Some("fab-microsoft"),
        frame_width: None,
        frame_height: None,
        title_setting: None,
    },
    Provider {
        name: "oauth2_basic",
        display_name: None,
        enable_setting: "oauth2_enabled",
        required_settings: oauth2_required,
        can_connect: oauth2_association_change,
        can_revoke: oauth2_association_change,
        provider_url: no_url,
        icon: None,
        frame_width: None,
        frame_height: None,
        title_setting: Some("oauth2_button_title"),
    },
    Provider {
        name: "oidc",
        display_name: None,
        enable_setting: "openid_connect_enabled",
        required_settings: oidc_required,
        can_connect: oidc_association_change,
        can_revoke: oidc_association_change,
        provider_url: no_url,
        icon: None,
        frame_width: None,
        frame_height: None,
        title_setting: None,
    },
    Provider {
        name: "patreon",
        display_name: None,
        enable_setting: "patreon_login_enabled",
        required_settings: patreon_required,
        can_connect: yes,
        can_revoke: yes,
        provider_url: no_url,
        icon: Some("patreon-new"),
        frame_width: None,
        frame_height: None,
        title_setting: None,
    },
];
