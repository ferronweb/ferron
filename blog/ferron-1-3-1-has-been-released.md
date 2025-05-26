---
title: "Ferron 1.3.1 has been released"
description: We are excited to announce the release of Ferron 1.3.1. This release brings several bug fixes.
date: 2025-05-26 17:25:00
cover: /img/covers/ferron-1-3-1-has-been-released.png
---

We are excited to introduce Ferron 1.3.1. This version brings several important bug fixes and improvements.

## Key improvements and fixes

### Corrected ASGI event type handling

Resolved an issue where the `http.request` ASGI event was mistakenly assigned the `lifespan.shutdown` event type. This fix ensures accurate event classification and improved ASGI compatibility.

### Improved configuration validation

Fixed a bug that caused improper validation of error and location configurations. Configuration files are now more strictly and correctly validated, reducing potential runtime misconfigurations.

## Thank you!

We appreciate all the feedback and contributions from our community. Your support helps us improve Ferron with each release. Thank you for being a part of this journey!

_The Ferron Team_
