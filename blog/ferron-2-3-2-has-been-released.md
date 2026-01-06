---
title: "Ferron 2.3.2 has been released"
description: We are excited to announce the release of Ferron 2.3.2. This release brings more canceled I/O operation crash fixes.
date: 2026-01-06 21:22:00
cover: ./covers/ferron-2-3-2-has-been-released.png
---

We are excited to introduce Ferron 2.3.2, with more canceled I/O operation crash fixes.

## Key improvements and fixes

### More canceled I/O operation crash fixes

Improved robustness by gracefully handling canceled I/O operations that could previously cause a crash (when io_uring was enabled) or 502 Bad Gateway errors (when io_uring was disabled), under rare conditions.

## Thank you!

We appreciate all the feedback and contributions from our community. Your support helps us improve Ferron with each release. Thank you for being a part of this journey!

_The Ferron Team_
