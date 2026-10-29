//! What discourse-assign adds to serializers (`add_to_serializer` in
//! plugin.rb): the assignee and the assignments of a topic's posts on the
//! topic view, topic list items, suggested topics, search results and
//! bookmarks; the assignee, note and status on posts; `can_assign` on
//! the current user, topic views, list items and posts.
//!
//! The assignees are shaped as DiscourseAssign::Helpers builds them
//! (`build_assigned_to_user`, `build_assigned_to_group`) or as
//! BasicUserSerializer and AssignedGroupSerializer do, by serializer, as
//! Rails picks per key.

use std::collections::HashMap;

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use super::{Assignment, Permissions};
use crate::guardian::Guardian;
use crate::plugins::PluginError;
use crate::site_settings::SiteSettings;
use crate::url::Urls;

/// An assignee user, with what its serializers read.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct UserRef {
    pub id: i32,
    pub username: String,
    pub name: Option<String>,
    pub uploaded_avatar_id: Option<i32>,
}

/// An assignee group, with what its serializers read.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct GroupRef {
    pub id: i32,
    pub name: String,
    pub flair_bg_color: Option<String>,
    pub flair_color: Option<String>,
    pub flair_icon: Option<String>,
    pub flair_upload_id: Option<i32>,
}

/// The assignees of some assignments, loaded in two queries.
#[derive(Debug, Default)]
pub struct Assignees {
    pub users: HashMap<i32, UserRef>,
    pub groups: HashMap<i32, GroupRef>,
}

/// An assignment's assignee, when it still exists.
pub enum Assignee<'a> {
    User(&'a UserRef),
    Group(&'a GroupRef),
}

impl Assignees {
    pub async fn load(
        conn: &mut PgConnection,
        assignments: &[&Assignment],
    ) -> Result<Self, sqlx::Error> {
        let user_ids: Vec<i32> = assignments
            .iter()
            .filter(|a| a.assigned_to_user())
            .map(|a| a.assigned_to_id)
            .collect();
        let group_ids: Vec<i32> = assignments
            .iter()
            .filter(|a| a.assigned_to_group())
            .map(|a| a.assigned_to_id)
            .collect();
        Self::load_ids(conn, &user_ids, &group_ids).await
    }

    pub async fn load_ids(
        conn: &mut PgConnection,
        user_ids: &[i32],
        group_ids: &[i32],
    ) -> Result<Self, sqlx::Error> {
        let users: Vec<UserRef> = if user_ids.is_empty() {
            Vec::new()
        } else {
            sqlx::query_as(
                "SELECT id, username, name, uploaded_avatar_id FROM users WHERE id = ANY($1)",
            )
            .bind(user_ids)
            .fetch_all(&mut *conn)
            .await?
        };
        let groups: Vec<GroupRef> = if group_ids.is_empty() {
            Vec::new()
        } else {
            sqlx::query_as(
                "SELECT id, name, flair_bg_color, flair_color, flair_icon, flair_upload_id \
                 FROM groups WHERE id = ANY($1)",
            )
            .bind(group_ids)
            .fetch_all(&mut *conn)
            .await?
        };
        Ok(Assignees {
            users: users.into_iter().map(|u| (u.id, u)).collect(),
            groups: groups.into_iter().map(|g| (g.id, g)).collect(),
        })
    }

    pub fn of(&self, assignment: &Assignment) -> Option<Assignee<'_>> {
        if assignment.assigned_to_user() {
            self.users
                .get(&assignment.assigned_to_id)
                .map(Assignee::User)
        } else if assignment.assigned_to_group() {
            self.groups
                .get(&assignment.assigned_to_id)
                .map(Assignee::Group)
        } else {
            None
        }
    }
}

/// What the key builders need beside the data.
pub struct Cx<'a> {
    pub settings: &'a SiteSettings,
    pub urls: &'a Urls<'a>,
    /// `logo_small_url`, for the system user's avatar.
    pub logo: Option<&'a str>,
}

impl Cx<'_> {
    fn avatar(&self, u: &UserRef) -> Result<String, PluginError> {
        Ok(crate::avatar::avatar_template(
            self.urls,
            u.id,
            &u.username,
            u.uploaded_avatar_id,
            self.logo,
        )?)
    }

    /// `Helpers.build_assigned_to_user`: username, name (whatever
    /// enable_names says), avatar.
    pub fn helper_user(&self, u: &UserRef) -> Result<Value, PluginError> {
        Ok(json!({
            "username": u.username,
            "name": u.name,
            "avatar_template": self.avatar(u)?,
        }))
    }

    /// `Helpers.build_assigned_to_group`
    pub fn helper_group(&self, g: &GroupRef) -> Value {
        json!({
            "name": g.name,
            "flair_bg_color": g.flair_bg_color,
            "flair_color": g.flair_color,
            "flair_icon": g.flair_icon,
            "flair_upload_id": g.flair_upload_id,
        })
    }

    /// BasicUserSerializer
    pub fn basic_user(&self, u: &UserRef) -> Result<Value, PluginError> {
        let mut out = Map::new();
        out.insert("id".into(), json!(u.id));
        out.insert("username".into(), json!(u.username));
        if self.settings.get("enable_names")?.truthy() {
            out.insert("name".into(), json!(u.name));
        }
        out.insert("avatar_template".into(), json!(self.avatar(u)?));
        Ok(Value::Object(out))
    }

    /// AssignedGroupSerializer
    pub fn assigned_group(&self, g: &GroupRef) -> Value {
        json!({ "id": g.id, "name": g.name })
    }

    fn helper(&self, assignee: &Assignee<'_>) -> Result<Value, PluginError> {
        match assignee {
            Assignee::User(u) => self.helper_user(u),
            Assignee::Group(g) => Ok(self.helper_group(g)),
        }
    }
}

/// `allowed_access`: assigns_public, or the viewer may assign the topic.
pub fn can_see_assignments(
    settings: &SiteSettings,
    permissions: &Permissions,
    guardian: &Guardian,
    category_id: Option<i32>,
) -> Result<bool, PluginError> {
    Ok(super::public(settings)? || permissions.can_assign_in(guardian, category_id))
}

/// A topic's active assignments: its own (`assigned_to`, with
/// `assignment_status`) and its posts' (`indirectly_assigned_to`).
#[derive(Debug, Default, Clone)]
pub struct TopicAssignments {
    pub direct: Option<Assignment>,
    /// The posts' assignments, with each post's number, by assignment id.
    pub posts: Vec<(Assignment, i32)>,
}

/// The active assignments of many topics, by topic.
pub async fn topic_assignments(
    conn: &mut PgConnection,
    topic_ids: &[i32],
) -> Result<HashMap<i32, TopicAssignments>, sqlx::Error> {
    let mut out: HashMap<i32, TopicAssignments> = HashMap::new();
    if topic_ids.is_empty() {
        return Ok(out);
    }
    // Rails reads the targets through the default scope: a deleted post's
    // assignment is skipped (`next unless assignment.target`).
    let rows: Vec<(Assignment, Option<i32>)> = sqlx::query_as::<_, AssignmentWithPost>(&format!(
        "SELECT {}, p.post_number FROM assignments a \
         LEFT JOIN posts p ON a.target_type = 'Post' AND p.id = a.target_id AND p.deleted_at IS NULL \
         WHERE a.active AND a.topic_id = ANY($1) ORDER BY a.id",
        super::COLUMNS
    ))
    .bind(topic_ids)
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .map(|r| (r.assignment, r.post_number))
    .collect();
    for (assignment, post_number) in rows {
        let entry = out.entry(assignment.topic_id).or_default();
        if assignment.target_type == "Topic" {
            if assignment.target_id == assignment.topic_id && entry.direct.is_none() {
                entry.direct = Some(assignment);
            }
        } else if let Some(post_number) = post_number {
            entry.posts.push((assignment, post_number));
        }
    }
    Ok(out)
}

#[derive(sqlx::FromRow)]
struct AssignmentWithPost {
    #[sqlx(flatten)]
    assignment: Assignment,
    post_number: Option<i32>,
}

/// The assignees a page of topics' assignments name.
pub async fn assignees_of(
    conn: &mut PgConnection,
    topics: &HashMap<i32, TopicAssignments>,
) -> Result<Assignees, sqlx::Error> {
    let mut all: Vec<&Assignment> = Vec::new();
    for t in topics.values() {
        all.extend(t.direct.iter());
        all.extend(t.posts.iter().map(|(a, _)| a));
    }
    Assignees::load(conn, &all).await
}

/// `Helpers.build_indirectly_assigned_to`: by post id, each assignee with
/// its post number, note and status. `with_details`: the topic view's
/// `indirectly_assigned_to` reads the note (and the status while statuses
/// are on); the preloaded lists leave them nil.
pub fn indirectly_assigned_to(
    cx: &Cx<'_>,
    assignees: &Assignees,
    posts: &[(Assignment, i32)],
    with_details: bool,
) -> Result<Value, PluginError> {
    let statuses = super::status_enabled(cx.settings)?;
    let mut out = Map::new();
    for (assignment, post_number) in posts {
        let Some(assignee) = assignees.of(assignment) else {
            continue;
        };
        out.insert(
            assignment.target_id.to_string(),
            json!({
                "assigned_to": cx.helper(&assignee)?,
                "post_number": post_number,
                "assignment_note": if with_details { json!(assignment.note) } else { Value::Null },
                "assignment_status": if with_details && statuses {
                    json!(assignment.status)
                } else {
                    Value::Null
                },
            }),
        );
    }
    Ok(Value::Object(out))
}

/// Which serializer's shapes the list keys take.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ListShape {
    /// TopicListItemSerializer: BasicUser and AssignedGroup.
    ListItem,
    /// SuggestedTopicSerializer: the helpers' shapes.
    Suggested,
    /// SearchTopicListItemSerializer: the helper's user, AssignedGroup.
    Search,
}

/// The assign keys of a topic list item, suggested topic or search
/// result, in plugin.rb's order, for a viewer who may see them.
pub fn list_keys(
    cx: &Cx<'_>,
    assignees: &Assignees,
    topic: &TopicAssignments,
    shape: ListShape,
    out: &mut Map<String, Value>,
) -> Result<(), PluginError> {
    let direct = topic.direct.as_ref().and_then(|a| assignees.of(a));
    let indirect = if topic.posts.iter().any(|(a, _)| assignees.of(a).is_some()) {
        Some(indirectly_assigned_to(cx, assignees, &topic.posts, false)?)
    } else {
        None
    };
    let user = |u: &UserRef| -> Result<Value, PluginError> {
        match shape {
            ListShape::ListItem => cx.basic_user(u),
            ListShape::Suggested | ListShape::Search => cx.helper_user(u),
        }
    };
    let group = |g: &GroupRef| -> Value {
        match shape {
            ListShape::ListItem | ListShape::Search => cx.assigned_group(g),
            ListShape::Suggested => cx.helper_group(g),
        }
    };
    if shape == ListShape::ListItem {
        // indirectly_assigned_to, assigned_to_user, assigned_to_group,
        // assignment_status
        if let Some(indirect) = &indirect {
            out.insert("indirectly_assigned_to".into(), indirect.clone());
        }
    }
    match &direct {
        Some(Assignee::User(u)) => {
            out.insert("assigned_to_user".into(), user(u)?);
        }
        Some(Assignee::Group(g)) => {
            out.insert("assigned_to_group".into(), group(g));
        }
        None => {}
    }
    match shape {
        ListShape::ListItem => {
            // preload_assignment_status: the direct assignment's status,
            // whatever enable_assign_status says, gated here by it.
            let status = topic
                .direct
                .as_ref()
                .and_then(|a| a.status.as_ref())
                .filter(|s| !crate::ruby::is_blank(s));
            if super::status_enabled(cx.settings)?
                && let Some(status) = status
            {
                out.insert("assignment_status".into(), json!(status));
            }
        }
        ListShape::Suggested | ListShape::Search => {
            if let Some(indirect) = indirect {
                out.insert("indirectly_assigned_to".into(), indirect);
            }
        }
    }
    Ok(())
}

/// The current user's keys: `never_auto_track_topics`, `can_assign`,
/// `can_assign_globally`.
pub async fn current_user_keys(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
    out: &mut Map<String, Value>,
) -> Result<(), PluginError> {
    let Some(user_id) = guardian.user_id() else {
        return Ok(());
    };
    let own: Option<Option<i32>> = sqlx::query_scalar(
        "SELECT auto_track_topics_after_msecs FROM user_options WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?;
    let msecs = match own.flatten() {
        Some(msecs) => i64::from(msecs),
        None => settings
            .get("default_other_auto_track_topics_after_msecs")?
            .to_i(),
    };
    let permissions = Permissions::load(&mut *conn, settings).await?;
    out.insert("never_auto_track_topics".into(), json!(msecs < 0));
    out.insert("can_assign".into(), json!(permissions.can_assign(guardian)));
    out.insert(
        "can_assign_globally".into(),
        json!(permissions.can_assign_globally(guardian)),
    );
    Ok(())
}

/// UserOptionSerializer's and CurrentUserOptionSerializer's
/// `notification_level_when_assigned`: the enum's name.
pub async fn user_option_keys(
    conn: &mut PgConnection,
    user_id: i32,
    out: &mut Map<String, Value>,
) -> Result<(), PluginError> {
    let level: Option<i32> = sqlx::query_scalar(
        "SELECT notification_level_when_assigned FROM user_options WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?;
    let name = level.and_then(|l| {
        crate::user_updater::NOTIFICATION_LEVELS_WHEN_ASSIGNED
            .iter()
            .find(|(_, v)| *v == i64::from(l))
            .map(|(n, _)| *n)
    });
    out.insert("notification_level_when_assigned".into(), json!(name));
    Ok(())
}

/// The assign keys for a topic view and its posts, loaded once per page.
pub struct TopicView {
    pub permissions: Permissions,
    /// `scope.can_assign?(topic)` (the posts share the topic's category).
    pub can_assign: bool,
    /// `assigns_public || can_assign`
    pub visible: bool,
    /// `topic.assignment`, active or not.
    pub assignment: Option<Assignment>,
    /// The active assignments of the topic's posts.
    pub topic: TopicAssignments,
    /// The posts' assignments (`post.assignment`), active or not, by post.
    pub posts: HashMap<i32, Assignment>,
    pub assignees: Assignees,
}

impl TopicView {
    /// None while the plugin is off.
    pub async fn load(
        conn: &mut PgConnection,
        settings: &SiteSettings,
        guardian: &Guardian,
        topic_id: i32,
        category_id: Option<i32>,
        post_ids: &[i32],
    ) -> Result<Option<Self>, PluginError> {
        if !super::enabled(settings)? {
            return Ok(None);
        }
        let permissions = Permissions::load(&mut *conn, settings).await?;
        let can_assign = permissions.can_assign_in(guardian, category_id);
        let visible = super::public(settings)? || can_assign;
        let assignment = Assignment::for_target(&mut *conn, "Topic", topic_id).await?;
        let topic = topic_assignments(&mut *conn, &[topic_id])
            .await?
            .remove(&topic_id)
            .unwrap_or_default();
        let posts: Vec<Assignment> = sqlx::query_as(&format!(
            "SELECT {} FROM assignments a WHERE a.target_type = 'Post' AND a.target_id = ANY($1) \
             ORDER BY a.id",
            super::COLUMNS
        ))
        .bind(post_ids)
        .fetch_all(&mut *conn)
        .await?;
        let mut all: Vec<&Assignment> = posts.iter().collect();
        all.extend(assignment.iter());
        all.extend(topic.posts.iter().map(|(a, _)| a));
        let assignees = Assignees::load(&mut *conn, &all).await?;
        let mut by_post = HashMap::new();
        for a in posts {
            by_post.entry(a.target_id).or_insert(a);
        }
        Ok(Some(TopicView {
            permissions,
            can_assign,
            visible,
            assignment,
            topic,
            posts: by_post,
            assignees,
        }))
    }

    /// TopicViewSerializer's keys, in plugin.rb's order.
    pub fn topic_keys(
        &self,
        cx: &Cx<'_>,
        guardian: &Guardian,
        out: &mut Map<String, Value>,
    ) -> Result<(), PluginError> {
        if guardian.is_authenticated() {
            out.insert("can_assign".into(), json!(self.can_assign));
        }
        if !self.visible {
            return Ok(());
        }
        // `topic.assigned_to`: the topic's assignment while active.
        let direct = self
            .assignment
            .as_ref()
            .filter(|a| a.active)
            .and_then(|a| self.assignees.of(a));
        match direct {
            Some(Assignee::User(u)) => {
                out.insert("assigned_to_user".into(), cx.helper_user(u)?);
            }
            Some(Assignee::Group(g)) => {
                out.insert("assigned_to_group".into(), cx.helper_group(g));
            }
            None => {}
        }
        if self
            .topic
            .posts
            .iter()
            .any(|(a, _)| self.assignees.of(a).is_some())
        {
            out.insert(
                "indirectly_assigned_to".into(),
                indirectly_assigned_to(cx, &self.assignees, &self.topic.posts, true)?,
            );
        }
        if let Some(a) = &self.assignment {
            out.insert("assignment_note".into(), json!(a.note));
        }
        // `topic.assignment_status`: while statuses are on and it is active.
        if super::status_enabled(cx.settings)?
            && let Some(status) = self
                .assignment
                .as_ref()
                .filter(|a| a.active)
                .and_then(|a| a.status.as_ref())
                .filter(|s| !crate::ruby::is_blank(s))
        {
            out.insert("assignment_status".into(), json!(status));
        }
        Ok(())
    }

    /// PostSerializer's keys for one post, in plugin.rb's order.
    pub fn post_keys(
        &self,
        cx: &Cx<'_>,
        guardian: &Guardian,
        post_id: i32,
        out: &mut Map<String, Value>,
    ) -> Result<(), PluginError> {
        if guardian.is_authenticated() {
            out.insert("can_assign".into(), json!(self.can_assign));
        }
        if !self.visible {
            return Ok(());
        }
        let Some(a) = self.posts.get(&post_id) else {
            return Ok(());
        };
        match self.assignees.of(a).filter(|_| a.active) {
            Some(Assignee::User(u)) => {
                out.insert("assigned_to_user".into(), cx.basic_user(u)?);
            }
            Some(Assignee::Group(g)) => {
                out.insert("assigned_to_group".into(), cx.assigned_group(g));
            }
            None => {}
        }
        out.insert("assignment_note".into(), json!(a.note));
        if super::status_enabled(cx.settings)? {
            out.insert("assignment_status".into(), json!(a.status));
        }
        Ok(())
    }
}

/// A page of topics' assign keys (TopicList.on_preload and the list item
/// serializers): None while the plugin is off.
#[derive(Debug)]
pub struct ListData {
    pub permissions: Permissions,
    pub topics: HashMap<i32, TopicAssignments>,
    pub assignees: Assignees,
}

impl ListData {
    pub async fn load(
        conn: &mut PgConnection,
        settings: &SiteSettings,
        guardian: &Guardian,
        topic_ids: &[i32],
    ) -> Result<Option<Self>, PluginError> {
        if !super::enabled(settings)? {
            return Ok(None);
        }
        let permissions = Permissions::load(&mut *conn, settings).await?;
        // allowed_access: assigns_public or the viewer may assign somewhere;
        // the keys' own condition is per topic.
        let allowed = super::public(settings)? || permissions.can_assign(guardian);
        let topics = if allowed {
            topic_assignments(&mut *conn, topic_ids).await?
        } else {
            HashMap::new()
        };
        let assignees = assignees_of(&mut *conn, &topics).await?;
        Ok(Some(ListData {
            permissions,
            topics,
            assignees,
        }))
    }

    /// A topic's keys: `can_assign` for a user (list items only), then the
    /// assignees when the viewer may see them.
    pub fn keys(
        &self,
        cx: &Cx<'_>,
        guardian: &Guardian,
        topic_id: i32,
        category_id: Option<i32>,
        shape: ListShape,
        out: &mut Map<String, Value>,
    ) -> Result<(), PluginError> {
        let can_assign = self.permissions.can_assign_in(guardian, category_id);
        if shape == ListShape::ListItem && guardian.is_authenticated() {
            out.insert("can_assign".into(), json!(can_assign));
        }
        if !(super::public(cx.settings)? || can_assign) {
            return Ok(());
        }
        if let Some(topic) = self.topics.get(&topic_id) {
            list_keys(cx, &self.assignees, topic, shape, out)?;
        }
        Ok(())
    }
}

/// GroupShowSerializer's keys: `assignment_count` (the active assignments
/// of live topics to the group or its members) for who may assign
/// anywhere, `assignable_level`, `can_show_assigned_tab`.
pub async fn group_show_keys(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
    group_id: i32,
    out: &mut Map<String, Value>,
) -> Result<(), PluginError> {
    if !super::enabled(settings)? {
        return Ok(());
    }
    let permissions = Permissions::load(&mut *conn, settings).await?;
    if permissions.can_assign_globally(guardian) {
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM topics JOIN assignments a \
               ON topics.id = a.topic_id AND a.assigned_to_id IS NOT NULL \
             WHERE a.active AND ((a.assigned_to_type = 'User' AND a.assigned_to_id IN \
                 (SELECT group_users.user_id FROM group_users WHERE group_id = $1)) \
               OR (a.assigned_to_type = 'Group' AND a.assigned_to_id = $1)) \
               AND topics.deleted_at IS NULL",
        )
        .bind(group_id)
        .fetch_one(&mut *conn)
        .await?;
        out.insert("assignment_count".into(), json!(count));
    }
    let level: i32 = sqlx::query_scalar("SELECT assignable_level FROM groups WHERE id = $1")
        .bind(group_id)
        .fetch_one(&mut *conn)
        .await?;
    out.insert("assignable_level".into(), json!(level));
    out.insert(
        "can_show_assigned_tab".into(),
        json!(super::can_show_assigned_tab(level)),
    );
    Ok(())
}

/// UserBookmarkBaseSerializer's keys for a topic or post bookmark: the
/// assignee of the topic (or of the post) while active, for viewers who
/// may see assignments in the topic's category.
#[allow(clippy::too_many_arguments)]
pub async fn bookmark_keys(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    guardian: &Guardian,
    is_topic: bool,
    topic_id: i32,
    post_id: i32,
    category_id: Option<i32>,
    out: &mut Map<String, Value>,
) -> Result<(), PluginError> {
    if !super::enabled(cx.settings)? {
        return Ok(());
    }
    let permissions = Permissions::load(&mut *conn, cx.settings).await?;
    if !can_see_assignments(cx.settings, &permissions, guardian, category_id)? {
        return Ok(());
    }
    let (target_type, target_id) = if is_topic {
        ("Topic", topic_id)
    } else {
        ("Post", post_id)
    };
    let Some(assignment) = Assignment::for_target(&mut *conn, target_type, target_id)
        .await?
        .filter(|a| a.active)
    else {
        return Ok(());
    };
    let assignees = Assignees::load(&mut *conn, &[&assignment]).await?;
    match assignees.of(&assignment) {
        Some(Assignee::User(u)) => {
            out.insert("assigned_to_user".into(), cx.basic_user(u)?);
        }
        Some(Assignee::Group(g)) => {
            out.insert("assigned_to_group".into(), cx.assigned_group(g));
        }
        None => {}
    }
    Ok(())
}

/// TopicListSerializer's `assigned_messages_count` for a list filtered by
/// `assigned`, shown to who may assign anywhere or to the user named: the
/// viewer's messages (theirs and their message groups') assigned to them
/// or one of their groups (`private_messages_assigned_query`).
pub async fn assigned_messages_count(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
    assigned: Option<&str>,
) -> Result<Option<i64>, PluginError> {
    let Some(assigned) = assigned.filter(|a| !crate::ruby::is_blank(a)) else {
        return Ok(None);
    };
    if !super::enabled(settings)? {
        return Ok(None);
    }
    let Some(user) = guardian.user() else {
        return Ok(None);
    };
    let permissions = Permissions::load(&mut *conn, settings).await?;
    if !permissions.can_assign_globally(guardian)
        && assigned.to_lowercase() != user.username.to_lowercase()
    {
        return Ok(None);
    }
    // group_with_messages_ids
    let groups = if guardian.is_admin()
        || guardian
            .in_setting_groups(settings, "personal_message_enabled_groups")
            .map_err(PluginError::Setting)?
    {
        "SELECT groups.id FROM groups JOIN group_users ON group_users.group_id = groups.id \
         WHERE group_users.user_id = $1 AND groups.has_messages"
    } else if guardian.is_moderator() {
        "SELECT groups.id FROM groups JOIN group_users ON group_users.group_id = groups.id \
         WHERE group_users.user_id = $1 AND groups.has_messages AND groups.id = 2"
    } else {
        "SELECT NULL::int"
    };
    let visible = if guardian.is_staff() {
        ""
    } else {
        " AND topics.visible"
    };
    let count: i64 = sqlx::query_scalar(&format!(
        "SELECT count(*) FROM topics WHERE topics.archetype = 'private_message' \
           AND topics.deleted_at IS NULL{visible} \
           AND topics.id IN (SELECT topic_id FROM topic_allowed_users WHERE user_id = $1 \
             UNION ALL SELECT topic_id FROM topic_allowed_groups WHERE group_id IN ({groups})) \
           AND {}",
        super::lists::assigned_to_user_or_groups(user.id)
    ))
    .bind(user.id)
    .fetch_one(&mut *conn)
    .await?;
    Ok(Some(count))
}
