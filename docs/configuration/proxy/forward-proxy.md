---
title: "Configuration: forward proxy"
description: "Forward proxy, CONNECT tunneling, IP and domain access control directives."
---

This page documents directives for configuring Ferron as an HTTP forward proxy. Ferron accepts requests from clients and forwards them to external destinations. It supports HTTP CONNECT tunneling (for HTTPS/WebSocket) and HTTP/1.x absolute URI forwarding.

## `forward_proxy`

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

| Nested directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `allow_domains` | `<string>...` | Allowed destination domains. Supports `*` wildcards. If empty, Ferron denies all domains (deny-by-default). | none (deny all) |
| `allow_ports` | `<int>...` | Allowed destination ports. | `80`, `443` |
| `deny_ips` | `<CIDR>...` | Denied destination IP ranges, applied after DNS resolution. | Loopback, RFC 1918, link-local, cloud metadata (see below) |
| `connect_method` | `<bool>` or bare | Enable HTTP CONNECT tunneling. When disabled, Ferron rejects CONNECT requests with 403. | `true` |
| `http_version` | `1.0` or `1.1` | HTTP version used for upstream connections. | `1.1` |

### Default denied IP ranges

When you specify no `deny_ips`, Ferron denies the following ranges by default:

| Range | Description |
| --- | --- |
| `127.0.0.0/8` | IPv4 loopback |
| `::1/128` | IPv6 loopback |
| `10.0.0.0/8` | RFC 1918 private network |
| `172.16.0.0/12` | RFC 1918 private network |
| `192.168.0.0/16` | RFC 1918 private network |
| `169.254.0.0/16` | Link-local |
| `100.64.0.0/10` | Shared address space (RFC 6598) |
| `192.0.2.0/24`, `198.51.100.0/24`, `203.0.113.0/24` | Documentation ranges (RFC 5737) |
| `fd00::/8` | IPv6 unique local addresses |
| `169.254.169.254/32` | Cloud metadata endpoint |

### Security model

The forward proxy uses a **deny-by-default** model:

1. **Domain control**: If you do not set `allow_domains`, Ferron denies all destination domains.
2. **Port control**: Ferron permits only explicitly allowed ports (defaults to 80 and 443).
3. **IP blocking**: After DNS resolution, Ferron checks the final IP against the deny list.

> [!note]
> DNS resolution happens at connect time. Ferron validates the resolved IP against the deny list to prevent DNS rebinding attacks.

## Request handling

### CONNECT tunneling

When a client sends an HTTP CONNECT request, Ferron:

1. Validates the destination against ACLs (domain, port, IP)
2. Establishes a TCP connection to the target
3. Returns `200 Connection Established` to the client
4. Bidirectionally forwards raw TCP data between client and target

### HTTP forwarding

When a client sends an HTTP request with an absolute URI, Ferron:

1. Validates the destination against ACLs
2. Connects to the target host
3. Rewrites the request URI to path-only form
4. Forwards the request via HTTP/1.1
5. Returns the upstream response to the client

Ferron supports only the `http` scheme. It rejects `https` requests with 400.

## Examples

### Basic forward proxy

```ferron
proxy.example.com {
    forward_proxy {
        allow_domains "example.com" "*.example.com" "api.service.internal"
        allow_ports 80 443
    }
}
```

### Forward proxy with explicit IP denylist

```ferron
proxy.example.com {
    forward_proxy {
        allow_domains "*.corp.example.com"
        allow_ports 80 443 8080
        deny_ips "10.0.0.0/8" "172.16.0.0/12"
    }
}
```

### Forward proxy without CONNECT

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
> For HTTPS forwarding, clients must use CONNECT tunneling. Direct `https://` URLs in HTTP requests are not supported.

## Observability

### Metrics

| Metric | Type | Attributes | Description |
|--------|------|------------|-------------|
| `ferron.forward_proxy.requests` | Counter | `ferron.forward_proxy.mode` (`"connect"` or `"request"`), `ferron.forward_proxy.result` (outcome), `http.response.status_code`, `error.type` (optional) | Forward-proxy requests by mode and outcome |

### Logs

- **`ERROR`**: Ferron logs this when a CONNECT upgrade fails and produces no upgrade future. It also logs when the backend TCP connection fails, the HTTP/1 handshake fails, or the request to the backend fails. The message includes the target address and error details.

- **`WARN`**: Ferron logs this when the ACL denies a CONNECT or HTTP request by port or domain. It also logs when the CONNECT method is off. It also logs when a client sends a malformed request or uses an unsupported scheme. It also logs when Ferron cannot set TCP_NODELAY, DNS resolution fails, or a CONNECT tunnel encounters an error.

- **`INFO`**: Ferron logs this when a CONNECT tunnel closes normally. The message includes byte counts for each direction.

### Structured logs

| Description (summary) | Level | Attributes |
|-----------------------|-------|------------|
| Forward proxy config error | ERROR | `error.message` (string) : configuration error details |
| Forward proxy CONNECT upgrade failed | ERROR | `forward_proxy.target` (string) : target address, `error.type` (string), `error.message` (string) |
| Forward proxy connection to target failed | ERROR | `forward_proxy.target` (string) : target address, `error.type` (string), `error.message` (string) |
| Forward proxy CONNECT tunnel error | WARN | `forward_proxy.target` (string) : target address, `error.type` (string), `error.message` (string) |
| Forward proxy: upstream connect failed | ERROR | `upstream.address` (string) : target address, `error.type` (string), `error.message` (string) |
| Forward proxy: HTTP/1 handshake failed | ERROR | `error.type` (string), `error.message` (string) |
| Forward proxy: request to backend failed | ERROR | `error.type` (string), `error.message` (string) |
| Forward proxy: port denied by ACL | WARN | `network.destination.port` (int) : denied port, `error.type` (string) |
| Forward proxy: domain denied by ACL | WARN | `network.destination.name` (string) : denied domain, `error.type` (string) |
| Forward proxy: DNS resolution failed | WARN | `dns.name` (string) : hostname that failed resolution, `error.type` (string) |
| Forward proxy: resolved IP denied | WARN | `dns.name` (string) : hostname, `error.type` (string) |
| Forward proxy: CONNECT disabled | WARN | `error.type` (string) |
| Forward proxy: bad CONNECT request | WARN | `error.type` (string) |
| Forward proxy: unsupported scheme | WARN | `url.scheme` (string) : the unsupported scheme, `error.type` (string) |
| Forward proxy: missing host | WARN | `error.type` (string) |

### Access log fields

The forward proxy module contributes the following field to the HTTP access log line:

| Field | Type | Description |
| --- | --- | --- |
| `ferron.fproxy.mode` | string | Forward proxy mode: `tunnel` (CONNECT) or `proxy` (standard). |

### Trace spans

The forward proxy stage sets the following attributes on its `ferron.stage.forward_proxy` span:

| Attribute | Type | Description |
| --- | --- | --- |
| `ferron.fproxy.mode` | string | Proxy mode: `tunnel` (CONNECT) or `proxy` (standard). |
| `ferron.fproxy.upstream` | string | The upstream host and port. |
| `http.response.status_code` | int | HTTP status code returned to the client. |
| `error.type` | string | Error type on failure, enabling trace UI highlighting. |

## Best practices

`ferron doctor` reports the following best-practice checks for directives on this page.

- **`allow_domains "*"`**: Allowing proxying to any public domain defeats the purpose of a forward proxy. Restrict to destinations your clients actually need.
- **Custom `deny_ips` without loopback and metadata ranges**: Overriding the default deny list without including `127.0.0.0/8`, `::1`, and `169.254.169.254/32` can expose internal services through the proxy.
