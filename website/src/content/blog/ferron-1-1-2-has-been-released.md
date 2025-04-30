---
title: "Ferron 1.1.2 has been released"
description: We are excited to announce the release of Ferron 1.0.0-beta9. This release brings a HTTP/3-related bugfix.
date: 2025-03-30 13:05:00
cover: /img/covers/ferron-1-1-2-has-been-released.png
---

We are excited to introduce Ferron 1.1.2, where we have fixed a bug related to the HTTP/3 protocol.

## Key improvements and fixes

### Corrected Alt-Svc header behavior

Resolved an issue where the server was advertising HTTP/3 support via the `Alt-Svc` header, even when HTTP/3 was disabled. This fix ensures accurate protocol signaling and prevents clients from attempting to initiate unsupported connections.

## Thank you!

We appreciate all the feedback and contributions from our community. Your support helps us improve Ferron with each release. Thank you for being a part of this journey!

_The Ferron Team_
