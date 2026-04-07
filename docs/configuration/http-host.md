# HTTP Host Directives

These directives are consumed from HTTP host blocks such as:

```ferron
example.com {
}

http example.com:8080 {
}
```

## Categories

- Protocol behavior: `http`
- TLS: `tls` (automatic for non-localhost hostnames)
- HTTPS redirect: `https_redirect`
- Client IP from headers: `client_ip_from_header`
- Server information: `admin_email`
- Authentication: `basicauth`

## Automatic TLS

When a hostname is specified (e.g. `example.com`) and no explicit port is given, Titanium starts **two listeners**:

- One on `default_http_port` (default: 80) — serves plain HTTP with no TLS
- One on `default_https_port` (default: 443) — serves HTTPS with automatic ACME TLS

On the HTTPS listener, if no explicit `tls` directive is present, Titanium **automatically enables TLS via the ACME provider** (Let's Encrypt by default). Certificates are obtained and renewed automatically at startup.

Hostnames that are **exempt** from the HTTPS listener and automatic TLS:

- `localhost`
- `127.0.0.1`
- `::1`

These hostnames only get the HTTP listener — no HTTPS listener is started for them unless they specify an explicit `tls` directive (in which case they are included in the HTTPS listener with the configured TLS).

To disable automatic TLS for a specific host on the HTTPS listener, use `tls false`:

```ferron
example.com {
    tls false
    root /var/www/html
}
```

To use manual TLS instead:

```ferron
example.com {
    tls {
        provider "manual"
        cert "/etc/ssl/cert.pem"
        key "/etc/ssl/key.pem"
    }
    root /var/www/html
}
```

When an **explicit port** is specified (e.g., `example.com:8080`), only a single listener is started on that port, and no automatic ACME TLS is applied — you must configure TLS explicitly.

See [ACME Automatic TLS](./tls-acme.md) for full ACME configuration details.

## HTTPS Redirect

When TLS is enabled (either automatically via ACME or explicitly), Titanium automatically redirects plain HTTP requests to their HTTPS equivalent.

### How it works

The redirect stage runs early in the HTTP pipeline and checks:

1. **TLS is enabled somewhere** — `ctx.https_port` is set (a separate HTTPS listener exists)
2. **Not already encrypted** — the request arrived over plain HTTP
3. **Not `X-Forwarded-Proto: https`** — avoids redirect loops behind TLS-terminating proxies
4. **Not a localhost hostname** — `localhost`, `127.0.0.1`, and `::1` are skipped since no HTTPS listener exists for them
5. **Listener port differs from HTTPS port** — when an explicit port is specified (e.g., `example.com:8080`), no separate HTTPS listener exists, so redirect is skipped

### `https_redirect`

Syntax:

```ferron
example.com {
    https_redirect false
}
```

| Arguments | Description | Default |
| --- | --- | --- |
| `<bool>` | Enables or disables automatic HTTP-to-HTTPS redirects. | `true` (when TLS is enabled) |

Notes:

- The redirect uses **308 Permanent Redirect**, which preserves the HTTP method and request body.
- The redirect respects the `X-Forwarded-Proto: https` header to avoid redirect loops when behind a TLS-terminating reverse proxy.
- `localhost` hostnames never get redirected — there is no HTTPS listener for them.
- When an explicit port is specified (e.g., `example.com:8080`), no redirect is performed since no separate HTTPS listener exists.
- The target port is `default_https_port` (default: `443`). When the port is `443`, it is omitted from the URL.

## Client IP from Forwarded Headers

When running behind a reverse proxy, load balancer, or CDN, the actual client IP address is typically forwarded via HTTP headers rather than being available from the TCP socket. The `client_ip_from_header` directive instructs Titanium to read the client IP from the specified header and make it available as `ctx.remote_address` for downstream stages, matchers, and logging.

### `client_ip_from_header`

Syntax:

```ferron
{
    client_ip_from_header x-forwarded-for
}
```

| Arguments | Description | Default |
| --- | --- | --- |
| `<string>` | Header to read the client IP from. Supported values: `x-forwarded-for`, `forwarded`. | disabled |

### Supported Headers

#### `x-forwarded-for`

Reads the `X-Forwarded-For` header and extracts the **first (leftmost)** IP address from the comma-separated chain:

```
X-Forwarded-For: 192.0.2.1, 10.0.0.1, 172.16.0.1
                  ^^^^^^^^^
                  Titanium uses this IP
```

Example:

```ferron
{
    client_ip_from_header x-forwarded-for
}

example.com {
    root /var/www/html
}
```

#### `forwarded` (RFC 7239)

Reads the `Forwarded` header and extracts the first `for=` token:

```
Forwarded: for=192.0.2.60;proto=https, for=10.0.0.1;proto=http
           ^^^^^^^^^^^^^
           Titanium uses this IP
```

Both quoted and unquoted values are supported:

```ferron
Forwarded: for="192.0.2.60";proto=https
Forwarded: for=192.0.2.60;proto=https
```

IPv6 addresses are also supported:

```
Forwarded: for="[2001:db8::1]"
```

Example:

```ferron
{
    client_ip_from_header forwarded
}

example.com {
    root /var/www/html
}
```

### Notes

- **Disabled by default.** The stage is only active when the directive is explicitly set.
- **Trust boundary.** This directive blindly trusts the header value. If the server is directly exposed to the internet (not behind a trusted proxy), a malicious client could spoof their IP address. Only enable this when running behind a trusted reverse proxy or load balancer.
- **Port preservation.** Only the IP address is replaced — the original remote port from the TCP socket is preserved.
- **Invalid or missing values.** If the header is absent, malformed, or contains an unparseable value (e.g., `for=_hidden`), the stage silently skips and `ctx.remote_address` remains unchanged.
- **Matcher variables.** The updated `remote_address` is available in conditional matchers and logging via the standard mechanisms.

## `http`

Syntax:

```ferron
example.com {
    http {
        protocols h1 h2
        timeout 30m
        h1_enable_early_hints false
        h2_initial_window_size 65535
        h2_max_frame_size 32768
        h2_max_concurrent_streams 128
        h2_max_header_list_size 16384
        h2_enable_connect_protocol false
    }
}
```

| Nested directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `protocols` | `<string>...` | Enabled HTTP protocols. Currently supported values are `h1` and `h2`. | `h1 h2` |
| `timeout` | `<duration>`, `<number>`, or `false` | Pipeline execution timeout. Accepts a duration string (e.g., `30m`, `1h`, `90s`), a number in milliseconds, or `false` to disable. When a timeout occurs, a 408 Request Timeout response is returned. | `300000` (5 minutes) |
| `h1_enable_early_hints` | `<bool>` | Enables HTTP/1.1 early hints support. | `false` |
| `h2_initial_window_size` | `<number>` | HTTP/2 initial flow-control window size. Must resolve to a non-negative integer at runtime. | unset |
| `h2_max_frame_size` | `<number>` | HTTP/2 maximum frame size. Must resolve to a non-negative integer at runtime. | unset |
| `h2_max_concurrent_streams` | `<number>` | HTTP/2 maximum concurrent streams. Must resolve to a non-negative integer at runtime. | unset |
| `h2_max_header_list_size` | `<number>` | HTTP/2 maximum header list size. Must resolve to a non-negative integer at runtime. | unset |
| `h2_enable_connect_protocol` | `<bool>` | Enables the HTTP/2 extended CONNECT protocol setting. | `false` |

Notes:

- `protocols` must leave at least one supported protocol enabled.
- `h3` is currently rejected.
- The `timeout` directive applies to both the main pipeline execution and file pipeline execution. If a timeout occurs, a 408 response is returned and the event is logged.

## `tls`

Preferred syntax:

```ferron
example.com {
    tls {
        provider manual
        cert "{{env.TLS_CERT}}"
        key "{{env.TLS_KEY}}"
    }
}
```

Accepted by the current validator:

- `tls [<bool>] { ... }`
- `tls <string> <string>`

| Nested directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `provider` | `<string>` | TLS provider name. Required when TLS is enabled through the block form. | none |

Current runtime behavior:

- If `tls` is absent, TLS is disabled for that host.
- If `tls false { ... }` is used, the block is ignored.
- The HTTP server currently reads the nested `provider` directive and then delegates the rest of the block to the selected TLS provider.

Bundled provider-specific options:

### `provider manual`

The bundled `manual` TLS provider reads:

| Nested directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `cert` | `<string>` | Path to a PEM certificate file. Interpolation is supported. | none |
| `key` | `<string>` | Path to a PEM private key file. Interpolation is supported. | none |

#### `cert`

Path to a PEM-encoded certificate file. Must contain at least one certificate.

```ferron
example.com {
    tls {
        provider manual
        cert "/etc/ssl/example.com/cert.pem"
        key "/etc/ssl/example.com/key.pem"
    }
}
```

#### `key`

Path to a PEM-encoded private key file.

#### `ticket_keys`

Configures TLS session ticket key management with optional automatic rotation.
See the [TLS Session Ticket Keys](./tls-session-tickets.md) page for full details.

```ferron
example.com {
    tls {
        provider manual
        cert "/etc/ssl/example.com/cert.pem"
        key "/etc/ssl/example.com/key.pem"
        ticket_keys {
            file "/etc/ssl/example.com/session_tickets.keys"
            auto_rotate true
            rotation_interval "12h"
            max_keys 3
        }
    }
}
```

| Nested directive | Type | Default | Description |
| --- | --- | --- | --- |
| `file` | `<string>` | - | Path to the ticket key file (required) |
| `auto_rotate` | `<bool>` | `false` | Enable automatic key rotation |
| `rotation_interval` | `<duration>` | `12h` | How often to rotate keys |
| `max_keys` | `<int>` | `3` | Maximum keys to retain (2-5) |

#### `ocsp`

Configures OCSP stapling for this TLS configuration. OCSP stapling is **enabled by default**.
See the [OCSP Stapling](./ocsp-stapling.md) page for full details.

```ferron
# Explicitly enable (default behavior)
example.com {
    tls {
        provider manual
        cert "/etc/ssl/example.com/cert.pem"
        key "/etc/ssl/example.com/key.pem"
        ocsp {
            enabled true
        }
    }
}

# Disable OCSP stapling
example.com {
    tls {
        provider manual
        cert "/etc/ssl/example.com/cert.pem"
        key "/etc/ssl/example.com/key.pem"
        ocsp {
            enabled false
        }
    }
}

# Bare directive — also enables (same as default)
example.com {
    tls {
        provider manual
        cert "/etc/ssl/example.com/cert.pem"
        key "/etc/ssl/example.com/key.pem"
        ocsp
    }
}
```

| Nested directive | Type | Default | Description |
| --- | --- | --- | --- |
| `enabled` | `<bool>` | `true` | Whether OCSP stapling is active |

When enabled, certificates with the OCSP Must-Staple extension (TLS Feature `status_request`, RFC 7633) are detected and preloaded immediately for faster initial stapling.

#### `cipher_suite`, `ecdh_curve`, `min_version`, `max_version`, `client_auth`, `client_auth_ca`

These directives configure cipher suites, key exchange groups, TLS protocol versions, and mutual TLS (mTLS).
See the [TLS Crypto Settings and mTLS](./tls-crypto.md) page for the full reference.

Quick example:

```ferron
example.com {
    tls {
        provider manual
        cert "/etc/ssl/example.com/cert.pem"
        key "/etc/ssl/example.com/key.pem"
        cipher_suite TLS_AES_256_GCM_SHA384
        ecdh_curve x25519
        min_version TLSv1.3
        max_version TLSv1.3
        client_auth true
        client_auth_ca "/etc/ssl/internal-ca/ca-bundle.pem"
    }
}
```

## `admin_email`

Syntax:

```ferron
{
    admin_email "admin@example.com"
}
```

| Arguments | Description | Default |
| --- | --- | --- |
| `<string>` | Server administrator's email address. Used in built-in error responses. Interpolation is supported. | none |

Notes:

- This directive is validated but the email is not yet included in error responses (reserved for future use).

## Notes

- These directives are host-scoped rather than global.
- HTTP host directives are consumed at runtime, but they are not yet registered as per-protocol validators in the current loader path.
