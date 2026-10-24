//! The list setting types' part of `SiteSetting::Update` (list, url_list,
//! host_list, value_list, simple_list, emoji_list, category_list,
//! group_list, tag_list): `add_override!`'s mandatory values and
//! disallowed groups, and the validator classes site_settings.yml names
//! for these settings (lib/validators, and the bundled plugins' the
//! port has).

use sqlx::PgConnection;

use crate::AppError;
use crate::i18n::I18n;
use crate::site_settings::{DataType, Definition, SiteSettings};

/// The types of this family.
pub fn is_list(data_type: DataType) -> bool {
    matches!(
        data_type,
        DataType::List
            | DataType::UrlList
            | DataType::HostList
            | DataType::ValueList
            | DataType::SimpleList
            | DataType::EmojiList
            | DataType::CategoryList
            | DataType::GroupList
            | DataType::TagList
    )
}

/// `Array#|`: the items of both, first occurrences kept in order.
fn union(a: &[&str], b: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for item in a.iter().chain(b) {
        if !out.iter().any(|o| o == item) {
            out.push(item.to_string());
        }
    }
    out
}

/// `add_override!`'s `mandatory_values` (always kept, first) and
/// `disallowed_groups` (always dropped) applied to the value saved.
pub fn apply_mandatory_and_disallowed(def: &Definition, value: &str) -> String {
    let mut value = value.to_string();
    if let Some(mandatory) = &def.mandatory_values {
        let mandatory = crate::ruby::split(mandatory, "|");
        value = union(&mandatory, &crate::ruby::split(&value, "|")).join("|");
    }
    if let Some(disallowed) = disallowed_groups(def) {
        let disallowed = crate::ruby::split(&disallowed, "|");
        value = crate::ruby::split(&value, "|")
            .into_iter()
            .filter(|v| !disallowed.contains(v))
            .collect::<Vec<_>>()
            .join("|");
    }
    value
}

/// The getter's value: `mandatory_values` merged in front.
pub fn with_mandatory(def: &Definition, value: &str) -> String {
    match &def.mandatory_values {
        Some(mandatory) => union(
            &crate::ruby::split(mandatory, "|"),
            &crate::ruby::split(value, "|"),
        )
        .join("|"),
        None => value.to_string(),
    }
}

fn disallowed_groups(def: &Definition) -> Option<String> {
    let v = def
        .options
        .as_ref()?
        .get(serde_yaml_ng::Value::String("disallowed_groups".into()))?;
    match v {
        serde_yaml_ng::Value::String(s) => Some(s.clone()),
        serde_yaml_ng::Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// The validator classes ported for these settings.
pub const VALIDATORS: &[&str] = &[
    "AllowedIframesValidator",
    "AtLeastOneGroupValidator",
    "ColorListValidator",
    "CspScriptSrcValidator",
    "EmojiPickerPinnedGroupsValidator",
    "MarkdownLinkifyTldsValidator",
    "MarkdownTypographerQuotationMarksValidator",
    "RandomUsernameWordListValidator",
    "RegexpListValidator",
    "TopMenuValidator",
    "Chat::DirectMessageEnabledGroupsValidator",
    "ReactionsExcludedFromLikeSiteSettingValidator",
];

/// `HostListSettingValidator#valid_value?`
pub fn host_list_valid(value: &str) -> bool {
    !value.contains('*') && !value.contains('?')
}

/// `<validator>.new(opts).valid_value?(value)`, Some(error_message) when
/// it refuses the value.
pub async fn validate(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    defs: &crate::site_settings::Definitions,
    i18n: &I18n,
    validator: &str,
    value: &str,
) -> Result<Option<String>, AppError> {
    let t = |key: &str| i18n.t(key).unwrap_or_default().to_string();
    let entries = crate::ruby::split(value, "|");
    let error = match validator {
        "AllowedIframesValidator" => {
            // %r{\Ahttps?://([^/]*/)+[^/]*\z}
            let valid = entries.iter().all(|url| {
                let rest = url
                    .strip_prefix("https://")
                    .or_else(|| url.strip_prefix("http://"));
                rest.is_some_and(|r| r.contains('/'))
            });
            (!valid).then(|| t("site_settings.errors.invalid_allowed_iframes_url"))
        }
        "AtLeastOneGroupValidator" => {
            if crate::ruby::is_blank(value) {
                Some(t("site_settings.errors.at_least_one_group_required"))
            } else {
                let ids: Vec<i64> = entries.iter().map(|e| crate::ruby::to_i(e)).collect();
                let found: Vec<i64> =
                    sqlx::query_scalar("SELECT id::bigint FROM groups WHERE id = ANY($1)")
                        .bind(&ids)
                        .fetch_all(&mut *conn)
                        .await?;
                ids.iter()
                    .any(|id| !found.contains(id))
                    .then(|| t("site_settings.errors.invalid_group"))
            }
        }
        "ColorListValidator" => {
            let valid = entries
                .iter()
                .all(|c| c.len() == 6 && c.chars().all(|ch| ch.is_ascii_hexdigit()));
            (!valid).then(|| t("site_settings.errors.invalid_hex_value"))
        }
        "CspScriptSrcValidator" => {
            let valid = entries.iter().all(|s| csp_source(s));
            (!valid).then(|| t("site_settings.errors.invalid_csp_script_src"))
        }
        "EmojiPickerPinnedGroupsValidator" => {
            if crate::ruby::is_blank(value) {
                None
            } else {
                let allowed = crate::emojis::allowed_groups(settings);
                let invalid: Vec<&str> = entries
                    .iter()
                    .map(|g| crate::ruby::strip(g))
                    .filter(|g| !crate::ruby::is_blank(g) && !allowed.iter().any(|a| a == g))
                    .collect();
                (!invalid.is_empty()).then(|| {
                    i18n.t_count(
                        "site_settings.errors.emoji_picker_pinned_groups_invalid",
                        invalid.len() as i64,
                        &[("groups", &invalid.join(", "))],
                    )
                    .unwrap_or_default()
                })
            }
        }
        "MarkdownLinkifyTldsValidator" => value
            .contains('*')
            .then(|| t("site_settings.errors.markdown_linkify_tlds")),
        "MarkdownTypographerQuotationMarksValidator" => {
            let valid = !crate::ruby::is_blank(value) && entries.len() == 4;
            (!valid).then(|| {
                i18n.t_count("site_settings.errors.list_value_count", 4, &[])
                    .unwrap_or_default()
            })
        }
        "RandomUsernameWordListValidator" => {
            let mut valid = !entries.is_empty();
            for word in &entries {
                let sanitized = crate::user_name_suggester::sanitize_username(settings, word)?;
                if sanitized != *word || crate::ruby::is_blank(&sanitized) {
                    valid = false;
                }
            }
            (!valid).then(|| t("site_settings.errors.invalid_random_username_word_list"))
        }
        "RegexpListValidator" => {
            // Ruby's own message names why a regex is refused; the port can
            // only tell regexes both engines accept.
            if entries.iter().any(|r| fancy_regex::Regex::new(r).is_err()) {
                return Err(crate::Unsupported(
                    "RegexpListValidator's message for a regex Ruby may refuse",
                )
                .into());
            }
            None
        }
        "TopMenuValidator" => {
            if crate::ruby::is_blank(value) {
                Some(t("site_settings.errors.must_include_latest"))
            } else {
                let choices: Vec<&str> = entries.iter().map(|c| crate::ruby::strip(c)).collect();
                let menu = defs
                    .get("top_menu")
                    .map(crate::site_setting_update::choice_values)
                    .unwrap_or_default();
                if choices.contains(&"unread") && settings.get("enable_unified_new")?.truthy() {
                    Some(t(
                        "site_settings.errors.top_menu_unread_not_allowed_with_unified_new",
                    ))
                } else if !choices.iter().all(|c| menu.iter().any(|m| m == c)) {
                    Some(t("site_settings.errors.invalid_string"))
                } else if !choices.contains(&"latest") {
                    Some(t("site_settings.errors.must_include_latest"))
                } else {
                    None
                }
            }
        }
        "Chat::DirectMessageEnabledGroupsValidator" => crate::ruby::is_blank(value)
            .then(|| t("site_settings.errors.direct_message_enabled_groups_invalid")),
        "ReactionsExcludedFromLikeSiteSettingValidator" => {
            let valid = crate::ruby::is_blank(value) || {
                let enabled = settings
                    .get("discourse_reactions_enabled_reactions")?
                    .to_s();
                let enabled = crate::ruby::split(&enabled, "|");
                let default = defs
                    .get("discourse_reactions_excluded_from_like")
                    .map(|d| d.default.to_s())
                    .unwrap_or_default();
                let default = crate::ruby::split(&default, "|");
                let like = settings
                    .get("discourse_reactions_reaction_for_like")?
                    .to_s();
                !entries.contains(&like.as_str())
                    && entries
                        .iter()
                        .all(|e| enabled.contains(e) || default.contains(e))
            };
            (!valid).then(|| t("site_settings.errors.invalid_excluded_emoji"))
        }
        _ => return Err(crate::Unsupported("site settings with this validator class").into()),
    };
    Ok(error)
}

/// CspScriptSrcValidator::VALID_SOURCE_REGEX
fn csp_source(s: &str) -> bool {
    if s == "'unsafe-eval'" || s == "'wasm-unsafe-eval'" {
        return true;
    }
    let Some(inner) = s.strip_prefix("'sha").and_then(|r| r.strip_suffix('\'')) else {
        return false;
    };
    let Some(rest) = ["256-", "384-", "512-"]
        .iter()
        .find_map(|p| inner.strip_prefix(p))
    else {
        return false;
    };
    let body = rest.trim_end_matches('=');
    rest.len() - body.len() <= 2
        && !body.is_empty()
        && body
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+/-_".contains(c))
}

/// `SiteSettings::Validations#validate_<name>` for the list settings that
/// have one (the category and tag defaults aside): Some(message) when it
/// raises.
pub fn validate_named(
    name: &str,
    value: &str,
    settings: &SiteSettings,
    i18n: &I18n,
) -> Result<Option<String>, AppError> {
    let error = |key: &str, args: &[(&str, &str)]| {
        i18n.t_with(&format!("errors.site_settings.{key}"), args)
            .unwrap_or_default()
    };
    Ok(match name {
        "share_quote_buttons" => (value.contains("facebook")
            && settings.get("facebook_app_id")?.is_blank())
        .then(|| error("share_quote_facebook_requirements", &[])),
        "cors_origins" => (!crate::ruby::is_blank(value)
            && crate::ruby::split(value, "|")
                .iter()
                .any(|o| o.ends_with('/')))
        .then(|| error("cors_origins_should_not_have_trailing_slash", &[])),
        "slow_down_crawler_user_agents" => {
            if crate::ruby::is_blank(value) {
                return Ok(None);
            }
            const PROHIBITED: &[&str] = &[
                "apple",
                "windows",
                "linux",
                "ubuntu",
                "gecko",
                "firefox",
                "chrome",
                "safari",
                "applewebkit",
                "webkit",
                "mozilla",
                "macintosh",
                "khtml",
                "intel",
                "osx",
                "os x",
                "iphone",
                "ipad",
                "mac",
            ];
            let lower = value.to_lowercase();
            for crawler in crate::ruby::split(&lower, "|") {
                if crawler.chars().count() < 3 {
                    return Ok(Some(error(
                        "slow_down_crawler_user_agent_must_be_at_least_3_characters",
                        &[],
                    )));
                }
                if PROHIBITED.iter().any(|p| p.contains(crawler)) {
                    let comma = i18n.t("word_connector.comma").unwrap_or(", ");
                    return Ok(Some(error(
                        "slow_down_crawler_user_agent_cannot_be_popular_browsers",
                        &[("values", &PROHIBITED.join(comma))],
                    )));
                }
            }
            None
        }
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csp_sources() {
        assert!(csp_source("'unsafe-eval'"));
        assert!(csp_source("'sha256-abc+/='"));
        assert!(!csp_source("'sha256-abc==='"));
        assert!(!csp_source("unsafe-eval"));
        assert!(!csp_source("'sha1-abc'"));
    }
}
