//! Cooking: the options the port builds for the markdown renderer and the
//! lookups its rules make, against what Rails recorded on the reference
//! (parity/pretty_text, written by scripts/record-pretty-text).

mod common;

use common::{TestDb, recorded_config, state};
use discourse_rs::pretty_text::helpers::{Helpers, translate};

use discourse_rs::pretty_text::{CookError, Host, MarkdownOptions, markdown, options};
use discourse_rs::site_settings::SiteSettings;
use serde_json::Value;

fn recorded(file: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("parity/pretty_text")
        .join(file);
    serde_json::from_str(
        &std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}")),
    )
    .unwrap()
}

/// Client settings Rails computes in class methods rather than declares in
/// YAML. The markdown engine reads none of them; they are left out of the
/// hash until something that does is ported.
const COMPUTED_CLIENT_SETTINGS: &[&str] = &[
    "available_locales",
    "available_content_localization_locales",
    "require_invite_code",
    "site_logo_url",
    "site_logo_small_url",
    "site_mobile_logo_url",
    "site_favicon_url",
    "site_logo_dark_url",
    "site_logo_small_dark_url",
    "site_mobile_logo_dark_url",
];

/// SiteSetting.client_settings_hash, core and plugin settings alike.
#[tokio::test]
async fn client_settings_match_rails() {
    let db = TestDb::new().await;
    let app_state = state(db.pool.clone(), recorded_config());
    let mut conn = db.pool.acquire().await.unwrap();
    let settings = SiteSettings::load(
        &mut conn,
        &app_state.site_setting_defs,
        &app_state.config.globals,
    )
    .await
    .unwrap();
    let ours = settings
        .client_settings_hash(&mut conn, &app_state.site_setting_defs)
        .await
        .unwrap();
    let rails = recorded("opt_input.json");
    let rails = rails["siteSettings"].as_object().unwrap();

    let mut differences = Vec::new();
    for (name, value) in rails {
        match ours.get(name) {
            None if COMPUTED_CLIENT_SETTINGS.contains(&name.as_str()) => {}
            None => differences.push(format!("{name}: missing (rails {value})")),
            Some(v) if v != value => differences.push(format!("{name}: {v} (rails {value})")),
            Some(_) => {}
        }
    }
    for name in ours.keys() {
        if !rails.contains_key(name) {
            differences.push(format!("{name}: not in rails"));
        }
    }
    assert!(
        differences.is_empty(),
        "{} of {} client settings differ:\n{}",
        differences.len(),
        rails.len(),
        differences.join("\n")
    );
}

fn host(db: &TestDb) -> Host {
    let app_state = state(db.pool.clone(), recorded_config());
    Host {
        pool: app_state.pool,
        config: app_state.config,
        site_setting_defs: app_state.site_setting_defs,
        i18n: app_state.i18n,
    }
}

/// `opt_input` of PrettyText.markdown. Rails' has what the reference's
/// plugins add; taking exactly those out must leave what the port builds.
#[tokio::test]
async fn options_match_rails_apart_from_plugin_contributions() {
    let db = TestDb::new().await;
    let host = host(&db);
    let mut conn = db.pool.acquire().await.unwrap();
    let settings = SiteSettings::load(&mut conn, &host.site_setting_defs, &host.config.globals)
        .await
        .unwrap();
    let ours = Value::Object(options(&host, &mut conn, &settings).await.unwrap());

    let mut rails = recorded("opt_input.json");
    // Compared on its own by client_settings_match_rails.
    let settings = rails["siteSettings"].as_object_mut().unwrap();
    for name in COMPUTED_CLIENT_SETTINGS {
        assert!(settings.remove(*name).is_some(), "{name}");
    }
    // discourse-narrative-bot's pretty_text_allowed_iframes modifier.
    let iframes = rails["allowedIframes"].as_array_mut().unwrap();
    let before = iframes.len();
    iframes.retain(|url| !url.as_str().unwrap().ends_with("/discobot/certificate.svg"));
    assert_eq!(iframes.len(), before - 1);
    // Chat's markdown options, hashtag type and icon.
    assert!(
        rails["additionalOptions"]
            .as_object_mut()
            .unwrap()
            .remove("chat")
            .is_some()
    );
    let types = rails["hashtagTypesInPriorityOrder"].as_array_mut().unwrap();
    assert_eq!(types.pop(), Some(Value::from("channel")));
    assert!(
        rails["hashtagIcons"]
            .as_object_mut()
            .unwrap()
            .remove("channel")
            .is_some()
    );

    for (key, value) in rails.as_object().unwrap() {
        assert_eq!(&ours[key], value, "{key}");
    }
    assert_eq!(
        ours.as_object().unwrap().len(),
        rails.as_object().unwrap().len()
    );
}

/// PrettyText::Helpers: every call Rails' cooking of the corpus made into
/// Ruby, answered here from the seed database with the same result.
#[tokio::test]
async fn helpers_answer_as_rails_did() {
    let db = TestDb::new().await;
    let host = host(&db);
    let mut conn = db.pool.acquire().await.unwrap();
    let settings = SiteSettings::load(&mut conn, &host.site_setting_defs, &host.config.globals)
        .await
        .unwrap();
    let calls = recorded("helpers.json");
    let calls = calls.as_array().unwrap();
    assert!(calls.len() > 30, "{} calls", calls.len());

    let mut differences = Vec::new();
    for call in calls {
        let method = call["method"].as_str().unwrap();
        let args = call["args"].as_array().unwrap();
        let arg = |i: usize| args.get(i).unwrap_or(&Value::Null);
        let mut helpers = Helpers {
            host: &host,
            conn: &mut conn,
            settings: &settings,
        };
        let ours = match method {
            "t" => Ok(Value::from(translate(&host, arg(0), arg(1)))),
            "format_username" => Ok(arg(0).clone()),
            "avatar_template" => helpers.avatar_template(arg(0).as_str()).await,
            "lookup_primary_user_group" => helpers.primary_user_group(arg(0).as_str()).await,
            "lookup_upload_urls" => helpers.upload_urls(arg(0)).await,
            "get_topic_info" => helpers.topic_info(arg(0)).await,
            "hashtag_lookup" => helpers.hashtag_lookup(arg(0), arg(1), arg(2)).await,
            "get_current_user" => helpers.current_user(arg(0)).await,
            other => panic!("unknown helper {other}"),
        };
        match ours {
            Ok(value) if value == call["result"] => {}
            Ok(value) => differences.push(format!(
                "{method}{}: {value} (rails {})",
                call["args"], call["result"]
            )),
            Err(e) => differences.push(format!("{method}{}: {e}", call["args"])),
        }
    }
    assert!(
        differences.is_empty(),
        "{} of {} helper calls differ:\n{}",
        differences.len(),
        calls.len(),
        differences.join("\n")
    );
}

/// What would need something not ported is refused, not answered
/// differently from Rails.
#[tokio::test]
async fn unported_inputs_are_refused() {
    let db = TestDb::new().await;
    let host = host(&db);
    let mut conn = db.pool.acquire().await.unwrap();

    // A chat channel the cooking user can see, named like the hashtag:
    // only chat could resolve it.
    sqlx::query(
        "INSERT INTO chat_channels (chatable_id, chatable_type, name, slug, type, created_at, updated_at) \
         VALUES (4, 'Category', 'Lounge', 'lounge', 'CategoryChannel', now(), now())",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    common::set_setting(&db.pool, "chat_enabled", 5, "t").await;
    let settings = SiteSettings::load(&mut conn, &host.site_setting_defs, &host.config.globals)
        .await
        .unwrap();
    let error = Helpers {
        host: &host,
        conn: &mut conn,
        settings: &settings,
    }
    .hashtag_lookup(
        &Value::from("lounge"),
        &Value::from(3),
        &serde_json::json!(["category", "tag"]),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, CookError::Unsupported(_)), "{error}");

    // A censored watched word (action 2) changes the options.
    sqlx::query(
        "INSERT INTO watched_words (word, action, created_at, updated_at) VALUES ('darn', 2, now(), now())",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let error = options(&host, &mut conn, &settings).await.unwrap_err();
    assert!(matches!(error, CookError::Unsupported(_)), "{error}");
}

/// Where two strings part ways, with some of what surrounds it.
fn first_difference(rails: &str, ours: &str) -> String {
    let rails: Vec<char> = rails.chars().collect();
    let ours: Vec<char> = ours.chars().collect();
    let at = rails
        .iter()
        .zip(ours.iter())
        .position(|(a, b)| a != b)
        .unwrap_or(rails.len().min(ours.len()));
    let window = |chars: &[char]| -> String {
        let from = at.saturating_sub(70);
        let to = (at + 150).min(chars.len());
        chars[from.min(to)..to].iter().collect()
    };
    format!(
        "  at {at}\n  rails: {:?}\n  ours:  {:?}",
        window(&rails),
        window(&ours)
    )
}

/// Corpus entries the renderer does not cook like Rails yet. The list
/// only shrinks: an entry that starts matching has to be taken out, one
/// that stops matching fails the test.
const NOT_COOKED_YET: &[&str] = &["sample-30"];

/// PrettyText.markdown over the recorded corpus (feature samples and every
/// seeded post), each byte-equal to what Rails cooked, apart from the
/// entries listed above.
#[tokio::test]
async fn cooking_matches_rails() {
    let db = TestDb::new().await;
    let host = host(&db);
    let corpus = recorded("corpus.json");
    let corpus = corpus.as_array().unwrap();
    assert!(corpus.len() > 70, "{} entries", corpus.len());

    let mut missed = Vec::new();
    let mut details = Vec::new();
    for entry in corpus {
        let id = entry["id"].as_str().unwrap();
        let rails = entry["markdown"].as_str().unwrap();
        let opts = MarkdownOptions {
            topic_id: entry["topic_id"].as_i64(),
            user_id: entry["user_id"].as_i64(),
            ..Default::default()
        };
        let ours = markdown(&host, entry["raw"].as_str().unwrap(), &opts)
            .await
            .unwrap_or_else(|e| format!("error: {e}"));
        if ours != rails {
            missed.push(id);
            if !NOT_COOKED_YET.contains(&id) || std::env::var("COOK_DIFF").is_ok() {
                details.push(format!("{id}\n{}", first_difference(rails, &ours)));
            }
        }
    }
    eprintln!(
        "{} of {} corpus entries cook byte-equal",
        corpus.len() - missed.len(),
        corpus.len()
    );
    if std::env::var("COOK_DIFF").is_ok() {
        eprintln!("{}", details.join("\n"));
    }
    assert_eq!(
        missed,
        NOT_COOKED_YET,
        "the entries that do not match Rails changed:\n{}",
        details.join("\n")
    );
}
