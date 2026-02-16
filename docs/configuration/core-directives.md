---
title: "Configuration: core directives"
description: "Core server directives for ports, protocols, networking, buffering, and process-level behavior."
---

This page covers core KDL directives that control Ferron's global HTTP behavior, network settings, and system-level limits.

## Global-only directives

### HTTP protocol & performance

- `default_http_port <default_http_port: integer|null>`
  - This directive specifies the default port for HTTP connections. If set as `default_http_port #null`, the implicit default HTTP port is disabled. Default: `default_http_port 80`
- `default_https_port <default_https_port: integer|null>`
  - This directive specifies the default port for HTTPS connections. If set as `default_https_port #null`, the implicit default HTTPS port is disabled. Default: `default_https_port 443`
- `protocols <protocol: string> [<protocol: string> ...]`
  - This directive specifies the enabled protocols for the web server. The supported protocols are `"h1"` (HTTP/1.x), `"h2"` (HTTP/2) and `"h3"` (HTTP/3; experimental). Default: `protocols "h1" "h2"`
- `timeout <timeout: integer|null>`
  - This directive specifies the maximum time (in milliseconds) for server to process the request, after which the server resets the connection. If set as `timeout #null`, the timeout is disabled. It's not recommended to disable the timeout, as this might leave the server vulnerable to Slow HTTP attacks. Default: `timeout 300000`
- `h2_initial_window_size <h2_initial_window_size: integer>`
  - This directive specifies the HTTP/2 initial window size. Default: Hyper defaults
- `h2_max_frame_size <h2_max_frame_size: integer>`
  - This directive specifies the maximum HTTP/2 frame size. Default: Hyper defaults
- `h2_max_concurrent_streams <h2_max_concurrent_streams: integer>`
  - This directive specifies the maximum amount of concurrent HTTP/2 streams. Default: Hyper defaults
- `h2_max_header_list_size <h2_max_header_list_size: integer>`
  - This directive specifies the maximum HTTP/2 frame size. Default: Hyper defaults
- `h2_enable_connect_protocol [h2_enable_connect_protocol: bool]`
  - This directive specifies whether the CONNECT protocol in HTTP/2 is enabled. Default: Hyper defaults
- `protocol_proxy [enable_proxy_protocol: bool]`
  - This directive specifies whether the PROXY protocol acceptation is enabled. If enabled, the server will expect the PROXY protocol header at the beginning of each connection. Default: `protocol_proxy #false`
- `buffer_request <request_buffer_size: integer|null>`
  - This directive specifies the buffer size in bytes for incoming requests. If set as `buffer_request #null`, the request buffer is disabled. The request buffer can serve as an additional protection for underlying backend servers against Slowloris-style attacks. Default: `buffer_request #null`
- `buffer_response <response_buffer_size: integer|null>`
  - This directive specifies the buffer size in bytes for outgoing responses. If set as `buffer_response #null`, the response buffer is disabled. Default: `buffer_response #null`

**Configuration example:**

```kdl
* {
    default_http_port 80
    default_https_port 443
    protocols "h1" "h2" "h3"
    timeout 300000
    h2_initial_window_size 65536
    h2_max_frame_size 16384
    h2_max_concurrent_streams 100
    h2_max_header_list_size 8192
    h2_enable_connect_protocol
    protocol_proxy #false
    buffer_request #null
    buffer_response #null
}
```

### Networking & system

- `listen_ip <listen_ip: string>`
  - This directive specifies the IP address to listen. Default: `listen_ip "::"`
- `io_uring [enable_io_uring: bool|null]`
  - This directive specifies whether `io_uring` is enabled. If set as `io_uring #null` (supported on Ferron 2.4.0 and newer), `io_uring` is enabled with fallback with `io_uring` disabled. This directive has no effect for systems that don't support `io_uring` and for web server builds that use Tokio instead of Monoio. Default: `io_uring #null` (Ferron 2.4.0 or newer), `io_uring #true` (Ferron 2.3.2 and older)
- `tcp_send_buffer <tcp_send_buffer: integer>`
  - This directive specifies the send buffer size in bytes for TCP listeners. Default: none
- `tcp_recv_buffer <tcp_recv_buffer: integer>`
  - This directive specifies the receive buffer size in bytes for TCP listeners. Default: none

**Configuration example:**

```kdl
* {
    listen_ip "0.0.0.0"
    io_uring
    tcp_send_buffer 65536
    tcp_recv_buffer 65536
}
```
