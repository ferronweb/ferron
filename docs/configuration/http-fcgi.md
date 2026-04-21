---
title: "Configuration: FastCGI support"
description: "Server-side FastCGI protocol support for backend application servers with connection pooling and keepalive."
---

This page documents the `fcgi` directive for configuring Ferron's FastCGI support. FastCGI enables dynamic content by forwarding requests to external application servers over TCP or Unix sockets, with support for connection pooling and keepalive for improved performance.

## `fcgi`

```ferron
example.com {
    fcgi {
        backend tcp://127.0.0.1:4000
        environment "APP_ENV" "production"
    }
}
```

The `fcgi` directive enables FastCGI protocol support. It can be written as a boolean flag to enable with defaults, with a backend URL to set the target, or as a block with nested directives to customize behavior.

| Form | Description |
| --- | --- |
| `fcgi` | Enables FastCGI with all defaults. Backend URL must be set via the `backend` nested directive. |
| `fcgi true` | Explicitly enables FastCGI. Backend URL must be set via the `backend` nested directive. |
| `fcgi false` | Disables FastCGI for the current scope. |
| `fcgi <url: string>` | Enables FastCGI and sets the backend URL directly. |
| `fcgi <url: string> { ... }` | Enables FastCGI, sets the backend URL, and configures nested directives. |
| `fcgi { ... }` | Enables FastCGI and configures nested directives. |

### `backend`

| Nested directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `backend` | `<url: string>` | This directive specifies the FastCGI backend server URL. Supports TCP URLs (`tcp://host:port`) and Unix socket URLs (`unix:///path/to/socket`). The URL supports interpolation syntax for dynamic values. | — |

**Configuration example:**

```ferron
example.com {
    fcgi {
        backend tcp://127.0.0.1:9000
    }
}
```

**Configuration example with Unix socket:**

```ferron
example.com {
    fcgi {
        backend unix:///run/php/php8.4-fpm.sock
    }
}
```

**Notes:**

- TCP URLs must include both host and port (e.g., `tcp://127.0.0.1:9000`).
- Unix socket paths must be absolute paths.
- When a connection failure occurs (connection refused, host unreachable, etc.), Ferron logs an error and returns a `503 Service Unavailable` response.

### `extension`

| Nested directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `extension` | `<string>` | This directive registers a file extension that should be processed by the FastCGI backend. Files with these extensions are handled by the FastCGI backend when `pass` is `false`. This directive can be specified multiple times, and each invocation can accept multiple extensions. | — |

**Configuration example:**

```ferron
example.com {
    fcgi {
        backend tcp://127.0.0.1:9000
        extension ".php"
        extension ".php5" ".php7"
    }
}
```

**Notes:**

- Extensions are matched case-insensitively.
- Files with these extensions are processed by the FastCGI backend regardless of their location in the document root.
- When `fcgi_php` is used instead, `.php` is registered automatically.

### `environment`

| Nested directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `environment` | `<name: string> <value: string>` | This directive sets a FastCGI environment variable passed to the backend server. Values are resolved with the same interpolation syntax as other directives. This directive can be specified multiple times. | — |

**Configuration example:**

```ferron
example.com {
    fcgi {
        backend tcp://127.0.0.1:9000
        environment "APP_ENV" "production"
        environment "APP_SECRET" "{{env.APP_SECRET}}"
        environment "RUBY_VERSION" "3.3"
    }
}
```

**Notes:**

- Environment variables take precedence over any existing variables with the same name.
- The `Proxy` header is automatically removed from the request to prevent the [httpoxy](https://httpoxy.org/) vulnerability.
- Ferron always sets `SERVER_SOFTWARE`, `SERVER_NAME`, `SERVER_ADDR`, `SERVER_PORT`, `REQUEST_URI`, `QUERY_STRING`, `PATH_INFO`, `SCRIPT_NAME`, `AUTH_TYPE`, `REMOTE_USER`, and `SERVER_ADMIN` automatically.

### `pass`

| Nested directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `pass` | `<boolean: optional>` | This directive controls whether all requests are passed to the FastCGI backend. When `true`, all requests are forwarded. When `false`, requests are passed to the file-processing pipeline, allowing the `extension` directive to match files. | `true` |

**Configuration example:**

```ferron
example.com {
    fcgi {
        backend tcp://127.0.0.1:9000
        pass false
        extension ".php"
    }
}
```

**Notes:**

- When `pass` is `false`, the FastCGI backend is only invoked for files matching a registered extension.
- This is useful for routing specific file types to the FastCGI backend while serving other files statically.

### `keepalive`

| Nested directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `keepalive` | `<boolean: optional>` | This directive enables connection keepalive to the FastCGI backend. When enabled, connections are reused across requests, reducing connection setup overhead. | `false` |

**Configuration example:**

```ferron
example.com {
    fcgi {
        backend tcp://127.0.0.1:9000
        keepalive
    }
}
```

**Notes:**

- Keepalive connections are managed in a connection pool.
- When combined with the `limit` directive, each upstream can have its own pool limit.
- Useful for high-traffic sites where connection setup overhead is significant.

## `fcgi_php`

```ferron
example.com {
    fcgi_php "unix:///run/php/php8.4-fpm.sock"
}
```

The `fcgi_php` directive is a convenience alias for PHP FastCGI backends. It enables FastCGI and automatically registers the `.php` file extension. This is the recommended way to host PHP applications with PHP-FPM.

| Form | Description |
| --- | --- |
| `fcgi_php <url: string>` | Enables PHP FastCGI with the specified backend URL. |
| `fcgi_php false` | Disables PHP FastCGI for the current scope. |

**Configuration example with TCP:**

```ferron
example.com {
    root "/var/www/html"
    fcgi_php "tcp://127.0.0.1:9000"
}
```

**Configuration example with Unix socket:**

```ferron
example.com {
    root "/var/www/html"
    fcgi_php "unix:///run/php/php8.4-fpm.sock"
}
```

**Notes:**

- `fcgi_php` automatically registers `.php` as a file extension.
- `fcgi_php false` can be used to disable PHP FastCGI for a specific scope.
- For PHP-FPM over Unix sockets, ensure the socket is accessible by the Ferron process (check owner/group/mode in your PHP-FPM pool configuration).

## Connection pooling

Ferron manages FastCGI backend connections using a connection pool. This reduces the overhead of establishing new connections for each request.

### `fcgi_concurrent_conns`

| Directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `fcgi_concurrent_conns` | `<number: positive>` or `false` | This directive sets the global maximum number of concurrent FastCGI connections across all backends. Set to `false` for no limit. | `16384` |

**Configuration example:**

```ferron
fcgi_concurrent_conns 8192
```

**Configuration example with no limit:**

```ferron
fcgi_concurrent_conns false
```

**Notes:**

- This is a global setting that applies to all FastCGI backends.
- Individual backends can also have their own per-upstream limits via the `limit` nested directive inside `fcgi`.
- When the pool is exhausted, new requests wait for a connection to become available.
- Setting to `false` disables the global limit (unlimited concurrent connections).

### Per-upstream connection limits

When using multiple FastCGI backends, you can set individual connection limits for each:

```ferron
example.com {
    fcgi {
        backend tcp://127.0.0.1:9000
        limit 64
    }
}
```

The `limit` directive sets the maximum number of concurrent connections for that specific backend.

## Environment variables

Ferron automatically sets the following FastCGI environment variables:

| Variable | Description |
| --- | --- |
| `SERVER_SOFTWARE` | Always `Ferron`. |
| `SERVER_NAME` | Server hostname. |
| `SERVER_ADDR` | Local server address. |
| `SERVER_PORT` | Server port. |
| `REQUEST_METHOD` | HTTP method. |
| `REQUEST_URI` | Original request URI. |
| `QUERY_STRING` | Query string (empty string if none). |
| `PATH_INFO` | Path info extracted from the request. |
| `SCRIPT_NAME` | The script path relative to the document root. |
| `AUTH_TYPE` | Authentication type from the `Authorization` header. |
| `REMOTE_USER` | Authenticated username, if available. |
| `SERVER_ADMIN` | Server administrator email (from `admin_email` configuration). |
| `HTTPS` | Set to `on` when the connection is encrypted. |

Additional variables set by `environment` directives override any automatically set variables with the same name.

## Authentication

When used alongside an authentication module (e.g., `http-basicauth`), Ferron automatically populates the `AUTH_TYPE` and `REMOTE_USER` environment variables in the FastCGI request. The authentication type is extracted from the `Authorization` header (e.g., `Basic` or `Bearer`).

## Observability

### Logs

- **`ERROR`**: logged when a connection to the FastCGI backend fails. The message includes the connection error details.
- **`WARN`**: logged when a FastCGI backend produces output on stderr. The message includes the trimmed stderr content.

## Examples

### PHP with PHP-FPM over a Unix socket

```ferron
example.com {
    root "/var/www/html"
    fcgi_php "unix:///run/php/php8.4-fpm.sock"
}
```

### PHP with PHP-FPM over TCP

```ferron
example.com {
    root "/var/www/html"
    fcgi_php "tcp://127.0.0.1:9000"
}
```

### FastCGI with environment variables

```ferron
example.com {
    fcgi {
        backend tcp://127.0.0.1:9000
        environment "APP_ENV" "production"
        environment "APP_SECRET" "{{env.APP_SECRET}}"
    }
}
```

### FastCGI with keepalive and connection limits

```ferron
example.com {
    fcgi {
        backend tcp://127.0.0.1:9000
        keepalive
        limit 64
        extension ".php"
    }
}
```

### FastCGI with selective file routing

```ferron
example.com {
    root "/var/www/html"

    # Only .php files are processed by the FastCGI backend
    fcgi {
        backend tcp://127.0.0.1:9000
        pass false
        extension ".php"
    }

    # Other files are served statically
}
```

## Notes and troubleshooting

- When a connection to the FastCGI backend fails, Ferron returns a `503 Service Unavailable` response and logs an error message.
- For TCP backends, ensure the host and port are specified in the URL (e.g., `tcp://127.0.0.1:9000`).
- For Unix socket backends, the path must be absolute (e.g., `unix:///run/php/php8.4-fpm.sock`).
- The `Proxy` header is always removed to prevent the [httpoxy](https://httpoxy.org/) vulnerability.
- Ferron sets `SERVER_SOFTWARE` to `Ferron` automatically.
- For authentication integration, FastCGI scripts receive `REMOTE_USER` and `AUTH_TYPE` only when used alongside a module like `http-basicauth` that sets `ctx.auth_user`.
- For static file serving alongside FastCGI, see [Static file serving](/docs/v3/configuration/static-content).
- For URL rewriting, see [URL rewriting](/docs/v3/configuration/http-rewrite).
- For response headers and CORS, see [HTTP headers and CORS](/docs/v3/configuration/http-headers).
- For PHP hosting use cases, see [PHP hosting](/docs/v3/use-cases/php).
- For the complete `fcgi` directive reference, see [Configuration: FastCGI support](/docs/v3/configuration/http-fcgi).
