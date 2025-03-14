---
layout: "../layouts/MarkdownPage.astro"
title: Vulnerabilities
description: Discover security vulnerabilities of outdated Ferron versions. Stay informed and protect your websites with timely updates against potential threats.
---

Some older versions of Ferron may contain security vulnerabilities. It's recommended to keep Ferron up-to-date.

## Fixed in Ferron 1.0.0-beta3

- An attacker could send a request body smaller than the specified length, wait for a long time, and repeat with many connections to possibly exhaust the server resources. This is because the server doesn't implement server timeouts. (CWE-400)

## Fixed in Ferron 1.0.0-beta2

- An attacker could send a partial request body to the server, and then other parts of partial request body to possibly exhaust the server resources. This is because the server only sends first part of request body into web application. (CWE-770; _cgi_ module; introduced in Project Karpacz 0.6.0).
- An attacker could send a partial request body to the server, and then other parts of partial request body to possibly exhaust the server resources. This is because the server only sends first part of request body into web application. (CWE-770; _fcgi_ module; introduced in Project Karpacz 0.6.0).
- An attacker could send a partial request body to the server, and then other parts of partial request body to possibly exhaust the server resources. This is because the server only sends first part of request body into web application. (CWE-770; _scgi_ module; introduced in Project Karpacz 0.6.0).

## Fixed in Project Karpacz 0.6.0

- An attacker could add double slashes to the request URL before "cgi-bin" to bypass the CGI handler and possibly leak the CGI scripts' source code. (CWE-22; _cgi_ module; introduced in Project Karpacz 0.5.0).

## Fixed in Project Karpacz 0.3.0

- An attacker could send a lot of concurrent requests (100 concurrent requests is enough) to make the server stop accepting HTTP requests. (CWE-410; _rproxy_ module; introduced in Project Karpacz 0.2.0).
