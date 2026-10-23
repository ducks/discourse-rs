//! The checklist plugin's server side (plugins/checklist): checking a box.
//! The markdown rule that draws the boxes (`span.chcklst-box` with its
//! `data-chk-src="<line>:<nth>"`) is in the markdown crate; the boxes are
//! clicked in static/js/checklist.js, which sends PUT /checklist/toggle
//! (routes/checklist.rs) to [`toggle`]: `Checklist::ToggleCheckbox`, with
//! `CheckboxLocator`, `SourceMap` and `Checkbox` finding each box in the
//! raw, then a PostRevisor revise with `bypass_bump`, `expected_raw` and
//! `preserve_cooked_token`.

use std::cell::OnceCell;
use std::collections::HashSet;
use std::sync::LazyLock;

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, Utc};
use markup5ever_rcdom::Handle;
use regex::Regex;
use serde_json::{Map, Value, json};
use sqlx::PgPool;

use crate::guardian::Guardian;
use crate::posting::Ctx;
use crate::posting::revise::{self, Changes};
use crate::pretty_text::cleanup::{
    all_elements_within, attr, element_name, fragment_root, has_class, parse,
};
use crate::site_settings::{SettingError, SiteSettings};
use crate::{AppError, Unsupported};

/// `Checklist::CheckboxesController::MAX_TOGGLE_REQUESTS_PER_MINUTE`
pub const MAX_TOGGLE_REQUESTS_PER_MINUTE: i64 = 60;
/// `Checklist::ToggleCheckbox::MAX_BATCH_SIZE`
const MAX_BATCH_SIZE: usize = 50;
/// `Post.cook_methods`
const COOK_REGULAR: i32 = 1;
const COOK_RAW_HTML: i32 = 2;

/// `enabled_site_setting :checklist_enabled`
pub fn enabled(settings: &SiteSettings) -> Result<bool, SettingError> {
    Ok(settings.get("checklist_enabled")?.truthy())
}

/// What the decorator (`checklistSyntax`) reads of the post model, for a
/// post whose cooked HTML has boxes: ` data-checklist-post` on its
/// `div.cooked`, the serialized post's `can_edit`, `updated_at` and
/// `version` as JSON, with its topic and number, where the decorator
/// fetches its raw. Ember has the model at hand; the page has only this.
pub fn post_model_attr(post: &Value) -> String {
    let has_boxes = post["cooked"]
        .as_str()
        .is_some_and(|c| c.contains("chcklst-box"));
    if !has_boxes {
        return String::new();
    }
    let model = json!({
        "topic_id": post["topic_id"],
        "post_number": post["post_number"],
        "can_edit": post["can_edit"] == true,
        "updated_at": post["updated_at"],
        "version": post["version"],
    });
    format!(
        " data-checklist-post=\"{}\"",
        crate::topic_list_view::escape(&model.to_string())
    )
}

/// `Checklist::Checkbox`: a box found in the raw, by its byte offset (Ruby
/// counts characters; the replacement is the same) and the marker there.
#[derive(Debug, Clone, PartialEq)]
pub struct Checkbox {
    pub offset: Option<usize>,
    pub segment: Option<String>,
    permanent: bool,
}

impl Checkbox {
    const CHECKED: &'static str = "[x]";
    const UNCHECKED: &'static str = "[ ]";

    pub fn new(offset: usize, segment: &str) -> Checkbox {
        Checkbox {
            offset: Some(offset),
            segment: Some(segment.to_string()),
            permanent: false,
        }
    }

    /// `Checkbox.permanent`: an `[X]`, which nobody toggles.
    pub fn permanent() -> Checkbox {
        Checkbox {
            offset: None,
            segment: None,
            permanent: true,
        }
    }

    pub fn is_permanent(&self) -> bool {
        self.permanent
    }

    pub fn is_toggleable(&self) -> bool {
        !self.permanent && self.offset.is_some()
    }

    pub fn is_checked(&self) -> bool {
        self.segment.as_deref() == Some(Self::CHECKED)
    }

    /// `replace_in(raw, checked:)`: the marker swapped for `[x]` or `[ ]`
    /// (an `[]` grows to three characters). Rails raises ArgumentError
    /// for a box that is not toggleable.
    pub fn replace_in(&self, raw: &str, checked: bool) -> Result<String, &'static str> {
        let (Some(offset), Some(segment)) = (self.offset, self.segment.as_deref()) else {
            return Err("checkbox is not toggleable");
        };
        if self.permanent {
            return Err("checkbox is not toggleable");
        }
        let marker = if checked {
            Self::CHECKED
        } else {
            Self::UNCHECKED
        };
        Ok(format!(
            "{}{marker}{}",
            &raw[..offset],
            &raw[offset + segment.len()..]
        ))
    }
}

/// `Checklist::SourceMap::MARKER_PATTERN`
static MARKER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[[ xX]?\]").unwrap());

/// `Checklist::SourceMap`: a `data-chk-src` location (`line:nth`, the nth
/// marker-shaped text on that physical line, CRLF and CR ending lines as
/// LF does) resolved in the raw.
pub struct SourceMap<'a> {
    raw: &'a str,
    line_starts: Vec<usize>,
}

impl<'a> SourceMap<'a> {
    pub fn new(raw: &'a str) -> SourceMap<'a> {
        // LINE_ENDING_PATTERN: \r\n|\r|\n
        let mut line_starts = vec![0];
        let bytes = raw.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'\r' if bytes.get(i + 1) == Some(&b'\n') => {
                    i += 2;
                    line_starts.push(i);
                }
                b'\r' | b'\n' => {
                    i += 1;
                    line_starts.push(i);
                }
                _ => i += 1,
            }
        }
        SourceMap { raw, line_starts }
    }

    /// `checkbox_at(line:, nth:)`: none past the raw's lines or markers,
    /// or for an `[X]` (TOGGLEABLE_SEGMENTS).
    pub fn checkbox_at(&self, line: usize, nth: usize) -> Option<Checkbox> {
        let start = *self.line_starts.get(line)?;
        let end = self
            .line_starts
            .get(line + 1)
            .copied()
            .unwrap_or(self.raw.len());
        let found = MARKER.find_iter(&self.raw[start..end]).nth(nth)?;
        if !matches!(found.as_str(), "[ ]" | "[x]" | "[]") {
            return None;
        }
        Some(Checkbox::new(start + found.start(), found.as_str()))
    }
}

/// `CheckboxLocator::Target`: a box by its `data-chk-src`, else by its
/// index among the rendered boxes.
#[derive(Debug, Clone, PartialEq)]
pub struct Target {
    pub index: usize,
    pub source: Option<String>,
}

/// `CheckboxLocator::Location`
#[derive(Debug, Clone, PartialEq)]
pub struct Location {
    /// The boxes rendered outside sourced quotes.
    pub count: usize,
    pub checkbox: Option<Checkbox>,
    pub needs_recook: bool,
}

/// `CheckboxLocator::SOURCE_PATTERN`, `\d` being ASCII digits in Ruby.
static SOURCE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([0-9]+):([0-9]+)$").unwrap());

/// A `data-chk-src` as its line and ordinal; None when it is not one (or
/// is too large to be a line, which no raw has).
fn source_of(value: &str) -> Option<Result<(usize, usize), ()>> {
    let captures = SOURCE.captures(value)?;
    Some(
        match (captures[1].parse::<usize>(), captures[2].parse::<usize>()) {
            (Ok(line), Ok(nth)) => Ok((line, nth)),
            _ => Err(()),
        },
    )
}

/// `inside_sourced_quote?`: under an `aside.quote` naming its author, post
/// or topic.
fn inside_sourced_quote(node: &Handle) -> bool {
    let mut current = node.parent.take();
    node.parent.set(current.clone());
    while let Some(weak) = current {
        let Some(ancestor) = weak.upgrade() else {
            break;
        };
        if element_name(&ancestor) == Some("aside")
            && has_class(&ancestor, "quote")
            && ["data-username", "data-post", "data-topic"]
                .iter()
                .any(|a| attr(&ancestor, a).is_some())
        {
            return true;
        }
        current = ancestor.parent.take();
        ancestor.parent.set(current.clone());
    }
    false
}

/// One pass of `CheckboxLocator#build` over a cooked HTML: each target's
/// location. Without valid source hints on every mutable box, or when a
/// hint does not agree with the raw, the locations ask for a recook
/// ([`needs_recook`]), which [`locate`] does once.
pub fn build(raw: &str, cooked: &str, targets: &[Target]) -> Vec<Location> {
    let dom = parse(cooked);
    let nodes: Vec<Handle> = all_elements_within(&fragment_root(&dom))
        .into_iter()
        .filter(|n| element_name(n) == Some("span") && has_class(n, "chcklst-box"))
        .filter(|n| !inside_sourced_quote(n))
        .collect();
    let count = nodes.len();
    // source_hints_valid?
    let mut sources = HashSet::new();
    let hints_valid = nodes.iter().all(|node| {
        if has_class(node, "permanent") {
            return true;
        }
        let hint = attr(node, "data-chk-src").unwrap_or_default();
        source_of(&hint).is_some() && sources.insert(hint)
    });
    if !hints_valid {
        return targets
            .iter()
            .map(|_| Location {
                count,
                checkbox: None,
                needs_recook: true,
            })
            .collect();
    }
    let source_map: OnceCell<SourceMap> = OnceCell::new();
    targets
        .iter()
        .map(|target| {
            let node = match &target.source {
                Some(source) => nodes
                    .iter()
                    .find(|n| attr(n, "data-chk-src").as_deref() == Some(source.as_str())),
                None => nodes.get(target.index),
            };
            let Some(node) = node else {
                return Location {
                    count,
                    checkbox: None,
                    needs_recook: false,
                };
            };
            if has_class(node, "permanent") {
                return Location {
                    count,
                    checkbox: Some(Checkbox::permanent()),
                    needs_recook: false,
                };
            }
            // checkbox_for: the hint's box in the raw, in the state drawn.
            let checkbox = match source_of(&attr(node, "data-chk-src").unwrap_or_default()) {
                Some(Ok((line, nth))) => source_map
                    .get_or_init(|| SourceMap::new(raw))
                    .checkbox_at(line, nth)
                    .filter(|c| c.is_checked() == has_class(node, "checked")),
                _ => None,
            };
            Location {
                count,
                needs_recook: checkbox.is_none(),
                checkbox,
            }
        })
        .collect()
}

pub fn needs_recook(locations: &[Location]) -> bool {
    locations.iter().any(|l| l.needs_recook)
}

/// The post as ToggleCheckbox loads it (`Post.find_by(id:)`, deleted
/// posts left out by Trashable's default scope).
#[derive(sqlx::FromRow)]
struct PostRow {
    id: i32,
    topic_id: i32,
    user_id: Option<i32>,
    last_editor_id: Option<i32>,
    raw: String,
    cooked: String,
    cook_method: i32,
    version: i32,
    updated_at: NaiveDateTime,
}

/// `CheckboxLocator.call_many(post:, targets:)`: from the stored cooked,
/// else from one `post.cook(post.raw)`.
async fn locate(
    pool: &PgPool,
    ctx: &Ctx<'_>,
    post: &PostRow,
    targets: &[Target],
) -> Result<Vec<Location>, AppError> {
    let locations = build(&post.raw, &post.cooked, targets);
    if !needs_recook(&locations) {
        return Ok(locations);
    }
    let cooked = match post.cook_method {
        COOK_RAW_HTML => post.raw.clone(),
        COOK_REGULAR => {
            let omit = match post.user_id {
                Some(uid) => {
                    let mut conn = pool.acquire().await?;
                    crate::posting::create::omit_nofollow(&mut conn, ctx, uid).await?
                }
                None => false,
            };
            crate::pretty_text::cook(
                ctx.host,
                &post.raw,
                &crate::pretty_text::MarkdownOptions {
                    topic_id: Some(i64::from(post.topic_id)),
                    post_id: Some(i64::from(post.id)),
                    user_id: post.last_editor_id.map(i64::from),
                    omit_nofollow: omit,
                    ..Default::default()
                },
            )
            .await?
        }
        _ => return Err(Unsupported("locating checkboxes in emailed posts (EmailCook)").into()),
    };
    Ok(build(&post.raw, &cooked, targets))
}

/// One toggle of the contract, as its `before_validation` casts it.
#[derive(Debug, Clone, PartialEq)]
struct Toggle {
    index: Option<i64>,
    count: Option<i64>,
    /// `checkbox_source.presence`, of whatever type it came as.
    source: Option<Value>,
    checked: Option<bool>,
}

impl Toggle {
    /// A toggle with nothing in it: what an element without `to_h` gives.
    fn empty() -> Toggle {
        Toggle {
            index: None,
            count: None,
            source: None,
            checked: None,
        }
    }

    fn source_str(&self) -> Option<&str> {
        self.source.as_ref().and_then(Value::as_str)
    }
}

/// `ActiveModel::Type::Integer#cast`
fn cast_integer(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Bool(b) => Some(i64::from(*b)),
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f.trunc() as i64)),
        Value::String(s) if crate::ruby::is_blank(s) => None,
        Value::String(s) => Some(crate::ruby::to_i(s)),
        _ => None,
    }
}

/// `ActiveModel::Type::Boolean#cast`
fn cast_boolean(value: Option<&Value>) -> Option<bool> {
    match value? {
        Value::Null => None,
        Value::Bool(b) => Some(*b),
        Value::String(s) if s.is_empty() => None,
        Value::String(s) => Some(!matches!(
            s.as_str(),
            "0" | "f" | "F" | "false" | "FALSE" | "off" | "OFF"
        )),
        Value::Number(n) => Some(n.as_f64() != Some(0.0)),
        _ => Some(true),
    }
}

/// `ActiveModel::Type::String#cast`, for the shapes a JSON body sends.
fn cast_string(value: Option<&Value>) -> Result<Option<String>, Unsupported> {
    Ok(match value {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Bool(b)) => Some(if *b { "t" } else { "f" }.to_string()),
        Some(Value::Number(n)) if n.is_i64() || n.is_u64() => Some(n.to_string()),
        Some(_) => return Err(Unsupported("checklist toggle params of another shape")),
    })
}

/// `Object#presence` on a param.
fn present(value: Option<&Value>) -> Option<Value> {
    match value? {
        Value::Null | Value::Bool(false) => None,
        Value::String(s) if crate::ruby::is_blank(s) => None,
        Value::Array(a) if a.is_empty() => None,
        Value::Object(o) if o.is_empty() => None,
        v => Some(v.clone()),
    }
}

/// `str.split(/,(?!.*\|)|\|(?!.*,)/)`: a comma with no pipe after it on
/// its line, or a pipe with no comma; trailing empty fields dropped.
fn split_list(s: &str) -> Vec<&str> {
    let mut fields = Vec::new();
    let mut start = 0;
    for (i, c) in s.char_indices() {
        let rest = &s[i + 1..];
        let line = rest.split('\n').next().unwrap_or_default();
        let split = match c {
            ',' => !line.contains('|'),
            '|' => !line.contains(','),
            _ => false,
        };
        if split {
            fields.push(&s[start..i]);
            start = i + 1;
        }
    }
    fields.push(&s[start..]);
    while fields.last().is_some_and(|f| f.is_empty()) {
        fields.pop();
    }
    fields
}

/// A toggle element as `toggle.to_h.symbolize_keys` reads it (after the
/// array type's `Integer(item, exception: false) || item`).
fn toggle_of(element: &Value) -> Result<Toggle, Unsupported> {
    let values: Map<String, Value> = match element {
        Value::Object(map) => map.clone(),
        Value::Array(pairs) => {
            // Array#to_h: pairs only, else Rails raises (a 500).
            let mut map = Map::new();
            for pair in pairs {
                match pair.as_array().map(Vec::as_slice) {
                    Some([key, value]) => {
                        if let Value::String(key) = key {
                            map.insert(key.clone(), value.clone());
                        }
                    }
                    _ => return Err(Unsupported("checklist toggles that are not pairs")),
                }
            }
            map
        }
        // nil.to_h; numbers, strings and booleans have no to_h.
        _ => return Ok(Toggle::empty()),
    };
    Ok(Toggle {
        index: cast_integer(values.get("checkbox_index")),
        count: cast_integer(values.get("checkbox_count")),
        source: present(values.get("checkbox_source")),
        checked: cast_boolean(values.get("checked")),
    })
}

/// `attribute :toggles, :array` with the contract's before_validation.
fn cast_toggles(value: Option<&Value>) -> Result<Option<Vec<Toggle>>, Unsupported> {
    Ok(match value {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(split_list(s).iter().map(|_| Toggle::empty()).collect()),
        // ActionDispatch's deep_munge drops the nils of a param's array.
        Some(Value::Array(items)) => Some(
            items
                .iter()
                .filter(|item| !item.is_null())
                .map(toggle_of)
                .collect::<Result<_, _>>()?,
        ),
        Some(object @ Value::Object(_)) => Some(vec![toggle_of(object)?]),
        Some(_) => Some(vec![Toggle::empty()]),
    })
}

/// The contract once valid.
struct Params {
    post_id: i64,
    toggles: Vec<Toggle>,
    expected_raw: String,
    expected_updated_at: String,
    mutation_id: String,
}

/// `params do ... end`: the params, or the contract's full messages.
fn contract(
    ctx: &Ctx<'_>,
    p: &Map<String, Value>,
) -> Result<Result<Params, Vec<String>>, AppError> {
    let blank = ctx.t("errors.messages.blank");
    let too_long = |n: i64| ctx.t_count("errors.messages.too_long", n);
    let post_id = cast_integer(p.get("post_id"));
    let toggles = cast_toggles(p.get("toggles"))?;
    let expected_raw = cast_string(p.get("expected_raw"))?;
    let expected_updated_at = cast_string(p.get("expected_updated_at"))?;
    let mutation_id = cast_string(p.get("mutation_id"))?;
    let is_blank = |s: &Option<String>| s.as_deref().is_none_or(crate::ruby::is_blank);
    let chars = |s: &Option<String>| s.as_deref().map_or(0, |s| s.chars().count());

    let mut errors = Vec::new();
    if post_id.is_none() {
        errors.push(format!("Post {blank}"));
    }
    let toggles_present = toggles.as_ref().is_some_and(|t| !t.is_empty());
    if !toggles_present {
        errors.push(format!("Toggles {blank}"));
    }
    if toggles.as_ref().is_some_and(|t| t.len() > MAX_BATCH_SIZE) {
        errors.push(format!("Toggles {}", too_long(MAX_BATCH_SIZE as i64)));
    }
    let max_post_length = ctx.settings.get("max_post_length")?.to_i();
    if is_blank(&expected_raw) {
        errors.push(format!("Expected raw {blank}"));
    }
    if chars(&expected_raw) as i64 > max_post_length {
        errors.push(format!("Expected raw {}", too_long(max_post_length)));
    }
    if is_blank(&expected_updated_at) {
        errors.push(format!("Expected updated at {blank}"));
    }
    if is_blank(&mutation_id) {
        errors.push(format!("Mutation {blank}"));
    }
    if chars(&mutation_id) > 100 {
        errors.push(format!("Mutation {}", too_long(100)));
    }
    if toggles_present {
        let toggles = toggles.as_deref().unwrap_or_default();
        // toggles_are_valid: one error per bad toggle.
        let invalid = ctx.t("errors.messages.invalid");
        for toggle in toggles {
            let valid_index = toggle.index.is_some_and(|i| i >= 0);
            let valid_count = toggle.count.is_some_and(|c| c > 0);
            let valid_source = toggle
                .source_str()
                .is_some_and(|s| SOURCE.is_match(s) && s.chars().count() <= 30);
            let valid_target = if toggle.source.is_some() {
                valid_source
            } else {
                valid_count
            };
            if !valid_index || !valid_target || toggle.checked.is_none() {
                errors.push(format!("Toggles {invalid}"));
            }
        }
        // toggle_targets_are_unique
        let targets: Vec<Value> = toggles
            .iter()
            .map(|t| {
                t.source.clone().unwrap_or_else(|| {
                    json!(format!(
                        "index:{}",
                        t.index.map(|i| i.to_string()).unwrap_or_default()
                    ))
                })
            })
            .collect();
        let unique = targets
            .iter()
            .enumerate()
            .all(|(i, t)| !targets[..i].contains(t));
        if !unique {
            errors.push(format!("Toggles {}", ctx.t("errors.messages.taken")));
        }
    }
    if !errors.is_empty() {
        return Ok(Err(errors));
    }
    Ok(Ok(Params {
        post_id: post_id.unwrap_or_default(),
        toggles: toggles.unwrap_or_default(),
        expected_raw: expected_raw.unwrap_or_default(),
        expected_updated_at: expected_updated_at.unwrap_or_default(),
        mutation_id: mutation_id.unwrap_or_default(),
    }))
}

/// `Time.iso8601` (Time.xmlschema): a date and time with optional
/// fraction and zone, a time without a zone being the server's (UTC). Out
/// of range days, an hour of 24 and a 60th second roll over as Time.utc
/// does; None where Ruby raises ArgumentError.
fn parse_iso8601(s: &str) -> Option<DateTime<Utc>> {
    static PATTERN: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"(?i)^[ \t\r\n\f\x0b]*(-?[0-9]+)-([0-9]{2})-([0-9]{2})T([0-9]{2}):([0-9]{2}):([0-9]{2})(\.[0-9]+)?(Z|[+-][0-9]{2}(?::?[0-9]{2})?)?[ \t\r\n\f\x0b]*$",
        )
        .unwrap()
    });
    let c = PATTERN.captures(s)?;
    let year: i32 = c[1].parse().ok()?;
    let num = |i: usize| c[i].parse::<i64>().ok();
    let (mon, day, hour, min, sec) = (num(2)?, num(3)?, num(4)?, num(5)?, num(6)?);
    if !(1..=12).contains(&mon)
        || !(1..=31).contains(&day)
        || !(0..=24).contains(&hour)
        || !(0..=59).contains(&min)
        || !(0..=60).contains(&sec)
        || (hour == 24 && (min > 0 || sec > 0))
    {
        return None;
    }
    // The fraction to the nanosecond; the comparison floors milliseconds.
    let nanos = c.get(7).map_or(0, |f| {
        let digits: String = f.as_str()[1..]
            .chars()
            .chain("000000000".chars())
            .take(9)
            .collect();
        digits.parse::<i64>().unwrap_or(0)
    });
    let offset = match c.get(8).map(|z| z.as_str()) {
        None => 0,
        Some(z) if z.eq_ignore_ascii_case("z") => 0,
        Some(z) => {
            let sign = if z.starts_with('-') { -1 } else { 1 };
            let digits: String = z[1..].chars().filter(char::is_ascii_digit).collect();
            let hours: i64 = digits[..2].parse().ok()?;
            let minutes: i64 = digits.get(2..4).map_or(Some(0), |m| m.parse().ok())?;
            sign * (hours * 3600 + minutes * 60)
        }
    };
    let first = NaiveDate::from_ymd_opt(year, mon as u32, 1)?.and_hms_opt(0, 0, 0)?;
    let at = first
        .checked_add_signed(Duration::days(day - 1))?
        .checked_add_signed(Duration::seconds(hour * 3600 + min * 60 + sec - offset))?
        .checked_add_signed(Duration::nanoseconds(nanos))?;
    Some(at.and_utc())
}

/// `post_unchanged`: the raw and the update time (to the millisecond)
/// the client saw.
fn post_unchanged(post: &PostRow, params: &Params) -> bool {
    post.raw == params.expected_raw
        && parse_iso8601(&params.expected_updated_at)
            .is_some_and(|t| t.timestamp_millis() == post.updated_at.and_utc().timestamp_millis())
}

/// `ToggleCheckbox.normalize_checkbox_states`
fn normalize_checkbox_states(raw: &str) -> String {
    static STATES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[(?: |x)?\]").unwrap());
    STATES.replace_all(raw, "[ ]").into_owned()
}

/// What PUT /checklist/toggle answers.
#[derive(Debug)]
pub enum Answer {
    /// 200: the post after the toggles.
    Done(Value),
    /// 400 `on_failed_contract`: the contract's full messages.
    InvalidContract(Vec<String>),
    /// 404 Discourse::NotFound.
    NotFound,
    /// 403 Discourse::InvalidAccess.
    Forbidden,
    /// 409 render_checklist_conflict: the post reloaded.
    Conflict {
        raw: String,
        updated_at: String,
        retryable: bool,
    },
    /// 422 with one of the plugin's errors.
    Unprocessable(String),
    /// 422 `on_failure` with no errors.
    Failed,
    /// The edit limiter refused the revision (RateLimiter::LimitExceeded).
    RateLimited(crate::rate_limiter::LimitExceeded),
}

async fn load_post(pool: &PgPool, post_id: i32) -> Result<Option<PostRow>, AppError> {
    Ok(sqlx::query_as(
        "SELECT id, topic_id, user_id, last_editor_id, raw, cooked, cook_method, version, updated_at \
         FROM posts WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(post_id)
    .fetch_optional(pool)
    .await?)
}

/// `render_checklist_conflict(post, retryable:)`, the post reloaded;
/// `retryable` asked of the reloaded raw (`retryable_conflict?`) when
/// `expected_raw` is given.
async fn conflict(
    pool: &PgPool,
    post_id: i32,
    expected_raw: Option<&str>,
) -> Result<Answer, AppError> {
    let (raw, updated_at): (String, NaiveDateTime) =
        sqlx::query_as("SELECT raw, updated_at FROM posts WHERE id = $1")
            .bind(post_id)
            .fetch_one(pool)
            .await?;
    let retryable = expected_raw.is_some_and(|expected| {
        normalize_checkbox_states(expected) == normalize_checkbox_states(&raw)
    });
    Ok(Answer::Conflict {
        updated_at: crate::topic_list::time_json(updated_at),
        raw,
        retryable,
    })
}

/// `Checklist::ToggleCheckbox.call(params:, guardian:)` as the controller
/// answers each outcome.
pub async fn toggle(
    pool: &PgPool,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    p: &Map<String, Value>,
) -> Result<Answer, AppError> {
    let params = match contract(ctx, p)? {
        Ok(params) => params,
        Err(errors) => return Ok(Answer::InvalidContract(errors)),
    };
    // policy :checklist_enabled (requires_plugin answers first).
    if !enabled(ctx.settings)? {
        return Ok(Answer::Failed);
    }
    // model :post
    let Ok(post_id) = i32::try_from(params.post_id) else {
        return Ok(Answer::NotFound);
    };
    let Some(post) = load_post(pool, post_id).await? else {
        return Ok(Answer::NotFound);
    };
    // policy :can_edit_post
    let can_edit = {
        let mut conn = pool.acquire().await?;
        match crate::posting::revisions::find_post_with_deleted(&mut conn, ctx, guardian, post.id)
            .await?
        {
            Some(access) => access.can_edit(ctx, guardian)?,
            None => false,
        }
    };
    if !can_edit {
        return Ok(Answer::Forbidden);
    }
    // policy :post_unchanged
    if !post_unchanged(&post, &params) {
        return conflict(pool, post.id, Some(&params.expected_raw)).await;
    }
    // model :resolved_toggles
    let targets: Vec<Target> = params
        .toggles
        .iter()
        .map(|t| Target {
            index: t.index.unwrap_or_default() as usize,
            source: t.source_str().map(str::to_string),
        })
        .collect();
    let locations = locate(pool, ctx, &post, &targets).await?;
    // policy :checkbox_counts_unchanged
    let counts_unchanged = params
        .toggles
        .iter()
        .zip(&locations)
        .all(|(toggle, location)| {
            toggle.source.is_some() || toggle.count == Some(location.count as i64)
        });
    if !counts_unchanged {
        return conflict(pool, post.id, None).await;
    }
    // policy :checkboxes_found
    let Some(checkboxes) = locations
        .iter()
        .map(|l| l.checkbox.clone())
        .collect::<Option<Vec<Checkbox>>>()
    else {
        return conflict(pool, post.id, None).await;
    };
    // policy :checkboxes_toggleable
    if !checkboxes.iter().all(Checkbox::is_toggleable) {
        return Ok(Answer::Unprocessable(ctx.t("checklist.checkbox_locked")));
    }
    let wanted = |i: usize| params.toggles[i].checked == Some(true);
    let mut changed: Vec<(usize, &Checkbox)> = checkboxes
        .iter()
        .enumerate()
        .filter(|(i, c)| c.is_checked() != wanted(*i))
        .collect();
    // only_if :checkbox_states_differ { step :revise_post }
    if !changed.is_empty() {
        changed.sort_by_key(|(_, c)| std::cmp::Reverse(c.offset));
        let mut new_raw = post.raw.clone();
        for (i, checkbox) in &changed {
            new_raw = checkbox
                .replace_in(&new_raw, wanted(*i))
                .map_err(Unsupported)?;
        }
        let outcome = revise::revise(
            pool,
            ctx,
            guardian,
            post.id,
            Changes {
                raw: Some(new_raw),
                bypass_bump: true,
                expected_raw: Some(post.raw.clone()),
                preserve_cooked_token: Some(params.mutation_id.clone()),
                ..Default::default()
            },
        )
        .await?;
        match outcome {
            revise::Outcome::Revised => {}
            revise::Outcome::EditConflict => {
                return conflict(pool, post.id, Some(&params.expected_raw)).await;
            }
            revise::Outcome::Invalid(_) => {
                return Ok(Answer::Unprocessable(ctx.t("checklist.revision_failed")));
            }
            revise::Outcome::RateLimited(refused) => return Ok(Answer::RateLimited(refused)),
        }
    }
    // on_success: the post as revised (or as loaded).
    let (cooked, last_editor_id, raw, updated_at, version): (
        String,
        Option<i32>,
        String,
        NaiveDateTime,
        i32,
    ) = if changed.is_empty() {
        (
            post.cooked,
            post.last_editor_id,
            post.raw,
            post.updated_at,
            post.version,
        )
    } else {
        sqlx::query_as(
            "SELECT cooked, last_editor_id, raw, updated_at, version FROM posts WHERE id = $1",
        )
        .bind(post.id)
        .fetch_one(pool)
        .await?
    };
    Ok(Answer::Done(json!({
        "cooked": cooked,
        "last_editor_id": last_editor_id,
        "raw": raw,
        "revised": !changed.is_empty(),
        "updated_at": crate::topic_list::time_json(updated_at),
        "version": version,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pretty_text::render::{self, RenderSettings};

    fn settings() -> RenderSettings {
        serde_json::from_value(json!({
            "breaks": true, "linkify_tlds": null, "typographer": false, "mentions": true,
            "emoji": true, "emoji_shortcuts": true, "inline_emoji": true,
            "emoji_set": "twitter", "emoji_base_path": "/images/emoji",
            "default_code_lang": "auto", "heading_anchor_label": "Heading anchor",
            "allowed_iframes": [], "allowed_href_schemes": [], "checklist": true,
            "footnotes": true, "poll": true, "poll_maximum_options": 20,
            "poll_voters_label": "voters", "local_dates": true,
            "local_dates_email_format": "YYYY-MM-DDTHH:mm:ss[Z]",
            "local_dates_email_timezone": "Etc/UTC", "spoiler": true, "policy": false,
            "avatar_sizes": [24, 48], "base_path": "", "secure_uploads": false,
            "topic_id": 1, "force_quote_link": false, "post_id": null, "chat": false,
        }))
        .unwrap()
    }

    fn cook(raw: &str) -> String {
        render::render(raw, &settings(), Default::default())
            .unwrap()
            .0
    }

    fn strip_hints(cooked: &str) -> String {
        Regex::new(r#" data-chk-src="[^"]+""#)
            .unwrap()
            .replace_all(cooked, "")
            .into_owned()
    }

    /// `CheckboxLocator.call(post:, index:)`: the stored cooked, else one
    /// recook (`recook`), counted in `cooks`.
    fn locate_with(
        raw: &str,
        index: usize,
        cooked: &str,
        recook: &str,
        cooks: &mut usize,
    ) -> Location {
        let targets = [Target {
            index,
            source: None,
        }];
        let locations = build(raw, cooked, &targets);
        if !needs_recook(&locations) {
            return locations[0].clone();
        }
        *cooks += 1;
        build(raw, recook, &targets)[0].clone()
    }

    fn locate(raw: &str, index: usize) -> Location {
        let cooked = cook(raw);
        let mut cooks = 0;
        locate_with(raw, index, &cooked, &cooked, &mut cooks)
    }

    fn replace(location: &Location, raw: &str, checked: bool) -> String {
        location
            .checkbox
            .as_ref()
            .expect("a checkbox")
            .replace_in(raw, checked)
            .unwrap()
    }

    #[test]
    fn source_map_resolves_ordinals_against_crlf_and_unicode_raw() {
        let raw = "🎉 ![](<https://example.test/[x]>)[ ] visible\r\n[x] second";
        let map = SourceMap::new(raw);
        let first = map.checkbox_at(0, 2).unwrap();
        let second = map.checkbox_at(1, 0).unwrap();
        assert!(
            first
                .replace_in(raw, true)
                .unwrap()
                .starts_with("🎉 ![](<https://example.test/[x]>)[x] visible")
        );
        assert!(
            second
                .replace_in(raw, false)
                .unwrap()
                .ends_with("\r\n[ ] second")
        );
    }

    #[test]
    fn source_map_rejects_permanent_and_invalid_locations() {
        let map = SourceMap::new("[X] permanent\n[ ] regular");
        assert_eq!(map.checkbox_at(0, 0), None);
        assert_eq!(map.checkbox_at(10, 0), None);
        assert_eq!(map.checkbox_at(1, 10), None);
    }

    #[test]
    fn checkbox_states_and_replacement() {
        assert!(Checkbox::new(0, "[x]").is_checked());
        assert!(!Checkbox::new(0, "[ ]").is_checked());
        assert!(!Checkbox::new(0, "[]").is_checked());
        assert!(Checkbox::permanent().is_permanent());
        assert!(!Checkbox::new(0, "[x]").is_permanent());
        assert!(Checkbox::new(0, "[ ]").is_toggleable());
        assert!(!Checkbox::permanent().is_toggleable());
        let unlocated = Checkbox {
            offset: None,
            segment: None,
            permanent: false,
        };
        assert!(!unlocated.is_toggleable());
        assert_eq!(
            Checkbox::new(2, "[ ]").replace_in("- [ ] first", true),
            Ok("- [x] first".to_string())
        );
        assert_eq!(
            Checkbox::new(0, "[x]").replace_in("[x] done", false),
            Ok("[ ] done".to_string())
        );
        assert_eq!(
            Checkbox::new(0, "[]").replace_in("[] first", true),
            Ok("[x] first".to_string())
        );
        assert_eq!(
            Checkbox::permanent().replace_in("[X] fixed", false),
            Err("checkbox is not toggleable")
        );
        // Ruby's offset 9 is in characters; the byte offset after 🎉 is 12.
        let raw = "🎉 party\n\n[ ] task";
        let offset = raw.find("[ ]").unwrap();
        assert_eq!(
            Checkbox::new(offset, "[ ]").replace_in(raw, true),
            Ok("🎉 party\n\n[x] task".to_string())
        );
    }

    #[test]
    fn uses_source_hints_from_stored_cooked_without_recooking() {
        let raw = "- [ ] first\n- [x] second";
        let cooked = cook(raw);
        let mut cooks = 0;
        let first = locate_with(raw, 0, &cooked, "", &mut cooks);
        let second = locate_with(raw, 1, &cooked, "", &mut cooks);
        assert_eq!(cooks, 0);
        let first_box = first.checkbox.unwrap();
        let second_box = second.checkbox.unwrap();
        assert_eq!(
            (first.count, first_box.offset, first_box.segment.as_deref()),
            (2, Some(2), Some("[ ]"))
        );
        assert_eq!(
            (
                second.count,
                second_box.offset,
                second_box.segment.as_deref()
            ),
            (2, Some(14), Some("[x]"))
        );
    }

    #[test]
    fn performs_one_fallback_cook_for_legacy_stored_html() {
        let raw = "- [ ] first\n- [x] second";
        let cooked = cook(raw);
        let mut cooks = 0;
        let location = locate_with(raw, 1, &strip_hints(&cooked), &cooked, &mut cooks);
        assert_eq!(cooks, 1);
        assert_eq!(replace(&location, raw, false), "- [ ] first\n- [ ] second");
    }

    #[test]
    fn ignores_marker_shaped_text_markdown_does_not_render_as_controls() {
        let raw = "`[x]`\n*[x]*\n~~[x]~~\n[x](https://example.com)\n[code][x][/code]\n```\n[x]\n```\n[ ] first\n[x] second\n";
        let first = locate(raw, 0);
        let second = locate(raw, 1);
        assert_eq!(first.count, 2);
        assert_eq!(
            first.checkbox.as_ref().unwrap().segment.as_deref(),
            Some("[ ]")
        );
        assert!(replace(&first, raw, true).ends_with("[x] first\n[x] second\n"));
        assert!(replace(&second, raw, false).ends_with("[ ] first\n[ ] second\n"));
    }

    #[test]
    fn maps_past_checkbox_shaped_image_and_link_metadata() {
        let raw = "![](<https://example.test/[x]>)[x]\n[link](<https://example.test/[x]> \"[x]\") [ ] visible\n";
        let image_line = locate(raw, 0);
        let link_line = locate(raw, 1);
        assert!(replace(&image_line, raw, false).starts_with("![](<https://example.test/[x]>)[ ]"));
        assert!(
            replace(&link_line, raw, true)
                .ends_with("[link](<https://example.test/[x]> \"[x]\") [x] visible\n")
        );
    }

    #[test]
    fn excludes_sourced_quote_checkboxes_but_maps_anonymous_ones() {
        let raw = "[quote=\"Other user\"]\n[ ] attributed quote\n[/quote]\n\n[quote=\", post: 1\"]\n[ ] post quote\n[/quote]\n\n[quote]\n[ ] anonymous quote\n[/quote]\n\n[ ] own\n";
        let anonymous_quote = locate(raw, 0);
        let own = locate(raw, 1);
        assert_eq!(anonymous_quote.count, 2);
        assert!(replace(&anonymous_quote, raw, true).contains("[x] anonymous quote"));
        assert!(replace(&own, raw, true).ends_with("[x] own\n"));
    }

    #[test]
    fn maps_checkboxes_in_blockquotes_lists_and_table_cells() {
        let raw = "> - [ ] quoted\n\n| a | b |\n|---|---|\n| [x] no | [ ] go |\n";
        assert!(replace(&locate(raw, 0), raw, true).contains("> - [x] quoted"));
        assert!(replace(&locate(raw, 1), raw, false).contains("| [ ] no | [ ] go |"));
        assert!(replace(&locate(raw, 2), raw, true).contains("| [x] no | [x] go |"));
    }

    #[test]
    fn handles_date_range_bbcode_before_checkboxes() {
        let raw = "[date-range from=2024-03-22 to=2024-03-23]\n\n[ ] task 1\n[x] task 2\n";
        assert!(replace(&locate(raw, 0), raw, true).contains("[x] task 1"));
        assert!(replace(&locate(raw, 1), raw, false).contains("[ ] task 2"));
    }

    #[test]
    fn handles_escaped_markers_unicode_legacy_markers_and_crlf() {
        let raw = "🎉 \\[x] escaped\r\n[] first\r\n[x] second";
        assert_eq!(
            replace(&locate(raw, 0), raw, true),
            "🎉 \\[x] escaped\r\n[x] first\r\n[x] second"
        );
        assert_eq!(
            replace(&locate(raw, 1), raw, false),
            "🎉 \\[x] escaped\r\n[] first\r\n[ ] second"
        );
    }

    #[test]
    fn identifies_permanent_checkboxes_without_a_source_hint() {
        let raw = "[X] permanent\n[x] regular";
        let permanent = locate(raw, 0);
        let regular = locate(raw, 1);
        assert_eq!(permanent.count, 2);
        assert!(permanent.checkbox.unwrap().is_permanent());
        assert_eq!(regular.count, 2);
        assert!(regular.checkbox.unwrap().is_toggleable());
    }

    #[test]
    fn an_invalid_index_has_the_count_without_a_checkbox() {
        let location = locate("[ ] only", 2);
        assert_eq!((location.count, location.checkbox), (1, None));
    }

    #[test]
    fn fails_closed_when_a_contextual_cook_has_no_source_hints() {
        let raw = "[ ] only";
        let hintless = strip_hints(&cook(raw));
        let mut cooks = 0;
        let location = locate_with(raw, 0, &hintless, &hintless, &mut cooks);
        assert_eq!(cooks, 1);
        assert_eq!((location.count, location.checkbox), (1, None));
    }

    #[test]
    fn fails_closed_when_a_source_hint_does_not_agree_with_raw() {
        let raw = "[ ] first\n[ ] second";
        let fresh = cook(raw);
        let cooked = fresh.replacen("data-chk-src=\"0:0\"", "data-chk-src=\"1:0\"", 1);
        let mut cooks = 0;
        let location = locate_with(raw, 0, &cooked, &fresh, &mut cooks);
        assert_eq!(cooks, 1);
        assert_eq!(location.checkbox.unwrap().offset, Some(0));
    }

    #[test]
    fn a_source_target_finds_its_box() {
        let raw = "[ ] first\n[x] second";
        let cooked = cook(raw);
        let located = build(
            raw,
            &cooked,
            &[Target {
                index: 0,
                source: Some("1:0".into()),
            }],
        );
        assert_eq!(located[0].checkbox, Some(Checkbox::new(10, "[x]")));
        let missing = build(
            raw,
            &cooked,
            &[Target {
                index: 0,
                source: Some("5:0".into()),
            }],
        );
        assert_eq!(
            (missing[0].checkbox.clone(), missing[0].needs_recook),
            (None, false)
        );
    }

    #[test]
    fn toggles_cast_as_the_contract_does() {
        assert_eq!(split_list("a,b|c"), vec!["a,b", "c"]);
        assert_eq!(split_list("1,2"), vec!["1", "2"]);
        assert_eq!(split_list("a,,"), vec!["a"]);
        assert_eq!(split_list(""), Vec::<&str>::new());
        assert_eq!(cast_integer(Some(&json!("abc"))), Some(0));
        assert_eq!(cast_integer(Some(&json!(" 7 "))), Some(7));
        assert_eq!(cast_integer(Some(&json!(1.9))), Some(1));
        assert_eq!(cast_integer(Some(&json!(" "))), None);
        assert_eq!(cast_integer(Some(&json!([1]))), None);
        assert_eq!(cast_boolean(Some(&json!(" "))), Some(true));
        assert_eq!(cast_boolean(Some(&json!("off"))), Some(false));
        assert_eq!(cast_boolean(Some(&json!(""))), None);
        assert_eq!(cast_boolean(Some(&json!(0))), Some(false));
        let toggles = cast_toggles(Some(&json!([5, null, [["checkbox_index", 1]]])))
            .unwrap()
            .unwrap();
        assert_eq!(toggles.len(), 2);
        assert_eq!(toggles[0], Toggle::empty());
        assert_eq!(toggles[1].index, Some(1));
        assert!(cast_toggles(Some(&json!([[1]]))).is_err());
    }

    #[test]
    fn iso8601_times_parse_as_ruby_does() {
        let ms = |s: &str| parse_iso8601(s).map(|t| t.timestamp_millis());
        let at = |s: &str| {
            NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.f")
                .unwrap()
                .and_utc()
                .timestamp_millis()
        };
        assert_eq!(ms("2026-01-01"), None);
        assert_eq!(ms("2026-01-01T10:00Z"), None);
        assert_eq!(ms("2026-01-01 10:00:00Z"), None);
        assert_eq!(
            ms(" 2026-01-01T10:00:00.1239Z "),
            Some(at("2026-01-01 10:00:00.123"))
        );
        assert_eq!(ms("2026-01-01T24:00:00Z"), Some(at("2026-01-02 00:00:00")));
        assert_eq!(ms("2026-01-01T10:00:60Z"), Some(at("2026-01-01 10:01:00")));
        assert_eq!(
            ms("2026-01-01T10:00:00+0530"),
            Some(at("2026-01-01 04:30:00"))
        );
        assert_eq!(
            ms("2026-01-01T10:00:00+05"),
            Some(at("2026-01-01 05:00:00"))
        );
        assert_eq!(ms("2026-02-30T10:00:00Z"), Some(at("2026-03-02 10:00:00")));
        assert_eq!(ms("2026-01-01t10:00:00z"), Some(at("2026-01-01 10:00:00")));
        assert_eq!(ms("2026-13-01T10:00:00Z"), None);
        assert_eq!(ms("2026-01-01T24:01:00Z"), None);
    }

    #[test]
    fn retryable_conflicts_normalize_checkbox_states() {
        assert_eq!(
            normalize_checkbox_states("[x] a [] b [ ] c [X] d"),
            "[ ] a [ ] b [ ] c [X] d"
        );
    }
}
