---
title: "Ferron 2.0.0-beta.2 has been released"
description: We are excited to announce the release of Ferron 2.0.0-beta.2. This release brings several new features, improvements, and fixes.
date: 2025-06-17 14:02:00
cover: ./covers/ferron-2-0-0-beta-2-has-been-released.png
---

We are excited to introduce Ferron 2.0.0-beta.2, with new features, enhancements, and some important fixes.

## Key improvements and fixes

### Smarter configuration handling

A new configuration adapter has been added to automatically detect the correct configuration path when running the web server inside a Docker image. This eliminates manual setup and improves deployment consistency.

### Substring replacement module

Ferron now includes a module (`replace`) for substring replacement in response bodies. This can be useful for transforming dynamic content, templating, or modifying responses on the fly.

### Rate limiting module

We’ve introduced a rate limiting module (`limit`) to help prevent abuse and protect resources by limiting the number of requests per client.

### Post-quantum cryptography support

Added support for key exchanges using post-quantum cryptography, strengthening Ferron's security against future quantum threats. When we added this, we also switched the cryptography provider for Rustls from _ring_ to AWS LC.

### Error handling fix

Resolved an issue causing infinite recursion in the error handler, ensuring stable and predictable error processing.

### Improved YAML configuration translations

Fixed translation issues in the YAML configuration:

- The `errorPages` property is now correctly interpreted.
- The `users` property is also properly translated, enhancing configuration reliability.

### Clearer KDL parsing errors

KDL parsing errors are now formatted for better readability, making debugging configuration files much easier.

## Thank you!

We appreciate all the feedback and contributions from our community. Your support helps us improve Ferron with each release. Thank you for being a part of this journey!

_The Ferron Team_
