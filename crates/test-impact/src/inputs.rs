//! Non-Rust files the code itself names, so a change to one selects the
//! tests that ran the Rust file naming it, without a rule:
//!
//! - `#[template(path = "x.html")]`: templates/x.html, and every template
//!   that extends, includes or imports it;
//! - `include_str!("p")` and `include_bytes!("p")`, relative to the file;
//!   one with `concat!("dir/", $name)` names everything under dir/.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use crate::Result;

#[derive(Default)]
pub struct Inputs {
    /// Input file -> the Rust files that name it.
    exact: BTreeMap<String, BTreeSet<String>>,
    /// Directory prefix -> the Rust files that include files under it.
    prefixes: Vec<(String, String)>,
    /// Template -> the templates it extends, includes or imports.
    template_parents: BTreeMap<String, BTreeSet<String>>,
}

impl Inputs {
    /// From the tracked Rust files and templates of the working tree.
    pub fn scan(tracked: &[String]) -> Result<Inputs> {
        let mut inputs = Inputs::default();
        for path in tracked {
            if path.ends_with(".rs") {
                let Ok(text) = fs::read_to_string(path) else {
                    continue;
                };
                inputs.add_rust(path, &text);
            } else if path.starts_with("templates/") && path.ends_with(".html") {
                let Ok(text) = fs::read_to_string(path) else {
                    continue;
                };
                let parents = template_references(&text)
                    .into_iter()
                    .map(|name| format!("templates/{name}"));
                inputs
                    .template_parents
                    .insert(path.clone(), parents.collect());
            }
        }
        Ok(inputs)
    }

    fn add_rust(&mut self, path: &str, text: &str) {
        let dir = path.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("");
        for template in after_each(text, "#[template(").filter_map(|rest| {
            let rest = rest.split_once("path")?.1.trim_start().strip_prefix('=')?;
            string_literal(rest.trim_start())
        }) {
            self.exact
                .entry(format!("templates/{template}"))
                .or_default()
                .insert(path.to_string());
        }
        for macro_name in ["include_str!(", "include_bytes!("] {
            for rest in after_each(text, macro_name) {
                let rest = rest.trim_start();
                if let Some(literal) = string_literal(rest) {
                    self.exact
                        .entry(join(dir, &literal))
                        .or_default()
                        .insert(path.to_string());
                } else if let Some(prefix) = rest
                    .strip_prefix("concat!(")
                    .and_then(|r| string_literal(r.trim_start()))
                {
                    self.prefixes.push((join(dir, &prefix), path.to_string()));
                }
            }
        }
    }

    /// The Rust files naming `path`, directly or through the templates that
    /// build on it. None when nothing names it.
    pub fn naming(&self, path: &str) -> Option<BTreeSet<String>> {
        let mut files = BTreeSet::new();
        let mut templates = vec![path.to_string()];
        let mut seen = BTreeSet::new();
        // A template's change reaches every template that builds on it.
        while let Some(template) = templates.pop() {
            if !seen.insert(template.clone()) {
                continue;
            }
            files.extend(self.exact.get(&template).into_iter().flatten().cloned());
            for (child, parents) in &self.template_parents {
                if parents.contains(&template) {
                    templates.push(child.clone());
                }
            }
        }
        for (prefix, file) in &self.prefixes {
            if path.starts_with(prefix.as_str()) {
                files.insert(file.clone());
            }
        }
        (!files.is_empty()).then_some(files)
    }
}

/// The text after each occurrence of `marker`.
fn after_each<'a>(text: &'a str, marker: &'a str) -> impl Iterator<Item = &'a str> {
    text.match_indices(marker)
        .map(move |(i, _)| &text[i + marker.len()..])
}

/// A leading `"..."`, without escapes (paths have none).
fn string_literal(text: &str) -> Option<String> {
    let rest = text.strip_prefix('"')?;
    Some(rest[..rest.find('"')?].to_string())
}

/// `{% extends "x" %}`, `{% include "x" %}`, `{% import "x" as y %}`.
fn template_references(text: &str) -> Vec<String> {
    after_each(text, "{%")
        .filter_map(|rest| {
            let rest = rest.trim_start_matches('-').trim_start();
            let (keyword, rest) = rest.split_once(char::is_whitespace)?;
            if !matches!(keyword, "extends" | "include" | "import" | "from") {
                return None;
            }
            string_literal(rest.trim_start())
        })
        .collect()
}

/// `dir` joined with a relative path, `..` resolved.
fn join(dir: &str, relative: &str) -> String {
    let mut parts: Vec<&str> = dir.split('/').filter(|p| !p.is_empty()).collect();
    for part in relative.split('/') {
        match part {
            ".." => {
                parts.pop();
            }
            "." | "" => {}
            _ => parts.push(part),
        }
    }
    parts.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs() -> Inputs {
        let mut inputs = Inputs::default();
        inputs.add_rust(
            "src/html.rs",
            r#"
            #[derive(Template)]
            #[template(path = "latest.html")]
            struct Latest;
            const CHECK: &str = include_str!("../templates/empty_states/check.svg");
            const KEY: &[u8] = include_bytes!(
                "../vendor/key.pem"
            );
            "#,
        );
        inputs.add_rust(
            "src/onebox/api_engines.rs",
            r#"include_str!(concat!("../../vendor/discourse/lib/onebox/templates/", $name, ".mustache"))"#,
        );
        inputs.template_parents.insert(
            "templates/latest.html".into(),
            template_references(r#"{% extends "layout.html" %}{% import "macros.html" as m %}"#)
                .into_iter()
                .map(|name| format!("templates/{name}"))
                .collect(),
        );
        inputs
    }

    #[test]
    fn finds_the_files_naming_a_template_or_an_included_file() {
        let inputs = inputs();
        let html = Some(BTreeSet::from(["src/html.rs".to_string()]));
        assert_eq!(inputs.naming("templates/latest.html"), html);
        assert_eq!(inputs.naming("templates/empty_states/check.svg"), html);
        assert_eq!(inputs.naming("vendor/key.pem"), html);
        assert_eq!(
            inputs.naming("vendor/discourse/lib/onebox/templates/github.mustache"),
            Some(BTreeSet::from(["src/onebox/api_engines.rs".to_string()]))
        );
        assert_eq!(inputs.naming("templates/unused.html"), None);
    }

    #[test]
    fn a_template_reaches_the_files_of_templates_built_on_it() {
        let inputs = inputs();
        let html = Some(BTreeSet::from(["src/html.rs".to_string()]));
        assert_eq!(inputs.naming("templates/layout.html"), html);
        assert_eq!(inputs.naming("templates/macros.html"), html);
    }
}
