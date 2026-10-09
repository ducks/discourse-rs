//! Chat::Message.cook: chat's markdown (its features and markdown-it
//! rules), its slash commands and hashtags, against what Rails cooked on
//! the reference (parity/chat_cook, written by scripts/record-chat-cooks).

mod common;

use common::{TestDb, recorded_config};
use discourse_rs::pretty_text::Host;
use serde_json::Value;

fn host(db: &TestDb) -> Host {
    Host {
        pool: db.pool.clone(),
        config: recorded_config(),
        site_setting_defs: std::sync::Arc::new(
            discourse_rs::site_settings::Definitions::vendored()
                .expect("vendored site_settings.yml"),
        ),
        i18n: std::sync::Arc::new(
            discourse_rs::i18n::I18n::vendored().expect("vendored server.en.yml"),
        ),
    }
}

#[tokio::test]
async fn chat_messages_cook_as_rails_cooks_them() {
    let db = TestDb::new().await;
    let host = host(&db);
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("parity/chat_cook/corpus.json");
    let corpus: Vec<Value> =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let mut differences = Vec::new();
    for sample in &corpus {
        let raw = sample["raw"].as_str().unwrap();
        let rails = sample["cooked"].as_str().unwrap();
        match discourse_rs::plugins::chat::cook::cook(&host, raw, 3, "user1").await {
            Ok(ours) if ours == rails => {}
            Ok(ours) => differences.push(format!("{raw:?}\n  ours:  {ours:?}\n  rails: {rails:?}")),
            Err(e) => differences.push(format!("{raw:?}\n  refused: {e}\n  rails: {rails:?}")),
        }
    }
    assert!(
        differences.is_empty(),
        "{} of {} samples differ:\n{}",
        differences.len(),
        corpus.len(),
        differences.join("\n")
    );
}
