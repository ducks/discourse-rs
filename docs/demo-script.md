# Demo script

A live click-through for the Discourse team, on the 8080 demo server (the
1,089-topic, 20,576-post corpus; docs/demo-servers.md). Every login's
password is `password`. Benchmark numbers come from BENCH.md; Rails is not
run live.

The order is the demo's: the core forum, then chat, then admin. Each step
names its persona. scripts/demo-rehearsal walks the same steps in Chromium
and reports any step that errors, so run it before showing.

| Persona | Login | Role |
|---|---|---|
| Visitor | none | anonymous |
| Newcomer | signs up during step 13 | trust level 0, activated by admin |
| Member | `user1` | trust level 2 |
| Second member | `user0` | trust level 1 |
| Moderator | `user2` | moderator |
| Admin | `admin` | admin |

## Core forum

1. Visitor: `/latest`, scroll, then `/categories`, a category, `/top`.
2. Visitor: open a long topic (`/t/68`), scroll through posts, open the
   lightbox on an image, follow a quote.
3. Visitor: search a common term from the header, then the full search
   page with a filter.
4. Visitor: a tag page, a user profile.
5. Member logs in from a topic and comes back to it.
6. Member: reply with formatting, a mention of `@user0`, an emoji, an
   uploaded image; the reply appears without a reload.
7. Second member (another browser): the mention notification arrives;
   open it, like the reply.
8. Member: new topic with a poll and a checklist; vote on the poll; tick a
   checklist item.
9. Member: bookmark a post, then the bookmarks list in the user menu.
10. Member: private message to Second member; Second member replies.
11. Member: preferences, switch to the dark palette and back.
12. Member: upload a video in a reply; it draws as a placeholder and plays.
13. Newcomer: sign up (no SMTP, so the account waits for activation).

## Chat

14. Member: open the chat drawer from the header, send a message in a
    channel; Second member sees it arrive.
15. Member: start a direct message with Second member, react to a message;
    switch the drawer to full-page chat and back.

## Admin and moderation

16. Admin: activate the Newcomer's account from the admin users list.
17. Second member flags a post as inappropriate.
18. Moderator: the review count in the header and sidebar; open /review,
    add a note, agree with the flag.
19. Admin: watched word with "require approval"; Newcomer posts with it;
    the post is held; approve it from /review.
20. Admin: the topic admin wrench: close and reopen a topic, unlist it.
21. Admin: the dashboard and a report, the staff action log showing the
    steps above.
22. Admin: change a list setting (authorized extensions) and a plain one
    from the settings page; customize a color palette.
23. Admin: silence a member and show the system message they receive;
    unsilence.

## Steer around

Known gaps that refuse with a "not ported yet" error:

- A brand-new member who types their first post very fast is held and
  auto-silenced, and a silenced author makes /review refuse. Type at a
  human pace in step 19.
- New-user approval cards (`must_approve_users`, 148), editing a queued
  post (194e), the review queue's silence and suspend client actions.
- Pinning from the topic wrench (194c) and most post admin actions (194d).
- Site setting types beyond lists (164b-h): uploads, objects, regex,
  dates.
- The composer's rich editor (196).
- Category group moderation (149).
- An MP4 won't play in Playwright's Chromium (no H.264); real browsers
  play it.
