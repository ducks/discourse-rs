//! discourse-cakeday: the anniversary of the day a user joined and their
//! birthday (core's `users.date_of_birth`), as the keys it adds to the
//! user, user card and post serializers, the anniversaries and birthdays
//! lists (`lists`), their pages and the cake beside a poster's name
//! (`view`), and its onceoff jobs (`jobs`).

pub mod jobs;
pub mod lists;
pub mod view;

use std::collections::HashMap;

use chrono::{NaiveDate, NaiveDateTime, TimeZone};
use chrono_tz::Tz;
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use super::PluginError;
use crate::guardian::Guardian;
use crate::site_settings::{SettingError, SiteSettings};

/// `enabled_site_setting :cakeday_enabled`
pub fn enabled(settings: &SiteSettings) -> Result<bool, SettingError> {
    Ok(settings.get("cakeday_enabled")?.truthy())
}

/// `SiteSetting.cakeday_birthday_enabled`, which counts only while the
/// plugin is enabled (add_to_serializer's respect_plugin_enabled).
pub fn birthday_enabled(settings: &SiteSettings) -> Result<bool, SettingError> {
    Ok(enabled(settings)? && settings.get("cakeday_birthday_enabled")?.truthy())
}

/// ActiveSupport::TimeZone::MAPPING: Rails' own zone names.
const RAILS_ZONES: &[(&str, &str)] = &[
    ("International Date Line West", "Etc/GMT+12"),
    ("Midway Island", "Pacific/Midway"),
    ("American Samoa", "Pacific/Pago_Pago"),
    ("Hawaii", "Pacific/Honolulu"),
    ("Alaska", "America/Juneau"),
    ("Pacific Time (US & Canada)", "America/Los_Angeles"),
    ("Tijuana", "America/Tijuana"),
    ("Mountain Time (US & Canada)", "America/Denver"),
    ("Arizona", "America/Phoenix"),
    ("Chihuahua", "America/Chihuahua"),
    ("Mazatlan", "America/Mazatlan"),
    ("Central Time (US & Canada)", "America/Chicago"),
    ("Saskatchewan", "America/Regina"),
    ("Guadalajara", "America/Mexico_City"),
    ("Mexico City", "America/Mexico_City"),
    ("Monterrey", "America/Monterrey"),
    ("Central America", "America/Guatemala"),
    ("Eastern Time (US & Canada)", "America/New_York"),
    ("Indiana (East)", "America/Indiana/Indianapolis"),
    ("Bogota", "America/Bogota"),
    ("Lima", "America/Lima"),
    ("Quito", "America/Lima"),
    ("Atlantic Time (Canada)", "America/Halifax"),
    ("Caracas", "America/Caracas"),
    ("La Paz", "America/La_Paz"),
    ("Santiago", "America/Santiago"),
    ("Asuncion", "America/Asuncion"),
    ("Newfoundland", "America/St_Johns"),
    ("Brasilia", "America/Sao_Paulo"),
    ("Buenos Aires", "America/Argentina/Buenos_Aires"),
    ("Montevideo", "America/Montevideo"),
    ("Georgetown", "America/Guyana"),
    ("Puerto Rico", "America/Puerto_Rico"),
    ("Greenland", "America/Nuuk"),
    ("Mid-Atlantic", "Atlantic/South_Georgia"),
    ("Azores", "Atlantic/Azores"),
    ("Cape Verde Is.", "Atlantic/Cape_Verde"),
    ("Dublin", "Europe/Dublin"),
    ("Edinburgh", "Europe/London"),
    ("Lisbon", "Europe/Lisbon"),
    ("London", "Europe/London"),
    ("Casablanca", "Africa/Casablanca"),
    ("Monrovia", "Africa/Monrovia"),
    ("UTC", "Etc/UTC"),
    ("Belgrade", "Europe/Belgrade"),
    ("Bratislava", "Europe/Bratislava"),
    ("Budapest", "Europe/Budapest"),
    ("Ljubljana", "Europe/Ljubljana"),
    ("Prague", "Europe/Prague"),
    ("Sarajevo", "Europe/Sarajevo"),
    ("Skopje", "Europe/Skopje"),
    ("Warsaw", "Europe/Warsaw"),
    ("Zagreb", "Europe/Zagreb"),
    ("Brussels", "Europe/Brussels"),
    ("Copenhagen", "Europe/Copenhagen"),
    ("Madrid", "Europe/Madrid"),
    ("Paris", "Europe/Paris"),
    ("Amsterdam", "Europe/Amsterdam"),
    ("Berlin", "Europe/Berlin"),
    ("Bern", "Europe/Zurich"),
    ("Zurich", "Europe/Zurich"),
    ("Rome", "Europe/Rome"),
    ("Stockholm", "Europe/Stockholm"),
    ("Vienna", "Europe/Vienna"),
    ("West Central Africa", "Africa/Algiers"),
    ("Bucharest", "Europe/Bucharest"),
    ("Cairo", "Africa/Cairo"),
    ("Helsinki", "Europe/Helsinki"),
    ("Kyiv", "Europe/Kiev"),
    ("Riga", "Europe/Riga"),
    ("Sofia", "Europe/Sofia"),
    ("Tallinn", "Europe/Tallinn"),
    ("Vilnius", "Europe/Vilnius"),
    ("Athens", "Europe/Athens"),
    ("Istanbul", "Europe/Istanbul"),
    ("Minsk", "Europe/Minsk"),
    ("Jerusalem", "Asia/Jerusalem"),
    ("Harare", "Africa/Harare"),
    ("Pretoria", "Africa/Johannesburg"),
    ("Kaliningrad", "Europe/Kaliningrad"),
    ("Moscow", "Europe/Moscow"),
    ("St. Petersburg", "Europe/Moscow"),
    ("Volgograd", "Europe/Volgograd"),
    ("Samara", "Europe/Samara"),
    ("Kuwait", "Asia/Kuwait"),
    ("Riyadh", "Asia/Riyadh"),
    ("Nairobi", "Africa/Nairobi"),
    ("Baghdad", "Asia/Baghdad"),
    ("Tehran", "Asia/Tehran"),
    ("Abu Dhabi", "Asia/Muscat"),
    ("Muscat", "Asia/Muscat"),
    ("Baku", "Asia/Baku"),
    ("Tbilisi", "Asia/Tbilisi"),
    ("Yerevan", "Asia/Yerevan"),
    ("Kabul", "Asia/Kabul"),
    ("Ekaterinburg", "Asia/Yekaterinburg"),
    ("Islamabad", "Asia/Karachi"),
    ("Karachi", "Asia/Karachi"),
    ("Tashkent", "Asia/Tashkent"),
    ("Chennai", "Asia/Kolkata"),
    ("Kolkata", "Asia/Kolkata"),
    ("Mumbai", "Asia/Kolkata"),
    ("New Delhi", "Asia/Kolkata"),
    ("Kathmandu", "Asia/Kathmandu"),
    ("Dhaka", "Asia/Dhaka"),
    ("Sri Jayawardenepura", "Asia/Colombo"),
    ("Almaty", "Asia/Almaty"),
    ("Astana", "Asia/Almaty"),
    ("Novosibirsk", "Asia/Novosibirsk"),
    ("Rangoon", "Asia/Rangoon"),
    ("Bangkok", "Asia/Bangkok"),
    ("Hanoi", "Asia/Bangkok"),
    ("Jakarta", "Asia/Jakarta"),
    ("Krasnoyarsk", "Asia/Krasnoyarsk"),
    ("Beijing", "Asia/Shanghai"),
    ("Chongqing", "Asia/Chongqing"),
    ("Hong Kong", "Asia/Hong_Kong"),
    ("Urumqi", "Asia/Urumqi"),
    ("Kuala Lumpur", "Asia/Kuala_Lumpur"),
    ("Singapore", "Asia/Singapore"),
    ("Taipei", "Asia/Taipei"),
    ("Perth", "Australia/Perth"),
    ("Irkutsk", "Asia/Irkutsk"),
    ("Ulaanbaatar", "Asia/Ulaanbaatar"),
    ("Seoul", "Asia/Seoul"),
    ("Osaka", "Asia/Tokyo"),
    ("Sapporo", "Asia/Tokyo"),
    ("Tokyo", "Asia/Tokyo"),
    ("Yakutsk", "Asia/Yakutsk"),
    ("Darwin", "Australia/Darwin"),
    ("Adelaide", "Australia/Adelaide"),
    ("Canberra", "Australia/Canberra"),
    ("Melbourne", "Australia/Melbourne"),
    ("Sydney", "Australia/Sydney"),
    ("Brisbane", "Australia/Brisbane"),
    ("Hobart", "Australia/Hobart"),
    ("Vladivostok", "Asia/Vladivostok"),
    ("Guam", "Pacific/Guam"),
    ("Port Moresby", "Pacific/Port_Moresby"),
    ("Magadan", "Asia/Magadan"),
    ("Srednekolymsk", "Asia/Srednekolymsk"),
    ("Solomon Is.", "Pacific/Guadalcanal"),
    ("New Caledonia", "Pacific/Noumea"),
    ("Fiji", "Pacific/Fiji"),
    ("Kamchatka", "Asia/Kamchatka"),
    ("Marshall Is.", "Pacific/Majuro"),
    ("Auckland", "Pacific/Auckland"),
    ("Wellington", "Pacific/Auckland"),
    ("Nuku'alofa", "Pacific/Tongatapu"),
    ("Tokelau Is.", "Pacific/Fakaofo"),
    ("Chatham Is.", "Pacific/Chatham"),
    ("Samoa", "Pacific/Apia"),
];

/// `ActiveSupport::TimeZone[name]`: a Rails zone name or a tz identifier,
/// None for anything else.
pub fn zone(name: &str) -> Option<Tz> {
    let id = RAILS_ZONES
        .iter()
        .find(|(rails, _)| *rails == name)
        .map_or(name, |(_, id)| *id);
    id.parse().ok()
}

/// The viewer's `user_option.timezone` as a zone: UTC without one, and
/// for a name Rails would raise on (`in_time_zone`), where the lists fall
/// back to the server's zone.
pub async fn viewer_zone(conn: &mut PgConnection, user_id: Option<i32>) -> Result<Tz, sqlx::Error> {
    let Some(user_id) = user_id else {
        return Ok(chrono_tz::UTC);
    };
    let name: Option<String> =
        sqlx::query_scalar("SELECT timezone FROM user_options WHERE user_id = $1")
            .bind(user_id)
            .fetch_optional(conn)
            .await?
            .flatten();
    Ok(name
        .as_deref()
        .filter(|n| !n.trim().is_empty())
        .and_then(zone)
        .unwrap_or(chrono_tz::UTC))
}

/// The viewer's date today, in their zone.
pub async fn viewer_today(
    conn: &mut PgConnection,
    user_id: Option<i32>,
) -> Result<NaiveDate, sqlx::Error> {
    let zone = viewer_zone(conn, user_id).await?;
    Ok(crate::clock::now().with_timezone(&zone).date_naive())
}

/// `created_at.in_time_zone(timezone).strftime("%Y-%m-%d")`
pub fn cakedate(created_at: NaiveDateTime, zone: Tz) -> String {
    zone.from_utc_datetime(&created_at)
        .format("%Y-%m-%d")
        .to_string()
}

/// The include condition every key shares: a viewer, and a profile not
/// hidden from them (`hide_profile` alone, whatever
/// allow_users_to_hide_profile says).
fn shown_to(guardian: &Guardian, user_id: i32, hide_profile: bool) -> bool {
    guardian
        .user_id()
        .is_some_and(|viewer| !hide_profile || viewer == user_id)
}

/// The user and user card serializers' keys: `cakedate` and, with
/// birthdays on, `birthdate` (null without a date of birth).
pub async fn user_keys(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
    user_id: i32,
    out: &mut Map<String, Value>,
) -> Result<(), PluginError> {
    if !enabled(settings)? {
        return Ok(());
    }
    let row: Option<(NaiveDateTime, Option<NaiveDate>, Option<bool>)> = sqlx::query_as(
        "SELECT u.created_at, u.date_of_birth, uo.hide_profile FROM users u \
         LEFT JOIN user_options uo ON uo.user_id = u.id WHERE u.id = $1",
    )
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((created_at, date_of_birth, hide_profile)) = row else {
        return Ok(());
    };
    if !shown_to(guardian, user_id, hide_profile.unwrap_or(false)) {
        return Ok(());
    }
    let zone = viewer_zone(conn, guardian.user_id()).await?;
    out.insert("cakedate".into(), json!(cakedate(created_at, zone)));
    if birthday_enabled(settings)? {
        out.insert("birthdate".into(), json!(date_of_birth.map(date_json)));
    }
    Ok(())
}

/// A date as Rails serializes one.
pub fn date_json(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

/// What the post serializer's keys read for a page of posts: each
/// poster's join date, date of birth and hidden profile, the viewer's
/// zone.
#[derive(Debug)]
pub struct PostsData {
    zone: Tz,
    birthdays: bool,
    users: HashMap<i32, (NaiveDateTime, Option<NaiveDate>, bool)>,
}

impl PostsData {
    /// None while the plugin is off or for an anonymous viewer, who gets
    /// neither key.
    pub async fn load(
        conn: &mut PgConnection,
        settings: &SiteSettings,
        guardian: &Guardian,
        user_ids: &[i32],
    ) -> Result<Option<Self>, PluginError> {
        if !enabled(settings)? || guardian.user_id().is_none() {
            return Ok(None);
        }
        let rows: Vec<(i32, NaiveDateTime, Option<NaiveDate>, Option<bool>)> = sqlx::query_as(
            "SELECT u.id, u.created_at, u.date_of_birth, uo.hide_profile FROM users u \
             LEFT JOIN user_options uo ON uo.user_id = u.id WHERE u.id = ANY($1)",
        )
        .bind(user_ids)
        .fetch_all(&mut *conn)
        .await?;
        Ok(Some(PostsData {
            zone: viewer_zone(conn, guardian.user_id()).await?,
            birthdays: birthday_enabled(settings)?,
            users: rows
                .into_iter()
                .map(|(id, c, d, h)| (id, (c, d, h.unwrap_or(false))))
                .collect(),
        }))
    }

    /// The post's keys, in the order the plugin adds them:
    /// `user_cakedate`, then `user_birthdate` for a poster with a date of
    /// birth.
    pub fn post_keys(
        &self,
        guardian: &Guardian,
        user_id: Option<i32>,
        out: &mut Map<String, Value>,
    ) {
        let Some((user_id, (created_at, date_of_birth, hide_profile))) =
            user_id.and_then(|id| self.users.get(&id).map(|u| (id, u)))
        else {
            return;
        };
        if !shown_to(guardian, user_id, *hide_profile) {
            return;
        }
        out.insert(
            "user_cakedate".into(),
            json!(cakedate(*created_at, self.zone)),
        );
        if self.birthdays
            && let Some(d) = date_of_birth
        {
            out.insert("user_birthdate".into(), json!(date_json(*d)));
        }
    }
}

/// ActiveModel's date cast, as `user.date_of_birth = value` applies it:
/// blank or null is none, YYYY-MM-DD is read directly and anything else
/// through `Date._parse(value, false)`, none unless it names a valid year,
/// month and day.
pub fn cast_date(value: &Value) -> Result<Option<NaiveDate>, crate::Unsupported> {
    let s = match value {
        Value::Null => return Ok(None),
        Value::String(s) => s,
        _ => return Err(crate::Unsupported("a date of birth that is not a string")),
    };
    if s.is_empty() {
        return Ok(None);
    }
    if !s.is_ascii() {
        return Err(crate::Unsupported("a date of birth with non-ASCII text"));
    }
    let b = s.as_bytes();
    if b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
    {
        return Ok(NaiveDate::parse_from_str(s, "%Y-%m-%d").ok());
    }
    let h = crate::ruby_date::parse(s);
    // Date._parse with comp false keeps a two digit year as written; the
    // port's parser completes it, so such a year is refused.
    if h.year.is_some_and(|y| (0..=99).contains(&y)) {
        return Err(crate::Unsupported("a date of birth with a two digit year"));
    }
    Ok(match (h.year, h.mon, h.mday) {
        (Some(0), Some(0), Some(0)) => None,
        (Some(y), Some(m), Some(d)) => i32::try_from(y).ok().and_then(|y| {
            NaiveDate::from_ymd_opt(y, u32::try_from(m).ok()?, u32::try_from(d).ok()?)
        }),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zones_take_rails_names_and_tz_identifiers() {
        assert_eq!(zone("Nuku'alofa"), Some(chrono_tz::Pacific::Tongatapu));
        assert_eq!(zone("Asia/Kolkata"), Some(chrono_tz::Asia::Kolkata));
        assert_eq!(zone("UTC"), Some(chrono_tz::Etc::UTC));
        assert_eq!(zone("Invalid/Timezone"), None);
    }

    #[test]
    fn cakedates_are_the_viewers_dates() {
        let at = NaiveDate::from_ymd_opt(2015, 9, 29)
            .unwrap()
            .and_hms_opt(20, 0, 0)
            .unwrap();
        assert_eq!(cakedate(at, chrono_tz::UTC), "2015-09-29");
        assert_eq!(cakedate(at, chrono_tz::Asia::Kolkata), "2015-09-30");
    }

    #[test]
    fn dates_cast_as_active_model_does() {
        let d = |y, m, day| Some(NaiveDate::from_ymd_opt(y, m, day).unwrap());
        assert_eq!(cast_date(&json!("1904-09-28")).unwrap(), d(1904, 9, 28));
        // What the date of birth input sends.
        assert_eq!(cast_date(&json!("1904-9-5")).unwrap(), d(1904, 9, 5));
        assert_eq!(cast_date(&json!("")).unwrap(), None);
        assert_eq!(cast_date(&Value::Null).unwrap(), None);
        assert_eq!(cast_date(&json!("1904-02-30")).unwrap(), None);
        assert_eq!(cast_date(&json!("1904-13-1")).unwrap(), None);
    }
}
