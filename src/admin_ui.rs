//! The ui-kit pieces the advanced and customize admin pages draw with:
//! DButton, DPageHeader's and DPageSubheader's actions with their icons
//! and disabled states (the Primary, Default, Danger and Wrapped buttons,
//! or in the mobile view the DMenu of their list items), DMenu's trigger
//! with its content kept in a template for the page's script, a tab
//! strip of DNavItems, and the strings a page's script reads.

use serde_json::Value;

use crate::admin_view::{AdminCx, t, t_opt};
use crate::topic_list_view::{escape, icon};

/// DButton's arguments, the label and title already translated.
#[derive(Default, Clone)]
pub struct Button<'a> {
    /// The classes given (`class=` and `@class`).
    pub class: &'a str,
    pub icon: Option<&'a str>,
    pub label: Option<String>,
    pub title: Option<String>,
    pub disabled: bool,
    /// Extra attributes, already escaped (` data-x="y"`).
    pub attrs: String,
    /// `@route`'s path (under the base path), which admin-ui.js follows.
    pub href: Option<&'a str>,
}

impl Button<'_> {
    pub fn render(&self) -> String {
        let content = match (self.icon, &self.label) {
            (Some(_), Some(_)) => " btn-icon-text",
            (Some(_), None) => " btn-icon",
            _ => "",
        };
        let no_text = if self.label.is_none() { " no-text" } else { "" };
        // dConcatClass's own classes, then the invocation's.
        let class = if self.class.is_empty() {
            String::new()
        } else {
            format!(" {}", escape(self.class))
        };
        let mut out = format!("<button class=\"btn{no_text}{content}{class}\"");
        if self.disabled {
            out.push_str(" disabled=\"\"");
        }
        if let Some(title) = &self.title {
            out.push_str(&format!(" title=\"{}\"", escape(title)));
        }
        if let Some(href) = self.href {
            out.push_str(&format!(" data-href=\"{}\"", escape(href)));
        }
        out.push_str(&format!(" type=\"button\"{}>", self.attrs));
        if let Some(name) = self.icon {
            out.push_str(&icon(name, None));
        }
        match &self.label {
            Some(label) => out.push_str(&format!(
                "<span class=\"d-button-label\">{}</span>",
                escape(label)
            )),
            None if self.icon.is_some() => {
                out.push_str("<span aria-hidden=\"true\">\u{200b}</span>")
            }
            None => {}
        }
        out.push_str("</button>");
        out
    }
}

/// `i18n(key)` for a DButton's `@label` or `@title`.
pub fn tr(cx: &AdminCx, key: &str) -> Option<String> {
    Some(t(cx.i18n, key))
}

/// A page action: PrimaryButton, DefaultButton, DangerButton, or a
/// WrappedButton's markup (UppyBackupUploader's label) as is.
pub enum Action<'a> {
    Primary(Button<'a>),
    Default(Button<'a>),
    Danger(Button<'a>),
    /// The wrapped markup for the desktop view and for the list item,
    /// each given the class its wrapper yields.
    Wrapped(Box<dyn Fn(&str) -> String + 'a>),
}

/// The actions of DPageHeader (`identifier` d-page-header) or
/// DPageSubheader (d-page-subheader). In the mobile view the More options
/// DMenu with the list items in its template, which admin.js opens; an
/// item clicks the button it stands for, kept hidden beside (the wrapped
/// ones are their own list items).
pub fn actions(cx: &AdminCx, identifier: &str, actions: &[Action]) -> String {
    let button = |b: &Button, kind: &str| {
        let class = format!("d-page-action-button btn-small btn-{kind} {}", b.class);
        Button {
            class: class.trim_end(),
            ..b.clone()
        }
        .render()
    };
    let buttons: String = actions
        .iter()
        .map(|a| match a {
            Action::Primary(b) => button(b, "primary"),
            Action::Default(b) => button(b, "default"),
            Action::Danger(b) => button(b, "danger"),
            Action::Wrapped(f) => format!(
                "<span class=\"d-page-action-wrapped-button\">{}</span>",
                f("")
            ),
        })
        .collect();
    if actions.is_empty() || !crate::mobile::mobile_view() {
        return buttons;
    }
    let items: String = actions
        .iter()
        .map(|a| {
            let (b, danger) = match a {
                Action::Primary(b) | Action::Default(b) => (b, false),
                Action::Danger(b) => (b, true),
                Action::Wrapped(f) => {
                    return format!(
                        "<li class=\"dropdown-menu__item d-page-action-list-item d-page-action-wrapped-list-item\">{}</li>",
                        f("btn-transparent")
                    );
                }
            };
            // A route button goes by itself; the others click their
            // hidden twin.
            let attrs = match b.class.split_whitespace().next() {
                Some(first) if b.href.is_none() => {
                    format!(" data-page-action=\"{}\"", escape(first))
                }
                _ => String::new(),
            };
            let class = format!(
                "d-page-action-button btn-small btn-transparent btn-{} {}",
                if danger { "danger" } else { "default" },
                b.class
            );
            format!(
                "<li class=\"dropdown-menu__item d-page-action-list-item\">{}</li>",
                Button {
                    class: class.trim_end(),
                    attrs,
                    ..b.clone()
                }
                .render()
            )
        })
        .collect();
    let hidden: String = actions
        .iter()
        .map(|a| match a {
            Action::Primary(b) if b.href.is_none() => button(b, "primary"),
            Action::Default(b) if b.href.is_none() => button(b, "default"),
            Action::Danger(b) if b.href.is_none() => button(b, "danger"),
            _ => String::new(),
        })
        .collect();
    let hidden = if hidden.is_empty() {
        hidden
    } else {
        format!("<div hidden>{hidden}</div>")
    };
    format!(
        "<button aria-expanded=\"false\" class=\"btn no-text btn-icon fk-d-menu__trigger {identifier}-mobile-actions-trigger btn-small btn-default\" title=\"{}\" data-identifier=\"{identifier}-mobile-actions\" data-trigger=\"\" type=\"button\">{}</button><template class=\"d-page-mobile-actions\"><ul class=\"dropdown-menu {identifier}__mobile-actions\">{items}</ul></template>{hidden}",
        escape(&t(cx.i18n, "more_options")),
        icon("ellipsis-vertical", None)
    )
}

/// DMenu's trigger (`@icon`, `@identifier`, `@title`, its classes) with
/// the content's markup in a template after it for the page's script.
pub fn menu(class: &str, identifier: &str, icon_name: &str, title: &str, content: &str) -> String {
    format!(
        "<button aria-expanded=\"false\" class=\"btn no-text btn-icon fk-d-menu__trigger {identifier}-trigger {}\" title=\"{}\" data-identifier=\"{identifier}\" data-trigger=\"\" type=\"button\">{}</button><template class=\"d-menu-content\" data-identifier=\"{identifier}\">{content}</template>",
        escape(class),
        escape(title),
        icon(icon_name, None)
    )
}

/// DPageSubheader with its title, actions and description (HTML).
pub fn subheader(title: &str, actions: &str, description: Option<&str>) -> String {
    let mut out = format!(
        "<div class=\"d-page-subheader\"><div class=\"d-page-subheader__title-row\"><h2 class=\"d-page-subheader__title\">{}</h2>",
        escape(title)
    );
    if !actions.is_empty() {
        out.push_str(&format!(
            "<div class=\"d-page-subheader__actions\">{actions}</div>"
        ));
    }
    out.push_str("</div>");
    if let Some(description) = description {
        out.push_str(&format!(
            "<p class=\"d-page-subheader__description\">{description}</p>"
        ));
    }
    out.push_str("</div>");
    out
}

/// DNavItems: (class, path, label key, active) under the base path.
pub fn tabs(cx: &AdminCx, items: &[(&str, &str, &str, bool)]) -> String {
    items
        .iter()
        .map(|(class, path, key, active)| {
            crate::admin_users_view::nav_item(
                class,
                &format!("{}{path}", cx.base_path),
                &t(cx.i18n, key),
                *active,
            )
        })
        .collect()
}

/// The strings a page's script translates with, as raw client strings
/// (`%{name}` left for the script), in a JSON script block with `id`;
/// with the dialog's own (dialog.js reads #dialog-strings) and
/// admin-ui.js, which the page's script uses, loaded before it.
pub fn strings(cx: &AdminCx, id: &str, keys: &[&str]) -> String {
    let own: serde_json::Map<String, Value> = keys
        .iter()
        .map(|k| {
            (
                k.to_string(),
                Value::String(t_opt(cx.i18n, k).unwrap_or_else(|| format!("[en.{k}]"))),
            )
        })
        .collect();
    let dialog = serde_json::json!({
        "ok_value": t(cx.i18n, "ok_value"),
        "cancel": t(cx.i18n, "cancel"),
        "yes_value": t(cx.i18n, "yes_value"),
        "no_value": t(cx.i18n, "no_value"),
        "modal.close": t(cx.i18n, "modal.close"),
    });
    format!(
        "<script type=\"application/json\" id=\"{id}\">{}</script><script type=\"application/json\" id=\"dialog-strings\">{}</script><script src=\"{}\"></script>",
        Value::Object(own).to_string().replace('<', "\\u003c"),
        dialog.to_string().replace('<', "\\u003c"),
        crate::assets::url(cx.base_path, "admin-ui.js")
    )
}

/// crate::admin_view::contents with data attributes (already escaped) on
/// the contents for the page's script.
pub fn contents(classes: &str, attrs: &str, inner: &str) -> String {
    crate::admin_view::contents(classes, inner).replacen(
        &format!("<div class=\"admin-contents {}\">", escape(classes)),
        &format!("<div class=\"admin-contents {}\"{attrs}>", escape(classes)),
        1,
    )
}

/// `dFormatDate` (format medium, with its title; `leaveAgo` the
/// medium-with-ago format): autoUpdatingRelativeAge's span, its text
/// relativeAgeMedium's. Dates are UTC here, the browser's zone in Ember.
pub fn format_date(cx: &AdminCx, at: chrono::DateTime<chrono::Utc>, leave_ago: bool) -> String {
    use chrono::Datelike;
    let now = crate::clock::now();
    let fmt = |at: chrono::DateTime<chrono::Utc>, key: &str| {
        crate::pretty_text::render::local_dates::format_utc(at, &t(cx.i18n, key))
            .unwrap_or_default()
    };
    let distance = ((now - at).num_milliseconds() as f64 / 1000.0).round() as i64;
    let text = if distance < 60 {
        t(cx.i18n, "now")
    } else if distance > 432_000 {
        if at.year() == now.year() {
            fmt(at, "dates.tiny.date_month")
        } else {
            fmt(at, "dates.medium.date_year")
        }
    } else {
        let minutes = (distance as f64 / 60.0).round() as i64;
        let (unit, count) = match minutes {
            1..=55 => ("x_minutes", minutes),
            56..=89 => ("x_hours", 1),
            90..=1409 => ("x_hours", (minutes as f64 / 60.0).round() as i64),
            1410..=2519 => ("x_days", 1),
            _ => ("x_days", (minutes as f64 / 1440.0).round() as i64),
        };
        let key = if leave_ago {
            "dates.medium_with_ago"
        } else {
            "dates.medium"
        };
        crate::admin_view::t_count(cx.i18n, &format!("{key}.{unit}"), count, &[])
    };
    format!(
        "<span class='relative-date date' title='{}' data-time='{}' data-format='{}'>{}</span>",
        escape(&fmt(at, "dates.long_with_year")),
        at.timestamp_millis(),
        if leave_ago {
            "medium-with-ago"
        } else {
            "medium"
        },
        escape(&text)
    )
}

/// A JSON time, for format_date.
pub fn time(v: &Value) -> Option<chrono::DateTime<chrono::Utc>> {
    v.as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&chrono::Utc))
}

/// `dAvatar user imageSize="small"`: the 24 pixel avatar titled with the
/// username.
pub fn avatar(user: &Value) -> String {
    crate::post_view::avatar_img(
        user["avatar_template"].as_str().unwrap_or_default(),
        24,
        &format!(
            " title=\"{}\"",
            escape(user["username"].as_str().unwrap_or_default())
        ),
    )
}

/// AdminConfigAreaEmptyList: the empty label (HTML) and the CTA button
/// to its route.
pub fn empty_list(
    cx: &AdminCx,
    cta_class: &str,
    cta_label: &str,
    href: &str,
    empty: &str,
) -> String {
    format!(
        "<div class=\"admin-config-area-empty-list\"><span class=\"admin-config-area-empty-list__title\">{}</span>{}</div>",
        t(cx.i18n, empty),
        Button {
            class: &format!(
                "btn-default btn-small admin-config-area-empty-list__cta-button {cta_class}"
            ),
            label: tr(cx, cta_label),
            href: Some(href),
            ..Default::default()
        }
        .render()
    )
}

/// BackButton: the link back with its label.
pub fn back_button(cx: &AdminCx, path: &str, label_key: &str) -> String {
    format!(
        "<a class=\"btn btn-transparent back-button\" href=\"{}{}\">{}{}</a>",
        cx.base_path,
        escape(path),
        icon("chevron-left", None),
        escape(&t(cx.i18n, label_key))
    )
}

/// `humanSize` (admin/helpers/human-size): I18n.toHumanSize.
pub fn human_size(cx: &AdminCx, bytes: i64) -> String {
    let units = ["", "kb", "mb", "gb", "tb"];
    let mut size = bytes as f64;
    let mut iterations = 0;
    while size >= 1024.0 && iterations < 4 {
        size /= 1024.0;
        iterations += 1;
    }
    let (unit, number) = if iterations == 0 {
        (
            crate::admin_view::t_count(
                cx.i18n,
                "number.human.storage_units.units.byte",
                bytes,
                &[],
            ),
            bytes.to_string(),
        )
    } else {
        let number = if size.fract() == 0.0 {
            format!("{size:.0}")
        } else {
            format!("{size:.1}")
        };
        (
            t(
                cx.i18n,
                &format!("number.human.storage_units.units.{}", units[iterations]),
            ),
            number,
        )
    };
    t(cx.i18n, "number.human.storage_units.format")
        .replace("%u", &unit)
        .replace("%n", &number)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buttons_take_dbuttons_classes() {
        let b = Button {
            class: "x",
            icon: Some("rocket"),
            label: Some("Go".into()),
            ..Default::default()
        };
        assert!(
            b.render()
                .starts_with("<button class=\"btn btn-icon-text x\"")
        );
        let b = Button {
            class: "x",
            icon: Some("rocket"),
            disabled: true,
            ..Default::default()
        };
        let html = b.render();
        assert!(html.starts_with("<button class=\"btn no-text btn-icon x\" disabled=\"\""));
        assert!(html.contains("<span aria-hidden=\"true\">\u{200b}</span>"));
        let b = Button {
            class: "x",
            label: Some("<Go>".into()),
            ..Default::default()
        };
        assert_eq!(
            b.render(),
            "<button class=\"btn x\" type=\"button\"><span class=\"d-button-label\">&lt;Go&gt;</span></button>"
        );
    }
}
