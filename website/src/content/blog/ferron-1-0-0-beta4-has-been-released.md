---
title: "Ferron 1.0.0-beta4 has been released"
description: We are excited to announce the release of Ferron 1.0.0-beta4. This release brings several new features, improvements, and fixes.
date: 2025-03-14 14:15:00
cover: /img/covers/ferron-1-0-0-beta4-has-been-released.png
---

We are excited to introduce Ferron 1.0.0-beta4, packed with new features, enhancements, and important fixes.

## Key improvements and fixes

### Option to disable backend server certificate verification
Added an option to disable backend server certificate verification for the reverse proxy, improving flexibility for deployments with self-signed or internal certificates.

### Enhanced HTTPS environment variable support
Added support for the CGI/SCGI/FastCGI "HTTPS" environment variable, ensuring better compatibility with various web server configurations.

### Configuration reload without full restart
Added support for configuration reloading without entirely restarting the server via a "SIGHUP" signal, reducing downtime and improving server management.

### Improved virtual host resolution for HTTP/2
Fixed an issue where virtual host resolution was not working for HTTP/2 connections, ensuring correct routing and domain handling.

## Thank you!

We appreciate all the feedback and contributions from our community. Your support helps us improve Ferron with each release. Thank you for being a part of this journey!

*The Ferron Team*

