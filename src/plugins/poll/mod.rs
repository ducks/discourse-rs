//! discourse-poll, on the tables Rails' plugin created (`polls`,
//! `poll_options`, `poll_votes`): the polls a post's cooked HTML holds,
//! validated (`PollsValidator`, `PostValidator`) and written when the
//! post is created, their closing scheduled, and the `polls` and
//! `polls_votes` keys of the post serializer.

use std::collections::HashMap;

use chrono::NaiveDateTime;
use markup5ever_rcdom::Handle;
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::guardian::Guardian;
use crate::i18n::I18n;
use crate::posting::Ctx;
use crate::pretty_text::cleanup::{
    all_elements_within, attr, element_name, fragment_root, has_class, node_html, parse,
};
use crate::site_settings::{SettingError, SiteSettings};
use crate::topic_list::time_json;
use crate::{AppError, Unsupported};

/// `DiscoursePoll::DEFAULT_POLL_NAME`
const DEFAULT_POLL_NAME: &str = "poll";
/// `DiscoursePoll::DATA_PREFIX`
const DATA_PREFIX: &str = "data-poll-";
/// `PollsValidator::MAX_VALUE`
const MAX_VALUE: i64 = 2_147_483_647;

const TYPES: [&str; 4] = ["regular", "multiple", "number", "ranked_choice"];
const STATUSES: [&str; 2] = ["open", "closed"];
const RESULTS: [&str; 4] = ["always", "on_vote", "on_close", "staff_only"];
const VISIBILITIES: [&str; 2] = ["secret", "everyone"];
const CHART_TYPES: [&str; 2] = ["bar", "pie"];

/// `enabled_site_setting :poll_enabled`
pub fn enabled(settings: &SiteSettings) -> Result<bool, SettingError> {
    Ok(settings.get("poll_enabled")?.truthy())
}

/// One poll `DiscoursePoll::Poll.extract` found: its `data-poll-*`
/// attributes (HTML-escaped, the prefix dropped), options and title.
#[derive(Debug, Clone, PartialEq)]
pub struct Extracted {
    pub attrs: HashMap<String, String>,
    /// `{id, html}`: the option's digest and inner HTML.
    pub options: Vec<(String, String)>,
    pub title: Option<String>,
}

impl Extracted {
    fn get(&self, key: &str) -> Option<&str> {
        self.attrs.get(key).map(String::as_str)
    }

    /// `poll["name"]`, the default unless the attribute names one.
    pub fn name(&self) -> &str {
        self.get("name").unwrap_or(DEFAULT_POLL_NAME)
    }

    /// `poll[key].presence`
    fn present(&self, key: &str) -> Option<&str> {
        self.get(key).filter(|v| !crate::ruby::is_blank(v))
    }
}

fn inner_html(node: &Handle) -> String {
    node.children.borrow().iter().map(node_html).collect()
}

/// Each `div.poll` without a blockquote among its ancestors.
fn poll_divs(node: &Handle, quoted: bool, out: &mut Vec<Handle>) {
    for child in node.children.borrow().iter() {
        let name = element_name(child);
        if !quoted && name == Some("div") && has_class(child, "poll") {
            out.push(child.clone());
        }
        poll_divs(child, quoted || name == Some("blockquote"), out);
    }
}

/// `DiscoursePoll::Poll.extract` over the cooked HTML: each `div.poll`
/// outside a blockquote.
pub fn extract(cooked: &str) -> Vec<Extracted> {
    let dom = parse(cooked);
    let mut divs = Vec::new();
    poll_divs(&fragment_root(&dom), false, &mut divs);
    let mut polls = Vec::new();
    for p in divs {
        let mut attrs = HashMap::new();
        if let markup5ever_rcdom::NodeData::Element { attrs: list, .. } = &p.data {
            for a in list.borrow().iter() {
                if let Some(key) = a.name.local.strip_prefix(DATA_PREFIX) {
                    attrs.insert(
                        key.to_string(),
                        crate::discourse_diff::escape_html(&a.value),
                    );
                }
            }
        }
        let within = all_elements_within(&p);
        let options = within
            .iter()
            .filter(|o| element_name(o) == Some("li"))
            .filter_map(|o| {
                attr(o, "data-poll-option-id").map(|id| (id, inner_html(o).trim().to_string()))
            })
            .collect();
        let title = within
            .iter()
            .find(|t| has_class(t, "poll-title"))
            .map(|t| inner_html(t).trim().to_string());
        polls.push(Extracted {
            attrs,
            options,
            title,
        });
    }
    polls
}

/// The polls a post's raw holds, cooked as `extract` cooks it.
pub async fn extract_raw(ctx: &Ctx<'_>, raw: &str) -> Result<Vec<Extracted>, AppError> {
    if !raw.contains("[/poll]") {
        return Ok(Vec::new());
    }
    let cooked = crate::pretty_text::cook(
        ctx.host,
        raw,
        &crate::pretty_text::MarkdownOptions::default(),
    )
    .await?;
    Ok(extract(&cooked))
}

/// `DiscoursePoll::PollsValidator#validate_polls`: the polls by name, or
/// the errors of the first invalid one.
pub fn validate_polls(
    settings: &SiteSettings,
    i18n: &I18n,
    extracted: Vec<Extracted>,
) -> Result<Result<Vec<Extracted>, Vec<String>>, SettingError> {
    let mut polls: Vec<Extracted> = Vec::new();
    for poll in extracted {
        let mut errors = Vec::new();
        valid_arguments(i18n, &poll, &mut errors);
        if errors.is_empty() {
            valid_numbers(i18n, &poll, &mut errors);
        }
        if errors.is_empty() {
            other_checks(settings, i18n, &polls, &poll, &mut errors)?;
        }
        if !errors.is_empty() {
            return Ok(Err(errors));
        }
        polls.push(poll);
    }
    Ok(Ok(polls))
}

fn t_with(i18n: &I18n, key: &str, args: &[(&str, &str)]) -> String {
    i18n.t_with(key, args).unwrap_or_else(|| key.to_string())
}

fn valid_arguments(i18n: &I18n, poll: &Extracted, errors: &mut Vec<String>) {
    for (argument, allowed) in [
        ("type", &TYPES[..]),
        ("status", &STATUSES[..]),
        ("results", &RESULTS[..]),
    ] {
        if let Some(value) = poll.present(argument)
            && !allowed.contains(&value)
        {
            errors.push(t_with(
                i18n,
                "poll.invalid_argument",
                &[("argument", argument), ("value", value)],
            ));
        }
    }
}

fn valid_numbers(i18n: &I18n, poll: &Extracted, errors: &mut Vec<String>) {
    if poll.get("type") != Some("number") {
        return;
    }
    let to_i = |key: &str, default: i64| match poll.present(key) {
        Some(v) => crate::ruby::to_i(v),
        None => default,
    };
    let min = crate::ruby::to_i(poll.get("min").unwrap_or(""));
    let max = to_i("max", MAX_VALUE);
    let step = to_i("step", 1);
    let greater = |count: &str| t_with(i18n, "errors.messages.greater_than", &[("count", count)]);
    let less = |count: &str| t_with(i18n, "errors.messages.less_than", &[("count", count)]);
    let max_value = MAX_VALUE.to_string();
    if min < 0 {
        errors.push(format!("Min {}", greater("0")));
    } else if min > MAX_VALUE {
        errors.push(format!("Min {}", less(&max_value)));
    }
    if max < min {
        errors.push(format!("Max {}", greater("min")));
    } else if max > MAX_VALUE {
        errors.push(format!("Max {}", less(&max_value)));
    }
    if step <= 0 {
        errors.push(format!("Step {}", greater("0")));
    } else if (max - min + 1).div_euclid(step) < 2 {
        errors.push(named(
            i18n,
            poll,
            "default_poll_must_have_at_least_1_option",
            "named_poll_must_have_at_least_1_option",
        ));
    }
}

/// The default poll's message, or the named poll's with its name.
fn named(i18n: &I18n, poll: &Extracted, default: &str, named: &str) -> String {
    if poll.name() == DEFAULT_POLL_NAME {
        t_with(i18n, &format!("poll.{default}"), &[])
    } else {
        t_with(i18n, &format!("poll.{named}"), &[("name", poll.name())])
    }
}

fn other_checks(
    settings: &SiteSettings,
    i18n: &I18n,
    polls: &[Extracted],
    poll: &Extracted,
    errors: &mut Vec<String>,
) -> Result<(), SettingError> {
    // unique_poll_name?
    if polls.iter().any(|p| p.name() == poll.name()) {
        errors.push(named(
            i18n,
            poll,
            "multiple_polls_without_name",
            "multiple_polls_with_same_name",
        ));
        return Ok(());
    }
    // unique_options?
    let mut ids: Vec<&str> = poll.options.iter().map(|(id, _)| id.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    if ids.len() != poll.options.len() {
        errors.push(named(
            i18n,
            poll,
            "default_poll_must_have_different_options",
            "named_poll_must_have_different_options",
        ));
        return Ok(());
    }
    // any_blank_options?
    if poll
        .options
        .iter()
        .any(|(_, html)| crate::ruby::is_blank(html))
    {
        errors.push(named(
            i18n,
            poll,
            "default_poll_must_not_have_any_empty_options",
            "named_poll_must_not_have_any_empty_options",
        ));
        return Ok(());
    }
    // at_least_one_option?
    if poll.options.is_empty() {
        errors.push(named(
            i18n,
            poll,
            "default_poll_must_have_at_least_1_option",
            "named_poll_must_have_at_least_1_option",
        ));
        return Ok(());
    }
    // valid_number_of_options?
    let maximum = settings.get("poll_maximum_options")?.to_i();
    if poll.options.len() as i64 > maximum {
        let count = maximum.to_string();
        let message = if poll.name() == DEFAULT_POLL_NAME {
            i18n.t_count(
                "poll.default_poll_must_have_less_options",
                maximum,
                &[("count", &count)],
            )
        } else {
            i18n.t_count(
                "poll.named_poll_must_have_less_options",
                maximum,
                &[("name", poll.name()), ("count", &count)],
            )
        };
        errors.push(message.unwrap_or_default());
        return Ok(());
    }
    // valid_multiple_choice_settings?
    if poll.get("type") == Some("multiple") {
        let options = poll.options.len() as i64;
        let min = poll.present("min").map_or(1, crate::ruby::to_i);
        let max = poll.present("max").map_or(options, crate::ruby::to_i);
        if min > max || min <= 0 || max <= 0 || max > options || min >= options {
            errors.push(named(
                i18n,
                poll,
                "default_poll_with_multiple_choices_has_invalid_parameters",
                "named_poll_with_multiple_choices_has_invalid_parameters",
            ));
        }
    }
    Ok(())
}

/// `DiscoursePoll::PostValidator#validate_post`: staff, a member of
/// `poll_create_allowed_groups`, or a message with one human in it.
pub async fn validate_post(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    i18n: &I18n,
    guardian: &Guardian,
    topic_id: Option<i32>,
) -> Result<Option<String>, AppError> {
    if guardian.is_staff() || guardian.in_setting_groups(settings, "poll_create_allowed_groups")? {
        return Ok(None);
    }
    if let Some(topic_id) = topic_id
        && pm_with_non_human_user(conn, topic_id).await?
    {
        return Ok(None);
    }
    Ok(Some(t_with(
        i18n,
        "poll.insufficient_rights_to_create",
        &[],
    )))
}

/// `Topic#pm_with_non_human_user?`
async fn pm_with_non_human_user(
    conn: &mut PgConnection,
    topic_id: i32,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM topics \
           LEFT JOIN topic_allowed_groups ON topics.id = topic_allowed_groups.topic_id \
           WHERE topic_allowed_groups.topic_id IS NULL AND topics.archetype = 'private_message' \
             AND topics.id = $1 \
             AND (SELECT COUNT(*) FROM topic_allowed_users \
                  WHERE topic_allowed_users.topic_id = $1 AND topic_allowed_users.user_id > 0) = 1)",
    )
    .bind(topic_id)
    .fetch_one(conn)
    .await
}

fn enum_value(values: &[&str], value: &str, column: &str) -> Result<i32, Unsupported> {
    values
        .iter()
        .position(|v| *v == value)
        .map(|i| i as i32)
        .ok_or(Unsupported(match column {
            "type" => "a poll with an unknown type",
            "status" => "a poll with an unknown status",
            "results" => "a poll with unknown results",
            _ => "a poll with an unknown chart type",
        }))
}

/// An integer column's cast and numericality validation: blank is nil,
/// anything but an integer within bounds fails `create!`.
fn integer_column(value: Option<&str>, min: i64) -> Result<Option<i32>, Unsupported> {
    let Some(value) = value.filter(|v| !crate::ruby::is_blank(v)) else {
        return Ok(None);
    };
    crate::ruby::integer(value.trim())
        .filter(|n| *n >= min)
        .and_then(|n| i32::try_from(n).ok())
        .map(Some)
        .ok_or(Unsupported(
            "a poll whose min, max or step fails validation",
        ))
}

/// `Time.zone.parse(poll["close"] || "")` in UTC.
fn close_at(value: Option<&str>) -> Result<Option<NaiveDateTime>, Unsupported> {
    let Some(value) = value.filter(|v| !v.trim().is_empty()) else {
        return Ok(None);
    };
    if let Ok(t) = chrono::DateTime::parse_from_rfc3339(value) {
        return Ok(Some(t.naive_utc()));
    }
    for format in [
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%d %H:%M",
    ] {
        if let Ok(t) = NaiveDateTime::parse_from_str(value, format) {
            return Ok(Some(t));
        }
    }
    Err(Unsupported("a poll close time other than ISO 8601"))
}

/// The rows `DiscoursePoll::Poll.create!` writes for one poll, checked
/// before anything is written.
pub struct NewPoll {
    name: String,
    close_at: Option<NaiveDateTime>,
    kind: i32,
    status: i32,
    visibility: i32,
    title: Option<String>,
    results: i32,
    min: Option<i32>,
    max: Option<i32>,
    step: Option<i32>,
    chart_type: i32,
    groups: Option<String>,
    dynamic: bool,
    options: Vec<(String, String)>,
}

impl NewPoll {
    pub fn from_extracted(poll: &Extracted) -> Result<NewPoll, Unsupported> {
        Ok(NewPoll {
            name: poll
                .present("name")
                .unwrap_or(DEFAULT_POLL_NAME)
                .to_string(),
            close_at: close_at(poll.get("close"))?,
            kind: enum_value(&TYPES, poll.present("type").unwrap_or("regular"), "type")?,
            status: enum_value(
                &STATUSES,
                poll.present("status").unwrap_or("open"),
                "status",
            )?,
            visibility: enum_value(
                &VISIBILITIES,
                if poll.get("public") == Some("true") {
                    "everyone"
                } else {
                    "secret"
                },
                "visibility",
            )?,
            title: poll.title.clone(),
            results: enum_value(
                &RESULTS,
                poll.present("results").unwrap_or("always"),
                "results",
            )?,
            min: integer_column(poll.get("min"), 0)?,
            max: integer_column(poll.get("max"), 1)?,
            step: integer_column(poll.get("step"), 1)?,
            chart_type: enum_value(
                &CHART_TYPES,
                poll.get("charttype").unwrap_or("bar"),
                "chart_type",
            )?,
            groups: poll.get("groups").map(str::to_string),
            dynamic: poll.get("dynamic") == Some("true"),
            options: poll.options.clone(),
        })
    }
}

/// The post's `after_save`: each poll and its options, then the
/// `has_polls` custom field.
pub async fn create(
    conn: &mut PgConnection,
    post_id: i32,
    polls: &[NewPoll],
) -> Result<(), sqlx::Error> {
    if polls.is_empty() {
        return Ok(());
    }
    for poll in polls {
        let poll_id: i64 = sqlx::query_scalar(
            "INSERT INTO polls (post_id, name, close_at, type, status, visibility, title, results, \
                                min, max, step, chart_type, groups, dynamic, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, \
                     clock_timestamp(), clock_timestamp()) RETURNING id",
        )
        .bind(post_id)
        .bind(&poll.name)
        .bind(poll.close_at)
        .bind(poll.kind)
        .bind(poll.status)
        .bind(poll.visibility)
        .bind(&poll.title)
        .bind(poll.results)
        .bind(poll.min)
        .bind(poll.max)
        .bind(poll.step)
        .bind(poll.chart_type)
        .bind(&poll.groups)
        .bind(poll.dynamic)
        .fetch_one(&mut *conn)
        .await?;
        for (digest, html) in &poll.options {
            sqlx::query(
                "INSERT INTO poll_options (poll_id, digest, html, created_at, updated_at) \
                 VALUES ($1, $2, $3, clock_timestamp(), clock_timestamp())",
            )
            .bind(poll_id)
            .bind(Some(digest.as_str()).filter(|d| !crate::ruby::is_blank(d)))
            .bind(Some(html.trim()).filter(|h| !h.is_empty()))
            .execute(&mut *conn)
            .await?;
        }
    }
    // save_custom_fields(true): the boolean field, written as "t".
    sqlx::query(
        "INSERT INTO post_custom_fields (post_id, name, value, created_at, updated_at) \
         VALUES ($1, 'has_polls', 't', clock_timestamp(), clock_timestamp())",
    )
    .bind(post_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// DiscourseEvent `:post_created` after `schedule_jobs`: a reply's polls
/// to `/polls/<topic_id>`, serialized for its author.
pub async fn publish_created(
    ctx: &Ctx<'_>,
    conn: &mut PgConnection,
    guardian: &Guardian,
    post_id: i32,
) -> Result<(), AppError> {
    let (topic_id, post_number, user_id): (i32, i32, Option<i32>) =
        sqlx::query_as("SELECT topic_id, post_number, user_id FROM posts WHERE id = $1")
            .bind(post_id)
            .fetch_one(&mut *conn)
            .await?;
    if post_number == 1 {
        return Ok(());
    }
    let has_polls: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM post_custom_fields WHERE post_id = $1 AND name = 'has_polls')",
    )
    .bind(post_id)
    .fetch_one(&mut *conn)
    .await?;
    if !has_polls {
        return Ok(());
    }
    let data = PostsData::load(conn, &[post_id], guardian.user_id()).await?;
    let polls = data.serialize(guardian, post_id, user_id)?;
    crate::bus::publish_post_message(
        ctx.bus,
        conn,
        post_id,
        &format!("/polls/{topic_id}"),
        &json!({"post_id": post_id, "polls": polls}),
    )
    .await?;
    Ok(())
}

/// `DiscoursePoll::Poll.schedule_jobs`: each open poll closing later
/// gets its ClosePoll, any earlier one cancelled.
pub async fn schedule_jobs(conn: &mut PgConnection, post_id: i32) -> Result<(), sqlx::Error> {
    let polls: Vec<(String, i32, Option<NaiveDateTime>)> =
        sqlx::query_as("SELECT name, status, close_at FROM polls WHERE post_id = $1 ORDER BY id")
            .bind(post_id)
            .fetch_all(&mut *conn)
            .await?;
    let now = crate::clock::now_naive();
    for (name, status, close_at) in polls {
        let args = json!({"post_id": post_id, "poll_name": name});
        sqlx::query(
            "DELETE FROM discourse_rs.jobs WHERE name = 'close_poll' AND failed_at IS NULL \
             AND locked_until IS NULL AND args = $1",
        )
        .bind(&args)
        .execute(&mut *conn)
        .await?;
        if let Some(at) = close_at.filter(|at| status == 0 && *at > now) {
            crate::jobs::enqueue_at(conn, at, "close_poll", args).await?;
        }
    }
    Ok(())
}

#[derive(sqlx::FromRow)]
struct PollRow {
    id: i64,
    post_id: i64,
    name: String,
    #[sqlx(rename = "type")]
    kind: i32,
    status: i32,
    results: i32,
    visibility: i32,
    min: Option<i32>,
    max: Option<i32>,
    step: Option<i32>,
    close_at: Option<NaiveDateTime>,
    chart_type: i32,
    groups: Option<String>,
    title: Option<String>,
    dynamic: bool,
    closed_at: Option<NaiveDateTime>,
    closed_by_id: Option<i32>,
    anonymous_voters: Option<i32>,
    voters: i64,
    has_voted: bool,
}

#[derive(sqlx::FromRow)]
struct OptionRow {
    poll_id: i64,
    digest: String,
    html: String,
    anonymous_votes: Option<i32>,
    votes: i64,
}

/// The polls of a page's posts (`Poll.preload!`), for the post
/// serializer's keys.
pub struct PostsData {
    polls: Vec<PollRow>,
    options: Vec<OptionRow>,
}

impl PostsData {
    pub async fn load(
        conn: &mut PgConnection,
        post_ids: &[i32],
        user_id: Option<i32>,
    ) -> Result<PostsData, sqlx::Error> {
        let polls: Vec<PollRow> = sqlx::query_as(
            "SELECT p.id, p.post_id, p.name, p.type, p.status, p.results, p.visibility, p.min, \
                    p.max, p.step, p.close_at, p.chart_type, p.groups, p.title, p.dynamic, \
                    p.closed_at, p.closed_by_id, p.anonymous_voters, \
                    (SELECT COUNT(DISTINCT v.user_id) FROM poll_votes v WHERE v.poll_id = p.id) AS voters, \
                    EXISTS (SELECT 1 FROM poll_votes v WHERE v.poll_id = p.id AND v.user_id = $2) AS has_voted \
             FROM polls p WHERE p.post_id = ANY($1) ORDER BY p.id",
        )
        .bind(post_ids)
        .bind(user_id)
        .fetch_all(&mut *conn)
        .await?;
        let ids: Vec<i64> = polls.iter().map(|p| p.id).collect();
        let options: Vec<OptionRow> = sqlx::query_as(
            "SELECT o.poll_id, o.digest, o.html, o.anonymous_votes, \
                    (SELECT COUNT(*) FROM poll_votes v WHERE v.poll_option_id = o.id) AS votes \
             FROM poll_options o WHERE o.poll_id = ANY($1) ORDER BY o.id",
        )
        .bind(&ids)
        .fetch_all(&mut *conn)
        .await?;
        Ok(PostsData { polls, options })
    }

    fn post_polls(&self, post_id: i32) -> impl Iterator<Item = &PollRow> {
        self.polls
            .iter()
            .filter(move |p| p.post_id == i64::from(post_id))
    }

    /// `PollSerializer` for each of the post's polls.
    pub fn serialize(
        &self,
        guardian: &Guardian,
        post_id: i32,
        post_user_id: Option<i32>,
    ) -> Result<Vec<Value>, Unsupported> {
        let now = crate::clock::now_naive();
        let mut out = Vec::new();
        for poll in self.post_polls(post_id) {
            let closed = poll.status == 1 || poll.close_at.is_some_and(|at| at <= now);
            let is_me = guardian.user_id().is_some() && guardian.user_id() == post_user_id;
            // can_see_results?
            let can_see_results = if poll.results == 3 {
                guardian.is_staff()
            } else {
                poll.results == 0 || (poll.results == 1 && (is_me || poll.has_voted)) || closed
            };
            let kind = TYPES[poll.kind as usize];
            let number_or_multiple = kind == "number" || kind == "multiple";
            let everyone = poll.visibility == 1;
            let mut p = Map::new();
            p.insert("id".into(), json!(poll.id));
            p.insert("name".into(), json!(poll.name));
            p.insert("type".into(), json!(kind));
            p.insert("status".into(), json!(STATUSES[poll.status as usize]));
            if everyone {
                p.insert("public".into(), json!(true));
            }
            p.insert("dynamic".into(), json!(poll.dynamic));
            p.insert("results".into(), json!(RESULTS[poll.results as usize]));
            if let Some(min) = poll.min.filter(|_| number_or_multiple) {
                p.insert("min".into(), json!(min));
            }
            if let Some(max) = poll.max.filter(|_| number_or_multiple) {
                p.insert("max".into(), json!(max));
            }
            if let Some(step) = poll.step.filter(|_| kind == "number") {
                p.insert("step".into(), json!(step));
            }
            let options: Vec<Value> = self
                .options
                .iter()
                .filter(|o| o.poll_id == poll.id)
                .map(|o| {
                    let mut option = Map::new();
                    option.insert("id".into(), json!(o.digest));
                    option.insert("html".into(), json!(o.html));
                    if can_see_results {
                        option.insert(
                            "votes".into(),
                            json!(o.votes + i64::from(o.anonymous_votes.unwrap_or(0))),
                        );
                    }
                    Value::Object(option)
                })
                .collect();
            p.insert("options".into(), Value::Array(options));
            p.insert(
                "voters".into(),
                json!(poll.voters + i64::from(poll.anonymous_voters.unwrap_or(0))),
            );
            if let Some(close) = poll.close_at {
                p.insert("close".into(), json!(time_json(close)));
            }
            // can_see_voters?: serialized_voters, nil without votes.
            if everyone && can_see_results {
                if poll.voters > 0 {
                    return Err(Unsupported("a public poll's voters"));
                }
                p.insert("preloaded_voters".into(), Value::Null);
            }
            p.insert(
                "chart_type".into(),
                json!(CHART_TYPES[poll.chart_type as usize]),
            );
            if let Some(groups) = poll.groups.as_ref().filter(|g| !crate::ruby::is_blank(g)) {
                p.insert("groups".into(), json!(groups));
            }
            p.insert("title".into(), json!(poll.title));
            // RankedChoice.outcome: nil without ballots.
            if kind == "ranked_choice" && can_see_results {
                if poll.voters > 0 {
                    return Err(Unsupported("a ranked choice poll's outcome"));
                }
                p.insert("ranked_choice_outcome".into(), Value::Null);
            }
            p.insert("closed_at".into(), json!(poll.closed_at.map(time_json)));
            if poll.closed_by_id.is_some_and(|id| id != -1) {
                return Err(Unsupported("a poll closed by a user"));
            }
            p.insert("closed_by".into(), Value::Null);
            out.push(Value::Object(p));
        }
        Ok(out)
    }

    /// The post serializer's `polls` (when the post has any) and
    /// `polls_votes` (when the viewer voted in one).
    pub fn post_keys(
        &self,
        guardian: &Guardian,
        post_id: i32,
        post_user_id: Option<i32>,
        out: &mut Map<String, Value>,
    ) -> Result<(), Unsupported> {
        if self.post_polls(post_id).next().is_none() {
            return Ok(());
        }
        out.insert(
            "polls".into(),
            Value::Array(self.serialize(guardian, post_id, post_user_id)?),
        );
        if guardian.user_id().is_some() && self.post_polls(post_id).any(|p| p.has_voted) {
            return Err(Unsupported("the viewer's poll votes"));
        }
        Ok(())
    }
}

/// The current user's `can_create_poll`.
pub fn current_user_keys(
    settings: &SiteSettings,
    guardian: &Guardian,
    out: &mut Map<String, Value>,
) -> Result<(), SettingError> {
    out.insert(
        "can_create_poll".into(),
        json!(
            guardian.is_staff()
                || guardian.in_setting_groups(settings, "poll_create_allowed_groups")?
        ),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const COOKED: &str = "<p>Which one?</p>\n<div class=\"poll\" data-poll-name=\"poll\" data-poll-results=\"always\" data-poll-status=\"open\" data-poll-type=\"regular\">\n<div class=\"poll-container\">\n<ul>\n<li data-poll-option-id=\"d704ae0a51c9cb797cdd37d02b1e1886\">Red</li>\n<li data-poll-option-id=\"3f560561c99e5625ea0a1e039eb5c01f\">Blue <em>sky</em></li>\n</ul>\n</div>\n</div>";

    #[test]
    fn extracts_attributes_and_options() {
        let polls = extract(COOKED);
        assert_eq!(polls.len(), 1);
        let poll = &polls[0];
        assert_eq!(poll.name(), "poll");
        assert_eq!(poll.get("results"), Some("always"));
        assert_eq!(
            poll.options,
            vec![
                (
                    "d704ae0a51c9cb797cdd37d02b1e1886".to_string(),
                    "Red".to_string()
                ),
                (
                    "3f560561c99e5625ea0a1e039eb5c01f".to_string(),
                    "Blue <em>sky</em>".to_string()
                ),
            ]
        );
        assert_eq!(poll.title, None);
    }

    #[test]
    fn skips_polls_in_blockquotes() {
        let quoted = format!("<blockquote>{COOKED}</blockquote>");
        assert!(extract(&quoted).is_empty());
    }

    #[test]
    fn escapes_attribute_values() {
        let polls = extract("<div class=\"poll\" data-poll-name=\"a&amp;b\"></div>");
        assert_eq!(polls[0].name(), "a&amp;b");
    }

    #[test]
    fn parses_close_times() {
        assert_eq!(close_at(Some("")).unwrap(), None);
        assert_eq!(
            close_at(Some("2026-10-20T10:00:00.000Z")).unwrap(),
            NaiveDateTime::parse_from_str("2026-10-20 10:00:00", "%Y-%m-%d %H:%M:%S").ok()
        );
    }
}
