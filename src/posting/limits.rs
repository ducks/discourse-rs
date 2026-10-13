//! The limits on creating posts and topics (RateLimiter::OnCreateRecord on
//! Post and Topic): performed where Rails' after_create fires, inside the
//! create's transaction but written outside it, as Redis is; when the
//! create then fails, the ones performed are rolled back (after_rollback).

use chrono::NaiveDateTime;
use sqlx::{PgConnection, PgPool};

use crate::AppError;
use crate::rate_limiter::{LimitExceeded, LimitUser, RateLimiter};
use crate::session::current::SessionUser;
use crate::site_settings::SiteSettings;

const DAY: i64 = 86_400;

/// What the limits read of the poster.
#[derive(Debug, Clone)]
pub struct Poster {
    pub id: i32,
    pub trust_level: i32,
    pub admin: bool,
    pub staff: bool,
    pub created_at: NaiveDateTime,
}

impl Poster {
    pub async fn load(conn: &mut PgConnection, user: &SessionUser) -> Result<Poster, AppError> {
        let created_at: NaiveDateTime =
            sqlx::query_scalar("SELECT created_at FROM users WHERE id = $1")
                .bind(user.id)
                .fetch_one(conn)
                .await?;
        Ok(Poster {
            id: user.id,
            trust_level: user.trust_level,
            admin: user.admin,
            staff: user.admin || user.moderator,
            created_at,
        })
    }

    fn limit_user(&self) -> Option<LimitUser> {
        Some(LimitUser {
            id: self.id,
            staff: self.staff,
        })
    }

    fn day_ago() -> NaiveDateTime {
        crate::clock::now_naive() - chrono::Duration::hours(24)
    }

    /// `User#new_user?`
    fn new_user(&self) -> bool {
        (self.created_at >= Self::day_ago() || self.trust_level == 0)
            && self.trust_level < 2
            && !self.staff
    }

    /// `User#new_user_posting_on_first_day?`
    fn posting_on_first_day(&self) -> bool {
        !self.staff && self.trust_level < 2 && self.created_at > Self::day_ago()
    }
}

/// `Topic#default_rate_limiter`, `limit_topics_per_day` and
/// `limit_private_messages_per_day`, in their after_create order.
pub fn topic_limiters(
    s: &SiteSettings,
    poster: &Poster,
    private_message: bool,
    subtype: Option<&str>,
) -> Result<Vec<RateLimiter>, AppError> {
    let user = poster.limit_user();
    let mut limiters = vec![RateLimiter::new(
        user,
        "create_topic",
        1,
        s.get("rate_limit_create_topic")?.to_i(),
    )];
    if !private_message {
        limiters.push(if poster.posting_on_first_day() {
            let max = if poster.trust_level == 1 {
                "tl1_max_topics_in_first_day"
            } else {
                "max_topics_in_first_day"
            };
            RateLimiter::new(user, "first-day-topics-per-day", s.get(max)?.to_i(), DAY)
        } else {
            RateLimiter::new(
                user,
                "topics-per-day",
                s.get("max_topics_per_day")?.to_i(),
                DAY,
            )
        });
    } else if subtype != Some("notify_moderators") {
        let admin_limit = s.get("limit_admin_personal_messages_per_day")?.to_i();
        limiters.push(if poster.admin && admin_limit > 0 {
            RateLimiter::new(user, "pms-per-day", admin_limit, DAY).apply_limit_to_staff()
        } else {
            RateLimiter::new(
                user,
                "pms-per-day",
                s.get("max_personal_messages_per_day")?.to_i(),
                DAY,
            )
        });
    }
    Ok(limiters)
}

/// `Post#default_rate_limiter` and `limit_posts_per_day`.
pub fn post_limiters(
    s: &SiteSettings,
    poster: &Poster,
    post_number: i32,
) -> Result<Vec<RateLimiter>, AppError> {
    let user = poster.limit_user();
    let secs = if poster.new_user() {
        s.get("rate_limit_new_user_create_post")?.to_i()
    } else {
        s.get("rate_limit_create_post")?.to_i()
    };
    let mut limiters = vec![RateLimiter::new(user, "create_post", 1, secs)];
    if poster.posting_on_first_day() && post_number > 1 {
        let max = if poster.trust_level == 1 {
            "tl1_max_replies_in_first_day"
        } else {
            "max_replies_in_first_day"
        };
        limiters.push(RateLimiter::new(
            user,
            "first-day-replies-per-day",
            s.get(max)?.to_i(),
            DAY,
        ));
    }
    Ok(limiters)
}

/// `EditRateLimiter.new(editor)`: max_edits_per_day, times the trust
/// level's multiplier (floored at 1) from TL2, per day and per date
/// (`Date.today`, the server's UTC date, in the key).
pub fn edit_limiter(
    s: &SiteSettings,
    editor: LimitUser,
    trust_level: i32,
) -> Result<RateLimiter, AppError> {
    let mut limit = s.get("max_edits_per_day")?.to_i();
    if trust_level >= 2 {
        let multiplier = crate::ruby::to_f(
            &s.get(&format!(
                "tl{trust_level}_additional_edits_per_day_multiplier"
            ))?
            .to_s(),
        )
        .max(1.0);
        limit = (limit as f64 * multiplier) as i64;
    }
    let today = crate::clock::now().date_naive().format("%Y-%m-%d");
    Ok(RateLimiter::new(Some(editor), "edit-post", limit, DAY).key_suffix(&format!(":{today}")))
}

/// PostsController#destroy's and #recover's limits for those who cannot
/// moderate the topic: per minute, then per day.
pub fn delete_limiters(s: &SiteSettings, user: LimitUser) -> Result<Vec<RateLimiter>, AppError> {
    Ok(vec![
        RateLimiter::new(
            Some(user),
            "delete_post_per_min",
            s.get("max_post_deletions_per_minute")?.to_i(),
            60,
        ),
        RateLimiter::new(
            Some(user),
            "delete_post_per_day",
            s.get("max_post_deletions_per_day")?.to_i(),
            DAY,
        ),
    ])
}

/// The limiters a create performed: rolled back (`rollback!`, newest
/// stamp popped) unless the create commits. A refusal undoes them
/// explicitly; an error path's drop undoes them on a spawned task.
pub struct Performed {
    pool: PgPool,
    enabled: bool,
    done: Vec<RateLimiter>,
}

impl Performed {
    /// `enabled` false: `disable_rate_limits!` (skip_validations) or the
    /// limiter turned off.
    pub fn new(pool: &PgPool, enabled: bool) -> Performed {
        Performed {
            pool: pool.clone(),
            enabled,
            done: Vec::new(),
        }
    }

    /// Each limiter performed in order; the first refusal stops the rest.
    pub async fn perform(
        &mut self,
        limiters: Vec<RateLimiter>,
    ) -> Result<Result<(), LimitExceeded>, AppError> {
        if !self.enabled {
            return Ok(Ok(()));
        }
        for limiter in limiters {
            if let Err(refused) = limiter.performed(&self.pool, true).await? {
                return Ok(Err(refused));
            }
            self.done.push(limiter);
        }
        Ok(Ok(()))
    }

    /// after_rollback: each performed limiter rolled back.
    pub async fn undo(mut self) -> Result<(), AppError> {
        for limiter in std::mem::take(&mut self.done) {
            limiter.rollback(&self.pool, true).await?;
        }
        Ok(())
    }

    /// The create committed: nothing to undo.
    pub fn commit(mut self) {
        self.done.clear();
    }
}

impl Drop for Performed {
    fn drop(&mut self) {
        if self.done.is_empty() {
            return;
        }
        let done = std::mem::take(&mut self.done);
        let pool = self.pool.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                for limiter in done {
                    if let Err(e) = limiter.rollback(&pool, true).await {
                        tracing::error!("rate limiter rollback: {e}");
                    }
                }
            });
        }
    }
}
