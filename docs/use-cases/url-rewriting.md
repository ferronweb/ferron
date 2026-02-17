---
title: URL rewriting
description: "Apply practical rewrite rules in Ferron for SPAs, PHP front controllers, and legacy URL migrations."
---

URL rewriting is useful when your application expects "pretty URLs" that map to a single entry script (common in PHP CMS/framework stacks), or when you need to preserve old URL structures after migrations.

For many applications behind reverse proxy, rewriting is not required. Those apps usually handle routing themselves, and Ferron only forwards requests with `proxy` (often using `location` and `remove_base`).

## Single-page application fallback

A common pattern is rewriting unknown routes to `/` so client-side routing works:

```kdl
// Example SPA rewrite configuration. Replace "example.com" with your domain name.
example.com {
    root "/var/www/html" // Replace with your SPA build directory
    rewrite "^/.*" "/" directory=#false file=#false last=#true
}
```

This preserves real files (for example `/assets/app.js`) while routing non-file paths (for example `/dashboard/settings`) to your SPA entry point.

## PHP front-controller pattern

Many PHP applications route requests through `index.php`:

```kdl
// Example rewrite for PHP front controller. Replace "example.com" with your domain name.
example.com {
    root "/var/www/app/public" // Replace with your application public directory
    rewrite "^/(.*)" "/index.php/$1" file=#false directory=#false last=#true
    fcgi_php "unix:///run/php/php8.4-fpm.sock" // Replace with your PHP FastCGI socket or TCP URL
}
```

This pattern is commonly used by CMS/framework setups where the app resolves routes internally.

## Legacy URL migration

To keep old URLs working after restructuring paths:

```kdl
// Example rewrite rules for migrated paths. Replace "example.com" with your domain name.
example.com {
    root "/var/www/html"
    rewrite "^/old-path/(.*)" "/new-path/$1" last=#true
    rewrite "^/blog/([^/]+)/?(?:$|[?#])" "/blog.php?slug=$1" last=#true
}
```

## Rewrite troubleshooting

- Use `rewrite_log` while debugging to verify which rules match.
- Prefer specific rules before broad catch-all rules.
- Use `file=#false` and `directory=#false` for front-controller/SPA rewrites so existing files/directories are still served directly.
- Keep `allow_double_slashes` disabled unless your app explicitly requires double-slash URLs.
- Avoid `disable_url_sanitizer` unless you have a concrete compatibility reason and have reviewed path traversal risk.
- For reverse-proxy routing patterns, see [Reverse proxying](/docs/use-cases/reverse-proxy).
- For directive reference (`rewrite`, `rewrite_log`, `allow_double_slashes`, `disable_url_sanitizer`), see [Configuration: routing & URL processing](/docs/configuration/routing-url-processing).
