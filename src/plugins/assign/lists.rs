//! discourse-assign's topic lists: AssignController#assigned's topics
//! (AssignedTopicSerializer), and the TopicQuery additions the list
//! routes use (`list_messages_assigned`, `group_topics_assigned_results`,
//! `private_messages_assigned_query`, the `assigned` filter).

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use super::keys::{Assignees, Cx};
use crate::guardian::Guardian;
use crate::site_settings::SiteSettings;
use crate::topic_list::{TopicListSerializer, time_json};
use crate::url::Urls;
use crate::{AppError, AppState};

/// AssignController#assigned: topics with a topic assignment (active or
/// not), by assignee id then bump, each as AssignedTopicSerializer with
/// its active assignee.
pub async fn assigned_topics(
    conn: &mut PgConnection,
    state: &AppState,
    settings: &SiteSettings,
    urls: &Urls<'_>,
    guardian: &Guardian,
    offset: i64,
    limit: i64,
) -> Result<Vec<Value>, AppError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        id: i32,
        title: String,
        fancy_title: Option<String>,
        slug: Option<String>,
        posts_count: i32,
        excerpt: Option<String>,
        category_id: Option<i32>,
        archetype: String,
        created_at: chrono::NaiveDateTime,
        updated_at: chrono::NaiveDateTime,
        user_id: Option<i32>,
        assignment_id: i64,
    }
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT topics.id, topics.title, topics.fancy_title, topics.slug, topics.posts_count, \
                topics.excerpt, topics.category_id, topics.archetype, topics.created_at, \
                topics.updated_at, topics.user_id, a.id AS assignment_id \
         FROM topics JOIN assignments a ON a.target_id = topics.id AND a.target_type = 'Topic' \
           AND a.assigned_to_id IS NOT NULL \
         WHERE topics.deleted_at IS NULL \
         ORDER BY a.assigned_to_id, topics.bumped_at DESC OFFSET $1 LIMIT $2",
    )
    .bind(offset)
    .bind(limit)
    .fetch_all(&mut *conn)
    .await?;
    let mut assignments = Vec::new();
    for row in &rows {
        if let Some(a) = super::Assignment::find(&mut *conn, row.assignment_id).await? {
            assignments.push(a);
        }
    }
    let refs: Vec<&super::Assignment> = assignments.iter().collect();
    let assignees = Assignees::load(&mut *conn, &refs).await?;
    let authors: Vec<i32> = rows.iter().filter_map(|r| r.user_id).collect();
    let authors = Assignees::load_ids(&mut *conn, &authors, &[]).await?;
    let logo = crate::admin_users::logo_small_url(&mut *conn, settings).await?;
    let cx = Cx {
        settings,
        urls,
        logo: logo.as_deref(),
    };
    let mut out = Vec::new();
    for row in rows {
        let assignment = assignments.iter().find(|a| a.id == row.assignment_id);
        let mut t = Map::new();
        t.insert("id".into(), json!(row.id));
        t.insert("title".into(), json!(row.title));
        t.insert(
            "fancy_title".into(),
            json!(row.fancy_title.clone().unwrap_or_else(|| row.title.clone())),
        );
        t.insert("slug".into(), json!(row.slug));
        t.insert("posts_count".into(), json!(row.posts_count));
        if settings.get("tagging_enabled")?.truthy()
            && (row.archetype != "private_message" || guardian.can_tag_pms(settings)?)
        {
            let (tags, descriptions) = TopicListSerializer {
                conn: &mut *conn,
                settings,
                i18n: &state.i18n,
                guardian,
                urls,
                more_topics_url: None,
                category_id: None,
                group_id: None,
                prefetched: Default::default(),
            }
            .tags(row.id)
            .await?;
            t.insert("tags".into(), tags);
            t.insert("tags_descriptions".into(), descriptions);
        }
        t.insert("excerpt".into(), json!(row.excerpt));
        t.insert("category_id".into(), json!(row.category_id));
        t.insert("created_at".into(), json!(time_json(row.created_at)));
        t.insert("updated_at".into(), json!(time_json(row.updated_at)));
        if let Some(a) = assignment.filter(|a| a.active) {
            match assignees.of(a) {
                Some(super::keys::Assignee::User(u)) => {
                    t.insert("assigned_to_user".into(), cx.basic_user(u)?);
                }
                Some(super::keys::Assignee::Group(g)) => {
                    let group = crate::groups::find_basic(&mut *conn, None, Some(g.id)).await?;
                    if let Some(group) = group {
                        t.insert(
                            "assigned_to_group".into(),
                            group.json(&state.i18n, guardian, settings, None)?,
                        );
                    }
                }
                None => {}
            }
        }
        let user = row
            .user_id
            .and_then(|id| authors.users.get(&id))
            .map(|u| cx.basic_user(u))
            .transpose()?;
        t.insert("user".into(), user.unwrap_or(Value::Null));
        out.push(Value::Object(t));
    }
    Ok(out)
}

/// Whose assignments a TopicQuery list shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignedList {
    /// `list_messages_assigned(user)`: the user's, and (unless direct) the
    /// groups' they are in.
    User { id: i32, direct: bool },
    /// `group_topics_assigned_results(group)`: the group's, and (unless
    /// direct) its members', on live topics and posts.
    Group { id: i32, direct: bool },
}

impl AssignedList {
    /// The list's clause over `topics`.
    pub fn clause(&self) -> String {
        match *self {
            AssignedList::User { id, direct } => {
                let groups = if direct {
                    ""
                } else {
                    "OR (assigned_to_id IN (group_users.group_id) AND assigned_to_type = 'Group' AND active)"
                };
                format!(
                    "topics.id IN (SELECT topic_id FROM assignments \
                     LEFT JOIN group_users ON group_users.user_id = {id} \
                     WHERE ((assigned_to_id = {id} AND assigned_to_type = 'User' AND active){groups}))"
                )
            }
            AssignedList::Group { id, direct } => {
                let members = if direct {
                    String::new()
                } else {
                    format!(
                        " OR (a.assigned_to_id IN (SELECT user_id from group_users where group_id = {id}) \
                         AND a.assigned_to_type = 'User')"
                    )
                };
                format!(
                    "topics.id IN (SELECT a.topic_id FROM assignments a \
                     LEFT JOIN topics t ON t.id = a.topic_id \
                     LEFT JOIN posts p ON p.id = a.target_id AND a.target_type = 'Post' \
                     WHERE a.active AND t.deleted_at IS NULL \
                       AND (a.target_type = 'Topic' OR (a.target_type = 'Post' AND p.deleted_at IS NULL \
                            AND p.deleted_by_id IS NULL AND p.user_deleted = false)) \
                       AND ((a.assigned_to_id = {id} AND a.assigned_to_type = 'Group'){members}))"
                )
            }
        }
    }
}

/// `private_messages_assigned_query`'s assignment clause: assigned,
/// active, to the user or one of their groups.
pub fn assigned_to_user_or_groups(user_id: i32) -> String {
    format!(
        "topics.id IN (SELECT topic_id FROM assignments WHERE active AND \
         ((assigned_to_id = {user_id} AND assigned_to_type = 'User') OR \
          (assigned_to_id IN (SELECT group_id FROM group_users WHERE user_id = {user_id}) \
           AND assigned_to_type = 'Group')))"
    )
}
