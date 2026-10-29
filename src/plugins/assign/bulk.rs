//! discourse-assign's bulk operations (`TopicsBulkAction.register_operation`):
//! `assign` the visible topics to a user or group (`username` or
//! `group_name`, with `status` and `note`), `unassign` the ones the user
//! may assign.

use serde_json::{Map, Value};
use sqlx::PgConnection;

use super::Permissions;
use super::assigner::{self, AssignOpts, Assigner, Reason, Resolve, Target};
use crate::guardian::Guardian;
use crate::posting::Ctx;
use crate::{AppError, Unsupported};

/// How an operation ends.
pub enum Outcome {
    /// The topics changed (`@changed_ids`) and the failures counted by
    /// message (`@errors`).
    Done {
        changed: Vec<i32>,
        errors: Vec<(String, i64)>,
    },
    InvalidParameters(String),
    InvalidAccess,
    NotFound,
}

pub struct Bulk<'a, 'b> {
    pub conn: &'a mut PgConnection,
    pub ctx: &'a Ctx<'b>,
    pub guardian: &'a Guardian,
    pub operation: &'a Map<String, Value>,
}

impl Bulk<'_, '_> {
    fn param(&self, key: &str) -> Option<String> {
        self.operation
            .get(key)
            .and_then(crate::params::scalar)
            .filter(|v| !crate::ruby::is_blank(v))
    }

    /// The `assign` operation.
    pub async fn assign(self, topic_ids: &[i32]) -> Result<Outcome, AppError> {
        let settings = self.ctx.settings;
        let permissions = Permissions::load(&mut *self.conn, settings).await?;
        if !permissions.can_assign(self.guardian) {
            return Ok(Outcome::InvalidAccess);
        }
        let (username, group_name) = (self.param("username"), self.param("group_name"));
        let (status, note) = (self.param("status"), self.param("note"));
        let assign_to =
            match assigner::resolve(&mut *self.conn, self.guardian, username, group_name).await? {
                Ok(assignee) => assignee,
                Err(Resolve::InvalidParameters) => {
                    return Ok(Outcome::InvalidParameters("assignee".into()));
                }
                Err(Resolve::NotFound) => return Ok(Outcome::NotFound),
                Err(Resolve::InvalidAccess) => return Ok(Outcome::InvalidAccess),
            };
        let by = self
            .guardian
            .user()
            .ok_or(Unsupported("bulk assigning anonymously"))?;
        let mut changed = Vec::new();
        let mut errors: Vec<(String, i64)> = Vec::new();
        for &topic_id in topic_ids {
            let Some(target) = Target::topic(&mut *self.conn, topic_id).await? else {
                continue;
            };
            if !target
                .visible_to(&mut *self.conn, settings, self.guardian)
                .await?
            {
                continue;
            }
            let result = Assigner {
                ctx: self.ctx,
                target: &target,
                by,
                guardian: self.guardian,
                permissions: &permissions,
            }
            .assign(
                &mut *self.conn,
                &assign_to,
                AssignOpts {
                    status: status.clone(),
                    note: note.clone(),
                    ..Default::default()
                },
            )
            .await?;
            match result {
                Ok(()) | Err(Reason::AlreadyAssigned | Reason::GroupAlreadyAssigned) => {
                    changed.push(topic_id);
                }
                Err(Reason::InvalidStatus) => {
                    return Err(Unsupported(
                        "bulk assigning with an invalid status (RecordInvalid)",
                    )
                    .into());
                }
                Err(reason) => {
                    let message = assigner::failure_message(
                        self.ctx.i18n,
                        settings,
                        reason,
                        Some(&assign_to),
                    )?;
                    match errors.iter_mut().find(|(m, _)| *m == message) {
                        Some((_, count)) => *count += 1,
                        None => errors.push((message, 1)),
                    }
                }
            }
        }
        Ok(Outcome::Done { changed, errors })
    }

    /// The `unassign` operation: no topic is reported changed.
    pub async fn unassign(self, topic_ids: &[i32]) -> Result<Outcome, AppError> {
        let settings = self.ctx.settings;
        let permissions = Permissions::load(&mut *self.conn, settings).await?;
        let Some(by) = self.guardian.user() else {
            return Err(Unsupported("bulk unassigning anonymously").into());
        };
        for &topic_id in topic_ids {
            let Some(target) = Target::topic(&mut *self.conn, topic_id).await? else {
                continue;
            };
            if !target
                .visible_to(&mut *self.conn, settings, self.guardian)
                .await?
                || !permissions.can_assign_in(self.guardian, target.category_id)
            {
                continue;
            }
            Assigner {
                ctx: self.ctx,
                target: &target,
                by,
                guardian: self.guardian,
                permissions: &permissions,
            }
            .unassign(&mut *self.conn, false, false)
            .await?;
        }
        Ok(Outcome::Done {
            changed: Vec::new(),
            errors: Vec::new(),
        })
    }
}
