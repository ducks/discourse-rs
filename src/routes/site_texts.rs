//! Admin::SiteTextsController: the site texts (translation overrides and
//! theme translations) under /admin/customize/site_texts, and the reseed
//! options. JSON only: the admin pages are Ember's.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::site_texts::{self, Overrides, Record, ThemeEntry};
use crate::{AppError, AppState, Unsupported};

const PER_PAGE: usize = 50;

fn json_request(headers: &HeaderMap, uri: &Uri) -> bool {
    uri.path().ends_with(".json")
        || headers
            .get("x-requested-with")
            .is_some_and(|v| v == "XMLHttpRequest")
        || super::wants_json(uri.query(), headers)
}

fn pairs(p: &Map<String, Value>) -> Vec<(String, String)> {
    p.iter()
        .filter_map(|(k, v)| crate::params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

/// The `:id` constraint, `/[\w.\-\+\%\&]+/i`, after `.json` is taken off.
fn route_id(raw: &str) -> Option<String> {
    let id = raw.strip_suffix(".json").unwrap_or(raw);
    let ok = !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | '-' | '+' | '%' | '&'));
    ok.then(|| id.to_string())
}

/// `fetch_locale`: available, and (in the port) English.
fn fetch_locale(
    state: &AppState,
    locale: Option<&str>,
) -> Result<Result<String, Response>, AppError> {
    let Some(locale) = locale.filter(|l| !crate::ruby::is_blank(l)) else {
        return Ok(Err(super::search::invalid_parameters(state, "locale")));
    };
    let available = crate::setting_enums::recorded("default_locale")
        .and_then(|r| r.get("valid_values"))
        .and_then(Value::as_array)
        .is_some_and(|values| {
            values
                .iter()
                .any(|v| v.get("value").and_then(Value::as_str) == Some(locale))
        });
    if !available {
        return Ok(Err(super::search::invalid_parameters(state, "locale")));
    }
    if locale != "en" {
        return Err(Unsupported("site texts in locales other than English").into());
    }
    Ok(Ok(locale.to_string()))
}

fn restricted(state: &AppState) -> Response {
    super::search::invalid_access_with(state, "email_template_cant_be_modified")
}

/// `find_site_text`: the record, or the 403 / 404.
async fn find_site_text(
    state: &AppState,
    overrides: &Overrides,
    conn: &mut PgConnection,
    id: &str,
    locale: &str,
) -> Result<Result<Record, Response>, AppError> {
    if site_texts::RESTRICTED_KEYS.contains(&id) {
        return Ok(Err(restricted(state)));
    }
    let overridden = site_texts::find_override(conn, locale, id).await?.is_some();
    if site_texts::tree().exists(id) || overridden {
        return Ok(Ok(site_texts::record_for(overrides, id, None, locale)?));
    }
    // PLURALIZED_REGEX: fix_plural_keys with nothing found, English's
    // forms (or `other`) for the missing ones.
    if let Some((base, form)) = id.rsplit_once('.')
        && matches!(form, "zero" | "one" | "two" | "few" | "many" | "other")
    {
        if !site_texts::PLURAL_KEYS.contains(&form) {
            return Ok(Err(super::topics::not_found_response(state)));
        }
        let fallback = match site_texts::tree().default_value(base)? {
            Value::Object(m) => m,
            // A string there makes Rails' `fallback_value[k]` raise.
            Value::String(s) if !s.starts_with("Translation missing: ") => {
                return Err(Unsupported("a plural site text of a string key").into());
            }
            _ => Map::new(),
        };
        let value = fallback
            .get(form)
            .or_else(|| fallback.get("other"))
            .cloned();
        return Ok(Ok(site_texts::record_for(overrides, id, value, "en")?));
    }
    Ok(Err(super::topics::not_found_response(state)))
}

fn refuse_non_admin(state: &AppState, guardian: &crate::guardian::Guardian) -> Option<Response> {
    (!guardian.is_admin()).then(|| super::topics::not_found_response(state))
}

/// One search hit before paging: a core record or a theme's site text.
enum Hit {
    Core(Record),
    Theme(Value),
}

impl Hit {
    fn id(&self) -> &str {
        match self {
            Hit::Core(r) => &r.id,
            Hit::Theme(v) => v["id"].as_str().unwrap_or_default(),
        }
    }

    fn value(&self) -> &str {
        match self {
            Hit::Core(r) => r.value.as_str().unwrap_or_default(),
            Hit::Theme(v) => v["value"].as_str().unwrap_or_default(),
        }
    }
}

/// `String#casecmp(other) == 0`: equal but for ASCII case.
fn casecmp_eq(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .all(|(x, y)| x.eq_ignore_ascii_case(&y))
}

/// GET /admin/customize/site_texts(.json)
pub async fn index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if let Some(r) = refuse_non_admin(&state, &guardian) {
        return Ok(r);
    }
    if !json_request(&headers, &uri) {
        return Err(Unsupported("the admin site texts page").into());
    }
    let query = crate::params::parse_query(uri.query().unwrap_or(""));
    let flag = |k: &str| crate::params::string(&query, k).as_deref() == Some("true");
    let overridden = flag("overridden");
    let outdated = flag("outdated");
    let untranslated = flag("untranslated");
    let only_selected_locale = flag("only_selected_locale");
    let mut conn = state.pool.acquire().await?;
    let themes: Vec<(i32, String, bool)> =
        sqlx::query_as("SELECT id, name, enabled FROM themes ORDER BY name, id")
            .fetch_all(&mut *conn)
            .await?;
    let mut extras = Map::new();
    extras.insert(
        "themes".into(),
        Value::Array(
            themes
                .iter()
                .map(|(id, name, enabled)| json!({"id": id, "name": name, "enabled": enabled}))
                .collect(),
        ),
    );
    let theme =
        match crate::params::string(&query, "theme_id").filter(|t| !crate::ruby::is_blank(t)) {
            Some(raw) => {
                let id = crate::ruby::to_i(&raw) as i32;
                if !themes.iter().any(|(t, _, _)| *t == id) {
                    return Ok(super::topics::not_found_response(&state));
                }
                Some(id)
            }
            None => None,
        };
    let q = crate::params::string(&query, "q").unwrap_or_default();
    let locale = match fetch_locale(&state, crate::params::string(&query, "locale").as_deref())? {
        Ok(l) => l,
        Err(r) => return Ok(r),
    };
    let overrides = Overrides::load(&mut conn, &locale).await?;

    let mut results: Vec<Hit> = Vec::new();
    if crate::ruby::is_blank(&q) && !overridden && !outdated && !untranslated && theme.is_none() {
        extras.insert("recommended".into(), Value::Bool(true));
        for key in site_texts::PREFERRED_KEYS {
            results.push(Hit::Core(site_texts::record_for(
                &overrides, key, None, &locale,
            )?));
        }
    } else {
        let regexp = site_texts::search_regexp(&q)?;
        if theme.is_none() {
            for r in find_translations(
                &mut conn,
                &overrides,
                &regexp,
                overridden,
                outdated,
                untranslated,
                only_selected_locale,
                &locale,
            )
            .await?
            {
                results.push(Hit::Core(r));
            }
        }
        // ThemeTranslationManager.search_site_texts
        if !outdated && !untranslated {
            let theme_ids: Vec<i32> = match theme {
                Some(id) => vec![id],
                // Theme.includes(...) without an order: the table's.
                None => {
                    sqlx::query_scalar("SELECT id FROM themes")
                        .fetch_all(&mut *conn)
                        .await?
                }
            };
            for id in theme_ids {
                for entry in site_texts::theme_translations(&mut conn, id).await? {
                    if overridden && entry.overridden.is_none() {
                        continue;
                    }
                    if regexp.is_match(&entry.id()) || regexp.is_match(entry.value()) {
                        results.push(Hit::Theme(entry.site_text()));
                    }
                }
            }
        }
        results.retain(|r| !site_texts::RESTRICTED_KEYS.contains(&r.id()));
        if !results.is_empty() {
            extras.insert(
                "regex".into(),
                Value::String(site_texts::search_regexp_string(&q)),
            );
        }
        let size = |h: &Hit| h.id().chars().count() + h.value().chars().count();
        crate::ruby::sort_by_block(&mut results, &mut |x, y| {
            if casecmp_eq(x.value(), &q) {
                std::cmp::Ordering::Less
            } else if casecmp_eq(y.value(), &q) {
                std::cmp::Ordering::Greater
            } else {
                size(x).cmp(&size(y))
            }
        });
    }

    let page = crate::params::string(&query, "page")
        .map(|p| crate::ruby::to_i(&p))
        .unwrap_or(0);
    if page < 0 {
        return Ok(super::search::invalid_parameters(&state, "page"));
    }
    let first = page as usize * PER_PAGE;
    let last = first + PER_PAGE;
    if results.len() > last {
        extras.insert("has_more".into(), Value::Bool(true));
    }
    let overridden_keys: Vec<String> = overrides.pairs.iter().map(|(k, _)| k.clone()).collect();
    let mut records = Vec::new();
    for hit in results.into_iter().skip(first).take(PER_PAGE) {
        match hit {
            Hit::Theme(v) => records.push(v),
            Hit::Core(r) => {
                records.push(site_texts::serialize(&mut conn, &r, Some(&overridden_keys)).await?)
            }
        }
    }
    Ok(Json(json!({
        "site_texts": records,
        "__rest_serializer": "1",
        "extras": extras,
    }))
    .into_response())
}

/// One search hit's value as found: a string, or a plural group.
enum Found {
    Text(String),
    Plural(Vec<(String, String)>),
}

/// `find_translations`
#[allow(clippy::too_many_arguments)]
async fn find_translations(
    conn: &mut PgConnection,
    overrides: &Overrides,
    regexp: &regex::Regex,
    overridden: bool,
    outdated: bool,
    untranslated: bool,
    only_selected_locale: bool,
    locale: &str,
) -> Result<Vec<Record>, AppError> {
    // I18n.search: English has nothing untranslated.
    if untranslated {
        return Ok(Vec::new());
    }
    let matches = |k: &str, v: &str| regexp.is_match(k) || regexp.is_match(v);
    let mut search: Vec<(String, String)> = Vec::new();
    let mut at: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    if !overridden {
        for (k, v) in site_texts::tree().strings_in_order() {
            if matches(k, v) {
                at.insert(k.to_string(), search.len());
                search.push((k.to_string(), v.to_string()));
            }
        }
    }
    for (k, v) in &overrides.pairs {
        if matches(k, v) {
            match at.get(k) {
                Some(&i) => search[i].1 = v.clone(),
                None => {
                    at.insert(k.clone(), search.len());
                    search.push((k.clone(), v.clone()));
                }
            }
        }
    }
    if outdated {
        let outdated_keys: Vec<String> = sqlx::query_scalar(
            "SELECT translation_key FROM translation_overrides WHERE status IN (1, 2)",
        )
        .fetch_all(&mut *conn)
        .await?;
        search.retain(|(k, _)| outdated_keys.contains(k));
    }
    // Plural forms grouped under their key, in first-seen order.
    let mut grouped: Vec<(String, Found)> = Vec::new();
    for (key, value) in search {
        let plural = key
            .rsplit_once('.')
            .filter(|(_, f)| matches!(*f, "zero" | "one" | "two" | "few" | "many" | "other"));
        match plural {
            Some((base, form)) => {
                let pos = grouped.iter().position(|(k, _)| k == base);
                match pos.map(|i| &mut grouped[i].1) {
                    Some(Found::Plural(forms)) => match forms.iter_mut().find(|(f, _)| f == form) {
                        Some(slot) => slot.1 = value,
                        None => forms.push((form.to_string(), value)),
                    },
                    Some(_) => {
                        return Err(Unsupported("a site text that is a string and a plural").into());
                    }
                    None => grouped.push((
                        base.to_string(),
                        Found::Plural(vec![(form.to_string(), value)]),
                    )),
                }
            }
            None => match grouped.iter().position(|(k, _)| *k == key) {
                Some(i) => grouped[i].1 = Found::Text(value),
                None => grouped.push((key, Found::Text(value))),
            },
        }
    }
    let mut results = Vec::new();
    for (key, found) in grouped {
        if !site_texts::tree().exists(&key) {
            continue;
        }
        match found {
            Found::Plural(forms) => {
                let complete = forms.len() == site_texts::PLURAL_KEYS.len()
                    && site_texts::PLURAL_KEYS
                        .iter()
                        .all(|p| forms.iter().any(|(f, _)| f == p));
                if complete {
                    // The hash itself comes back: each pair's last element,
                    // its value, is what record_for gets as the locale.
                    for (form, value) in forms {
                        results.push(site_texts::record_for(
                            overrides,
                            &format!("{key}.{form}"),
                            Some(Value::String(value.clone())),
                            &value,
                        )?);
                    }
                    continue;
                }
                let fallback = match site_texts::tree().default_value(&key)? {
                    Value::Object(m) => m,
                    Value::String(s) if !s.starts_with("Translation missing: ") => {
                        return Err(Unsupported("a plural site text of a string key").into());
                    }
                    _ => Map::new(),
                };
                for form in site_texts::PLURAL_KEYS {
                    let plural_key = format!("{key}.{form}");
                    match forms.iter().find(|(f, _)| f == form) {
                        Some((_, value)) => {
                            let value = if only_selected_locale {
                                overrides.translate(&plural_key)?
                            } else {
                                Value::String(value.clone())
                            };
                            results.push(site_texts::record_for(
                                overrides,
                                &plural_key,
                                Some(value),
                                locale,
                            )?);
                        }
                        None => {
                            let value = fallback
                                .get(form)
                                .or_else(|| fallback.get("other"))
                                .cloned()
                                .unwrap_or(Value::Null);
                            results.push(site_texts::record_for(
                                overrides,
                                &plural_key,
                                Some(value),
                                "en",
                            )?);
                        }
                    }
                }
            }
            Found::Text(value) => {
                let value = if only_selected_locale {
                    overrides.translate(&key)?
                } else {
                    Value::String(value)
                };
                results.push(site_texts::record_for(
                    overrides,
                    &key,
                    Some(value),
                    locale,
                )?);
            }
        }
    }
    Ok(results)
}

/// GET /admin/customize/site_texts/:id(.json)
pub async fn show(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(raw): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if let Some(r) = refuse_non_admin(&state, &guardian) {
        return Ok(r);
    }
    let Some(id) = route_id(&raw) else {
        return Ok(super::topics::not_found_response(&state));
    };
    if !json_request(&headers, &uri) {
        return Err(Unsupported("the admin site texts page").into());
    }
    let query = crate::params::parse_query(uri.query().unwrap_or(""));
    let locale = match fetch_locale(&state, crate::params::string(&query, "locale").as_deref())? {
        Ok(l) => l,
        Err(r) => return Ok(r),
    };
    let mut conn = state.pool.acquire().await?;
    if site_texts::theme_text_id(&id).is_some() {
        return Ok(match site_texts::find_theme_text(&mut conn, &id).await? {
            Some(entry) => theme_text_response(&entry),
            None => super::topics::not_found_response(&state),
        });
    }
    let overrides = Overrides::load(&mut conn, &locale).await?;
    let record = match find_site_text(&state, &overrides, &mut conn, &id, &locale).await? {
        Ok(r) => r,
        Err(response) => return Ok(response),
    };
    site_text_response(&mut conn, &record).await
}

fn theme_text_response(entry: &ThemeEntry) -> Response {
    Json(json!({"site_text": entry.site_text(), "__rest_serializer": "1"})).into_response()
}

async fn site_text_response(
    conn: &mut PgConnection,
    record: &Record,
) -> Result<Response, AppError> {
    let body = site_texts::serialize(conn, record, None).await?;
    Ok(Json(json!({"site_text": body, "__rest_serializer": "1"})).into_response())
}

/// `StaffActionLogger#log_site_text_change`
async fn log_change(
    conn: &mut PgConnection,
    acting_user_id: i32,
    subject: &str,
    new_value: Option<&str>,
    previous_value: Option<&str>,
) -> Result<(), AppError> {
    let action = crate::staff_action_logs::action_id("change_site_text")
        .ok_or(Unsupported("a staff action missing from enums.json"))?;
    sqlx::query(
        "INSERT INTO user_histories (action, acting_user_id, subject, previous_value, new_value, \
         admin_only, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, TRUE, now(), now())",
    )
    .bind(action)
    .bind(acting_user_id)
    .bind(subject)
    .bind(previous_value)
    .bind(new_value)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

fn string_value(v: &Value) -> Result<&str, AppError> {
    v.as_str()
        .ok_or_else(|| Unsupported("site texts of a hash key").into())
}

async fn write_prelude(
    state: &AppState,
    guardian: &crate::guardian::Guardian,
    headers: &HeaderMap,
    uri: &Uri,
    body: &Bytes,
    method: &str,
) -> Result<Result<Map<String, Value>, Response>, AppError> {
    if let Some(r) = refuse_non_admin(state, guardian) {
        return Ok(Err(r));
    }
    let p = crate::params::parse(uri.query(), headers, body);
    if !super::session::csrf_ok(state, headers, &pairs(&p), uri.path(), method) {
        return Ok(Err(super::session::bad_csrf()));
    }
    if !json_request(headers, uri) {
        return Ok(Err(super::session::render_empty()));
    }
    Ok(Ok(p))
}

/// PUT /admin/customize/site_texts/:id(.json)
pub async fn update(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(raw): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let Some(id) = route_id(&raw) else {
        return Ok(super::topics::not_found_response(&state));
    };
    let p = match write_prelude(&state, &guardian, &headers, &uri, &body, "PUT").await? {
        Ok(p) => p,
        Err(r) => return Ok(r),
    };
    let acting = guardian
        .user_id()
        .ok_or(Unsupported("an admin without a user"))?;
    let site_text = p.get("site_text").and_then(Value::as_object);
    let locale_param = site_text
        .and_then(|s| s.get("locale"))
        .and_then(crate::params::scalar);
    let locale = match fetch_locale(&state, locale_param.as_deref())? {
        Ok(l) => l,
        Err(r) => return Ok(r),
    };
    let mut tx = state.pool.begin().await?;
    if site_texts::theme_text_id(&id).is_some() {
        let response = update_theme_text(&state, &mut tx, acting, &id, site_text, false).await?;
        tx.commit().await?;
        return Ok(response);
    }
    let overrides = Overrides::load(&mut tx, &locale).await?;
    let mut record = match find_site_text(&state, &overrides, &mut tx, &id, &locale).await? {
        Ok(r) => r,
        Err(response) => return Ok(response),
    };
    let value: Option<String> = site_text
        .and_then(|s| s.get("value"))
        .and_then(crate::params::scalar);
    record.value = value.clone().map(Value::String).unwrap_or(Value::Null);
    let old_value = overrides.translate(&id)?;
    let old_value = string_value(&old_value)?.to_string();

    // TranslationOverride.upsert!
    if id.ends_with("_MF") {
        return Err(Unsupported("MessageFormat site texts (MessageFormat.compile)").into());
    }
    let sanitized = match &value {
        Some(v) => Some(crate::sanitize_field::sanitize_field(
            v,
            &["data-auto-route", "target"],
        )?),
        None => None,
    };
    let mut errors = Vec::new();
    let blank = sanitized.as_deref().is_none_or(crate::ruby::is_blank);
    if blank {
        errors.push("Value can't be blank".to_string());
    } else {
        let invalid =
            site_texts::invalid_interpolation_keys(&id, sanitized.as_deref().unwrap_or(""))?;
        if !invalid.is_empty() {
            let comma = state.i18n.t("word_connector.comma").unwrap_or(", ");
            let message = state
                .i18n
                .t_count(
                    "activerecord.errors.models.translation_overrides.attributes.value.invalid_interpolation_keys",
                    invalid.len() as i64,
                    &[("keys", &invalid.join(comma))],
                )
                .ok_or(Unsupported("a missing validation message"))?;
            errors.push(message);
        }
    }
    if !errors.is_empty() {
        return Ok((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"failed": "FAILED", "message": errors.join("\n\n")})),
        )
            .into_response());
    }
    let sanitized = sanitized.unwrap_or_default();
    let original = site_texts::tree().default_value(&site_texts::transform_pluralized_key(&id))?;
    let original = string_value(&original)?.to_string();
    match site_texts::find_override(&mut tx, &locale, &id).await? {
        None => {
            sqlx::query(
                "INSERT INTO translation_overrides (locale, translation_key, value, original_translation, \
                 status, created_at, updated_at) VALUES ($1, $2, $3, $4, 0, now(), now())",
            )
            .bind(&locale)
            .bind(&id)
            .bind(&sanitized)
            .bind(&original)
            .execute(&mut *tx)
            .await?;
        }
        Some(row) => {
            let (original, status) = site_texts::refreshed_status(&id, &sanitized)?;
            if row.value != sanitized
                || row.original_translation != original
                || row.status != status
            {
                sqlx::query(
                    "UPDATE translation_overrides SET value = $2, original_translation = $3, status = $4, \
                     updated_at = now() WHERE id = $1",
                )
                .bind(row.id)
                .bind(&sanitized)
                .bind(&original)
                .bind(status)
                .execute(&mut *tx)
                .await?;
            }
        }
    }
    log_change(&mut tx, acting, &id, value.as_deref(), Some(&old_value)).await?;
    let parts: Vec<&str> = id.split('.').collect();
    if let Some(badge_id) = site_texts::system_badge_id(&id)
        && parts.first() == Some(&"badges")
        && parts.get(2) == Some(&"name")
    {
        crate::jobs::enqueue(
            &mut tx,
            crate::admin_badges::BULK_USER_TITLE_UPDATE,
            json!({"new_title": value, "granted_badge_id": badge_id, "action": "update"}),
        )
        .await?;
    }
    let response = site_text_response(&mut tx, &record).await?;
    tx.commit().await?;
    Ok(response)
}

/// `update_theme_text`: the theme's override written (or removed when it
/// is the default, or on revert), the log, the site text.
async fn update_theme_text(
    state: &AppState,
    conn: &mut PgConnection,
    acting: i32,
    id: &str,
    site_text: Option<&Map<String, Value>>,
    revert: bool,
) -> Result<Response, AppError> {
    let Some(entry) = site_texts::find_theme_text(conn, id).await? else {
        return Ok(super::topics::not_found_response(state));
    };
    let old_value = entry.value().to_string();
    let theme_id = entry.theme_id;
    let mut changed = false;
    if revert {
        changed = entry.overridden.is_some();
        sqlx::query(
            "DELETE FROM theme_translation_overrides WHERE theme_id = $1 AND locale = 'en' \
             AND translation_key = $2",
        )
        .bind(theme_id)
        .bind(&entry.key)
        .execute(&mut *conn)
        .await?;
    } else {
        let Some(site_text) = site_text.filter(|s| !s.is_empty()) else {
            return Ok(super::accounts::param_missing("site_text"));
        };
        let Some(value) = site_text.get("value").and_then(Value::as_str) else {
            return Ok(super::search::invalid_parameters(state, "value"));
        };
        // theme.with_lock
        sqlx::query("SELECT id FROM themes WHERE id = $1 FOR UPDATE")
            .bind(theme_id)
            .execute(&mut *conn)
            .await?;
        if value == entry.default {
            if entry.overridden.is_some() {
                changed = true;
                sqlx::query(
                    "DELETE FROM theme_translation_overrides WHERE theme_id = $1 AND locale = 'en' \
                     AND translation_key = $2",
                )
                .bind(theme_id)
                .bind(&entry.key)
                .execute(&mut *conn)
                .await?;
            }
        } else if let Some(current) = &entry.overridden {
            if current != value {
                changed = true;
                sqlx::query(
                    "UPDATE theme_translation_overrides SET value = $3, updated_at = now() \
                     WHERE theme_id = $1 AND locale = 'en' AND translation_key = $2",
                )
                .bind(theme_id)
                .bind(&entry.key)
                .bind(value)
                .execute(&mut *conn)
                .await?;
            }
        } else {
            changed = true;
            sqlx::query(
                "INSERT INTO theme_translation_overrides (theme_id, locale, translation_key, value, \
                 created_at, updated_at) VALUES ($1, 'en', $2, $3, now(), now())",
            )
            .bind(theme_id)
            .bind(&entry.key)
            .bind(value)
            .execute(&mut *conn)
            .await?;
        }
    }
    // ThemeTranslationOverride after_commit: the theme's translation
    // fields to be baked again.
    if changed {
        sqlx::query(
            "UPDATE theme_fields SET value_baked = NULL WHERE theme_id = $1 AND target_id = 4",
        )
        .bind(theme_id)
        .execute(&mut *conn)
        .await?;
    }
    let entry = site_texts::find_theme_text(conn, id)
        .await?
        .ok_or(Unsupported("a theme text gone once written"))?;
    log_change(
        conn,
        acting,
        &entry.id(),
        Some(entry.value()),
        Some(&old_value),
    )
    .await?;
    Ok(theme_text_response(&entry))
}

/// DELETE /admin/customize/site_texts/:id(.json) (revert)
pub async fn revert(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(raw): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let Some(id) = route_id(&raw) else {
        return Ok(super::topics::not_found_response(&state));
    };
    let p = match write_prelude(&state, &guardian, &headers, &uri, &body, "DELETE").await? {
        Ok(p) => p,
        Err(r) => return Ok(r),
    };
    let acting = guardian
        .user_id()
        .ok_or(Unsupported("an admin without a user"))?;
    let locale = match fetch_locale(&state, crate::params::string(&p, "locale").as_deref())? {
        Ok(l) => l,
        Err(r) => return Ok(r),
    };
    let mut tx = state.pool.begin().await?;
    if site_texts::theme_text_id(&id).is_some() {
        let response = update_theme_text(&state, &mut tx, acting, &id, None, true).await?;
        tx.commit().await?;
        return Ok(response);
    }
    let overrides = Overrides::load(&mut tx, &locale).await?;
    let record = match find_site_text(&state, &overrides, &mut tx, &id, &locale).await? {
        Ok(r) => r,
        Err(response) => return Ok(response),
    };
    let old_text = string_value(&overrides.translate(&record.id)?)?.to_string();
    sqlx::query("DELETE FROM translation_overrides WHERE locale = $1 AND translation_key = $2")
        .bind(&locale)
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    let overrides = Overrides::load(&mut tx, &locale).await?;
    let record = match find_site_text(&state, &overrides, &mut tx, &id, &locale).await? {
        Ok(r) => r,
        Err(response) => return Ok(response),
    };
    let new_text = string_value(&record.value)?.to_string();
    log_change(&mut tx, acting, &id, Some(&new_text), Some(&old_text)).await?;
    if let Some(badge_id) = site_texts::system_badge_id(&id) {
        crate::jobs::enqueue(
            &mut tx,
            crate::admin_badges::BULK_USER_TITLE_UPDATE,
            json!({"granted_badge_id": badge_id, "action": "reset"}),
        )
        .await?;
    }
    let response = site_text_response(&mut tx, &record).await?;
    tx.commit().await?;
    Ok(response)
}

/// PUT /admin/customize/site_texts/:id/dismiss_outdated(.json)
pub async fn dismiss_outdated(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(raw): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let Some(id) = route_id(&raw) else {
        return Ok(super::topics::not_found_response(&state));
    };
    let p = match write_prelude(&state, &guardian, &headers, &uri, &body, "PUT").await? {
        Ok(p) => p,
        Err(r) => return Ok(r),
    };
    let locale = match fetch_locale(&state, crate::params::string(&p, "locale").as_deref())? {
        Ok(l) => l,
        Err(r) => return Ok(r),
    };
    let mut conn = state.pool.acquire().await?;
    let Some(row) = site_texts::find_override(&mut conn, &locale, &id).await? else {
        return Ok(super::topics::not_found_response(&state));
    };
    if row.status != 1 {
        return Ok((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"failed": "FAILED", "message": "Can only dismiss outdated translations"})),
        )
            .into_response());
    }
    // make_up_to_date!: update_attribute!, which runs refresh_status.
    let (original, status) = site_texts::refreshed_status(&row.translation_key, &row.value)?;
    sqlx::query(
        "UPDATE translation_overrides SET original_translation = $2, status = $3, updated_at = now() \
         WHERE id = $1",
    )
    .bind(row.id)
    .bind(original)
    .bind(status)
    .execute(&mut *conn)
    .await?;
    Ok(Json(json!({"success": "OK"})).into_response())
}

/// GET /admin/customize/reseed(.json): SeedData's categories and topics
/// with whether each is still as seeded.
pub async fn reseed_options(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if let Some(r) = refuse_non_admin(&state, &guardian) {
        return Ok(r);
    }
    if !json_request(&headers, &uri) {
        return Err(Unsupported("the admin site texts page").into());
    }
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    if settings.get("default_locale")?.to_s() != "en" {
        return Err(Unsupported("reseeding in locales other than English").into());
    }
    let mut categories = Vec::new();
    for name in [
        "uncategorized_category_id",
        "meta_category_id",
        "staff_category_id",
        "general_category_id",
    ] {
        let id = settings.get(name)?.to_i();
        if id <= 0 {
            continue;
        }
        let category: Option<(String, Option<i32>)> =
            sqlx::query_as("SELECT name, topic_id FROM categories WHERE id = $1")
                .bind(id as i32)
                .fetch_optional(&mut *conn)
                .await?;
        let Some((cat_name, topic_id)) = category else {
            continue;
        };
        // unchanged?: the description post's last editor is the system user.
        let editor: Option<Option<i32>> = match topic_id {
            Some(t) => {
                sqlx::query_scalar(
                    "SELECT last_editor_id FROM posts WHERE topic_id = $1 AND post_number = 1 \
                 AND deleted_at IS NULL",
                )
                .bind(t)
                .fetch_optional(&mut *conn)
                .await?
            }
            None => None,
        };
        let selected = match editor {
            Some(e) => e == Some(-1),
            None => true,
        };
        categories.push(json!({"id": name, "name": cat_name, "selected": selected}));
    }
    let mut topics = Vec::new();
    for name in [
        "guidelines_topic_id",
        "welcome_topic_id",
        "admin_quick_start_topic_id",
    ] {
        let id = settings.get(name)?.to_i();
        if id < 1 {
            continue;
        }
        let post: Option<(Option<i32>, Option<i32>, String)> = sqlx::query_as(
            "SELECT p.last_editor_id, p.deleted_by_id, t.title FROM posts p JOIN topics t ON t.id = p.topic_id \
             WHERE p.topic_id = $1 AND p.post_number = 1 AND p.deleted_at IS NULL ORDER BY p.id LIMIT 1",
        )
        .bind(id as i32)
        .fetch_optional(&mut *conn)
        .await?;
        let Some((editor, deleted_by, title)) = post else {
            continue;
        };
        let selected = editor == Some(-1) && deleted_by.is_none_or(|d| d == -1);
        topics.push(json!({"id": name, "name": title, "selected": selected}));
    }
    Ok(Json(json!({"categories": categories, "topics": topics})).into_response())
}

/// POST /admin/customize/reseed: SeedData rewrites the seeded categories
/// and topics.
pub async fn reseed() -> Result<Response, AppError> {
    Err(Unsupported("reseeding categories and topics").into())
}
