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

## The numbers (30 seconds, before or after the click-through)

From docs/bench-20261024.md: release v20261024.0.0 against production Rails
on the same 20,576-post corpus and the same PostgreSQL 18 server.

- Same answers: latest, categories, topic posts with cooked HTML and search
  matched field for field.
- Uncached JSON: 4x to 18x the throughput (latest 2,173 vs 122 req/s), with
  p50 latency 7 vs 124 ms. Search narrows to 2x to 3x.
- Memory: one process at 50 MB idle against Rails' 0.85 GiB (PSS, the fair
  sum across its 11 processes; 5 GiB summed RSS).
- Startup: 87 ms to the first answered request against 9.8 s.
- Say the caveats with them: one laptop runs client, servers and database,
  anonymous traffic, no keep-alive, a 20k-post synthetic corpus. In the browser
  comparison Rails served its own assets without nginx and sent HTML and CSS
  uncompressed, so quote its ready times (content visible) loosely.

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
    message shows it, and clicking it opens the lightbox.
18. Admin: `/chat/browse`, New channel: a name (the slug fills in), the
    General category, Enable threading; send a first message, then click
    the channel's title for its settings page (threading shown on).
19. Member: open the new channel from `/chat/browse` and join it; Reply on
    the admin's message starts a thread (the indicator appears under it);
    open the thread in the side panel and reply there (the indicator counts
    two replies); the thread's back button leads to the channel's threads
    list (as in Discourse, a thread is listed a few seconds after its
    first reply, once its reply count is updated).
20. Moderator: join the new channel, then in #General select step 17's
    image message from its menu (Select), tick step 14's message too, and
    Move to Channel: the new channel opens on the moved messages.
21. Admin: `/admin/plugins/chat`, Incoming webhooks, New: a name and the
    new channel; the edit page shows the webhook's URL, and a POST to it
    (`curl -d text=hello <url>`) posts in the channel.

## Admin and moderation

22. Admin: activate the Newcomer's account from the admin users list.
23. Second member flags a post as inappropriate.
24. Moderator: the review count in the header and sidebar; open /review,
    add a note, agree with the flag.
25. Admin: watched word with "require approval"; Newcomer posts with it;
    the post is held; approve it from /review. Type the post rather than
    pasting it: as in Discourse, a first post typed in under
    fast_typing_threshold (3 seconds) is held and its author silenced
    (the card shows the silence, and approving the post lifts it).
26. Admin: the topic admin wrench: close and reopen a topic, unlist it.
27. Admin: the dashboard and a report, the staff action log showing the
    steps above.
28. Admin: change a list setting (authorized extensions) and a plain one
    from the settings page; customize a color palette.
29. Admin: silence a member and show the system message they receive;
    unsilence.

## Plugins

The bundled plugins are Rust in the same binary, switched on at runtime.
Solved, Topic Voting, Reactions, Poll, Checklist and Templates are on by
default as in Discourse; step 30 turns on the others the steps use.

30. Admin: `/admin/plugins`, switch on Assign, Policy, User Notes,
    Gamification and Post Voting.
31. Admin: New Category from `/categories` with the Support and Ideas
    types ticked (Solved and Topic Voting for that category); it opens
    with the Votes and My Votes lists.
32. Member: a question in the new category; Second member votes for it
    (the count goes to 1) and answers.
33. Member: Solution on the answer; the title shows the solved status
    without a reload.
34. Second member: hover the question's heart and pick the clap from the
    reaction picker.
35. Admin: the question's footer Assign, to `user2`; the topic redraws in
    place with the assignee under the first post and the reassign menu.
36. Admin: reply with a policy (`[policy group=trust_level_1 accept="I
    agree" revoke="Withdraw"]...[/policy]`); Second member clicks I agree
    and the accepted count goes to 1.
37. Moderator: User notes on `user0`'s profile; add a note, staff only.
38. Admin: `/admin/plugins/discourse-gamification/leaderboards`,
    Recalculate scores (last 10 days), then `/leaderboard` with its podium.
39. Member: New Topic, the caret on the composer's title (the composer
    actions menu), Toggle post voting: the title says Create post voting
    topic; post it, and the question draws its vote controls; Reply
    answers.

## Rehearsal

`scripts/demo-rehearsal` walks the 39 steps in Chromium as the presenter
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
needs a skipped one (19 to 21 need 18's channel, 32 to 36 need 31's
category and 32's question, 35 to 39 need 30's plugins) is skipped too.

It can run again on the same database. What it makes is labelled
"rehearsal <time>Z", the newcomer is `rh<timestamp>`, the watched word
`rhword<timestamp>`, the chat channel "Rehearsal <timestamp>" and the
category "Rehearsal ideas <timestamp>". It waits between a persona's posts
as a presenter would (Discourse refuses a second topic within 15 seconds),
and replies elsewhere when user1 already posted last in `/t/118`. At the
end it deletes the watched word, the color palette, the incoming webhook
and the chat channel it made through the admin, puts back the settings it
changed, switches off the plugins step 30 switched on and unsilences
whoever it silenced; the report lists what it cleaned up and what it left
behind: the replies, the topics (step 8's unlisted by step 26), the
category, the like, reaction, vote, solution, assignment, policy and its
acceptance, the user note, the bookmark, chat messages and reaction, the
newcomer's account, the staff log's entries and the uploads. Step 38's
recalculation counts against Gamification's five a day; past that the
step shows the leaderboard as it is. Each run also spends logins against
`max_logins_per_ip_per_hour`; a run that can't log in says so.

## Steer around

Known gaps that refuse with a "not ported yet" error or misbehave:

- New-user approval cards (`must_approve_users`, 148), editing a queued
  post (194e), the review queue's silence and suspend client actions.
- Flagging a chat message: the flag is saved, but /review can't draw its
  item, and while it is pending the review queue refuses to list (plugin
  reviewables, 217 with 148), which breaks step 24.
- Pinning from the topic wrench (194c) and most post admin actions (194d).
- Site setting types beyond lists (164b-h): uploads, objects, regex,
  dates.
- The composer's rich editor (196).
- Category group moderation (149).
- Full-page chat search: the sidebar's "Search chat" (`/chat/search`)
  answers 405, only the API is ported (220). The drawer's routes for
  threads, pins, members and search are being built (223c).
- Pinned chat messages sit behind the hidden `chat_pinned_messages`
  setting, off as in Discourse, which the admin can't switch.
- An MP4 won't play in Playwright's Chromium (no H.264); real browsers
  play it.
