---
title: "Configuration: routing & URL processing"
description: "Response header customization, error-page behavior, URL handling, and rewrite rules."
---

This page explains KDL directives for request routing, URL normalization and rewriting, and response header behavior.

## Directives

### Headers & response customization

- `header <header_name: string> <header_value: string>`
  - This directive specifies a header to be added to HTTP responses. The header values supports placeholders like `{path}` which will be replaced with the request path. This directive can be specified multiple times. Default: none
- `server_administrator_email <server_administrator_email: string>`
  - This directive specifies the server administrator's email address to be used in the default 500 Internal Server Error page. Default: none
- `error_page <status_code: integer> <path: string>`
  - This directive specifies a custom error page to be served by the web server. Default: none
- `header_remove <header_name: string>`
  - This directive specifies a header to be removed from HTTP responses. This directive can be specified multiple times. Default: none
- `header_replace <header_name: string> <header_value: string>`
  - This directive specifies a header to be added to HTTP responses, potentially replacing existing headers. The header values supports placeholders like `{path}` which will be replaced with the request path. This directive can be specified multiple times. Default: none

## Configuration example

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

### URL processing & routing

- `allow_double_slashes [allow_double_slashes: bool]`
  - This directive specifies whether double slashes are allowed in the URL. Default: `allow_double_slashes #false`
- `no_redirect_to_https [no_redirect_to_https: bool]`
  - This directive specifies whether not to redirect from HTTP URL to HTTPS URL. This directive is always effectively set to `no_redirect_to_https` when the server port is explicitly specified in the configuration. Default: `no_redirect_to_https #false`
- `wwwredirect [enable_wwwredirect: bool]`
  - This directive specifies whether to redirect from URL without "www." to URL with "www.". Default: `wwwredirect #false`
- `rewrite <regex: string> <replacement: string> [directory=<directory: bool>] [file=<file: bool>] [last=<last: bool>] [allow_double_slashes=<allow_double_slashes: bool>]`
  - This directive specifies the URL rewriting rule. This directive can be specified multiple times. The first value is a regular expression (like `^/ferron(?:$|[/#?])`). The `directory` prop specifies whether the rewrite rule is applied when the path would correspond to directory (if `#false`, then it's not applied). The `file` prop specifies whether the rewrite rule is applied when the path would correspond to file (if `#false`, then it's not applied). The `last` prop specifies whether the rewrite rule is the last rule applied. The `allow_double_slashes` prop specifies whether the rewrite rule allows double slashes in the request URL. Default: none
- `rewrite_log [rewrite_log: bool]`
  - This directive specifies whether URL rewriting operations are logged into the error log. Default: `rewrite_log #false`
- `no_trailing_redirect [no_trailing_redirect: bool]`
  - This directive specifies whenever not to redirect the URL without a trailing slash to one with a trailing slash, if it refers to a directory. Default: `no_trailing_redirect #false`
- `disable_url_sanitizer [disable_url_sanitizer: bool]` (Ferron 2.3.0 or newer)
  - This directive specifies whenever URL sanitation is disabled. Disabling URL sanitation allows the server to process the request URL as is, without rewriting the URL with potential path traversal sequences; this can be useful for certain applications that require raw URLs, for [RFC 3986 compliance](https://datatracker.ietf.org/doc/html/rfc3986#section-2.2). **Disabling URL sanitation may lead to risk of path traversal vulnerabilities, although built-in static file serving, CGI, SCGI and FastCGI module functionality would perform additional checks to prevent path traversal attacks.** Default: `disable_url_sanitizer #false`

## Configuration example

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
