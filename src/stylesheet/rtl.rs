//! Rtlcss.flip_css (the rtlcss gem 0.2.1, rtlcss 4 (MIT) under
//! mini_racer) as Stylesheet::Compiler runs it on a right-to-left
//! target's compiled CSS: every declaration through rtlcss's default
//! plugin's processors with its default options (no renaming, no urls,
//! `env(safe-area-inset-*)` swapped, no calc for lengths). Directives
//! (`/*! rtl:... */` comments) are refused.

use std::sync::LazyLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use fancy_regex::{Captures, Regex};

use super::{Decl, Node};
use crate::Unsupported;

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("an rtlcss regexp")
}

/// JavaScript's `\w`.
const W: &str = "[A-Za-z0-9_]";
/// util's number: `\-?(\d*?\.\d+|\d+)`.
const NUMBER: &str = r"\-?([0-9]*?\.[0-9]+|[0-9]+)";
/// A guarded function's token.
const TOKEN: &str = "«[0-9]+:[0-9]+»";

/// `o`: a calc() token or a number not starting a `3d(` name.
static NUMBER_OR_CALC: LazyLock<Regex> =
    LazyLock::new(|| re(&format!("(?i)(calc¤)|({NUMBER})(?!d\\()")));
static TOKENS: LazyLock<Regex> = LazyLock::new(|| re(&format!("(?i){TOKEN}")));
static NAMED_TOKENS: LazyLock<Regex> = LazyLock::new(|| re(&format!("(?i){W}*?{TOKEN}")));
static PLACEHOLDER: LazyLock<Regex> = LazyLock::new(|| re("(?i)¤"));
static COMMENT_PLACEHOLDER: LazyLock<Regex> = LazyLock::new(|| re("(?i)\u{fffd}"));
static COMMENTS: LazyLock<Regex> = LazyLock::new(|| re(r"(?im)/\*[\s\S]*?\*/"));
static FUNCTION: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)\([^()]+\)"));
static HEX: LazyLock<Regex> = LazyLock::new(|| re("(?i)#[a-f0-9]{3,6}"));
static CALC: LazyLock<Regex> = LazyLock::new(|| re("calc"));
static WORDS: LazyLock<Regex> = LazyLock::new(|| re("[^\\s\u{fffd}]+"));

/// util.regex: the alternatives named, in order.
fn util_regex(parts: &[&str], flags: &str) -> Regex {
    let mut out = String::new();
    for part in parts {
        out.push('|');
        match *part {
            "percent" => out.push_str(&format!("({NUMBER}%)")),
            "length" => out.push_str(&format!(
                "({NUMBER})(?:ex|ch|r?em|vh|vw|vmin|vmax|px|mm|cm|in|pt|pc)?"
            )),
            "number" => out.push_str(&format!("({NUMBER})")),
            "position" => out.push_str("(left|center|right|top|bottom)"),
            "calc" => out.push_str(&format!("(calc{TOKEN})")),
            _ => unreachable!("a util.regex part"),
        }
    }
    let case = if flags.contains('i') { "(?i)" } else { "" };
    re(&format!("{case}{}", &out[1..]))
}

fn replace_all(regex: &Regex, text: &str, mut f: impl FnMut(&Captures) -> String) -> String {
    regex.replace_all(text, |c: &Captures| f(c)).into_owned()
}

fn replace_first(regex: &Regex, text: &str, mut f: impl FnMut(&Captures) -> String) -> String {
    regex.replace(text, |c: &Captures| f(c)).into_owned()
}

fn matches(regex: &Regex, text: &str) -> Vec<String> {
    regex
        .find_iter(text)
        .filter_map(|m| m.ok().map(|m| m.as_str().to_string()))
        .collect()
}

fn is_match(regex: &Regex, text: &str) -> bool {
    regex.is_match(text).unwrap_or(false)
}

/// A JavaScript number as `String(n)` writes it.
fn js_number(n: f64) -> Result<String, Unsupported> {
    if n == 0.0 {
        return Ok("0".to_string());
    }
    let abs = n.abs();
    if !(1e-6..1e21).contains(&abs) {
        return Err(Unsupported(
            "rtlcss on a number JavaScript writes with an exponent",
        ));
    }
    Ok(format!("{n}"))
}

/// `Number.parseFloat` of a number util's regexps matched.
fn parse_float(text: &str) -> f64 {
    text.parse::<f64>().unwrap_or(f64::NAN)
}

// ---------------------------------------------------------------------------
// util

/// util.swap: `a` and `b` swapped as whole words.
fn swap(text: &str, a: &str, b: &str, ignore_case: bool) -> String {
    let pattern = format!(
        "{}(?<![A-Za-z0-9_])({}|{})(?![A-Za-z0-9_])",
        if ignore_case { "(?i)" } else { "" },
        fancy_regex::escape(a),
        fancy_regex::escape(b)
    );
    replace_all(&re(&pattern), text, |c| {
        let m = &c[0];
        let is_a = if ignore_case {
            m.to_lowercase() == a.to_lowercase()
        } else {
            m == a
        };
        if is_a { b.to_string() } else { a.to_string() }
    })
}

fn swap_left_right(text: &str) -> String {
    swap(text, "left", "right", true)
}

fn swap_ltr_rtl(text: &str) -> String {
    swap(text, "ltr", "rtl", true)
}

/// util.save: what `pattern` matches stored and replaced by
/// `replacement` (but what `keep` matches).
struct Saved {
    value: String,
    store: Vec<String>,
    restorer: &'static LazyLock<Regex>,
}

fn save(
    pattern: &Regex,
    text: &str,
    replacement: &str,
    restorer: &'static LazyLock<Regex>,
    keep: Option<&Regex>,
) -> Saved {
    let mut store = Vec::new();
    let value = replace_all(pattern, text, |c| {
        let m = c[0].to_string();
        if keep.is_some_and(|k| is_match(k, &m)) {
            m
        } else {
            store.push(m);
            replacement.to_string()
        }
    });
    Saved {
        value,
        store,
        restorer,
    }
}

/// util.restore. A placeholder with nothing stored for it is JavaScript's
/// `undefined`.
fn restore(saved: Saved) -> String {
    let mut stored = saved.store.into_iter();
    replace_all(saved.restorer, &saved.value, |_| {
        stored.next().unwrap_or_else(|| "undefined".to_string())
    })
}

fn save_tokens(text: &str, named: bool) -> Saved {
    if named {
        save(&NAMED_TOKENS, text, "¤", &PLACEHOLDER, Some(&CALC))
    } else {
        save(&TOKENS, text, "¤", &PLACEHOLDER, None)
    }
}

/// negate (the first number), negateAll, complement.
fn arithmetic(text: &str, all: bool, op: &str) -> Result<String, Unsupported> {
    let mut saved = save_tokens(text, false);
    let mut error = None;
    let mut f = |c: &Captures| {
        let m = &c[0];
        if m.contains('¤') {
            return m.replace('¤', &format!("({op}¤)"));
        }
        let n = parse_float(m);
        let result = if op == "-1*" { -n } else { 100.0 - n };
        js_number(result).unwrap_or_else(|e| {
            error = Some(e);
            String::new()
        })
    };
    saved.value = if all {
        replace_all(&NUMBER_OR_CALC, &saved.value, &mut f)
    } else {
        replace_first(&NUMBER_OR_CALC, &saved.value, &mut f)
    };
    match error {
        Some(e) => Err(e),
        None => Ok(restore(saved)),
    }
}

fn negate(text: &str) -> Result<String, Unsupported> {
    arithmetic(text, false, "-1*")
}

fn negate_all(text: &str) -> Result<String, Unsupported> {
    arithmetic(text, true, "-1*")
}

fn complement(text: &str) -> Result<String, Unsupported> {
    arithmetic(text, false, "100% - ")
}

/// util.guard: each innermost `pattern` (a function's parentheses, a hex
/// color) stored and replaced by a token, until none is left.
struct Guarded {
    value: String,
    store: Vec<String>,
    token: String,
}

static GUARDS: AtomicUsize = AtomicUsize::new(0);

fn guard(pattern: &Regex, text: &str, global: bool) -> Guarded {
    let n = GUARDS.fetch_add(1, Ordering::Relaxed) + 1;
    let mut guarded = Guarded {
        value: text.to_string(),
        store: Vec::new(),
        token: format!("«{n}"),
    };
    while is_match(pattern, &guarded.value) {
        let mut stored = Vec::new();
        let token = guarded.token.clone();
        let count = guarded.store.len();
        // JavaScript's replace: every match for a global pattern (the hex
        // colors'), the first for the functions' one.
        let mut f = |c: &Captures| {
            stored.push(c[0].to_string());
            format!("{token}:{}»", count + stored.len())
        };
        guarded.value = if global {
            replace_all(pattern, &guarded.value, &mut f)
        } else {
            replace_first(pattern, &guarded.value, &mut f)
        };
        guarded.store.extend(stored);
    }
    guarded
}

/// util.unguard: the tokens put back, outermost first, each through
/// `update(stored, the word before the token)`.
fn unguard(
    guarded: Guarded,
    mut update: impl FnMut(String, &str) -> Result<String, Unsupported>,
) -> Result<String, Unsupported> {
    let pattern = re(&format!(
        "(?i)({W}*?){}:([0-9]+)»",
        fancy_regex::escape(&guarded.token)
    ));
    let mut value = guarded.value;
    while is_match(&pattern, &value) {
        let mut error = None;
        value = replace_first(&pattern, &value, |c| {
            let word = c[1].to_string();
            let index: usize = c[2].parse().unwrap_or(0);
            let stored = guarded
                .store
                .get(index.wrapping_sub(1))
                .cloned()
                .unwrap_or_else(|| "undefined".to_string());
            match update(stored, &word) {
                Ok(updated) => format!("{word}{updated}"),
                Err(e) => {
                    error = Some(e);
                    String::new()
                }
            }
        });
        if let Some(e) = error {
            return Err(e);
        }
    }
    Ok(value)
}

fn unguard_plain(guarded: Guarded) -> String {
    unguard(guarded, |stored, _| Ok(stored)).expect("no update to fail")
}

// ---------------------------------------------------------------------------
// The processors.

static FOUR_VALUE: LazyLock<Regex> =
    LazyLock::new(|| re("(?i)^(margin|padding|border-(color|style|width))$"));
static BORDER_RADIUS: LazyLock<Regex> = LazyLock::new(|| re("(?i)border-radius"));
static SHADOW: LazyLock<Regex> = LazyLock::new(|| re("(?i)shadow"));
static ORIGIN: LazyLock<Regex> = LazyLock::new(|| re("(?i)(?:transform|perspective)-origin"));
static TRANSFORM: LazyLock<Regex> = LazyLock::new(|| re("(?i)^(?!text-).*?transform$"));
static TRANSITION: LazyLock<Regex> = LazyLock::new(|| re("(?i)transition(-property)?$"));
static BACKGROUND: LazyLock<Regex> =
    LazyLock::new(|| re("(?i)(background|object)(-position(-x)?|-image)?$"));
static KEYWORD: LazyLock<Regex> =
    LazyLock::new(|| re("(?i)float|clear|text-align|justify-(content|items|self)"));
static CURSOR: LazyLock<Regex> = LazyLock::new(|| re("(?i)cursor"));

/// The default plugin's processors, the first whose expression matches
/// the property: the declaration's new property and value.
fn process(prop: &str, value: &str) -> Result<Option<(String, String)>, Unsupported> {
    static VARIABLE: LazyLock<Regex> = LazyLock::new(|| re("(?im)^--"));
    static DIRECTION: LazyLock<Regex> = LazyLock::new(|| re("(?im)direction"));
    static LEFT: LazyLock<Regex> = LazyLock::new(|| re("(?im)left"));
    static RIGHT: LazyLock<Regex> = LazyLock::new(|| re("(?im)right"));
    let value = value.to_string();
    if is_match(&VARIABLE, prop) {
        return Ok(Some((prop.to_string(), value)));
    }
    if is_match(&DIRECTION, prop) {
        return Ok(Some((prop.to_string(), swap_ltr_rtl(&value))));
    }
    if is_match(&LEFT, prop) {
        return Ok(Some((
            replace_first(&LEFT, prop, |_| "right".into()),
            value,
        )));
    }
    if is_match(&RIGHT, prop) {
        return Ok(Some((
            replace_first(&RIGHT, prop, |_| "left".into()),
            value,
        )));
    }
    if is_match(&FOUR_VALUE, prop) {
        return Ok(Some((prop.to_string(), four_value(&value))));
    }
    if is_match(&BORDER_RADIUS, prop) {
        return Ok(Some((prop.to_string(), border_radius(&value))));
    }
    if is_match(&SHADOW, prop) {
        return Ok(Some((prop.to_string(), shadow(&value)?)));
    }
    if is_match(&ORIGIN, prop) {
        return Ok(Some((prop.to_string(), origin(&value)?)));
    }
    if is_match(&TRANSFORM, prop) {
        return Ok(Some((prop.to_string(), transform(&value)?)));
    }
    if is_match(&TRANSITION, prop) {
        return Ok(Some((prop.to_string(), swap_left_right(&value))));
    }
    if is_match(&BACKGROUND, prop) {
        return Ok(Some((prop.to_string(), background(prop, &value)?)));
    }
    if is_match(&KEYWORD, prop) {
        return Ok(Some((prop.to_string(), swap_left_right(&value))));
    }
    if is_match(&CURSOR, prop) {
        return Ok(Some((prop.to_string(), cursor(&value))));
    }
    Ok(None)
}

/// four-value syntax: the second and fourth values swapped.
fn four_value(value: &str) -> String {
    let mut guarded = guard(&FUNCTION, value, false);
    let words = matches(&WORDS, &guarded.value);
    if words.len() == 4 && (!guarded.store.is_empty() || words[1] != words[3]) {
        let mut e = 0;
        guarded.value = replace_all(&WORDS, &guarded.value, |_| {
            let w = words[(4 - e) % 4].clone();
            e += 1;
            w
        });
    }
    unguard_plain(guarded)
}

/// border radius: each side of a `/` flipped.
fn border_radius(value: &str) -> String {
    static SLASH: LazyLock<Regex> = LazyLock::new(|| re("[^/]+"));
    static WHITE: LazyLock<Regex> = LazyLock::new(|| re(r"(^\s*)"));
    let mut guarded = guard(&FUNCTION, value, false);
    guarded.value = replace_all(&SLASH, &guarded.value, |c| {
        let e = c[0].to_string();
        let t = matches(&WORDS, &e);
        match t.len() {
            2 if t[0] != t[1] => {
                let mut r = 1usize;
                replace_all(&WORDS, &e, |_| {
                    let w = t[r].clone();
                    r = r.wrapping_sub(1);
                    w
                })
            }
            3 => replace_first(&WHITE, &e, |c| format!("{}{} ", &c[0], t[1])),
            4 if !(t[0] == t[1] && t[2] == t[3]) => {
                let mut r = 0;
                replace_all(&WORDS, &e, |_| {
                    let w = t[(5 - r) % 4].clone();
                    r += 1;
                    w
                })
            }
            _ => e,
        }
    });
    unguard_plain(guarded)
}

/// shadow: each shadow's first number negated.
fn shadow(value: &str) -> Result<String, Unsupported> {
    static PARTS: LazyLock<Regex> = LazyLock::new(|| re("[^,]+"));
    let mut hex = guard(&HEX, value, true);
    let mut functions = guard(&FUNCTION, &hex.value, false);
    let mut error = None;
    functions.value = replace_all(&PARTS, &functions.value, |c| {
        negate(&c[0]).unwrap_or_else(|e| {
            error = Some(e);
            String::new()
        })
    });
    if let Some(e) = error {
        return Err(e);
    }
    hex.value = unguard_plain(functions);
    Ok(unguard_plain(hex))
}

/// transform and perspective origin: left and right swapped, else the
/// first position flipped.
fn origin(value: &str) -> Result<String, Unsupported> {
    static X_KEYWORD: LazyLock<Regex> = LazyLock::new(|| re("(?i)(left|right)"));
    static MATCH: LazyLock<Regex> =
        LazyLock::new(|| util_regex(&["calc", "percent", "length"], "g"));
    static PERCENT: LazyLock<Regex> = LazyLock::new(|| util_regex(&["calc", "percent"], "i"));
    if is_match(&X_KEYWORD, value) {
        return Ok(swap_left_right(value));
    }
    let mut guarded = guard(&FUNCTION, value, false);
    let mut found = matches(&MATCH, &guarded.value);
    if !found.is_empty() {
        let first = &found[0];
        found[0] = if first == "0" {
            "100%".to_string()
        } else if is_match(&PERCENT, first) {
            complement(first)?
        } else {
            // A length: flipLength without useCalc.
            first.clone()
        };
        let mut found = found.into_iter();
        guarded.value = replace_all(&MATCH, &guarded.value, |_| {
            found.next().unwrap_or_else(|| "undefined".to_string())
        });
    }
    Ok(unguard_plain(guarded))
}

/// transform: the translations and rotations about the vertical axis
/// negated, the skews and the matrices' horizontal parts.
fn transform(value: &str) -> Result<String, Unsupported> {
    static NEGATABLE: LazyLock<Regex> =
        LazyLock::new(|| re("(?i)((translate)(x|3d)?|rotate(z|y)?)$"));
    static UNIT: LazyLock<Regex> = LazyLock::new(|| util_regex(&["calc", "number"], "g"));
    static MATRIX: LazyLock<Regex> = LazyLock::new(|| re("(?i)matrix$"));
    static MATRIX3D: LazyLock<Regex> = LazyLock::new(|| re("(?i)matrix3d$"));
    static SKEW: LazyLock<Regex> = LazyLock::new(|| re("(?i)skew(x|y)?$"));
    static ROTATE3D: LazyLock<Regex> = LazyLock::new(|| re("(?i)rotate3d$"));
    let flip = |args: &str, negated: &[usize]| -> Result<String, Unsupported> {
        let mut index = 0;
        let mut error = None;
        let out = replace_all(&UNIT, args, |c| {
            index += 1;
            if negated.contains(&index) {
                negate(&c[0]).unwrap_or_else(|e| {
                    error = Some(e);
                    String::new()
                })
            } else {
                c[0].to_string()
            }
        });
        match error {
            Some(e) => Err(e),
            None => Ok(out),
        }
    };
    let guarded = guard(&FUNCTION, value, false);
    unguard(guarded, |args, name| {
        if name.is_empty() {
            return Ok(args);
        }
        if is_match(&MATRIX3D, name) {
            flip(&args, &[2, 4, 5, 13])
        } else if is_match(&MATRIX, name) {
            flip(&args, &[2, 3, 5])
        } else if is_match(&ROTATE3D, name) {
            flip(&args, &[1, 4])
        } else if is_match(&SKEW, name) {
            negate_all(&args)
        } else if is_match(&NEGATABLE, name) {
            negate(&args)
        } else {
            Ok(args)
        }
    })
}

/// background: each layer's horizontal position flipped, gradients
/// turned.
fn background(prop: &str, value: &str) -> Result<String, Unsupported> {
    static MATCH: LazyLock<Regex> =
        LazyLock::new(|| util_regex(&["position", "percent", "length", "calc"], "ig"));
    static PERCENT: LazyLock<Regex> = LazyLock::new(|| util_regex(&["calc", "percent"], "i"));
    static POSITION: LazyLock<Regex> = LazyLock::new(|| util_regex(&["position"], "g"));
    static LENGTH: LazyLock<Regex> = LazyLock::new(|| util_regex(&["length"], "gi"));
    static GRADIENT: LazyLock<Regex> = LazyLock::new(|| re("(?i)gradient$"));
    static ANGLE: LazyLock<Regex> = LazyLock::new(|| re("(?i)[0-9]+(deg|g?rad|turn)"));
    let lower = prop.to_lowercase();
    let x_only = lower.ends_with("-x");
    let mut hex = guard(&HEX, value, true);
    let mut functions = guard(&FUNCTION, &hex.value, false);
    if lower != "background-image" {
        let mut layers = Vec::new();
        for layer in functions.value.split(',') {
            let mut saved = save_tokens(layer, true);
            let mut found = matches(&MATCH, &saved.value);
            if !found.is_empty() {
                let positions = matches(&POSITION, &saved.value).len();
                if found.len() >= 3 || positions == 2 {
                    saved.value = swap_left_right(&saved.value);
                } else {
                    if found[0] == "0" {
                        found[0] = "100%".to_string();
                    } else if is_match(&PERCENT, &found[0]) {
                        found[0] = complement(&found[0])?;
                    } else if is_match(&LENGTH, &found[0]) {
                        if x_only {
                            // flipLength without useCalc
                        } else if found.len() == 1 {
                            found[0] = format!("right {} top 50%", found[0]);
                        } else if positions == 0 && found.len() == 2 {
                            found[0] = format!("right {}", found[0]);
                            found[1] = format!("top {}", found[1]);
                        }
                    } else {
                        found[0] = swap_left_right(&found[0]);
                    }
                    let mut found = found.into_iter();
                    saved.value = replace_all(&MATCH, &saved.value, |_| {
                        found.next().unwrap_or_else(|| "undefined".to_string())
                    });
                }
            }
            layers.push(restore(saved));
        }
        functions.value = layers.join(",");
    }
    hex.value = unguard(functions, |args, name| {
        if is_match(&GRADIENT, name) {
            let swapped = swap_left_right(&args);
            if is_match(&ANGLE, &swapped) {
                return negate(&swapped);
            }
            return Ok(swapped);
        }
        Ok(args)
    })?;
    Ok(unguard_plain(hex))
}

/// cursor: the diagonal resize cursors mirrored.
fn cursor(value: &str) -> String {
    static RESIZE: LazyLock<Regex> =
        LazyLock::new(|| re("(?i)(?<![A-Za-z0-9_])(ne|nw|se|sw|nesw|nwse)-resize"));
    static E: LazyLock<Regex> = LazyLock::new(|| re("(?i)e"));
    static WEST: LazyLock<Regex> = LazyLock::new(|| re("(?i)w"));
    static STAR: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)\*"));
    let mut guarded = guard(&FUNCTION, value, false);
    guarded.value = guarded
        .value
        .split(',')
        .map(|part| {
            replace_all(&RESIZE, part, |c| {
                let whole = c[0].to_string();
                let t = c[1].to_string();
                let flipped = replace_first(&E, &t, |_| "*".into());
                let flipped = replace_first(&WEST, &flipped, |_| "e".into());
                let flipped = replace_first(&STAR, &flipped, |_| "w".into());
                whole.replacen(&t, &flipped, 1)
            })
        })
        .collect::<Vec<_>>()
        .join(",");
    unguard_plain(guarded)
}

// ---------------------------------------------------------------------------
// The stylesheet.

/// rtlcss on every declaration of the tree.
pub(super) fn flip(nodes: &mut [Node]) -> Result<(), Unsupported> {
    static DIRECTIVE: LazyLock<Regex> = LazyLock::new(|| re(r"^\s*!?\s*rtl:"));
    for node in nodes {
        match node {
            Node::Decl(d) => flip_decl(d)?,
            Node::Rule(r) => flip(&mut r.nodes)?,
            Node::AtRule(a) => {
                if let Some(kids) = a.nodes.as_mut() {
                    flip(kids)?;
                }
            }
            Node::Comment(c) => {
                let text = c
                    .text
                    .strip_prefix("/*")
                    .and_then(|t| t.strip_suffix("*/"))
                    .unwrap_or(&c.text);
                if is_match(&DIRECTIVE, text) {
                    return Err(Unsupported("rtlcss directives"));
                }
            }
        }
    }
    Ok(())
}

fn flip_decl(decl: &mut Decl) -> Result<(), Unsupported> {
    if decl.value.contains("/*") || decl.between.contains("/*") {
        return Err(Unsupported("comments in a declaration under rtlcss"));
    }
    // saveComments (none here), then processEnv's swap.
    let saved = save(
        &COMMENTS,
        &decl.value,
        "\u{fffd}",
        &COMMENT_PLACEHOLDER,
        None,
    );
    let value = swap(
        &saved.value,
        "safe-area-inset-left",
        "safe-area-inset-right",
        false,
    );
    let Some((prop, value)) = process(&decl.prop, &value)? else {
        return Ok(());
    };
    let value = restore(Saved { value, ..saved });
    if prop != decl.prop || value != decl.value {
        decl.prop = prop;
        decl.value = value;
    }
    Ok(())
}
