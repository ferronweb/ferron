---
title: "Ferron 2.1.0 has been released"
description: We are excited to announce the release of Ferron 2.1.0. This release brings new features, improvements and fixes.
date: 2025-11-26 15:58:00
cover: ./covers/ferron-2-1-0-has-been-released.png
---

We are excited to introduce Ferron 2.1.0, with new features, improvements and fixes.

## Key improvements and fixes

### Language matching support

Added a language matching subcondition (`is_language`) that can be used inside `condition` blocks to match the language of the current page against the preferred language of the user. This allows for more granular control over content display based on user preferences.

Below is an example configuration that involves a language matching subcondition:

```kdl
// Snippet for common language settings
snippet "LANG_COMMON" {
    set_constant "LANGUAGES" "en,de,pl"
}

example.com {
    // Generic response
    status 200 body="lang: Unknown"

    condition "LANG_PL" {
        use "LANG_COMMON"
        is_language "pl"
    }

    condition "LANG_DE" {
        use "LANG_COMMON"
        is_language "de"
    }

    condition "LANG_EN" {
        use "LANG_COMMON"
        is_language "en"
    }

    if "LANG_PL" {
        // Polish language
        status 200 body="lang: Polski"
    }

    if "LANG_DE" {
        // German language
        status 200 body="lang: Deutsch"
    }

    if "LANG_EN" {
        // English language
        status 200 body="lang: English"
    }
}
```

### Custom MIME types for static file serving

Added support for specifying custom MIME types (via `mime_type` directives) for static file serving. This allows you to define custom MIME types for specific file extensions, ensuring accurate content type headers are sent to clients, in case the default MIME type might be missing.

Below is an example configuration that involves custom MIME types:

```kdl
// Replace "example.com" with your domain name.
example.com {
    root "/var/www/html" // Replace "/var/www/html" with your document root.
    mime_type ".pdf" "application/pdf"
}
```

### Dynamic content compression

Added support for compressing dynamic content (via `dynamic_compressed` directive). This allows you to compress dynamic content, reducing the size of the response and improving the performance of your website.

Below is an example configuration that involves dynamic content compression:

```kdl
// Replace "example.com" with your domain name.
example.com {
    proxy "http://localhost:3000" // Replace "http://localhost:3000" with your backend server address.
    dynamic_compressed
}
```

### HTTP/2-only (and gRPC over plain text) backend server support

Added support for HTTP/2-only (and gRPC over plain text) backend server support (via `proxy_http2_only` directive). This allows you to specify that the backend server only supports HTTP/2 over plain text, allowing reverse proxying to gRPC services.

Below is an example configuration that involves HTTP/2-only backend server support:

```kdl
// Replace "example.com" with your domain name.
example.com {
    proxy "http://localhost:3000" // Replace "http://localhost:3000" with your backend server address.
    proxy_http2_only
}
```

### PROXY protocol header support in reverse proxying

Added support for PROXY protocol header support (via `proxy_proxy_header` directive). This allows you to specify that the backend server supports the PROXY protocol, allowing reverse proxying to services that require the PROXY protocol.

Below is an example configuration that involves PROXY protocol header support:

```kdl
// Replace "example.com" with your domain name.
example.com {
    proxy "http://localhost:3000" // Replace "http://localhost:3000" with your backend server address.
    proxy_proxy_header "v2" // PROXY protocol version 2
}
```

### Support for constants inside conditions

Added support for setting constants inside conditions. This makes it easier to manage complex configurations.

### Custom index file support

Added support for custom directory index file support (via `index` directive). This allows you to specify a custom index file for static file serving, CGI and FastCGI.

Below is an example configuration that involves custom index files:

```kdl
// Replace "example.com" with your domain name.
example.com {
    root "/var/www/html" // Replace "/var/www/html" with your document root.
    index "index.html" "default.html"
}
```

### Support for snippets inside conditions

Added support for including snippets (specified using `snippet` blocks and included using `use` directives) inside conditions. This allows you to reuse common configurations across multiple conditions.

### Improved error reporting for configuration validation and module loading errors

Configuration validation and module loading errors now include the block where invalid configuration was detected. This can help you identify and fix configuration-related issues more quickly.

Example output (before; Ferron 2.0.1; for an invalid `root` directive value):

```
Error while running a server: Invalid webroot
```

Example output (after; Ferron 2.1.0; for an invalid `root` directive value):

```
Error while running a server: Invalid webroot (at ":8080" host block)
```

### Fixed configuration validation for CGI interpreter directive

Fixed a bug that led to incorrect validation of the CGI interpreter directive (`cgi_interpreter`).

### Error configuration bugfix

Fixed a bug preventing some configuration properties in `error_config` blocks from being applied correctly.

### Access control directives are no longer global-only

The `block` and `allow` directives (used for access control) are no longer global-only. They can now be used within host blocks to control access to specific resources.

### HTTP/2 disabled for backend servers when client requests an HTTP upgrade

The server now disables HTTP/2 for backend servers when `proxy_http2` directive is used, and the request contains an `Upgrade` header (that is, the client requests an HTTP upgrade). This allows the reverse proxy to handle HTTP upgrades correctly, allowing for WebSocket connections to be established.

### Removal of "Forwarded" header before sending the request to backend servers

The server now removes the `Forwarded` header (Ferron sets `X-Forwarded-For` and similar headers) before sending the request to backend servers, to protect against client IP spoofing, when the backend service supports the `Forwarded` header.

## Thank you!

We appreciate all the feedback and contributions from our community. Your support helps us improve Ferron with each release. Thank you for being a part of this journey!

_The Ferron Team_
