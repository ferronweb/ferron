---
title: "Configuration: forward proxy"
description: "Forward proxy, CONNECT tunneling, access control, and domain allowlisting directives."
---

This page documents directives for configuring Ferron to act as an HTTP forward proxy, accepting requests from clients and forwarding them to external destinations. It supports both HTTP CONNECT tunneling (for HTTPS/WebSocket) and HTTP/1.x absolute URI forwarding.

## `forward_proxy`

```ferron
proxy.example.com {
    forward_proxy {
        allow_domains "example.com" "*.example.com"
        allow_ports 80 443
        deny_ips "127.0.0.0/8" "169.254.169.254/32"

        connect_method true
        http_version "1.1"
    }
}
```

| Nested directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `allow_domains` | `<string>...` | Allowed destination domains. Supports `*` wildcards. If empty, all domains are denied (deny-by-default). | none (deny all) |
| `allow_ports` | `<int>...` | Allowed destination ports. | `80`, `443` |
| `deny_ips` | `<CIDR>...` | Denied destination IP ranges, applied after DNS resolution. | Loopback, RFC 1918, link-local, cloud metadata (see below) |
| `connect_method` | `<bool>` or bare | Enable HTTP CONNECT tunneling. When disabled, CONNECT requests are rejected with 403. | `true` |
| `http_version` | `1.0` or `1.1` | HTTP version used for upstream connections. | `1.1` |

### Default denied IP ranges

When no `deny_ips` is specified, the following ranges are denied by default:

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

The forward proxy operates on a **deny-by-default** model:

1. **Domain allowlisting**: If `allow_domains` is not configured, all destination domains are denied.
2. **Port allowlisting**: Only explicitly listed ports are allowed. Defaults to `80` and `443`.
3. **IP denylisting**: After DNS resolution, the resolved IP is checked against the deny list (including defaults). This prevents SSRF attacks.

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

Only `http` scheme is supported. Requests with `https` scheme are rejected with 400.

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

## Notes and troubleshooting

- The `forward_proxy` directive is scoped to individual HTTP host blocks.
- Domain patterns support `*` wildcards (e.g. `*.example.com` matches `api.example.com`).
- DNS resolution happens at connect time. The resolved IP is validated against the deny list to prevent DNS rebinding attacks.
- For HTTPS forwarding, clients must use CONNECT tunneling — direct `https://` URLs in HTTP requests are not supported.
- For reverse proxy configuration, see [Reverse proxy](/docs/v3/configuration/reverse-proxying).
- For HTTP host directives, see [HTTP host directives](/docs/v3/configuration/http-host).
