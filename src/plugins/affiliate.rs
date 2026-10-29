//! discourse-affiliate: `on(:post_process_cooked)`, AffiliateProcessor's
//! rewrite of each cooked link to an Amazon store or ldlc.com so that it
//! carries the site's affiliate code (lib/affiliate_processor.rb). The
//! post processor calls [`post_process_cooked`] on the document as it
//! finishes (crate::pretty_text::cooked_post_processor).

use markup5ever_rcdom::RcDom;

use crate::pretty_text::cleanup::{all_elements, attr, element_name, set_attr};
use crate::site_settings::{SettingError, SiteSettings};

/// `enabled_site_setting :affiliate_enabled`
pub fn enabled(settings: &SiteSettings) -> Result<bool, SettingError> {
    Ok(settings.get("affiliate_enabled")?.truthy())
}

/// What a host's rule does with a url.
enum Rule {
    /// `create_amazon_rule(domain)`: the code of `affiliate_amazon_<domain>`
    /// with its dots as underscores.
    Amazon(String),
    /// ldlc's rule: the code of `affiliate_ldlc_com` as the fragment.
    Ldlc,
}

/// `AffiliateProcessor.rules`: the rule for a host, as Ruby's URI reads
/// it (not downcased).
fn rule(host: &str) -> Option<Rule> {
    const POSTFIXES: [&str; 17] = [
        "com", "com.au", "com.br", "com.mx", "ca", "cn", "co.jp", "co.uk", "de", "es", "fr", "in",
        "it", "nl", "to", "co", "eu",
    ];
    match host {
        "amzn.com" | "amzn.to" | "a.co" => return Some(Rule::Amazon("com".into())),
        "amzn.eu" => return Some(Rule::Amazon("eu".into())),
        "www.ldlc.com" | "ldlc.com" => return Some(Rule::Ldlc),
        _ => {}
    }
    let postfix = host
        .strip_prefix("www.amazon.")
        .or_else(|| host.strip_prefix("smile.amazon."))
        .or_else(|| host.strip_prefix("amazon."))?;
    POSTFIXES
        .contains(&postfix)
        .then(|| Rule::Amazon(postfix.to_string()))
}

/// `URI.decode_www_form_component`; None where Ruby raises (a `%` not
/// followed by two hex digits).
fn decode_component(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' => {
                let hex = s.get(i + 1..i + 3)?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 2;
            }
            b => out.push(b),
        }
        i += 1;
    }
    Some(String::from_utf8_lossy(&out).into_owned())
}

/// `URI.encode_www_form_component`: everything but `*-._` and
/// alphanumerics escaped, a space as `+`.
fn encode_component(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b' ' => out.push('+'),
            b'*' | b'-' | b'.' | b'_' | b'0'..=b'9' | b'A'..=b'Z' | b'a'..=b'z' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// `URI.decode_www_form(query).to_h`'s value for a key (the last pair
/// with it), or None where decoding raises.
fn query_values(query: &str) -> Option<Vec<(String, String)>> {
    let mut pairs = Vec::new();
    for part in query.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = part.split_once('=').unwrap_or((part, ""));
        pairs.push((decode_component(k)?, decode_component(v)?));
    }
    Some(pairs)
}

/// `AffiliateProcessor.apply(url)`, `code` giving a setting's value (None
/// for a setting that does not exist, where `SiteSetting.get` raises and
/// the url is kept).
pub fn apply(url: &str, code: &dyn Fn(&str) -> Option<String>) -> String {
    apply_rule(url, code).unwrap_or_else(|| url.to_string())
}

fn apply_rule(url: &str, code: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    let mut uri = crate::wildcard_url::parse(url)?;
    let scheme = uri.scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let rule = rule(uri.host.as_deref()?)?;
    match rule {
        Rule::Amazon(domain) => {
            let code = code(&format!("affiliate_amazon_{}", domain.replace('.', "_")))?;
            if code.trim().is_empty() {
                return None;
            }
            let pairs = query_values(uri.query.as_deref().unwrap_or(""))?;
            let last = |key: &str| {
                pairs
                    .iter()
                    .rev()
                    .find(|(k, _)| k == key)
                    .map(|(_, v)| v.clone())
                    .filter(|v| !v.trim().is_empty())
            };
            let mut query = vec![("tag", code)];
            // A search's `k`, and its `ref` along with it.
            if let Some(k) = last("k") {
                query.push(("k", k));
                if let Some(r) = last("ref") {
                    query.push(("ref", r));
                }
            }
            if let Some(node) = last("node") {
                query.push(("node", node));
            }
            uri.query = Some(
                query
                    .iter()
                    .map(|(k, v)| format!("{}={}", encode_component(k), encode_component(v)))
                    .collect::<Vec<_>>()
                    .join("&"),
            );
        }
        Rule::Ldlc => {
            let code = code("affiliate_ldlc_com")?;
            if code.trim().is_empty() {
                return None;
            }
            uri.fragment = Some(code);
        }
    }
    Some(to_s(&uri, &scheme))
}

/// `URI::Generic#to_s`: the default port left out.
fn to_s(uri: &crate::wildcard_url::Uri, scheme: &str) -> String {
    let mut out = format!("{scheme}:");
    if let Some(host) = &uri.host {
        out.push_str("//");
        if let Some(userinfo) = &uri.userinfo {
            out.push_str(userinfo);
            out.push('@');
        }
        out.push_str(host);
        let default = if scheme == "https" { 443 } else { 80 };
        if uri.explicit_port
            && let Some(port) = uri.port.filter(|p| *p != default)
        {
            out.push_str(&format!(":{port}"));
        }
    }
    out.push_str(&uri.path);
    if let Some(query) = &uri.query {
        out.push('?');
        out.push_str(query);
    }
    if let Some(fragment) = &uri.fragment {
        out.push('#');
        out.push_str(fragment);
    }
    out
}

/// `AffiliateProcessor.apply_to_doc(doc)`, while the plugin is on (its
/// `on` handler).
pub fn post_process_cooked(dom: &RcDom, settings: &SiteSettings) -> Result<(), SettingError> {
    if !enabled(settings)? {
        return Ok(());
    }
    let code = |name: &str| settings.get(name).ok().map(|v| v.to_s().to_string());
    for a in all_elements(dom) {
        if element_name(&a) != Some("a") {
            continue;
        }
        if let Some(href) = attr(&a, "href") {
            let rewritten = apply(&href, &code);
            if rewritten != href {
                set_attr(&a, "href", &rewritten);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// spec/lib/affiliate_processor_spec.rb's settings: the codes set,
    /// every other existing setting blank, unknown ones missing.
    fn code(name: &str) -> Option<String> {
        let set = [
            ("affiliate_ldlc_com", "samsshop"),
            ("affiliate_amazon_com", "sams-shop"),
            ("affiliate_amazon_ca", "ca-sams-shop"),
            ("affiliate_amazon_com_au", "au-sams-shop"),
            ("affiliate_amazon_eu", "eu-sams-shop"),
        ];
        if let Some((_, v)) = set.iter().find(|(k, _)| *k == name) {
            return Some(v.to_string());
        }
        let exists = [
            "affiliate_amazon_cn",
            "affiliate_amazon_co_jp",
            "affiliate_amazon_co_uk",
            "affiliate_amazon_com_br",
            "affiliate_amazon_com_mx",
            "affiliate_amazon_de",
            "affiliate_amazon_es",
            "affiliate_amazon_fr",
            "affiliate_amazon_in",
            "affiliate_amazon_it",
            "affiliate_amazon_nl",
        ];
        exists.contains(&name).then(String::new)
    }

    fn r(url: &str) -> String {
        apply(url, &code)
    }

    #[test]
    fn applies_the_code_to_ldlc() {
        assert_eq!(
            r("http://www.ldlc.com/some_product?xyz=1"),
            "http://www.ldlc.com/some_product?xyz=1#samsshop"
        );
        assert_eq!(
            r("https://ldlc.com/some_product?xyz=1"),
            "https://ldlc.com/some_product?xyz=1#samsshop"
        );
    }

    #[test]
    fn applies_the_code_to_amazon() {
        let cases = [
            (
                "https://www.amazon.com",
                "https://www.amazon.com?tag=sams-shop",
            ),
            (
                "http://www.amazon.com/some_product?xyz=1",
                "http://www.amazon.com/some_product?tag=sams-shop",
            ),
            (
                "https://www.amazon.com/some_product?xyz=1",
                "https://www.amazon.com/some_product?tag=sams-shop",
            ),
            (
                "https://www.amazon.com?hello=1&tag=bobs-shop",
                "https://www.amazon.com?tag=sams-shop",
            ),
            (
                "https://amzn.com/some_product?xyz=1",
                "https://amzn.com/some_product?tag=sams-shop",
            ),
            (
                "https://smile.amazon.com/some_product?xyz=1",
                "https://smile.amazon.com/some_product?tag=sams-shop",
            ),
            (
                "https://www.amazon.com.au/some_product?xyz=1",
                "https://www.amazon.com.au/some_product?tag=au-sams-shop",
            ),
            (
                "https://www.amazon.ca/Dragon-Quest-Echoes-Elusive-Age-PlayStation/dp/B07BP3J6RG/ref=br_asw_pdt-5?pf_rd_m=ATVPDKIKX0DER&pf_rd_s=&pf_rd_r=XFGPRSG0SVD5K3RKX5T3&pf_rd_t=36701&pf_rd_p=f8585743-c043-4665-80a7-0cc5fe97d596&pf_rd_i=desktop&th=1",
                "https://www.amazon.ca/Dragon-Quest-Echoes-Elusive-Age-PlayStation/dp/B07BP3J6RG/ref=br_asw_pdt-5?tag=ca-sams-shop",
            ),
            (
                "https://amzn.to/d/some_short_link",
                "https://amzn.to/d/some_short_link?tag=sams-shop",
            ),
            (
                "https://amzn.eu/d/some_short_link",
                "https://amzn.eu/d/some_short_link?tag=eu-sams-shop",
            ),
            (
                "https://a.co/some_short_link",
                "https://a.co/some_short_link?tag=sams-shop",
            ),
            // keep node (BrowseNodeSearch) query parameter
            (
                "https://www.amazon.com/b?ie=UTF8&node=13548845011",
                "https://www.amazon.com/b?tag=sams-shop&node=13548845011",
            ),
        ];
        for (url, want) in cases {
            assert_eq!(r(url), want, "{url}");
        }
    }

    #[test]
    fn searches_keep_their_keywords_and_ref() {
        assert_eq!(
            r("https://www.amazon.com/s?k=usb%20c+hub&ref=nb_sb_noss&x=1"),
            "https://www.amazon.com/s?tag=sams-shop&k=usb+c+hub&ref=nb_sb_noss"
        );
        assert_eq!(
            r("https://www.amazon.com/s?ref=nb&x=1"),
            "https://www.amazon.com/s?tag=sams-shop",
            "ref only with k"
        );
    }

    #[test]
    fn other_links_are_left_alone() {
        for url in [
            // no code set for the store
            "https://www.amazon.de/dp/B0C3WGSSWC",
            // the rules for amazon.to and amazon.co read settings that do
            // not exist, which raises and keeps the url
            "https://www.amazon.to/x",
            "https://www.amazon.co/x",
            // Ruby's URI keeps the host's case
            "https://WWW.AMAZON.COM/x",
            // a space URI.parse refuses
            "https://www.amazon.com/s?k=usb c",
            "ftp://www.amazon.com/x",
            "/t/topic/1",
            "https://example.com/?tag=x",
            "https://www.amazon.com/x?bad=%zz",
        ] {
            assert_eq!(r(url), url);
        }
        assert_eq!(
            r("https://www.amazon.com:443/x"),
            "https://www.amazon.com/x?tag=sams-shop",
            "to_s leaves the default port out"
        );
    }
}
