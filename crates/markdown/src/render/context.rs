//! What a cook looks up outside the text: avatars and groups of quoted
//! users, quoted topics, hashtags, uploads. The rules cannot wait on the
//! database, so a cook renders twice: the first pass notes what it would
//! have looked up (`Needs`), the caller resolves that, and the second
//! pass renders with the answers (`Lookups`).

use std::collections::HashMap;
use std::sync::Mutex;

use markdown_it::parser::extset::MarkdownItExt;

/// `get_topic_info`'s answer.
#[derive(Debug, Clone)]
pub struct TopicInfo {
    pub title: String,
    pub href: String,
}

/// `hashtag_lookup`'s answer.
#[derive(Debug, Clone)]
pub struct Hashtag {
    pub relative_url: String,
    pub text: String,
    pub kind: String,
    pub slug: String,
    pub reference: String,
    pub id: i64,
    pub style_type: Option<String>,
    pub emoji: Option<String>,
    pub icon: Option<String>,
}

/// One entry of `lookup_upload_urls`' answer.
#[derive(Debug, Clone)]
pub struct Upload {
    pub url: String,
    pub short_path: String,
    pub base62_sha1: String,
}

#[derive(Debug, Clone, Default)]
pub struct Lookups {
    /// username -> `avatar_template`, empty for nobody.
    pub avatars: HashMap<String, String>,
    /// username -> primary group name, empty for none.
    pub primary_groups: HashMap<String, String>,
    pub topics: HashMap<i64, Option<TopicInfo>>,
    /// The hashtag as written -> what it resolves to for the cooking user.
    pub hashtags: HashMap<String, Option<Hashtag>>,
    /// Short url -> the upload, for the ones that exist.
    pub uploads: HashMap<String, Upload>,
}

#[derive(Debug, Clone, Default)]
pub struct Needs {
    pub usernames: Vec<String>,
    pub topics: Vec<i64>,
    pub hashtags: Vec<String>,
    pub uploads: Vec<String>,
}

impl Needs {
    pub fn is_empty(&self) -> bool {
        self.usernames.is_empty()
            && self.topics.is_empty()
            && self.hashtags.is_empty()
            && self.uploads.is_empty()
    }
}

fn note<T: PartialEq>(list: &mut Vec<T>, item: T) {
    if !list.contains(&item) {
        list.push(item);
    }
}

/// Shared by the rules of one pass.
#[derive(Debug, Default)]
pub struct Context {
    pub lookups: Lookups,
    needs: Mutex<Needs>,
    /// The first thing met that is not ported.
    unsupported: Mutex<Option<&'static str>>,
}

impl MarkdownItExt for Context {}

impl Context {
    pub fn with(lookups: Lookups) -> Context {
        Context {
            lookups,
            ..Default::default()
        }
    }

    pub fn needs(&self) -> Needs {
        self.needs.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn unsupported(&self) -> Option<&'static str> {
        *self.unsupported.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn refuse(&self, what: &'static str) {
        self.unsupported
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_or_insert(what);
    }

    fn noting(&self, f: impl FnOnce(&mut Needs)) {
        f(&mut self.needs.lock().unwrap_or_else(|e| e.into_inner()));
    }

    /// `lookupAvatar(username)`'s template.
    pub fn avatar(&self, username: &str) -> &str {
        self.noting(|n| note(&mut n.usernames, username.to_string()));
        self.lookups
            .avatars
            .get(username)
            .map_or("", String::as_str)
    }

    /// `lookupPrimaryUserGroup(username)`
    pub fn primary_group(&self, username: &str) -> &str {
        self.noting(|n| note(&mut n.usernames, username.to_string()));
        self.lookups
            .primary_groups
            .get(username)
            .map_or("", String::as_str)
    }

    /// `getTopicInfo(topicId)`
    pub fn topic(&self, id: i64) -> Option<&TopicInfo> {
        self.noting(|n| note(&mut n.topics, id));
        self.lookups.topics.get(&id).and_then(Option::as_ref)
    }

    /// `hashtagLookup(slug, ...)`
    pub fn hashtag(&self, slug: &str) -> Option<&Hashtag> {
        self.noting(|n| note(&mut n.hashtags, slug.to_string()));
        self.lookups.hashtags.get(slug).and_then(Option::as_ref)
    }

    /// One short url of `lookupUploadUrls`.
    pub fn upload(&self, short_url: &str) -> Option<&Upload> {
        self.noting(|n| note(&mut n.uploads, short_url.to_string()));
        self.lookups.uploads.get(short_url)
    }
}
