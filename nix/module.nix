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
  share = "${cfg.package}/share/discourse-rs";
  pg = config.services.postgresql.package;
  # Discourse.base_url, stored with a fresh site's VAPID keys.
  baseUrl = "${if cfg.forceHttps then "https" else "http"}://${cfg.hostname}";
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
        description = "The local database's name.";
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
          With `createLocally`, load the vendored schema
          (vendor/discourse/db/structure.sql) and seeds (seed/*.sql) into
          the database when it has no tables yet, as scripts/db-load does:
          a fresh site. discourse-rs-db-init.service does it before the
          server starts. In the same transaction it gives the site secrets
          of its own (the seed's are in the repo), sets the admin's password
          from INITIAL_ADMIN_PASSWORD in `environmentFile` (required; the
          load fails without it) and its address from INITIAL_ADMIN_EMAIL
          when set, and locks the seed's fixture users out. Turn off to
          restore a backup into the empty database instead.
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
        external DATABASE_URL, and for a fresh site INITIAL_ADMIN_PASSWORD
        (at least 15 characters) and optionally INITIAL_ADMIN_EMAIL, which
        discourse-rs-db-init reads once. Read by systemd as root, so it can be
        root:root 0600; it never enters the Nix store (give a path as a
        string, not a Nix path literal). Without DISCOURSE_SECRET_KEY_BASE
        sessions die with every restart.
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
        DISCOURSE_DB_POOL = "20";
      };
      description = ''
        More environment for the server, over the module's own. Every
        DISCOURSE_* variable is a GlobalSetting. Not for secrets: these
        land in the Nix store.
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

    # The role and database, and on an empty database the schema and seeds
    # (scripts/db-load). Runs as the superuser on every start and only does
    # what is missing. The extensions are made as the superuser (vector is
    # untrusted); the schema as the service's role, so it owns the tables,
    # as Rails' database user does; the seeds as the superuser, since
    # their DISABLE TRIGGER ALL covers foreign key triggers. One
    # transaction: a failed load leaves the database empty for the next
    # try, not half loaded. Its last part, from discourse-rs-fresh-site,
    # replaces the seed's public secrets and admin password and locks the
    # fixture users out, so a loaded site never has the seed's credentials.
    systemd.services.discourse-rs-db-init = mkIf cfg.database.createLocally {
      description = "discourse-rs database setup";
      requires = [ "postgresql.service" ];
      after = [ "postgresql.service" ];
      path = [
        pg
        pkgs.gnugrep
      ];
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
        User = "postgres";
        Group = "postgres";
        RuntimeDirectory = "discourse-rs-db-init";
        # INITIAL_ADMIN_PASSWORD for a fresh site.
        EnvironmentFile = optional (cfg.environmentFile != null) cfg.environmentFile;
      };
      script = ''
        set -euo pipefail
        psql() { command psql -X -q -v ON_ERROR_STOP=1 "$@"; }
        db=${lib.escapeShellArg cfg.database.name}

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
          # The seed's admin password and secrets are public: a fresh site
          # gets its own, in the load's transaction, or nothing is loaded.
          if [ -z "''${INITIAL_ADMIN_PASSWORD:-}" ]; then
            echo "INITIAL_ADMIN_PASSWORD is not set in services.discourse-rs.environmentFile; a fresh site needs the admin's password. Nothing loaded." >&2
            exit 1
          fi
          umask 077
          ${cfg.package}/bin/discourse-rs-fresh-site ${lib.escapeShellArg baseUrl} \
            > "$RUNTIME_DIRECTORY/secure.sql"
          echo "loading the vendored schema and seeds into $db"
          seeds=()
          for seed in ${share}/seed/*.sql; do
            seeds+=(-f "$seed")
          done
          grep '^CREATE EXTENSION' ${share}/db/structure.sql > "$RUNTIME_DIRECTORY/extensions.sql"
          psql -d "$db" --single-transaction \
            -f "$RUNTIME_DIRECTORY/extensions.sql" \
            -c 'SET ROLE "${user}"' \
            -f ${share}/db/structure.sql \
            -c 'RESET ROLE' \
            "''${seeds[@]}" \
            -f "$RUNTIME_DIRECTORY/secure.sql" \
            -o /dev/null
          rm -f "$RUNTIME_DIRECTORY/secure.sql"
          echo "loaded $db: $(psql -d "$db" -Atc "SELECT count(*) FROM pg_tables WHERE schemaname = 'public'") tables ($(sed -n 's/^commit=//p' ${share}/DISCOURSE_REF | cut -c1-11))"
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
        RAILS_ENV = "production";
        DISCOURSE_HOSTNAME = cfg.hostname;
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
      // optionalAttrs cfg.forceHttps { DISCOURSE_FORCE_HTTPS = "true"; }
      // optionalAttrs (!cfg.jobs.enable) { DISCOURSE_RS_JOBS = "off"; }
      // cfg.extraEnvironment;

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
