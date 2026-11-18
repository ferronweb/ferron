---
title: Status codes & redirects
description: "Return custom HTTP status codes and redirect old URLs with the status directive."
---

The `status` directive returns a custom status code or a redirect.

Redirects keep search engines from indexing duplicate content and keep old links working after a migration. Status codes also signal maintenance (`503 Service Unavailable`) or removed content (`410 Gone`).

> [!info]
> To rewrite a URL without a redirect, see [URL rewriting](/docs/use-cases/traffic/url-rewriting).

## Redirect variants

| Code | Meaning              | Method and body |
| ---- | -------------------- | --------------- |
| 301  | Moved Permanently    | Changed to GET  |
| 302  | Found                | Changed to GET  |
| 307  | Temporary Redirect   | Preserved       |
| 308  | Permanent Redirect   | Preserved       |

Use 307 or 308 when the client must keep the method and body.

## Canonical domain

Redirect between a `www` subdomain and the apex domain:

```ferron
www.example.com {
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

## Move to a new domain

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

## Migrate a legacy route

Use a regex to redirect a group of old paths to new paths:

```ferron
blog.example.com {
    status 308 {
        regex r"^/blog($|[/?#].*)"
        location "/posts$1"
    }
}
```

## Remove content

Return `410 Gone` for content that will not come back:

```ferron
old.example.com {
    status 410 {
        url /posts/some-deleted-content
    }
}
```

## Maintenance mode

Return `503 Service Unavailable` for the whole host:

```ferron
example.com {
    status 503
}
```

> [!tip]
> Add an `error_page` rule to serve a custom maintenance page. See [Error pages](/docs/use-cases/traffic/error-pages).
