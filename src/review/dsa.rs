//! The review queue's DSA statement classification data (Discourse
//! 6f5643427d1): DsaClassificationSerializer's shared options, sent once
//! as the list's `extras.dsa_classification`, and each reviewable's
//! pending statement ids (ReviewableSerializer#dsa_classification).
//! Classifying (DsaStatementsController#classify) is not ported.

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgConnection;

use crate::AppError;
use crate::guardian::Guardian;
use crate::site_settings::SiteSettings;
use crate::web_hooks::Payloads as Ctx;

/// `DsaStatementOfRecord::CATEGORIES`
const CATEGORIES: [&str; 16] = [
    "ANIMAL_WELFARE",
    "CONSUMER_INFORMATION",
    "CYBER_VIOLENCE",
    "CYBER_VIOLENCE_AGAINST_WOMEN",
    "DATA_PROTECTION_AND_PRIVACY_VIOLATIONS",
    "ILLEGAL_OR_HARMFUL_SPEECH",
    "INTELLECTUAL_PROPERTY_INFRINGEMENTS",
    "NEGATIVE_EFFECTS_ON_CIVIC_DISCOURSE_OR_ELECTIONS",
    "NOT_SPECIFIED_NOTICE",
    "OTHER_VIOLATION_TC",
    "PROTECTION_OF_MINORS",
    "RISK_FOR_PUBLIC_SECURITY",
    "SCAMS_AND_FRAUD",
    "SELF_HARM",
    "UNSAFE_AND_PROHIBITED_PRODUCTS",
    "VIOLENCE",
];

/// `DsaStatementOfRecord::TERMS_VIOLATION_CATEGORY`
const TERMS_VIOLATION_CATEGORY: &str = "STATEMENT_CATEGORY_OTHER_VIOLATION_TC";

/// `JSON.parse(SiteSetting.dsa_community_rules)`
fn rules(s: &SiteSettings) -> Result<Vec<Value>, AppError> {
    let raw = s.get("dsa_community_rules")?.to_s();
    Ok(match serde_json::from_str::<Value>(&raw) {
        Ok(Value::Array(rules)) => rules,
        _ => Vec::new(),
    })
}

/// `DsaStatementOfRecord.classification_enabled?`
pub fn classification_enabled(s: &SiteSettings) -> Result<bool, AppError> {
    Ok(s.get("dsa_reporting_enabled")?.truthy() && !rules(s)?.is_empty())
}

/// ActiveSupport's `to_json`: JSON with `<`, `>`, `&`, U+2028 and U+2029
/// escaped.
fn as_json_string(v: &Value) -> String {
    v.to_string()
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

/// A community rule's id: the SHA-256 of `[ground, explanation].to_json`.
fn rule_id(rule: &Value) -> String {
    let pair = json!([
        rule.get("ground").cloned().unwrap_or(Value::Null),
        rule.get("explanation").cloned().unwrap_or(Value::Null),
    ]);
    Sha256::digest(as_json_string(&pair).as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// `DsaClassificationSerializer.new({}, scope: guardian)`: the rules,
/// categories and default category while classification is enabled (the
/// queue's viewers can always see it), else nothing.
pub fn classification_options(ctx: &Ctx<'_>) -> Result<Value, AppError> {
    if !classification_enabled(ctx.settings)? {
        return Ok(json!({}));
    }
    let community_rules: Vec<Value> = rules(ctx.settings)?
        .iter()
        .map(|rule| {
            // rule.merge(id =>).slice(id, ground, explanation)
            let mut out = serde_json::Map::new();
            out.insert("id".into(), json!(rule_id(rule)));
            for key in ["ground", "explanation"] {
                if let Some(v) = rule.get(key) {
                    out.insert(key.into(), v.clone());
                }
            }
            Value::Object(out)
        })
        .collect();
    let categories: Vec<Value> = CATEGORIES
        .iter()
        .map(|c| {
            let id = format!("STATEMENT_CATEGORY_{c}");
            let name = ctx
                .i18n
                .t(&format!("dsa_statements.categories.{id}"))
                .unwrap_or(&id)
                .to_string();
            json!({ "id": id, "name": name })
        })
        .collect();
    Ok(json!({
        "community_rules": community_rules,
        "categories": categories,
        "default_category": TERMS_VIOLATION_CATEGORY,
    }))
}

/// ReviewableSerializer#dsa_classification: the pending statement ids,
/// for staff while classification is enabled and the reviewable has
/// statements.
pub async fn statement_ids(
    conn: &mut PgConnection,
    s: &SiteSettings,
    guardian: &Guardian,
    reviewable_id: i64,
) -> Result<Option<Value>, AppError> {
    if !guardian.is_staff() || !classification_enabled(s)? {
        return Ok(None);
    }
    let statements: Vec<(i64, i32)> = sqlx::query_as(
        "SELECT id::bigint, status::int FROM dsa_statement_of_records WHERE reviewable_id = $1 ORDER BY ctid",
    )
    .bind(reviewable_id)
    .fetch_all(conn)
    .await?;
    if statements.is_empty() {
        return Ok(None);
    }
    let pending: Vec<i64> = statements
        .into_iter()
        .filter(|(_, status)| *status == 0)
        .map(|(id, _)| id)
        .collect();
    Ok(Some(json!({ "statement_ids": pending })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_ids_hash_the_active_support_json() {
        let rule = json!({ "ground": "a<b", "explanation": "c" });
        let expected: String = Sha256::digest("[\"a\\u003cb\",\"c\"]".as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(rule_id(&rule), expected);
    }
}
