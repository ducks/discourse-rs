//! discourse-policy, on the tables Rails' plugin created (`post_policies`,
//! `post_policy_groups`, `policy_users`): a post's `[policy]` (cooked by
//! the markdown crate) kept in sync with its rows after post processing
//! (`lifecycle`), who may add or change one (`PostValidator`), accepting
//! and revoking (`PolicyUser.add!` / `remove!`), the post serializer's
//! `policy_*` keys, the current user's `can_create_policy`, the reminders
//! and renewals (`check`), the policy email (`email`), the
//! unaccepted-policies report and the UI drawn in the post (`view`).

pub mod check;
pub mod email;
pub mod lifecycle;
pub mod view;

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::guardian::Guardian;
use crate::plugins::PluginError;
use crate::site_settings::{SettingError, SiteSettings};
use crate::url::Urls;

/// `DiscoursePolicy::HAS_POLICY`, the post custom field.
pub const HAS_POLICY: &str = "HasPolicy";
/// `DiscoursePolicy::POLICY_USER_DEFAULT_LIMIT`
pub const POLICY_USER_DEFAULT_LIMIT: i64 = 25;
/// `PostPolicy.renew_intervals`, by stored value.
pub const RENEW_INTERVALS: [&str; 3] = ["monthly", "quarterly", "yearly"];
/// `UserOption.policy_email_frequencies`, by stored value.
pub const EMAIL_FREQUENCIES: [&str; 3] = ["never", "when_away", "always"];

/// `enabled_site_setting :policy_enabled`
pub fn enabled(settings: &SiteSettings) -> Result<bool, SettingError> {
    Ok(settings.get("policy_enabled")?.truthy())
}

/// `User#in_any_groups?(SiteSetting.create_policy_allowed_groups_map)`
/// for a user by id; false without one.
pub async fn user_allowed(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    user_id: Option<i32>,
) -> Result<bool, PluginError> {
    let Some(user_id) = user_id else {
        return Ok(false);
    };
    let groups = settings.group_ids("create_policy_allowed_groups")?;
    let granular = settings
        .get("granular_anonymous_and_logged_in_groups_permissions")?
        .truthy();
    if (groups.contains(&0) && !granular) || groups.contains(&5) {
        return Ok(true);
    }
    // The system user counts as in its automatic groups even without the
    // rows.
    if user_id == -1 && groups.iter().any(|g| (1..=14).contains(g) && *g != 4) {
        return Ok(true);
    }
    let member: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM group_users WHERE user_id = $1 AND group_id = ANY($2))",
    )
    .bind(user_id)
    .bind(groups.iter().map(|g| *g as i32).collect::<Vec<i32>>())
    .fetch_one(conn)
    .await?;
    Ok(member)
}

/// `Guardian.new(user).can_edit_group?(group)` for a user by id (None is
/// an anonymous guardian).
pub async fn can_edit_group(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    user_id: Option<i32>,
    group_id: i32,
) -> Result<bool, PluginError> {
    let Some(group) = crate::groups::find_basic(&mut *conn, None, Some(group_id)).await? else {
        return Ok(false);
    };
    let guardian = guardian_of(conn, user_id).await?;
    let membership = crate::groups::membership(&mut *conn, &guardian, group_id).await?;
    Ok(crate::groups::can_edit_group(
        &guardian, settings, &group, membership,
    )?)
}

/// The guardian of a user by id, anonymous without one.
pub async fn guardian_of(
    conn: &mut PgConnection,
    user_id: Option<i32>,
) -> Result<Guardian, sqlx::Error> {
    let Some(user_id) = user_id else {
        return Ok(Guardian::anonymous());
    };
    match crate::session::current::SessionUser::load(&mut *conn, user_id).await? {
        Some(user) => Guardian::for_user(conn, &user).await,
        None => Ok(Guardian::anonymous()),
    }
}

/// `Group.find_by_name`: the id of the group with exactly that name.
pub async fn group_id_by_name(
    conn: &mut PgConnection,
    name: &str,
) -> Result<Option<i32>, sqlx::Error> {
    sqlx::query_scalar("SELECT id FROM groups WHERE name = $1 LIMIT 1")
        .bind(name)
        .fetch_optional(conn)
        .await
}

/// A `post_policies` row.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PostPolicy {
    pub id: i64,
    pub post_id: i64,
    pub version: Option<String>,
    pub private: bool,
    pub add_users_to_group: Option<i32>,
}

impl PostPolicy {
    /// `post.post_policy`
    pub async fn for_post(
        conn: &mut PgConnection,
        post_id: i32,
    ) -> Result<Option<PostPolicy>, sqlx::Error> {
        sqlx::query_as(
            "SELECT id, post_id, version, private, add_users_to_group FROM post_policies \
             WHERE post_id = $1 ORDER BY id LIMIT 1",
        )
        .bind(i64::from(post_id))
        .fetch_optional(conn)
        .await
    }

    /// `groups.pluck(:id)`: the policy's groups that still exist.
    pub async fn group_ids(&self, conn: &mut PgConnection) -> Result<Vec<i32>, sqlx::Error> {
        sqlx::query_scalar(
            "SELECT g.id FROM post_policy_groups ppg JOIN groups g ON g.id = ppg.group_id \
             WHERE ppg.post_policy_id = $1 ORDER BY ppg.id",
        )
        .bind(self.id)
        .fetch_all(conn)
        .await
    }

    /// `accepted_policy_users` (`policy_users.accepted.with_version(version)`)
    /// as SQL over `pu`, for this policy.
    fn accepted_where(&self) -> &'static str {
        "pu.post_policy_id = $1 AND pu.accepted_at IS NOT NULL AND pu.revoked_at IS NULL \
         AND pu.expired_at IS NULL AND pu.version IS NOT DISTINCT FROM $2"
    }

    /// The users of `kind`, a page of them (`limit`, `offset`) and their
    /// count: `accepted_by`, `revoked_by` and `not_accepted_by`, none while
    /// the policy has no group left (`groups.exists?`).
    pub async fn users(
        &self,
        conn: &mut PgConnection,
        kind: Users,
        limit: Option<i64>,
        offset: i64,
    ) -> Result<Vec<i32>, sqlx::Error> {
        let sql = format!(
            "SELECT u.id {} ORDER BY u.id LIMIT $4 OFFSET $5",
            self.from(kind)
        );
        sqlx::query_scalar(&sql)
            .bind(self.id)
            .bind(&self.version)
            .bind(crate::clock::now_naive())
            .bind(limit)
            .bind(offset)
            .fetch_all(conn)
            .await
    }

    /// The size of [`PostPolicy::users`] without a page.
    pub async fn count(&self, conn: &mut PgConnection, kind: Users) -> Result<i64, sqlx::Error> {
        let sql = format!("SELECT COUNT(*) {}", self.from(kind));
        sqlx::query_scalar(&sql)
            .bind(self.id)
            .bind(&self.version)
            .bind(crate::clock::now_naive())
            .fetch_one(conn)
            .await
    }

    /// Whether the user is among [`PostPolicy::users`] (`exists?(id:)`).
    pub async fn includes(
        &self,
        conn: &mut PgConnection,
        kind: Users,
        user_id: i32,
    ) -> Result<bool, sqlx::Error> {
        let sql = format!("SELECT EXISTS (SELECT 1 {} AND u.id = $4)", self.from(kind));
        sqlx::query_scalar(&sql)
            .bind(self.id)
            .bind(&self.version)
            .bind(crate::clock::now_naive())
            .bind(user_id)
            .fetch_one(conn)
            .await
    }

    /// The FROM and WHERE of a user set: `User.activated.not_suspended`
    /// ($3 the time), the policy $1 at version $2.
    fn from(&self, kind: Users) -> String {
        let groups = "EXISTS (SELECT 1 FROM post_policy_groups ppg JOIN groups g ON g.id = ppg.group_id \
                      WHERE ppg.post_policy_id = $1)";
        let active = "u.active AND (u.suspended_till IS NULL OR u.suspended_till <= $3)";
        let accepted = format!(
            "SELECT pu.user_id FROM policy_users pu WHERE {}",
            self.accepted_where()
        );
        match kind {
            Users::Accepted => {
                format!("FROM users u WHERE {groups} AND {active} AND u.id IN ({accepted})")
            }
            Users::Revoked => format!(
                "FROM users u WHERE {groups} AND {active} AND u.id IN (\
                   SELECT pu.user_id FROM policy_users pu WHERE pu.post_policy_id = $1 \
                   AND pu.revoked_at IS NOT NULL AND pu.accepted_at IS NULL \
                   AND pu.expired_at IS NULL AND pu.version IS NOT DISTINCT FROM $2)"
            ),
            // policy_group_users: members of any of the policy's groups.
            Users::NotAccepted => format!(
                "FROM users u WHERE {groups} AND {active} AND u.id IN (\
                   SELECT gu.user_id FROM group_users gu JOIN post_policy_groups ppg \
                   ON ppg.group_id = gu.group_id WHERE ppg.post_policy_id = $1) \
                 AND u.id NOT IN ({accepted})"
            ),
        }
    }
}

/// Which of a policy's user sets.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Users {
    Accepted,
    Revoked,
    NotAccepted,
}

/// `PolicyUser.add!(user, post_policy)`: the user's revoked rows of this
/// version accepted again, then a new accepted row.
pub async fn accept(
    conn: &mut PgConnection,
    policy: &PostPolicy,
    user_id: i32,
) -> Result<(), sqlx::Error> {
    let now = crate::clock::now_naive();
    sqlx::query(
        "UPDATE policy_users SET accepted_at = $4 WHERE post_policy_id = $1 AND user_id = $2 \
         AND revoked_at IS NOT NULL AND accepted_at IS NULL AND expired_at IS NULL \
         AND version IS NOT DISTINCT FROM $3",
    )
    .bind(policy.id)
    .bind(user_id)
    .bind(&policy.version)
    .bind(now)
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "INSERT INTO policy_users (post_policy_id, user_id, accepted_at, version, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $3, $3)",
    )
    .bind(policy.id)
    .bind(user_id)
    .bind(now)
    .bind(&policy.version)
    .execute(conn)
    .await?;
    Ok(())
}

/// `PolicyUser.remove!(user, post_policy)`: the user's accepted rows of
/// this version revoked, then a new revoked row.
pub async fn revoke(
    conn: &mut PgConnection,
    policy: &PostPolicy,
    user_id: i32,
) -> Result<(), sqlx::Error> {
    let now = crate::clock::now_naive();
    sqlx::query(
        "UPDATE policy_users SET revoked_at = $4 WHERE post_policy_id = $1 AND user_id = $2 \
         AND accepted_at IS NOT NULL AND revoked_at IS NULL AND expired_at IS NULL \
         AND version IS NOT DISTINCT FROM $3",
    )
    .bind(policy.id)
    .bind(user_id)
    .bind(&policy.version)
    .bind(now)
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "INSERT INTO policy_users (post_policy_id, user_id, revoked_at, version, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $3, $3)",
    )
    .bind(policy.id)
    .bind(user_id)
    .bind(now)
    .bind(&policy.version)
    .execute(conn)
    .await?;
    Ok(())
}

/// `Guardian.new(user).can_see_groups_members?(groups)`: every group is
/// one whose members they may see (none is never).
pub async fn can_see_groups_members(
    conn: &mut PgConnection,
    guardian: &Guardian,
    group_ids: &[i32],
) -> Result<bool, sqlx::Error> {
    if group_ids.is_empty() {
        return Ok(false);
    }
    let visible: i64 = sqlx::query_scalar(&format!(
        "SELECT COUNT(*) FROM groups g WHERE g.id = ANY($1) AND {}",
        crate::groups::members_visible_groups_where(guardian, "g")
    ))
    .bind(group_ids)
    .fetch_one(conn)
    .await?;
    let mut unique = group_ids.to_vec();
    unique.sort_unstable();
    unique.dedup();
    Ok(visible as usize == unique.len() && unique.len() == group_ids.len())
}

/// `BasicUserSerializer` for each user, in order.
pub async fn basic_users(
    conn: &mut PgConnection,
    urls: &Urls<'_>,
    ids: &[i32],
) -> Result<Vec<Value>, PluginError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        id: i32,
        username: String,
        name: Option<String>,
        uploaded_avatar_id: Option<i32>,
    }
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT id, username, name, uploaded_avatar_id FROM users WHERE id = ANY($1)",
    )
    .bind(ids)
    .fetch_all(&mut *conn)
    .await?;
    let names = urls.settings.get("enable_names")?.truthy();
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        let Some(row) = rows.iter().find(|r| r.id == *id) else {
            continue;
        };
        let mut user = Map::new();
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
                None
            )?),
        );
        out.push(Value::Object(user));
    }
    Ok(out)
}

/// The post serializer's policy keys for a page of posts: each post with
/// the `HasPolicy` custom field and its policy, its keys for the viewer
/// computed up front (`PostSerializerExtension`).
#[derive(Debug, Default)]
pub struct PostsData {
    keys: Vec<(i32, Map<String, Value>)>,
}

impl PostsData {
    pub async fn load(
        conn: &mut PgConnection,
        settings: &SiteSettings,
        urls: &Urls<'_>,
        guardian: &Guardian,
        post_ids: &[i32],
    ) -> Result<PostsData, PluginError> {
        let with_field: Vec<i32> = sqlx::query_scalar(
            "SELECT DISTINCT post_id FROM post_custom_fields WHERE post_id = ANY($1) AND name = $2 \
             ORDER BY post_id",
        )
        .bind(post_ids)
        .bind(HAS_POLICY)
        .fetch_all(&mut *conn)
        .await?;
        let mut data = PostsData::default();
        let easy_revoke = settings.get("policy_easy_revoke")?.truthy();
        for post_id in with_field {
            // `delegate :post_policy`: a field without its policy has
            // nothing to serialize.
            let Some(policy) = PostPolicy::for_post(&mut *conn, post_id).await? else {
                continue;
            };
            let mut out = Map::new();
            let user_id = guardian.user_id();
            let (accepted, not_accepted, revoked) = match user_id {
                Some(uid) => (
                    policy.includes(&mut *conn, Users::Accepted, uid).await?,
                    policy.includes(&mut *conn, Users::NotAccepted, uid).await?,
                    policy.includes(&mut *conn, Users::Revoked, uid).await?,
                ),
                None => (false, false, false),
            };
            let authed = user_id.is_some();
            out.insert(
                "policy_can_accept".into(),
                json!(authed && (easy_revoke || not_accepted)),
            );
            out.insert(
                "policy_can_revoke".into(),
                json!(authed && (easy_revoke || accepted)),
            );
            out.insert("policy_accepted".into(), json!(authed && accepted));
            out.insert("policy_revoked".into(), json!(authed && revoked));
            // include_policy_stats?
            let stats = if guardian.is_admin() {
                true
            } else if policy.private {
                false
            } else {
                let groups = policy.group_ids(&mut *conn).await?;
                can_see_groups_members(&mut *conn, guardian, &groups).await?
            };
            if stats {
                let ids = policy
                    .users(
                        &mut *conn,
                        Users::NotAccepted,
                        Some(POLICY_USER_DEFAULT_LIMIT),
                        0,
                    )
                    .await?;
                out.insert(
                    "policy_not_accepted_by".into(),
                    Value::Array(basic_users(&mut *conn, urls, &ids).await?),
                );
                out.insert(
                    "policy_not_accepted_by_count".into(),
                    json!(policy.count(&mut *conn, Users::NotAccepted).await?),
                );
                let ids = policy
                    .users(
                        &mut *conn,
                        Users::Accepted,
                        Some(POLICY_USER_DEFAULT_LIMIT),
                        0,
                    )
                    .await?;
                out.insert(
                    "policy_accepted_by".into(),
                    Value::Array(basic_users(&mut *conn, urls, &ids).await?),
                );
                out.insert(
                    "policy_accepted_by_count".into(),
                    json!(policy.count(&mut *conn, Users::Accepted).await?),
                );
                out.insert("policy_stats".into(), json!(true));
            }
            data.keys.push((post_id, out));
        }
        Ok(data)
    }

    /// The post's keys, in the serializer's order, after core's.
    pub fn post_keys(&self, post_id: i32, out: &mut Map<String, Value>) {
        if let Some((_, keys)) = self.keys.iter().find(|(id, _)| *id == post_id) {
            for (k, v) in keys {
                out.insert(k.clone(), v.clone());
            }
        }
    }
}

/// The current user's `can_create_policy`.
pub async fn current_user_keys(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    user_id: i32,
    out: &mut Map<String, Value>,
) -> Result<(), PluginError> {
    out.insert(
        "can_create_policy".into(),
        json!(user_allowed(conn, settings, Some(user_id)).await?),
    );
    Ok(())
}

/// UserOptionSerializer's `policy_email_frequency`.
pub async fn user_option_keys(
    conn: &mut PgConnection,
    user_id: i32,
    out: &mut Map<String, Value>,
) -> Result<(), sqlx::Error> {
    let frequency: Option<i32> =
        sqlx::query_scalar("SELECT policy_email_frequency FROM user_options WHERE user_id = $1")
            .bind(user_id)
            .fetch_optional(conn)
            .await?;
    out.insert(
        "policy_email_frequency".into(),
        frequency
            .and_then(|f| EMAIL_FREQUENCIES.get(usize::try_from(f).ok()?))
            .map(|f| json!(f))
            .unwrap_or(Value::Null),
    );
    Ok(())
}

/// The unaccepted-policies report's rows (`add_report`): each topic with
/// a member of a policy's group who has not accepted its current version.
pub async fn unaccepted_policies(conn: &mut PgConnection) -> Result<Vec<(i32, i32)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT DISTINCT t.id AS topic_id, gu.user_id AS user_id \
         FROM post_policies pp \
         JOIN post_policy_groups pg ON pg.post_policy_id = pp.id \
         JOIN posts p ON p.id = pp.post_id AND p.deleted_at IS NULL \
         JOIN topics t ON t.id = p.topic_id AND t.deleted_at IS NULL \
         JOIN group_users gu ON gu.group_id = pg.group_id \
         LEFT JOIN policy_users pu ON \
           pu.user_id = gu.user_id AND \
           pu.post_policy_id = pp.id AND \
           pu.accepted_at IS NOT NULL AND \
           pu.revoked_at IS NULL AND \
           (pu.expired_at IS NULL OR pu.expired_at < pu.accepted_at) AND \
           ((pu.version IS NULL AND pp.version IS NULL) OR \
           (pp.version IS NOT NULL AND pu.version IS NOT NULL AND pu.version = pp.version)) \
         WHERE pu.id IS NULL",
    )
    .fetch_all(conn)
    .await
}

/// `add_report("unaccepted-policies")`: a table of each topic and member
/// who has not accepted its policy.
pub async fn report(
    conn: &mut PgConnection,
    cx: &crate::reports::Cx<'_>,
    r: &mut crate::reports::Report,
) -> Result<(), crate::AppError> {
    r.modes = vec!["table".into()];
    r.labels = json!([
        {"property": "topic_id", "title": cx.t("reports.unaccepted-policies.labels.topic_id")?},
        {"property": "user_id", "title": cx.t("reports.unaccepted-policies.labels.user_id")?},
    ]);
    r.data = Value::Array(
        unaccepted_policies(conn)
            .await?
            .into_iter()
            .map(|(topic_id, user_id)| json!({"user_id": user_id, "topic_id": topic_id}))
            .collect(),
    );
    Ok(())
}
