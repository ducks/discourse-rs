//! `Jobs::UpdateHeatSettings` (monthly): HeatSettingsUpdater, the topic
//! list's heat thresholds for views and likes per post moved to the
//! site's own percentiles.

use sqlx::PgConnection;

use crate::site_settings::{SiteSettings, Value};
use crate::{AppError, AppState};

/// `Jobs::UpdateHeatSettings`, every month.
pub const UPDATE_HEAT_SETTINGS: &str = "update_heat_settings";

/// `Discourse.system_user`
const SYSTEM_USER_ID: i32 = -1;

/// The percentiles each level takes: high the top 10%, medium 25%, low
/// 45%.
const LEVELS: [(&str, i64); 3] = [("high", 10), ("medium", 25), ("low", 45)];

/// `HeatSettingsUpdater.update` (when automatic_topic_heat_values): the
/// views thresholds, then the like ratio ones, each set and logged by
/// the system user on its own.
pub async fn update_heat_settings(state: &AppState) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    let s = SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    if !s.get("automatic_topic_heat_values")?.truthy() {
        return Ok(());
    }
    // views_thresholds
    let views: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT (ranked.bucket * 5)::bigint AS percentile, MIN(ranked.views)::bigint AS views \
         FROM (SELECT NTILE(20) OVER (ORDER BY t.views DESC) AS bucket, t.views \
               FROM (SELECT views FROM topics WHERE deleted_at IS NULL \
                       AND archetype <> 'private_message' AND visible = TRUE) t) ranked \
         WHERE bucket <= 9 GROUP BY bucket",
    )
    .fetch_all(&mut *conn)
    .await?;
    for (level, percentile) in LEVELS {
        let found = views
            .iter()
            .find(|(p, _)| *p == percentile)
            .map(|(_, v)| *v);
        update_setting(
            &mut conn,
            state,
            &format!("topic_views_heat_{level}"),
            found.map(Threshold::Views),
        )
        .await?;
    }
    // like_ratio_thresholds: NUMERIC ratios, and each rounded to two
    // places half up (BigDecimal#round), as the database rounds them.
    let ratios: Vec<(i64, f64, f64)> = sqlx::query_as(
        "SELECT (ranked.bucket * 5)::bigint AS percentile, MIN(ranked.ratio)::float8, \
                ROUND(MIN(ranked.ratio), 2)::float8 \
         FROM (SELECT NTILE(20) OVER (ORDER BY t.ratio DESC) AS bucket, t.ratio \
               FROM (SELECT like_count::decimal / posts_count AS ratio FROM topics \
                     WHERE deleted_at IS NULL AND archetype <> 'private_message' AND visible = TRUE \
                       AND posts_count >= 10 AND like_count > 0 \
                     ORDER BY created_at DESC LIMIT 1000) t) ranked \
         WHERE bucket <= 9 GROUP BY bucket",
    )
    .fetch_all(&mut *conn)
    .await?;
    for (level, percentile) in LEVELS {
        let found = ratios
            .iter()
            .find(|(p, _, _)| *p == percentile)
            .map(|(_, ratio, rounded)| Threshold::Ratio(*ratio, *rounded));
        update_setting(
            &mut conn,
            state,
            &format!("topic_post_like_heat_{level}"),
            found,
        )
        .await?;
    }
    Ok(())
}

/// A percentile's value: views, or a like ratio with its rounding.
enum Threshold {
    Views(i64),
    Ratio(f64, f64),
}

/// `HeatSettingsUpdater.update_setting`: back to the default when there is
/// no value or it is at most the default; else, moved by 5% or more (or
/// from zero), the value rounded (two leading digits of a count, two
/// places of a ratio) when it differs.
async fn update_setting(
    conn: &mut PgConnection,
    state: &AppState,
    name: &str,
    threshold: Option<Threshold>,
) -> Result<(), AppError> {
    let s = SiteSettings::load(&mut *conn, &state.site_setting_defs, &state.config.globals).await?;
    let def = state
        .site_setting_defs
        .get(name)
        .ok_or(crate::Unsupported("a heat setting without its definition"))?;
    let as_f64 = |v: &Value| match v {
        Value::Int(i) => *i as f64,
        Value::Float(f) => *f,
        _ => 0.0,
    };
    let default = def.default.clone();
    let current = s.get(name)?.clone();
    let new_value = threshold.as_ref().map(|t| match t {
        Threshold::Views(v) => *v as f64,
        Threshold::Ratio(r, _) => *r,
    });
    let target = match new_value {
        None => Some(default.clone()),
        Some(v) if v <= as_f64(&default) => Some(default.clone()),
        Some(v) => {
            let cur = as_f64(&current);
            if cur == 0.0 || (v / cur - 1.0).abs() >= 0.05 {
                Some(match threshold {
                    Some(Threshold::Views(v)) => Value::Int(two_leading_digits(v)),
                    Some(Threshold::Ratio(_, rounded)) => Value::Float(rounded),
                    None => unreachable!("a new value has its threshold"),
                })
            } else {
                None
            }
        }
    };
    let Some(target) = target else {
        return Ok(());
    };
    if as_f64(&current) == as_f64(&target) {
        return Ok(());
    }
    crate::site_setting_update::set_and_log(
        &mut *conn,
        &state.bus,
        def,
        &current,
        &target,
        SYSTEM_USER_ID,
    )
    .await
}

/// A count past nine keeps its two leading digits: 5678 is 5600, 12345
/// is 12000.
fn two_leading_digits(v: i64) -> i64 {
    if v <= 9 {
        return v;
    }
    let digits = v.to_string().len() as u32;
    let lead = v / 10_i64.pow(digits - 2);
    lead * 10_i64.pow(digits - 2)
}

#[cfg(test)]
mod tests {
    use super::two_leading_digits;

    #[test]
    fn counts_keep_two_leading_digits() {
        assert_eq!(two_leading_digits(5678), 5600);
        assert_eq!(two_leading_digits(12345), 12000);
        assert_eq!(two_leading_digits(99), 99);
        assert_eq!(two_leading_digits(10), 10);
        assert_eq!(two_leading_digits(7), 7);
    }
}
