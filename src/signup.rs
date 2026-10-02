//! Signing up: users_controller.rb#create with the User model's validations
//! and create callbacks (stats, options, profile, avatar row, the trust
//! level group, default category, tag and sidebar preferences, the search
//! index), UserActivator's activation email, and what activating a user
//! saves (EmailToken.confirm's `user.active = true`).
//!
//! What a plain local signup does not reach is refused: invite codes, user
//! fields, staged users, OAuth sessions, approval (must_approve_users and
//! auto-approved domains), random avatars, screened emails, unicode
//! usernames, and validation errors other than the username's.

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::accounts::{self, token_scopes};
use crate::i18n::I18n;
use crate::site_settings::SiteSettings;
use crate::{AppError, Unsupported};

/// `UsernameValidator::MAX_CHARS`
const MAX_CHARS: usize = 60;

/// Single-segment `/u/<name>` routes that are not users#show
/// (`UsernameValidator.clashing_with_existing_route?`).
const CLASHING_ROUTES: [&str; 23] = [
    "account-created",
    "admin-login",
    "check_email",
    "check_username",
    "confirm-session",
    "create_passkey",
    "create_second_factor_security_key",
    "create_second_factor_totp",
    "disable_second_factor",
    "email-login",
    "enable_second_factor_totp",
    "random-username",
    "read-faq",
    "recent-searches",
    "register_passkey",
    "register_second_factor_security_key",
    "second_factor",
    "second_factors",
    "second_factors_backup",
    "security_key",
    "toggle-anon",
    "trusted-session",
    "update-activation-email",
];

/// `User.reserved_username?` or a clash with a route.
pub fn reserved_username(settings: &SiteSettings, username: &str) -> Result<bool, AppError> {
    let lower = accounts::normalize_username(username)?;
    if settings.get("here_mention")?.to_s() == lower {
        return Ok(true);
    }
    for reserved in settings.get("reserved_usernames")?.to_s().split('|') {
        if reserved.is_empty() {
            continue;
        }
        let pattern = format!(
            "^{}$",
            regex::escape(&reserved.to_lowercase()).replace("\\*", ".*")
        );
        if regex::Regex::new(&pattern).is_ok_and(|r| r.is_match(&lower)) {
            return Ok(true);
        }
    }
    Ok(CLASHING_ROUTES.contains(&lower.as_str()))
}

/// `UsernameValidator#valid_format?` for ASCII usernames: the first error.
fn username_format_error(
    settings: &SiteSettings,
    i18n: &I18n,
    username: &str,
) -> Result<Option<String>, AppError> {
    let t = |key: &str| i18n.t(key).unwrap_or(key).to_string();
    let count = |key: &str, n: i64| {
        let form = if n == 1 { "one" } else { "other" };
        i18n.t_with(&format!("{key}.{form}"), &[("count", &n.to_string())])
            .unwrap_or_default()
    };
    if username.trim().is_empty() {
        return Ok(Some(t("user.username.blank")));
    }
    if settings.get("unicode_usernames")?.truthy() {
        return Err(Unsupported("unicode usernames at signup").into());
    }
    let min = settings.get("min_username_length")?.to_i();
    let max = settings.get("max_username_length")?.to_i();
    let len = username.chars().count();
    if (len as i64) < min {
        return Ok(Some(count("user.username.short", min)));
    }
    if (len as i64) > max {
        return Ok(Some(count("user.username.long", max)));
    }
    if len > MAX_CHARS {
        return Ok(Some(t("user.username.too_long")));
    }
    if username
        .chars()
        .any(|c| !(c.is_ascii_alphanumeric() || "_.-".contains(c)))
    {
        return Ok(Some(t("user.username.characters")));
    }
    if !username
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Ok(Some(t(
            "user.username.must_begin_with_alphanumeric_or_underscore",
        )));
    }
    if !username
        .chars()
        .last()
        .is_some_and(|c| c.is_ascii_alphanumeric())
    {
        return Ok(Some(t("user.username.must_end_with_alphanumeric")));
    }
    let specials: Vec<bool> = username.chars().map(|c| "_.-".contains(c)).collect();
    if specials.windows(2).any(|w| w[0] && w[1]) {
        return Ok(Some(t(
            "user.username.must_not_contain_two_special_chars_in_seq",
        )));
    }
    let lower = username.to_lowercase();
    let confusing = [
        "js", "json", "css", "htm", "html", "xml", "jpg", "jpeg", "png", "gif", "bmp", "ico",
        "tif", "tiff", "woff",
    ];
    if confusing
        .iter()
        .any(|ext| lower.ends_with(&format!(".{ext}")))
    {
        return Ok(Some(t("user.username.must_not_end_with_confusing_suffix")));
    }
    Ok(None)
}

/// What users#create answers.
pub enum Created {
    /// The JSON body (always a 200).
    Json(Value),
    /// The session's activation key and the JSON body.
    Signed { user_id: i32, body: Value },
}

/// The fields of a local signup.
pub struct Signup<'a> {
    pub name: Option<&'a str>,
    pub email: &'a str,
    pub password: Option<&'a str>,
    pub username: &'a str,
    pub locale: Option<&'a str>,
    pub timezone: Option<&'a str>,
    pub ip: &'a str,
}

fn fail_with(i18n: &I18n, key: &str) -> Created {
    Created::Json(json!({"success": false, "message": i18n.t(key).unwrap_or(key)}))
}

/// users#create after the honeypot and the required params.
pub async fn create(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    i18n: &I18n,
    s: &Signup<'_>,
) -> Result<Created, AppError> {
    if settings.get("invite_code")?.presence().is_some() {
        return Err(Unsupported("invite codes at signup").into());
    }
    if settings.get("enable_discourse_connect")?.truthy() {
        return Ok(fail_with(
            i18n,
            "login.new_registrations_disabled_discourse_connect",
        ));
    }
    if !settings.get("allow_new_registrations")?.truthy() {
        return Ok(fail_with(i18n, "login.new_registrations_disabled"));
    }
    if s.password.is_some_and(|p| p.chars().count() > 200) {
        return Ok(fail_with(i18n, "login.password_too_long"));
    }
    if s.username.chars().count() > MAX_CHARS * 3 {
        return Err(Unsupported("overlong usernames at signup").into());
    }
    if s.email.len() > 254 + 1 + 253 {
        return Ok(fail_with(i18n, "login.email_too_long"));
    }
    if reserved_username(settings, s.username)? {
        return Ok(fail_with(i18n, "login.reserved_username"));
    }
    let email = s.email.trim().to_lowercase();
    let staged: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM users u JOIN user_emails e ON e.user_id = u.id \
         WHERE u.staged AND lower(e.email) = $1)",
    )
    .bind(&email)
    .fetch_one(&mut *conn)
    .await?;
    if staged {
        return Err(Unsupported("signing up a staged user").into());
    }
    if settings
        .get("auto_approve_email_domains")?
        .presence()
        .is_some()
        || settings.get("must_approve_users")?.truthy()
    {
        return Err(Unsupported("approval at signup").into());
    }
    let user_fields: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM user_fields)")
        .fetch_one(&mut *conn)
        .await?;
    if user_fields {
        return Err(Unsupported("user fields at signup").into());
    }
    if !settings.get("enable_local_logins")?.truthy() {
        return Err(Unsupported("signup with local logins off (403)").into());
    }

    // user.save: the validations, in User's order.
    let mut errors: Vec<(&str, String)> = Vec::new();
    if let Some(e) = username_format_error(settings, i18n, s.username)? {
        errors.push(("username", e));
    } else {
        let lower = accounts::normalize_username(s.username)?;
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM users WHERE username_lower = $1) \
             OR EXISTS (SELECT 1 FROM groups WHERE lower(name) = $1)",
        )
        .bind(&lower)
        .fetch_one(&mut *conn)
        .await?;
        if exists {
            errors.push((
                "username",
                i18n.t("user.username.unique").unwrap_or("").to_string(),
            ));
        }
    }
    let password_error = match s.password {
        Some(p) if !p.is_empty() => accounts::password_error(
            settings,
            p,
            &accounts::PasswordOwner {
                admin: false,
                username: s.username,
                name: s.name,
                email: Some(&email),
                current: None,
            },
        )?,
        _ => Some("blank"),
    };
    if s.name.is_none_or(|n| n.trim().is_empty())
        && settings.get("full_name_requirement")?.to_s() == "required_at_signup"
    {
        return Err(Unsupported("signup validation errors other than the username").into());
    }
    if s.name.is_some_and(|n| n.chars().count() > 255) {
        return Err(Unsupported("signup validation errors other than the username").into());
    }
    // allowed_ip_address: screened IPs, then the per-IP new account cap.
    if accounts::ip_blocked(&mut *conn, s.ip).await? {
        return Err(Unsupported("signup validation errors other than the username").into());
    }
    if prevent_registration_from_ip(&mut *conn, settings, s.ip).await? {
        return Err(Unsupported("signup validation errors other than the username").into());
    }
    // primary_email
    if !accounts::valid_email(&email) || !email_allowed(settings, &email)? {
        return Err(Unsupported("signup validation errors other than the username").into());
    }
    let screened: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM screened_emails WHERE email = $1)")
            .bind(&email)
            .fetch_one(&mut *conn)
            .await?;
    if screened {
        return Err(Unsupported("screened emails at signup").into());
    }
    let normalized = normalize_email(&email);
    let normalize = settings.get("normalize_emails")?.truthy();
    let taken: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM user_emails WHERE lower(email) = $1 OR ($3 AND lower(normalized_email) = $2))",
    )
    .bind(&email)
    .bind(&normalized)
    .bind(normalize)
    .fetch_one(&mut *conn)
    .await?;
    if taken {
        return Err(Unsupported("signing up with a taken email").into());
    }
    if password_error.is_some() {
        return Err(Unsupported("signup validation errors other than the username").into());
    }
    if !errors.is_empty() {
        let full: Vec<String> = errors
            .iter()
            .map(|(attr, message)| {
                let label = i18n
                    .t(&format!("activerecord.attributes.user.{attr}"))
                    .map(str::to_string)
                    .unwrap_or_else(|| humanize(attr));
                format!("{label} {message}")
            })
            .collect();
        let mut by_attr = Map::new();
        for (attr, message) in &errors {
            by_attr
                .entry(attr.to_string())
                .or_insert_with(|| json!([]))
                .as_array_mut()
                .expect("an array")
                .push(json!(message));
        }
        let message = i18n
            .t_with("login.errors", &[("errors", &full.join("\n"))])
            .unwrap_or_default();
        return Ok(Created::Json(json!({
            "success": false,
            "message": message,
            "errors": by_attr,
            "values": {"name": s.name, "username": s.username, "email": email},
            "is_developer": false,
        })));
    }

    // The insert and the after_create callbacks.
    let trust_level = settings.get("default_trust_level")?.to_i() as i32;
    let locale = s.locale.filter(|l| !l.is_empty()).unwrap_or("en");
    let user_id: i32 = sqlx::query_scalar(
        "INSERT INTO users (username, username_lower, name, active, approved, trust_level, ip_address, \
                            registration_ip_address, locale, created_at, updated_at) \
         VALUES ($1, $2, $3, FALSE, FALSE, $4, $5::inet, $5::inet, $6, clock_timestamp(), clock_timestamp()) \
         RETURNING id",
    )
    .bind(s.username)
    .bind(accounts::normalize_username(s.username)?)
    .bind(s.name)
    .bind(trust_level)
    .bind(s.ip)
    .bind(locale)
    .fetch_one(&mut *conn)
    .await?;
    sqlx::query(
        "INSERT INTO user_emails (user_id, email, \"primary\", normalized_email, created_at, updated_at) \
         VALUES ($1, $2, TRUE, $3, clock_timestamp(), clock_timestamp())",
    )
    .bind(user_id)
    .bind(&email)
    .bind(&normalized)
    .execute(&mut *conn)
    .await?;
    if let Some(password) = s.password {
        accounts::set_password(&mut *conn, user_id, password).await?;
    }
    // create_email_token (then replaced by the activation email's)
    accounts::create_email_token(&mut *conn, user_id, &email, token_scopes::SIGNUP).await?;
    sqlx::query("INSERT INTO user_stats (user_id, new_since) VALUES ($1, clock_timestamp())")
        .bind(user_id)
        .execute(&mut *conn)
        .await?;
    create_user_option(&mut *conn, settings, user_id).await?;
    sqlx::query("INSERT INTO user_profiles (user_id) VALUES ($1)")
        .bind(user_id)
        .execute(&mut *conn)
        .await?;
    if settings
        .get("selectable_avatars_random_on_signup")?
        .truthy()
        && settings.get("selectable_avatars_mode")?.to_s() != "disabled"
    {
        return Err(Unsupported("random avatars at signup").into());
    }
    ensure_in_trust_level_group(&mut *conn, user_id, trust_level).await?;
    default_category_and_tag_preferences(&mut *conn, settings, user_id).await?;
    default_sidebar_links(&mut *conn, settings, user_id).await?;
    // after_save: refresh_avatar (the avatar row and its gravatar job),
    // then the search index.
    let avatar_id: i32 = sqlx::query_scalar(
        "INSERT INTO user_avatars (user_id, created_at, updated_at) VALUES ($1, clock_timestamp(), clock_timestamp()) \
         RETURNING id",
    )
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?;
    if settings.get("automatically_download_gravatars")?.truthy() {
        enqueue_gravatar(&mut *conn, user_id, avatar_id).await?;
    }
    let username_lower = accounts::normalize_username(s.username)?;
    crate::posting::search_index::index_user(
        &mut *conn,
        settings,
        user_id,
        &username_lower,
        s.name,
    )
    .await?;

    // activation.finish: EmailActivator.
    let token =
        accounts::create_email_token(&mut *conn, user_id, &email, token_scopes::SIGNUP).await?;
    crate::jobs::enqueue(
        &mut *conn,
        "critical_user_email",
        json!({"type": "signup", "user_id": user_id, "email_token": token, "to_address": null}),
    )
    .await?;
    if let Some(tz) = s.timezone.filter(|t| !t.is_empty()) {
        if tz.contains('/') || tz == "UTC" {
            sqlx::query(
                "UPDATE user_options SET timezone = $2 WHERE user_id = $1 AND timezone IS NULL",
            )
            .bind(user_id)
            .bind(tz)
            .execute(&mut *conn)
            .await?;
        }
    }
    let message = i18n
        .t_with(
            "login.activate_email",
            &[("email", &html_escape::encode_text(&email))],
        )
        .unwrap_or_default();
    let mut body = json!({"success": true, "active": false, "message": message});
    if !settings.get("hide_email_address_taken")?.truthy() {
        body["user_id"] = json!(user_id);
    }
    Ok(Created::Signed { user_id, body })
}

fn humanize(attr: &str) -> String {
    let mut s = attr.replace('_', " ");
    if let Some(first) = s.get(0..1) {
        s.replace_range(0..1, &first.to_uppercase());
    }
    s
}

/// `UserEmail.normalize`: dots and +tags out of the local part.
pub fn normalize_email(email: &str) -> String {
    match email.split_once('@') {
        Some((local, domain)) if !local.is_empty() && !domain.is_empty() => {
            let local = local.replace('.', "");
            let local = local.split('+').next().unwrap_or("");
            format!("{local}@{domain}")
        }
        _ => String::new(),
    }
}

/// `EmailValidator.allowed?`
fn email_allowed(settings: &SiteSettings, email: &str) -> Result<bool, AppError> {
    let matches = |setting: &str| {
        let domains = setting.replace('.', "\\.");
        regex::RegexBuilder::new(&format!("@(.+\\.)?({domains})$"))
            .case_insensitive(true)
            .build()
            .is_ok_and(|r| r.is_match(email))
    };
    if let Some(allowed) = settings.get("allowed_email_domains")?.presence() {
        return Ok(matches(&allowed));
    }
    if let Some(blocked) = settings.get("blocked_email_domains")?.presence() {
        return Ok(!matches(&blocked));
    }
    Ok(true)
}

/// `SpamHandler.should_prevent_registration_from_ip?`, its allowed-IP
/// match recorded as ScreenedIpAddress.is_allowed? does.
async fn prevent_registration_from_ip(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    ip: &str,
) -> Result<bool, AppError> {
    let max = settings.get("max_new_accounts_per_registration_ip")?.to_i();
    if max <= 0 {
        return Ok(false);
    }
    let trusted: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM users WHERE trust_level >= 2 AND ip_address = $1::inet) \
         OR EXISTS (SELECT 1 FROM users u JOIN group_users gu ON gu.user_id = u.id \
                    WHERE gu.group_id = 3 AND u.id > 0 AND u.ip_address = $1::inet)",
    )
    .bind(ip)
    .fetch_one(&mut *conn)
    .await?;
    if trusted {
        return Ok(false);
    }
    // ScreenedIpAddress.is_allowed? (do_nothing = 2), recording the match.
    let screening: Option<(i32, i32)> = sqlx::query_as(
        "SELECT id, action_type FROM screened_ip_addresses WHERE $1::inet <<= ip_address \
         ORDER BY masklen(ip_address) DESC LIMIT 1",
    )
    .bind(ip)
    .fetch_optional(&mut *conn)
    .await?;
    if let Some((id, 2)) = screening {
        sqlx::query(
            "UPDATE screened_ip_addresses SET match_count = match_count + 1, \
             last_match_at = clock_timestamp(), updated_at = clock_timestamp() WHERE id = $1",
        )
        .bind(id)
        .execute(&mut *conn)
        .await?;
        return Ok(false);
    }
    let tl0: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM users WHERE trust_level = 0 AND ip_address = $1::inet",
    )
    .bind(ip)
    .fetch_one(&mut *conn)
    .await?;
    Ok(tl0 >= max)
}

/// `UserOption#set_defaults`: the options a new user starts with.
async fn create_user_option(
    conn: &mut PgConnection,
    s: &SiteSettings,
    user_id: i32,
) -> Result<(), AppError> {
    let text_size = match s.get("default_text_size")?.to_s().as_str() {
        "normal" => 0,
        "larger" => 1,
        "largest" => 2,
        "smaller" => 3,
        "smallest" => 4,
        _ => return Err(Unsupported("an unknown default_text_size").into()),
    };
    let title_count_mode = match s.get("default_title_count_mode")?.to_s().as_str() {
        "notifications" => 0,
        "contextual" => 1,
        _ => return Err(Unsupported("an unknown default_title_count_mode").into()),
    };
    let digest = s.get("default_email_digest_frequency")?.to_i();
    let b = |name: &str| -> Result<bool, AppError> { Ok(s.get(name)?.truthy()) };
    let i = |name: &str| -> Result<i64, AppError> { Ok(s.get(name)?.to_i()) };
    sqlx::query(
        "INSERT INTO user_options (user_id, mailing_list_mode, mailing_list_mode_frequency, email_level, \
           email_messages_level, automatically_unpin_topics, email_previous_replies, email_in_reply_to, \
           enable_quoting, enable_smart_lists, enable_markdown_monospace_font, external_links_in_new_tab, \
           dynamic_favicon, skip_new_user_tips, new_topic_duration_minutes, auto_track_topics_after_msecs, \
           notification_level_when_replying, like_notification_frequency, email_digests, digest_after_minutes, \
           include_tl0_in_digests, text_size_key, title_count_mode_key, hide_profile, hide_presence, \
           sidebar_link_to_filtered_list, sidebar_show_count_of_new_items, composition_mode, \
           watched_precedence_over_muted) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, \
                 $21, $22, $23, $24, $25, $26, $27, $28, $29)",
    )
    .bind(user_id)
    .bind(b("default_email_mailing_list_mode")?)
    .bind(i("default_email_mailing_list_mode_frequency")? as i32)
    .bind(i("default_email_level")? as i32)
    .bind(i("default_email_messages_level")? as i32)
    .bind(b("default_topics_automatic_unpin")?)
    .bind(i("default_email_previous_replies")? as i32)
    .bind(b("default_email_in_reply_to")?)
    .bind(b("default_other_enable_quoting")?)
    .bind(b("default_other_enable_smart_lists")?)
    .bind(b("default_other_enable_markdown_monospace_font")?)
    .bind(b("default_other_external_links_in_new_tab")?)
    .bind(b("default_other_dynamic_favicon")?)
    .bind(b("default_other_skip_new_user_tips")?)
    .bind(i("default_other_new_topic_duration_minutes")? as i32)
    .bind(i("default_other_auto_track_topics_after_msecs")? as i32)
    .bind(i("default_other_notification_level_when_replying")? as i32)
    .bind(i("default_other_like_notification_frequency")? as i32)
    .bind(digest > 0)
    .bind(digest as i32)
    .bind(b("default_include_tl0_in_digests")?)
    .bind(text_size)
    .bind(title_count_mode)
    .bind(b("default_hide_profile")?)
    .bind(b("default_hide_presence")?)
    .bind(b("default_sidebar_link_to_filtered_list")?)
    .bind(b("default_sidebar_show_count_of_new_items")?)
    .bind(i("default_composition_mode")? as i32)
    .bind(b("default_watched_precedence_over_muted")?)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// `Group.user_trust_level_change!` for a new user: the trust level groups
/// up to theirs (trust_level_0 is group 10), with GroupManager's side
/// effects for groups that have none to apply.
async fn ensure_in_trust_level_group(
    conn: &mut PgConnection,
    user_id: i32,
    trust_level: i32,
) -> Result<(), AppError> {
    for level in 0..=trust_level {
        let group_id = 10 + level;
        #[derive(sqlx::FromRow)]
        struct G {
            default_notification_level: i32,
            title: Option<String>,
            primary_group: bool,
            grant_trust_level: Option<i32>,
        }
        let Some(group): Option<G> = sqlx::query_as(
            "SELECT default_notification_level, title, primary_group, grant_trust_level FROM groups WHERE id = $1",
        )
        .bind(group_id)
        .fetch_optional(&mut *conn)
        .await?
        else {
            return Err(Unsupported("refreshing a missing automatic group").into());
        };
        let defaults: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM group_category_notification_defaults WHERE group_id = $1) \
             OR EXISTS (SELECT 1 FROM group_tag_notification_defaults WHERE group_id = $1)",
        )
        .bind(group_id)
        .fetch_one(&mut *conn)
        .await?;
        if group.title.is_some_and(|t| !t.is_empty())
            || group.primary_group
            || group.grant_trust_level.is_some()
            || defaults
        {
            return Err(Unsupported(
                "automatic groups with titles, flair or notification defaults",
            )
            .into());
        }
        let inserted = sqlx::query(
            "INSERT INTO group_users (group_id, user_id, notification_level, created_at, updated_at) \
             VALUES ($1, $2, $3, clock_timestamp(), clock_timestamp()) ON CONFLICT DO NOTHING",
        )
        .bind(group_id)
        .bind(user_id)
        .bind(group.default_notification_level)
        .execute(&mut *conn)
        .await?
        .rows_affected();
        if inserted > 0 {
            sqlx::query("UPDATE groups SET user_count = user_count + 1 WHERE id = $1")
                .bind(group_id)
                .execute(&mut *conn)
                .await?;
        }
    }
    Ok(())
}

/// `set_default_categories_preferences` and `set_default_tags_preferences`.
async fn default_category_and_tag_preferences(
    conn: &mut PgConnection,
    s: &SiteSettings,
    user_id: i32,
) -> Result<(), AppError> {
    for (setting, level) in [
        ("default_categories_watching", 3),
        ("default_categories_watching_first_post", 4),
        ("default_categories_tracking", 2),
        ("default_categories_normal", 1),
        ("default_categories_muted", 0),
    ] {
        for id in s.get(setting)?.to_s().split('|').map(crate::ruby::to_i) {
            if id == 0 {
                continue;
            }
            sqlx::query("INSERT INTO category_users (user_id, category_id, notification_level) VALUES ($1, $2, $3)")
                .bind(user_id)
                .bind(id as i32)
                .bind(level)
                .execute(&mut *conn)
                .await?;
        }
    }
    for setting in [
        "default_tags_watching",
        "default_tags_watching_first_post",
        "default_tags_tracking",
        "default_tags_muted",
    ] {
        if s.get(setting)?.presence().is_some() {
            return Err(Unsupported("default tag preferences at signup").into());
        }
    }
    Ok(())
}

/// `set_default_sidebar_section_links`: the default navigation menu
/// categories (and tags, refused) as the user's sidebar links.
async fn default_sidebar_links(
    conn: &mut PgConnection,
    s: &SiteSettings,
    user_id: i32,
) -> Result<(), AppError> {
    let ids: Vec<i32> = s
        .get("default_navigation_menu_categories")?
        .to_s()
        .split('|')
        .filter(|v| !v.is_empty())
        .map(|v| crate::ruby::to_i(v) as i32)
        .collect();
    if !ids.is_empty() {
        let existing: Vec<i32> = sqlx::query_scalar("SELECT id FROM categories WHERE id = ANY($1)")
            .bind(&ids)
            .fetch_all(&mut *conn)
            .await?;
        for id in existing.into_iter().take(500) {
            sqlx::query(
                "INSERT INTO sidebar_section_links (user_id, linkable_id, linkable_type, created_at, updated_at) \
                 VALUES ($1, $2, 'Category', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)",
            )
            .bind(user_id)
            .bind(id)
            .execute(&mut *conn)
            .await?;
        }
    }
    if s.get("tagging_enabled")?.truthy()
        && s.get("default_navigation_menu_tags")?.presence().is_some()
    {
        return Err(Unsupported("default sidebar tags at signup").into());
    }
    Ok(())
}

/// `refresh_avatar`'s gravatar download, a pending one replaced.
async fn enqueue_gravatar(
    conn: &mut PgConnection,
    user_id: i32,
    avatar_id: i32,
) -> Result<(), AppError> {
    let args = json!({"user_id": user_id, "avatar_id": avatar_id});
    sqlx::query(
        "DELETE FROM discourse_rs.jobs WHERE name = 'update_gravatar' AND failed_at IS NULL \
         AND locked_until IS NULL AND args = $1",
    )
    .bind(&args)
    .execute(&mut *conn)
    .await?;
    crate::jobs::enqueue_in(&mut *conn, 1, "update_gravatar", args).await?;
    Ok(())
}

/// `user.active = true; user.save!` from EmailToken.confirm: the
/// after_save avatar refresh (the gravatar job until one has run) and the
/// welcome message (which the narrative bot turns off when enabled).
pub async fn activate_user(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    user_id: i32,
) -> Result<(), AppError> {
    sqlx::query("UPDATE users SET active = TRUE, updated_at = clock_timestamp() WHERE id = $1")
        .bind(user_id)
        .execute(&mut *conn)
        .await?;
    let avatar: Option<(i32, Option<chrono::NaiveDateTime>)> = sqlx::query_as(
        "SELECT id, last_gravatar_download_attempt FROM user_avatars WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?;
    if let Some((avatar_id, None)) = avatar {
        if settings.get("automatically_download_gravatars")?.truthy() {
            enqueue_gravatar(&mut *conn, user_id, avatar_id).await?;
        }
    }
    if settings.get("send_welcome_message")?.truthy()
        && !settings.get("discourse_narrative_bot_enabled")?.truthy()
    {
        return Err(Unsupported("welcome messages").into());
    }
    Ok(())
}
