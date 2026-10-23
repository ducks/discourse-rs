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
//! Not ported yet, and said in place: the results (PollResultsTabs,
//! PollResultsPie, the number poll's average), ranked choice's options
//! with their rank dropdowns, and the actions behind
//! PollButtonsDropdown (the tally toggle, the breakdown, export, close
//! and open), whose buttons are drawn but refuse when clicked.

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

/// A part of the widget not ported yet, said in its place.
fn unsupported(what: &'static str) -> String {
    format!(
        "<div class=\"poll-unsupported\" role=\"note\">{}.</div>",
        escape(&capitalize(&crate::Unsupported(what).to_string()))
    )
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
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
        Value::Object(state)
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
            format!(
                "<div class=\"{class}\"> {} </div>",
                unsupported(
                    "the poll's results (PollResultsTabs, PollResultsPie, the average rating)"
                )
            )
        } else if self.show_voted_choices() {
            self.voted_choices()
        } else {
            self.poll_options()
        }
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
            return unsupported("ranked choice options and their rank dropdowns");
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
            // showTally starts false: the percentages are shown.
            contents.push((
                "toggleDisplayMode",
                "btn-default show-tally",
                "poll.show-tally.label",
                "info",
            ));
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
const SCRIPT_STRINGS: [&str; 2] = ["poll.error_while_casting_votes", "generic_error"];

/// The topic page's script for its polls, with its strings. The dialogs
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
        "<script type=\"application/json\" id=\"poll-strings\">{}</script><script src=\"{}\"></script>",
        Value::Object(own).to_string().replace('<', "\\u003c"),
        crate::assets::url(base_path, "poll.js"),
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
}
