---
title: Access control
description: "Protect routes in Ferron with IP-based access control, Basic Auth, and conditional configuration."
---

Ferron supports several access control patterns, from simple IP-based `block`/`allow` rules to authenticated areas with HTTP Basic Authentication.

## Restrict a path by client IP (block/allow)

Use `block` and `allow` directives to control access by IP or CIDR range:

```ferron
# Replace "example.com" with your domain name.
example.com {
    location / {
        root /var/www/html
    }

    // Only allow these networks to access /admin; everyone else gets 403.
    location /admin {
        allow "203.0.113.0/24" "2001:db8:1234::/48"
        root /var/www/admin
    }
}
```

When `allow` is configured, only the listed IPs/CIDRs are permitted. All others receive a **403 Forbidden** response.

### Combined block and allow

When both `block` and `allow` are configured:

1. If the IP matches an `allow` entry **and** a `block` entry → **blocked** (block takes precedence)
2. If the IP matches only an `allow` entry → **allowed**
3. If the IP matches only a `block` entry → **blocked**
4. If the IP matches neither → **allowed** (unless the allow list is non-empty)

```ferron
example.com {
    root /var/www/html

    allow "192.168.1.0/24"
    block "192.168.1.100"
}
```

In this example: `192.168.1.50` → allowed, `192.168.1.100` → blocked, `10.0.0.1` → denied.

### Block sensitive paths globally

Use a global block to deny access across all hosts:

```ferron
* {
    // Block known abusive addresses globally.
    block "198.51.100.10" "203.0.113.0/24"
}
```

## Protect an area with Basic Auth

Use the `basic_auth` directive to require HTTP Basic Authentication:

```ferron
# Replace "example.com" with your domain name.
example.com {
    location / {
        root /var/www/html
    }

    location /admin {
        basic_auth {
            realm "Admin Area"
            users {
                admin "$argon2id$v=19$m=19456,t=2,p=1$..."
            }
        }

        root /var/www/admin
    }
}
```

Only **hashed passwords** are supported. The following hash formats are accepted:

| Prefix | Algorithm |
| --- | --- |
| `$argon2id$` | Argon2id (recommended) |
| `$argon2i$` | Argon2i |
| `$argon2d$` | Argon2d |
| `$pbkdf2$` | PBKDF2 |
| `$pbkdf2-sha256$` | PBKDF2-SHA256 |
| `$scrypt$` | scrypt |

### Brute-force protection

Brute-force protection is **enabled by default**:

```ferron
example.com {
    location /admin {
        basic_auth {
            realm "Admin Area"
            users {
                admin "$argon2id$v=19$m=19456,t=2,p=1$..."
            }

            brute_force_protection {
                enabled true
                max_attempts 5
                lockout_duration 15m
                window 5m
            }
        }
    }
}
```

## Conditional access control

Use named matchers with `if`/`if_not` for more complex access control logic:

```ferron
match internal_network {
    request.header.x_forwarded_for ~ "^10\\.0\\.0\\."
}

example.com {
    location /admin {
        if_not internal_network {
            abort
        }

        root /var/www/admin
    }
}
```

This aborts the connection for any request that does not come from the `10.0.0.0/8` network.

## Deny access to sensitive files

Use conditional matching to block access to dotfiles and sensitive paths:

```ferron
match sensitive_path {
    request.uri.path ~ "^/(?:\\. |config|private|backup)"
}

example.com {
    root /var/www/html

    if sensitive_path {
        status 403 {
            body "Access denied"
        }
    }
}
```

## Notes and troubleshooting

- If Ferron is behind a reverse proxy/load balancer, configure `client_ip_from_header` so IP-based rules use the client IP rather than the proxy IP. See [HTTP host directives](/docs/v3/configuration/http-host).
- Test restrictive rules with a temporary endpoint first to avoid locking yourself out.
- Prefer `location` matches when possible; use conditional matchers only when you need pattern matching.
- For Basic Auth, always use TLS — credentials are sent in the `Authorization` header on every request.
- For complex logic (method/header/path combinations), use conditional configuration. See [Conditionals and variables](/docs/v3/configuration/conditionals).
- For directive details (`block`, `allow`), see [HTTP response control](/docs/v3/configuration/http-response).
- For Basic Auth details, see [HTTP basic authentication](/docs/v3/configuration/http-basicauth).
