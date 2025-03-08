---
title: "Ferron 1.0.0-beta2 has been released"
description: We are excited to announce the release of Ferron 1.0.0-beta2. This release brings several new features, improvements, and fixes.
date: 2025-03-08 14:51:00
cover: /img/covers/ferron-1-0-0-beta2-has-been-released.png
---

We are excited to introduce Ferron 1.0.0-beta2, packed with new features, enhancements, and important fixes. This update brings greater flexibility, improved performance, and stronger compatibility, ensuring a more robust and efficient web server experience.

## Key improvements and fixes

### Forwarded authentication module ("fauth")

A new authentication module has been introduced, allowing Ferron to handle forwarded authentication. This enhances security and flexibility for authentication setups in various deployment environments.

### Per-location configuration support

Ferron now supports per-location configuration, enabling more granular control over settings and behaviors based on different request paths.

### Enhanced reverse proxy header support

The "rproxy" module now includes support for the `X-Forwarded-Proto` and `X-Forwarded-Host` headers, improving compatibility with proxies and load balancers.

### FastCGI connection handling fix

A bug was fixed where FastCGI connections were not being closed properly when only a partial request body was sent. This resolves potential resource leaks and ensures better stability.

### Performance optimization without CGI execution

Performance improvements have been made for cases where no CGI program is executed, reducing overhead and making the server more efficient.

## Thank you!

We appreciate all the feedback and contributions from our community. Your support helps us improve Ferron with each release. Thank you for being a part of this journey!

*The Ferron Team*