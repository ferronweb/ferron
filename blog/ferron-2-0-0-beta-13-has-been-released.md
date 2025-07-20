---
title: "Ferron 2.0.0-beta.13 has been released"
description: We are excited to announce the release of Ferron 2.0.0-beta.13. This release brings new features, several improvements and fixes.
date: 2025-07-20 22:29:00
cover: ./covers/ferron-2-0-0-beta-13-has-been-released.png
---

We are excited to introduce Ferron 2.0.0-beta.13, with new features, several improvements and fixes.

## Key improvements and fixes

### Support for Amazon Route 53

Ferron now supports Amazon Route 53 as a DNS provider for DNS-01 ACME challenges, expanding compatibility for automated certificate management.

### Automatic TLS on demand

This release adds support for on-demand TLS, allowing Ferron to automatically obtain TLS certificates after the first client request.

Below is the example configuration for automatic TLS on demand:

```kdl
* {
  // It's recommended to configure an endpoint for determining if a TLS certificate can be issued for a given hostname.
  auto_tls_on_demand_ask "http://on-demand-tls.example.com/ask"
}

*.example {
  auto_tls_on_demand
  root "/var/www/example"
}
```

### HTTP/2 backend connections

Ferron now supports connecting to backend servers using HTTP/2 when acting as a reverse proxy. This functionality is disabled by default for compatibility with the WebSocket protocol.

### Global configuration support

You can now define global configurations that don't imply a host, making setup cleaner and more modular.

For example, you can configure logging (without implying a host) like this:

```kdl
globals {
  log "/var/log/ferron/access.log"
  error_log "/var/log/ferron/error.log"
}
```

### Multi-hostname host blocks

Host blocks can now define multiple hostnames, simplifying configuration for services served under several domains.

For example, you can serve static files from multiple domains:

```kdl
example.com,example.org {
  root "/var/www/example"
}
```

### SNI handling fix for non-standard ports

Fixed an issue with SNI hostname recognition when using HTTPS on non-default ports. This ensures the correct certificate is selected for each connection.

### Improved graceful shutdowns

Graceful restarts now gracefully shut down the existing connections. Before this, HTTP/1.1 and HTTP/2 connections were still kept alive, while HTTP/3 connections were interrupted.

### Multiple "Vary" headers support

The server now supports multiple `Vary` headers in responses, improving cache control.

## Thank you!

We appreciate all the feedback and contributions from our community. Your support helps us improve Ferron with each release. Thank you for being a part of this journey!

_The Ferron Team_
