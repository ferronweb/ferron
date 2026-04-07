# HTTP Basic Authentication

The `basic_auth` directive configures HTTP Basic Authentication for request-level access control.
When configured, requests without valid credentials receive a 401 Unauthorized response with a
`WWW-Authenticate` header. For CONNECT requests (forward proxy), a 407 Proxy Authentication
Required response is returned instead.

Only **hashed passwords** are supported — plaintext passwords are rejected at configuration
validation time for security reasons.

## Overview

- HTTP Basic Authentication (RFC 7617)
- Hashed passwords only (Argon2, PBKDF2, scrypt)
- Built-in brute-force protection enabled by default
- Per-username attempt tracking with automatic lockout
- Works with both regular and forward proxy (CONNECT) requests
- Sets `ctx.auth_user` on successful authentication for downstream use

## Syntax

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
            lockout_duration 15m
            window 5m
        }
    }
}
```

Multiple `basic_auth` blocks can be defined — users from all blocks are merged:

```ferron
example.com {
    basic_auth {
        realm "Admin Area"
        users {
            admin "$argon2id$v=19$m=19456,t=2,p=1$..."
        }
    }

    basic_auth {
        users {
            deploy "$argon2id$v=19$m=19456,t=2,p=1$..."
        }
    }
}
```

## Directives

| Nested directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `realm` | string | Authentication realm shown in the browser auth dialog | `Restricted Access` |
| `users` | block | User credentials block (username → hash mappings) | — (required) |
| `brute_force_protection` | block | Brute-force attack protection settings | enabled (see below) |

### `users` Block

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

Plaintext passwords are **rejected at configuration validation time** with a clear error message.

### Generating Password Hashes

You can generate Argon2 hashes using the `argon2` CLI tool or any compatible library.
Example using the `argon2` command-line tool:

```bash
echo -n "mysecretpassword" | argon2 "$(openssl rand -base64 16)" -id -t 2 -m 16 -p 1
```

Or using Python with the `argon2-cffi` library:

```python
from argon2 import PasswordHasher
ph = PasswordHasher()
hash = ph.hash("mysecretpassword")
print(hash)
```

### `brute_force_protection` Block

Brute-force protection is **enabled by default** to protect against credential-guessing attacks.

| Nested directive | Type | Default | Description |
| --- | --- | --- | --- |
| `enabled` | boolean | `true` | Whether brute-force protection is active |
| `max_attempts` | integer | `5` | Maximum failed attempts before lockout |
| `lockout_duration` | duration | `15m` | How long to lock the account after exceeding max attempts |
| `window` | duration | `5m` | Sliding window for counting attempts |

### Duration Strings

The `lockout_duration` and `window` directives accept duration values:

| Suffix | Unit | Example | Result |
| --- | --- | --- | --- |
| `s` or `S` | Seconds | `30s` | 30 seconds |
| `m` or `M` | Minutes | `15m` | 15 minutes (900 seconds) |
| `h` or `H` | Hours | `1h` | 1 hour (3600 seconds) |
| `d` or `D` | Days | `1d` | 1 day (86400 seconds) |
| (none) | Seconds | `900` | 900 seconds |

## Behavior

### Authentication Flow

1. The stage extracts the `Authorization: Basic <credentials>` header from the request.
2. If the header is missing or malformed, a 401 response is returned with a `WWW-Authenticate` challenge.
3. The credentials are decoded from base64 (`username:password`).
4. Brute-force lockout is checked — if the account is locked, the request is rejected immediately.
5. The username is looked up in the configured `users` block.
6. If the user exists, the password is verified against the stored hash using `password-auth`.
7. On success, `ctx.auth_user` is set to the authenticated username and brute-force history is cleared.
8. On failure, the attempt is recorded and a 401 response is returned.

### Forward Proxy (CONNECT) Support

When a CONNECT request is received and authentication fails, a **407 Proxy Authentication Required**
response is returned instead of 401, with a `Proxy-Authenticate` header instead of `WWW-Authenticate`.
This is consistent with the HTTP/1.1 proxy authentication specification (RFC 7235).

### Brute-Force Protection

When brute-force protection is enabled:

- Each failed authentication attempt is recorded per-username with a timestamp.
- If `max_attempts` failures occur within the `window` duration, the account is locked.
- During lockout, **all** authentication attempts for that username are rejected immediately.
- After `lockout_duration`, the lockout expires and the attempt history is reset.
- On successful authentication, the attempt history is cleared for that user.

This prevents attackers from guessing passwords through repeated trial-and-error, even if they
target a specific username.

### Stage Ordering

The `basic_auth` stage runs early in the pipeline:

- **After** `client_ip_from_header` (ensures accurate remote address)
- **Before** `forward_proxy` (auth before forwarding)
- **Before** `reverse_proxy` (auth before proxying)
- **Before** `static_file` (auth before serving files)

This ensures authentication is checked before any content is served or forwarded.

## Examples

### Basic Authentication with Argon2 Hashes

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

### Forward Proxy with Authentication

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
            lockout_duration 30m
            window 10m
        }
    }

    forward_proxy {
        allow_domains example.com *.example.com
        allow_ports 80 443
    }
}
```

This requires authentication before any forwarding occurs. CONNECT requests that fail
authentication receive a 407 response.

### Disabling Brute-Force Protection

In rare cases (e.g., behind an external WAF that already handles brute-force protection),
you may want to disable it:

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

> **Warning:** Disabling brute-force protection exposes your users to credential-guessing
> attacks. Only do this if you have equivalent protection at another layer.

## Security Considerations

- **Always use TLS.** Basic Auth credentials are sent in the `Authorization` header, which is
  base64-encoded (not encrypted). Without TLS, credentials can be intercepted in transit.
- **Use Argon2id.** This is the recommended algorithm for password hashing — it is resistant
  to GPU-based attacks and side-channel attacks.
- **Use strong passwords.** The security of the hash depends on the entropy of the original
  password. Weak passwords can be cracked even with strong hashing.
- **Plaintext passwords are rejected.** This module does not support plaintext passwords at
  all — the configuration validator will reject any value that is not a recognized hash format.
- **Brute-force protection is enabled by default.** This provides a reasonable baseline of
  protection without requiring additional configuration.

## Notes

- The `realm` value is shown in the browser's authentication dialog.
- Unknown users are still tracked for brute-force purposes — repeated attempts with a
  non-existent username will eventually trigger a lockout for that username.
- Successful authentication clears the brute-force history for that user.
- Configuration validation fails if any password value is not a recognized hash format.
- This module does not currently support session-based authentication — credentials are
  checked on every request.

## See Also

- [HTTP Host Directives](http-host.md)
- [Forward Proxy Directives](http-fproxy.md)
- [HTTP Control Directives](http-control.md) (`location`, `if`, `if_not`)
