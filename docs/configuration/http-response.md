# HTTP Response Control

The HTTP response module provides directives for returning custom status codes, aborting connections, and IP-based access control.

## Overview

- Return custom HTTP status codes with optional response bodies
- Match requests by exact URL path or regular expression
- Redirect with 3xx status codes and `Location` header
- Immediately abort connections without sending a response
- Block or allow specific IP addresses and CIDR ranges

## `status`

Syntax:

```ferron
example.com {
    status 403
}
```

| Arguments | Description | Default |
| --- | --- | --- |
| `<integer>` | HTTP status code to return (100–599, required) | — |

### Nested directives

When using the block form, you can customize the response further:

| Nested directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `url` | `<string>` | Only apply this status to requests matching this exact path | all requests |
| `regex` | `<string>` | Only apply this status to requests matching this regular expression | all requests |
| `body` | `<string>` | Response body to include | empty body |
| `location` | `<string>` | Redirect destination for 3xx responses | no redirect |

### Examples

#### Return a status code for all requests

```ferron
example.com {
    status 503 {
        body "Service temporarily unavailable"
    }
}
```

#### Return a status code for a specific URL

```ferron
example.com {
    status 404 {
        url "/old-endpoint"
        body "This endpoint has been removed"
    }
}
```

#### Redirect with a custom status

```ferron
example.com {
    status 301 {
        url "/legacy"
        location "/new"
    }
}
```

#### Regex-based status matching

```ferron
example.com {
    status 410 {
        regex "^/api/v1/.*"
        body "API v1 has been deprecated"
    }
}
```

Multiple `status` directives can be defined. They are evaluated in order — the first matching rule wins.

## `abort`

Syntax:

```ferron
example.com {
    abort true
}
```

| Arguments | Description | Default |
| --- | --- | --- |
| `<bool>` | When `true`, immediately close the connection without sending any response | `false` |

### Behavior

When `abort true` is set, the connection is terminated immediately with no HTTP response sent. This is useful for:

- Silently dropping requests from unwanted clients
- Denial-of-service mitigation at the connection level

The client will see a connection reset error rather than an HTTP status code.

### Example

```ferron
example.com {
    abort true
}
```

## `block`

Syntax:

```ferron
example.com {
    block "10.0.0.0/8" "192.168.1.100"
}
```

| Arguments | Description | Default |
| --- | --- | --- |
| `<string>...` | One or more IP addresses or CIDR ranges to block | none |

Blocked IPs receive a **403 Forbidden** response. Both individual IPs and CIDR ranges (e.g., `192.168.1.0/24`) are supported.

### Examples

```ferron
example.com {
    # Block specific IPs
    block "192.168.1.100" "10.0.0.50"

    # Block an entire subnet
    block "203.0.113.0/24"
}
```

## `allow`

Syntax:

```ferron
example.com {
    allow "192.168.1.0/24" "10.0.0.0/8"
}
```

| Arguments | Description | Default |
| --- | --- | --- |
| `<string>...` | One or more IP addresses or CIDR ranges to allow | none (all allowed) |

When `allow` is configured, **only** the listed IPs/CIDRs are permitted. All other IPs receive a **403 Forbidden** response.

### Example

```ferron
internal.example.com {
    # Only allow access from the internal network
    allow "10.0.0.0/8" "172.16.0.0/12" "192.168.0.0/16"
}
```

## Combined block and allow

When both `block` and `allow` are configured:

1. If the IP matches an `allow` entry **and** a `block` entry → **blocked** (block takes precedence)
2. If the IP matches only an `allow` entry → **allowed**
3. If the IP matches only a `block` entry → **blocked**
4. If the IP matches neither → **allowed** (not in the allow list, but not blocked either — unless the allow list is non-empty, in which case non-listed IPs are denied)

### Example

```ferron
example.com {
    # Allow the entire /24 subnet
    allow "192.168.1.0/24"

    # But block one specific host within it
    block "192.168.1.100"
}
```

In this example:
- `192.168.1.50` → allowed
- `192.168.1.100` → blocked (block takes precedence over allow)
- `10.0.0.1` → denied (not in the allow list)

## Scoping

All directives (`status`, `abort`, `block`, `allow`) can be placed at different configuration levels:

- **Host level** — applies to all requests for that host
- **`location` block** — applies only to requests matching that path prefix
- **`if` / `if_not` blocks** — applies conditionally based on a matcher

```ferron
example.com {
    # Host-level: blocks apply to everything
    block "203.0.113.0/24"

    location /admin {
        # Location-level: stricter status
        status 403 {
            body "Admin area restricted"
        }
    }

    if sensitive_matcher {
        # Conditional: abort certain requests
        abort true
    }
}
```

## See Also

- [HTTP Control Directives](./http-control.md) (`location`, `if`, `if_not`)
- [HTTP Host Directives](./http-host.md)
- [Conditionals And Variables](./conditionals.md)
