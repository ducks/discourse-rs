#!/usr/bin/env python3
"""scripts/tally-refusals' two halves that want more than awk.

fill <db-url> <routes> <work>: the URLs to crawl. Every GET route of
`discourse-rs routes`, its path parameters filled with real ids, slugs and
usernames from the database (the member's username where a route names a
user); the pages of the corpus (every topic, category, tag and user); and
the JSON the Ember client loads beside a page. Writes <work>/urls ("path
TAB route" lines), <work>/unfilled (routes with a parameter nothing here
knows how to fill, or whose sample row the database lacks) and
<work>/skipped (static files and routes that would end the crawl's
session).

report <work>: ranks what <work>/results and <work>/server.log hold.
"""
import collections
import json
import re
import subprocess
import sys
from pathlib import Path
from urllib.parse import quote

# Not crawled: files served from disk (bundled assets, uploads, fonts,
# plugin files), the message bus's long polls, and routes that end or
# switch the crawl's session (Rails' dev-only `session/:username/become`,
# logout).
SKIP = re.compile(
    r"^/(assets|uploads|images|fonts|highlight-js|plugins)/"
    r"|^/(uploads|images)$|^/logout|^/session/\{username\}(/become)?$|^/bus/"
)

# One sample of each thing a route can name, from the database. A query
# that fails (a plugin's table missing) or finds nothing leaves the key
# unset, and the routes that need it are reported as unfilled.
SAMPLES = {
    "member_username": "SELECT username FROM users WHERE id = {member_id}",
    "admin_username": "SELECT username FROM users WHERE id = {admin_id}",
    "topic_id": """SELECT id FROM topics WHERE archetype = 'regular' AND deleted_at IS NULL
        AND visible ORDER BY posts_count DESC, id LIMIT 1""",
    "topic_slug": "SELECT slug FROM topics WHERE id = {topic_id}",
    "pm_topic_id": """SELECT t.id FROM topics t JOIN topic_allowed_users a ON a.topic_id = t.id
        WHERE t.archetype = 'private_message' AND a.user_id = {member_id} AND t.deleted_at IS NULL
        ORDER BY t.id DESC LIMIT 1""",
    "post_id": """SELECT id FROM posts WHERE topic_id = {topic_id} AND deleted_at IS NULL
        ORDER BY (post_number = 2) DESC, post_number LIMIT 1""",
    "post_number": "SELECT post_number FROM posts WHERE id = {post_id}",
    "revised_post_id": "SELECT post_id FROM post_revisions ORDER BY id LIMIT 1",
    "revision": "SELECT number FROM post_revisions WHERE post_id = {revised_post_id} ORDER BY number LIMIT 1",
    "poll_post_id": "SELECT post_id FROM polls ORDER BY id LIMIT 1",
    "poll_name": "SELECT name FROM polls WHERE post_id = {poll_post_id} ORDER BY id LIMIT 1",
    "poll_topic_id": "SELECT topic_id FROM posts WHERE id = {poll_post_id}",
    "post_action_id": "SELECT id FROM post_actions WHERE deleted_at IS NULL ORDER BY id LIMIT 1",
    "category_id": """SELECT id FROM categories WHERE parent_category_id IS NULL AND read_restricted = false
        AND id <> (SELECT value::int FROM site_settings WHERE name = 'uncategorized_category_id'
                   UNION ALL SELECT -1 LIMIT 1)
        ORDER BY topic_count DESC, id LIMIT 1""",
    "category_slug": "SELECT slug FROM categories WHERE id = {category_id}",
    "subcategory_path": """SELECT p.slug || '/' || c.slug || '/' || c.id FROM categories c
        JOIN categories p ON p.id = c.parent_category_id ORDER BY c.topic_count DESC, c.id LIMIT 1""",
    "tag": "SELECT name FROM tags ORDER BY topic_count DESC, id LIMIT 1",
    "tag2": "SELECT name FROM tags ORDER BY topic_count DESC, id OFFSET 1 LIMIT 1",
    "tag_id": "SELECT id FROM tags WHERE name = '{tag}'",
    "tag_group_id": "SELECT id FROM tag_groups ORDER BY id LIMIT 1",
    "group_name": """SELECT name FROM groups WHERE id > 0 OR name LIKE 'trust_level_%'
        ORDER BY automatic, user_count DESC, id LIMIT 1""",
    "group_id": "SELECT id FROM groups WHERE name = '{group_name}'",
    "badge_id": "SELECT id FROM badges WHERE enabled ORDER BY grant_count DESC, id LIMIT 1",
    "badge_slug": """SELECT trim(both '-' from regexp_replace(lower(name), '[^a-z0-9]+', '-', 'g'))
        FROM badges WHERE id = {badge_id}""",
    "user_badge_id": "SELECT id FROM user_badges WHERE user_id = {member_id} ORDER BY id LIMIT 1",
    "chat_channel_id": """SELECT ch.id FROM chat_channels ch JOIN categories c ON c.id = ch.chatable_id
        WHERE ch.deleted_at IS NULL AND ch.chatable_type = 'Category'
        ORDER BY c.read_restricted, ch.messages_count DESC, ch.id LIMIT 1""",
    "chat_channel_slug": "SELECT coalesce(slug, 'channel') FROM chat_channels WHERE id = {chat_channel_id}",
    "chat_message_id": """SELECT id FROM chat_messages WHERE chat_channel_id = {chat_channel_id}
        AND deleted_at IS NULL ORDER BY id DESC LIMIT 1""",
    "chat_thread_id": "SELECT id FROM chat_threads WHERE channel_id = {chat_channel_id} ORDER BY id LIMIT 1",
    "chat_webhook_id": "SELECT id FROM incoming_chat_webhooks ORDER BY id LIMIT 1",
    "reviewable_id": "SELECT id FROM reviewables ORDER BY id DESC LIMIT 1",
    "invite_key": "SELECT invite_key FROM invites WHERE deleted_at IS NULL ORDER BY id DESC LIMIT 1",
    "bookmark_id": "SELECT id FROM bookmarks ORDER BY user_id = {member_id} DESC, id LIMIT 1",
    "draft_key": "SELECT draft_key FROM drafts ORDER BY user_id = {member_id} DESC, id LIMIT 1",
    "notification_id": "SELECT id FROM notifications WHERE user_id = {member_id} ORDER BY id DESC LIMIT 1",
    "theme_id": "SELECT id FROM themes WHERE component = false ORDER BY id LIMIT 1",
    "component_id": "SELECT id FROM themes WHERE component ORDER BY id LIMIT 1",
    "color_scheme_id": "SELECT id FROM color_schemes ORDER BY id LIMIT 1",
    "web_hook_id": "SELECT id FROM web_hooks ORDER BY id LIMIT 1",
    "web_hook_event_id": "SELECT id FROM web_hook_events WHERE web_hook_id = {web_hook_id} ORDER BY id LIMIT 1",
    "api_key_id": "SELECT id FROM api_keys ORDER BY id LIMIT 1",
    "permalink_id": "SELECT id FROM permalinks ORDER BY id LIMIT 1",
    "embeddable_host_id": "SELECT id FROM embeddable_hosts ORDER BY id LIMIT 1",
    "user_field_id": "SELECT id FROM user_fields ORDER BY id LIMIT 1",
    "flag_id": "SELECT id FROM flags ORDER BY id LIMIT 1",
    "form_template_id": "SELECT id FROM form_templates ORDER BY id LIMIT 1",
    "sidebar_section_id": "SELECT id FROM sidebar_sections ORDER BY id LIMIT 1",
    "incoming_email_id": "SELECT id FROM incoming_emails ORDER BY id DESC LIMIT 1",
    "screened_email_id": "SELECT id FROM screened_emails ORDER BY id LIMIT 1",
    "screened_ip_id": "SELECT id FROM screened_ip_addresses ORDER BY id LIMIT 1",
    "watched_word_id": "SELECT id FROM watched_words ORDER BY id LIMIT 1",
    "admin_notice_id": "SELECT id FROM admin_notices ORDER BY id LIMIT 1",
    "custom_emoji_id": "SELECT id FROM custom_emojis ORDER BY id LIMIT 1",
    "leaderboard_id": "SELECT id FROM gamification_leaderboards ORDER BY id LIMIT 1",
    "upload_sha1": "SELECT sha1 FROM uploads WHERE id > 0 ORDER BY id DESC LIMIT 1",
    "template_id": """SELECT t.id FROM topics t JOIN categories c ON c.id = t.category_id
        WHERE c.id = (SELECT value::int FROM site_settings WHERE name = 'discourse_templates_categories'
                      AND value ~ '^[0-9]+$') ORDER BY t.id LIMIT 1""",
}

# Literal values for parameters that name a kind of thing, not a row.
LITERALS = {
    "period": "weekly",
    "size": "48",
    "version": "1",
    "letter": "a",
    "color": "f0a",
    "hostname": "localhost",
    "locale": "en",
}

# Which sample fills a parameter: (regex on the route up to and including
# the parameter, value). The first match wins; a value is a SAMPLES key, a
# "=literal", or a template of keys ("{category_slug}/{category_id}").
RULES = [
    # Wildcards: a real path under them, as the client asks for it.
    (r"^/c/\{\*path\}", "{category_slug}/{category_id}"),
    (r"^/tags/c/\{\*path\}", "{category_slug}/{category_id}/{tag}"),
    (r"^/tags/intersection/\{\*path\}", "{tag}/{tag2}"),
    (r"^/tag/\{\*path\}", "{tag}"),
    (r"^/my/\{\*path\}", "=preferences"),
    (r"^/(u|users)/\{username\}/\{\*rest\}", "=summary"),
    (r"^/(g|groups)/\{name\}/\{\*tab\}", "=members"),
    (r"^/topics/private-messages-group/\{username\}/\{\*rest\}", "{group_name}"),
    # Topics and posts.
    (r"^/t/\{slug\}/\{(id|topic_id)\}", "topic_id"),
    (r"^/t/\{id\}", "topic_id"),
    (r"^/polls/\{topic_id\}", "poll_topic_id"),
    (r"^/live/poll/\{post_id\}", "poll_post_id"),
    (r"\{poll_name\}", "poll_name"),
    (r"^/posts/\{id\}/revisions/\{revision\}", "revision"),
    (r"^/posts/\{id\}/revisions", "revised_post_id"),
    (r"\{topic_id\}", "topic_id"),
    (r"\{(post_number|number)\}", "post_number"),
    (r"^/(posts|post_actions)/\{id\}", "post_id"),
    (r"^/discourse-reactions/posts/\{id\}", "post_id"),
    (r"\{post_id\}", "post_id"),
    (r"^/live/post/\{id\}", "post_id"),
    (r"^/reviewable_claimed_topics/\{id\}", "topic_id"),
    (r"^/discourse_templates/\{id\}", "template_id"),
    # Categories and tags.
    (r"^/(categories|category)/\{(id|category_id)\}", "category_id"),
    (r"\{category_id\}", "category_id"),
    (r"^/tag_groups/\{id\}", "tag_group_id"),
    # Users and groups.
    (r"^/(g|groups|group-card)/\{name\}", "group_name"),
    (r"^/(g|groups)/by-id/\{id\}", "group_id"),
    (r"^/admin/groups/\{id\}", "group_id"),
    (r"\{group_id\}", "group_id"),
    (r"\{(groupname|group_name)\}", "group_name"),
    (r"^/topics/\{kind\}", "=private-messages"),
    (r"\{(username|username_lower)\}", "member_username"),
    (r"\{user_id\}", "member_id"),
    (r"^/user_badges/\{id\}", "user_badge_id"),
    (r"^/(admin/)?badges/\{id\}/\{slug\}", "badge_slug"),
    (r"^/(admin/)?badges/\{id\}", "badge_id"),
    (r"\{badge_id\}", "badge_id"),
    (r"^/bookmarks/\{id\}", "bookmark_id"),
    (r"^/drafts/\{id\}", "draft_key"),
    (r"^/notifications/\{id\}", "notification_id"),
    (r"^/invites/(show/)?\{id\}", "invite_key"),
    (r"^/sidebar_sections/(reset/)?\{id\}", "sidebar_section_id"),
    (r"^/(admin/customize/)?form-templates/\{id\}", "form_template_id"),
    # Chat.
    (r"^/chat/c/\{slug\}/\{id\}/info/\{tab\}", "=members"),
    (r"^/chat/browse/\{tab\}", "=open"),
    (r"^/chat/c/\{slug\}$", "chat_channel_slug"),
    (r"^/chat/c/\{slug\}/\{id\}", "chat_channel_id"),
    (r"^/chat/api/channels/\{id\}", "chat_channel_id"),
    (r"^/chat/api/category-chatables/\{id\}", "category_id"),
    (r"^/svg-sprite/search/\{keyword\}", "=heart"),
    (r"^/svg-sprite/.*\{name\}", "=heart"),
    (r"^/stylesheets/\{file\}", "=desktop.css"),
    (r"^/chat/\{id\}", "chat_channel_id"),
    (r"\{channel_id\}", "chat_channel_id"),
    (r"\{channel_title\}", "chat_channel_slug"),
    (r"\{message_id\}", "chat_message_id"),
    (r"\{thread_id\}", "chat_thread_id"),
    (r"^/admin/plugins/chat/hooks/\{id\}", "chat_webhook_id"),
    # Review.
    (r"\{reviewable_id\}", "reviewable_id"),
    # Admin.
    (r"^/admin/(customize/)?(themes|components)/\{id\}", "theme_id"),
    (r"\{theme_id\}", "theme_id"),
    (r"^/admin/customize/components/\{id\}", "component_id"),
    (r"^/(admin/(customize/|config/)?colors?(_schemes)?|color-scheme-stylesheet)/\{id\}", "color_scheme_id"),
    (r"^/admin/api/keys/\{id\}", "api_key_id"),
    (r"^/admin/api/web_hooks?(_events)?/\{id\}", "web_hook_id"),
    (r"\{event_id\}", "web_hook_event_id"),
    (r"^/admin/(config/)?permalinks/\{id\}", "permalink_id"),
    (r"^/admin/(customize/embedding|embeddable_hosts)/\{id\}", "embeddable_host_id"),
    (r"^/admin/config/user[-_]fields/\{id\}", "user_field_id"),
    (r"^/admin/config/flags/\{id\}", "flag_id"),
    (r"\{flag_id\}", "flag_id"),
    (r"^/admin/email-logs/incoming(_from_bounced)?/\{id\}", "incoming_email_id"),
    (r"^/admin/email-logs/\{kind\}", "=sent"),
    (r"^/admin/email/templates/\{id\}", "=user_notifications.signup"),
    (r"^/admin/logs/screened_emails/\{id\}", "screened_email_id"),
    (r"^/admin/logs/screened_ip_addresses/\{id\}", "screened_ip_id"),
    (r"^/admin/customize/watched_words/action/\{id\}", "=block"),
    (r"^/admin/customize/watched_words/\{id\}", "watched_word_id"),
    (r"^/admin/customize/site_texts/\{id\}", "=js.topic.create"),
    (r"^/admin/customize/email_style/\{field\}", "=html"),
    (r"^/admin/site_settings/category/\{category\}", "=required"),
    (r"^/admin/site_settings/\{id\}", "=title"),
    (r"^/admin/reports/\{kind\}", "=signups"),
    (r"^/admin/plugins/\{plugin_id\}", "=chat"),
    (r"^/admin/users/list/\{query\}", "=active"),
    (r"^/admin/admin_notices/\{id\}", "admin_notice_id"),
    (r"^/admin/config/emoji/\{id\}", "custom_emoji_id"),
    (r"^/admin/config/category-management/\{tab\}", "=categories"),
    (r"\{target\}", "=common"),
    (r"\{field_name\}", "=scss"),
    (r"leaderboards?/\{id\}", "leaderboard_id"),
    # Everything else that names a kind, not a row.
    (r"^/cakeday/(anniversaries|birthdays)/\{filter\}", "=today"),
    (r"^/new-category/\{tab\}", "=general"),
    (r"^/wizard/steps/\{id\}", "=introduction"),
    (r"^/styleguide/\{category\}", "=atoms"),
    (r"^/styleguide/\{category\}/\{section\}", "=colors"),
    (r"^/sitemap_\{page\}", "=1"),
    (r"^/user_avatar/\{hostname\}/\{username\}", "member_username"),
]

# The JSON the Ember client loads beside a page (the application route's
# preload, the header's menus, a topic's and a user's extra requests), with
# the query strings it sends; from frontend/discourse/app's ajax calls.
CLIENT = """
/session/current.json
/site.json
/site/basic-info.json
/srv/status
/notifications.json?recent=true&limit=30&bump_last_seen_reviewable=true
/notifications/totals.json
/notifications.json?username={member_username}&filter=read
/review/count.json
/review/user-menu-list.json
/review.json?status=pending
/u/{member_username}/user-menu-bookmarks.json
/u/{member_username}/user-menu-private-messages.json
/u/{member_username}.json
/u/{member_username}/card.json
/u/{member_username}/emails.json
/u/{member_username}/summary.json
/u/{member_username}/badges.json
/u/{member_username}/bookmarks.json
/u/{member_username}/notifications
/u/{member_username}/activity/topics
/u/{member_username}/activity/replies
/u/{member_username}/activity/likes-given
/u/{member_username}/activity/drafts
/u/{member_username}/activity/bookmarks
/u/{member_username}/activity/read
/u/{member_username}/messages
/u/{member_username}/messages/sent
/u/{member_username}/invited
/u/{member_username}/badges
/u/{member_username}/preferences
/u/{member_username}/preferences/account
/u/{member_username}/preferences/security
/u/{member_username}/preferences/profile
/u/{member_username}/preferences/emails
/u/{member_username}/preferences/notifications
/u/{member_username}/preferences/tracking
/u/{member_username}/preferences/users
/u/{member_username}/preferences/interface
/u/{member_username}/preferences/navigation-menu
/u/{member_username}/preferences/chat
/u/{member_username}/preferences/apps
/user_actions.json?offset=0&username={member_username}&filter=4,5
/user_actions.json?offset=0&username={member_username}&filter=1
/user_actions.json?offset=0&username={member_username}&filter=11
/user_badges.json?username={member_username}
/user-badges/{member_username}.json?grouped=true
/topics/created-by/{member_username}.json
/topics/private-messages/{member_username}.json
/topics/private-messages-sent/{member_username}.json
/topics/private-messages-unread/{member_username}.json
/drafts.json?offset=0&limit=50
/directory_items.json?period=weekly&order=likes_received
/directory_items.json?period=all&order=post_count&group={group_name}
/about.json
/badges.json
/groups.json
/g/{group_name}.json
/groups/{group_name}/members.json?offset=0&limit=50
/groups/{group_name}/posts.json
/latest.json?no_definitions=true&page=1
/latest.json?order=activity
/top.json?period=weekly
/categories.json?include_subcategories=true
/c/{category_slug}/{category_id}/l/latest.json
/c/{category_slug}/{category_id}/l/top.json?period=monthly
/c/{category_slug}/{category_id}/find_by_slug.json
/tags.json
/tag/{tag}/l/latest.json
/tag/{tag}/info.json
/tags/filter/search.json?q=
/t/{topic_id}.json?track_visit=true&forceLoad=true
/t/{topic_id}/posts.json?post_ids[]={post_id}
/t/{topic_id}/view-stats.json
/t/{topic_slug}/{topic_id}/print
/t/{pm_topic_id}.json
/posts/{post_id}.json
/posts/{post_id}/reply-history.json
/posts/{post_id}/replies.json
/posts/{post_id}/reply-ids.json
/post_action_users.json?id={post_id}&post_action_type_id=2
/post_replies?post_id={post_id}
/search.json?q=the
/search/query.json?term=the&include_blurbs=true
/u/search/users.json?term=a
/hashtags/search.json?term=a
/composer_messages.json?composer_action=createTopic
/emojis.json
/presence/get?channels[]=/chat-reply/{chat_channel_id}
/chat/api/me/channels
/chat/api/me/threads
/chat/api/channels/{chat_channel_id}.json
/chat/api/channels/{chat_channel_id}/messages.json?page_size=50
/chat/api/channels/{chat_channel_id}/threads.json
/chat/api/channels/{chat_channel_id}/memberships.json
/chat/c/{chat_channel_slug}/{chat_channel_id}
/chat/browse/open
/polls/voters.json?post_id={poll_post_id}&poll_name={poll_name}
/admin/dashboard.json
/admin/dashboard/general.json
/admin/site_settings.json
/admin/users/list/active.json?show_emails=false
/admin/users/{member_id}.json
/admin/plugins.json
/admin/customize/themes.json
/admin/logs/staff_action_logs.json
/admin/reports/signups.json
/admin/badges.json
/admin/api/keys.json
/admin/email.json
/admin/backups.json
"""


def psql(db, sql):
    run = subprocess.run(["psql", "-At", "-v", "ON_ERROR_STOP=1", db, "-c", sql],
                         capture_output=True, text=True)
    if run.returncode != 0:
        return None
    out = run.stdout.strip().split("\n")[0]
    return out or None


def samples(db, member_id, admin_id):
    values = {"member_id": member_id, "admin_id": admin_id}
    for key, sql in SAMPLES.items():
        try:
            filled = sql.format(**values)
        except KeyError:
            continue  # what it depends on is missing
        value = psql(db, filled)
        if value is not None:
            values[key] = value
    return values


def expand(template, values):
    """The template with each {key} from values, None if any is unset."""
    try:
        return re.sub(r"\{(\w+)\}", lambda m: quote(values[m.group(1)], safe="/"), template)
    except KeyError:
        return None


def fill(route, values):
    """(path, None) or (None, why) for a route's path parameters."""
    out = ""
    pos = 0
    for m in re.finditer(r"\{(\*?)(\w+)\}", route):
        out += route[pos:m.start()]
        upto = route[:m.end()]
        rule = next((v for r, v in RULES if re.search(r + "$", upto)), None)
        # /t/{slug} is the slug before an id, else the topic id itself
        # (Rails' `t/:topic_id/posts` and friends).
        if upto == "/t/{slug}":
            rule = "topic_slug" if re.search(r"\{(id|topic_id)\}", route) else "topic_id"
        if rule is None:
            rule = "=" + LITERALS[m.group(2)] if m.group(2) in LITERALS else None
        if rule is None:
            return None, f"no rule for {{{m.group(1)}{m.group(2)}}}"
        if rule.startswith("="):
            value = rule[1:]
        elif "{" in rule:
            value = expand(rule, values)
        else:
            value = values.get(rule)
            value = quote(value, safe="") if value is not None else None
        if value is None:
            return None, f"no sample for {{{m.group(2)}}} ({rule})"
        out += value
        pos = m.end()
    return out + route[pos:], None


def cmd_fill(db, routes_file, work, member_id, admin_id):
    work = Path(work)
    values = samples(db, member_id, admin_id)
    (work / "samples").write_text("".join(f"{k}\t{v}\n" for k, v in sorted(values.items())))
    urls = {}  # path -> route
    unfilled, skipped = [], []
    get_routes = 0
    for line in Path(routes_file).read_text().splitlines():
        methods, route = line.split(" ", 1)
        if "GET" not in methods.split(","):
            continue
        get_routes += 1
        if SKIP.search(route):
            skipped.append(route)
            continue
        path, why = fill(route, values)
        if path is None:
            unfilled.append(f"{route}\t{why}")
        else:
            urls.setdefault(path, route)
    filled = get_routes - len(skipped) - len(unfilled)
    # The corpus's pages, as the old crawl had them: every topic (and its
    # later pages), category, tag and user, with the JSON of each.
    pages = ["/", "/latest", "/top", "/hot", "/categories", "/tags"]
    q = lambda sql: (subprocess.run(["psql", "-At", db, "-c", sql], capture_output=True,
                                    text=True, check=True).stdout.split())
    pages += q("SELECT '/latest?page=' || g FROM generate_series(1, (SELECT count(*) FROM topics) / 30 + 1) g")
    pages += q("SELECT '/c/' || slug || '/' || id FROM categories")
    pages += q("SELECT '/tag/' || name FROM tags")
    pages += q("SELECT '/t/' || slug || '/' || id FROM topics WHERE archetype = 'regular'")
    pages += q("""SELECT '/t/' || slug || '/' || id || '?page=' || g FROM topics,
        generate_series(2, (posts_count - 1) / 20 + 1) g WHERE archetype = 'regular' AND posts_count > 20""")
    for suffix in ["", "/summary", "/activity"]:
        pages += q(f"SELECT '/u/' || username || '{suffix}' FROM users WHERE id > 0")
    pages += [f"/search?q={t}" for t in ["the", "a", "test", "discourse"]]
    for page in pages:
        urls.setdefault(page, "")
        base, _, query = page.partition("?")
        json_path = ("/latest" if base == "/" else base) + ".json" + ("?" + query if query else "")
        urls.setdefault(json_path, "")
    client_unfilled = []
    for template in CLIENT.split():
        path = expand(template, values)
        if path is None:
            client_unfilled.append(template)
        else:
            urls.setdefault(path, "")
    with open(work / "urls", "w") as f:
        for path, route in urls.items():
            f.write(f"{path}\t{route}\n")
    (work / "unfilled").write_text("".join(f"{u}\n" for u in unfilled)
                                   + "".join(f"{c}\tclient JSON, no sample\n" for c in client_unfilled))
    (work / "skipped").write_text("".join(f"{s}\n" for s in skipped))
    (work / "coverage").write_text(
        f"GET routes: {get_routes}, filled: {filled}, unfilled: {len(unfilled)}, "
        f"skipped (static files, session-ending): {len(skipped)}\n"
        f"URLs per identity: {len(urls)}\n")


class Router:
    """Matches a crawled path to its route the way matchit ranks them:
    static segments before parameters, parameters before wildcards."""

    def __init__(self, routes):
        self.routes = []
        for route in routes:
            regex = "^"
            for part in re.split(r"(\{\*?\w+\})", route):
                if part.startswith("{*"):
                    regex += ".+"
                elif part.startswith("{"):
                    regex += "[^/]+"
                else:
                    regex += re.escape(part)
            segments = route.split("/")
            rank = [0 if "{*" in s else 1 if "{" in s else 2 for s in segments]
            self.routes.append((rank, route, re.compile(regex + "$")))
        self.routes.sort(key=lambda r: r[0], reverse=True)

    def match(self, path):
        path = path.split("?")[0]
        for _, route, regex in self.routes:
            if regex.match(path):
                return route
        return "(unrouted)"


def cmd_report(work, top):
    work = Path(work)
    routes = [line.split(" ", 1)[1] for line in (work / "routes").read_text().splitlines()]
    router = Router(routes)
    route_of = {}
    for line in (work / "urls").read_text().splitlines():
        path, route = line.split("\t")
        route_of[path] = route or router.match(path)
    results = []  # (identity, status, path)
    for line in (work / "results").read_text().splitlines():
        who, status, path = line.split(" ", 2)
        results.append((who, int(status), path))

    # The log, split at the crawl's markers: what each request logged.
    ansi = re.compile(r"\x1b\[[0-9;]*m")
    refusals = collections.defaultdict(set)  # reason -> {(who, path)}
    errors = {}  # (who, path) -> first error line
    key = None
    for raw in open(work / "server.log", errors="replace"):
        line = ansi.sub("", raw).rstrip()
        if line.startswith("== "):
            _, who, path = line.split(" ", 2)
            key = (who, path)
            continue
        if key is None:
            continue
        m = re.search(r"not ported yet: (.*)$", line)
        if m:
            refusals[m.group(1).strip()].add(key)
        # tower_http's "response failed" line only repeats the status.
        elif " ERROR " in line and "response failed" not in line and key not in errors:
            errors[key] = re.sub(r"^\S+\s+ERROR\s+", "", line)[:300]

    print(f"== Coverage\n{(work / 'coverage').read_text()}")
    by_who = collections.Counter(w for w, _, _ in results)
    print("requests: " + ", ".join(f"{w} {n}" for w, n in by_who.items()))
    for who in by_who:
        codes = collections.Counter(s for w, s, _ in results if w == who)
        print(f"  {who}: " + ", ".join(f"{c} {n}" for c, n in sorted(codes.items())))
    refused_keys = set().union(*refusals.values()) if refusals else set()
    print(f"refusals: {len(refusals)} distinct reasons on {len(refused_keys)} requests")

    rows = []
    for reason, keys in refusals.items():
        by_route = collections.Counter(route_of.get(p) or router.match(p) for _, p in keys)
        who = sorted({w for w, _ in keys})
        rows.append((len({p for _, p in keys}), len(by_route), reason, by_route, who))
    print(f"\n== Refusals by URL count (top {top})")
    print("URLs  routes  reason  [identities]  top route")
    for urls, nroutes, reason, by_route, who in sorted(rows, key=lambda r: (-r[0], r[2]))[:top]:
        print(f"{urls:5} {nroutes:6}  {reason}  [{','.join(who)}]  {by_route.most_common(1)[0][0]}")
    print(f"\n== Refusals by distinct route (top {top})")
    print("routes  URLs  reason: routes")
    for urls, nroutes, reason, by_route, _ in sorted(rows, key=lambda r: (-r[1], -r[0], r[2]))[:top]:
        shown = ", ".join(r for r, _ in by_route.most_common(4))
        more = f" (+{nroutes - 4})" if nroutes > 4 else ""
        print(f"{nroutes:6} {urls:5}  {reason}: {shown}{more}")

    print("\n== 5xx without a refusal (bugs)")
    bugs = collections.defaultdict(list)  # (route, error) -> [(who, status, path)]
    for who, status, path in results:
        if status >= 500 and (who, path) not in refused_keys:
            route = route_of.get(path) or router.match(path)
            bugs[(route, errors.get((who, path), "(no error logged)"))].append((who, status, path))
    for (route, error), hits in sorted(bugs.items(), key=lambda b: (-len(b[1]), b[0])):
        who = ",".join(sorted({w for w, _, _ in hits}))
        print(f"{len(hits):4}  {route}  [{who}] e.g. {hits[0][2]} ({hits[0][1]})\n      {error}")

    print("\n== Routes never reached (could not fill)")
    print((work / "unfilled").read_text(), end="")
    print("\n== 4xx sample (a crawl that mostly 404s is not a clean one)")
    four = collections.Counter((w, s, route_of.get(p) or router.match(p))
                               for w, s, p in results if 400 <= s < 500)
    for (who, status, route), n in four.most_common(30):
        print(f"{n:5} {status} {who} {route}")
    (work / "report.json").write_text(json.dumps({
        "refusals": [{"reason": r, "urls": u, "routes": dict(br)} for u, _, r, br, _ in rows],
        "bugs": [{"route": r, "error": e, "hits": h} for (r, e), h in bugs.items()],
    }, indent=1))


if __name__ == "__main__":
    if sys.argv[1] == "fill":
        cmd_fill(*sys.argv[2:7])
    elif sys.argv[1] == "report":
        cmd_report(sys.argv[2], int(sys.argv[3]) if len(sys.argv) > 3 else 40)
    else:
        sys.exit(__doc__)
