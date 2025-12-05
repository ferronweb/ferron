---
title: "Ferron 2.2.1 has been released"
description: We are excited to announce the release of Ferron 2.2.1. This release brings several fixes.
date: 2025-12-05 18:40:00
cover: ./covers/ferron-2-2-1-has-been-released.png
---

We are excited to introduce Ferron 2.2.1, with several fixes.

## Key improvements and fixes

### Graceful configuration reloading with OTLP deadlock fix

Fixed a bug causing a deadlock when the server is gracefully reloading its configuration and OTLP observability backend was enabled before. This would allow gracefully reloading the configuration when OTLP was enabled before.

### X-Forwarded-\* header override changes

The server now no longer overrides `X-Forwarded-Host` and `X-Forwarded-Proto` request headers before sending them to backend servers, when they exist, and the `X-Forwarded-For` header is trusted. This would allow chaining Ferron reverse proxies without losing the original request information.

## Thank you!

We appreciate all the feedback and contributions from our community. Your support helps us improve Ferron with each release. Thank you for being a part of this journey!

_The Ferron Team_
