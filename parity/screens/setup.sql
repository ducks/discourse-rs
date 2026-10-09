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
