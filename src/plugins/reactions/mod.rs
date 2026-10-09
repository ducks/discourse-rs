//! discourse-reactions, read side: the keys it adds to every serialized
//! post (`reactions`, `current_user_reaction`, `reaction_users_count`,
//! `current_user_used_main_reaction`) and to the topic view
//! (`valid_reactions`), on the tables Rails' plugin created
//! (`discourse_reactions_reactions`, `discourse_reactions_reaction_users`),
//! as ReactionsSerializerHelpers.preload_post_reactions computes them.
//!
//! The model behind it: the main reaction (discourse_reactions_reaction_for_
//! like) is a plain like (a PostAction); a reaction excluded from like is a
//! ReactionUser only; any other reaction is both. A post's reactions are its
//! emoji reactions with users, and its likes that aren't a reaction's shadow
//! counted under the main reaction.

use std::collections::{HashMap, HashSet};

use chrono::NaiveDateTime;
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::site_settings::{SettingError, SiteSettings};

/// `PostActionType::LIKE_POST_ACTION_ID`
const LIKE: i32 = 2;

/// `enabled_site_setting :discourse_reactions_enabled`
pub fn enabled(settings: &SiteSettings) -> Result<bool, SettingError> {
    Ok(settings.get("discourse_reactions_enabled")?.truthy())
}

/// The reaction settings, as DiscourseReactions::Reaction reads them.
pub struct Reactions {
    /// `main_reaction_id`
    pub main: String,
    /// `valid_reactions`: the main one, then the enabled ones, once each.
    pub valid: Vec<String>,
    /// `reactions_excluded_from_like`
    pub excluded: Vec<String>,
    pub allow_any_emoji: bool,
    pub undo_window_mins: i64,
}

impl Reactions {
    pub fn load(settings: &SiteSettings) -> Result<Self, SettingError> {
        let main = settings
            .get("discourse_reactions_reaction_for_like")?
            .to_s()
            .replace('-', "");
        let list = |name: &str| -> Result<Vec<String>, SettingError> {
            Ok(settings
                .get(name)?
                .to_s()
                .split('|')
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect())
        };
        let mut valid = vec![main.clone()];
        for r in list("discourse_reactions_enabled_reactions")? {
            if !valid.contains(&r) {
                valid.push(r);
            }
        }
        Ok(Reactions {
            main,
            valid,
            excluded: list("discourse_reactions_excluded_from_like")?,
            allow_any_emoji: settings
                .get("discourse_reactions_allow_any_emoji")?
                .truthy(),
            undo_window_mins: settings.get("post_undo_action_window_mins")?.to_i(),
        })
    }

    /// `reactions_counting_as_like`
    fn counts_as_like(&self, value: &str) -> bool {
        self.valid.iter().any(|v| v == value)
            && !self.excluded.iter().any(|e| e == value)
            && value != self.main
    }

    /// The topic view's `valid_reactions`.
    pub fn valid_json(&self) -> Value {
        json!(self.valid)
    }
}

/// `Emoji.exists?`: a standard emoji or alias, or a custom one; `:name:`
/// and a `:tN` skin tone are accepted around the name.
fn emoji_exists(name: &str, custom: &HashSet<String>) -> bool {
    let name = name.trim_matches(':');
    let name = match name.rsplit_once(":t") {
        Some((base, tone))
            if tone.len() == 1 && ('1'..='6').contains(&tone.chars().next().unwrap_or('0')) =>
        {
            base
        }
        _ => name,
    };
    discourse_markdown::emoji::DATA.exists(name) || custom.contains(name)
}

#[derive(Debug, sqlx::FromRow)]
struct ReactionRow {
    post_id: i32,
    reaction_value: Option<String>,
    reaction_users_count: Option<i32>,
    /// Its users other than the viewer's ignored ones.
    unignored: i64,
    /// The viewer's ReactionUser on it, if any: when.
    viewer_at: Option<NaiveDateTime>,
}

/// One post's keys.
#[derive(Debug, Default)]
struct PostKeys {
    reactions: Vec<Value>,
    reaction_users_count: i64,
    current_user_reaction: Value,
    current_user_used_main_reaction: bool,
}

/// The posts' reaction keys, loaded for a page of posts and a viewer.
#[derive(Debug, Default)]
pub struct PostsData {
    posts: HashMap<i32, PostKeys>,
}

/// What `current_user_reaction` needs to know about the viewer's like on a
/// post: whether the guardian lets them undo it.
pub struct ViewerLike {
    pub can_undo: bool,
}

impl PostsData {
    /// `preload_post_reactions(posts, user)`; `likes` is the viewer's
    /// undeleted like per post, with `can_delete_post_action?` already
    /// answered by the caller (it holds the topic).
    pub async fn load(
        conn: &mut PgConnection,
        settings: &Reactions,
        post_ids: &[i32],
        viewer: Option<i32>,
        likes: &HashMap<i32, ViewerLike>,
    ) -> Result<Self, sqlx::Error> {
        let viewer_id = viewer.unwrap_or(-1);
        // user.ignored_user_ids
        let ignored: Vec<i32> = match viewer {
            Some(id) => {
                sqlx::query_scalar("SELECT ignored_user_id FROM ignored_users WHERE user_id = $1")
                    .bind(id)
                    .fetch_all(&mut *conn)
                    .await?
            }
            None => Vec::new(),
        };
        let custom: HashSet<String> = sqlx::query_scalar("SELECT name FROM custom_emojis")
            .fetch_all(&mut *conn)
            .await?
            .into_iter()
            .collect();
        // The posts' reactions, by id as the association loads them, with
        // their users not ignored and the viewer's own.
        let rows: Vec<ReactionRow> = sqlx::query_as(
            "SELECT r.post_id, r.reaction_value, r.reaction_users_count, \
                    (SELECT count(*) FROM discourse_reactions_reaction_users ru \
                     WHERE ru.reaction_id = r.id AND ru.user_id <> ALL($2)) AS unignored, \
                    (SELECT ru.created_at FROM discourse_reactions_reaction_users ru \
                     WHERE ru.reaction_id = r.id AND ru.user_id = $3 LIMIT 1) AS viewer_at \
             FROM discourse_reactions_reactions r WHERE r.post_id = ANY($1) ORDER BY r.id",
        )
        .bind(post_ids)
        .bind(&ignored)
        .bind(viewer_id)
        .fetch_all(&mut *conn)
        .await?;
        // TopicViewSerializer.posts_reaction_users_count
        let counts: Vec<(i32, i64)> = sqlx::query_as(
            "SELECT post_id, COUNT(DISTINCT user_id) FROM ( \
                SELECT user_id, post_id FROM post_actions \
                 WHERE post_id = ANY($1) AND post_action_type_id = $2 AND deleted_at IS NULL \
                   AND user_id <> ALL($3) \
              UNION ALL \
                SELECT ru.user_id, posts.id FROM posts \
                  LEFT JOIN discourse_reactions_reactions r ON r.post_id = posts.id \
                  LEFT JOIN discourse_reactions_reaction_users ru ON ru.reaction_id = r.id \
                 WHERE posts.id = ANY($1) AND (ru.user_id IS NULL OR ru.user_id <> ALL($3)) \
             ) AS u WHERE post_id IS NOT NULL GROUP BY post_id",
        )
        .bind(post_ids)
        .bind(LIKE)
        .bind(&ignored)
        .fetch_all(&mut *conn)
        .await?;
        // Likes that aren't a reaction's shadow: not by a user whose
        // reaction on the post counts as a like, nor one with the main
        // reaction.
        let likes_counts: Vec<(i32, i64)> = sqlx::query_as(
            "SELECT pa.post_id, COUNT(*) FROM post_actions pa \
             WHERE pa.deleted_at IS NULL AND pa.post_id = ANY($1) AND pa.post_action_type_id = $2 \
               AND pa.user_id <> ALL($5) \
               AND NOT EXISTS (SELECT 1 FROM discourse_reactions_reaction_users dru \
                   JOIN discourse_reactions_reactions dr ON dr.id = dru.reaction_id \
                   WHERE dr.post_id = pa.post_id AND dru.user_id = pa.user_id \
                     AND dr.reaction_value <> $3 AND dr.reaction_value <> ALL($4)) \
               AND NOT EXISTS (SELECT 1 FROM discourse_reactions_reaction_users dru \
                   JOIN discourse_reactions_reactions dr ON dr.id = dru.reaction_id \
                   WHERE dr.post_id = pa.post_id AND dru.user_id = pa.user_id \
                     AND dr.reaction_value = $3) \
             GROUP BY pa.post_id",
        )
        .bind(post_ids)
        .bind(LIKE)
        .bind(&settings.main)
        .bind(&settings.excluded)
        .bind(&ignored)
        .fetch_all(&mut *conn)
        .await?;
        let counts: HashMap<i32, i64> = counts.into_iter().collect();
        let likes_counts: HashMap<i32, i64> = likes_counts.into_iter().collect();
        let now = crate::clock::now_naive();

        let mut out = PostsData::default();
        for &post_id in post_ids {
            let emoji: Vec<&ReactionRow> = rows
                .iter()
                .filter(|r| r.post_id == post_id)
                .filter(|r| {
                    r.reaction_value
                        .as_deref()
                        .is_some_and(|v| emoji_exists(v, &custom))
                })
                .collect();
            let mut reactions: Vec<(String, i64)> = emoji
                .iter()
                .filter(|r| r.reaction_users_count.unwrap_or(0) > 0)
                .filter_map(|r| {
                    let count = if ignored.is_empty() {
                        i64::from(r.reaction_users_count.unwrap_or(0))
                    } else {
                        r.unignored
                    };
                    (count != 0).then(|| (r.reaction_value.clone().unwrap_or_default(), count))
                })
                .collect();
            let like_count = likes_counts.get(&post_id).copied().unwrap_or(0);
            if like_count > 0 {
                let (main, mut rest): (Vec<_>, Vec<_>) = reactions
                    .into_iter()
                    .partition(|(id, _)| *id == settings.main);
                rest.push((
                    settings.main.clone(),
                    like_count + main.iter().map(|(_, c)| c).sum::<i64>(),
                ));
                reactions = rest;
            }
            reactions.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

            // current_user_reaction: the viewer's reaction, else their like.
            let mut current = Value::Null;
            if viewer.is_some() {
                if let Some((r, at)) = emoji
                    .iter()
                    .filter(|r| r.reaction_users_count.is_some())
                    .find_map(|r| r.viewer_at.map(|at| (r, at)))
                {
                    current = json!({
                        "id": r.reaction_value,
                        "type": "emoji",
                        "can_undo": at > now - chrono::Duration::minutes(settings.undo_window_mins),
                    });
                } else if let Some(like) = likes.get(&post_id) {
                    current =
                        json!({ "id": settings.main, "type": "emoji", "can_undo": like.can_undo });
                }
            }
            // current_user_used_main_reaction: a like that isn't the shadow
            // of one of their reactions.
            let used_main = viewer.is_some()
                && likes.contains_key(&post_id)
                && !emoji.iter().any(|r| {
                    r.viewer_at.is_some()
                        && r.reaction_value.as_deref().is_some_and(|v| {
                            if settings.allow_any_emoji {
                                v != settings.main
                            } else {
                                settings.counts_as_like(v)
                            }
                        })
                });
            out.posts.insert(
                post_id,
                PostKeys {
                    reactions: reactions
                        .into_iter()
                        .map(|(id, count)| json!({ "id": id, "type": "emoji", "count": count }))
                        .collect(),
                    reaction_users_count: counts.get(&post_id).copied().unwrap_or(0),
                    current_user_reaction: current,
                    current_user_used_main_reaction: used_main,
                },
            );
        }
        Ok(out)
    }

    /// The post's keys, in the order the plugin adds them.
    pub fn post_keys(&self, post_id: i32, out: &mut Map<String, Value>) {
        let keys = self.posts.get(&post_id);
        out.insert(
            "reactions".into(),
            json!(keys.map(|k| k.reactions.clone()).unwrap_or_default()),
        );
        out.insert(
            "current_user_reaction".into(),
            keys.map(|k| k.current_user_reaction.clone())
                .unwrap_or(Value::Null),
        );
        out.insert(
            "reaction_users_count".into(),
            json!(keys.map(|k| k.reaction_users_count).unwrap_or(0)),
        );
        out.insert(
            "current_user_used_main_reaction".into(),
            json!(keys.is_some_and(|k| k.current_user_used_main_reaction)),
        );
    }
}

/// The user action stream's join and filter (the plugin's
/// user_action_stream_builder modifier): a like that is a reaction's
/// shadow isn't listed.
pub const STREAM_JOIN: &str = " LEFT JOIN discourse_reactions_reaction_users \
    ON discourse_reactions_reaction_users.post_id = a.target_post_id \
    AND discourse_reactions_reaction_users.user_id = a.acting_user_id";
pub const STREAM_WHERE: &str = " AND (discourse_reactions_reaction_users.id IS NULL)";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emoji_names_with_colons_and_tones_resolve() {
        let custom = HashSet::from(["partyparrot".to_string()]);
        assert!(emoji_exists("heart", &custom));
        assert!(emoji_exists(":+1:", &custom));
        assert!(emoji_exists("clap:t3", &custom));
        assert!(emoji_exists("partyparrot", &custom));
        assert!(!emoji_exists("not_an_emoji_at_all", &custom));
    }
}
