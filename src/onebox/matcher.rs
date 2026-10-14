//! lib/onebox/matcher.rb and the engines' matching (Onebox::Engine's
//! `===`, matches_regexp, matches_domain, matches_path,
//! matches_content_type, requires_iframe_origins): which engine oneboxes a
//! URL.

use std::sync::LazyLock;

use fancy_regex::Regex;

use crate::final_destination::Uri;

/// How an engine recognizes its URLs.
enum Match {
    /// `matches_regexp`: the URL matches (case-insensitively when true).
    Regex(&'static str, bool),
    /// GoogleMapsOnebox's `===`: any of its regexes.
    AnyRegex(&'static [&'static str]),
    /// `matches_domain` (subdomains too when allowed), then the engine's
    /// `matches_path`, if it has one.
    Domains {
        domains: &'static [&'static str],
        subdomains: bool,
        path: Option<&'static str>,
    },
    /// AllowlistedGenericOnebox's `===`: any host but an IP address.
    Generic,
}

pub struct Engine {
    pub name: &'static str,
    matcher: Match,
    /// `matches_content_type`
    content_type: Option<&'static str>,
    /// `requires_iframe_origins`
    pub iframe_origins: &'static [&'static str],
    /// The class's `always_https`.
    pub always_https: bool,
}

/// A Ruby regexp's source as a Rust one: `^` and `$` at line breaks, and
/// `\w`, `\d` and `\p{Alnum}` as Onigmo has them.
pub fn ruby_regex(source: &str, case_insensitive: bool) -> Regex {
    let converted = source
        .replace(r"\p{Alnum}", r"[\p{Alphabetic}\p{Nd}]")
        .replace(r"\w", "[0-9A-Za-z_]")
        .replace(r"\d", "[0-9]");
    let flags = if case_insensitive { "(?mi)" } else { "(?m)" };
    Regex::new(&format!("{flags}{converted}")).unwrap_or_else(|e| panic!("{source}: {e}"))
}

/// `Onebox::Engine.engines` as `Matcher#ordered_engines` sorts them by
/// priority, in the order the reference's sort leaves equal priorities
/// (Ruby's sort_by isn't stable), with the plugins' engines it loads.
pub static ENGINES: &[Engine] = &[
    Engine {
        name: "ThreadsStatusOnebox",
        matcher: Match::Domains {
            domains: &["threads.net", "www.threads.net"],
            subdomains: false,
            path: Some(r#"^/t/[\d\w_-]+/?"#),
        },
        content_type: None,
        iframe_origins: &[],
        always_https: true,
    },
    Engine {
        name: "GfycatOnebox",
        matcher: Match::Domains {
            domains: &["gfycat.com"],
            subdomains: false,
            path: None,
        },
        content_type: None,
        iframe_origins: &[],
        always_https: true,
    },
    Engine {
        name: "WikimediaOnebox",
        matcher: Match::Domains {
            domains: &["commons.wikimedia.org"],
            subdomains: false,
            path: Some(r#"^/wiki/File:.+"#),
        },
        content_type: None,
        iframe_origins: &[],
        always_https: true,
    },
    Engine {
        name: "GithubActionsOnebox",
        matcher: Match::Regex(
            r#"^https?://(?:www\.)?(?:(?:\w)+\.)?github\.com/(?<org>.+)/(?<repo>.+)/(actions/runs/[[:digit:]]+|pull/[[:digit:]]*/checks\?check_run_id=[[:digit:]]+)"#,
            false,
        ),
        content_type: None,
        iframe_origins: &[],
        always_https: true,
    },
    Engine {
        name: "SpotifyOnebox",
        matcher: Match::Domains {
            domains: &["open.spotify.com"],
            subdomains: false,
            path: None,
        },
        content_type: None,
        iframe_origins: &["https://open.spotify.com"],
        always_https: true,
    },
    Engine {
        name: "SoundCloudOnebox",
        matcher: Match::Domains {
            domains: &["soundcloud.com", "www.soundcloud.com"],
            subdomains: false,
            path: None,
        },
        content_type: None,
        iframe_origins: &["https://w.soundcloud.com"],
        always_https: true,
    },
    Engine {
        name: "SlidesOnebox",
        matcher: Match::Domains {
            domains: &["slides.com"],
            subdomains: false,
            path: Some(r#"^/[\p{Alnum}_\-]+/[\p{Alnum}_\-]+$"#),
        },
        content_type: None,
        iframe_origins: &["https://slides.com"],
        always_https: true,
    },
    Engine {
        name: "SketchFabOnebox",
        matcher: Match::Domains {
            domains: &["sketchfab.com"],
            subdomains: false,
            path: Some(r#"^/(models/|3d-models/[^/\s]+-)?[a-z0-9]{32}"#),
        },
        content_type: None,
        iframe_origins: &["https://sketchfab.com"],
        always_https: true,
    },
    Engine {
        name: "SimplecastOnebox",
        matcher: Match::Domains {
            domains: &["simplecast.com"],
            subdomains: true,
            path: Some(r#"^/(episodes|s)/.+"#),
        },
        content_type: None,
        iframe_origins: &["https://player.simplecast.com"],
        always_https: true,
    },
    Engine {
        name: "RestreamOnebox",
        matcher: Match::Domains {
            domains: &["player.restream.io"],
            subdomains: false,
            path: Some(r#"\A/?\z"#),
        },
        content_type: None,
        iframe_origins: &["https://player.restream.io"],
        always_https: true,
    },
    Engine {
        name: "ReplitOnebox",
        matcher: Match::Domains {
            domains: &["replit.com", "repl.it"],
            subdomains: false,
            path: None,
        },
        content_type: None,
        iframe_origins: &[],
        always_https: true,
    },
    Engine {
        name: "RedditMediaOnebox",
        matcher: Match::Domains {
            domains: &[
                "reddit.com",
                "www.reddit.com",
                "old.reddit.com",
                "np.reddit.com",
                "new.reddit.com",
            ],
            subdomains: false,
            path: Some(r#"^/(?:r|user)/[^/]+/comments/[^/]+"#),
        },
        content_type: None,
        iframe_origins: &["https://embed.reddit.com", "https://sh.reddit.com"],
        always_https: true,
    },
    Engine {
        name: "PubmedOnebox",
        matcher: Match::Domains {
            domains: &["ncbi.nlm.nih.gov"],
            subdomains: true,
            path: Some(r#"^/pubmed/\d+$"#),
        },
        content_type: None,
        iframe_origins: &[],
        always_https: false,
    },
    Engine {
        name: "PdfOnebox",
        matcher: Match::Regex(r#"^(https?:)?//.*\.pdf(\?.*)?$"#, true),
        content_type: None,
        iframe_origins: &[],
        always_https: true,
    },
    Engine {
        name: "PastebinOnebox",
        matcher: Match::Domains {
            domains: &["pastebin.com"],
            subdomains: false,
            path: None,
        },
        content_type: None,
        iframe_origins: &[],
        always_https: true,
    },
    Engine {
        name: "MotokoOnebox",
        matcher: Match::Domains {
            domains: &["embed.motoko.org", "embed.smartcontracts.org"],
            subdomains: false,
            path: None,
        },
        content_type: None,
        iframe_origins: &[
            "https://embed.motoko.org",
            "https://embed.smartcontracts.org",
        ],
        always_https: true,
    },
    Engine {
        name: "MixcloudOnebox",
        matcher: Match::Domains {
            domains: &["www.mixcloud.com"],
            subdomains: false,
            path: None,
        },
        content_type: None,
        iframe_origins: &["https://www.mixcloud.com"],
        always_https: true,
    },
    Engine {
        name: "LoomOnebox",
        matcher: Match::Domains {
            domains: &["loom.com", "www.loom.com"],
            subdomains: false,
            path: Some(r#"^/share/\w+(/\w+)?/?$"#),
        },
        content_type: None,
        iframe_origins: &["https://www.loom.com"],
        always_https: true,
    },
    Engine {
        name: "KalturaOnebox",
        matcher: Match::Domains {
            domains: &["kaltura.com"],
            subdomains: true,
            path: Some(r#"^/id/[a-zA-Z0-9]+$"#),
        },
        content_type: None,
        iframe_origins: &["https://*.kaltura.com"],
        always_https: true,
    },
    Engine {
        name: "InstagramOnebox",
        matcher: Match::Domains {
            domains: &["www.instagram.com", "instagram.com", "instagr.am"],
            subdomains: false,
            path: Some(r#"^/(?:[\w\-]+/)?(?:p|tv)/[a-zA-Z\d_-]+/?(?:\?.*)?$"#),
        },
        content_type: None,
        iframe_origins: &["https://www.instagram.com"],
        always_https: true,
    },
    Engine {
        name: "StackExchangeOnebox",
        matcher: Match::Regex(
            r#"^https?://(?:(?:(?<subsubdomain>\w*)\.)?(?<subdomain>\w*)\.)?(?<domain>stackexchange\.com|stackoverflow\.com|superuser\.com|serverfault\.com|askubuntu\.com|stackapps\.com|mathoverflow\.net)/((?:questions|q)/(?<question_id>\d*)(/.*/(?<answer_id1>\d*))?|(a/(?<answer_id2>\d*)))"#,
            false,
        ),
        content_type: None,
        iframe_origins: &[],
        always_https: false,
    },
    Engine {
        name: "SteamStoreOnebox",
        matcher: Match::Domains {
            domains: &["store.steampowered.com"],
            subdomains: false,
            path: Some(r#"^/app/\d+$"#),
        },
        content_type: None,
        iframe_origins: &["https://store.steampowered.com"],
        always_https: true,
    },
    Engine {
        name: "TiktokOnebox",
        matcher: Match::Domains {
            domains: &["tiktok.com", "www.tiktok.com", "m.tiktok.com"],
            subdomains: false,
            path: Some(r#"^(/@.+/video/\d+|/v/\d+)(/\w+)?/?$"#),
        },
        content_type: None,
        iframe_origins: &["https://www.tiktok.com"],
        always_https: true,
    },
    Engine {
        name: "TrelloOnebox",
        matcher: Match::Domains {
            domains: &["trello.com"],
            subdomains: false,
            path: Some(r#"^/[bc]/\w*"#),
        },
        content_type: None,
        iframe_origins: &["https://trello.com"],
        always_https: true,
    },
    Engine {
        name: "TwitchClipsOnebox",
        matcher: Match::Regex(
            r#"^https?://(?:clips\.twitch\.tv/embed\?clip=|www\.twitch\.tv/[a-zA-Z0-9_]+/clip/|clips\.twitch\.tv/)([a-zA-Z0-9_\-]+)"#,
            false,
        ),
        content_type: None,
        iframe_origins: &["https://clips.twitch.tv"],
        always_https: false,
    },
    Engine {
        name: "TwitchStreamOnebox",
        matcher: Match::Regex(
            r#"^https?://(?:www\.|go\.)?twitch\.tv/(?!directory)([a-zA-Z0-9_]{4,25})$"#,
            false,
        ),
        content_type: None,
        iframe_origins: &["https://player.twitch.tv"],
        always_https: false,
    },
    Engine {
        name: "TwitchVideoOnebox",
        matcher: Match::Regex(r#"^https?://(?:www\.)?twitch\.tv/videos/([0-9]+)"#, false),
        content_type: None,
        iframe_origins: &["https://player.twitch.tv"],
        always_https: false,
    },
    Engine {
        name: "TwitterStatusOnebox",
        matcher: Match::Domains {
            domains: &[
                "twitter.com",
                "www.twitter.com",
                "mobile.twitter.com",
                "x.com",
                "www.x.com",
            ],
            subdomains: false,
            path: Some(r#"^/.+?/status(es)?/\d+(/(video|photo)/\d?)?(/?\?.*)?/?$"#),
        },
        content_type: None,
        iframe_origins: &[],
        always_https: true,
    },
    Engine {
        name: "TypeformOnebox",
        matcher: Match::Domains {
            domains: &["typeform.com"],
            subdomains: true,
            path: Some(r#"^/to/[a-zA-Z0-9]+$"#),
        },
        content_type: None,
        iframe_origins: &["https://*.typeform.com"],
        always_https: true,
    },
    Engine {
        name: "VideoOnebox",
        matcher: Match::Regex(r#"^(https?:)?//.*\.(mov|mp4|webm|ogv)(\?.*)?$"#, true),
        content_type: None,
        iframe_origins: &[],
        always_https: false,
    },
    Engine {
        name: "VimeoOnebox",
        matcher: Match::Domains {
            domains: &["vimeo.com", "www.vimeo.com"],
            subdomains: false,
            path: Some(r#"^/\d+(/\w+)?/?$"#),
        },
        content_type: None,
        iframe_origins: &["https://player.vimeo.com"],
        always_https: true,
    },
    Engine {
        name: "WikipediaOnebox",
        matcher: Match::Domains {
            domains: &["wikipedia.com", "wikipedia.org"],
            subdomains: true,
            path: None,
        },
        content_type: None,
        iframe_origins: &[],
        always_https: true,
    },
    Engine {
        name: "WistiaOnebox",
        matcher: Match::Domains {
            domains: &["wistia.com", "wi.st"],
            subdomains: true,
            path: Some(r#"^/(medias|embed)/.*$"#),
        },
        content_type: None,
        iframe_origins: &["https://fast.wistia.com", "https://fast.wistia.net"],
        always_https: true,
    },
    Engine {
        name: "XkcdOnebox",
        matcher: Match::Domains {
            domains: &["xkcd.com", "www.xkcd.com", "m.xkcd.com"],
            subdomains: false,
            path: Some(r#"^/\d+$"#),
        },
        content_type: None,
        iframe_origins: &[],
        always_https: true,
    },
    Engine {
        name: "YoukuOnebox",
        matcher: Match::Domains {
            domains: &["youku.com", "youku.tv"],
            subdomains: true,
            path: None,
        },
        content_type: None,
        iframe_origins: &["https://player.youku.com"],
        always_https: true,
    },
    Engine {
        name: "YoutubeOnebox",
        matcher: Match::Domains {
            domains: &[
                "youtube.com",
                "www.youtube.com",
                "m.youtube.com",
                "youtu.be",
            ],
            subdomains: false,
            path: Some(r#"^/.+$"#),
        },
        content_type: None,
        iframe_origins: &["https://www.youtube.com"],
        always_https: true,
    },
    Engine {
        name: "GithubCommitOnebox",
        matcher: Match::Domains {
            domains: &["github.com", "www.github.com"],
            subdomains: false,
            path: Some(r#"^/[\w\-]+/[\w\-]+/commit/[a-f0-9]{40}$"#),
        },
        content_type: None,
        iframe_origins: &[],
        always_https: true,
    },
    Engine {
        name: "GithubBlobOnebox",
        matcher: Match::Regex(r#"^https?://(www\.)?github\.com.*/blob/"#, false),
        content_type: None,
        iframe_origins: &[],
        always_https: true,
    },
    Engine {
        name: "FlickrShortenedOnebox",
        matcher: Match::Domains {
            domains: &["flic.kr"],
            subdomains: false,
            path: Some(r#"\A/p/"#),
        },
        content_type: None,
        iframe_origins: &[],
        always_https: true,
    },
    Engine {
        name: "FlickrOnebox",
        matcher: Match::Domains {
            domains: &["www.flickr.com"],
            subdomains: false,
            path: Some(r#"\A/photos/"#),
        },
        content_type: None,
        iframe_origins: &[],
        always_https: true,
    },
    Engine {
        name: "FiveHundredPxOnebox",
        matcher: Match::Domains {
            domains: &["500px.com"],
            subdomains: false,
            path: Some(r#"^/photo/\d+/"#),
        },
        content_type: None,
        iframe_origins: &[],
        always_https: true,
    },
    Engine {
        name: "FacebookMediaOnebox",
        matcher: Match::Domains {
            domains: &["facebook.com", "www.facebook.com"],
            subdomains: false,
            path: None,
        },
        content_type: None,
        iframe_origins: &["https://www.facebook.com"],
        always_https: true,
    },
    Engine {
        name: "DiscourseTopicOnebox",
        matcher: Match::Regex(r#"/t/.*(/\d+)?"#, false),
        content_type: None,
        iframe_origins: &[],
        always_https: false,
    },
    Engine {
        name: "CoubOnebox",
        matcher: Match::Domains {
            domains: &["coub.com"],
            subdomains: false,
            path: Some(r#"\A/view/"#),
        },
        content_type: None,
        iframe_origins: &[],
        always_https: true,
    },
    Engine {
        name: "CloudAppOnebox",
        matcher: Match::Domains {
            domains: &["cl.ly"],
            subdomains: false,
            path: None,
        },
        content_type: None,
        iframe_origins: &[],
        always_https: true,
    },
    Engine {
        name: "BandCampOnebox",
        matcher: Match::Domains {
            domains: &["bandcamp.com"],
            subdomains: true,
            path: Some(r#"^/(album|track)/"#),
        },
        content_type: None,
        iframe_origins: &["https://bandcamp.com"],
        always_https: true,
    },
    Engine {
        name: "AudioboomOnebox",
        matcher: Match::Domains {
            domains: &["audioboom.com"],
            subdomains: false,
            path: Some(r#"^/posts/\d+$"#),
        },
        content_type: None,
        iframe_origins: &[],
        always_https: true,
    },
    Engine {
        name: "AudioOnebox",
        matcher: Match::Regex(r#"^(https?:)?//.*\.(mp3|ogg|opus|wav|m4a)(\?.*)?$"#, true),
        content_type: None,
        iframe_origins: &[],
        always_https: false,
    },
    Engine {
        name: "AudioComOnebox",
        matcher: Match::Domains {
            domains: &["audio.com"],
            subdomains: false,
            path: None,
        },
        content_type: None,
        iframe_origins: &["https://audio.com"],
        always_https: true,
    },
    Engine {
        name: "AsciinemaOnebox",
        matcher: Match::Domains {
            domains: &["asciinema.org"],
            subdomains: false,
            path: Some(r#"^/a/[\p{Alnum}_\-]+$"#),
        },
        content_type: None,
        iframe_origins: &[],
        always_https: true,
    },
    Engine {
        name: "AnimatedImageOnebox",
        matcher: Match::Domains {
            domains: &["giphy.com", "gph.is", "tenor.com"],
            subdomains: false,
            path: None,
        },
        content_type: None,
        iframe_origins: &[],
        always_https: true,
    },
    Engine {
        name: "AmazonOnebox",
        matcher: Match::Regex(
            r#"^https?://(?:www\.)?(?:smile\.)?(amazon|amzn)\.(?<tld>com|ca|de|it|es|fr|co\.jp|co\.uk|cn|in|com\.br|com\.mx|nl|pl|sa|sg|se|com\.tr|ae)/"#,
            false,
        ),
        content_type: None,
        iframe_origins: &[],
        always_https: true,
    },
    Engine {
        name: "ImgurOnebox",
        matcher: Match::Domains {
            domains: &["imgur.com", "www.imgur.com"],
            subdomains: false,
            path: None,
        },
        content_type: None,
        iframe_origins: &[],
        always_https: true,
    },
    Engine {
        name: "ImageOnebox",
        matcher: Match::Regex(
            r#"^(https?:)?//.+\.(png|jpg|jpeg|gif|bmp|tif|tiff|webp|avif)(\?.*)?$"#,
            true,
        ),
        content_type: Some(r#"^image/(png|jpg|jpeg|gif|bmp|tif|tiff|webp|avif)$"#),
        iframe_origins: &[],
        always_https: false,
    },
    Engine {
        name: "HackernewsOnebox",
        matcher: Match::Regex(
            r#"^https?://news\.ycombinator\.com/item\?id=(?<item_id>\d+)"#,
            false,
        ),
        content_type: None,
        iframe_origins: &[],
        always_https: true,
    },
    Engine {
        name: "GooglePlayAppOnebox",
        matcher: Match::Regex(
            r#"^https?://play\.(?:(?:\w)+\.)?(google)\.com(?:/)?/store/apps/"#,
            false,
        ),
        content_type: None,
        iframe_origins: &[],
        always_https: true,
    },
    Engine {
        name: "GooglePhotosOnebox",
        matcher: Match::Domains {
            domains: &["photos.google.com", "photos.app.goo.gl"],
            subdomains: false,
            path: Some(r#"^/.*$"#),
        },
        content_type: None,
        iframe_origins: &[],
        always_https: true,
    },
    Engine {
        name: "GoogleMeetOnebox",
        matcher: Match::Domains {
            domains: &["meet.google.com"],
            subdomains: false,
            path: Some(
                r#"(?i:\A/(?<code>[a-z]{3}-[a-z]{4}-[a-z]{3})/?\z)|(?i:\A/lookup/[a-z0-9_-]{3,64}/?\z)"#,
            ),
        },
        content_type: None,
        iframe_origins: &[],
        always_https: true,
    },
    Engine {
        name: "GoogleMapsOnebox",
        matcher: Match::AnyRegex(&[
            r#"^(https?:)?//goo\.gl/maps/"#,
            r#"^(?:https?:)?//www\.google(?:\.(?:\w{2,}))+/maps/d/(?:edit|viewer|embed)\?mid=.+$"#,
            r#"^(?:https?:)?//www\.google(?:\.(?:\w{2,}))+/maps[^@]+@(?<lon>-?[\d.]+),(?<lat>-?[\d.]+),(?:\d+)a,(?<zoom>[\d.]+)y,(?<heading>[\d.]+)h,(?<pitch>[\d.]+)t.+?data=.*?!1s(?<pano>[^!]{22})"#,
            r#"^(?:https?:)?//www\.google(?:\.(?:\w{2,}))+/maps"#,
            r#"^(?:https?:)?//maps\.google(?:\.(?:\w{2,}))+/maps\?"#,
        ]),
        content_type: None,
        iframe_origins: &["https://maps.google.com", "https://google.com"],
        always_https: true,
    },
    Engine {
        name: "GoogleDriveOnebox",
        matcher: Match::Regex(
            r#"^(https?:)?//(drive\.google\.com)/file/d/(?<key>[\w-]*)/.+$"#,
            false,
        ),
        content_type: None,
        iframe_origins: &[],
        always_https: true,
    },
    Engine {
        name: "GoogleDocsOnebox",
        matcher: Match::Regex(
            r#"^(https?:)?//(docs\.google\.com)/(?<endpoint>(spreadsheets|document|forms|presentation))/d/((?<key>[\w-]*)).+$"#,
            false,
        ),
        content_type: None,
        iframe_origins: &[],
        always_https: true,
    },
    Engine {
        name: "GoogleCalendarOnebox",
        matcher: Match::Domains {
            domains: &[
                "www.google.com",
                "google.com",
                "calendar.google.com",
                "goo.gl",
            ],
            subdomains: false,
            path: Some(r#"^/calendar/.*$"#),
        },
        content_type: None,
        iframe_origins: &["https://calendar.google.com"],
        always_https: true,
    },
    Engine {
        name: "GitlabBlobOnebox",
        matcher: Match::Regex(r#"^https?://(?:www\.)?gitlab\.com/.*/blob/.*"#, false),
        content_type: None,
        iframe_origins: &[],
        always_https: true,
    },
    Engine {
        name: "GithubRepoOnebox",
        matcher: Match::Domains {
            domains: &["github.com", "www.github.com"],
            subdomains: false,
            path: Some(r#"^/[^/]+/[^/]+/?$"#),
        },
        content_type: None,
        iframe_origins: &[],
        always_https: true,
    },
    Engine {
        name: "GithubPullRequestOnebox",
        matcher: Match::Domains {
            domains: &["github.com", "www.github.com"],
            subdomains: false,
            path: Some(r#".*/pull"#),
        },
        content_type: None,
        iframe_origins: &[],
        always_https: true,
    },
    Engine {
        name: "GithubIssueOnebox",
        matcher: Match::Regex(
            r#"^https?://(?:www\.)?(?:(?:\w)+\.)?github\.com/(?<org>.+)/(?<repo>.+)/issues/([[:digit:]]+)"#,
            false,
        ),
        content_type: None,
        iframe_origins: &[],
        always_https: true,
    },
    Engine {
        name: "GithubFolderOnebox",
        matcher: Match::Domains {
            domains: &["github.com", "www.github.com"],
            subdomains: false,
            path: Some(r#"^/[\w\-]+/[\w\-]+/tree/"#),
        },
        content_type: None,
        iframe_origins: &[],
        always_https: true,
    },
    Engine {
        name: "GithubGistOnebox",
        matcher: Match::Domains {
            domains: &["gist.github.com"],
            subdomains: false,
            path: Some(r#"^/[\w\-]+/[a-f0-9]+(/|$)"#),
        },
        content_type: None,
        iframe_origins: &[],
        always_https: true,
    },
    Engine {
        name: "AllowlistedGenericOnebox",
        matcher: Match::Generic,
        content_type: None,
        iframe_origins: &[],
        always_https: false,
    },
];

struct Compiled {
    matchers: Vec<Vec<Regex>>,
    content_types: Vec<Option<Regex>>,
}

static COMPILED: LazyLock<Compiled> = LazyLock::new(|| Compiled {
    matchers: ENGINES
        .iter()
        .map(|e| match &e.matcher {
            Match::Regex(source, icase) => vec![ruby_regex(source, *icase)],
            Match::AnyRegex(sources) => sources.iter().map(|s| ruby_regex(s, false)).collect(),
            Match::Domains { path, .. } => path.iter().map(|p| ruby_regex(p, false)).collect(),
            Match::Generic => Vec::new(),
        })
        .collect(),
    content_types: ENGINES
        .iter()
        .map(|e| e.content_type.map(|c| ruby_regex(c, false)))
        .collect(),
});

fn is_match(re: &Regex, s: &str) -> bool {
    re.is_match(s).unwrap_or(false)
}

/// `engine === uri`
fn handles(index: usize, url: &str, uri: &Uri) -> bool {
    let engine = &ENGINES[index];
    let regexes = &COMPILED.matchers[index];
    match &engine.matcher {
        Match::Regex(..) | Match::AnyRegex(_) => regexes.iter().any(|r| is_match(r, url)),
        Match::Domains {
            domains,
            subdomains,
            ..
        } => {
            let host = uri.host.to_lowercase();
            let domain = if *subdomains {
                domains
                    .iter()
                    .any(|d| host == *d || host.ends_with(&format!(".{d}")))
            } else {
                domains.contains(&host.as_str())
            };
            domain && regexes.iter().all(|r| is_match(r, uri.path()))
        }
        Match::Generic => uri.host.parse::<std::net::IpAddr>().is_err(),
    }
}

/// `Engine.origins_to_regexes`: an origin allowlist as regexes.
pub fn origins_to_regexes(origins: &[String]) -> Vec<regex::Regex> {
    if origins.iter().any(|o| o == "*") {
        return vec![regex::Regex::new(".*").unwrap()];
    }
    static HOST_ONLY: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"(?i)\Ahttps?://[^/?#]+\z").unwrap());
    static PORT: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r":\d+\z").unwrap());
    origins
        .iter()
        .map(|origin| {
            let mut escaped = regex::escape(origin);
            if origin.starts_with("*.")
                || origin.starts_with("https://*.")
                || origin.starts_with("http://*.")
            {
                escaped = escaped.replacen(r"\*", r"[^/?#\\]*", 1);
            }
            let boundary = if HOST_ONLY.is_match(origin) {
                if PORT.is_match(origin) {
                    r"(?:[/?#]|\z)"
                } else {
                    r"(?::\d+(?:[/?#]|\z)|[/?#]|\z)"
                }
            } else {
                ""
            };
            regex::Regex::new(&format!(r"(?i)\A{escaped}{boundary}")).unwrap()
        })
        .collect()
}

/// `Engine.all_iframe_origins`
pub fn all_iframe_origins() -> Vec<String> {
    let mut sorted: Vec<&Engine> = ENGINES.iter().collect();
    // Engine.engines: by class name.
    sorted.sort_by_key(|e| e.name);
    let mut out: Vec<String> = Vec::new();
    for o in sorted.iter().flat_map(|e| e.iframe_origins.iter()) {
        if !out.iter().any(|x| x == o) {
            out.push(o.to_string());
        }
    }
    out
}

/// `Matcher#oneboxed`: the first engine, by priority, that handles the
/// URL (or its content type) and whose iframes the allowlist lets in.
pub fn oneboxed(
    url: &str,
    content_type: Option<&str>,
    allowed_iframe_regexes: &[regex::Regex],
) -> Option<&'static Engine> {
    // A protocol-relative URL has a host but no scheme, and no port
    // unless it names one.
    let relative = url.starts_with("//");
    let uri = if relative {
        Uri::parse(&format!("http:{url}"))?
    } else {
        Uri::parse(url)?
    };
    let explicit_port = url
        .split("//")
        .nth(1)
        .and_then(|rest| rest.split(['/', '?', '#']).next())
        .is_some_and(|authority| authority.contains(':'));
    // Onebox::DEFAULTS' allowed_ports and allowed_schemes.
    if (!relative || explicit_port) && uri.port != 80 && uri.port != 443 {
        return None;
    }
    (0..ENGINES.len())
        .map(|i| (i, &ENGINES[i]))
        .find_map(|(i, engine)| {
            let by_type = match (&COMPILED.content_types[i], content_type) {
                (Some(re), Some(ct)) => is_match(re, ct),
                _ => false,
            };
            let allowed = engine
                .iframe_origins
                .iter()
                .all(|o| allowed_iframe_regexes.iter().any(|r| r.is_match(o)));
            ((by_type || handles(i, url, &uri)) && allowed).then_some(engine)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_engine_regex_compiles() {
        assert_eq!(COMPILED.matchers.len(), ENGINES.len());
    }
}
