//! `GroupMessage` (app/services/group_message.rb): a system message to a
//! group, its title and body from `system_messages.<type>`, sent at most
//! once per `limit_once_per` seconds (the redis key `grpmsg:...`, kept in
//! discourse_rs.cached_values), and the earlier ones it replaces.

use sqlx::{PgConnection, PgPool};

use crate::guardian::Guardian;
use crate::posting::Ctx;
use crate::posting::create::{self, NewPost, Outcome};
use crate::session::current::SessionUser;
use crate::url::Urls;
use crate::{AppError, Unsupported};

/// `GroupMessage::RECENT_MESSAGE_PERIOD`
const RECENT_MESSAGE_PERIOD: &str = "3 months";

pub struct GroupMessage<'a> {
    pub group_name: &'a str,
    pub message_type: &'a str,
    /// `limit_once_per:` seconds, the default a day; None is `false`.
    pub limit_once_per: Option<i64>,
    /// `message_params:`, over `base_url`.
    pub message_params: Vec<(String, String)>,
}

impl GroupMessage<'_> {
    /// `message_params`: base_url, then the given params.
    fn params(&self, ctx: &Ctx<'_>) -> Result<Vec<(String, String)>, AppError> {
        let urls = Urls {
            config: ctx.config,
            settings: ctx.settings,
        };
        let mut params = vec![("base_url".to_string(), urls.base_url()?)];
        for (key, value) in &self.message_params {
            params.retain(|(k, _)| k != key);
            params.push((key.clone(), value.clone()));
        }
        Ok(params)
    }

    /// `I18n.t("system_messages.<type>.<part>", message_params)`, by the
    /// `count` param's plural form when the template has them.
    fn template(&self, ctx: &Ctx<'_>, part: &str) -> Result<String, AppError> {
        let params = self.params(ctx)?;
        let args: Vec<(&str, &str)> = params
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let key = format!("system_messages.{}.{part}", self.message_type);
        let text = match ctx.i18n.t_with(&key, &args) {
            Some(text) => Some(text),
            None => match args.iter().find(|(k, _)| *k == "count") {
                // Ruby's pluralization: 1 (or 1.0) is one.
                Some((_, count)) => {
                    let form = if count.parse::<f64>().is_ok_and(|c| c == 1.0) {
                        "one"
                    } else {
                        "other"
                    };
                    ctx.i18n.t_with(&format!("{key}.{form}"), &args)
                }
                None => None,
            },
        };
        text.ok_or_else(|| Unsupported("group messages without a translation").into())
    }

    fn sent_recently_key(&self) -> String {
        format!("grpmsg:{}:{}:", self.group_name, self.message_type)
    }

    /// `sent_recently?`
    async fn sent_recently(&self, conn: &mut PgConnection) -> Result<bool, AppError> {
        if self.limit_once_per.is_none() {
            return Ok(false);
        }
        Ok(
            crate::owned_schema::cached_get(conn, &self.sent_recently_key())
                .await?
                .is_some(),
        )
    }

    /// `create`: nothing when sent recently; else PostCreator (with its
    /// validations) for the system user to the group, then the message
    /// remembered as sent. The post's id, None when refused.
    pub async fn create(&self, pool: &PgPool, ctx: &Ctx<'_>) -> Result<Option<i32>, AppError> {
        let mut conn = pool.acquire().await?;
        if self.sent_recently(&mut conn).await? {
            return Ok(None);
        }
        let title = self.template(ctx, "subject_template")?;
        let raw = self.template(ctx, "text_body_template")?;
        // The id bound, not inlined: a parameterless statement here met
        // "bind message supplies 0 parameters" on a reused connection.
        let system: SessionUser = SessionUser::load(&mut conn, -1)
            .await?
            .ok_or(Unsupported("group messages without the system user"))?;
        let guardian = Guardian::for_user(&mut conn, &system).await?;
        drop(conn);
        let outcome = create::create(
            pool,
            ctx,
            &guardian,
            NewPost {
                raw,
                title: Some(title),
                pm_recipients: Some(Vec::new()),
                pm_groups: vec![self.group_name.to_lowercase()],
                subtype: Some("system_message".to_string()),
                ..Default::default()
            },
        )
        .await?;
        let post_id = match outcome {
            Outcome::Created { post_id } => Some(post_id),
            Outcome::Invalid(_) => None,
            _ => return Err(Unsupported("a group message PostCreator refused").into()),
        };
        if let Some(seconds) = self.limit_once_per {
            let mut conn = pool.acquire().await?;
            crate::owned_schema::cached_setex(&mut conn, &self.sent_recently_key(), seconds, "1")
                .await?;
        }
        Ok(post_id)
    }

    /// `delete_previous!(respect_sent_recently: true, match_raw:)`: the
    /// system's recent one-post messages to the group with this title (and
    /// body) destroyed by the system user.
    pub async fn delete_previous(
        &self,
        pool: &PgPool,
        ctx: &Ctx<'_>,
        match_raw: bool,
    ) -> Result<(), AppError> {
        let mut conn = pool.acquire().await?;
        if self.sent_recently(&mut conn).await? {
            return Ok(());
        }
        let title = self.template(ctx, "subject_template")?;
        let raw = if match_raw {
            Some(
                self.template(ctx, "text_body_template")?
                    .trim_end()
                    .to_string(),
            )
        } else {
            None
        };
        // find_each: in id order.
        let previous: Vec<i32> = sqlx::query_scalar(&format!(
            "SELECT posts.id FROM posts \
             INNER JOIN topics ON topics.id = posts.topic_id \
             INNER JOIN topic_allowed_groups ON topic_allowed_groups.topic_id = topics.id \
             INNER JOIN groups ON groups.id = topic_allowed_groups.group_id \
             WHERE posts.deleted_at IS NULL AND topics.deleted_at IS NULL \
               AND topics.posts_count = 1 AND topics.user_id = -1 \
               AND topics.archetype = 'private_message' AND topics.subtype = 'system_message' \
               AND topics.title = $1 AND groups.name = $2 \
               AND posts.created_at > now() - interval '{RECENT_MESSAGE_PERIOD}' \
               AND ($3::text IS NULL OR posts.raw = $3) \
             ORDER BY posts.id"
        ))
        .bind(&title)
        .bind(self.group_name)
        .bind(&raw)
        .fetch_all(&mut *conn)
        .await?;
        drop(conn);
        let context = ctx.t("staff_action_logs.reminder_deleted");
        for post_id in previous {
            crate::post_destroyer::destroy_as_system(pool, ctx, post_id, &context).await?;
        }
        Ok(())
    }
}
