//! The guidelines (FAQ), terms of service and privacy pages: what
//! app/views/static/show.html.erb renders for StaticController#show, byte
//! for byte, since the XHR answer is that fragment alone (layout: false)
//! and the Ember routes (faq.js, guidelines.js, conduct.js, rules.js,
//! tos.js, privacy.js) put it inside templates/faq.gjs or tos.gjs's
//! `.body-page` as it comes (StaticPage.find, `trustHTML`).

use crate::email::notification::html_escape as h;

/// What show.html.erb reads.
pub struct StaticPage<'a> {
    pub base_path: &'a str,
    /// `@page` after the guidelines alias: faq, tos or privacy.
    pub page: &'a str,
    /// `@topic.first_post.updated_at`, as an ISO 8601 date.
    pub last_reviewed: &'a str,
    /// `SiteSetting.login_required? && current_user.nil?`: no navigation
    /// to the about and guidelines pages.
    pub login_wall: bool,
    /// SiteSetting.faq_url, empty when unset (`@faq_overridden` when not):
    /// `faq_path` links there.
    pub faq_url: &'a str,
    /// `@rename_faq_to_guidelines`
    pub rename_faq_to_guidelines: bool,
    /// The `tos_url` and `privacy_policy_url` helpers (Discourse.tos_url).
    pub tos_url: Option<&'a str>,
    pub privacy_policy_url: Option<&'a str>,
    /// `staff?`: the Edit this page link to `@topic.url`.
    pub edit_url: Option<&'a str>,
    /// `@body`, the first post's cooked HTML.
    pub body: &'a str,
    /// I18n: js.about.simple_title, js.guidelines, js.faq, js.tos,
    /// js.privacy and edit_this_page.
    pub labels: Labels<'a>,
}

pub struct Labels<'a> {
    pub about: &'a str,
    pub guidelines: &'a str,
    pub faq: &'a str,
    pub tos: &'a str,
    pub privacy: &'a str,
    pub edit_this_page: &'a str,
}

/// show.html.erb with ERB's trim mode: lines holding only `<% %>` tags
/// leave nothing; the `content_for` line at the end leaves its indent.
pub fn fragment(p: &StaticPage) -> String {
    let base = p.base_path;
    let active = |page: &str| if p.page == page { "active" } else { "" };
    let mut out = String::new();
    out.push_str("<div itemscope itemtype='http://schema.org/WebPage'>\n");
    out.push_str(&format!(
        "  <meta itemprop='lastReviewed' content='{}'>\n",
        h(p.last_reviewed)
    ));
    out.push_str("  <ul class='nav-pills' role='navigation' itemscope itemtype='http://schema.org/SiteNavigationElement'>\n");
    if !p.login_wall {
        // link_to's double quotes, and the template's stray </a>.
        out.push_str(&format!(
            "      <li class=\"nav-item-about\"><a href=\"{}\">{}</a></a></li>\n",
            h(&format!("{base}/about")),
            h(p.labels.about)
        ));
        let guidelines = format!(
            "        <li class='nav-item-guidelines'><a class='{}' href='{}'>{}</a></li>\n",
            active("faq"),
            h(&format!("{base}/guidelines")),
            h(p.labels.guidelines)
        );
        if !p.faq_url.is_empty() {
            out.push_str(&guidelines);
            out.push_str(&format!(
                "        <li class='nav-item-faq'><a href='{}'>{}</a></li>\n",
                h(p.faq_url),
                h(p.labels.faq)
            ));
        } else if p.rename_faq_to_guidelines {
            out.push_str(&guidelines);
        } else {
            out.push_str(&format!(
                "        <li class='nav-item-faq'><a class='{}' href='{}'>{}</a></li>\n",
                active("faq"),
                h(&format!("{base}/faq")),
                h(p.labels.faq)
            ));
        }
    }
    if let Some(url) = p.tos_url {
        out.push_str(&format!(
            "      <li class='nav-item-tos'><a href='{}' class='{}'>{}</a></li>\n",
            h(url),
            active("tos"),
            h(p.labels.tos)
        ));
    }
    if let Some(url) = p.privacy_policy_url {
        out.push_str(&format!(
            "      <li class='nav-item-privacy'><a href='{}' class='{}'>{}</a></li>\n",
            h(url),
            active("privacy"),
            h(p.labels.privacy)
        ));
    }
    out.push_str("  </ul>\n\n");
    if let Some(url) = p.edit_url {
        out.push_str(&format!(
            "    <p><a href='{}'>\n      <svg class=\"fa d-icon d-icon-pencil svg-icon\" xmlns=\"http://www.w3.org/2000/svg\">\n        <use href=\"#pencil\"></use>\n      </svg>\n      {}\n    </a></p>\n",
            h(url),
            h(p.labels.edit_this_page)
        ));
    }
    out.push_str(&format!(
        "\n  <div itemprop='mainContentOfPage'>\n    {}\n  </div>\n</div>\n\n  \n",
        p.body
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page<'a>() -> StaticPage<'a> {
        StaticPage {
            base_path: "",
            page: "faq",
            last_reviewed: "2026-07-25",
            login_wall: false,
            faq_url: "",
            rename_faq_to_guidelines: true,
            tos_url: None,
            privacy_policy_url: None,
            edit_url: None,
            body: "<p>Hi</p>",
            labels: Labels {
                about: "About",
                guidelines: "Guidelines",
                faq: "FAQ",
                tos: "Terms of Service",
                privacy: "Privacy",
                edit_this_page: "Edit this page",
            },
        }
    }

    #[test]
    fn guidelines_as_rails_renders_them() {
        assert_eq!(
            fragment(&page()),
            "<div itemscope itemtype='http://schema.org/WebPage'>\n  <meta itemprop='lastReviewed' content='2026-07-25'>\n  <ul class='nav-pills' role='navigation' itemscope itemtype='http://schema.org/SiteNavigationElement'>\n      <li class=\"nav-item-about\"><a href=\"/about\">About</a></a></li>\n        <li class='nav-item-guidelines'><a class='active' href='/guidelines'>Guidelines</a></li>\n  </ul>\n\n\n  <div itemprop='mainContentOfPage'>\n    <p>Hi</p>\n  </div>\n</div>\n\n  \n"
        );
    }

    #[test]
    fn login_wall_hides_the_site_navigation_but_not_legal_links() {
        let mut p = page();
        p.login_wall = true;
        p.page = "tos";
        p.tos_url = Some("/tos");
        let html = fragment(&p);
        assert!(!html.contains("nav-item-about"));
        assert!(!html.contains("nav-item-guidelines"));
        assert!(html.contains("<li class='nav-item-tos'><a href='/tos' class='active'>"));
    }

    #[test]
    fn overridden_faq_links_both() {
        let mut p = page();
        p.faq_url = "https://example.com/faq?a=1&b=2";
        let html = fragment(&p);
        assert!(html.contains("nav-item-guidelines'><a class='active' href='/guidelines'>"));
        assert!(html.contains(
            "<li class='nav-item-faq'><a href='https://example.com/faq?a=1&amp;b=2'>FAQ</a></li>"
        ));
    }
}
