---
title: "Configuration: HTTP basic authentication"
description: "HTTP Basic Authentication with hashed passwords, brute-force protection, and forward proxy support."
---

This page documents the `basic_auth` directive for HTTP Basic Authentication that requests use for access control. Ferron supports only **hashed passwords**. For security reasons, plaintext passwords cause a configuration validation error.

## Global directives

### `basic_auth_concurrency`

```ferron
{
    basic_auth_concurrency 64
}
```

This is a **global-only** directive that limits the number of concurrent password verification tasks across all `basic_auth` blocks. Password hashing is computationally expensive, and this limit prevents a flood of authentication requests from exhausting server resources.

| Value type | Description | Default |
| --- | --- | --- |
| `<positive integer>` | Maximum concurrent password verification tasks. | `128` |
| `false` | Disable the limit (unlimited concurrency). | disabled |

**Configuration example: reduce concurrency**

```ferron
{
    basic_auth_concurrency 32
}
```

**Configuration example: disable the limit**

```ferron
{
    basic_auth_concurrency false
}
```

**Configuration example: set minimum of 1**

```ferron
{
    basic_auth_concurrency 1
}
```

> [!note]
>
> - Ferron treats values less than `1` as `1`.
> - Setting this too low may cause authentication requests to queue under load, increasing latency.
> - Set this to `false` only if you understand the resource implications, since that removes the limit.
> - When Ferron reaches the limit, further authentication requests wait for a free slot instead of failing.

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
            enabled
            max_attempts 5
            lockout_duration "15m"
            window "5m"
        }
    }
}
```

You can define multiple `basic_auth` blocks. Ferron merges the users from all blocks.

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

**Ferron accepts only hashed passwords.** Ferron supports the following hash formats:

| Prefix | Algorithm |
| --- | --- |
| `$argon2id$` | Argon2id (recommended) |
| `$argon2i$` | Argon2i |
| `$argon2d$` | Argon2d |
| `$pbkdf2$` | PBKDF2 |
| `$pbkdf2-sha256$` | PBKDF2-SHA256 |
| `$scrypt$` | scrypt |

> [!note]
>
> - Use the `ferron-passwd` utility, which ships with Ferron, to create password hashes.
> - Ferron shows the `realm` value in the browser authentication dialog.
> - Configuration validation fails if any password value is not a recognized hash format.

### `brute_force_protection` block

Brute-force protection is **enabled by default** to protect against credential-guessing attacks.

| Nested directive | Type | Default | Description |
| --- | --- | --- | --- |
| `enabled` | `<bool>` | `true` | Whether brute-force protection is active. |
| `max_attempts` | `<int>` | `5` | Maximum failed attempts before lockout. |
| `lockout_duration` | `<duration>` | `15m` | How long to lock the account after exceeding max attempts. |
| `window` | `<duration>` | `5m` | Sliding window for counting attempts. |

Duration strings accept suffixes: `30s`, `15m`, `1h`, `1d`. Ferron treats plain numbers without a suffix as seconds.

### Authentication flow

1. The stage extracts the `Authorization: Basic <credentials>` header from the request.
2. If the Authorization header is absent or malformed, the stage returns a 401 response with a `WWW-Authenticate` challenge.
3. The stage decodes the credentials from base64 (`username:password`).
4. The stage checks the brute-force lockout. A locked IP gets an immediate 429 response.
5. The stage looks up the username in the configured `users` block.
6. If the user exists, the stage verifies the password against the stored hash.
7. On success, the stage sets `ctx.auth_user` to the authenticated username.
8. On failure, the stage records the attempt and returns a 401 response.

### Forward proxy (CONNECT) support

When authentication fails for a CONNECT request, the stage returns a **407 Proxy Authentication Required** response instead of 401.

### Brute-force protection behavior

When brute-force protection is active:

- The stage records each failed authentication attempt per-IP with a timestamp.
- If `max_attempts` failures occur within the `window` duration, Ferron locks the IP.
- During lockout, the stage rejects **all** authentication attempts for that IP immediately.
- After `lockout_duration`, the lockout expires and the stage resets the attempt history.

### Stage ordering

The `basic_auth` stage runs early in the pipeline:

- **After** `client_ip_from_header` (makes sure the remote address is accurate)
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

> [!warning]
> Disabling brute-force protection exposes your users to credential-guessing attacks. Only do this if you have equivalent protection at another layer.

## Security considerations

- **Always use TLS.** Basic Auth credentials travel in the `Authorization` header. Ferron base64-encodes them but does not encrypt them. Without TLS, an attacker can intercept the credentials in transit.
- **Use Argon2id.** This is the recommended algorithm for password hashing. It resists GPU-based attacks and side-channel attacks.
- **Use strong passwords.** The security of the hash depends on the entropy of the original password.
- **Ferron rejects plaintext passwords.** It does not accept plaintext passwords at all.
- **Brute-force protection runs by default.** This gives a reasonable baseline of protection without requiring additional configuration.
- **Tune `basic_auth_concurrency` to your workload.** Setting it too low may cause authentication queuing under high load. Setting it too high may allow a flood of expensive hash operations to exhaust resources.

## Best practices

`ferron doctor` reports the following best-practice checks for directives on this page.

- **`basic_auth_concurrency false`**: Disabling the global password-verification concurrency limit removes backpressure on expensive hash checks. Keep a bounded limit.
- **Non-Argon2id password hashes**: Prefer Argon2id for new Basic Auth credentials. Other hash algorithms are weaker against offline attacks.
- **`brute_force_protection { enabled false }`**: Disabling credential-guessing protection removes a layer of security. Only disable when equivalent protection exists at another layer.

## Observability

### Access log fields

The basic authentication module contributes the following field to the HTTP access log line:

| Field | Type | Description |
| --- | --- | --- |
| `ferron.basicauth.result` | string | Auth outcome: `skip`, `failure`, or `success`. |

### Trace spans

The basic authentication stage sets the following attributes on its `ferron.stage.basicauth` span:

| Attribute | Type | Description |
| --- | --- | --- |
| `ferron.basicauth.result` | string | Authentication result: `success`, `failure`, or `skip`. |
| `user.name` | string | The authenticated username, on success. |
| `error.type` | string | Set to `auth_failed` on authentication failure, enabling trace UI highlighting. |
