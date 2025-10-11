---
title: Server configuration
---

Ferron 2.0.0-beta.1 and newer can be configured in a [KDL-format](https://kdl.dev/) configuration file (often named `ferron.kdl`). Below are the descriptions of configuration properties for this server.

## Configuration blocks

At the top level of the server configration, the confguration blocks representing specific virtual host are specified. Below are the examples of such configuration blocks:

```kdl
globals {
  // Global configuration that doesn't imply any virtual host (Ferron 2.0.0-beta.13 or newer)
}

* {
  // Global configuration
}

*:80 {
  // Configuration for port 80
}

example.com {
  // Configuration for "example.com" virtual host
}

"192.168.1.1" {
  // Configuration for "192.168.1.1" IP virtual host
}

example.com:8080 {
  // Configuration for "example.com" virtual host with port 8080
}

"192.168.1.1:8080" {
  // Configuration for "192.168.1.1" IP virtual host with port 8080
}

api.example.com {
  // Below is the location configuration for paths beginning with "/v1/". If there was "remove_base=#true", the request URL for the location would be rewritten to remove the base URL
  location "/v1" remove_base=#false {
    // ...

    // Below is the error handler configuration for any status code.
    error_config {
      // ...
    }
  }

  // In Ferron 2.0.0-beta.14 and earlier, the location configuration order was important; in this host configuration, first the "/v1" location is checked, then the "/" location.
  // In Ferron 2.0.0-beta.15 and newer, the location and conditionals' configuration order is automatically determined based on the location and conditionals' depth
  location "/" {
    // ...
  }

  // Below is the error handler configuration for 404 Not Found status code. If "404" wasn't included, it would be for all errors.
  error_config 404 {
    // ...
  }
}

example.com,example.org {
  // Configuration for example.com and example.org (Ferron 2.0.0-beta.13 or newer)
  // The virtual host identifiers (like example.com or "192.168.1.1") are comma-separated, but adding spaces will not be interpreted,
  // For example "example.com, example.org" will not work for "example.org", but "example.com,example.org" will work.
}

with-conditions.example.com {
  condition "SOME_CONDITION" {
    // Here are defined subconditions in the condition. The condition will pass if all subconditions will also pass
  }

  if "SOME_CONDITION" {
    // Conditional configuration (Ferron 2.0.0-beta.15 or newer)
    // Conditions can be nested
  }

  if_not "SOME_CONDITION" {
    // Configuration, in case of condition not being met (Ferron 2.0.0-beta.15 or newer)
  }
}

snippet "EXAMPLE" {
  // Example snippet configuration (Ferron 2.0.0-beta.15 or newer)
}

with-snippet.example.com {
  // Import from snippet (Ferron 2.0.0-beta.15 or newer)
  use "EXAMPLE"
}

with-snippet.example.org {
  // Snippets can be reusable
  use "EXAMPLE"
}

inheritance.example.com {
  // The "proxy" directive is used as an example for demonstrating inheritance.
  proxy "http://10.0.0.2:3000"
  proxy "http://10.0.0.3:3000"

  // Here, these directives take effect:
  //   proxy "http://10.0.0.2:3000"
  //   proxy "http://10.0.0.3:3000"

  location "/somelocation" {
    // Here, `some_directive` directives are inherited from the parent block.
    // These directives take effect:
    //   proxy "http://10.0.0.2:3000"
    //   proxy "http://10.0.0.3:3000"
  }

  location "/anotherlocation" {
    // The directives from the parent block are not inherited if there are other directives with the same name in the block.
    // Here, these directives take effect:
    //   proxy "http://10.0.0.4:3000"
    proxy "http://10.0.0.4:3000"
  }
}
```

Also, it's possible to include other configuration files using an `include <included_configuration_path: string>` directive, like this:

```kdl
include "/etc/ferron.d/**/*.kdl"
```

## Directive categories overview

This configuration reference organizes directives by both **scope** (where they can be used) and **functional categories** (what they do). This makes it easier to find the directives you need based on your specific requirements.

### Scopes

- **Global-only** - can only be used in the global configuration scope
- **Global and virtual host** - can be used in both global and virtual host scopes
- **General directives** - can be used in various scopes including virtual hosts and location blocks

### Functional categories

- **TLS/SSL & security** - certificate management, encryption settings, and security policies
- **HTTP protocol & performance** - protocol settings, timeouts, and performance tuning
- **Networking & system** - network configuration and system-level settings
- **Caching** - HTTP caching configuration and cache management
- **Load balancing** - health checks and load balancer settings
- **Static file serving** - file serving, compression, and directory listings
- **URL processing & routing** - URL rewriting, redirects, and routing rules
- **Headers & response customization** - custom headers and response modification
- **Security & access control** - authentication, authorization, and access restrictions
- **Reverse proxy & load balancing** - proxy configuration and backend management
- **Forward proxy** - forward proxy functionality
- **Authentication forwarding** - external authentication integration
- **CGI & application servers** - CGI, FastCGI, SCGI, and other gateway interfaces configuration
- **Content processing** - response body modification and filtering
- **Rate limiting** - request rate limiting and throttling
- **Logging** - access and error logging configuration

## Global-only directives

### TLS/SSL & security

- `tls_cipher_suite <tls_cipher_suite: string> [<tls_cipher_suite_2: string> ...]`
  - This directive specifies the supported TLS cipher suites. This directive can be specified multiple times. Default: default TLS cipher suite for Rustls
- `tls_ecdh_curves <ecdh_curve: string> [<ecdh_curve: string> ...]`
  - This directive specifies the supported TLS ECDH curves. This directive can be specified multiple times. Default: default ECDH curves for Rustls
- `tls_client_certificate [enable_tls_client_certificate: bool]`
  - This directive specifies whenever the TLS client certificate verification is enabled. Default: `tls_client_certificate #false`
- `ocsp_stapling [enable_ocsp_stapling: bool]`
  - This directive specifies whenever OCSP stapling is enabled. Default: `ocsp_stapling #true`
- `block <blocked_ip: string> [<blocked_ip: string> ...]`
  - This directive specifies IP addresses to be blocked. This directive can be specified multiple times. Default: none
- `allow <allowed_ip: string> [<allowed_ip: string> ...]` (Ferron 2.0.0-beta.9 or newer)
  - This directive specifies IP addresses to be allowed. This directive can be specified multiple times. Default: none
- `auto_tls_on_demand_ask <auto_tls_on_demand_ask_url: string|null>` (Ferron 2.0.0-beta.13 or newer)
  - This directive specifies the URL to be used for asking whenever to the hostname for automatic TLS on demand is allowed. The server will append the `domain` query parameter with the domain name for the certificate to issue as a value to the URL. It's recommended to configure this option when using automatic TLS on demand to prevent abuse. Default: `auto_tls_on_demand_ask #null`
- `auto_tls_on_demand_ask_no_verification [auto_tls_on_demand_ask_no_verification: bool]` (Ferron 2.0.0-beta.13 or newer)
  - This directive specifies whenever the server should not verify the TLS certificate of the automatic TLS on demand asking endpoint. Default: `auto_tls_on_demand_ask_no_verification #false`

**Configuration example:**

```kdl
* {
    tls_cipher_suite "TLS_AES_256_GCM_SHA384" "TLS_AES_128_GCM_SHA256"
    tls_ecdh_curves "secp256r1" "secp384r1"
    tls_client_certificate #false
    ocsp_stapling
    block "192.168.1.100" "10.0.0.5"
    allow "192.168.1.0/24" "10.0.0.0/8"
    auto_tls_on_demand_ask "https://auth.example.com/check"
    auto_tls_on_demand_ask_no_verification #false
}
```

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
  - This directive specifies whenever the CONNECT protocol in HTTP/2 is enabled. Default: Hyper defaults
- `protocol_proxy [enable_proxy_protocol: bool]` (Ferron 2.0.0-beta.10 or newer)
  - This directive specifies whenever the PROXY protocol acceptation is enabled. If enabled, the server will expect the PROXY protocol header at the beginning of each connection. Default: `protocol_proxy #false`
- `buffer_request <request_buffer_size: integer|null>` (Ferron 2.0.0-beta.14 or newer)
  - This directive specifies the buffer size in bytes for incoming requests. If set as `buffer_request #null`, the request buffer is disabled. The request buffer can serve as an additional protection for underlying backend servers against Slowloris-style attacks. Default: `buffer_request #null`
- `buffer_response <response_buffer_size: integer|null>` (Ferron 2.0.0-beta.14 or newer)
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

### Caching

- `cache_max_entries <cache_max_entries: integer|null>` (_cache_ module)
  - This directive specifies the maximum number of entries that can be stored in the HTTP cache. If set as `cache_max_entries #null`, the cache can theoretically store an unlimited number of entries. The cache keys for entries depend on the request method, the rewritten request URL, the "Host" header value, and varying request headers. Default: `cache_max_entries 1024`

**Configuration example:**

```kdl
* {
    cache_max_entries 2048
}
```

### Load balancing

- `lb_health_check_window <lb_health_check_window: integer>` (_rproxy_ module)
  - This directive specifies the window size (in milliseconds) for load balancer health checks. Default: `lb_health_check_window 5000`

**Configuration example:**

```kdl
* {
    lb_health_check_window 5000
}
```

### Networking & system

- `listen_ip <listen_ip: string>`
  - This directive specifies the IP address to listen. Default: `listen_ip "::1"`
- `io_uring [enable_io_uring: bool]`
  - This directive specifies whenever `io_uring` is enabled. This directive has no effect for systems that don't support `io_uring` and for web server builds that use Tokio instead of Monoio. Default: `io_uring #true`
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

## Global and virtual host directives

### TLS/SSL & security

- `tls <certificate_path: string> <private_key_path: string>`
  - This directive specifies the path to the TLS certificate and private key. Default: none
- `auto_tls [enable_automatic_tls: bool]`
  - This directive specifies whenever automatic TLS is enabled. Default: `auto_tls #true` when port isn't explicitly specified and if the hostname doesn't look like a local address (`127.0.0.1`, `::1`, `localhost`), otherwise `auto_tls #false`
- `auto_tls_contact <auto_tls_contact: string|null>`
  - This directive specifies the email address used to register an ACME account for automatic TLS. Default: `auto_tls_contact #null`
- `auto_tls_cache <auto_tls_cache: string|null>`
  - This directive specifies the directory to store cached ACME data, such as cached account data and certifies. Default: OS-specific directory, for example on GNU/Linux it can be `/home/user/.local/share/ferron-acme` for the "user" user, on macOS it can be `/Users/user/Library/Application Support/ferron-acme` for the "user" user, on Windows it can be `C:\Users\user\AppData\Local\ferron-acme` for the "user" user. On Docker, it would be `/var/lib/ferron-acme`.
- `auto_tls_letsencrypt_production [enable_auto_tls_letsencrypt_production: bool]`
  - This directive specifies whenever the production Let's Encrypt ACME endpoint is used. If set as `auto_tls_letsencrypt_production #false`, the staging Let's Encrypt ACME endpoint is used. Default: `auto_tls_letsencrypt_production #true`
- `auto_tls_challenge <acme_challenge_type: string> [provider=<acme_challenge_provider: string>] [...]`
  - This directive specifies the used ACME challenge type. The supported types are `"http-01"` (HTTP-01 ACME challenge), `"tls-alpn-01"` (TLS-ALPN-01 ACME challenge) and `"dns-01"` (DNS-01 ACME challenge; Ferron 2.0.0-beta.9 or newer). The `provider` prop defines the DNS provider to use for DNS-01 challenges. Additional props can be passed as parameters for the DNS provider, see automatic TLS documentation. Default: `auto_tls_challenge "tls-alpn-01"`
- `auto_tls_directory <auto_tls_directory: string>` (Ferron 2.0.0-beta.3 or newer)
  - This directive specifies the ACME directory URL from which the certificates are obtained. Overrides `auto_tls_letsencrypt_production` directive. Default: none
- `auto_tls_no_verification [auto_tls_no_verification: bool]` (Ferron 2.0.0-beta.3 or newer)
  - This directive specifies whenever to disable the certificate verification of the ACME server. Default: `auto_tls_no_verification #false`
- `auto_tls_profile <auto_tls_profile: string|null>` (Ferron 2.0.0-beta.9 or newer)
  - This directive specifies the ACME profile to use for the certificates. Default: `auto_tls_profile #null`
- `auto_tls_on_demand <auto_tls_on_demand: bool>` (Ferron 2.0.0-beta.13 or newer)
  - This directive specifies whenever to enable the automatic TLS on demand. The functionality obtains TLS certificates automatically when a website is accessed for the first time. It's recommended to use either HTTP-01 or TLS-ALPN-01 ACME challenges, as DNS-01 ACME challenges might be slower due to DNS propagation delays. It's also recommended to configure the `auto_tls_on_demand_ask` directive alongside this directive. Default: `auto_tls_on_demand #false`
- `auto_tls_eab (<auto_tls_eab_key_id: string> <auto_tls_eab_key_hmac: string>)|<auto_tls_eab_disabled: null>` (Ferron 2.0.0-beta.15 or newer)
  - This directive specifies the EAB key ID and HMAC for the ACME External Account Binding. The HMAC key value is encoded in a URL-safe Base64 encoding. If set as `auto_tls_eab_disabled #null`, the EAB is disabled. Default: `auto_tls_eab_disabled #null`

**Configuration example:**

```kdl
example.com {
    auto_tls
    auto_tls_contact "admin@example.com"
    auto_tls_cache "/var/cache/ferron-acme"
    auto_tls_letsencrypt_production
    auto_tls_challenge "tls-alpn-01"
    auto_tls_profile "default"
    auto_tls_on_demand #false
    auto_tls_eab #null
}

manual-tls.example.com {
    tls "/etc/ssl/certs/example.com.crt" "/etc/ssl/private/example.com.key"
}
```

### Logging

- `log <log_file_path: string>`
  - This directive specifies the path to the access log file, which contains the HTTP response logs in Combined Log Format. This directive was global-only until Ferron 2.0.0-beta.3. Default: none
- `error_log <error_log_file_path: string>`
  - This directive specifies the path to the error log file. This directive was global-only until Ferron 2.0.0-beta.3. Default: none

**Configuration example:**

```kdl
example.com {
    log "/var/log/ferron/example.com.access.log"
    error_log "/var/log/ferron/example.com.error.log"
}
```

## Directives

### Headers & response customization

- `header <header_name: string> <header_value: string>`
  - This directive specifies a header to be added to HTTP responses. The header values supports placeholders like `{path}` which will be replaced with the request path. This directive can be specified multiple times. Default: none
- `server_administrator_email <server_administrator_email: string>`
  - This directive specifies the server administrator's email address to be used in the default 500 Internal Server Error page. Default: none
- `error_page <status_code: integer> <path: string>`
  - This directive specifies a custom error page to be served by the web server. Default: none
- `header_remove <header_name: string>` (Ferron 2.0.0-beta.5 or newer)
  - This directive specifies a header to be removed from HTTP responses. This directive can be specified multiple times. Default: none
- `header_replace <header_name: string> <header_value: string>` (Ferron 2.0.0-beta.9 or newer)
  - This directive specifies a header to be added to HTTP responses, potentially replacing existing headers. The header values supports placeholders like `{path}` which will be replaced with the request path. This directive can be specified multiple times. Default: none

**Configuration example:**

```kdl
example.com {
    header "X-Frame-Options" "DENY"
    header "X-Content-Type-Options" "nosniff"
    header "X-XSS-Protection" "1; mode=block"
    header "Strict-Transport-Security" "max-age=31536000; includeSubDomains"
    header "X-Custom-Header" "Custom value with {path} placeholder"

    header_remove "X-Header-To-Remove"
    header_replace "X-Powered-By" "Ferron"

    server_administrator_email "admin@example.com"
    error_page 404 "/var/www/errors/404.html"
    error_page 500 "/var/www/errors/500.html"
}
```

### Security & access control

- `trust_x_forwarded_for [trust_x_forwarded_for: bool]`
  - This directive specifies whenever to trust the value of the `X-Forwarded-For` header. It's recommended to configure this directive if behind a reverse proxy. Default: `trust_x_forwarded_for #false`
- `status <status_code: integer> [url=<url: string>|regex=<regex: string>] [location=<location: string>] [realm=<realm: string>] [brute_protection=<enable_brute_protection: bool>] [users=<users: string>] [allowed=<allowed: string>] [not_allowed=<not_allowed: string>] [body=<response_body: string>]`
  - This directive specifies the custom status code. This directive can be specified multiple times. The `url` prop specifies the request path for this status code. The `regex` prop specifies the regular expression (like `^/ferron(?:$|[/#?])`) for the custom status code. The `location` prop specifies the destination for the redirect; it supports placeholders (on Ferron 2.0.0-beta.15 and newer) like `{path}` which will be replaced with the request path. The `realm` prop specifies the HTTP basic authentication realm. The `brute_protection` prop specifies whenever the brute-force protection is enabled. The `users` prop is a comma-separated list of allowed users for HTTP authentication. The `allowed` prop is a comma-separated list of IP addresses applicable for the status code. The `not_allowed` prop is a comma-separated list of IP addresses not applicable for the status code. The `body` prop (Ferron 2.0.0-beta.5 or newer) specifies the response body to be sent. Default: none
- `user [username: string] [password_hash: string]`
  - This directive specifies an user with a password hash used for the HTTP basic authentication (it can be either Argon2, PBKDF2, or `scrypt` one). It's recommended to use the `ferron-passwd` tool to generate the password hash. This directive can be specified multiple times. Default: none

**Configuration example:**

```kdl
example.com {
    trust_x_forwarded_for

    // Basic authentication with custom status codes
    status 401 url="/admin" realm="Admin Area" users="admin,moderator"
    status 403 url="/restricted" allowed="192.168.1.0/24" body="Access denied"
    status 301 url="/old-page" location="/new-page"

    // User definitions for authentication (use `ferron-passwd` to generate password hashes)
    users "admin" "$2b$10$hashedpassword12345"
    users "moderator" "$2b$10$anotherhashedpassword"
}
```

### URL processing & routing

- `allow_double_slashes [allow_double_slashes: bool]`
  - This directive specifies whenever double slashes are allowed in the URL. Default: `allow_double_slashes #false`
- `no_redirect_to_https [no_redirect_to_https: bool]`
  - This directive specifies whenever not to redirect from HTTP URL to HTTPS URL. This directive is always effectively set to `no_redirect_to_https` when the server port is explicitly specified in the configuration. Default: `no_redirect_to_https #false`
- `wwwredirect [enable_wwwredirect: bool]`
  - This directive specifies whenever to redirect from URL without "www." to URL with "www.". Default: `wwwredirect #false`
- `rewrite <regex: string> <replacement: string> [directory=<directory: bool>] [file=<file: bool>] [last=<last: bool>] [allow_double_slashes=<allow_double_slashes: bool>]`
  - This directive specifies the URL rewriting rule. This directive can be specified multiple times. The first value is a regular expression (like `^/ferron(?:$|[/#?])`). The `directory` prop specifies whenever the rewrite rule is applied when the path would correspond to directory (if `#false`, then it's not applied). The `file` prop specifies whenever the rewrite rule is applied when the path would correspond to file (if `#false`, then it's not applied). The `last` prop specifies whenever the rewrite rule is the last rule applied. The `allow_double_slashes` prop specifies whenever the rewrite rule allows double slashes in the request URL. Default: none
- `rewrite_log [rewrite_log: bool]`
  - This directive specifies whenever URL rewriting operations are logged into the error log. Default: `rewrite_log #false`
- `no_trailing_redirect [no_trailing_redirect: bool]`
  - This directive specifies whenerver not to redirect the URL without a trailing slash to one with a trailing slash, if it refers to a directory. Default: `no_trailing_redirect #false`

**Configuration example:**

```kdl
example.com {
    allow_double_slashes #false
    no_redirect_to_https #false
    wwwredirect #false

    // URL rewriting examples
    rewrite "^/old-path/(.*)" "/new-path/$1" last=#true
    rewrite "^/api/v1/(.*)" "/api/v2/$1" file=#false directory=#false
    rewrite "^/blog/([^/]+)/?(?:$|[?#])" "/blog.php?slug=$1" last=#true

    rewrite_log
    no_trailing_redirect #false
}
```

### Static file serving

- `root <webroot: string|null>`
  - This directive specifies the webroot from which static files are served. If set as `root #null`, the static file serving functionality is disabled. Default: none
- `etag [enable_etag: bool]` (_static_ module)
  - This directive specifies whenever the ETag header is enabled. Default: `etag #true`
- `compressed [enable_compression: bool]` (_static_ module)
  - This directive specifies whenever the HTTP compression for static files is enabled. Default: `compressed #true`
- `directory_listing [enable_directory_listing: bool]` (_static_ module)
  - This directive specifies whenever the directory listings are enabled. Default: `directory_listing #false`
- `precompressed [enable_precompression: bool]` (_static_ module; Ferron 2.0.0-beta.18 or newer)
  - This directive specifies whenever serving the precompressed static files is enabled. The precompressed static files would additionally have `.gz` extension for gzip, `.deflate` for Deflate, `.br` for Brotli, or `.zst` for Zstandard. Default: `precompressed #false`

**Configuration example:**

```kdl
example.com {
    root "/var/www/example.com"
    etag
    compressed
    directory_listing #false

    // Set "Cache-Control" header for static files
    file_cache_control "public, max-age=3600"
}
```

### Caching

- `cache [enable_cache: bool]` (_cache_ module)
  - This directive specifies whenever the HTTP cache is enabled. Default: `cache #false`
- `cache_max_response_size <cache_max_response_size: integer|null>` (_cache_ module)
  - This directive specifies the maximum size of the response (in bytes) that can be stored in the HTTP cache. If set as `cache_max_response_size #null`, the cache can theoretically store responses of any size. Default: `cache_max_response_size 2097152`
- `cache_vary <varying_request_header: string> [<varying_request_header: string> ...]` (_cache_ module)
  - This directive specifies the request headers that are used to vary the cache entries. This directive can be specified multiple times. Default: none
- `cache_ignore <ignored_response_header: string> [<ignored_response_header: string> ...]` (_cache_ module)
  - This directive specifies the response headers that are ignored when caching the response. This directive can be specified multiple times. Default: none
- `file_cache_control <cache_control: string|null>` (_static_ module; Ferron 2.0.0-beta.9 or newer)
  - This directive specifies the Cache-Control header value for static files. If set as `file_cache_control #null`, the Cache-Control header is not set. Default: `file_cache_control #null`

**Configuration example:**

```kdl
example.com {
    cache
    cache_max_response_size 2097152
    cache_vary "Accept-Encoding" "Accept-Language"
    cache_ignore "Set-Cookie" "Cache-Control"
}
```

### Reverse proxy & load balancing

- `proxy <proxy_to: string|null> [unix=<unix_socket_path: string>]` (_rproxy_ module)
  - This directive specifies the URL to which the reverse proxy should forward requests. HTTP (for example `http://localhost:3000/`) and HTTPS URLs (for example `https://localhost:3000/`) are supported. Unix sockets are also supported (Ferron 2.0.0-beta.19 and newer) via the `unix` prop set to the path to the socket, supported only on Unix and Unix-like systems. This directive can be specified multiple times. Default: none
- `lb_health_check [enable_lb_health_check: bool]` (_rproxy_ module)
  - This directive specifies whenever the load balancer passive health check is enabled. Default: `lb_health_check #false`
- `lb_health_check_max_fails <max_fails: integer>` (_rproxy_ module)
  - This directive specifies the maximum number of consecutive failures before the load balancer marks a backend as unhealthy. Default: `lb_health_check_max_fails 3`
- `proxy_no_verification [proxy_no_verification: bool]` (_rproxy_ module)
  - This directive specifies whenever the reverse proxy should not verify the TLS certificate of the backend. Default: `proxy_no_verification #false`
- `proxy_intercept_errors [proxy_intercept_errors: bool]` (_rproxy_ module)
  - This directive specifies whenever the reverse proxy should intercept errors from the backend. Default: `proxy_intercept_errors #false`
- `proxy_request_header <header_name: string> <header_value: string>` (_rproxy_ module; Ferron 2.0.0-beta.5 or newer)
  - This directive specifies a header to be added to HTTP requests sent by the reverse proxy. The header values supports placeholders (on Ferron 2.0.0-beta.9 and newer) like `{path}` which will be replaced with the request path. This directive can be specified multiple times. Default: none
- `proxy_request_header_remove <header_name: string>` (_rproxy_ module; Ferron 2.0.0-beta.5 or newer)
  - This directive specifies a header to be removed from HTTP requests sent by the reverse proxy. This directive can be specified multiple times. Default: none
- `proxy_keepalive [proxy_keepalive: bool]` (_rproxy_ module; Ferron 2.0.0-beta.5 or newer)
  - This directive specifies whenever the reverse proxy should keep the connection to the backend alive. Default: `proxy_keepalive #true`
- `proxy_request_header_replace <header_name: string> <header_value: string>` (_rproxy_ module; Ferron 2.0.0-beta.9 or newer)
  - This directive specifies a header to be added to HTTP requests sent by the reverse proxy, potentially replacing existing headers. The header values supports placeholders (on Ferron 2.0.0-beta.9 and newer) like `{path}` which will be replaced with the request path. This directive can be specified multiple times. Default: none
- `proxy_http2 [enable_proxy_http2: bool]` (_rproxy_ module; Ferron 2.0.0-beta.13 or newer)
  - This directive specifies whenever the reverse proxy can use HTTP/2 protocol when connecting to backend servers. Default: `proxy_http2 #false`
- `lb_retry_connection [enable_lb_retry_connection: bool]` (_rproxy_ module; Ferron 2.0.0-beta.15 or newer)
  - This directive specifies whenever the load balancer should retry connections to another backend server, in case of TCP connection or TLS handshake failure. Default: `lb_retry_connection #true`

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
  - This directive specifies whenever the forward proxy functionality is enabled. Default: `forward_proxy #false`

**Configuration example:**

```kdl
* {
    forward_proxy
}
```

### Authentication forwarding

- `auth_to <auth_to: string|null>` (_fauth_ module)
  - This directive specifies the URL to which the web server should send requests for forwarded authentication. Default: none
- `auth_to_no_verification [auth_to_no_verification: bool]` (_fauth_ module)
  - This directive specifies whenever the server should not verify the TLS certificate of the backend authentication server. Default: `auth_to_no_verification #false`
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

### CGI & application servers

- `cgi [enable_cgi: bool]` (_cgi_ module)
  - This directive specifies whenever the CGI handler is enabled. Default: `cgi #false`
- `cgi_extension <cgi_extension: string|null>` (_cgi_ module)
  - This directive specifies CGI script extensions, which will be handled via the CGI handler outside the `cgi-bin` directory. This directive can be specified multiple times. Default: none
- `cgi_interpreter <cgi_extension: string> <cgi_interpreter: string|null> [<cgi_interpreter_argument: string> ...]` (_cgi_ module)
  - This directive specifies CGI script interpreters used by the CGI handler. If CGI interpreter is set to `#null`, the default interpreter settings will be disabled. This directive can be specified multiple times. Default: specified for `.pl`, `.py`, `.sh`, `.ksh`, `.csh`, `.rb` and `.php` extensions, and additionally `.exe`, `.bat` and `.vbs` extensions for Windows
- `cgi_environment <environment_variable_name: string> <environment_variable_value: string>` (_cgi_ module)
  - This directive specifies an environment variable passed into CGI applications. Default: none
- `scgi <scgi_to: string|null>` (_scgi_ module)
  - This directive specifies whenever SCGI is enabled and the base URL to which the SCGI client will send requests. TCP (for example `tcp://localhost:4000/`) and Unix socket URLs (only on Unix systems; for example `unix:///run/scgi.sock`) are supported. Default: `scgi #null`
- `scgi_environment <environment_variable_name: string> <environment_variable_value: string>` (_scgi_ module)
  - This directive specifies an environment variable passed into SCGI server. Default: none
- `fcgi <fcgi_to: string|null> [pass=<fcgi_pass: bool>]` (_fcgi_ module)
  - This directive specifies whenever FastCGI is enabled and the base URL to which the FastCGI client will send requests. The `pass` prop specified whenever to pass the all the requests to the FastCGI request handler. TCP (for example `tcp://localhost:4000/`) and Unix socket URLs (only on Unix systems; for example `unix:///run/scgi.sock`) are supported. Default: `fcgi #null pass=#true`
- `fcgi_php <fcgi_php_to: string|null>` (_fcgi_ module)
  - This directive specifies whenever PHP through FastCGI is enabled and the base URL to which the FastCGI client will send requests for ".php" files. TCP (for example `tcp://localhost:4000/`) and Unix socket URLs (only on Unix systems; for example `unix:///run/scgi.sock`) are supported. Default: `fcgi_php #null`
- `fcgi_extension <fcgi_extension: string|null>` (_fcgi_ module)
  - This directive specifies file extensions, which will be handled via the FastCGI handle. This directive can be specified multiple times. Default: none
- `fcgi_environment <environment_variable_name: string> <environment_variable_value: string>` (_fcgi_ module)
  - This directive specifies an environment variable passed into FastCGI server. Default: none

**Configuration example:**

```kdl
cgi.example.com {
    // CGI configuration
    cgi
    cgi_extension ".cgi" ".pl" ".py"
    cgi_interpreter ".py" "/usr/bin/python3"
    cgi_interpreter ".pl" "/usr/bin/perl"
    cgi_environment "PATH" "/usr/bin:/bin"
    cgi_environment "SCRIPT_ROOT" "/var/www/cgi-bin"
}

scgi.example.com {
    // SCGI configuration
    scgi "tcp://localhost:4000/"
    scgi_environment "SCRIPT_NAME" "/app"
    scgi_environment "SERVER_NAME" "example.com"
}

fastcgi.example.com {
    // FastCGI configuration
    fcgi "tcp://localhost:9000/" pass=#true
    fcgi_php "tcp://localhost:9000/"
    fcgi_extension ".php" ".php5"
    fcgi_environment "SCRIPT_FILENAME" "/var/www/example.com{path}"
    fcgi_environment "DOCUMENT_ROOT" "/var/www/example.com"
}
```

### Content processing

- `replace <searched_string: string> <replaced_string: string> [once=<replace_once: bool>]` (_replace_ module; Ferron 2.0.0-beta.2 or newer)
  - This directive specifies the string to be replaced in a response body, and a replacement string. The `once` prop specifies whenever the string will be replaced once, by default this prop is set to `#true`. Default: none
- `replace_last_modified [preserve_last_modified: bool]` (_replace_ module; Ferron 2.0.0-beta.2 or newer)
  - This directive specifies whenever to preserve the "Last-Modified" header in the response. Default: `replace_last_modified #false`
- `replace_filter_types <filter_type: string> [<filter_type: string> ...]` (_replace_ module; Ferron 2.0.0-beta.2 or newer)
  - This directive specifies the response MIME type filters. The filter can be either a specific MIME type (like `text/html`) or a wildcard (`*`) specifying that responses with all MIME types are processed for replacement. This directive can be specified multiple times. Default: `replace_filter_types "text/html"`

**Configuration example:**

```kdl
example.com {
    // Disabling HTTP compression is required for string replacement
    compressed #false

    // String replacement in response bodies (works with HTTP compression disabled)
    replace "old-company-name" "new-company-name" once=#false
    replace "http://old-domain.com" "https://new-domain.com" once=#true

    replace_last_modified
    replace_filter_types "text/html" "text/css" "application/javascript"
}
```

### Rate limiting

- `limit [enable_limit: bool] [rate=<rate: integer|float>] [burst=<rate: integer|float>]` (_limit_ module; Ferron 2.0.0-beta.2 or newer)
  - This directive specifies whenever the rate limiting is enabled. The `rate` prop specifies the maximum average amount of requests per second, defaults to 25 requests per second. The `burst` prop specifies the maximum peak amount of requests per second, defaults to 4 times the maximum average amount of requests per second. Default: `limit #false`

**Configuration example:**

```kdl
example.com {
    // Global rate limiting
    limit rate=100 burst=200

    // Different rate limits for different paths
    location "/api" {
        limit rate=10 burst=20
    }

    location "/login" {
        limit rate=5 burst=10
    }
}
```

### Logging

- `log_date_format <log_date_format: string>` (Ferron 2.0.0-beta.19 or newer)
  - This directive specifies the date format (according to POSIX) for the access log file. Default: `"%d/%b/%Y:%H:%M:%S %z"`
- `log_format <log_format: string>` (Ferron 2.0.0-beta.19 or newer)
  - This directive specifies the entry format for the access log file. The placeholders can be found in the reference below the section specifying. Default: `"{server_ip} - {auth_user} [{timestamp}] \"{method} {path_and_query} {version}\" {status_code} {content_length} \"{header:Referer}\" \"{header:User-Agent}\""` (Combined Log Format)

**Configuration example:**

```kdl
* {
    log_date_format "%d/%b/%Y:%H:%M:%S %z"
    log_format "{server_ip} - {auth_user} [{timestamp}] \"{method} {path_and_query} {version}\" {status_code} {content_length} \"{header:Referer}\" \"{header:User-Agent}\""
}
```

## Subconditions

Ferron 2.0.0-beta.15 and newer supports conditional configuration based on conditions. This allows you to configure different settings based on the request method, path, or other conditions.

Below is the list of supported subconditions:

- `is_remote_ip <remote_ip: string> [<remote_ip: string> ...]` (Ferron 2.0.0-beta.15 or newer)
  - This subcondition checks if the request is coming from a specific remote IP address or a list of IP addresses.
- `is_forwarded_for <remote_ip: string> [<remote_ip: string> ...]` (Ferron 2.0.0-beta.15 or newer)
  - This subcondition checks if the request (with respect for `X-Forwarded-For` header) is coming from a specific forwarded IP address or a list of IP addresses.
- `is_not_remote_ip <remote_ip: string> [<remote_ip: string> ...]` (Ferron 2.0.0-beta.15 or newer)
  - This subcondition checks if the request is not coming from a specific remote IP address or a list of IP addresses.
- `is_not_forwarded_for <remote_ip: string> [<remote_ip: string> ...]` (Ferron 2.0.0-beta.15 or newer)
  - This subcondition checks if the request (with respect for `X-Forwarded-For` header) is not coming from a specific forwarded IP address or a list of IP addresses.
- `is_equal <left_side: string> <right_side: string>` (Ferron 2.0.0-beta.15 or newer)
  - This subcondition checks if the left side is equal to the right side.
- `is_not_equal <left_side: string> <right_side: string>` (Ferron 2.0.0-beta.15 or newer)
  - This subcondition checks if the left side is not equal to the right side.
- `is_regex <value: string> <regex: string> [case_insensitive=<case_insensitive: bool>]` (Ferron 2.0.0-beta.15 or newer)
  - This subcondition checks if the value matches the regular expression. The `case_insensitive` prop specifies whether the regex should be case insensitive (`#false` by default).
- `is_not_regex <value: string> <regex: string> [case_insensitive=<case_insensitive: bool>]` (Ferron 2.0.0-beta.15 or newer)
  - This subcondition checks if the value does not match the regular expression. The `case_insensitive` prop specifies whether the regex should be case insensitive (`#false` by default).

## Placeholders

Ferron supports the following placeholders for header values, subconditions, reverse proxying, and redirect destinations:

- `{path}` - the request URI with path (for example, `/index.html`)
  <<<<<<< HEAD
  =======
- `{path_and_query}` (Ferron 2.0.0-beta.19 or newer) - the request URI with path and query string (for example, `/index.html?param=value`)
  > > > > > > > develop-2.x
- `{method}` (Ferron 2.0.0-beta.9 or newer) - the request method
- `{version}` (Ferron 2.0.0-beta.9 or newer) - the HTTP version of the request
- `{header:<header_name>}` (Ferron 2.0.0-beta.9 or newer) - the header value of the request URI
- `{scheme}` (Ferron 2.0.0-beta.9 or newer) - the scheme of the request URI (`http` or `https`), applicable only for subconditions, reverse proxying and redirect destinations.
- `{client_ip}` (Ferron 2.0.0-beta.9 or newer) - the client IP address, applicable only for subconditions, reverse proxying and redirect destinations.
- `{client_port}` (Ferron 2.0.0-beta.9 or newer) - the client port number, applicable only for subconditions, reverse proxying and redirect destinations.
- `{server_ip}` (Ferron 2.0.0-beta.9 or newer) - the server IP address, applicable only for subconditions, reverse proxying and redirect destinations.
- `{server_port}` (Ferron 2.0.0-beta.9 or newer) - the server port number, applicable only for subconditions, reverse proxying and redirect destinations.

## Log placeholders

Ferron 2.0.0-beta.19 and newer supports the following placeholders for access logs:

- `{path}` - the request URI with path (for example, `/index.html`)
- `{path_and_query}` - the request URI with path and query string (for example, `/index.html?param=value`)
- `{method}` - the request method
- `{version}` - the HTTP version of the request
- `{header:<header_name>}` - the header value of the request URI (`-`, if header is missing)
- `{scheme}` - the scheme of the request URI (`http` or `https`).
- `{client_ip}` - the client IP address.
- `{client_port}` - the client port number.
- `{server_ip}` - the server IP address.
- `{server_port}` - the server port number.
- `{auth_user}` - the username of the authenticated user (`-`, if not authenticated)
- `{timestamp}` - the formatted timestamp of the entry
- `{status_code}` - the HTTP status code of the response
- `{content_length}` - the content length of the response (`-`, if not available)

## Location block example

Below is an example of Ferron configuration involving location blocks:

```kdl
example.com {
    root "/var/www/example.com"

    // Static assets with different settings
    location "/static" remove_base=#true {
        root "/var/www/static"
        compressed
        file_cache_control "public, max-age=31536000"
    }

    // API endpoints with proxy
    location "/api" {
        proxy "http://backend:8080"
        proxy_request_header "X-Forwarded-For" "{client_ip}"
        limit rate=50 burst=100
    }

    // Admin area with authentication
    location "/admin" {
        status 401 realm="Admin Access" users="admin"
        root "/var/www/admin"
    }

    // PHP files
    fcgi_php "tcp://localhost:9000/"
}
```

## Complete example combining multiple sections

Below is a complete example of Ferron configuration, combining multiple sections:

```kdl
// Global configuration
* {
    // Protocol and performance settings
    protocols "h1" "h2"
    h2_initial_window_size 65536
    h2_max_concurrent_streams 100
    timeout 300000

    // Network settings
    listen_ip "0.0.0.0"
    default_http_port 80
    default_https_port 443

    // Security defaults
    tls_cipher_suite "TLS_AES_256_GCM_SHA384" "TLS_AES_128_GCM_SHA256"
    ocsp_stapling
    block "192.168.1.100"

    // Global caching
    cache_max_entries 1024

    // Load balancing settings
    lb_health_check_window 5000

    // Logging
    log "/var/log/ferron/access.log"
    error_log "/var/log/ferron/error.log"

    // Static file defaults
    compressed
    etag
}

// Define reusable snippets
snippet "security_headers" {
    // Common security headers
    header "X-Frame-Options" "DENY"
    header "X-Content-Type-Options" "nosniff"
    header "X-XSS-Protection" "1; mode=block"
    header "Referrer-Policy" "strict-origin-when-cross-origin"
}

snippet "cors_headers" {
    // CORS headers for API endpoints
    header "Access-Control-Allow-Origin" "*"
    header "Access-Control-Allow-Methods" "GET, POST, PUT, DELETE, OPTIONS"
    header "Access-Control-Allow-Headers" "Authorization, Content-Type, X-Requested-With"
    header "Access-Control-Max-Age" "86400"
}

snippet "static_caching" {
    // Aggressive caching for static assets
    file_cache_control "public, max-age=31536000, immutable"
    compressed
    etag
}

snippet "admin_protection" {
    // Admin area protection
    status 401 realm="Admin Area" users="admin,superuser"
    users "admin" "$2b$10$hashedpassword12345"
    users "superuser" "$2b$10$anotherhashpassword67890"
    limit rate=10 burst=20
}

snippet "mobile_condition" {
    condition "is_mobile" {
        is_regex "{header:User-Agent}" "(Mobile|Android|iPhone|iPad)" case_insensitive=#true
    }
}

snippet "admin_ip_condition" {
    condition "is_admin_ip" {
        is_remote_ip "192.168.1.10" "10.0.0.5"
    }
}

snippet "api_request_condition" {
    condition "is_api_request" {
        is_regex "{path}" "^/api/"
    }
}

snippet "static_asset_condition" {
    condition "is_static_asset" {
        is_regex "{path}" "\\.(css|js|png|jpg|jpeg|gif|svg|woff|woff2|ttf|eot|ico)(?:$|[?#])" case_insensitive=#true
    }
}

snippet "development_condition" {
    condition "is_development" {
        is_equal "{header:X-Environment}" "development"
    }
}

// Main website with conditional configuration
example.com {
    // TLS configuration
    tls "/etc/ssl/certs/example.com.crt" "/etc/ssl/private/example.com.key"
    auto_tls_contact "admin@example.com"

    // Basic settings
    root "/var/www/example.com"
    server_administrator_email "admin@example.com"

    // Use security headers snippet
    use "security_headers"

    // Import condition snippets
    use "mobile_condition"
    use "admin_ip_condition"
    use "api_request_condition"
    use "static_asset_condition"
    use "development_condition"

    // Additional security header
    header "Strict-Transport-Security" "max-age=31536000; includeSubDomains"

    // Conditional configuration based on mobile detection
    if "is_mobile" {
        // Add headers that aren't inherited
        use "security_headers"
        header "Strict-Transport-Security" "max-age=31536000; includeSubDomains"

        // Mobile-specific settings
        header "X-Mobile-Detected" "true"
        root "/var/www/example.com/mobile"

        // Lighter rate limiting for mobile
        limit rate=50 burst=100
    }

    if_not "is_mobile" {
        // Add headers that aren't inherited
        use "security_headers"
        header "Strict-Transport-Security" "max-age=31536000; includeSubDomains"

        // Desktop settings
        header "X-Mobile-Detected" "false"
        limit rate=100 burst=200
    }

    // Admin IP gets special treatment
    if "is_admin_ip" {
        // Add headers that aren't inherited
        use "security_headers"
        header "Strict-Transport-Security" "max-age=31536000; includeSubDomains"

        // No rate limiting for admin IPs
        limit #false

        // Additional debug headers
        header "X-Admin-Access" "true"
        header "X-Client-IP" "{client_ip}"

        // Enhanced logging for admin access
        log "/var/log/ferron/admin-access.log"
    }

    // Development environment conditional settings
    if "is_development" {
        // Add headers that aren't inherited
        use "security_headers"
        header "Strict-Transport-Security" "max-age=31536000; includeSubDomains"

        // Development-specific headers
        header "X-Environment" "development"
        header "X-Debug-Mode" "enabled"

        // Disable caching in development
        header "Cache-Control" "no-cache, no-store, must-revalidate"
        header "Pragma" "no-cache"
        header "Expires" "0"
    }

    if_not "is_development" {
        // Add headers that aren't inherited
        use "security_headers"
        header "Strict-Transport-Security" "max-age=31536000; includeSubDomains"

        // Production caching
        cache
        cache_vary "Accept-Encoding" "Accept-Language"
        file_cache_control "public, max-age=3600"
    }

    // URL rewriting
    rewrite "^/old-section/(.*)" "/new-section/$1" last=#true

    // Error pages
    error_page 404 "/var/www/errors/404.html"
    error_page 500 "/var/www/errors/500.html"

    // Static assets location with conditional caching
    location "/assets" remove_base=#true {
        root "/var/www/assets"

        if "is_static_asset" {
            // Add headers that aren't inherited
            use "security_headers"
            header "Strict-Transport-Security" "max-age=31536000; includeSubDomains"

            use "static_caching"
        }

        // CORS for web fonts
        if_not "is_static_asset" {
            // Add headers that aren't inherited
            use "security_headers"
            header "Strict-Transport-Security" "max-age=31536000; includeSubDomains"

            header "Access-Control-Allow-Origin" "*"
        }
    }

    // API endpoints
    location "/api" {
        if "is_api_request"
            // Add headers that aren't inherited
            use "security_headers"
            header "Strict-Transport-Security" "max-age=31536000; includeSubDomains"

            use "cors_headers"

            // API-specific rate limiting
            limit rate=1000 burst=2000

            // Proxy to API backend
            proxy "http://api-backend:8080"
            proxy_request_header_replace "X-Real-IP" "{client_ip}"
            proxy_request_header "X-Forwarded-Proto" "{scheme}"
        }
    }

    // Admin area with conditional access
    location "/admin" {
        if "is_admin_ip" {
            // Add headers that aren't inherited
            use "security_headers"
            header "Strict-Transport-Security" "max-age=31536000; includeSubDomains"

            // Admin IPs get direct access
            root "/var/www/admin"

            // Special admin headers
            header "X-Admin-Direct-Access" "true"
        }

        if_not "is_admin_ip" {
            // Add headers that aren't inherited
            use "security_headers"
            header "Strict-Transport-Security" "max-age=31536000; includeSubDomains"

            // Non-admin IPs require authentication
            use "admin_protection"
        }
    }

    // PHP application
    fcgi_php "tcp://localhost:9000/"
}

// API subdomain with extensive conditional logic
api.example.com {
    // TLS configuration
    tls "/etc/ssl/certs/api.example.com.crt" "/etc/ssl/private/api.example.com.key"

    // Use CORS headers snippet
    use "cors_headers"

    // Import reusable condition
    use "admin_ip_condition"

    // Conditional backend selection based on request path
    condition "is_v1_api" {
        is_regex "{path}" "^/v1/"
    }

    condition "is_v2_api" {
        is_regex "{path}" "^/v2/"
    }

    condition "is_auth_request" {
        is_regex "{path}" "^/(login|register|refresh|logout)"
    }

    // Version-specific backend routing
    if "is_v1_api" {
        // Use CORS headers snippet (again, since it's not inherited)
        use "cors_headers"

        proxy "http://api-v1-backend1:8080"
        proxy "http://api-v1-backend2:8080"

        // V1 specific headers
        header "X-API-Version" "1.0"

        // More restrictive rate limiting for legacy API
        limit rate=500 burst=1000
    }

    if "is_v2_api" {
        // Use CORS headers snippet (again, since it's not inherited)
        use "cors_headers"

        proxy "http://api-v2-backend1:8080"
        proxy "http://api-v2-backend2:8080"
        proxy "http://api-v2-backend3:8080"

        // V2 specific headers
        header "X-API-Version" "2.0"

        // Higher rate limits for new API
        limit rate=2000 burst=4000
    }

    // Authentication endpoints get special treatment
    if "is_auth_request" {
        // Use CORS headers snippet (again, since it's not inherited)
        use "cors_headers"

        // Route to dedicated auth service
        proxy "http://auth-service:9090"

        // Stricter rate limiting for auth endpoints
        limit rate=100 burst=200

        // Additional security headers
        header "X-Auth-Endpoint" "true"
        header "Strict-Transport-Security" "max-age=31536000; includeSubDomains; preload"
    }

    // Admin IP gets enhanced access
    if "is_admin_ip" {
        // Use CORS headers snippet (again, since it's not inherited)
        use "cors_headers"

        // No rate limiting for admin IPs
        limit #false

        // Admin-specific headers
        header "X-Admin-API-Access" "true"

        // Enhanced logging
        log "/var/log/ferron/api-admin.log"
    }

    // Health checking
    lb_health_check
    lb_health_check_max_fails 3

    // Proxy settings
    proxy_keepalive
    proxy_request_header_replace "X-Real-IP" "{client_ip}"
    proxy_request_header "X-Forwarded-Proto" "{scheme}"

    // Health check endpoint
    status 200 url="/health" body="OK"

    // Version info endpoint
    status 200 url="/version" body="{\"api_versions\":[\"v1\",\"v2\"],\"server\":\"Ferron\"}"
}

// Development subdomain with environment-based configuration
dev.example.com {
    // Basic TLS
    auto_tls
    auto_tls_contact "dev@example.com"

    // Use security headers but with relaxed settings
    use "security_headers"

    // Import reusable condition
    use "admin_ip_condition"

    // Override some security headers for development
    header "X-Frame-Options" "SAMEORIGIN"

    // Enhanced logging
    log "/var/log/ferron/dev.access.log"
    error_log "/var/log/ferron/dev.error.log"

    // Development-specific conditions
    condition "is_hot_reload" {
        is_regex "{path}" "^/(sockjs-node|__webpack_hmr)"
    }

    condition "is_source_map" {
        is_regex "{path}" "\\.map(?:$|[?#])"
    }

    // Hot reload support
    if "is_hot_reload" {
        // Non-inherited headers, again
        use "security_headers"
        header "X-Frame-Options" "SAMEORIGIN"

        proxy "http://dev-hmr:3001"

        // WebSocket support
        proxy_request_header "Connection" "Upgrade"
        proxy_request_header "Upgrade" "websocket"

        // No caching for hot reload
        header "Cache-Control" "no-cache"
    }

    // Source maps handling
    if "is_source_map" {
        // Only allow source maps for admin IPs
        if "is_admin_ip" {
            root "/var/www/dev/sourcemaps"
        }

        if_not "is_admin_ip" {
            status 404 body="Not found"
        }
    }

    // Test endpoints with conditional responses
    if "is_admin_ip" {
        status 200 url="/test" body="Development server is working (Admin Access)"
        status 200 url="/debug" body="{\"client_ip\":\"{client_ip}\",\"method\":\"{method}\",\"path\":\"{path}\"}"
    }

    if_not "is_admin_ip" {
        status 200 url="/test" body="Development server is working"
    }

    // Default proxy to development backend
    proxy "http://dev-backend:3000"
    proxy_request_header "X-Dev-Mode" "true"
    proxy_request_header "X-Environment" "development"

    // Relaxed rate limiting
    limit rate=1000 burst=5000
}

// Static content CDN with intelligent caching
cdn.example.com {
    // TLS configuration
    tls "/etc/ssl/certs/cdn.example.com.crt" "/etc/ssl/private/cdn.example.com.key"

    // Static file serving
    root "/var/www/cdn"
    directory_listing #false

    // Use static caching snippet
    use "static_caching"

    // Import reusable condition
    use "admin_ip_condition"

    // Conditional caching based on file type
    condition "is_image" {
        is_regex "{path}" "\\.(png|jpg|jpeg|gif|webp|svg)(?:$|[?#])" case_insensitive=#true
    }

    condition "is_font" {
        is_regex "{path}" "\\.(woff|woff2|ttf|eot|otf)(?:$|[?#])" case_insensitive=#true
    }

    condition "is_media" {
        is_regex "{path}" "\\.(mp4|webm|ogg|mp3|wav)(?:$|[?#])" case_insensitive=#true
    }

    // Image-specific settings
    if "is_image" {
        // Extra long caching for images
        file_cache_control "public, max-age=2592000, immutable"

        // Image-specific headers
        header "X-Content-Type" "image"
    }

    // Font-specific settings
    if "is_font" {
        // CORS for web fonts
        header "Access-Control-Allow-Origin" "*"
        header "Access-Control-Allow-Methods" "GET, HEAD, OPTIONS"

        // Font-specific caching
        file_cache_control "public, max-age=31536000, immutable"
    }

    // Media-specific settings
    if "is_media" {
        // Partial content support for media
        header "Accept-Ranges" "bytes"

        // Media-specific caching
        file_cache_control "public, max-age=604800"
    }

    // Admin IP gets special access
    if "is_admin_ip" {
        // Allow directory listing for admin IPs
        directory_listing #true

        // Admin headers
        header "X-Admin-CDN-Access" "true"
    }

    // No rate limiting for CDN
    limit #false

    // Geographic optimization (example condition)
    condition "is_european_request" {
        is_regex "{header:CF-IPCountry}" "^(DE|FR|GB|IT|ES|NL|BE|CH|AT|SE|NO|DK|FI|PL)$"
    }

    if "is_european_request" {
        header "X-CDN-Region" "Europe"
        // Could proxy to European CDN nodes here
    }

    if_not "is_european_request" {
        header "X-CDN-Region" "Global"
    }
}
```
