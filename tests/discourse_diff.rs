//! DiscourseDiff against what Rails rendered for edits between pretty_text
//! corpus entries (scripts/record-revision-fixtures).

use discourse_rs::discourse_diff::body_changes;
use serde_json::Value;

/// Pairs not rendered like Rails yet. The list only shrinks.
const NOT_YET: &[&str] = &[];

#[test]
fn body_changes_match_rails() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("parity/revisions/fixtures.json");
    let fixtures: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mut failing = Vec::new();
    let mut report = Vec::new();
    let pairs = fixtures["pairs"].as_array().unwrap();
    for pair in pairs {
        let name = pair["name"].as_str().unwrap();
        let s = |side: &str, field: &str| pair[side][field].as_str().unwrap().to_string();
        let ours = body_changes(
            &s("before", "cooked"),
            &s("after", "cooked"),
            &s("before", "raw"),
            &s("after", "raw"),
        );
        let rails = &pair["diff"];
        let matches = match &ours {
            Err(_) => rails.get("error").is_some(),
            Ok(c) => {
                rails["inline"] == c.inline.as_str()
                    && rails["side_by_side"] == c.side_by_side.as_str()
                    && rails["side_by_side_markdown"] == c.side_by_side_markdown.as_str()
            }
        };
        if !matches {
            failing.push(name.to_string());
            if !NOT_YET.contains(&name) {
                report.push(format!("{name}\n  rails: {rails}\n  ours:  {ours:?}"));
            }
        }
    }
    eprintln!(
        "{} of {} pairs match Rails",
        pairs.len() - failing.len(),
        pairs.len()
    );
    assert_eq!(failing, NOT_YET, "{}", report.join("\n"));
}
