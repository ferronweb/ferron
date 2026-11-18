---
title: "Configuration: HTTP host directives"
description: "Per-host directives for protocol behavior, TLS, HTTPS redirects, client IP resolution, and server metadata."
---

HTTP host blocks such as `example.com` and `http example.com:8080` accept the directives on this page.

> [!note]
> Ferron scopes these directives to individual hosts, not globally.
>
> The HTTP server engine (`http-server` module) handles connection management, request routing, and TLS termination. It supports HTTP/1, HTTP/2, and HTTP/3.

> [!info]
> For ACME settings, see [ACME automatic TLS](/docs/configuration/security/acme). For crypto and mTLS settings, see [Security and TLS](/docs/configuration/security/tls).

## Automatic TLS

A hostname without an explicit port (for example, `example.com`) starts two listeners:

- `default_http_port` (default: 80) serves plain HTTP.
- `default_https_port` (default: 443) serves HTTPS with automatic ACME TLS.

On the HTTPS listener, Ferron enables TLS through the ACME provider, which is Let's Encrypt by default. The provider gets and renews certificates at startup. An explicit `tls` directive overrides this behavior.

`localhost`, `127.0.0.1`, and `::1` use the local TLS provider instead of ACME. This gives HTTPS for development without public certificates.

To disable automatic TLS on the HTTPS listener:

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
        cert /etc/ssl/cert.pem
        key /etc/ssl/key.pem
    }
    root /var/www/html
}
```

The alias form takes the certificate and key paths directly:

```ferron
example.com {
    tls /etc/ssl/cert.pem /etc/ssl/key.pem
    root /var/www/html
}
```

An explicit port (for example, `example.com:8080`) starts one listener on that port only. Ferron does not add ACME TLS, so configure TLS explicitly.

## HTTPS redirect

`https_redirect <bool>` enables or disables the automatic HTTP to HTTPS redirect. The redirect uses `308 Permanent Redirect`, which preserves the method and body.

Default: `true` when TLS is enabled.

A named host does not inherit `https_redirect` from the wildcard `*` host. It still inherits the global value, and a `location` block inherits it from its own host.

```ferron
example.com {
    https_redirect false
}
```

> [!note]
> Ferron never redirects `localhost` hostnames, because they have no HTTPS listener.
>
> An explicit port such as `example.com:8080` has no separate HTTPS listener, so no redirect happens.
>
> The redirect target is `default_https_port` (default: `443`). Ferron omits port `443` from the URL.

## Client IP from forwarded headers

`client_ip_from_header <header: string> { ... }` sets the header that carries the client IP. Values: `x-forwarded-for` and `forwarded`. The directive is per host and disabled by default.

A named host does not inherit `client_ip_from_header` from the wildcard `*` host. It still inherits the global value.

| Nested directive | Arguments                 | Description                                                                            | Default |
| ---------------- | ------------------------- | -------------------------------------------------------------------------------------- | ------- |
| `trusted_proxy`  | `<ip-or-cidr: string>...` | Reverse-proxy IPs or CIDR ranges that you trust to supply forwarded client IP headers. | none    |

```ferron
{
    client_ip_from_header x-forwarded-for {
        trusted_proxy "10.0.0.0/8"
        trusted_proxy "192.168.0.0/16"
    }
}

example.com {
    root /var/www/html
}
```

### `x-forwarded-for`

Ferron reads the `X-Forwarded-For` header and takes the first (leftmost) IP address from the comma-separated chain.

### `forwarded` (RFC 7239)

Ferron reads the `Forwarded` header and takes the first `for=` token. Quoted and unquoted values are supported, including IPv6 addresses.

> [!warning]
> Ferron trusts a forwarded client IP header only when the connecting peer matches at least one `trusted_proxy` entry. With an empty `trusted_proxy` list, Ferron ignores the header. Keep the list limited to reverse proxies and load balancers that you control.

## HTTP protocol settings

The `http` block holds these directives. Ferron reads them at different points in request handling, so their scope differs:

| Read when                   | Directives                                                                              | Scope                                                                                                                      |
| --------------------------- | --------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------- |
| While accepting a connection | `protocol_proxy`                                                                        | Global, or a host block without a hostname                                                                                 |
| When the connection starts  | `protocols`, `timeout`, `h1_enable_early_hints`, `h2_*`, `h3_*`                          | Global, with a per-host override                                                                                           |
| Before routing a request    | `url_sanitize`, `url_reject_backslash`, `trace`, `trace_sampling`                        | Global only for `url_sanitize` and `url_reject_backslash`. Global with a per-host override for `trace` and `trace_sampling` |
| After routing a request     | `options_allowed_methods`                                                               | Global, with a per-host override                                                                                           |

A per-host override applies to requests whose host matches the host block. A host block without the directive inherits it from the global `http` block. Ferron reads global-only directives from the top-level `http` block only. Placing `url_sanitize` or `url_reject_backslash` in a host block has no effect.

### `protocols`

`protocols <protocols: string>...` sets the enabled HTTP protocols:

- `h1`: HTTP/1.1
- `h2`: HTTP/2
- `h2c`: HTTP/2 over cleartext with prior knowledge
- `h3`: HTTP/3

Default: `protocols h1 h2 h3`.

At least one protocol must stay enabled.

### `timeout`

`timeout <duration>` sets the pipeline execution timeout. It accepts a duration string (`30m`, `1h`, `90s`), a number of seconds, or `false` to disable.

Default: `timeout "5m"`.

### `options_allowed_methods`

`options_allowed_methods <methods: string>` sets the methods advertised in the `Allow` header for `OPTIONS *` requests. Ferron sends them as a comma-separated list. The directive applies to server-wide `OPTIONS *` only, not to `OPTIONS /path`.

The value supports [interpolated strings](/docs/configuration/fundamentals/conditionals). Because Ferron sends it as the `Allow` response header, it must not contain control bytes or line breaks. `ferron validate` reports an unusable configured value. If an interpolated value resolves to something that cannot be sent as a header, Ferron falls back to the default list.

Default: `options_allowed_methods "GET, HEAD, POST, OPTIONS"`.

The default excludes `PUT`, `DELETE`, `PATCH`, `CONNECT`, and `TRACE`, which reduces the attack surface that security scanners report. Customize the list to match the server.

### `h1_enable_early_hints`

`h1_enable_early_hints <bool>` enables 103 Early Hints for HTTP/1.1.

Default: `false`.

The `early_hints` block sets the links. See [Response control](/docs/configuration/routing/response).

### HTTP/2 and HTTP/3 tuning

| Directive                      | Argument          | Default | Notes                                                                                       |
| ------------------------------ | ----------------- | ------- | ------------------------------------------------------------------------------------------- |
| `h2_initial_window_size`       | `<size: integer>` | unset   | Initial flow-control window. Range: `0` to `2147483647`.                                    |
| `h2_max_frame_size`            | `<size: integer>` | unset   | Maximum frame size. Range: `16384` to `16777215`.                                           |
| `h2_max_concurrent_streams`    | `<count: integer>`| unset   | Maximum concurrent streams.                                                                 |
| `h2_max_header_list_size`      | `<size: integer>` | unset   | Maximum header list size. A high value enables HPACK memory exhaustion attacks.             |
| `h2_enable_connect_protocol`   | `[bool]`          | `false` | Extended CONNECT protocol.                                                                  |
| `h3_qpack_max_table_capacity`  | `<size: integer>` | unset   | Maximum QPACK table capacity.                                                               |
| `h3_qpack_blocked_streams`     | `<count: integer>`| unset   | Number of blocked streams.                                                                  |
| `h3_max_field_section_size`    | `<size: integer>` | unset   | Maximum field section size. A high value enables QPACK memory exhaustion attacks.           |
| `h3_enable_connect_protocol`   | `[bool]`          | `false` | Extended CONNECT protocol.                                                                  |

Enabling HTTP/3 also starts a QUIC listener on the same port. Ferron adds an `Alt-Svc` header to responses so that clients discover HTTP/3.

### `url_sanitize`

`url_sanitize [bool]` enables URL path sanitization based on URL canonicalization. Ferron removes or normalizes path traversal attempts (`../`, `..\`), null bytes, and invalid percent-encodings.

Default: `true`.

Ferron applies sanitization early in request processing, before configuration resolution. The directive applies to global scope only.

> [!note]
> Disabling sanitization can improve RFC 3986 compliance for URLs with valid but unusual encodings. The file resolution stage still canonicalizes paths and rejects requests that escape the configured webroot.

> [!warning]
> With `url_sanitize` disabled, Ferron does not protect backend services from path traversal attacks in a reverse proxy setup.

### `url_reject_backslash`

`url_reject_backslash [bool]` rejects URLs that contain backslashes. Ferron responds with `400 Bad Request` for a literal `\` or a percent-encoded backslash (`%5C` or `%5c`) in the path. This prevents path interpretation problems on Windows backends.

Default: `true`.

Ferron applies the check early in request processing, before configuration resolution and URL sanitization. The directive applies to global scope only.

> [!warning]
> Disable `url_reject_backslash` only when a Windows backend legitimately uses backslashes in URLs. The change can expose backends to path interpretation vulnerabilities.

### `protocol_proxy`

`protocol_proxy [bool]` enables PROXY protocol v1 and v2 parsing for incoming TCP connections. Ferron reads the PROXY header from HAProxy or a similar load balancer before it processes the HTTP request. The client and server addresses in the header replace the socket addresses for the life of the connection.

Default: `false`.

Ferron reads the directive from the global `http` block or from a host block without a hostname, because it parses the header before it reads the request. A named host block cannot enable it.

> [!note]
> Ferron supports PROXY protocol v1 (text) and v2 (binary). A parse failure rejects the connection and logs an error.
>
> Each listener has its own setting. To enable the PROXY protocol on the HTTP and HTTPS listeners, set it in the global `http` block, or in both `*:80` and `*:443`.

### Trace context and sampling

The `http` block also accepts `trace { generate, trust_request }` and `trace_sampling`. Ferron reads both before it routes the request. A host block can set them for one host, and the global block sets the defaults.

```ferron
{
    http {
        trace {
            trust_request
        }
        trace_sampling parentbased_traceidratio {
            ratio 0.1
        }
    }
}

noisy.example.com {
    root /var/www/noisy
    http {
        trace_sampling always_off
    }
}
```

> [!info]
> For the trace ID response header, see [`trace_id_header`](/docs/configuration/observability/tracing#trace_id_header). For all trace directives, see [Tracing](/docs/configuration/observability/tracing).

### Example

```ferron
{
    http {
        protocols h1 h2 h3
        options_allowed_methods "GET, HEAD, POST, PUT, DELETE, OPTIONS"
        timeout "30m"
        h1_enable_early_hints false
    }
}

api.example.com {
    root /var/www/api
    http {
        protocols h2 h3
        timeout "10s"
    }
}

static.example.com {
    root /var/www/static
    http {
        options_allowed_methods "GET, HEAD, OPTIONS"
    }
}
```

## TLS provider

`provider <name: string>` sets the TLS provider. It is required when TLS is enabled through the block form.

| Value    | Module      |
| -------- | ----------- |
| `manual` | `tls-manual`|
| `acme`   | `tls-acme`  |

Default: none.

> [!info]
> For crypto settings (`cipher_suite`, `ecdh_curve`, `min_version`, `max_version`, `client_auth`, `client_auth_ca`), see [Security and TLS](/docs/configuration/security/tls).
>
> For OCSP stapling, see [OCSP stapling](/docs/configuration/security/ocsp). For session ticket keys, see [TLS session ticket keys](/docs/configuration/security/session-tickets).

## `admin_email`

`admin_email <email: string>` sets the server administrator address. Ferron includes it in built-in error responses. The value supports interpolation.

Default: none.

## Metrics

The HTTP server emits these OpenTelemetry-style metrics through the observability event system:

| Metric                             | Type          | Attributes                                                                                                                          | Description                             |
| ---------------------------------- | ------------- | ----------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------- |
| `http.server.active_requests`      | UpDownCounter | `http.request.method`, `url.scheme`, `network.protocol.name`, `network.protocol.version`                                            | Number of active HTTP requests          |
| `http.server.request.duration`     | Histogram     | `http.request.method`, `url.scheme`, `network.protocol.name`, `network.protocol.version`, `http.response.status_code`, `error.type` | How long HTTP requests take, in seconds |
| `ferron.http.server.request_count` | Counter       | `http.request.method`, `url.scheme`, `network.protocol.name`, `network.protocol.version`, `http.response.status_code`, `error.type` | Total number of HTTP requests completed |

Every metric carries `http.request.method`, `url.scheme`, `network.protocol.name`, and `network.protocol.version`. `http.server.request.duration` and `ferron.http.server.request_count` also carry `http.response.status_code` and `error.type` for 4xx and 5xx responses.

## Best practices

`ferron doctor` reports these checks for directives on this page.

### Client IP resolution

- `trusted_proxy 0.0.0.0/0` or `::/0`: trusting every source allows spoofing. Restrict `trusted_proxy` to the reverse proxy addresses.
- `client_ip_from_header` without `trusted_proxy`: without trusted ranges, Ferron ignores or distrusts forwarded headers. Add `trusted_proxy` entries for the reverse proxies.

### URL processing

- `url_sanitize false`: disabling path traversal normalization can expose backend path interpretation issues. Keep sanitization enabled unless a backend requires raw paths.
- `url_reject_backslash false`: allowing backslashes in request paths can confuse backend routing. Keep rejection enabled unless required.

### Timeouts

- `timeout false`: without a pipeline timeout, slow requests can exhaust server resources. Set a bounded value.

### HTTP methods

- `options_allowed_methods` with `TRACE` or `CONNECT`: advertising these methods can expose attack surface. Remove them unless the server supports them.

### TLS deployment

- HTTP-only host without TLS: for a non-localhost host block without `tls`, `ferron doctor` emits a reminder that an upstream proxy or load balancer should terminate TLS. The reminder is informational. HTTP-only is legitimate behind CDNs, load balancers, and Kubernetes ingress controllers that terminate TLS.

## Observability

### Trace spans

The HTTP server sets these attributes on per-stage spans:

| Span name                            | Attributes                                             | Description                  |
| ------------------------------------ | ------------------------------------------------------ | ---------------------------- |
| `ferron.stage.client_ip_from_header` | `ferron.client_ip.source`, `ferron.client_ip.original` | Client IP resolution stage   |
| `ferron.stage.https_redirect`        | `ferron.redirect.target`                               | HTTP to HTTPS redirect stage |
