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
        && let Some(url) = logo_small_url
    {
        // Discourse.store.cdn_url: unchanged without a CDN.
        if urls.config.globals.cdn_url().is_some() {
            return Err(AvatarError::Unsupported(Unsupported(
                "system avatar behind a CDN",
            )));
        }
        return Ok(url.to_string());
    }
    class_avatar_template(urls, username, uploaded_avatar_id)
}

impl AvatarError {
    /// As an AppError, an Unsupported kept recognizable as one.
    pub fn into_app(self) -> crate::AppError {
        match self {
            AvatarError::Unsupported(u) => crate::AppError::from(u),
            other => crate::AppError::from(other),
        }
    }
}

/// `User#avatar_template` for the user with this id (None when there is
/// none), the site's small logo looked up for the system user.
pub async fn template_for(
    conn: &mut sqlx::PgConnection,
    urls: &Urls<'_>,
    user_id: i32,
) -> Result<Option<String>, crate::AppError> {
    let user: Option<(String, Option<i32>)> =
        sqlx::query_as("SELECT username, uploaded_avatar_id FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(&mut *conn)
            .await?;
    let Some((username, uploaded_avatar_id)) = user else {
        return Ok(None);
    };
    let logo = if user_id == SYSTEM_USER_ID {
        crate::admin_users::logo_small_url(&mut *conn, urls.settings).await?
    } else {
        None
    };
    avatar_template(
        urls,
        user_id,
        &username,
        uploaded_avatar_id,
        logo.as_deref(),
    )
    .map(Some)
    .map_err(AvatarError::into_app)
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

/// `User.default_template(username)`: one of default_avatars (picked by
/// the username's hash) when configured, else the system avatar.
pub(crate) fn default_template(
    settings: &SiteSettings,
    urls: &Urls<'_>,
    username: &str,
) -> Result<String, AvatarError> {
    if let Some(list) = settings.get("default_avatars")?.presence() {
        // split("\n") on a url_list (stored with "|"), as Rails does.
        let mut choices: Vec<&str> = list.split('\n').collect();
        while choices.last() == Some(&"") {
            choices.pop();
        }
        if !choices.is_empty() {
            let index = username_hash(username) % choices.len() as i64;
            return Ok(choices[index as usize].to_string());
        }
    }
    system_avatar_template(urls, username)
}

/// `User.username_hash`: Java's String#hashCode over the characters, in
/// 32 bits, made positive.
fn username_hash(username: &str) -> i64 {
    let hash = username.chars().fold(0i32, |result, c| {
        (result << 5).wrapping_sub(result).wrapping_add(c as i32)
    });
    i64::from(hash).abs()
}

/// `User.normalize_username`: NFC, then downcased.
pub fn normalize_username(username: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    username.nfc().collect::<String>().to_lowercase()
}

/// `User.system_avatar_template(username)`: external_system_avatars_url
/// filled in, or the local letter avatar when it is blank.
pub fn system_avatar_template(urls: &Urls<'_>, username: &str) -> Result<String, AvatarError> {
    let settings = urls.settings;
    let normalized = normalize_username(username);
    let base = urls.config.globals.relative_url_root();
    let Some(template) = settings.get("external_system_avatars_url")?.presence() else {
        return Ok(format!(
            "{base}/letter_avatar/{normalized}/{{size}}/{}.png",
            letter_avatar::version(&urls.config.letter_avatar_font)
        ));
    };
    let url = if template.starts_with("http://") || template.starts_with("https://") {
        template
    } else {
        format!("{base}{template}")
    };
    let restricted = settings.get("restrict_letter_avatar_colors")?.presence();
    let Some(color) = letter_avatar::color(&normalized, restricted.as_deref()) else {
        return Err(AvatarError::Unsupported(Unsupported(
            "restrict_letter_avatar_colors entries that are not six characters (Rails raises)",
        )));
    };
    Ok(url
        .replace("{color}", &color)
        .replace("{username}", &encode_component(username))
        .replace(
            "{first_letter}",
            &encode_component(first_grapheme(&normalized)),
        )
        .replace("{hostname}", &urls.current_hostname()?))
}

/// `grapheme_clusters.first` for a username: its first character and the
/// combining marks after it (usernames hold letters, marks, digits and
/// `._-` only).
fn first_grapheme(s: &str) -> &str {
    let mut chars = s.char_indices();
    let Some(_) = chars.next() else {
        return "";
    };
    let end = chars
        .find(|(_, c)| !unicode_normalization::char::is_combining_mark(*c))
        .map_or(s.len(), |(i, _)| i);
    &s[..end]
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
