//! Port of User#avatar_template and friends (app/models/user.rb,
//! app/models/user_avatar.rb).

use crate::Unsupported;
use crate::letter_avatar;
use crate::site_settings::{SettingError, SiteSettings};
use crate::url::{UrlError, Urls};

/// `Discourse::SYSTEM_USER_ID`
pub const SYSTEM_USER_ID: i32 = -1;

/// `OptimizedImage::VERSION`
const OPTIMIZED_IMAGE_VERSION: i32 = 2;

#[derive(Debug)]
pub enum AvatarError {
    Setting(SettingError),
    Url(UrlError),
    Unsupported(Unsupported),
}

impl std::fmt::Display for AvatarError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AvatarError::Setting(e) => e.fmt(f),
            AvatarError::Url(e) => e.fmt(f),
            AvatarError::Unsupported(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for AvatarError {}

impl From<SettingError> for AvatarError {
    fn from(e: SettingError) -> Self {
        AvatarError::Setting(e)
    }
}

impl From<UrlError> for AvatarError {
    fn from(e: UrlError) -> Self {
        AvatarError::Url(e)
    }
}

/// `User#avatar_template`. `logo_small_url` is the URL of the logo_small
/// upload (caller-resolved, since it needs the database), used for the
/// system user when use_site_small_logo_as_system_avatar is on.
pub fn avatar_template(
    urls: &Urls<'_>,
    user_id: i32,
    username: &str,
    uploaded_avatar_id: Option<i32>,
    logo_small_url: Option<&str>,
) -> Result<String, AvatarError> {
    let settings = urls.settings;
    if user_id == SYSTEM_USER_ID
        && settings
            .get("use_site_small_logo_as_system_avatar")?
            .truthy()
        && settings.get("logo_small")?.to_i() != 0
    {
        if let Some(url) = logo_small_url {
            // Discourse.store.cdn_url: unchanged without a CDN.
            if urls.config.globals.cdn_url().is_some() {
                return Err(AvatarError::Unsupported(Unsupported(
                    "system avatar behind a CDN",
                )));
            }
            return Ok(url.to_string());
        }
    }
    class_avatar_template(urls, username, uploaded_avatar_id)
}

/// `User.avatar_template(username, upload_id)`, the class method: the
/// uploaded avatar or the default template, with no system-user special
/// case. BasicUserSerializer uses it for hash-wrapped users (participants).
pub fn class_avatar_template(
    urls: &Urls<'_>,
    username: &str,
    uploaded_avatar_id: Option<i32>,
) -> Result<String, AvatarError> {
    match uploaded_avatar_id {
        Some(upload_id) => Ok(uploaded_avatar_template(urls, username, upload_id)?),
        None => default_template(urls.settings, urls, username),
    }
}

/// `User.avatar_template(username, upload_id)` -> UserAvatar.local_avatar_template
fn uploaded_avatar_template(
    urls: &Urls<'_>,
    username: &str,
    upload_id: i32,
) -> Result<String, AvatarError> {
    Ok(format!(
        "{}/user_avatar/{}/{}/{{size}}/{}_{}.png",
        urls.config.globals.relative_url_root(),
        urls.current_hostname()?,
        username.to_lowercase(),
        upload_id,
        OPTIMIZED_IMAGE_VERSION
    ))
}

/// `User.default_template(username)`: the letter avatar unless
/// default_avatars is configured.
fn default_template(
    settings: &SiteSettings,
    urls: &Urls<'_>,
    username: &str,
) -> Result<String, AvatarError> {
    if settings.get("default_avatars")?.presence().is_some() {
        return Err(AvatarError::Unsupported(Unsupported(
            "default_avatars setting",
        )));
    }
    if settings
        .get("restrict_letter_avatar_colors")?
        .presence()
        .is_some()
    {
        return Err(AvatarError::Unsupported(Unsupported(
            "restrict_letter_avatar_colors",
        )));
    }
    // User.system_avatar_template
    let normalized = username.to_lowercase();
    let first_letter = normalized
        .chars()
        .next()
        .map(|c| c.to_string())
        .unwrap_or_default();
    let Some(template) = settings.get("external_system_avatars_url")?.presence() else {
        return Err(AvatarError::Unsupported(Unsupported(
            "local letter avatars (LetterAvatar)",
        )));
    };
    let url = template
        .replace("{color}", &letter_avatar::color(&normalized))
        .replace("{username}", &encode_component(username))
        .replace("{first_letter}", &encode_component(&first_letter))
        .replace("{hostname}", &urls.current_hostname()?);
    Ok(
        if url.starts_with("http://") || url.starts_with("https://") {
            url
        } else {
            format!("{}{url}", urls.config.globals.relative_url_root())
        },
    )
}

/// `UrlHelper.encode_component` = Addressable::URI.encode_component with its
/// default class: reserved and unreserved characters pass through.
fn encode_component(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric()
            || matches!(b, b'-' | b'_' | b'.' | b'~')
            || b"!*'();:@&=+$,/?#[]".contains(&b)
        {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_components() {
        assert_eq!(encode_component("a"), "a");
        assert_eq!(encode_component("é"), "%C3%A9");
    }
}
