//! Port of Admin::EmailTemplatesController: the email texts an admin may
//! customize (index, show, revert). Customizing them writes translation
//! overrides, which every mail the port renders would then have to read
//! through I18n; that layer is not ported, so `update` is refused, and so
//! is anything that would show or revert an existing override.

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};
use sqlx::PgConnection;

use super::session::{bad_csrf, csrf_ok};
use crate::i18n::I18n;
use crate::params;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

/// `Admin::EmailTemplatesController.email_keys` (no restricted key is in
/// the list, and no plugin on the reference adds any).
pub const EMAIL_KEYS: [&str; 119] = [
    "admin_confirmation_mailer",
    "custom_invite_forum_mailer",
    "custom_invite_mailer",
    "download_backup_mailer",
    "email_login_code_mailer",
    "invite_forum_mailer",
    "invite_mailer",
    "invite_password_instructions",
    "new_version_mailer",
    "new_version_mailer_with_notes",
    "password_reset_code_mailer",
    "system_messages.backup_failed",
    "system_messages.backup_succeeded",
    "system_messages.bulk_invite_failed",
    "system_messages.bulk_invite_succeeded",
    "system_messages.csv_export_failed",
    "system_messages.csv_export_succeeded",
    "system_messages.download_remote_images_disabled",
    "system_messages.email_error_notification",
    "system_messages.email_reject_attachment",
    "system_messages.email_reject_auto_generated",
    "system_messages.email_reject_bad_destination_address",
    "system_messages.email_reject_email_alias",
    "system_messages.email_reject_empty",
    "system_messages.email_reject_inactive_user",
    "system_messages.email_reject_insufficient_trust_level",
    "system_messages.email_reject_invalid_access",
    "system_messages.email_reject_invalid_post",
    "system_messages.email_reject_invalid_post_action",
    "system_messages.email_reject_invalid_post_specified",
    "system_messages.email_reject_not_allowed_email",
    "system_messages.email_reject_old_destination",
    "system_messages.email_reject_parsing",
    "system_messages.email_reject_post_too_short",
    "system_messages.email_reject_reply_key",
    "system_messages.email_reject_reply_not_allowed",
    "system_messages.email_reject_reply_to_digest",
    "system_messages.email_reject_reply_user_not_matching",
    "system_messages.email_reject_screened_email",
    "system_messages.email_reject_silenced_user",
    "system_messages.email_reject_strangers_not_allowed",
    "system_messages.email_reject_too_many_recipients",
    "system_messages.email_reject_topic_closed",
    "system_messages.email_reject_topic_not_found",
    "system_messages.email_reject_unrecognized_error",
    "system_messages.email_reject_user_not_found",
    "system_messages.email_revoked",
    "system_messages.flags_agreed_and_post_deleted",
    "system_messages.flags_agreed_and_post_deleted_for_responders",
    "system_messages.flags_disagreed",
    "system_messages.ignored_users_summary",
    "system_messages.new_user_of_the_month",
    "system_messages.pending_users_reminder",
    "system_messages.post_hidden",
    "system_messages.post_hidden_again",
    "system_messages.queued_by_staff",
    "system_messages.queued_posts_reminder",
    "system_messages.restore_failed",
    "system_messages.restore_succeeded",
    "system_messages.reviewable_queued_post_revise_and_reject",
    "system_messages.reviewable_queued_post_revise_and_reject_new_topic",
    "system_messages.reviewables_reminder",
    "system_messages.silenced_by_staff",
    "system_messages.spam_post_blocked",
    "system_messages.tl2_promotion_message",
    "system_messages.too_many_spam_flags",
    "system_messages.too_many_tl3_flags",
    "system_messages.unsilenced",
    "system_messages.user_added_to_group_as_member",
    "system_messages.user_added_to_group_as_owner",
    "system_messages.user_automatically_silenced",
    "system_messages.user_automatically_silenced_with_reason",
    "system_messages.user_posts_deleted",
    "system_messages.welcome_invite",
    "system_messages.welcome_staff",
    "system_messages.welcome_tl1_user",
    "system_messages.welcome_user",
    "test_mailer",
    "unsubscribe_mailer",
    "user_notifications.account_created",
    "user_notifications.account_deleted",
    "user_notifications.account_exists",
    "user_notifications.account_second_factor_disabled",
    "user_notifications.account_silenced",
    "user_notifications.account_silenced_forever",
    "user_notifications.account_suspended",
    "user_notifications.account_suspended_forever",
    "user_notifications.activation_reminder",
    "user_notifications.admin_login",
    "user_notifications.confirm_new_email",
    "user_notifications.confirm_new_email_via_admin",
    "user_notifications.email_login",
    "user_notifications.forgot_password",
    "user_notifications.notify_old_email",
    "user_notifications.notify_old_email_add",
    "user_notifications.post_approved",
    "user_notifications.set_password",
    "user_notifications.signup",
    "user_notifications.signup_after_approval",
    "user_notifications.signup_after_reject",
    "user_notifications.suspicious_login",
    "user_notifications.user_group_mentioned",
    "user_notifications.user_group_mentioned_pm",
    "user_notifications.user_group_mentioned_pm_group",
    "user_notifications.user_invited_to_private_message_pm",
    "user_notifications.user_invited_to_private_message_pm_group",
    "user_notifications.user_invited_to_private_message_pm_staged",
    "user_notifications.user_invited_to_topic",
    "user_notifications.user_linked",
    "user_notifications.user_mentioned",
    "user_notifications.user_mentioned_pm",
    "user_notifications.user_posted",
    "user_notifications.user_posted_pm",
    "user_notifications.user_posted_pm_staged",
    "user_notifications.user_quoted",
    "user_notifications.user_replied",
    "user_notifications.user_replied_pm",
    "user_notifications.user_watching_category_or_tag",
    "user_notifications.user_watching_first_post",
];

/// `TranslationOverride::ALLOWED_CUSTOM_INTERPOLATION_KEYS`
const CUSTOM_INTERPOLATION_KEYS: [(&[&str], &[&str]); 4] = [
    (
        &[
            "user_notifications.user_",
            "user_notifications.only_reply_by_email",
            "user_notifications.reply_by_email",
            "user_notifications.visit_link_to_respond",
            "user_notifications.header_instructions",
            "user_notifications.pm_participants",
            "unsubscribe_mailing_list",
            "unsubscribe_link_and_mail",
            "unsubscribe_link",
        ],
        &[
            "topic_title",
            "topic_title_url_encoded",
            "message",
            "url",
            "post_id",
            "topic_id",
            "context",
            "username",
            "recipient_username",
            "group_name",
            "unsubscribe_url",
            "subject_pm",
            "participants",
            "site_description",
            "site_title",
            "site_title_url_encoded",
            "site_name",
            "optional_re",
            "optional_pm",
            "optional_cat",
            "optional_tags",
        ],
    ),
    (
        &["system_messages.welcome_user"],
        &["username", "name", "name_or_username"],
    ),
    (&["js.welcome_banner.header"], &["site_name"]),
    (&["email_from"], &["site_name"]),
];

/// `TranslationOverride.custom_interpolation_keys(key)`
fn custom_interpolation_keys(key: &str) -> &'static [&'static str] {
    CUSTOM_INTERPOLATION_KEYS
        .iter()
        .find(|(prefixes, _)| prefixes.iter().any(|p| key.starts_with(p)))
        .map_or(&[], |(_, keys)| keys)
}

/// `I18nInterpolationKeysFinder.find`: the names in `%{name}`,
/// `%<name>...` and `{{name}}`, first appearance first.
fn interpolation_keys(text: &str, keys: &mut Vec<String>) {
    static PATTERN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r"%%|%\{([\w|]+)\}|%<(\w+)>[^\d]*?\d*\.?\d*[bBdiouxXeEfgGcps]|\{\{(\w+)\}\}",
        )
        .expect("interpolation pattern")
    });
    for c in PATTERN.captures_iter(text) {
        if let Some(m) = c.get(1).or(c.get(2)).or(c.get(3)) {
            let name = m.as_str().to_string();
            if !keys.contains(&name) {
                keys.push(name);
            }
        }
    }
}

/// The request's locale (the admin's own when users may pick one, else
/// the site's) must be English: the port's I18n has only it.
async fn english(conn: &mut PgConnection, s: &SiteSettings, user_id: i32) -> Result<(), AppError> {
    let refuse = || Err(Unsupported("email templates in locales other than English").into());
    if s.get("default_locale")?.to_s() != "en"
        || s.get("set_locale_from_accept_language_header")?.truthy()
    {
        return refuse();
    }
    if s.get("allow_user_locale")?.truthy() {
        let locale: Option<String> = sqlx::query_scalar("SELECT locale FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(&mut *conn)
            .await?
            .flatten();
        if locale.is_some_and(|l| !l.is_empty() && l != "en") {
            return refuse();
        }
    }
    Ok(())
}

/// `I18n.t(key)` of a template's subject or body: its string, or the
/// hash a pluralized one holds.
enum Text {
    Str(String),
    Plural(Vec<(String, String)>),
}

fn lookup(i18n: &I18n, key: &str) -> Result<Text, Unsupported> {
    if let Some(s) = i18n.t(key) {
        return Ok(Text::Str(s.to_string()));
    }
    let Some(Value::Object(map)) = i18n.subtree(key) else {
        return Err(Unsupported(
            "an email template text missing from the locale",
        ));
    };
    let mut forms = Vec::new();
    for form in ["zero", "one", "two", "few", "many", "other"] {
        match map.get(form) {
            Some(Value::String(s)) => forms.push((form.to_string(), s.clone())),
            Some(_) => return Err(Unsupported("a nested email template text")),
            None => {}
        }
    }
    if forms.len() != map.len() {
        return Err(Unsupported(
            "an email template text with unknown plural forms",
        ));
    }
    Ok(Text::Plural(forms))
}

/// Ruby's `String#inspect` for the texts I18n holds.
fn ruby_inspect(s: &str) -> Result<String, Unsupported> {
    let mut out = String::from("\"");
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '#' if matches!(chars.peek(), Some('{' | '$' | '@')) => out.push_str("\\#"),
            c if c.is_control() => {
                return Err(Unsupported("control characters in a template text"));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    Ok(out)
}

impl Text {
    /// The value as a text column takes it (`to_s`, a hash inspected).
    fn to_column(&self) -> Result<String, Unsupported> {
        match self {
            Text::Str(s) => Ok(s.clone()),
            Text::Plural(forms) => {
                let mut parts = Vec::new();
                for (k, v) in forms {
                    parts.push(format!("{k}: {}", ruby_inspect(v)?));
                }
                Ok(format!("{{{}}}", parts.join(", ")))
            }
        }
    }
}

/// AdminEmailTemplateSerializer, with no override in place.
fn serialize(i18n: &I18n, key: &str) -> Result<Value, AppError> {
    let title = match i18n.t(&format!("{key}.title")) {
        Some(t) => t.to_string(),
        None => crate::plugins::poll::vote::titleize(key.rsplit('.').next().unwrap_or(key))?,
    };
    let subject_key = format!("{key}.subject_template");
    let body_key = format!("{key}.text_body_template");
    let subject = if i18n.t(&format!("{subject_key}.other")).is_some() {
        Value::Null
    } else {
        match lookup(i18n, &subject_key)? {
            Text::Str(s) => json!(s),
            Text::Plural(_) => return Err(Unsupported("a pluralized subject without other").into()),
        }
    };
    let body = match lookup(i18n, &body_key)? {
        Text::Str(s) => json!(s),
        // The hash, as JSON.
        Text::Plural(forms) => Value::Object(
            forms
                .into_iter()
                .map(|(k, v)| (k, Value::String(v)))
                .collect(),
        ),
    };
    let mut keys: Vec<String> = Vec::new();
    for k in [&subject_key, &body_key] {
        if let Some(text) = i18n.t(k) {
            interpolation_keys(text, &mut keys);
        }
    }
    for k in custom_interpolation_keys(key) {
        if !keys.iter().any(|x| x == k) {
            keys.push((*k).to_string());
        }
    }
    keys.sort();
    Ok(json!({
        "id": key,
        "title": title,
        "subject": subject,
        "body": body,
        "can_revert": false,
        "interpolation_keys": keys,
    }))
}

/// The overrides of these templates' texts, for the request's locale.
async fn overridden(conn: &mut PgConnection, keys: &[String]) -> Result<bool, AppError> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM translation_overrides WHERE locale = 'en' AND translation_key = ANY($1))",
    )
    .bind(keys)
    .fetch_one(conn)
    .await?)
}

fn template_keys(key: &str) -> Vec<String> {
    vec![
        format!("{key}.subject_template"),
        format!("{key}.text_body_template"),
    ]
}

fn refuse_html(uri: &Uri, headers: &HeaderMap) -> Result<(), AppError> {
    if super::wants_json(uri.query(), headers) || uri.path().ends_with(".json") {
        return Ok(());
    }
    Err(Unsupported("the admin email templates pages (Ember)").into())
}

/// The route's `id` constraint (`[0-9a-z_.]+`): anything else has no
/// route.
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_digit() || c.is_ascii_lowercase() || c == '_' || c == '.')
}

/// GET /admin/email/templates(.json)
pub async fn index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if !guardian.is_admin() {
        return Ok(super::topics::not_found_response(&state));
    }
    if super::admin_email::html_request(&uri, &headers) {
        return super::admin_email_pages::page(
            &state,
            &guardian,
            &headers,
            &uri,
            crate::admin_email_view::Tab::Templates,
        )
        .await;
    }
    let mut conn = state.pool.acquire().await?;
    let s = SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let doc = document(
        &mut conn,
        &state,
        &s,
        guardian.user_id().unwrap_or_default(),
    )
    .await?;
    Ok(axum::Json(doc).into_response())
}

/// The templates' document, the locale's texts as they stand.
pub(super) async fn document(
    conn: &mut PgConnection,
    state: &AppState,
    s: &SiteSettings,
    user_id: i32,
) -> Result<Value, AppError> {
    english(&mut *conn, s, user_id).await?;
    let all: Vec<String> = EMAIL_KEYS.iter().flat_map(|k| template_keys(k)).collect();
    if overridden(&mut *conn, &all).await? {
        return Err(Unsupported("customized email templates (translation overrides)").into());
    }
    let templates = EMAIL_KEYS
        .iter()
        .map(|k| serialize(&state.i18n, k))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(json!({
        "email_templates": templates,
        "__rest_serializer": "1",
    }))
}

/// GET /admin/email/templates/:id: an empty action.
pub async fn show(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if !guardian.is_admin() || !valid_id(&id) {
        return Ok(super::topics::not_found_response(&state));
    }
    refuse_html(&uri, &headers)?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// PUT /admin/email/templates/:id
pub async fn update(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    if !guardian.is_admin() || !valid_id(&id) {
        return Ok(super::topics::not_found_response(&state));
    }
    Err(Unsupported("customizing email templates (translation overrides read by I18n)").into())
}

/// DELETE /admin/email/templates/:id: the template's overrides removed,
/// and both texts logged as changed.
pub async fn revert(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !guardian.is_admin() || !valid_id(&id) {
        return Ok(super::topics::not_found_response(&state));
    }
    let pairs: Vec<(String, String)> = p
        .iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect();
    if !csrf_ok(&state, &headers, &pairs, uri.path(), "DELETE") {
        return Ok(bad_csrf());
    }
    // The id takes any `.json` in: no such template.
    if !EMAIL_KEYS.contains(&id.as_str()) {
        return Ok(super::topics::not_found_response(&state));
    }
    let acting = guardian.user_id().unwrap_or_default();
    let mut tx = state.pool.begin().await?;
    let s = SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    english(&mut tx, &s, acting).await?;
    let keys = template_keys(&id);
    if overridden(&mut tx, &keys).await? {
        return Err(Unsupported("reverting a customized email template").into());
    }
    // TranslationOverride.revert!: nothing to delete; I18n reloaded all
    // the same.
    state
        .bus
        .publish(&mut tx, "/i18n-flush", &json!({"refresh": true}), None)
        .await?;
    let action = crate::staff_action_logs::action_id("change_site_text")
        .ok_or(Unsupported("a staff action missing from enums.json"))?;
    for key in &keys {
        let text = lookup(&state.i18n, key)?.to_column()?;
        // log_site_text_change(key, new, old): the same text.
        sqlx::query(
            "INSERT INTO user_histories (action, acting_user_id, subject, previous_value, new_value, \
                                         admin_only, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $4, TRUE, now(), now())",
        )
        .bind(action)
        .bind(acting)
        .bind(key)
        .bind(&text)
        .execute(&mut *tx)
        .await?;
    }
    let out = serialize(&state.i18n, &id)?;
    tx.commit().await?;
    Ok(axum::Json(json!({
        "email_template": out,
        "__rest_serializer": "1",
    }))
    .into_response())
}
