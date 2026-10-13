//! `PollSerializer` and the post serializer's `polls` and `polls_votes`,
//! with what they read loaded up front (`Poll.preload!`,
//! `preload_serialized_voters!`, `RankedChoice.outcome`).

use std::collections::HashMap;

use chrono::NaiveDateTime;
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use super::{CHART_TYPES, RESULTS, STATUSES, TYPES};
use crate::guardian::Guardian;
use crate::plugins::PluginError;
use crate::topic_list::time_json;
use crate::url::Urls;

#[derive(sqlx::FromRow)]
pub(super) struct PollRow {
    pub(super) id: i64,
    pub(super) post_id: i64,
    pub(super) name: String,
    #[sqlx(rename = "type")]
    pub(super) kind: i32,
    pub(super) status: i32,
    results: i32,
    visibility: i32,
    pub(super) min: Option<i32>,
    pub(super) max: Option<i32>,
    step: Option<i32>,
    pub(super) close_at: Option<NaiveDateTime>,
    chart_type: i32,
    pub(super) groups: Option<String>,
    title: Option<String>,
    dynamic: bool,
    closed_at: Option<NaiveDateTime>,
    closed_by_id: Option<i32>,
    anonymous_voters: Option<i32>,
    voters: i64,
    has_voted: bool,
    /// The post's author (`is_me?`).
    post_user_id: Option<i32>,
}

impl PollRow {
    pub(super) fn kind(&self) -> &'static str {
        TYPES[self.kind as usize]
    }

    /// `is_closed?`
    pub(super) fn is_closed(&self) -> bool {
        self.status == 1
            || self
                .close_at
                .is_some_and(|at| at <= crate::clock::now_naive())
    }

    /// `can_see_results?(user)`
    pub(super) fn can_see_results(&self, guardian: &Guardian) -> bool {
        if self.results == 3 {
            return guardian.is_staff();
        }
        let is_me = guardian.user_id().is_some() && guardian.user_id() == self.post_user_id;
        self.results == 0 || (self.results == 1 && (is_me || self.has_voted)) || self.is_closed()
    }

    /// `can_see_voters?(user)`
    pub(super) fn can_see_voters(&self, guardian: &Guardian) -> bool {
        self.visibility == 1 && self.can_see_results(guardian)
    }
}

#[derive(sqlx::FromRow)]
pub(super) struct OptionRow {
    pub(super) id: i64,
    poll_id: i64,
    pub(super) digest: String,
    pub(super) html: String,
    anonymous_votes: Option<i32>,
    votes: i64,
}

/// Polls with what their serializer reads, for one viewer.
pub struct PostsData {
    polls: Vec<PollRow>,
    options: Vec<OptionRow>,
    /// `serialized_voters` (first page), for public polls with votes.
    voters: HashMap<i64, Value>,
    /// `RankedChoice.outcome`, for ranked choice polls with votes.
    outcomes: HashMap<i64, Value>,
    /// The viewer's votes, for the polls they voted in.
    viewer_votes: HashMap<i64, Value>,
    /// `closed_by`'s BasicUserSerializer.
    closers: HashMap<i32, Value>,
}

const POLLS_SQL: &str = "SELECT p.id, p.post_id, p.name, p.type, p.status, p.results, p.visibility, p.min, \
        p.max, p.step, p.close_at, p.chart_type, p.groups, p.title, p.dynamic, \
        p.closed_at, p.closed_by_id, p.anonymous_voters, \
        (SELECT COUNT(DISTINCT v.user_id) FROM poll_votes v WHERE v.poll_id = p.id) AS voters, \
        EXISTS (SELECT 1 FROM poll_votes v WHERE v.poll_id = p.id AND v.user_id = $2) AS has_voted, \
        (SELECT user_id FROM posts WHERE id = p.post_id) AS post_user_id \
     FROM polls p";

impl PostsData {
    /// The polls of the posts.
    pub async fn load(
        conn: &mut PgConnection,
        urls: &Urls<'_>,
        post_ids: &[i32],
        user_id: Option<i32>,
    ) -> Result<PostsData, PluginError> {
        let ids: Vec<i64> = post_ids.iter().map(|id| i64::from(*id)).collect();
        let polls: Vec<PollRow> = sqlx::query_as(&format!(
            "{POLLS_SQL} WHERE p.post_id = ANY($1) ORDER BY p.id"
        ))
        .bind(&ids)
        .bind(user_id)
        .fetch_all(&mut *conn)
        .await?;
        Self::with_polls(conn, urls, polls, user_id).await
    }

    /// One poll (`Poll.find_by(post_id:, name:)`).
    pub(super) async fn load_poll(
        conn: &mut PgConnection,
        urls: &Urls<'_>,
        poll_id: i64,
        user_id: Option<i32>,
    ) -> Result<PostsData, PluginError> {
        let polls: Vec<PollRow> = sqlx::query_as(&format!("{POLLS_SQL} WHERE p.id = $1"))
            .bind(poll_id)
            .bind(user_id)
            .fetch_all(&mut *conn)
            .await?;
        Self::with_polls(conn, urls, polls, user_id).await
    }

    async fn with_polls(
        conn: &mut PgConnection,
        urls: &Urls<'_>,
        polls: Vec<PollRow>,
        user_id: Option<i32>,
    ) -> Result<PostsData, PluginError> {
        let ids: Vec<i64> = polls.iter().map(|p| p.id).collect();
        let options: Vec<OptionRow> = sqlx::query_as(
            "SELECT o.id, o.poll_id, o.digest, o.html, o.anonymous_votes, \
                    (SELECT COUNT(*) FROM poll_votes v WHERE v.poll_option_id = o.id) AS votes \
             FROM poll_options o WHERE o.poll_id = ANY($1) ORDER BY o.id",
        )
        .bind(&ids)
        .fetch_all(&mut *conn)
        .await?;
        let mut data = PostsData {
            polls: Vec::new(),
            options,
            voters: HashMap::new(),
            outcomes: HashMap::new(),
            viewer_votes: HashMap::new(),
            closers: HashMap::new(),
        };
        for poll in &polls {
            if poll.voters > 0 && poll.visibility == 1 {
                let voters = serialized_voters(conn, urls, poll, 1, 25, None).await?;
                data.voters.insert(poll.id, voters);
            }
            if poll.voters > 0 && poll.kind() == "ranked_choice" {
                let outcome = ranked_choice_outcome(conn, poll.id).await?;
                data.outcomes.insert(poll.id, outcome);
            }
            if let Some(uid) = user_id.filter(|_| poll.has_voted) {
                data.viewer_votes
                    .insert(poll.id, viewer_votes(conn, poll, uid).await?);
            }
            if let Some(closer) = poll.closed_by_id.filter(|id| *id != -1)
                && !data.closers.contains_key(&closer)
            {
                data.closers
                    .insert(closer, basic_user(conn, urls, closer, false).await?);
            }
        }
        data.polls = polls;
        Ok(data)
    }

    pub(super) fn poll(&self) -> Option<&PollRow> {
        self.polls.first()
    }

    pub(super) fn options_of(&self, poll_id: i64) -> impl Iterator<Item = &OptionRow> {
        self.options.iter().filter(move |o| o.poll_id == poll_id)
    }

    fn post_polls(&self, post_id: i32) -> impl Iterator<Item = &PollRow> {
        self.polls
            .iter()
            .filter(move |p| p.post_id == i64::from(post_id))
    }

    /// `PollSerializer` for each of the post's polls.
    pub fn serialize(&self, guardian: &Guardian, post_id: i32) -> Vec<Value> {
        self.post_polls(post_id)
            .map(|poll| self.serialize_poll(guardian, poll))
            .collect()
    }

    pub(super) fn serialize_poll(&self, guardian: &Guardian, poll: &PollRow) -> Value {
        let can_see_results = poll.can_see_results(guardian);
        let kind = poll.kind();
        let number_or_multiple = kind == "number" || kind == "multiple";
        let mut p = Map::new();
        p.insert("id".into(), json!(poll.id));
        p.insert("name".into(), json!(poll.name));
        p.insert("type".into(), json!(kind));
        p.insert("status".into(), json!(STATUSES[poll.status as usize]));
        if poll.visibility == 1 {
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
            .options_of(poll.id)
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
        if poll.can_see_voters(guardian) {
            p.insert(
                "preloaded_voters".into(),
                self.voters.get(&poll.id).cloned().unwrap_or(Value::Null),
            );
        }
        p.insert(
            "chart_type".into(),
            json!(CHART_TYPES[poll.chart_type as usize]),
        );
        if let Some(groups) = poll.groups.as_ref().filter(|g| !crate::ruby::is_blank(g)) {
            p.insert("groups".into(), json!(groups));
        }
        p.insert("title".into(), json!(poll.title));
        if kind == "ranked_choice" && can_see_results {
            p.insert(
                "ranked_choice_outcome".into(),
                self.outcomes.get(&poll.id).cloned().unwrap_or(Value::Null),
            );
        }
        p.insert("closed_at".into(), json!(poll.closed_at.map(time_json)));
        p.insert(
            "closed_by".into(),
            poll.closed_by_id
                .and_then(|id| self.closers.get(&id).cloned())
                .unwrap_or(Value::Null),
        );
        Value::Object(p)
    }

    /// The post serializer's `polls` (when the post has any) and
    /// `polls_votes` (when the viewer voted in one: each poll's name with
    /// the viewer's votes).
    pub fn post_keys(&self, guardian: &Guardian, post_id: i32, out: &mut Map<String, Value>) {
        if self.post_polls(post_id).next().is_none() {
            return;
        }
        out.insert(
            "polls".into(),
            Value::Array(self.serialize(guardian, post_id)),
        );
        if guardian.user_id().is_some() && self.post_polls(post_id).any(|p| p.has_voted) {
            let votes: Map<String, Value> = self
                .post_polls(post_id)
                .map(|p| {
                    (
                        p.name.clone(),
                        self.viewer_votes
                            .get(&p.id)
                            .cloned()
                            .unwrap_or_else(|| json!([])),
                    )
                })
                .collect();
            out.insert("polls_votes".into(), Value::Object(votes));
        }
    }
}

/// `BasicUserSerializer` (with `title`, `UserNameSerializer`).
async fn basic_user(
    conn: &mut PgConnection,
    urls: &Urls<'_>,
    user_id: i32,
    with_title: bool,
) -> Result<Value, PluginError> {
    #[derive(sqlx::FromRow)]
    struct UserRow {
        username: String,
        name: Option<String>,
        uploaded_avatar_id: Option<i32>,
        title: Option<String>,
    }
    let user: Option<UserRow> =
        sqlx::query_as("SELECT username, name, uploaded_avatar_id, title FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(&mut *conn)
            .await?;
    let Some(UserRow {
        username,
        name,
        uploaded_avatar_id,
        title,
    }) = user
    else {
        return Ok(Value::Null);
    };
    let mut out = Map::new();
    out.insert("id".into(), json!(user_id));
    out.insert("username".into(), json!(username));
    if urls.settings.get("enable_names")?.truthy() {
        out.insert("name".into(), json!(name));
    }
    out.insert(
        "avatar_template".into(),
        json!(crate::avatar::avatar_template(
            urls,
            user_id,
            &username,
            uploaded_avatar_id,
            None
        )?),
    );
    if with_title {
        out.insert("title".into(), json!(title));
    }
    Ok(Value::Object(out))
}

/// `DiscoursePoll::Poll.serialized_voters(poll, opts)`: a page of each
/// option's voters (UserNameSerializer), in vote order; nil without any.
pub(super) async fn serialized_voters(
    conn: &mut PgConnection,
    urls: &Urls<'_>,
    poll: &PollRow,
    page: i64,
    limit: i64,
    option_digest: Option<&str>,
) -> Result<Value, PluginError> {
    let offset = (page - 1) * limit;
    let votes: Vec<(String, i64, String)> = sqlx::query_as(
        "SELECT digest, user_id, CASE int_rank WHEN 0 THEN 'Abstain' ELSE CAST(int_rank AS text) END AS rank \
         FROM (SELECT po.digest, pv.rank AS int_rank, pv.user_id, \
                      ROW_NUMBER() OVER (PARTITION BY pv.poll_option_id ORDER BY pv.created_at, pv.user_id) AS row \
               FROM poll_votes pv \
               JOIN poll_options po ON pv.poll_id = po.poll_id AND pv.poll_option_id = po.id \
               WHERE pv.poll_id = $1 AND ($4::text IS NULL OR po.digest = $4)) v \
         WHERE row BETWEEN $2 AND $3 \
         ORDER BY digest, int_rank, row",
    )
    .bind(poll.id)
    .bind(offset + 1)
    .bind(offset + limit)
    .bind(option_digest)
    .fetch_all(&mut *conn)
    .await?;
    if votes.is_empty() {
        return Ok(Value::Null);
    }
    let mut users: HashMap<i64, Value> = HashMap::new();
    for (_, user_id, _) in &votes {
        if !users.contains_key(user_id) {
            let user = basic_user(conn, urls, *user_id as i32, true).await?;
            users.insert(*user_id, user);
        }
    }
    let user = |id: &i64| users.get(id).cloned().unwrap_or(Value::Null);
    if poll.kind() == "number" {
        return Ok(Value::Array(
            votes.iter().map(|(_, id, _)| user(id)).collect(),
        ));
    }
    let mut by_digest = Map::new();
    for (digest, user_id, rank) in &votes {
        let entry = by_digest.entry(digest.clone()).or_insert_with(|| json!([]));
        let value = if poll.kind() == "ranked_choice" {
            json!({"rank": rank, "user": user(user_id)})
        } else {
            user(user_id)
        };
        if let Value::Array(list) = entry {
            list.push(value);
        }
    }
    Ok(Value::Object(by_digest))
}

/// The viewer's votes in a poll they voted in: digests, or for ranked
/// choice `{digest, rank}`.
async fn viewer_votes(
    conn: &mut PgConnection,
    poll: &PollRow,
    user_id: i32,
) -> Result<Value, sqlx::Error> {
    let votes: Vec<(String, i32)> = sqlx::query_as(
        "SELECT po.digest, pv.rank FROM poll_votes pv \
         JOIN poll_options po ON po.id = pv.poll_option_id \
         WHERE pv.poll_id = $1 AND pv.user_id = $2 ORDER BY pv.poll_option_id",
    )
    .bind(poll.id)
    .bind(i64::from(user_id))
    .fetch_all(&mut *conn)
    .await?;
    Ok(if poll.kind() == "ranked_choice" {
        Value::Array(
            votes
                .into_iter()
                .map(|(digest, rank)| json!({"digest": digest, "rank": rank}))
                .collect(),
        )
    } else {
        Value::Array(votes.into_iter().map(|(d, _)| json!(d)).collect())
    })
}

/// `DiscoursePoll::RankedChoice::MAX_ROUNDS`
const MAX_ROUNDS: usize = 50;

/// `DiscoursePoll::RankedChoice.outcome(poll_id)`: each voter's ranked
/// options (abstentions left out) run as instant runoff; nil without
/// ballots.
pub(super) async fn ranked_choice_outcome(
    conn: &mut PgConnection,
    poll_id: i64,
) -> Result<Value, sqlx::Error> {
    let options: Vec<(String, String)> =
        sqlx::query_as("SELECT digest, html FROM poll_options WHERE poll_id = $1 ORDER BY id")
            .bind(poll_id)
            .fetch_all(&mut *conn)
            .await?;
    let votes: Vec<(i64, i32, String)> = sqlx::query_as(
        "SELECT pv.user_id, pv.rank, po.digest FROM poll_votes pv \
         JOIN poll_options po ON po.id = pv.poll_option_id \
         WHERE pv.poll_id = $1 ORDER BY pv.user_id, pv.rank",
    )
    .bind(poll_id)
    .fetch_all(&mut *conn)
    .await?;
    let mut ballots: Vec<(i64, Vec<String>)> = Vec::new();
    for (user_id, rank, digest) in votes {
        if ballots.last().is_none_or(|(u, _)| *u != user_id) {
            ballots.push((user_id, Vec::new()));
        }
        if rank > 0
            && let Some((_, paper)) = ballots.last_mut()
        {
            paper.push(digest);
        }
    }
    if ballots.is_empty() {
        return Ok(Value::Null);
    }
    Ok(run(ballots.into_iter().map(|(_, b)| b).collect(), &options))
}

/// `RankedChoice.run`
fn run(mut votes: Vec<Vec<String>>, options: &[(String, String)]) -> Value {
    let enrich = |digest: &str| {
        let html = options
            .iter()
            .find(|(d, _)| d == digest)
            .map(|(_, h)| h.as_str());
        json!({"digest": digest, "html": html})
    };
    let mut round_activity = Vec::new();
    let mut potential_winners: Vec<String> = Vec::new();
    for round in 1..=MAX_ROUNDS {
        // tally_votes: every candidate still on a ballot, in order seen,
        // with its first-place count.
        let mut tally: Vec<(String, i64)> = Vec::new();
        for vote in &votes {
            for candidate in vote {
                if !tally.iter().any(|(c, _)| c == candidate) {
                    tally.push((candidate.clone(), 0));
                }
            }
        }
        for vote in &votes {
            if let Some(first) = vote.first()
                && let Some(entry) = tally.iter_mut().find(|(c, _)| c == first)
            {
                entry.1 += 1;
            }
        }
        let max_votes = tally.iter().map(|(_, n)| *n).max();
        potential_winners = tally
            .iter()
            .filter(|(_, n)| Some(*n) == max_votes)
            .map(|(c, _)| c.clone())
            .collect();
        let total: i64 = tally.iter().map(|(_, n)| n).sum();
        if let Some(max) = max_votes
            && max > total.div_euclid(2)
        {
            let majority = enrich(&potential_winners[0]);
            round_activity
                .push(json!({"round": round, "majority": majority.clone(), "eliminated": null}));
            return json!({
                "tied": false,
                "tied_candidates": null,
                "winner": true,
                "winning_candidate": majority,
                "round_activity": round_activity,
            });
        }
        let min_votes = tally.iter().map(|(_, n)| *n).min();
        let losers: Vec<String> = tally
            .iter()
            .filter(|(_, n)| Some(*n) == min_votes)
            .map(|(c, _)| c.clone())
            .collect();
        for vote in &mut votes {
            vote.retain(|c| !losers.contains(c));
        }
        let losers: Vec<Value> = losers.iter().map(|l| enrich(l)).collect();
        round_activity
            .push(json!({"round": round, "majority": null, "eliminated": losers.clone()}));
        if votes.iter().all(Vec::is_empty) {
            return json!({
                "tied": true,
                "tied_candidates": losers,
                "winner": null,
                "winning_candidate": null,
                "round_activity": round_activity,
            });
        }
    }
    let tied: Vec<Value> = potential_winners.iter().map(|w| enrich(w)).collect();
    json!({
        "tied": true,
        "tied_candidates": tied,
        "winner": null,
        "winning_candidate": null,
        "round_activity": round_activity,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options() -> Vec<(String, String)> {
        ["a", "b", "c"]
            .iter()
            .map(|d| (d.to_string(), d.to_uppercase()))
            .collect()
    }

    fn ballots(papers: &[&[&str]]) -> Vec<Vec<String>> {
        papers
            .iter()
            .map(|p| p.iter().map(|s| s.to_string()).collect())
            .collect()
    }

    #[test]
    fn majority_in_the_first_round() {
        let out = run(ballots(&[&["a", "b"], &["a"], &["b"]]), &options());
        assert_eq!(out["winner"], json!(true));
        assert_eq!(out["winning_candidate"]["digest"], json!("a"));
        assert_eq!(out["round_activity"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn eliminates_the_least_then_wins() {
        let out = run(
            ballots(&[&["a"], &["a"], &["b"], &["b"], &["c", "a"]]),
            &options(),
        );
        assert_eq!(
            out["round_activity"][0]["eliminated"][0]["digest"],
            json!("c")
        );
        assert_eq!(out["winning_candidate"]["digest"], json!("a"));
    }

    #[test]
    fn a_tie_eliminates_everyone() {
        let out = run(ballots(&[&["a"], &["b"]]), &options());
        assert_eq!(out["tied"], json!(true));
        assert_eq!(out["tied_candidates"].as_array().unwrap().len(), 2);
    }
}
