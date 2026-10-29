//! discourse-policy's hooks on the post's life: `on(:post_process_cooked)`,
//! which keeps the post's `post_policies` row, its groups and the
//! `HasPolicy` custom field in step with the first `.policy` of the
//! processed HTML, and `validate(:post, :validate_policy)`
//! (`DiscoursePolicy::PostValidator`), who may add, change or remove one.

use chrono::{NaiveDate, NaiveDateTime};
use markup5ever_rcdom::Handle;
use sqlx::PgConnection;

use super::{HAS_POLICY, RENEW_INTERVALS, can_edit_group, enabled, group_id_by_name, user_allowed};
use crate::pretty_text::cleanup::{attr, element_name, fragment_root, has_class, node_html, parse};
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

/// Each element with the class `policy` (`div.policy` when `div_only`)
/// without a blockquote among its ancestors, in document order.
fn policies(node: &Handle, quoted: bool, div_only: bool, out: &mut Vec<Handle>) {
    for child in node.children.borrow().iter() {
        let name = element_name(child);
        if !quoted && has_class(child, "policy") && (!div_only || name == Some("div")) {
            out.push(child.clone());
        }
        policies(child, quoted || name == Some("blockquote"), div_only, out);
    }
}

/// `PostValidator#extract_policies`: each `div.policy` outside a
/// blockquote, as HTML.
pub fn extract(cooked: &str) -> Vec<String> {
    if cooked.trim().is_empty() {
        return Vec::new();
    }
    let dom = parse(cooked);
    let mut found = Vec::new();
    policies(&fragment_root(&dom), false, true, &mut found);
    found.iter().map(node_html).collect()
}

/// `policy_target_group_name`: the policy's `data-add-users-to-group`.
fn target_group_name(policy_html: &str) -> Option<String> {
    let dom = parse(policy_html);
    let mut found = Vec::new();
    policies(&fragment_root(&dom), false, true, &mut found);
    found
        .first()
        .and_then(|p| attr(p, "data-add-users-to-group"))
}

/// What the validator reads of the post being saved.
pub struct Saving<'a> {
    /// The cooked HTML as it stands before the save (none for a new post).
    pub old_cooked: Option<&'a str>,
    pub new_raw: &'a str,
    pub wiki: bool,
    /// `acting_user` (the editor, else the author).
    pub acting_user_id: Option<i32>,
    pub user_id: Option<i32>,
}

/// `validate(:post, :validate_policy)` on a save whose raw or wiki flag
/// changed, while the plugin is on: the error it adds, if any.
pub async fn validate_post(
    conn: &mut PgConnection,
    host: &crate::pretty_text::Host,
    settings: &SiteSettings,
    i18n: &crate::i18n::I18n,
    post: &Saving<'_>,
) -> Result<Option<String>, AppError> {
    if !enabled(settings)? {
        return Ok(None);
    }
    let t = |key: &str| i18n.t(key).unwrap_or(key).to_string();
    // Neither side can hold one: nothing to cook.
    let mentions = |s: &str| s.to_ascii_lowercase().contains("policy");
    if !post.old_cooked.is_some_and(mentions) && !mentions(post.new_raw) {
        return Ok(None);
    }
    let old = extract(post.old_cooked.unwrap_or(""));
    // PrettyText.cook(new_raw, {})
    let cooked = crate::pretty_text::cook(
        host,
        post.new_raw,
        &crate::pretty_text::MarkdownOptions::default(),
    )
    .await?;
    let new = extract(&cooked);
    if post.wiki && !new.is_empty() {
        return Ok(Some(t("discourse_policy.errors.policy_cannot_be_wiki")));
    }
    if old == new {
        return Ok(None);
    }
    let denied = Some(t("discourse_policy.errors.no_policy_permission"));
    if !user_allowed(conn, settings, post.acting_user_id).await?
        || !user_allowed(conn, settings, post.user_id).await?
    {
        return Ok(denied);
    }
    // acting_user_can_manage_target_groups?: a target group set or changed
    // must be one the editor may manage, when the author may.
    let old_targets: Vec<Option<String>> = old.iter().map(|p| target_group_name(p)).collect();
    for (index, policy) in new.iter().enumerate() {
        let Some(target) = target_group_name(policy).filter(|t| !crate::ruby::is_blank(t)) else {
            continue;
        };
        if old_targets.get(index).cloned().flatten().as_deref() == Some(target.as_str()) {
            continue;
        }
        let Some(group_id) = group_id_by_name(conn, &target).await? else {
            continue;
        };
        if can_edit_group(conn, settings, post.user_id, group_id).await?
            && !can_edit_group(conn, settings, post.acting_user_id, group_id).await?
        {
            return Ok(denied);
        }
    }
    Ok(None)
}

/// A `post_policies` row as the handler reads and writes it.
#[derive(Debug, Clone, Default, PartialEq, sqlx::FromRow)]
struct Row {
    id: Option<i64>,
    renew_start: Option<NaiveDateTime>,
    renew_days: Option<i32>,
    next_renew_at: Option<NaiveDateTime>,
    reminder: Option<String>,
    last_reminded_at: Option<NaiveDateTime>,
    version: Option<String>,
    renew_interval: Option<i32>,
    private: bool,
    add_users_to_group: Option<i32>,
}

/// `Date.parse(value)` for `data-renew-start`: None where Ruby raises
/// ArgumentError (nothing that reads as a date).
fn parse_date(value: &str) -> Result<Option<NaiveDate>, Unsupported> {
    let h = crate::ruby_date::parse(value);
    match (h.year, h.mon, h.mday) {
        (Some(y), Some(m), Some(d)) => Ok(NaiveDate::from_ymd_opt(y as i32, m as u32, d as u32)),
        (None, None, None) if h.yday.is_none() && h.cwyear.is_none() && h.wday.is_none() => {
            Ok(None)
        }
        // Date.parse completes partial dates from today; not followed.
        _ => Err(Unsupported("a policy renew-start that is not a full date")),
    }
}

/// `on(:post_process_cooked)`: the first `.policy` outside a blockquote,
/// for an author in `create_policy_allowed_groups`, becomes the post's
/// policy, kept only while one of its groups exists; otherwise the
/// post's policy and its `HasPolicy` field go.
pub async fn post_process_cooked(
    state: &AppState,
    conn: &mut PgConnection,
    settings: &SiteSettings,
    post_id: i32,
    user_id: Option<i32>,
    cooked: &str,
) -> Result<(), AppError> {
    if !enabled(settings)? {
        return Ok(());
    }
    let now = crate::clock::now_naive();
    let has_field: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM post_custom_fields WHERE post_id = $1 AND name = $2)",
    )
    .bind(post_id)
    .bind(HAS_POLICY)
    .fetch_one(&mut *conn)
    .await?;
    let mut has_group = false;
    // `post_policy&.new_record?`: a policy built here and never saved.
    let mut built_unsaved = false;
    // The first policy's attributes, read before any await (the parsed
    // document is not Send).
    let first: Option<Vec<(String, String)>> = if user_allowed(conn, settings, user_id).await? {
        let dom = parse(cooked);
        let mut found = Vec::new();
        policies(&fragment_root(&dom), false, false, &mut found);
        found.first().map(|p| match &p.data {
            markup5ever_rcdom::NodeData::Element { attrs, .. } => attrs
                .borrow()
                .iter()
                .map(|a| (a.name.local.to_string(), a.value.to_string()))
                .collect(),
            _ => Vec::new(),
        })
    } else {
        None
    };
    if let Some(policy) = first {
        let existing: Option<Row> = sqlx::query_as(
            "SELECT id, renew_start, renew_days, next_renew_at, reminder, last_reminded_at, \
                    version, renew_interval, private, add_users_to_group \
             FROM post_policies WHERE post_id = $1 ORDER BY id LIMIT 1",
        )
        .bind(i64::from(post_id))
        .fetch_optional(&mut *conn)
        .await?;
        let before = existing.clone();
        let mut row = existing.unwrap_or_default();
        let data = |name: &str| {
            let key = format!("data-{name}");
            policy
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.clone())
        };

        let mut names: Vec<String> = Vec::new();
        if let Some(group) = data("group") {
            names.push(group);
        }
        if let Some(groups) = data("groups") {
            names.extend(groups.split(',').map(str::to_string));
        }
        let new_group_ids: Vec<i32> =
            sqlx::query_scalar("SELECT id FROM groups WHERE name = ANY($1) ORDER BY id")
                .bind(&names)
                .fetch_all(&mut *conn)
                .await?;
        has_group = !new_group_ids.is_empty();
        // `post_policy.post_policy_groups = new_relations`: a saved policy's
        // groups change at once; a new one's with its save.
        if let Some(id) = row.id {
            sync_groups(conn, id, &new_group_ids, now).await?;
        }

        let renew = data("renew");
        let interval = renew
            .as_deref()
            .and_then(|r| RENEW_INTERVALS.iter().position(|i| *i == r));
        let renew_days = renew.as_deref().map(crate::ruby::to_i).unwrap_or(0);
        if renew_days > 0 || interval.is_some() {
            row.renew_days = if interval.is_some() {
                None
            } else {
                Some(renew_days as i32)
            };
            row.renew_interval = if row.renew_days.is_some() {
                None
            } else {
                interval.map(|i| i as i32)
            };
            row.renew_start = None;
            match data("renew-start") {
                Some(start) => {
                    if let Some(date) = parse_date(&start)? {
                        let start = date.and_hms_opt(0, 0, 0).expect("midnight exists");
                        row.renew_start = Some(start);
                        if row.next_renew_at.is_none_or(|at| at < start) {
                            row.next_renew_at = Some(start);
                        }
                    }
                }
                None => row.next_renew_at = None,
            }
        } else {
            row.renew_days = None;
            row.renew_start = None;
            row.next_renew_at = None;
        }

        let mut version_changed = false;
        if let Some(version) = data("version") {
            let old_version = row.version.clone().unwrap_or_else(|| "1".to_string());
            if version != old_version {
                row.version = Some(version);
                version_changed = true;
                // The users who accepted (this new version) leave the
                // group accepting adds them to.
                if let (Some(policy_id), Some(group_id)) = (row.id, row.add_users_to_group) {
                    let accepted: Vec<i32> = sqlx::query_scalar(
                        "SELECT user_id FROM policy_users WHERE post_policy_id = $1 \
                         AND accepted_at IS NOT NULL AND revoked_at IS NULL AND expired_at IS NULL \
                         AND version IS NOT DISTINCT FROM $2",
                    )
                    .bind(policy_id)
                    .bind(&row.version)
                    .fetch_all(&mut *conn)
                    .await?;
                    if let Some(group) = crate::group_manager::find(&mut *conn, group_id).await? {
                        bulk_remove(state, conn, settings, &group, &accepted).await?;
                    }
                }
            }
        }

        if let Some(reminder) = data("reminder") {
            row.reminder = Some(reminder);
            row.last_reminded_at.get_or_insert(now);
        }
        row.private = data("private").as_deref() == Some("true");
        row.add_users_to_group = match data("add-users-to-group").filter(|g| !g.trim().is_empty()) {
            Some(name) => match group_id_by_name(conn, &name).await? {
                Some(group_id) if can_edit_group(conn, settings, user_id, group_id).await? => {
                    Some(group_id)
                }
                _ => None,
            },
            None => None,
        };

        if has_group {
            if !has_field {
                sqlx::query(
                    "INSERT INTO post_custom_fields (post_id, name, value, created_at, updated_at) \
                     VALUES ($1, $2, 't', $3, $3)",
                )
                .bind(post_id)
                .bind(HAS_POLICY)
                .bind(now)
                .execute(&mut *conn)
                .await?;
            }
            save(
                conn,
                post_id,
                &row,
                before.as_ref(),
                version_changed,
                &new_group_ids,
                now,
            )
            .await?;
        } else {
            built_unsaved = row.id.is_none();
        }
    }
    if !has_group && (has_field || !built_unsaved) {
        sqlx::query("DELETE FROM post_custom_fields WHERE post_id = $1 AND name = $2")
            .bind(post_id)
            .bind(HAS_POLICY)
            .execute(&mut *conn)
            .await?;
        // `PostPolicy.where(post_id:).destroy_all`, their groups with them.
        sqlx::query(
            "DELETE FROM post_policy_groups WHERE post_policy_id IN \
             (SELECT id FROM post_policies WHERE post_id = $1)",
        )
        .bind(i64::from(post_id))
        .execute(&mut *conn)
        .await?;
        sqlx::query("DELETE FROM post_policies WHERE post_id = $1")
            .bind(i64::from(post_id))
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

/// A policy's groups made `group_ids`: the others destroyed, the missing
/// ones created.
async fn sync_groups(
    conn: &mut PgConnection,
    policy_id: i64,
    group_ids: &[i32],
    now: NaiveDateTime,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "DELETE FROM post_policy_groups WHERE post_policy_id = $1 AND NOT (group_id = ANY($2))",
    )
    .bind(policy_id)
    .bind(group_ids)
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "INSERT INTO post_policy_groups (post_policy_id, group_id, created_at, updated_at) \
         SELECT $1, g, $3, $3 FROM unnest($2::int[]) WITH ORDINALITY AS t(g, n) \
         WHERE NOT EXISTS (SELECT 1 FROM post_policy_groups WHERE post_policy_id = $1 AND group_id = g) \
         ORDER BY n",
    )
    .bind(policy_id)
    .bind(group_ids)
    .bind(now)
    .execute(conn)
    .await?;
    Ok(())
}

/// `post_policy.save!`: a new policy inserted with its groups, a saved
/// one updated when it changed; `bump_policy` stamps `last_bumped_at`
/// when the version did.
async fn save(
    conn: &mut PgConnection,
    post_id: i32,
    row: &Row,
    before: Option<&Row>,
    version_changed: bool,
    group_ids: &[i32],
    now: NaiveDateTime,
) -> Result<(), sqlx::Error> {
    let bumped = version_changed.then_some(now);
    match row.id {
        None => {
            let id: i64 = sqlx::query_scalar(
                "INSERT INTO post_policies (post_id, renew_start, renew_days, next_renew_at, reminder, \
                    last_reminded_at, version, renew_interval, private, add_users_to_group, \
                    last_bumped_at, created_at, updated_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $12) RETURNING id",
            )
            .bind(i64::from(post_id))
            .bind(row.renew_start)
            .bind(row.renew_days)
            .bind(row.next_renew_at)
            .bind(&row.reminder)
            .bind(row.last_reminded_at)
            .bind(&row.version)
            .bind(row.renew_interval)
            .bind(row.private)
            .bind(row.add_users_to_group)
            .bind(bumped)
            .bind(now)
            .fetch_one(&mut *conn)
            .await?;
            sync_groups(conn, id, group_ids, now).await?;
        }
        Some(id) => {
            if before == Some(row) {
                return Ok(());
            }
            sqlx::query(
                "UPDATE post_policies SET renew_start = $2, renew_days = $3, next_renew_at = $4, \
                    reminder = $5, last_reminded_at = $6, version = $7, renew_interval = $8, \
                    private = $9, add_users_to_group = $10, \
                    last_bumped_at = COALESCE($11, last_bumped_at), updated_at = $12 \
                 WHERE id = $1",
            )
            .bind(id)
            .bind(row.renew_start)
            .bind(row.renew_days)
            .bind(row.next_renew_at)
            .bind(&row.reminder)
            .bind(row.last_reminded_at)
            .bind(&row.version)
            .bind(row.renew_interval)
            .bind(row.private)
            .bind(row.add_users_to_group)
            .bind(bumped)
            .bind(now)
            .execute(conn)
            .await?;
        }
    }
    Ok(())
}

/// `Group#bulk_remove(user_ids)`
async fn bulk_remove(
    state: &AppState,
    conn: &mut PgConnection,
    settings: &SiteSettings,
    group: &crate::group_manager::Group,
    user_ids: &[i32],
) -> Result<(), AppError> {
    if user_ids.is_empty() {
        return Ok(());
    }
    let host = crate::pretty_text::Host::from_state(state);
    let promotion = crate::promotion::Ctx {
        settings,
        i18n: &state.i18n,
        host: &host,
        config: &state.config,
    };
    let cx = crate::group_manager::Cx {
        state,
        promotion: &promotion,
    };
    crate::group_manager::remove_many(conn, &cx, group, user_ids).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoted_policies_are_left_out() {
        let cooked = "<div class=\"policy\" data-group=\"a\"><p>x</p></div>\
            <blockquote><div class=\"policy\" data-group=\"b\"></div></blockquote>\
            <span class=\"policy\"></span>";
        let found = extract(cooked);
        assert_eq!(found.len(), 1);
        assert!(found[0].contains("data-group=\"a\""));
        assert_eq!(target_group_name(&found[0]), None);
    }

    #[test]
    fn renew_start_reads_full_dates_only() {
        assert_eq!(
            parse_date("2020-10-17").unwrap(),
            NaiveDate::from_ymd_opt(2020, 10, 17)
        );
        assert_eq!(parse_date("banana").unwrap(), None);
        assert!(parse_date("2020-02-31").unwrap().is_none());
    }
}
