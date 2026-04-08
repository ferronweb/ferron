---
title: "Configuration: HTTP response control"
description: "Custom status codes, connection aborting, IP-based access control, and 103 Early Hints."
---

This page documents directives for returning custom status codes, aborting connections, IP-based access control, and 103 Early Hints responses.

## Directives

### Custom status codes

- `status <code: integer>` (`ferron-http-response`)
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

- `abort [bool: boolean]` (`ferron-http-response`)
  - This directive specifies whether the connection is immediately closed without sending any response. When `true` or when omitted, the connection is terminated immediately. Default: `abort false`

**Configuration example:**

```ferron
example.com {
    abort
}
```

When `abort` is set, the connection is terminated immediately with no HTTP response sent. This is useful for silently dropping requests from unwanted clients or for denial-of-service mitigation.

### IP access control

- `block <ip-or-cidr: string>...` (`ferron-http-response`)
  - This directive specifies one or more IP addresses or CIDR ranges to block. Blocked IPs receive a **403 Forbidden** response. Default: none
- `allow <ip-or-cidr: string>...` (`ferron-http-response`)
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

### 103 Early Hints

- `early_hints` (`ferron-http-response`)
  - This directive specifies a 103 Early Hints response to send before the final response is ready. The 103 response includes `Link` headers that allow the browser to begin preloading resources (stylesheets, scripts, fonts, etc.) while the server is still preparing the final response. Default: none

#### Subdirectives

| Subdirective | Arguments | Description | Default |
| --- | --- | --- | --- |
| `link` | `<string>` | A `Link` header value to include in the 103 response. Multiple `link` entries produce multiple `Link` headers. | none |

**Configuration example:**

```ferron
example.com {
    early_hints {
        link "</assets/main.css>; rel=preload; as=style"
        link "</assets/main.js>; rel=preload; as=script"
        link "</fonts/inter.woff2>; rel=preload; as=font; crossorigin"
    }
}
```

Multiple `link` entries can be defined within a single `early_hints` block. Multiple `early_hints` blocks can also be defined at different scoping levels (host, `location`, `if` / `if_not`).

#### HTTP/1.1 support

By default, 103 Early Hints is supported natively on HTTP/2 and HTTP/3 connections. For HTTP/1.1, you must enable support via the [`h1_enable_early_hints`](/docs/v3/configuration/http-host) directive in your `http` block:

```ferron
http {
    h1_enable_early_hints true
}
```

Without this option, 103 Early Hints is silently skipped on HTTP/1.1 connections (a warning is logged).

#### Scoping

The `early_hints` directive can be placed at different configuration levels:

- **Host level** — applies to all requests for that host
- **`location` block** — applies only to requests matching that path prefix
- **`if` / `if_not` blocks** — applies conditionally based on a matcher

## Scoping

All directives (`status`, `abort`, `block`, `allow`, `early_hints`) can be placed at different configuration levels:

- **Host level** — applies to all requests for that host
- **`location` block** — applies only to requests matching that path prefix
- **`if` / `if_not` blocks** — applies conditionally based on a matcher

## Observability

### Metrics

- `ferron.response.aborted` (Counter) — connections aborted via the `abort` directive.
- `ferron.response.ip_blocked` (Counter) — connections blocked via `block`/`allow` directives. This metric does **not** include raw IP addresses.
- `ferron.response.status_rule_matched` (Counter) — custom status codes returned via `status` directives. Includes `http.response.status_code` and `ferron.rule_id` attributes.

## Notes and troubleshooting

- For `location`, `if`, and `if_not` syntax, see [Routing and URL processing](/docs/v3/configuration/routing-url-processing).
- For conditionals and matchers, see [Conditionals and variables](/docs/v3/configuration/conditionals).
- For HTTP host directives including `h1_enable_early_hints`, see [HTTP host directives](/docs/v3/configuration/http-host).
- 103 Early Hints is only effective on HTTP/2+ connections by default. Most browsers restrict 103 to HTTP/2 or later for security reasons. See [RFC 8297 Section 3](https://www.rfc-editor.org/rfc/rfc8297#section-3).
- If 103 Early Hints is not being sent on HTTP/1.1, ensure `h1_enable_early_hints true` is set in your `http` block.
- If `send_early_hints` fails (e.g., connection already closing), a warning is logged and the request continues normally.
