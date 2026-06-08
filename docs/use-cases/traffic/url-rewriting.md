---
title: URL rewriting
description: "Apply practical rewrite rules in Ferron for SPAs, PHP front controllers, and legacy URL migrations."
---

URL rewriting is useful when your application expects "pretty URLs" that map to a single entry script (common in PHP CMS/framework stacks), or when you need to preserve old URL structures after migrations.

Rewrites are applied early in the request pipeline, before proxying or static file serving, so the rewritten URL is used for routing. The client sees no redirect — the rewrite is transparent.

For many applications behind reverse proxy, rewriting is not required. Those apps usually handle routing themselves, and Ferron only forwards requests with `proxy` (often using `location` blocks).

> [!tip]
> Use `rewrite_log true` while debugging to verify which rules match. Each rewrite operation is logged to the error log.

> [!info]
> For directive reference, see [Configuration: URL rewriting](/docs/v3/configuration/routing/rewrite).

## Single-page application fallback

A common pattern is rewriting unknown routes to `/` so client-side routing works:

```ferron
example.com {
    root /var/www/html
    rewrite "^/.*" "/" {
        last
        directory false
        file false
    }
}
```

This preserves real files (for example `/assets/app.js`) while routing non-file paths (for example `/dashboard/settings`) to your SPA entry point.

## PHP front-controller pattern

Many PHP applications route requests through `index.php`:

```ferron
example.com {
    root /var/www/app/public
    rewrite "^/(.*)" "/index.php/$1" {
        file false
        directory false
        last
    }
}
```

This pattern is commonly used by CMS/framework setups where the app resolves routes internally.

## Legacy URL migration

To keep old URLs working after restructuring paths:

```ferron
example.com {
    root /var/www/html
    rewrite "^/old-path/(.*)" "/new-path/$1" {
        last
    }
    rewrite "^/blog/([^/]+)/?(?:$|[?#])" "/blog.php?slug=$1" {
        last
    }
}
```

## Chained rules without `last`

Without `last true`, multiple rewrite rules can chain together:

```ferron
example.com {
    rewrite "^/legacy/(.*)" "/modern/$1"
    rewrite "^/modern/(.*)" "/current/$1"
}
```

A request to `/legacy/foo` is first rewritten to `/modern/foo`, then the second rule rewrites it to `/current/foo`.
