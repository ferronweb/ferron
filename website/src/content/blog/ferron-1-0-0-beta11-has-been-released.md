---
title: "Ferron 1.0.0-beta11 has been released"
description: We are excited to introduce Ferron 1.0.0-beta11. This release brings several enhancements, and some important fixes.
date: 2025-04-05 18:48:00
cover: /img/covers/ferron-1-0-0-beta11-has-been-released.png
---

We are excited to introduce Ferron 1.0.0-beta11, packed with enhancements and important fixes.

## Key improvements and fixes

### Improved ETag handling  
ETags are now consistently wrapped in double quotes, and they vary based on the compression algorithm used. This ensures better cache validation and interoperability with HTTP clients and proxies.

### Corrected `s-maxage` directive parsing  
Resolved a bug where the `s-maxage` directive in the `Cache-Control` header was not being handled correctly. This fix improves support for shared caches and enhances HTTP caching behavior.

### Added `Vary` header for static content  
The server now automatically adds the `Vary` header to responses serving static content. This improves caching correctness when content varies based on headers such as `Accept-Encoding`.

### Removed incorrect `Status` header in responses  
The server no longer includes the `Status` header from CGI/SCGI/FastCGI as an HTTP response header, aligning behavior more closely with HTTP standards.

## Thank you!

We appreciate all the feedback and contributions from our community. Your support helps us improve Ferron with each release. Thank you for being a part of this journey!

*The Ferron Team*
