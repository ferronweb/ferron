---
title: "Installation on NixOS"
description: "Install Ferron 3 on NixOS with the official flake: pick a prebuilt or source-built package, then declare hosts with the NixOS module."
---

Ferron 3 ships a Nix flake with two packages and a NixOS module. The module generates `ferron.conf` from your Nix configuration and runs the server under systemd. You do not edit a config file on the host.

## Flake setup

### 1. Add the flake input

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    ferron.url = "github:ferronweb/ferron/3.x";
  };
}
```

The `3.x` URL follows the release branch. Run `nix flake update` to move to the latest published release.

### 2. Expose the packages

```nix
nixpkgs.overlays = [ inputs.ferron.overlays.default ];
```

The overlay adds `pkgs.ferron-bin` and `pkgs.ferron` to your package set. The module uses `pkgs.ferron-bin` as its default package, so you need the overlay unless you set `services.ferron.package` yourself. Importing the module does not need the overlay.

### 3. Import the module

```nix
{
  imports = [ inputs.ferron.nixosModules.default ];
}
```

## Packages

| Package      | Use it when                                                                                                                          |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------ |
| `ferron-bin` | You run a published release. It installs prebuilt binaries, so the install does not need a Rust toolchain.                           |
| `ferron`     | You need a revision without published archives, such as a fork or an unreleased commit. It compiles the workspace from `Cargo.lock`. |

Both packages install the same six binaries. See [File locations](#file-locations).

Select the source build with the package option:

```nix
services.ferron.package = inputs.ferron.packages.${pkgs.stdenv.hostPlatform.system}.ferron;
```

> [!note]
> The source build compiles without PGO, so it can run slower than the release archives under load. Prebuilt binaries also embed the real build date and git commit. Source builds record `1980-01-01` and an empty commit to keep builds reproducible.

> [!tip]
> Pin an exact commit with `github:ferronweb/ferron/3.x?rev=<sha>`. The pinned commit must still contain the `nix/` directory.

## Minimal configuration

```nix
{
  services.ferron = {
    enable = true;
    hosts."*:80".root = "/var/www/ferron";
  };
}
```

Activate the configuration with `sudo nixos-rebuild switch --flake .#hostname`. Ferron then serves `/var/www/ferron` on port 80.

> [!tip]
> If you declare no `hosts`, the module still adds a `*:80` host that serves the demo web root from the package. So `enable = true` alone gives you a working test page. When you declare your own `hosts`, the default `*:80` host stays. Set `hosts."*:80".config` yourself to replace its content.

## What the module manages

- **Service user**: the server runs as the `ferron` system user and group. Change the names with `user` and `group`.
- **Privileges**: the service gets `CAP_NET_BIND_SERVICE`, so it binds ports 80 and 443 without root.
- **Directories**: systemd creates `/var/lib/ferron` for state, `/var/log/ferron` for logs, and `/run/ferron` for runtime files.
- **Config validation**: the unit runs `ferron validate` in `ExecStartPre` before every start. An invalid config fails the start instead of a running server.
- **Firewall**: the module opens TCP ports 80 and 443 in `networking.firewall`. Set `openFirewall = false` if you serve custom ports or manage the firewall yourself.
- **Recovery**: the unit restarts the server after a failure. `systemctl reload ferron` sends `SIGHUP`, which makes Ferron read its config again.

## Host configuration

`services.ferron.hosts` maps a Ferron host selector to a set of options. The module writes each attribute name into `ferron.conf` as it is. Any selector that Ferron understands works, such as `*:80`, `example.com`, or `*.example.com`.

### How a host block is generated

The module writes typed options first, in a fixed order. It appends the verbatim `config` string last. Ferron resolves duplicate directives per directive. For example, `root` and `index` keep the first value, while `header` keeps the last one.

So a directive in `config` does not always override the same typed option. Use typed options for the directives they cover, and use `config` for directives that have no typed option. If you must repeat a typed directive, check its page in the configuration reference first.

This configuration:

```nix
services.ferron.hosts."example.com" = {
  root = "/var/www/example";
  index = [ "index.html" "index.htm" ];
  spaFallback = true;
  headers = {
    set.Content-Security-Policy = "default-src 'self'";
    add."X-Client-IP" = "{{remote.ip}}";
    unset = [ "Server" ];
  };
  cache = {
    enable = true;
    maxResponseSize = 2097152;
  };
};
```

generates this host block:

```ferron
example.com {
    root /var/www/example
    index index.html index.htm
    cache {
        max_response_size 2097152
    }
    header Content-Security-Policy "default-src 'self'"
    header +X-Client-IP "{{remote.ip}}"
    header -Server
    rewrite r"^/.*" "/" {
        last
        directory false
        file false
    }
}
```

The fixed order is TLS, HTTPS redirect, `root`, `index`, `cache`, `proxy`, `fcgi_php`, headers, and the SPA rewrite.

### Static files

- `root` sets the directory that the host serves. The default `null` omits the directive. Paths are written as they are, so use `config` for paths with spaces.
- `index` lists the files that Ferron tries when a request resolves to a directory. The default `null` keeps the Ferron default of `index.html`, `index.htm`, and `index.xhtml`.
- `spaFallback` serves client-side routes. The server rewrites requests that match no file or directory to `/`. So `/dashboard/settings` returns `index.html` instead of a 404. This option requires `root`.

See [Static file serving](/docs/configuration/content/static-files) for the directive details.

### Reverse proxy

- `proxy` sets one upstream URL. The module writes the `proxy <url>` shorthand.
- `proxyExtraConfig` places lines inside a `proxy { ... }` block instead of the shorthand. The default empty string renders the one-line form.

```nix
services.ferron.hosts."app.example.com" = {
  proxy = "http://127.0.0.1:3000";
  proxyExtraConfig = "algorithm round_robin";
};
```

Use `config` when you need several upstreams, because the typed `proxy` option takes one URL. See [Reverse proxying](/docs/configuration/proxy/reverse-proxy) for the nested options.

### PHP

`fcgiPhp` sets a PHP-FPM backend as `tcp://host:port` or `unix:///run/php/php-fpm.sock`. Pair it with `root` so that Ferron also serves static files. See [FastCGI and CGI](/docs/configuration/content/fastcgi) for details.

### TLS

- `tls.enable` defaults to `true`, which writes no directive and keeps the Ferron automatic TLS behavior. Set it to `false` to write `tls false` and serve plain HTTP only.
- `tls.cert` and `tls.key` set a certificate and a private key. The module writes them as `tls <cert> <key>`. Set both or neither.
- `httpsRedirect` accepts `true` or `false` and writes `https_redirect`. The default `null` omits the directive, so Ferron keeps redirecting to HTTPS when TLS is active.

The module passes values as they are, so `{{env.VAR}}` interpolation works in paths. See [TLS](/docs/configuration/security/tls) for certificate formats and providers.

### Response headers

- `headers.set` replaces a response header. The module writes `header <Name> "<value>"`.
- `headers.add` appends a header and keeps existing values. The module writes `header +<Name> "<value>"`. Use it for headers that may repeat.
- `headers.unset` is a list of header names to remove. The module writes `header -<Name>` for each name.

The module adds the quotes, so values with spaces or semicolons need no escaping. It escapes backslashes and double quotes. Placeholders such as `{{remote.ip}}` pass through, and Ferron fills them per request. See [HTTP headers and CORS](/docs/configuration/content/headers) for details.

### Response caching

- `cache.enable` turns on the in-memory response cache for the host. The module writes the bare `cache` directive. The default is `false`. To disable caching again for a sub-path, put `cache false` in `config`.
- `cache.maxResponseSize` sets the largest response body that the server stores, in bytes. The default `null` keeps the Ferron default of 2 MiB. Ferron still serves larger responses.
- `cache.vary` lists request headers that the server adds to the cache key.
- `cache.varyCookies` lists cookie names that the server adds to the cache key.
- `cache.ignore` lists response headers that the server drops from the stored copy.
- `cache.litespeedOverrideCacheControl` and `cache.emitLitespeedHeaders` support applications that expect LSCache response headers.

See [HTTP cache](/docs/configuration/content/cache) for all cache directives.

### Option reference

| Option                                | Default | Writes                                    |
| ------------------------------------- | ------- | ----------------------------------------- |
| `root`                                | `null`  | `root <path>`                             |
| `index`                               | `null`  | `index <name>...`                         |
| `spaFallback`                         | `false` | Rewrite to `/` for unmatched requests     |
| `proxy`                               | `null`  | `proxy <url>`                             |
| `proxyExtraConfig`                    | `""`    | Lines inside the `proxy { ... }` block    |
| `fcgiPhp`                             | `null`  | `fcgi_php <url>`                          |
| `tls.enable`                          | `true`  | Nothing, or `tls false` when set to false |
| `tls.cert`, `tls.key`                 | `null`  | `tls <cert> <key>`                        |
| `httpsRedirect`                       | `null`  | `https_redirect true` or `false`          |
| `headers.set`                         | `{}`    | `header <Name> "<value>"`                 |
| `headers.add`                         | `{}`    | `header +<Name> "<value>"`                |
| `headers.unset`                       | `[]`    | `header -<Name>`                          |
| `cache.enable`                        | `false` | `cache`                                   |
| `cache.maxResponseSize`               | `null`  | `max_response_size <bytes>`               |
| `cache.vary`, `cache.varyCookies`     | `[]`    | `vary ...`, `vary_cookies ...`            |
| `cache.ignore`                        | `[]`    | `ignore ...`                              |
| `cache.litespeedOverrideCacheControl` | `false` | `litespeed_override_cache_control`        |
| `cache.emitLitespeedHeaders`          | `false` | `emit_litespeed_headers`                  |

### Escape hatches

- `hosts."<selector>".config` holds raw lines for that host. The module appends them after the typed directives. Use them for directives that have no typed option.
- `extraConfig` holds raw text that the module appends after all host blocks. Use it for matchers and other top-level directives.
- `configFile` points at a config file that you manage. When it is set, the module ignores `globalConfig`, `hosts`, and `extraConfig`. It also adds no default host.

```nix
services.ferron.configFile = pkgs.writeText "ferron.conf" ''
  example.com {
      root /var/www/example
  }
'';
```

## Global configuration

`globalConfig` holds the contents of the global `{ ... }` block. The default sets access log and error log rotation. `extraConfig` appends text after the host blocks.

```nix
services.ferron = {
  globalConfig = ''
    log /var/log/ferron/access.log {
        access_log_rotate_size 10485760
        access_log_rotate_keep 7
    }
  '';
};
```

See [Configuration syntax](/docs/configuration/fundamentals/syntax) for the global block directives.

## Module options

| Option         | Default           | Purpose                                                          |
| -------------- | ----------------- | ---------------------------------------------------------------- |
| `enable`       | `false`           | Runs the server and opens the firewall ports.                    |
| `package`      | `pkgs.ferron-bin` | Binary that the service runs.                                    |
| `user`         | `"ferron"`        | System user for the service.                                     |
| `group`        | `"ferron"`        | System group for the service.                                    |
| `hosts`        | `{}`              | Host blocks, keyed by selector. Adds a default `*:80` demo host. |
| `globalConfig` | Log rotation      | Contents of the global `{ ... }` block.                          |
| `extraConfig`  | `""`              | Raw text that the module appends after the host blocks.          |
| `configFile`   | `null`            | Uses your own config file and ignores the settings above.        |
| `openFirewall` | `true`            | Opens TCP ports 80 and 443 in `networking.firewall`.             |

## File locations

- `/nix/store/.../bin/`: the `ferron` server, plus `ferron-fmt`, `ferron-kdl2ferron`, `ferron-passwd`, `ferron-precompress`, and `ferron-serve`.
- `/nix/store/.../share/ferron/ferron.conf.example`: example config from the upstream package.
- `/nix/store/.../share/ferron/wwwroot/`: demo web root, used by the default `*:80` host.
- `/var/log/ferron/access.log` and `/var/log/ferron/error.log`: server logs.
- `/var/lib/ferron` and `/run/ferron`: state and runtime files.

The generated config lives in the Nix store. Run `sudo systemctl cat ferron` to see the exact path that the server reads.

## Managing the service

```sh
sudo systemctl status ferron      # process state and recent log lines
sudo systemctl reload ferron      # read the config again
sudo systemctl restart ferron     # stop and start the server
sudo journalctl -u ferron.service # all service log entries
```

## Keeping versions current

`nix/package-bin.nix` pins the release version and the download hashes. `nix/package.nix` pins the two git submodules that the build uses.
