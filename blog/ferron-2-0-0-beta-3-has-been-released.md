---
title: "Ferron 2.0.0-beta.3 has been released"
description: We are excited to announce the release of Ferron 2.0.0-beta.3. This release brings several new features, improvements, and fixes.
date: 2025-06-22 12:20:00
cover: /img/covers/ferron-2-0-0-beta-3-has-been-released.png
---

We are excited to introduce Ferron 2.0.0-beta.3, with new features, enhancements, and some important fixes.

## Key improvements and fixes

### Automatic TLS enhancements

Several new automatic TLS-related configuration directives have been added to simplify secure deployments and improve compatibility with modern ACME setups.

### Common ACME account cache

The server now uses a common ACME account cache directory for automatic TLS, allowing smoother certificate management across multiple configurations.

### Per-host logging support

Ferron now supports per-host logging, enabling better observability and troubleshooting for multi-host setups.

### Reverse proxy stability fixes

- **Fixed 502 errors in Docker environments** - resolved an issue where canceled operations during reverse proxying caused 502 errors when running Ferron behind Docker.
- **Fixed Rust panics with HTTP/3** - addressed a panic that could occur when reverse proxying traffic over HTTP/3.

### Routing logic fix

Corrected a bug where location configurations were being evaluated in the wrong order, ensuring more predictable and accurate request routing.

### Configuration compatibility

Fixed the translation of the `maximumCacheEntries` YAML property from Ferron 1.x, improving compatibility and easing the migration path for existing users.

## Thank you!

We appreciate all the feedback and contributions from our community. Your support helps us improve Ferron with each release. Thank you for being a part of this journey!

_The Ferron Team_
