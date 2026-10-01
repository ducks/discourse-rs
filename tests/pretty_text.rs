//! Cooking: the options the port hands the markdown engine and the HTML it
//! gets back, against what Rails recorded on the reference
//! (parity/pretty_text, written by scripts/record-pretty-text).

mod common;

use common::{TestDb, recorded_config, state};
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
