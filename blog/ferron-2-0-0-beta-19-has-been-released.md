---
title: "Ferron 2.0.0-beta.19 has been released"
description: We are excited to announce the release of Ferron 2.0.0-beta.19. This release brings various new features.
date: 2025-10-11 14:24:00
cover: ./covers/ferron-2-0-0-beta-19-has-been-released.png
---

We are excited to introduce Ferron 2.0.0-beta.19, with various new features.

## Key improvements and fixes

### `{path_and_query}` placeholder

Added support for `{path_and_query}` placeholder, which would be replaced with the request path and query string.

### ACME Renewal Information (ARI) support

Added support for ACME Renewal Information (ARI) support, which allows Ferron to determine when to renew SSL/TLS certificates using the ACME protocol. This allows Ferron to follow the recommended renewal schedule and ensure that certificates are always up-to-date.

### Unix socket reverse proxy backend support

Added support for connecting to backend servers via Unix sockets as a reverse proxy, allowing Ferron to serve content from backend servers running on the same machine without the need for a backend server to listen to a TCP port.

To configure reverse proxying to a backend server via an Unix socket, you can use this configuration:

```kdl
// Example configuration with reverse proxy to an Unix socket. Replace "example.com" with your domain name.
example.com {
    proxy "http://hostname/" unix="/run/example.sock" // Replace "/run/example.sock" with the path to the Unix socket file
}
```

### Custom access log format support

Added support for custom access log formats, allowing more flexibility in logging requests and responses.

To configure custom access log formats, you can use this configuration:

```kdl
// Example configuration with custom access log format. Replace "example.com" with your domain name.
example.com {
    log "/var/log/ferron/access.log" // Replace "/var/log/ferron/access.log" with the path to the access log file

    // The log format is Combined Log Format.
    log_format "{client_ip} - {auth_user} [{timestamp}] \"{method} {path_and_query} {version}\" {status_code} {content_length} \"{header:Referer}\" \"{header:User-Agent}\""
    log_date_format "%d/%b/%Y:%H:%M:%S %z"

    // Serve static files (for this example)
    root "/var/www/html"
}
```

## Thank you!

We appreciate all the feedback and contributions from our community. Your support helps us improve Ferron with each release. Thank you for being a part of this journey!

_The Ferron Team_
