//! discourse-presence: who is replying to a topic (and whispering in it),
//! editing a post or translating it, as the composer and the topic's
//! footer show. Its channels (plugin.rb's register_presence_channel_prefix
//! "discourse-presence") decide who may see that, by the topic's and the
//! post's own audience.

use std::collections::BTreeSet;

use sqlx::PgConnection;

use crate::AppError;
use crate::guardian::Guardian;
use crate::presence::Config;
use crate::site_settings::SiteSettings;

/// admins and moderators.
const STAFF_GROUPS: [i32; 2] = [1, 2];
const ADMINS: i32 = 1;
const EVERYONE: i32 = 0;
const LOGGED_IN_USERS: i32 = 5;
const TRUST_LEVEL_0: i32 = 10;

/// `enabled_site_setting :presence_enabled`
pub fn enabled(settings: &SiteSettings) -> Result<bool, AppError> {
    Ok(settings.get("presence_enabled")?.truthy())
}

struct Topic {
    id: i32,
    private_message: bool,
    category_id: Option<i32>,
    /// `secure_group_ids`: the category's groups when it is read restricted.
    secure_group_ids: Option<Vec<i32>>,
}

/// `Topic.find`: trashed topics are out of its default scope.
async fn find_topic(conn: &mut PgConnection, topic_id: i64) -> Result<Option<Topic>, AppError> {
    let row: Option<(i32, String, Option<i32>, Option<bool>)> = sqlx::query_as(
        "SELECT t.id, t.archetype, t.category_id, c.read_restricted FROM topics t \
         LEFT JOIN categories c ON c.id = t.category_id \
         WHERE t.id = $1 AND t.deleted_at IS NULL",
    )
    .bind(topic_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((id, archetype, category_id, read_restricted)) = row else {
        return Ok(None);
    };
    let secure_group_ids = match (category_id, read_restricted) {
        (Some(category_id), Some(true)) => Some(
            sqlx::query_scalar(
                "SELECT group_id FROM category_groups WHERE category_id = $1 ORDER BY id",
            )
            .bind(category_id)
            .fetch_all(&mut *conn)
            .await?,
        ),
        _ => None,
    };
    Ok(Some(Topic {
        id,
        private_message: archetype == "private_message",
        category_id,
        secure_group_ids,
    }))
}

/// `topic.allowed_users`
async fn allowed_users(conn: &mut PgConnection, topic_id: i32) -> Result<Vec<i32>, AppError> {
    Ok(sqlx::query_scalar(
        "SELECT user_id FROM topic_allowed_users WHERE topic_id = $1 ORDER BY id",
    )
    .bind(topic_id)
    .fetch_all(&mut *conn)
    .await?)
}

/// `topic.allowed_groups`
async fn allowed_groups(conn: &mut PgConnection, topic_id: i32) -> Result<Vec<i32>, AppError> {
    Ok(sqlx::query_scalar(
        "SELECT group_id FROM topic_allowed_groups WHERE topic_id = $1 ORDER BY id",
    )
    .bind(topic_id)
    .fetch_all(&mut *conn)
    .await?)
}

/// `topic.all_allowed_users`: the topic's users, its groups' members and,
/// for a flagged PM or an official warning, the moderators.
async fn all_allowed_users(conn: &mut PgConnection, topic_id: i32) -> Result<Vec<i32>, AppError> {
    Ok(sqlx::query_scalar(
        "SELECT user_id FROM topic_allowed_users WHERE topic_id = $1 \
         UNION SELECT gu.user_id FROM group_users gu \
           JOIN topic_allowed_groups tag ON tag.group_id = gu.group_id WHERE tag.topic_id = $1 \
         UNION SELECT u.id FROM users u WHERE u.moderator AND ( \
           EXISTS (SELECT 1 FROM reviewables r WHERE r.topic_id = $1 \
                   AND r.type = 'ReviewableFlaggedPost' AND r.status = 0) \
           OR EXISTS (SELECT 1 FROM topics t WHERE t.id = $1 AND t.subtype = 'moderator_warning'))",
    )
    .bind(topic_id)
    .fetch_all(&mut *conn)
    .await?)
}

/// The members of any of the groups.
async fn members(conn: &mut PgConnection, groups: &[i32]) -> Result<BTreeSet<i32>, AppError> {
    let ids: Vec<i32> =
        sqlx::query_scalar("SELECT DISTINCT user_id FROM group_users WHERE group_id = ANY($1)")
            .bind(groups)
            .fetch_all(&mut *conn)
            .await?;
    Ok(ids.into_iter().collect())
}

fn group_setting(settings: &SiteSettings, name: &str) -> Result<Vec<i32>, AppError> {
    Ok(settings
        .group_ids(name)?
        .into_iter()
        .map(|g| g as i32)
        .collect())
}

/// Ruby's `a & b`: a's members also in b, once each, in a's order.
fn intersect(a: &[i32], b: &[i32]) -> Vec<i32> {
    let mut out: Vec<i32> = Vec::new();
    for x in a {
        if b.contains(x) && !out.contains(x) {
            out.push(*x);
        }
    }
    out
}

fn uniq(mut v: Vec<i32>) -> Vec<i32> {
    let mut seen = BTreeSet::new();
    v.retain(|x| seen.insert(*x));
    v
}

/// `.presence`: None for an empty list.
fn presence(v: Vec<i32>) -> Option<Vec<i32>> {
    (!v.is_empty()).then_some(v)
}

/// The config for a `/discourse-presence/...` channel, None when its
/// topic or post isn't found or the name means nothing.
pub async fn config(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    name: &str,
) -> Result<Option<Config>, AppError> {
    if let Some(id) = id_after(name, "/discourse-presence/reply/") {
        let Some(topic) = find_topic(conn, id).await? else {
            return Ok(None);
        };
        return Ok(Some(reply(conn, &topic).await?));
    }
    if let Some(id) = id_after(name, "/discourse-presence/whisper/") {
        let Some(topic) = find_topic(conn, id).await? else {
            return Ok(None);
        };
        return Ok(Some(whisper(conn, settings, &topic).await?));
    }
    if let Some(id) = id_after(name, "/discourse-presence/edit/") {
        return edit(conn, settings, id).await;
    }
    if let Some(id) = id_after(name, "/discourse-presence/translate/") {
        return translate(conn, settings, id).await;
    }
    Ok(None)
}

/// `channel_name[%r{/discourse-presence/<kind>/(\d+)}, 1]`: the digits
/// after the first match of the path, anywhere in the name.
fn id_after(name: &str, path: &str) -> Option<i64> {
    let at = name.find(path)?;
    let digits: String = name[at + path.len()..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

async fn reply(conn: &mut PgConnection, topic: &Topic) -> Result<Config, AppError> {
    let mut config = Config::default();
    if topic.private_message {
        config.allowed_user_ids = Some(allowed_users(conn, topic.id).await?);
        let mut groups = allowed_groups(conn, topic.id).await?;
        groups.extend(STAFF_GROUPS);
        config.allowed_group_ids = Some(groups);
    } else if let Some(secure) = &topic.secure_group_ids {
        let mut groups = secure.clone();
        groups.push(ADMINS);
        config.allowed_group_ids = Some(groups);
    } else {
        // public: true would tell anonymous readers too; trust level 0
        // is every member instead.
        config.allowed_group_ids = Some(vec![TRUST_LEVEL_0]);
    }
    Ok(config)
}

async fn whisper(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    topic: &Topic,
) -> Result<Config, AppError> {
    let whisperers = group_setting(settings, "whispers_allowed_groups")?;
    let suppress = settings
        .get("suppress_secured_categories_from_admin")?
        .truthy();
    let shared_draft: bool = !topic.private_message
        && sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM shared_drafts WHERE topic_id = $1)")
            .bind(topic.id)
            .fetch_one(&mut *conn)
            .await?;
    if shared_draft {
        // Members of every required group set see it; those in a group
        // all of them share are covered by the group itself.
        let draft_groups = group_setting(settings, "shared_drafts_allowed_groups")?;
        let mut required = vec![whisperers.clone()];
        if intersect(&draft_groups, &[EVERYONE, LOGGED_IN_USERS]).is_empty() {
            required.push(draft_groups);
        }
        if let Some(secure) = &topic.secure_group_ids {
            required.push(secure.clone());
        }
        let mut shared = required[0].clone();
        for groups in &required[1..] {
            shared = intersect(&shared, groups);
        }
        let mut users = members(conn, &required[0]).await?;
        for groups in &required[1..] {
            let in_groups = members(conn, groups).await?;
            users.retain(|u| in_groups.contains(u));
        }
        if !shared.is_empty() {
            let in_shared = members(conn, &shared).await?;
            users.retain(|u| !in_shared.contains(u));
        }
        let mut users: Vec<i32> = users.into_iter().collect();
        if !suppress && !whisperers.contains(&ADMINS) {
            let admins = members(conn, &[ADMINS]).await?;
            users.extend(
                members(conn, &whisperers)
                    .await?
                    .into_iter()
                    .filter(|u| admins.contains(u)),
            );
        } else if !suppress {
            shared.push(ADMINS);
        }
        return Ok(Config {
            allowed_user_ids: presence(uniq(users)),
            allowed_group_ids: presence(uniq(shared)),
            ..Config::default()
        });
    }
    let (mut topic_groups, topic_users) = if topic.private_message {
        (
            allowed_groups(conn, topic.id).await?,
            all_allowed_users(conn, topic.id).await?,
        )
    } else if let Some(secure) = &topic.secure_group_ids {
        (secure.clone(), Vec::new())
    } else {
        return Ok(Config {
            allowed_group_ids: Some(whisperers),
            ..Config::default()
        });
    };
    if !suppress {
        topic_groups.push(ADMINS);
    }
    let shared = intersect(&whisperers, &topic_groups);
    let whisper_only: Vec<i32> = whisperers
        .iter()
        .copied()
        .filter(|g| !shared.contains(g))
        .collect();
    let topic_only: Vec<i32> = topic_groups
        .iter()
        .copied()
        .filter(|g| !shared.contains(g))
        .collect();
    let mut users: Vec<i32> = if topic.private_message && !whisper_only.is_empty() {
        let whisper_members = members(conn, &whisper_only).await?;
        topic_users
            .into_iter()
            .filter(|u| whisper_members.contains(u))
            .collect()
    } else {
        Vec::new()
    };
    // Groups combine with OR, so only members of both sides need naming.
    if !whisper_only.is_empty() && !topic_only.is_empty() {
        let topic_members = members(conn, &topic_only).await?;
        users.extend(
            members(conn, &whisper_only)
                .await?
                .into_iter()
                .filter(|u| topic_members.contains(u)),
        );
    }
    Ok(Config {
        allowed_user_ids: presence(uniq(users)),
        allowed_group_ids: presence(shared),
        ..Config::default()
    })
}

struct Post {
    user_id: Option<i32>,
    topic_id: i64,
    locked: bool,
    whisper: bool,
    wiki: bool,
}

/// `Post.find`: trashed posts are out of its default scope.
async fn find_post(conn: &mut PgConnection, post_id: i64) -> Result<Option<Post>, AppError> {
    // locked?: locked_by_id present; whisper?: Post.types[:whisper] (4).
    let row: Option<(Option<i32>, i64, bool, bool, bool)> = sqlx::query_as(
        "SELECT user_id, topic_id::bigint, locked_by_id IS NOT NULL, post_type = 4, wiki \
         FROM posts WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(post_id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.map(|(user_id, topic_id, locked, whisper, wiki)| Post {
        user_id,
        topic_id,
        locked,
        whisper,
        wiki,
    }))
}

/// `post.user.guardian.can_see?(topic)`
async fn author_sees(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    user_id: Option<i32>,
    topic_id: i32,
) -> Result<bool, AppError> {
    let Some(user_id) = user_id else {
        return Ok(false);
    };
    let Some(user) = crate::session::current::SessionUser::load(&mut *conn, user_id).await? else {
        return Ok(false);
    };
    let guardian = Guardian::for_user(&mut *conn, &user).await?;
    let Some(topic) =
        crate::topic_guardian::TopicCtx::load(&mut *conn, settings, &guardian, topic_id).await?
    else {
        return Ok(false);
    };
    let secure = guardian.secure_category_ids(&mut *conn, settings).await?;
    Ok(guardian.can_see_topic(settings, &topic, true, &secure)?)
}

/// A setting's groups, narrowed to the topic's secure groups when it has
/// them.
fn within(groups: Vec<i32>, topic: &Topic) -> Vec<i32> {
    match &topic.secure_group_ids {
        Some(secure) => intersect(&groups, secure),
        None => groups,
    }
}

async fn edit(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    post_id: i64,
) -> Result<Option<Config>, AppError> {
    let Some(post) = find_post(conn, post_id).await? else {
        return Ok(None);
    };
    let Some(topic) = find_topic(conn, post.topic_id).await? else {
        return Ok(None);
    };
    let mut groups = STAFF_GROUPS.to_vec();
    let mut users: Option<Vec<i32>> = None;
    // Locked posts are staff only.
    if post.locked {
        return Ok(Some(Config {
            allowed_group_ids: Some(groups),
            ..Config::default()
        }));
    }
    if post.whisper {
        if topic.private_message {
            users = Some(allowed_users(conn, topic.id).await?);
            groups.extend(allowed_groups(conn, topic.id).await?);
        } else {
            groups.extend(within(
                group_setting(settings, "whispers_allowed_groups")?,
                &topic,
            ));
        }
        return Ok(Some(Config {
            allowed_user_ids: users,
            allowed_group_ids: Some(groups),
            ..Config::default()
        }));
    }
    if author_sees(conn, settings, post.user_id, topic.id).await? {
        users = post.user_id.map(|id| vec![id]);
    }
    if topic.private_message && post.wiki {
        let mut all = users.unwrap_or_default();
        all.extend(allowed_users(conn, topic.id).await?);
        users = Some(all);
        groups.extend(allowed_groups(conn, topic.id).await?);
    } else if post.wiki {
        groups.extend(within(
            group_setting(settings, "edit_wiki_post_allowed_groups")?,
            &topic,
        ));
    }
    let edit_all = group_setting(settings, "edit_all_post_groups")?;
    if !topic.private_message && !edit_all.is_empty() {
        groups.extend(within(edit_all, &topic));
    }
    if settings.get("enable_category_group_moderation")?.truthy()
        && let Some(category_id) = topic.category_id
    {
        let moderating: Vec<i32> = sqlx::query_scalar(
            "SELECT group_id FROM category_moderation_groups WHERE category_id = $1 ORDER BY id",
        )
        .bind(category_id)
        .fetch_all(&mut *conn)
        .await?;
        groups.extend(within(moderating, &topic));
    }
    Ok(Some(Config {
        allowed_user_ids: users,
        allowed_group_ids: Some(uniq(groups)),
        ..Config::default()
    }))
}

async fn translate(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    post_id: i64,
) -> Result<Option<Config>, AppError> {
    let Some(post) = find_post(conn, post_id).await? else {
        return Ok(None);
    };
    let Some(topic) = find_topic(conn, post.topic_id).await? else {
        return Ok(None);
    };
    let mut groups = STAFF_GROUPS.to_vec();
    let mut users: Vec<i32> = Vec::new();
    if topic.private_message {
        users.extend(allowed_users(conn, topic.id).await?);
        groups.extend(allowed_groups(conn, topic.id).await?);
    } else if let Some(secure) = &topic.secure_group_ids {
        groups.extend(secure.iter().copied());
    } else if settings.get("content_localization_enabled")?.truthy() {
        groups.extend(group_setting(
            settings,
            "content_localization_allowed_groups",
        )?);
    }
    if settings
        .get("content_localization_allow_author_localization")?
        .truthy()
        && author_sees(conn, settings, post.user_id, topic.id).await?
        && let Some(id) = post.user_id
    {
        users.push(id);
    }
    Ok(Some(Config {
        allowed_user_ids: Some(users),
        allowed_group_ids: Some(uniq(groups)),
        ..Config::default()
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_follow_the_path_anywhere_in_the_name() {
        assert_eq!(
            id_after("/discourse-presence/reply/35", "/discourse-presence/reply/"),
            Some(35)
        );
        assert_eq!(
            id_after(
                "/discourse-presence/reply/35/x",
                "/discourse-presence/reply/"
            ),
            Some(35)
        );
        assert_eq!(
            id_after("/discourse-presence/reply/", "/discourse-presence/reply/"),
            None
        );
        assert_eq!(
            id_after("/discourse-presence/edit/7", "/discourse-presence/reply/"),
            None
        );
    }

    #[test]
    fn ruby_set_operations() {
        assert_eq!(intersect(&[3, 1, 2, 1], &[1, 3]), vec![3, 1]);
        assert_eq!(uniq(vec![1, 2, 1, 3, 2]), vec![1, 2, 3]);
        assert_eq!(presence(vec![]), None);
    }
}
