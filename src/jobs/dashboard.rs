//! Jobs::WarmDashboardReports: AdminDashboardCacheWarmer fills the report
//! cache for the dashboard's KPIs over the windows a viewer's timezone
//! can ask for, while staff have been around.

use chrono::Duration;

use crate::reports::{self, Opts, Viewer};
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

/// `Jobs::WarmDashboardReports`, every 30 minutes.
pub const WARM_DASHBOARD_REPORTS: &str = "warm_dashboard_reports";

/// `Jobs::WarmDashboardReports#execute`
pub async fn warm_dashboard_reports(state: &AppState) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    if !settings.get("dashboard_improvements")?.truthy() {
        return Ok(());
    }
    // recently_active_staff?
    let active: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM users WHERE id > 0 AND (admin OR moderator) \
           AND last_seen_at > now() - interval '7 days')",
    )
    .fetch_one(&mut *conn)
    .await?;
    if !active {
        return Ok(());
    }
    let urls = crate::url::Urls {
        config: &state.config,
        settings: &settings,
    };
    let cx = reports::context(
        &mut conn,
        &settings,
        &state.site_setting_defs,
        &state.i18n,
        &urls,
    )
    .await?;
    // report_specs: the KPI reports (highlights' then engagement's, each
    // once) with the previous period, and the trust level pipeline.
    let mut kpi_reports = vec!["signups", "dau_by_mau", "new_contributors"];
    if crate::admin_dashboard::solved_kpi_on(&mut conn, &cx).await? {
        kpi_reports.push("accepted_solutions");
    }
    for r in ["dau_by_mau", "daily_engaged_users", "signups"] {
        if !kpi_reports.contains(&r) {
            kpi_reports.push(r);
        }
    }
    let viewer = Viewer { guardian: None };
    let today = crate::clock::now_naive().date();
    for offset in -1..=1 {
        let end = today + Duration::days(offset);
        let start = reports::beginning_of_day(end - Duration::days(29));
        let end = reports::end_of_day(end);
        let mut specs: Vec<(&str, Opts)> = kpi_reports
            .iter()
            .map(|r| {
                (
                    *r,
                    Opts {
                        start_date: Some(start),
                        end_date: Some(end),
                        facets: Some(vec!["prev_period".into()]),
                        ..Opts::default()
                    },
                )
            })
            .collect();
        specs.push((
            "trust_level_pipeline",
            Opts {
                start_date: Some(start),
                end_date: Some(end),
                ..Opts::default()
            },
        ));
        for (kind, opts) in specs {
            if let Some(report) = reports::find(&mut conn, &cx, &viewer, kind, &opts).await?
                && report.error.is_none()
            {
                reports::cache(&mut conn, &cx, &report).await?;
            }
        }
    }
    Ok(())
}
