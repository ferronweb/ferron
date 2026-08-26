---
title: "Configuration: HTTP response control"
description: "Custom status codes, connection aborting, IP-based access control, and 103 Early Hints."
---

This page documents directives for returning custom status codes, aborting connections, IP-based access control, and 103 Early Hints responses.

> [!info]
>
> - For `location`, `if`, and `if_not` syntax, see [Routing and URL processing](/docs/v3/configuration/routing/url-processing).
> - For conditionals and matchers, see [Conditionals and variables](/docs/v3/configuration/fundamentals/conditionals).
> - For HTTP host directives including `h1_enable_early_hints`, see [HTTP host directives](/docs/v3/configuration/server/host).

## Directives

### Custom status codes

- `status <code: integer>` (`http-response`)
  - This directive specifies an HTTP status code to return. In block form, supports nested `url`, `regex`, `body`, and `location` directives. Default: none

#### Block form options

| Nested directive | Arguments  | Description                                                          | Default      |
| ---------------- | ---------- | -------------------------------------------------------------------- | ------------ |
| `url`            | `<string>` | Only apply this status to requests matching this exact path.         | all requests |
| `regex`          | `<string>` | Only apply this status to requests matching this regular expression. | all requests |
| `body`           | `<string>` | Response body to include.                                            | empty body   |
| `location`       | `<string>` | Redirect destination for 3xx responses.                              | no redirect  |

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

You can define multiple `status` directives. Ferron evaluates them in order (the first matching rule wins).

### Connection abort

- `abort [bool: boolean]` (`http-response`)
  - This directive specifies whether Ferron closes the connection immediately without sending any response. When `true` or when omitted, Ferron terminates the connection immediately. Default: `abort false`

**Configuration example:**

```ferron
example.com {
    abort
}
```

With `abort` set, Ferron terminates the connection immediately and sends no HTTP response. This is useful for silently dropping requests from unwanted clients or for denial-of-service mitigation.

### IP access control

- `block <ip-or-cidr: string>...` (`http-response`)
  - This directive specifies one or more IP addresses or CIDR ranges to block. Blocked IPs receive a 403 Forbidden response. Default: none
- `allow <ip-or-cidr: string>...` (`http-response`)
  - This directive specifies one or more IP addresses or CIDR ranges to allow. When configured, Ferron permits only the listed IPs/CIDRs. All other IPs receive a 403 Forbidden response. Default: none (all allowed)

**Configuration example:**

```ferron
example.com {
    block "192.168.1.100" "10.0.0.50"
    block "203.0.113.0/24"

    allow "10.0.0.0/8" "172.16.0.0/12" "192.168.0.0/16"
}
```

#### Combined block and allow

When you set both `block` and `allow`:

1. If the IP matches an `allow` entry and a `block` entry → blocked (block takes precedence)
2. If the IP matches only an `allow` entry → allowed
3. If the IP matches only a `block` entry → blocked
4. If the IP matches neither → allowed (unless the allow list is not empty, in which case Ferron denies non-listed IPs)

```ferron
example.com {
    allow "192.168.1.0/24"
    block "192.168.1.100"
}
```

In this example: `192.168.1.50` → allowed, `192.168.1.100` → blocked, `10.0.0.1` → denied.

### 103 Early Hints

- `early_hints` (`http-response`)
  - This directive specifies a 103 Early Hints response to send before the final response is ready. The 103 response includes `Link` headers that let the browser preload resources (stylesheets, scripts, fonts, and so on). This happens while the server is still preparing the final response. Default: none

#### Subdirectives

| Subdirective | Arguments  | Description                                                                                                    | Default |
| ------------ | ---------- | -------------------------------------------------------------------------------------------------------------- | ------- |
| `link`       | `<string>` | A `Link` header value to include in the 103 response. Multiple `link` entries produce multiple `Link` headers. | none    |

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

You can define multiple `link` entries within a single `early_hints` block. You can also define multiple `early_hints` blocks at different scoping levels (host, `location`, `if` / `if_not`).

#### HTTP/1.1 support

By default, HTTP/2 and HTTP/3 connections support 103 Early Hints natively. For HTTP/1.1, you must enable support via the [`h1_enable_early_hints`](/docs/v3/configuration/server/host) directive in your `http` block:

```ferron
{
    http {
        h1_enable_early_hints true
    }
}
```

Without this option, Ferron silently skips 103 Early Hints on HTTP/1.1 connections and logs a warning.

> [!note]
> 103 Early Hints is only effective on HTTP/2+ connections by default. For HTTP/1.1, enable `h1_enable_early_hints true` in your `http` block. If `send_early_hints` fails, Ferron logs a warning and the request continues normally.

## Observability

### Metrics

| Metric                                | Type    | Attributes                                    | Description                                                                           |
| ------------------------------------- | ------- | --------------------------------------------- | ------------------------------------------------------------------------------------- |
| `ferron.response.aborted`             | Counter | None                                          | Connections aborted via the `abort` directive                                         |
| `ferron.response.ip_blocked`          | Counter | None                                          | Connections blocked via `block`/`allow` directives. Does not include raw IP addresses |
| `ferron.response.status_rule_matched` | Counter | `http.response.status_code`, `ferron.rule_id` | Custom status codes returned via `status` directives                                  |

### Access log fields

The response control module contributes the following field to the HTTP access log line:

| Field                    | Type   | Description                                     |
| ------------------------ | ------ | ----------------------------------------------- |
| `ferron.response.action` | string | Response action: `abort`, `block`, or `status`. |

### Trace spans

The response control stage sets the following attributes on its `ferron.stage.http_response` span:

| Attribute                   | Type   | Description                                                              |
| --------------------------- | ------ | ------------------------------------------------------------------------ |
| `ferron.response.action`    | string | Action taken: `abort`, `ip_block`, or `status_rule`.                     |
| `http.response.status_code` | int    | HTTP status code returned to the client.                                 |
| `error.type`                | string | Set when the action results in an error, enabling trace UI highlighting. |
