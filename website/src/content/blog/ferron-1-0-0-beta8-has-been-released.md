---
title: "Ferron 1.0.0-beta8 has been released"
description: We are excited to announce the release of Ferron 1.0.0-beta8. This release brings several new features, improvements, and fixes.
date: 2025-03-28 21:00:00
cover: /img/covers/ferron-1-0-0-beta8-has-been-released.png
---

We are excited to introduce Ferron 1.0.0-beta8, packed with new features, enhancements, and important fixes.

## Key improvements and fixes

### Enhanced Custom Header Support  
Added support for `{path}` placeholders in custom header values, allowing more dynamic and flexible request handling.  

### Improved request URL handling  
The server now correctly uses the original request URL before rewriting when setting the CGI, SCGI, and FastCGI `REQUEST_URI` environment variables. This resolves redirect loop issues when URL rewriting is used with Joomla and similar setups.  

### More accurate directory listings  
The server now respects the original request URL before rewriting when generating directory listings, improving consistency in file browsing scenarios.  

## Thank you!

We appreciate all the feedback and contributions from our community. Your support helps us improve Ferron with each release. Thank you for being a part of this journey!

_The Ferron Team_