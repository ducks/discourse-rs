//! Port of lib/topic_query/private_message_lists.rb: the personal and
//! group message lists. The viewer (`guardian`, who drives `visible` and
//! the serializer) and the mailbox owner (`owner`, whose allowed topics,
//! archive and `tu` row the query reads) differ when an admin reads
//! someone else's inbox.

use chrono::NaiveDateTime;
use sqlx::PgConnection;

use crate::Unsupported;
use crate::guardian::Guardian;
use crate::site_settings::SiteSettings;
use crate::topic_query::{Options, TOPIC_COLUMNS, TopicList, TopicQueryError, TopicRow};

/// Which message list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PmList {
    Inbox,
    Sent,
    Archive,
    Unread,
    New,
    Warnings,
    /// discourse-assign's `list_private_messages_assigned`: the owner's
    /// messages (theirs and their message groups') assigned to them or to
    /// one of their groups.
    Assigned,
    /// `/group/:name`, optionally its archive, new or unread view.
    Group {
        id: i32,
        name: String,
        view: GroupView,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupView {
    Inbox,
    Archive,
    New,
    Unread,
}

pub struct PmQuery<'a> {
    pub conn: &'a mut PgConnection,
    pub settings: &'a SiteSettings,
    pub guardian: &'a Guardian,
    pub options: Options,
    pub owner_id: i32,
}

impl PmQuery<'_> {
    /// `list_private_messages_*(owner)` through `create_list(:private_messages)`.
    pub async fn list(&mut self, list: &PmList) -> Result<TopicList, TopicQueryError> {
        let per_page = self
            .options
            .per_page
            .unwrap_or(crate::topic_query::DEFAULT_PER_PAGE);
        let owner = self.owner_id;
        let viewer = self
            .guardian
            .user_id()
            .ok_or(Unsupported("message lists for anonymous"))?;
        let mut joins = vec![format!(
            "LEFT OUTER JOIN topic_users AS tu ON (topics.id = tu.topic_id AND tu.user_id = {owner})"
        )];
        let mut wheres = vec![
            "topics.deleted_at IS NULL".to_string(),
            "topics.archetype = 'private_message'".to_string(),
        ];
        // `.visible` for viewers who are regular users.
        if !self.guardian.is_staff() {
            wheres.push("topics.visible = TRUE".to_string());
        }
        let personal = format!(
            "(topics.id IN (SELECT topic_id FROM topic_allowed_users WHERE user_id = {owner}))"
        );
        match list {
            PmList::Inbox => {
                joins.push(format!(
                    "LEFT JOIN user_archived_messages um ON um.user_id = {owner} AND um.topic_id = topics.id"
                ));
                wheres.push(personal);
                wheres.push("(um.user_id IS NULL)".to_string());
                // have_posts_from_others
                wheres.push(format!(
                    "(NOT (topics.participant_count = 1 AND topics.user_id = {owner} AND topics.moderator_posts_count = 0))"
                ));
            }
            PmList::Sent => {
                joins.push(format!(
                    "LEFT JOIN user_archived_messages um ON um.user_id = {owner} AND um.topic_id = topics.id"
                ));
                wheres.push(personal);
                wheres.push(format!(
                    "(EXISTS (SELECT 1 FROM posts WHERE posts.topic_id = topics.id AND posts.user_id = {owner}))"
                ));
                wheres.push("(um.user_id IS NULL)".to_string());
            }
            PmList::Archive => {
                joins.push(
                    "INNER JOIN user_archived_messages ON user_archived_messages.topic_id = topics.id"
                        .to_string(),
                );
                wheres.push(personal);
                wheres.push(format!("(user_archived_messages.user_id = {owner})"));
            }
            PmList::Unread => {
                wheres.push(personal);
                wheres.push(self.unread_filter(viewer, self.settings)?);
                let first_unread: Option<Option<NaiveDateTime>> = sqlx::query_scalar(
                    "SELECT first_unread_pm_at FROM user_stats WHERE user_id = $1",
                )
                .bind(owner)
                .fetch_optional(&mut *self.conn)
                .await?;
                if let Some(Some(at)) = first_unread {
                    wheres.push(format!("(topics.updated_at >= '{}')", sql_time(at)));
                }
            }
            PmList::New => {
                joins.push(format!(
                    "LEFT JOIN dismissed_topic_users ON dismissed_topic_users.topic_id = topics.id \
                     AND dismissed_topic_users.user_id = {owner}"
                ));
                wheres.push(personal);
                wheres.push(self.new_filter().await?);
                if let Some(muted) = self.muted_tags_clause(owner).await? {
                    wheres.push(muted);
                }
                wheres.push("(dismissed_topic_users.id IS NULL)".to_string());
            }
            PmList::Warnings => {
                wheres.push(personal);
                wheres.push("(topics.subtype = 'moderator_warning')".to_string());
                wheres.push(format!("topics.user_id <> {owner}"));
            }
            PmList::Assigned => {
                wheres.push(self.personal_and_groups_clause(owner).await?);
                wheres.push(crate::plugins::assign::lists::assigned_to_user_or_groups(
                    owner,
                ));
            }
            PmList::Group { id, name, view } => {
                let lowered = name.to_lowercase().replace('\'', "''");
                joins.push(format!(
                    "INNER JOIN topic_allowed_groups tag ON tag.topic_id = topics.id \
                     AND tag.group_id IN (SELECT id FROM groups WHERE LOWER(name) = '{lowered}')"
                ));
                if !self.guardian.is_admin() {
                    joins.push(format!(
                        "INNER JOIN group_users gu ON gu.group_id = tag.group_id AND gu.user_id = {owner}"
                    ));
                }
                let publish: bool =
                    sqlx::query_scalar("SELECT publish_read_state FROM groups WHERE id = $1")
                        .bind(id)
                        .fetch_one(&mut *self.conn)
                        .await?;
                if publish {
                    return Err(Unsupported("group message lists with publish_read_state").into());
                }
                match view {
                    GroupView::Inbox => {
                        joins.push(format!(
                            "LEFT JOIN group_archived_messages gm ON gm.topic_id = topics.id AND gm.group_id = {id}"
                        ));
                        wheres.push("(gm.id IS NULL)".to_string());
                    }
                    GroupView::Archive => {
                        joins.push(format!(
                            "INNER JOIN group_archived_messages gm ON gm.topic_id = topics.id AND gm.group_id = {id}"
                        ));
                    }
                    GroupView::New => {
                        joins.push(format!(
                            "LEFT JOIN dismissed_topic_users ON dismissed_topic_users.topic_id = topics.id \
                             AND dismissed_topic_users.user_id = {owner}"
                        ));
                        wheres.push(self.new_filter().await?);
                        wheres.push("(dismissed_topic_users.id IS NULL)".to_string());
                    }
                    GroupView::Unread => {
                        wheres.push(self.unread_filter(viewer, self.settings)?);
                        let first_unread: Option<Option<NaiveDateTime>> = sqlx::query_scalar(
                            "SELECT first_unread_pm_at FROM group_users WHERE user_id = $1 AND group_id = $2",
                        )
                        .bind(owner)
                        .bind(id)
                        .fetch_optional(&mut *self.conn)
                        .await?;
                        if let Some(Some(at)) = first_unread {
                            wheres.push(format!("(topics.updated_at >= '{}')", sql_time(at)));
                        }
                    }
                }
            }
        }
        self.fetch(&joins, &wheres, per_page).await
    }

    /// `list_private_messages_direct_and_groups(owner,
    /// groups_messages_notification_level: :watching)` less the topics the
    /// block excludes: the user menu's messages tab.
    pub async fn direct_and_groups_watching(
        &mut self,
        exclude_topic_ids: &[i32],
    ) -> Result<TopicList, TopicQueryError> {
        let per_page = self
            .options
            .per_page
            .unwrap_or(crate::topic_query::DEFAULT_PER_PAGE);
        let owner = self.owner_id;
        let joins = vec![
            format!(
                "LEFT OUTER JOIN topic_users AS tu ON (topics.id = tu.topic_id AND tu.user_id = {owner})"
            ),
            // not_archived, not_archived_in_groups
            format!(
                "LEFT JOIN user_archived_messages um ON um.user_id = {owner} AND um.topic_id = topics.id"
            ),
            "LEFT OUTER JOIN group_archived_messages ON group_archived_messages.topic_id = topics.id"
                .to_string(),
        ];
        let mut wheres = vec![
            "topics.deleted_at IS NULL".to_string(),
            "topics.archetype = 'private_message'".to_string(),
        ];
        if !self.guardian.is_staff() {
            wheres.push("topics.visible = TRUE".to_string());
        }
        // user_personal_and_groups_private_messages: the owner's message
        // groups (group_with_messages_ids), their topics only where the
        // owner watches them.
        let owner_user = crate::session::current::SessionUser::load(&mut *self.conn, owner)
            .await?
            .ok_or(Unsupported("message list owner without a user row"))?;
        let owner_guardian = Guardian::for_user(&mut *self.conn, &owner_user).await?;
        let groups: Vec<i32> = sqlx::query_scalar(
            "SELECT groups.id FROM groups INNER JOIN group_users ON group_users.group_id = groups.id \
             WHERE group_users.user_id = $1 AND groups.has_messages = TRUE ORDER BY groups.id",
        )
        .bind(owner)
        .fetch_all(&mut *self.conn)
        .await?;
        let moderators = crate::guardian::auto_groups::MODERATORS as i32;
        let groups: Vec<i32> = if owner_guardian.is_admin()
            || owner_guardian.in_setting_groups(self.settings, "personal_message_enabled_groups")?
        {
            groups
        } else if owner_guardian.is_moderator() {
            groups.into_iter().filter(|id| *id == moderators).collect()
        } else {
            Vec::new()
        };
        if groups.is_empty() {
            wheres.push(format!(
                "EXISTS (SELECT 1 FROM topic_allowed_users tau WHERE tau.topic_id = topics.id AND tau.user_id = {owner})"
            ));
        } else {
            let ids = groups
                .iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
                .join(",");
            wheres.push(format!(
                "topics.id IN (SELECT topic_id FROM topic_allowed_users WHERE user_id = {owner} \
                 UNION ALL SELECT topic_allowed_groups.topic_id FROM topic_allowed_groups \
                 INNER JOIN topic_users ON topic_users.topic_id = topic_allowed_groups.topic_id AND topic_users.user_id = {owner} \
                 WHERE group_id IN ({ids}) AND topic_users.notification_level >= 3)"
            ));
        }
        wheres.push("(um.user_id IS NULL)".to_string());
        wheres.push("(group_archived_messages.id IS NULL)".to_string());
        // have_posts_from_others
        wheres.push(format!(
            "(NOT (topics.participant_count = 1 AND topics.user_id = {owner} AND topics.moderator_posts_count = 0))"
        ));
        if !exclude_topic_ids.is_empty() {
            let ids = exclude_topic_ids
                .iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
                .join(",");
            wheres.push(format!("topics.id NOT IN ({ids})"));
        }
        self.fetch(&joins, &wheres, per_page).await
    }

    /// `user_personal_and_groups_private_messages(owner)` without a
    /// notification level: the owner's messages and their message groups'
    /// (`group_with_messages_ids`).
    async fn personal_and_groups_clause(&mut self, owner: i32) -> Result<String, TopicQueryError> {
        let owner_user = crate::session::current::SessionUser::load(&mut *self.conn, owner)
            .await?
            .ok_or(Unsupported("message list owner without a user row"))?;
        let owner_guardian = Guardian::for_user(&mut *self.conn, &owner_user).await?;
        let groups: Vec<i32> = sqlx::query_scalar(
            "SELECT groups.id FROM groups INNER JOIN group_users ON group_users.group_id = groups.id \
             WHERE group_users.user_id = $1 AND groups.has_messages = TRUE ORDER BY groups.id",
        )
        .bind(owner)
        .fetch_all(&mut *self.conn)
        .await?;
        let moderators = crate::guardian::auto_groups::MODERATORS as i32;
        let groups: Vec<i32> = if owner_guardian.is_admin()
            || owner_guardian.in_setting_groups(self.settings, "personal_message_enabled_groups")?
        {
            groups
        } else if owner_guardian.is_moderator() {
            groups.into_iter().filter(|id| *id == moderators).collect()
        } else {
            Vec::new()
        };
        Ok(if groups.is_empty() {
            format!(
                "EXISTS (SELECT 1 FROM topic_allowed_users tau WHERE tau.topic_id = topics.id AND tau.user_id = {owner})"
            )
        } else {
            let ids = groups
                .iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
                .join(",");
            format!(
                "topics.id IN (SELECT topic_id FROM topic_allowed_users WHERE user_id = {owner} \
                 UNION ALL SELECT topic_id FROM topic_allowed_groups WHERE group_id IN ({ids}))"
            )
        })
    }

    /// The list's page in the options' order.
    async fn fetch(
        &mut self,
        joins: &[String],
        wheres: &[String],
        per_page: i64,
    ) -> Result<TopicList, TopicQueryError> {
        let order = crate::topic_query::sortable_order(
            self.options.order.as_deref(),
            self.options.ascending,
            self.settings,
        )?;
        let sql = format!(
            "SELECT {TOPIC_COLUMNS} FROM topics {} WHERE {} ORDER BY {order} LIMIT $1 OFFSET $2",
            joins.join(" "),
            wheres.join(" AND ")
        );
        let topics: Vec<TopicRow> = sqlx::query_as(&sql)
            .bind(per_page)
            .bind(self.options.page * per_page)
            .fetch_all(&mut *self.conn)
            .await?;
        Ok(TopicList {
            filter: "private_messages",
            topics,
            per_page,
            tag_ids: Vec::new(),
            assigned: self.options.assigned.clone(),
        })
    }

    /// `TopicQuery.unread_filter` with the owner's `tu` row; whisperers
    /// (the owner, as Rails passes `user.whisperer?`) compare the staff
    /// count.
    fn unread_filter(
        &self,
        _viewer: i32,
        settings: &SiteSettings,
    ) -> Result<String, TopicQueryError> {
        let column = if self.guardian.is_whisperer(settings)? {
            "highest_staff_post_number"
        } else {
            "highest_post_number"
        };
        Ok(format!(
            "(tu.last_read_post_number < topics.{column}) AND (COALESCE(tu.notification_level, 1) >= 2)"
        ))
    }

    /// `TopicQuery.new_filter` with the owner's new-topic start date.
    async fn new_filter(&mut self) -> Result<String, TopicQueryError> {
        let owner = crate::session::current::SessionUser::load(&mut *self.conn, self.owner_id)
            .await?
            .ok_or(Unsupported("message list owner without a user row"))?;
        let owner_guardian = Guardian::for_user(&mut *self.conn, &owner).await?;
        let start = owner_guardian
            .treat_as_new_topic_start_date(&mut *self.conn, self.settings)
            .await?;
        let start = match start {
            Some(t) => format!("(topics.created_at >= '{}')", sql_time(t)),
            None => "FALSE".to_string(),
        };
        Ok(format!(
            "{start} AND (tu.last_read_post_number IS NULL) AND (COALESCE(tu.notification_level, 2) >= 2)"
        ))
    }

    /// `remove_muted_tags(list, user, skip_categories: true)`
    async fn muted_tags_clause(&mut self, owner: i32) -> Result<Option<String>, TopicQueryError> {
        if !self.settings.get("tagging_enabled")?.truthy()
            || self.settings.get("remove_muted_tags_from_latest")?.to_s() == "never"
        {
            return Ok(None);
        }
        let ids: Vec<i32> = sqlx::query_scalar(
            "SELECT tag_id FROM tag_users WHERE user_id = $1 AND notification_level = 0 ORDER BY tag_id",
        )
        .bind(owner)
        .fetch_all(&mut *self.conn)
        .await?;
        if ids.is_empty() {
            return Ok(None);
        }
        let ids = ids
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(",");
        Ok(Some(
            match self
                .settings
                .get("remove_muted_tags_from_latest")?
                .to_s()
                .as_str()
            {
                "always" => format!(
                    "(NOT EXISTS (SELECT 1 FROM topic_tags tt WHERE tt.tag_id IN ({ids}) AND tt.topic_id = topics.id))"
                ),
                "only_muted" => format!(
                    "(EXISTS (SELECT 1 FROM topic_tags tt WHERE tt.tag_id NOT IN ({ids}) AND tt.topic_id = topics.id) \
                 OR NOT EXISTS (SELECT 1 FROM topic_tags tt WHERE tt.topic_id = topics.id))"
                ),
                _ => return Ok(None),
            },
        ))
    }
}

fn sql_time(t: NaiveDateTime) -> String {
    t.format("%Y-%m-%d %H:%M:%S%.6f").to_string()
}
