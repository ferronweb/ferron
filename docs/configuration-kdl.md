---
title: Server configuration
---

Ferron 2.0.0-beta.1 and newer can be configured in a [KDL-format](https://kdl.dev/) configuration file (often named `ferron.kdl`). Below are the descriptions of configuration properties for this server.

## Configuration blocks

At the top level of the server configration, the confguration blocks representing specific virtual host are specified. Below are the examples of such configuration blocks:

```kdl
* {
  // Global configuration
}

*:80 {
  // Configuration for port 80
}

example.com {
  // Configuration for "example.com" virtual host
}

192.168.1.1 {
  // Configuration for "192.168.1.1" IP virtual host
}

example.com:8080 {
  // Configuration for "example.com" virtual host with port 8080
}

192.168.1.1:8080 {
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

  // Below is the error handler configuration for 404 Not Found status code. If "404" wasn't included, it would be for all errors.
  error_config 404 {
    // ...
  }
}
```

Also, it's possible to include other configuration files using an `include <included_configuration_path: string>` directive, like this:

```kdl
include /etc/ferron.d/**/*.kdl
```

## Global-only directives

- `log <log_file_path: string>`
  - This directive specifies the path to the access log file, which contains the HTTP response logs in Combined Log Format. Default: none
- `error_log <error_log_file_path: string>`
  - This directive specifies the path to the error log file. Default: none
- `tls_cipher_suite <tls_cipher_suite: string> [<tls_cipher_suite_2: string> ...]`
  - This directive specifies the supported TLS cipher suites. This directive can be specified multiple times. Default: default TLS cipher suite for Rustls
- `tls_ecdh_curves <ecdh_curve: string> [<ecdh_curve: string> ...]`
  - This directive specifies the supported TLS ECDH curves. This directive can be specified multiple times. Default: default ECDH curves for Rustls
- `tls_client_certificate [enable_tls_client_certificate: bool]`
  - This directive specifies whenever the TLS client certificate verification is enabled. Default: `tls_client_certificate #false`
- `ocsp_stapling [enable_ocsp_stapling: bool]`
  - This directive specifies whenever OCSP stapling is enabled. Default: `ocsp_stapling #true`
- `default_http_port <default_http_port: integer|null>`
  - This directive specifies the default port for HTTP connections. If set as `default_http_port #null`, the implicit default HTTP port is disabled. Default: `default_http_port 80`
- `default_https_port <default_https_port: integer|null>`
  - This directive specifies the default port for HTTPS connections. If set as `default_https_port #null`, the implicit default HTTPS port is disabled. Default: `default_https_port 443`
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
- `protocols <protocol: string> [<protocol: string> ...]`
  - This directive specifies the enabled protocols for the web server. The supported protocols are `"h1"` (HTTP/1.x), `"h2"` (HTTP/2) and `"h3"` (HTTP/3; experimental). Default: `protocols "h1" "h2"`
- `timeout <timeout: integer|null>`
  - This directive specifies the maximum time (in milliseconds) for server to process the request, after which the server resets the connection. If set as `timeout #null`, the timeout is disabled. It's not recommended to disable the timeout, as this might leave the server vulnerable to Slow HTTP attacks. Default: `timeout 300000`
- `block <blocked_ip: string> [<blocked_ip: string> ...]`
  - This directive specifies IP addresses to be blocked. This directive can be specified multiple times. Default: none
- `cache_max_entries <cache_max_entries: integer|null>` (_cache_ module)
  - This directive specifies the maximum number of entries that can be stored in the HTTP cache. If set as `cache_max_entries #null`, the cache can theoretically store an unlimited number of entries. The cache keys for entries depend on the request method, the rewritten request URL, the "Host" header value, and varying request headers. Default: `cache_max_entries #null`
- `lb_health_check_window <lb_health_check_window: integer>` (_rproxy_ module)
  - This directive specifies the window size (in milliseconds) for load balancer health checks. Default: `lb_health_check_window 5000`
- `listen_ip <listen_ip: string>`
  - This directive specifies the IP address to listen. Default: `listen_ip "::1"`
- `io_uring [enable_io_uring: bool]`
  - This directive specifies whenever `io_uring` is enabled. This directive has no effect for systems that don't support `io_uring` and for web server builds that use Tokio instead of Monoio. Default: `io_uring #true`
- `wsgi_clear_imports [wsgi_clear_imports: bool]` (_wsgi_ module)
  - This directive specifies whenever to enable Python module import path clearing. Setting this option as `wsgi_clear_imports #true` improves the compatiblity with setups involving multiple WSGI applications, however module imports inside functions must not be used in the WSGI application. Default: `wsgi_clear_imports #false`
- `asgi_clear_imports [asgi_clear_imports: bool]` (_asgi_ module)
  - This directive specifies whenever to enable Python module import path clearing. Setting this option as `asgi_clear_imports #true` improves the compatiblity with setups involving multiple ASGI applications, however module imports inside functions must not be used in the ASGI application. Default: `asgi_clear_imports #false`
- `tcp_send_buffer <tcp_send_buffer: integer>`
  - This directive specifies the send buffer size in bytes for TCP listeners. Default: none
- `tcp_recv_buffer <tcp_recv_buffer: integer>`
  - This directive specifies the receive buffer size in bytes for TCP listeners. Default: none

## Global and virtual host directives

- `tls <certificate_path: string> <private_key_path: string>`
  - This directive specifies the path to the TLS certificate and private key. Default: none
- `auto_tls [enable_automatic_tls: bool]`
  - This directive specifies whenever automatic TLS is enabled. Default: `auto_tls #true` when port isn't explicitly specified, otherwise `auto_tls #false`
- `auto_tls_contact <auto_tls_contact: string|null>`
  - This directive specifies the email address used to register an ACME account for automatic TLS. Default: `auto_tls_contact #null`
- `auto_tls_cache <auto_tls_cache: string|null>`
  - This directive specifies the directory to store cached ACME data, such as cached account data and certifies. Default: OS-specific directory, for example on GNU/Linux it can be `/home/user/.local/share/ferron-acme` for the "user" user, on macOS it can be `/Users/user/Library/Application Support/ferron-acme` for the "user" user, on Windows it can be `C:\Users\user\AppData\Local\ferron-acme` for the "user" user.
- `auto_tls_letsencrypt_production [enable_auto_tls_letsencrypt_production: bool]`
  - This directive specifies whenever the production Let's Encrypt ACME endpoint is used. If set as `auto_tls_letsencrypt_production #false`, the staging Let's Encrypt ACME endpoint is used. Default: `auto_tls_letsencrypt_production #true`
- `auto_tls_challenge <acme_challenge_type: string>`
  - This directive specifies the used ACME challenge type. The supported types are `"http-01"` (HTTP-01 ACME challenge) and `"tls-alpn-01"` (TLS-ALPN-01 ACME challenge). Default: `auto_tls_challenge "tls-alpn-01"`

## Directives

- `header <header_name: string> <header_value: string>`
  - This directive specifies a header to be added to HTTP responses. This directive can be specified multiple times. Default: none
- `allow_double_slashes [h2_enable_connect_protocol: bool]`
  - This directive specifies whenever double slashes are allowed in the URL. Default: `allow_double_slashes #false`
- `server_administrator_email <server_administrator_email: string>`
  - This directive specifies the server administrator's email address to be used in the default 500 Internal Server Error page. Default: none
- `error_page <status_code: integer> <path: string>`
  - This directive specifies a custom error page to be served by the web server. Default: none
- `trust_x_forwarded_for [trust_x_forwarded_for: bool]`
  - This directive specifies whenever to trust the value of the `X-Forwarded-For` header. It's recommended to configure this directive if behind a reverse proxy. Default: `trust_x_forwarded_for #false`
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
- `status <status_code: integer> url=<url: string>|regex=<regex: string> [location=<location: string>] [realm=<realm: string>] [brute_protection=<enable_brute_protection: bool>] [users=<users: string>] [allowed=<allowed: string>]`
  - This directive specifies the custom status code. This directive can be specified multiple times. The `url` prop specifies the request path for this status code. The `regex` prop specifies the regular expression (like `^/ferron(?:$|[/#?])`) for the custom status code. The `location` prop specifies the destination for the redirect. The `realm` prop specifies the HTTP basic authentication realm. The `brute_protection` prop specifies whenever the brute-force protection is enabled. The `users` prop is a comma-separated list of allowed users for HTTP authentication. The `allowed` prop is a comma-separated list of allowed IP addresses. Default: none
- `users [username: string] [password_hash: string]`
  - This directive specifies an user with a password hash used for the HTTP basic authentication (it can be either Argon2, PBKDF2, or `scrypt` one). It's recommended to use the `ferron-passwd` tool to generate the password hash. This directive can be specified multiple times. Default: none
- `root <webroot: string|null>`
  - This directive specifies the webroot from which static files are served. If set as `root #null`, the static file serving functionality is disabled. Default: none
- `etag [enable_etag: bool]` (_static_ module)
  - This directive specifies whenever the ETag header is enabled. Default: `etag #true`
- `compressed [enable_compression: bool]`]` (_static_ module)
  - This directive specifies whenever the HTTP compression is enabled. Default: `compressed #true`
- `directory_listing [enable_directory_listing: bool]`]` (_static_ module)
  - This directive specifies whenever the directory listings are enabled. Default: `directory_listing #false`
- `cache [enable_cache: bool]` (_cache_ module)
  - This directive specifies whenever the HTTP cache is enabled. Default: `cache #false`
- `cache_max_response_size <cache_max_response_size: integer|null>` (_cache_ module)
  - This directive specifies the maximum size of the response (in bytes) that can be stored in the HTTP cache. If set as `cache_max_response_size #null`, the cache can theoretically store responses of any size. Default: `cache_max_response_size #null`
- `cache_vary <varying_request_header: string> [<varying_request_header: string> ...]` (_cache_ module)
  - This directive specifies the request headers that are used to vary the cache entries. This directive can be specified multiple times. Default: none
- `cache_ignore <ignored_response_header: string> [<ignored_response_header: string> ...]` (_cache_ module)
  - This directive specifies the response headers that are ignored when caching the response. This directive can be specified multiple times. Default: none
- `proxy <proxy_to: string|null>` (_rproxy_ module)
  - This directive specifies the URL to which the reverse proxy should forward requests. This directive can be specified multiple times. Default: none
- `lb_health_check [enable_lb_health_check: bool]` (_rproxy_ module)
  - This directive specifies whenever the load balancer passive health check is enabled. Default: `lb_health_check #false`
- `lb_health_check_max_fails <max_fails: integer>` (_rproxy_ module)
  - This directive specifies the maximum number of consecutive failures before the load balancer marks a backend as unhealthy. Default: `lb_health_check_max_fails 3`
- `proxy_no_verification [proxy_no_verification: bool]` (_rproxy_ module)
  - This directive specifies whenever the reverse proxy should not verify the TLS certificate of the backend. Default: `proxy_no_verification #false`
- `proxy_intercept_errors [proxy_intercept_errors: bool]` (_rproxy_ module)
  - This directive specifies whenever the reverse proxy should intercept errors from the backend. Default: `proxy_intercept_errors #false`
- `auth_to <auth_to: string|null>` (_fauth_ module)
  - This directive specifies the URL to which the web server should send requests for forwarded authentication. Default: none
- `auth_to_no_verification [auth_to_no_verification: bool]` (_fauth_ module)
  - This directive specifies whenever the server should not verify the TLS certificate of the backend authentication server. Default: `auth_to_no_verification #false`
- `auth_to_copy <request_header_to_copy: string> [<request_header_to_copy: string> ...]` (_fauth_ module)
  - This directive specifies the request headers that will be copied and sent to the forwarded authentication backend server. This directive can be specified multiple times. Default: none
- `example_handler [enable_example_handler: bool]` (_example_ module)
  - This directive specifies whenever an example handler is enabled. This handler responds with "Hello World" for "/hello" request paths. Default: `example_handler #false`
- `forward_proxy [enable_forward_proxy: bool]` (_fproxy_ module)
  - This directive specifies whenever the forward proxy functionality is enabled. Default: `forward_proxy #false`
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
- `fcgi <fcgi_to: string|null> [pass=<fcgi_pass: bool>]` (_scgi_ module)
  - This directive specifies whenever FastCGI is enabled and the base URL to which the FastCGI client will send requests. The `pass` prop specified whenever to pass the all the requests to the FastCGI request handler. TCP (for example `tcp://localhost:4000/`) and Unix socket URLs (only on Unix systems; for example `unix:///run/scgi.sock`) are supported. Default: `fcgi #null pass=#true`
- `fcgi_php <fcgi_php_to: string|null>` (_fcgi_ module)
  - This directive specifies whenever PHP through FastCGI is enabled and the base URL to which the FastCGI client will send requests for ".php" files. TCP (for example `tcp://localhost:4000/`) and Unix socket URLs (only on Unix systems; for example `unix:///run/scgi.sock`) are supported. Default: `fcgi_php #null`
- `fcgi_extension <fcgi_extension: string|null>` (_fcgi_ module)
  - This directive specifies file extensions, which will be handled via the FastCGI handle. This directive can be specified multiple times. Default: none
- `fcgi_environment <environment_variable_name: string> <environment_variable_value: string>` (_fcgi_ module)
  - This directive specifies an environment variable passed into FastCGI server. Default: none
- `wsgi <wsgi_application_path: string|null>` (_wsgi_ module)
  - This directive specifies whenever WSGI is enabled and the path to the WSGI application. The WSGI application must have an `application` entry point. Default: `wsgi #null`
- `wsgid <wsgi_application_path: string|null>` (_wsgid_ module)
  - This directive specifies whenever WSGI with pre-forked process pool is enabled and the path to the WSGI application. The WSGI application must have an `application` entry point. Default: `wsgi #null`
- `asgi <asgi_application_path: string|null>` (_asgi_ module)
  - This directive specifies whenever ASGI is enabled and the path to the ASGI application. The WSGI application must have an `application` entry point. Default: `wsgi #null`

## Example configuration

Below is the example Ferron web server configuration:

```kdl
* {
    // Protocols
    protocols h1 h2

    // HTTP/2 settings
    h2_initial_window_size 65536
    h2_max_frame_size 16384
    h2_max_concurrent_streams 100
    h2_max_header_list_size 8192
    h2_enable_connect_protocol

    // Logging
    log "/var/log/ferron/access.log"
    error_log "/var/log/ferron/error.log"

    // Blocklist
    block "192.168.1.100"
    block "10.0.0.5"

    // Static file serving settings
    compressed
    directory_listing

    // Environment variables
    environment "APP_MODE" "production"
    environment "MAX_THREADS" "16"

    // TLS settings
    tls "/etc/ssl/certs/fallback.crt" "/etc/ssl/private/fallback.key"
    tls_min_version "TLSv1.2"
    tls_max_version "TLSv1.3"
    ocsp_stapling
}

example.com {
    // TLS settings
    tls "/etc/ssl/certs/example-com.crt" "/etc/ssl/private/example-com.key"
}

*.example.com {
    // TLS settings
    tls "/etc/ssl/certs/example-com.crt" "/etc/ssl/private/example-com.key"
}

example.com {
    // Server administrator email
    server_administrator_email "admin@example.com"

    // Headers
    header "X-Frame-Options" "DENY"
    header "X-Content-Type-Options" "nosniff"

    // URL rewriting
    rewrite "^/old-path/(.*)" "/new-path/$1" file=#false directory=#false last=#true

    // Webroot
    root "/var/www/example"

    // Error pages
    error_page 404 "/var/www/example/errors/404.html"
    error_page 500 "/var/www/example/errors/500.html"

    // Location configuration
    location "/static" remove_base=#true {
        root "/var/www/static"
    }
}

api.example.com {
    // Server administrator email
    server_administrator_email "api-admin@example.com"

    // Enable ETags
    etag

    // Users
    user "admin" "$2b$10$hashedpassword12345"

    // Enable HTTP authentication
    status 401 url="/restricted.html"

    proxy "http://backend-service:5000"

    // Uncomment to enable error configuration
    // error_config 404 {
    //     proxy "http://backend-fallback:5000"
    // }
}

// Uncomment to enable configuration file includes
// include "/etc/ferron.d/**/*.kdl"
```
