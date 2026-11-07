-- What the browser specs share, applied by scripts/browser-test to its
-- throwaway copy of the test database before the server boots. Each
-- setting is one an admin turns on for the demo; a spec that needs a row
-- of its own makes it through the app (or helpers.js sql()) instead.
INSERT INTO site_settings (name, data_type, value, created_at, updated_at)
VALUES
  -- The interface color selector in the sidebar footer (dark mode).
  ('interface_color_selector', 7, 'sidebar_footer', now(), now()),
  -- The post menu's plugin buttons: Solved on every topic, Assign.
  ('allow_solved_on_all_topics', 5, 't', now(), now()),
  ('assign_enabled', 5, 't', now(), now()),
  -- Chat threads: the drawer's threads route and the footer's item
  -- (223c), the thread specs' threaded channels (160a); a hidden
  -- setting the admin can't save.
  ('chat_threads_enabled', 5, 't', now(), now()),
  -- The composer actions menu's whisper toggle (staff whisper) and
  -- discourse-post-voting's toggle.
  ('whispers_allowed_groups', 20, '3', now(), now()),
  ('post_voting_enabled', 5, 't', now(), now()),
  -- User status: the user menu's item, the modal, the emoji by names
  -- (user-status.spec.js sets statuses only on members of its own).
  ('enable_user_status', 5, 't', now(), now())
ON CONFLICT (name) DO UPDATE SET value = EXCLUDED.value;

-- The seed has everyone on the rich editor (Discourse's
-- default_composition_mode); the suite's members write in the markdown
-- editor (composition_mode 0), with its preview, as markdown stays the
-- port's default until the rich editor has its extensions (ROADMAP 196r5).
-- composer.spec.js opts its own members in.
UPDATE user_options SET composition_mode = 0;
