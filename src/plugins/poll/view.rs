//! The poll widget as Ember draws it (assets/javascripts/discourse:
//! initializers/extend-for-poll.gjs attachPolls, components/poll.gjs and
//! the components it renders), from the post serializer's `polls` and
//! `polls_votes`: each cooked `div.poll` of a post in the topic's stream
//! becomes its `div.poll-outer` with the options to vote on or the voted
//! choices, poll-info and the buttons.
//!
//! The server draws the widget in the state Ember starts it in, and again
//! for static/js/poll.js (GET /live/poll/:post_id/:poll_name) in the state
//! the viewer brought it to: what the component keeps (the results shown
//! or not) and what Ember keeps on the poll object across redraws (the
//! vote in progress, the results toggle, amending). `UiState` carries
//! them; the widget's `data-poll-state` hands the outcome back to the
//! script.
//!
//! The results are drawn too: PollResultsTabs with PollResultsStandard
//! (even-round percentages or the tally, the public voters lists) and
//! PollResultsRankedChoice (the rounds and the outcome), the number poll's
//! average rating, and PollResultsPie's canvas and legend, which poll.js
//! fills with Chart.js from the options the state carries.
//!
//! Not ported yet: the actions behind PollButtonsDropdown other than the
//! tally toggle (the breakdown, export, close and open), whose buttons
//! are drawn but refuse when clicked.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::pretty_text::cleanup::{
    attr, element_name, fragment_root, has_class, new_element, node_html, parse, replace, to_html,
};
use crate::site_settings::{SettingError, SiteSettings};
use crate::topic_list_view::{ListContext, escape, icon, t, t_count, t_with};

const ON_VOTE: &str = "on_vote";
const ON_CLOSE: &str = "on_close";
const STAFF_ONLY: &str = "staff_only";
const MULTIPLE: &str = "multiple";
const NUMBER: &str = "number";
const REGULAR: &str = "regular";
const RANKED_CHOICE: &str = "ranked_choice";
const PIE: &str = "pie";

/// The site settings the widget reads.
#[derive(Clone)]
pub struct PollUi {
    /// `poll_groupable_user_fields`: the breakdown's fields.
    pub groupable_user_fields: Vec<String>,
    /// discourse-data-explorer's `data_explorer_enabled`, for Export.
    pub data_explorer_enabled: bool,
    pub export_query_id: i64,
    pub export_ranked_choice_query_id: i64,
}

impl PollUi {
    /// The widget's settings; none while the plugin is off, when Ember
    /// loads none of its client.
    pub fn load(settings: &SiteSettings) -> Result<Option<PollUi>, SettingError> {
        if !super::enabled(settings)? {
            return Ok(None);
        }
        Ok(Some(PollUi {
            groupable_user_fields: settings
                .get("poll_groupable_user_fields")?
                .to_s()
                .split('|')
                .filter(|f| !f.is_empty())
                .map(str::to_string)
                .collect(),
            data_explorer_enabled: settings
                .get("data_explorer_enabled")
                .map(|v| v.truthy())
                .unwrap_or(false),
            export_query_id: settings.get("poll_export_data_explorer_query_id")?.to_i(),
            export_ranked_choice_query_id: settings
                .get("poll_export_ranked_choice_data_explorer_query_id")?
                .to_i(),
        }))
    }
}

/// What the widget reads of the current user: `id`, `staff`, `admin` and
/// the names of `visibleGroups`, lowercased (checkUserGroups).
#[derive(Clone, Debug, Default)]
pub struct PollViewer {
    pub id: i64,
    pub staff: bool,
    pub admin: bool,
    pub groups: Vec<String>,
}

impl PollViewer {
    pub async fn load(
        conn: &mut sqlx::PgConnection,
        user_id: i32,
    ) -> Result<Option<PollViewer>, sqlx::Error> {
        let row: Option<(bool, bool)> =
            sqlx::query_as("SELECT admin, moderator FROM users WHERE id = $1")
                .bind(user_id)
                .fetch_optional(&mut *conn)
                .await?;
        let Some((admin, moderator)) = row else {
            return Ok(None);
        };
        let groups = crate::current_user::visible_groups(&mut *conn, user_id, admin, moderator)
            .await?
            .into_iter()
            .map(|(_, name, _)| name.to_lowercase())
            .collect();
        Ok(Some(PollViewer {
            id: i64::from(user_id),
            staff: admin || moderator,
            admin,
            groups,
        }))
    }

    /// The member's, when one of the posts has polls.
    pub async fn for_posts(
        conn: &mut sqlx::PgConnection,
        user_id: Option<i32>,
        posts: &[Value],
    ) -> Result<Option<PollViewer>, sqlx::Error> {
        match user_id {
            Some(id) if posts.iter().any(|p| p["polls"].is_array()) => {
                PollViewer::load(conn, id).await
            }
            _ => Ok(None),
        }
    }
}

/// What poll.js keeps of a widget between draws.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct UiState {
    /// The component's `_showResults` once it was drawn; none for a new
    /// component, which starts as `initialShowResults` says.
    #[serde(default)]
    pub results: Option<bool>,
    /// `poll.showResultsToggle`, which a new component starts from.
    #[serde(default)]
    pub toggle: Option<bool>,
    /// `isAmendingVote` (`poll.amendingVoteToggle`).
    #[serde(default)]
    pub amending: bool,
    /// `poll.inProgressVote`: digests, or `{digest, rank}` for ranked
    /// choice; none for the saved vote.
    #[serde(default)]
    pub vote: Option<Value>,
    /// The widget is a poll of a post quoted in an expanded quote,
    /// drawn from `post.quoted[id]`, which has no topic: its
    /// `topic.archived` is undefined.
    #[serde(default)]
    pub quoted: bool,
    /// The component's `showTally`: the results as vote counts.
    #[serde(default)]
    pub tally: bool,
    /// PollResultsTabs' `activeTab` ("votes" or "outcome") while the
    /// results show; none for a new tabs component.
    #[serde(default)]
    pub tab: Option<String>,
    /// The component's `preloadedVoters` once a load more expanded them
    /// (`voterListExpanded`): each option's voters as `[username,
    /// avatar_template]`, for ranked choice `[username, avatar_template,
    /// rank]`. None while the poll's `preloaded_voters` are drawn.
    #[serde(default)]
    pub voters: Option<Map<String, Value>>,
}

/// A voter in a PollVoters list.
#[derive(Clone, Debug, PartialEq)]
struct Voter {
    username: String,
    avatar_template: String,
    /// Ranked choice's rank, as `/polls/voters.json` sends it: the number
    /// as a string, or "Abstain".
    rank: Option<String>,
}

impl Voter {
    /// A voter of the serializer's `preloaded_voters` or voters.json:
    /// a user, or ranked choice's `{rank, user}`.
    fn from_serialized(v: &Value, ranked: bool) -> Option<Voter> {
        let user = if ranked { &v["user"] } else { v };
        Some(Voter {
            username: user["username"].as_str()?.to_string(),
            avatar_template: user["avatar_template"].as_str().unwrap_or("").to_string(),
            rank: ranked.then(|| rank_key(&v["rank"])),
        })
    }

    /// A voter as UiState keeps it.
    fn from_compact(v: &Value, ranked: bool) -> Option<Voter> {
        let items = v.as_array()?;
        Some(Voter {
            username: items.first()?.as_str()?.to_string(),
            avatar_template: items
                .get(1)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            rank: ranked.then(|| rank_key(items.get(2).unwrap_or(&Value::Null))),
        })
    }

    fn to_compact(&self) -> Value {
        match &self.rank {
            Some(rank) => json!([self.username, self.avatar_template, rank]),
            None => json!([self.username, self.avatar_template]),
        }
    }
}

/// A rank as a JavaScript object key: `String(rank)`.
fn rank_key(rank: &Value) -> String {
    match rank {
        Value::String(s) => s.clone(),
        Value::Null => "undefined".to_string(),
        other => other.to_string(),
    }
}

/// Whether a key is one `Object.keys` lists first, in numeric order: an
/// array index (a canonical number below 2^32 - 1).
fn array_index(key: &str) -> Option<u32> {
    let n: u32 = key.parse().ok()?;
    (n != u32::MAX && n.to_string() == key).then_some(n)
}

/// lib/even-round.js: percentages rounded down, the error made up by
/// adding 1 to those with the greatest decimal parts until they sum to
/// 100.
fn even_round(mut percentages: Vec<f64>) -> Vec<i64> {
    fn sums_up_to_100(percentages: &[f64]) -> bool {
        percentages.iter().map(|p| p.floor()).sum::<f64>() == 100.0
    }
    if percentages.is_empty() {
        return Vec::new();
    }
    let mut decimals: Vec<f64> = percentages.iter().map(|a| a % 1.0).collect();
    let sum_of_decimals = decimals.iter().sum::<f64>().ceil();
    let mut i = 0usize;
    while (i as f64) < sum_of_decimals && i < decimals.len() {
        let mut greatest = 0.0;
        let mut index = 0;
        for (j, d) in decimals.iter().enumerate() {
            if *d > greatest {
                index = j;
                greatest = *d;
            }
        }
        percentages[index] += 1.0;
        decimals[index] = 0.0;
        if sums_up_to_100(&percentages) {
            break;
        }
        i += 1;
    }
    percentages.iter().map(|p| p.floor() as i64).collect()
}

/// JavaScript's `a < b` on strings: by UTF-16 code units.
fn js_less(a: &str, b: &str) -> bool {
    a.encode_utf16().lt(b.encode_utf16())
}

/// `Math.round`: halves toward positive infinity.
fn js_round(x: f64) -> f64 {
    let floor = x.floor();
    if x - floor >= 0.5 { floor + 1.0 } else { floor }
}

/// lib/round.js's `round(value, -2)` (decimal-adjust): shifted by the
/// exponent as strings, rounded, shifted back.
fn round_2(value: f64) -> f64 {
    if value.is_nan() {
        return f64::NAN;
    }
    let shift = |v: f64, by: i32| -> f64 {
        let s = js_number(v);
        let (mantissa, exp) = match s.split_once('e') {
            Some((m, e)) => (m.to_string(), e.parse::<i32>().unwrap_or(0)),
            None => (s, 0),
        };
        format!("{mantissa}e{}", exp + by)
            .parse()
            .unwrap_or(f64::NAN)
    };
    shift(js_round(shift(value, 2)), -2)
}

/// A number as JavaScript's `String(n)` writes it, for the values a poll
/// shows (no exponent below 1e21).
fn js_number(n: f64) -> String {
    if n.is_nan() {
        "NaN".to_string()
    } else if n.is_infinite() {
        if n > 0.0 { "Infinity" } else { "-Infinity" }.to_string()
    } else if n == 0.0 {
        "0".to_string()
    } else if n.abs() >= 1e21 {
        let s = format!("{n:e}");
        match s.split_once('e') {
            Some((m, e)) if !e.starts_with('-') => format!("{m}e+{e}"),
            _ => s,
        }
    } else {
        n.to_string()
    }
}

/// One poll to draw.
pub struct Widget<'a> {
    pub cx: &'a ListContext<'a>,
    pub ui: &'a PollUi,
    pub viewer: Option<&'a PollViewer>,
    pub post_id: i64,
    pub post_user_id: Option<i64>,
    /// `post.topic.archived`
    pub topic_archived: bool,
    /// The serialized poll.
    pub poll: &'a Value,
    /// `post.polls_votes[poll.name]`
    pub saved: Vec<Value>,
    /// The cooked `.poll-title`'s outer HTML.
    pub title_html: Option<String>,
    /// The cooked poll's `data-poll-dynamic`.
    pub dynamic_attr: bool,
    pub state: &'a UiState,
}

/// A vote, as the component holds it.
#[derive(Clone, Debug, PartialEq)]
enum Vote {
    Digests(Vec<String>),
    Ranks(Vec<(String, i64)>),
}

impl Vote {
    fn len(&self) -> usize {
        match self {
            Vote::Digests(d) => d.len(),
            Vote::Ranks(r) => r.len(),
        }
    }

    fn includes(&self, digest: &str) -> bool {
        match self {
            Vote::Digests(d) => d.iter().any(|x| x == digest),
            Vote::Ranks(_) => false,
        }
    }

    /// The vote as poll.js sends and keeps it.
    fn to_json(&self) -> Value {
        match self {
            Vote::Digests(d) => json!(d),
            Vote::Ranks(r) => Value::Array(
                r.iter()
                    .map(|(d, rank)| json!({"digest": d, "rank": rank}))
                    .collect(),
            ),
        }
    }
}

/// `parseInt(x, 10)` of a JSON value, none for NaN.
fn parse_int(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f.trunc() as i64)),
        Value::String(s) => {
            let s = s.trim_start();
            let (neg, digits) = match s.strip_prefix('-') {
                Some(rest) => (true, rest),
                None => (false, s.strip_prefix('+').unwrap_or(s)),
            };
            let end = digits
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(digits.len());
            let n: i64 = digits[..end].parse().ok()?;
            Some(if neg { -n } else { n })
        }
        _ => None,
    }
}

fn parse_vote(v: &Value, ranked: bool) -> Vote {
    let items = v.as_array().cloned().unwrap_or_default();
    if ranked {
        Vote::Ranks(
            items
                .iter()
                .filter_map(|o| {
                    Some((
                        o["digest"].as_str()?.to_string(),
                        parse_int(&o["rank"]).unwrap_or(0),
                    ))
                })
                .collect(),
        )
    } else {
        Vote::Digests(
            items
                .iter()
                .filter_map(|d| d.as_str().map(str::to_string))
                .collect(),
        )
    }
}

/// `dIcon` as a string.
fn d_icon(name: &str) -> String {
    icon(name, None)
}

/// A JSON value as a template condition takes it.
fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

/// `dBoundAvatarTemplate(template, "tiny")`: 24 pixels from the image of
/// getRawAvatarSize(24), a placeholder without a template.
fn avatar_html(cx: &ListContext, template: &str) -> String {
    if template.is_empty() {
        return "<div class=\"avatar-placeholder\"></div>".to_string();
    }
    format!(
        "<img alt=\"\" width=\"24\" height=\"24\" src=\"{}{}\" class=\"avatar\">",
        if template.starts_with('/') && !template.starts_with("//") {
            cx.base_path
        } else {
            ""
        },
        escape(&template.replace("{size}", &cx.settings.avatar_size_24.to_string()))
    )
}

impl Widget<'_> {
    fn s(&self, key: &str) -> &str {
        self.poll[key].as_str().unwrap_or("")
    }

    fn kind(&self) -> &str {
        self.s("type")
    }

    fn name(&self) -> &str {
        self.s("name")
    }

    fn is_ranked_choice(&self) -> bool {
        self.kind() == RANKED_CHOICE
    }

    fn is_multiple(&self) -> bool {
        self.kind() == MULTIPLE
    }

    fn is_number(&self) -> bool {
        self.kind() == NUMBER
    }

    fn results(&self) -> &str {
        self.s("results")
    }

    fn staff_only(&self) -> bool {
        self.results() == STAFF_ONLY
    }

    fn is_staff(&self) -> bool {
        self.viewer.is_some_and(|v| v.staff)
    }

    fn is_public(&self) -> bool {
        self.poll["public"] == true
    }

    fn is_me(&self) -> bool {
        self.viewer.is_some_and(|v| Some(v.id) == self.post_user_id)
    }

    fn options(&self) -> Vec<&Value> {
        self.poll["options"]
            .as_array()
            .map(|o| o.iter().collect())
            .unwrap_or_default()
    }

    fn voters(&self) -> i64 {
        self.poll["voters"].as_i64().unwrap_or(0)
    }

    /// lib/poll-bounds.js
    fn bounds(&self) -> (i64, i64) {
        let count = self.options().len() as i64;
        let min = match parse_int(&self.poll["min"]) {
            Some(m) if m >= 0 => m,
            _ => 1,
        };
        let max = match parse_int(&self.poll["max"]) {
            Some(m) if m <= count => m,
            _ => count,
        };
        (min, max)
    }

    fn closes_at(&self) -> Option<DateTime<Utc>> {
        self.poll["close"]
            .as_str()
            .and_then(|c| DateTime::parse_from_rfc3339(c).ok())
            .map(|d| d.with_timezone(&Utc))
    }

    fn is_automatically_closed(&self) -> bool {
        self.closes_at().is_some_and(|at| at <= self.cx.now)
    }

    fn closed(&self) -> bool {
        self.s("status") == "closed" || self.is_automatically_closed()
    }

    fn raw_saved_vote(&self) -> Vote {
        parse_vote(&Value::Array(self.saved.clone()), self.is_ranked_choice())
    }

    fn has_saved_vote(&self) -> bool {
        !self.saved.is_empty()
    }

    /// `this.vote`: the vote in progress, else the saved one.
    fn vote(&self) -> Vote {
        match &self.state.vote {
            Some(v) => parse_vote(v, self.is_ranked_choice()),
            None => self.raw_saved_vote(),
        }
    }

    fn results_visibility_allowed(&self) -> bool {
        !(self.results() == ON_CLOSE && !self.closed()) && !(self.staff_only() && !self.is_staff())
    }

    fn hide_results_disabled(&self) -> bool {
        !self.staff_only() && (self.closed() || self.topic_archived)
    }

    fn results_toggle_allowed(&self) -> bool {
        !self.hide_results_disabled() && self.results_visibility_allowed()
    }

    /// `_showResults`: the component's, else initialShowResults.
    fn show_results_state(&self) -> bool {
        if let Some(results) = self.state.results {
            return results;
        }
        if let Some(toggle) = self.state.toggle
            && self.results_toggle_allowed()
        {
            return toggle;
        }
        self.results_visibility_allowed() && (self.has_saved_vote() || self.hide_results_disabled())
    }

    fn results_revealed_by_close(&self) -> bool {
        self.results() == ON_CLOSE
            && self.closed()
            && self.options().iter().all(|o| !o["votes"].is_null())
    }

    fn show_results(&self) -> bool {
        self.show_results_state() || self.results_revealed_by_close()
    }

    fn has_hidden_saved_vote(&self) -> bool {
        self.has_saved_vote() && !self.show_results() && !self.results_visibility_allowed()
    }

    fn show_voted_choices(&self) -> bool {
        self.has_hidden_saved_vote() && !self.state.amending
    }

    /// `checkUserGroups`
    fn in_voting_group(&self) -> bool {
        let Some(groups) = self.poll["groups"].as_str() else {
            return true;
        };
        let poll_groups: Vec<String> = groups.split(',').map(|g| g.to_lowercase()).collect();
        self.viewer
            .is_some_and(|v| poll_groups.iter().any(|g| v.groups.contains(g)))
    }

    fn is_vote_dirty(&self) -> bool {
        fn key(v: &Vote) -> String {
            let mut parts: Vec<String> = match v {
                Vote::Ranks(r) => r
                    .iter()
                    .filter(|(_, rank)| *rank != 0)
                    .map(|(d, rank)| format!("{d}:{rank}"))
                    .collect(),
                Vote::Digests(d) => d.clone(),
            };
            parts.sort();
            parts.join(",")
        }
        key(&self.raw_saved_vote()) != key(&self.vote())
    }

    /// `areRanksValid`
    fn ranks_valid(vote: &Vote) -> bool {
        let Vote::Ranks(ranks) = vote else {
            return false;
        };
        let mut seen = Vec::new();
        let mut all_zeros = true;
        for (_, rank) in ranks {
            if *rank != 0 {
                all_zeros = false;
                if seen.contains(rank) {
                    return false;
                }
                seen.push(*rank);
            }
        }
        !all_zeros
    }

    fn can_cast_votes(&self) -> bool {
        if self.closed() || self.viewer.is_none() {
            return false;
        }
        if self.has_saved_vote() && !self.is_vote_dirty() {
            return false;
        }
        let vote = self.vote();
        let count = vote.len() as i64;
        if self.is_multiple() {
            let (min, max) = self.bounds();
            return count >= min && count <= max;
        }
        if self.is_ranked_choice() {
            return self.options().len() == vote.len() && Self::ranks_valid(&vote);
        }
        count > 0
    }

    fn can_change_vote(&self) -> bool {
        !self.closed() && !self.hide_results_disabled()
    }

    /// The widget, `div.poll-outer` and all.
    pub fn render(&self) -> String {
        let mut classes = String::from("poll-outer");
        if self.s("chart_type") == PIE {
            classes.push_str(" pie");
        }
        format!(
            "<div data-poll-name=\"{}\" data-poll-type=\"{}\" class=\"{classes}\" data-poll-state=\"{}\"><div class=\"poll\"> <div class=\"poll-container\"> {}{} {} </div> {} {} </div></div>",
            escape(self.name()),
            escape(self.kind()),
            escape(&self.state_json().to_string()),
            self.title_html.clone().unwrap_or_default(),
            self.groups_alert(),
            self.body(),
            self.info(),
            self.buttons(),
        )
    }

    /// What poll.js reads back: the state it drew and what its clicks
    /// decide on.
    fn state_json(&self) -> Value {
        let mut state = Map::new();
        state.insert("post_id".into(), json!(self.post_id));
        state.insert("name".into(), json!(self.name()));
        state.insert("type".into(), json!(self.kind()));
        state.insert("results_type".into(), json!(self.results()));
        state.insert("closed".into(), json!(self.closed()));
        state.insert(
            "automatically_closed".into(),
            json!(self.is_automatically_closed()),
        );
        state.insert("archived".into(), json!(self.topic_archived));
        state.insert("member".into(), json!(self.viewer.is_some()));
        state.insert("staff".into(), json!(self.is_staff()));
        state.insert("in_group".into(), json!(self.in_voting_group()));
        state.insert("saved".into(), json!(self.has_saved_vote()));
        state.insert("results".into(), json!(self.show_results_state()));
        if let Some(toggle) = self.state.toggle {
            state.insert("toggle".into(), json!(toggle));
        }
        state.insert("amending".into(), json!(self.state.amending));
        if self.state.vote.is_some() {
            state.insert("vote".into(), self.vote().to_json());
        }
        state.insert("current_vote".into(), self.vote().to_json());
        if self.state.quoted {
            state.insert("quoted".into(), json!(true));
        }
        state.insert(
            "show_voted_choices".into(),
            json!(self.show_voted_choices()),
        );
        if self.state.tally {
            state.insert("tally".into(), json!(true));
        }
        if let Some(tab) = self.active_tab() {
            state.insert("tab".into(), json!(tab));
        }
        if self.is_public() {
            // What a load more merges into, and whether one did.
            let voters: Map<String, Value> = self
                .preloaded_voters()
                .into_iter()
                .map(|(digest, list)| {
                    (
                        digest,
                        Value::Array(list.iter().map(Voter::to_compact).collect()),
                    )
                })
                .collect();
            state.insert("voters".into(), Value::Object(voters));
            state.insert("expanded".into(), json!(self.state.voters.is_some()));
        }
        if self.show_results() && self.is_pie() {
            state.insert(
                "pie".into(),
                json!({
                    "id": self.component_id(),
                    "data": self.options().iter().map(|o| o["votes"].clone()).collect::<Vec<_>>(),
                    "labels": self.options().iter().map(|o| o["html"].clone()).collect::<Vec<_>>(),
                }),
            );
        }
        if self.is_ranked_choice() {
            state.insert(
                "options".into(),
                json!(
                    self.options()
                        .iter()
                        .map(|o| o["id"].clone())
                        .collect::<Vec<_>>()
                ),
            );
        }
        Value::Object(state)
    }

    /// The component's `id`.
    fn component_id(&self) -> String {
        format!("{}-{}", self.name(), self.post_id)
    }

    fn is_pie(&self) -> bool {
        !self.is_number() && self.s("chart_type") == PIE
    }

    /// `preloadedVoters`: each option's voters as the load mores left
    /// them, else the poll's (defaultPreloadedVoters).
    fn preloaded_voters(&self) -> Vec<(String, Vec<Voter>)> {
        let ranked = self.is_ranked_choice();
        let source: Option<(&Map<String, Value>, bool)> = match &self.state.voters {
            Some(kept) => Some((kept, true)),
            None if self.is_public() => self.poll["preloaded_voters"]
                .as_object()
                .map(|p| (p, false)),
            None => None,
        };
        let mut out: Vec<(String, Vec<Voter>)> = Vec::new();
        if let Some((map, compact)) = source {
            for (digest, list) in map {
                let voters = list
                    .as_array()
                    .map(|l| {
                        l.iter()
                            .filter_map(|v| {
                                if compact {
                                    Voter::from_compact(v, ranked)
                                } else {
                                    Voter::from_serialized(v, ranked)
                                }
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                out.push((digest.clone(), voters));
            }
        }
        for o in self.options() {
            if let Some(digest) = o["id"].as_str()
                && !out.iter().any(|(d, _)| d == digest)
            {
                out.push((digest.to_string(), Vec::new()));
            }
        }
        out
    }

    fn voters_of(&self, digest: &str) -> Vec<Voter> {
        self.preloaded_voters()
            .into_iter()
            .find(|(d, _)| d == digest)
            .map(|(_, v)| v)
            .unwrap_or_default()
    }

    /// PollResultsTabs' tabs: Votes unless ranked choice is private,
    /// Outcome for ranked choice.
    fn tabs(&self) -> Vec<&'static str> {
        let mut tabs = Vec::new();
        if !self.is_ranked_choice() || self.is_public() {
            tabs.push("votes");
        }
        if self.is_ranked_choice() {
            tabs.push("outcome");
        }
        tabs
    }

    /// The tab shown, while PollResultsTabs is: the one chosen, else
    /// Outcome for a public ranked choice and the first for the rest.
    fn active_tab(&self) -> Option<&'static str> {
        if !self.show_results() || self.is_number() || self.is_pie() {
            return None;
        }
        let tabs = self.tabs();
        if let Some(chosen) = self.state.tab.as_deref()
            && let Some(tab) = tabs.iter().find(|t| **t == chosen)
        {
            return Some(tab);
        }
        if self.is_ranked_choice() && self.is_public() {
            tabs.get(1).copied()
        } else {
            tabs.first().copied()
        }
    }

    fn groups_alert(&self) -> String {
        if self.in_voting_group() {
            return String::new();
        }
        format!(
            " <div class=\"alert alert-danger\">{}</div>",
            escape(&t_with(
                self.cx,
                "poll.results.groups.title",
                &[("groups", self.s("groups"))]
            ))
        )
    }

    fn body(&self) -> String {
        if self.show_results() {
            let class = if self.is_number() || self.s("chart_type") != PIE {
                format!("discourse-poll-{}-results", self.kind())
            } else {
                "discourse-poll-pie-chart".to_string()
            };
            let inner = if self.is_number() {
                format!("<span>{}</span>", self.average_rating())
            } else if self.is_pie() {
                self.results_pie()
            } else {
                self.results_tabs()
            };
            format!("<div class=\"{class}\"> {inner} </div>")
        } else if self.show_voted_choices() {
            self.voted_choices()
        } else {
            self.poll_options()
        }
    }

    /// `averageRating`: each option's number times its votes, over the
    /// voters, to two decimals.
    fn average_rating(&self) -> String {
        let total = self.options().iter().fold(0.0, |total, o| {
            let html = parse_int(&o["html"]).map_or(f64::NAN, |n| n as f64);
            let votes = parse_int(&o["votes"]).map_or(f64::NAN, |n| n as f64);
            total + html * votes
        });
        let voters = self.voters();
        let average = if voters == 0 {
            0.0
        } else {
            round_2(total / voters as f64)
        };
        t_with(
            self.cx,
            "poll.average_rating",
            &[("average", &js_number(average))],
        )
    }

    /// PollResultsPie: the canvas and legend poll.js draws the chart in.
    fn results_pie(&self) -> String {
        let id = escape(&self.component_id());
        format!(
            "<div class=\"poll-results-chart\"> <canvas class=\"poll-results-canvas\" id=\"poll-results-chart-{id}\"></canvas> <ul class=\"pie-chart-legends\" id=\"poll-results-legend-{id}\"> </ul> </div>"
        )
    }

    /// PollResultsTabs
    fn results_tabs(&self) -> String {
        let active = self.active_tab();
        let tabs: Vec<String> = self
            .tabs()
            .iter()
            .map(|tab| {
                let label = t(self.cx, &format!("poll.results.tabs.{tab}"));
                format!(
                    "<li class=\"tab nav-item{}\"> <button class=\"nav-btn btn no-text\" type=\"button\" data-poll-tab=\"{tab}\"> {} </button> </li>",
                    if Some(*tab) == active { " active" } else { "" },
                    escape(&label)
                )
            })
            .collect();
        let content = match active {
            Some("votes") => self.results_standard(),
            Some("outcome") => self.results_ranked_choice(),
            _ => String::new(),
        };
        format!(
            "<div class=\"tab-container\"> <ul class=\"tabs nav nav-items\"> {} </ul> <div class=\"tab-content\"> {content} </div> </div>",
            tabs.join(" ")
        )
    }

    /// PollResultsStandard: the options by votes, with their percentages
    /// (or tally) and bars, and the public voters.
    fn results_standard(&self) -> String {
        let ranked = self.is_ranked_choice();
        let vote = self.vote();
        let mut ordered: Vec<(&Value, i64)> = self
            .options()
            .into_iter()
            .map(|o| (o, o["votes"].as_i64().unwrap_or(0)))
            .collect();
        // orderOptions: most votes first, ties by their HTML.
        ordered.sort_by(|(a, av), (b, bv)| {
            use std::cmp::Ordering;
            if av < bv {
                Ordering::Greater
            } else if av == bv {
                if js_less(
                    a["html"].as_str().unwrap_or(""),
                    b["html"].as_str().unwrap_or(""),
                ) {
                    Ordering::Less
                } else {
                    Ordering::Greater
                }
            } else {
                Ordering::Less
            }
        });
        let voters_count = self.voters();
        let percentages: Vec<f64> = ordered
            .iter()
            .map(|(_, votes)| {
                if voters_count == 0 {
                    0.0
                } else {
                    (100.0 * *votes as f64) / voters_count as f64
                }
            })
            .collect();
        let rounded: Vec<i64> = if self.is_multiple() {
            percentages.iter().map(|p| p.floor() as i64).collect()
        } else {
            even_round(percentages)
        };
        let items: Vec<String> = ordered
            .iter()
            .zip(rounded)
            .map(|((o, votes), percentage)| {
                let digest = o["id"].as_str().unwrap_or("");
                let per = percentage.to_string();
                let figure = if ranked {
                    String::new()
                } else if self.state.tally {
                    format!(
                        "<span class=\"absolute\">{}</span> ",
                        escape(&t_count(self.cx, "poll.votes", *votes, &[]))
                    )
                } else {
                    format!(
                        "<span class=\"percentage\">{}</span> ",
                        escape(&t_with(self.cx, "number.percent", &[("count", &per)]))
                    )
                };
                let bar = if ranked {
                    String::new()
                } else {
                    format!(
                        " <div class=\"bar-back\"> <div class=\"bar\" style=\"width:{per}%\"></div> </div>"
                    )
                };
                let voters = if self.is_public() {
                    format!(" {}", self.poll_voters(digest, *votes))
                } else {
                    String::new()
                };
                format!(
                    "<li class=\"{}\"> <div class=\"option\"> <p> {figure}<span class=\"option-text\">{}</span> </p>{bar}{voters} </div> </li>",
                    if vote.includes(digest) { "chosen" } else { "" },
                    o["html"].as_str().unwrap_or("")
                )
            })
            .collect();
        format!("<ul class=\"results\"> {} </ul>", items.join(" "))
    }

    /// PollVoters: an option's voters, and a load more while there are
    /// more votes than voters drawn.
    fn poll_voters(&self, digest: &str, total_votes: i64) -> String {
        let voters = self.voters_of(digest);
        let avatar = |v: &Voter| {
            format!(
                "<li> <a data-user-card=\"{u}\" title=\"{u}\"> {} </a> </li>",
                avatar_html(self.cx, &v.avatar_template),
                u = escape(&v.username)
            )
        };
        let list = if self.is_ranked_choice() {
            // PollVotersRankedChoice: grouped by rank, as Object.keys
            // lists the groups (numeric ranks in order, then Abstain).
            let mut groups: Vec<(String, Vec<&Voter>)> = Vec::new();
            for v in &voters {
                let rank = v.rank.clone().unwrap_or_default();
                match groups.iter_mut().find(|(r, _)| *r == rank) {
                    Some((_, list)) => list.push(v),
                    None => groups.push((rank, vec![v])),
                }
            }
            let (mut indexed, named): (Vec<_>, Vec<_>) = groups
                .into_iter()
                .partition(|(rank, _)| array_index(rank).is_some());
            indexed.sort_by_key(|(rank, _)| array_index(rank));
            indexed
                .into_iter()
                .chain(named)
                .map(|(rank, list)| {
                    let label = if rank == "Abstain" {
                        d_icon("ban")
                    } else {
                        escape(&rank)
                    };
                    format!(
                        "<ul> <span class=\"rank\">{label}</span> {} </ul>",
                        list.iter().map(|v| avatar(v)).collect::<Vec<_>>().join(" ")
                    )
                })
                .collect::<Vec<_>>()
                .join(" ")
        } else {
            voters.iter().map(avatar).collect::<Vec<_>>().join(" ")
        };
        let more = if (voters.len() as i64) < total_votes {
            format!(
                " <div class=\"loading-container\"> <button class=\"poll-voters-toggle-expand btn no-text btn-icon\" type=\"button\" data-poll-voters=\"{}\">{}<span aria-hidden=\"true\">&#8203;</span></button> </div>",
                escape(digest),
                d_icon("chevron-down")
            )
        } else {
            String::new()
        };
        format!(
            "<div class=\"poll-voters\"> <ul class=\"poll-voters-list\"> {list} </ul>{more} </div>"
        )
    }

    /// PollResultsRankedChoice: the rounds, then the winner or the tie.
    fn results_ranked_choice(&self) -> String {
        let outcome = &self.poll["ranked_choice_outcome"];
        let none = escape(&t(self.cx, "poll.ranked_choice.none"));
        let rounds: Vec<String> = outcome["round_activity"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[])
            .iter()
            .map(|round| {
                let number = escape(&rank_key(&round["round"]));
                if truthy(&round["majority"]) {
                    format!(
                        "<tr> <td>{number}</td> <td>{}</td> <td>{none}</td> </tr>",
                        round["majority"]["html"].as_str().unwrap_or("")
                    )
                } else {
                    let eliminated: Vec<String> = round["eliminated"]
                        .as_array()
                        .map(Vec::as_slice)
                        .unwrap_or(&[])
                        .iter()
                        .map(|e: &Value| {
                            format!("<span>{}</span>", e["html"].as_str().unwrap_or(""))
                        })
                        .collect();
                    format!(
                        "<tr> <td>{number}</td> <td>{none}</td> <td> {} </td> </tr>",
                        eliminated.join(" ")
                    )
                }
            })
            .collect();
        let rounds_count = outcome["round_activity"]
            .as_array()
            .map_or(0, |r| r.len() as i64);
        let result = if !truthy(outcome) {
            String::new()
        } else if truthy(&outcome["tied"]) {
            let candidates: Vec<String> = outcome["tied_candidates"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or(&[])
                .iter()
                .map(|c: &Value| {
                    format!(
                        "<li class=\"poll-results-ranked-choice-tied-candidate\">{}</li>",
                        c["html"].as_str().unwrap_or("")
                    )
                })
                .collect();
            format!(
                " <span class=\"poll-results-ranked-choice-info\">{}</span> <ul class=\"poll-results-ranked-choice-tied-candidates\"> {} </ul>",
                escape(&t_count(
                    self.cx,
                    "poll.ranked_choice.tied",
                    rounds_count,
                    &[]
                )),
                candidates.join(" ")
            )
        } else {
            format!(
                " <span class=\"poll-results-ranked-choice-info\">{}</span>",
                t_count(
                    self.cx,
                    "poll.ranked_choice.winner",
                    rounds_count,
                    &[(
                        "winner",
                        outcome["winning_candidate"]["html"].as_str().unwrap_or("")
                    )]
                )
            )
        };
        format!(
            "<h3 class=\"poll-results-ranked-choice-subtitle-rounds\"> {} </h3> <table class=\"poll-results-ranked-choice\"> <thead> <tr> <th>{}</th> <th>{}</th> <th>{}</th> </tr> </thead> <tbody> {} </tbody> </table> <h3 class=\"poll-results-ranked-choice-subtitle-outcome\"> {} </h3>{result}",
            escape(&t(self.cx, "poll.ranked_choice.title.rounds")),
            escape(&t(self.cx, "poll.ranked_choice.round")),
            escape(&t(self.cx, "poll.ranked_choice.majority")),
            escape(&t(self.cx, "poll.ranked_choice.eliminated")),
            rounds.join(" "),
            escape(&t(self.cx, "poll.ranked_choice.title.outcome")),
        )
    }

    /// PollOptions for ranked choice: each option with its rank dropdown
    /// (a login button for anonymous readers).
    fn ranked_choice_options(&self) -> String {
        let vote = self.vote();
        let options = self.options();
        // rankedChoiceDropdownContent: Abstain, then a rank per option.
        let count = options.len();
        let content: Vec<(i64, String)> =
            std::iter::once((0, t(self.cx, "poll.options.ranked_choice.abstain")))
                .chain((0..count).map(|i| {
                    let mut priority = String::new();
                    if i == 0 {
                        priority = format!(
                            " {}",
                            t(self.cx, "poll.options.ranked_choice.highest_priority")
                        );
                    }
                    if i == count - 1 {
                        priority = format!(
                            " {}",
                            t(self.cx, "poll.options.ranked_choice.lowest_priority")
                        );
                    }
                    (i as i64 + 1, format!("{}{priority}", i + 1))
                }))
                .collect();
        let menu = format!(
            "<ul class=\"dropdown-menu\"> {} </ul>",
            content
                .iter()
                .map(|(rank, name)| format!(
                    "<li class=\"dropdown-menu__item\"> <button class=\"btn-transparent poll-option-dropdown btn\" type=\"button\" data-poll-rank=\"{rank}\"><span class=\"d-button-label\">{}</span></button> </li>",
                    escape(name)
                ))
                .collect::<Vec<_>>()
                .join(" ")
        );
        let items: Vec<String> = options
            .iter()
            .map(|o| {
                let digest = o["id"].as_str().unwrap_or("");
                let rank = match &vote {
                    Vote::Ranks(ranks) => ranks
                        .iter()
                        .find(|(d, _)| d == digest)
                        .map_or(0, |(_, r)| *r),
                    Vote::Digests(_) => 0,
                };
                let control = if self.viewer.is_some() {
                    let label = if rank == 0 {
                        t(self.cx, "poll.options.ranked_choice.abstain")
                    } else {
                        rank.to_string()
                    };
                    format!(
                        "<button aria-expanded=\"false\" class=\"btn no-text fk-d-menu__trigger -trigger\" data-trigger=\"\" type=\"button\" data-poll-menu=\"{}\"> <span class=\"d-button-label\"> {} </span> {} </button>",
                        escape(&menu),
                        escape(&label),
                        d_icon("angle-down")
                    )
                } else {
                    format!(
                        "<button class=\"btn-default btn\" type=\"button\" data-poll-login=\"\"><span class=\"d-button-label\">{}</span></button>",
                        escape(&t(self.cx, "poll.options.ranked_choice.login"))
                    )
                };
                format!(
                    "<div class=\"ranked-choice-poll-option\" data-poll-option-id=\"{}\" data-poll-option-rank=\"{rank}\" tabindex=\"0\"> {control} <span class=\"option-text\">{}</span> </div>",
                    escape(digest),
                    o["html"].as_str().unwrap_or("")
                )
            })
            .collect();
        format!(
            "<ul class=\"ranked-choice-poll-options options\"> {} </ul>",
            items.join(" ")
        )
    }

    /// PollVotedChoices
    fn voted_choices(&self) -> String {
        let saved = self.raw_saved_vote();
        let options = self.options();
        let choices: Vec<String> = match &saved {
            Vote::Ranks(ranks) => {
                let mut chosen: Vec<(i64, &Value)> = options
                    .iter()
                    .filter_map(|o| {
                        let digest = o["id"].as_str()?;
                        let rank = ranks
                            .iter()
                            .find(|(d, rank)| d == digest && *rank > 0)
                            .map(|(_, rank)| *rank)?;
                        Some((rank, *o))
                    })
                    .collect();
                chosen.sort_by_key(|(rank, _)| *rank);
                chosen
                    .iter()
                    .map(|(rank, o)| {
                        format!(
                            "<li class=\"poll-voted-choices__choice\"> <span class=\"poll-voted-choices__rank\">{rank}</span> <span class=\"option-text\">{}</span> </li>",
                            o["html"].as_str().unwrap_or("")
                        )
                    })
                    .collect()
            }
            Vote::Digests(digests) => options
                .iter()
                .filter(|o| o["id"].as_str().is_some_and(|d| digests.iter().any(|x| x == d)))
                .map(|o| {
                    format!(
                        "<li class=\"poll-voted-choices__choice\"> {} <span class=\"option-text\">{}</span> </li>",
                        d_icon("check"),
                        o["html"].as_str().unwrap_or("")
                    )
                })
                .collect(),
        };
        format!(
            "<div class=\"poll-voted-choices\"> <span class=\"poll-voted-choices__title\"> {} </span> <ul class=\"poll-voted-choices__list\"> {} </ul> </div>",
            escape(&t(self.cx, "poll.voted_choices.title")),
            choices.join(" ")
        )
    }

    /// PollOptions
    fn poll_options(&self) -> String {
        let vote = self.vote();
        let options = self.options();
        if self.is_ranked_choice() {
            return self.ranked_choice_options();
        }
        let checkbox = self.is_multiple();
        let items: Vec<String> = options
            .iter()
            .map(|o| {
                let digest = o["id"].as_str().unwrap_or("");
                let icon_name = match (vote.includes(digest), checkbox) {
                    (true, true) => "far-square-check",
                    (true, false) => "circle",
                    (false, true) => "far-square",
                    (false, false) => "far-circle",
                };
                format!(
                    "<li data-poll-option-id=\"{}\"> <button> {} <span class=\"option-text\">{}</span> </button> </li>",
                    escape(digest),
                    d_icon(icon_name),
                    o["html"].as_str().unwrap_or("")
                )
            })
            .collect();
        format!("<ul class=\"options\"> {} </ul>", items.join(" "))
    }

    /// PollInfo
    fn info(&self) -> String {
        let voters = self.voters();
        let show_results = self.show_results();
        let closed = self.closed();
        let is_multiple = self.is_multiple();
        let mut counts = format!(
            "<div class=\"poll-info_counts-count\"> <span class=\"info-number\">{voters}</span> <span class=\"info-label\">{}</span> </div>",
            escape(&t_count(self.cx, "poll.voters", voters, &[]))
        );
        if is_multiple && (show_results || closed) {
            let total: Option<i64> = self
                .options()
                .iter()
                .try_fold(0i64, |sum, o| Some(sum + parse_int(&o["votes"])?));
            let (number, label) = match total {
                Some(total) => (
                    total.to_string(),
                    t_count(self.cx, "poll.total_votes", total, &[]),
                ),
                None => (
                    "NaN".to_string(),
                    t_count(self.cx, "poll.total_votes", 2, &[]),
                ),
            };
            counts.push_str(&format!(
                " <div class=\"poll-info_counts-count\"> <span class=\"info-number\">{number}</span> <span class=\"info-label\">{}</span> </div>",
                escape(&label)
            ));
        }
        let showing_voted_choices = self.show_voted_choices();
        let is_dynamic = self.dynamic_attr || self.poll["dynamic"] == true;
        let mut items: Vec<String> = Vec::new();
        let li = |class: &str, icon_name: &str, html: &str| {
            format!(
                "<li class=\"{class}\"> {} <span>{html}</span> </li>",
                d_icon(icon_name)
            )
        };
        if is_dynamic {
            items.push(li(
                "is-dynamic",
                "shuffle",
                &escape(&t(self.cx, "poll.dynamic.enabled_hint")),
            ));
        }
        if showing_voted_choices {
            items.push(li(
                "vote-recorded",
                "check",
                &escape(&t(self.cx, "poll.vote_recorded")),
            ));
        }
        if is_multiple
            && !show_results
            && !showing_voted_choices
            && !closed
            && let Some(help) = self.multiple_help_text()
        {
            items.push(li("multiple-help-text", "list-ul", &help));
        }
        let auto_closed = self.is_automatically_closed();
        if let Some(closes_at) = self.closes_at()
            && (auto_closed || !closed)
        {
            items.push(self.automatic_close(closes_at, auto_closed));
        }
        if let Some(username) = self.poll["closed_by"]["username"].as_str() {
            items.push(li(
                "poll-info_closed-by",
                "lock",
                &escape(&t_with(
                    self.cx,
                    "poll.closed_by",
                    &[("username", username)],
                )),
            ));
        }
        // PollInfo's postUserId is `this.poll.post.user_id`, undefined:
        // the post's author gets the hint too.
        if self.results() == ON_VOTE && self.vote().len() == 0 {
            items.push(li(
                "results-on-vote",
                "check",
                &t(self.cx, "poll.results.vote.title"),
            ));
        }
        if self.results() == ON_CLOSE && !closed {
            items.push(li(
                "results-on-close",
                "lock",
                &t(self.cx, "poll.results.closed.title"),
            ));
        }
        if self.staff_only() && !self.is_staff() {
            items.push(li(
                "results-staff-only",
                "shield-halved",
                &t(self.cx, "poll.results.staff.title"),
            ));
        }
        if !closed && !show_results && self.is_public() && !self.staff_only() {
            items.push(li("is-public", "far-eye", &t(self.cx, "poll.public.title")));
        }
        let instructions = if items.is_empty() {
            String::new()
        } else {
            format!(
                " <ul class=\"poll-info_instructions\"> {} </ul>",
                items.join(" ")
            )
        };
        format!(
            "<div class=\"poll-info\"> <div class=\"poll-info_counts\"> {counts} </div>{instructions} </div>"
        )
    }

    fn multiple_help_text(&self) -> Option<String> {
        let (min, max) = self.bounds();
        let count = self.options().len() as i64;
        if max <= 0 {
            return None;
        }
        if min == max && min > 1 {
            return Some(t_count(self.cx, "poll.multiple.help.x_options", min, &[]));
        }
        if min > 1 {
            if max < count {
                return Some(t_with(
                    self.cx,
                    "poll.multiple.help.between_min_and_max_options",
                    &[("min", &min.to_string()), ("max", &max.to_string())],
                ));
            }
            return Some(t_count(
                self.cx,
                "poll.multiple.help.at_least_min_options",
                min,
                &[],
            ));
        }
        if max <= count {
            return Some(t_count(
                self.cx,
                "poll.multiple.help.up_to_max_options",
                max,
                &[],
            ));
        }
        None
    }

    /// The automatic close line: "Closes in" while open, "Closed" with its
    /// age once past; static/js/dates.js keeps both current. The title is
    /// `closesAt.format("LLL")` of a `moment.utc`: in UTC for everyone.
    fn automatic_close(&self, at: DateTime<Utc>, auto_closed: bool) -> String {
        let title = format!(
            " title=\"{}\"",
            escape(&crate::viewer_dates::fill_utc(
                self.cx,
                &crate::viewer_dates::marker(at, "LLL")
            ))
        );
        let (format, key, arg) = if auto_closed {
            ("@tiny-ago", "poll.automatic_close.age", "age")
        } else {
            ("@humanize", "poll.automatic_close.closes_in", "timeLeft")
        };
        let template = crate::viewer_dates::marker(at, format);
        let (tz, text) = crate::viewer_dates::text(self.cx, &template);
        let placeholder = "\u{0}";
        let html = t_with(self.cx, key, &[(arg, placeholder)]).replace(
            &format!("<strong>{placeholder}</strong>"),
            &format!("<strong{tz}>{text}</strong>"),
        );
        format!(
            "<li{title}> {} <span>{html}</span> </li>",
            d_icon(if auto_closed { "lock" } else { "far-clock" })
        )
    }

    /// The buttons under the poll.
    fn buttons(&self) -> String {
        let show_results = self.show_results();
        let show_voted_choices = self.show_voted_choices();
        let has_saved_vote = self.has_saved_vote();
        let can_change_vote = self.can_change_vote();
        let mut out: Vec<String> = Vec::new();
        let button = |class: &str, title_key: &str, icon_name: &str, label: &str| {
            format!(
                "<button class=\"{class}\" title=\"{}\"> {} <span class=\"d-button-label\">{}</span> </button>",
                escape(&t(self.cx, title_key)),
                d_icon(icon_name),
                escape(label)
            )
        };
        if self.has_hidden_saved_vote() && self.state.amending {
            out.push(button(
                "btn btn-default keep-vote",
                "poll.keep-vote.title",
                "chevron-left",
                &t(self.cx, "poll.keep-vote.label"),
            ));
        }
        if (self.is_multiple() || self.is_ranked_choice()) && !show_results && !show_voted_choices {
            let can_cast = self.can_cast_votes();
            let (title_key, label_key) = if has_saved_vote {
                (
                    "poll.cast-votes.update_title",
                    "poll.cast-votes.update_label",
                )
            } else {
                ("poll.cast-votes.title", "poll.cast-votes.label")
            };
            out.push(format!(
                "<button class=\"btn cast-votes {}\"{} title=\"{}\"> {} <span class=\"d-button-label\">{}</span> </button>",
                if can_cast { "btn-primary" } else { "btn-default" },
                if can_cast { "" } else { " disabled=\"\"" },
                escape(&t(self.cx, title_key)),
                d_icon(if can_cast { "check" } else { "far-square" }),
                escape(&t(self.cx, label_key))
            ));
        }
        if can_change_vote && !show_results && has_saved_vote && !show_voted_choices {
            out.push(button(
                "btn btn-default remove-vote",
                "poll.remove-vote.title",
                "arrow-rotate-left",
                &t(self.cx, "poll.remove-vote.label"),
            ));
        }
        if can_change_vote && show_voted_choices {
            out.push(button(
                "btn btn-default amend-vote",
                "poll.amend-vote.title",
                "pencil",
                &t(self.cx, "poll.amend-vote.label"),
            ));
        }
        if show_results && !self.hide_results_disabled() {
            out.push(button(
                "btn btn-default toggle-results",
                "poll.hide-results.title",
                "chevron-left",
                &t(self.cx, "poll.hide-results.label"),
            ));
        }
        if !show_results
            && self.results_toggle_allowed()
            && !(self.results() == ON_VOTE && !has_saved_vote && !self.is_me())
            && self.voters() > 0
        {
            out.push(button(
                "btn btn-default toggle-results",
                "poll.show-results.title",
                "chart-bar",
                &t(self.cx, "poll.show-results.label"),
            ));
        }
        out.push(self.dropdown());
        format!("<div class=\"poll-buttons\"> {} </div>", out.join(" "))
    }

    /// PollButtonsDropdown: one action as a button, more behind the gear.
    fn dropdown(&self) -> String {
        // (action, class, label, icon)
        let mut contents: Vec<(&str, &str, &str, &str)> = Vec::new();
        let show_results = self.show_results();
        if show_results && self.s("chart_type") != PIE && [REGULAR, MULTIPLE].contains(&self.kind())
        {
            // availableDisplayMode: the other of tally and percentages.
            contents.push(if self.state.tally {
                (
                    "toggleDisplayMode",
                    "btn-default show-percentage",
                    "poll.show-percentage.label",
                    "info",
                )
            } else {
                (
                    "toggleDisplayMode",
                    "btn-default show-tally",
                    "poll.show-tally.label",
                    "info",
                )
            });
        }
        let voters = self.voters();
        if !self.ui.groupable_user_fields.is_empty() && voters > 0 {
            contents.push((
                "showBreakdown",
                "btn-default show-breakdown",
                "poll.breakdown.breakdown",
                "chart-pie",
            ));
        }
        let export_query = if self.is_ranked_choice() {
            self.ui.export_ranked_choice_query_id
        } else {
            self.ui.export_query_id
        };
        if self.viewer.is_some_and(|v| v.admin)
            && self.ui.data_explorer_enabled
            && voters > 0
            && export_query != 0
        {
            contents.push((
                "exportResults",
                "btn-default export-results",
                "poll.export-results.label",
                "download",
            ));
        }
        if self.viewer.is_some() && (self.is_me() || self.is_staff()) && !self.topic_archived {
            if self.closed() {
                if !self.is_automatically_closed() {
                    contents.push((
                        "toggleStatus",
                        "btn-default toggle-status",
                        "poll.open.label",
                        "unlock-keyhole",
                    ));
                }
            } else {
                contents.push((
                    "toggleStatus",
                    "btn-default toggle-status",
                    "poll.close.label",
                    "lock",
                ));
            }
        }
        let widget_button = |(action, class, label, icon_name): &(&str, &str, &str, &str)| {
            format!(
                "<button class=\"btn btn-icon-text widget-button {class}\" type=\"button\" data-poll-action=\"{action}\"> {} <span class=\"d-button-label\">{}</span> </button>",
                d_icon(icon_name),
                escape(&t(self.cx, label))
            )
        };
        let inner = match contents.len() {
            0 => String::new(),
            1 => widget_button(&contents[0]),
            _ => {
                let items: Vec<String> = contents
                    .iter()
                    .enumerate()
                    .map(|(i, c)| {
                        let divider = if i > 0 {
                            "<li><hr class=\"dropdown-menu__divider\"></li> "
                        } else {
                            ""
                        };
                        format!(
                            "{divider}<li class=\"dropdown-menu__item\"> {} </li>",
                            widget_button(c)
                        )
                    })
                    .collect();
                format!(
                    "<button aria-expanded=\"false\" class=\"btn no-text fk-d-menu__trigger -trigger widget-dropdown-header\" data-trigger=\"\" type=\"button\" data-poll-menu=\"{}\"> {} </button>",
                    escape(&format!(
                        "<ul class=\"dropdown-menu\"> {} </ul>",
                        items.join(" ")
                    )),
                    d_icon("gear")
                )
            }
        };
        format!("<div class=\"poll-buttons-dropdown\"> {inner} </div>")
    }
}

/// The js.* strings poll.js shows: its errors.
const SCRIPT_STRINGS: [&str; 3] = [
    "poll.error_while_casting_votes",
    "poll.error_while_fetching_voters",
    "generic_error",
];

/// The topic page's script for its polls, with its strings and where
/// Chart.js is, which it loads for a pie as loadChartJS does. The dialogs
/// it reports errors in come with the page's dialog.js.
pub fn page_scripts(i18n: &crate::i18n::I18n, base_path: &str) -> String {
    let own: Map<String, Value> = SCRIPT_STRINGS
        .iter()
        .map(|k| {
            let full = format!("js.{k}");
            (
                k.to_string(),
                Value::String(i18n.t(&full).unwrap_or(&full).to_string()),
            )
        })
        .collect();
    format!(
        "<script type=\"application/json\" id=\"poll-strings\">{}</script><script src=\"{}\" data-chart-js=\"{}\"></script>",
        Value::Object(own).to_string().replace('<', "\\u003c"),
        crate::assets::url(base_path, "poll.js"),
        crate::assets::url(base_path, "chart.umd.min.js"),
    )
}

/// What the cooked poll carries for the widget: its title and dynamic flag.
pub struct CookedPoll {
    pub name: String,
    pub title_html: Option<String>,
    pub dynamic: bool,
}

/// attachPolls over a post's cooked HTML: each `.poll` whose parent is not
/// a blockquote and whose name the post's polls have, replaced by
/// `draw(cooked poll)`. Unchanged when there is none.
pub fn attach(cooked: &str, polls: &[Value], draw: impl Fn(&CookedPoll) -> String) -> String {
    if polls.is_empty() || !cooked.contains("poll") {
        return cooked.to_string();
    }
    let dom = parse(cooked);
    let mut widgets = Vec::new();
    for (node, cooked_poll) in cooked_polls(&dom) {
        if !polls.iter().any(|p| p["name"] == cooked_poll.name.as_str()) {
            continue;
        }
        let marker = format!("poll-widget-{}", widgets.len());
        widgets.push((marker.clone(), draw(&cooked_poll)));
        replace(&node, new_element(&marker, &[]));
    }
    if widgets.is_empty() {
        return cooked.to_string();
    }
    let mut html = to_html(&dom);
    for (marker, widget) in widgets {
        html = html.replacen(&format!("<{marker}></{marker}>"), &widget, 1);
    }
    html
}

/// The cooked poll of that name attachPolls would draw.
pub fn cooked_poll(cooked: &str, name: &str) -> Option<CookedPoll> {
    cooked_polls(&parse(cooked))
        .into_iter()
        .map(|(_, p)| p)
        .find(|p| p.name == name)
}

/// The `.poll` nodes attachPolls draws, with what they carry.
fn cooked_polls(dom: &markup5ever_rcdom::RcDom) -> Vec<(markup5ever_rcdom::Handle, CookedPoll)> {
    let mut found = Vec::new();
    fn walk(
        node: &markup5ever_rcdom::Handle,
        parent_is_quote: bool,
        out: &mut Vec<markup5ever_rcdom::Handle>,
    ) {
        for child in node.children.borrow().iter() {
            if element_name(child).is_some() && has_class(child, "poll") && !parent_is_quote {
                out.push(child.clone());
                // Ember's static list replaces the outer poll first; one
                // inside it goes with it.
                continue;
            }
            walk(child, element_name(child) == Some("blockquote"), out);
        }
    }
    walk(&fragment_root(dom), false, &mut found);
    found
        .into_iter()
        .filter_map(|node| {
            let name = attr(&node, "data-poll-name")?;
            let title_html = crate::pretty_text::cleanup::all_elements_within(&node)
                .into_iter()
                .find(|e| has_class(e, "poll-title"))
                .map(|e| node_html(&e));
            let cooked_poll = CookedPoll {
                dynamic: attr(&node, "data-poll-dynamic").as_deref() == Some("true"),
                name,
                title_html,
            };
            Some((node, cooked_poll))
        })
        .collect()
}

/// The polls of a post as the topic page draws them, in their first
/// state.
pub fn decorate(
    cx: &ListContext,
    ui: &PollUi,
    viewer: Option<&PollViewer>,
    topic_archived: bool,
    post: &Value,
    cooked: &str,
) -> String {
    let Some(polls) = post["polls"].as_array() else {
        return cooked.to_string();
    };
    let state = UiState::default();
    attach(cooked, polls, |cooked_poll| {
        let poll = polls
            .iter()
            .find(|p| p["name"] == cooked_poll.name.as_str())
            .expect("attach draws only the post's polls");
        Widget {
            cx,
            ui,
            viewer,
            post_id: post["id"].as_i64().unwrap_or(0),
            post_user_id: post["user_id"].as_i64(),
            topic_archived,
            poll,
            saved: post["polls_votes"][cooked_poll.name.as_str()]
                .as_array()
                .cloned()
                .unwrap_or_default(),
            title_html: cooked_poll.title_html.clone(),
            dynamic_attr: cooked_poll.dynamic,
            state: &state,
        }
        .render()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ints_as_parse_int() {
        assert_eq!(parse_int(&json!("12abc")), Some(12));
        assert_eq!(parse_int(&json!(" -3")), Some(-3));
        assert_eq!(parse_int(&json!("x")), None);
        assert_eq!(parse_int(&json!(4)), Some(4));
        assert_eq!(parse_int(&Value::Null), None);
    }

    #[test]
    fn attaches_the_posts_polls_but_not_a_quotes() {
        let cooked = "<p>Hi</p>\n<div class=\"poll\" data-poll-name=\"poll\" data-poll-dynamic=\"true\"><div class=\"poll-title\">Pick</div><ul><li>A</li></ul></div>\n<aside class=\"quote\"><blockquote><div class=\"poll\" data-poll-name=\"poll\"></div></blockquote></aside>\n<div class=\"poll\" data-poll-name=\"gone\"></div>";
        let polls = [json!({"name": "poll"})];
        let html = attach(cooked, &polls, |p| {
            format!(
                "[{} {} {}]",
                p.name,
                p.dynamic,
                p.title_html.as_deref().unwrap_or("-")
            )
        });
        assert_eq!(
            html,
            "<p>Hi</p>\n[poll true <div class=\"poll-title\">Pick</div>]\n<aside class=\"quote\"><blockquote><div class=\"poll\" data-poll-name=\"poll\"></div></blockquote></aside>\n<div class=\"poll\" data-poll-name=\"gone\"></div>"
        );
        // A post without polls keeps its cooked content as it is.
        assert_eq!(attach(cooked, &[], |_| String::new()), cooked);
    }

    #[test]
    fn rounds_percentages_evenly() {
        // 13/30, 10/30, 7/30: the greatest decimal part gets the point.
        assert_eq!(
            even_round(vec![
                100.0 * 13.0 / 30.0,
                100.0 * 10.0 / 30.0,
                100.0 * 7.0 / 30.0
            ]),
            [44, 33, 23]
        );
        assert_eq!(
            even_round(vec![90.0, 100.0 * 2.0 / 30.0, 100.0 / 30.0]),
            [90, 7, 3]
        );
        assert_eq!(even_round(vec![0.0, 0.0]), [0, 0]);
        assert_eq!(even_round(vec![50.0, 50.0]), [50, 50]);
        assert!(even_round(Vec::new()).is_empty());
    }

    #[test]
    fn rounds_to_two_decimals_as_decimal_adjust() {
        assert_eq!(js_number(round_2(72.0 / 23.0)), "3.13");
        // The shift goes through the decimal string: 2.675 is not 2.67.
        assert_eq!(js_number(round_2(2.675)), "2.68");
        assert_eq!(js_number(round_2(1.005)), "1.01");
        assert_eq!(js_number(round_2(3.0)), "3");
        assert_eq!(js_number(round_2(f64::NAN)), "NaN");
    }

    #[test]
    fn orders_rank_groups_as_object_keys() {
        assert_eq!(array_index("2"), Some(2));
        assert_eq!(array_index("02"), None);
        assert_eq!(array_index("Abstain"), None);
        assert!(js_less("Beta", "beta"));
        assert!(!js_less("b", "B"));
    }

    fn list_settings() -> crate::topic_list_view::ListSettings {
        crate::topic_list_view::ListSettings {
            show_pinned_excerpt_desktop: true,
            show_pinned_excerpt_mobile: true,
            suppress_uncategorized_badge: true,
            uncategorized_category_id: 1,
            tag_style: "simple".into(),
            suppress_overlapping_tags_in_list: false,
            topic_views_heat: [1000, 2000, 5000],
            topic_post_like_heat: [0.5, 1.0, 2.0],
            cold_age_days: [14.0, 30.0, 60.0],
            relative_date_duration: 30,
            avatar_size_24: 48,
            avatar_size_48: 96,
            prioritize_name: false,
            emojis: std::sync::Arc::new(crate::emojis::Emojis::default()),
            support_mixed_text_direction: false,
        }
    }

    /// The widget of `poll` for user 7 (post by user 9), in `state`.
    fn draw(poll: &Value, saved: Vec<Value>, state: &UiState) -> String {
        let i18n = crate::i18n::I18n::vendored().unwrap();
        let categories = std::collections::HashMap::new();
        let cx = ListContext {
            i18n: &i18n,
            base_path: "/forum",
            now: Utc::now(),
            categories: &categories,
            expand_all_pinned: false,
            hide_category: false,
            member_trust_level: None,
            settings: list_settings(),
        };
        let ui = PollUi {
            groupable_user_fields: Vec::new(),
            data_explorer_enabled: false,
            export_query_id: 0,
            export_ranked_choice_query_id: 0,
        };
        let viewer = PollViewer {
            id: 7,
            ..Default::default()
        };
        Widget {
            cx: &cx,
            ui: &ui,
            viewer: Some(&viewer),
            post_id: 3,
            post_user_id: Some(9),
            topic_archived: false,
            poll,
            saved,
            title_html: None,
            dynamic_attr: false,
            state,
        }
        .render()
    }

    fn shown() -> UiState {
        UiState {
            results: Some(true),
            ..Default::default()
        }
    }

    fn user(name: &str) -> Value {
        json!({"id": 1, "username": name, "avatar_template": format!("/letter/{name}/{{size}}.png")})
    }

    #[test]
    fn draws_standard_results_by_votes_with_their_voters() {
        let poll = json!({
            "name": "poll", "type": "regular", "status": "open", "results": "always",
            "public": true, "chart_type": "bar", "voters": 30,
            "options": [
                {"id": "b", "html": "Beta", "votes": 2},
                {"id": "a", "html": "Alpha", "votes": 27},
                {"id": "g", "html": "Gamma", "votes": 1},
            ],
            "preloaded_voters": {"b": [user("ann"), user("bob")], "a": [user("cy")]},
        });
        let html = draw(&poll, vec![json!("b")], &shown());
        let alpha = html.find("Alpha").unwrap();
        assert!(
            alpha < html.find("Beta").unwrap()
                && html.find("Beta").unwrap() < html.find("Gamma").unwrap()
        );
        assert!(
            html.contains(
                "<span class=\"percentage\">90%</span> <span class=\"option-text\">Alpha</span>"
            ),
            "{html}"
        );
        assert!(
            html.contains("<div class=\"bar\" style=\"width:7%\"></div>"),
            "{html}"
        );
        assert!(html.contains("<li class=\"chosen\"> <div class=\"option\"> <p> <span class=\"percentage\">7%</span>"), "{html}");
        assert!(
            html.contains(
                "<li class=\"\"> <div class=\"option\"> <p> <span class=\"percentage\">90%</span>"
            ),
            "{html}"
        );
        // The tabs: Votes only.
        assert!(html.contains("<li class=\"tab nav-item active\"> <button class=\"nav-btn btn no-text\" type=\"button\" data-poll-tab=\"votes\"> Votes </button> </li> </ul>"), "{html}");
        // Voters at getRawAvatarSize(24), a load more where votes exceed them.
        assert!(html.contains("<li> <a data-user-card=\"ann\" title=\"ann\"> <img alt=\"\" width=\"24\" height=\"24\" src=\"/forum/letter/ann/48.png\" class=\"avatar\"> </a> </li>"), "{html}");
        // Beta has its two; Alpha and Gamma have more votes than voters.
        assert_eq!(
            html.matches("poll-voters-toggle-expand").count(),
            2,
            "{html}"
        );
        assert!(html.contains("data-poll-voters=\"a\""), "{html}");
        assert!(html.contains("data-poll-voters=\"g\""), "{html}");
        // The gear's one action: the tally.
        assert!(html.contains("show-tally"), "{html}");

        let tally = UiState {
            tally: true,
            ..shown()
        };
        let html = draw(&poll, vec![json!("b")], &tally);
        assert!(
            html.contains("<span class=\"absolute\">27 votes</span>"),
            "{html}"
        );
        assert!(
            html.contains("<span class=\"absolute\">1 vote</span>"),
            "{html}"
        );
        assert!(html.contains("show-percentage"), "{html}");

        // Voters a load more brought replace the preloaded ones.
        let mut kept = Map::new();
        kept.insert("a".into(), json!([["cy", "/c/{size}.png"], ["dee", ""]]));
        let expanded = UiState {
            voters: Some(kept),
            ..shown()
        };
        let html = draw(&poll, vec![json!("b")], &expanded);
        assert!(html.contains("<a data-user-card=\"dee\" title=\"dee\"> <div class=\"avatar-placeholder\"></div> </a>"), "{html}");
        assert!(!html.contains("data-user-card=\"ann\""), "{html}");
        assert!(html.contains("&quot;expanded&quot;:true"), "{html}");
    }

    #[test]
    fn multiple_choice_percentages_round_down() {
        let poll = json!({
            "name": "m", "type": "multiple", "status": "open", "results": "always",
            "chart_type": "bar", "voters": 3,
            "options": [{"id": "r", "html": "Red", "votes": 2}, {"id": "g", "html": "Green", "votes": 1}],
        });
        let html = draw(&poll, Vec::new(), &shown());
        assert!(
            html.contains("<span class=\"percentage\">66%</span>"),
            "{html}"
        );
        assert!(
            html.contains("<span class=\"percentage\">33%</span>"),
            "{html}"
        );
        // Not public: no voters.
        assert!(!html.contains("poll-voters"), "{html}");
    }

    #[test]
    fn number_polls_show_their_average_and_pies_their_canvas() {
        let poll = json!({
            "name": "n", "type": "number", "status": "open", "results": "always",
            "chart_type": "pie", "voters": 3,
            "options": [{"id": "1", "html": "1", "votes": 1}, {"id": "5", "html": "5", "votes": 2}],
        });
        let html = draw(&poll, Vec::new(), &shown());
        assert!(html.contains("<div class=\"discourse-poll-number-results\"> <span>Average rating: <strong>3.67</strong>.</span> </div>"), "{html}");
        let empty = json!({
            "name": "n", "type": "number", "status": "open", "results": "always",
            "voters": 0, "options": [{"id": "1", "html": "1", "votes": 0}],
        });
        assert!(draw(&empty, Vec::new(), &shown()).contains("<strong>0</strong>"));

        let pie = json!({
            "name": "p", "type": "regular", "status": "open", "results": "always",
            "chart_type": "pie", "voters": 3,
            "options": [{"id": "c", "html": "Cats", "votes": 2}, {"id": "d", "html": "Dogs", "votes": 1}],
        });
        let html = draw(&pie, Vec::new(), &shown());
        assert!(html.contains("<div class=\"discourse-poll-pie-chart\"> <div class=\"poll-results-chart\"> <canvas class=\"poll-results-canvas\" id=\"poll-results-chart-p-3\"></canvas> <ul class=\"pie-chart-legends\" id=\"poll-results-legend-p-3\"> </ul> </div> </div>"), "{html}");
        // What poll.js draws the chart from; no tally for a pie.
        assert!(html.contains("&quot;pie&quot;:{&quot;id&quot;:&quot;p-3&quot;,&quot;data&quot;:[2,1],&quot;labels&quot;:[&quot;Cats&quot;,&quot;Dogs&quot;]}"), "{html}");
        assert!(!html.contains("show-tally"), "{html}");
    }

    fn ranked(public: bool) -> Value {
        let mut poll = json!({
            "name": "rc", "type": "ranked_choice", "status": "open", "results": "always",
            "chart_type": "bar", "voters": 3,
            "options": [
                {"id": "x", "html": "Xeno", "votes": 3},
                {"id": "y", "html": "Yarn", "votes": 3},
            ],
            "ranked_choice_outcome": {
                "tied": false, "tied_candidates": null, "winner": true,
                "winning_candidate": {"digest": "y", "html": "Yarn"},
                "round_activity": [
                    {"round": 1, "majority": null, "eliminated": [{"digest": "x", "html": "Xeno"}]},
                    {"round": 2, "majority": {"digest": "y", "html": "Yarn"}, "eliminated": null},
                ],
            },
        });
        if public {
            poll["public"] = json!(true);
            poll["preloaded_voters"] = json!({
                "x": [
                    {"rank": "Abstain", "user": user("ann")},
                    {"rank": "2", "user": user("bob")},
                    {"rank": "1", "user": user("cy")},
                ],
            });
        }
        poll
    }

    #[test]
    fn ranked_choice_results_show_the_outcome_and_the_votes() {
        let html = draw(&ranked(true), Vec::new(), &shown());
        // A public ranked choice starts on its Outcome tab.
        assert!(html.contains("<li class=\"tab nav-item\"> <button class=\"nav-btn btn no-text\" type=\"button\" data-poll-tab=\"votes\"> Votes </button> </li> <li class=\"tab nav-item active\">"), "{html}");
        assert!(html.contains("<tr> <td>1</td> <td>None</td> <td> <span>Xeno</span> </td> </tr> <tr> <td>2</td> <td>Yarn</td> <td>None</td> </tr>"), "{html}");
        assert!(html.contains("<span class=\"poll-results-ranked-choice-info\">Winner was Yarn after 2 rounds.</span>"), "{html}");

        let votes = UiState {
            tab: Some("votes".into()),
            ..shown()
        };
        let html = draw(&ranked(true), Vec::new(), &votes);
        // No percentage or bar; voters grouped by rank, Abstain last.
        assert!(
            !html.contains("percentage") && !html.contains("bar-back"),
            "{html}"
        );
        let one = html.find("<span class=\"rank\">1</span>").unwrap();
        let two = html.find("<span class=\"rank\">2</span>").unwrap();
        let abstain = html
            .find("<span class=\"rank\"><svg class=\"fa d-icon d-icon-ban")
            .unwrap();
        assert!(one < two && two < abstain, "{html}");

        // A private one has the Outcome tab only.
        let html = draw(&ranked(false), Vec::new(), &votes);
        assert!(!html.contains("data-poll-tab=\"votes\""), "{html}");
        assert!(html.contains("<li class=\"tab nav-item active\"> <button class=\"nav-btn btn no-text\" type=\"button\" data-poll-tab=\"outcome\"> Outcome </button>"), "{html}");

        let mut tied = ranked(false);
        tied["ranked_choice_outcome"] = json!({
            "tied": true, "winner": null, "winning_candidate": null,
            "tied_candidates": [{"digest": "x", "html": "Xeno"}, {"digest": "y", "html": "Yarn"}],
            "round_activity": [{"round": 1, "majority": null, "eliminated": [{"digest": "x", "html": "Xeno"}, {"digest": "y", "html": "Yarn"}]}],
        });
        let html = draw(&tied, Vec::new(), &shown());
        assert!(html.contains("<span class=\"poll-results-ranked-choice-info\">Tied after one round between the following candidates:</span> <ul class=\"poll-results-ranked-choice-tied-candidates\"> <li class=\"poll-results-ranked-choice-tied-candidate\">Xeno</li> <li class=\"poll-results-ranked-choice-tied-candidate\">Yarn</li> </ul>"), "{html}");
    }

    #[test]
    fn ranked_choice_options_carry_their_rank_dropdowns() {
        let state = UiState {
            vote: Some(json!([{"digest": "y", "rank": 1}, {"digest": "x", "rank": 0}])),
            ..Default::default()
        };
        let html = draw(&ranked(false), Vec::new(), &state);
        assert!(html.contains("<ul class=\"ranked-choice-poll-options options\"> <div class=\"ranked-choice-poll-option\" data-poll-option-id=\"x\" data-poll-option-rank=\"0\" tabindex=\"0\"> <button aria-expanded=\"false\" class=\"btn no-text fk-d-menu__trigger -trigger\" data-trigger=\"\" type=\"button\""), "{html}");
        assert!(
            html.contains("<span class=\"d-button-label\"> Abstain </span>"),
            "{html}"
        );
        assert!(html.contains("data-poll-option-rank=\"1\""), "{html}");
        assert!(
            html.contains("<span class=\"d-button-label\"> 1 </span>"),
            "{html}"
        );
        // The dropdown: Abstain, then each rank with the ends named.
        assert!(html.contains("data-poll-rank&#x3D;&quot;1&quot;&gt;&lt;span class&#x3D;&quot;d-button-label&quot;&gt;1 (highest priority)&lt;/span&gt;"), "{html}");
        assert!(html.contains("2 (lowest priority)"), "{html}");
        // Every option ranked, the ranks valid: the vote can be cast.
        assert!(
            html.contains("<button class=\"btn cast-votes btn-primary\""),
            "{html}"
        );
        assert!(
            html.contains("&quot;options&quot;:[&quot;x&quot;,&quot;y&quot;]"),
            "{html}"
        );
    }
}
