-- Custom groups for the group pages (parity/screens/groups-pages), run on
-- both sides with `scripts/parity-screenshots --setup
-- parity/screens/groups-setup.sql --pages parity/screens/groups-pages`.
-- The same groups as the parity/writes/groups read cases.

INSERT INTO groups (id, name, full_name, bio_raw, bio_cooked, visibility_level, members_visibility_level, public_admission, public_exit, title, flair_icon, flair_bg_color, flair_color, mentionable_level, messageable_level, user_count, created_at, updated_at)
VALUES (9001, 'testers', 'test team', 'We **test** things. See https://example.com', '<p>We <strong>test</strong> things. See <a href="https://example.com">https://example.com</a></p>', 0, 0, TRUE, TRUE, 'Tester', 'heart', 'ff0000', '00ff00', 99, 99, 3, '2026-10-01 10:00:00', '2026-10-01 10:00:00');
INSERT INTO groups (id, name, visibility_level, members_visibility_level, allow_membership_requests, membership_request_template, mentionable_level, messageable_level, user_count, created_at, updated_at)
VALUES (9002, 'members_only', 2, 2, TRUE, 'Why?', 3, 3, 1, '2026-10-01 10:00:00', '2026-10-01 10:00:00');
INSERT INTO groups (id, name, full_name, visibility_level, members_visibility_level, mentionable_level, messageable_level, default_notification_level, user_count, created_at, updated_at)
VALUES (9003, 'hidden_members', 'Hidden Members', 1, 4, 4, 1, 1, 2, '2026-10-01 10:00:00', '2026-10-01 10:00:00');
INSERT INTO group_users (id, group_id, user_id, owner, notification_level, created_at, updated_at)
VALUES (90001, 9001, 1, TRUE, 3, '2026-10-01 10:00:00', '2026-10-01 10:00:00'),
       (90002, 9001, 3, FALSE, 3, '2026-10-01 11:00:00', '2026-10-01 11:00:00'),
       (90003, 9001, 4, FALSE, 3, '2026-10-01 12:00:00', '2026-10-01 12:00:00'),
       (90004, 9002, 3, TRUE, 3, '2026-10-01 10:00:00', '2026-10-01 10:00:00'),
       (90005, 9003, 2, FALSE, 1, '2026-10-01 10:00:00', '2026-10-01 10:00:00'),
       (90006, 9003, 4, TRUE, 1, '2026-10-01 10:00:00', '2026-10-01 10:00:00');
INSERT INTO group_requests (id, group_id, user_id, reason, created_at, updated_at)
VALUES (9001, 9002, 2, 'Let me in', '2026-10-02 10:00:00', '2026-10-02 10:00:00');
