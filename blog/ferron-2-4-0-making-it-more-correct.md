---
title: "Ferron 2.4.0: making it more correct"
description: We are excited to announce the release of Ferron 2.4.0, which focuses on correctness, standards compliance, and robustness.
date: 2026-01-17 06:20:00
cover: ./covers/ferron-2-4-0-making-it-more-correct.png
---

We are excited to introduce Ferron 2.4.0, which focuses on correctness, standards compliance, and robustness, fixing several edge cases discovered through real-world usage.

## Key improvements and fixes

### New features and capabilities

We have received a [feature request](https://github.com/ferronweb/ferron/issues/337) for a DNS provider for DNS-01 ACME challenges that would use bunny.net, so we have added support for it. We have additionally added support for DigitalOcean and OVH DNS providers. You can read the [automatic TLS notes in the documentation](/docs/automatic-tls) to learn how to use them.

Also, we have added support for HTTP Basic authentication for forward proxying after we received [another feature request](https://github.com/ferronweb/ferron/issues/396). Here's the example configuration for HTTP Basic authentication for forward proxying:

```kdl
:8080 {
  user "test" "$2b$10$hashedpassword12345" // Replace with your username and hashed password (which can be generated with "ferron-passwd")
  forward_proxy_auth
  forward_proxy
}
```

### TLS/ACME improvements

We have fixed ACME cache file handling during certificate renewals. Cache files are now correctly truncated when rewritten, preventing stale data from causing parse failures. We have fixed this issue after seeing a [bug report](https://github.com/ferronweb/ferron/issues/407) (this one also showed how corrupted ACME cache files can be fixed).

Also, the server now performs cleanup of TLS-ALPN-01 and HTTP-01 challenges after successful certificate issuance (thank you GitHub Copilot for the suggestion!), to prevent stale ACME challenge data from being served to clients.

### URL rewriting and routing correctness improvements

We have fixed the original request URL not being preserved when the server is configured to rewrite URLs using the `rewrite` directive, trailing slash redirects leading to a URL without base when the `remove_base` property of a location block is set to `#true`, and URL rewrites not being applied when the `remove_base` property of a location block is set to `#true`.

We have fixed these issues after we spotted URL rewriting bugs after seeing [a GitHub issue with a question about configuring Ferron for single page applications served on subdirectories](https://github.com/ferronweb/ferron/issues/404).

### Static file serving improvements

We have fixed precompressed files not being picked up when the original filename doesn't have a file extension.

Also, we have improved compliance of static file serving functionality with RFC 7232 (conditional requests) and RFC 7233 (range requests), allowing for better interoperability with HTTP clients.

Thank you to GitHub Copilot for spotting both issues!

### Proxying improvements

We have fixed a bug (thank you GitHub Copilot for spotting it!), where `Connection` header would be sometimes set to `keep-alive, keep-alive` when it should have been `keep-alive`, before sending the request to the backend server. This should prevent issues with some backend servers that expect a single `keep-alive` value.

Also, the forwarded authentication module (`fauth`) now uses an unlimited idle keep-alive connection pool, just like the reverse proxy (`rproxy`) module.

### Performance, stability and resource handling fixes

We have fixed graceful shutdown (during configuration reloading) for the HTTP/3 server, potentially improving the memory usage of the server.

Also, the server now reuses connections that aren't ready after waiting for readiness when the concurrency limit is reached, instead of establishing a new connection. This should improve connection reuse when the concurrency limit is reached.

Additionally, the server now falls back with `io_uring` disabled when `io_uring` couldn't be initialized and `io_uring` is implicitly enabled. This was done after we saw a [crash report](https://github.com/ferronweb/ferron/issues/417), that we could reproduce by causing `io_uring` to fail to initialize.

### Configuration validation and diagnostics improvements

We have fixed brute-force protection not being able to be disabled due to a wrong configuration validation check, after we saw [a relevant bug report](https://github.com/ferronweb/ferron/issues/410).

Also, the server now logs a warning if the `status 200` directive is used without specifying a response body (which is often caused by misconfiguration).

## Thank you!

We appreciate all the feedback and contributions from our community. Your support helps us improve Ferron with each release. Thank you for being a part of this journey!

_The Ferron Team_
