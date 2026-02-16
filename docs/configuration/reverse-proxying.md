---
title: "Configuration: reverse proxying"
description: "Reverse proxy, load balancing, forward proxy, and forwarded-authentication directives."
---

This page documents KDL directives for reverse proxying, backend balancing, forward proxying, and external auth forwarding.

## Global-only directives

### Reverse proxy & load balancing

- `proxy_concurrent_conns <proxy_concurrent_conns: integer|null>` (_rproxy_ module; Ferron 2.3.0 or newer)
  - This directive specifies the limit of TCP connections being established to backend servers, to prevent exhaustion of network resources. If set as `proxy_concurrent_conns #null`, the reverse proxy can theoretically establish an unlimited number of connections. Default: `proxy_concurrent_conns 16384`

**Configuration example:**

```kdl
* {
    proxy_concurrent_conns 16384
}
```

### Authentication forwarding

- `auth_to_concurrent_conns <auth_to_concurrent_conns: integer|null>` (_fauth_ module; Ferron 2.4.0 or newer)
  - This directive specifies the limit of TCP connections being established to backend servers (for forwarded authentication), to prevent exhaustion of network resources. If set as `auth_to_concurrent_conns #null`, the reverse proxy can theoretically establish an unlimited number of connections. Default: `auth_to_concurrent_conns 16384`

**Configuration example:**

```kdl
* {
    auth_to_concurrent_conns 16384
}
```

## Directives

### Reverse proxy & load balancing

- `proxy <proxy_to: string|null> [unix=<unix_socket_path: string>] [limit=<conn_limit: integer|null>] [idle_timeout=<idle_timeout: integer|null>]` (_rproxy_ module)
  - This directive specifies the URL to which the reverse proxy should forward requests. HTTP (for example `http://localhost:3000/`) and HTTPS URLs (for example `https://localhost:3000/`) are supported. Unix sockets are also supported via the `unix` prop set to the path to the socket (and the main value is set to the URL of the website), supported only on Unix and Unix-like systems. Established connections can be limited by the `limit` prop (Ferron 2.3.0 and newer); this can be useful for backend server that don't utilize event-driven I/O. Timeout for idle kept-alive connections (in milliseconds) can also be specified via the `idle_timeout` prop (Ferron 2.3.0 and newer); by default it is set to `60000` (60 seconds). This directive can be specified multiple times. Default: none
- `lb_health_check [enable_lb_health_check: bool]` (_rproxy_ module)
  - This directive specifies whether the load balancer passive health check is enabled. Default: `lb_health_check #false`
- `lb_health_check_max_fails <max_fails: integer>` (_rproxy_ module)
  - This directive specifies the maximum number of consecutive failures before the load balancer marks a backend as unhealthy. Default: `lb_health_check_max_fails 3`
- `proxy_no_verification [proxy_no_verification: bool]` (_rproxy_ module)
  - This directive specifies whether the reverse proxy should not verify the TLS certificate of the backend. Default: `proxy_no_verification #false`
- `proxy_intercept_errors [proxy_intercept_errors: bool]` (_rproxy_ module)
  - This directive specifies whether the reverse proxy should intercept errors from the backend. Default: `proxy_intercept_errors #false`
- `proxy_request_header <header_name: string> <header_value: string>` (_rproxy_ module)
  - This directive specifies a header to be added to HTTP requests sent by the reverse proxy. The header values supports placeholders like `{path}` which will be replaced with the request path. This directive can be specified multiple times. Default: none
- `proxy_request_header_remove <header_name: string>` (_rproxy_ module)
  - This directive specifies a header to be removed from HTTP requests sent by the reverse proxy. This directive can be specified multiple times. Default: none
- `proxy_keepalive [proxy_keepalive: bool]` (_rproxy_ module)
  - This directive specifies whether the reverse proxy should keep the connection to the backend alive. Default: `proxy_keepalive #true`
- `proxy_request_header_replace <header_name: string> <header_value: string>` (_rproxy_ module)
  - This directive specifies a header to be added to HTTP requests sent by the reverse proxy, potentially replacing existing headers. The header values supports placeholders like `{path}` which will be replaced with the request path. This directive can be specified multiple times. Default: none
- `proxy_http2 [enable_proxy_http2: bool]` (_rproxy_ module)
  - This directive specifies whether the reverse proxy can use HTTP/2 protocol when connecting to backend servers. This directive would have effect only if the backend server supports HTTP/2 and is connected via HTTPS. Default: `proxy_http2 #false`
- `lb_retry_connection [enable_lb_retry_connection: bool]` (_rproxy_ module)
  - This directive specifies whether the load balancer should retry connections to another backend server, in case of TCP connection or TLS handshake failure. Default: `lb_retry_connection #true`
- `lb_algorithm <lb_algorithm: string>` (_rproxy_ module)
  - This directive specifies the load balancing algorithm to be used. The supported algorithms are `random` (random selection), `round_robin` (round-robin), `least_conn` (least connections, "connections" would mean concurrent requests here), and `two_random` (power of two random choices; after two random choices, the backend server with the least concurrent requests is chosen). Default: `lb_algorithm "two_random"`
- `lb_health_check_window <lb_health_check_window: integer>` (_rproxy_ module)
  - This directive specifies the window size (in milliseconds) for load balancer health checks. Default: `lb_health_check_window 5000`
- `proxy_keepalive_idle_conns <proxy_keepalive_idle_conns: integer>` (_rproxy_ module; Ferron 2.2.1 or older; **REMOVED**) - This directive used to specify the maximum number of idle connections to backend servers to keep alive. The default was `proxy_keepalive_idle_conns 48`. In Ferron 2.3.0 and newer, this directive is no longer supported.
- `proxy_http2_only [enable_proxy_http2_only: bool]` (_rproxy_ module; Ferron 2.1.0 or newer)
  - This directive specifies whether the reverse proxy uses HTTP/2 protocol (without HTTP/1.1 fallback) when connecting to backend servers. When the backend server is connected via HTTPS, the reverse proxy negotiates HTTP/2 during the TLS handshake. When the backend server is connected via HTTP, the reverse proxy uses HTTP/2 with prior knowledge. This directive can be used when proxying gRPC requests. Default: `proxy_http2_only #false`
- `proxy_proxy_header <proxy_version_version: string|null>` (_rproxy_ module; Ferron 2.1.0 or newer)
  - This directive specifies the version of the PROXY protocol header to be sent to backend servers when acting as a reverse proxy. Supported versions are `"v1"` (PROXY protocol version 1) and `"v2"` (PROXY protocol version 2). If specified with `#null` value, no PROXY protocol header is sent. Default: `proxy_proxy_header #null`
- `proxy_srv <proxy_srv_to: string|null> [limit=<conn_limit: integer|null>] [idle_timeout=<idle_timeout: integer|null> [dns_servers=<dns_servers: string|null>]` (_rproxy_ module)
  - This directive specifies the URL (with hostname leading to an SRV record) to which the reverse proxy should forward requests. HTTP (for example `http://_http._tcp.example.com/`) and HTTPS URLs (for example `https://_https._tcp.example.com/`) are supported. Established connections can be limited by the `limit` prop (Ferron 2.3.0 and newer); this can be useful for backend server that don't utilize event-driven I/O. Timeout for idle kept-alive connections (in milliseconds) can also be specified via the `idle_timeout` prop (Ferron 2.3.0 and newer); by default it is set to `60000` (60 seconds). Custom DNS resolvers are also supported via specifying comma-separated IP addresses of DNS servers in the `dns_server` prop. This directive can be specified multiple times. Default: none

**Configuration example:**

```kdl
api.example.com {
    // Backends for load balancing
    // (or you can also use a single backend by specifying only one `proxy` directive)
    proxy "http://backend1:8080"
    proxy "http://backend2:8080"
    proxy "http://backend3:8080"

    // Health check configuration
    lb_health_check
    lb_health_check_max_fails 3
    lb_health_check_window 5000

    // Proxy settings
    proxy_no_verification #false
    proxy_intercept_errors #false
    proxy_keepalive
    proxy_http2 #false

    // Proxy headers
    proxy_request_header "X-Custom-Header" "CustomValue"

    proxy_request_header_remove "X-Internal-Token"
    proxy_request_header_replace "X-Real-IP" "{client_ip}"
}
```

### Forward proxy

- `forward_proxy [enable_forward_proxy: bool]` (_fproxy_ module)
  - This directive specifies whether the forward proxy functionality is enabled. Default: `forward_proxy #false`
- `forward_proxy_auth [enable_forward_proxy_auth: bool] [realm=<realm: string>] [brute_protection=<enable_brute_protection: bool>] [users=<users: string>]` (_fproxyauth_ module; Ferron 2.4.0 or newer)
  - This directive specifies whether the forward proxy authentication (HTTP Basic authentication) is enabled. The `realm` prop specifies the HTTP basic authentication realm. The `brute_protection` prop specifies whether the brute-force protection is enabled. The `users` prop is a comma-separated list of allowed users for HTTP authentication. Default: `forward_proxy #false`

**Configuration example:**

```kdl
* {
    forward_proxy
}
```

### Authentication forwarding

- `auth_to <auth_to: string|null> [unix=<unix_socket_path: string>] [limit=<conn_limit: integer|null>] [idle_timeout=<idle_timeout: integer|null>]` (_fauth_ module)
  - This directive specifies the URL to which the web server should send requests for forwarded authentication. Unix sockets are also supported via the `unix` prop set to the path to the socket (and the main value is set to the URL of the website), supported only on Unix and Unix-like systems (Ferron UNRELEASED and newer). Established connections can be limited by the `limit` prop (Ferron 2.4.0 and newer); this can be useful for backend server that don't utilize event-driven I/O. Timeout for idle kept-alive connections (in milliseconds) can also be specified via the `idle_timeout` prop (Ferron 2.4.0 and newer); by default it is set to `60000` (60 seconds). Default: none
- `auth_to_no_verification [auth_to_no_verification: bool]` (_fauth_ module)
  - This directive specifies whether the server should not verify the TLS certificate of the backend authentication server. Default: `auth_to_no_verification #false`
- `auth_to_copy <request_header_to_copy: string> [<request_header_to_copy: string> ...]` (_fauth_ module)
  - This directive specifies the request headers that will be copied and sent to the forwarded authentication backend server. This directive can be specified multiple times. Default: none

**Configuration example:**

```kdl
app.example.com {
    // Forward authentication to external service
    auth_to "https://auth.example.com/validate"
    auth_to_no_verification #false
    auth_to_copy "Authorization" "X-User-Token" "X-Session-ID"
}
```
