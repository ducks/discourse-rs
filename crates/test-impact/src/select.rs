//! Selecting from the map for a diff.
//!
//! A hunk in a workspace Rust file selects the tests that ran the innermost
//! function it touches (a closure inside a method is its own function). A
//! hunk outside every function (struct fields, derives, consts, `use`)
//! selects every test that ran the file. A function no test ran selects
//! nothing and is reported as untested.
//!
//! Other files: a write case's files select that case; the rules in
//! .config/test-impact.toml map other inputs to tests, to nothing, or to the
//! whole suite; anything no rule names runs the whole suite. So do a crate
//! root's module list, a new source file, and a file no test ran when a
//! hunk falls outside its functions.

use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;

use serde::Deserialize;

use crate::Result;
use crate::inputs::Inputs;
use crate::map::Map;

const RULES: &str = ".config/test-impact.toml";
/// A crate root's lines outside functions are its module list.
const CRATE_ROOTS: &[&str] = &["src/lib.rs", "src/main.rs"];
/// Its module list: a new test file's tests are new tests (`--list`).
const TEST_ROOT: &str = "tests/integration.rs";
const WRITES_BINARY: &str = "discourse-rs::writes";

#[derive(Default)]
pub struct Selection {
    pub tests: BTreeSet<u32>,
    pub filters: BTreeSet<String>,
    /// Changes that need the whole suite, and why.
    pub full: Vec<String>,
    pub untested: Vec<String>,
}

#[derive(Deserialize)]
struct Rules {
    #[serde(default)]
    rule: Vec<Rule>,
}

#[derive(Deserialize)]
struct Rule {
    globs: Vec<String>,
    /// A nextest filterset of the tests that read these files.
    filter: Option<String>,
    /// Rust files whose tests read these files: the tests that ran them.
    ran: Option<Vec<String>>,
    /// "all" or "none".
    select: Option<String>,
}

struct Change {
    old: Option<String>,
    new: Option<String>,
    /// Old-side line ranges.
    hunks: Vec<(u32, u32)>,
    /// Each hunk's removed and added lines.
    lines: Vec<Vec<String>>,
}

pub fn select_command(map_path: &str, args: &[String]) -> Result<u8> {
    let mut base = "origin/main".to_string();
    let mut list = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match (arg.as_str(), args.next()) {
            ("--base", Some(value)) => base = value.clone(),
            ("--list", Some(value)) => list = Some(value.clone()),
            _ => return Err(format!("select: unexpected {arg}").into()),
        }
    }
    let map = Map::load(map_path)?;
    let rules = load_rules()?;
    let inputs = Inputs::scan(
        &git(&["ls-files"])?
            .lines()
            .map(String::from)
            .collect::<Vec<_>>(),
    )?;
    let merge_base = git(&["merge-base", &base, "HEAD"])?.trim().to_string();
    // Against the working tree, so uncommitted changes count.
    let diff = git(&["diff", "-U0", "--no-color", "-M", &merge_base])?;
    let selection = select(&map, &rules, &inputs, &parse_diff(&diff));

    for change in &selection.untested {
        eprintln!("untested: {change}");
    }
    if !selection.full.is_empty() {
        for reason in &selection.full {
            eprintln!("whole suite: {reason}");
        }
        println!("all()");
        return Ok(0);
    }
    let mut filters: Vec<String> = selection.filters.iter().cloned().collect();
    filters.extend(by_binary(&map, &selection.tests));
    if let Some(list) = list {
        let new = new_tests(&map, &list)?;
        eprintln!(
            "new tests: {}",
            new.values().map(BTreeSet::len).sum::<usize>()
        );
        filters.extend(
            new.iter()
                .map(|(binary, names)| filter(binary, names.iter())),
        );
    }
    eprintln!(
        "selected {} of {} recorded tests ({:.0}s of {:.0}s), {} rule filters",
        selection.tests.len(),
        map.tests.len(),
        seconds(&map, &selection.tests),
        seconds(&map, &(0..map.tests.len() as u32).collect()),
        selection.filters.len(),
    );
    println!(
        "{}",
        if filters.is_empty() {
            "none()".to_string()
        } else {
            filters.join(" | ")
        }
    );
    Ok(0)
}

fn select(map: &Map, rules: &Rules, inputs: &Inputs, changes: &[Change]) -> Selection {
    let mut selection = Selection::default();
    for change in changes {
        let path = change
            .new
            .as_deref()
            .or(change.old.as_deref())
            .unwrap_or_default();
        if path.ends_with(".rs") {
            select_rust(map, change, path, &mut selection);
        } else if let Some(test) = write_case(path) {
            if change.new.is_some() {
                selection
                    .filters
                    .insert(filter(WRITES_BINARY, [test].iter()));
            }
        } else if let Some(files) = inputs.naming(path) {
            ran_files(map, &files, path, &mut selection);
        } else {
            match rules
                .rule
                .iter()
                .find(|rule| rule.globs.iter().any(|g| glob(g, path)))
            {
                Some(Rule {
                    filter: Some(filter),
                    ..
                }) => {
                    selection.filters.insert(filter.clone());
                }
                Some(Rule {
                    ran: Some(files), ..
                }) => {
                    ran_files(map, &files.iter().cloned().collect(), path, &mut selection);
                }
                Some(Rule {
                    select: Some(kind), ..
                }) if kind == "none" => {}
                Some(_) => selection.full.push(format!("{path} (a rule)")),
                None => selection.full.push(format!("{path} (no rule names it)")),
            }
        }
    }
    selection
}

/// For an input the given Rust files read (paths or globs): the tests that
/// ran them.
fn ran_files(map: &Map, files: &BTreeSet<String>, path: &str, selection: &mut Selection) {
    let tests: BTreeSet<u32> = map
        .files
        .iter()
        .filter(|(name, _)| files.iter().any(|file| file == *name || glob(file, name)))
        .map(|(_, file)| file)
        .flat_map(|file| {
            file.ranges
                .iter()
                .flat_map(|(_, _, ids)| ids.iter().copied())
        })
        .collect();
    selection.tests.extend(&tests);
    if tests.is_empty() {
        selection.untested.push(format!(
            "{path} (read by {})",
            files.iter().cloned().collect::<Vec<_>>().join(", ")
        ));
    }
}

fn select_rust(map: &Map, change: &Change, path: &str, selection: &mut Selection) {
    let file = change.old.as_deref().and_then(|old| map.files.get(old));
    let Some(file) = file else {
        if change.old.is_some() && change.new.is_none() {
            return;
        }
        // A new file runs only where changed code calls it, which selects
        // its tests; a new test file's tests are new tests (`--list`).
        if change.old.is_none() {
            selection
                .untested
                .push(format!("{path} (new: selected through its callers)"));
        } else {
            // Compiled into no recorded binary (build.rs, a bin), or newer
            // than the map.
            selection.full.push(format!("{path} (not in the map)"));
        }
        return;
    };
    // A rename or deletion: whatever ran it.
    if change.hunks.is_empty() || change.new.is_none() {
        selection.tests.extend(
            file.ranges
                .iter()
                .flat_map(|(_, _, ids)| ids.iter().copied()),
        );
        return;
    }
    for (index, &(first, last)) in change.hunks.iter().enumerate() {
        let touched: Vec<(u32, u32)> = file
            .functions
            .iter()
            .copied()
            .filter(|&(s, e)| s <= last && e >= first)
            .collect();
        let innermost: Vec<(u32, u32)> = touched
            .iter()
            .copied()
            .filter(|&(s, e)| {
                !touched
                    .iter()
                    .any(|&(ts, te)| (ts, te) != (s, e) && ts >= s && te <= e)
            })
            .collect();
        for &(s, e) in &innermost {
            let reached: Vec<&Vec<u32>> = file
                .ranges
                .iter()
                .filter(|r| (r.0, r.1) == (s, e))
                .map(|r| &r.2)
                .collect();
            if reached.is_empty() {
                selection.untested.push(format!("{path}:{s}-{e}"));
            }
            for ids in reached {
                selection.tests.extend(ids.iter().copied());
            }
        }
        let outside =
            (first..=last).any(|line| !touched.iter().any(|&(s, e)| (s..=e).contains(&line)));
        if !outside {
            continue;
        }
        if path == TEST_ROOT {
            continue;
        }
        let crate_root = CRATE_ROOTS
            .iter()
            .any(|root| path == *root || path.ends_with(&format!("/{root}")));
        let lines = change
            .lines
            .get(index)
            .map(Vec::as_slice)
            .unwrap_or_default();
        if crate_root && lines.iter().all(|line| module_list(line)) {
            // A module's behaviour is in its file; a broken re-export
            // doesn't compile.
        } else if crate_root {
            selection
                .full
                .push(format!("{path}:{first}-{last} (crate root)"));
        } else if file.ranges.is_empty() {
            selection.full.push(format!(
                "{path}:{first}-{last} (outside functions, no test ran the file)"
            ));
        } else {
            selection.tests.extend(
                file.ranges
                    .iter()
                    .flat_map(|(_, _, ids)| ids.iter().copied()),
            );
        }
    }
}

/// A crate root's module list: `mod` and `use` lines, the names inside a
/// multi-line `use`, blanks and comments.
fn module_list(line: &str) -> bool {
    let line = line.trim();
    let line = line
        .strip_prefix("pub(crate) ")
        .or_else(|| line.strip_prefix("pub "))
        .unwrap_or(line);
    line.is_empty()
        || line.starts_with("//")
        || line.starts_with("#[cfg")
        || line.starts_with("mod ")
        || line.starts_with("use ")
        || line
            .chars()
            .all(|c| c.is_alphanumeric() || " _:{},*;".contains(c))
}

/// parity/writes/<area>/<name>.case.json and its recording <name>.json are
/// the write case `<area>::<name>`.
fn write_case(path: &str) -> Option<String> {
    let rest = path.strip_prefix("parity/writes/")?;
    let (area, file) = rest.split_once('/')?;
    if area == "files" || file.contains('/') {
        return None;
    }
    let name = file
        .strip_suffix(".case.json")
        .or_else(|| file.strip_suffix(".json"))?;
    Some(format!("{area}::{name}"))
}

fn by_binary(map: &Map, ids: &BTreeSet<u32>) -> Vec<String> {
    let mut grouped: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for &id in ids {
        let test = &map.tests[id as usize];
        grouped.entry(&test.binary).or_default().push(&test.name);
    }
    grouped
        .iter()
        .map(|(binary, names)| filter(binary, names.iter()))
        .collect()
}

fn filter<S: AsRef<str>>(binary: &str, names: impl Iterator<Item = S>) -> String {
    let tests: Vec<String> = names
        .map(|name| format!("test(={})", escape(name.as_ref())))
        .collect();
    format!("(binary_id(={}) & ({}))", escape(binary), tests.join(" | "))
}

/// nextest's filterset strings end at `)` or `,` unless escaped.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '(' | ')' | ',' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Tests in `cargo nextest list --message-format json` output that aren't
/// in the map: new since it was recorded.
fn new_tests(map: &Map, list_path: &str) -> Result<BTreeMap<String, BTreeSet<String>>> {
    #[derive(Deserialize)]
    struct List {
        #[serde(rename = "rust-suites")]
        suites: BTreeMap<String, Suite>,
    }
    #[derive(Deserialize)]
    struct Suite {
        testcases: BTreeMap<String, serde_json::Value>,
    }
    let list: List = serde_json::from_slice(&std::fs::read(list_path)?)?;
    let known: BTreeSet<(&str, &str)> = map
        .tests
        .iter()
        .map(|t| (t.binary.as_str(), t.name.as_str()))
        .collect();
    let mut new: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (binary, suite) in list.suites {
        for name in suite.testcases.into_keys() {
            if !known.contains(&(binary.as_str(), name.as_str())) {
                new.entry(binary.clone()).or_default().insert(name);
            }
        }
    }
    Ok(new)
}

fn seconds(map: &Map, ids: &BTreeSet<u32>) -> f64 {
    ids.iter().map(|&id| map.tests[id as usize].seconds).sum()
}

/// For each of the last `count` first-parent commits on origin/main, what
/// its diff would have selected: sha, tests selected, whether the whole
/// suite was needed, the selected share of recorded test time, subject,
/// and why the whole suite was needed.
pub fn replay(map_path: &str, count: Option<&String>) -> Result<()> {
    let map = Map::load(map_path)?;
    let rules = load_rules()?;
    let inputs = Inputs::scan(
        &git(&["ls-files"])?
            .lines()
            .map(String::from)
            .collect::<Vec<_>>(),
    )?;
    let count = count.map(String::as_str).unwrap_or("200");
    let total = seconds(&map, &(0..map.tests.len() as u32).collect());
    let mut shares = Vec::new();
    for sha in git(&["rev-list", "--first-parent", "-n", count, "origin/main"])?.lines() {
        let diff = git(&["diff", "-U0", "--no-color", "-M", &format!("{sha}^"), sha])?;
        let selection = select(&map, &rules, &inputs, &parse_diff(&diff));
        let full = !selection.full.is_empty();
        // Rule filters aren't resolved here; they're small next to coverage.
        let share = if full {
            1.0
        } else {
            seconds(&map, &selection.tests) / total
        };
        shares.push(share);
        let subject = git(&["log", "-1", "--format=%s", sha])?;
        println!(
            "{}\t{}\t{}\t{:.3}\t{}\t{}",
            &sha[..10],
            selection.tests.len(),
            if full { "whole" } else { "selected" },
            share,
            subject.trim().chars().take(60).collect::<String>(),
            selection.full.join("; "),
        );
    }
    let selected = shares.iter().filter(|&&s| s < 1.0).count();
    eprintln!(
        "{} commits: {} selected, mean share of test time {:.2}",
        shares.len(),
        selected,
        shares.iter().sum::<f64>() / shares.len().max(1) as f64,
    );
    Ok(())
}

fn load_rules() -> Result<Rules> {
    match std::fs::read_to_string(RULES) {
        Ok(text) => Ok(toml::from_str(&text)?),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Rules { rule: Vec::new() }),
        Err(err) => Err(err.into()),
    }
}

/// `git diff -U0`: each file's old and new path (None when added or
/// deleted) and the old-side range of each hunk. A pure insertion covers
/// the lines on either side of it.
fn parse_diff(text: &str) -> Vec<Change> {
    let mut changes: Vec<Change> = Vec::new();
    for line in text.lines() {
        if let Some(paths) = line.strip_prefix("diff --git a/") {
            if let Some((old, new)) = paths.split_once(" b/") {
                changes.push(Change {
                    old: Some(old.into()),
                    new: Some(new.into()),
                    hunks: Vec::new(),
                    lines: Vec::new(),
                });
            }
        } else if let Some(change) = changes.last_mut() {
            // Before the first hunk, `+++` and `---` are the header.
            if let (Some(lines), Some('+' | '-')) = (change.lines.last_mut(), line.chars().next()) {
                lines.push(line[1..].to_string());
            } else if line.starts_with("new file mode ") {
                change.old = None;
            } else if line.starts_with("deleted file mode ") {
                change.new = None;
            } else if let Some(rest) = line.strip_prefix("@@ -") {
                let old = rest.split(' ').next().unwrap_or_default();
                let (start, count) = match old.split_once(',') {
                    Some((start, count)) => {
                        (start.parse().unwrap_or(0), count.parse().unwrap_or(1))
                    }
                    None => (old.parse().unwrap_or(0), 1),
                };
                change.hunks.push(if count == 0 {
                    (start, start + 1)
                } else {
                    (start, start + count - 1)
                });
                change.lines.push(Vec::new());
            }
        }
    }
    changes
}

/// `*` matches within a path segment, `**` across them.
fn glob(pattern: &str, path: &str) -> bool {
    fn matches(p: &[u8], s: &[u8]) -> bool {
        match p {
            [] => s.is_empty(),
            [b'*', b'*', b'/', rest @ ..] => {
                matches(rest, s)
                    || s.iter()
                        .enumerate()
                        .any(|(i, &c)| c == b'/' && matches(rest, &s[i + 1..]))
            }
            [b'*', b'*', rest @ ..] => (0..=s.len()).any(|i| matches(rest, &s[i..])),
            [b'*', rest @ ..] => (0..=s.len())
                .take_while(|&i| !s[..i].contains(&b'/'))
                .any(|i| matches(rest, &s[i..])),
            [c, rest @ ..] => s.first() == Some(c) && matches(rest, &s[1..]),
        }
    }
    matches(pattern.as_bytes(), path.as_bytes())
}

fn git(args: &[&str]) -> Result<String> {
    let output = Command::new("git").args(args).output()?;
    if !output.status.success() {
        return Err(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(String::from_utf8(output.stdout)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::{File, Test};

    fn map() -> Map {
        let test = |name: &str| Test {
            binary: "discourse-rs::integration".into(),
            name: name.into(),
            seconds: 1.0,
        };
        let mut files = BTreeMap::new();
        // src/topics.rs: `create` at 10-30 with a closure at 20-24, `idle`
        // at 40-50, a struct above them.
        files.insert(
            "src/topics.rs".to_string(),
            File {
                functions: vec![(10, 30), (20, 24), (40, 50)],
                ranges: vec![(10, 30, vec![0, 1]), (20, 24, vec![1])],
            },
        );
        files.insert(
            "src/lib.rs".to_string(),
            File {
                functions: vec![],
                ranges: vec![],
            },
        );
        Map {
            tests: vec![test("topics::creates"), test("topics::creates_with_tags")],
            files,
        }
    }

    fn change(path: &str, hunks: &[(u32, u32)]) -> Change {
        changed_lines(path, hunks, "run();")
    }

    fn changed_lines(path: &str, hunks: &[(u32, u32)], line: &str) -> Change {
        Change {
            old: Some(path.into()),
            new: Some(path.into()),
            hunks: hunks.to_vec(),
            lines: vec![vec![line.to_string()]; hunks.len()],
        }
    }

    fn rules() -> Rules {
        toml::from_str(
            r#"
            [[rule]]
            globs = ["docs/**", "**/*.md"]
            select = "none"
            [[rule]]
            globs = ["parity/onebox_engines/**"]
            filter = "test(/onebox/)"
            "#,
        )
        .unwrap()
    }

    #[test]
    fn selects_the_tests_that_ran_the_innermost_function() {
        let outer = select(
            &map(),
            &rules(),
            &Inputs::default(),
            &[change("src/topics.rs", &[(12, 12)])],
        );
        let inner = select(
            &map(),
            &rules(),
            &Inputs::default(),
            &[change("src/topics.rs", &[(22, 22)])],
        );
        assert_eq!(outer.tests, [0, 1].into());
        assert_eq!(inner.tests, [1].into());
    }

    #[test]
    fn reports_a_function_no_test_ran() {
        let selection = select(
            &map(),
            &rules(),
            &Inputs::default(),
            &[change("src/topics.rs", &[(45, 45)])],
        );
        assert!(selection.tests.is_empty());
        assert_eq!(selection.untested, ["src/topics.rs:40-50"]);
    }

    #[test]
    fn selects_every_test_that_ran_the_file_for_a_change_outside_functions() {
        let selection = select(
            &map(),
            &rules(),
            &Inputs::default(),
            &[change("src/topics.rs", &[(3, 4)])],
        );
        assert_eq!(selection.tests, [0, 1].into());
    }

    #[test]
    fn needs_the_whole_suite_for_crate_root_code_but_not_its_module_list() {
        let code = select(
            &map(),
            &rules(),
            &Inputs::default(),
            &[change("src/lib.rs", &[(5, 5)])],
        );
        let modules = select(
            &map(),
            &rules(),
            &Inputs::default(),
            &[changed_lines("src/lib.rs", &[(5, 5)], "pub mod polls;")],
        );
        assert_eq!(code.full, ["src/lib.rs:5-5 (crate root)"]);
        assert!(modules.full.is_empty() && modules.tests.is_empty());
    }

    #[test]
    fn selects_a_new_source_file_through_its_callers() {
        let new = Change {
            old: None,
            new: Some("src/polls.rs".into()),
            hunks: vec![(0, 1)],
            lines: vec![vec!["pub fn vote() {}".into()]],
        };
        let selection = select(&map(), &rules(), &Inputs::default(), &[new]);
        assert!(selection.full.is_empty() && selection.tests.is_empty());
        assert_eq!(
            selection.untested,
            ["src/polls.rs (new: selected through its callers)"]
        );
    }

    #[test]
    fn keeps_each_hunks_lines() {
        let diff = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -5 +5 @@\n-pub mod a;\n+pub mod b;\n";
        assert_eq!(parse_diff(diff)[0].lines, [["pub mod a;", "pub mod b;"]]);
    }

    #[test]
    fn selects_a_write_case_for_its_files() {
        let selection = select(
            &map(),
            &rules(),
            &Inputs::default(),
            &[change("parity/writes/topics/create.case.json", &[(1, 1)])],
        );
        assert_eq!(
            selection.filters.into_iter().collect::<Vec<_>>(),
            ["(binary_id(=discourse-rs::writes) & (test(=topics::create)))"]
        );
    }

    #[test]
    fn applies_the_rules_and_runs_everything_for_files_no_rule_names() {
        let selection = select(
            &map(),
            &rules(),
            &Inputs::default(),
            &[
                change("docs/plugin-wasm.md", &[(1, 1)]),
                change("parity/onebox_engines/github.json", &[(1, 1)]),
                change("templates/topic.html", &[(1, 1)]),
            ],
        );
        assert_eq!(
            selection.filters.into_iter().collect::<Vec<_>>(),
            ["test(/onebox/)"]
        );
        assert_eq!(selection.full, ["templates/topic.html (no rule names it)"]);
    }

    #[test]
    fn parses_hunks_on_the_old_side() {
        let diff = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -5,2 +5,3 @@ fn\n@@ -9,0 +11 @@\n";
        let changes = parse_diff(diff);
        assert_eq!(changes[0].hunks, [(5, 6), (9, 10)]);
    }

    #[test]
    fn globs_match_within_and_across_segments() {
        assert!(glob("docs/**", "docs/a/b.md"));
        assert!(glob("**/*.md", "README.md"));
        assert!(glob("**/*.md", "docs/x/README.md"));
        assert!(!glob("templates/*.html", "templates/a/b.html"));
        assert!(glob("templates/*.html", "templates/b.html"));
    }
}
