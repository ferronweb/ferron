---
title: "Configuration: forward proxy"
description: "Forward proxy, CONNECT tunneling, and IP and domain access control directives."
---

Ferron can act as an HTTP forward proxy. Clients send requests to Ferron, and Ferron connects to the external destination. Ferron supports HTTP CONNECT tunneling for HTTPS and WebSocket traffic, and HTTP/1.x absolute URI forwarding.

## `forward_proxy`

`forward_proxy` configures the forward proxy for a host.

> [!note]
> Host isolation: a named host does not inherit `forward_proxy` from the wildcard `*` host. It inherits from global defaults. A `location` block inherits it from its own host.

```ferron
proxy.example.com {
    forward_proxy {
        allow_domains "example.com" "*.example.com"
        allow_ports 80 443
        deny_ips "127.0.0.0/8" "169.254.169.254/32"

        connect_method
        http_version "1.1"
    }
}
```

| Nested directive | Arguments        | Description                                                                                | Default                                                    |
| ---------------- | ---------------- | ------------------------------------------------------------------------------------------ | ---------------------------------------------------------- |
| `allow_domains`  | `<string>...`    | Allowed destination domains. Supports `*` wildcards. An empty list denies all domains.     | none (deny all)                                            |
| `allow_ports`    | `<int>...`       | Allowed destination ports.                                                                 | `80`, `443`                                                |
| `deny_ips`       | `<CIDR>...`      | Denied destination IP ranges, checked after DNS resolution.                                | Loopback, RFC 1918, link-local, and metadata ranges (below) |
| `connect_method` | `<bool>` or bare | Enable HTTP CONNECT tunneling. When disabled, Ferron rejects CONNECT requests with 403.    | `true`                                                     |
| `http_version`   | `1.0` or `1.1`   | HTTP version for upstream connections.                                                     | `1.1`                                                      |

### Default denied IP ranges

When `deny_ips` is absent, Ferron denies these ranges:

| Range                                               | Description                     |
| --------------------------------------------------- | ------------------------------- |
| `0.0.0.0/8`                                         | Current network                 |
| `::/128`                                            | IPv6 unspecified address        |
| `127.0.0.0/8`                                       | IPv4 loopback                   |
| `::1/128`                                           | IPv6 loopback                   |
| `10.0.0.0/8`                                        | RFC 1918 private network        |
| `172.16.0.0/12`                                     | RFC 1918 private network        |
| `192.168.0.0/16`                                    | RFC 1918 private network        |
| `169.254.0.0/16`                                    | Link-local                      |
| `100.64.0.0/10`                                     | Shared address space (RFC 6598) |
| `192.0.2.0/24`, `198.51.100.0/24`, `203.0.113.0/24` | Documentation ranges (RFC 5737) |
| `fd00::/8`                                          | IPv6 unique local addresses     |
| `169.254.169.254/32`                                | Cloud metadata endpoint         |

### Security model

The forward proxy denies by default:

- Domains: if `allow_domains` is empty, Ferron denies every destination domain.
- Ports: Ferron permits only the ports in `allow_ports`, which defaults to 80 and 443.
- IPs: after DNS resolution, Ferron checks the final IP against the deny list.

DNS resolution happens at connect time. Ferron checks the resolved IP against the deny list to prevent DNS rebinding attacks.

## Request handling

### CONNECT tunneling

When a client sends an HTTP CONNECT request, Ferron:

1. Validates the destination against the domain, port, and IP ACLs.
2. Establishes a TCP connection to the target.
3. Returns `200 Connection Established` to the client.
4. Forwards raw TCP data in both directions between client and target.

### HTTP forwarding

When a client sends an HTTP request with an absolute URI, Ferron:

1. Validates the destination against the ACLs.
2. Connects to the target host.
3. Rewrites the request URI to path-only form.
4. Forwards the request over HTTP/1.1.
5. Returns the upstream response to the client.

Ferron supports only the `http` scheme. It rejects `https` requests with 400.

## Examples

### Allow specific destinations

```ferron
proxy.example.com {
    forward_proxy {
        allow_domains "example.com" "*.example.com" "api.service.internal"
        allow_ports 80 443
    }
}
```

### Disable CONNECT tunneling

```ferron
proxy.example.com {
    forward_proxy {
        allow_domains example.com
        allow_ports 80
        connect_method false
    }
}
```

> [!note]
> For HTTPS forwarding, clients must use CONNECT tunneling. Ferron does not support direct `https://` URLs in HTTP requests.

## Observability

### Metrics

| Metric                          | Type    | Attributes                                                                                                                                              | Description                                |
| ------------------------------- | ------- | ------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------ |
| `ferron.forward_proxy.requests` | Counter | `ferron.forward_proxy.mode` (`"connect"` or `"request"`), `ferron.forward_proxy.result` (outcome), `http.response.status_code`, `error.type` (optional) | Forward proxy requests by mode and outcome |

### Logs

- `ERROR`: Ferron logs a CONNECT upgrade that produces no upgrade future. It also logs a failed backend TCP connection, a failed HTTP/1 handshake, or a failed request to the backend. The message includes the target address and error details.
- `WARN`: Ferron logs an ACL denial by port or domain, a disabled CONNECT method, a malformed request, or an unsupported scheme. It also logs a failure to set TCP_NODELAY, a DNS resolution failure, or a CONNECT tunnel error.
- `INFO`: Ferron logs a CONNECT tunnel that closes normally. The message includes byte counts for each direction.

### Structured logs

| Description (summary)                     | Level | Attributes                                                                                        |
| ----------------------------------------- | ----- | ------------------------------------------------------------------------------------------------- |
| Forward proxy config error                | ERROR | `error.message` (string): configuration error details                                             |
| Forward proxy CONNECT upgrade failed      | ERROR | `forward_proxy.target` (string): target address, `error.type` (string), `error.message` (string) |
| Forward proxy connection to target failed | ERROR | `forward_proxy.target` (string): target address, `error.type` (string), `error.message` (string) |
| Forward proxy CONNECT tunnel error        | WARN  | `forward_proxy.target` (string): target address, `error.type` (string), `error.message` (string) |
| Forward proxy: upstream connect failed    | ERROR | `upstream.address` (string): target address, `error.type` (string), `error.message` (string)     |
| Forward proxy: HTTP/1 handshake failed    | ERROR | `error.type` (string), `error.message` (string)                                                   |
| Forward proxy: request to backend failed  | ERROR | `error.type` (string), `error.message` (string)                                                   |
| Forward proxy: port denied by ACL         | WARN  | `network.destination.port` (int): denied port, `error.type` (string)                             |
| Forward proxy: domain denied by ACL       | WARN  | `network.destination.name` (string): denied domain, `error.type` (string)                        |
| Forward proxy: DNS resolution failed      | WARN  | `dns.name` (string): hostname that failed resolution, `error.type` (string)                      |
| Forward proxy: resolved IP denied         | WARN  | `dns.name` (string): hostname, `error.type` (string)                                             |
| Forward proxy: CONNECT disabled           | WARN  | `error.type` (string)                                                                             |
| Forward proxy: bad CONNECT request        | WARN  | `error.type` (string)                                                                             |
| Forward proxy: unsupported scheme         | WARN  | `url.scheme` (string): the unsupported scheme, `error.type` (string)                             |
| Forward proxy: missing host               | WARN  | `error.type` (string)                                                                             |

### Access log fields

The forward proxy module adds one field to the HTTP access log line:

| Field                | Type   | Description                                                   |
| -------------------- | ------ | ------------------------------------------------------------- |
| `ferron.fproxy.mode` | string | Forward proxy mode: `tunnel` (CONNECT) or `proxy` (standard). |

### Trace spans

The forward proxy stage sets these attributes on its `ferron.stage.forward_proxy` span:

| Attribute                   | Type   | Description                                            |
| --------------------------- | ------ | ------------------------------------------------------ |
| `ferron.fproxy.mode`        | string | Proxy mode: `tunnel` (CONNECT) or `proxy` (standard).  |
| `ferron.fproxy.upstream`    | string | The upstream host and port.                            |
| `http.response.status_code` | int    | HTTP status code returned to the client.               |
| `error.type`                | string | Error type on failure, for trace UI highlighting.      |

## Best practices

`ferron doctor` reports the following best-practice checks for directives on this page.

- `allow_domains "*"`: allowing any public domain defeats the purpose of a forward proxy. Restrict destinations to what your clients need.
- Custom `deny_ips` without loopback and metadata ranges: overriding the default deny list without `127.0.0.0/8`, `::1`, and `169.254.169.254/32` can expose internal services through the proxy.
