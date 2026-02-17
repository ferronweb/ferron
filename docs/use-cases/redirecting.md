---
title: Redirecting
description: "Redirect traffic in Ferron using status + Location, including path-preserving and canonical host redirects."
---

Ferron can issue redirects using `status` with the `location` property. This is useful for domain moves, canonical hostnames, and temporary maintenance routing.

## Redirect an entire site to another URL

Use `302` for temporary redirects or `301` for permanent redirects:

```kdl
// Example configuration with a redirect to another website. Replace "example.org" with your domain name.
example.org {
    status 302 location="https://www.example.com" // Replace with your destination URL; use 301 for permanent redirects.
}
```

## Redirect and preserve request path

If you want to keep the original request path:

```kdl
// Example configuration with a redirect to another website. Replace "example.org" with your domain name.
example.org {
    status 302 location="https://www.example.com{path}" // Replace with your destination host; use 301 for permanent redirects.
}
```

For example, `/docs/page` on `example.org` redirects to `/docs/page` on `www.example.com`.

## Canonical host redirect (non-www -> www)

To redirect all traffic from `example.com` to `www.example.com`:

```kdl
// Example configuration with a redirect from URL without "www." to URL with "www.". Replace "example.com" with your domain name.
example.com {
    status 301 location="https://www.example.com{path}"
}

www.example.com {
    // For this example, let's serve static files
    root "/var/www/example"
}
```

## Canonical host redirect (www -> non-www)

If you prefer non-www as canonical host:

```kdl
// Example configuration with a redirect from URL with "www." to URL without "www.". Replace "example.com" with your domain name.
www.example.com {
    status 301 location="https://example.com{path}"
}

example.com {
    // For this example, let's serve static files
    root "/var/www/example"
}
```

## Notes and troubleshooting

- If browsers keep using old redirects after config changes, clear browser cache or test with a private window; `301` responses are cached aggressively.
- Use `302` while testing and switch to `301` only when you are confident the redirect is final.
- If redirect chains occur, verify only one host/protocol canonicalization rule applies for each request.
- For automatic `www` redirects, see `wwwredirect` in [Configuration: routing & URL processing](/docs/configuration/routing-url-processing).
- For URL rewriting (internal path transforms), see [URL rewriting](/docs/use-cases/url-rewriting).
- For reference of `status` directive and its props, see [Configuration: security & TLS](/docs/configuration/security-tls).
