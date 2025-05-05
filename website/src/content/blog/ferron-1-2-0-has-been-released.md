---
title: "Ferron 1.2.0 has been released"
description: We are excited to announce the release of Ferron 1.2.0. This release brings several new features, improvements, and fixes.
date: 2025-05-03 12:18:00
cover: /img/covers/ferron-1-2-0-has-been-released.png
---

We are excited to introduce Ferron 1.2.0, packed with new features, enhancements, and important fixes.

## Key improvements and fixes

### Environment variable overrides

You can now override Ferron’s configuration using environment variables that begin with `FERRON_`. This provides more flexibility in dynamic and containerized deployment environments.

### Improved HTTP/3 support

The server now includes a `Date` header in all HTTP/3 responses, improving compliance with standard HTTP expectations and aiding in client-side debugging.

### Reverse proxy enhancements

When Ferron is configured as a reverse proxy, it now forwards the original request headers to the backend WebSocket server. This ensures greater fidelity and context preservation in proxied connections.

### ASGI WebSocket header forwarding

The server now forwards original request headers to ASGI applications during WebSocket connections, enabling more advanced routing, authentication, or analytics based on client-provided data.

### Configuration logging fix

Resolved an issue where the `http2Settings` configuration property was being logged as unused, even when configured. Ferron now correctly recognizes and utilizes this setting.

## Thank you!

We appreciate all the feedback and contributions from our community. Your support helps us improve Ferron with each release. Thank you for being a part of this journey!

_The Ferron Team_