---
title: Access control
description: "Protect routes with IP rules, Basic Auth, forwarded authentication, and conditional matchers."
---

Ferron has several access control patterns. Use `allow` and `block` for IP rules, `basic_auth` for authenticated areas, or `auth_to` for an external authentication backend.

> [!tip]
> Test restrictive rules on a temporary path before you protect a real admin path. A wrong rule can lock you out.

## Restrict a path by client IP

List the allowed networks in a `location` block:

```ferron
example.com {
    location / {
        root /var/www/html
    }

    location /admin {
        allow "203.0.113.0/24" "2001:db8:1234::/48"
        root /var/www/admin
    }
}
```

When `allow` is present, only the listed IP addresses and CIDR ranges get access. All other clients receive `403 Forbidden`.

> [!important]
> Behind a reverse proxy or load balancer, set `client_ip_from_header`. Without it, the rules see the proxy IP instead of the client IP. See [HTTP host directives](/docs/configuration/server/host).

### Combine block and allow

When a request matches both lists, `block` wins:

- Matches `allow` and `block`: blocked.
- Matches only `allow`: allowed.
- Matches only `block`: blocked.
- Matches neither: allowed, unless `allow` is present. In that case, Ferron denies clients that do not match an `allow` entry.

```ferron
example.com {
    root /var/www/html

    allow "192.168.1.0/24"
    block "192.168.1.100"
}
```

In this example, `192.168.1.50` is allowed. `192.168.1.100` and `10.0.0.1` are denied.

### Block addresses for all hosts

Use a global block to deny an address on every host:

```ferron
* {
    block "198.51.100.10" "203.0.113.0/24"
}
```

## Protect an area with Basic Auth

```ferron
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

Ferron accepts only hashed passwords:

| Prefix                 | Algorithm              |
| ---------------------- | ---------------------- |
| `$argon2id$`           | Argon2id (recommended) |
| `$argon2i$`            | Argon2i                |
| `$argon2d$`            | Argon2d                |
| `$pbkdf2$`             | PBKDF2                 |
| `$pbkdf2-sha256$`      | PBKDF2-SHA256          |
| `$scrypt$`             | scrypt                 |
| `$2a$`, `$2b$`, `$2y$` | bcrypt                 |

> [!important]
> Use TLS with Basic Auth. The client sends the `Authorization` header on every request, so credentials are exposed without TLS.

### Brute-force protection

Brute-force protection is enabled by default. To tune it:

```ferron
example.com {
    location /admin {
        basic_auth {
            realm "Admin Area"
            users {
                admin "$argon2id$v=19$m=19456,t=2,p=1$..."
            }

            brute_force_protection {
                max_attempts 5
                lockout_duration "15m"
                window "5m"
            }
        }
    }
}
```

To disable it, set `brute_force_protection false`.

## Conditional access control

Use a named matcher with `if_not` when the rule needs pattern matching:

```ferron
match internal_network {
    request.header.x_forwarded_for ~ r"^10\.0\.0\."
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

This closes the connection for requests outside `10.0.0.0/8`.

## Deny sensitive paths

Match dotfiles and common private paths, then return `403`:

```ferron
match sensitive_path {
    request.uri.path ~ r"^/(?:\.|config|private|backup)"
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

## Forwarded authentication

`auth_to` sends each request to an authentication backend before Ferron handles it:

```ferron
example.com {
    auth_to http://auth.example.com/auth

    root /var/www/html
}
```

A 2xx response continues the request. A 4xx or 5xx response is returned to the client. To copy headers from the auth response to the original request:

```ferron
api.example.com {
    auth_to http://auth.example.com/validate {
        copy X-Auth-User X-Auth-Roles
    }

    proxy http://backend:8080
}
```

> [!info]
> For all options, see [Forwarded authentication](/docs/configuration/security/forwarded-auth).
