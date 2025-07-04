---
title: "Ferron 2.0.0-beta.6 has been released"
description: We are excited to announce the release of Ferron 2.0.0-beta.6. This release brings several improvements, and fixes.
date: 2025-07-04 10:11:00
cover: ./covers/ferron-2-0-0-beta-6-has-been-released.png
---

We are excited to introduce Ferron 2.0.0-beta.6, with enhancements, and some important fixes.

## Key improvements and fixes

### Improved TCP connection handling

Fixed an issue where TCP connections were not properly closed when requested by the server. This improves stability and resource management, especially under high-load scenarios.

### ACME implementation update

Replaced the existing ACME implementation to pave the way for future support of **DNS-01 challenges**. This is a preparatory step to broaden certificate issuance capabilities.

**Note:** After updating, Ferron might obtain TLS certificates again from the certificate authority, even with the ACME cache enabled.

## Thank you!

We appreciate all the feedback and contributions from our community. Your support helps us improve Ferron with each release. Thank you for being a part of this journey!

_The Ferron Team_
