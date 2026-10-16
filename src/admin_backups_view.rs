//! The admin backups pages (templates/admin/backups.gjs): the page header
//! with AdminBackupsActions and the Settings, Backup files and Logs tabs,
//! over the backup files' table (backups/index.gjs), the logs
//! (AdminBackupsLogs) or the backups category's AdminAreaSettings.
//! static/js/admin-backups.js does what the route's actions do.

use serde_json::Value;

use crate::admin_ui::{Action, Button, tr};
use crate::admin_view::{AdminCx, Crumb, PageHeader, t, t_with};
use crate::topic_list_view::{escape, icon};

/// Which tab is open.
#[derive(Clone, Copy, PartialEq)]
pub enum Tab {
    Settings,
    Files,
    Logs,
}

/// BackupStatus, from BackupRestore.operations_status, with what the
/// actions read of the site.
pub struct Status {
    pub is_operation_running: bool,
    pub can_rollback: bool,
    pub allow_restore: bool,
    pub enable_backups: bool,
    pub read_only: bool,
    pub site_archived: bool,
    pub local_storage: bool,
}

impl Status {
    pub fn from_json(v: &Value) -> Status {
        let flag = |k: &str| v[k].as_bool().unwrap_or(false);
        Status {
            is_operation_running: flag("is_operation_running"),
            can_rollback: flag("can_rollback"),
            allow_restore: flag("allow_restore"),
            enable_backups: false,
            read_only: false,
            site_archived: false,
            local_storage: true,
        }
    }

    fn restore_enabled(&self) -> bool {
        self.allow_restore && !self.is_operation_running
    }
}

/// AdminBackupsActions
fn header_actions(cx: &AdminCx, s: &Status) -> String {
    let mut actions = Vec::new();
    if s.is_operation_running {
        actions.push(Action::Danger(Button {
            class: "admin-backups__cancel",
            icon: Some("xmark"),
            label: tr(cx, "admin.backups.operations.cancel.label"),
            title: tr(cx, "admin.backups.operations.cancel.title"),
            ..Default::default()
        }));
    } else {
        actions.push(Action::Primary(Button {
            class: "admin-backups__start",
            icon: Some("rocket"),
            label: tr(cx, "admin.backups.operations.backup.label"),
            title: tr(cx, "admin.backups.operations.backup.title"),
            ..Default::default()
        }));
    }
    if s.can_rollback {
        actions.push(Action::Default(Button {
            class: "admin-backups__rollback",
            icon: Some("truck-medical"),
            label: tr(cx, "admin.backups.operations.rollback.label"),
            title: tr(cx, "admin.backups.operations.rollback.title"),
            disabled: !(s.restore_enabled()),
            ..Default::default()
        }));
    }
    let ro = if s.read_only { "disable" } else { "enable" };
    actions.push(Action::Default(Button {
        class: "admin-backups__toggle-read-only",
        icon: Some(if s.read_only {
            "far-eye-slash"
        } else {
            "far-eye"
        }),
        label: tr(cx, &format!("admin.backups.read_only.{ro}.label")),
        title: tr(cx, &format!("admin.backups.read_only.{ro}.title")),
        disabled: s.is_operation_running,
        ..Default::default()
    }));
    let archived = if s.site_archived { "disable" } else { "enable" };
    actions.push(Action::Default(Button {
        class: "admin-backups__toggle-site-archived",
        icon: Some("box-archive"),
        label: tr(cx, &format!("admin.backups.site_archived.{archived}.label")),
        title: tr(cx, &format!("admin.backups.site_archived.{archived}.title")),
        disabled: s.is_operation_running,
        ..Default::default()
    }));
    crate::admin_ui::actions(cx, "d-page-header", &actions)
}

/// The page: the header, then the tab's content in the main area.
pub fn render(cx: &AdminCx, tab: Tab, s: &Status, content: &str) -> String {
    let title = t(cx.i18n, "admin.config.backups.title");
    let mut crumbs = vec![
        Crumb {
            label: t(cx.i18n, "admin_title"),
            path: "/admin",
        },
        Crumb {
            label: title.clone(),
            path: "/admin/backups",
        },
    ];
    match tab {
        Tab::Logs => crumbs.push(Crumb {
            label: t(cx.i18n, "admin.backups.menu.logs"),
            path: "/admin/backups/logs",
        }),
        // AdminAreaSettings adds its own crumb.
        Tab::Settings => crumbs.push(Crumb {
            label: t(cx.i18n, "settings"),
            path: "/admin/backups/settings",
        }),
        Tab::Files => {}
    }
    let tabs = crate::admin_ui::tabs(
        cx,
        &[
            (
                "admin-backups-tabs__settings",
                "/admin/backups/settings",
                "settings",
                tab == Tab::Settings,
            ),
            (
                "admin-backups-tabs__files",
                "/admin/backups",
                "admin.backups.menu.backup_files",
                tab == Tab::Files,
            ),
            (
                "admin-backups-tabs__logs",
                "/admin/backups/logs",
                "admin.backups.menu.logs",
                tab == Tab::Logs,
            ),
        ],
    );
    let header = PageHeader {
        crumbs,
        title,
        actions: if s.enable_backups {
            header_actions(cx, s)
        } else {
            String::new()
        },
        description: Some(escape(&t(cx.i18n, "admin.config.backups.header_description"))),
        learn_more_url: Some(
            "https://meta.discourse.org/t/create-download-and-restore-a-backup-of-your-discourse-database/122710"
                .into(),
        ),
        tabs: Some(tabs),
    }
    .render(cx);
    format!(
        "<div class=\"admin-backups admin-config-page\">{header}<div class=\"admin-container admin-config-page__main-area\">{content}</div></div>"
    )
}

/// UppyBackupUploader's label, given the class its wrapper yields.
fn uploader(cx: &AdminCx, class: &str) -> String {
    format!(
        "<span><label class=\"btn btn-small btn-primary admin-backups-upload{}\" title=\"{}\">{}<input accept=\".gz\" class=\"hidden-upload-field\" type=\"file\"></label></span>",
        if class.is_empty() {
            String::new()
        } else {
            format!(" {class}")
        },
        escape(&t(cx.i18n, "admin.backups.upload.title")),
        escape(&t(cx.i18n, "admin.backups.upload.label"))
    )
}

/// backups/index.gjs: the subheader with the uploader, the restore
/// notice, and the files' table.
pub fn index(cx: &AdminCx, s: &Status, files: &[Value]) -> String {
    // `actions.Wrapped` is yielded whether or not the uploader is in it.
    let enabled = s.enable_backups;
    let actions = [Action::Wrapped(Box::new(move |class: &str| {
        if enabled {
            uploader(cx, class)
        } else {
            String::new()
        }
    }))];
    let mut out = crate::admin_ui::subheader(
        &t(cx.i18n, "admin.backups.files_title"),
        &crate::admin_ui::actions(cx, "d-page-subheader", &actions),
        None,
    );
    let restore_disabled = !s.restore_enabled();
    if restore_disabled {
        out.push_str(&format!(
            "<div class=\"backup-message alert alert-info\">{}{}</div>",
            icon("circle-info", None),
            t_with(
                cx.i18n,
                "admin.backups.operations.restore.is_disabled",
                &[(
                    "url",
                    &format!(
                        "{}/admin/backups/settings?filter=allow_restore",
                        cx.base_path
                    )
                )]
            )
        ));
    }
    out.push_str(&format!(
        "<table class=\"d-table admin-backups-list\"><thead class=\"d-table__header\"><tr class=\"d-table__row\"><th class=\"d-table__header-cell\">{}</th><th class=\"backup-size\">{}</th><th class=\"d-table__header-cell\"></th></tr></thead><tbody class=\"d-table__body\">",
        escape(&t(cx.i18n, "admin.backups.columns.filename")),
        escape(&t(cx.i18n, "admin.backups.columns.size"))
    ));
    let restore_title = if !s.allow_restore {
        "admin.backups.operations.restore.is_disabled_title"
    } else if s.is_operation_running {
        "admin.backups.operations.is_running"
    } else {
        "admin.backups.operations.restore.title"
    };
    let delete_title = if s.is_operation_running {
        "admin.backups.operations.is_running"
    } else {
        "admin.backups.operations.destroy.title"
    };
    for backup in files {
        let filename = backup["filename"].as_str().unwrap_or_default();
        let size = backup["size"].as_i64().unwrap_or(0);
        out.push_str(&format!(
            "<tr class=\"d-table__row backup-item-row\" data-backup-filename=\"{f}\"><td class=\"d-table__cell --overview\"><div class=\"backup-filename\">{f}</div></td><td class=\"d-table__cell --detail backup-size\"><div class=\"d-table__mobile-label\">{}</div>{}</td><td class=\"d-table__cell --controls backup-controls\"><div class=\"d-table__cell-actions\">",
            escape(&t(cx.i18n, "admin.backups.columns.size")),
            escape(&crate::admin_ui::human_size(cx, size)),
            f = escape(filename)
        ));
        out.push_str(
            &Button {
                class: "btn-default btn-small backup-item-row__download",
                label: tr(cx, "admin.backups.operations.download.label"),
                title: tr(cx, "admin.backups.operations.download.title"),
                ..Default::default()
            }
            .render(),
        );
        if s.enable_backups {
            let content = format!(
                "<ul class=\"dropdown-menu\"><li class=\"dropdown-menu__item\">{}</li><li class=\"dropdown-menu__item\">{}</li></ul>",
                Button {
                    class: "btn-transparent backup-item-row__restore",
                    icon: Some("play"),
                    label: tr(cx, "admin.backups.operations.restore.label"),
                    title: tr(cx, restore_title),
                    disabled: restore_disabled,
                    ..Default::default()
                }
                .render(),
                Button {
                    class: "btn-transparent --danger backup-item-row__delete",
                    icon: Some("trash-can"),
                    label: tr(cx, "admin.backups.operations.destroy.title"),
                    title: tr(cx, delete_title),
                    disabled: s.is_operation_running,
                    ..Default::default()
                }
                .render()
            );
            out.push_str(&crate::admin_ui::menu(
                "btn-default btn-small",
                "backup-item-menu",
                "ellipsis-vertical",
                &t(cx.i18n, "more_options"),
                &content,
            ));
        }
        out.push_str("</div></td></tr>");
    }
    if files.is_empty() {
        out.push_str(&format!(
            "<tr><td>{}</td><td></td><td></td></tr>",
            escape(&t(cx.i18n, "admin.backups.none"))
        ));
    }
    out.push_str("</tbody></table>");
    out
}

/// AdminBackupsLogs: the logs since the operation started, less the
/// status lines (`[STARTED]` and the like), and the spinner while it runs.
pub fn logs(cx: &AdminCx, s: &Status, logs: &[Value]) -> String {
    let lines: String = logs
        .iter()
        .filter(|log| {
            let m = log["message"].as_str().unwrap_or_default();
            !m.is_empty() && !m.starts_with('[')
        })
        .map(|log| {
            format!(
                "[{}] {}\n",
                log["timestamp"].as_str().unwrap_or_default(),
                log["message"].as_str().unwrap_or_default()
            )
        })
        .collect();
    let body = if lines.is_empty() {
        format!("<p>{}</p>", escape(&t(cx.i18n, "admin.backups.logs.none")))
    } else {
        format!("<pre>{}</pre>", escape(&lines))
    };
    let spinner = if s.is_operation_running {
        "<div class=\"loading-container visible inline-spinner\" data-loading=\"true\"><div class=\"spinner small\"></div></div>"
    } else {
        "<div class=\"loading-container inline-spinner\"></div>"
    };
    format!("<div class=\"admin-backups-logs\">{body}{spinner}</div>")
}

/// The strings admin-backups.js shows.
pub fn strings(cx: &AdminCx) -> String {
    crate::admin_ui::strings(
        cx,
        "admin-backups-strings",
        &[
            "admin.backups.read_only.enable.confirm",
            "admin.backups.site_archived.enable.confirm",
            "admin.backups.operations.backup.confirm",
            "admin.backups.operations.backup.include_uploads",
            "admin.backups.operations.backup.s3_upload_warning",
            "admin.backups.operations.cancel.confirm",
            "admin.backups.operations.destroy.confirm",
            "admin.backups.operations.restore.confirm",
            "admin.backups.operations.rollback.confirm",
            "admin.backups.operations.download.alert",
            "admin.backups.operations.failed",
            "admin.backups.upload.success",
            "admin.backups.upload.error",
            "admin.backups.upload.uploading_progress",
            "admin.backups.logs.none",
            "yes_value",
            "cancel",
            "generic_error",
        ],
    )
}
