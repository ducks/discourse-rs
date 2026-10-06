//! `TopicTrackingState.report`: a member's new and unread regular topics,
//! which Rails preloads on every page as `topicTrackingStates`, and the
//! counts Ember's topic-tracking-state.js takes from it for the nav pills
//! and the sidebar (`countNew`, `countUnread`, `countNewAndUnread`).
//! Here the counts are made on the server, for the page and for its live
//! updates.

use std::collections::{HashMap, HashSet};

use chrono::NaiveDateTime;
use sqlx::PgConnection;

use crate::AppError;
use crate::guardian::Guardian;
use crate::site_settings::SiteSettings;

/// `TopicTrackingState::MAX_TOPICS`
const MAX_TOPICS: i64 = 5000;

/// `User::NewTopicDuration::ALWAYS`, `::LAST_VISIT`
const ALWAYS: i64 = -1;
const LAST_VISIT: i64 = -2;

/// A row of the report, as TopicTrackingStateItemSerializer writes it.
#[derive(Debug, Clone, PartialEq)]
pub struct State {
    pub topic_id: i32,
    pub highest_post_number: i32,
    pub last_read_post_number: Option<i32>,
    pub category_id: i32,
    /// Only reported with show_category_definitions_in_topic_lists off.
    pub is_category_topic: bool,
    pub notification_level: Option<i32>,
    pub created_in_new_period: bool,
    /// The topic's tag ids, reported with tagging on.
    pub tags: Option<Vec<i32>>,
}

impl State {
    /// `isNew`; the report has no `is_seen`, so every topic is unseen.
    fn is_new(&self) -> bool {
        self.last_read_post_number.is_none()
            && self.notification_level.is_none_or(|l| l >= TRACKING)
            && self.created_in_new_period
    }

    /// `isUnread`
    fn is_unread(&self) -> bool {
        self.last_read_post_number
            .is_some_and(|n| n < self.highest_post_number)
            && self.notification_level.is_some_and(|l| l >= TRACKING)
    }
}

/// `NotificationLevels.TRACKING`
const TRACKING: i32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    New,
    Unread,
    NewAndUnread,
}

/// A member's tracking state and what the counts read besides it.
#[derive(Debug, Clone, Default)]
pub struct Tracking {
    pub states: Vec<State>,
    /// `muted_category_ids` and `indirectly_muted_category_ids`
    pub muted_category_ids: HashSet<i32>,
    /// Each category's parent, for `getSubCategoryIds`.
    pub parents: HashMap<i32, i32>,
    /// `unified_new_enabled`
    pub unified_new: bool,
}

impl Tracking {
    /// The category and its descendants (`Category#descendants`).
    fn descendants(&self, category_id: i32) -> HashSet<i32> {
        let mut ids = HashSet::from([category_id]);
        let mut grew = true;
        while grew {
            grew = false;
            for (child, parent) in &self.parents {
                if ids.contains(parent) && ids.insert(*child) {
                    grew = true;
                }
            }
        }
        ids
    }

    /// `countCategoryByState`
    pub fn count(&self, kind: Kind, category_id: Option<i32>, tag_id: Option<i32>) -> i64 {
        let subcategories = category_id.map(|id| self.descendants(id));
        self.states
            .iter()
            .filter(|t| match kind {
                Kind::New => t.is_new(),
                Kind::Unread => t.is_unread(),
                Kind::NewAndUnread => t.is_new() || t.is_unread(),
            })
            .filter(|t| {
                subcategories
                    .as_ref()
                    .is_none_or(|ids| ids.contains(&t.category_id))
            })
            .filter(|t| category_id.is_none_or(|id| !t.is_category_topic || id == t.category_id))
            .filter(|t| {
                tag_id.is_none_or(|id| t.tags.as_ref().is_some_and(|tags| tags.contains(&id)))
            })
            .filter(|t| kind != Kind::New || !self.muted_category_ids.contains(&t.category_id))
            .count() as i64
    }

    /// `lookupCount` for the `new` and `unread` nav items, unscoped.
    pub fn lookup(&self, filter: &str) -> i64 {
        match filter {
            "new" if self.unified_new => {
                self.count(Kind::New, None, None) + self.count(Kind::Unread, None, None)
            }
            "new" => self.count(Kind::New, None, None),
            "unread" => self.count(Kind::Unread, None, None),
            _ => 0,
        }
    }
}

/// `TopicTrackingState.report(user)` with the counts' context. None for
/// an anonymous visitor.
pub async fn load(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
) -> Result<Option<Tracking>, AppError> {
    let Some(user_id) = guardian.user_id() else {
        return Ok(None);
    };
    let staff = guardian.is_staff();
    let admin = guardian.is_admin();
    let whisperer = guardian.is_whisperer(settings)?;
    let nesting3 = settings.get("max_category_nesting")?.to_i() == 3;
    let default_level = if settings.get("mute_all_categories_by_default")?.truthy() {
        0
    } else {
        1
    };
    let muted_tag_ids: Vec<i32> = sqlx::query_scalar(
        "SELECT tag_id FROM tag_users WHERE user_id = $1 AND notification_level = 0",
    )
    .bind(user_id)
    .fetch_all(&mut *conn)
    .await?;
    let first_unread_at: Option<NaiveDateTime> =
        sqlx::query_scalar("SELECT first_unread_at FROM user_stats WHERE user_id = $1")
            .bind(user_id)
            .fetch_optional(&mut *conn)
            .await?
            .flatten();
    let shape = Shape {
        staff,
        admin,
        whisperer,
        nesting3,
        muted_tag_ids: &muted_tag_ids,
        remove_muted_tags: settings
            .get("remove_muted_tags_from_latest")?
            .to_s()
            .to_string(),
        category_definitions: settings
            .get("show_category_definitions_in_topic_lists")?
            .truthy(),
    };
    let mut sql = format!(
        "{}\nUNION ALL\n\n{}",
        report_raw_sql(&shape, Half::New),
        report_raw_sql(&shape, Half::Unread)
    );
    let tagging = settings.get("tagging_enabled")?.truthy();
    if tagging {
        // tags_included_wrapped_sql
        sql = format!(
            "WITH tags_included_cte AS (\n{sql}\n)\n\
             SELECT *, ARRAY(SELECT tags.id FROM topic_tags JOIN tags ON tags.id = topic_tags.tag_id \
             WHERE topic_id = tags_included_cte.topic_id) tags FROM tags_included_cte"
        );
    } else {
        sql = format!("SELECT *, NULL::int[] tags FROM (\n{sql}\n) report");
    }
    sql.push_str("\n\n LIMIT $6");

    type Row = (
        i32,
        i32,
        Option<i32>,
        NaiveDateTime,
        i32,
        Option<i32>,
        Option<i32>,
        NaiveDateTime,
        Option<Vec<i32>>,
    );
    let min_date = chrono::DateTime::from_timestamp(settings.get("min_new_topics_time")?.to_i(), 0)
        .map(|t| t.naive_utc())
        .unwrap_or_default();
    let rows: Vec<Row> = sqlx::query_as(&sql)
        .bind(user_id)
        .bind(crate::clock::now_naive())
        .bind(
            settings
                .get("default_other_new_topic_duration_minutes")?
                .to_i(),
        )
        .bind(min_date)
        .bind(first_unread_at)
        .bind(MAX_TOPICS)
        .bind(default_level)
        .fetch_all(&mut *conn)
        .await?;
    let states = rows
        .into_iter()
        .map(
            |(
                topic_id,
                highest_post_number,
                last_read_post_number,
                created_at,
                category_id,
                category_topic_id,
                notification_level,
                treat_as_new_topic_start_date,
                tags,
            )| State {
                topic_id,
                highest_post_number,
                last_read_post_number,
                category_id,
                is_category_topic: !shape.category_definitions
                    && category_topic_id == Some(topic_id),
                notification_level,
                created_in_new_period: created_at >= treat_as_new_topic_start_date,
                tags: if tagging { tags } else { None },
            },
        )
        .collect();

    let mut muted_category_ids: HashSet<i32> = sqlx::query_scalar::<_, i32>(
        "SELECT category_id FROM category_users WHERE user_id = $1 AND notification_level = 0",
    )
    .bind(user_id)
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .collect();
    muted_category_ids.extend(
        crate::current_user::indirectly_muted_category_ids(&mut *conn, settings, user_id).await?,
    );
    let parents: HashMap<i32, i32> = sqlx::query_as::<_, (i32, i32)>(
        "SELECT id, parent_category_id FROM categories WHERE parent_category_id IS NOT NULL",
    )
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .collect();
    Ok(Some(Tracking {
        states,
        muted_category_ids,
        parents,
        unified_new: guardian
            .upcoming_change_enabled(&mut *conn, settings, "enable_unified_new")
            .await?,
    }))
}

/// What report_raw_sql's arguments are for the user.
struct Shape<'a> {
    staff: bool,
    admin: bool,
    whisperer: bool,
    nesting3: bool,
    muted_tag_ids: &'a [i32],
    remove_muted_tags: String,
    category_definitions: bool,
}

#[derive(PartialEq)]
enum Half {
    /// `skip_unread: true`
    New,
    /// `skip_new: true, filter_old_unread: true`
    Unread,
}

/// `treat_as_new_topic_clause`'s GREATEST, on the params $2 (now), $3
/// (default_other_new_topic_duration_minutes) and $4 (min_new_topics_time).
fn treat_as_new() -> String {
    format!(
        "GREATEST(CASE \
         WHEN COALESCE(uo.new_topic_duration_minutes, $3) = {ALWAYS} THEN u.created_at \
         WHEN COALESCE(uo.new_topic_duration_minutes, $3) = {LAST_VISIT} THEN COALESCE(u.previous_visit_at,u.created_at) \
         ELSE ($2::timestamp - INTERVAL '1 MINUTE' * COALESCE(uo.new_topic_duration_minutes, $3)) \
         END, u.created_at, $4)"
    )
}

/// `CategoryUser.muted_category_ids_query(user, include_direct: true)`,
/// $7 being the default notification level.
fn muted_category_ids_query(shape: &Shape) -> String {
    let mut sql = String::from(
        "SELECT categories.id FROM categories \
         LEFT JOIN categories categories2 ON categories2.id = categories.parent_category_id \
         LEFT JOIN category_users ON category_users.category_id = categories.id AND category_users.user_id = $1 \
         LEFT JOIN category_users category_users2 ON category_users2.category_id = categories2.id AND category_users2.user_id = $1 ",
    );
    let mut conditions = vec![
        "(category_users.id IS NULL AND COALESCE(category_users2.notification_level, $7) = 0)",
        "COALESCE(category_users.notification_level, $7) = 0",
    ];
    if shape.nesting3 {
        sql.push_str(
            "LEFT JOIN categories categories3 ON categories3.id = categories2.parent_category_id \
             LEFT JOIN category_users category_users3 ON category_users3.category_id = categories3.id AND category_users3.user_id = $1 ",
        );
        conditions.push(
            "(category_users.id IS NULL AND category_users2.id IS NULL AND COALESCE(category_users3.notification_level, $7) = 0)",
        );
    }
    sql.push_str(&format!("WHERE {}", conditions.join(" OR ")));
    sql
}

/// `report_raw_sql` for one half of the report; $5 is the user's
/// `first_unread_at`.
fn report_raw_sql(shape: &Shape, half: Half) -> String {
    let highest = if shape.whisperer {
        "highest_staff_post_number"
    } else {
        "highest_post_number"
    };
    let unread = match half {
        Half::New => "1=0".to_string(),
        // TopicQuery.unread_filter
        Half::Unread => format!(
            "tu.last_read_post_number < topics.{highest} AND COALESCE(tu.notification_level, 1) >= 2"
        ),
    };
    let new = match half {
        // TopicQuery.new_filter with the clause, then new_filter_sql's own.
        Half::New => format!(
            "topics.created_at >= {} AND tu.last_read_post_number IS NULL \
             AND COALESCE(tu.notification_level, 2) >= 2 \
             AND topics.created_at > $4 AND dismissed_topic_users.id IS NULL",
            treat_as_new()
        ),
        Half::Unread => "1=0".to_string(),
    };
    let filter_old_unread = match half {
        Half::Unread => " topics.updated_at >= $5 AND ",
        Half::New => "",
    };
    let category_topic_id = if shape.category_definitions {
        ""
    } else {
        "c.topic_id AS category_topic_id,"
    };
    let category_topic_null = if shape.category_definitions {
        "NULL::int AS category_topic_id,"
    } else {
        ""
    };
    let category_filter = if shape.admin {
        String::new()
    } else {
        "(NOT c.read_restricted OR u.admin OR c.id IN ( \
         SELECT c2.id FROM categories c2 \
         JOIN category_groups cg ON cg.category_id = c2.id \
         JOIN group_users gu ON gu.user_id = $1 AND cg.group_id = gu.group_id \
         WHERE c2.read_restricted )) AND"
            .to_string()
    };
    let visibility_filter = if shape.staff {
        ""
    } else {
        "(topics.visible OR u.admin OR u.moderator) AND"
    };
    let tags_filter = if !shape.muted_tag_ids.is_empty()
        && ["always", "only_muted"].contains(&shape.remove_muted_tags.as_str())
    {
        let existing =
            "(select array_agg(tag_id) from topic_tags where topic_tags.topic_id = topics.id)";
        let muted = format!(
            "ARRAY[{}]",
            shape
                .muted_tag_ids
                .iter()
                .map(i32::to_string)
                .collect::<Vec<_>>()
                .join(",")
        );
        if shape.remove_muted_tags == "always" {
            format!("NOT ( COALESCE({existing}, ARRAY[]::int[]) && {muted} ) AND")
        } else {
            format!("NOT ( COALESCE({existing}, ARRAY[-999]) <@ {muted} ) AND")
        }
    } else {
        String::new()
    };
    // The unread half already requires a read post, which the muted
    // categories' condition excludes.
    let exclude_muted_categories = match half {
        Half::Unread => "1=1".to_string(),
        Half::New => format!(
            "NOT ( tu.last_read_post_number IS NULL AND ( topics.category_id IN ({}) AND tu.notification_level <= 1 ) )",
            muted_category_ids_query(shape)
        ),
    };
    let dismissed_join = match half {
        Half::New => {
            "LEFT JOIN dismissed_topic_users ON dismissed_topic_users.topic_id = topics.id AND dismissed_topic_users.user_id = $1"
        }
        Half::Unread => "",
    };
    format!(
        "SELECT DISTINCT topics.id as topic_id, topics.{highest} AS highest_post_number, \
         last_read_post_number, topics.created_at, c.id as category_id, \
         {category_topic_id}{category_topic_null} tu.notification_level, \
         {} AS treat_as_new_topic_start_date \
         FROM topics \
         JOIN users u on u.id = $1 \
         JOIN user_options AS uo ON uo.user_id = u.id \
         JOIN categories c ON c.id = topics.category_id \
         LEFT JOIN topic_users tu ON tu.topic_id = topics.id AND tu.user_id = u.id \
         {dismissed_join} \
         WHERE u.id = $1 AND \
         {filter_old_unread} \
         topics.archetype <> 'private_message' AND \
         (({unread}) OR ({new})) AND \
         {visibility_filter} \
         {tags_filter} \
         topics.deleted_at IS NULL AND \
         {category_filter} \
         {exclude_muted_categories}",
        treat_as_new()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(topic_id: i32, category_id: i32) -> State {
        State {
            topic_id,
            highest_post_number: 3,
            last_read_post_number: None,
            category_id,
            is_category_topic: false,
            notification_level: None,
            created_in_new_period: true,
            tags: Some(vec![]),
        }
    }

    #[test]
    fn counts_follow_ember() {
        let mut unread = state(2, 10);
        unread.last_read_post_number = Some(1);
        unread.notification_level = Some(2);
        let mut read = state(3, 11);
        read.last_read_post_number = Some(3);
        read.notification_level = Some(3);
        let mut old = state(4, 11);
        old.created_in_new_period = false;
        let mut muted = state(5, 12);
        muted.tags = Some(vec![7]);
        let mut definition = state(6, 10);
        definition.is_category_topic = true;
        let tracking = Tracking {
            states: vec![state(1, 11), unread, read, old, muted, definition],
            muted_category_ids: HashSet::from([12]),
            parents: HashMap::from([(11, 10)]),
            unified_new: false,
        };
        // New: 1, 6; 5 is in a muted category, 4 is too old.
        assert_eq!(tracking.count(Kind::New, None, None), 2);
        assert_eq!(tracking.count(Kind::Unread, None, None), 1);
        // A category counts its subcategories, not their definitions.
        assert_eq!(tracking.count(Kind::New, Some(10), None), 2);
        assert_eq!(tracking.count(Kind::New, Some(11), None), 1);
        assert_eq!(tracking.count(Kind::NewAndUnread, Some(10), None), 3);
        // Muted categories are left out of the new count alone.
        assert_eq!(tracking.count(Kind::NewAndUnread, Some(12), Some(7)), 1);
        assert_eq!(tracking.count(Kind::New, None, Some(7)), 0);
        assert_eq!(tracking.lookup("new"), 2);
        let unified = Tracking {
            unified_new: true,
            ..tracking
        };
        assert_eq!(unified.lookup("new"), 3);
    }
}
