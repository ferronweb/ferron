---
title: "Configuration: placeholders"
description: "Environment variable interpolation and request/logging placeholders available across headers, subconditions, proxy directives, and log formats."
---

This page describes placeholder variables that can be used in KDL directives. There are two kinds: **environment variable placeholders** (resolved when the configuration file is loaded) and **request/logging placeholders** (resolved per request).

## Environment variable placeholders

Environment variable placeholders let you reference environment variables directly in your KDL configuration. They are resolved when the server starts, so the values are baked into the parsed configuration.

### Syntax

```text
{env:VARIABLE_NAME}
```

### Behavior

- If the environment variable exists, its value is substituted in place of the placeholder.
- If the environment variable is missing or unset, the placeholder text is kept as-is (for example, `{env:MY_VAR}`).
- Unknown placeholder kinds (anything other than `env`) are also kept as-is.

### Examples

Given these environment variables:

```text
export APP_ROOT="/var/www/app"
export DB_HOST="db.example.com"
```

You can use them in a KDL configuration like this:

```kdl
site {
  root "{env:APP_ROOT}/public"

  reverseProxy {
    to "http://{env:DB_HOST}:5432"
  }
}
```

At load time, these resolve to:

```kdl
site {
  root "/var/www/app/public"

  reverseProxy {
    to "http://db.example.com:5432"
  }
}
```

### Notes

- Environment variable placeholders work in any string value throughout the KDL configuration file.
- They are resolved early, before the configuration is parsed, so they behave like compile-time constants.
- If you need a variable that changes at runtime (per request), use one of the [request placeholders](#placeholders) instead.

## Placeholders

Ferron supports the following placeholders for header values, subconditions, reverse proxying, and redirect destinations:

- `{path}` - the request URI with path (for example, `/index.html`)
- `{path_and_query}` - the request URI with path and query string (for example, `/index.html?param=value`)
- `{method}` - the request method
- `{version}` - the HTTP version of the request
- `{header:<header_name>}` - the value of a header with the specified name (`<header_name>` is replaced with the name of the header, for example, `Content-Type`)
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
- `{header:<header_name>}` - the value of a header with the specified name (`<header_name>` is replaced with the name of the header, for example, `Content-Type`; `-`, if header is missing)
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

These placeholders can be used both in `log_format` and in `log_json` additional property templates.
