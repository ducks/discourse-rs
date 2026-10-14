//! Which onebox engine handles a URL (Oneboxer.engine) for the URLs of
//! Discourse's onebox specs and a few more, and what the engines that
//! need no fetch render (parity/onebox_engines/expected.json, recorded by
//! scripts/record-onebox-engines), under the reference's iframe settings.

use discourse_rs::onebox::matcher::{all_iframe_origins, oneboxed, origins_to_regexes};
use serde_json::{Value, json};

fn recording() -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("parity/onebox_engines/expected.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// What differs by chance: GithubRepoOnebox's thumbnail URL holds
/// SecureRandom.hex.
fn scrub(v: &Value) -> Value {
    static HEX: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"opengraph\.githubassets\.com/[0-9a-f]{32}/").unwrap()
    });
    match v {
        Value::String(s) => Value::String(
            HEX.replace_all(s, "opengraph.githubassets.com/HEX/")
                .into_owned(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(scrub).collect()),
        Value::Object(o) => Value::Object(o.iter().map(|(k, v)| (k.clone(), scrub(v))).collect()),
        v => v.clone(),
    }
}

/// The settings the engines read, as recorded.
fn engine_settings(s: &Value) -> discourse_rs::onebox::engines::EngineSettings {
    let text = |k: &str| s[k].as_str().unwrap_or("").to_string();
    discourse_rs::onebox::engines::EngineSettings {
        github_tokens: text("github_onebox_access_tokens"),
        github_pr_status: s["github_pr_status_enabled"] == true,
        facebook_token: text("facebook_app_access_token"),
        twitter_credentials: s["twitter_credentials"] == true,
        emojis: std::sync::Arc::new(discourse_rs::emojis::Emojis {
            set: text("emoji_set"),
            ..Default::default()
        }),
        base_path: text("base_path"),
        lazy_youtube: s["lazy_youtube"] == true,
        lazy_vimeo: s["lazy_vimeo"] == true,
        lazy_tiktok: s["lazy_tiktok"] == true,
    }
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
    let regexes = iframe_regexes(s);
    let ctx = discourse_rs::onebox::engines::Ctx {
        iframes: &regexes,
        cookie: None,
        hostname: s["current_hostname"].as_str().unwrap(),
        disable_media_download_controls: s["disable_onebox_media_download_controls"] == true,
        i18n: &i18n,
        settings: engine_settings(s),
    };
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

/// What Onebox.preview gives: processed and sanitized.
#[test]
fn previews_like_rails() {
    let rec = recording();
    let regexes = iframe_regexes(&rec["settings"]);
    let mut failures = Vec::new();
    for case in rec["previews"].as_array().unwrap() {
        let url = case["url"].as_str().unwrap();
        for (raw, done) in [("raw", "html"), ("raw_placeholder", "placeholder")] {
            let ours = discourse_rs::onebox::preview::finish(case[raw].as_str().unwrap(), &regexes)
                .unwrap_or_else(|e| format!("refused: {}", e.0));
            if ours != case[done].as_str().unwrap() {
                failures.push(format!(
                    "{url} [{done}]\n  ours  {ours:?}\n  rails {}",
                    case[done]
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn fixtures() -> discourse_rs::onebox::fetch::Fixtures {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("parity/onebox_engines");
    let manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("fixtures.json")).unwrap()).unwrap();
    discourse_rs::onebox::fetch::Fixtures {
        pages: serde_json::from_value(manifest["pages"].clone()).unwrap(),
        dir: dir.join("fixtures"),
    }
}

/// Every engine on the fixture pages (their fetches answered from
/// parity/onebox_engines/fixtures/ as the recorder's are): the engine's
/// HTML and placeholder, as processed and sanitized, the errors and data
/// it found, and what Oneboxer.compute_external_onebox makes of them.
#[tokio::test]
async fn fetches_like_rails() {
    use discourse_rs::onebox::engines::{Ctx, Fail, PORTED, preview};
    use discourse_rs::onebox::external::{Errors, engine_onebox};
    let rec = recording();
    let s = &rec["settings"];
    let i18n = discourse_rs::i18n::I18n::vendored().unwrap();
    let regexes = iframe_regexes(s);
    let ctx = Ctx {
        iframes: &regexes,
        cookie: None,
        hostname: s["current_hostname"].as_str().unwrap(),
        disable_media_download_controls: s["disable_onebox_media_download_controls"] == true,
        i18n: &i18n,
        settings: engine_settings(s),
    };
    let errors = Errors {
        i18n: &i18n,
        emojis: std::sync::Arc::new(discourse_rs::emojis::Emojis {
            enabled: s["enable_emoji"] == true,
            shortcuts: s["enable_emoji_shortcuts"] == true,
            inline: s["enable_inline_emoji_translation"] == true,
            set: s["emoji_set"].as_str().unwrap_or("").to_string(),
            ..Default::default()
        }),
        base_path: "",
    };
    let mut fetch = fixtures();
    let mut failures = Vec::new();
    let mut skipped = Vec::new();
    let cases = rec["fetched"].as_array().unwrap();
    for case in cases {
        let url = case["url"].as_str().unwrap();
        let rails_engine = case["engine"].as_str();
        let content_type = fetch
            .pages
            .get(url.split('#').next().unwrap())
            .and_then(|p| p.content_type.clone());
        let engine = oneboxed(url, content_type.as_deref(), &regexes);
        if engine.map(|e| e.name) != rails_engine {
            failures.push(format!(
                "{url}: engine ours {:?} rails {rails_engine:?}",
                engine.map(|e| e.name)
            ));
            continue;
        }
        let Some(engine) = engine else { continue };
        if !PORTED.contains(&engine.name) {
            skipped.push(format!("{url} ({})", engine.name));
            continue;
        }
        let rendered = match preview(engine, url, &ctx, &mut fetch).await {
            Ok(r) => r,
            Err(e) => {
                failures.push(format!("{url}: {e:?}"));
                continue;
            }
        };
        let mut differ = |key: &str, ours: Value| {
            if scrub(&ours) != scrub(&case[key]) {
                failures.push(format!(
                    "{url} [{key}]\n  ours  {ours}\n  rails {}",
                    case[key]
                ));
            }
        };
        let rails_raised = |k: &str| json!(case[k].as_str().map(|_| "raised"));
        let raised = |r: &Option<String>| json!(r.as_ref().map(|_| "raised"));
        if raised(&rendered.html_raised) != rails_raised("raw_error") {
            differ("raw_error", json!(rendered.html_raised));
        }
        if raised(&rendered.placeholder_raised) != rails_raised("placeholder_error") {
            differ("placeholder_error", json!(rendered.placeholder_raised));
        }
        let finish = |h: &Option<String>| {
            discourse_rs::onebox::preview::finish(h.as_deref().unwrap_or(""), &regexes)
                .unwrap_or_else(|e| format!("refused: {}", e.0))
        };
        let process = |h: &Option<String>| {
            h.as_deref()
                .map(|h| json!(discourse_rs::onebox::preview::process_html(h)))
                .unwrap_or(Value::Null)
        };
        differ("raw", json!(rendered.html));
        differ("processed", process(&rendered.html));
        if rendered.html_raised.is_none() {
            differ("html", json!(finish(&rendered.html)));
        }
        differ("raw_placeholder", json!(rendered.placeholder));
        differ("processed_placeholder", process(&rendered.placeholder));
        if rendered.placeholder_raised.is_none() {
            differ("placeholder", json!(finish(&rendered.placeholder)));
        }
        let mut ours_errors = rendered.errors.clone();
        ours_errors.sort();
        let mut rails_errors: Vec<String> = case["errors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e.as_str().unwrap().to_string())
            .collect();
        rails_errors.sort();
        if ours_errors != rails_errors {
            differ("errors", json!(ours_errors));
        }
        if !case["verified_data"].is_null() {
            differ(
                "verified_data",
                rendered.verified_data.clone().unwrap_or(Value::Null),
            );
        }
        drop(differ);
        let external = match engine_onebox(engine, url, &ctx, &mut fetch, &errors).await {
            Ok(o) => json!({"onebox": o.onebox, "preview": o.preview}),
            Err(Fail::Raised(_)) => json!("raised"),
            Err(e) => json!(format!("{e:?}")),
        };
        let rails_external = if case["external"].is_string() {
            json!("raised")
        } else {
            case["external"].clone()
        };
        if scrub(&external) != scrub(&rails_external) {
            failures.push(format!(
                "{url} [external]\n  ours  {external}\n  rails {rails_external}"
            ));
        }
    }
    eprintln!(
        "{} fetched cases, {} with engines not ported:\n  {}",
        cases.len(),
        skipped.len(),
        skipped.join("\n  ")
    );
    assert!(
        failures.is_empty(),
        "{} differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Date._parse and Time.parse on the date strings of fixtures.json: the
/// fields read, and the times printed (or refused where they'd depend on
/// today's date or on Ruby's rolling over).
#[test]
fn parses_times_like_rails() {
    use discourse_rs::onebox::ruby_time::{Parsed, duration, parse};
    let rec = recording();
    let mut failures = Vec::new();
    let mut refused = Vec::new();
    for case in rec["times"].as_array().unwrap() {
        let input = case["input"].as_str().unwrap();
        let h = discourse_rs::ruby_date::parse(input);
        let mut ours = serde_json::Map::new();
        for (k, v) in [
            ("year", h.year),
            ("mon", h.mon),
            ("mday", h.mday),
            ("yday", h.yday),
            ("hour", h.hour),
            ("min", h.min),
            ("sec", h.sec),
        ] {
            if let Some(v) = v {
                ours.insert(k.into(), json!(v));
            }
        }
        if let Some(z) = &h.zone {
            ours.insert("zone".into(), json!(z));
        }
        if Value::Object(ours.clone()) != case["hash"] {
            failures.push(format!(
                "{input:?} hash\n  ours  {}\n  rails {}",
                Value::Object(ours),
                case["hash"]
            ));
        }
        let rails = &case["time"];
        match parse(input) {
            Ok(Parsed::Time(f)) => {
                if json!([f.short(), f.long()]) != json!([rails[0], rails[1]]) {
                    failures.push(format!("{input:?} time\n  ours  {f:?}\n  rails {rails}"));
                }
            }
            Ok(Parsed::Raised) => {
                if rails != "raised" {
                    failures.push(format!("{input:?} time: ours raised, rails {rails}"));
                }
            }
            Err(e) => refused.push(format!("{input:?} ({})", e.0)),
        }
        match duration(input) {
            Ok(Parsed::Time(f)) => {
                if json!(f.hours_minutes()) != rails[2] {
                    failures.push(format!(
                        "{input:?} duration\n  ours  {}\n  rails {rails}",
                        f.hours_minutes()
                    ));
                }
            }
            Ok(Parsed::Raised) => {
                if rails != "raised" {
                    failures.push(format!("{input:?} duration: ours raised, rails {rails}"));
                }
            }
            Err(_) => {}
        }
    }
    eprintln!("refused:\n  {}", refused.join("\n  "));
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
