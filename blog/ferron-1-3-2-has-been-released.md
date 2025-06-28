---
title: "Ferron 1.3.2 has been released"
description: We are excited to announce the release of Ferron 1.3.2. This release brings better compliance with HTTP and a fix related to reverse proxying and HTTP/3.
date: 2025-06-21 16:43:00
cover: ./covers/ferron-1-3-2-has-been-released.png
---

We are excited to introduce Ferron 1.3.2, where we have improved the compliance with HTTP specification and fixed a bug related to reverse proxying and HTTP/3.

## Key improvements and fixes

### Reverse proxying with HTTP/3

We've fixed an issue that caused Rust panics when using reverse proxying with HTTP/3. This resolves instability for users leveraging Ferron in HTTP/3 environments and ensures better compatibility and reliability.

### ETag formatting for partial content requests

Ferron now wraps ETags in quotes when responding to partial content requests, aligning with the HTTP specification. This improves compatibility with clients that strictly enforce correct header formatting.

## Thank you!

We appreciate all the feedback and contributions from our community. Your support helps us improve Ferron with each release. Thank you for being a part of this journey!

_The Ferron Team_
