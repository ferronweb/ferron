# HTTP Forward Proxy Directives

The `forward_proxy` directive configures Titanium to act as an HTTP forward proxy, accepting requests from clients and forwarding them to external destinations. It supports both HTTP CONNECT tunneling (for HTTPS/WebSocket) and HTTP/1.x absolute URI forwarding.

## Categories

- Main directive: `forward_proxy`
- Access control: `allow_domains`, `allow_ports`, `deny_ips`
- Protocol: `connect_method`, `http_version`

## `forward_proxy`

Syntax:

```ferron
proxy.example.com {
    forward_proxy {
        allow_domains example.com *.example.com
        allow_ports 80 443
        deny_ips 127.0.0.0/8 169.254.169.254/32

        connect_method true
        http_version "1.1"
    }
}
```

| Nested directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `allow_domains` | `<string>...` | Allowed destination domains. Supports `*` wildcards (e.g., `*.example.com`). If empty, all domains are denied (deny-by-default). | none (deny all) |
| `allow_ports` | `<int>...` | Allowed destination ports. | `80`, `443` |
| `deny_ips` | `<CIDR>...` | Denied destination IP ranges, applied after DNS resolution. Blocks the resolved IP if it falls within any listed range. | Loopback, RFC 1918, link-local, shared, cloud metadata (see below) |
| `connect_method` | `<bool>` or bare | Enable HTTP CONNECT tunneling. When disabled, CONNECT requests are rejected with 403. | `true` |
| `http_version` | `1.0` or `1.1` | HTTP version used for upstream connections when forwarding HTTP requests. | `1.1` |

### Default Denied IP Ranges

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

### Security Model

The forward proxy operates on a **deny-by-default** model:

1. **Domain allowlisting**: If `allow_domains` is not configured, all destination domains are denied.
2. **Port allowlisting**: Only explicitly listed ports are allowed. Defaults to `80` and `443`.
3. **IP denylisting**: After DNS resolution, the resolved IP is checked against the deny list (including defaults). This prevents SSRF attacks via cloud metadata or internal network access.

## Request Handling

### CONNECT Tunneling

When a client sends an HTTP CONNECT request (e.g., `CONNECT example.com:443 HTTP/1.1`), Titanium:

1. Validates the destination against ACLs (domain, port, IP)
2. Establishes a TCP connection to the target
3. Returns `200 Connection Established` to the client
4. Bidirectionally forwards raw TCP data between client and target

This is used for HTTPS traffic and WebSocket upgrades.

### HTTP Forwarding

When a client sends an HTTP request with an absolute URI (e.g., `GET http://example.com/path HTTP/1.1`), Titanium:

1. Validates the destination against ACLs
2. Connects to the target host
3. Rewrites the request URI to path-only form
4. Forwards the request via HTTP/1.1
5. Returns the upstream response to the client

Only `http` scheme is supported. Requests with `https` scheme are rejected with 400.

## Examples

### Basic Forward Proxy

```ferron
proxy.example.com {
    forward_proxy {
        allow_domains example.com *.example.com api.service.internal
        allow_ports 80 443
    }
}
```

This allows proxying to `example.com`, any subdomain of `example.com`, and `api.service.internal` on ports 80 and 443 only.

### Forward Proxy with Explicit IP Denylist

```ferron
proxy.example.com {
    forward_proxy {
        allow_domains *.corp.example.com
        allow_ports 80 443 8080
        deny_ips 10.0.0.0/8 172.16.0.0/12
    }
}
```

### Forward Proxy Without CONNECT

```ferron
proxy.example.com {
    forward_proxy {
        allow_domains example.com
        allow_ports 80
        connect_method false
    }
}
```

This configuration only supports HTTP/1.x absolute URI forwarding — CONNECT tunneling is disabled.

## Notes

- The `forward_proxy` directive is scoped to individual HTTP host blocks.
- Domain patterns support `*` wildcards (e.g., `*.example.com` matches `api.example.com`).
- DNS resolution happens at connect time. The resolved IP is validated against the deny list to prevent DNS rebinding attacks.
- The forward proxy stage runs before the `not_found` stage in the HTTP pipeline.
- For HTTPS forwarding, clients must use CONNECT tunneling — direct `https://` URLs in HTTP requests are not supported.

## See Also

- [HTTP Host Directives](./http-host.md)
- [Reverse Proxy Directives](./http-proxy.md)
