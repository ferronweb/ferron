---
title: "Ferron 2.0.0-beta.17 has been released"
description: We are excited to announce the release of Ferron 2.0.0-beta.17. This release brings a fix for a crash when resolving a TLS certificate.
date: 2025-07-31 09:36:00
cover: ./covers/ferron-2-0-0-beta-16-has-been-released.png
---

We are excited to introduce Ferron 2.0.0-beta.17, with a fix for a crash when resolving a TLS certificate.

This release is also the first release since the delay of the development (we apologize for the delay), and the rebranding of the project.

## Key improvements and fixes

### TLS certificate resolution crash fix

Fixed a crash that occurred when resolving a TLS certificate when a client connects to the server via HTTPS. This crash only affected versions compiled with Tokio only (no Monoio), such as prebuilt executables for Windows Server.

## Thank you!

We appreciate all the feedback and contributions from our community. Your support helps us improve Ferron with each release. Thank you for being a part of this journey!

_The Ferron Team_
