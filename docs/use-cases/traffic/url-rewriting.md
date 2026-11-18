---
title: URL rewriting
description: "Rewrite URLs for SPA fallback, PHP front controllers, clean URLs, and legacy path migrations."
---

A rewrite changes the request path before proxying or file serving. The client sees no redirect.

Ferron selects the `location` block once, from the original URL. A rewrite changes the URL for later stages only. It does not run `location` matching again.

Many proxied applications handle routing themselves. Those applications usually need only `proxy` and `location`, without rewrites.

> [!tip]
> Set `rewrite_log true` while you debug. Ferron logs each rewrite operation to the error log.

> [!info]
> For directive reference, see [Configuration: URL rewriting](/docs/configuration/routing/rewrite).

## Single-page application fallback

Rewrite unmatched paths to `/` so client-side routing works:

```ferron
example.com {
    root /var/www/html
    rewrite r"^/.*" "/" {
        last
        directory false
        file false
    }
}
```

The guards keep real files such as `/assets/app.js` on the file path. Non-file paths such as `/dashboard/settings` use the SPA entry point.

## PHP front controller

Route requests through `index.php`:

```ferron
example.com {
    root /var/www/app/public
    rewrite r"^/(.*)" "/index.php/$1" {
        file false
        directory false
        last
    }
}
```

The application resolves routes internally.

## Migrate legacy paths

Keep old URLs working after you move content:

```ferron
example.com {
    root /var/www/html
    rewrite r"^/old-path/(.*)" "/new-path/$1" {
        last
    }
    rewrite r"^/blog/([^/]+)/?(?:$|[?#])" "/blog.php?slug=$1" {
        last
    }
}
```

## Clean URLs for static files

To serve `/test` from `test.html` without a redirect, rewrite the path and add the extension:

```ferron
example.com {
    root /var/www/html

    rewrite r"^/([^?#]*[^?#/])($|[?#].*)" "/$1.html$2" {
        file false
    }

    status 301 {
        regex r"^/([^?#]*[^?#/])/($|[?#].*)"
        location /$1$2
    }
}
```

The second rule redirects `/test/` to `/test` to keep one canonical form.

## Chain rules

Without `last`, Ferron applies the rules in sequence:

```ferron
example.com {
    rewrite r"^/legacy/(.*)" "/modern/$1"
    rewrite r"^/modern/(.*)" "/current/$1"
}
```

A request to `/legacy/foo` becomes `/modern/foo`, then `/current/foo`.
