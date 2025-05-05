---
title: "Ferron 1.1.1 has been released"
description: We are excited to announce the release of Ferron 1.1.1. This release brings HTTP/3-related bugfixes.
date: 2025-04-29 18:14:00
cover: /img/covers/ferron-1-1-1-has-been-released.png
---

We are excited to introduce Ferron 1.1.1, a maintenance release that addresses important issues related to HTTP/3, ensuring improved stability and performance.

## Key improvements and fixes

### Resolved infinite loop in HTTP/3 client

Fixed an issue that caused an infinite loop when fetching the request body from the HTTP/3 client. This resolves potential hangs and improves overall reliability in data handling.

### Removed duplicate entries in Alt-Svc Header 

Fixed a bug where duplicate alternative services were being added to the `Alt-Svc` header when using the `cache` module. This improves compatibility with clients and reduces unnecessary header bloat.

## Thank you!

We appreciate all the feedback and contributions from our community. Your support helps us improve Ferron with each release. Thank you for being a part of this journey!

_The Ferron Team_