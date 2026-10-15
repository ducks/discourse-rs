//! Admin::WatchedWordsController's writes and lists: WatchedWord and
//! WatchedWordGroup (their normalization, validations and saves), the
//! staff action logs, and WatchedWordListSerializer.
//!
//! Refused: admin regexps the port can't read as Ruby and JavaScript do
//! (see `word_watcher`).

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::i18n::I18n;
use crate::site_settings::SiteSettings;
use crate::word_watcher::{self as ww, ACTIONS, LINK, MAX_WORDS_PER_ACTION, REPLACE, TAG};
use crate::{AppError, Unsupported};

/// `WatchedWord.normalize_word`: a Ruby regexp's `(?-mix:...)` wrapper
/// dropped (with the last character), stripped, runs of `*` squeezed.
pub fn normalize_word(word: &str) -> String {
    let word = match word.strip_prefix("(?-mix:") {
        Some(rest) => {
            let mut chars: Vec<char> = rest.chars().collect();
            chars.pop();
            chars.into_iter().collect()
        }
        None => word.to_string(),
    };
    let stripped = word
        .trim_matches(|c: char| matches!(c, '\0' | '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r' | ' '));
    let mut out = String::with_capacity(stripped.len());
    for c in stripped.chars() {
        if c == '*' && out.ends_with('*') {
            continue;
        }
        out.push(c);
    }
    out
}

/// The attributes `create_or_update_word` is given.
pub struct WordParams<'a> {
    pub word: &'a str,
    pub replacement: Option<&'a str>,
    /// `action_key`: None (and so no action) for an unknown one.
    pub action: Option<i32>,
    pub case_sensitive: Option<bool>,
    pub html: bool,
}

/// A watched word row.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Row {
    pub id: i32,
    pub word: String,
    pub action: i32,
    pub replacement: Option<String>,
    pub case_sensitive: bool,
    pub html: bool,
    pub watched_word_group_id: Option<i64>,
}

const ROW_COLUMNS: &str =
    "id, word, action, replacement, case_sensitive, html, watched_word_group_id";

/// What `create_or_update_word` left: the saved row, or its errors.
pub enum Saved {
    Row(Row),
    Invalid(Vec<String>),
}

/// `errors.full_messages` for an attribute without a translated name.
fn attribute_error(_: &I18n, attribute: &str, message: &str) -> String {
    format!("{attribute} {message}")
}

fn t(i18n: &I18n, key: &str) -> String {
    i18n.t(key).unwrap_or_default().to_string()
}

fn too_long(i18n: &I18n, count: i64) -> String {
    i18n.t_with(
        "errors.messages.too_long.other",
        &[("count", &count.to_string())],
    )
    .unwrap_or_default()
}

/// `WatchedWord.create_or_update_word(params)`: the word found by
/// `for(word:)` or a new one, its attributes set, validated and saved.
pub async fn create_or_update_word(
    conn: &mut PgConnection,
    s: &SiteSettings,
    i18n: &I18n,
    base_url: &str,
    p: &WordParams<'_>,
    group_id: Option<i64>,
) -> Result<Saved, AppError> {
    let word = normalize_word(p.word);
    let existing: Option<Row> = sqlx::query_as(&format!(
        "SELECT {ROW_COLUMNS} FROM watched_words \
         WHERE (word ILIKE $1 AND case_sensitive = 'f') OR (word LIKE $1 AND case_sensitive = 't') \
         ORDER BY id LIMIT 1"
    ))
    .bind(&word)
    .fetch_optional(&mut *conn)
    .await?;
    let mut row = existing.clone().unwrap_or(Row {
        id: 0,
        word: word.clone(),
        action: 0,
        replacement: None,
        case_sensitive: false,
        html: false,
        watched_word_group_id: None,
    });
    if let Some(r) = p.replacement {
        row.replacement = Some(r.to_string());
    }
    // action_key=: nil for an unknown key.
    row.action = p.action.unwrap_or(0);
    if let Some(cs) = p.case_sensitive {
        row.case_sensitive = cs;
    }
    if p.html {
        row.html = true;
    }
    row.watched_word_group_id = group_id;
    // before_validation
    row.word = normalize_word(&row.word);
    if let Some(r) = row
        .replacement
        .as_deref()
        .filter(|r| !crate::ruby::is_blank(r))
    {
        row.replacement = Some(normalize_word(r));
    }
    if row.action == LINK {
        let r = row.replacement.clone().unwrap_or_default();
        if !(r.starts_with("http://") || r.starts_with("https://")) {
            let slash = if r.starts_with('/') { "" } else { "/" };
            row.replacement = Some(format!("{base_url}{slash}{r}"));
        }
    }
    // validations, in their order
    let mut errors = Vec::new();
    if crate::ruby::is_blank(&row.word) {
        errors.push(attribute_error(
            i18n,
            "Word",
            &t(i18n, "errors.messages.blank"),
        ));
    } else {
        let taken: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM watched_words WHERE word = $1 AND id <> $2)",
        )
        .bind(&row.word)
        .bind(row.id)
        .fetch_one(&mut *conn)
        .await?;
        if taken {
            errors.push(attribute_error(
                i18n,
                "Word",
                &t(i18n, "errors.messages.taken"),
            ));
        }
    }
    if row.word.chars().count() > 100 {
        errors.push(attribute_error(i18n, "Word", &too_long(i18n, 100)));
    }
    if row
        .replacement
        .as_ref()
        .is_some_and(|r| r.chars().count() > 100)
    {
        errors.push(attribute_error(i18n, "Replacement", &too_long(i18n, 100)));
    }
    if row.action == 0 {
        errors.push(attribute_error(
            i18n,
            "Action",
            &t(i18n, "errors.messages.blank"),
        ));
    }
    if row.action == LINK && !uri_regexp_matches(row.replacement.as_deref().unwrap_or("")) {
        errors.push(t(
            i18n,
            "activerecord.errors.models.watched_word.attributes.base.invalid_url",
        ));
    }
    // replacement_is_tag_list: existing tags, each named once.
    if row.action == TAG {
        let list: Vec<String> = match row.replacement.as_deref() {
            // String#split(","): trailing empty pieces dropped.
            Some(r) => {
                let mut parts: Vec<String> = r.split(',').map(str::to_string).collect();
                while parts.last().is_some_and(String::is_empty) {
                    parts.pop();
                }
                parts
            }
            None => Vec::new(),
        };
        let found: i64 = sqlx::query_scalar("SELECT count(*) FROM tags WHERE name = ANY($1)")
            .bind(&list)
            .fetch_one(&mut *conn)
            .await?;
        if list.is_empty() || found == 0 || list.len() as i64 != found {
            errors.push(t(
                i18n,
                "activerecord.errors.models.watched_word.attributes.base.invalid_tag_list",
            ));
        }
    }
    if row.html
        && row
            .replacement
            .as_deref()
            .is_some_and(|r| !crate::ruby::is_blank(r))
        && row.action != REPLACE
    {
        errors.push(t(
            i18n,
            "activerecord.errors.models.watched_word.attributes.base.invalid_html",
        ));
    }
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM watched_words WHERE action = $1")
        .bind(row.action)
        .fetch_one(&mut *conn)
        .await?;
    if count >= MAX_WORDS_PER_ACTION {
        errors.push(attribute_error(
            i18n,
            "Word",
            &t(
                i18n,
                "activerecord.errors.models.watched_word.attributes.word.too_many",
            ),
        ));
    }
    let _ = s;
    if !errors.is_empty() {
        return Ok(Saved::Invalid(errors));
    }
    // A new word is inserted (no id), an existing one updated.
    let sql = if existing.is_some() {
        format!(
            "UPDATE watched_words SET word = $2, action = $3, replacement = $4, case_sensitive = $5, \
                                      html = $6, watched_word_group_id = $7, updated_at = clock_timestamp() \
             WHERE id = $1 RETURNING {ROW_COLUMNS}"
        )
    } else {
        format!(
            "INSERT INTO watched_words (word, action, replacement, case_sensitive, html, \
                                        watched_word_group_id, created_at, updated_at) \
             SELECT $2, $3, $4, $5, $6, $7, clock_timestamp(), clock_timestamp() WHERE $1::int IS NULL \
             RETURNING {ROW_COLUMNS}"
        )
    };
    let saved: Row = sqlx::query_as(&sql)
        .bind(existing.as_ref().map(|r| r.id))
        .bind(&row.word)
        .bind(row.action)
        .bind(&row.replacement)
        .bind(row.case_sensitive)
        .bind(row.html)
        .bind(row.watched_word_group_id)
        .fetch_one(&mut *conn)
        .await?;
    Ok(Saved::Row(saved))
}

/// `replacement =~ URI.regexp`: somewhere in the text, a scheme followed
/// by a colon (URI.regexp matches any absolute URI, unanchored).
fn uri_regexp_matches(text: &str) -> bool {
    static SCHEME: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"[A-Za-z][A-Za-z0-9+\-.]*:").unwrap());
    SCHEME.is_match(text)
}

/// The outcome of the create route.
pub enum Created {
    /// The group's words.
    Words(Vec<Row>),
    /// `record_invalid`: the group's errors.
    Invalid(Vec<String>),
}

/// `WatchedWordGroup#create_or_update_members` then the creation log:
/// each word saved on its own (no group), then the group saved with them
/// (or every write rolled back with its errors).
#[allow(clippy::too_many_arguments)]
pub async fn create_group(
    conn: &mut PgConnection,
    s: &SiteSettings,
    i18n: &I18n,
    base_url: &str,
    acting_user_id: i32,
    words: &[String],
    action_key: &str,
    replacement: Option<&str>,
    case_sensitive: Option<bool>,
    html: bool,
) -> Result<Created, AppError> {
    let action = ww::action_id(action_key);
    let mut saved = Vec::new();
    let mut word_errors = Vec::new();
    for w in words {
        let p = WordParams {
            word: w,
            replacement,
            action,
            case_sensitive,
            html,
        };
        match create_or_update_word(&mut *conn, s, i18n, base_url, &p, None).await? {
            Saved::Row(row) => saved.push(row),
            Saved::Invalid(errors) => word_errors.push(errors),
        }
    }
    // WatchedWordGroup validations: action presence, the words' errors,
    // none at all; then the association's own "is invalid".
    let mut errors = Vec::new();
    if action.is_none() {
        errors.push(attribute_error(
            i18n,
            "Action",
            &t(i18n, "errors.messages.blank"),
        ));
    }
    for e in &word_errors {
        errors.extend(e.iter().cloned());
    }
    if words.is_empty() {
        errors.push(attribute_error(
            i18n,
            "Watched words",
            &t(i18n, "errors.messages.empty"),
        ));
    }
    if !word_errors.is_empty() {
        errors.push(attribute_error(
            i18n,
            "Watched words",
            &t(i18n, "errors.messages.invalid"),
        ));
    }
    if !errors.is_empty() {
        return Ok(Created::Invalid(errors));
    }
    let action = action.unwrap_or_default();
    let group_id: i64 = sqlx::query_scalar(
        "INSERT INTO watched_word_groups (action, created_at, updated_at) \
         VALUES ($1, clock_timestamp(), clock_timestamp()) RETURNING id",
    )
    .bind(action)
    .fetch_one(&mut *conn)
    .await?;
    let mut rows = Vec::new();
    for row in saved {
        let row: Row = sqlx::query_as(&format!(
            "UPDATE watched_words SET watched_word_group_id = $2, updated_at = clock_timestamp() \
             WHERE id = $1 RETURNING {ROW_COLUMNS}"
        ))
        .bind(row.id)
        .bind(group_id)
        .fetch_one(&mut *conn)
        .await?;
        rows.push(row);
    }
    let key = ww::action_key(action).unwrap_or_default();
    let names: Vec<&str> = rows.iter().map(|r| r.word.as_str()).collect();
    log(
        &mut *conn,
        "create_watched_word_group",
        acting_user_id,
        &format!("{key} \u{2192} {}", names.join(", ")),
        key,
    )
    .await?;
    Ok(Created::Words(rows))
}

/// `WatchedWord#action_log_details`
fn word_details(row: &Row) -> String {
    match row
        .replacement
        .as_deref()
        .filter(|r| !crate::ruby::is_blank(r))
    {
        Some(r) => format!("{} \u{2192} {r}", row.word),
        None => row.word.clone(),
    }
}

/// StaffActionLogger's watched word entries.
async fn log(
    conn: &mut PgConnection,
    action: &str,
    acting_user_id: i32,
    details: &str,
    context: &str,
) -> Result<(), AppError> {
    let id = crate::staff_action_logs::action_id(action)
        .ok_or(Unsupported("an unknown UserHistory action"))?;
    sqlx::query(
        "INSERT INTO user_histories (action, acting_user_id, details, context, admin_only, \
                                     created_at, updated_at) \
         VALUES ($1, $2, $3, $4, FALSE, clock_timestamp(), clock_timestamp())",
    )
    .bind(id)
    .bind(acting_user_id)
    .bind(details)
    .bind(context)
    .execute(conn)
    .await?;
    Ok(())
}

/// `destroy`: a word alone in its group takes the group with it (logged
/// as the group, its words already gone); otherwise the word. False for
/// an unknown id.
pub async fn destroy(
    conn: &mut PgConnection,
    acting_user_id: i32,
    id: i32,
) -> Result<bool, AppError> {
    let row: Option<Row> = sqlx::query_as(&format!(
        "SELECT {ROW_COLUMNS} FROM watched_words WHERE id = $1"
    ))
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(row) = row else {
        return Ok(false);
    };
    let key = ww::action_key(row.action).unwrap_or_default().to_string();
    if let Some(group_id) = row.watched_word_group_id {
        let group: Option<(i32, i64)> = sqlx::query_as(
            "SELECT g.action, (SELECT COUNT(*) FROM watched_words w WHERE w.watched_word_group_id = g.id) \
             FROM watched_word_groups g WHERE g.id = $1",
        )
        .bind(group_id)
        .fetch_optional(&mut *conn)
        .await?;
        if let Some((group_action, 1)) = group {
            sqlx::query("DELETE FROM watched_words WHERE watched_word_group_id = $1")
                .bind(group_id)
                .execute(&mut *conn)
                .await?;
            sqlx::query("DELETE FROM watched_word_groups WHERE id = $1")
                .bind(group_id)
                .execute(&mut *conn)
                .await?;
            let group_key = ww::action_key(group_action).unwrap_or_default();
            log(
                &mut *conn,
                "delete_watched_word_group",
                acting_user_id,
                &format!("{group_key} \u{2192} "),
                group_key,
            )
            .await?;
            return Ok(true);
        }
    }
    sqlx::query("DELETE FROM watched_words WHERE id = $1")
        .bind(id)
        .execute(&mut *conn)
        .await?;
    log(
        &mut *conn,
        "watched_word_destroy",
        acting_user_id,
        &word_details(&row),
        &key,
    )
    .await?;
    Ok(true)
}

/// `clear_all`: each of the action's words destroyed and logged, by id
/// (groups stay).
pub async fn clear_all(
    conn: &mut PgConnection,
    acting_user_id: i32,
    action: i32,
) -> Result<(), AppError> {
    let rows: Vec<Row> = sqlx::query_as(&format!(
        "SELECT {ROW_COLUMNS} FROM watched_words WHERE action = $1 ORDER BY id"
    ))
    .bind(action)
    .fetch_all(&mut *conn)
    .await?;
    let key = ww::action_key(action).unwrap_or_default();
    for row in rows {
        sqlx::query("DELETE FROM watched_words WHERE id = $1")
            .bind(row.id)
            .execute(&mut *conn)
            .await?;
        log(
            &mut *conn,
            "watched_word_destroy",
            acting_user_id,
            &word_details(&row),
            key,
        )
        .await?;
    }
    Ok(())
}

/// Every word, by action then word (tag words left out without tagging).
pub async fn all(conn: &mut PgConnection, tagging: bool) -> Result<Vec<Row>, AppError> {
    Ok(sqlx::query_as(&format!(
        "SELECT {ROW_COLUMNS} FROM watched_words WHERE $1 OR action <> $2 ORDER BY action, word"
    ))
    .bind(tagging)
    .bind(TAG)
    .fetch_all(conn)
    .await?)
}

/// WatchedWordListSerializer over `words`.
pub async fn list_json(
    conn: &mut PgConnection,
    s: &SiteSettings,
    words: &[Row],
) -> Result<Value, AppError> {
    let regex = ww::regex_mode(s)?;
    let tagging = s.get("tagging_enabled")?.truthy();
    let actions: Vec<&str> = ACTIONS
        .iter()
        .map(|(k, _)| *k)
        .filter(|k| tagging || *k != "tag")
        .collect();
    let words: Vec<Value> = words.iter().map(|w| word_json(w, regex)).collect();
    let mut compiled = Map::new();
    for key in &actions {
        let action = ww::action_id(key).unwrap_or_default();
        compiled.insert(
            (*key).to_string(),
            ww::serialize(&ww::compiled_for_action(&mut *conn, s, action).await?),
        );
    }
    Ok(json!({
        "actions": actions,
        "words": words,
        "compiled_regular_expressions": compiled,
    }))
}

/// WatchedWordSerializer
fn word_json(row: &Row, regex: bool) -> Value {
    let mut out = Map::new();
    out.insert("id".into(), json!(row.id));
    out.insert("word".into(), json!(row.word));
    out.insert(
        "regexp".into(),
        json!(ww::word_regexp(&row.word, true, regex)),
    );
    if ww::has_replacement(row.action) {
        out.insert("replacement".into(), json!(row.replacement));
    }
    out.insert("action".into(), json!(ww::action_key(row.action)));
    out.insert("case_sensitive".into(), json!(row.case_sensitive));
    out.insert(
        "watched_word_group_id".into(),
        json!(row.watched_word_group_id),
    );
    if row.action == REPLACE && row.html {
        out.insert("html".into(), json!(true));
    }
    Value::Object(out)
}

/// `download`: a BOM, then `word,replacement,case_sensitive` per word of
/// the action (CSV.generate).
pub async fn csv(conn: &mut PgConnection, action: i32) -> Result<String, AppError> {
    let rows: Vec<(String, Option<String>, bool)> = sqlx::query_as(
        "SELECT word, replacement, case_sensitive FROM watched_words WHERE action = $1",
    )
    .bind(action)
    .fetch_all(conn)
    .await?;
    let mut out = String::from("\u{FEFF}");
    for (word, replacement, cs) in rows {
        out.push_str(&csv_field(&word));
        out.push(',');
        out.push_str(&replacement.as_deref().map(csv_field).unwrap_or_default());
        out.push(',');
        out.push_str(if cs { "true" } else { "false" });
        out.push('\n');
    }
    Ok(out)
}

/// Ruby CSV's quoting: a field with a comma, quote, CR or LF is quoted,
/// quotes doubled.
fn csv_field(field: &str) -> String {
    if field.contains([',', '"', '\r', '\n']) {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_like_rails() {
        assert_eq!(normalize_word("(?-mix:abc)  "), "abc)");
        assert_eq!(normalize_word("  (?-mix:Loud**)  "), "(?-mix:Loud*)");
        assert_eq!(normalize_word("(?-mix:Loud**)"), "Loud*");
        assert_eq!(normalize_word(" a***b "), "a*b");
    }
}
