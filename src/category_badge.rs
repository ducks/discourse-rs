//! `CategoryBadge.html_for` with the browser styles: the server-rendered
//! badge (single-quoted attributes, unlike the client's), for pages Rails
//! renders in ERB such as the not-found page.

use sqlx::PgConnection;

use crate::AppError;
use crate::category::Category;
use crate::site_settings::SiteSettings;

/// `ERB::Util.html_escape`
pub fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// `CategoryBadge.html_for(category)`: "" for Uncategorized.
pub async fn html_for(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    base_path: &str,
    category_id: i32,
) -> Result<String, AppError> {
    if i64::from(category_id) == settings.get("uncategorized_category_id")?.to_i() {
        return Ok(String::new());
    }
    #[derive(sqlx::FromRow)]
    struct Row {
        id: i32,
        name: String,
        color: String,
        text_color: String,
        style_type: i32,
        icon: Option<String>,
        emoji: Option<String>,
        parent_category_id: Option<i32>,
    }
    const SQL: &str = "SELECT id, name, color, text_color, style_type, icon, emoji, parent_category_id \
                       FROM categories WHERE id = $1";
    let Some(category) = sqlx::query_as::<_, Row>(SQL)
        .bind(category_id)
        .fetch_optional(&mut *conn)
        .await?
    else {
        return Ok(String::new());
    };
    let parent = match category.parent_category_id {
        Some(id) => {
            sqlx::query_as::<_, Row>(SQL)
                .bind(id)
                .fetch_optional(&mut *conn)
                .await?
        }
        None => None,
    };
    let url = Category::find(&mut *conn, category.id)
        .await?
        .ok_or(crate::Unsupported("a category that vanished"))?
        .url(&mut *conn, base_path)
        .await?;

    // style_type: square 0, icon 1, emoji 2
    let style_class = match category.style_type {
        1 => "--style-icon",
        2 => "--style-emoji",
        _ => "--style-square",
    };
    let has_parent = if parent.is_some() { "--has-parent" } else { "" };
    let mut styles = format!(
        "--category-badge-color: #{}; --category-badge-text-color: #{};",
        html_escape(&category.color),
        html_escape(&category.text_color)
    );
    if let Some(p) = &parent {
        styles.push_str(&format!(
            " --parent-category-badge-color: #{};",
            html_escape(&p.color)
        ));
    }
    let mut out = format!("<span data-category-id='{}' style='{styles}'", category.id);
    if let Some(p) = &parent {
        out.push_str(&format!(" data-parent-category-id='{}'", p.id));
    }
    out.push_str(&format!(
        " data-drop-close='true' class='badge-category {style_class} {has_parent}'>"
    ));
    match (category.style_type, &category.icon, &category.emoji) {
        (1, Some(icon), _) if !icon.is_empty() => out.push_str(&crate::svg_sprite::raw_svg(icon)?),
        (2, _, Some(emoji)) if !emoji.is_empty() => {
            out.push_str(&codes_to_img(settings, base_path, &format!(":{emoji}:"))?)
        }
        _ => {}
    }
    out.push_str("<span class='badge-category__name'>");
    out.push_str(&html_escape(&category.name));
    out.push_str("</span></span>");
    Ok(format!(
        "<a class='badge-category__wrapper ' href='{url}'>{out}</a>"
    ))
}

/// `ERB::Util.html_escape_once`: like html_escape, but an entity already
/// there is left alone.
pub fn html_escape_once(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for (i, c) in s.char_indices() {
        match c {
            '&' if is_entity(&s[i + 1..]) => out.push('&'),
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// `([a-zA-Z]+|(#\d+)|(#[xX][\dA-Fa-f]+));` at the start.
fn is_entity(rest: &str) -> bool {
    let body: &str = match rest.find(';') {
        Some(i) => &rest[..i],
        None => return false,
    };
    if let Some(num) = body.strip_prefix('#') {
        if let Some(hex) = num.strip_prefix('x').or_else(|| num.strip_prefix('X')) {
            return !hex.is_empty() && hex.chars().all(|c| c.is_ascii_hexdigit());
        }
        return !num.is_empty() && num.chars().all(|c| c.is_ascii_digit());
    }
    !body.is_empty() && body.chars().all(|c| c.is_ascii_alphabetic())
}

/// `Emoji.codes_to_img`: each `:code:` of a known emoji as its image, the
/// text between escaped once. Custom emojis are not consulted: callers
/// refuse sites that have them.
pub fn codes_to_img(settings: &SiteSettings, base_path: &str, s: &str) -> Result<String, AppError> {
    static CODE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let code_re = CODE.get_or_init(|| regex::Regex::new(r":([\w\-+]+(?::t\d)?):").unwrap());
    let set = settings.get("emoji_set")?.to_s();
    let mut out = String::new();
    let mut last = 0;
    for m in code_re.captures_iter(s) {
        let whole = m.get(0).expect("match");
        let code = &m[1];
        out.push_str(&html_escape_once(&s[last..whole.start()]));
        let (name, tone) = match code.split_once(":t") {
            Some((name, tone)) => (name, Some(tone)),
            None => (code, None),
        };
        if crate::emoji::DATA.exists(name) {
            let path = match tone {
                Some(t) => format!("{name}/{t}"),
                None => name.to_string(),
            };
            let url = format!(
                "{base_path}/images/emoji/{set}/{path}.png?v={}",
                crate::emoji::image_version()
            );
            let code = html_escape(code);
            out.push_str(&format!(
                "<img src=\"{}\" title=\"{code}\" class=\"emoji\" alt=\"{code}\" loading=\"lazy\" width=\"20\" height=\"20\">",
                html_escape(&url)
            ));
        } else {
            out.push_str(&html_escape_once(whole.as_str()));
        }
        last = whole.end();
    }
    out.push_str(&html_escape_once(&s[last..]));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_once_keeps_entities() {
        assert_eq!(
            html_escape_once("Tom &amp; Jerry & <b> &#39; &#x2F; &bogus"),
            "Tom &amp; Jerry &amp; &lt;b&gt; &#39; &#x2F; &amp;bogus"
        );
    }
}
