---
title: "Configuration: HTTP response control"
description: "Custom status codes, connection aborting, and IP-based access control directives."
---

This page documents directives for returning custom status codes, aborting connections, and IP-based access control.

## Directives

### Custom status codes

- `status <code: integer>` (_http_response_ module)
  - This directive specifies an HTTP status code to return. In block form, supports nested `url`, `regex`, `body`, and `location` directives. Default: none

#### Block form options

| Nested directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `url` | `<string>` | Only apply this status to requests matching this exact path. | all requests |
| `regex` | `<string>` | Only apply this status to requests matching this regular expression. | all requests |
| `body` | `<string>` | Response body to include. | empty body |
| `location` | `<string>` | Redirect destination for 3xx responses. | no redirect |

**Configuration example:**

```ferron
example.com {
    status 503 {
        body "Service temporarily unavailable"
    }

    status 404 {
        url "/old-endpoint"
        body "This endpoint has been removed"
    }

    status 301 {
        url "/legacy"
        location "/new"
    }

    status 410 {
        regex "^/api/v1/.*"
        body "API v1 has been deprecated"
    }
}
```

Multiple `status` directives can be defined. They are evaluated in order — the first matching rule wins.

### Connection abort

- `abort [bool: boolean]` (_http_response_ module)
  - This directive specifies whether the connection is immediately closed without sending any response. When `true` or when omitted, the connection is terminated immediately. Default: `abort false`

**Configuration example:**

```ferron
example.com {
    abort
}
```

When `abort` is set, the connection is terminated immediately with no HTTP response sent. This is useful for silently dropping requests from unwanted clients or for denial-of-service mitigation.

### IP access control

- `block <ip-or-cidr: string>...` (_http_response_ module)
  - This directive specifies one or more IP addresses or CIDR ranges to block. Blocked IPs receive a **403 Forbidden** response. Default: none
- `allow <ip-or-cidr: string>...` (_http_response_ module)
  - This directive specifies one or more IP addresses or CIDR ranges to allow. When configured, **only** the listed IPs/CIDRs are permitted. All other IPs receive a **403 Forbidden** response. Default: none (all allowed)

**Configuration example:**

```ferron
example.com {
    block "192.168.1.100" "10.0.0.50"
    block "203.0.113.0/24"

    allow "10.0.0.0/8" "172.16.0.0/12" "192.168.0.0/16"
}
```

#### Combined block and allow

When both `block` and `allow` are configured:

1. If the IP matches an `allow` entry **and** a `block` entry → **blocked** (block takes precedence)
2. If the IP matches only an `allow` entry → **allowed**
3. If the IP matches only a `block` entry → **blocked**
4. If the IP matches neither → **allowed** (unless the allow list is non-empty, in which case non-listed IPs are denied)

```ferron
example.com {
    allow "192.168.1.0/24"
    block "192.168.1.100"
}
```

In this example: `192.168.1.50` → allowed, `192.168.1.100` → blocked, `10.0.0.1` → denied.

## Scoping

All directives (`status`, `abort`, `block`, `allow`) can be placed at different configuration levels:

- **Host level** — applies to all requests for that host
- **`location` block** — applies only to requests matching that path prefix
- **`if` / `if_not` blocks** — applies conditionally based on a matcher

## Notes and troubleshooting

- For `location`, `if`, and `if_not` syntax, see [Routing and URL processing](/docs/v3/routing-url-processing).
- For conditionals and matchers, see [Conditionals and variables](/docs/v3/conditionals).
- For HTTP host directives, see [HTTP host directives](/docs/v3/http-host).
