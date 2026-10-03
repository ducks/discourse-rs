//! Port of lib/post_action_type_view.rb (the `flags` table as action
//! types), `Guardian#post_can_act?` and `PostSerializer#actions_summary`.

use std::collections::HashMap;

use chrono::NaiveDateTime;
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::Unsupported;
use crate::guardian::{Guardian, GuardianError};
use crate::site_settings::SiteSettings;
use crate::topic_guardian::{PostCtx, TopicCtx};

/// `PostActionType::LIKE_POST_ACTION_ID`
pub const LIKE: i64 = 2;

#[derive(Debug, Clone, sqlx::FromRow)]
struct FlagRow {
    id: i64,
    name_key: String,
    notify_type: bool,
    require_message: bool,
    applies_to: Vec<String>,
    enabled: bool,
    score_type: bool,
    auto_action_type: bool,
}

/// `PostActionTypeView`: the action types in `types` order (like, then
/// the flags by position).
#[derive(Debug, Clone, Default)]
pub struct ActionTypes {
    /// `types`: `(name_key, id)` in iteration order.
    pub types: Vec<(String, i64)>,
    notify_flag_ids: Vec<i64>,
    additional_message_ids: Vec<i64>,
    disabled: Vec<String>,
    /// `auto_action_flag_types`: flags that may hide a post.
    auto_action_ids: Vec<i64>,
    /// `topic_flag_types` ids by position.
    pub topic_flag_ids: Vec<i64>,
}

impl ActionTypes {
    pub async fn load(conn: &mut PgConnection) -> Result<ActionTypes, sqlx::Error> {
        let rows: Vec<FlagRow> = sqlx::query_as(
            "SELECT id, name_key, notify_type, require_message, applies_to, enabled, score_type, auto_action_type \
             FROM flags ORDER BY position",
        )
        .fetch_all(conn)
        .await?;
        let flags: Vec<&FlagRow> = rows
            .iter()
            .filter(|f| !f.score_type && f.id != LIKE)
            .collect();
        let mut types = vec![("like".to_string(), LIKE)];
        types.extend(flags.iter().map(|f| (f.name_key.clone(), f.id)));
        Ok(ActionTypes {
            types,
            notify_flag_ids: flags
                .iter()
                .filter(|f| f.notify_type)
                .map(|f| f.id)
                .collect(),
            additional_message_ids: flags
                .iter()
                .filter(|f| f.require_message)
                .map(|f| f.id)
                .collect(),
            disabled: rows
                .iter()
                .filter(|f| !f.enabled)
                .map(|f| f.name_key.clone())
                .collect(),
            auto_action_ids: flags
                .iter()
                .filter(|f| f.auto_action_type)
                .map(|f| f.id)
                .collect(),
            topic_flag_ids: flags
                .iter()
                .filter(|f| f.applies_to.iter().any(|a| a == "Topic"))
                .map(|f| f.id)
                .collect(),
        })
    }

    /// The type's `name_key`.
    pub fn name(&self, id: i64) -> Option<&str> {
        self.types
            .iter()
            .find(|(_, i)| *i == id)
            .map(|(k, _)| k.as_str())
    }

    /// `notify_flag_type_ids`: flags that go to review.
    pub fn is_notify_flag(&self, id: i64) -> bool {
        self.notify_flag_ids.contains(&id)
    }

    /// `additional_message_types`: flags that send a message.
    pub fn requires_message(&self, id: i64) -> bool {
        self.additional_message_ids.contains(&id)
    }

    /// `auto_action_flag_types`
    pub fn is_auto_action(&self, id: i64) -> bool {
        self.auto_action_ids.contains(&id)
    }

    /// `is_flag?`: notify or message flags, i.e. everything but like.
    fn is_flag(&self, id: i64) -> bool {
        self.notify_flag_ids.contains(&id) || self.additional_message_ids.contains(&id)
    }
}

/// A viewer's post_actions row on a post (`PostAction.counts_for`). Undone
/// ones are not counted: PostAction is Trashable, whose default scope
/// leaves out deleted rows.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TakenAction {
    pub post_id: i32,
    pub post_action_type_id: i32,
    pub user_id: i32,
    pub created_at: NaiveDateTime,
}

/// `PostAction.counts_for(posts, user)`: `{post_id => {type_id => row}}`.
pub async fn taken_actions(
    conn: &mut PgConnection,
    post_ids: &[i32],
    user_id: Option<i32>,
) -> Result<HashMap<i32, HashMap<i64, TakenAction>>, sqlx::Error> {
    let Some(user_id) = user_id else {
        return Ok(HashMap::new());
    };
    let rows: Vec<TakenAction> = sqlx::query_as(
        "SELECT post_id, post_action_type_id, user_id, created_at FROM post_actions \
         WHERE post_id = ANY($1) AND user_id = $2 AND deleted_at IS NULL ORDER BY id",
    )
    .bind(post_ids)
    .bind(user_id)
    .fetch_all(conn)
    .await?;
    let mut out: HashMap<i32, HashMap<i64, TakenAction>> = HashMap::new();
    for row in rows {
        out.entry(row.post_id)
            .or_default()
            .insert(i64::from(row.post_action_type_id), row);
    }
    Ok(out)
}

/// The per-post inputs of `post_can_act?` beyond the post itself.
pub struct ActOpts<'a> {
    pub topic: &'a TopicCtx,
    pub post: &'a PostCtx,
    pub taken: Option<&'a HashMap<i64, TakenAction>>,
    pub can_see_post: bool,
    /// The author's user row is missing (`post.user.blank?`).
    pub author_missing: bool,
}

impl Guardian {
    /// `post_can_act?(post, action_key, opts:, can_see_post:)`
    pub fn post_can_act(
        &self,
        settings: &SiteSettings,
        types: &ActionTypes,
        action: (&str, i64),
        opts: &ActOpts<'_>,
    ) -> Result<bool, GuardianError> {
        let (key, id) = action;
        if !opts.can_see_post {
            return Ok(false);
        }
        if key == "notify_user" && opts.author_missing {
            return Ok(false);
        }
        let taken: Vec<i64> = opts
            .taken
            .map(|t| t.keys().copied().collect())
            .unwrap_or_default();
        let is_flag = types.is_flag(id);
        let already_taken_this_action = taken.contains(&id);
        let already_did_flagging = taken.iter().any(|t| types.notify_flag_ids.contains(t));
        let Some(user) = self.user() else {
            return Ok(false);
        };
        if settings.get("allow_anonymous_mode")?.truthy() {
            return Err(Unsupported("anonymous posting mode").into());
        }
        if (is_flag || key == "like") && self.is_silenced() {
            return Ok(false);
        }
        if is_flag && opts.post.hidden {
            return Ok(false);
        }
        if is_flag && !settings.get("allow_flagging_staff")?.truthy() && opts.post.author_staff {
            return Ok(false);
        }
        if is_flag && types.disabled.iter().any(|d| d == key) {
            return Ok(false);
        }
        if key == "notify_user"
            && !self.in_setting_groups(settings, "personal_message_enabled_groups")?
        {
            return Ok(false);
        }
        let flaggable = is_flag
            && !already_did_flagging
            && (self.in_setting_groups(settings, "flag_post_allowed_groups")?
                || opts.topic.private_message());
        let illegal = key == "illegal"
            && settings
                .get("allow_all_users_to_flag_illegal_content")?
                .truthy();
        let own_like = key == "like" && (opts.author_missing || opts.post.user_id == Some(user.id));
        let plain = !(is_flag
            || already_taken_this_action
            || opts.topic.archived
            || opts.post.trashed()
            || own_like);
        Ok(flaggable || illegal || plain)
    }

    /// `PostSerializer#actions_summary` for a post, given its `*_count`
    /// columns by type id and the viewer's taken actions.
    #[allow(clippy::too_many_arguments)]
    pub fn actions_summary(
        &self,
        settings: &SiteSettings,
        types: &ActionTypes,
        counts: &HashMap<i64, i32>,
        opts: &ActOpts<'_>,
        author_bot: bool,
    ) -> Result<Value, GuardianError> {
        let mut result = Vec::new();
        let taken = opts.taken;
        for (key, id) in &types.types {
            let count = counts.get(id).copied().unwrap_or(0);
            let mut summary = Map::new();
            summary.insert("id".into(), json!(id));
            summary.insert("count".into(), json!(count));
            let mut can_act = self.post_can_act(settings, types, (key, *id), opts)?;
            if key == "notify_user"
                && ((self.is_authenticated() && opts.post.user_id == self.user_id()) || author_bot)
            {
                can_act = false;
            }
            if can_act {
                summary.insert("can_act".into(), json!(true));
            }
            let mut acted = false;
            if let Some(action) = taken.and_then(|t| t.get(id)) {
                acted = true;
                summary.insert("acted".into(), json!(true));
                if self.can_delete_post_action(
                    settings,
                    opts.topic,
                    action.user_id,
                    action.created_at,
                )? {
                    summary.insert("can_undo".into(), json!(true));
                }
            }
            // Non-staff see only their own flag counts.
            let count = if self.is_staff() || *id == LIKE {
                count
            } else {
                i32::from(acted)
            };
            if count == 0 {
                summary.remove("count");
            } else {
                summary.insert("count".into(), json!(count));
            }
            if can_act || count > 0 || acted {
                result.push(Value::Object(summary));
            }
        }
        Ok(Value::Array(result))
    }
}
