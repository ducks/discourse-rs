//! The advanced filters' blocks (lib/search.rb's `advanced_filter`s and
//! the plugins'), each narrowing posts_query's query.

use unicode_normalization::UnicodeNormalization;

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
