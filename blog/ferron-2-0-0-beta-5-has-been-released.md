---
title: "Ferron 2.0.0-beta.5 has been released"
description: We are excited to announce the release of Ferron 2.0.0-beta.5. This release brings several new features, improvements, and fixes.
date: 2025-06-29 11:00:00
cover: /img/covers/ferron-2-0-0-beta-5-has-been-released.png
---

We are excited to introduce Ferron 2.0.0-beta.5, with new features, enhancements, and some important fixes.

## Key improvements and fixes

### Response header removal

A new configuration directive has been added to allow the removal of response headers. This gives users finer control over outbound traffic and enhances privacy and security.

### Reverse proxy enhancements

- **HTTP keep-alive control** - you can now disable HTTP keep-alive for reverse proxy connections, providing more flexibility for managing connection behavior.
- **Custom request headers** - support has been added for setting headers on HTTP requests initiated by the reverse proxy. This allows better integration with backend services and APIs.

### Custom response bodies

The `status` directive now supports specifying custom response bodies. This enables more informative and user-friendly responses for static status codes.

### Port handling fix

A bug has been fixed where explicitly defined HTTP-only ports were mistakenly marked as HTTPS ports. This resolves potential confusion and ensures accurate port configuration.

### Default cache limit

Ferron now applies a default size limit to the HTTP cache, improving memory management and preventing unintended resource usage.

## Thank you!

We appreciate all the feedback and contributions from our community. Your support helps us improve Ferron with each release. Thank you for being a part of this journey!

_The Ferron Team_
