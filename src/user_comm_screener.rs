//! `UserCommScreener` (lib/user_comm_screener.rb): whether target users
//! take communication (messages, alerts, chat DMs) from an acting user,
//! and whether the acting user takes it from them. Both SQL queries are
//! Rails' own, quirks included (the actor's 'disallowed_pm' join).

use std::collections::HashMap;

use sqlx::PgConnection;

/// `UserCommPref`: one target's preferences towards the actor.
#[derive(Debug, Clone, Default)]
struct Pref {
    user_id: i32,
    is_muting: bool,
    is_ignoring: bool,
    is_disallowing_all_pms: bool,
    is_disallowing_pms_from_acting_user: bool,
}

impl Pref {
    fn communication_prevented(&self) -> bool {
        self.ignoring_or_muting() || self.disallowing_pms()
    }

    fn ignoring_or_muting(&self) -> bool {
        self.is_muting || self.is_ignoring
    }

    fn disallowing_pms(&self) -> bool {
        self.is_disallowing_all_pms || self.is_disallowing_pms_from_acting_user
    }
}

/// A user id outside the screener's targets (`Discourse::NotFound`).
#[derive(Debug)]
pub struct NotATarget(pub i32);

impl std::fmt::Display for NotATarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "user {} is not one of the screener's targets", self.0)
    }
}

impl std::error::Error for NotATarget {}

/// The actor's own preferences towards the targets (`actor_preferences`).
#[derive(Debug, Default)]
struct ActorPrefs {
    muting: Vec<i32>,
    ignoring: Vec<i32>,
    disallowed_pms_from: Vec<i32>,
}

/// The acting user as the screener reads it.
#[derive(Debug, Clone, Copy)]
pub struct Actor {
    pub id: i32,
    pub staff: bool,
}

pub struct UserCommScreener {
    actor: Actor,
    allow_private_messages: bool,
    enable_allowed_pm_users: bool,
    /// The targets, in `pluck` order, the actor left out.
    target_ids: Vec<i32>,
    /// The preference map in insertion order.
    prefs: Vec<Pref>,
    actor_prefs: ActorPrefs,
}

impl UserCommScreener {
    /// `UserCommScreener.new(acting_user:, target_user_ids:)`, with both
    /// queries run up front.
    pub async fn new(
        conn: &mut PgConnection,
        actor: Actor,
        target_user_ids: &[i32],
    ) -> Result<UserCommScreener, sqlx::Error> {
        let wanted: Vec<i32> = target_user_ids
            .iter()
            .copied()
            .filter(|id| *id != actor.id)
            .collect();
        let target_ids: Vec<i32> = sqlx::query_scalar("SELECT id FROM users WHERE id = ANY($1)")
            .bind(&wanted)
            .fetch_all(&mut *conn)
            .await?;
        let (allow_private_messages, enable_allowed_pm_users): (bool, bool) = sqlx::query_as(
            "SELECT COALESCE(allow_private_messages, TRUE), COALESCE(enable_allowed_pm_users, FALSE) \
             FROM user_options WHERE user_id = $1",
        )
        .bind(actor.id)
        .fetch_optional(&mut *conn)
        .await?
        .unwrap_or((true, false));

        // load_preference_map: none for a staff actor.
        let mut prefs: Vec<Pref> = Vec::new();
        if !actor.staff {
            let rows: Vec<(i32, bool, bool, bool, Option<bool>)> = sqlx::query_as(
                "SELECT users.id, \
                 CASE WHEN muted_users.muted_user_id IS NOT NULL THEN true ELSE false END AS is_muting, \
                 CASE WHEN ignored_users.ignored_user_id IS NOT NULL THEN true ELSE false END AS is_ignoring, \
                 CASE WHEN user_options.allow_private_messages THEN false ELSE true END AS is_disallowing_all_pms, \
                 user_options.enable_allowed_pm_users \
                 FROM users \
                 LEFT JOIN user_options ON user_options.user_id = users.id \
                 LEFT JOIN muted_users ON muted_users.user_id = users.id AND muted_users.muted_user_id = $1 \
                 LEFT JOIN ignored_users ON ignored_users.user_id = users.id AND ignored_users.ignored_user_id = $1 \
                 WHERE users.id = ANY($2) AND \
                 ( \
                   NOT user_options.allow_private_messages OR \
                   user_options.enable_allowed_pm_users OR \
                   muted_users.user_id IS NOT NULL OR \
                   ignored_users.user_id IS NOT NULL \
                 )",
            )
            .bind(actor.id)
            .bind(&target_ids)
            .fetch_all(&mut *conn)
            .await?;
            for (user_id, is_muting, is_ignoring, is_disallowing_all_pms, _) in &rows {
                prefs.push(Pref {
                    user_id: *user_id,
                    is_muting: *is_muting,
                    is_ignoring: *is_ignoring,
                    is_disallowing_all_pms: *is_disallowing_all_pms,
                    is_disallowing_pms_from_acting_user: false,
                });
            }
            let with_allowed: Vec<i32> = rows
                .iter()
                .filter(|r| r.4 == Some(true))
                .map(|r| r.0)
                .collect();
            if !with_allowed.is_empty() {
                let can_pm: Vec<i32> = sqlx::query_scalar(
                    "SELECT DISTINCT user_id FROM allowed_pm_users \
                     WHERE allowed_pm_user_id = $1 AND user_id = ANY($2)",
                )
                .bind(actor.id)
                .bind(&with_allowed)
                .fetch_all(&mut *conn)
                .await?;
                for user_id in with_allowed.into_iter().filter(|id| !can_pm.contains(id)) {
                    match prefs.iter_mut().find(|p| p.user_id == user_id) {
                        Some(p) => p.is_disallowing_pms_from_acting_user = true,
                        None => prefs.push(Pref {
                            user_id,
                            is_disallowing_pms_from_acting_user: true,
                            ..Pref::default()
                        }),
                    }
                }
            }
        }

        // actor_communication_preferences
        let rows: Vec<(i32, String)> = sqlx::query_as(
            "SELECT users.id AS target_user_id, 'disallowed_pm' AS preference_type FROM users \
             LEFT JOIN allowed_pm_users ON allowed_pm_users.allowed_pm_user_id = users.id \
             WHERE users.id = ANY($2) \
             AND (allowed_pm_users.user_id = $1 OR allowed_pm_users.user_id IS NULL) \
             AND allowed_pm_users.allowed_pm_user_id IS NULL \
             UNION \
             SELECT ignored_user_id AS target_user_id, 'ignored' AS preference_type \
             FROM ignored_users \
             WHERE user_id = $1 AND ignored_user_id = ANY($2) \
             UNION \
             SELECT muted_user_id AS target_user_id, 'muted' AS preference_type \
             FROM muted_users \
             WHERE user_id = $1 AND muted_user_id = ANY($2)",
        )
        .bind(actor.id)
        .bind(&target_ids)
        .fetch_all(&mut *conn)
        .await?;
        let mut by_type: HashMap<String, Vec<i32>> = HashMap::new();
        for (id, kind) in rows {
            by_type.entry(kind).or_default().push(id);
        }
        let actor_prefs = ActorPrefs {
            muting: by_type.remove("muted").unwrap_or_default(),
            ignoring: by_type.remove("ignored").unwrap_or_default(),
            disallowed_pms_from: if enable_allowed_pm_users {
                by_type.remove("disallowed_pm").unwrap_or_default()
            } else {
                Vec::new()
            },
        };
        Ok(UserCommScreener {
            actor,
            allow_private_messages,
            enable_allowed_pm_users,
            target_ids,
            prefs,
            actor_prefs,
        })
    }

    fn pref(&self, user_id: i32) -> Option<&Pref> {
        self.prefs.iter().find(|p| p.user_id == user_id)
    }

    fn validate_user_id(&self, user_id: i32) -> Result<(), NotATarget> {
        if user_id == self.actor.id || self.target_ids.contains(&user_id) {
            Ok(())
        } else {
            Err(NotATarget(user_id))
        }
    }

    /// `allowing_actor_communication`
    pub fn allowing_actor_communication(&self) -> Vec<i32> {
        let mut out: Vec<i32> = self
            .prefs
            .iter()
            .filter(|p| self.actor.staff || !p.communication_prevented())
            .map(|p| p.user_id)
            .collect();
        for id in &self.target_ids {
            if self.pref(*id).is_none() && !out.contains(id) {
                out.push(*id);
            }
        }
        out
    }

    /// `preventing_actor_communication`
    pub fn preventing_actor_communication(&self) -> Vec<i32> {
        if self.actor.staff {
            return Vec::new();
        }
        self.prefs
            .iter()
            .filter(|p| p.communication_prevented())
            .map(|p| p.user_id)
            .collect()
    }

    /// `ignoring_or_muting_actor?(user_id)`
    pub fn ignoring_or_muting_actor(&self, user_id: i32) -> Result<bool, NotATarget> {
        self.validate_user_id(user_id)?;
        Ok(!self.actor.staff && self.pref(user_id).is_some_and(Pref::ignoring_or_muting))
    }

    /// `disallowing_pms_from_actor?(user_id)`
    pub fn disallowing_pms_from_actor(&self, user_id: i32) -> Result<bool, NotATarget> {
        self.validate_user_id(user_id)?;
        let disallowing =
            !self.actor.staff && self.pref(user_id).is_some_and(Pref::disallowing_pms);
        Ok(disallowing || self.ignoring_or_muting_actor(user_id)?)
    }

    /// `actor_preventing_communication`
    pub fn actor_preventing_communication(&self) -> Vec<i32> {
        let mut out: Vec<i32> = Vec::new();
        for id in self
            .actor_prefs
            .ignoring
            .iter()
            .chain(&self.actor_prefs.muting)
            .chain(&self.actor_prefs.disallowed_pms_from)
        {
            if !out.contains(id) {
                out.push(*id);
            }
        }
        out
    }

    /// `actor_allowing_communication`
    pub fn actor_allowing_communication(&self) -> Vec<i32> {
        let preventing = self.actor_preventing_communication();
        self.target_ids
            .iter()
            .copied()
            .filter(|id| !preventing.contains(id))
            .collect()
    }

    /// `actor_ignoring?(user_id)`
    pub fn actor_ignoring(&self, user_id: i32) -> Result<bool, NotATarget> {
        self.validate_user_id(user_id)?;
        Ok(self.actor_prefs.ignoring.contains(&user_id))
    }

    /// `actor_muting?(user_id)`
    pub fn actor_muting(&self, user_id: i32) -> Result<bool, NotATarget> {
        self.validate_user_id(user_id)?;
        Ok(self.actor_prefs.muting.contains(&user_id))
    }

    /// `actor_disallowing_pms?(user_id)`
    pub fn actor_disallowing_pms(&self, user_id: i32) -> Result<bool, NotATarget> {
        self.validate_user_id(user_id)?;
        if self.actor_disallowing_all_pms() {
            return Ok(true);
        }
        if !self.enable_allowed_pm_users {
            return Ok(false);
        }
        Ok(self.actor_prefs.disallowed_pms_from.contains(&user_id))
    }

    /// `actor_disallowing_all_pms?`
    pub fn actor_disallowing_all_pms(&self) -> bool {
        !self.allow_private_messages
    }
}
