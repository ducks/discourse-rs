//! `CertificateGenerator`: the SVG certificates the tutorials end with,
//! from the plugin's ERB templates (vendored), and the plugin's public
//! files (`/plugins/discourse-narrative-bot/...`: the images its messages
//! show, the certificates' font).

use chrono::NaiveDate;

use crate::AppError;
use crate::url::Urls;

const NEW_USER_SVG: &str = include_str!(
    "../../../vendor/discourse/plugins/discourse-narrative-bot/lib/discourse_narrative_bot/templates/new_user.svg.erb"
);
const ADVANCED_USER_SVG: &str = include_str!(
    "../../../vendor/discourse/plugins/discourse-narrative-bot/lib/discourse_narrative_bot/templates/advanced_user.svg.erb"
);

/// The certificate's inputs (`assign_options`).
pub struct Certificate<'a> {
    /// `@user.username.titleize`
    pub name: String,
    /// The date, `I18n.l(date, format: :date_only)`.
    pub date: String,
    pub avatar_url: String,
    pub discobot_username: String,
    /// `SiteSetting.site_logo_small_url`, absolute; None when blank.
    pub logo_url: Option<String>,
    pub i18n: crate::i18n::Localized<'a>,
    pub font_url: String,
}

/// `Date.parse(date)`, or today when the date is invalid. The forms the
/// plugin's own links carry (`%b %d %Y`) and ISO dates are read; Ruby's
/// parser reads many more.
pub fn parse_date(date: &str, today: NaiveDate) -> NaiveDate {
    let date = date.trim();
    for format in [
        "%b %d %Y",
        "%B %d %Y",
        "%Y-%m-%d",
        "%d %b %Y",
        "%b %d, %Y",
        "%B %d, %Y",
    ] {
        if let Ok(parsed) = NaiveDate::parse_from_str(date, format) {
            return parsed;
        }
    }
    today
}

/// `I18n.l(date, format: :date_only)` in the locale.
pub fn localize_date(i18n: &crate::i18n::Localized<'_>, date: NaiveDate) -> String {
    use chrono::Datelike;
    let format = i18n.t("date.formats.date_only").unwrap_or("%B %-d, %Y");
    let month = i18n
        .list("date.month_names")
        .and_then(|names| names.get(date.month() as usize).cloned().flatten())
        .unwrap_or_else(|| date.format("%B").to_string());
    let abbr = i18n
        .list("date.abbr_month_names")
        .and_then(|names| names.get(date.month() as usize).cloned().flatten())
        .unwrap_or_else(|| date.format("%b").to_string());
    let format = format.replace("%B", &month).replace("%b", &abbr);
    date.format(&format).to_string()
}

/// `String#titleize` for a username: words split at underscores, dashes
/// and case changes, each capitalized, joined by spaces.
pub fn titleize(s: &str) -> String {
    // underscore: CamelCase to snake_case, dashes to underscores.
    let mut snake = String::new();
    let chars: Vec<char> = s.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if c == '-' {
            snake.push('_');
        } else if c.is_uppercase() {
            let prev = i.checked_sub(1).map(|j| chars[j]);
            let next = chars.get(i + 1);
            let boundary = prev.is_some_and(|p| p.is_lowercase() || p.is_ascii_digit())
                || (prev.is_some_and(char::is_uppercase) && next.is_some_and(|n| n.is_lowercase()));
            if boundary {
                snake.push('_');
            }
            snake.extend(c.to_lowercase());
        } else {
            snake.push(c);
        }
    }
    // humanize: underscores to spaces, a trailing _id dropped.
    let human = snake
        .strip_suffix("_id")
        .unwrap_or(&snake)
        .replace('_', " ");
    // titleize: each word's first letter up.
    let mut out = String::new();
    let mut start = true;
    for c in human.chars() {
        if start && c.is_alphabetic() {
            out.extend(c.to_uppercase());
        } else {
            out.push(c);
        }
        start = !(c.is_alphanumeric() || c == '\'');
    }
    out.trim().to_string()
}

/// `ERB::Util.html_escape`, as `<%= %>` writes values.
fn h(s: &str) -> String {
    super::narrative::escape_html(s)
}

/// Ruby's `Float#to_s`.
fn float(f: f64) -> String {
    if f.fract() == 0.0 {
        format!("{f:.1}")
    } else {
        format!("{f}")
    }
}

/// The track's certificate (`new_user_track` or `advanced_user_track`).
pub fn render(advanced: bool, c: &Certificate<'_>) -> String {
    let (template, width, logo_size, logo_height, avatar_shift, track) = if advanced {
        (
            ADVANCED_USER_SVG,
            722.8,
            40,
            350,
            14.0,
            "advanced_user_narrative",
        )
    } else {
        (NEW_USER_SVG, 538.583, 55, 280, 12.5, "new_user_narrative")
    };
    let t = |key: &str| {
        c.i18n
            .t(&format!("discourse_narrative_bot.{track}.{key}"))
            .unwrap_or_default()
            .to_string()
    };
    let mut out = without_logo_block(template, c.logo_url.is_some());
    let replacements = [
        ("<%= @width %>".to_string(), float(width)),
        (
            "<%= GlobalPath.full_cdn_url(\"/plugins/discourse-narrative-bot/fonts/GreatVibes-Regular.ttf\") %>"
                .to_string(),
            h(&c.font_url),
        ),
        (
            format!("<%= I18n.t('discourse_narrative_bot.{track}.cert_title') %>"),
            h(&t("cert_title")),
        ),
        (
            format!("<%= I18n.t('discourse_narrative_bot.{track}.title') %>"),
            h(&t("title")),
        ),
        ("<%= @date %>".to_string(), h(&c.date)),
        (
            format!("<%= @width / 2 - {} %>", float(avatar_shift).trim_end_matches(".0")),
            float(width / 2.0 - avatar_shift),
        ),
        ("<%= @width / 2 %>".to_string(), float(width / 2.0)),
        ("<%= @avatar_url %>".to_string(), h(&c.avatar_url)),
        (
            "<%= @discobot_user.username %>".to_string(),
            h(&c.discobot_username),
        ),
        ("<%= @name %>".to_string(), h(&c.name)),
        (
            "<%= @logo_group[:width] / 2 - (@logo_group[:size] / 2) %>".to_string(),
            float(width / 2.0 - f64::from(logo_size / 2)),
        ),
        ("<%= @logo_group[:height] %>".to_string(), logo_height.to_string()),
        ("<%= @logo_group[:size] %>".to_string(), logo_size.to_string()),
        (
            "<%= @logo_group[:logo_uri] %>".to_string(),
            h(c.logo_url.as_deref().unwrap_or_default()),
        ),
    ];
    for (tag, value) in replacements {
        out = out.replace(&tag, &value);
    }
    out
}

/// The `<% if @logo_group.present? %>` block, kept or dropped, its tag
/// lines trimmed as Erubi's trim mode does.
fn without_logo_block(template: &str, keep: bool) -> String {
    let open = "<% if @logo_group.present? %>";
    let close = "<% end %>";
    let line_start = |at: usize| template[..at].rfind('\n').map_or(0, |i| i + 1);
    let (Some(o), Some(e)) = (template.find(open), template.find(close)) else {
        return template.to_string();
    };
    let open_start = line_start(o);
    let open_end = o + open.len() + usize::from(template[o + open.len()..].starts_with('\n'));
    let close_start = line_start(e);
    let close_end = e + close.len() + usize::from(template[e + close.len()..].starts_with('\n'));
    let mut out = String::with_capacity(template.len());
    out.push_str(&template[..open_start]);
    if keep {
        out.push_str(&template[open_end..close_start]);
    }
    out.push_str(&template[close_end..]);
    out
}

/// `logo_group`'s logo: the small logo's url, a relative one joined to
/// the base url as `"#{Discourse.base_url}/#{uri.path}"` does.
pub fn logo_uri(urls: &Urls<'_>, site_logo_small_url: &str) -> Result<Option<String>, AppError> {
    if site_logo_small_url.trim().is_empty() {
        return Ok(None);
    }
    let absolute =
        site_logo_small_url.starts_with("http://") || site_logo_small_url.starts_with("https://");
    if absolute {
        return Ok(Some(site_logo_small_url.to_string()));
    }
    let path = site_logo_small_url
        .split(['?', '#'])
        .next()
        .unwrap_or_default();
    let path = match path.strip_prefix("//") {
        // A protocol-relative url's path follows its host.
        Some(rest) => rest.find('/').map_or("", |i| &rest[i..]).to_string(),
        None => path.to_string(),
    };
    Ok(Some(format!("{}/{path}", urls.base_url()?)))
}

/// A public file of the plugin: its bytes and content type.
pub fn public_file(path: &str) -> Option<(&'static [u8], &'static str)> {
    macro_rules! file {
        ($name:literal, $type:literal) => {
            (
                $name,
                include_bytes!(concat!(
                    "../../../vendor/discourse/plugins/discourse-narrative-bot/public/",
                    $name
                ))
                .as_slice(),
                $type,
            )
        };
    }
    let files: [(&str, &'static [u8], &'static str); 16] = [
        file!("fonts/GreatVibes-Regular.ttf", "font/ttf"),
        file!("fonts/OFL.txt", "text/plain; charset=utf-8"),
        file!("images/capybara-eating.gif", "image/gif"),
        file!("images/dog-walk.gif", "image/gif"),
        file!("images/font-awesome-bookmark.png", "image/png"),
        file!("images/font-awesome-ellipsis.png", "image/png"),
        file!("images/font-awesome-flag.png", "image/png"),
        file!("images/font-awesome-gear.png", "image/png"),
        file!("images/font-awesome-link.png", "image/png"),
        file!("images/font-awesome-pencil.png", "image/png"),
        file!("images/font-awesome-plus.png", "image/png"),
        file!("images/font-awesome-rotate-left.png", "image/png"),
        file!("images/font-awesome-search.png", "image/png"),
        file!("images/font-awesome-smile.png", "image/png"),
        file!("images/font-awesome-trash.png", "image/png"),
        file!("images/unicorn.png", "image/png"),
    ];
    files
        .iter()
        .find(|(name, _, _)| *name == path)
        .map(|(_, bytes, kind)| (*bytes, *kind))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titleizes_usernames() {
        assert_eq!(titleize("jeff_atwood"), "Jeff Atwood");
        assert_eq!(titleize("bruceLee"), "Bruce Lee");
        assert_eq!(titleize("user1"), "User1");
        assert_eq!(titleize("some-name"), "Some Name");
    }

    #[test]
    fn invalid_dates_are_today() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 28).unwrap();
        assert_eq!(parse_date("2017-00-10", today), today);
        assert_eq!(
            parse_date("Mar 05 2017", today),
            NaiveDate::from_ymd_opt(2017, 3, 5).unwrap()
        );
    }

    #[test]
    fn the_logo_block_is_trimmed() {
        let t = "a\n    <% if @logo_group.present? %>\n      logo\n    <% end %>\nb";
        assert_eq!(without_logo_block(t, true), "a\n      logo\nb");
        assert_eq!(without_logo_block(t, false), "a\nb");
    }

    #[test]
    fn floats_print_as_ruby_does() {
        assert_eq!(float(722.8), "722.8");
        assert_eq!(float(361.4), "361.4");
        assert_eq!(float(538.583 / 2.0 - 12.5), "256.7915");
        assert_eq!(float(2.0), "2.0");
    }
}
