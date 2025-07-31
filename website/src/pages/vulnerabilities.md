---
layout: "../layouts/MarkdownPage.astro"
title: Vulnerabilities
description: Discover security vulnerabilities of outdated Ferron versions. Stay informed and protect your websites with timely updates against potential threats.
---

Some older versions of Ferron may contain security vulnerabilities. It's recommended to keep Ferron up-to-date.

## Fixed in Ferron 1.3.5

- An attacker could send a lot of concurrent requests that have a header defining accepted compression algorithm to be Brotli (for example using `ferrbench -c 20000 -d 1h -t 12 -H "Cache-Control: no-cache" -H "Accept-Encoding: br" -h https://victim.example --http2` command) to cause the server to consume too much memory. (CWE-400)

## Fixed in Ferron 1.3.4 and Ferron 2.0.0-beta.14

- An attacker could request a resource with a URL that would be replaced with a sanitized one, to possibly bypass security restrictions, if they're configured in location configurations. (CWE-20; introduced in Ferron 1.0.0-beta2)

## Fixed in Ferron 1.3.2 and Ferron 2.0.0-beta.3

- An attacker could connect to the server acting as a reverse proxy via HTTP/3 to cause a Rust panic in the server, and in effect crash the server. (CWE-248; _fauth_ module; introduced in Ferron 1.1.0)
- An attacker could connect to the server acting as a reverse proxy via HTTP/3 to cause a Rust panic in the server, and in effect crash the server. (CWE-248; _rproxy_ module; introduced in Ferron 1.1.0)

## Fixed in Ferron 1.1.1

- An attacker could connect to the server that handles request bodies via HTTP/3 and send a request body to make the server stop accepting HTTP requests due to the server entering an infinite loop after the client finished sending the request body. (CWE-835; introduced in Ferron 1.1.0)

## Fixed in Ferron 1.0.0-beta6

- An attacker could send a lot of concurrent requests that have a header defining accepted compression algorithm to be Brotli (for example using `ferrbench -c 100 -d 1h -t 12 -H "Accept-Encoding: br" -h https://victim.example --http2` command) to make the server stop accepting HTTP requests, due to inefficient compression process. (CWE-400).

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
