---
title: Status codes & redirects
description: "Configure Ferron to respond with custom HTTP status codes, or redirect old URLs."
---

Redirects are useful for example for preventing search engines from indexing duplicate content (so to not fragment search traffic between duplicates), and legacy website migration.

You can also use HTTP status codes for specific paths to signal temporary maintenance (`503 Service Not Available`) or intentionally-deleted content (`410 Gone`).

Ferron has `status` directive for configuring both custom HTTP status codes, and redirects.

> [!info]
> If you want to rewrite URLs instead of redirecting them, see [URL rewriting](/docs/v3/use-cases/traffic/url-rewriting).

## Domain canonicalization

A common pattern is redirecting URLs from `www` subdomain to an apex domain (or apex domain to `www` subdomain):

```ferron
www.example.com {
    # 301 - Moved Permanently (permanent redirect with always GET)
    # 302 - Found (temporary redirect with always GET)
    # 307 - Temporary Redirect (temporary redirect with HTTP method preservation)
    # 308 - Permanent Redirect (permanent redirect with HTTP method preservation)
    status 308 {
        location "https://example.com{{request.uri}}"
    }
}

example.org {
    status 308 {
        location "https://www.example.org{{request.uri}}"
    }
}
```

## Moving domain names

If you want to move domain names, use redirects to help with that:

```ferron
old.example.com {
    status 308 {
        location "https://example.com{{request.uri}}"
    }
}

example.com {
    root /var/www/html
}
```

## Legacy route migration

Some websites have legacy routes that need to be migrated (for example, for legacy clients or software). In this case you can configure Ferron below:

```ferron
blog.example.com {
    status 308 {
        regex r"^/blog($|[/?#].*)"
        location "/posts$1"
    }
}
```

## Intentional removal

For intentionally-removed resources, use this configuration:

```ferron
old.example.com {
    # 410 Gone
    status 410 {
        url /posts/some-deleted-content
    }
}
```

## Maintenance mode

When closing a website for maintenance, use this configuration:

```ferron
example.com {
    # 503 Service Unavailable
    status 503
}
```

> [!tip]
> You can also configure custom maintenance error page, see [Error pages](/docs/v3/use-cases/traffic/error-pages).
