//! lib/user_name_suggester.rb: a free username made from a name, an
//! email or a provider's nickname, for users created by DiscourseConnect
//! and external logins. ASCII usernames only (`unicode_usernames` is
//! refused, as signing up refuses it).

use sqlx::PgConnection;

use crate::site_settings::SiteSettings;
use crate::{AppError, Unsupported};

/// `UserNameSuggester::GENERIC_NAMES`
const GENERIC_NAMES: [&str; 11] = [
    "i",
    "me",
    "info",
    "support",
    "admin",
    "webmaster",
    "hello",
    "mail",
    "office",
    "contact",
    "team",
];

/// `UserNameSuggester::LAST_RESORT_USERNAME`
const LAST_RESORT_USERNAME: &str = "user";

/// `UsernameValidator::MAX_CHARS`
const MAX_CHARS: usize = 60;

/// `User::EMAIL` (`/([^@]+)@([^\.]+)/`): the local part and the first
/// label of the domain.
fn email_parts(s: &str) -> Option<(&str, &str)> {
    // The leftmost match: the first "@" with a non-empty part since the
    // previous "@" before it, and a label after it (`[^\.]+`, which may
    // hold "@").
    let mut local_start = 0;
    for (i, _) in s.match_indices('@') {
        let local = &s[local_start..i];
        let after = &s[i + 1..];
        let label = &after[..after.find('.').unwrap_or(after.len())];
        if !local.is_empty() && !label.is_empty() {
            return Some((local, label));
        }
        local_start = i + 1;
    }
    None
}

/// `parse_name_from_email`: walter@white.com is "walter", and
/// me@eviltrout.com "eviltrout".
fn parse_name_from_email(name_or_email: &str) -> String {
    match email_parts(name_or_email) {
        Some((local, domain)) if GENERIC_NAMES.contains(&local) => domain.to_string(),
        Some((local, _)) => local.to_string(),
        None => name_or_email.to_string(),
    }
}

/// `ActiveSupport::Inflector.transliterate`: Latin letters to their ASCII
/// approximations, anything else to "?".
fn transliterate(name: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if c.is_ascii() {
            out.push(c);
            continue;
        }
        let special = match c {
            'Æ' => Some("AE"),
            'æ' => Some("ae"),
            'Ð' | 'Đ' => Some("D"),
            'ð' | 'đ' => Some("d"),
            'Ø' => Some("O"),
            'ø' => Some("o"),
            'Þ' => Some("Th"),
            'þ' => Some("th"),
            'ß' => Some("ss"),
            'Ħ' => Some("H"),
            'ħ' => Some("h"),
            'ı' => Some("i"),
            'Ĳ' => Some("IJ"),
            'ĳ' => Some("ij"),
            'ĸ' => Some("k"),
            'Ŀ' | 'Ł' => Some("L"),
            'ŀ' | 'ł' => Some("l"),
            'ŉ' => Some("'n"),
            'Ŋ' => Some("NG"),
            'ŋ' => Some("ng"),
            'Œ' => Some("OE"),
            'œ' => Some("oe"),
            'Ŧ' => Some("T"),
            'ŧ' => Some("t"),
            '×' => Some("x"),
            _ => None,
        };
        if let Some(s) = special {
            out.push_str(s);
            continue;
        }
        let base: String = c
            .to_string()
            .nfd()
            .filter(|d| !unicode_normalization::char::is_combining_mark(*d))
            .collect();
        if !base.is_empty() && base.is_ascii() {
            out.push_str(&base);
        } else {
            out.push('?');
        }
    }
    out
}

fn alnum(c: char) -> bool {
    c.is_ascii_alphanumeric()
}

/// `sanitize_username` for ASCII usernames.
pub fn sanitize_username(settings: &SiteSettings, name: &str) -> Result<String, AppError> {
    if settings.get("unicode_usernames")?.truthy() {
        return Err(Unsupported("unicode usernames (UserNameSuggester)").into());
    }
    if settings
        .get("allowed_unicode_username_characters")?
        .presence()
        .is_some()
    {
        return Err(Unsupported("allowed_unicode_username_characters (UserNameSuggester)").into());
    }
    // transliterate, then ASCII_INVALID_CHAR_PATTERN (`[^\w.-]`) to "_".
    let mut name: String = transliterate(name)
        .chars()
        .map(|c| {
            if alnum(c) || c == '_' || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    // INVALID_LEADING_CHAR_PATTERN: `\A[^\p{Alnum}\p{M}_]+`
    let lead = name
        .find(|c: char| alnum(c) || c == '_')
        .unwrap_or(name.len());
    name.drain(..lead);
    // CONFUSING_EXTENSIONS: a trailing ".js" (and the like) to "_".
    let lower = name.to_lowercase();
    for ext in [
        "json", "js", "css", "html", "htm", "xml", "jpeg", "jpg", "png", "gif", "bmp", "ico",
        "tiff", "tif", "woff",
    ] {
        let suffix = format!(".{ext}");
        if lower.ends_with(&suffix) {
            name.truncate(name.len() - suffix.len());
            name.push('_');
            break;
        }
    }
    // INVALID_TRAILING_CHAR_PATTERN: `[^\p{Alnum}\p{M}]+\z`
    while name.ends_with(|c: char| !alnum(c)) {
        name.pop();
    }
    // REPEATED_SPECIAL_CHAR_PATTERN: `[-_.]{2,}` to "_".
    let mut out = String::with_capacity(name.len());
    let mut run = 0;
    for c in name.chars() {
        if "-_.".contains(c) {
            run += 1;
            if run == 1 {
                out.push(c);
            } else if run == 2 {
                out.pop();
                out.push('_');
            }
        } else {
            run = 0;
            out.push(c);
        }
    }
    Ok(out)
}

/// `truncate(name, max_grapheme_clusters)`, for ASCII.
fn truncate(name: &str, max: usize) -> String {
    name.chars().take(max.min(MAX_CHARS)).collect()
}

/// `rightsize_username`
fn rightsize_username(settings: &SiteSettings, name: &str) -> Result<String, AppError> {
    let max = settings.get("max_username_length")?.to_i() as usize;
    let min = settings.get("min_username_length")?.to_i() as usize;
    let mut name = truncate(name, max);
    while name.ends_with(|c: char| !alnum(c)) {
        name.pop();
    }
    while name.chars().count() < min {
        name.push('1');
    }
    Ok(name)
}

/// `fix_username`
pub fn fix_username(
    settings: &SiteSettings,
    i18n: &crate::i18n::I18n,
    name: Option<&str>,
    allow_generic_fallback: bool,
) -> Result<Option<String>, AppError> {
    let mut fixed = sanitize_username(settings, name.unwrap_or(""))?;
    if fixed.is_empty() {
        if !allow_generic_fallback {
            return Ok(None);
        }
        fixed.push_str(&sanitize_username(
            settings,
            i18n.t("fallback_username").unwrap_or(LAST_RESORT_USERNAME),
        )?);
        if fixed.is_empty() {
            fixed.push_str(LAST_RESORT_USERNAME);
        }
    }
    Ok(Some(rightsize_username(settings, &fixed)?))
}

/// `UserNameSuggester.suggest(*input, current_username:,
/// allow_generic_fallback:)`
pub async fn suggest(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    i18n: &crate::i18n::I18n,
    input: &[Option<&str>],
    current_username: Option<&str>,
    allow_generic_fallback: bool,
) -> Result<Option<String>, AppError> {
    let use_email = settings
        .get("use_email_for_username_and_name_suggestions")?
        .truthy();
    let mut name: Option<String> = None;
    for item in input {
        let item = item.unwrap_or("");
        if !use_email && email_parts(item).is_some() {
            continue;
        }
        let parsed = parse_name_from_email(item);
        if !sanitize_username(settings, &parsed)?.is_empty() {
            name = Some(parsed);
            break;
        }
    }
    let Some(name) = fix_username(settings, i18n, name.as_deref(), allow_generic_fallback)? else {
        return Ok(None);
    };
    if name.is_empty() {
        return Ok(None);
    }
    Ok(Some(
        find_available_username_based_on(conn, settings, &name, current_username).await?,
    ))
}

/// `User.username_available?(name)`: not reserved, no user or group has it.
async fn username_available(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    name: &str,
) -> Result<bool, AppError> {
    let lower = crate::accounts::normalize_username(name)?;
    if crate::signup::reserved_by_settings(settings, &lower)? {
        return Ok(false);
    }
    let taken: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM users WHERE username_lower = $1) \
         OR EXISTS (SELECT 1 FROM groups WHERE lower(name) = $1)",
    )
    .bind(&lower)
    .fetch_one(&mut *conn)
    .await?;
    Ok(!taken)
}

/// `find_available_username_based_on`
pub async fn find_available_username_based_on(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    name: &str,
    current_username: Option<&str>,
) -> Result<String, AppError> {
    let max = settings.get("max_username_length")?.to_i() as usize;
    let current = match current_username {
        Some(c) => Some(crate::accounts::normalize_username(c)?),
        None => None,
    };
    let mut offset: Option<i64> = None;
    let mut i = 1i64;
    let mut attempt = name.to_string();
    let mut normalized_attempt = crate::accounts::normalize_username(&attempt)?;
    while !(current.as_deref() == Some(normalized_attempt.as_str())
        || username_available(conn, settings, &attempt).await?
        || i > 100)
    {
        let offset = match offset {
            Some(o) => o,
            None => {
                let normalized = crate::accounts::normalize_username(name)?;
                let count: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM users WHERE username_lower LIKE $1 AND username_lower SIMILAR TO $2",
                )
                .bind(format!("{normalized}%"))
                .bind(format!("{normalized}(0|1|2|3|4|5|6|7|8|9)+"))
                .fetch_one(&mut *conn)
                .await?;
                let o = if count > 0 {
                    let available: Option<i32> = sqlx::query_scalar(
                        "WITH numbers AS (SELECT generate_series(1, $1) AS n) \
                         SELECT n FROM numbers LEFT JOIN users ON (username_lower = $2 || n::varchar) \
                         AND (username_lower <> $3) WHERE users.id IS NULL ORDER BY n ASC LIMIT 1",
                    )
                    .bind((count + 10) as i32)
                    .bind(&normalized)
                    .bind(current.as_deref().unwrap_or(""))
                    .fetch_optional(&mut *conn)
                    .await?;
                    (i64::from(available.unwrap_or(0)) - 1).max(0)
                } else {
                    0
                };
                offset = Some(o);
                o
            }
        };
        let suffix = (i + offset).to_string();
        let keep = max.saturating_sub(suffix.len());
        attempt = format!("{}{suffix}", truncate(name, keep));
        normalized_attempt = crate::accounts::normalize_username(&attempt)?;
        i += 1;
    }
    if !(current.as_deref() == Some(normalized_attempt.as_str())
        || username_available(conn, settings, &attempt).await?)
    {
        return Err(Unsupported("random username suggestions").into());
    }
    if current.as_deref() == Some(normalized_attempt.as_str()) {
        return Ok(current_username.unwrap_or_default().to_string());
    }
    Ok(attempt)
}

/// `User.suggest_name(string)`: the part before "@" (or after it), dots
/// as spaces, titleized.
pub fn suggest_name(string: &str) -> String {
    if string.trim().is_empty() {
        return String::new();
    }
    let part = match string.split_once('@') {
        Some((before, _)) if !before.is_empty() => before.to_string(),
        Some(_) => string.rsplit('@').next().unwrap_or("").to_string(),
        None => string.to_string(),
    };
    titleize(&part.replace('.', " "))
}

/// ActiveSupport's `titleize`: `humanize(underscore(s))`, then each
/// lowercase letter starting a word capitalized unless a word character
/// and an apostrophe or parenthesis come before it.
fn titleize(s: &str) -> String {
    // underscore: "::" to "/", CamelCase to snake_case, "-" to "_".
    let chars: Vec<char> = s.replace("::", "/").chars().collect();
    let mut under = String::with_capacity(s.len() + 4);
    for (i, &c) in chars.iter().enumerate() {
        if c.is_ascii_uppercase() && i > 0 {
            let prev = chars[i - 1];
            let next_lower = chars.get(i + 1).is_some_and(|n| n.is_ascii_lowercase());
            if prev.is_ascii_lowercase()
                || prev.is_ascii_digit()
                || (prev.is_ascii_uppercase() && next_lower)
            {
                under.push('_');
            }
        }
        under.push(if c == '-' { '_' } else { c });
    }
    let under = under.to_lowercase();
    // humanize: a leading "_" and a trailing "_id" dropped, "_" to " ",
    // the first letter capitalized.
    let h = under.trim_start_matches('_');
    let h = h.strip_suffix("_id").unwrap_or(h);
    let h = h.replace('_', " ");
    let word = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let chars: Vec<char> = h.chars().collect();
    let mut out = String::with_capacity(h.len());
    for (i, &c) in chars.iter().enumerate() {
        let boundary = i == 0 || !word(chars[i - 1]);
        let guarded = i >= 2
            && word(chars[i - 2])
            && ['\'', '\u{2019}', '`', '(', ')'].contains(&chars[i - 1]);
        if c.is_ascii_lowercase() && boundary && !guarded {
            out.push(c.to_ascii_uppercase());
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_from_emails() {
        assert_eq!(parse_name_from_email("walter@white.com"), "walter");
        assert_eq!(parse_name_from_email("me@eviltrout.com"), "eviltrout");
        assert_eq!(parse_name_from_email("Walter White"), "Walter White");
    }

    #[test]
    fn suggested_names() {
        assert_eq!(suggest_name("walter.white@example.com"), "Walter White");
        assert_eq!(suggest_name("sam"), "Sam");
        assert_eq!(suggest_name(""), "");
    }

    #[test]
    fn transliterates_latin() {
        assert_eq!(transliterate("Jösé Ærø"), "Jose AEro");
        assert_eq!(transliterate("日本"), "??");
    }
}
