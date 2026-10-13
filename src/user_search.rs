//! `UserSearch` (app/models/user_search.rb) and the users half of
//! UsersController#search_users: who an `@` autocomplete offers, exact
//! username first, then people in the topic, then members of its secure
//! category, then anyone matching, then (optionally) the recently seen.

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::guardian::Guardian;
use crate::site_settings::SiteSettings;
use crate::url::Urls;
use crate::{AppError, Unsupported};

/// `UserSearch::MAX_SIZE_PRIORITY_MENTION`
const MAX_SIZE_PRIORITY_MENTION: i32 = 500;

#[derive(Default)]
pub struct Options {
    pub topic_id: Option<i32>,
    pub category_id: Option<i32>,
    pub prioritized_user_id: Option<i32>,
    /// `topic_allowed_users == "true"`
    pub topic_allowed_users: bool,
    /// The ids of the `groups` the users must be in.
    pub groups: Option<Vec<i32>>,
    pub include_staged_users: bool,
    pub last_seen_users: bool,
    pub can_review: bool,
    pub limit: Option<i64>,
}

/// Why the search can't run: what `Topic.find`, `Category.find` and the
/// guardian's `ensure_can_see_*!` raise.
pub enum Refused {
    NotFound,
    InvalidAccess,
}

pub struct UserSearch<'a> {
    conn: &'a mut PgConnection,
    settings: &'a SiteSettings,
    guardian: &'a Guardian,
    term: String,
    term_like: String,
    opts: Options,
    include_staged: bool,
    last_seen: bool,
    limit: i64,
    /// The topic's category when it is read restricted
    /// (`@topic&.category&.read_restricted`).
    topic_restricted_category: Option<i32>,
}

impl<'a> UserSearch<'a> {
    /// `UserSearch.new(term, opts)`, with its finds and checks.
    pub async fn new(
        conn: &'a mut PgConnection,
        settings: &'a SiteSettings,
        guardian: &'a Guardian,
        term: &str,
        opts: Options,
    ) -> Result<Result<UserSearch<'a>, Refused>, AppError> {
        let term = term.to_lowercase();
        let term_like = format!("{}%", like_escape(&term));
        // Topic.find and Category.find, before any check.
        let topic = match opts.topic_id {
            Some(id) => {
                match crate::topic_guardian::TopicCtx::load(&mut *conn, settings, guardian, id)
                    .await?
                {
                    Some(topic) if !topic.trashed() => Some(topic),
                    _ => return Ok(Err(Refused::NotFound)),
                }
            }
            None => None,
        };
        let category: Option<(bool, Option<String>, bool)> = match opts.category_id {
            Some(id) => {
                let row = sqlx::query_as(
                    "SELECT read_restricted, email_in, email_in_allow_strangers FROM categories WHERE id = $1",
                )
                .bind(id)
                .fetch_optional(&mut *conn)
                .await?;
                match row {
                    Some(row) => Some(row),
                    None => return Ok(Err(Refused::NotFound)),
                }
            }
            None => None,
        };
        // @groups.each { ensure_can_see_group_and_members! }
        for group_id in opts.groups.iter().flatten() {
            let (visibility, members): (i32, i32) = sqlx::query_as(
                "SELECT visibility_level, members_visibility_level FROM groups WHERE id = $1",
            )
            .bind(group_id)
            .fetch_one(&mut *conn)
            .await?;
            if !crate::groups::can_see_group_and_members(
                &mut *conn, guardian, *group_id, visibility, members,
            )
            .await?
            {
                return Ok(Err(Refused::InvalidAccess));
            }
        }
        let secure = guardian.secure_category_ids(&mut *conn, settings).await?;
        // ensure_can_see_category!
        if let (Some(id), Some((read_restricted, email_in, strangers))) =
            (opts.category_id, &category)
        {
            let admin_sees = guardian.is_admin()
                && !settings
                    .get("suppress_secured_categories_from_admin")?
                    .truthy();
            let staged_sees = guardian.user().is_some_and(|u| u.staged)
                && email_in.as_deref().is_some_and(|e| !e.is_empty())
                && *strangers;
            if !(admin_sees || !read_restricted || staged_sees || secure.contains(&id)) {
                return Ok(Err(Refused::InvalidAccess));
            }
        }
        // ensure_can_see_topic!
        if let Some(topic) = &topic
            && !guardian.can_see_topic(settings, topic, true, &secure)?
        {
            return Ok(Err(Refused::InvalidAccess));
        }
        let include_staged = opts.include_staged_users && guardian.is_staff();
        let last_seen = opts.last_seen_users && !include_staged;
        let limit = opts.limit.unwrap_or(20);
        let topic_restricted_category = topic
            .as_ref()
            .filter(|t| t.read_restricted == Some(true))
            .and_then(|t| t.category_id);
        Ok(Ok(UserSearch {
            conn,
            settings,
            guardian,
            term,
            term_like,
            opts,
            include_staged,
            last_seen,
            limit,
            topic_restricted_category,
        }))
    }

    /// `scoped_users` for callers that list them (chat's chatables).
    pub fn scoped_users_sql(&self) -> Result<(String, String), AppError> {
        self.scoped_users()
    }

    /// `scoped_users` as joins and a condition over `users`.
    fn scoped_users(&self) -> Result<(String, String), AppError> {
        let mut joins = String::new();
        let mut conditions = vec!["users.active = TRUE".to_string()];
        if self.settings.get("must_approve_users")?.truthy() {
            conditions.push("users.approved = TRUE".into());
        }
        if !self.include_staged {
            conditions.push("users.staged = FALSE".into());
        }
        if !self.guardian.is_staff() {
            conditions
                .push("(users.suspended_till IS NULL OR users.suspended_till <= now())".into());
        }
        if let Some(groups) = &self.opts.groups {
            joins.push_str(" INNER JOIN group_users ON group_users.user_id = users.id");
            // An empty relation still joins: nobody is in no group.
            let ids = if groups.is_empty() {
                "NULL".to_string()
            } else {
                id_list(groups)
            };
            conditions.push(format!("group_users.group_id IN ({ids})"));
        }
        if self.opts.can_review {
            if self
                .settings
                .get("enable_category_group_moderation")?
                .truthy()
            {
                if self.opts.groups.is_some() {
                    return Err(Unsupported("user search by groups among reviewers").into());
                }
                joins.push_str(" LEFT OUTER JOIN group_users ON group_users.user_id = users.id");
                conditions.push(
                    "(users.admin OR users.moderator OR group_users.group_id IN \
                     (SELECT DISTINCT group_id FROM category_moderation_groups))"
                        .into(),
                );
            } else {
                conditions.push("(users.moderator OR users.admin)".into());
            }
        }
        // Only show users who have access to the private topic.
        if self.opts.topic_allowed_users
            && let Some(category_id) = self.topic_restricted_category
        {
            conditions.push(format!(
                "(users.admin OR EXISTS (SELECT 1 FROM group_users sgu \
                   JOIN category_groups scg ON scg.group_id = sgu.group_id \
                   WHERE sgu.user_id = users.id AND scg.category_id = {category_id}))"
            ));
        }
        Ok((joins, conditions.join(" AND ")))
    }

    /// `filtered_by_term_users`: joins, condition and the ORDER BY it
    /// starts with.
    async fn filtered_by_term_users(&mut self) -> Result<(String, String, Vec<String>), AppError> {
        let (mut joins, mut condition) = self.scoped_users()?;
        let mut order = Vec::new();
        if self.term.is_empty() {
        } else if self.settings.get("enable_names")?.truthy()
            && !self.term.contains(['_', '.', '-'])
        {
            // tsvector_filtered_users: Search.ts_query(term:, ts_config: "simple")
            let mut value = crate::search::ts_query_value(&self.term, "", true);
            if self.settings.get("search_ignore_accents")?.truthy() {
                value = sqlx::query_scalar("SELECT unaccent($1)")
                    .bind(&value)
                    .fetch_one(&mut *self.conn)
                    .await?;
            }
            joins.push_str(
                " LEFT OUTER JOIN user_search_data ON user_search_data.user_id = users.id",
            );
            condition.push_str(&format!(
                " AND user_search_data.search_data @@ REGEXP_REPLACE(TO_TSQUERY('simple', {})::text, '<->|<\\d+>', '&', 'g')::tsquery",
                quote(&value)
            ));
            order.push(format!(
                "CASE WHEN username_lower LIKE {} THEN 0 ELSE 1 END ASC",
                quote(&self.term_like)
            ));
        } else {
            condition.push_str(&format!(
                " AND username_lower LIKE {}",
                quote(&self.term_like)
            ));
        }
        Ok((joins, condition, order))
    }

    async fn pluck(&mut self, sql: &str, ids: &mut Vec<i32>) -> Result<(), AppError> {
        let found: Vec<i32> = sqlx::query_scalar(sql).fetch_all(&mut *self.conn).await?;
        for id in found {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        Ok(())
    }

    /// `search_ids`
    pub async fn search_ids(&mut self) -> Result<Vec<i32>, AppError> {
        let mut ids: Vec<i32> = Vec::new();
        let limit = self.limit;
        let remaining = |ids: &Vec<i32>| limit - ids.len() as i64;
        let searcher = self.guardian.user_id();

        // 1. exact username matches
        if !self.term.is_empty() {
            let (joins, condition) = self.scoped_users()?;
            let mut sql = format!(
                "SELECT users.id FROM users{joins} WHERE {condition} AND users.username_lower = {}",
                quote(&self.term)
            );
            // Don't pollute mentions with users who haven't shown up in a year.
            if self.opts.topic_id.is_some() || self.opts.category_id.is_some() {
                sql.push_str(" AND users.last_seen_at > now() - interval '1 year'");
            }
            sql.push_str(&format!(" LIMIT {}", limit.max(0)));
            self.pluck(&sql, &mut ids).await?;
        }
        if remaining(&ids) <= 0 {
            return Ok(ids);
        }

        // 2. in topic
        if let Some(topic_id) = self.opts.topic_id {
            let (joins, mut condition, mut order) = self.filtered_by_term_users().await?;
            condition.push_str(&format!(
                " AND users.id IN (SELECT user_id FROM posts WHERE topic_id = {topic_id} AND post_type = 1 AND deleted_at IS NULL)"
            ));
            if let Some(id) = searcher {
                condition.push_str(&format!(" AND users.id <> {id}"));
            }
            if let Some(id) = self.opts.prioritized_user_id {
                order.push(format!("CASE WHEN users.id = {id} THEN 0 ELSE 1 END"));
            }
            order.push("last_seen_at DESC NULLS LAST".into());
            let sql = format!(
                "SELECT users.id FROM users{joins} WHERE {condition} ORDER BY {} LIMIT {}",
                order.join(", "),
                remaining(&ids)
            );
            self.pluck(&sql, &mut ids).await?;
        }
        if remaining(&ids) <= 0 {
            return Ok(ids);
        }

        // 3. in a secure category
        let secure_category: Option<i32> = match (self.opts.category_id, self.opts.topic_id) {
            (Some(id), _) => {
                sqlx::query_scalar("SELECT id FROM categories WHERE read_restricted AND id = $1")
                    .bind(id)
                    .fetch_optional(&mut *self.conn)
                    .await?
            }
            (None, Some(topic_id)) => {
                sqlx::query_scalar(
                    "SELECT id FROM categories WHERE read_restricted AND id IN (SELECT category_id FROM topics WHERE id = $1)",
                )
                .bind(topic_id)
                .fetch_optional(&mut *self.conn)
                .await?
            }
            (None, None) => None,
        };
        if let Some(category_id) = secure_category {
            let mut sql = format!(
                "SELECT groups.id FROM groups WHERE groups.id IN (SELECT group_id FROM category_groups \
                 JOIN groups g ON group_id = g.id WHERE category_id = {category_id} AND user_count < {MAX_SIZE_PRIORITY_MENTION})"
            );
            if searcher.is_some() {
                sql.push_str(&format!(
                    " AND {}",
                    crate::groups::members_visible_groups_where(self.guardian, "groups")
                ));
            }
            let groups: Vec<i32> = sqlx::query_scalar(&sql).fetch_all(&mut *self.conn).await?;
            let (joins, mut condition, mut order) = self.filtered_by_term_users().await?;
            let groups = if groups.is_empty() {
                "NULL".to_string()
            } else {
                id_list(&groups)
            };
            condition.push_str(&format!(
                " AND users.id IN (SELECT gu.user_id FROM group_users gu WHERE group_id IN ({groups}) LIMIT 200)"
            ));
            if let Some(id) = searcher {
                condition.push_str(&format!(" AND users.id <> {id}"));
            }
            order.push("last_seen_at DESC NULLS LAST".into());
            let sql = format!(
                "SELECT users.id FROM users{joins} WHERE {condition} ORDER BY {} LIMIT {}",
                order.join(", "),
                remaining(&ids)
            );
            self.pluck(&sql, &mut ids).await?;
        }
        if remaining(&ids) <= 0 {
            return Ok(ids);
        }

        // 4. global matches
        if !self.term.is_empty() {
            let (joins, condition, mut order) = self.filtered_by_term_users().await?;
            order.push("last_seen_at DESC NULLS LAST".into());
            let sql = format!(
                "SELECT users.id FROM users{joins} WHERE {condition} ORDER BY {} LIMIT {}",
                order.join(", "),
                remaining(&ids)
            );
            self.pluck(&sql, &mut ids).await?;
        }
        if remaining(&ids) <= 0 {
            return Ok(ids);
        }

        // 5. last seen users
        if self.last_seen {
            let (joins, condition) = self.scoped_users()?;
            let sql = format!(
                "SELECT users.id FROM users{joins} WHERE {condition} ORDER BY last_seen_at DESC NULLS LAST LIMIT {}",
                remaining(&ids)
            );
            self.pluck(&sql, &mut ids).await?;
        }
        Ok(ids)
    }

    /// `scoped_users.where(username_lower: usernames).limit(limit)`, as
    /// the controller asks with `usernames`.
    pub async fn by_usernames(
        &mut self,
        usernames: &[String],
        limit: Option<i64>,
    ) -> Result<Vec<i32>, AppError> {
        let (joins, condition) = self.scoped_users()?;
        let names: Vec<String> = usernames.iter().map(|u| quote(u)).collect();
        let mut sql = format!(
            "SELECT users.id FROM users{joins} WHERE {condition} AND users.username_lower IN ({})",
            names.join(", ")
        );
        if let Some(limit) = limit {
            sql.push_str(&format!(" LIMIT {limit}"));
        }
        let mut ids = Vec::new();
        self.pluck(&sql, &mut ids).await?;
        Ok(ids)
    }
}

/// `FoundUserSerializer` (with `include_status: true`) for the users, in
/// the order given.
pub async fn found_users(
    conn: &mut PgConnection,
    urls: &Urls<'_>,
    guardian: &Guardian,
    ids: &[i32],
) -> Result<Vec<Value>, AppError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    #[derive(sqlx::FromRow)]
    struct Row {
        id: i32,
        username: String,
        name: Option<String>,
        uploaded_avatar_id: Option<i32>,
        silenced: bool,
        hide_profile: Option<bool>,
        status_description: Option<String>,
        status_emoji: Option<String>,
        status_ends_at: Option<chrono::NaiveDateTime>,
    }
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT users.id, users.username, users.name, users.uploaded_avatar_id, \
           COALESCE(users.silenced_till > now(), FALSE) AS silenced, user_options.hide_profile, \
           user_statuses.description AS status_description, user_statuses.emoji AS status_emoji, \
           user_statuses.ends_at AS status_ends_at \
         FROM users \
         LEFT JOIN user_options ON user_options.user_id = users.id \
         LEFT JOIN user_statuses ON user_statuses.user_id = users.id \
           AND (user_statuses.ends_at IS NULL OR user_statuses.ends_at >= now()) \
         WHERE users.id = ANY($1)",
    )
    .bind(ids)
    .fetch_all(&mut *conn)
    .await?;
    let settings = urls.settings;
    let names = settings.get("enable_names")?.truthy();
    let statuses = settings.get("enable_user_status")?.truthy();
    let logo = crate::admin_users::logo_small_url(&mut *conn, settings).await?;
    let mut out = Vec::new();
    for id in ids {
        let Some(row) = rows.iter().find(|r| r.id == *id) else {
            continue;
        };
        let mut user = Map::new();
        // include_status?: the status attribute comes first.
        let hidden = row.hide_profile == Some(true);
        let can_see =
            !hidden && (!row.silenced || guardian.user_id() == Some(row.id) || guardian.is_staff());
        if statuses
            && can_see
            && let (Some(description), Some(emoji)) = (&row.status_description, &row.status_emoji)
        {
            user.insert(
                "status".into(),
                json!({
                    "description": description,
                    "emoji": emoji,
                    "ends_at": row.status_ends_at.map(|t| t.and_utc().format("%Y-%m-%dT%H:%M:%SZ").to_string()),
                    // MessageBus.last_id("/user-status"): the port keeps no
                    // per-channel ids.
                    "message_bus_last_id": 0,
                }),
            );
        }
        user.insert("id".into(), json!(row.id));
        user.insert("username".into(), json!(row.username));
        if names {
            user.insert("name".into(), json!(row.name));
        }
        user.insert(
            "avatar_template".into(),
            json!(crate::avatar::avatar_template(
                urls,
                row.id,
                &row.username,
                row.uploaded_avatar_id,
                logo.as_deref()
            )?),
        );
        out.push(Value::Object(user));
    }
    Ok(out)
}

/// `Group.search_groups(term, groups:, sort: :auto)` over the groups the
/// condition picks: names and full names containing the term (its LIKE
/// wildcards live), those starting with it first, then by name.
pub fn search_groups_sql(condition: &str, term: &str) -> String {
    let contains = quote(&format!("%{term}%"));
    let prefix = quote(&format!("{}%", term.replace('_', "\\_")));
    format!(
        "SELECT groups.name, groups.full_name FROM groups WHERE ({condition}) \
         AND (groups.name ILIKE {contains} OR groups.full_name ILIKE {contains}) \
         ORDER BY CASE WHEN groups.name ILIKE {prefix} OR groups.full_name ILIKE {prefix} THEN 0 ELSE 1 END ASC, groups.name ASC"
    )
}

/// `Group.visible_groups(user)`: real groups the user may see.
pub fn visible_groups_where(guardian: &Guardian) -> String {
    format!(
        "groups.id > 0 AND groups.id NOT IN (4, 5) AND {}",
        crate::groups::visible_groups_where(guardian, "groups")
    )
}

/// `Group.alias_levels(user)`
fn alias_levels(guardian: &Guardian) -> &'static str {
    if guardian.is_admin() {
        "99, 1, 2, 3, 4"
    } else if guardian.is_moderator() {
        "99, 2, 3, 4"
    } else {
        "99"
    }
}

/// `Group.mentionable(user)` (include_public: true)
pub fn mentionable_where(guardian: &Guardian, user_id: i32) -> String {
    format!(
        "groups.mentionable_level IN ({levels}) \
         OR (groups.mentionable_level = 3 AND groups.id IN (SELECT group_id FROM group_users WHERE user_id = {user_id})) \
         OR (groups.mentionable_level = 4 AND groups.id IN (SELECT group_id FROM group_users WHERE user_id = {user_id} AND owner IS TRUE)) \
         OR groups.visibility_level = 0",
        levels = alias_levels(guardian)
    )
}

/// `Group.messageable(user)`
pub fn messageable_where(guardian: &Guardian, user_id: i32) -> String {
    format!(
        "groups.messageable_level IN ({levels}) \
         OR (groups.messageable_level = 3 AND groups.id IN (SELECT group_id FROM group_users WHERE user_id = {user_id})) \
         OR (groups.messageable_level = 4 AND groups.id IN (SELECT group_id FROM group_users WHERE user_id = {user_id} AND owner IS TRUE))",
        levels = alias_levels(guardian)
    )
}

fn id_list(ids: &[i32]) -> String {
    ids.iter()
        .map(i32::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// A SQL string literal.
pub(crate) fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// `sanitize_sql_like`
fn like_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_search_escapes_only_underscores_in_the_prefix() {
        let sql = search_groups_sql("TRUE", "trust_");
        assert!(sql.contains("ILIKE '%trust_%'"));
        assert!(sql.contains("ILIKE 'trust\\_%'"));
    }

    #[test]
    fn like_terms_match_literally() {
        assert_eq!(like_escape("a_b%c\\"), "a\\_b\\%c\\\\");
    }
}
