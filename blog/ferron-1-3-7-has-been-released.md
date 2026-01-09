---
title: "Ferron 1.3.7 has been released"
description: We are excited to announce the release of Ferron 1.3.7. This release brings a new feature and a configuration-related bugfix.
date: 2025-12-23 12:25:00
cover: ./covers/ferron-1-3-7-has-been-released.png
---

We are excited to introduce Ferron 1.3.7, with support for accepting CIDR ranges for IP blocklists and a fix for a panic when the global web server configuration is not present in the configuration file.

## Key improvements and fixes

### Support for CIDR ranges in IP blocklists

Added support for accepting CIDR ranges for IP blocklists (backported from Ferron 2). This allows specifying IP ranges in the blocklist configuration.

### Fix for panic when global web server configuration is not present

Fixed a panic when the global web server configuration is not present in the configuration file.

## Thank you!

We appreciate all the feedback and contributions from our community. Your support helps us improve Ferron with each release. Thank you for being a part of this journey!

_The Ferron Team_
