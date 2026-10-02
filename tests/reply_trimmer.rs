//! The email_reply_trimmer gem's own corpus (parity/email_reply_trimmer,
//! copied from the gem's test/ folder with its license): every email
//! trims to the gem's reply and elided text.

use discourse_rs::email::reply_trimmer::trim;

fn read(dir: &str, name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("parity/email_reply_trimmer")
        .join(dir)
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The gem's test helpers read each file and `strip` it.
fn stripped(s: &str) -> String {
    discourse_rs::email::reply_trimmer::ruby_strip(s).to_string()
}

#[test]
fn the_gem_corpus_trims_like_the_gem() {
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("parity/email_reply_trimmer/emails");
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert!(
        names.len() >= 70,
        "the corpus is missing: {} emails",
        names.len()
    );
    let mut failures = Vec::new();
    for name in &names {
        let email = stripped(&read("emails", name));
        let (trimmed, elided) = trim(&email).unwrap_or_default();
        if trimmed != stripped(&read("trimmed", name)) {
            failures.push(format!(
                "{name} [TRIMMED]\n--- ours\n{trimmed}\n--- gem\n{}",
                stripped(&read("trimmed", name))
            ));
        }
        if elided != stripped(&read("elided", name)) {
            failures.push(format!(
                "{name} [ELIDED]\n--- ours\n{elided}\n--- gem\n{}",
                stripped(&read("elided", name))
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} emails differ:\n{}",
        failures.len(),
        names.len(),
        failures.join("\n\n")
    );
}
