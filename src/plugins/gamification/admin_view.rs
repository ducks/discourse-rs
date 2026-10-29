//! The plugin's admin pages (admin-plugins/show/discourse-gamification-
//! leaderboards): its configuration nav (Settings, Leaderboards), the
//! leaderboards list with the create form, the delete confirmation and
//! the recalculate scores modal, and a leaderboard's edit form
//! (AdminEditLeaderboard). static/js/gamification.js sends what they ask
//! to the admin endpoints.
//!
//! The group and category choosers are native multiple selects, not
//! select-kit's GroupChooser and CategorySelector.

use chrono::{Datelike, Duration, NaiveDate, TimeZone, Utc};

use super::{Leaderboard, PERIODS};
use crate::admin_form_kit as fk;
use crate::admin_ui::{Action, Button};
use crate::admin_view::{AdminCx, t, t_count};
use crate::topic_list_view::escape;

/// The plugin's admin show route.
pub const SHOW_PATH: &str = "/admin/plugins/discourse-gamification";
/// Its leaderboards page.
pub const LEADERBOARDS_PATH: &str = "/admin/plugins/discourse-gamification/leaderboards";

/// The scorable keys in the order the edit form lists their weights.
const FORM_KEYS: [&str; 15] = [
    "like_given",
    "like_received",
    "post_created",
    "topic_created",
    "solution",
    "day_visited",
    "flag_created",
    "post_read",
    "time_read",
    "user_invited",
    "reaction_given",
    "reaction_received",
    "chat_message_created",
    "chat_reaction_given",
    "chat_reaction_received",
];

/// The configuration nav as the plugin page's tabs: the settings, then
/// the leaderboards (`addAdminPluginConfigurationNav`).
pub fn config_nav(cx: &AdminCx, leaderboards: bool) -> String {
    crate::admin_ui::tabs(
        cx,
        &[
            (
                "",
                &format!("{SHOW_PATH}/settings"),
                "admin.plugins.change_settings_short",
                !leaderboards,
            ),
            (
                "",
                LEADERBOARDS_PATH,
                "gamification.leaderboard.title",
                leaderboards,
            ),
        ],
    )
}

/// The strings gamification.js shows on the admin pages.
pub fn strings(cx: &AdminCx) -> String {
    crate::admin_ui::strings(
        cx,
        "gamification-admin-strings",
        &[
            "gamification.leaderboard.confirm_destroy",
            "gamification.leaderboard.create_success",
            "gamification.leaderboard.delete_success",
            "gamification.leaderboard.save_success",
            "gamification.recalculate",
            "gamification.recalculating",
            "gamification.completed",
            "gamification.close",
            "gamification.cancel",
            "gamification.apply",
            "gamification.daily_update_scores_availability.one",
            "gamification.daily_update_scores_availability.other",
            "generic_error",
        ],
    )
}

/// A date the way `pattern` (a moment format) writes it.
fn format_day(d: NaiveDate, pattern: &str) -> String {
    let at = Utc.from_utc_datetime(&d.and_hms_opt(0, 0, 0).unwrap_or_default());
    crate::pretty_text::render::local_dates::format_utc(at, pattern).unwrap_or_default()
}

/// The period column: the dates when it has a start, else its default
/// period's name.
fn period_cell(cx: &AdminCx, lb: &Leaderboard) -> String {
    let date = |d: Option<NaiveDate>| {
        d.map(|d| {
            crate::admin_ui::format_date(
                cx,
                Utc.from_utc_datetime(&d.and_hms_opt(0, 0, 0).unwrap_or_default()),
                false,
            )
        })
        .unwrap_or_default()
    };
    if lb.from_date.is_some() {
        return format!("{} - {}", date(lb.from_date), date(lb.to_date));
    }
    lb.default_period
        .and_then(|p| usize::try_from(p).ok())
        .map(|p| {
            // LEADERBOARD_PERIODS[n], undefined past the end.
            let key = PERIODS.get(p).copied().unwrap_or("undefined");
            escape(&t(
                cx.i18n,
                &format!("gamification.leaderboard.period.{key}"),
            ))
        })
        .unwrap_or_default()
}

/// The create form (AdminCreateLeaderboard), shown by New.
fn create_form(cx: &AdminCx) -> String {
    let name = fk::Field {
        name: "name",
        control: "input",
        title: None,
        inner: fk::input(
            "name",
            "",
            "text",
            Some(&t(cx.i18n, "gamification.leaderboard.name_placeholder")),
            false,
        ),
        ..Default::default()
    }
    .render();
    let actions = fk::actions(&format!(
        "{}{}",
        fk::submit("", &t(cx.i18n, "submit")),
        fk::button(
            "new-leaderboard__cancel form-kit__button btn-default",
            &t(cx.i18n, "gamification.cancel"),
            None
        )
    ));
    format!(
        "<div class=\"new-leaderboard-container\" data-created-by-id=\"{}\" hidden>{}</div>",
        cx.viewer.id,
        fk::form(
            "new-leaderboard-form",
            &format!(
                "<div class=\"form-kit__row\"><div class=\"new-leaderboard__name\">{name}</div></div>{actions}"
            )
        )
    )
}

/// The recalculate scores modal's body and footer (RecalculateScoresForm),
/// for gamification.js: the ranges with the date each starts from, the
/// custom date, and the recalculations left today.
fn recalculate_modal(cx: &AdminCx, remaining: i64, today: NaiveDate) -> String {
    let pattern = t(cx.i18n, "dates.long_with_year_no_time");
    let ranges: [(&str, Option<NaiveDate>); 6] = [
        ("last_10_days", Some(today - Duration::days(10))),
        ("last_30_days", Some(today - Duration::days(30))),
        ("last_90_days", Some(today - Duration::days(90))),
        (
            "last_year",
            today
                .with_year(today.year() - 1)
                .or(Some(today - Duration::days(365))),
        ),
        ("all_time", None),
        ("custom_date_range", None),
    ];
    let mut options = String::new();
    for (i, (key, from)) in ranges.iter().enumerate() {
        let (from_value, range) = match (key, from) {
            (_, Some(d)) => (
                d.format("%Y-%m-%d").to_string(),
                format!(
                    "{} - {}",
                    format_day(*d, &pattern),
                    format_day(today, &pattern)
                ),
            ),
            (&"all_time", None) => ("2014-8-26".to_string(), String::new()),
            _ => (String::new(), String::new()),
        };
        options.push_str(&format!(
            "<option value=\"{i}\" data-from=\"{}\" data-range=\"{}\"{}>{}</option>",
            escape(&from_value),
            escape(&range),
            if i == 0 { " selected" } else { "" },
            escape(&t(cx.i18n, &format!("gamification.update_range.{key}")))
        ));
    }
    let first_range = format!(
        "{} - {}",
        format_day(today - Duration::days(10), &pattern),
        format_day(today, &pattern)
    );
    let apply = Button {
        class: "btn-primary",
        label: Some(t(cx.i18n, "gamification.apply")),
        disabled: remaining <= 0,
        attrs: " id=\"apply-section\"".into(),
        ..Default::default()
    };
    let cancel = Button {
        class: "btn-default",
        label: Some(t(cx.i18n, "gamification.cancel")),
        attrs: " id=\"cancel-section\"".into(),
        ..Default::default()
    };
    format!(
        "<template class=\"recalculate-scores-template\" data-remaining=\"{remaining}\">\
         <div class=\"recalculate-modal__body\"><form class=\"form-horizontal\"><div class=\"input-group\">\
         <label>{}</label><select id=\"update-range\" class=\"recalculate-modal__range\">{options}</select>\
         <div class=\"input-group -custom-range\" hidden><label>{}</label>\
         <input class=\"date-input\" id=\"custom-from-date\" type=\"date\" placeholder=\"yyyy-mm-dd\"></div>\
         <div class=\"recalculate-modal__date-range\">{}</div></div></form></div>\
         <div class=\"recalculate-modal__footer\">{}{}<div class=\"recalculate-modal__footer-text\">{}</div></div>\
         </template>",
        escape(&t(cx.i18n, "gamification.update_scores_help")),
        escape(&t(cx.i18n, "gamification.custom_range_from")),
        escape(&first_range),
        apply.render(),
        cancel.render(),
        escape(&t_count(
            cx.i18n,
            "gamification.daily_update_scores_availability",
            remaining,
            &[]
        )),
    )
}

/// The leaderboards page: the subheader with New and Recalculate (once
/// there are leaderboards), the create form, the table or the empty
/// state, and the modal's template.
pub fn leaderboards(
    cx: &AdminCx,
    leaderboards: &[Leaderboard],
    remaining: i64,
    today: NaiveDate,
) -> String {
    let actions = if leaderboards.is_empty() {
        String::new()
    } else {
        crate::admin_ui::actions(
            cx,
            "d-page-subheader",
            &[
                Action::Primary(Button {
                    class: "leaderboard-admin__btn-new",
                    label: Some(t(cx.i18n, "gamification.leaderboard.new")),
                    title: Some(t(cx.i18n, "gamification.leaderboard.new")),
                    ..Default::default()
                }),
                Action::Default(Button {
                    class: "leaderboard-admin__btn-recalculate",
                    label: Some(t(cx.i18n, "gamification.recalculate")),
                    title: Some(t(cx.i18n, "gamification.recalculate")),
                    ..Default::default()
                }),
            ],
        )
    };
    let mut out = String::from("<div class=\"discourse-gamification__leaderboards admin-detail\">");
    out.push_str(&crate::admin_ui::subheader(
        &t(cx.i18n, "gamification.leaderboard.title"),
        &actions,
        None,
    ));
    out.push_str(&create_form(cx));
    out.push_str("<div class=\"leaderboards\">");
    if leaderboards.is_empty() {
        let cta = Button {
            class: "btn-default btn-small leaderboard-admin__cta-new",
            label: Some(t(cx.i18n, "gamification.leaderboard.cta")),
            ..Default::default()
        };
        out.push_str(&format!(
            "<div class=\"admin-plugin-config-area__empty-list\">{}{}</div>",
            escape(&t(cx.i18n, "gamification.leaderboard.none")),
            cta.render()
        ));
    } else {
        out.push_str(&format!(
            "<table><thead><th>{}</th><th>{}</th><th></th></thead><tbody>",
            escape(&t(cx.i18n, "gamification.admin.name")),
            escape(&t(cx.i18n, "gamification.admin.period"))
        ));
        for lb in leaderboards {
            let delete = Button {
                class: "btn-small leaderboard-admin__delete btn-danger",
                icon: Some("trash-can"),
                title: Some(t(cx.i18n, "gamification.delete")),
                attrs: format!(" data-leaderboard-id=\"{}\"", lb.id),
                ..Default::default()
            };
            out.push_str(&format!(
                "<tr id=\"leaderboard-admin__row-{id}\"><td><a href=\"{base}/leaderboard/{id}\">{}</a></td><td>{}</td>\
                 <td style=\"width: 120px\"><div class=\"leaderboard-admin__listitem-action\">\
                 <a class=\"btn btn-default leaderboard-admin__edit btn-text btn-small\" href=\"{base}{LEADERBOARDS_PATH}/{id}\">{} </a>{}</div></td></tr>",
                escape(&lb.name),
                period_cell(cx, lb),
                escape(&t(cx.i18n, "gamification.edit")),
                delete.render(),
                id = lb.id,
                base = cx.base_path,
            ));
        }
        out.push_str("</tbody></table>");
    }
    out.push_str("</div>");
    out.push_str(&recalculate_modal(cx, remaining, today));
    out.push_str("</div>");
    out
}

/// FormKit's section: title, subtitle, fields.
fn section(title: &str, subtitle: Option<&str>, inner: &str) -> String {
    format!(
        "<div class=\"form-kit__section\"><h2 class=\"form-kit__section-title\">{}</h2>{}{inner}</div>",
        escape(title),
        subtitle
            .map(|s| format!(
                "<span class=\"form-kit__section-subtitle\">{}</span>",
                escape(s)
            ))
            .unwrap_or_default()
    )
}

/// A row of 6-wide columns.
fn row(cols: &[String]) -> String {
    let cols: String = cols
        .iter()
        .map(|c| format!("<div class=\"form-kit__col --col-6\">{c}</div>"))
        .collect();
    format!("<div class=\"form-kit__row\">{cols}</div>")
}

/// A multiple select of (id, name) with the chosen ids selected.
fn multi_select(name: &str, options: &[(i64, String)], chosen: &[i32]) -> String {
    let opts: String = options
        .iter()
        .map(|(id, label)| {
            format!(
                "<option value=\"{id}\"{}>{}</option>",
                if chosen.iter().any(|c| i64::from(*c) == *id) {
                    " selected"
                } else {
                    ""
                },
                escape(label)
            )
        })
        .collect();
    format!(
        "<div class=\"form-kit__control-custom\"><select multiple class=\"leaderboard-edit__multi-select\" id=\"control-{n}-input\" name=\"{n}\">{opts}</select></div>",
        n = escape(name)
    )
}

/// The edit page (AdminEditLeaderboard): back, then the form.
pub fn edit(
    cx: &AdminCx,
    lb: &Leaderboard,
    groups: &[(i64, String)],
    categories: &[(i64, String)],
) -> String {
    let tr = |key: &str| t(cx.i18n, key);
    let date = |d: Option<NaiveDate>| {
        d.map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_default()
    };
    let mut f = fk::Field {
        name: "name",
        control: "input",
        title: Some(tr("gamification.leaderboard.name")),
        inner: fk::input("name", &lb.name, "text", None, false),
        ..Default::default()
    }
    .render();
    let from = fk::Field {
        name: "from_date",
        control: "input-date",
        title: Some(tr("gamification.leaderboard.date.from")),
        inner: fk::input("from_date", &date(lb.from_date), "date", None, false),
        ..Default::default()
    }
    .render();
    let to = fk::Field {
        name: "to_date",
        control: "input-date",
        title: Some(tr("gamification.leaderboard.date.to")),
        inner: fk::input("to_date", &date(lb.to_date), "date", None, false),
        ..Default::default()
    }
    .render();
    let period_options: Vec<(String, String)> = PERIODS
        .iter()
        .enumerate()
        .map(|(i, p)| {
            (
                i.to_string(),
                tr(&format!("gamification.leaderboard.period.{p}")),
            )
        })
        .collect();
    let time = format!(
        "{}{}{}",
        row(&[from, to]),
        fk::Field {
            name: "period_filter_disabled",
            control: "toggle",
            title: Some(tr("gamification.leaderboard.period_filter_disabled")),
            inner: fk::toggle("period_filter_disabled", lb.period_filter_disabled, false),
            ..Default::default()
        }
        .render(),
        fk::Field {
            name: "default_period",
            control: "custom",
            title: Some(tr("gamification.leaderboard.default_period")),
            inner: fk::select(
                "default_period",
                &period_options,
                &lb.default_period.map(|p| p.to_string()).unwrap_or_default(),
                None,
                false,
            ),
            ..Default::default()
        }
        .render()
    );
    f.push_str(&section(
        &tr("gamification.leaderboard.section.time_period"),
        Some(&tr(
            "gamification.leaderboard.section.time_period_description",
        )),
        &time,
    ));
    let mut group_fields = String::new();
    for (name, key, chosen) in [
        (
            "included_groups_ids",
            "gamification.leaderboard.included_groups",
            &lb.included_groups_ids,
        ),
        (
            "excluded_groups_ids",
            "gamification.leaderboard.excluded_groups",
            &lb.excluded_groups_ids,
        ),
        (
            "visible_to_groups_ids",
            "gamification.leaderboard.visible_to_groups",
            &lb.visible_to_groups_ids,
        ),
    ] {
        group_fields.push_str(
            &fk::Field {
                name,
                control: "custom",
                title: Some(tr(key)),
                inner: multi_select(name, groups, chosen),
                ..Default::default()
            }
            .render(),
        );
    }
    f.push_str(&section(
        &tr("gamification.leaderboard.section.groups"),
        Some(&tr("gamification.leaderboard.section.groups_description")),
        &group_fields,
    ));
    let categories_field = fk::Field {
        name: "scorable_category_ids",
        control: "custom",
        title: Some(tr("gamification.leaderboard.scorable_categories")),
        description: Some(tr("gamification.leaderboard.scorable_categories_help")),
        inner: multi_select(
            "scorable_category_ids",
            categories,
            lb.scorable_category_ids.as_deref().unwrap_or(&[]),
        ),
        ..Default::default()
    }
    .render();
    let weight = |key: &str| {
        let name = format!("score_override_{key}");
        let value = lb
            .score_overrides
            .as_ref()
            .and_then(|o| o.get(key))
            .map(|v| match v {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .unwrap_or_default();
        let input = fk::input(&name, &value, "number", None, false).replacen(
            " type=\"number\"",
            " min=\"0\" step=\"1\" type=\"number\"",
            1,
        );
        fk::Field {
            name: &name,
            control: "input-number",
            title: Some(tr(&format!("gamification.leaderboard.weight.{key}"))),
            inner: input,
            ..Default::default()
        }
        .render()
    };
    let mut weights = String::new();
    for pair in FORM_KEYS.chunks(2) {
        if pair.len() == 2 {
            weights.push_str(&row(&[weight(pair[0]), weight(pair[1])]));
        } else {
            weights.push_str(&weight(pair[0]));
        }
    }
    f.push_str(&format!(
        "<details class=\"leaderboard-scoring-details\"><summary>{}</summary><p class=\"leaderboard-scoring-details__description\">{}</p>{}{}</details>",
        escape(&tr("gamification.leaderboard.scoring_configuration")),
        escape(&tr("gamification.leaderboard.scoring_configuration_description")),
        section(&tr("gamification.leaderboard.scorable_categories"), None, &categories_field),
        section(
            &tr("gamification.leaderboard.score_overrides"),
            Some(&tr("gamification.leaderboard.score_overrides_help")),
            &weights
        ),
    ));
    f.push_str(&fk::submit("", &tr("submit")));
    format!(
        "<div class=\"admin-detail\">{}{}</div>",
        crate::admin_ui::back_button(cx, LEADERBOARDS_PATH, "gamification.back"),
        fk::form("edit-create-leaderboard-form", &f).replacen(
            "<form ",
            &format!("<form data-leaderboard-id=\"{}\" ", lb.id),
            1
        )
    )
}

/// The leaderboards page's crumb (DBreadcrumbsItem).
pub fn crumb(cx: &AdminCx) -> String {
    t(cx.i18n, "gamification.leaderboard.title")
}
