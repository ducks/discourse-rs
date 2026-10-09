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
