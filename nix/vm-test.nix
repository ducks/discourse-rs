# The NixOS module end to end: a VM with a local PostgreSQL, a fresh site
# made by discourse-rs-db-init (`discourse-rs setup`, then `discourse-rs
# admin create` from a credential), the server with its job worker, Caddy
# in front, then the team-only options switched on.
# Run: nix build .#checks.x86_64-linux.vm -L
{ pkgs, module }:

let
  adminEmail = "Admin@Forum.Test";
  adminPassword = "vm-test admin password";
in
pkgs.testers.runNixOSTest {
  name = "discourse-rs";

  nodes.machine =
    { pkgs, ... }:
    {
      imports = [ module ];

      virtualisation.memorySize = 2048;
      virtualisation.diskSize = 4096;

      services.discourse-rs = {
        enable = true;
        # Caddy's internal CA serves localhost, so the proxy is tested
        # without ACME.
        hostname = "localhost";
        forceHttps = false;
        environmentFile = "/etc/discourse-rs/env";
        initialAdmin = {
          email = adminEmail;
          username = "siteowner";
          passwordFile = "/etc/discourse-rs/admin-password";
        };
        caddy.enable = true;
        database.pool = 4;
        database.statementLimit = 100;
      };

      specialisation.team-only.configuration.services.discourse-rs = {
        loginRequired = true;
        inviteOnly = true;
      };

      # Test secrets; a real host keeps these files out of the store.
      environment.etc."discourse-rs/env" = {
        mode = "0600";
        text = ''
          DISCOURSE_SECRET_KEY_BASE=${builtins.hashString "sha512" "discourse-rs vm test"}
        '';
      };
      environment.etc."discourse-rs/admin-password" = {
        mode = "0600";
        text = adminPassword;
      };

      # scripts/db-chown, tested on a superuser's load below.
      environment.etc."discourse-rs-test/db-chown".source = ../scripts/db-chown;

      environment.systemPackages = [
        pkgs.curl
        pkgs.jq
      ];
    };

  testScript =
    { nodes, ... }:
    let
      package = nodes.machine.services.discourse-rs.package;
    in
    ''
      import json
      import shlex

      base = "http://localhost:8080"
      script = "-H 'X-Requested-With: XMLHttpRequest' -H 'Accept: application/json'"
      exe = "${package}/bin/discourse-rs"

      def status(path):
          return machine.succeed(f"curl -s -o /dev/null -w '%{{http_code}}' '{base}{path}'").strip()

      def expect_200(path):
          code = status(path)
          assert code == "200", f"{path}: {code}"

      def sql(query, db="discourse"):
          return machine.succeed(
              f"sudo -u postgres psql -X -At -d {db} -c {shlex.quote(query)}"
          ).strip()

      def setting(name, db="discourse"):
          return sql(f"SELECT value FROM site_settings WHERE name = '{name}'", db)

      def login(username, password, jar):
          """POST /session as the login form's script does; the reply."""
          cookies = f"-c {jar} -b {jar}"
          csrf = json.loads(machine.succeed(f"curl -sf {cookies} {script} {base}/session/csrf.json"))["csrf"]
          auth = f"{cookies} {script} -H 'X-CSRF-Token: {csrf}'"
          reply = machine.succeed(
              f"curl -s {auth} --data-urlencode login={shlex.quote(username)} "
              f"--data-urlencode password={shlex.quote(password)} {base}/session"
          )
          return auth, json.loads(reply)

      machine.wait_for_unit("discourse-rs-db-init.service")
      machine.wait_for_unit("discourse-rs.service")
      machine.wait_for_open_port(8080)

      with subtest("setup loaded the schema and db:seed, owned by the service role"):
          tables = int(sql("SELECT count(*) FROM pg_tables WHERE schemaname = 'public'"))
          assert 380 < tables < 400, tables
          assert int(sql("SELECT count(*) FROM schema_migrations")) > 2000
          assert sql(
              "SELECT string_agg(DISTINCT tableowner, ',') FROM pg_tables WHERE schemaname IN ('public', 'discourse_rs')"
          ) == "discourse-rs"
          assert sql("SELECT pg_get_userbyid(nspowner) FROM pg_namespace WHERE nspname = 'discourse_rs'") == "discourse-rs"
          assert sql("SELECT string_agg(name, ',' ORDER BY id) FROM categories") == "Uncategorized,Site Feedback,Staff,General"
          machine.succeed("journalctl -u discourse-rs-db-init.service -b | grep -q 'setting up a fresh site in discourse'")

      with subtest("no test users: db:seed's and the admin from the credential"):
          users = sql("SELECT string_agg(id || ':' || username, ',' ORDER BY id) FROM users")
          assert users == "-2:discobot,-1:system,1:siteowner", users
          assert sql("SELECT count(*) FROM users WHERE id > 0") == "1", users
          assert sql("SELECT count(*) FROM users WHERE username IN ('admin', 'user0', 'user1', 'user2', 'user3') AND id <> 1") == "0"
          assert sql("SELECT email FROM user_emails WHERE user_id = 1 AND \"primary\"") == "admin@forum.test"
          assert sql("SELECT admin AND active AND approved FROM users WHERE id = 1") == "t"
          # admin create cleared the finish installation notice.
          assert setting("has_login_hint") == "f"
          # The credential, not the environment, carried the password.
          env = machine.succeed("systemctl show -p Environment -p EnvironmentFiles discourse-rs-db-init.service")
          assert "INITIAL_ADMIN" not in env and "${adminPassword}" not in env, env

      with subtest("the pool and statement limit reach the server"):
          env = machine.succeed("systemctl show -p Environment discourse-rs.service")
          assert "DISCOURSE_DB_POOL=4" in env and "DISCOURSE_DB_STATEMENT_LIMIT=100" in env, env

      with subtest("the secrets are the site's own"):
          assert setting("vapid_base_url") == "http://localhost"
          assert len(setting("vapid_public_key")) == 88
          assert len(setting("push_api_secret_key")) == 32
          # noreply@localhost is not a valid address: left at its default.
          assert setting("notification_email") == ""
          # The parity fixture's keys are nowhere.
          assert sql("SELECT count(*) FROM site_settings WHERE value IN ("
                     "'BAGOjc4U2dZziyL8MGXELb1RmtazJPY65gEjFCMbQOBfjv4kOA-PsxrJFi-BBJAAF5aan-Z_OWuT4wchuDZLnTQ=', "
                     "'Pq2dkgm-AKMs_9pGwvBu4VeVd33iCtV0nRjkMLa-2NU=', '96b10b7ce1e537c0a1213353b2a30558')") == "0"

      with subtest("another fresh site gets keys of its own"):
          machine.succeed("sudo -u postgres createdb -O discourse-rs -T template0 -E UTF8 --locale=en_US.UTF-8 second")
          machine.succeed(
              f"sudo -u postgres env DATABASE_URL='postgresql:///second?host=/run/postgresql' "
              f"RAILS_ENV=production DISCOURSE_HOSTNAME=localhost {exe} setup --owner discourse-rs"
          )
          for name in ["vapid_public_key", "vapid_private_key", "push_api_secret_key"]:
              first, second = setting(name), setting(name, "second")
              assert first and second and first != second, f"{name}: {first} {second}"
          # Without an admin it shows the finish installation page.
          assert setting("has_login_hint", "second") == "t"
          # Setup refuses a database with tables.
          machine.fail(
              f"sudo -u postgres env DATABASE_URL='postgresql:///second?host=/run/postgresql' {exe} setup"
          )
          machine.succeed("sudo -u postgres dropdb second")

      with subtest("the env file's secret is read"):
          machine.fail("journalctl -u discourse-rs.service | grep 'DISCOURSE_SECRET_KEY_BASE is invalid'")
          assert sql("SELECT count(*) FROM discourse_rs.persistent_values WHERE key = 'SECRET_TOKEN'") == "0"

      with subtest("pages"):
          expect_200("/latest")
          expect_200("/t/welcome-to-discourse/5")
          # There is an admin: the finish installation page is gone.
          assert status("/finish-installation") == "403"

      with subtest("assets from discourse-assets"):
          expect_200("/images/discourse-logo-sketch.png")
          expect_200("/images/wizard/tada.svg")
          expect_200("/images/emoji/twitter/smile.png?v=15")
          expect_200("/fonts/Roboto-Regular.woff2?v=0.0.19")

      with subtest("Caddy proxies to the server"):
          machine.wait_for_unit("caddy.service")
          machine.wait_for_open_port(443)
          machine.wait_until_succeeds(
              "curl -sfk -o /dev/null https://localhost/latest", timeout=60
          )

      with subtest("the admin from the credential logs in"):
          _, reply = login("admin@forum.test", "password", "/tmp/jar-wrong")
          assert "error" in reply, reply
          auth, reply = login("${adminEmail}", "${adminPassword}", "/tmp/jar")
          assert "error" not in reply, reply
          current = json.loads(machine.succeed(f"curl -sf -c /tmp/jar -b /tmp/jar {script} {base}/session/current.json"))
          assert current["current_user"]["id"] == 1, current
          assert current["current_user"]["admin"], current

      with subtest("a reply is processed by the job worker"):
          # The first emoji outside a quote earns the First Emoji badge
          # (41), which only the process_post job grants.
          assert sql("SELECT count(*) FROM user_badges WHERE user_id = 1 AND badge_id = 41") == "0"
          post = json.loads(machine.succeed(
              f"curl -sf {auth} --data-urlencode topic_id=5 "
              "--data-urlencode 'raw=A reply from the NixOS VM test :smile:' "
              f"{base}/posts.json"
          ))
          post_id = post.get("post", post)["id"]
          machine.wait_until_succeeds(
              "test \"$(sudo -u postgres psql -X -At -d discourse -c "
              "'SELECT count(*) FROM user_badges WHERE user_id = 1 AND badge_id = 41')\" = 1",
              timeout=120,
          )
          assert sql(
              f"SELECT count(*) FROM discourse_rs.jobs WHERE name = 'process_post' AND args->>'post_id' = '{post_id}'"
          ) == "0", sql("SELECT name, last_error FROM discourse_rs.jobs WHERE last_error IS NOT NULL")

      with subtest("the units verify"):
          machine.succeed("systemd-analyze verify /etc/systemd/system/discourse-rs.service")
          machine.succeed("systemd-analyze verify /etc/systemd/system/discourse-rs-db-init.service")

      with subtest("a restart loads nothing again"):
          vapid = setting("vapid_public_key")
          machine.succeed("systemctl restart discourse-rs-db-init.service")
          machine.succeed("systemctl restart discourse-rs.service")
          machine.wait_for_open_port(8080)
          machine.succeed("journalctl -u discourse-rs-db-init.service -b | grep -c 'setting up a fresh site' | grep -qx 1")
          assert setting("vapid_public_key") == vapid
          assert sql("SELECT count(*) FROM users WHERE id > 0") == "1"

      with subtest("db-chown hands a superuser's load to the service"):
          share = "${package}/share/discourse-rs"
          machine.succeed("sudo -u postgres createdb -T template0 restored")
          machine.succeed(
              f"sudo -u postgres psql -X -q -v ON_ERROR_STOP=1 -d restored -f {share}/db/structure.sql -o /dev/null"
          )
          owners = lambda: machine.succeed(
              "sudo -u postgres psql -X -At -d restored -c "
              + shlex.quote(
                  "SELECT (SELECT tableowner FROM pg_tables WHERE tablename = 'posts') || ' ' || "
                  "(SELECT pg_get_userbyid(nspowner) FROM pg_namespace WHERE nspname = 'discourse_functions') || ' ' || "
                  "(SELECT string_agg(DISTINCT pg_get_userbyid(proowner), ',') FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace "
                  "WHERE n.nspname = 'discourse_functions') || ' ' || "
                  "(SELECT pg_get_userbyid(typowner) FROM pg_type WHERE typname = 'hotlinked_media_status') || ' ' || "
                  "(SELECT pg_get_userbyid(relowner) FROM pg_class WHERE relname = 'posts_id_seq')"
              )
          ).strip()
          assert owners() == "postgres postgres postgres postgres postgres", owners()
          machine.succeed("sudo -u postgres bash /etc/discourse-rs-test/db-chown restored discourse-rs")
          assert owners() == "discourse-rs discourse-rs discourse-rs discourse-rs discourse-rs", owners()
          assert machine.succeed(
              "sudo -u postgres psql -X -At -d postgres -c "
              "\"SELECT pg_get_userbyid(datdba) FROM pg_database WHERE datname = 'restored'\""
          ).strip() == "discourse-rs"

      with subtest("team-only: loginRequired sends the anonymous to the login"):
          machine.succeed("/run/booted-system/specialisation/team-only/bin/switch-to-configuration test")
          machine.wait_for_unit("discourse-rs.service")
          machine.wait_for_open_port(8080)
          env = machine.succeed("systemctl show -p Environment discourse-rs.service")
          assert "DISCOURSE_LOGIN_REQUIRED=true" in env and "DISCOURSE_INVITE_ONLY=true" in env, env
          answer = machine.succeed(
              f"curl -s -o /dev/null -w '%{{http_code}} %{{redirect_url}}' {base}/latest"
          ).strip()
          assert answer == "302 http://localhost:8080/login", answer
          machine.succeed("journalctl -u discourse-rs-db-init.service -b | grep -c 'setting up a fresh site' | grep -qx 1")
    '';
}
