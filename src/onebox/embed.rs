//! Onebox::Engine::StandardEmbed as an engine instance holds it: the page
//! (`html_doc`), its OpenGraph, Twitter and oEmbed data and `raw`, each
//! fetched or read once and kept, for the engines that include it.

use std::sync::LazyLock;

use serde_json::{Map, Value, json};

use super::engines::{Ctx, Fail};
use super::fetch::{Fetch, FetchError, FetchOpts};
use super::generic::{
    Kind, Normalizer, OEMBED_PROVIDERS, Page, active_support_truncate, oembed_data, read_page,
    set_from, squish, str_of,
};
use super::html_doc::fetch_html_doc;

pub struct Embed<'a> {
    /// The engine's `url`.
    pub url: String,
    pub ctx: &'a Ctx<'a>,
    /// The engine's own `get_oembed_url`, where it overrides it.
    pub oembed_url: Option<String>,
    html: Option<Option<String>>,
    page: Option<Page>,
    oembed: Option<Map<String, Value>>,
    raw: Option<Map<String, Value>>,
}

impl<'a> Embed<'a> {
    pub(super) fn new(url: &str, ctx: &'a Ctx<'a>) -> Embed<'a> {
        Embed {
            url: url.to_string(),
            ctx,
            oembed_url: None,
            html: None,
            page: None,
            oembed: None,
            raw: None,
        }
    }

    /// `html_doc`: the page's HTML, None for the empty document a failed
    /// fetch leaves.
    pub(super) async fn html_doc<F: Fetch>(
        &mut self,
        fetch: &mut F,
    ) -> Result<Option<String>, Fail> {
        if let Some(h) = &self.html {
            return Ok(h.clone());
        }
        let headers: Vec<(String, String)> = self
            .ctx
            .cookie
            .map(|c| vec![("Cookie".to_string(), c.to_string())])
            .unwrap_or_default();
        let html = fetch_html_doc(fetch, &self.url, &headers).await?;
        self.html = Some(html.clone());
        Ok(html)
    }

    async fn page<F: Fetch>(&mut self, fetch: &mut F) -> Result<&mut Page, Fail> {
        if self.page.is_none() {
            let html = self.html_doc(fetch).await?;
            self.page = Some(read_page(html.as_deref(), &self.url)?);
        }
        Ok(self.page.as_mut().unwrap())
    }

    /// `get_opengraph`
    pub(super) async fn opengraph<F: Fetch>(&mut self, fetch: &mut F) -> Result<Normalizer, Fail> {
        Ok(Normalizer {
            kind: Kind::OpenGraph,
            data: self.page(fetch).await?.og.clone(),
        })
    }

    /// `get_twitter`
    pub(super) async fn twitter<F: Fetch>(
        &mut self,
        fetch: &mut F,
    ) -> Result<Map<String, Value>, Fail> {
        Ok(self.page(fetch).await?.twitter.clone())
    }

    /// `get_favicon`
    pub(super) async fn favicon<F: Fetch>(
        &mut self,
        fetch: &mut F,
    ) -> Result<Option<String>, Fail> {
        Ok(self.page(fetch).await?.favicon.clone())
    }

    /// `get_oembed_url`
    async fn get_oembed_url<F: Fetch>(&mut self, fetch: &mut F) -> Result<Option<String>, Fail> {
        if let Some(u) = &self.oembed_url {
            return Ok(Some(u.clone()));
        }
        let mut url = OEMBED_PROVIDERS
            .iter()
            .find(|(r, _)| r.is_match(&self.url))
            .map(|(_, endpoint)| format!("{endpoint}?url={}", self.url));
        let links = self.page(fetch).await?.oembed_links.clone();
        if url.as_deref().is_none_or(crate::ruby::is_blank) {
            url = links.0;
        }
        if url.as_deref().is_none_or(crate::ruby::is_blank) {
            url = links.1;
        }
        Ok(url)
    }

    /// `get_oembed`: kept once it parses.
    pub(super) async fn oembed<F: Fetch>(&mut self, fetch: &mut F) -> Result<Normalizer, Fail> {
        if let Some(data) = &self.oembed {
            return Ok(Normalizer {
                kind: Kind::Oembed,
                data: data.clone(),
            });
        }
        let response = match self
            .get_oembed_url(fetch)
            .await?
            .filter(|u| !crate::ruby::is_blank(u))
        {
            None => "{}".to_string(),
            Some(u) => match fetch.fetch_response(&u, &FetchOpts::default()).await? {
                Ok(body) => body,
                Err(FetchError::Refused(e)) => return Err(e.into()),
                Err(_) => "{}".to_string(),
            },
        };
        let data = oembed_data(&response)?;
        self.oembed = Some(data.clone());
        Ok(Normalizer {
            kind: Kind::Oembed,
            data,
        })
    }

    /// `raw`: once begun it is kept, what was filled standing when it
    /// raised.
    pub(super) async fn raw<F: Fetch>(
        &mut self,
        fetch: &mut F,
    ) -> Result<Map<String, Value>, Fail> {
        if let Some(raw) = &self.raw {
            return Ok(raw.clone());
        }
        self.raw = Some(Map::new());
        let result = self.fill_raw(fetch).await;
        match result {
            Ok(()) => Ok(self.raw.clone().unwrap()),
            Err(e) => Err(e),
        }
    }

    /// The partly filled `raw` after it raised.
    pub(super) fn raw_so_far(&self) -> Map<String, Value> {
        self.raw.clone().unwrap_or_default()
    }

    async fn fill_raw<F: Fetch>(&mut self, fetch: &mut F) -> Result<(), Fail> {
        let og = self.opengraph(fetch).await?;
        set_from(self.raw.as_mut().unwrap(), &og, false)?;
        let raw = self.raw.as_mut().unwrap();
        raw.remove("title_attr");
        for (k, v) in self.twitter(fetch).await? {
            let raw = self.raw.as_mut().unwrap();
            if !raw.get(&k).is_some_and(|e| !e.is_null()) {
                raw.insert(k, v);
            }
        }
        let oembed = self.oembed(fetch).await?;
        let skip = oembed.data.get("type").and_then(Value::as_str) == Some("rich");
        set_from(self.raw.as_mut().unwrap(), &oembed, skip)?;
        let page = self.page(fetch).await?;
        let json_ld = std::mem::replace(&mut page.json_ld, Ok(Map::new()));
        let favicon = page.favicon.clone();
        let description = page.description.clone();
        let section_title = page.section_title.clone();
        let section_description = page.section_description.clone();
        let json_ld = json_ld?;
        let raw = self.raw.as_mut().unwrap();
        set_from(
            raw,
            &Normalizer {
                kind: Kind::JsonLd,
                data: json_ld,
            },
            false,
        )?;
        if let Some(f) = favicon.filter(|f| !crate::ruby::is_blank(f)) {
            raw.insert("favicon".into(), json!(f));
        }
        if !raw.get("description").is_some_and(|d| !d.is_null())
            && let Some(d) = description.filter(|d| !crate::ruby::is_blank(d))
        {
            raw.insert("description".into(), json!(d));
        }
        // enhance_title_with_anchor
        if let Some(title) = str_of(raw.get("title")).filter(|t| !crate::ruby::is_blank(t))
            && let Some(section) = section_title.filter(|s| !crate::ruby::is_blank(s))
        {
            static MARKS: LazyLock<regex::Regex> =
                LazyLock::new(|| regex::Regex::new("[\u{00B6}\u{00A7}#]").unwrap());
            let cleaned =
                active_support_truncate(&squish(&MARKS.replace_all(&section, "")), 80, "...");
            if !crate::ruby::is_blank(&cleaned) && !title.contains(&cleaned) {
                let enhanced = format!("{cleaned} - {title}");
                raw.insert("title".into(), json!(enhanced));
            }
        }
        // enhance_description_with_anchor
        if let Some(section) = section_description.filter(|s| !crate::ruby::is_blank(s)) {
            let cleaned = active_support_truncate(&squish(&section), 300, "\u{2026}");
            let existing = str_of(raw.get("description"))
                .filter(|d| !crate::ruby::is_blank(d))
                .map(str::to_string);
            if !crate::ruby::is_blank(&cleaned) {
                match existing {
                    Some(d) if d.contains(&cleaned) => {}
                    Some(d) => {
                        raw.insert("description".into(), json!(format!("{cleaned} | {d}")));
                    }
                    None => {
                        raw.insert("description".into(), json!(cleaned));
                    }
                }
            }
        }
        Ok(())
    }
}
