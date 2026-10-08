//! HtmlPrettify on the strings of Discourse's own spec and Topic.fancy_title
//! on a few titles (parity/fancy_title/expected.json, recorded by
//! scripts/record-fancy-titles), under the reference's emoji settings.
//! Titles with unicode emoji must refuse rather than differ.

use discourse_rs::html_prettify::render;
use discourse_rs::posting::text::{EmojiEscape, fancy_title};
use serde_json::Value;

#[test]
fn prettifies_like_rails() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("parity/fancy_title/expected.json");
    let rec: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let s = &rec["settings"];
    let opts = EmojiEscape {
        shortcuts: s["enable_emoji"] == true && s["enable_emoji_shortcuts"] == true,
        inline: s["enable_inline_emoji_translation"] == true,
    };
    let mut failures = Vec::new();
    let mut refused = 0;
    for case in rec["prettify"].as_array().unwrap() {
        let html = case["html"].as_str().unwrap();
        let ours = render(html);
        if ours != case["rendered"].as_str().unwrap() {
            failures.push(format!(
                "[render] {html:?}\n  ours  {ours:?}\n  rails {}",
                case["rendered"]
            ));
        }
    }
    let titles = rec["fancy_title"].as_array().unwrap();
    for case in titles {
        let title = case["title"].as_str().unwrap();
        match fancy_title(title, opts) {
            Ok(ours) if ours == case["fancy"].as_str().unwrap() => {}
            Ok(ours) => failures.push(format!(
                "[fancy_title] {title:?}\n  ours  {ours:?}\n  rails {}",
                case["fancy"]
            )),
            Err(_) => refused += 1,
        }
    }
    assert!(
        failures.is_empty(),
        "{} differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert_eq!(refused, 1, "only the unicode emoji title refuses");
    assert!(titles.len() >= 40);
}
