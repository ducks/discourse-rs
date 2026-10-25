# services.discourse-rs: the server as deploy/discourse-rs.service runs
# it, its database on the host's PostgreSQL (or an external one), and
# optionally Caddy in front as deploy/Caddyfile has it.
#
# The flake's nixosModules.default imports this and sets `package` and
# `assets` to its own builds.
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.services.discourse-rs;
  inherit (lib)
    mkEnableOption
    mkIf
    mkOption
    optional
    optionalAttrs
    types
    ;

  user = "discourse-rs";
  stateDir = "/var/lib/discourse-rs";
  localDatabaseUrl = "postgresql:///${cfg.database.name}?host=/run/postgresql";
  pg = config.services.postgresql.package;
  # The environment the server and its one-off commands share: what makes
  # up Discourse.base_url and the globals.
  siteEnvironment = {
    RAILS_ENV = "production";
    DISCOURSE_HOSTNAME = cfg.hostname;
  }
  // optionalAttrs cfg.forceHttps { DISCOURSE_FORCE_HTTPS = "true"; }
  // optionalAttrs cfg.loginRequired { DISCOURSE_LOGIN_REQUIRED = "true"; }
  // optionalAttrs cfg.inviteOnly { DISCOURSE_INVITE_ONLY = "true"; }
  // optionalAttrs (cfg.database.pool != null) { DISCOURSE_DB_POOL = toString cfg.database.pool; }
  // optionalAttrs (cfg.database.statementLimit != null) {
    DISCOURSE_DB_STATEMENT_LIMIT = toString cfg.database.statementLimit;
  }
  // cfg.extraEnvironment;
  hasInitialAdmin = cfg.initialAdmin.email != null;
in
{
  options.services.discourse-rs = {
    enable = mkEnableOption "discourse-rs, a Rust port of Discourse";

    package = mkOption {
      type = types.package;
      description = "The discourse-rs package.";
    };

    assets = mkOption {
      type = types.package;
      description = ''
        What DISCOURSE_SRC points at: Discourse's stock images, the
        discourse-fonts and discourse-emojis gems (the flake's
        discourse-assets).
      '';
    };

    hostname = mkOption {
      type = types.str;
      example = "forum.example.com";
      description = "DISCOURSE_HOSTNAME: the host name in generated URLs, and Caddy's site.";
    };

    forceHttps = mkOption {
      type = types.bool;
      default = true;
      description = ''
        DISCOURSE_FORCE_HTTPS: https in generated URLs and Secure on the
        session cookie. Leave on behind Caddy. Off leaves it to the
        force_https site setting (a backup of a site that had it on
        needs nothing here).
      '';
    };

    listenAddress = mkOption {
      type = types.str;
      default = "127.0.0.1";
      description = ''
        Address the server binds to (BIND_ADDR). Keep it on loopback: the
        server trusts X-Forwarded-For, so only the proxy may reach it.
      '';
    };

    port = mkOption {
      type = types.port;
      default = 8080;
      description = "Port the server binds to (BIND_ADDR).";
    };

    database = {
      createLocally = mkOption {
        type = types.bool;
        default = true;
        description = ''
          Run PostgreSQL 16 with pgvector on this host and create the
          database, owned by the discourse-rs role, which the service
          reaches over the local socket with peer auth. Off: set
          `database.url`, or DATABASE_URL in `environmentFile` when the
          URL carries a password.
        '';
      };

      name = mkOption {
        type = types.str;
        default = "discourse";
        description = ''
          The local database's name. discourse-rs-db-init creates it with
          the en_US.UTF-8 locale, the reference's collation (as
          scripts/db-load does), so the host must have that locale in
          `i18n.supportedLocales`, as it does by default; the module
          refuses a configuration without it.
        '';
      };

      url = mkOption {
        type = types.nullOr types.str;
        default = null;
        example = "postgresql://discourse-rs@db.internal/discourse";
        description = ''
          DATABASE_URL of an external database (with `createLocally` off).
          It lands in the Nix store: keep a password out of it and put the
          whole URL in `environmentFile` instead.
        '';
      };

      loadSchema = mkOption {
        type = types.bool;
        default = true;
        description = ''
          With `createLocally`, make a fresh site in the database while it
          has no tables: discourse-rs-db-init.service runs `discourse-rs
          setup` (the vendored schema and what Discourse's db:seed creates,
          in one transaction, owned by the discourse-rs role) before the
          server starts, and then `discourse-rs admin create` for
          `initialAdmin`. Turn off to restore a backup into the empty
          database instead.
        '';
      };

      pool = mkOption {
        type = types.nullOr types.ints.positive;
        default = null;
        example = 4;
        description = ''
          DISCOURSE_DB_POOL: connections the server's pool may open; null
          keeps the server's default of 10. The rate limiter's 4 and
          pg-bus's listener come on top. Lower it to fit several forums
          on one PostgreSQL: at 4 a load test lost about 8% throughput.
        '';
      };

      statementLimit = mkOption {
        type = types.nullOr types.ints.positive;
        default = null;
        example = 1000;
        description = ''
          DISCOURSE_DB_STATEMENT_LIMIT: prepared statements each connection
          keeps; null keeps the server's default of 250. Their plans live
          in the PostgreSQL backend: under load a busy forum's backends
          held about 240 MB at 250 and 700 MB at 1000. Below about 150 the
          topic page re-plans on every request.
        '';
      };
    };

    initialAdmin = {
      email = mkOption {
        type = types.nullOr types.str;
        default = null;
        example = "you@example.com";
        description = ''
          The first admin's email. While the site has no admin,
          discourse-rs-db-init runs `discourse-rs admin create` for it
          (as `rake admin:create`: a new account, activated, granted
          admin), with the password from `initialAdmin.passwordFile`.
          Without it, register the admin on the finish installation page
          (DISCOURSE_DEVELOPER_EMAILS and SMTP), or run `discourse-rs
          admin create` yourself.
        '';
      };

      passwordFile = mkOption {
        type = types.nullOr types.str;
        default = null;
        example = "/run/keys/discourse-admin-password";
        description = ''
          A file holding the first admin's password (at least
          min_password_length, 10, characters; min_admin_password_length,
          15, for a DISCOURSE_DEVELOPER_EMAILS address, as Rails checks
          it), handed to
          discourse-rs-db-init as a systemd credential (LoadCredential), so
          it can be root:root 0600 and never enters the Nix store or the
          environment. A string, not a Nix path literal.
        '';
      };

      username = mkOption {
        type = types.nullOr types.str;
        default = null;
        example = "siteowner";
        description = ''
          The first admin's username, checked as a signup's. Without it
          the account gets the one UserNameSuggester makes from the email,
          as `rake admin:create` does.
        '';
      };
    };

    environmentFile = mkOption {
      type = types.nullOr types.path;
      default = null;
      example = "/etc/discourse-rs/env";
      description = ''
        An EnvironmentFile for the secrets: DISCOURSE_SECRET_KEY_BASE,
        DISCOURSE_SMTP_ADDRESS / _PORT / _USER_NAME / _PASSWORD, an
        external DATABASE_URL. Read by systemd as root, so it can be
        root:root 0600; it never enters the Nix store (give a path as a
        string, not a Nix path literal). Without DISCOURSE_SECRET_KEY_BASE
        the server generates one and keeps it in the database, as Rails
        keeps it in Redis.
      '';
    };

    jobs.enable = mkOption {
      type = types.bool;
      default = true;
      description = ''
        Run the job worker in the server process (Sidekiq's role): post
        processing, notifications, email, the scheduled jobs. Off sets
        DISCOURSE_RS_JOBS=off and leaves the queue to another process.
      '';
    };

    letterAvatarFont = mkOption {
      type = types.nullOr types.path;
      default = pkgs.fetchurl {
        url = "https://raw.githubusercontent.com/ArtifexSoftware/urw-base35-fonts/20200910/fonts/NimbusSans-Regular.otf";
        sha256 = "020ddndnqlyxgc1xgjmvvbvpyrahf4ki0nxq18426m8mg16vw9bw";
      };
      defaultText = lib.literalMD "NimbusSans-Regular.otf from urw-base35-fonts, as shell.nix has it";
      description = ''
        LETTER_AVATAR_FONT: the font letter avatars are drawn with when
        the external_system_avatars_url site setting is blank.
      '';
    };

    extraEnvironment = mkOption {
      type = types.attrsOf types.str;
      default = { };
      example = {
        RUST_LOG = "info,discourse_rs=debug";
        DISCOURSE_MAX_REQS_PER_IP_MODE = "warn";
      };
      description = ''
        More environment for the server, over the module's own. Every
        DISCOURSE_* variable is a GlobalSetting. Not for secrets: these
        land in the Nix store.
      '';
    };

    loginRequired = mkOption {
      type = types.bool;
      default = false;
      description = ''
        Sets DISCOURSE_LOGIN_REQUIRED=true: the login_required site
        setting, shadowed as Rails' GlobalSetting shadows it, so only
        signed-in members see the forum and the admin can't turn it off.
        Off leaves it to the site setting.
      '';
    };

    inviteOnly = mkOption {
      type = types.bool;
      default = false;
      description = ''
        Sets DISCOURSE_INVITE_ONLY=true: the invite_only site setting,
        shadowed, so accounts come only from invites. Off leaves it to the
        site setting.
      '';
    };

    caddy.enable = mkOption {
      type = types.bool;
      default = false;
      description = ''
        A services.caddy virtual host for `hostname` proxying to the
        server, as deploy/Caddyfile: compression, X-Forwarded-For from the
        connection, TLS from Let's Encrypt. Open ports 80 and 443 in the
        firewall yourself.
      '';
    };
  };

  config = mkIf cfg.enable {
    assertions = [
      {
        assertion = cfg.database.createLocally -> cfg.database.url == null;
        message = "services.discourse-rs.database.url is for an external database; turn createLocally off to use it.";
      }
      {
        assertion =
          !cfg.database.createLocally -> (cfg.database.url != null || cfg.environmentFile != null);
        message = "services.discourse-rs: an external database needs database.url, or DATABASE_URL in environmentFile.";
      }
      {
        assertion =
          cfg.database.createLocally
          -> (
            builtins.elem "all" config.i18n.supportedLocales
            || builtins.elem "en_US.UTF-8/UTF-8" config.i18n.supportedLocales
          );
        message = ''services.discourse-rs: the local database is created with the en_US.UTF-8 locale; add "en_US.UTF-8/UTF-8" to i18n.supportedLocales.'';
      }
      {
        assertion = hasInitialAdmin -> cfg.initialAdmin.passwordFile != null;
        message = "services.discourse-rs.initialAdmin.email needs initialAdmin.passwordFile.";
      }
      {
        assertion = hasInitialAdmin -> (cfg.database.createLocally && cfg.database.loadSchema);
        message = "services.discourse-rs.initialAdmin is made by discourse-rs-db-init, which needs database.createLocally and database.loadSchema; run `discourse-rs admin create` yourself otherwise.";
      }
    ];

    users.users.${user} = {
      isSystemUser = true;
      group = user;
      home = stateDir;
      description = "discourse-rs";
    };
    users.groups.${user} = { };

    services.postgresql = mkIf cfg.database.createLocally {
      enable = true;
      # Over the module's stateVersion default, under a host's own choice.
      package = lib.mkOverride 900 pkgs.postgresql_16;
      # structure.sql creates hstore, pg_trgm, unaccent (contrib) and
      # vector (pgvector, for the bundled discourse-ai tables).
      extensions = ps: [ ps.pgvector ];
    };

    # The role and database, and on an empty database a fresh site
    # (`discourse-rs setup`, Discourse's db:migrate on a new install), then
    # the first admin while there is none (`discourse-rs admin create`).
    # Runs as the superuser on every start and only does what is missing.
    # setup creates the extensions as the superuser (vector is untrusted)
    # and the rest as the service's role (--owner), so it owns the tables,
    # as Rails' database user does, in one transaction: a failed load
    # leaves the database empty for the next try. admin create runs as the
    # service's role too (PGOPTIONS), with the password as a credential.
    systemd.services.discourse-rs-db-init = mkIf cfg.database.createLocally {
      description = "discourse-rs database setup";
      requires = [ "postgresql.service" ];
      after = [ "postgresql.service" ];
      path = [ pg ];
      # The globals setup and admin create read (the base url, the
      # developer emails), as the server has them.
      environment = siteEnvironment // {
        DATABASE_URL = localDatabaseUrl;
        RUST_LOG = "warn";
      };
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
        User = "postgres";
        Group = "postgres";
        EnvironmentFile = optional (cfg.environmentFile != null) cfg.environmentFile;
        LoadCredential = optional (
          hasInitialAdmin && cfg.initialAdmin.passwordFile != null
        ) "initial-admin-password:${cfg.initialAdmin.passwordFile}";
      };
      script = ''
        set -euo pipefail
        psql() { command psql -X -q -v ON_ERROR_STOP=1 "$@"; }
        db=${lib.escapeShellArg cfg.database.name}
        # The local database, whatever the environment file says.
        export DATABASE_URL=${lib.escapeShellArg localDatabaseUrl}

        if [ "$(psql -d postgres -Atc "SELECT 1 FROM pg_roles WHERE rolname = '${user}'")" != 1 ]; then
          psql -d postgres -c 'CREATE ROLE "${user}" LOGIN'
        fi
        if [ "$(psql -d postgres -Atc "SELECT 1 FROM pg_database WHERE datname = '$db'")" != 1 ]; then
          # The reference's collation, as scripts/db-load creates it.
          psql -d postgres -c "CREATE DATABASE \"$db\" OWNER \"${user}\" TEMPLATE template0 ENCODING 'UTF8' LOCALE 'en_US.UTF-8'"
        fi
      ''
      + lib.optionalString cfg.database.loadSchema ''
        tables="$(psql -d "$db" -Atc "SELECT count(*) FROM pg_tables WHERE schemaname = 'public'")"
        if [ "$tables" = 0 ]; then
          echo "setting up a fresh site in $db"
          ${lib.getExe cfg.package} setup --owner ${user}
        fi
      ''
      + lib.optionalString (cfg.database.loadSchema && hasInitialAdmin) ''
        admins="$(psql -d "$db" -Atc "SELECT count(*) FROM users WHERE admin AND id > 0")"
        if [ "$admins" = 0 ]; then
          PGOPTIONS='-c role=${user}' ${lib.getExe cfg.package} admin create \
            --email ${lib.escapeShellArg cfg.initialAdmin.email}${
              lib.optionalString (cfg.initialAdmin.username != null)
                " --username ${lib.escapeShellArg cfg.initialAdmin.username}"
            } \
            --password-file "$CREDENTIALS_DIRECTORY/initial-admin-password"
        fi
      '';
    };
    systemd.services.discourse-rs = {
      description = "discourse-rs";
      wantedBy = [ "multi-user.target" ];
      wants = [ "network-online.target" ];
      after = [
        "network-online.target"
        "postgresql.service"
      ]
      ++ optional cfg.database.createLocally "discourse-rs-db-init.service";
      requires = optional cfg.database.createLocally "discourse-rs-db-init.service";

      environment = {
        BIND_ADDR = "${
          if lib.hasInfix ":" cfg.listenAddress then "[${cfg.listenAddress}]" else cfg.listenAddress
        }:${toString cfg.port}";
        DISCOURSE_SRC = "${cfg.assets}";
        # The backup's uploads/ go here; the local store writes uploads here.
        PUBLIC_DIR = "${stateDir}/public";
        # Backups and restores are staged here, and the letter avatar cache.
        TMP_DIR = "${stateDir}/tmp";
        RUST_LOG = "info";
      }
      // optionalAttrs cfg.database.createLocally { DATABASE_URL = localDatabaseUrl; }
      // optionalAttrs (cfg.database.url != null) { DATABASE_URL = cfg.database.url; }
      // optionalAttrs (cfg.letterAvatarFont != null) { LETTER_AVATAR_FONT = "${cfg.letterAvatarFont}"; }
      // optionalAttrs (!cfg.jobs.enable) { DISCOURSE_RS_JOBS = "off"; }
      // siteEnvironment;

      serviceConfig = {
        Type = "simple";
        User = user;
        Group = user;
        EnvironmentFile = optional (cfg.environmentFile != null) cfg.environmentFile;
        ExecStart = lib.getExe cfg.package;
        WorkingDirectory = stateDir;
        Restart = "on-failure";
        RestartSec = 2;

        # The server writes under /var/lib/discourse-rs (uploads, backups,
        # TMP_DIR); the rest of the system is read only to it.
        StateDirectory = [
          "discourse-rs"
          "discourse-rs/public"
          "discourse-rs/tmp"
        ];
        StateDirectoryMode = "0750";
        NoNewPrivileges = true;
        PrivateTmp = true;
        PrivateDevices = true;
        ProtectHome = true;
        ProtectSystem = "strict";
        ProtectKernelTunables = true;
        ProtectControlGroups = true;
        RestrictAddressFamilies = [
          "AF_INET"
          "AF_INET6"
          "AF_UNIX"
        ];
      };
    };

    services.caddy = mkIf cfg.caddy.enable {
      enable = true;
      # Caddy sets X-Forwarded-For from the connection and drops a client's
      # own value; it passes the Host through.
      virtualHosts.${cfg.hostname}.extraConfig = ''
        encode zstd gzip
        reverse_proxy ${
          if lib.hasInfix ":" cfg.listenAddress then "[${cfg.listenAddress}]" else cfg.listenAddress
        }:${toString cfg.port}
      '';
    };
  };
}
