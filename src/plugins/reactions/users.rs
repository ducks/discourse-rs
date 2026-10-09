//! discourse-reactions' CustomReactionsController reads: who reacted to a
//! post (`post_reactions_users`, by reaction, and `reactions_users_list`,
//! one list through PostReactionsQuery), and a user's reactions given and
//! received (UserReactionSerializer, with the post as GroupPostSerializer
//! has it).

use std::collections::HashMap;

use chrono::NaiveDateTime;
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use super::{LIKE, Reactions};
use crate::guardian::Guardian;
use crate::posting::revisions::PostAccess;
use crate::site_settings::SiteSettings;
use crate::topic_list::time_json;
use crate::url::Urls;
use crate::{AppError, Unsupported, avatar};

/// `MAX_USERS_COUNT`
const MAX_USERS_COUNT: i64 = 26;
/// `PAGE_SIZE`
const PAGE_SIZE: i64 = 20;

/// Ruby's `Time#to_s` for a UTC time.
fn ruby_time(t: NaiveDateTime) -> String {
    t.format("%Y-%m-%d %H:%M:%S UTC").to_string()
}

/// `PostReactionsQuery.apply_ignored_users_filter` on `column`, for the
/// viewer bound as `$<param>`; nothing for anonymous viewers.
fn ignored_filter(viewer: Option<i32>, column: &str, param: usize) -> String {
    match viewer {
        None => String::new(),
        Some(_) => format!(
            " AND NOT EXISTS (SELECT 1 FROM ignored_users ig WHERE ig.user_id = ${param} \
               AND ig.ignored_user_id = {column} AND ig.ignored_user_id <> ${param})"
        ),
    }
}

pub struct Reader<'a> {
    pub conn: &'a mut PgConnection,
    pub settings: &'a SiteSettings,
    pub urls: &'a Urls<'a>,
    pub guardian: &'a Guardian,
}

#[derive(sqlx::FromRow)]
struct Reactor {
    user_id: i32,
    username: String,
    name: Option<String>,
    uploaded_avatar_id: Option<i32>,
    created_at: NaiveDateTime,
}

impl Reader<'_> {
    fn enable_names(&self) -> Result<bool, AppError> {
        Ok(self.settings.get("enable_names")?.truthy())
    }

    /// `format_user(user, avatar_template:, **extra)`
    fn format_user(
        &self,
        username: &str,
        name: Option<&str>,
        avatar_template: String,
    ) -> Result<Map<String, Value>, AppError> {
        let mut user = Map::new();
        user.insert("username".into(), json!(username));
        if self.enable_names()? {
            user.insert("name".into(), json!(name));
        }
        user.insert("avatar_template".into(), json!(avatar_template));
        Ok(user)
    }

    /// `User#avatar_template`
    async fn user_avatar(
        &mut self,
        id: i32,
        username: &str,
        uploaded_avatar_id: Option<i32>,
    ) -> Result<String, AppError> {
        let logo = if id == avatar::SYSTEM_USER_ID {
            crate::admin_users::logo_small_url(&mut *self.conn, self.settings).await?
        } else {
            None
        };
        Ok(avatar::avatar_template(
            self.urls,
            id,
            username,
            uploaded_avatar_id,
            logo.as_deref(),
        )?)
    }

    /// GET /discourse-reactions/posts/:id/reactions-users: the post's
    /// reactions with their users, likes that aren't a reaction's shadow
    /// under the main reaction.
    pub async fn post_reactions_users(
        &mut self,
        post: &PostAccess,
        reaction_value: Option<&str>,
    ) -> Result<Value, AppError> {
        let reactions = Reactions::load(self.settings)?;
        let viewer = self.guardian.user_id();
        let post_id = post.post.id;
        let main = reactions.main.as_str();

        let mut likes: Option<Vec<(i32, Reactor)>> = None;
        let mut main_reaction: Option<(i64, Option<i32>)> = None;
        if reaction_value.is_none_or(|v| v == main) {
            // filter_reaction_likes_sql, less the likes of users whose
            // reaction is no longer a valid one (historical_reaction_likes).
            let sql = format!(
                "SELECT pa.id, pa.user_id, u.username, u.name, u.uploaded_avatar_id, pa.created_at \
                 FROM post_actions pa JOIN users u ON u.id = pa.user_id \
                 WHERE pa.post_id = $1 AND pa.post_action_type_id = $2 AND pa.deleted_at IS NULL \
                   AND NOT EXISTS (SELECT 1 FROM discourse_reactions_reaction_users ru \
                       JOIN discourse_reactions_reactions r ON r.id = ru.reaction_id \
                       WHERE ru.user_id = pa.user_id AND ru.post_id = pa.post_id \
                         AND r.reaction_value = ANY($3)) \
                   AND NOT EXISTS (SELECT 1 FROM discourse_reactions_reaction_users ru \
                       JOIN discourse_reactions_reactions r ON r.id = ru.reaction_id \
                       WHERE ru.user_id = pa.user_id AND ru.post_id = pa.post_id \
                         AND NOT (r.reaction_value = ANY($3))) \
                   {} \
                 ORDER BY pa.id",
                ignored_filter(viewer, "pa.user_id", 4)
            );
            #[derive(sqlx::FromRow)]
            struct LikeRow {
                id: i32,
                #[sqlx(flatten)]
                reactor: Reactor,
            }
            let rows: Vec<LikeRow> = sqlx::query_as(&sql)
                .bind(post_id)
                .bind(LIKE)
                .bind(&reactions.valid)
                .bind(viewer)
                .fetch_all(&mut *self.conn)
                .await?;
            likes = Some(rows.into_iter().map(|r| (r.id, r.reactor)).collect());
            main_reaction = sqlx::query_as(
                "SELECT id, reaction_users_count FROM discourse_reactions_reactions \
                 WHERE reaction_value = $1 AND post_id = $2 ORDER BY id LIMIT 1",
            )
            .bind(main)
            .bind(post_id)
            .fetch_optional(&mut *self.conn)
            .await?;
        }

        // The post's reactions with a count (zero counts too), the main
        // one aside.
        let listed: Vec<(i64, String, i32)> = match reaction_value {
            Some(v) if v == main => Vec::new(),
            Some(v) => {
                sqlx::query_as(
                    "SELECT id, reaction_value, reaction_users_count FROM discourse_reactions_reactions \
                     WHERE post_id = $1 AND reaction_value = $2 AND reaction_users_count IS NOT NULL ORDER BY id",
                )
                .bind(post_id)
                .bind(v)
                .fetch_all(&mut *self.conn)
                .await?
            }
            None => {
                sqlx::query_as(
                    "SELECT id, reaction_value, reaction_users_count FROM discourse_reactions_reactions \
                     WHERE post_id = $1 AND reaction_users_count IS NOT NULL AND reaction_value <> $2 ORDER BY id",
                )
                .bind(post_id)
                .bind(main)
                .fetch_all(&mut *self.conn)
                .await?
            }
        };
        let mut for_counts: Vec<(i64, i32)> = listed.iter().map(|(id, _, c)| (*id, *c)).collect();
        if let Some((id, Some(count))) = main_reaction {
            for_counts.push((id, count));
        }
        let counts = self.filtered_counts(&for_counts).await?;

        let mut out = Vec::new();
        if let Some(likes) = likes.filter(|l| !l.is_empty()) {
            let mut count = likes.len() as i64;
            let mut users = Vec::new();
            for (_, like) in likes.iter().take(MAX_USERS_COUNT as usize + 1) {
                // can_delete_post_action?(like)
                let can_undo = self.guardian.is_authenticated()
                    && self.guardian.can_delete_post_action(
                        self.settings,
                        &post.topic,
                        like.user_id,
                        like.created_at,
                    )?;
                users.push(self.reactor_json(like, can_undo).await?);
            }
            if let Some((id, Some(_))) = main_reaction {
                users.extend(self.reaction_users(id, &reactions).await?);
                users.sort_by(|a, b| a["created_at"].as_str().cmp(&b["created_at"].as_str()));
                count += counts.get(&id).copied().unwrap_or(0);
            }
            users.reverse();
            users.truncate(MAX_USERS_COUNT as usize + 1);
            out.push(json!({"id": main, "count": count, "users": users}));
        }
        for (id, value, _) in &listed {
            let users = self.reaction_users(*id, &reactions).await?;
            out.push(json!({
                "id": value,
                "count": counts.get(id).copied().unwrap_or(0),
                "users": users,
            }));
        }
        Ok(json!({ "reaction_users": out }))
    }

    async fn reactor_json(&mut self, r: &Reactor, can_undo: bool) -> Result<Value, AppError> {
        let avatar = self
            .user_avatar(r.user_id, &r.username, r.uploaded_avatar_id)
            .await?;
        let mut user = self.format_user(&r.username, r.name.as_deref(), avatar)?;
        user.insert("can_undo".into(), json!(can_undo));
        user.insert("created_at".into(), json!(ruby_time(r.created_at)));
        Ok(Value::Object(user))
    }

    /// `filtered_reaction_users_counts`: the stored counts, or for a
    /// viewer ignoring someone, the reaction users counted without them.
    async fn filtered_counts(
        &mut self,
        reactions: &[(i64, i32)],
    ) -> Result<HashMap<i64, i64>, AppError> {
        if reactions.is_empty() {
            return Ok(HashMap::new());
        }
        let ignores: bool = match self.guardian.user_id() {
            None => false,
            Some(id) => {
                sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM ignored_users WHERE user_id = $1)")
                    .bind(id)
                    .fetch_one(&mut *self.conn)
                    .await?
            }
        };
        if !ignores {
            return Ok(reactions.iter().map(|(id, c)| (*id, *c as i64)).collect());
        }
        let ids: Vec<i64> = reactions.iter().map(|(id, _)| *id).collect();
        let sql = format!(
            "SELECT ru.reaction_id, COUNT(*) FROM discourse_reactions_reaction_users ru \
             WHERE ru.reaction_id = ANY($1) {} GROUP BY ru.reaction_id",
            ignored_filter(self.guardian.user_id(), "ru.user_id", 2)
        );
        let rows: Vec<(i64, i64)> = sqlx::query_as(&sql)
            .bind(&ids)
            .bind(self.guardian.user_id())
            .fetch_all(&mut *self.conn)
            .await?;
        Ok(rows.into_iter().collect())
    }

    /// `get_users(reaction)`: its newest users, each with whether the
    /// reaction can still be undone.
    async fn reaction_users(
        &mut self,
        reaction_id: i64,
        reactions: &Reactions,
    ) -> Result<Vec<Value>, AppError> {
        let sql = format!(
            "SELECT ru.user_id, u.username, u.name, u.uploaded_avatar_id, ru.created_at \
             FROM discourse_reactions_reaction_users ru JOIN users u ON u.id = ru.user_id \
             WHERE ru.reaction_id = $1 {} ORDER BY ru.created_at DESC LIMIT $2",
            ignored_filter(self.guardian.user_id(), "ru.user_id", 3)
        );
        let rows: Vec<Reactor> = sqlx::query_as(&sql)
            .bind(reaction_id)
            .bind(MAX_USERS_COUNT + 1)
            .bind(self.guardian.user_id())
            .fetch_all(&mut *self.conn)
            .await?;
        let now = crate::clock::now_naive();
        let mut out = Vec::new();
        for r in &rows {
            // ReactionUser#can_undo?
            let can_undo =
                r.created_at > now - chrono::Duration::minutes(reactions.undo_window_mins);
            out.push(self.reactor_json(r, can_undo).await?);
        }
        Ok(out)
    }

    /// GET /discourse-reactions/posts/:id/reactions-users-list:
    /// PostReactionsQuery's page of reactors, oldest first, and its total.
    pub async fn reactions_users_list(
        &mut self,
        post_id: i32,
        filter: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<Value, AppError> {
        let reactions = Reactions::load(self.settings)?;
        let main = reactions.main.as_str();
        let viewer = self.guardian.user_id();
        let filter = filter.filter(|f| !f.trim().is_empty());
        let specific = filter.is_some_and(|f| f != main);
        let main_only = filter == Some(main);
        // $1 post, $2 like, $3 main, $4 limit, $5 offset, $6 viewer, $7 filter
        let reactions_select = format!(
            "SELECT u.id, u.username, u.name, u.uploaded_avatar_id, \
                    dr.reaction_value AS reaction, drru.created_at \
             FROM discourse_reactions_reaction_users drru \
             INNER JOIN discourse_reactions_reactions dr ON dr.id = drru.reaction_id \
             INNER JOIN users u ON u.id = drru.user_id \
             WHERE drru.post_id = $1 {}",
            ignored_filter(viewer, "drru.user_id", 6)
        );
        // strict_filter_reaction_likes_sql
        let shadow_filter = "post_actions.post_action_type_id = $2 AND post_actions.deleted_at IS NULL \
             AND NOT EXISTS (SELECT 1 FROM discourse_reactions_reaction_users \
                 WHERE discourse_reactions_reaction_users.user_id = post_actions.user_id \
                   AND discourse_reactions_reaction_users.post_id = post_actions.post_id)";
        let likes_select = format!(
            "SELECT u.id, u.username, u.name, u.uploaded_avatar_id, \
                    $3::text AS reaction, post_actions.created_at \
             FROM post_actions INNER JOIN users u ON u.id = post_actions.user_id \
             WHERE post_actions.post_id = $1 AND ({shadow_filter}) {}",
            ignored_filter(viewer, "post_actions.user_id", 6)
        );
        let sql = if specific {
            format!(
                "{reactions_select} AND dr.reaction_value = $7 \
                 ORDER BY drru.created_at ASC LIMIT $4 OFFSET $5"
            )
        } else if main_only {
            format!("{likes_select} ORDER BY post_actions.created_at ASC LIMIT $4 OFFSET $5")
        } else {
            format!(
                "SELECT * FROM ({reactions_select} UNION ALL {likes_select}) combined \
                 ORDER BY created_at ASC LIMIT $4 OFFSET $5"
            )
        };
        #[derive(sqlx::FromRow)]
        struct Row {
            id: i32,
            username: String,
            name: Option<String>,
            uploaded_avatar_id: Option<i32>,
            reaction: Option<String>,
        }
        let rows: Vec<Row> = sqlx::query_as(&sql)
            .bind(post_id)
            .bind(LIKE)
            .bind(main)
            .bind(limit)
            .bind(offset)
            .bind(viewer)
            .bind(filter)
            .fetch_all(&mut *self.conn)
            .await?;

        // total: $1 post, $2 the reaction (or like type), $3 viewer.
        let ru_count = format!(
            "SELECT COUNT(*) FROM discourse_reactions_reaction_users drru \
             JOIN discourse_reactions_reactions dr ON dr.id = drru.reaction_id \
             WHERE drru.post_id = $1 AND ($2::text IS NULL OR dr.reaction_value = $2) {}",
            ignored_filter(viewer, "drru.user_id", 3)
        );
        let likes_count = format!(
            "SELECT COUNT(*) FROM post_actions WHERE post_actions.post_id = $1 AND ({shadow_filter}) {}",
            ignored_filter(viewer, "post_actions.user_id", 3)
        );
        let reaction_users: i64 = if main_only {
            0
        } else {
            sqlx::query_scalar(&ru_count)
                .bind(post_id)
                .bind(filter.filter(|_| specific))
                .bind(viewer)
                .fetch_one(&mut *self.conn)
                .await?
        };
        let plain_likes: i64 = if specific {
            0
        } else {
            sqlx::query_scalar(&likes_count)
                .bind(post_id)
                .bind(LIKE)
                .bind(viewer)
                .fetch_one(&mut *self.conn)
                .await?
        };
        let total = reaction_users + plain_likes;

        let mut users = Vec::new();
        for r in &rows {
            let avatar =
                avatar::class_avatar_template(self.urls, &r.username, r.uploaded_avatar_id)?;
            let mut user = self.format_user(&r.username, r.name.as_deref(), avatar)?;
            user.insert("id".into(), json!(r.id));
            user.insert("reaction".into(), json!(r.reaction));
            users.push(Value::Object(user));
        }
        Ok(json!({"users": users, "total_rows": total}))
    }
}

/// A row UserReactionSerializer renders: a ReactionUser, or a like
/// dressed as one (`translate_to_reactions`).
struct UserReaction {
    id: i64,
    user_id: i32,
    post_id: i32,
    created_at: NaiveDateTime,
    reaction_id: i64,
    reaction_value: String,
    reaction_users_count: Option<i32>,
    reaction_created_at: NaiveDateTime,
}

#[derive(sqlx::FromRow)]
struct ReactionUserRow {
    id: i64,
    user_id: i32,
    post_id: i32,
    created_at: NaiveDateTime,
    reaction_id: i64,
    reaction_value: Option<String>,
    reaction_users_count: Option<i32>,
    reaction_created_at: NaiveDateTime,
}

impl From<ReactionUserRow> for UserReaction {
    fn from(r: ReactionUserRow) -> Self {
        UserReaction {
            id: r.id,
            user_id: r.user_id,
            post_id: r.post_id,
            created_at: r.created_at,
            reaction_id: r.reaction_id,
            reaction_value: r.reaction_value.unwrap_or_default(),
            reaction_users_count: r.reaction_users_count,
            reaction_created_at: r.reaction_created_at,
        }
    }
}

const REACTION_USER_COLUMNS: &str = "ru.id, ru.user_id, ru.post_id, ru.created_at, r.id AS reaction_id, \
     r.reaction_value, r.reaction_users_count, r.created_at AS reaction_created_at";

/// What `reactions_received` reads from the request.
pub struct ReceivedParams<'a> {
    pub before_reaction_user_id: Option<i64>,
    pub before_like_id: Option<i64>,
    pub acting_username: Option<&'a str>,
    pub include_likes: bool,
}

impl Reader<'_> {
    /// GET /discourse-reactions/posts/reactions: the user's reactions, as
    /// the viewer may see their posts (UserAction.apply_common_filters for
    /// the viewer).
    pub async fn reactions_given(
        &mut self,
        user_id: i32,
        before_reaction_user_id: Option<i64>,
    ) -> Result<Value, AppError> {
        let viewer = self.guardian.user_id().unwrap_or(-2);
        let mut sql = format!(
            "SELECT {REACTION_USER_COLUMNS} FROM discourse_reactions_reaction_users ru \
             INNER JOIN discourse_reactions_reactions r ON r.id = ru.reaction_id \
             INNER JOIN posts p ON p.id = ru.post_id AND p.deleted_at IS NULL \
             INNER JOIN topics t ON t.id = p.topic_id AND t.deleted_at IS NULL \
             INNER JOIN posts p2 ON p2.topic_id = t.id AND p2.post_number = 1 AND p.deleted_at IS NULL \
             LEFT JOIN categories c ON c.id = t.category_id \
             WHERE ru.user_id = $1 AND r.reaction_users_count IS NOT NULL AND t.deleted_at IS NULL"
        );
        if !self.guardian.can_see_deleted_posts() {
            sql.push_str(
                " AND p.deleted_at IS NULL AND p2.deleted_at IS NULL \
                 AND (NOT COALESCE(p.hidden, p2.hidden, false) \
                      OR CASE WHEN p.id IS NULL THEN p2.user_id ELSE p.user_id END = $2)",
            );
        }
        let visible = self.guardian.visible_post_types(self.settings)?;
        sql.push_str(&format!(
            " AND COALESCE(p.post_type, p2.post_type) IN ({})",
            visible
                .iter()
                .map(i32::to_string)
                .collect::<Vec<_>>()
                .join(",")
        ));
        // The filters run for the viewer's own id: t.visible never applies,
        // and messages are theirs to see when they take part.
        if !self.guardian.is_admin() {
            sql.push_str(
                " AND (t.archetype <> 'private_message' \
                   OR EXISTS (SELECT 1 FROM topic_allowed_users tu WHERE tu.topic_id = t.id AND tu.user_id = $2) \
                   OR EXISTS (SELECT 1 FROM topic_allowed_groups tg WHERE tg.topic_id = t.id AND tg.group_id IN ( \
                        SELECT group_id FROM group_users gu WHERE gu.user_id = $2)))",
            );
            let secure = self
                .guardian
                .secure_category_ids(&mut *self.conn, self.settings)
                .await?;
            if secure.is_empty() {
                sql.push_str(" AND (c.read_restricted IS NULL OR NOT c.read_restricted)");
            } else {
                sql.push_str(&format!(
                    " AND (c.read_restricted IS NULL OR NOT c.read_restricted \
                       OR (c.read_restricted AND c.id IN ({})))",
                    secure
                        .iter()
                        .map(i32::to_string)
                        .collect::<Vec<_>>()
                        .join(",")
                ));
            }
        }
        sql.push_str(
            " AND NOT EXISTS (SELECT 1 FROM ignored_users ig INNER JOIN users iu ON iu.id = ig.ignored_user_id \
               WHERE ig.user_id = $2 AND ig.ignored_user_id = ru.user_id AND ig.ignored_user_id <> $2)",
        );
        if before_reaction_user_id.is_some() {
            sql.push_str(" AND ru.id < $3");
        }
        sql.push_str(" ORDER BY ru.created_at DESC LIMIT $4");
        let rows: Vec<ReactionUserRow> = sqlx::query_as(&sql)
            .bind(user_id)
            .bind(viewer)
            .bind(before_reaction_user_id)
            .bind(PAGE_SIZE)
            .fetch_all(&mut *self.conn)
            .await?;
        let rows: Vec<UserReaction> = rows.into_iter().map(UserReaction::from).collect();
        self.serialize(&rows).await
    }

    /// GET /discourse-reactions/posts/reactions-received: reactions on the
    /// user's posts the viewer can see, and with `include_likes` the likes
    /// that aren't a reaction's shadow.
    pub async fn reactions_received(
        &mut self,
        user_id: i32,
        params: &ReceivedParams<'_>,
    ) -> Result<Value, AppError> {
        let viewer = self
            .guardian
            .user_id()
            .ok_or(Unsupported("reactions received, anonymously"))?;
        let post_ids = self.visible_post_ids(user_id).await?;
        let mut sql = format!(
            "SELECT {REACTION_USER_COLUMNS} FROM discourse_reactions_reaction_users ru \
             INNER JOIN discourse_reactions_reactions r ON r.id = ru.reaction_id \
             WHERE ru.post_id = ANY($1) AND r.reaction_users_count IS NOT NULL {}",
            ignored_filter(Some(viewer), "ru.user_id", 2)
        );
        if params.before_reaction_user_id.is_some() {
            sql.push_str(" AND ru.id < $3");
        }
        if params.acting_username.is_some() {
            sql.push_str(" AND ru.user_id IN (SELECT id FROM users WHERE username = $4)");
        }
        sql.push_str(" ORDER BY ru.created_at DESC LIMIT $5");
        let rows: Vec<ReactionUserRow> = sqlx::query_as(&sql)
            .bind(&post_ids)
            .bind(viewer)
            .bind(params.before_reaction_user_id)
            .bind(params.acting_username)
            .bind(PAGE_SIZE)
            .fetch_all(&mut *self.conn)
            .await?;
        let mut rows: Vec<UserReaction> = rows.into_iter().map(UserReaction::from).collect();

        if params.include_likes {
            let mut sql = format!(
                "SELECT pa.id, pa.user_id, pa.post_id, pa.created_at FROM post_actions pa \
                 LEFT JOIN discourse_reactions_reaction_users ru \
                   ON ru.post_id = pa.post_id AND ru.user_id = pa.user_id \
                 WHERE pa.post_id = ANY($1) AND pa.deleted_at IS NULL AND pa.post_action_type_id = $6 \
                   AND ru.id IS NULL {}",
                ignored_filter(Some(viewer), "pa.user_id", 2)
            );
            if params.before_like_id.is_some() {
                sql.push_str(" AND pa.id < $3");
            }
            if params.acting_username.is_some() {
                sql.push_str(" AND pa.user_id IN (SELECT id FROM users WHERE username = $4)");
            }
            sql.push_str(" ORDER BY pa.created_at DESC LIMIT $5");
            let likes: Vec<(i32, i32, i32, NaiveDateTime)> = sqlx::query_as(&sql)
                .bind(&post_ids)
                .bind(viewer)
                .bind(params.before_like_id)
                .bind(params.acting_username)
                .bind(PAGE_SIZE)
                .bind(LIKE)
                .fetch_all(&mut *self.conn)
                .await?;
            let main = Reactions::load(self.settings)?.main;
            // translate_to_reactions
            rows.extend(
                likes
                    .into_iter()
                    .map(|(id, user_id, post_id, created_at)| UserReaction {
                        id: id as i64,
                        user_id,
                        post_id,
                        created_at,
                        reaction_id: id as i64,
                        reaction_value: main.clone(),
                        reaction_users_count: Some(1),
                        reaction_created_at: created_at,
                    }),
            );
            rows.sort_by_key(|r| std::cmp::Reverse(r.created_at));
        }
        rows.truncate(PAGE_SIZE as usize);
        self.serialize(&rows).await
    }

    /// `visible_posts_for_reactions_received`: the user's live posts in
    /// topics the viewer can see, of the types they see, hidden ones only
    /// when they may see those.
    async fn visible_post_ids(&mut self, user_id: i32) -> Result<Vec<i32>, AppError> {
        let topic_ids: Vec<i32> = sqlx::query_scalar(
            "SELECT DISTINCT p.topic_id FROM posts p JOIN topics t ON t.id = p.topic_id AND t.deleted_at IS NULL \
             WHERE p.user_id = $1 AND p.deleted_at IS NULL",
        )
        .bind(user_id)
        .fetch_all(&mut *self.conn)
        .await?;
        let visible_topics = self
            .guardian
            .can_see_topic_ids(&mut *self.conn, self.settings, &topic_ids)
            .await?;
        let types = self.guardian.visible_post_types(self.settings)?;
        let all_hidden = self.guardian.can_see_all_hidden_posts(self.settings)?;
        Ok(sqlx::query_scalar(
            "SELECT p.id FROM posts p JOIN topics t ON t.id = p.topic_id AND t.deleted_at IS NULL \
             WHERE p.user_id = $1 AND p.deleted_at IS NULL AND p.topic_id = ANY($2) AND p.post_type = ANY($3) \
               AND ($4 OR p.hidden = FALSE OR p.user_id = $5)",
        )
        .bind(user_id)
        .bind(&visible_topics)
        .bind(&types)
        .bind(all_hidden)
        .bind(self.guardian.user_id())
        .fetch_all(&mut *self.conn)
        .await?)
    }

    /// UserReactionSerializer for each row.
    async fn serialize(&mut self, rows: &[UserReaction]) -> Result<Value, AppError> {
        if self.settings.get("content_localization_enabled")?.truthy() {
            return Err(Unsupported("reactions with content localization").into());
        }
        let post_ids: Vec<i32> = rows.iter().map(|r| r.post_id).collect();
        #[derive(sqlx::FromRow)]
        struct PostRow {
            id: i32,
            created_at: NaiveDateTime,
            topic_id: i32,
            post_number: i32,
            post_type: i32,
            user_id: Option<i32>,
            cooked: String,
            title: String,
            fancy_title: Option<String>,
            slug: Option<String>,
            posts_count: i32,
            category_id: Option<i32>,
        }
        let posts: Vec<PostRow> = sqlx::query_as(
            "SELECT p.id, p.created_at, p.topic_id, p.post_number, p.post_type, p.user_id, p.cooked, \
                    t.title, t.fancy_title, t.slug, t.posts_count, t.category_id \
             FROM posts p JOIN topics t ON t.id = p.topic_id WHERE p.id = ANY($1)",
        )
        .bind(&post_ids)
        .fetch_all(&mut *self.conn)
        .await?;
        let posts: HashMap<i32, PostRow> = posts.into_iter().map(|p| (p.id, p)).collect();
        let mut user_ids: Vec<i32> = rows.iter().map(|r| r.user_id).collect();
        user_ids.extend(posts.values().filter_map(|p| p.user_id));
        let users = self.group_post_users(&user_ids).await?;
        let enable_names = self.enable_names()?;

        let mut out = Vec::new();
        for r in rows {
            let Some(post) = posts.get(&r.post_id) else {
                return Err(Unsupported("reactions on posts gone from under them").into());
            };
            let Some(slug) = post.slug.clone() else {
                return Err(Unsupported("topics without a stored slug (Slug.for)").into());
            };
            let fancy = crate::topic_query::fancy_title(
                &mut *self.conn,
                self.settings,
                post.topic_id,
                &post.title,
                post.fancy_title.as_deref(),
            )
            .await?;
            let author = post.user_id.and_then(|id| users.get(&id));

            // GroupPostSerializer, PostItemExcerpt's attributes first.
            let mut p = Map::new();
            p.insert(
                "excerpt".into(),
                json!(crate::excerpt::excerpt(
                    &post.cooked,
                    300,
                    &crate::excerpt::Options {
                        keep_emoji_images: true,
                        ..Default::default()
                    },
                )),
            );
            if post.cooked.chars().count() > 300 {
                p.insert("truncated".into(), json!(true));
            }
            p.insert("id".into(), json!(post.id));
            p.insert("created_at".into(), json!(time_json(post.created_at)));
            p.insert("topic_id".into(), json!(post.topic_id));
            p.insert("topic_title".into(), json!(post.title));
            p.insert("topic_slug".into(), json!(slug));
            p.insert("topic_html_title".into(), json!(fancy));
            p.insert(
                "url".into(),
                json!(format!("/t/{slug}/{}/{}", post.topic_id, post.post_number)),
            );
            p.insert("category_id".into(), json!(post.category_id));
            p.insert("post_number".into(), json!(post.post_number));
            p.insert("posts_count".into(), json!(post.posts_count));
            p.insert("post_type".into(), json!(post.post_type));
            p.insert("user_id".into(), json!(author.map(|u| u["id"].clone())));
            p.insert(
                "username".into(),
                json!(author.map(|u| u["username"].clone())),
            );
            if enable_names {
                p.insert("name".into(), json!(author.map(|u| u["name"].clone())));
            }
            p.insert(
                "avatar_template".into(),
                json!(author.map(|u| u["avatar_template"].clone())),
            );
            p.insert(
                "user_title".into(),
                json!(author.map(|u| u["title"].clone())),
            );
            p.insert(
                "primary_group_name".into(),
                json!(author.map(|u| u["primary_group_name"].clone())),
            );
            p.insert("user".into(), json!(author));
            p.insert(
                "topic".into(),
                json!({
                    "fancy_title": fancy,
                    "id": post.topic_id,
                    "title": post.title,
                    "slug": slug,
                    "posts_count": post.posts_count,
                }),
            );

            out.push(json!({
                "id": r.id,
                "user_id": r.user_id,
                "post_id": r.post_id,
                "created_at": time_json(r.created_at),
                "user": users.get(&r.user_id),
                "post": p,
                "reaction": {
                    "id": r.reaction_id,
                    "post_id": r.post_id,
                    "reaction_type": "emoji",
                    "reaction_value": r.reaction_value,
                    "reaction_users_count": r.reaction_users_count,
                    "created_at": time_json(r.reaction_created_at),
                },
            }));
        }
        Ok(Value::Array(out))
    }

    /// GroupPostUserSerializer for each user.
    async fn group_post_users(&mut self, ids: &[i32]) -> Result<HashMap<i32, Value>, AppError> {
        #[derive(sqlx::FromRow)]
        struct Row {
            id: i32,
            username: String,
            name: Option<String>,
            uploaded_avatar_id: Option<i32>,
            title: Option<String>,
            primary_group_name: Option<String>,
        }
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT u.id, u.username, u.name, u.uploaded_avatar_id, u.title, g.name AS primary_group_name \
             FROM users u LEFT JOIN groups g ON g.id = u.primary_group_id WHERE u.id = ANY($1)",
        )
        .bind(ids)
        .fetch_all(&mut *self.conn)
        .await?;
        let enable_names = self.enable_names()?;
        let mut out = HashMap::new();
        for r in rows {
            let avatar = self
                .user_avatar(r.id, &r.username, r.uploaded_avatar_id)
                .await?;
            let mut user = Map::new();
            user.insert("id".into(), json!(r.id));
            user.insert("username".into(), json!(r.username));
            if enable_names {
                user.insert("name".into(), json!(r.name));
            }
            user.insert("avatar_template".into(), json!(avatar));
            user.insert("title".into(), json!(r.title));
            user.insert("primary_group_name".into(), json!(r.primary_group_name));
            out.insert(r.id, Value::Object(user));
        }
        Ok(out)
    }
}
