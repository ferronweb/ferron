---
layout: "../layouts/MarkdownPage.astro"
title: Vulnerabilities
description: Discover security vulnerabilities of outdated Ferron versions. Stay informed and protect your websites with timely updates against potential threats.
---

Some older versions of Ferron may contain security vulnerabilities. It's recommended to keep Ferron up-to-date.

## Fixed in Project Karpacz 0.6.0

- An attacker could add double slashes to the request URL before "cgi-bin" to bypass the CGI handler and possibly leak the CGI scripts' source code. (CWE-22; _cgi_ module; introduced in Project Karpacz 0.5.0).

## Fixed in Project Karpacz 0.3.0

- An attacker could send a lot of concurrent requests (100 concurrent requests is enough) to make the server stop accepting HTTP requests. (CWE-410; _rproxy_ module; introduced in Project Karpacz 0.2.0).
