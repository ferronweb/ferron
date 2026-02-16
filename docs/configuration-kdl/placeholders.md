---
title: "Configuration: placeholders"
description: "Request and logging placeholders available across headers, subconditions, proxy directives, and log formats."
---

This page lists placeholder variables that can be used in KDL directives for routing, proxying, conditions, and logging.

## Placeholders

Ferron supports the following placeholders for header values, subconditions, reverse proxying, and redirect destinations:

- `{path}` - the request URI with path (for example, `/index.html`)
- `{path_and_query}` - the request URI with path and query string (for example, `/index.html?param=value`)
- `{method}` - the request method
- `{version}` - the HTTP version of the request
- `{header:<header_name>}` - the header value of the request URI
- `{scheme}` - the scheme of the request URI (`http` or `https`), applicable only for subconditions, reverse proxying and redirect destinations.
- `{client_ip}` - the client IP address, applicable only for subconditions, reverse proxying and redirect destinations.
- `{client_port}` - the client port number, applicable only for subconditions, reverse proxying and redirect destinations.
- `{client_ip_canonical}` (Ferron 2.3.0 or newer) - the client IP address in canonical form (IPv4-mapped IPv6 addresses, like `::ffff:127.0.0.1`, are converted to IPv4, like `127.0.0.1`), applicable only for subconditions, reverse proxying and redirect destinations.
- `{server_ip}` - the server IP address, applicable only for subconditions, reverse proxying and redirect destinations.
- `{server_port}` - the server port number, applicable only for subconditions, reverse proxying and redirect destinations.
- `{server_ip_canonical}` (Ferron 2.3.0 or newer) - the server IP address in canonical form (IPv4-mapped IPv6 addresses, like `::ffff:127.0.0.1`, are converted to IPv4, like `127.0.0.1`), applicable only for subconditions, reverse proxying and redirect destinations.

## Log placeholders

Ferron 2.0.0 and newer supports the following placeholders for access logs:

- `{path}` - the request URI with path (for example, `/index.html`)
- `{path_and_query}` - the request URI with path and query string (for example, `/index.html?param=value`)
- `{method}` - the request method
- `{version}` - the HTTP version of the request
- `{header:<header_name>}` - the header value of the request URI (`-`, if header is missing)
- `{scheme}` - the scheme of the request URI (`http` or `https`).
- `{client_ip}` - the client IP address.
- `{client_port}` - the client port number.
- `{client_ip_canonical}` (Ferron 2.3.0 or newer) - the client IP address in canonical form (IPv4-mapped IPv6 addresses, like `::ffff:127.0.0.1`, are converted to IPv4, like `127.0.0.1`).
- `{server_ip}` - the server IP address.
- `{server_port}` - the server port number.
- `{server_ip_canonical}` (Ferron 2.3.0 or newer) - the server IP address in canonical form (IPv4-mapped IPv6 addresses, like `::ffff:127.0.0.1`, are converted to IPv4, like `127.0.0.1`).
- `{auth_user}` - the username of the authenticated user (`-`, if not authenticated)
- `{timestamp}` - the formatted timestamp of the entry
- `{status_code}` - the HTTP status code of the response
- `{content_length}` - the content length of the response (`-`, if not available)
