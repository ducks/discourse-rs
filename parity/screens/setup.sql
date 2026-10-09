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
