---
title: Troubleshooting
description: "Quick checks for Ferron 3 when a site, listener, proxy, TLS setup, static files, or config stops working."
---

Start with the service state, then check networking, TLS, routing, permissions, and configuration.

> [!tip]
> If you ask for help, include the Ferron version, your deployment type, a minimal config snippet, and the error log lines. If you suspect a Ferron bug, open an issue on [GitHub](https://github.com/ferronweb/ferron/issues).

## First checks

### Is Ferron running?

- Linux (systemd): `sudo systemctl status ferron`
- Windows Server: `sc query ferron`
- Docker: `docker ps` and `docker logs <container>`

Restart the service and read the logs again:

- Linux: `sudo systemctl restart ferron`
- Windows Server: `net stop ferron && net start ferron`

If you start Ferron manually, keep the terminal open and read stderr first.

### Is the port correct?

- Host blocks without explicit ports listen on `default_http_port` and `default_https_port` (`80` and `443` by default).
- `ferron-serve` defaults to `127.0.0.1:3000`.
- A host block with an explicit port starts one listener on that port.
- If you bind to a specific interface, check the `listen` directive.

Testing the wrong port can make a healthy server appear down.

### Are the logs enabled?

Configure at least one access log and one error log:

- File logs: `log` and `error_log`
- Linux packages: `/var/log/ferron/access.log` and `/var/log/ferron/error.log`
- Windows Event Log, when using Ferron on Windows
- For structured output, see [Observability and logging](/docs/configuration/observability/logging)

Read the newest error lines first. Reproduce the request once, then read the log again.

### What does the log say?

| Message                                    | Likely cause                                                        |
| ------------------------------------------ | ------------------------------------------------------------------- |
| `connection refused`                       | The target process does not listen on that host, port, or socket.   |
| Timeout errors                             | The target is unreachable, blocked, overloaded, or too slow.        |
| TLS verification or certificate errors     | The upstream certificate or trust chain is wrong.                   |
| `permission denied` or file-open errors    | Filesystem ownership or mode issues.                                |

## Reverse proxy issues

### Can you reach the upstream directly?

Test the upstream URL or socket from the host where Ferron runs. If direct access fails, fix the upstream first. Ferron cannot proxy to an unreachable backend.

### Is the upstream bound to `127.0.0.1`?

If Ferron and the upstream run in different hosts, containers, or namespaces, bind the upstream to `0.0.0.0` or the correct interface. A loopback bind is reachable only from the same network namespace.

### Does the upstream rate-limit or ban clients?

Upstream anti-abuse rules can block Ferron requests when the upstream cannot identify the client IP. Enable `X-Forwarded-For` or `Forwarded` handling on the upstream. Ferron sends these headers by default.

### Is TLS verification failing?

For HTTPS upstreams with private or self-signed certificates:

- Prefer installing the correct CA chain.
- Use `no_verification` only to confirm the diagnosis. Do not use it as a production default.

### Timeout or connection refused?

- `connection refused`: wrong port or host, or the service is down.
- Timeout: route, firewall, DNS, or upstream responsiveness problem.

> [!info]
> See [Reverse proxying](/docs/use-cases/traffic/reverse-proxy) and [Configuration: reverse proxying](/docs/configuration/proxy/reverse-proxy).

## TLS issues

### Certificate does not match the hostname

Check that the requested hostname matches a certificate SAN or the CN. Also check that the request reaches the correct Ferron host block.

### Missing DNS records

Automatic TLS requires a DNS record that points the hostname to the Ferron server.

### A firewall blocks port `80` or `443`

ACME validation fails when a firewall, cloud policy, or proxy path blocks these ports.

### ACME challenge fails

- Use the Let's Encrypt staging endpoint while you test.
- If Ferron sits behind an HTTPS-terminating proxy, choose the challenge type that matches that setup.
- Keep the ACME cache on persistent storage.

### `localhost` looks different from a public hostname

`localhost`, `127.0.0.1`, and `::1` use the local TLS provider, not ACME. The local certificate is for development only.

### Is the traffic bot or scanner noise?

Public servers receive background scanning traffic. This traffic can look like a misconfiguration.

- Requests for `/wp-login.php`, `/.env`, or `/phpmyadmin` are common probes against unrelated sites.
- Some TLS handshake failures happen before HTTP parsing. They can appear in the error log without a matching access log line.
- If legitimate traffic works and failures come from random IPs and paths, the cause is often external.

Reduce the impact with targeted controls:

- Add route-specific `limit` rules for sensitive endpoints.
- Use `allow` and `block` rules where they apply.
- Keep authentication and admin paths narrow and explicit.

## Static file issues

### Incorrect root path

Check that `root` points to the directory that contains the deployed files for that host or location.

### Permissions

Ferron must read files and traverse every directory in the path.

### Path rewriting

Single-page applications usually need a route fallback rewrite. Enable `rewrite_log` while you diagnose rewrite rules. Guard fallback rules with `file false` and `directory false`. Without the guards, static assets also match the fallback.

### Symlink 403 errors

Symlinks return `403 Forbidden` by default (`disable_symlinks true`). The error page does not name the link. Check each path component for links. Then allow them with `disable_symlinks false` or `disable_symlinks if_not_owner`.

> [!info]
> See [Static file serving](/docs/use-cases/content/static-files), [URL rewriting](/docs/use-cases/traffic/url-rewriting), and [Configuration: routing and URL processing](/docs/configuration/routing/url-processing).

## Access control and rate limiting

### Is a condition blocking traffic?

Check `status` rules, `allow` and `block` rules, and conditional logic.

### Is client IP detection correct?

If Ferron is behind a proxy or load balancer, check client IP handling. Wrong client IP handling breaks access control and rate limiting.

### Is `client_ip_from_header` configured correctly?

Set `client_ip_from_header` only when the trusted proxy path supplies forwarded client IP headers. Use `trusted_proxy` to restrict who can send them. For PROXY protocol frontends, use `protocol_proxy`.

> [!info]
> See [HTTP host directives](/docs/configuration/server/host), [Rate limiting](/docs/use-cases/security/rate-limiting), [Access control](/docs/use-cases/security/access-control), and [Configuration: conditionals](/docs/configuration/fundamentals/conditionals).

## Configuration mistakes

### Wrong nesting

Directive placement matters. A directive in the wrong block can pass validation but not apply where you expect.

### Missing directive

Common examples:

- `root` where the site needs `proxy`.
- No `fcgi_php` or `cgi` plus `extension` for PHP.
- No `location` separation between static files and API traffic.

### Unexpected conditional behavior

All subconditions in one `condition` block must pass. If one fails, the condition fails.

### Isolate the change

Start with a minimal known-good configuration. Validate it with `ferron validate -c ferron.conf`. Then add one change at a time.

> [!info]
> See [Getting started](/docs/getting-started), [Configuration syntax](/docs/configuration/fundamentals/syntax), and [Core directives](/docs/configuration/server/core-directives).

## Observability and logs

### Enable debug visibility

Ferron has no single global debug mode. For troubleshooting visibility:

- Keep `error_log` on.
- Keep access logging on with `log`.
- Enable `rewrite_log` temporarily when you debug rewrites.
- Use OTLP export if you already have centralized tracing or metrics.

### Read logs effectively

- Reproduce one request at a time.
- Match the request path and status in access logs with timestamps in error logs.
- Check whether the failure happened before upstream routing, during the upstream connection, or in local file or routing logic.

### Common access log status codes

- `404 Not Found`: wrong `root`, wrong path rewrite, or backend route mismatch.
- `403 Forbidden`: access-control rule or filesystem permission issue.
- `502 Bad Gateway`: upstream connection or handshake failure.
- `504 Gateway Timeout`: upstream reachable but too slow or unresponsive.
