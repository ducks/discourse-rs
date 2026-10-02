//! The sample emails in parity/incoming_mail/emails, parsed and cleaned
//! as Email::Receiver and Email::Cleaner do on the reference
//! (parity/incoming_mail/expected.json, scripts/record-incoming-mail).

use discourse_rs::email::incoming::{self, Incoming};
use discourse_rs::email::receiver;
use discourse_rs::site_settings::{Definitions, SiteSettings};
use serde_json::{Value, json};

fn dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("parity/incoming_mail")
}

/// The fields this layer reads, in the recording's shape.
fn fields(m: &Incoming) -> Value {
    let defs = Definitions::vendored().unwrap();
    let globals = discourse_rs::config::GlobalSettings::from_vars(Vec::<(String, String)>::new());
    let s = SiteSettings::resolve(&defs, Vec::new(), &globals).unwrap();
    let i18n = discourse_rs::i18n::I18n::vendored().unwrap();
    let from = m.from.as_ref().map(|(a, _)| a.as_str());
    let body = receiver::select_body(m, &s, "http://localhost:3042").map_err(|e| format!("{e:?}"));
    let part = |i: usize| {
        body.as_ref()
            .map(|b| {
                b.as_ref()
                    .map(|t| match i {
                        0 => json!(t.0),
                        1 => json!(t.1),
                        _ => json!(t.2),
                    })
                    .unwrap_or(Value::Null)
            })
            .map_err(Clone::clone)
    };
    let reply_ids: Vec<String> = {
        let mut ids: Vec<String> = Vec::new();
        for id in m.in_reply_to.iter().chain(&m.references) {
            if !id.is_empty() && !ids.contains(id) {
                ids.push(id.clone());
            }
        }
        ids.truncate(5);
        ids
    };
    json!({
        "cleaned": incoming::clean(m, 100_000).map_err(|e| e.0),
        "message_id": m.message_id,
        "from": m.from.as_ref().map(|(a, n)| json!([a, n])).unwrap_or(json!([null, null])),
        "to": m.to,
        "cc": m.cc,
        "date": m.date.map(|d| d.format("%Y-%m-%dT%H:%M:%SZ").to_string()),
        "reply_message_ids": reply_ids,
        "destinations": m.all_destinations(),
        "text": Incoming::fix_charset(m.text_part()).map_err(|e| e.0),
        "html": Incoming::fix_charset(m.html_part()).map_err(|e| e.0),
        "attachments": m.attachment_count(),
        "subject": receiver::subject(m, from, &i18n),
        "auto_generated": receiver::is_auto_generated(m, &s, from).unwrap(),
        "body": part(0),
        "elided": part(1),
        "format": part(2),
    })
}

/// Samples a field of which is refused for now, and why.
const REFUSED: &[(&str, &str)] = &[("multipart_alternative", "HTML email bodies")];

#[test]
fn samples_parse_and_clean_like_rails() {
    let expected: serde_json::Map<String, Value> =
        serde_json::from_str(&std::fs::read_to_string(dir().join("expected.json")).unwrap())
            .unwrap();
    assert!(expected.len() >= 19);
    let mut failures = Vec::new();
    for (name, rails) in &expected {
        let raw =
            std::fs::read_to_string(dir().join("emails").join(format!("{name}.eml"))).unwrap();
        let ours = match incoming::parse(&raw) {
            Ok(m) => fields(&m),
            Err(e) => {
                failures.push(format!("{name}: refused: {}", e.0));
                continue;
            }
        };
        for (key, value) in ours.as_object().unwrap() {
            let theirs = &rails[key];
            let ours = match value {
                Value::Object(o) if o.contains_key("Ok") => o["Ok"].clone(),
                Value::Object(o) if o.contains_key("Err") => {
                    let reason = o["Err"].to_string();
                    if REFUSED.iter().any(|(n, r)| n == name && reason.contains(r)) {
                        continue;
                    }
                    failures.push(format!("{name}.{key}: refused: {}", o["Err"]));
                    continue;
                }
                v => v.clone(),
            };
            if &ours != theirs {
                failures.push(format!("{name}.{key}:\n  ours  {ours}\n  rails {theirs}"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
