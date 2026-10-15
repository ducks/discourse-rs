//! The advanced filters' blocks (lib/search.rb's `advanced_filter`s and
//! the plugins'), each narrowing posts_query's query.

use unicode_normalization::UnicodeNormalization;

use super::advanced::Filter;
use super::posts_query::Query;
use super::{Search, SearchError};

impl Search<'_> {
    /// `@username`: the user's posts; `@me` the viewer's when no user has
    /// that name.
    pub(super) async fn at_filter(&mut self, q: &mut Query, name: &str) -> Result<(), SearchError> {
        // User.normalize_username
        let username = name.nfc().collect::<String>().to_lowercase();
        let mut user_id: Option<i32> = sqlx::query_scalar(
            "SELECT id FROM users WHERE users.staged = FALSE AND users.username_lower = $1 LIMIT 1",
        )
        .bind(&username)
        .fetch_optional(&mut *self.conn)
        .await?;
        if user_id.is_none() && username == "me" {
            user_id = self.guardian.user_id();
        }
        match user_id {
            Some(id) => q.and(format!("posts.user_id = {id}")),
            None => q.none(),
        }
        Ok(())
    }

    /// `user:` a username or a user id.
    pub(super) async fn user_filter(
        &mut self,
        q: &mut Query,
        name: &str,
    ) -> Result<(), SearchError> {
        let user_id: Option<i32> = sqlx::query_scalar(
            "SELECT id FROM users WHERE users.staged = FALSE \
             AND (username_lower = $1 OR id = $2) LIMIT 1",
        )
        .bind(name.to_lowercase())
        .bind(crate::ruby::to_i(name))
        .fetch_optional(&mut *self.conn)
        .await?;
        match user_id {
            Some(id) => q.and(format!("posts.user_id = {id}")),
            None => q.none(),
        }
        Ok(())
    }

    /// `created:@username`: the topics the user started (their first
    /// posts); a name nobody has matches the first posts without a user.
    pub(super) async fn created_by_filter(
        &mut self,
        q: &mut Query,
        name: &str,
    ) -> Result<(), SearchError> {
        let user_id: Option<i32> =
            sqlx::query_scalar("SELECT id FROM users WHERE users.username_lower = $1 LIMIT 1")
                .bind(name.to_lowercase())
                .fetch_optional(&mut *self.conn)
                .await?;
        match user_id {
            Some(id) => q.and(format!("posts.user_id = {id} AND posts.post_number = 1")),
            None => q.and("posts.user_id IS NULL AND posts.post_number = 1"),
        }
        Ok(())
    }

    /// `group:` a group's name (ILIKE) or id, among the groups whose
    /// members the viewer may see: its members' posts.
    pub(super) async fn group_filter(
        &mut self,
        q: &mut Query,
        name: &str,
    ) -> Result<(), SearchError> {
        let visible = crate::groups::visible_groups_scope(self.guardian, "groups", false, false);
        let members_visible = crate::groups::members_visible_groups_where(self.guardian, "groups");
        let group_id: Option<i32> = sqlx::query_scalar(&format!(
            "SELECT groups.id FROM groups WHERE ({visible}) AND ({members_visible}) \
             AND (groups.name ILIKE $1 OR (groups.id = $2 AND groups.id > 0)) \
             ORDER BY groups.name ASC, name ASC LIMIT 1"
        ))
        .bind(name)
        .bind(crate::ruby::to_i(name))
        .fetch_optional(&mut *self.conn)
        .await?;
        match group_id {
            Some(id) => q.and(format!(
                "posts.user_id IN (select gu.user_id from group_users gu where gu.group_id = {id})"
            )),
            None => q.none(),
        }
        Ok(())
    }

    /// The `in:` filters over the viewer's own activity; anonymous
    /// searches ignore them.
    pub(super) fn viewer_filter(&self, q: &mut Query, filter: Filter, capture: &str) {
        let Some(uid) = self.guardian.user_id() else {
            return;
        };
        let timings = format!(
            "post_timings ON post_timings.topic_id = posts.topic_id \
             AND post_timings.post_number = posts.post_number AND post_timings.user_id = {uid}"
        );
        match filter {
            // PostActionType.types[:like]
            Filter::Likes => q.and(format!(
                "posts.id IN (SELECT pa.post_id FROM post_actions pa WHERE pa.user_id = {uid} \
                 AND pa.post_action_type_id = 2 AND deleted_at IS NULL)"
            )),
            Filter::Bookmarks => q.and(format!(
                "posts.id IN (SELECT bookmarkable_id FROM bookmarks \
                 WHERE bookmarks.user_id = {uid} AND bookmarks.bookmarkable_type = 'Post')"
            )),
            Filter::Posted => q.and(format!("posts.user_id = {uid}")),
            Filter::Created => q.and(format!("posts.user_id = {uid} AND posts.post_number = 1")),
            Filter::WatchingTracking => {
                // TopicUser.notification_levels
                let level = if capture.eq_ignore_ascii_case("watching") {
                    3
                } else {
                    2
                };
                q.and(format!(
                    "posts.topic_id IN (SELECT tu.topic_id FROM topic_users tu \
                     WHERE tu.user_id = {uid} AND tu.notification_level >= {level})"
                ));
            }
            Filter::Seen => q.join(&format!("INNER JOIN {timings}")),
            Filter::Unseen => {
                q.join(&format!("LEFT JOIN {timings}"));
                q.and("post_timings.user_id IS NULL");
            }
            _ => {}
        }
    }

    /// `search_tags`: `tags:a,b` any of them, `tags:a+b` all of them
    /// (synonyms read as their target); `-tags:` none of them.
    pub(super) async fn tags_filter(
        &mut self,
        q: &mut Query,
        tags: &str,
        positive: bool,
    ) -> Result<(), SearchError> {
        let lower = tags.to_lowercase();
        let modifier = if positive { "" } else { "NOT" };
        if lower.contains('+') {
            let names = self
                .resolve_tag_synonyms(crate::ruby::split(&lower, "+"))
                .await?;
            let config = super::ts_config(&self.settings.get("default_locale")?.to_s());
            q.and(format!(
                "topics.id {modifier} IN (SELECT tt.topic_id FROM topic_tags tt, tags \
                 WHERE tt.tag_id = tags.id GROUP BY tt.topic_id \
                 HAVING to_tsvector('{config}', array_to_string(array_agg(lower(tags.name)), ' ')) \
                 @@ to_tsquery('{config}', {}))",
                crate::user_search::quote(&names.join("&"))
            ));
            return Ok(());
        }
        let names = self
            .resolve_tag_synonyms(crate::ruby::split(&lower, ","))
            .await?;
        let lowered: Vec<String> = names.iter().map(|n| n.to_lowercase()).collect();
        let ids: Vec<i32> =
            sqlx::query_scalar("SELECT id FROM tags WHERE lower(tags.name) = ANY($1)")
                .bind(&lowered)
                .fetch_all(&mut *self.conn)
                .await?;
        if ids.is_empty() {
            if positive {
                q.none();
            }
            return Ok(());
        }
        q.and(format!(
            "{modifier} EXISTS (SELECT 1 FROM topic_tags tt WHERE tt.topic_id = topics.id \
             AND tt.tag_id IN ({}))",
            ids.iter()
                .map(i32::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ));
        Ok(())
    }

    /// `resolve_tag_synonyms`: each name that is a synonym becomes its
    /// target's, lowercased.
    async fn resolve_tag_synonyms(&mut self, names: Vec<&str>) -> Result<Vec<String>, SearchError> {
        let lowered: Vec<String> = names.iter().map(|n| n.to_lowercase()).collect();
        let synonyms: Vec<(String, String)> = sqlx::query_as(
            "SELECT lower(s.name), lower(t.name) FROM tags s JOIN tags t ON t.id = s.target_tag_id \
             WHERE lower(s.name) = ANY($1) AND s.target_tag_id IS NOT NULL",
        )
        .bind(&lowered)
        .fetch_all(&mut *self.conn)
        .await?;
        Ok(names
            .iter()
            .map(|n| {
                synonyms
                    .iter()
                    .find(|(s, _)| s == n)
                    .map(|(_, t)| t.clone())
                    .unwrap_or_else(|| n.to_string())
            })
            .collect())
    }

    /// `status:public`: the topics in categories everyone can read.
    pub(super) async fn public_filter(&mut self, q: &mut Query) -> Result<(), SearchError> {
        let ids: Vec<i32> =
            sqlx::query_scalar("SELECT id FROM categories WHERE read_restricted = FALSE")
                .fetch_all(&mut *self.conn)
                .await?;
        // An empty list is `IN (NULL)` to ActiveRecord.
        let list = if ids.is_empty() {
            "NULL".to_string()
        } else {
            ids.iter()
                .map(i32::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        };
        q.and(format!("topics.category_id in ({list})"));
        Ok(())
    }

    /// `filetypes:` a comma list of extensions: posts linking to such
    /// files or with such uploads.
    pub(super) fn filetypes_filter(&self, q: &mut Query, types: &str) {
        let list = crate::ruby::split(types, ",")
            .iter()
            .map(|t| crate::user_search::quote(&t.to_lowercase()))
            .collect::<Vec<_>>()
            .join(", ");
        let list = if list.is_empty() {
            "NULL".to_string()
        } else {
            list
        };
        q.and(format!(
            "posts.id IN (SELECT post_id FROM topic_links WHERE extension IN ({list}) \
             UNION SELECT upload_references.target_id FROM uploads \
             JOIN upload_references ON upload_references.target_type = 'Post' \
             AND upload_references.upload_id = uploads.id \
             WHERE lower(uploads.extension) IN ({list}))"
        ));
    }

    /// `locale:` none, any, or a language (its base, `pt` for `pt_BR`).
    pub(super) fn locale_filter(&self, q: &mut Query, locale: &str) {
        match locale.to_lowercase().as_str() {
            "none" | "null" => q.and("posts.locale IS NULL"),
            "any" | "present" => q.and("posts.locale IS NOT NULL"),
            other => {
                let normalized = other.replace('-', "_");
                let base = normalized.split('_').next().unwrap_or("");
                q.and(format!(
                    "posts.locale LIKE {}",
                    crate::user_search::quote(&format!("{base}%"))
                ));
            }
        }
    }

    /// discourse-solved's `status:unsolved`: topics without an answer,
    /// where answers are accepted (all topics, or the categories and tags
    /// enabling them).
    pub(super) async fn unsolved_filter(&mut self, q: &mut Query) -> Result<(), SearchError> {
        q.and("topics.id NOT IN (SELECT topic_id FROM discourse_solved_solved_topics)");
        if !self.settings.get("allow_solved_on_all_topics")?.truthy() {
            let names: Vec<String> = self
                .settings
                .get("enable_solved_tags")?
                .to_s()
                .split('|')
                .map(str::to_string)
                .collect();
            let ids: Vec<i32> = sqlx::query_scalar("SELECT id FROM tags WHERE name = ANY($1)")
                .bind(&names)
                .fetch_all(&mut *self.conn)
                .await?;
            let list = if ids.is_empty() {
                "NULL".to_string()
            } else {
                ids.iter()
                    .map(i32::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            q.and(format!(
                "topics.id IN (SELECT t.id FROM topics t JOIN category_custom_fields cc \
                 ON t.category_id = cc.category_id AND cc.name = 'enable_accepted_answers' \
                 AND cc.value = 'true') \
                 OR topics.id IN (SELECT topic_id FROM topic_tags WHERE tag_id IN ({list}))"
            ));
        }
        q.and("NOT (topics.archetype = 'private_message')");
        Ok(())
    }

    /// `group_messages:` a messageable group's name (ILIKE) or id, among
    /// the visible groups whose members are visible: its messages.
    pub(super) async fn group_messages_filter(
        &mut self,
        q: &mut Query,
        name: &str,
    ) -> Result<(), SearchError> {
        let visible = crate::groups::visible_groups_scope(self.guardian, "groups", false, false);
        let members_visible = crate::groups::members_visible_groups_where(self.guardian, "groups");
        let group_id: Option<i32> = sqlx::query_scalar(&format!(
            "SELECT groups.id FROM groups WHERE ({visible}) AND ({members_visible}) \
             AND (has_messages = TRUE) AND (name ilike $1 OR (id = $2 AND id > 0)) \
             ORDER BY groups.name ASC, name ASC LIMIT 1"
        ))
        .bind(name)
        .bind(crate::ruby::to_i(name))
        .fetch_optional(&mut *self.conn)
        .await?;
        match group_id {
            Some(id) => q.and(format!(
                "posts.topic_id IN (SELECT topic_id FROM topic_allowed_groups WHERE group_id = {id})"
            )),
            None => q.none(),
        }
        Ok(())
    }

    /// `badge:` a badge's name (ILIKE) or id: the posts of its holders.
    pub(super) async fn badge_filter(
        &mut self,
        q: &mut Query,
        name: &str,
    ) -> Result<(), SearchError> {
        let badge_id: Option<i32> =
            sqlx::query_scalar("SELECT id FROM badges WHERE (name ilike $1 OR id = $2) LIMIT 1")
                .bind(name)
                .bind(crate::ruby::to_i(name))
                .fetch_optional(&mut *self.conn)
                .await?;
        match badge_id {
            Some(id) => q.and(format!(
                "posts.user_id IN (SELECT ub.user_id FROM user_badges ub WHERE ub.badge_id = {id})"
            )),
            None => q.none(),
        }
        Ok(())
    }
}
