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

## Directives

### Automatic TLS

When a hostname is specified (e.g. `example.com`) and no explicit port is given, Ferron starts **two listeners**:

- One on `default_http_port` (default: 80) — serves plain HTTP with no TLS
- One on `default_https_port` (default: 443) — serves HTTPS with automatic ACME TLS

On the HTTPS listener, if no explicit `tls` directive is present, Ferron **automatically enables TLS via the ACME provider** (Let's Encrypt by default). Certificates are obtained and renewed automatically at startup.

Hostnames that are **exempt** from the HTTPS listener and automatic TLS:

- `localhost`
- `127.0.0.1`
- `::1`

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

When an **explicit port** is specified (e.g. `example.com:8080`), only a single listener is started on that port, and no automatic ACME TLS is applied — you must configure TLS explicitly.

See [ACME automatic TLS](/docs/v3/configuration/tls-acme) for full ACME configuration details.

### HTTPS redirect

- `https_redirect <bool>`
  - This directive specifies whether automatic HTTP-to-HTTPS redirects are enabled. The redirect uses **308 Permanent Redirect**, which preserves the HTTP method and request body. Default: `https_redirect true` (when TLS is enabled)

**Configuration example:**

```ferron
example.com {
    https_redirect false
}
```

Notes:

- The redirect respects the `X-Forwarded-Proto: https` header to avoid redirect loops when behind a TLS-terminating reverse proxy.
- `localhost` hostnames never get redirected — there is no HTTPS listener for them.
- When an explicit port is specified (e.g. `example.com:8080`), no redirect is performed since no separate HTTPS listener exists.
- The target port is `default_https_port` (default: `443`). When the port is `443`, it is omitted from the URL.

### Client IP from forwarded headers

- `client_ip_from_header <header: string>` (global scope)
  - This directive specifies the header to read the client IP from. Supported values: `x-forwarded-for`, `forwarded`. Default: disabled

**Configuration example:**

```ferron
{
    client_ip_from_header x-forwarded-for
}

example.com {
    root /var/www/html
}
```

#### `x-forwarded-for`

Reads the `X-Forwarded-For` header and extracts the **first (leftmost)** IP address from the comma-separated chain.

#### `forwarded` (RFC 7239)

Reads the `Forwarded` header and extracts the first `for=` token. Both quoted and unquoted values are supported. IPv6 addresses are also supported.

**Trust boundary warning:** This directive blindly trusts the header value. If the server is directly exposed to the internet (not behind a trusted proxy), a malicious client could spoof their IP address. Only enable this when running behind a trusted reverse proxy or load balancer.

### HTTP protocol settings

- `protocols <protocols: string>...`
  - This directive specifies the enabled HTTP protocols. Currently supported values are `h1` and `h2`. Default: `protocols h1 h2`
- `options_allowed_methods <methods: string>`
  - This directive specifies the HTTP methods advertised in the `Allow` header for `OPTIONS *` requests (per RFC 2616 Section 9.2). The methods are returned as a comma-separated list. This only applies to server-wide `OPTIONS *` requests, not resource-specific `OPTIONS /path` requests. Default: `options_allowed_methods "GET, HEAD, POST, OPTIONS"`
- `timeout <duration>`
  - This directive specifies the pipeline execution timeout. Accepts a duration string (e.g. `30m`, `1h`, `90s`), a number in milliseconds, or `false` to disable. Default: `timeout 300000` (5 minutes)
- `h1_enable_early_hints <bool>`
  - This directive specifies whether HTTP/1.1 early hints support is enabled. Default: `h1_enable_early_hints false`
- `h2_initial_window_size <size: integer>`
  - This directive specifies the HTTP/2 initial flow-control window size. Default: unset
- `h2_max_frame_size <size: integer>`
  - This directive specifies the HTTP/2 maximum frame size. Default: unset
- `h2_max_concurrent_streams <count: integer>`
  - This directive specifies the HTTP/2 maximum concurrent streams. Default: unset
- `h2_max_header_list_size <size: integer>`
  - This directive specifies the HTTP/2 maximum header list size. Default: unset
- `h2_enable_connect_protocol <bool>`
  - This directive specifies whether the HTTP/2 extended CONNECT protocol setting is enabled. Default: `h2_enable_connect_protocol false`
- `url_sanitize [bool: boolean]`
  - This directive specifies whether URL path sanitization is enabled. When enabled (the default), dangerous sequences such as path traversal attempts (`../`, `..\\`), null bytes, and invalid percent-encodings are removed or normalized. This directive is applicable only for global scope. Default: `url_sanitize true`
- `url_reject_backslash [bool: boolean]`
  - This directive specifies whether URLs containing backslashes are rejected. When enabled (the default), requests with literal `\` or percent-encoded backslashes (`%5C`) in the path are rejected with a 400 Bad Request response. This prevents path interpretation issues on Windows backends where backslashes may be treated as path separators. This directive is applicable only for global scope. Default: `url_reject_backslash true`

**Configuration example:**

```ferron
example.com {
    http {
        protocols h1 h2
        options_allowed_methods "GET, HEAD, POST, PUT, DELETE, OPTIONS"
        timeout 30m
        h1_enable_early_hints false
    }
}
```

Notes:

- `protocols` must leave at least one supported protocol enabled.
- `h3` is currently rejected.
- The default `options_allowed_methods` value (`GET, HEAD, POST, OPTIONS`) intentionally excludes methods like `PUT`, `DELETE`, `PATCH`, `CONNECT`, and `TRACE` to reduce the attack surface reported by security scanners. You can customize this list based on your server's requirements.

Notes for `url_sanitize`:

- URL sanitization is applied early in request processing, before configuration resolution.
- This directive is only read from the **global** configuration block. Per-host settings are not currently supported.
- Disabling URL sanitization may improve RFC 3986 compliance for URLs that use valid but unusual encodings.
- **Warning:** When disabled, Ferron will not protect backend services from path traversal attacks if reverse proxying is implemented. Use with caution.
- Even when disabled, the file resolution stage still canonicalizes paths and rejects requests that escape the configured webroot.

Notes for `url_reject_backslash`:

- Backslash rejection is applied early in request processing, before configuration resolution and URL sanitization.
- This directive is only read from the **global** configuration block. Per-host settings are not currently supported.
- Both literal backslashes (`\`) and percent-encoded backslashes (`%5C`/`%5c`) are rejected.
- Disabling this directive may be necessary if you have Windows backends that legitimately use backslashes in URLs, but **warning**: this can expose backends to path interpretation vulnerabilities.

### TLS

- `provider <name: string>` (`tls-manual`, `tls-acme`)
  - This directive specifies the TLS provider name. Required when TLS is enabled through the block form. Supported providers: `manual` (`tls-manual`), `acme` (`tls-acme`). Default: none

For crypto settings (`cipher_suite`, `ecdh_curve`, `min_version`, `max_version`, `client_auth`, `client_auth_ca`), see [Security and TLS](/docs/v3/configuration/security-tls).

For OCSP stapling configuration, see [OCSP stapling](/docs/v3/configuration/ocsp-stapling).

For session ticket keys, see [TLS session ticket keys](/docs/v3/configuration/tls-session-tickets).

### `admin_email`

- `admin_email <email: string>`
  - This directive specifies the server administrator's email address. Used in built-in error responses. Interpolation is supported. Default: none

### Metrics

The HTTP server emits the following OpenTelemetry-style metrics via the observability event system:

- `http.server.active_requests` (UpDownCounter) — number of active HTTP requests.
- `http.server.request.duration` (Histogram) — duration of HTTP requests in seconds.
- `ferron.http.server.request_count` (Counter) — total number of HTTP requests completed.

All metrics include attributes for `http.request.method`, `url.scheme`, `network.protocol.name`, and `network.protocol.version`. The `http.server.request.duration` and `ferron.http.server.request_count` metrics also include `http.response.status_code` and `error.type` (for 4xx/5xx responses).

## Notes and troubleshooting

- These directives are host-scoped rather than global.
- The HTTP server engine (`http-server` module) handles connection management, request routing, TLS termination, and HTTP/1 and HTTP/2 protocol support.
- For ACME configuration details, see [ACME automatic TLS](/docs/v3/configuration/tls-acme).
- For crypto and mTLS settings, see [Security and TLS](/docs/v3/configuration/security-tls).
