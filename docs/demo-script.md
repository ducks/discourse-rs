# Demo script

A live click-through for the Discourse team, on the 8080 demo server (the
1,089-topic, 20,576-post corpus; docs/demo-servers.md). Every login's
password is `password`. Benchmark numbers come from BENCH.md; Rails is not
run live.

The order is the demo's: the core forum, then chat, then admin, then the
bundled plugins. Each step names its persona. scripts/demo-rehearsal walks
the same steps in Chromium and reports any step that errors, so run it
before showing.

| Persona | Login | Role |
|---|---|---|
| Visitor | none | anonymous |
| Newcomer | signs up during step 13 | trust level 0, activated by admin |
| Member | `user1` | trust level 2 |
| Second member | `user0` | trust level 1 |
| Moderator | `user2` | moderator |
| Admin | `admin` | admin |

## Core forum

1. Visitor: `/latest`, scroll (the next pages load in place), then
   `/categories`, a category, `/top`.
2. Visitor: open a long topic (`/t/118`, 100 posts) and scroll through it;
   the posts past the first 20 load as it scrolls, down to the footer and
   Reply, and the timeline follows. (The corpus has no lightbox images or
   linked quotes, so step 6 shows the lightbox on the member's own upload.)
3. Visitor: search a common term from the header, then the full search
   page with a filter.
4. Visitor: a tag page, a user profile.
5. Member logs in from a topic and comes back to it.
6. Member: reply with formatting, a mention of `@user0`, an emoji, an
   uploaded image; the reply appears without a reload, its image gets a
   lightbox once processed, and opening it shows the full image.
7. Second member (another browser): the mention notification arrives;
   open it, like the reply.
8. Member: new topic with a poll and a checklist: the poll from the
   toolbar's options menu (the circled plus), Build poll, three options
   (Enter adds the next), Insert Poll; vote on the poll; tick a checklist
   item.
9. Member: bookmark a post, then the bookmarks list in the user menu.
10. Member: private message to Second member from their profile's Message
    button (the recipients chooser above the title); Second member replies.
11. Member: preferences, switch to the dark palette and back.
12. Member: upload a video in a reply; it draws as a placeholder and plays.
13. Newcomer: sign up (no SMTP, so the account waits for activation).

## Chat

14. Member: open the chat drawer from the header, send a message in a
    channel; Second member sees it arrive.
15. Member: start a direct message with Second member, react to a message;
    switch the drawer to full-page chat and back.
16. Member: in full-page chat (`/chat/c/general/2`), send a message with a
    typo, fix it from the message's "..." menu (Edit), then delete it from
    the same menu.
17. Member: attach an image from the composer's + menu and send it; the
    message shows it, and clicking it opens the lightbox. Then send a
    topic's link on a line of its own: a moment later the message redraws
    with the topic's onebox (Second member sees it too). The sidebar's
    "Search chat" opens full-page search: type a word from that message,
    and its result opens the message in #General.
18. Admin: `/chat/browse`, New channel: a name (the slug fills in), the
    General category, Enable threading; send a first message, then click
    the channel's title for its settings page (threading shown on).
19. Member: open the new channel from `/chat/browse` and join it; Reply on
    the admin's message starts a thread (the indicator appears under it);
    open the thread in the side panel and reply there (the indicator counts
    two replies); the thread's back button leads to the channel's threads
    list (as in Discourse, a thread is listed a few seconds after its
    first reply, once its reply count is updated).
20. Member: in that thread's side panel, react to the second reply from
    its hover toolbar (it has no Reply in a thread), then Edit it from its
    "..." menu: the thread's composer takes the text, the edit is drawn in
    place; a reload keeps both.
21. Admin: open the thread, click its title in the panel's header (the
    original message's author and admins can): the thread settings modal,
    a title; the header shows it, and after a reload too.
22. Member: from a forum page, open the drawer, the new channel, its
    threads list button: the thread (by step 21's title), opened in the
    drawer; reply there, back to the list and the channel. Click the
    channel's title: its settings, then the Members tab; back. The page
    never reloads and stays on the forum page.
23. Moderator: join the new channel, then in #General select step 17's
    image message from its menu (Select), tick step 14's message too, and
    Move to Channel: the new channel opens on the moved messages.
24. Admin: `/admin/plugins/chat`, Incoming webhooks, New: a name and the
    new channel; the edit page shows the webhook's URL, and a POST to it
    (`curl -d text=hello <url>`) posts in the channel.
25. Member: in the new channel, mention `@user3` (who can see the channel
    but hasn't joined it): a notice says so and offers to invite them;
    Invite, "Invitation sent". Then the navbar's filter (the magnifier):
    type `demo`, the admin's first message highlighted with its position
    (1/1); Esc. Then click the "Today" date separator: the day's first
    message highlighted, in place.
26. Admin: turn on `chat_allow_archiving_channels` in the site settings;
    the new channel's settings, Archive channel: a new topic's title, the
    General category, Archive. The job quotes the messages into the
    topic; the channel shows it is archived (no composer; the archive
    unfollows its members, so a card says it can't be joined) with a link to
    the topic, whose posts are the chat transcript, threads included. As
    in Discourse, the retention reminder floats over that link: dismiss
    it (its x) first; the link opens the topic in a new tab.

## Admin and moderation

27. Admin: activate the Newcomer's account from the admin users list.
28. Second member flags a post as inappropriate.
29. Moderator: the review count in the header and sidebar; open /review,
    add a note, agree with the flag. Then Member sends a message in
    #General and Second member flags it from the message's "..." menu
    (Flag, inappropriate): in /review it sits beside the post flag with its
    channel and author; Yes > Delete Message, and it is gone from #General.
30. Admin: watched word with "require approval"; Newcomer posts with it;
    the post is held; approve it from /review. Type the post rather than
    pasting it: as in Discourse, a first post typed in under
    fast_typing_threshold (3 seconds) is held and its author silenced
    (the card shows the silence, and approving the post lifts it).
31. Admin: the topic admin wrench: close and reopen a topic, unlist it.
32. Admin: the dashboard and a report, the staff action log showing the
    steps above.
33. Admin: change a list setting (authorized extensions) and a plain one
    from the settings page; customize a color palette.
34. Admin: silence a member and show the system message they receive;
    unsilence.

## Plugins

The bundled plugins are Rust in the same binary, switched on at runtime.
Solved, Topic Voting, Reactions, Poll, Checklist and Templates are on by
default as in Discourse; step 35 turns on the others the steps use.

35. Admin: `/admin/plugins`, switch on Assign, Policy, User Notes,
    Gamification and Post Voting.
36. Admin: New Category from `/categories` with the Support and Ideas
    types ticked (Solved and Topic Voting for that category); it opens
    with the Votes and My Votes lists.
37. Member: a question in the new category; Second member votes for it
    (the count goes to 1) and answers.
38. Member: Solution on the answer; the title shows the solved status
    without a reload.
39. Second member: hover the question's heart and pick the clap from the
    reaction picker.
40. Admin: the question's footer Assign, to `user2`; the topic redraws in
    place with the assignee under the first post and the reassign menu.
41. Admin: reply with a policy (`[policy group=trust_level_1 accept="I
    agree" revoke="Withdraw"]...[/policy]`); Second member clicks I agree
    and the accepted count goes to 1.
42. Moderator: User notes on `user0`'s profile; add a note, staff only.
43. Admin: `/admin/plugins/discourse-gamification/leaderboards`,
    Recalculate scores (last 10 days), then `/leaderboard` with its podium.
44. Member: New Topic, the caret on the composer's title (the composer
    actions menu), Toggle post voting: the title says Create post voting
    topic; post it, and the question draws its vote controls; Reply
    answers.

## Benchmark table (end of the demo)

Recorded October 24, release v20261024.0.0 and Rails bf55a44, on the
same 20,576-post corpus and PostgreSQL 18 server. These are measurements
of that release, not a benchmark of the current demo build.

| Measurement | Rust | Rails |
|---|---:|---:|
| Latest JSON, uncached req/s | 2,173 | 122 |
| Categories JSON, uncached req/s | 845 | 203 |
| Twenty-post topic JSON, uncached req/s | 1,051 | 112 |
| Topic HTML, uncached req/s | 490 | 74 |
| Search, common term, uncached req/s | 125 | 65 |
| Search, selective term, uncached req/s | 504 | 172 |
| Startup to first response | 87 ms | 9.8 s |

One laptop ran the client, servers and database; anonymous requests,
no keep-alive, and a small corpus. Core response fields matched. See
[the recorded comparison](bench-20261024.md) and [BENCH.md](../BENCH.md)
for the protocol, memory measurements and October 30 Rust rerun. That
rerun measured 113 ms startup and identified rate-limiter contention
under artificially raised search limits; item 285 remains low priority.

## Rehearsal

`scripts/demo-rehearsal` walks the 44 steps in Chromium as the presenter
does, by clicking and typing, one browser context per persona (Member and
Second member stay open together for the live updates). Each step records
ok, fail or skipped (a step that needs an earlier step's outcome), how long
it took, a screenshot, and what went wrong: responses of 500 and up, page
and console errors, visible "not ported yet" or "Something went wrong"
text, images that didn't load, and the expected outcome that didn't
appear. `index.html` in the output directory has a row per step;
`results.json` has the same.

It writes to the site: replies, topics, a category, chat messages, a chat
channel, a new account. Run it against a scratch copy of the demo database
on a server of its own, never the database the live demo uses:

```bash
nix-shell --run '
  createdb -U postgres -O "$USER" discourse_rs_rehearsal
  pg_dump discourse_rs_demo | psql -X -q -d discourse_rs_rehearsal
'
# Then start tmp/demo/discourse-rs with drs-demo@.service's environment,
# DATABASE_URL pointing at discourse_rs_rehearsal, BIND_ADDR and
# UNICORN_PORT 8099, TMP_DIR and PUBLIC_DIR in a scratch directory and a
# DISCOURSE_SECRET_KEY_BASE of its own, logging to a file.
make rehearsal RS_URL=http://127.0.0.1:8099 SERVER_LOG=/path/to/server.log
# or: scripts/demo-rehearsal --url http://127.0.0.1:8099 --out <new dir> [--server-log <file>]
```

With `--server-log` each step also lists the server's request errors (the
"not ported yet" reason behind a 500) and, as warnings, the jobs it
refused; a job runs after its request, so its line can land in the next
step. Warnings (4xx answers, the htmx SSE stream's `[object Event]`
console error on every navigation away) are listed but don't fail a step.
`--only 18,19` runs some steps; the rest are skipped, and a step that
needs a skipped one (19 to 26 need 18's channel, 20 to 22 also 19's
thread, 37 to 41 need 36's category and 37's question, 40 to 44 need 35's
plugins) is skipped too.

It can run again on the same database. What it makes is labelled
"rehearsal <time>Z", the newcomer is `rh<timestamp>`, the watched word
`rhword<timestamp>`, the chat channel "Rehearsal <timestamp>" and the
category "Rehearsal ideas <timestamp>". It waits between a persona's posts
as a presenter would (Discourse refuses a second topic within 15 seconds),
and replies elsewhere when user1 already posted last in `/t/118`. At the
end it deletes the watched word, the color palette, the incoming webhook
and the chat channel it made through the admin (archived by step 26),
puts back the settings it changed (`chat_allow_archiving_channels`
included), switches off the plugins step 35 switched on and unsilences
whoever it silenced; the report lists what it cleaned up and what it left
behind: the replies, the topics (step 8's unlisted by step 31, step 26's
archive topic), the category, the like, reaction, vote, solution,
assignment, policy and its acceptance, the user note, the bookmark, chat
messages, reactions and the thread's title, user3's chat invitation, the
newcomer's account, the staff log's entries and the uploads. Step 43's
recalculation counts against Gamification's five a day; past that the
step shows the leaderboard as it is. Each run also spends logins against
`max_logins_per_ip_per_hour`; a run that can't log in says so.

## Steer around

Known gaps that refuse with a "not ported yet" error or misbehave:

- New-user approval cards (`must_approve_users`, 148), editing a queued
  post (194e), the review queue's silence and suspend client actions.
- Pinning from the topic wrench (194c) and most post admin actions (194d).
- Site setting types beyond lists (164b-h): uploads, objects, regex,
  dates.
- The composer's rich editor (196).
- Category group moderation (149).
- Pinned chat messages sit behind the hidden `chat_pinned_messages`
  setting, and My threads (the drawer footer's item, the sidebar's
  section) behind the hidden `chat_threads_enabled`, both off as in
  Discourse, which the admin can't switch.
- An MP4 won't play in Playwright's Chromium (no H.264); real browsers
  play it.
