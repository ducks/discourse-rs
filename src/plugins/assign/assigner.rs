//! `Assigner` (lib/assigner.rb): assigning a topic or a post to a user or
//! a group, with the checks that refuse it (`forbidden_reasons`), the
//! small action or whisper that records it, the notification jobs, the
//! assignee's topic tracking, the live updates and the web hooks; and
//! unassigning. The caller holds the transaction.
//!
//! Refused rather than ported: inviting the assignee to a message
//! (invite_on_assign) when they are not in it yet, the assignment email
//! (assign_mailer), and the discourse-workflows triggers on assigned and
//! unassigned, each when it would act.

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use super::keys::{Assignees, Cx, UserRef};
use super::{Assignment, Permissions, Scope};
use crate::guardian::Guardian;
use crate::posting::Ctx;
use crate::posting::small_action::{self, SmallAction};
use crate::session::current::SessionUser;
use crate::site_settings::SiteSettings;
use crate::topic_guardian::{PostCtx, TopicCtx};
use crate::{AppError, Unsupported};

/// `Assigner::ASSIGNMENTS_PER_TOPIC_LIMIT`
pub const ASSIGNMENTS_PER_TOPIC_LIMIT: i64 = 5;

/// What can be assigned: a topic or one of its posts, loaded through the
/// default scope (not trashed).
#[derive(Debug, Clone)]
pub struct Target {
    pub is_post: bool,
    pub id: i32,
    pub topic_id: i32,
    /// The post's number; 1 for a topic.
    pub post_number: i32,
    pub category_id: Option<i32>,
    pub archetype: String,
    pub topic_title: String,
}

impl Target {
    /// `target_type.constantize.find_by(id:)`: "Topic" or "Post", any
    /// case (`Assignment.valid_type?`); None for another type or a target
    /// that is gone.
    pub async fn load(
        conn: &mut PgConnection,
        target_type: &str,
        id: i32,
    ) -> Result<Option<Target>, sqlx::Error> {
        let row: Option<(i32, i32, i32, Option<i32>, String, String)> =
            match target_type.to_lowercase().as_str() {
                "topic" => {
                    sqlx::query_as(
                        "SELECT t.id, t.id, 1, t.category_id, t.archetype, t.title FROM topics t \
                         WHERE t.id = $1 AND t.deleted_at IS NULL",
                    )
                    .bind(id)
                    .fetch_optional(conn)
                    .await?
                }
                "post" => {
                    sqlx::query_as(
                        "SELECT p.id, t.id, p.post_number, t.category_id, t.archetype, t.title \
                         FROM posts p JOIN topics t ON t.id = p.topic_id AND t.deleted_at IS NULL \
                         WHERE p.id = $1 AND p.deleted_at IS NULL",
                    )
                    .bind(id)
                    .fetch_optional(conn)
                    .await?
                }
                _ => return Ok(None),
            };
        let is_post = target_type.eq_ignore_ascii_case("post");
        Ok(row.map(
            |(id, topic_id, post_number, category_id, archetype, topic_title)| Target {
                is_post,
                id,
                topic_id,
                post_number,
                category_id,
                archetype,
                topic_title,
            },
        ))
    }

    pub async fn topic(
        conn: &mut PgConnection,
        topic_id: i32,
    ) -> Result<Option<Target>, sqlx::Error> {
        Self::load(conn, "Topic", topic_id).await
    }

    pub fn type_name(&self) -> &'static str {
        if self.is_post { "Post" } else { "Topic" }
    }

    pub fn scope(&self) -> Scope {
        Scope::Category(self.category_id)
    }

    pub fn private_message(&self) -> bool {
        self.archetype == "private_message"
    }

    /// `guardian.can_see?(target)`
    pub async fn visible_to(
        &self,
        conn: &mut PgConnection,
        settings: &SiteSettings,
        guardian: &Guardian,
    ) -> Result<bool, AppError> {
        let Some(topic) = TopicCtx::load(&mut *conn, settings, guardian, self.topic_id).await?
        else {
            return Ok(false);
        };
        let secure = guardian.secure_category_ids(&mut *conn, settings).await?;
        let topic_can_see = guardian.can_see_topic(settings, &topic, true, &secure)?;
        if !self.is_post {
            return Ok(topic_can_see);
        }
        let Some(post) = PostCtx::load(&mut *conn, self.id).await? else {
            return Ok(false);
        };
        Ok(guardian.can_see_post(settings, &post, topic_can_see)?)
    }
}

/// Who a target is assigned to.
#[derive(Debug, Clone)]
pub enum Assignee {
    User(UserRef),
    Group { id: i32, name: String },
}

impl Assignee {
    pub fn id(&self) -> i32 {
        match self {
            Assignee::User(u) => u.id,
            Assignee::Group { id, .. } => *id,
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Assignee::User(_) => "User",
            Assignee::Group { .. } => "Group",
        }
    }

    /// The username or the group's name.
    pub fn name(&self) -> &str {
        match self {
            Assignee::User(u) => &u.username,
            Assignee::Group { name, .. } => name,
        }
    }

    pub async fn user(conn: &mut PgConnection, id: i32) -> Result<Option<Assignee>, sqlx::Error> {
        let user: Option<UserRef> = sqlx::query_as(
            "SELECT id, username, name, uploaded_avatar_id FROM users WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(conn)
        .await?;
        Ok(user.map(Assignee::User))
    }

    /// `User.find_by_username`
    pub async fn by_username(
        conn: &mut PgConnection,
        username: &str,
    ) -> Result<Option<Assignee>, sqlx::Error> {
        let user: Option<UserRef> = sqlx::query_as(
            "SELECT id, username, name, uploaded_avatar_id FROM users WHERE username_lower = $1",
        )
        .bind(username.to_lowercase())
        .fetch_optional(conn)
        .await?;
        Ok(user.map(Assignee::User))
    }

    pub async fn of(
        conn: &mut PgConnection,
        assigned_to_type: &str,
        assigned_to_id: i32,
    ) -> Result<Option<Assignee>, sqlx::Error> {
        if assigned_to_type == "Group" {
            let name: Option<String> = sqlx::query_scalar("SELECT name FROM groups WHERE id = $1")
                .bind(assigned_to_id)
                .fetch_optional(conn)
                .await?;
            Ok(name.map(|name| Assignee::Group {
                id: assigned_to_id,
                name,
            }))
        } else {
            Self::user(conn, assigned_to_id).await
        }
    }
}

/// Why `AssigneeResolver.resolve!` raised.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Resolve {
    /// Neither a username nor a group name (`InvalidParameters(:assignee)`).
    InvalidParameters,
    NotFound,
    /// A group the guardian may not see.
    InvalidAccess,
}

/// `DiscourseAssign::AssigneeResolver.resolve!`: the user by username,
/// else the group by name (any case) the guardian may see.
pub async fn resolve(
    conn: &mut PgConnection,
    guardian: &Guardian,
    username: Option<String>,
    group_name: Option<String>,
) -> Result<Result<Assignee, Resolve>, AppError> {
    let username = username
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty());
    let group_name = group_name
        .map(|g| g.trim().to_string())
        .filter(|g| !g.is_empty());
    if let Some(username) = username {
        return Ok(Assignee::by_username(&mut *conn, &username)
            .await?
            .ok_or(Resolve::NotFound));
    }
    let Some(group_name) = group_name else {
        return Ok(Err(Resolve::InvalidParameters));
    };
    let group: Option<(i32, String, i32)> = sqlx::query_as(
        "SELECT id, name, visibility_level FROM groups WHERE LOWER(name) = $1 ORDER BY id LIMIT 1",
    )
    .bind(group_name.to_lowercase())
    .fetch_optional(&mut *conn)
    .await?;
    let Some((id, name, visibility)) = group else {
        return Ok(Err(Resolve::NotFound));
    };
    // ensure_can_see_group!: its visibility level alone.
    if !crate::groups::can_see_group_and_members(&mut *conn, guardian, id, visibility, 0).await? {
        return Ok(Err(Resolve::InvalidAccess));
    }
    Ok(Ok(Assignee::Group { id, name }))
}

/// Why an assignment was refused: the `discourse_assign.<reason>` key.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Reason {
    NoAssignee,
    ForbiddenAssignerNotAllowed,
    ForbiddenAssigneeNotPmParticipant,
    ForbiddenAssigneeCantSeeTopic,
    ForbiddenGroupAssigneeNotPmParticipant,
    ForbiddenGroupAssigneeCantSeeTopic,
    ForbiddenAssignTo,
    ForbiddenGroupAssignTo,
    AssignmentNoteTooLong,
    AlreadyAssigned,
    GroupAlreadyAssigned,
    TooManyAssignsForTopic,
    TooManyAssigns,
    /// The status isn't one of assign_statuses (Assignment's validation,
    /// a RecordInvalid in Rails).
    InvalidStatus,
}

impl Reason {
    pub fn key(self) -> &'static str {
        match self {
            Reason::NoAssignee => "no_assignee",
            Reason::ForbiddenAssignerNotAllowed => "forbidden_assigner_not_allowed",
            Reason::ForbiddenAssigneeNotPmParticipant => "forbidden_assignee_not_pm_participant",
            Reason::ForbiddenAssigneeCantSeeTopic => "forbidden_assignee_cant_see_topic",
            Reason::ForbiddenGroupAssigneeNotPmParticipant => {
                "forbidden_group_assignee_not_pm_participant"
            }
            Reason::ForbiddenGroupAssigneeCantSeeTopic => "forbidden_group_assignee_cant_see_topic",
            Reason::ForbiddenAssignTo => "forbidden_assign_to",
            Reason::ForbiddenGroupAssignTo => "forbidden_group_assign_to",
            Reason::AssignmentNoteTooLong => "assignment_note_too_long",
            Reason::AlreadyAssigned => "already_assigned",
            Reason::GroupAlreadyAssigned => "group_already_assigned",
            Reason::TooManyAssignsForTopic => "too_many_assigns_for_topic",
            Reason::TooManyAssigns => "too_many_assigns",
            Reason::InvalidStatus => "invalid_status",
        }
    }
}

/// `Assigner.failure_message(reason, assign_to)`
pub fn failure_message(
    i18n: &crate::i18n::I18n,
    settings: &SiteSettings,
    reason: Reason,
    assign_to: Option<&Assignee>,
) -> Result<String, AppError> {
    let name = assign_to.map(Assignee::name).unwrap_or("");
    let limit = ASSIGNMENTS_PER_TOPIC_LIMIT.to_string();
    let max = settings.get("max_assigned_topics")?.to_s();
    let note_max = settings.get("max_post_length")?.to_s();
    let key = format!("discourse_assign.{}", reason.key());
    Ok(i18n
        .t_with(
            &key,
            &[
                ("username", name),
                ("group", name),
                ("limit", &limit),
                ("max", &max),
                ("note_max", &note_max),
            ],
        )
        .unwrap_or_else(|| format!("Translation missing: en.{key}")))
}

/// `assign`'s options.
#[derive(Debug, Clone)]
pub struct AssignOpts {
    pub note: Option<String>,
    pub status: Option<String>,
    pub skip_small_action_post: bool,
    pub allow_self_reassign: bool,
    pub should_notify: bool,
}

impl Default for AssignOpts {
    fn default() -> Self {
        AssignOpts {
            note: None,
            status: None,
            skip_small_action_post: false,
            allow_self_reassign: false,
            should_notify: true,
        }
    }
}

/// `Assigner.new(target, user)`
pub struct Assigner<'a> {
    pub ctx: &'a Ctx<'a>,
    pub target: &'a Target,
    /// The user assigning, and their guardian.
    pub by: &'a SessionUser,
    pub guardian: &'a Guardian,
    pub permissions: &'a Permissions,
}

/// A guardian for another user (`Guardian.new(user)`).
pub async fn guardian_for(
    conn: &mut PgConnection,
    user_id: i32,
) -> Result<Option<Guardian>, AppError> {
    let Some(user) = SessionUser::load(&mut *conn, user_id).await? else {
        return Ok(None);
    };
    Ok(Some(Guardian::for_user(&mut *conn, &user).await?))
}

/// `Guardian.new(Discourse.system_user)` with the user row.
pub async fn system_user(conn: &mut PgConnection) -> Result<(SessionUser, Guardian), AppError> {
    let user = SessionUser::load(&mut *conn, -1)
        .await?
        .ok_or(Unsupported("a site without its system user"))?;
    let guardian = Guardian::for_user(&mut *conn, &user).await?;
    Ok((user, guardian))
}

/// discourse-workflows' triggers on assign events aren't ported: an
/// active workflow with one is refused.
async fn workflows_unsupported(conn: &mut PgConnection, trigger: &str) -> Result<(), AppError> {
    let workflows: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM discourse_workflows_workflows \
         WHERE active_version_id IS NOT NULL AND nodes::text LIKE $1)",
    )
    .bind(format!("%{trigger}%"))
    .fetch_one(conn)
    .await?;
    if workflows {
        return Err(Unsupported("discourse-workflows triggers on assignments").into());
    }
    Ok(())
}

impl Assigner<'_> {
    fn settings(&self) -> &SiteSettings {
        self.ctx.settings
    }

    /// `topic.all_allowed_users`: a message's users, its groups' members,
    /// and its moderators when it is flagged or an official warning.
    async fn private_message_allowed_user_ids(
        &self,
        conn: &mut PgConnection,
    ) -> Result<Vec<i32>, AppError> {
        let min_score = crate::reviewables::min_score(&mut *conn, self.settings()).await?;
        Ok(sqlx::query_scalar(
            "SELECT user_id FROM topic_allowed_users WHERE topic_id = $1 \
             UNION SELECT gu.user_id FROM topic_allowed_groups tg \
               JOIN group_users gu ON gu.group_id = tg.group_id WHERE tg.topic_id = $1 \
             UNION SELECT u.id FROM users u WHERE u.moderator AND EXISTS (\
               SELECT 1 FROM topics t WHERE t.id = $1 AND (t.subtype = 'moderator_warning' \
                 OR EXISTS (SELECT 1 FROM reviewables r WHERE r.topic_id = t.id \
                   AND r.type = 'ReviewableFlaggedPost' AND r.status = 0 AND r.score >= $2)))",
        )
        .bind(self.target.topic_id)
        .bind(min_score)
        .fetch_all(&mut *conn)
        .await?)
    }

    /// `can_assignee_see_target?`
    async fn can_assignee_see_target(
        &self,
        conn: &mut PgConnection,
        user_id: i32,
        pm_users: &Option<Vec<i32>>,
    ) -> Result<bool, AppError> {
        if let Some(pm_users) = pm_users
            && !pm_users.contains(&user_id)
        {
            return Ok(false);
        }
        let Some(guardian) = guardian_for(&mut *conn, user_id).await? else {
            return Ok(false);
        };
        self.target
            .visible_to(&mut *conn, self.settings(), &guardian)
            .await
    }

    /// `can_be_assigned?`
    async fn can_be_assigned(
        &self,
        conn: &mut PgConnection,
        assign_to: &Assignee,
    ) -> Result<bool, AppError> {
        let sql = match assign_to {
            Assignee::User(u) => format!(
                "SELECT EXISTS (SELECT 1 FROM ({}) AS users WHERE users.id = {})",
                self.permissions
                    .assignable_user_ids_sql(self.target.scope(), None),
                u.id
            ),
            Assignee::Group { id, .. } => format!(
                "SELECT EXISTS (SELECT 1 FROM groups WHERE ({}) AND groups.id = {id})",
                self.permissions
                    .assignable_groups_where(self.target.scope(), self.guardian)
            ),
        };
        Ok(sqlx::query_scalar(&sql).fetch_one(&mut *conn).await?)
    }

    /// `can_assign_to?`: a group always; a user under max_assigned_topics
    /// (active assignments on live topics), unless assigning themselves.
    async fn can_assign_to(
        &self,
        conn: &mut PgConnection,
        assign_to: &Assignee,
    ) -> Result<bool, AppError> {
        let Assignee::User(u) = assign_to else {
            return Ok(true);
        };
        if u.id == self.by.id {
            return Ok(true);
        }
        let total: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM assignments \
             JOIN topics ON topics.id = assignments.target_id AND assignments.target_type = 'Topic' \
               AND topics.deleted_at IS NULL \
             WHERE topics.deleted_at IS NULL AND assignments.assigned_to_id = $1 AND assignments.active",
        )
        .bind(u.id)
        .fetch_one(&mut *conn)
        .await?;
        Ok(total < self.settings().get("max_assigned_topics")?.to_i())
    }

    /// `forbidden_reasons`
    async fn forbidden_reason(
        &self,
        conn: &mut PgConnection,
        assign_to: &Assignee,
        current: &Option<Assignment>,
        opts: &AssignOpts,
    ) -> Result<Option<Reason>, AppError> {
        let pm = self.target.private_message();
        let pm_users = if pm {
            Some(self.private_message_allowed_user_ids(conn).await?)
        } else {
            None
        };
        match assign_to {
            Assignee::User(u) => {
                if !self.can_assignee_see_target(conn, u.id, &pm_users).await? {
                    return Ok(Some(if pm {
                        Reason::ForbiddenAssigneeNotPmParticipant
                    } else {
                        Reason::ForbiddenAssigneeCantSeeTopic
                    }));
                }
            }
            Assignee::Group { id, .. } => {
                let members: Vec<i32> = sqlx::query_scalar(
                    "SELECT user_id FROM group_users WHERE group_id = $1 ORDER BY id",
                )
                .bind(id)
                .fetch_all(&mut *conn)
                .await?;
                for member in members {
                    if !self
                        .can_assignee_see_target(conn, member, &pm_users)
                        .await?
                    {
                        return Ok(Some(if pm {
                            Reason::ForbiddenGroupAssigneeNotPmParticipant
                        } else {
                            Reason::ForbiddenGroupAssigneeCantSeeTopic
                        }));
                    }
                }
            }
        }
        if !self.can_be_assigned(conn, assign_to).await? {
            return Ok(Some(match assign_to {
                Assignee::User(_) => Reason::ForbiddenAssignTo,
                Assignee::Group { .. } => Reason::ForbiddenGroupAssignTo,
            }));
        }
        if let Some(note) = opts.note.as_deref().filter(|n| !crate::ruby::is_blank(n))
            && note.chars().count() as i64 > self.settings().get("max_post_length")?.to_i()
        {
            return Ok(Some(Reason::AssignmentNoteTooLong));
        }
        if !opts.allow_self_reassign && self.already_assigned(current, assign_to, opts)? {
            return Ok(Some(match assign_to {
                Assignee::User(_) => Reason::AlreadyAssigned,
                Assignee::Group { .. } => Reason::GroupAlreadyAssigned,
            }));
        }
        let on_topic: i64 =
            sqlx::query_scalar("SELECT count(*) FROM assignments WHERE topic_id = $1 AND active")
                .bind(self.target.topic_id)
                .fetch_one(&mut *conn)
                .await?;
        let reassign = current.as_ref().is_some_and(|a| a.active);
        if on_topic >= ASSIGNMENTS_PER_TOPIC_LIMIT && !reassign {
            return Ok(Some(Reason::TooManyAssignsForTopic));
        }
        if !self.can_assign_to(conn, assign_to).await? {
            return Ok(Some(Reason::TooManyAssigns));
        }
        Ok(None)
    }

    /// `already_assigned?` (`assignment_eq?`)
    fn already_assigned(
        &self,
        current: &Option<Assignment>,
        assign_to: &Assignee,
        opts: &AssignOpts,
    ) -> Result<bool, AppError> {
        let Some(a) = current.as_ref().filter(|a| a.active) else {
            return Ok(false);
        };
        if a.assigned_to_id != assign_to.id() || a.assigned_to_type != assign_to.type_name() {
            return Ok(false);
        }
        if a.note != opts.note {
            return Ok(false);
        }
        Ok(a.status == opts.status
            || (opts.status.is_none() && a.status == super::default_status(self.settings())?))
    }

    /// `assign(assign_to, note:, skip_small_action_post:, status:,
    /// allow_self_reassign:, should_notify:)`
    pub async fn assign(
        &self,
        conn: &mut PgConnection,
        assign_to: &Assignee,
        opts: AssignOpts,
    ) -> Result<Result<(), Reason>, AppError> {
        if !self
            .permissions
            .can_assign_in(self.guardian, self.target.category_id)
        {
            return Ok(Err(Reason::ForbiddenAssignerNotAllowed));
        }
        if self.target.private_message() && self.settings().get("invite_on_assign")?.truthy() {
            self.invite(conn, assign_to).await?;
        }
        let current =
            Assignment::for_target(&mut *conn, self.target.type_name(), self.target.id).await?;
        if let Some(reason) = self
            .forbidden_reason(conn, assign_to, &current, &opts)
            .await?
        {
            return Ok(Err(reason));
        }
        let no_assignee_change = current.as_ref().is_some_and(|a| {
            a.assigned_to_id == assign_to.id() && a.assigned_to_type == assign_to.type_name()
        });
        // details_change?
        let details_change = opts
            .note
            .as_deref()
            .is_some_and(|n| !crate::ruby::is_blank(n))
            || current.as_ref().map(|a| &a.status) != Some(&opts.status);
        if let Some(current) = current
            .as_ref()
            .filter(|_| no_assignee_change && details_change)
        {
            return self.update_details(conn, assign_to, current, &opts).await;
        }

        // before_validation :default_status, validate :validate_status
        let mut status = opts.status.clone();
        if super::status_enabled(self.settings())? {
            if status.is_none() {
                status = super::default_status(self.settings())?;
            }
            let valid = status
                .as_ref()
                .is_some_and(|s| super::statuses(self.settings()).is_ok_and(|all| all.contains(s)));
            if !valid {
                return Ok(Err(Reason::InvalidStatus));
            }
        }

        let topic_assigned = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM assignments WHERE target_type = 'Topic' AND target_id = $1)",
        )
        .bind(self.target.topic_id)
        .fetch_one(&mut *conn)
        .await?;
        let skip_small_action_post =
            opts.skip_small_action_post || (!opts.allow_self_reassign && no_assignee_change);

        if let Some(current) = &current {
            super::jobs::enqueue_unassign_notification(conn, current).await?;
            destroy(conn, current.id).await?;
        }
        let assignment: Assignment = sqlx::query_as(&format!(
            "INSERT INTO assignments AS a (topic_id, assigned_to_id, assigned_by_user_id, created_at, \
                 updated_at, assigned_to_type, target_id, target_type, active, note, status) \
             VALUES ($1, $2, $3, clock_timestamp(), clock_timestamp(), $4, $5, $6, TRUE, $7, $8) \
             RETURNING {}",
            super::COLUMNS
        ))
        .bind(self.target.topic_id)
        .bind(assign_to.id())
        .bind(self.by.id)
        .bind(assign_to.type_name())
        .bind(self.target.id)
        .bind(self.target.type_name())
        .bind(&opts.note)
        .bind(&status)
        .fetch_one(&mut *conn)
        .await?;

        self.publish_first_post_revised(conn).await?;
        if opts.should_notify {
            super::jobs::enqueue_assign_notification(conn, assignment.id).await?;
        } else {
            // This assignment should never be notified.
            sqlx::query(
                "INSERT INTO silenced_assignments (assignment_id, created_at, updated_at) \
                 VALUES ($1, clock_timestamp(), clock_timestamp())",
            )
            .bind(assignment.id)
            .execute(&mut *conn)
            .await?;
        }
        publish_topic_assignment(conn, self.ctx, self.permissions, &assignment).await?;

        if let Assignee::User(u) = assign_to {
            self.track_topic(conn, u.id).await?;
            self.send_email(conn, u.id).await?;
        }

        if !skip_small_action_post {
            let action_code = match (self.target.is_post, assign_to) {
                (true, Assignee::User(_)) => "assigned_to_post",
                (true, Assignee::Group { .. }) => "assigned_group_to_post",
                (false, Assignee::User(_)) if topic_assigned => "reassigned",
                (false, Assignee::User(_)) => "assigned",
                (false, Assignee::Group { .. }) if topic_assigned => "reassigned_group",
                (false, Assignee::Group { .. }) => "assigned_group",
            };
            self.add_small_action_post(conn, action_code, assign_to, opts.note.as_deref())
                .await?;
        }

        if crate::web_hooks::active(&mut *conn, "assigned").await? {
            let mut payload = Map::new();
            payload.insert("type".into(), json!("assigned"));
            payload.insert("topic_id".into(), json!(self.target.topic_id));
            payload.insert("topic_title".into(), json!(self.target.topic_title));
            payload.insert("assigned_by_id".into(), json!(self.by.id));
            payload.insert("assigned_by_username".into(), json!(self.by.username));
            match assign_to {
                Assignee::User(u) => {
                    payload.insert("assigned_to_id".into(), json!(u.id));
                    payload.insert("assigned_to_username".into(), json!(u.username));
                }
                Assignee::Group { id, name } => {
                    payload.insert("assigned_to_group_id".into(), json!(id));
                    payload.insert("assigned_to_group_name".into(), json!(name));
                }
            }
            enqueue_assign_hooks(conn, "assigned", Value::Object(payload)).await?;
        }
        workflows_unsupported(conn, "trigger:assigned").await?;
        Ok(Ok(()))
    }

    /// `update_details`: the same assignee, a new note or status.
    async fn update_details(
        &self,
        conn: &mut PgConnection,
        assign_to: &Assignee,
        current: &Assignment,
        opts: &AssignOpts,
    ) -> Result<Result<(), Reason>, AppError> {
        let note = opts.note.as_deref().filter(|n| !crate::ruby::is_blank(n));
        let status = opts.status.as_deref();
        let old_status = current.status.as_deref().unwrap_or("");
        let new_status = status.unwrap_or("");
        let (change_type, text) = if let Some(note) = note
            && status.is_some_and(|s| !s.is_empty())
            && current.note.as_deref() != Some(note)
            && current.status.as_deref() != status
        {
            (
                Some("details"),
                Some(format!("Status: {old_status} → {new_status}\n\n{note}\n")),
            )
        } else if let Some(note) = note.filter(|n| current.note.as_deref() != Some(*n)) {
            (Some("note"), Some(note.to_string()))
        } else if current.status.as_deref() != status {
            (Some("status"), Some(format!("{old_status} → {new_status}")))
        } else {
            (None, None)
        };
        // update!(note:, status:): the validation reads the status given,
        // defaulted when nil.
        let mut status = opts.status.clone();
        if super::status_enabled(self.settings())? {
            if status.is_none() {
                status = super::default_status(self.settings())?;
            }
            if !status
                .as_ref()
                .is_some_and(|s| super::statuses(self.settings()).is_ok_and(|all| all.contains(s)))
            {
                return Ok(Err(Reason::InvalidStatus));
            }
        }
        let assignment: Assignment = sqlx::query_as(&format!(
            "UPDATE assignments AS a SET note = $2, status = $3, updated_at = clock_timestamp() \
             WHERE a.id = $1 RETURNING {}",
            super::COLUMNS
        ))
        .bind(current.id)
        .bind(&opts.note)
        .bind(&status)
        .fetch_one(&mut *conn)
        .await?;
        if opts.should_notify {
            super::jobs::enqueue_assign_notification(conn, assignment.id).await?;
        }
        publish_topic_assignment(conn, self.ctx, self.permissions, &assignment).await?;
        if !opts.skip_small_action_post {
            // "#{change_type}_change": "_change" when nothing changed.
            let action_code = format!("{}_change", change_type.unwrap_or(""));
            self.add_small_action_post(conn, &action_code, assign_to, text.as_deref())
                .await?;
        }
        Ok(Ok(()))
    }

    /// `unassign(silent:, deactivate:)`
    pub async fn unassign(
        &self,
        conn: &mut PgConnection,
        silent: bool,
        deactivate: bool,
    ) -> Result<(), AppError> {
        let Some(assignment) =
            Assignment::for_target(&mut *conn, self.target.type_name(), self.target.id).await?
        else {
            return Ok(());
        };
        if deactivate {
            sqlx::query(
                "UPDATE assignments SET active = FALSE, updated_at = clock_timestamp() WHERE id = $1",
            )
            .bind(assignment.id)
            .execute(&mut *conn)
            .await?;
        } else {
            destroy(conn, assignment.id).await?;
        }
        // return if first_post.blank?
        let first_post: Option<i32> = sqlx::query_scalar(
            "SELECT id FROM posts WHERE topic_id = $1 AND post_number = 1 AND deleted_at IS NULL",
        )
        .bind(self.target.topic_id)
        .fetch_optional(&mut *conn)
        .await?;
        if first_post.is_none() {
            return Ok(());
        }
        self.publish_first_post_revised(conn).await?;
        super::jobs::enqueue_unassign_notification(conn, &assignment).await?;
        let assigned_to = Assignee::of(
            &mut *conn,
            &assignment.assigned_to_type,
            assignment.assigned_to_id,
        )
        .await?;

        if self
            .settings()
            .get("unassign_creates_tracking_post")?
            .truthy()
            && !silent
            && let Some(assigned_to) = &assigned_to
        {
            let suffix = if self.target.is_post {
                "_from_post"
            } else {
                ""
            };
            let action_code = if assignment.assigned_to_user() {
                format!("unassigned{suffix}")
            } else {
                format!("unassigned_group{suffix}")
            };
            self.add_small_action_post(conn, &action_code, assigned_to, None)
                .await?;
        }

        if let Some(assigned_to) = &assigned_to
            && crate::web_hooks::active(&mut *conn, "unassigned").await?
        {
            let mut payload = Map::new();
            payload.insert("type".into(), json!("unassigned"));
            payload.insert("topic_id".into(), json!(self.target.topic_id));
            payload.insert("topic_title".into(), json!(self.target.topic_title));
            payload.insert("unassigned_by_id".into(), json!(self.by.id));
            payload.insert("unassigned_by_username".into(), json!(self.by.username));
            match assigned_to {
                Assignee::User(u) => {
                    payload.insert("unassigned_to_id".into(), json!(u.id));
                    payload.insert("unassigned_to_username".into(), json!(u.username));
                }
                Assignee::Group { id, name } => {
                    payload.insert("unassigned_to_group_id".into(), json!(id));
                    payload.insert("unassigned_to_group_name".into(), json!(name));
                }
            }
            enqueue_assign_hooks(conn, "unassigned", Value::Object(payload)).await?;
        }
        workflows_unsupported(conn, "trigger:unassigned").await?;

        let allowed =
            super::assignable_user_ids(&mut *conn, self.permissions, self.target.scope()).await?;
        if !allowed.is_empty() {
            let message = json!({
                "type": "unassigned",
                "topic_id": self.target.topic_id,
                "post_id": if self.target.is_post { json!(self.target.id) } else { json!(false) },
                "post_number": if self.target.is_post { json!(self.target.post_number) } else { json!(false) },
                "assigned_type": if assignment.assigned_to_group() { "Group" } else { "User" },
                "assignment_note": null,
                "assignment_status": null,
            });
            publish_to_users(
                self.ctx.bus,
                conn,
                "/staff/topic-assignment",
                &message,
                &allowed,
            )
            .await?;
        }
        Ok(())
    }

    /// invite_on_assign: assignees outside the message would be invited
    /// (`invite_user`, `invite_group`), which isn't ported.
    async fn invite(&self, conn: &mut PgConnection, assign_to: &Assignee) -> Result<(), AppError> {
        let needed: bool = match assign_to {
            Assignee::User(u) => !self
                .private_message_allowed_user_ids(conn)
                .await?
                .contains(&u.id),
            Assignee::Group { id, .. } => {
                sqlx::query_scalar(
                    "SELECT NOT EXISTS (SELECT 1 FROM topic_allowed_groups WHERE topic_id = $1 AND group_id = $2) \
                     AND EXISTS (SELECT 1 FROM group_users gu WHERE gu.group_id = $2 AND gu.user_id NOT IN (\
                       SELECT user_id FROM topic_allowed_users WHERE topic_id = $1 \
                       UNION SELECT g2.user_id FROM topic_allowed_groups tg JOIN group_users g2 \
                         ON g2.group_id = tg.group_id WHERE tg.topic_id = $1))",
                )
                .bind(self.target.topic_id)
                .bind(id)
                .fetch_one(&mut *conn)
                .await?
            }
        };
        if needed {
            return Err(
                Unsupported("inviting an assignee to the message (invite_on_assign)").into(),
            );
        }
        Ok(())
    }

    /// `first_post&.publish_change_to_clients!(:revised, reload_topic: true)`
    async fn publish_first_post_revised(&self, conn: &mut PgConnection) -> Result<(), AppError> {
        let first_post: Option<i32> = sqlx::query_scalar(
            "SELECT id FROM posts WHERE topic_id = $1 AND post_number = 1 AND deleted_at IS NULL",
        )
        .bind(self.target.topic_id)
        .fetch_optional(&mut *conn)
        .await?;
        if let Some(post_id) = first_post {
            let mut opts = Map::new();
            opts.insert("reload_topic".into(), json!(true));
            crate::bus::publish_post_change(self.ctx, conn, post_id, "revised", opts, false)
                .await?;
        }
        Ok(())
    }

    /// The assignee watches or tracks the topic, by their
    /// notification_level_when_assigned (do nothing 1, track 2, watch 3),
    /// unless they already follow it more closely.
    async fn track_topic(&self, conn: &mut PgConnection, user_id: i32) -> Result<(), AppError> {
        use crate::posting::notification_levels::{TRACKING, WATCHING};
        let when_assigned: Option<i32> = sqlx::query_scalar(
            "SELECT notification_level_when_assigned FROM user_options WHERE user_id = $1",
        )
        .bind(user_id)
        .fetch_optional(&mut *conn)
        .await?;
        let when_assigned = when_assigned.unwrap_or(3);
        if when_assigned == 1 {
            return Ok(());
        }
        let level = if when_assigned == 2 {
            TRACKING
        } else {
            WATCHING
        };
        let current: Option<i32> = sqlx::query_scalar(
            "SELECT notification_level FROM topic_users WHERE user_id = $1 AND topic_id = $2",
        )
        .bind(user_id)
        .bind(self.target.topic_id)
        .fetch_optional(&mut *conn)
        .await?;
        if current.is_none_or(|c| c < level) {
            // TopicUser.notification_reasons[:plugin_changed]
            const PLUGIN_CHANGED: i32 = 9;
            crate::posting::change_topic_user(
                self.ctx.bus,
                conn,
                user_id,
                self.target.topic_id,
                &[crate::posting::TopicUserAttr::NotificationLevel(
                    level,
                    PLUGIN_CHANGED,
                )],
            )
            .await?;
        }
        Ok(())
    }

    /// AssignMailer, by assign_mailer: never, different_users or always,
    /// unless the assignee muted the topic. Not ported: refused when it
    /// would send.
    async fn send_email(&self, conn: &mut PgConnection, user_id: i32) -> Result<(), AppError> {
        let level = self.settings().get("assign_mailer")?.to_s();
        let send = level == "always" || (level == "different_users" && self.by.id != user_id);
        if !send {
            return Ok(());
        }
        let muted: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM topic_users WHERE user_id = $1 AND topic_id = $2 \
             AND notification_level = 0)",
        )
        .bind(user_id)
        .bind(self.target.topic_id)
        .fetch_one(&mut *conn)
        .await?;
        if !muted {
            return Err(Unsupported("the assignment email (assign_mailer)").into());
        }
        Ok(())
    }

    /// `small_action_username_or_name`
    async fn who(&self, assign_to: &Assignee) -> Result<String, AppError> {
        Ok(match assign_to {
            Assignee::User(u) if self.settings().get("prioritize_full_name_in_ux")?.truthy() => u
                .name
                .clone()
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| u.username.clone()),
            Assignee::User(u) => u.username.clone(),
            Assignee::Group { name, .. } => name.clone(),
        })
    }

    /// `add_small_action_post`: a small action while assigns are public,
    /// else a whisper, by the assigner, not bumping.
    async fn add_small_action_post(
        &self,
        conn: &mut PgConnection,
        action_code: &str,
        assign_to: &Assignee,
        text: Option<&str>,
    ) -> Result<i32, AppError> {
        let who = self.who(assign_to).await?;
        let action = SmallAction {
            user_id: self.by.id,
            topic_id: self.target.topic_id,
            action_code,
            bump: false,
            who: Some(&who),
        };
        let post_id = if super::public(self.settings())? {
            small_action::add_with_message(conn, self.ctx, &action, text, false).await?
        } else {
            small_action::add_whisper(conn, self.ctx, &action, text.unwrap_or("")).await?
        };
        if self.target.is_post {
            sqlx::query(
                "INSERT INTO post_custom_fields (post_id, name, value, created_at, updated_at) \
                 VALUES ($1, 'action_code_path', $2, clock_timestamp(), clock_timestamp()), \
                        ($1, 'action_code_post_id', $3, clock_timestamp(), clock_timestamp())",
            )
            .bind(post_id)
            .bind(format!("/p/{}", self.target.id))
            .bind(self.target.id.to_string())
            .execute(&mut *conn)
            .await?;
        }
        Ok(post_id)
    }
}

/// `assignment.destroy!`: the row and its silenced row.
pub async fn destroy(conn: &mut PgConnection, assignment_id: i64) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM assignments WHERE id = $1")
        .bind(assignment_id)
        .execute(conn)
        .await?;
    Ok(())
}

/// `WebHook.enqueue_assign_hooks(event, payload.to_json)`
async fn enqueue_assign_hooks(
    conn: &mut PgConnection,
    event: &str,
    payload: Value,
) -> Result<(), AppError> {
    let mut opts = Map::new();
    opts.insert("payload".into(), json!(crate::web_hooks::to_json(&payload)));
    crate::web_hooks::enqueue_hooks(conn, "assign", event, opts).await
}

/// `MessageBus.publish(channel, message, user_ids:)`
pub async fn publish_to_users(
    bus: &pg_bus::Bus,
    conn: &mut PgConnection,
    channel: &str,
    message: &Value,
    user_ids: &[i32],
) -> Result<(), AppError> {
    if user_ids.is_empty() {
        return Ok(());
    }
    let tags: Vec<String> = user_ids.iter().copied().map(crate::bus::user_tag).collect();
    bus.publish(conn, channel, message, Some(&tags)).await?;
    Ok(())
}

/// `Assignment#publish_topic_assignment`: the assignment to the users who
/// may be assigned its target, the assignee as BasicUserSerializer or
/// BasicGroupSerializer for an anonymous guardian.
pub async fn publish_topic_assignment(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    permissions: &Permissions,
    assignment: &Assignment,
) -> Result<(), AppError> {
    let assigned_to = if assignment.assigned_to_group() {
        let Some(group) =
            crate::groups::find_basic(&mut *conn, None, Some(assignment.assigned_to_id)).await?
        else {
            return Ok(());
        };
        group.json(ctx.i18n, &Guardian::anonymous(), ctx.settings, None)?
    } else {
        let assignees = Assignees::load(&mut *conn, &[assignment]).await?;
        let Some(user) = assignees.users.get(&assignment.assigned_to_id) else {
            return Ok(());
        };
        let urls = crate::url::Urls {
            config: ctx.config,
            settings: ctx.settings,
        };
        let logo = crate::admin_users::logo_small_url(&mut *conn, ctx.settings).await?;
        Cx {
            settings: ctx.settings,
            urls: &urls,
            logo: logo.as_deref(),
        }
        .basic_user(user)?
    };
    // assignable_user_ids(target || topic)
    let scope = super::topic_scope(&mut *conn, assignment.topic_id).await?;
    let allowed = super::assignable_user_ids(&mut *conn, permissions, scope).await?;
    if allowed.is_empty() {
        return Ok(());
    }
    let post_number: Option<i32> = if assignment.on_post() {
        sqlx::query_scalar("SELECT post_number FROM posts WHERE id = $1")
            .bind(assignment.target_id)
            .fetch_optional(&mut *conn)
            .await?
    } else {
        None
    };
    let message = json!({
        "type": "assigned",
        "topic_id": assignment.topic_id,
        "post_id": if assignment.on_post() { json!(assignment.target_id) } else { json!(false) },
        "post_number": if assignment.on_post() { json!(post_number) } else { json!(false) },
        "assigned_type": assignment.assigned_to_type,
        "assigned_to": assigned_to,
        "assignment_note": assignment.note,
        "assignment_status": assignment.status,
    });
    publish_to_users(ctx.bus, conn, "/staff/topic-assignment", &message, &allowed).await
}

/// `Assigner.publish_topic_tracking_state(topic, user_id)`: a message's
/// assignee hears of it on /private-messages/assigned.
pub async fn publish_topic_tracking_state(
    bus: &pg_bus::Bus,
    conn: &mut PgConnection,
    topic_id: i32,
    user_id: i32,
) -> Result<(), AppError> {
    let pm: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM topics WHERE id = $1 AND archetype = 'private_message')",
    )
    .bind(topic_id)
    .fetch_one(&mut *conn)
    .await?;
    if pm {
        publish_to_users(
            bus,
            conn,
            "/private-messages/assigned",
            &json!({ "topic_id": topic_id }),
            &[user_id],
        )
        .await?;
    }
    Ok(())
}

/// `TopicTrackingState.publish_assigned_private_message(topic, assignee)`:
/// to the assigned user, or the group's members.
pub async fn publish_assigned_private_message(
    bus: &pg_bus::Bus,
    conn: &mut PgConnection,
    topic_id: i32,
    assigned_to_type: &str,
    assigned_to_id: i32,
) -> Result<(), AppError> {
    let pm: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM topics WHERE id = $1 AND archetype = 'private_message')",
    )
    .bind(topic_id)
    .fetch_one(&mut *conn)
    .await?;
    if !pm {
        return Ok(());
    }
    let tags = if assigned_to_type == "User" {
        vec![crate::bus::user_tag(assigned_to_id)]
    } else {
        vec![crate::bus::group_tag(assigned_to_id)]
    };
    bus.publish(
        conn,
        "/private-messages/assigned",
        &json!({ "topic_id": topic_id }),
        Some(&tags),
    )
    .await?;
    Ok(())
}

/// `Assigner.assigned_self?` / `assigned_other?`: the regex setting
/// matches the text; a blank or invalid regex never does.
fn regex_matches(settings: &SiteSettings, setting: &str, text: &str) -> Result<bool, AppError> {
    let pattern = settings.get(setting)?.to_s();
    if text.trim().is_empty() || pattern.trim().is_empty() {
        return Ok(false);
    }
    // Regexp.new(setting) rescue nil: an invalid one never matches.
    Ok(crate::word_watcher::ruby_regex(&pattern, true, true)
        .is_ok_and(|re| re.is_match(text).unwrap_or(false)))
}

/// `Assigner.auto_assign(post, force:)`: with assigns_by_staff_mention, a
/// post by someone who may assign its topic, matching assign_other_regex
/// and mentioning an assignable user, or assign_self_regex, assigns the
/// topic when it is their last post among the assignable users'.
pub async fn auto_assign(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    post_id: i32,
    force: bool,
) -> Result<bool, AppError> {
    let settings = ctx.settings;
    if !settings.get("assigns_by_staff_mention")?.truthy() {
        return Ok(false);
    }
    let post: Option<(Option<i32>, i32, i32, String)> = sqlx::query_as(
        "SELECT p.user_id, p.topic_id, p.post_number, p.cooked FROM posts p \
         JOIN topics t ON t.id = p.topic_id AND t.deleted_at IS NULL WHERE p.id = $1",
    )
    .bind(post_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((Some(user_id), topic_id, post_number, cooked)) = post else {
        return Ok(false);
    };
    let Some(target) = Target::topic(&mut *conn, topic_id).await? else {
        return Ok(false);
    };
    let Some(user) = SessionUser::load(&mut *conn, user_id).await? else {
        return Ok(false);
    };
    let guardian = Guardian::for_user(&mut *conn, &user).await?;
    let permissions = Permissions::load(&mut *conn, settings).await?;
    if !permissions.can_assign_in(&guardian, target.category_id) {
        return Ok(false);
    }
    if !force
        && Assignment::for_target(&mut *conn, "Topic", topic_id)
            .await?
            .is_some()
    {
        return Ok(false);
    }
    let text = text_without_quotes(&cooked);
    let assignable = permissions.assignable_user_ids_sql(target.scope(), None);
    let assign_other = if regex_matches(settings, "assign_other_regex", &text)? {
        // mentioned_staff: the first assignable human user mentioned.
        let mentions = crate::posting::validate::analyze(&cooked, "")?.mention_names;
        if mentions.is_empty() {
            None
        } else {
            let id: Option<i32> = sqlx::query_scalar(&format!(
                "SELECT id FROM users WHERE id > 0 AND id IN ({assignable}) \
                 AND username_lower = ANY($1) LIMIT 1"
            ))
            .bind(&mentions)
            .fetch_optional(&mut *conn)
            .await?;
            match id {
                Some(id) => Assignee::user(&mut *conn, id).await?,
                None => None,
            }
        }
    } else {
        None
    };
    let assign_self = if regex_matches(settings, "assign_self_regex", &text)? {
        Assignee::user(&mut *conn, user_id).await?
    } else {
        None
    };
    let Some(assign_to) = assign_other.or(assign_self) else {
        return Ok(false);
    };
    // is_last_staff_post?
    let last: Option<i32> = sqlx::query_scalar(&format!(
        "SELECT MAX(post_number) FROM posts WHERE topic_id = $1 AND deleted_at IS NULL \
         AND user_id IN ({assignable})"
    ))
    .bind(topic_id)
    .fetch_one(&mut *conn)
    .await?;
    if last != Some(post_number) {
        return Ok(false);
    }
    let assigner = Assigner {
        ctx,
        target: &target,
        by: &user,
        guardian: &guardian,
        permissions: &permissions,
    };
    let result = assigner
        .assign(
            conn,
            &assign_to,
            AssignOpts {
                skip_small_action_post: true,
                ..Default::default()
            },
        )
        .await?;
    Ok(result.is_ok())
}

/// The cooked text without quotes, oneboxes and code
/// (`doc.css(".quote, .onebox, pre, code").remove`), stripped.
fn text_without_quotes(cooked: &str) -> String {
    use crate::pretty_text::cleanup;
    use markup5ever_rcdom::{Handle, NodeData};
    fn walk(node: &Handle, out: &mut String) {
        if let NodeData::Text { contents } = &node.data {
            out.push_str(&contents.borrow());
        }
        for child in node.children.borrow().iter() {
            let removed = matches!(cleanup::element_name(child), Some("pre" | "code"))
                || cleanup::has_class(child, "quote")
                || cleanup::has_class(child, "onebox");
            if !removed {
                walk(child, out);
            }
        }
    }
    let dom = cleanup::parse(cooked);
    let mut out = String::new();
    walk(&cleanup::fragment_root(&dom), &mut out);
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_text_leaves_out_quotes_oneboxes_and_code() {
        let cooked = "<aside class=\"quote\"><blockquote>my list</blockquote></aside>\
                      <p>hi <code>my list</code> there</p><pre>x</pre>";
        assert_eq!(text_without_quotes(cooked), "hi  there");
    }
}
