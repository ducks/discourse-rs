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
  ('assign_enabled', 5, 't', now(), now())
ON CONFLICT (name) DO UPDATE SET value = EXCLUDED.value;

-- user1 writes in the markdown editor (composition_mode 0), so the
-- composer draws its preview; the seed has everyone on the rich editor,
-- which the port draws as the textarea alone (196).
UPDATE user_options SET composition_mode = 0
WHERE user_id = (SELECT id FROM users WHERE username = 'user1');
