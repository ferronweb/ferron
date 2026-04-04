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
- TLS: `tls`
- Server information: `admin_email`

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
