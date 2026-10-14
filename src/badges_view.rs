//! The badge serializers: BadgeSerializer (with its badge type), and
//! DetailedUserBadgeSerializer with its badges, badge types and granters
//! side-loaded the way AMS embeds ids.

use chrono::NaiveDateTime;
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::guardian::Guardian;
use crate::i18n::I18n;
use crate::site_settings::SiteSettings;
use crate::topic_list::{TopicListSerializer, time_json};
use crate::url::Urls;
use crate::{AppError, Unsupported};

#[derive(Debug, sqlx::FromRow)]
pub(crate) struct BadgeRow {
    pub id: i32,
    pub name: String,
    pub description: Option<String>,
    pub long_description: Option<String>,
    pub grant_count: i32,
    pub allow_title: bool,
    pub multiple_grant: bool,
    pub icon: Option<String>,
    pub image_upload_url: Option<String>,
    pub listable: bool,
    pub enabled: bool,
    pub badge_grouping_id: i32,
    pub system: bool,
    pub show_in_post_header: bool,
    pub show_posts: bool,
    pub badge_type_id: i32,
    pub badge_type_name: String,
    pub grouping_position: Option<i32>,
}

pub(crate) const BADGE_ROW_SELECT: &str = "SELECT b.id, b.name, b.description, b.long_description, b.grant_count, b.allow_title, \
            b.multiple_grant, b.icon, u.url AS image_upload_url, b.listable, b.enabled, \
            b.badge_grouping_id, b.system, b.show_in_post_header, b.show_posts, b.badge_type_id, \
            bt.name AS badge_type_name, bg.position AS grouping_position \
     FROM badges b JOIN badge_types bt ON bt.id = b.badge_type_id \
     LEFT JOIN badge_groupings bg ON bg.id = b.badge_grouping_id \
     LEFT JOIN uploads u ON u.id = b.image_upload_id";

impl BadgeRow {
    pub(crate) async fn load(conn: &mut PgConnection, id: i32) -> Result<BadgeRow, sqlx::Error> {
        sqlx::query_as(&format!("{BADGE_ROW_SELECT} WHERE b.id = $1"))
            .bind(id)
            .fetch_one(conn)
            .await
    }

    fn i18n_name(&self) -> String {
        self.name.to_lowercase().replace(' ', "_")
    }

    /// `display_name`
    pub(crate) fn display_name(&self, i18n: &I18n) -> String {
        i18n.t(&format!("badges.{}.name", self.i18n_name()))
            .unwrap_or(&self.name)
            .to_string()
    }
}

/// `Badge#description` / `#long_description`: the locale's text with
/// `base_uri` and `max_likes_per_day`, else the column, else "".
fn badge_text(
    i18n: &I18n,
    settings: &SiteSettings,
    base_path: &str,
    key: &str,
    column: Option<&String>,
) -> Result<String, AppError> {
    let max_likes = settings.get("max_likes_per_day")?.to_s();
    Ok(i18n
        .t_with(
            key,
            &[("base_uri", base_path), ("max_likes_per_day", &max_likes)],
        )
        .or_else(|| column.cloned())
        .unwrap_or_default())
}

/// BadgeSerializer, its badge_type as an id; `long_description` with
/// `include_long_description`, `has_badge` when the viewer has it.
pub(crate) fn badge_json(
    i18n: &I18n,
    settings: &SiteSettings,
    urls: &Urls<'_>,
    base_path: &str,
    b: &BadgeRow,
    long_description: bool,
    has_badge: bool,
) -> Result<Value, AppError> {
    let name = b.display_name(i18n);
    let key = b.i18n_name();
    let description = badge_text(
        i18n,
        settings,
        base_path,
        &format!("badges.{key}.description"),
        b.description.as_ref(),
    )?;
    if urls.config.globals.cdn_url().is_some() && b.image_upload_url.is_some() {
        return Err(Unsupported("badge images behind a CDN").into());
    }
    let mut out = Map::new();
    out.insert("id".into(), json!(b.id));
    out.insert("name".into(), json!(name));
    out.insert("description".into(), json!(description));
    out.insert("grant_count".into(), json!(b.grant_count));
    out.insert("allow_title".into(), json!(b.allow_title));
    out.insert("multiple_grant".into(), json!(b.multiple_grant));
    out.insert("icon".into(), json!(b.icon));
    out.insert("image_url".into(), json!(b.image_upload_url));
    out.insert("listable".into(), json!(b.listable));
    out.insert("enabled".into(), json!(b.enabled));
    out.insert("badge_grouping_id".into(), json!(b.badge_grouping_id));
    out.insert("system".into(), json!(b.system));
    if long_description {
        let long = badge_text(
            i18n,
            settings,
            base_path,
            &format!("badges.{key}.long_description"),
            b.long_description.as_ref(),
        )?;
        out.insert("long_description".into(), json!(long));
    }
    out.insert("slug".into(), json!(slug_for(&name)));
    if has_badge {
        out.insert("has_badge".into(), json!(true));
    }
    out.insert("manually_grantable".into(), json!(!b.system));
    out.insert("show_in_post_header".into(), json!(b.show_in_post_header));
    out.insert("badge_type_id".into(), json!(b.badge_type_id));
    Ok(Value::Object(out))
}

/// BadgeTypeSerializer
pub(crate) fn badge_type_json(id: i32, name: &str) -> Value {
    json!({ "id": id, "name": name, "sort_order": 10 - id })
}

/// `Slug.for(name, "-")` for badge names: ASCII-lowercased words joined
/// by hyphens (the default `ascii` method).
pub(crate) fn slug_for(name: &str) -> String {
    let mut out = String::new();
    let mut last_dash = true;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() {
        "badge".to_string()
    } else {
        out
    }
}

/// A user_badges row as the serializers read it; `count` for the grouped
/// (`select_for_grouping`) rows.
#[derive(Debug, sqlx::FromRow)]
pub(crate) struct UserBadgeRow {
    pub id: i32,
    pub badge_id: i32,
    pub user_id: i32,
    pub granted_at: NaiveDateTime,
    pub granted_by_id: i32,

    pub post_id: Option<i32>,
    pub is_favorite: Option<bool>,
    pub count: Option<i64>,
}

pub(crate) const USER_BADGE_COLUMNS: &str = "user_badges.id, user_badges.badge_id, user_badges.user_id, \
     user_badges.granted_at, user_badges.granted_by_id, \
     user_badges.post_id, user_badges.is_favorite, NULL::bigint AS count";

/// The objects a document side-loads, deduped, in first-seen order.
#[derive(Default)]
pub(crate) struct Side {
    pub badges: Vec<Value>,
    pub badge_types: Vec<Value>,
    pub users: Vec<Value>,
    badge_ids: Vec<i32>,
    badge_type_ids: Vec<i32>,
    user_ids: Vec<i32>,
}

pub(crate) struct BadgesView<'a> {
    pub conn: &'a mut PgConnection,
    pub settings: &'a SiteSettings,
    pub i18n: &'a I18n,
    pub guardian: &'a Guardian,
    pub urls: &'a Urls<'a>,
    pub base_path: &'a str,
}

/// A grant's post as the post and topic attributes show it.
struct GrantPost {
    post_number: i32,
    topic_id: i32,
    topic_title: Option<String>,
}

impl BadgesView<'_> {
    fn list(&mut self) -> TopicListSerializer<'_> {
        TopicListSerializer {
            conn: &mut *self.conn,
            settings: self.settings,
            i18n: self.i18n,
            guardian: self.guardian,
            urls: self.urls,
            more_topics_url: None,
            category_id: None,
            group_id: None,
            prefetched: Default::default(),
        }
    }

    /// The badge (and its type) side-loaded once; its row.
    pub(crate) async fn side_badge(
        &mut self,
        side: &mut Side,
        badge_id: i32,
        long_description: bool,
    ) -> Result<BadgeRow, AppError> {
        let badge = BadgeRow::load(&mut *self.conn, badge_id).await?;
        if !side.badge_ids.contains(&badge_id) {
            side.badge_ids.push(badge_id);
            side.badges.push(badge_json(
                self.i18n,
                self.settings,
                self.urls,
                self.base_path,
                &badge,
                long_description,
                false,
            )?);
            if !side.badge_type_ids.contains(&badge.badge_type_id) {
                side.badge_type_ids.push(badge.badge_type_id);
                side.badge_types
                    .push(badge_type_json(badge.badge_type_id, &badge.badge_type_name));
            }
        }
        Ok(badge)
    }

    /// A user (UserBadgeSerializer::UserSerializer) side-loaded once.
    pub(crate) async fn side_user(&mut self, side: &mut Side, id: i32) -> Result<(), AppError> {
        if side.user_ids.contains(&id) {
            return Ok(());
        }
        let users = self.list().user_lookup_for(&[id]).await?;
        let Some(u) = users.get(&id) else {
            return Ok(());
        };
        let groups = crate::groups::load(
            &mut *self.conn,
            &[u.primary_group_id, u.flair_group_id]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>(),
        )
        .await?;
        let logo = self.list().logo_small_url().await?;
        let json = self.list().serialize_user(u, logo.as_deref(), &groups)?;
        side.user_ids.push(id);
        side.users.push(json);
        Ok(())
    }

    /// UserBadgePostAndTopicAttributesMixin: the grant's post when the
    /// badge shows posts and the viewer is an admin or the post's topic is
    /// one of `allowed_topic_ids`.
    async fn grant_post(
        &mut self,
        badge: &BadgeRow,
        post_id: Option<i32>,
        allowed_topic_ids: Option<&[i32]>,
    ) -> Result<Option<GrantPost>, AppError> {
        let Some(post_id) = post_id.filter(|_| badge.show_posts) else {
            return Ok(None);
        };
        // object.post (posts default scope: not deleted) and post.topic
        let post: Option<(i32, i32, Option<String>)> = sqlx::query_as(
            "SELECT p.post_number, p.topic_id, t.title FROM posts p \
             LEFT JOIN topics t ON t.id = p.topic_id AND t.deleted_at IS NULL \
             WHERE p.id = $1 AND p.deleted_at IS NULL",
        )
        .bind(post_id)
        .fetch_optional(&mut *self.conn)
        .await?;
        let Some((post_number, topic_id, topic_title)) = post else {
            return Ok(None);
        };
        let allowed = self.guardian.is_admin()
            || allowed_topic_ids.is_some_and(|ids| ids.contains(&topic_id));
        Ok(allowed.then_some(GrantPost {
            post_number,
            topic_id,
            topic_title,
        }))
    }

    /// DetailedUserBadgeSerializer, its badge, badge type and granter
    /// side-loaded. `badge` is the row as the request loaded it (the
    /// grant count before a grant's increment).
    pub(crate) async fn detailed_user_badge(
        &mut self,
        side: &mut Side,
        row: &UserBadgeRow,
        badge: Option<BadgeRow>,
        allowed_topic_ids: Option<&[i32]>,
    ) -> Result<Value, AppError> {
        let badge = match badge {
            Some(b) => {
                if !side.badge_ids.contains(&b.id) {
                    side.badge_ids.push(b.id);
                    side.badges.push(badge_json(
                        self.i18n,
                        self.settings,
                        self.urls,
                        self.base_path,
                        &b,
                        false,
                        false,
                    )?);
                    if !side.badge_type_ids.contains(&b.badge_type_id) {
                        side.badge_type_ids.push(b.badge_type_id);
                        side.badge_types
                            .push(badge_type_json(b.badge_type_id, &b.badge_type_name));
                    }
                }
                b
            }
            None => self.side_badge(side, row.badge_id, false).await?,
        };
        let post = self
            .grant_post(&badge, row.post_id, allowed_topic_ids)
            .await?;
        let mut ub = Map::new();
        ub.insert("id".into(), json!(row.id));
        ub.insert("granted_at".into(), json!(time_json(row.granted_at)));
        if let Some(count) = row.count {
            ub.insert("count".into(), json!(count));
        }
        ub.insert("grouping_position".into(), json!(badge.grouping_position));
        if let Some(p) = &post {
            ub.insert("post_number".into(), json!(p.post_number));
            // include_topic_attributes?: post.topic present
            if p.topic_title.is_some() {
                ub.insert("topic_id".into(), json!(p.topic_id));
                ub.insert("topic_title".into(), json!(p.topic_title));
            }
        }
        ub.insert("is_favorite".into(), json!(row.is_favorite));
        let can_favorite = self.settings.get("max_favorite_badges")?.to_i() > 0
            && self.guardian.user_id() == Some(row.user_id)
            && !(1..=4).contains(&row.badge_id);
        ub.insert("can_favorite".into(), json!(can_favorite));
        ub.insert("badge_id".into(), json!(row.badge_id));
        ub.insert("granted_by_id".into(), json!(row.granted_by_id));
        self.side_user(side, row.granted_by_id).await?;
        Ok(Value::Object(ub))
    }

    pub(crate) async fn user_badge_row(&mut self, id: i32) -> Result<UserBadgeRow, sqlx::Error> {
        sqlx::query_as(&format!(
            "SELECT {USER_BADGE_COLUMNS} FROM user_badges WHERE id = $1"
        ))
        .bind(id)
        .fetch_one(&mut *self.conn)
        .await
    }
}

/// `{badges, badge_types, granted_bies, <root>}` as AMS orders a
/// DetailedUserBadgeSerializer document.
pub(crate) fn detailed_document(side: Side, root: &str, value: Value) -> Value {
    let mut out = Map::new();
    out.insert("badges".into(), Value::Array(side.badges));
    out.insert("badge_types".into(), Value::Array(side.badge_types));
    out.insert("granted_bies".into(), Value::Array(side.users));
    out.insert(root.into(), value);
    Value::Object(out)
}
