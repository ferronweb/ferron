---
title: "Migrating from NGINX to Ferron 3"
description: "A practical guide for moving NGINX server blocks to Ferron 3, with directive mappings and known differences."
---

This guide helps you move an NGINX setup to Ferron 3. It maps the NGINX directives you know to Ferron 3 directives. It also flags behavior differences that surprise NGINX users.

The examples assume a host that serves static files and proxies API traffic to a local backend. The patterns apply to single-page apps, PHP front controllers, and multi-service hosts.

## Before you start: how Ferron differs from NGINX

NGINX evaluates `server` and `location` blocks per request and can repeat `location` search after a rewrite. Ferron resolves configuration once per request, before pipeline stages run. There is no repeat search.

| Concept | NGINX | Ferron 3 |
| ------- | ----- | -------- |
| Virtual host | `server` + `server_name` | Host block (`example.com { ... }`) |
| Path matching | `location` with prefix, `=`, `^~`, and regex (`~`, `~*`) | `location` with prefix only, plus `match` and `if` for patterns |
| Rewrite loop | `rewrite ... last` repeats `location` search, up to 10 times | `rewrite` runs once in the pipeline and never re-triggers `location` matching |
| File fallback | `try_files` | `rewrite` with `file false` and `directory false` |
| Path remap | `alias` | `location` prefix stripping with `root`, or a separate `location` per path |
| Variables from patterns | `map` in `http` context | `map` in `http *`, host, or `location` blocks, never in the bare global block |
| Variables from regex | `set` | `set_var` |
| Static compression | `gzip on` | `compressed` (static files only) |
| Proxy compression | `gzip` on proxied responses | `dynamic_compressed` (dynamic responses only) |
| Symlinks | `disable_symlinks off` by default (links allowed) | `disable_symlinks true` by default (links return 403) |
| Direct response | `return` | `status` |

> [!important]
> Read [Request pipeline order](/docs/configuration/fundamentals/request-pipeline) before you migrate rewrites. Ferron selects the `location` block once, on the original URL. Later rewrites change the URL for proxying and file serving. They do not move the request to a different `location` block.

## A complete example

NGINX:

```nginx
server {
    listen 80
    server_name example.com
    root /srv/www/example

    location / {
        try_files $uri $uri/ /index.html
    }

    location /api/ {
        proxy_pass http://127.0.0.1:3000
    }
}
```

Ferron 3:

```ferron
example.com {
    root /srv/www/example

    # Front controller fallback. Real files win. All else serves index.html.
    rewrite "^(.*)$" "/index.html" {
        file false
        directory false
        last
    }

    location /api {
        proxy http://127.0.0.1:3000
    }
}
```

Ferron strips the `/api` prefix before proxying. The backend receives the path without `/api`. When the backend needs the full path, append it to the upstream URL. See [Reverse proxy](#reverse-proxy-base-paths) below.

## Location blocks

NGINX selects a location in two passes. It finds the longest matching prefix first. It then tests regex locations in file order and uses the first match. `^~` skips the regex pass. `=` matches one exact URI.

Ferron uses one pass with prefixes only. The longest matching prefix wins. There is no regex form.

```nginx
# NGINX
location = /health {
    return 200 "ok"
}

location ^~ /assets/ {
    root /srv/www/example
}

location ~* \.(gif|jpg|jpeg)$ {
    root /srv/www/images
}
```

```ferron
# Ferron 3
match image_request {
    request.uri.path ~ r"\.(gif|jpg|jpeg)$"
}

example.com {
    root /srv/www/example

    location /health {
        status 200 {
            body "ok"
        }
    }

    location /assets {
        root /srv/www/example
    }

    if image_request {
        root /srv/www/images
    }
}
```

> [!tip]
> Move each NGINX regex location to a `match` block with `if` or `if_not`. Keep plain prefixes as `location` blocks. Test overlapping paths, because Ferron never consults regex order across `location` blocks.

## Rewrites run without a new location search

NGINX runs `server` level rewrites first. It then searches for a location. When a rewrite uses the `last` flag, NGINX repeats the search with the new URI.

Ferron has no repeat search. It resolves the `location` block first, strips the prefix, and then runs `rewrite` rules in the pipeline. The `last` option stops further rewrite rules. It does not start routing over.

This changes broad patterns. A catch-all rule at host level also sees requests for static assets, because those assets share the host block. Guard the rule so real files skip it.

```nginx
# NGINX: location search repeats, so /libs/app.js can still match a static location
server {
    rewrite ^/([^/]+)/(.*)$ /tenant/$1/app/$2 last
}
```

```ferron
# Ferron 3: guard the rule so static files keep working
example.com {
    root /srv/www/example

    rewrite "^/([^/]+)/(.*)$" "/tenant/$1/app/$2" {
        file false
        last
    }
}
```

> [!warning]
> Test every broad rewrite against static asset URLs such as `/libs/app.js` and `/assets/style.css`. Add `file false` and `directory false` unless the rule must also rewrite real files. Enable `rewrite_log true` while you test.

## `try_files` becomes a guarded rewrite

Ferron has no `try_files` directive. Use a `rewrite` with `file false` and `directory false`. The guards test the URL against `root` and skip the rule for real files and directories.

```nginx
# NGINX
location / {
    try_files $uri $uri/ /index.html
}
```

```ferron
# Ferron 3
example.com {
    root /srv/www/example

    rewrite "^(.*)$" "/index.html" {
        file false
        directory false
        last
    }
}
```

For a PHP front controller, send the fallback to the entry script instead.

```ferron
example.com {
    root /var/www/html
    fcgi_php "unix:///run/php/php8.4-fpm.sock"

    rewrite "^(.*)$" "/index.php" {
        file false
        directory false
    }
}
```

## `alias` becomes `root` with location stripping

Ferron has no `alias` directive. In most cases `location` prefix stripping with `root` gives the same result. Ferron removes the matched prefix and resolves the rest under `root`.

```nginx
# NGINX
location /i/ {
    alias /data/w3/images/
}
```

```ferron
# Ferron 3
example.com {
    location /i {
        root /data/w3/images
    }
}
```

A request for `/i/top.gif` serves `/data/w3/images/top.gif` in both servers.

When one directory must answer under several URL prefixes, declare one `location` block per prefix. When content must stay in place and appear under another path, use a symlink and allow it explicitly, because Ferron blocks symlinks by default.

```ferron
example.com {
    root /srv/www/example
    disable_symlinks false

    location /docs {
        root /srv/shared/docs
    }
}
```

> [!warning]
> `disable_symlinks false` trusts every link under the path. Prefer `disable_symlinks if_not_owner` on multi user systems. A blocked link returns `403 Forbidden` without naming the link. See [Static file serving](/docs/configuration/content/static-files#symlink-handling).

## `map` and `set`

NGINX `map` lives in the `http` context. The Ferron counterpart lives in an `http *` block, a host block, or a `location` block. It has no effect in the bare global `{ ... }` block.

```nginx
# NGINX
http {
    map $http_user_agent $is_mobile {
        default 0
        ~*mobile 1
        ~*android 1
    }
}
```

```ferron
# Ferron 3
http * {
    map request.header.user_agent is_mobile {
        default "0"
        regex "mobile" "1" {
            case_insensitive
        }
        regex "android" "1" {
            case_insensitive
        }
    }
}
```

NGINX `set` assigns a variable anywhere. Ferron `set_var` sets a variable when a source value matches a regex.

```nginx
# NGINX
server {
    set $backend "http://127.0.0.1:3000";
}
```

```ferron
# Ferron 3
example.com {
    set_var request.uri.path r"\.pdf$" is_pdf
}
```

> [!note]
> `set_var` runs before `map` and `rewrite`. Variables from `set_var` are ready for maps and rewrite rules. See [Request pipeline order](/docs/configuration/fundamentals/request-pipeline).

## Reverse proxy base paths

NGINX `proxy_pass` with a URI part replaces the matched prefix. Ferron always strips the `location` prefix and then proxies the rest. These two configs match.

```nginx
# NGINX
location /api/ {
    proxy_pass http://127.0.0.1:3000/v2/;
}
```

```ferron
# Ferron 3
example.com {
    location /api {
        proxy http://127.0.0.1:3000/v2
    }
}
```

When the backend expects the original prefix, include it in the upstream URL.

```ferron
example.com {
    location /api {
        proxy http://backend/api
    }
}
```

## Redirects and `return`

NGINX `return` stops processing and answers at once. Ferron `status` does the same, with a nested `location` for the target URL or a `body` for text.

```nginx
# NGINX
server {
    server_name www.example.com
    return 301 https://example.com$request_uri
}
```

```ferron
# Ferron 3
www.example.com {
    https_redirect false
    status 301 {
        location "https://example.com{{request.uri}}"
    }
}
```

NGINX `rewrite ... permanent` and `rewrite ... redirect` also map to `status`.

```nginx
# NGINX
rewrite ^/old/(.*)$ /new/$1 permanent
```

```ferron
# Ferron 3
example.com {
    status 301 {
        regex "^/old/(.*)"
        location /new/$1
    }
}
```

## Compression

NGINX uses one `gzip` switch for static and proxied responses. Ferron splits the paths.

- Static files from `root` use `compressed`.
- Proxy, FastCGI, and CGI responses use `dynamic_compressed`.

```nginx
# NGINX
gzip on
```

```ferron
# Ferron 3: enable each path separately
example.com {
    root /srv/www/example
    compressed
    dynamic_compressed
    proxy http://127.0.0.1:3000
}
```

Ferron skips `101 Switching Protocols` responses, so WebSocket upgrades pass through unchanged.

## Symlinks

NGINX allows symlinks unless `disable_symlinks on` blocks them. Ferron blocks them unless you allow them. After migration, check for `403 Forbidden` on paths that use links.

```ferron
example.com {
    root /srv/www/example
    disable_symlinks false
}
```

## Migration checklist

1. Copy each NGINX `server` block to a Ferron host block with the same name.
2. Convert plain prefix locations to `location` blocks. Convert regex locations to `match` plus `if` blocks.
3. Replace each `try_files` fallback with a guarded `rewrite` using `file false` and `directory false`.
4. Replace each `alias` with `root` plus `location` stripping, or with a symlink you allow explicitly.
5. Move `map` blocks to `http *` or host scope. Move `set` assignments to `set_var`.
6. Check proxy base paths. Ferron strips the `location` prefix before proxying.
7. Split compression into `compressed` for files and `dynamic_compressed` for proxy responses.
8. Allow symlinks where content needs them. Keep the default block elsewhere.
9. Validate the configuration: `ferron validate -c ferron.conf`.
10. Run `ferron doctor -c ferron.conf` to catch TLS, redirect, and timeout issues.
11. Start Ferron and confirm static assets, API paths, fallback routes, redirects, and upgrades such as WebSockets.

> [!tip]
> Keep the old NGINX config until Ferron has served production traffic successfully. You can then roll back by repointing the service.

## See also

- [Request pipeline order](/docs/configuration/fundamentals/request-pipeline): how resolution and stages run.
- [URL rewriting](/docs/configuration/routing/rewrite): `rewrite` syntax and the regex engine.
- [Routing and URL processing](/docs/configuration/routing/url-processing): `location`, `if`, and `if_not`.
- [HTTP map](/docs/configuration/routing/map): `map` scope and matching priority.
- [Reverse proxying](/docs/use-cases/traffic/reverse-proxy): proxy patterns.
- [Static file serving](/docs/use-cases/content/static-files): roots, listings, and symlinks.
