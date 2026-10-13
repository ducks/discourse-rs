-- Fixture rows the screens need that the seed lacks, run on both sides by
-- scripts/parity-screenshots before capturing.

-- discourse-topic-voting: General votes; the replies-and-posters topic
-- has two votes (user1's and user2's), the archived one an archived vote.
INSERT INTO topic_voting_category_settings (category_id, created_at, updated_at)
VALUES (4, '2026-10-01 00:00:00', '2026-10-01 00:00:00');
INSERT INTO topic_voting_votes (topic_id, user_id, archive, created_at, updated_at)
VALUES (35, 3, FALSE, '2026-10-01 01:00:00', '2026-10-01 01:00:00'),
       (35, 4, FALSE, '2026-10-01 02:00:00', '2026-10-01 02:00:00'),
       (41, 3, TRUE, '2026-10-01 03:00:00', '2026-10-01 03:00:00');
INSERT INTO topic_voting_topic_vote_count (topic_id, votes_count, created_at, updated_at)
VALUES (35, 2, '2026-10-01 00:00:00', '2026-10-01 00:00:00'),
       (41, 1, '2026-10-01 00:00:00', '2026-10-01 00:00:00')
ON CONFLICT (topic_id) DO UPDATE SET votes_count = EXCLUDED.votes_count;

-- discourse-reactions: on the replies-and-posters topic's first post
-- user1's clap and admin's laughing (each with its shadow like) and
-- user2's plain like, so the post shows a reactions summary; user0's
-- hugs on user2's post.
INSERT INTO discourse_reactions_reactions (id, post_id, reaction_type, reaction_value, reaction_users_count, created_at, updated_at)
VALUES (1, 35, 0, 'clap', 1, '2026-10-01 04:00:00', '2026-10-01 04:00:00'),
       (2, 35, 0, 'laughing', 1, '2026-10-01 05:00:00', '2026-10-01 05:00:00'),
       (3, 37, 0, 'hugs', 1, '2026-10-01 06:00:00', '2026-10-01 06:00:00');
SELECT setval('discourse_reactions_reactions_id_seq', (SELECT MAX(id) FROM discourse_reactions_reactions));
INSERT INTO discourse_reactions_reaction_users (reaction_id, user_id, post_id, created_at, updated_at)
VALUES (1, 3, 35, '2026-10-01 04:00:00', '2026-10-01 04:00:00'),
       (2, 1, 35, '2026-10-01 05:00:00', '2026-10-01 05:00:00'),
       (3, 2, 37, '2026-10-01 06:00:00', '2026-10-01 06:00:00');
INSERT INTO post_actions (post_id, user_id, post_action_type_id, created_at, updated_at)
VALUES (35, 3, 2, '2026-10-01 04:00:00', '2026-10-01 04:00:00'),
       (35, 1, 2, '2026-10-01 05:00:00', '2026-10-01 05:00:00'),
       (35, 4, 2, '2026-10-01 05:30:00', '2026-10-01 05:30:00'),
       (37, 2, 2, '2026-10-01 06:00:00', '2026-10-01 06:00:00');
UPDATE posts SET like_count = like_count + 3 WHERE id = 35;
UPDATE posts SET like_count = like_count + 1 WHERE id = 37;
UPDATE topics SET like_count = like_count + 4 WHERE id = 35;

-- discourse-solved: General takes accepted answers; on the
-- replies-and-posters topic, user0 (its author) accepted user2's reply.
INSERT INTO category_custom_fields (category_id, name, value, created_at, updated_at)
VALUES (4, 'enable_accepted_answers', 'true', '2026-10-01 00:00:00', '2026-10-01 00:00:00');
INSERT INTO discourse_solved_solved_topics (topic_id, answer_post_id, accepter_user_id, created_at, updated_at)
VALUES (35, 37, 2, '2026-10-01 07:00:00', '2026-10-01 07:00:00');
INSERT INTO discourse_solved_topic_answers (solved_topic_id, answer_post_id, accepter_user_id, created_at, updated_at)
SELECT id, 37, 2, '2026-10-01 07:00:00', '2026-10-01 07:00:00' FROM discourse_solved_solved_topics WHERE topic_id = 35;

-- chat: General's messages (parity/writes/chat/messages.case.json): a reply, an
-- edit with reactions, a deleted message, a /me action and a link; user1
-- has read to the second and bookmarked the first.
INSERT INTO chat_messages (id, chat_channel_id, user_id, created_at, updated_at, message, cooked, cooked_version, last_editor_id) VALUES (1, 2, 2, now() - interval '50 minutes', now() - interval '50 minutes', 'Hello **world**', '<p>Hello <strong>world</strong></p>', 1, 2);
INSERT INTO chat_messages (id, chat_channel_id, user_id, created_at, updated_at, message, cooked, cooked_version, last_editor_id, in_reply_to_id) VALUES (2, 2, 3, now() - interval '40 minutes', now() - interval '40 minutes', '@user0 hi back', '<p><a class="mention" href="/u/user0">@user0</a> hi back</p>', 1, 3, 1);
INSERT INTO chat_messages (id, chat_channel_id, user_id, created_at, updated_at, message, cooked, cooked_version, last_editor_id) VALUES (3, 2, 1, now() - interval '30 minutes', now() - interval '30 minutes', 'edited message :heart:', '<p>edited message <img src="/images/emoji/twitter/heart.png?v=14" title=":heart:" class="emoji" alt=":heart:" loading="lazy" width="20" height="20"></p>', 1, 1);
INSERT INTO chat_messages (id, chat_channel_id, user_id, created_at, updated_at, message, cooked, cooked_version, last_editor_id, deleted_at, deleted_by_id) VALUES (4, 2, 2, now() - interval '20 minutes', now() - interval '20 minutes', 'gone', '<p>gone</p>', 1, 2, now() - interval '10 minutes', 2);
INSERT INTO chat_messages (id, chat_channel_id, user_id, created_at, updated_at, message, cooked, cooked_version, last_editor_id) VALUES (5, 2, 3, now() - interval '15 minutes', now() - interval '15 minutes', '/me waves', '<p>/me waves</p>', 1, 3);
INSERT INTO chat_messages (id, chat_channel_id, user_id, created_at, updated_at, message, cooked, cooked_version, last_editor_id) VALUES (6, 2, 2, now() - interval '5 minutes', now() - interval '5 minutes', 'https://example.com', '<p><a href="https://example.com" rel="noopener nofollow ugc">https://example.com</a></p>', 1, 2);
SELECT setval('chat_messages_id_seq', 6);
UPDATE chat_channels SET last_message_id = 6, messages_count = 6 WHERE id = 2;
INSERT INTO chat_message_revisions (chat_message_id, old_message, new_message, created_at, updated_at, user_id) VALUES (3, 'message', 'edited message :heart:', now() - interval '25 minutes', now() - interval '25 minutes', 1);
INSERT INTO chat_message_reactions (chat_message_id, user_id, emoji, created_at, updated_at) VALUES (3, 3, 'heart', now() - interval '29 minutes', now() - interval '29 minutes'), (3, 2, 'heart', now() - interval '28 minutes', now() - interval '28 minutes'), (3, 2, '+1', now() - interval '27 minutes', now() - interval '27 minutes'), (3, 3, 'notanemoji', now() - interval '26 minutes', now() - interval '26 minutes');
INSERT INTO chat_mentions (chat_message_id, type, target_id, created_at, updated_at) VALUES (2, 'Chat::UserMention', 2, now() - interval '40 minutes', now() - interval '40 minutes');
INSERT INTO bookmarks (user_id, bookmarkable_id, bookmarkable_type, name, auto_delete_preference, created_at, updated_at) VALUES (3, 1, 'ChatMessage', 'later', 3, now() - interval '45 minutes', now() - interval '45 minutes');
UPDATE user_chat_channel_memberships SET last_read_message_id = 2 WHERE chat_channel_id = 2 AND user_id = 3;

-- chat direct messages: user1 and user2 (two messages), and a group DM
-- named "Weekend plans" with user1, user2 and user3 (one message); both
-- followed by their members so they show in the sidebar.
INSERT INTO direct_message_channels (id, "group", created_at, updated_at) VALUES (1, false, now() - interval '2 hours', now() - interval '2 hours'), (2, true, now() - interval '2 hours', now() - interval '2 hours');
SELECT setval('direct_message_channels_id_seq', 2);
INSERT INTO direct_message_users (direct_message_channel_id, user_id, created_at, updated_at) VALUES (1, 3, now() - interval '2 hours', now() - interval '2 hours'), (1, 4, now() - interval '2 hours', now() - interval '2 hours'), (2, 3, now() - interval '2 hours', now() - interval '2 hours'), (2, 4, now() - interval '2 hours', now() - interval '2 hours'), (2, 5, now() - interval '2 hours', now() - interval '2 hours');
INSERT INTO chat_channels (id, chatable_type, chatable_id, type, status, user_count, name, messages_count, created_at, updated_at) VALUES (3, 'DirectMessage', 1, 'DirectMessageChannel', 0, 2, NULL, 2, now() - interval '2 hours', now() - interval '2 hours'), (4, 'DirectMessage', 2, 'DirectMessageChannel', 0, 3, 'Weekend plans', 1, now() - interval '2 hours', now() - interval '2 hours');
SELECT setval('chat_channels_id_seq', 4);
INSERT INTO user_chat_channel_memberships (user_id, chat_channel_id, following, muted, notification_level, created_at, updated_at) VALUES (3, 3, true, false, 2, now() - interval '2 hours', now() - interval '2 hours'), (4, 3, true, false, 2, now() - interval '2 hours', now() - interval '2 hours'), (3, 4, true, false, 2, now() - interval '2 hours', now() - interval '2 hours'), (4, 4, true, false, 2, now() - interval '2 hours', now() - interval '2 hours'), (5, 4, true, false, 2, now() - interval '2 hours', now() - interval '2 hours');
INSERT INTO chat_messages (id, chat_channel_id, user_id, created_at, updated_at, message, cooked, cooked_version, last_editor_id) VALUES (7, 3, 3, now() - interval '90 minutes', now() - interval '90 minutes', 'Are you around later?', '<p>Are you around later?</p>', 1, 3), (8, 3, 4, now() - interval '80 minutes', now() - interval '80 minutes', 'Yes, after five', '<p>Yes, after five</p>', 1, 4), (9, 4, 5, now() - interval '70 minutes', now() - interval '70 minutes', 'Hike on Saturday?', '<p>Hike on Saturday?</p>', 1, 5);
SELECT setval('chat_messages_id_seq', 9);
UPDATE chat_channels SET last_message_id = 8 WHERE id = 3;
UPDATE chat_channels SET last_message_id = 9 WHERE id = 4;
UPDATE user_chat_channel_memberships SET last_read_message_id = 7 WHERE chat_channel_id = 3 AND user_id = 3;

-- A message to the moderators group from user1 (admin a member), for the
-- group inbox.
INSERT INTO topics (id, title, fancy_title, slug, user_id, last_post_user_id, archetype, subtype, created_at, updated_at, bumped_at, last_posted_at, posts_count, highest_post_number, visible) VALUES (90, 'A group message the moderators received', 'A group message the moderators received', 'a-group-message-the-moderators-received', 3, 3, 'private_message', 'user_to_user', now() - interval '1 hour', now() - interval '1 hour', now() - interval '1 hour', now() - interval '1 hour', 1, 1, true);
INSERT INTO posts (id, topic_id, user_id, post_number, raw, cooked, created_at, updated_at, last_version_at, sort_order) VALUES (900, 90, 3, 1, 'The first post of a group message to the moderators.', '<p>The first post of a group message to the moderators.</p>', now() - interval '1 hour', now() - interval '1 hour', now() - interval '1 hour', 1);
INSERT INTO topic_allowed_users (topic_id, user_id, created_at, updated_at) VALUES (90, 3, now(), now());
INSERT INTO topic_allowed_groups (topic_id, group_id) VALUES (90, 2);
UPDATE groups SET has_messages = true WHERE id = 2;
INSERT INTO topic_users (user_id, topic_id, notification_level, notifications_reason_id, posted, last_read_post_number) VALUES (3, 90, 3, 1, true, 1), (1, 90, 2, 2, false, NULL);

-- A post with the cooked content the base stylesheets style (hr, emoji,
-- details, spoiler, footnote, checklist, code, table), cooked by Rails.
INSERT INTO topics (id, title, fancy_title, slug, user_id, last_post_user_id, archetype, category_id, created_at, updated_at, bumped_at, last_posted_at, posts_count, highest_post_number, visible) VALUES (91, 'Cooked content basics for the stylesheets', 'Cooked content basics for the stylesheets', 'cooked-content-basics-for-the-stylesheets', 1, 1, 'regular', 4, now() - interval '1 hour', now() - interval '1 hour', now() - interval '1 hour', now() - interval '1 hour', 1, 1, true);
INSERT INTO posts (id, topic_id, user_id, post_number, raw, cooked, created_at, updated_at, last_version_at, sort_order) VALUES (910, 91, 1, 1, 'Cooked basics, @user1 and @admin.

---

:smile:

<details><summary>More</summary>Hidden text</details>

[details="Open details" open]
Shown text
[/details]

[spoiler]a secret[/spoiler]

Footnote here[^1].

[^1]: The note.

[ ] unchecked
[x] checked

```ruby
def hello
  puts "hi"
end
```

| a | b |
|---|---|
| 1 | 2 |

In a list:
- item :tada:

Text with <small>small :wave:</small> and <sup>up :wave:</sup>.', '<p>Cooked basics, <a class="mention" href="/u/user1">@user1</a> and <a class="mention" href="/u/admin">@admin</a>.</p>
<hr>
<p><img src="/images/emoji/twitter/smile.png?v=15" title=":smile:" class="emoji only-emoji" alt=":smile:" loading="lazy" width="20" height="20"></p>
<details><summary>More</summary>Hidden text</details>
<details open="">
<summary>
Open details</summary>
<p>Shown text</p>
</details>
<div class="spoiler">
<p>a secret</p>
</div>
<p>Footnote here<sup class="footnote-ref"><a href="#fn1" id="fnref1">[1]</a></sup>.</p>
<p><span class="chcklst-box fa fa-square-o" data-chk-src="18:0"></span> unchecked<br>
<span class="chcklst-box checked fa fa-square-check-o" data-chk-src="19:0"></span> checked</p>
<pre data-code-wrap="ruby"><code class="lang-ruby">def hello
  puts "hi"
end
</code></pre>
<div class="md-table">
<table>
<thead>
<tr>
<th>a</th>
<th>b</th>
</tr>
</thead>
<tbody>
<tr>
<td>1</td>
<td>2</td>
</tr>
</tbody>
</table>
</div><p>In a list:</p>
<ul>
<li>item <img src="/images/emoji/twitter/tada.png?v=15" title=":tada:" class="emoji" alt=":tada:" loading="lazy" width="20" height="20"></li>
</ul>
<p>Text with <small>small <img src="/images/emoji/twitter/wave.png?v=15" title=":wave:" class="emoji" alt=":wave:" loading="lazy" width="20" height="20"></small> and <sup>up <img src="/images/emoji/twitter/wave.png?v=15" title=":wave:" class="emoji" alt=":wave:" loading="lazy" width="20" height="20"></sup>.</p>
<hr class="footnotes-sep">

<ol class="footnotes-list">
<li id="fn1" class="footnote-item"><p>The note. <a href="#fnref1" class="footnote-backref">↩︎</a></p>
</li>
</ol>', now() - interval '1 hour', now() - interval '1 hour', now() - interval '1 hour', 1);
