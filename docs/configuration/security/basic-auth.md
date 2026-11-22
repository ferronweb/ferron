---
title: "Configuration: HTTP basic authentication"
description: "HTTP Basic Authentication with hashed passwords, brute-force protection, and forward proxy support."
---

The `basic_auth` directive protects a host or location with HTTP Basic Authentication. Ferron accepts only hashed passwords. A plaintext password is a configuration validation error.

## basic_auth_concurrency

`basic_auth_concurrency` is global-only. It limits concurrent password verification tasks across all `basic_auth` blocks. Password hashing is expensive, and the limit prevents a flood of authentication requests from exhausting server resources.

| Value               | Behavior                                            |
| ------------------- | --------------------------------------------------- |
| `<positive integer>` | Maximum concurrent verification tasks. Default `128`. |
| `false`             | No limit.                                           |

```ferron
{
    basic_auth_concurrency 32
}
```

- Values less than `1` are treated as `1`.
- When Ferron reaches the limit, further authentication requests wait for a free slot instead of failing.
- A low limit queues authentication under load and increases latency.
- `false` removes the limit. Set it only when you understand the resource impact.

## basic_auth

```ferron
example.com {
    basic_auth {
        realm "Restricted Area"
        users {
            alice "$argon2id$v=19$m=19456,t=2,p=1$..."
            bob "$argon2id$v=19$m=19456,t=2,p=1$..."
        }

        brute_force_protection {
            max_attempts 5
            lockout_duration "15m"
            window "5m"
        }
    }
}
```

You can define multiple `basic_auth` blocks. Ferron merges the users from all blocks.

> [!note]
> A named host does not inherit `basic_auth` from the wildcard `*` host. It still inherits from global defaults. A `location` block inherits `basic_auth` from its own host.

| Nested directive         | Arguments        | Description                                                                       | Default             |
| ------------------------ | ---------------- | --------------------------------------------------------------------------------- | ------------------- |
| `realm`                  | `<string>`       | Authentication realm shown in the browser auth dialog.                            | `Restricted Access` |
| `users`                  | block            | User credentials block (username to hash mappings). Required.                     | none                |
| `brute_force_protection` | block or `false` | Brute-force protection settings. Set to `false` to disable the protection.         | enabled (see below) |

### users block

Each entry maps a username to a hashed password:

```ferron
users {
    alice "$argon2id$v=19$m=19456,t=2,p=1$..."
    bob "$argon2id$v=19$m=19456,t=2,p=1$..."
}
```

Ferron supports these hash formats:

| Prefix            | Algorithm              |
| ----------------- | ---------------------- |
| `$argon2id$`      | Argon2id (recommended) |
| `$argon2i$`       | Argon2i                |
| `$argon2d$`       | Argon2d                |
| `$pbkdf2$`        | PBKDF2 (HMAC-SHA1)     |
| `$pbkdf2-sha256$` | PBKDF2-SHA256          |
| `$pbkdf2-sha384$` | PBKDF2-SHA384          |
| `$pbkdf2-sha512$` | PBKDF2-SHA512          |
| `$scrypt$`        | scrypt                 |
| `$2a$`            | bcrypt (legacy)        |
| `$2b$`            | bcrypt (modern)        |
| `$2x$`            | bcrypt (broken PHP)    |
| `$2y$`            | bcrypt (PHP default)   |

- Use `ferron-passwd`, which ships with Ferron, to create password hashes.
- Ferron embeds `realm` in the authentication challenge header, so it must not contain control bytes or line breaks. `ferron validate` reports an unusable value.
- Configuration validation fails if any password value is not a recognized hash format.
- Base64 salt and hash fields accept padded and unpadded encodings.
- PBKDF2 hashes accept a bare iteration count (`$pbkdf2-sha256$600000$...`) or the PHC parameter form (`$pbkdf2-sha256$i=600000,l=32$...`).

### brute_force_protection block

Brute-force protection is enabled by default.

| Nested directive   | Type         | Default | Description                                                |
| ------------------ | ------------ | ------- | ---------------------------------------------------------- |
| `max_attempts`     | `<int>`      | `5`     | Maximum failed attempts before lockout.                    |
| `lockout_duration` | `<duration>` | `15m`   | How long to lock the account after exceeding max attempts. |
| `window`           | `<duration>` | `5m`    | Sliding window for counting attempts.                      |

Duration strings accept suffixes: `30s`, `15m`, `1h`, `1d`. Plain numbers without a suffix are seconds.

When brute-force protection is active:

- The stage records each failed authentication attempt per IP with a timestamp.
- If `max_attempts` failures occur within `window`, Ferron locks the IP.
- During lockout, the stage rejects all authentication attempts for that IP and returns `429 Too Many Requests` with a `Retry-After` header.
- After `lockout_duration`, the lockout expires and the stage resets the attempt history.

### Forward proxy (CONNECT) support

For a CONNECT request, failed authentication returns `407 Proxy Authentication Required` instead of `401`.

## Examples

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

### Disable brute-force protection

```ferron
example.com {
    basic_auth {
        realm "Behind WAF"
        users {
            deploy "$argon2id$v=19$m=19456,t=2,p=1$..."
        }

        brute_force_protection false
    }
}
```

> [!warning]
> Disabling brute-force protection exposes your users to credential-guessing attacks. Only do this if you have equivalent protection at another layer.

## Security considerations

Basic Auth credentials travel in the `Authorization` header as base64, which is encoding, not encryption. Without TLS, an attacker can intercept them in transit. Always use TLS for authenticated hosts.

- Use Argon2id for password hashes. It resists GPU-based attacks and side-channel attacks.
- The security of a hash depends on the entropy of the original password. Use strong passwords.
- Brute-force protection runs by default and gives a baseline of protection without extra configuration.
- Tune `basic_auth_concurrency` to your workload. Too low queues authentication under high load. Too high allows a flood of expensive hash operations to exhaust resources.

## Observability

### Access log fields

| Field                     | Type   | Description                                    |
| ------------------------- | ------ | ---------------------------------------------- |
| `ferron.basicauth.result` | string | Auth outcome: `skip`, `failure`, or `success`. |

### Trace spans

The authentication stage sets these attributes on its `ferron.stage.basicauth` span:

| Attribute                 | Type   | Description                                                                     |
| ------------------------- | ------ | ------------------------------------------------------------------------------- |
| `ferron.basicauth.result` | string | Authentication result: `success`, `failure`, or `skip`.                         |
| `user.name`               | string | The authenticated username, on success.                                         |
| `error.type`              | string | Set to `auth_failed` on authentication failure, enabling trace UI highlighting. |

## Best practices

`ferron doctor` reports the following best-practice checks for directives on this page.

- `basic_auth_concurrency false`: Disabling the global password-verification concurrency limit removes backpressure on expensive hash checks. Keep a bounded limit.
- Non-Argon2id password hashes: Prefer Argon2id for new Basic Auth credentials. Other hash algorithms are weaker against offline attacks.
- `brute_force_protection false`: Disabling credential-guessing protection removes a layer of security. Only disable it when equivalent protection exists at another layer.
