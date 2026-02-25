---
title: Troubleshooting
description: "What to check when Ferron or your site stops working: quick triage for service, proxy, TLS, static files, access control, config, and logs."
---

When something goes wrong with Ferron or your website, start here. The goal is to quickly isolate whether the issue is service state, networking, TLS, routing, permissions, or configuration logic.

## Quick diagnostic checklist

1. Check the logs.
2. Confirm Ferron is running.
3. Test upstream directly (if you are reverse proxying).
4. Confirm port and firewall.
5. Restart Ferron after config changes.

## First checks

### Is Ferron running?

Check service status first (commands are given for common installation methods).

- Linux (systemd) - `sudo systemctl status ferron`
- Windows Server - `sc query ferron`
- Docker - `docker ps` (and `docker logs <container>`)

If Ferron is not running, restart it and re-check logs right away.

- Linux - `sudo systemctl restart ferron`
- Windows Server - `net stop ferron && net start ferron`

### Is the port correct?

- Standard Ferron config typically serves on ports `80` and `443`.
- `ferron serve` defaults to port `3000`.

If you are testing the wrong port, the server can be healthy but still appear down.

Relevant directives: `default_http_port`, `default_https_port`, `listen_ip`.
See [Configuration: core directives](/docs/configuration/core-directives).

### Is the firewall open?

Ensure inbound traffic is allowed for the ports you use (usually `80` and `443`).

- Local host firewall
- Cloud/VPS security groups
- Any external reverse proxy/load balancer policy

### Are logs enabled?

Use at least one access log output and one error log output.

- File logs - `log`, `error_log`
- Container-friendly streams - `log_stdout`, `error_log_stderr`
- Common package paths - `/var/log/ferron/access.log`, `/var/log/ferron/error.log` (Linux packages) and `%SystemDrive%\ferron\log\access.log`, `%SystemDrive%\ferron\log\error.log` (Windows installer)

See [Logging & observability](/docs/use-cases/logging-observability) and [Configuration: observability & logging](/docs/configuration/observability-logging).

### What does the log say?

Read the newest error lines first, then reproduce the request once and read again.

Common meanings:

- `connection refused` - target process is not listening on that host/port/socket.
- timeout errors - target is unreachable, blocked, overloaded, or too slow.
- TLS verification/certificate errors - upstream cert or trust chain problem.
- `permission denied` / file open errors - filesystem ownership/mode issue.

## Reverse proxy issues

### Can you reach the upstream directly?

From the same host where Ferron runs, test the upstream URL/socket directly.

If direct access fails, fix the upstream first; Ferron cannot proxy to an unreachable backend.

### Is the upstream bound to `127.0.0.1` instead of `0.0.0.0`?

If Ferron and upstream are in different hosts/containers/namespaces, upstream binding to `127.0.0.1` can make it unreachable externally. Bind to `0.0.0.0` (or the correct interface) when needed.

### Is TLS verification failing?

For HTTPS upstreams with private/self-signed certs:

- Prefer fixing trust (install proper CA chain) rather than disabling verification.
- `proxy_no_verification` can help confirm diagnosis, but keep it disabled in production unless you fully understand the risk.

### Timeout vs connection refused?

- `connection refused` - wrong port/host, or service down.
- timeout - route/firewall/DNS issue, or upstream accepted but did not respond in time.

See [Reverse proxying](/docs/use-cases/reverse-proxy) and [Configuration: reverse proxying](/docs/configuration/reverse-proxying).

## TLS issues

### Certificate not matching hostname

Ensure the requested hostname matches the certificate SAN/CN and the correct Ferron host block.

### Missing DNS records

For automatic TLS, DNS must point the hostname to the Ferron server.

### Port `80`/`443` blocked

ACME validation can fail when these ports are blocked by firewall, cloud policy, or proxy path.

### ACME challenge failing

- Use Let's Encrypt staging first - `auto_tls_letsencrypt_production #false`
- If behind an HTTPS-terminating proxy, use `auto_tls_challenge "http-01"` (or `"dns-01"`)
- Use `"dns-01"` for wildcard certificates
- Keep `auto_tls_cache` on persistent storage

### Could this be bot or scanner traffic?

On public servers, some TLS and HTTP errors are just background Internet scanning noise.

- Requests like `/wp-login.php`, `/.env`, or `/phpmyadmin` are common bot probes against non-WordPress/non-PHP sites.
- Some TLS handshake failures happen before HTTP request parsing, so they may appear in error logs without a matching access-log line.
- If legitimate traffic works and failures come from random IPs/paths, this is often not a Ferron misconfiguration.

If noise is high, reduce impact with targeted controls:

- Add route-specific `limit` rules for sensitive endpoints.
- Use `allow`/`block` rules where appropriate.
- Keep authentication and admin paths narrow and explicit.

See [Automatic TLS](/docs/use-cases/automatic-tls) and [Configuration: security & TLS](/docs/configuration/security-tls).

## Static file issues

### Incorrect root path

Verify `root` points to the directory containing deployed files for that host/location.

### Permissions

Ferron must be able to read files and traverse directories (`x` on directories for Unix and Unix-like systems).

### Path rewriting confusion

For SPAs, route fallback rewrites are commonly required. If rewrites are involved, enable `rewrite_log` while diagnosing.

See [Static file serving](/docs/use-cases/static-file-serving), [URL rewriting](/docs/use-cases/url-rewriting), and [Configuration: routing & URL processing](/docs/configuration/routing-url-processing).

## Rate limiting and access control

### Is a condition blocking traffic?

Check `status` rules (`allowed`, `not_allowed`), `allow`/`block`, and conditional logic (`condition`, `if`, `if_not`).

### Is client IP detection correct?

If Ferron is behind a proxy/load balancer and IP handling is wrong, access-control and rate-limit behavior can be wrong too.

### Is `trust_x_forwarded_for` configured?

Set `trust_x_forwarded_for` only when behind a trusted proxy path that sets `X-Forwarded-For`. For PROXY protocol frontends, use `protocol_proxy`.

See [Client IP](/docs/use-cases/client-ip), [Rate limiting](/docs/use-cases/rate-limiting), [Access control](/docs/use-cases/access-control), and [Configuration: conditionals](/docs/configuration/conditionals).

## Configuration mistakes

### Wrong nesting

KDL block placement matters. A directive in the wrong block can look valid but not apply where expected.

### Missing directive

Common examples:

- Using `root` when you intended `proxy`
- Missing `fcgi_php` (or `cgi` + `cgi_extension`) for PHP
- Missing `location` split between static files and API

### Unexpected conditional behavior

All subconditions in one `condition` block must pass. If one fails, the condition fails.

### Quick isolation method

Start with a minimal known-good config, confirm it works, then add one change at a time.

See [Getting started](/docs/getting-started), [Configuration: fundamentals](/docs/configuration/fundamentals), and [Configuration: examples](/docs/configuration/examples).

## Observability & logs

### How to enable debug logging

Ferron does not use a single global "debug mode" directive. For troubleshooting visibility:

- Ensure `error_log` (or `error_log_stderr`) is enabled
- Ensure access logs are enabled (`log`, `log_stdout`, or `log_stderr`)
- Enable `rewrite_log` temporarily when debugging rewrite behavior
- Optionally export logs/metrics/traces via OTLP

### How to read logs effectively

- Reproduce one request at a time.
- Match request path/status in access logs with timestamps in error logs.
- Check whether failure happened before upstream, during upstream connection, or in local file/routing logic.

### How to read Combined Log Format access lines (default)

Ferron's default access-log format is:

`{client_ip} - {auth_user} [{timestamp}] "{method} {path_and_query} {version}" {status_code} {content_length} "{header:Referer}" "{header:User-Agent}"`

This matches the default `log_format` directive in [Configuration: observability & logging](/docs/configuration/observability-logging).

Example line:

`::ffff:203.0.113.42 - - [19/Feb/2026:14:26:11 +0000] "GET /api/health HTTP/1.1" 200 27 "-" "curl/8.5.0"`

Field breakdown:

- `::ffff:203.0.113.42` - client IP (`client_ip`)
- first `-` - RFC 1413 identity placeholder (unused)
- second `-` - authenticated user (`auth_user`), `-` when not authenticated
- `[19/Feb/2026:14:26:11 +0000]` - request timestamp (`timestamp`)
- `"GET /api/health HTTP/1.1"` - method, path+query, and protocol version
- `200` - response status code
- `27` - response content length in bytes
- `"-"` - `Referer` header (missing in this example)
- `"curl/8.5.0"` - `User-Agent` header

Quick reading heuristics:

- Focus on status code first (`2xx` success, `3xx` redirects, `4xx` client-side/request-side, `5xx` server/upstream-side).
- Repeated `4xx` on unrelated paths from many IPs usually indicates bot scanning.
- Bursts of `502`/`504` for valid routes usually indicate upstream/backhaul problems.
- Correlate the same timestamp window in error logs for root cause details.

### How to interpret common errors

- `404 Not Found` - often wrong `root`, wrong path rewrite, or backend route mismatch.
- `403 Forbidden` - often access-control rule or filesystem permissions.
- `502 Bad Gateway` - upstream connection/handshake failure.
- `504 Gateway Timeout` - upstream reachable but too slow/unresponsive.

## Still stuck?

- Use [support options](/support) for troubleshooting help.
- If you suspect a Ferron bug, open an issue on [GitHub](https://github.com/ferronweb/ferron/issues).

When asking for help, include Ferron version (`ferron --version`), your deployment type, a minimal config snippet, and the relevant error-log lines.
