---
title: "Ferron 1.0.0-beta7 has been released"
description: We are excited to introduce Ferron 1.0.0-beta7. This release brings several enhancements, and some important fixes.
date: 2025-03-27 15:49:00
cover: /img/covers/ferron-1-0-0-beta7-has-been-released.png
---

We are excited to introduce Ferron 1.0.0-beta7, packed with enhancements and important fixes.

## Key improvements and fixes

### Dropped support for dynamically-loaded server modules
Ferron now only supports compiled-in optional modules that can be disabled via Cargo features. This change improves security and simplifies deployment by reducing runtime dependencies. We did this, because of complexities and bugs related to Tokio and dynamically linked libraries.

### HTTP/2 enabled by default for encrypted connections
Encrypted connections will now use HTTP/2 by default, improving performance, reducing latency, and enabling more efficient multiplexing of requests.

### Refactored HTTP connection acceptance logic
The HTTP connection handling has been refactored to be more correct.

## Thank you!

We appreciate all the feedback and contributions from our community. Your support helps us improve Ferron with each release. Thank you for being a part of this journey!

*The Ferron Team*
