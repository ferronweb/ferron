---
title: Access control
description: "Protect routes in Ferron with 403/401 responses, IP filtering, Basic Auth, and allow/block lists."
---

Ferron supports several access control patterns, from simple `403 Forbidden` rules to authenticated areas and IP allow/block lists.

## Restrict a path by client IP (return 403)

Use `status 403` with `not_allowed` to allow only specific IPs or CIDR ranges:

```kdl
// Replace "example.com" with your domain name.
example.com {
    root "/var/www/html"

    // Only allow these networks to access /admin; everyone else gets 403.
    status 403 url="/admin" not_allowed="203.0.113.0/24,2001:db8:1234::/48" body="Access denied"
}
```

## Block sensitive paths with regex (return 403)

Use regex matching to deny access to sensitive files/directories:

```kdl
// Replace "example.com" with your domain name.
example.com {
    root "/var/www/html"

    // Deny dotfiles and selected sensitive paths.
    status 403 regex="/\\."
    status 403 regex="^/(?:config|private|backup)(?:$|[/?#])"
}
```

## Protect an area with Basic Auth (return 401)

Use `status 401` + `users` + `realm` and define users with `user`:

```kdl
// Replace "example.com" with your domain name.
example.com {
    root "/var/www/html"

    status 401 url="/admin" realm="Admin Area" users="admin"
    user "admin" "$2b$10$replace_with_password_hash"
}
```

For password hashes, use `ferron-passwd`.

## Block or allow IPs globally or per host

Use `block` and `allow` for broader access policy:

```kdl
* {
    // Block known abusive addresses globally.
    block "198.51.100.10" "203.0.113.0/24"
}

// Replace "example.com" with your domain name.
example.com {
    root "/var/www/html"

    // Only allow these ranges for this host.
    allow "192.168.1.0/24" "10.0.0.0/8"
}
```

## Notes and troubleshooting

- If Ferron is behind a reverse proxy/load balancer, configure `trust_x_forwarded_for` so IP-based rules use client IP rather than proxy IP.
- Test restrictive rules with a temporary endpoint first to avoid locking yourself out.
- Prefer `url` matches when possible; use `regex` only when you need pattern matching.
- For complex logic (method/header/path combinations), use conditional configuration. See [Configuration: conditionals](/docs/configuration/conditionals).
- For directive details (`status`, `user`, `allow`, `block`, `trust_x_forwarded_for`), see [Configuration: security & TLS](/docs/configuration/security-tls).
