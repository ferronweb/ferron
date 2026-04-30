---
title: Troubleshooting
description: "Quick checks for Ferron 3 when a site, listener, proxy, TLS setup, static files, or config stops working."
---

When something goes wrong with Ferron or your site, start here. The fastest way to find the cause is to separate service state, networking, TLS, routing, permissions, and configuration.

## Quick diagnostic checklist

1. Check the logs.
2. Confirm Ferron is running.
3. Test the upstream directly if you proxy requests.
4. Confirm the port and firewall.
5. Validate the config after changes.

## First checks

### Is Ferron running?

Check service status first.

- Linux (systemd) - `sudo systemctl status ferron`
- Windows Server - `sc query ferron`
- Docker - `docker ps` and `docker logs <container>`

If Ferron is not running, restart it and re-check logs right away.

- Linux - `sudo systemctl restart ferron`
- Windows Server - `net stop ferron && net start ferron`

If you start Ferron manually, keep the terminal open and read stderr first.

### Is the port correct?

- Host blocks without explicit ports usually serve on `default_http_port` and `default_https_port` (`80` and `443` by default).
- `ferron-serve` defaults to `127.0.0.1:3000`.
- If you set an explicit port in the host block, Ferron starts a single listener on that port.
- If you bind to a specific interface, check the `listen` directive too.

If you are testing the wrong port, the server can be healthy but still appear down.

### Are the logs enabled?

Use at least one access log and one error log.

- File logs - `log` and `error_log`
- Common package paths - `/var/log/ferron/access.log` and `/var/log/ferron/error.log` on Linux packages
- Windows Event Log (if using Ferron on Windows)
- If you prefer structured output, use [Observability and logging](/docs/v3/configuration/observability-logging) instead of relying on defaults

Read the newest error lines first, then reproduce the request once and read again.

### What does the log say?

Common meanings:

- `connection refused` - the target process is not listening on that host, port, or socket.
- timeout errors - the target is unreachable, blocked, overloaded, or too slow.
- TLS verification or certificate errors - the upstream certificate or trust chain is wrong.
- `permission denied` or file-open errors - filesystem ownership or mode issues.

## Reverse proxy issues

### Can you reach the upstream directly?

From the same host where Ferron runs, test the upstream URL or socket directly.

If direct access fails, fix the upstream first. Ferron cannot proxy to an unreachable backend.

### Is the upstream bound to `127.0.0.1` instead of `0.0.0.0`?

If Ferron and the upstream are in different hosts, containers, or namespaces, binding the upstream to `127.0.0.1` can make it unreachable externally. Bind to `0.0.0.0` or the correct interface when needed.

### Is TLS verification failing?

For HTTPS upstreams with private or self-signed certs:

- Prefer fixing trust by installing the correct CA chain.
- Use `no_verification` only to confirm the diagnosis, not as a production default.

### Timeout vs connection refused?

- `connection refused` - wrong port or host, or the service is down.
- timeout - route, firewall, DNS, or upstream responsiveness problem.

See [Reverse proxying](/docs/v3/use-cases/reverse-proxy) and [Configuration: reverse proxying](/docs/v3/configuration/reverse-proxying).

## TLS issues

### Certificate does not match the hostname

Ensure the requested hostname matches the certificate SAN or CN and the correct Ferron host block.

### Missing DNS records

For automatic TLS, DNS must point the hostname to the Ferron server.

### Port `80` or `443` is blocked

ACME validation can fail when these ports are blocked by a firewall, cloud policy, or proxy path.

### ACME challenge is failing

- Use the Let's Encrypt staging endpoint first if you are still testing.
- If Ferron sits behind an HTTPS-terminating proxy, choose the challenge type that matches that setup.
- Keep the ACME cache on persistent storage.

### `localhost` looks different from a public hostname

`localhost`, `127.0.0.1`, and `::1` use Ferron's local TLS provider, not ACME. That certificate is for local development only.

## Static file issues

### Incorrect root path

Verify `root` points to the directory containing the deployed files for that host or location.

### Permissions

Ferron must be able to read files and traverse directories.

### Path rewriting confusion

For SPAs, route fallback rewrites are commonly required. If rewrites are involved, enable `rewrite_log` while diagnosing.

See [Static file serving](/docs/v3/use-cases/static-file-serving), [URL rewriting](/docs/v3/use-cases/url-rewriting), and [Configuration: routing and URL processing](/docs/v3/configuration/routing-url-processing).

## Rate limiting and access control

### Is a condition blocking traffic?

Check `status` rules, `allow`/`block`, and conditional logic.

### Is client IP detection correct?

If Ferron is behind a proxy or load balancer and client IP handling is wrong, access control and rate limiting can be wrong too.

### Is `client_ip_from_header` configured correctly?

Set `client_ip_from_header` only when the trusted proxy path supplies forwarded client IP headers. Use `trusted_proxy` to restrict who can provide them. For PROXY protocol frontends, use `protocol_proxy`.

See [HTTP host directives](/docs/v3/configuration/http-host), [Rate limiting](/docs/v3/use-cases/rate-limiting), [Access control](/docs/v3/use-cases/access-control), and [Configuration: conditionals](/docs/v3/configuration/conditionals).

## Configuration mistakes

### Wrong nesting

KDL placement matters. A directive in the wrong block can look valid but not apply where you expect.

### Missing directive

Common examples:

- Using `root` when you meant `proxy`
- Missing `fcgi_php` or `cgi` plus `extension` for PHP
- Missing `location` separation between static files and API traffic

### Unexpected conditional behavior

All subconditions in one `condition` block must pass. If one fails, the condition fails.

### Quick isolation method

Start with a minimal known-good config, validate it with `ferron validate -c ferron.conf`, then add one change at a time.

See [Getting started](/docs/v3/getting-started), [Configuration syntax](/docs/v3/configuration/syntax), and [Core directives](/docs/v3/configuration/core-directives).

## Observability and logs

### How to enable debug visibility

Ferron does not have a single global debug mode. For troubleshooting visibility:

- Ensure `error_log` is enabled.
- Ensure access logging is enabled via `log`.
- Temporarily enable `rewrite_log` when debugging rewrites.
- Use OTLP export if you already have centralized tracing or metrics.

### How to read logs effectively

- Reproduce one request at a time.
- Match request path and status in access logs with timestamps in error logs.
- Check whether the failure happened before upstream routing, during upstream connection, or in local file or routing logic.

### Common access log interpretations

- `404 Not Found` - often wrong `root`, wrong path rewrite, or backend route mismatch.
- `403 Forbidden` - often an access-control rule or filesystem permission issue.
- `502 Bad Gateway` - upstream connection or handshake failure.
- `504 Gateway Timeout` - upstream reachable but too slow or unresponsive.

## Notes and troubleshooting

- If you are asking for help, include the Ferron version, your deployment type, a minimal config snippet, and the relevant error-log lines.
- If the logs show unrelated random paths or scanners, compare them against the access log before changing your config.
- If you suspect a Ferron bug, open an issue on [GitHub](https://github.com/ferronweb/ferron/issues).
