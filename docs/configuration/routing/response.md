---
title: "Configuration: HTTP response control"
description: "Custom status codes, connection aborting, IP-based access control, and 103 Early Hints."
---

`status`, `abort`, `block`, `allow`, and `early_hints` control what Ferron returns and to which clients.

> [!info]
> For `location`, `if`, and `if_not` syntax, see [Routing and URL processing](/docs/configuration/routing/url-processing). For conditionals and matchers, see [Conditionals and variables](/docs/configuration/fundamentals/conditionals). For `h1_enable_early_hints`, see [HTTP host directives](/docs/configuration/server/host).

## Custom status codes

`status <code: integer>` returns an HTTP status code. The block form supports nested `url`, `regex`, `body`, `location`, and `name` directives. Default: none.

| Nested directive | Arguments  | Description                                                                                                                | Default                                    |
| ---------------- | ---------- | -------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------ |
| `url`            | `<string>` | Apply this status only to requests that match this exact path.                                                             | all requests                               |
| `regex`          | `<string>` | Apply this status only to requests that match this regular expression.                                                     | all requests                               |
| `body`           | `<string>` | Response body to include.                                                                                                  | empty body                                 |
| `location`       | `<string>` | Redirect destination for 3xx responses.                                                                                    | no redirect                                |
| `name`           | `<string>` | Rule identifier for the `ferron.rule_id` metric label. Must be a plain string without interpolation.                       | derived from status code and rule position |

```ferron
example.com {
    status 503 {
        body "Service temporarily unavailable"
    }

    status 404 {
        url /old-endpoint
        body "This endpoint has been removed"
    }

    status 301 {
        url /legacy
        location /new
    }

    status 410 {
        regex r"^/api/v1/.*"
        body "API v1 has been deprecated"
    }

    status 410 {
        name "legacy-docs"
        url /docs/old
        body "Docs have moved"
    }
}
```

You can define multiple `status` directives. Ferron evaluates them in order, and the first matching rule wins.

## Abort a connection

`abort [bool: boolean]` closes the connection immediately without sending a response. When the directive is true or has no argument, Ferron terminates the connection. Default: `false`.

```ferron
example.com {
    abort
}
```

Use `abort` to drop requests from unwanted clients or to mitigate denial-of-service traffic.

## IP access control

| Directive | Arguments                 | Description                                                                                                       | Default            |
| --------- | ------------------------- | ----------------------------------------------------------------------------------------------------------------- | ------------------ |
| `block`   | `<ip-or-cidr: string>...` | IP addresses or CIDR ranges to block. Blocked IPs receive 403 Forbidden.                                          | none               |
| `allow`   | `<ip-or-cidr: string>...` | IP addresses or CIDR ranges to allow. When set, Ferron permits only these IPs and CIDRs. All others receive 403.  | none (all allowed) |

```ferron
example.com {
    block "192.168.1.100" "10.0.0.50"
    block "203.0.113.0/24"

    allow "10.0.0.0/8" "172.16.0.0/12" "192.168.0.0/16"
}
```

### `block` and `allow` together

When both `allow` and `block` are set:

- Matches an `allow` entry and a `block` entry: denied. Block wins.
- Matches only an `allow` entry: allowed.
- Matches only a `block` entry: denied.
- Matches neither: allowed, unless the allow list is not empty. In that case Ferron denies IPs that are not listed.

```ferron
example.com {
    allow "192.168.1.0/24"
    block "192.168.1.100"
}
```

Here, `192.168.1.50` is allowed. `192.168.1.100` and `10.0.0.1` are denied.

## 103 Early Hints

`early_hints` sends a 103 Early Hints response before the final response is ready. The 103 response includes `Link` headers that tell the browser to preload resources such as stylesheets, scripts, and fonts. Default: none.

| Subdirective | Arguments  | Description                                                                                                    | Default |
| ------------ | ---------- | -------------------------------------------------------------------------------------------------------------- | ------- |
| `link`       | `<string>` | A `Link` header value for the 103 response. Multiple `link` entries produce multiple `Link` headers.            | none    |

```ferron
example.com {
    early_hints {
        link "</assets/main.css>; rel=preload; as=style"
        link "</assets/main.js>; rel=preload; as=script"
        link "</fonts/inter.woff2>; rel=preload; as=font; crossorigin"
    }
}
```

You can define multiple `link` entries in one `early_hints` block. You can also define multiple `early_hints` blocks at different scopes: host, `location`, and `if` or `if_not`.

### HTTP/1.1 support

HTTP/2 and HTTP/3 connections support 103 Early Hints natively. For HTTP/1.1, enable support with [`h1_enable_early_hints`](/docs/configuration/server/host) in your `http` block:

```ferron
{
    http {
        h1_enable_early_hints
    }
}
```

Without this option, Ferron skips 103 Early Hints on HTTP/1.1 connections and logs a warning. If sending early hints fails, Ferron also logs a warning and continues with the request normally.

## Observability

### Metrics

| Metric                                | Type    | Attributes                                    | Description                                                                                                                                                                                                                             |
| ------------------------------------- | ------- | --------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `ferron.response.aborted`             | Counter | None                                          | Connections aborted through `abort`                                                                                                                                                                                                     |
| `ferron.response.ip_blocked`          | Counter | None                                          | Connections blocked through `block` or `allow`. Does not include raw IP addresses                                                                                                                                                        |
| `ferron.response.status_rule_matched` | Counter | `http.response.status_code`, `ferron.rule_id` | Custom status codes returned through `status`. `ferron.rule_id` is the rule `name` when set. Otherwise it is `status-<code>-rule-<n>`, where `<n>` is the 1-based position among the evaluated `status` directives.                      |

### Access log fields

The response control module adds one field to the HTTP access log line:

| Field                    | Type   | Description                                     |
| ------------------------ | ------ | ----------------------------------------------- |
| `ferron.response.action` | string | Response action: `abort`, `block`, or `status`. |

### Trace spans

The response control stage sets these attributes on its `ferron.stage.http_response` span:

| Attribute                   | Type   | Description                                                              |
| --------------------------- | ------ | ------------------------------------------------------------------------ |
| `ferron.response.action`    | string | Action taken: `abort`, `ip_block`, or `status_rule`.                     |
| `http.response.status_code` | int    | HTTP status code returned to the client.                                 |
| `error.type`                | string | Set when the action results in an error, for trace UI highlighting.      |
