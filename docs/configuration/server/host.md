---
title: "Configuration: HTTP host directives"
description: "Per-host directives for protocol behavior, TLS, HTTPS redirects, client IP resolution, and server metadata."
---

This page documents directives consumed from HTTP host blocks such as:

```ferron
example.com {
}

http example.com:8080 {
}
```

> [!note]
>
> - Ferron scopes these directives to individual hosts, not globally.
> - The HTTP server engine (`http-server` module) handles connection management, request routing, and TLS termination.
> - The engine supports HTTP/1, HTTP/2, and HTTP/3.

> [!info]
> For ACME configuration details, see [ACME automatic TLS](/docs/v3/configuration/security/acme). For crypto and mTLS settings, see [Security and TLS](/docs/v3/configuration/security/tls).

## Directives

### Automatic TLS

When you give a hostname (for example, `example.com`) without an explicit port, Ferron starts **two listeners**:

- One on `default_http_port` (default: 80) serves plain HTTP with no TLS
- One on `default_https_port` (default: 443) serves HTTPS with automatic ACME TLS

On the HTTPS listener, Ferron **automatically enables TLS via the ACME provider** (Let's Encrypt by default). An explicit `tls` directive overrides this behavior. The ACME provider gets and renews certificates at startup.

Hostnames that have **special automatic TLS behavior**:

- `localhost`, `127.0.0.1`, `::1`: these loopback addresses use the **local TLS provider** instead of ACME. This gives HTTPS for development without public certificates.

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
        provider manual
        cert "/etc/ssl/cert.pem"
        key "/etc/ssl/key.pem"
    }
    root /var/www/html
}
```

Or with an alias:

```ferron
example.com {
    tls /etc/ssl/cert.pem /etc/ssl/key.pem
    root /var/www/html
}
```

When you specify an **explicit port** (for example, `example.com:8080`), Ferron starts only a single listener on that port. Ferron does not apply automatic ACME TLS (you must configure TLS explicitly).

> [!info]
> See [ACME automatic TLS](/docs/v3/configuration/security/acme) for full ACME configuration details.

### HTTPS redirect

- `https_redirect <bool>`
  - This directive enables or disables automatic HTTP-to-HTTPS redirects. The redirect uses **308 Permanent Redirect**, which preserves the HTTP method and request body. Default: `https_redirect true` (when you enable TLS)

**Configuration example:**

```ferron
example.com {
    https_redirect false
}
```

> [!note]
>
> - Ferron never redirects `localhost` hostnames (no HTTPS listener exists for them).
> - When you specify an explicit port (for example, `example.com:8080`), no redirect happens since no separate HTTPS listener exists.
> - The target port is `default_https_port` (default: `443`). When the port is `443`, Ferron omits it from the URL.

### Client IP from forwarded headers

- `client_ip_from_header <header: string> { ... }` (global scope)
  - This directive specifies the header to read the client IP from. Supported values: `x-forwarded-for`, `forwarded`. Default: disabled

| Nested directive | Arguments                 | Description                                                                            | Default |
| ---------------- | ------------------------- | -------------------------------------------------------------------------------------- | ------- |
| `trusted_proxy`  | `<ip-or-cidr: string>...` | Reverse-proxy IPs or CIDR ranges that you trust to supply forwarded client IP headers. | none    |

**Configuration example:**

```ferron
{
    client_ip_from_header x-forwarded-for {
        # Only trust forwarded headers from these proxy networks.
        trusted_proxy "10.0.0.0/8"
        trusted_proxy "192.168.0.0/16"
    }
}

example.com {
    root /var/www/html
}
```

#### `x-forwarded-for`

Reads the `X-Forwarded-For` header and extracts the **first (leftmost)** IP address from the comma-separated chain.

#### `forwarded` (RFC 7239)

Reads the `Forwarded` header and extracts the first `for=` token. Ferron supports both quoted and unquoted values. Ferron also supports IPv6 addresses.

> [!warning]
> Ferron only trusts forwarded client IP headers when the connecting peer matches at least one `trusted_proxy` entry. If the `trusted_proxy` list is empty, Ferron ignores the header. Keep this list limited to the reverse proxies or load balancers that you control.

### HTTP protocol settings

- `protocols <protocols: string>...`
  - This directive specifies the enabled HTTP protocols. Supported values are `h1` (HTTP/1.1), `h2` (HTTP/2), and `h3` (HTTP/3). Default: `protocols h1 h2 h3`

- `options_allowed_methods <methods: string>`
  - This directive specifies the HTTP methods advertised in the `Allow` header for `OPTIONS *` requests (per RFC 2616 Section 9.2). Ferron returns the methods as a comma-separated list. This only applies to server-wide `OPTIONS *` requests, not resource-specific `OPTIONS /path` requests. Default: `options_allowed_methods "GET, HEAD, POST, OPTIONS"`

- `timeout <duration>`
  - This directive specifies the pipeline execution timeout. Accepts a duration string (for example, `30m`, `1h`, `90s`), a number in milliseconds, or `false` to disable. Default: `timeout "5m"` (5 minutes)

- `h1_enable_early_hints <bool>`
  - This directive enables or disables HTTP/1.1 early hints support. Default: `h1_enable_early_hints false`

- `h2_initial_window_size <size: integer>`
  - This directive specifies the HTTP/2 initial flow-control window size. Default: unset

- `h2_max_frame_size <size: integer>`
  - This directive specifies the HTTP/2 maximum frame size. Default: unset

- `h2_max_concurrent_streams <count: integer>`
  - This directive specifies the HTTP/2 maximum concurrent streams. Default: unset

- `h2_max_header_list_size <size: integer>`
  - This directive specifies the HTTP/2 maximum header list size. It is not recommended to set the value high, as this leads to HPACK memory exhaustion vulnerabilities. Default: unset

- `h2_enable_connect_protocol [bool: boolean]`
  - This directive enables or disables the HTTP/2 extended CONNECT protocol setting. Default: `h2_enable_connect_protocol false`

- `h3_qpack_max_table_capacity <size: integer>`
  - This directive specifies the maximum QPACK table capacity for HTTP/3. Default: unset

- `h3_qpack_blocked_streams <count: integer>`
  - This directive specifies the number of blocked streams for HTTP/3. Default: unset

- `h3_max_field_section_size <size: integer>`
  - This directive specifies the maximum field section size for HTTP/3. It is not recommended to set the value high, as this leads to QPACK memory exhaustion vulnerabilities. Default: unset

- `h3_enable_connect_protocol [bool: boolean]`
  - This directive enables or disables the HTTP/3 extended CONNECT protocol setting. Default: `h3_enable_connect_protocol false`

- `url_sanitize [bool: boolean]`
  - This directive enables or disables URL path sanitization. When enabled (the default), Ferron removes or normalizes dangerous sequences such as path traversal attempts (`../`, `..\\`), null bytes, and invalid percent-encodings. This directive applies only to global scope. Default: `url_sanitize true`

- `url_reject_backslash [bool: boolean]`
  - This directive controls whether Ferron rejects URLs containing backslashes. When enabled (the default), Ferron responds with 400 Bad Request for requests containing literal `\` or percent-encoded backslashes (`%5C`) in the path. This prevents path interpretation issues on Windows backends where systems may treat backslashes as path separators. This directive applies only to global scope. Default: `url_reject_backslash true`

**Configuration example:**

```ferron
example.com {
    http {
        protocols h1 h2 h3
        options_allowed_methods "GET, HEAD, POST, PUT, DELETE, OPTIONS"
        timeout "30m"
        h1_enable_early_hints false
    }
}
```

> [!note]
>
> - `protocols` must leave at least one supported protocol enabled.
> - When you enable HTTP/3, Ferron starts an additional QUIC listener on the same port for HTTP/3 traffic.

> [!note]
>
> - The default `options_allowed_methods` value (`GET, HEAD, POST, OPTIONS`) intentionally excludes methods like `PUT`, `DELETE`, `PATCH`, `CONNECT`, and `TRACE`. This reduces the attack surface reported by security scanners. You can customize this list based on the requirements of your server.
> - When you enable HTTP/3, the server automatically adds an `Alt-Svc` header to responses to advertise HTTP/3 support to clients.

> [!note] Notes for "url_sanitize"
>
> - Ferron applies URL sanitization early in request processing, before configuration resolution.
> - This directive is only read from the **global** configuration block. Per-host settings are not currently supported.

> [!note]
>
> - Disabling URL sanitization may improve RFC 3986 compliance for URLs that use valid but unusual encodings.
> - Even when disabled, the file resolution stage still canonicalizes paths and rejects requests that escape the configured webroot.

> [!warning]
> When you disable `url_sanitize`, Ferron does not protect backend services from path traversal attacks if you implement reverse proxying. Use with caution.

> [!note] Notes for "url_reject_backslash"
>
> - Ferron applies backslash rejection early in request processing, before configuration resolution and URL sanitization.
> - This directive is only read from the **global** configuration block. Per-host settings are not currently supported.
> - Ferron rejects both literal backslashes (`\`) and percent-encoded backslashes (`%5C`/`%5c`).

> [!warning]
> Disabling the `url_reject_backslash` directive may be necessary if you have Windows backends that legitimately use backslashes in URLs. However, this can expose backends to path interpretation vulnerabilities.

### TLS

- `provider <name: string>` (`tls-manual`, `tls-acme`)
  - This directive specifies the TLS provider name. Required when you enable TLS through the block form. Supported providers: `manual` (`tls-manual`), `acme` (`tls-acme`). Default: none

> [!info]
>
> - For crypto settings (`cipher_suite`, `ecdh_curve`, `min_version`, `max_version`, `client_auth`, `client_auth_ca`), see [Security and TLS](/docs/v3/configuration/security/tls).
> - For OCSP stapling configuration, see [OCSP stapling](/docs/v3/configuration/security/ocsp).
> - For session ticket keys, see [TLS session ticket keys](/docs/v3/configuration/security/session-tickets).

### `admin_email`

- `admin_email <email: string>`
  - This directive specifies the email address of the server administrator. Ferron uses it in built-in error responses. Ferron supports interpolation. Default: none

## Metrics

The HTTP server emits the following OpenTelemetry-style metrics via the observability event system:

| Metric                             | Type          | Attributes                                                                                                                          | Description                             |
| ---------------------------------- | ------------- | ----------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------- |
| `http.server.active_requests`      | UpDownCounter | `http.request.method`, `url.scheme`, `network.protocol.name`, `network.protocol.version`                                            | Number of active HTTP requests          |
| `http.server.request.duration`     | Histogram     | `http.request.method`, `url.scheme`, `network.protocol.name`, `network.protocol.version`, `http.response.status_code`, `error.type` | How long HTTP requests take, in seconds |
| `ferron.http.server.request_count` | Counter       | `http.request.method`, `url.scheme`, `network.protocol.name`, `network.protocol.version`, `http.response.status_code`, `error.type` | Total number of HTTP requests completed |

All metrics include attributes for `http.request.method`, `url.scheme`, `network.protocol.name`, and `network.protocol.version`. The `http.server.request.duration` and `ferron.http.server.request_count` metrics also include `http.response.status_code` and `error.type` (for 4xx/5xx responses).

## Best practices

`ferron doctor` reports the following best-practice checks for directives on this page.

### Client IP resolution

- **`trusted_proxy 0.0.0.0/0` or `::/0`**: Trusting every source address for forwarded client IP headers allows spoofing. Restrict `trusted_proxy` to specific reverse proxy addresses.
- **`client_ip_from_header` without `trusted_proxy`**: If you configure `client_ip_from_header` without trusted proxy ranges, Ferron either ignores or does not trust forwarded headers. Add explicit `trusted_proxy` entries for your reverse proxies.

### URL processing

- **`url_sanitize false`**: Disabling path traversal normalization can expose backend path interpretation issues. Keep sanitization enabled unless a specific backend requires raw paths.
- **`url_reject_backslash false`**: Permitting backslashes in request paths can cause backend routing confusion. Keep rejection enabled unless required.

### Timeouts

- **`timeout false`**: Disabling request pipeline timeouts lets slow requests exhaust server resources. Set a bounded timeout value.

### HTTP methods

- **`options_allowed_methods` with TRACE or CONNECT**: Advertising TRACE or CONNECT in OPTIONS responses may expose unintended attack surface. Remove these methods unless intentionally supported.

### TLS deployment

- **HTTP-only host without TLS**: When a non-localhost host block has no `tls` configuration, `ferron doctor` emits a reminder. The reminder states that an upstream proxy or load balancer should terminate TLS. This is informational, not prescriptive. Legitimate HTTP-only setups include deployments behind CDNs, load balancers, or Kubernetes ingress controllers that handle TLS termination.

## Observability

### Trace spans

The HTTP server sets the following attributes on per-stage spans:

| Span name                            | Attributes                                             | Description                  |
| ------------------------------------ | ------------------------------------------------------ | ---------------------------- |
| `ferron.stage.client_ip_from_header` | `ferron.client_ip.source`, `ferron.client_ip.original` | Client IP resolution stage   |
| `ferron.stage.https_redirect`        | `ferron.redirect.target`                               | HTTP to HTTPS redirect stage |
