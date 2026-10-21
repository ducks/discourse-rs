//! autoprefixer 10.5.6 (MIT) as AssetProcessor#post_css runs it, for
//! frontend/discourse/config/targets.js's browsers: Processor#remove
//! (outdated at-rules, selectors, properties, values and transition
//! entries), then Processor#add (@supports conditions, property and value
//! prefixes, transitions). Its tables for those browsers come from the
//! reference's own autoprefixer and caniuse-lite (CC BY 4.0)
//! (vendor/autoprefixer/discourse-browsers.json, written by
//! scripts/record-autoprefixer-data); the hacks the tables leave to code
//! (text-decoration, text-decoration-skip-ink, user-select, the intrinsic
//! sizes, cross-fade) are ported here.
//!
//! The input is dart-sass's compressed output: nothing before a node, so
//! the visual cascade (only for declarations on lines of their own) never
//! applies. It runs on what the light-dark() polyfill made of it, as
//! postcss runs autoprefixer's OnceExit after every plugin's node
//! visitors. Control comments (`autoprefixer: off`, `autoprefixer grid:`,
//! `ignore next`) are refused; the grid option is off, as in Rails.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use regex::{Captures, Regex};
use serde::Deserialize;

use super::{AtRule, Decl, Node};
use crate::Unsupported;
use crate::stylesheet::value_parser as vp;

// ---------------------------------------------------------------------------
// The tables.

#[derive(Deserialize)]
struct RawOldValue {
    unprefixed: String,
    string: String,
    regexp: (String, String),
}

#[derive(Deserialize)]
struct RawOldSelector {
    prefixed: String,
    regexp: (String, String),
    prefixeds: Vec<(String, (String, String))>,
    unprefixed: String,
    name_regexp: (String, String),
}

#[derive(Deserialize)]
struct RawValue {
    name: String,
    prefixes: Vec<String>,
    kind: String,
    regexp: (String, String),
}

#[derive(Deserialize)]
struct RawData {
    prefixes: Vec<String>,
    normalize: HashMap<String, String>,
    prefixed: HashMap<String, String>,
    add_props: HashMap<String, Vec<String>>,
    add_values: HashMap<String, Vec<RawValue>>,
    remove_props: Vec<String>,
    remove_values: HashMap<String, Vec<RawOldValue>>,
    remove_selectors: Vec<RawOldSelector>,
    remove_at_rules: Vec<String>,
    resolution_prefixes: Vec<String>,
    cleaner_remove_props: Vec<String>,
    cleaner_remove_values: HashMap<String, Vec<RawOldValue>>,
}

/// OldValue: a prefixed value to clean.
struct OldValue {
    unprefixed: String,
    string: String,
    regexp: Regex,
}

impl OldValue {
    fn check(&self, value: &str) -> bool {
        value.contains(&self.string) && self.regexp.is_match(value)
    }
}

/// OldSelector: a prefixed selector to clean.
struct OldSelector {
    prefixed: String,
    regexp: Regex,
    prefixeds: Vec<(String, Regex)>,
    unprefixed: String,
    name_regexp: Regex,
}

#[derive(PartialEq)]
enum ValueKind {
    Plain,
    CrossFade,
    Intrinsic,
}

/// A Value prefixer: a value keyword or function to prefix.
struct ValuePrefixer {
    name: String,
    prefixes: Vec<String>,
    kind: ValueKind,
    regexp: Regex,
}

struct Data {
    prefixes: Vec<String>,
    normalize: HashMap<String, String>,
    prefixed: HashMap<String, String>,
    add_props: HashMap<String, Vec<String>>,
    add_values: HashMap<String, Vec<ValuePrefixer>>,
    remove_props: HashSet<String>,
    remove_values: HashMap<String, Vec<OldValue>>,
    remove_selectors: Vec<OldSelector>,
    remove_at_rules: HashSet<String>,
    resolution_prefixes: Vec<String>,
    cleaner_remove_props: HashSet<String>,
    cleaner_remove_values: HashMap<String, Vec<OldValue>>,
}

/// A JavaScript RegExp's source and flags (`g` is how it is used, `i`
/// the only one that changes matching).
fn regexp((source, flags): &(String, String)) -> Regex {
    let case = if flags.contains('i') { "(?i)" } else { "" };
    Regex::new(&format!("{case}{source}")).expect("an autoprefixer regexp")
}

fn old_values(raw: &HashMap<String, Vec<RawOldValue>>) -> HashMap<String, Vec<OldValue>> {
    raw.iter()
        .map(|(prop, values)| {
            (
                prop.clone(),
                values
                    .iter()
                    .map(|v| OldValue {
                        unprefixed: v.unprefixed.clone(),
                        string: v.string.clone(),
                        regexp: regexp(&v.regexp),
                    })
                    .collect(),
            )
        })
        .collect()
}

static DATA: LazyLock<Data> = LazyLock::new(|| {
    let raw: RawData = serde_json::from_str(include_str!(
        "../../vendor/autoprefixer/discourse-browsers.json"
    ))
    .expect("vendor/autoprefixer/discourse-browsers.json");
    Data {
        prefixes: raw.prefixes,
        normalize: raw.normalize,
        prefixed: raw.prefixed,
        add_props: raw.add_props,
        add_values: raw
            .add_values
            .into_iter()
            .map(|(prop, values)| {
                (
                    prop,
                    values
                        .into_iter()
                        .map(|v| ValuePrefixer {
                            kind: match v.kind.as_str() {
                                "CrossFade" => ValueKind::CrossFade,
                                "Intrinsic" => ValueKind::Intrinsic,
                                "Value" => ValueKind::Plain,
                                other => panic!("an unported value hack: {other}"),
                            },
                            regexp: regexp(&v.regexp),
                            name: v.name,
                            prefixes: v.prefixes,
                        })
                        .collect(),
                )
            })
            .collect(),
        remove_props: raw.remove_props.into_iter().collect(),
        remove_values: old_values(&raw.remove_values),
        remove_selectors: raw
            .remove_selectors
            .iter()
            .map(|s| OldSelector {
                prefixed: s.prefixed.clone(),
                regexp: regexp(&s.regexp),
                prefixeds: s
                    .prefixeds
                    .iter()
                    .map(|(name, r)| (name.clone(), regexp(r)))
                    .collect(),
                unprefixed: s.unprefixed.clone(),
                name_regexp: regexp(&s.name_regexp),
            })
            .collect(),
        remove_at_rules: raw.remove_at_rules.into_iter().collect(),
        resolution_prefixes: raw.resolution_prefixes,
        cleaner_remove_props: raw.cleaner_remove_props.into_iter().collect(),
        cleaner_remove_values: old_values(&raw.cleaner_remove_values),
    }
});

// ---------------------------------------------------------------------------
// Names.

/// `vendor.prefix`: the `-\w+-` a name starts with, or "".
fn vendor_prefix(name: &str) -> &str {
    let bytes = name.as_bytes();
    if bytes.first() != Some(&b'-') {
        return "";
    }
    let word = bytes[1..]
        .iter()
        .take_while(|b| b.is_ascii_alphanumeric() || **b == b'_')
        .count();
    if word > 0 && bytes.get(1 + word) == Some(&b'-') {
        &name[..word + 2]
    } else {
        ""
    }
}

/// `vendor.unprefixed`
fn vendor_unprefixed(name: &str) -> &str {
    &name[vendor_prefix(name).len()..]
}

/// `Prefixes#normalize`: the hack's own name for a property.
fn normalize(prop: &str) -> String {
    DATA.normalize
        .get(prop)
        .cloned()
        .unwrap_or_else(|| prop.to_string())
}

/// `Prefixes#unprefixed`
fn unprefixed(prop: &str) -> String {
    let value = normalize(vendor_unprefixed(prop));
    if value == "flex-direction" {
        "flex-flow".to_string()
    } else {
        value
    }
}

/// `Prefixes#prefixed`
fn prefixed(prop: &str, prefix: &str) -> String {
    let key = format!("{prefix}{}", vendor_unprefixed(prop));
    DATA.prefixed.get(&key).cloned().unwrap_or(key)
}

/// `utils.removeNote`: "-webkit-" of "-webkit- old".
fn remove_note(prefix: &str) -> &str {
    prefix.split(' ').next().unwrap_or(prefix)
}

/// `Browsers.withPrefix`
fn with_prefix(value: &str) -> bool {
    DATA.prefixes.iter().any(|p| value.contains(p.as_str()))
}

/// A browser prefix, or none for any other (`parentPrefix`'s check).
fn known_prefix(prefix: &str) -> Option<String> {
    DATA.prefixes
        .iter()
        .any(|p| p == prefix)
        .then(|| prefix.to_string())
}

/// `Prefixes#values`: the global value cleaners or adders, then the
/// property's.
fn values<'a, T>(table: &'a HashMap<String, Vec<T>>, prop: &str) -> Vec<&'a T> {
    let mut out: Vec<&T> = Vec::new();
    for key in ["*", prop] {
        if let Some(list) = table.get(key) {
            out.extend(list.iter());
        }
        if prop == "*" {
            break;
        }
    }
    out
}

// ---------------------------------------------------------------------------
// postcss's list.split.

fn list_split(string: &str, separators: &[char], last: bool) -> Vec<String> {
    let mut array = Vec::new();
    let mut current = String::new();
    let mut func = 0;
    let mut in_quote = false;
    let mut prev_quote = '\0';
    let mut escape = false;
    for letter in string.chars() {
        let mut split = false;
        if escape {
            escape = false;
        } else if letter == '\\' {
            escape = true;
        } else if in_quote {
            if letter == prev_quote {
                in_quote = false;
            }
        } else if letter == '"' || letter == '\'' {
            in_quote = true;
            prev_quote = letter;
        } else if letter == '(' {
            func += 1;
        } else if letter == ')' {
            if func > 0 {
                func -= 1;
            }
        } else if func == 0 && separators.contains(&letter) {
            split = true;
        }
        if split {
            let value = current.trim().to_string();
            if last || !value.is_empty() {
                array.push(value);
            }
            current.clear();
        } else {
            current.push(letter);
        }
    }
    let value = current.trim().to_string();
    if last || !value.is_empty() {
        array.push(value);
    }
    array
}

/// `list.comma`
fn list_comma(string: &str) -> Vec<String> {
    list_split(string, &[','], true)
}

/// `list.space`
fn list_space(string: &str) -> Vec<String> {
    list_split(string, &[' ', '\n', '\t'], false)
}

// ---------------------------------------------------------------------------
// Walking the tree as postcss's walk* do: depth first, a node's children
// after it, nodes inserted before the current one not visited, a removed
// node's children not visited (what happens to them can't show).

/// What a container passes down: `parentPrefix` of its children, and the
/// selector of the rule a declaration sits in.
#[derive(Clone)]
struct Parent {
    prefix: Option<String>,
    rule_selector: Option<String>,
    root: bool,
}

const ROOT: Parent = Parent {
    prefix: None,
    rule_selector: None,
    root: true,
};

enum Visit {
    /// The node stays, with this many inserted before it.
    Kept(usize),
    Removed,
}

type Visitor<'a> = dyn FnMut(&mut Vec<Node>, usize, &Parent) -> Result<Visit, Unsupported> + 'a;

fn walk(nodes: &mut Vec<Node>, parent: &Parent, f: &mut Visitor<'_>) -> Result<(), Unsupported> {
    let mut i = 0;
    while i < nodes.len() {
        match f(nodes, i, parent)? {
            Visit::Removed => continue,
            Visit::Kept(inserted) => i += inserted,
        }
        match &mut nodes[i] {
            Node::Rule(r) => {
                let inner = Parent {
                    prefix: rule_prefix(&r.selector).unwrap_or_else(|| parent.prefix.clone()),
                    rule_selector: Some(r.selector.clone()),
                    root: false,
                };
                walk(&mut r.nodes, &inner, f)?;
            }
            Node::AtRule(a) => {
                let prefix = if a.name.starts_with('-') {
                    known_prefix(vendor_prefix(&a.name))
                } else {
                    parent.prefix.clone()
                };
                if let Some(kids) = a.nodes.as_mut() {
                    let inner = Parent {
                        prefix,
                        rule_selector: None,
                        root: false,
                    };
                    walk(kids, &inner, f)?;
                }
            }
            Node::Decl(_) | Node::Comment(_) => {}
        }
        i += 1;
    }
    Ok(())
}

/// `parentPrefix` of a rule: the prefix of a `:-prefix-` in its selector
/// (none for an unknown one), else (Option::None) its parent's.
fn rule_prefix(selector: &str) -> Option<Option<String>> {
    static PSEUDO: LazyLock<Regex> = LazyLock::new(|| Regex::new(r":(-[A-Za-z0-9_]+-)").unwrap());
    if !selector.contains(":-") {
        return None;
    }
    PSEUDO
        .captures(selector)
        .map(|c| known_prefix(c.get(1).map_or("", |m| m.as_str())))
}

/// `parentPrefix` of a declaration.
fn decl_prefix(decl: &Decl, parent: &Parent) -> Option<String> {
    if decl.prop.starts_with('-') {
        known_prefix(vendor_prefix(&decl.prop))
    } else {
        parent.prefix.clone()
    }
}

/// `removeChild`, with Root's: the first node's raws.before goes to the
/// next.
fn remove_child(nodes: &mut Vec<Node>, i: usize, parent: &Parent) {
    let removed = nodes.remove(i);
    if parent.root && i == 0 {
        let before = match removed {
            Node::Rule(n) => n.before,
            Node::AtRule(n) => n.before,
            Node::Decl(n) => n.before,
            Node::Comment(n) => n.before,
        };
        match nodes.first_mut() {
            Some(Node::Rule(n)) => n.before = before,
            Some(Node::AtRule(n)) => n.before = before,
            Some(Node::Decl(n)) => n.before = before,
            Some(Node::Comment(n)) => n.before = before,
            None => {}
        }
    }
}

fn decl_at(nodes: &[Node], i: usize) -> Option<&Decl> {
    match nodes.get(i) {
        Some(Node::Decl(d)) => Some(d),
        _ => None,
    }
}

/// `Prefixes#group(decl)`'s up or down: the declarations of the same
/// unprefixed property around the one at `index`.
fn group(nodes: &[Node], index: usize, step: isize, callback: impl Fn(&Decl) -> bool) -> bool {
    let Some(decl) = decl_at(nodes, index) else {
        return false;
    };
    let base = unprefixed(&decl.prop);
    let length = nodes.len() as isize;
    let mut i = index as isize + step;
    while i >= 0 && i < length {
        if let Node::Decl(other) = &nodes[i as usize] {
            if step == -1 && other.prop == base && !with_prefix(&other.value) {
                break;
            }
            if unprefixed(&other.prop) != base {
                break;
            } else if callback(other) {
                return true;
            }
            if step == 1 && other.prop == base && !with_prefix(&other.value) {
                break;
            }
        }
        i += step;
    }
    false
}

fn group_down(nodes: &[Node], index: usize, callback: impl Fn(&Decl) -> bool) -> bool {
    group(nodes, index, 1, callback)
}

fn group_up(nodes: &[Node], index: usize, callback: impl Fn(&Decl) -> bool) -> bool {
    group(nodes, index, -1, callback)
}

fn has_prop(nodes: &[Node], prop: &str) -> bool {
    nodes
        .iter()
        .any(|n| matches!(n, Node::Decl(d) if d.prop == prop))
}

// ---------------------------------------------------------------------------
// The processor.

/// autoprefixer's `Processor#remove` then `Processor#add` on the
/// stylesheet.
pub(super) fn autoprefixer(nodes: &mut Vec<Node>) -> Result<(), Unsupported> {
    refuse_control_comments(nodes)?;
    remove(nodes)?;
    add(nodes)
}

fn refuse_control_comments(nodes: &[Node]) -> Result<(), Unsupported> {
    for node in nodes {
        match node {
            Node::Comment(c) if c.text.to_ascii_lowercase().contains("autoprefixer") => {
                return Err(Unsupported("autoprefixer control comments"));
            }
            Node::Rule(r) => refuse_control_comments(&r.nodes)?,
            Node::AtRule(AtRule {
                nodes: Some(kids), ..
            }) => refuse_control_comments(kids)?,
            _ => {}
        }
    }
    Ok(())
}

fn remove(nodes: &mut Vec<Node>) -> Result<(), Unsupported> {
    // At-rules: the outdated prefixed ones, and the prefixed resolution
    // queries in @media.
    walk(nodes, &ROOT, &mut |nodes, i, parent| {
        let Node::AtRule(a) = &mut nodes[i] else {
            return Ok(Visit::Kept(0));
        };
        let name = format!("@{}", a.name);
        if DATA.remove_at_rules.contains(&name)
            || (name == "@resolution" && !DATA.resolution_prefixes.is_empty())
        {
            remove_child(nodes, i, parent);
            return Ok(Visit::Removed);
        }
        if a.name == "media" && a.params.contains("-resolution") {
            a.params = clean_resolution(&a.params);
        }
        Ok(Visit::Kept(0))
    })?;
    // Selectors.
    walk(nodes, &ROOT, &mut |nodes, i, parent| {
        let Node::Rule(r) = &nodes[i] else {
            return Ok(Visit::Kept(0));
        };
        for checker in &DATA.remove_selectors {
            if r.selector.contains(&checker.prefixed)
                && checker.regexp.is_match(&r.selector)
                && !selector_hack(checker, nodes, i)
            {
                remove_child(nodes, i, parent);
                return Ok(Visit::Removed);
            }
        }
        Ok(Visit::Kept(0))
    })?;
    // Declarations.
    walk(nodes, &ROOT, &mut |nodes, i, _| {
        let Some(decl) = decl_at(nodes, i) else {
            return Ok(Visit::Kept(0));
        };
        let decl = decl.clone();
        if decl.before.as_deref().is_some_and(|b| b.contains('\n')) {
            return Err(Unsupported("autoprefixer's visual cascade"));
        }
        let base = unprefixed(&decl.prop);
        if decl.prop == "transition" || decl.prop == "transition-property" {
            match transition_remove(nodes, i) {
                Visit::Removed => {
                    // Rails goes on with the removed declaration, its index
                    // pointing at the next node.
                    if values(&DATA.remove_values, &base)
                        .iter()
                        .any(|c| c.check(&decl.value))
                    {
                        return Err(Unsupported("a prefixed value in a removed transition"));
                    }
                    return Ok(Visit::Removed);
                }
                Visit::Kept(_) => {}
            }
        }
        let decl = decl_at(nodes, i).expect("the declaration").clone();
        if DATA.remove_props.contains(&decl.prop) {
            let mut not_hack = group_down(nodes, i, |o| normalize(&o.prop) == base);
            if base == "flex-flow" {
                not_hack = true;
            }
            if decl.prop == "-webkit-box-orient"
                && !has_prop(nodes, "flex-direction")
                && !has_prop(nodes, "flex-flow")
            {
                return Ok(Visit::Kept(0));
            }
            let hack_value = (decl.prop == "-webkit-background-clip" && decl.value == "text")
                || (decl.prop == "-webkit-box-orient" && has_prop(nodes, "-webkit-line-clamp"));
            if not_hack && !hack_value {
                nodes.remove(i);
                return Ok(Visit::Removed);
            }
        }
        for checker in values(&DATA.remove_values, &base) {
            if !checker.check(&decl.value) {
                continue;
            }
            if group_down(nodes, i, |o| o.value.contains(&checker.unprefixed)) {
                nodes.remove(i);
                return Ok(Visit::Removed);
            }
        }
        Ok(Visit::Kept(0))
    })
}

/// OldSelector#isHack: the prefixed rule is kept unless the unprefixed
/// one follows it (through other prefixed copies).
fn selector_hack(checker: &OldSelector, nodes: &[Node], i: usize) -> bool {
    for node in &nodes[i + 1..] {
        let Node::Rule(r) = node else {
            return true;
        };
        let before = &r.selector;
        if before.contains(&checker.unprefixed) && checker.name_regexp.is_match(before) {
            return false;
        }
        let some = checker
            .prefixeds
            .iter()
            .any(|(string, regexp)| before.contains(string) && regexp.is_match(before));
        if !some {
            return true;
        }
    }
    true
}

/// `utils.editList`: a comma list filtered, joined with its first comma
/// and the spaces after it (or `, `).
fn edit_list(value: &str, keep: impl Fn(&str) -> bool) -> String {
    let origin = list_comma(value);
    let join = Regex::new(r",\s*")
        .unwrap()
        .find(value)
        .map_or(", ".to_string(), |m| m.as_str().to_string());
    origin
        .into_iter()
        .filter(|q| keep(q))
        .collect::<Vec<_>>()
        .join(&join)
}

/// Resolution#clean: @media queries on a prefixed device-pixel-ratio
/// dropped.
fn clean_resolution(params: &str) -> String {
    if DATA.resolution_prefixes.is_empty() {
        return params.to_string();
    }
    let mut bad = Vec::new();
    for prefix in &DATA.resolution_prefixes {
        for name in ["min", "max"] {
            bad.push(if prefix == "-moz-" {
                format!("{name}--moz-device-pixel-ratio")
            } else {
                format!("{prefix}{name}-device-pixel-ratio")
            });
        }
    }
    edit_list(params, |query| {
        bad.iter().all(|b| !query.contains(b.as_str()))
    })
}

fn add(nodes: &mut Vec<Node>) -> Result<(), Unsupported> {
    // At-rules: @supports conditions (the browsers need no prefixed
    // @keyframes, @viewport or resolution queries).
    walk(nodes, &ROOT, &mut |nodes, i, _| {
        if let Node::AtRule(a) = &mut nodes[i]
            && a.name == "supports"
        {
            a.params = supports(&a.params);
        }
        Ok(Visit::Kept(0))
    })?;
    // Properties (no selector needs a prefix for these browsers).
    walk(nodes, &ROOT, &mut |nodes, i, parent| {
        let Some(decl) = decl_at(nodes, i) else {
            return Ok(Visit::Kept(0));
        };
        // disabledDecl: grid is off.
        if decl.prop.contains("grid") || decl.prop == "justify-items" {
            return Ok(Visit::Kept(0));
        }
        if decl.prop == "transition" || decl.prop == "transition-property" {
            return Ok(Visit::Kept(transition_add(nodes, i, parent)));
        }
        let prop = decl.prop.clone();
        if matches!(prop.as_str(), "align-self" | "justify-self" | "place-self") {
            // Flexbox's align-self needs no prefix here; the grid ones are
            // off.
            return Ok(Visit::Kept(0));
        }
        match DATA.add_props.get(&prop) {
            Some(prefixes) => Ok(Visit::Kept(declaration_process(nodes, i, prefixes, parent))),
            None => Ok(Visit::Kept(0)),
        }
    })?;
    // Values.
    walk(nodes, &ROOT, &mut |nodes, i, parent| {
        let Some(decl) = decl_at(nodes, i) else {
            return Ok(Visit::Kept(0));
        };
        // disabledValue
        if decl.prop == "content" || (decl.prop == "display" && decl.value.contains("grid")) {
            return Ok(Visit::Kept(0));
        }
        let prefix = decl_prefix(decl, parent);
        let found = prefix_values(decl, prefix.as_deref());
        Ok(Visit::Kept(save_values(nodes, i, &found)))
    })
}

// ---------------------------------------------------------------------------
// Declarations.

/// Declaration#process for a property the browsers need prefixed: a
/// prefixed copy before it for each prefix; how many were inserted.
fn declaration_process(
    nodes: &mut Vec<Node>,
    mut i: usize,
    prefixes: &[String],
    parent: &Parent,
) -> usize {
    let decl = decl_at(nodes, i).expect("a declaration").clone();
    // TextDecoration#check: basic values are not prefixed.
    const BASIC: [&str; 8] = [
        "none",
        "underline",
        "overline",
        "line-through",
        "blink",
        "inherit",
        "initial",
        "unset",
    ];
    if decl.prop == "text-decoration"
        && split_whitespace_js(&decl.value).all(|w| BASIC.contains(&w))
    {
        return 0;
    }
    let parent_prefix = decl_prefix(&decl, parent);
    let mut inserted = 0;
    for prefix in prefixes {
        if parent_prefix
            .as_deref()
            .is_some_and(|p| p != remove_note(prefix))
        {
            continue;
        }
        let name = prefixed(&decl.prop, prefix);
        // isAlready
        if group_up(nodes, i, |o| o.prop == name) || group_down(nodes, i, |o| o.prop == name) {
            continue;
        }
        if other_prefixes(&decl.value, prefix) {
            continue;
        }
        // UserSelect#insert
        if decl.prop == "user-select"
            && ((decl.value == "all" && prefix == "-ms-")
                || (decl.value == "contain" && (prefix == "-moz-" || prefix == "-webkit-")))
        {
            continue;
        }
        let mut clone = decl.clone();
        // TextDecorationSkipInk#set
        if decl.prop == "text-decoration-skip-ink" && decl.value == "auto" {
            clone.prop = format!("{prefix}text-decoration-skip");
            clone.value = "ink".to_string();
        } else {
            clone.prop = name;
        }
        if nodes
            .iter()
            .any(|n| matches!(n, Node::Decl(o) if o.prop == clone.prop && o.value == clone.value))
        {
            continue;
        }
        nodes.insert(i, Node::Decl(clone));
        i += 1;
        inserted += 1;
    }
    inserted
}

/// JavaScript's `split(/\s+/)`: empty pieces at the ends kept.
fn split_whitespace_js(value: &str) -> impl Iterator<Item = &str> {
    static WS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());
    WS.split(value)
}

/// Declaration#otherPrefixes: the value names another browser's prefix
/// (outside its first var()).
fn other_prefixes(value: &str, prefix: &str) -> bool {
    static VAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"var\([^)]+\)").unwrap());
    for other in &DATA.prefixes {
        if other == prefix {
            continue;
        }
        if value.contains(other.as_str()) {
            return VAR.replace(value, "").contains(other.as_str());
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Values.

impl ValuePrefixer {
    fn check(&self, value: &str) -> bool {
        value.contains(&self.name) && self.regexp.is_match(value)
    }

    fn stretch(&self) -> bool {
        matches!(self.name.as_str(), "stretch" | "fill" | "fill-available")
    }

    /// `replace`, once over the value.
    fn replace(&self, value: &str, prefix: &str) -> String {
        match self.kind {
            ValueKind::Intrinsic
                if self.stretch() && (prefix == "-moz-" || prefix == "-webkit-") =>
            {
                let keyword = if prefix == "-moz-" {
                    "-moz-available"
                } else {
                    "-webkit-fill-available"
                };
                self.regexp
                    .replace_all(value, |c: &Captures| {
                        format!(
                            "{}{keyword}{}",
                            c.get(1).map_or("", |m| m.as_str()),
                            c.get(3).map_or("", |m| m.as_str())
                        )
                    })
                    .into_owned()
            }
            ValueKind::CrossFade => cross_fade(&self.name, value, prefix),
            _ => self
                .regexp
                .replace_all(value, |c: &Captures| {
                    format!(
                        "{}{prefix}{}",
                        c.get(1).map_or("", |m| m.as_str()),
                        c.get(2).map_or("", |m| m.as_str())
                    )
                })
                .into_owned(),
        }
    }
}

/// CrossFade#replace
fn cross_fade(name: &str, value: &str, prefix: &str) -> String {
    static AMOUNT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d*.?\d+%?").unwrap());
    list_space(value)
        .into_iter()
        .map(|item| {
            let open = format!("{name}(");
            if !item.starts_with(&open) {
                return item;
            }
            let close = item.rfind(')').unwrap_or(item.len());
            let after = item.get(close + 1..).unwrap_or("").to_string();
            let mut args = item[open.len()..close.max(open.len())].to_string();
            if prefix == "-webkit-" {
                match AMOUNT.find(&args).map(|m| m.as_str().to_string()) {
                    Some(amount) => {
                        args = args[amount.len()..].trim().to_string();
                        args.push_str(&format!(", {amount}"));
                    }
                    None => args.push_str(", 0.5"),
                }
            }
            format!("{prefix}{name}({args}){after}")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The Value prefixers of a declaration (Prefixer#process with Value#add
/// each): its prefixed values by prefix, in the order first added.
fn prefix_values(decl: &Decl, parent_prefix: Option<&str>) -> Vec<(String, String)> {
    let base = unprefixed(&decl.prop);
    let mut found: Vec<(String, String)> = Vec::new();
    for prefixer in values(&DATA.add_values, &base) {
        if !prefixer.check(&decl.value) {
            continue;
        }
        for prefix in &prefixer.prefixes {
            if parent_prefix.is_some_and(|p| p != remove_note(prefix)) {
                continue;
            }
            // Intrinsic#add: grid properties only get -webkit-.
            if prefixer.kind == ValueKind::Intrinsic
                && decl.prop.contains("grid")
                && prefix != "-webkit-"
            {
                continue;
            }
            let slot = found.iter().position(|(p, _)| p == prefix);
            let mut value = match slot {
                Some(s) => found[s].1.clone(),
                None => decl.value.clone(),
            };
            loop {
                let before = value.clone();
                value = prefixer.replace(&value, prefix);
                if value == before {
                    break;
                }
            }
            match slot {
                Some(s) => found[s].1 = value,
                None => found.push((prefix.clone(), value)),
            }
        }
    }
    found
}

/// Value.save: a copy of the declaration before it for each prefixed
/// value (or the value itself changed, for a property of that prefix);
/// how many were inserted.
fn save_values(nodes: &mut Vec<Node>, mut i: usize, found: &[(String, String)]) -> usize {
    static FIRST_WS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());
    let mut inserted = 0;
    for (prefix, value) in found {
        let decl = decl_at(nodes, i).expect("a declaration").clone();
        if *value == decl.value {
            continue;
        }
        let prop_prefix = vendor_prefix(&decl.prop);
        if prop_prefix == "-pie-" {
            continue;
        }
        if prop_prefix == prefix {
            if let Some(Node::Decl(d)) = nodes.get_mut(i) {
                d.value = value.clone();
            }
            continue;
        }
        let name = prefixed(&decl.prop, prefix);
        if has_prop(nodes, &name) {
            continue;
        }
        let trimmed = FIRST_WS.replace(value, " ");
        let already = nodes.iter().any(|n| {
            matches!(n, Node::Decl(o) if o.prop == decl.prop && FIRST_WS.replace(&o.value, " ") == trimmed)
        });
        if already {
            continue;
        }
        let mut clone = decl.clone();
        clone.value = value.clone();
        nodes.insert(i, Node::Decl(clone));
        i += 1;
        inserted += 1;
    }
    inserted
}

// ---------------------------------------------------------------------------
// Transitions.

type Param = Vec<vp::Node>;

/// Transition#parse: the value's comma separated parts, each with its
/// comma.
fn transition_params(value: &str) -> Vec<Param> {
    let mut result = Vec::new();
    let mut param = Vec::new();
    for node in vp::parse(value) {
        let div = node.kind == vp::Kind::Div && node.value == ",";
        param.push(node);
        if div {
            result.push(std::mem::take(&mut param));
        }
    }
    result.push(param);
    result.into_iter().filter(|p| !p.is_empty()).collect()
}

/// Transition#findProp
fn find_prop(param: &Param) -> String {
    let prop = param[0].value.clone();
    if prop.starts_with(|c: char| c.is_ascii_digit()) {
        for token in &param[1..] {
            if token.kind == vp::Kind::Word {
                return token.value.clone();
            }
        }
    }
    prop
}

/// Transition#div: the value's first comma, or a new `, `.
fn transition_div(params: &[Param]) -> vp::Node {
    params
        .iter()
        .flatten()
        .find(|n| n.kind == vp::Kind::Div && n.value == ",")
        .cloned()
        .unwrap_or_else(vp::Node::comma)
}

/// Transition#stringify: each part ended by a comma, the first and last
/// comma dropped.
fn transition_stringify(params: &[Param]) -> String {
    if params.is_empty() {
        return String::new();
    }
    let div = transition_div(params);
    let mut nodes: Vec<vp::Node> = Vec::new();
    for param in params {
        nodes.extend(param.iter().cloned());
        if param.last().is_some_and(|n| n.kind != vp::Kind::Div) {
            nodes.push(div.clone());
        }
    }
    if nodes.first().is_some_and(|n| n.kind == vp::Kind::Div) {
        nodes.remove(0);
    }
    if nodes.last().is_some_and(|n| n.kind == vp::Kind::Div) {
        nodes.pop();
    }
    vp::stringify(&nodes)
}

/// Transition#remove: parts naming an outdated prefixed property dropped.
fn transition_remove(nodes: &mut Vec<Node>, i: usize) -> Visit {
    let decl = decl_at(nodes, i).expect("a declaration").clone();
    let params: Vec<Param> = transition_params(&decl.value)
        .into_iter()
        .filter(|p| !DATA.remove_props.contains(&find_prop(p)))
        .collect();
    let value = transition_stringify(&params);
    if decl.value == value {
        return Visit::Kept(0);
    }
    if params.is_empty() {
        nodes.remove(i);
        return Visit::Removed;
    }
    let double = nodes
        .iter()
        .any(|n| matches!(n, Node::Decl(o) if o.prop == decl.prop && o.value == value));
    let smaller = nodes.iter().enumerate().any(|(j, n)| {
        j != i && matches!(n, Node::Decl(o) if o.prop == decl.prop && o.value.len() > value.len())
    });
    if double || smaller {
        nodes.remove(i);
        return Visit::Removed;
    }
    if let Some(Node::Decl(d)) = nodes.get_mut(i) {
        d.value = value;
    }
    Visit::Kept(0)
}

/// Transition#add: the prefixed properties the browsers need added to the
/// transition, and the prefixed copies of the declaration; how many
/// declarations were inserted before it.
fn transition_add(nodes: &mut Vec<Node>, i: usize, parent: &Parent) -> usize {
    let decl = decl_at(nodes, i).expect("a declaration").clone();
    // ruleVendorPrefixes (`transition` itself needs no prefix here).
    let vendor_prefixes: Option<Vec<String>> = parent
        .rule_selector
        .as_deref()
        .filter(|s| s.contains(":-"))
        .map(|s| {
            DATA.prefixes
                .iter()
                .filter(|p| s.contains(&format!(":{p}")))
                .cloned()
                .collect::<Vec<_>>()
        })
        .filter(|v| !v.is_empty());
    let decl_prefixes: Vec<String> = vendor_prefixes.clone().unwrap_or_default();
    let mut params = transition_params(&decl.value);
    let names: Vec<String> = params.iter().map(find_prop).collect();
    if names.iter().any(|n| n.starts_with('-')) {
        return 0;
    }
    let mut added: Vec<Param> = Vec::new();
    for param in &params {
        let prop = find_prop(param);
        let Some(prefixes) = DATA.add_props.get(&prop) else {
            continue;
        };
        for prefix in prefixes {
            if let Some(v) = &vendor_prefixes
                && !v.iter().any(|p| prefix.contains(p.as_str()))
            {
                continue;
            }
            let name = prefixed(&prop, prefix);
            if name != "-ms-transform" && !names.contains(&name) {
                // Transition#clone: the first word naming the property
                // renamed.
                let mut changed = false;
                let clone: Param = param
                    .iter()
                    .map(|n| {
                        if !changed && n.kind == vp::Kind::Word && n.value == prop {
                            changed = true;
                            vp::Node::word(&name)
                        } else {
                            n.clone()
                        }
                    })
                    .collect();
                added.push(clone);
            }
        }
    }
    params.extend(added);
    // The first stringify gives every part without a comma the value's
    // first one (or a new `, `), and the parts keep it for the later ones.
    let div = transition_div(&params);
    for param in &mut params {
        if param.last().is_some_and(|n| n.kind != vp::Kind::Div) {
            param.push(div.clone());
        }
    }
    let value = transition_stringify(&params);
    let webkit_clean = transition_stringify(&clean_from_unprefixed(&params, "-webkit-"));
    let mut before: Vec<(String, String)> = Vec::new();
    if decl_prefixes.iter().any(|p| p == "-webkit-") {
        before.push((format!("-webkit-{}", decl.prop), webkit_clean.clone()));
    }
    before.push((decl.prop.clone(), webkit_clean));
    if decl_prefixes.iter().any(|p| p == "-o-") {
        before.push((
            format!("-o-{}", decl.prop),
            transition_stringify(&clean_from_unprefixed(&params, "-o-")),
        ));
    }
    for prefix in &decl_prefixes {
        if prefix != "-webkit-" && prefix != "-o-" {
            let kept: Vec<Param> = params
                .iter()
                .filter(|p| {
                    let current = vendor_prefix(&find_prop(p)).to_string();
                    current.is_empty() || current == *prefix
                })
                .cloned()
                .collect();
            before.push((
                format!("{prefix}{}", decl.prop),
                transition_stringify(&kept),
            ));
        }
    }
    let mut at = i;
    for (prop, value) in before {
        let already = nodes
            .iter()
            .any(|n| matches!(n, Node::Decl(o) if o.prop == prop && o.value == value));
        if !already {
            let mut clone = decl.clone();
            clone.prop = prop;
            clone.value = value;
            nodes.insert(at, Node::Decl(clone));
            at += 1;
        }
    }
    let already = nodes
        .iter()
        .any(|n| matches!(n, Node::Decl(o) if o.prop == decl.prop && o.value == value));
    if value != decl.value && !already {
        nodes.insert(at, Node::Decl(decl.clone()));
        at += 1;
        if let Some(Node::Decl(d)) = nodes.get_mut(at) {
            d.value = value;
        }
    }
    at - i
}

/// Transition#cleanFromUnprefixed: the parts of that prefix or none, less
/// those whose property also comes with the prefix.
fn clean_from_unprefixed(params: &[Param], prefix: &str) -> Vec<Param> {
    let remove: Vec<String> = params
        .iter()
        .map(find_prop)
        .filter(|p| p.starts_with(prefix))
        .map(|p| unprefixed(&p))
        .collect();
    params
        .iter()
        .filter(|param| {
            let prop = find_prop(param);
            let p = vendor_prefix(&prop);
            !remove.contains(&prop) && (p == prefix || p.is_empty())
        })
        .cloned()
        .collect()
}

// ---------------------------------------------------------------------------
// @supports.

/// A brackets node: text, or a bracketed list.
#[derive(Debug, Clone)]
enum Bracket {
    Text(String),
    List(Vec<Bracket>),
}

/// brackets.parse
fn brackets_parse(text: &str) -> Vec<Bracket> {
    // The stack of open lists, each a path of indexes from the root.
    let mut root: Vec<Bracket> = vec![Bracket::Text(String::new())];
    let mut path: Vec<usize> = Vec::new();
    fn current<'a>(root: &'a mut Vec<Bracket>, path: &[usize]) -> &'a mut Vec<Bracket> {
        let mut list = root;
        for &i in path {
            list = match &mut list[i] {
                Bracket::List(inner) => inner,
                Bracket::Text(_) => unreachable!("a path through lists"),
            };
        }
        list
    }
    for sym in text.chars() {
        if sym == '(' {
            let list = current(&mut root, &path);
            list.push(Bracket::List(vec![Bracket::Text(String::new())]));
            path.push(list.len() - 1);
            continue;
        }
        if sym == ')' {
            if !path.is_empty() {
                path.pop();
                current(&mut root, &path).push(Bracket::Text(String::new()));
            } else if let Some(Bracket::Text(t)) = current(&mut root, &path).last_mut() {
                t.push(sym);
            }
            continue;
        }
        match current(&mut root, &path).last_mut() {
            Some(Bracket::Text(t)) => t.push(sym),
            _ => unreachable!("a list ends with text"),
        }
    }
    root
}

/// brackets.stringify
fn brackets_stringify(nodes: &[Bracket]) -> String {
    let mut out = String::new();
    for node in nodes {
        match node {
            Bracket::List(inner) => {
                out.push('(');
                out.push_str(&brackets_stringify(inner));
                out.push(')');
            }
            Bracket::Text(t) => out.push_str(t),
        }
    }
    out
}

impl Bracket {
    fn text(&self) -> Option<&str> {
        match self {
            Bracket::Text(t) => Some(t),
            Bracket::List(_) => None,
        }
    }

    /// isProp: a list of one text.
    fn prop(&self) -> Option<&str> {
        match self {
            Bracket::List(inner) if inner.len() == 1 => inner[0].text(),
            _ => None,
        }
    }
}

/// Supports#process
fn supports(params: &str) -> String {
    let ast = supports_normalize(brackets_parse(params));
    let ast = supports_remove(ast, params);
    let ast = supports_add(ast);
    let ast = clean_brackets(ast);
    brackets_stringify(&ast)
}

/// Supports#normalize: empty texts dropped, a condition squashed into one
/// text.
fn supports_normalize(nodes: Vec<Bracket>) -> Vec<Bracket> {
    let nodes: Vec<Bracket> = nodes.into_iter().filter(|n| n.text() != Some("")).collect();
    if let Some(first) = nodes.first().and_then(Bracket::text) {
        let first = first.trim();
        if first.contains(':') || first == "selector" || first == "not selector" {
            return vec![Bracket::Text(brackets_stringify(&nodes))];
        }
    }
    nodes
        .into_iter()
        .map(|n| match n {
            Bracket::List(inner) => Bracket::List(supports_normalize(inner)),
            text => text,
        })
        .collect()
}

fn is_not(node: Option<&Bracket>) -> bool {
    static NOT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)not\s*").unwrap());
    node.and_then(Bracket::text)
        .is_some_and(|t| NOT.is_match(t))
}

fn is_or(node: Option<&Bracket>) -> bool {
    static OR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\s*or\s*").unwrap());
    node.and_then(Bracket::text).is_some_and(|t| OR.is_match(t))
}

/// Supports#remove: an outdated prefixed condition before an `or`
/// dropped with it.
fn supports_remove(mut nodes: Vec<Bracket>, all: &str) -> Vec<Bracket> {
    let mut i = 0;
    while i < nodes.len() {
        let prev = if i > 0 { nodes.get(i - 1) } else { None };
        if !is_not(prev) && nodes[i].prop().is_some() && is_or(nodes.get(i + 1)) {
            if supports_to_remove(nodes[i].prop().unwrap_or(""), all) {
                nodes.drain(i..i + 2);
                continue;
            }
            i += 2;
            continue;
        }
        if let Bracket::List(inner) = &mut nodes[i] {
            *inner = supports_remove(std::mem::take(inner), all);
        }
        i += 1;
    }
    nodes
}

/// Supports#parse: a condition's property and value.
fn supports_split(text: &str) -> (String, String) {
    let mut parts = text.split(':');
    let prop = parts.next().unwrap_or("").trim().to_string();
    let value = parts.next().unwrap_or("").trim().to_string();
    (prop, value)
}

/// Supports#toRemove, with the cleaner (every prefix outdated).
fn supports_to_remove(text: &str, all: &str) -> bool {
    let (prop, value) = supports_split(text);
    let base = unprefixed(&prop);
    if DATA.cleaner_remove_props.contains(&prop) {
        // isHack: no unprefixed condition of the property.
        let check = Regex::new(&format!(r"(\(|\s){}:", regex::escape(&base))).unwrap();
        if check.is_match(all) {
            return true;
        }
    }
    values(&DATA.cleaner_remove_values, &base)
        .iter()
        .any(|c| c.check(&value))
}

/// Supports#add: a condition the browsers need prefixed made `(prefixed)
/// or (unprefixed)`.
fn supports_add(nodes: Vec<Bracket>) -> Vec<Bracket> {
    nodes
        .into_iter()
        .map(|node| {
            if let Some(text) = node.prop() {
                let decls = supports_prefixed(text);
                if decls.len() > 1 {
                    let mut out = vec![Bracket::Text(String::new())];
                    for (prop, value) in decls {
                        out.push(Bracket::List(vec![Bracket::Text(format!(
                            "{prop}: {value}"
                        ))]));
                        out.push(Bracket::Text(" or ".to_string()));
                    }
                    *out.last_mut().expect("a condition") = Bracket::Text(String::new());
                    return Bracket::List(out);
                }
                return node;
            }
            match node {
                Bracket::List(inner) => Bracket::List(supports_add(inner)),
                text => text,
            }
        })
        .collect()
}

/// Supports#prefixed: the condition as a declaration in a rule of its
/// own, prefixed as any other.
fn supports_prefixed(text: &str) -> Vec<(String, String)> {
    let (prop, value) = supports_split(text);
    let decl = Decl {
        before: Some(String::new()),
        prop: prop.clone(),
        between: ":".to_string(),
        value,
        important: None,
    };
    let mut nodes = vec![Node::Decl(decl)];
    // Supports#disabled: grid is off.
    let disabled = prop.contains("grid")
        || prop == "justify-items"
        || (prop == "display" && decl_at(&nodes, 0).is_some_and(|d| d.value.contains("grid")));
    if !disabled {
        let parent = Parent {
            prefix: None,
            rule_selector: Some("a".to_string()),
            root: false,
        };
        if let Some(prefixes) = DATA.add_props.get(&prop) {
            declaration_process(&mut nodes, 0, prefixes, &parent);
        }
        // The values of the condition's property, on each declaration.
        let mut i = 0;
        while i < nodes.len() {
            let decl = decl_at(&nodes, i).expect("a declaration").clone();
            let mut found = Vec::new();
            let adders = values(&DATA.add_values, &prop);
            for prefixer in adders {
                if !prefixer.check(&decl.value) {
                    continue;
                }
                let own = decl_prefix(&decl, &parent);
                for prefix in &prefixer.prefixes {
                    if own.as_deref().is_some_and(|p| p != remove_note(prefix)) {
                        continue;
                    }
                    if prefixer.kind == ValueKind::Intrinsic
                        && decl.prop.contains("grid")
                        && prefix != "-webkit-"
                    {
                        continue;
                    }
                    let slot = found
                        .iter()
                        .position(|(p, _): &(String, String)| p == prefix);
                    let mut value = match slot {
                        Some(s) => found[s].1.clone(),
                        None => decl.value.clone(),
                    };
                    loop {
                        let before = value.clone();
                        value = prefixer.replace(&value, prefix);
                        if value == before {
                            break;
                        }
                    }
                    match slot {
                        Some(s) => found[s].1 = value,
                        None => found.push((prefix.clone(), value)),
                    }
                }
            }
            // `for (let decl of rule.nodes)` while Value.save inserts
            // before the current one: the next index is visited, the
            // current declaration again if one was inserted.
            save_values(&mut nodes, i, &found);
            i += 1;
        }
    }
    nodes
        .into_iter()
        .filter_map(|n| match n {
            Node::Decl(d) => Some((d.prop, d.value)),
            _ => None,
        })
        .collect()
}

/// Supports#cleanBrackets: a list of one list is that list.
fn clean_brackets(nodes: Vec<Bracket>) -> Vec<Bracket> {
    nodes
        .into_iter()
        .map(|node| match node {
            Bracket::List(inner) => {
                if inner.len() == 1 && matches!(inner[0], Bracket::List(_)) {
                    match inner.into_iter().next() {
                        Some(Bracket::List(only)) => Bracket::List(clean_brackets(only)),
                        _ => unreachable!("a list of one list"),
                    }
                } else {
                    Bracket::List(clean_brackets(inner))
                }
            }
            text => text,
        })
        .collect()
}
