---
title: "Configuration: HTTP basic authentication"
description: "HTTP Basic Authentication with hashed passwords, brute-force protection, and forward proxy support."
---

This page documents the `basic_auth` directive for configuring HTTP Basic Authentication for request-level access control. Only **hashed passwords** are supported — plaintext passwords are rejected at configuration validation time for security reasons.

## `basic_auth`

```ferron
example.com {
    basic_auth {
        realm "Restricted Area"
        users {
            alice "$argon2id$v=19$m=19456,t=2,p=1$..."
            bob "$argon2id$v=19$m=19456,t=2,p=1$..."
        }

        brute_force_protection {
            enabled true
            max_attempts 5
            lockout_duration "15m"
            window "5m"
        }
    }
}
```

Multiple `basic_auth` blocks can be defined — users from all blocks are merged.

| Nested directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `realm` | `<string>` | Authentication realm shown in the browser auth dialog. | `Restricted Access` |
| `users` | block | User credentials block (username to hash mappings). Required. | — |
| `brute_force_protection` | block | Brute-force attack protection settings. | enabled (see below) |

### `users` block

Each entry inside the `users` block maps a username to a **hashed password**:

```ferron
users {
    alice "$argon2id$v=19$m=19456,t=2,p=1$..."
    bob "$argon2id$v=19$m=19456,t=2,p=1$..."
}
```

**Only hashed passwords are accepted.** The following hash formats are supported:

| Prefix | Algorithm |
| --- | --- |
| `$argon2id$` | Argon2id (recommended) |
| `$argon2i$` | Argon2i |
| `$argon2d$` | Argon2d |
| `$pbkdf2$` | PBKDF2 |
| `$pbkdf2-sha256$` | PBKDF2-SHA256 |
| `$scrypt$` | scrypt |

### `brute_force_protection` block

Brute-force protection is **enabled by default** to protect against credential-guessing attacks.

| Nested directive | Type | Default | Description |
| --- | --- | --- | --- |
| `enabled` | `<bool>` | `true` | Whether brute-force protection is active. |
| `max_attempts` | `<int>` | `5` | Maximum failed attempts before lockout. |
| `lockout_duration` | `<duration>` | `15m` | How long to lock the account after exceeding max attempts. |
| `window` | `<duration>` | `5m` | Sliding window for counting attempts. |

Duration strings accept suffixes: `30s`, `15m`, `1h`, `1d`. Plain numbers without a suffix are treated as seconds.

### Authentication flow

1. The stage extracts the `Authorization: Basic <credentials>` header from the request.
2. If the header is missing or malformed, a 401 response is returned with a `WWW-Authenticate` challenge.
3. The credentials are decoded from base64 (`username:password`).
4. Brute-force lockout is checked — if the account is locked, the request is rejected immediately.
5. The username is looked up in the configured `users` block.
6. If the user exists, the password is verified against the stored hash.
7. On success, `ctx.auth_user` is set to the authenticated username and brute-force history is cleared.
8. On failure, the attempt is recorded and a 401 response is returned.

### Forward proxy (CONNECT) support

When a CONNECT request is received and authentication fails, a **407 Proxy Authentication Required** response is returned instead of 401.

### Brute-force protection behavior

When brute-force protection is enabled:

- Each failed authentication attempt is recorded per-username with a timestamp.
- If `max_attempts` failures occur within the `window` duration, the account is locked.
- During lockout, **all** authentication attempts for that username are rejected immediately.
- After `lockout_duration`, the lockout expires and the attempt history is reset.
- On successful authentication, the attempt history is cleared for that user.

### Stage ordering

The `basic_auth` stage runs early in the pipeline:

- **After** `client_ip_from_header` (ensures accurate remote address)
- **Before** `forward_proxy` (auth before forwarding)
- **Before** `reverse_proxy` (auth before proxying)
- **Before** `static_file` (auth before serving files)

## Examples

### Basic authentication with Argon2 hashes

```ferron
admin.example.com {
    basic_auth {
        realm "Admin Panel"
        users {
            admin "$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$..."
        }
    }

    root /var/www/admin
}
```

### Forward proxy with authentication

```ferron
proxy.example.com {
    basic_auth {
        realm "Proxy Access"
        users {
            user1 "$argon2id$v=19$m=19456,t=2,p=1$..."
            user2 "$argon2id$v=19$m=19456,t=2,p=1$..."
        }

        brute_force_protection {
            max_attempts 3
            lockout_duration "30m"
            window "10m"
        }
    }

    forward_proxy {
        allow_domains "example.com" "*.example.com"
        allow_ports 80 443
    }
}
```

### Disabling brute-force protection

```ferron
example.com {
    basic_auth {
        realm "Behind WAF"
        users {
            deploy "$argon2id$v=19$m=19456,t=2,p=1$..."
        }

        brute_force_protection {
            enabled false
        }
    }
}
```

> **Warning:** Disabling brute-force protection exposes your users to credential-guessing attacks. Only do this if you have equivalent protection at another layer.

## Security considerations

- **Always use TLS.** Basic Auth credentials are sent in the `Authorization` header, which is base64-encoded (not encrypted). Without TLS, credentials can be intercepted in transit.
- **Use Argon2id.** This is the recommended algorithm for password hashing — it is resistant to GPU-based attacks and side-channel attacks.
- **Use strong passwords.** The security of the hash depends on the entropy of the original password.
- **Plaintext passwords are rejected.** This module does not support plaintext passwords at all.
- **Brute-force protection is enabled by default.** This provides a reasonable baseline of protection without requiring additional configuration.

## Notes and troubleshooting

- The `realm` value is shown in the browser's authentication dialog.
- Unknown users are still tracked for brute-force purposes — repeated attempts with a non-existent username will eventually trigger a lockout for that username.
- Configuration validation fails if any password value is not a recognized hash format.
- This module does not currently support session-based authentication — credentials are checked on every request.
- For forward proxy configuration, see [Forward proxy](/docs/v3/configuration/http-fproxy).
- For routing and URL processing, see [Routing and URL processing](/docs/v3/configuration/routing-url-processing).
