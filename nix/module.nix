# NixOS module for Ferron.
#
# Importable as `inputs.ferron.nixosModules.default` (see flake.nix), e.g.:
#
#   inputs.ferron.url = "github:ferronweb/ferron/3.x"; # release branch
#   modules = [ inputs.ferron.nixosModules.default ];
#
#   services.ferron = {
#     enable = true; # prebuilt release binaries by default
#     hosts."*:80" = {
#       root = "/var/www/ferron";
#       spaFallback = true;
#     };
#   };
#
# Config model: typed Nix options generate ferron.conf text
# (readable/auditable, checked by `ferron validate`), with `config` and
# `extraConfig` as verbatim escape hatches.
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.services.ferron;

  indent4 = s: "    " + builtins.replaceStrings [ "\n" ] [ "\n    " ] s;

  # Header values are always double-quoted (CSP et al. contain spaces
  # and semicolons). Backslashes and double quotes are escaped; `{{...}}`
  # interpolation passes through untouched.
  escapeHeaderValue = s: "\"" + builtins.replaceStrings [ "\\" "\"" ] [ "\\\\" "\\\"" ] s + "\"";

  headersLines =
    headers:
    (lib.mapAttrsToList (name: value: "header ${name} ${escapeHeaderValue value}") headers.set)
    ++ (lib.mapAttrsToList (name: value: "header +${name} ${escapeHeaderValue value}") headers.add)
    ++ (map (name: "header -${name}") headers.unset);

  cacheLines =
    cache:
    let
      sub =
        (lib.optional cache.litespeedOverrideCacheControl "litespeed_override_cache_control")
        ++ (lib.optional cache.emitLitespeedHeaders "emit_litespeed_headers")
        ++ (lib.optional (cache.vary != [ ]) "vary ${lib.concatStringsSep " " cache.vary}")
        ++ (lib.optional (
          cache.varyCookies != [ ]
        ) "vary_cookies ${lib.concatStringsSep " " cache.varyCookies}")
        ++ (lib.optional (cache.ignore != [ ]) "ignore ${lib.concatStringsSep " " cache.ignore}")
        ++ (lib.optional (
          cache.maxResponseSize != null
        ) "max_response_size ${toString cache.maxResponseSize}");
    in
    if !cache.enable then
      [ ]
    else if sub == [ ] then
      [ "cache" ]
    else
      [ "cache {\n${indent4 (lib.concatStringsSep "\n" sub)}\n}" ];

  hostDirectives =
    host:
    # TLS first: affects listener setup (see docs/configuration/security/tls.md).
    (lib.optional (!host.tls.enable) "tls false")
    ++ (lib.optional (
      host.tls.enable && host.tls.cert != null && host.tls.key != null
    ) "tls ${host.tls.cert} ${host.tls.key}")
    ++ (lib.optional (
      host.httpsRedirect != null
    ) "https_redirect ${lib.boolToString host.httpsRedirect}")
    ++ (lib.optional (host.root != null) "root ${host.root}")
    ++ (lib.optional (
      host.index != null && host.index != [ ]
    ) "index ${lib.concatStringsSep " " host.index}")
    ++ (cacheLines host.cache)
    ++ (lib.optional (host.proxy != null) (
      if host.proxyExtraConfig == "" then
        "proxy ${host.proxy}"
      else
        "proxy ${host.proxy} {\n${indent4 host.proxyExtraConfig}\n}"
    ))
    ++ (lib.optional (host.fcgiPhp != null) "fcgi_php ${host.fcgiPhp}")
    ++ (headersLines host.headers)
    ++ (lib.optional host.spaFallback ''
      rewrite r"^/.*" "/" {
          last
          directory false
          file false
      }'');

  hostBlock = name: host: ''
    ${name} {
    ${lib.concatStringsSep "\n" (hostDirectives host)}
    ${host.config}
    }
  '';

  generatedConf = pkgs.writeText "ferron.conf" ''
    {
    ${cfg.globalConfig}
    }

    ${lib.concatStringsSep "\n" (lib.mapAttrsToList hostBlock cfg.hosts)}

    ${cfg.extraConfig}
  '';

  confFile = if cfg.configFile != null then cfg.configFile else generatedConf;
in
{
  options.services.ferron = {
    enable = lib.mkEnableOption "Ferron web server";

    package = lib.mkOption {
      type = lib.types.package;
      default = pkgs.ferron-bin;
      defaultText = lib.literalExpression "pkgs.ferron-bin";
      description = ''
        Ferron package to run. Defaults to the prebuilt release binaries
        (fast install, PGO-optimized); use
        `inputs.ferron.packages.''${system}.ferron` for the source build
        instead (unpublished revs, forks, arches without published
        archives). Works out of the box when the flake overlay is applied;
        otherwise set explicitly.
      '';
    };

    user = lib.mkOption {
      type = lib.types.str;
      default = "ferron";
      description = "System user the service runs as.";
    };

    group = lib.mkOption {
      type = lib.types.str;
      default = "ferron";
      description = "System group the service runs as.";
    };

    globalConfig = lib.mkOption {
      type = lib.types.lines;
      default = ''
        log /var/log/ferron/access.log {
            access_log_rotate_size 10485760
            access_log_rotate_keep 7
        }
        error_log /var/log/ferron/error.log {
            error_log_rotate_size 10485760
            error_log_rotate_keep 7
        }
      '';
      description = ''
        Inner content of the global `{ ... }` block (see
        configs/ferron.pkgunix.conf and
        docs/configuration/fundamentals/syntax.md).
      '';
    };

    hosts = lib.mkOption {
      type = lib.types.attrsOf (
        lib.types.submodule {
          options.config = lib.mkOption {
            type = lib.types.lines;
            default = "";
            example = ''
              directory_listing
            '';
            description = ''
              Verbatim content appended at the end of this `<selector> { ... }`
              host block. Covers directives not (yet) modeled as typed
              options, and wins over them by position.
            '';
          };

          options.root = lib.mkOption {
            type = lib.types.nullOr lib.types.str;
            default = null;
            example = "/var/www/example";
            description = ''
              Web root (`root <path>`, see
              docs/configuration/content/static-files.md). Renders before
              `config`. Values are written bare; paths with spaces need
              quoting via `config` instead.
            '';
          };

          options.index = lib.mkOption {
            type = lib.types.nullOr (lib.types.listOf lib.types.str);
            default = null;
            example = [
              "index.html"
              "index.htm"
            ];
            description = ''
              Directory index files (`index <name>...`). Null keeps
              Ferron's default (`index.html index.htm index.xhtml`).
            '';
          };

          options.proxy = lib.mkOption {
            type = lib.types.nullOr lib.types.str;
            default = null;
            example = "http://localhost:3000";
            description = ''
              Reverse-proxy upstream shorthand (`proxy <url>`, see
              docs/configuration/proxy/reverse-proxy.md). Combine with
              `proxyExtraConfig` for per-proxy block options.
            '';
          };

          options.proxyExtraConfig = lib.mkOption {
            type = lib.types.lines;
            default = "";
            example = "request_header +X-Forwarded-Proto https";
            description = ''
              Verbatim lines inside the `proxy <url> { ... }` block.
              Empty renders the one-line `proxy <url>` shorthand.
            '';
          };

          options.tls = {
            enable = lib.mkOption {
              type = lib.types.bool;
              default = true;
              description = ''
                Set to false to render `tls false` (disables automatic TLS
                on this host).
              '';
            };
            cert = lib.mkOption {
              type = lib.types.nullOr lib.types.str;
              default = null;
              example = "/etc/ssl/certs/example.com.crt";
              description = ''
                TLS certificate path for manual TLS. Requires `key`;
                together they render the `tls <cert> <key>` shorthand (see
                docs/configuration/security/tls.md). Supports
                `{{env.VAR}}` interpolation verbatim.
              '';
            };
            key = lib.mkOption {
              type = lib.types.nullOr lib.types.str;
              default = null;
              example = "/etc/ssl/private/example.com.key";
              description = ''
                TLS private key path for manual TLS. Requires `cert`.
              '';
            };
          };

          options.httpsRedirect = lib.mkOption {
            type = lib.types.nullOr lib.types.bool;
            default = null;
            example = false;
            description = ''
              Renders `https_redirect true|false`. Null omits it
              (Ferron redirects to HTTPS by default when TLS is enabled).
            '';
          };

          options.spaFallback = lib.mkOption {
            type = lib.types.bool;
            default = false;
            description = ''
              Single-page-app fallback: rewrites non-file, non-directory
              requests to `/` (`rewrite r"^/.*" "/"` with `last`,
              `directory false`, `file false`), so client-side routing
              works. Requires `root`.
            '';
          };

          options.fcgiPhp = lib.mkOption {
            type = lib.types.nullOr lib.types.str;
            default = null;
            example = "unix:///run/php/php-fpm.sock";
            description = ''
              PHP-FPM backend (`fcgi_php <url>`, see
              docs/configuration/content/fastcgi.md). Accepts
              `tcp://host:port` or `unix:///abs/path`. Always pair with
              `root`.
            '';
          };

          options.headers = {
            set = lib.mkOption {
              type = lib.types.attrsOf lib.types.str;
              default = { };
              example = {
                Content-Security-Policy = "default-src 'self'";
                X-Frame-Options = "DENY";
              };
              description = ''
                Replace (set) response headers: renders
                `header <Name> "<value>"` per entry (see
                docs/configuration/content/headers.md). Values are
                double-quoted automatically; `{{...}}` interpolation
                passes through verbatim.
              '';
            };
            add = lib.mkOption {
              type = lib.types.attrsOf lib.types.str;
              default = { };
              example = {
                X-Client-IP = "{{remote.ip}}";
              };
              description = ''
                Append response headers (allows duplicates): renders
                `header +<Name> "<value>"` per entry.
              '';
            };
            unset = lib.mkOption {
              type = lib.types.listOf lib.types.str;
              default = [ ];
              example = [ "Server" ];
              description = ''
                Remove response headers: renders `header -<Name>` per entry.
              '';
            };
          };

          options.cache = {
            enable = lib.mkOption {
              type = lib.types.bool;
              default = false;
              description = ''
                Enables HTTP response caching for this host (bare `cache`;
                host default is disabled, so false renders nothing — use
                `config` with `cache false` to disable per-location).
              '';
            };
            litespeedOverrideCacheControl = lib.mkOption {
              type = lib.types.bool;
              default = false;
              description = ''
                LSCache compatibility: honor `X-LiteSpeed-Cache-Control`
                over `Cache-Control`/`Expires`
                (`litespeed_override_cache_control`).
              '';
            };
            emitLitespeedHeaders = lib.mkOption {
              type = lib.types.bool;
              default = false;
              description = ''
                Echo `X-LiteSpeed-*` headers on cache hits
                (`emit_litespeed_headers`).
              '';
            };
            vary = lib.mkOption {
              type = lib.types.listOf lib.types.str;
              default = [ ];
              example = [
                "Accept-Encoding"
                "Accept-Language"
              ];
              description = "Response partitioning dimensions (`vary ...`).";
            };
            varyCookies = lib.mkOption {
              type = lib.types.listOf lib.types.str;
              default = [ ];
              example = [ "lang" ];
              description = "Cookie partitioning dimensions (`vary_cookies ...`).";
            };
            ignore = lib.mkOption {
              type = lib.types.listOf lib.types.str;
              default = [ ];
              example = [ "Set-Cookie" ];
              description = "Cache-control tokens to ignore (`ignore ...`).";
            };
            maxResponseSize = lib.mkOption {
              type = lib.types.nullOr lib.types.int;
              default = null;
              example = 2097152;
              description = "Largest cacheable response body in bytes.";
            };
          };
        }
      );
      default = { };
      example = {
        "example.com" = {
          root = "/var/www/ferron";
          spaFallback = true;
        };
      };
      description = ''
        Host blocks keyed by selector (`*:80`, `example.com`, ...).
        Typed options (`root`, `proxy`, `tls`, ...) render first, then
        `config` verbatim — so any ferron.conf directive works without
        waiting for typed-option coverage.
        Unless `configFile` is set, a default `*:80` vhost serving the
        package web root is always present; set `hosts."*:80".config`
        explicitly to replace its content.
      '';
    };

    extraConfig = lib.mkOption {
      type = lib.types.lines;
      default = "";
      description = ''
        Raw ferron.conf text appended after the generated blocks
        (matchers, snippets, includes). Escape hatch for anything the
        structured options do not cover yet.
      '';
    };

    configFile = lib.mkOption {
      type = lib.types.nullOr lib.types.path;
      default = null;
      description = ''
        Full override: use this config file instead of the generated one.
        When set, `globalConfig`, `hosts` and `extraConfig` are ignored.
      '';
    };

    openFirewall = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = ''
        Open TCP ports 80 (HTTP) and 443 (HTTPS) in the NixOS firewall,
        so the server is reachable from outside without extra firewall
        rules. Set to false when only serving custom ports or when the
        firewall is managed elsewhere.
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    # Default demo vhost as an overridable definition (mkDefault), not
    # an option `default`: option defaults do not merge per-key with
    # user-defined hosts, while definitions do. User values for the same
    # keys take precedence per-option. Skipped for `configFile` overrides
    # (which ignore generated hosts entirely, and should not force a
    # package build for the web root path).
    services.ferron.hosts."*:80".config = lib.mkIf (cfg.configFile == null) (
      lib.mkDefault "root ${cfg.package}/share/ferron/wwwroot"
    );

    users.users = lib.mkIf (cfg.user == "ferron") {
      ferron = {
        isSystemUser = true;
        group = cfg.group;
        home = "/var/lib/ferron";
        description = "Ferron web server";
      };
    };

    users.groups = lib.mkIf (cfg.group == "ferron") {
      ferron = { };
    };

    networking.firewall = lib.mkIf cfg.openFirewall {
      allowedTCPPorts = [
        80
        443
      ];
    };

    systemd.services.ferron = {
      description = "Ferron web server";
      after = [ "network.target" ];
      wantedBy = [ "multi-user.target" ];

      serviceConfig = {
        Type = "simple";
        User = cfg.user;
        Group = cfg.group;
        # Validate before (re)start. NOTE: not `preStart`, which only takes
        # effect combined with `script`; with a raw `ExecStart` it is
        # silently ignored. ExecStartPre always lands in the unit.
        ExecStartPre = "${lib.getExe cfg.package} validate -c ${confFile}";
        ExecStart = "${lib.getExe cfg.package} run -c ${confFile}";
        ExecReload = "${pkgs.coreutils}/bin/kill -HUP $MAINPID";
        Restart = "on-failure";

        # NixOS-native state handling (replaces installer 30_dirs.sh /
        # deb postinst imperatives): /run/ferron, /var/lib/ferron,
        # /var/log/ferron are managed by systemd.
        RuntimeDirectory = "ferron";
        StateDirectory = "ferron";
        LogsDirectory = "ferron";

        # Allow binding :80/:443 without running as root
        # (same as packaging/deb|rpm/ferron.service).
        AmbientCapabilities = "CAP_NET_BIND_SERVICE";
      };
    };
  };
}
