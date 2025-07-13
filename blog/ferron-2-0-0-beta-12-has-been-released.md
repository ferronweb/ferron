---
title: "Ferron 2.0.0-beta.12 has been released"
description: We are excited to announce the release of Ferron 2.0.0-beta.12. This release brings a TCP listener-related fix.
date: 2025-07-13 15:03:00
cover: ./covers/ferron-2-0-0-beta-12-has-been-released.png
---

We are excited to introduce Ferron 2.0.0-beta.12, with a TCP listener-related fix.

## Key improvements and fixes

### "Address already in use" error fixes after server restart

Fixed an error that caused the server to fail listening on TCP ports with "address already in use" errors after stopping and short after starting the server. This improves the convenience of restarting the server without waiting some time.

## Thank you!

We appreciate all the feedback and contributions from our community. Your support helps us improve Ferron with each release. Thank you for being a part of this journey!

_The Ferron Team_
