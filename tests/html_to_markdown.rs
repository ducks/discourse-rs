//! HtmlToMarkdown on the HTML of Discourse's own spec and a few samples
//! (parity/html_to_markdown/expected.json, recorded by
//! scripts/record-html-to-markdown), with no options and with the ones
//! Email::Receiver passes.

use discourse_rs::email::html_to_markdown::{Options, to_markdown};
use serde_json::Value;

#[test]
fn converts_like_rails() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("parity/html_to_markdown/expected.json");
    let cases: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert!(cases.len() >= 200);
    // allowed_href_schemes is empty on the reference.
    let schemes = "";
    let receiver = Options {
        keep_img_tags: true,
        keep_cid_imgs: true,
    };
    let mut failures = Vec::new();
    for case in &cases {
        let html = case["html"].as_str().unwrap();
        for (opts, key) in [
            (Options::default(), "markdown"),
            (receiver.clone(), "receiver"),
        ] {
            let ours = to_markdown(html, &opts, schemes);
            let rails = case[key].as_str().unwrap();
            if ours != rails {
                failures.push(format!(
                    "[{key}] {html:?}\n  ours  {ours:?}\n  rails {rails:?}"
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} differ:\n{}",
        failures.len(),
        cases.len() * 2,
        failures.join("\n")
    );
}
