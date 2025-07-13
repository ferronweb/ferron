---
title: "Ferron 2.0.0-beta.9 has been released"
description: We are excited to announce the release of Ferron 2.0.0-beta.9. This release brings several new features, and improvements.
date: 2025-07-11 11:51:00
cover: ./covers/ferron-2-0-0-beta-9-has-been-released.png
---

We are excited to introduce Ferron 2.0.0-beta.9, with new features and enhancements.

## Key improvements and fixes

### ACME profile support

Ferron now supports ACME profiles, allowing more flexible and secure certificate provisioning configurations. ACME profiles define the set of attributes about the certificate being issued, such as validity period, and more. It's now possible to obtain certificates that last 6 days from Let's Encrypt using the "shortlived" profile.

### DNS-01 challenge support

Added support for the DNS-01 ACME challenge, enabling users to prove domain ownership via DNS for certificate issuance. This allows for certificates with wildcard domains.

These DNS providers are supported as of this release:

- deSEC
- Cloudflare
- Porkbun
- RFC 2136

### Header replacement

Introduced header replacement capabilities, giving users more control over HTTP header manipulation.

### IP allowlists

You can now define IP allowlists to restrict access to services based on IP addresses for added security.

### Enhanced header value placeholders

Support for additional header value placeholders has been added, improving configuration flexibility.

### `Cache-Control` header support

Ferron now allows setting the `Cache-Control` header for static files to fine-tune browser and intermediary caching behavior.

### Sequential TLS certificate acquisition

The server now obtains TLS certificates from the ACME server sequentially, reducing potential rate-limit issues, race conditions, and improving reliability.

## Thank you!

We appreciate all the feedback and contributions from our community. Your support helps us improve Ferron with each release. Thank you for being a part of this journey!

_The Ferron Team_
