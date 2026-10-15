//! The setup wizard: Wizard with Wizard::Builder's one step (`setup`; no
//! bundled plugin adds one), WizardSerializer, Wizard::StepUpdater with
//! the step's on_update, and Wizard.user_requires_completion?.

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::site_settings::{Definitions, SiteSettings};
use crate::{AppError, Unsupported};

/// `UserHistory.actions[:wizard_step]`
const WIZARD_STEP: i32 = 40;
/// `Wizard#max_topics_to_require_completion`
const MAX_TOPICS_TO_REQUIRE_COMPLETION: i64 = 15;
/// The builder's steps with fields, in order.
const STEPS: [&str; 1] = ["setup"];

/// `completed_steps?(steps_with_fields)`: every step has a wizard_step log.
pub async fn completed(conn: &mut PgConnection) -> Result<bool, AppError> {
    let done: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT context FROM user_histories WHERE action = $1 AND context = ANY($2) \
         ORDER BY context",
    )
    .bind(WIZARD_STEP)
    .bind(&STEPS[..])
    .fetch_all(&mut *conn)
    .await?;
    let mut steps: Vec<&str> = STEPS.to_vec();
    steps.sort_unstable();
    Ok(steps == done.iter().map(String::as_str).collect::<Vec<_>>())
}

/// `Wizard#start`: the first step without a log, else the first step.
async fn start(conn: &mut PgConnection) -> Result<&'static str, AppError> {
    let done: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT context FROM user_histories WHERE action = $1 AND context = ANY($2)",
    )
    .bind(WIZARD_STEP)
    .bind(&STEPS[..])
    .fetch_all(&mut *conn)
    .await?;
    Ok(STEPS
        .iter()
        .find(|s| !done.iter().any(|d| d == *s))
        .copied()
        .unwrap_or(STEPS[0]))
}

/// `Wizard.user_requires_completion?(user)`: the wizard is on and not
/// bypassed, the site has at most 15 topics (past that it bypasses the
/// check from now on), and the user is the first admin to have logged in
/// and hasn't finished it.
pub async fn requires_completion(
    conn: &mut PgConnection,
    defs: &Definitions,
    settings: &SiteSettings,
    user_id: i32,
) -> Result<bool, AppError> {
    if !settings.get("wizard_enabled")?.truthy() || settings.get("bypass_wizard_check")?.truthy() {
        return Ok(false);
    }
    let topics: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM (SELECT 1 FROM topics WHERE deleted_at IS NULL LIMIT $1) t",
    )
    .bind(MAX_TOPICS_TO_REQUIRE_COMPLETION + 1)
    .fetch_one(&mut *conn)
    .await?;
    if topics > MAX_TOPICS_TO_REQUIRE_COMPLETION {
        // SiteSetting.bypass_wizard_check = true (hidden, not a client
        // setting: nothing published).
        let def = defs.get("bypass_wizard_check").ok_or(Unsupported(
            "bypass_wizard_check missing from site_settings.yml",
        ))?;
        crate::site_setting_update::save_override(conn, def, "t").await?;
        return Ok(false);
    }
    // User.first_login_admin_id
    let first_admin: Option<i32> = sqlx::query_scalar(
        "SELECT users.id FROM users JOIN user_auth_tokens ON user_auth_tokens.user_id = users.id \
         WHERE users.admin AND users.id > 0 ORDER BY user_auth_tokens.created_at LIMIT 1",
    )
    .fetch_optional(&mut *conn)
    .await?;
    if first_admin != Some(user_id) {
        return Ok(false);
    }
    Ok(!completed(conn).await?)
}

/// What the wizard's JSON needs beside the connection.
pub struct Cx<'a> {
    pub settings: &'a SiteSettings,
    pub defs: &'a Definitions,
    pub i18n: &'a crate::i18n::I18n,
}

impl Cx<'_> {
    /// `I18n.exists?(key) ? I18n.t(key) : nil`
    fn translate(&self, key: &str) -> Option<String> {
        self.i18n
            .t(key)
            .map(|t| t.replace("%{base_path}", ""))
            .filter(|t| !crate::ruby::is_blank(t))
    }
}

/// A field of the setup step: id, type, required, value, choices.
struct Field {
    id: &'static str,
    kind: &'static str,
    required: bool,
    value: String,
    choices: &'static [&'static str],
}

/// Wizard::Builder#append_setup_step's fields, as the settings stand.
fn setup_fields(cx: &Cx<'_>) -> Result<Vec<Field>, AppError> {
    let title = cx.settings.get("title")?.to_s().to_string();
    let default_title = cx
        .defs
        .get("title")
        .map(|d| d.default.to_s())
        .unwrap_or_default();
    let on = |name: &str| -> Result<bool, AppError> { Ok(cx.settings.get(name)?.truthy()) };
    Ok(vec![
        Field {
            id: "title",
            kind: "text",
            required: true,
            value: if title == default_title {
                String::new()
            } else {
                title
            },
            choices: &[],
        },
        Field {
            id: "default_locale",
            kind: "dropdown",
            required: false,
            value: cx.settings.get("default_locale")?.to_s().to_string(),
            choices: &[],
        },
        Field {
            id: "login_required",
            kind: "radio",
            required: false,
            value: if on("login_required")? {
                "private"
            } else {
                "public"
            }
            .into(),
            choices: &["public", "private"],
        },
        Field {
            id: "invite_only",
            kind: "radio",
            required: false,
            value: if on("invite_only")? {
                "invite_only"
            } else {
                "sign_up"
            }
            .into(),
            choices: &["sign_up", "invite_only"],
        },
        Field {
            id: "must_approve_users",
            kind: "radio",
            required: false,
            value: if on("must_approve_users")? {
                "yes"
            } else {
                "no"
            }
            .into(),
            choices: &["no", "yes"],
        },
    ])
}

/// WizardFieldSerializer
fn field_json(cx: &Cx<'_>, step: &str, f: &Field) -> Value {
    let key = format!("wizard.step.{step}.fields.{}", f.id);
    let mut out = Map::new();
    out.insert("id".into(), json!(f.id));
    out.insert("type".into(), json!(f.kind));
    out.insert("required".into(), json!(f.required));
    if !crate::ruby::is_blank(&f.value) {
        out.insert("value".into(), json!(f.value));
    }
    for sub in ["label", "placeholder", "description", "extra_description"] {
        if let Some(text) = cx.translate(&format!("{key}.{sub}")) {
            out.insert(sub.into(), json!(text));
        }
    }
    if matches!(f.kind, "dropdown" | "radio") {
        let choices: Vec<Value> = f
            .choices
            .iter()
            .map(|choice| {
                let choice_key = format!("{key}.choices.{choice}");
                let mut c = Map::new();
                c.insert("id".into(), json!(choice));
                c.insert(
                    "label".into(),
                    json!(cx.i18n.t(&format!("{choice_key}.label")).unwrap_or(choice)),
                );
                if let Some(text) = cx
                    .i18n
                    .t(&format!("{choice_key}.description"))
                    .filter(|t| !crate::ruby::is_blank(t))
                {
                    c.insert("description".into(), json!(text));
                }
                Value::Object(c)
            })
            .collect();
        out.insert("choices".into(), Value::Array(choices));
    }
    Value::Object(out)
}

/// WizardSerializer, under its root.
pub async fn wizard_json(conn: &mut PgConnection, cx: &Cx<'_>) -> Result<Value, AppError> {
    let start = start(conn).await?;
    let completed = completed(conn).await?;
    // The default theme's palette's colors, else the base colors.
    let default_theme = cx.settings.get("default_theme_id")?.to_i() as i32;
    let scheme: Option<i32> = sqlx::query_scalar(
        "SELECT cs.id FROM themes t JOIN color_schemes cs ON cs.id = t.color_scheme_id \
         AND NOT cs.remote_copy WHERE t.id = $1",
    )
    .bind(default_theme)
    .fetch_optional(&mut *conn)
    .await?;
    let colors: Vec<(String, String)> =
        match scheme {
            Some(id) => sqlx::query_as(
                "SELECT name, hex FROM color_scheme_colors WHERE color_scheme_id = $1 ORDER BY id",
            )
            .bind(id)
            .fetch_all(&mut *conn)
            .await?,
            None => crate::color_scheme::base_colors().to_vec(),
        };
    let colors: Vec<Value> = colors
        .into_iter()
        .map(|(name, hex)| json!({"name": name, "hex": hex}))
        .collect();
    let fields: Vec<Value> = setup_fields(cx)?
        .iter()
        .map(|f| field_json(cx, "setup", f))
        .collect();
    let mut step = Map::new();
    step.insert("id".into(), json!("setup"));
    if let Some(title) = cx.translate("wizard.step.setup.title") {
        step.insert("title".into(), json!(title));
    }
    step.insert("index".into(), json!(0));
    step.insert("emoji".into(), json!("rocket"));
    step.insert("fields".into(), Value::Array(fields));
    Ok(json!({
        "wizard": {
            "start": start,
            "completed": completed,
            "current_color_scheme": colors,
            "steps": [Value::Object(step)],
        }
    }))
}

/// How a step update ends.
pub enum StepOutcome {
    Done,
    /// The updater's errors, by field.
    Invalid(Vec<(String, String)>),
    /// `log_wizard_step(nil)`: InvalidParameters(:step).
    UnknownStep,
}

/// The step's settings write: `SiteSetting.set_and_log` for one changed
/// setting.
async fn set_and_log(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    cx: &Cx<'_>,
    name: &str,
    value: crate::site_settings::Value,
    actor: i32,
) -> Result<(), AppError> {
    let def = cx.defs.get(name).ok_or(Unsupported(
        "a wizard setting missing from site_settings.yml",
    ))?;
    let previous = cx.settings.get(name)?.clone();
    if previous == value {
        return Ok(());
    }
    crate::site_setting_update::set_and_log(conn, bus, def, &previous, &value, actor).await?;
    // must_approve_users turned on: the users without a pending reviewable
    // approved (the site_setting_changed handler).
    if name == "must_approve_users" && value.truthy() {
        sqlx::query(
            "UPDATE users SET approved = TRUE WHERE NOT approved AND NOT EXISTS (SELECT 1 FROM \
             reviewables r WHERE r.target_id = users.id AND r.target_type = 'User' AND r.status = 0)",
        )
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// `StepsController#update`: the step's on_update over the fields, then
/// the wizard_step log.
pub async fn update_step(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    cx: &Cx<'_>,
    actor: i32,
    step_id: &str,
    fields: &Map<String, Value>,
) -> Result<StepOutcome, AppError> {
    // step_id.dasherize: "_" as "-".
    let step = step_id.replace('_', "-");
    if !STEPS.contains(&step.as_str()) {
        return Ok(StepOutcome::UnknownStep);
    }
    let field = |id: &str| -> Option<String> { fields.get(id).and_then(crate::params::scalar) };
    let mut errors: Vec<(String, String)> = Vec::new();
    // ensure_changed(:title): the default title is no title.
    let default_title = cx
        .defs
        .get("title")
        .map(|d| d.default.to_s())
        .unwrap_or_default();
    let title = field("title");
    if title.as_deref() == Some(default_title.as_str()) {
        errors.push(("title".into(), String::new()));
    }
    if errors.is_empty() {
        // apply_settings(:title)
        let value = crate::ruby::strip(title.as_deref().unwrap_or("")).to_string();
        if *cx.settings.get("title")? != crate::site_settings::Value::Str(value.clone()) {
            return Err(Unsupported(
                "a wizard step changing the title (its change handler rewrites the welcome topic)",
            )
            .into());
        }
    }
    // apply_setting(:default_locale)
    let locale = crate::ruby::strip(field("default_locale").as_deref().unwrap_or("")).to_string();
    if *cx.settings.get("default_locale")? != crate::site_settings::Value::Str(locale) {
        return Err(Unsupported(
            "a wizard step changing the default locale (other locales and the reseed)",
        )
        .into());
    }
    set_and_log(
        conn,
        bus,
        cx,
        "login_required",
        crate::site_settings::Value::Bool(field("login_required").as_deref() == Some("private")),
        actor,
    )
    .await?;
    set_and_log(
        conn,
        bus,
        cx,
        "invite_only",
        crate::site_settings::Value::Bool(field("invite_only").as_deref() == Some("invite_only")),
        actor,
    )
    .await?;
    set_and_log(
        conn,
        bus,
        cx,
        "must_approve_users",
        crate::site_settings::Value::Bool(field("must_approve_users").as_deref() == Some("yes")),
        actor,
    )
    .await?;
    if !errors.is_empty() {
        return Ok(StepOutcome::Invalid(errors));
    }
    // StaffActionLogger#log_wizard_step
    sqlx::query(
        "INSERT INTO user_histories (action, acting_user_id, context, admin_only, created_at, \
         updated_at) VALUES ($1, $2, $3, FALSE, now(), now())",
    )
    .bind(WIZARD_STEP)
    .bind(actor)
    .bind(&step)
    .execute(&mut *conn)
    .await?;
    Ok(StepOutcome::Done)
}
