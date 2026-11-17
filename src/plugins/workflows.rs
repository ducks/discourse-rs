//! discourse-workflows' review queue filters (plugin.rb's two
//! `add_custom_reviewable_filter`s): the workflow that flagged a
//! reviewable (its score's context, `discourse_workflows:workflow:<id>`)
//! and the AI agent a workflow handed it to (discourse_workflows_reviewable_agents).
//! Their Type and Reason options are the plugin's filtered registers, there
//! while it is enabled; the filters themselves (`additional_filters`) are
//! registered whether it is or not.

use serde_json::Value;
use sqlx::PgConnection;

use crate::AppError;
use crate::site_settings::SiteSettings;

/// The Type filter's id (`type_filter: { id:, value: :all }`).
pub const TYPE_FILTER_ID: &str = "discourse_workflows:workflow";

/// The plugin's two custom filter keys, in registration order.
pub const FILTER_KEYS: [&str; 2] = ["workflow_id", "workflow_review_agent_id"];

/// A custom filter of the plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    WorkflowId,
    ReviewAgentId,
}

impl Filter {
    pub fn by_key(key: &str) -> Option<Filter> {
        match key {
            "workflow_id" => Some(Filter::WorkflowId),
            "workflow_review_agent_id" => Some(Filter::ReviewAgentId),
            _ => None,
        }
    }
}

/// What a filter adds to the list's query.
#[derive(Debug, PartialEq)]
pub enum Clause {
    /// `EXISTS (... reviewable_scores.context LIKE :context)`
    ScoreContextLike(String),
    /// `id IN (SELECT reviewable_id FROM discourse_workflows_reviewable_agents
    /// WHERE ai_agent_id = :id)`
    ReviewAgent(i64),
}

/// A filter's value: `:all` (the Type option) or what was sent.
pub enum FilterValue<'a> {
    All,
    Sent(&'a Value),
}

/// `value.to_s` for a JSON value as Rails parsed it.
fn to_s(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// The filter's proc: None when it leaves the results as they are.
pub fn clause(filter: Filter, value: FilterValue<'_>) -> Option<Clause> {
    match filter {
        Filter::WorkflowId => match value {
            FilterValue::All => Some(Clause::ScoreContextLike(
                "discourse_workflows:workflow:%".into(),
            )),
            // value.to_s[/\A[1-9]\d*\z/], then reviewable_score_context
            FilterValue::Sent(v) => {
                let s = to_s(v);
                let valid = s.starts_with(|c: char| ('1'..='9').contains(&c))
                    && s.bytes().all(|b| b.is_ascii_digit());
                valid.then(|| Clause::ScoreContextLike(format!("discourse_workflows:workflow:{s}")))
            }
        },
        Filter::ReviewAgentId => {
            let s = match value {
                FilterValue::All => "all".to_string(),
                FilterValue::Sent(v) => to_s(v),
            };
            // agent_id.match?(/\A-?[1-9][0-9]*\z/), then to_i
            let digits = s.strip_prefix('-').unwrap_or(&s);
            let valid = digits.starts_with(|c: char| ('1'..='9').contains(&c))
                && digits.bytes().all(|b| b.is_ascii_digit());
            if !valid {
                return None;
            }
            s.parse().ok().map(Clause::ReviewAgent)
        }
    }
}

/// A Reason option (`reason_filters`): its id and name, and the filter
/// and value it applies.
pub struct ReasonOption {
    pub id: String,
    pub name: String,
    pub filter: Filter,
    pub value: i64,
}

/// Whether the plugin's filtered registers count (`plugin.enabled?`).
pub fn enabled(s: &SiteSettings) -> Result<bool, AppError> {
    Ok(s.get("enable_discourse_workflows")?.truthy())
}

/// `Reviewable.custom_reason_filter_options` of the plugin: the workflows
/// by name, then the AI agents a workflow handed a reviewable to, by name.
pub async fn reason_filter_options(
    conn: &mut PgConnection,
) -> Result<Vec<ReasonOption>, sqlx::Error> {
    let workflows: Vec<(i64, String)> =
        sqlx::query_as("SELECT id::bigint, name FROM discourse_workflows_workflows ORDER BY name")
            .fetch_all(&mut *conn)
            .await?;
    let agents: Vec<(i64, String)> = sqlx::query_as(
        "SELECT id::bigint, name FROM ai_agents WHERE EXISTS ( \
           SELECT 1 FROM discourse_workflows_reviewable_agents \
           WHERE discourse_workflows_reviewable_agents.ai_agent_id = ai_agents.id) \
         ORDER BY name",
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(workflows
        .into_iter()
        .map(|(id, name)| ReasonOption {
            id: format!("discourse_workflows:workflow:{id}"),
            name,
            filter: Filter::WorkflowId,
            value: id,
        })
        .chain(agents.into_iter().map(|(id, name)| ReasonOption {
            id: format!("discourse_workflows:agent:{id}"),
            name,
            filter: Filter::ReviewAgentId,
            value: id,
        }))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn workflow_id_takes_positive_integers() {
        assert_eq!(
            clause(Filter::WorkflowId, FilterValue::Sent(&json!("12"))),
            Some(Clause::ScoreContextLike(
                "discourse_workflows:workflow:12".into()
            ))
        );
        assert_eq!(
            clause(Filter::WorkflowId, FilterValue::Sent(&json!(7))),
            Some(Clause::ScoreContextLike(
                "discourse_workflows:workflow:7".into()
            ))
        );
        assert_eq!(
            clause(Filter::WorkflowId, FilterValue::Sent(&json!("07"))),
            None
        );
        assert_eq!(
            clause(Filter::WorkflowId, FilterValue::Sent(&json!("1a"))),
            None
        );
        assert_eq!(
            clause(Filter::WorkflowId, FilterValue::All),
            Some(Clause::ScoreContextLike(
                "discourse_workflows:workflow:%".into()
            ))
        );
    }

    #[test]
    fn review_agent_id_takes_signed_integers() {
        assert_eq!(
            clause(Filter::ReviewAgentId, FilterValue::Sent(&json!("-3"))),
            Some(Clause::ReviewAgent(-3))
        );
        assert_eq!(
            clause(Filter::ReviewAgentId, FilterValue::Sent(&json!(0))),
            None
        );
    }
}
