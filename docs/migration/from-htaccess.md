---
title: "Migrating from Apache .htaccess to Ferron 3 (PHP hosting)"
description: "A practical guide for moving Apache-based PHP, WordPress, and Joomla hosting (with .htaccess) to Ferron 3 with PHP-FPM, covering the front-controller pattern, www/HTTPS redirects, IP ACLs, error pages, directory listings, and more."
---

This guide helps you replace an Apache + `mod_php`/`mod_rewrite` setup with Ferron 3 in front of [PHP-FPM](/docs/v3/use-cases/content/php). It maps the `.htaccess` patterns you already know to Ferron 3 directives, and highlights the differences in how the two servers think about configuration.

The examples assume a single WordPress-style site served through PHP-FPM over a Unix socket, but the patterns apply equally to Joomla and other PHP applications.

## Before you start: how Ferron differs from `.htaccess`

Apache evaluates `.htaccess` per directory, per request, at runtime. Ferron uses a single central configuration file (`ferron.conf`) that is loaded and **validated at startup** — there is no per-directory `.htaccess` file, and changes require a server reload.

| Concept | Apache `.htaccess` | Ferron 3 |
| --- | --- | --- |
| Config location | One `.htaccess` per directory in the web root | One `ferron.conf` for the whole server |
| PHP execution | `mod_php`, `mod_proxy_fcgi` + `SetHandler` | `fcgi_php` (FastCGI to PHP-FPM) |
| URL rewriting | `RewriteRule`/`RewriteCond` | `rewrite` (regex) |
| Path matching | Directory context + `RewriteBase` | `location` (prefix) + `match` (expressions) |
| Access rules | `Allow`/`Deny`, `Require` | `allow`/`block` |
| Error pages | `ErrorDocument` | `error_page` |
| Headers | `Header` / `RequestHeader` | `header` / `request_header` |
| Auth | `AuthType Basic` + `htpasswd` | `basic_auth` (hashed passwords) |

> [!important]
> Ferron deliberately does **not** read `.htaccess` files. After migration, leaving an `.htaccess` in the document root has no effect — move every relevant rule into `ferron.conf`.

### PHP-FPM prerequisites

Install and run PHP-FPM, and point Ferron at its socket. For example, on Debian/Ubuntu:

```bash
sudo apt install php8.4-fpm
sudo systemctl enable --now php8.4-fpm
```

Make the socket reachable by the Ferron process:

```ini
; /etc/php/8.4/fpm/pool.d/www.conf
listen = /run/php/php8.4-fpm.sock
listen.owner = ferron
listen.group = ferron
```

Then reference it from Ferron with `fcgi_php "unix:///run/php/php8.4-fpm.sock"`.

## A complete WordPress example

Here is a full Ferron 3 configuration that reproduces what a typical `WordPress` `.htaccess` + Apache vhost provides: PHP-FPM execution, the front-controller rewrite, HTTPS enforcement, a www→non-www canonical redirect, custom error pages, IP-based admin protection, and baseline security headers.

```ferron
example.com {
    root /var/www/html
    fcgi_php "unix:///run/php/php8.4-fpm.sock"

    # Front-controller fallback: route anything that is not a real file or
    # directory to index.php (WordPress reads REQUEST_URI to route).
    rewrite "^(.*)$" "/index.php" {
        file false
        directory false
    }

    # HTTPS enforcement (enabled by default once TLS is configured).
    https_redirect

    # Custom error pages.
    error_page 404 /var/www/html/404.html
    error_page 500 502 503 504 /var/www/html/50x.html

    # Baseline security headers.
    header "X-Content-Type-Options" "nosniff"
    header "X-Frame-Options" "DENY"
    header "Referrer-Policy" "strict-origin-when-cross-origin"
    header "Strict-Transport-Security" "max-age=31536000; includeSubDomains"

    match WP_ADMIN {
        request.uri ~ r"/wp-login\.php|/wp-admin(?:/|$)"
    }
    
    # Protect the admin area by IP.
    if WP_ADMIN {
        allow "203.0.113.0/24"
        block "0.0.0.0/0"
    }
}

# Canonical redirect: www -> non-www (handles both HTTP and HTTPS in one 301).
www.example.com {
    https_redirect false
    status 301 {
        location "https://example.com{{request.uri}}"
    }
}
```

The sections below explain each pattern in detail.

## Front-controller pattern (WordPress, Joomla)

Apache `.htaccess` for WordPress typically looks like this:

```apache
<IfModule mod_rewrite.c>
RewriteEngine On
RewriteBase /
RewriteRule ^index\.php$ - [L]
RewriteCond %{REQUEST_FILENAME} !-f
RewriteCond %{REQUEST_FILENAME} !-d
RewriteRule . /index.php [L]
</IfModule>
```

The goal: serve real files directly, and send everything else to `index.php`, which uses the request path to route internally.

In Ferron, `fcgi_php` already sends `.php` files to PHP-FPM, and `root` serves existing static files. The only remaining piece is the fallback for URLs that do not map to real files — use `rewrite` with `file false` and `directory false`:

```ferron
example.com {
    root /var/www/html
    fcgi_php "unix:///run/php/php8.4-fpm.sock"

    # Only rewrite when the URL is NOT an existing file or directory.
    rewrite "^(.*)$" "/index.php" {
        file false
        directory false
    }
}
```

- `file false` — do not apply this rule when the URL corresponds to an existing file.
- `directory false` — do not apply when it corresponds to an existing directory.

This produces the same behavior as the Apache rules: existing files (CSS, images, uploaded `.php`) are served as-is, and clean URLs fall through to `index.php`. WordPress/Joomla read `REQUEST_URI` (set automatically by Ferron to the original request URI), so no query-string juggling is needed.

> [!tip]
> If your application expects the original path in `PATH_INFO` instead of `REQUEST_URI`, rewrite to the script plus the captured group: `rewrite "^(.*)$" "/index.php/$1"`.

### Joomla

Joomla uses the same front-controller idea with `index.php` as the entry script, so the rule is identical:

```ferron
example.com {
    root /var/www/joomla
    fcgi_php "unix:///run/php/php8.4-fpm.sock"

    rewrite "^(.*)$" "/index.php" {
        file false
        directory false
    }
}
```

### Serving some paths without PHP

If a subtree should be static-only (no PHP execution), disable FastCGI with `fcgi_php false` in a `location` block. This mirrors Apache's `SetHandler none` / `<Files>` exceptions:

```ferron
example.com {
    root /var/www/html
    fcgi_php "unix:///run/php/php8.4-fpm.sock"

    location /static {
        fcgi_php false
        root /var/www/static
    }
}
```

## www / non-www canonical redirect

Apache:

```apache
RewriteEngine On
RewriteCond %{HTTP_HOST} ^www\.(.*)$ [NC]
RewriteRule ^(.*)$ https://%1/$1 [R=301,L]
```

Ferron has no host-rewriting `RewriteRule`. Instead, declare a separate host block for the `www` name that returns a 301 to the canonical host. Use `https_redirect false` on the `www` block so a single redirect goes straight to HTTPS on the canonical host:

```ferron
www.example.com {
    https_redirect false
    status 301 {
        location "https://example.com{{request.uri}}"
    }
}

example.com {
    root /var/www/html
    fcgi_php "unix:///run/php/php8.4-fpm.sock"
    # https_redirect is enabled by default because TLS is configured.
}
```

The `{{request.uri}}` placeholder expands to the original path and query string, so `/about?ref=newsletter` on `www.example.com` becomes `https://example.com/about?ref=newsletter`.

To redirect the **other** way (non-www → www), swap the host names in the two blocks.

## HTTPS redirect

Apache forces HTTPS with:

```apache
RewriteEngine On
RewriteCond %{HTTPS} off
RewriteRule ^(.*)$ https://%{HTTP_HOST}%{REQUEST_URI} [R=301,L]
```

In Ferron, HTTPS redirection is automatic once a host name has TLS enabled. When you declare a host by name (e.g. `example.com`) with no explicit port, Ferron starts both an HTTP (`:80`) and HTTPS (`:443`) listener and issues a **308 Permanent Redirect** from HTTP to HTTPS by default. There is nothing to configure unless you want to disable it:

```ferron
example.com {
    https_redirect          # default when TLS is present
    root /var/www/html
    fcgi_php "unix:///run/php/php8.4-fpm.sock"
}
```

Set `https_redirect false` only if you intentionally serve plain HTTP (for example, behind a TLS-terminating load balancer that already redirects).

> [!note]
> The 308 status preserves the request method and body, so `POST` form submissions redirect correctly — unlike Apache's common `R=301` which can change `POST` to `GET`.

## IP-based access control (ACLs)

Apache (legacy `Allow`/`Deny`):

```apache
Order Deny,Allow
Deny from all
Allow from 203.0.113.0/24
```

Apache (current `Require`):

```apache
Require ip 203.0.113.0/24
```

Ferron uses `allow` and `block`. When `allow` is present, **only** the listed networks are permitted. Everything else gets `403 Forbidden`. Add `block` entries to deny specific addresses even within an allowed range (`block` always wins over `allow`):

```ferron
example.com {
    root /var/www/html
    fcgi_php "unix:///run/php/php8.4-fpm.sock"

    # Allow only the office network, deny one bad host inside it.
    allow "203.0.113.0/24"
    block "203.0.113.50"
}
```

Place these inside a `location` or `if` block to protect a single path (such as `/wp-admin` or `/administrator`).

> [!important]
> If Ferron sits behind a reverse proxy or load balancer, configure `client_ip_from_header` with a `trusted_proxy` list so the IP rules evaluate the real client IP rather than the proxy's address.

### Deny access to sensitive files

Apache commonly blocks dotfiles and config files:

```apache
<FilesMatch "^\.">
    Require all denied
</FilesMatch>
```

In Ferron, use a named matcher with `if` and a custom `403` status:

```ferron
match sensitive_path {
    request.uri.path ~ "^/(?:\\.|wp-config\\.php|\\.env)"
}

example.com {
    root /var/www/html
    fcgi_php "unix:///run/php/php8.4-fpm.sock"

    if sensitive_path {
        status 403 {
            body "Access denied"
        }
    }
}
```

## Custom error pages

Apache:

```apache
ErrorDocument 404 /404.html
ErrorDocument 500 /50x.html
```

Ferron uses `error_page`, mapping one or more status codes to an absolute file path:

```ferron
example.com {
    root /var/www/html
    fcgi_php "unix:///run/php/php8.4-fpm.sock"

    error_page 404 /var/www/html/404.html
    error_page 500 502 503 504 /var/www/html/50x.html
}
```

When reverse proxying (rather than serving PHP directly), enable `intercept_errors` on the `proxy` block so upstream 5xx responses are replaced with your custom page:

```ferron
example.com {
    location / {
        proxy http://127.0.0.1:3000 {
            intercept_errors
        }
    }

    error_page 502 /var/www/html/502.html
    error_page 503 /var/www/html/503.html
}
```

> [!note]
> Missing error page files are skipped silently and Ferron's built-in page is used instead, so always verify the paths exist.

## Directory listings

Apache enables or disables indexes per directory:

```apache
Options +Indexes
# or
Options -Indexes
```

Ferron's `directory_listing` is disabled by default. Enable it to mirror `+Indexes`; leave it off (the default) to mirror `-Indexes`:

```ferron
example.com {
    root /var/www/html
    directory_listing            # mirrors Options +Indexes

    # Optionally
    #index index.php index.html
}
```

`index` controls the files tried when a directory is requested (Ferron defaults to `index.html index.htm index.xhtml`; `index.php` is included as well for PHP entry points when using PHP-FPM).

## Security and other response headers

Apache:

```apache
Header always set X-Content-Type-Options "nosniff"
Header always set Content-Security-Policy "default-src 'self'"
Header unset X-Powered-By
```

Ferron's `header` directive has three forms — add (`+`), remove (`-`), and replace (bare name):

```ferron
example.com {
    root /var/www/html
    fcgi_php "unix:///run/php/php8.4-fpm.sock"

    header +X-Content-Type-Options "nosniff"
    header +Content-Security-Policy "default-src 'self'"
    header -X-Powered-By
    header Server "Ferron"
}
```

Apply headers inside a `location` block for per-path policies (for example `Cache-Control: no-store` on `/wp-admin`).

## Password-protected areas (Basic Auth)

Apache uses `htpasswd` files:

```apache
AuthType Basic
AuthName "Admin Area"
AuthUserFile /etc/apache2/.htpasswd
Require valid-user
```

Ferron's `basic_auth` does **not** read `htpasswd` files. It expects **hashed** passwords (Argon2id recommended). Generate a hash with the `ferron-passwd` tool that ships with Ferron, then list the user in the config:

```ferron
example.com {
    root /var/www/html

    match WP_ADMIN {
        request.uri ~ r"/wp-login\.php|/wp-admin(?:/|$)"
    }
    
    if WP_ADMIN {
        basic_auth {
            realm "Admin Area"
            users {
                admin "$argon2id$v=19$m=19456,t=2,p=1$..."
            }
        }
    }
}
```

Brute-force protection is enabled by default. You can tune it:

```ferron
example.com {
    basic_auth {
        realm "Admin Area"
        users {
            admin "$argon2id$v=19$m=19456,t=2,p=1$..."
        }
        brute_force_protection {
            enabled
            max_attempts 5
            lockout_duration "15m"
            window "5m"
        }
    }
}
```

> [!important]
> Basic Auth must be used over HTTPS — credentials travel in the `Authorization` header on every request.

## Forcing download / MIME types

Apache:

```apache
AddType application/octet-stream .pdf
<FilesMatch "\.pdf$">
    ForceType application/octet-stream
</FilesMatch>
```

Ferron maps extensions to MIME types with `mime_type`, and you can add a `Content-Disposition` header for a subtree to force download:

```ferron
example.com {
    root /var/www/html
    mime_type ".pdf" "application/octet-stream"

    match DOWNLOADS {
        request.uri.path ~ r"^/downloads(?:/|$)"
    }
    
    if DOWNLOADS {
        header +Content-Disposition "attachment"
    }
}
```

## Browser caching (Expires / Cache-Control)

Apache uses `mod_expires`:

```apache
<IfModule mod_expires.c>
    ExpiresActive On
    ExpiresByType image/png "access plus 1 month"
    ExpiresDefault "access plus 1 week"
</IfModule>
```

Ferron does not compute `Expires` from durations, but you can set `Cache-Control` directly and let the client derive expiry. Use `file_cache_control` for static files:

```ferron
example.com {
    root /var/www/html
    file_cache_control "public, max-age=604800"   # 1 week default

    match ASSETS {
        request.uri.path ~ r"^/assets(?:/|$)"
    }
    
    if ASSETS {
        file_cache_control "public, max-age=2592000"   # 30 days
    }
}
```

## Hotlink protection

Apache blocks off-site referrers:

```apache
RewriteEngine On
RewriteCond %{HTTP_REFERER} !^$
RewriteCond %{HTTP_REFERER} !^https://example\.com/ [NC]
RewriteRule \.(jpg|png|gif)$ - [F]
```

In Ferron, match the `referer` header and return `403`:

```ferron
match hotlink {
    request.header.referer !~ r"^https://example\.com/"
    request.header.referer != ""
}

example.com {
    root /var/www/html

    if hotlink {
        status 403 {
            body "Hotlinking not allowed"
        }
    }
}
```

## Blocking bad user agents / bots

Apache:

```apache
RewriteCond %{HTTP_USER_AGENT} (BadBot|EvilScraper) [NC]
RewriteRule ^ - [F]
```

Ferron uses a `match` on `user_agent` and a `403` status:

```ferron
match bad_bot {
    request.header.user_agent ~ "(?i)badbot|evilscraper"
}

example.com {
    if bad_bot {
        status 403 {
            body "Blocked"
        }
    }
}
```

For silently dropping connections instead of returning a response, use `abort` in place of `status 403`.

## Maintenance mode

Apache takes the site down with:

```apache
RewriteEngine On
RewriteCond %{REMOTE_ADDR} !^203\.0\.113\.10
RewriteRule ^ - [R=503,L]
```

Ferron returns a `503` for everyone except an allowed IP:

```ferron
match not_maintainer {
    remote.ip != "203.0.113.10"
}

example.com {
    if not_maintainer {
        status 503 {
            body "Site under maintenance"
        }
    }
}
```

## Migration checklist

1. Install and start PHP-FPM. Confirm the socket (or TCP port) is reachable by the Ferron user.
2. Create `ferron.conf` with the `root`, `fcgi_php`, and TLS settings for your domain.
3. Recreate each `.htaccess` rule using the mapping above — `rewrite` for routing, `allow`/`block` for ACLs, `error_page` for error documents, `header` for headers, `basic_auth` for password areas.
4. Move the front-controller rewrite (and any `index`/`directory_listing` settings) to the host block.
5. Validate the configuration: `ferron validate -c ferron.conf`.
6. Run `ferron doctor -c ferron.conf` to catch TLS, redirect, and timeout best-practice issues.
7. Start Ferron and confirm:
   - `https://example.com/` loads the home page (PHP executed).
   - A clean URL (e.g. `/sample-post`) renders via `index.php`.
   - `http://` redirects to `https://` (308).
   - `www.` redirects to the canonical host (301) if configured.
   - Custom 404/50x pages appear for missing routes and backend errors.
   - Protected areas require a password or allowed IP.

> [!tip]
> Keep the old Apache config until Ferron has served production traffic successfully, so you can roll back by repointing the service.

## See also

- [PHP hosting](/docs/v3/use-cases/content/php) — PHP via FastCGI and CGI, plus troubleshooting.
- [FastCGI support](/docs/v3/configuration/content/fastcgi) — `fcgi`/`fcgi_php` directives and environment variables.
- [URL rewriting](/docs/v3/configuration/routing/rewrite) — `rewrite` syntax and regex engine.
- [Access control](/docs/v3/use-cases/security/access-control) — `allow`/`block`, `basic_auth`, and `auth_to`.
- [Error pages](/docs/v3/use-cases/traffic/error-pages) — `error_page` and `intercept_errors`.
- [Conditionals and variables](/docs/v3/configuration/fundamentals/conditionals) — `match`/`if` and available variables.
