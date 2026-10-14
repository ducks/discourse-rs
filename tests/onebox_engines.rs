//! Which onebox engine handles a URL (Oneboxer.engine) for the URLs of
//! Discourse's onebox specs and a few more, and what the engines that
//! need no fetch render (parity/onebox_engines/expected.json, recorded by
//! scripts/record-onebox-engines), under the reference's iframe settings.

use discourse_rs::onebox::matcher::{all_iframe_origins, oneboxed, origins_to_regexes};
use serde_json::Value;

fn recording() -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("parity/onebox_engines/expected.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// `Oneboxer.allowed_iframe_origins` from the recorded settings.
fn iframe_regexes(settings: &Value) -> Vec<regex::Regex> {
    let split = |v: &Value| -> Vec<String> {
        v.as_str()
            .unwrap_or("")
            .split('|')
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect()
    };
    let mut allowed = split(&settings["allowed_onebox_iframes"]);
    if allowed.iter().any(|o| o == "*") {
        allowed = all_iframe_origins();
    }
    allowed.extend(split(&settings["allowed_iframes"]));
    origins_to_regexes(&allowed)
}

#[test]
fn picks_engines_like_rails() {
    let rec = recording();
    let regexes = iframe_regexes(&rec["settings"]);
    let mut failures = Vec::new();
    for case in rec["engines"].as_array().unwrap() {
        let url = case["url"].as_str().unwrap();
        let ours = oneboxed(url, None, &regexes).map(|e| e.name);
        if ours != case["engine"].as_str() {
            failures.push(format!(
                "{url}\n  ours  {ours:?}\n  rails {}",
                case["engine"]
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} differ:\n{}",
        failures.len(),
        rec["engines"].as_array().unwrap().len(),
        failures.join("\n")
    );
}

/// The engines' own HTML and placeholders, before Onebox::Preview
/// processes and sanitizes them.
#[test]
fn renders_engines_like_rails() {
    let rec = recording();
    let s = &rec["settings"];
    let i18n = discourse_rs::i18n::I18n::vendored().unwrap();
    let ctx = discourse_rs::onebox::engines::Ctx {
        hostname: s["current_hostname"].as_str().unwrap(),
        disable_media_download_controls: s["disable_onebox_media_download_controls"] == true,
        i18n: &i18n,
    };
    let regexes = iframe_regexes(s);
    let mut failures = Vec::new();
    let previews = rec["previews"].as_array().unwrap();
    for case in previews {
        let url = case["url"].as_str().unwrap();
        let engine = oneboxed(url, None, &regexes).unwrap();
        let ours = match discourse_rs::onebox::engines::render(engine, url, &ctx) {
            Ok(h) => h,
            Err(e) => {
                failures.push(format!("{url}: refused ({})", e.0));
                continue;
            }
        };
        for (key, got) in [("raw", &ours.html), ("raw_placeholder", &ours.placeholder)] {
            if got != case[key].as_str().unwrap() {
                failures.push(format!(
                    "{url} [{key}]\n  ours  {got:?}\n  rails {}",
                    case[key]
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} differ:\n{}",
        failures.len(),
        previews.len(),
        failures.join("\n")
    );
}

/// `Preview#process_html` on the engines' HTML and placeholders.
#[test]
fn processes_previews_like_rails() {
    let rec = recording();
    let mut failures = Vec::new();
    for case in rec["previews"].as_array().unwrap() {
        let url = case["url"].as_str().unwrap();
        for (raw, processed) in [
            ("raw", "processed"),
            ("raw_placeholder", "processed_placeholder"),
        ] {
            let ours = discourse_rs::onebox::preview::process_html(case[raw].as_str().unwrap());
            if ours != case[processed].as_str().unwrap() {
                failures.push(format!(
                    "{url} [{processed}]\n  ours  {ours:?}\n  rails {}",
                    case[processed]
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
