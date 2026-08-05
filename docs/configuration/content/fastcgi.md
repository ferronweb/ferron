---
title: "Configuration: FastCGI support"
description: "Server-side FastCGI protocol support for backend application servers with connection pooling and keepalive."
---

This page documents the `fcgi` directive, which configures FastCGI support in Ferron. FastCGI enables dynamic content by forwarding requests to external application servers over TCP or Unix sockets. It also supports connection pooling and keepalive.

## `fcgi`

```ferron
example.com {
    fcgi {
        backend tcp://127.0.0.1:4000
        environment "APP_ENV" "production"
    }
}
```

The `fcgi` directive enables FastCGI protocol support. You can write it as a boolean flag to enable with defaults. You can write a backend URL to set the target. You can also write it as a block with nested directives to customize behavior.

| Form                         | Description                                                                                 |
| ---------------------------- | ------------------------------------------------------------------------------------------- |
| `fcgi`                       | Enables FastCGI with all defaults. Set the backend URL with the `backend` nested directive. |
| `fcgi true`                  | Explicitly enables FastCGI. Set the backend URL with the `backend` nested directive.        |
| `fcgi false`                 | Disables FastCGI for the current scope.                                                     |
| `fcgi <url: string>`         | Enables FastCGI and sets the backend URL directly.                                          |
| `fcgi <url: string> { ... }` | Enables FastCGI, sets the backend URL, and configures nested directives.                    |
| `fcgi { ... }`               | Enables FastCGI and configures nested directives.                                           |

### `backend`

| Nested directive | Arguments       | Description                                                                                                                                                                                               | Default |
| ---------------- | --------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------- |
| `backend`        | `<url: string>` | This directive specifies the FastCGI backend server URL. Supports TCP URLs (`tcp://host:port`) and Unix socket URLs (`unix:///path/to/socket`). The URL supports interpolation syntax for dynamic values. | none    |

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

> [!note]
>
> - TCP URLs must include both host and port (for example, `tcp://127.0.0.1:9000`).
> - Unix socket paths must be absolute paths.
> - When a connection failure occurs (connection refused, host unreachable, etc.), Ferron logs an error and returns a `503 Service Unavailable` response.
> - If a FastCGI server returns a non-zero status, Ferron logs a `WARN` message and returns a `500 Internal Server Error` response. Ferron trims stderr output before logging it as a warning.

### `extension`

| Nested directive | Arguments  | Description                                                                                                                                                                                                                                                       | Default |
| ---------------- | ---------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------- |
| `extension`      | `<string>` | This directive registers a file extension that the FastCGI backend should process. The FastCGI backend handles files with these extensions when `pass` is `false`. You can specify this directive multiple times. Each invocation can accept multiple extensions. | none    |

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

> [!note]
>
> - Ferron matches extensions case-insensitively.
> - The FastCGI backend processes files with these extensions, regardless of their location in the document root.
> - When you use `fcgi_php` instead, Ferron registers `.php` automatically.

### `environment`

| Nested directive | Arguments                        | Description                                                                                                                                                                                                        | Default |
| ---------------- | -------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------- |
| `environment`    | `<name: string> <value: string>` | This directive sets a FastCGI environment variable passed to the backend server. The server resolves values with the same interpolation syntax as other directives. You can specify this directive multiple times. | none    |

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

> [!note]
>
> - Environment variables take precedence over any existing variables with the same name.
> - The `Proxy` header is automatically removed from the request to prevent the [httpoxy](https://httpoxy.org/) vulnerability.
> - Ferron always sets `SERVER_SOFTWARE`, `SERVER_NAME`, `SERVER_ADDR`, `SERVER_PORT`, `REQUEST_URI`, `QUERY_STRING`, `PATH_INFO`, `SCRIPT_NAME`, `AUTH_TYPE`, `REMOTE_USER`, and `SERVER_ADMIN` automatically.
> - Ferron sets the working directory to the directory containing the script file.

### `pass`

| Nested directive | Arguments             | Description                                                                                                                                                                                                                                               | Default |
| ---------------- | --------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------- |
| `pass`           | `<boolean: optional>` | This directive controls whether Ferron passes all requests to the FastCGI backend. When `true`, Ferron forwards all requests. When `false`, Ferron passes requests to the file-processing pipeline. This allows the `extension` directive to match files. | `true`  |

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

> [!note]
>
> - When `pass` is `false`, Ferron only invokes the FastCGI backend for files matching a registered extension.
> - This is useful for routing specific file types to the FastCGI backend while serving other files statically.

### `keepalive`

| Nested directive | Arguments             | Description                                                                                                                                                          | Default |
| ---------------- | --------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------- |
| `keepalive`      | `<boolean: optional>` | This directive enables connection keepalive to the FastCGI backend. When enabled, Ferron reuses connections across requests. This reduces connection setup overhead. | `false` |

**Configuration example:**

```ferron
example.com {
    fcgi {
        backend tcp://127.0.0.1:9000
        keepalive
    }
}
```

> [!note]
>
> - Ferron manages keepalive connections in a connection pool.
> - When you combine this with the `limit` directive, each upstream can have its own pool limit.
> - This is useful for high-traffic sites where connection setup overhead is significant.

## `fcgi_php`

```ferron
example.com {
    fcgi_php "unix:///run/php/php8.4-fpm.sock"
}
```

The `fcgi_php` directive is an alias for PHP FastCGI backends. It enables FastCGI and automatically registers the `.php` file extension. This is the recommended way to host PHP applications with PHP-FPM.

| Form                     | Description                                         |
| ------------------------ | --------------------------------------------------- |
| `fcgi_php <url: string>` | Enables PHP FastCGI with the specified backend URL. |
| `fcgi_php false`         | Disables PHP FastCGI for the current scope.         |

**Configuration example with TCP:**

```ferron
example.com {
    root /var/www/html
    fcgi_php "tcp://127.0.0.1:9000"
}
```

**Configuration example with Unix socket:**

```ferron
example.com {
    root /var/www/html
    fcgi_php "unix:///run/php/php8.4-fpm.sock"
}
```

> [!note]
>
> - `fcgi_php` automatically registers `.php` as a file extension.
> - Use `fcgi_php false` to disable PHP FastCGI for a specific scope.
> - For PHP-FPM over Unix sockets, make sure the Ferron process can access the socket. Check the owner, group, and mode in your PHP-FPM pool configuration.

## Connection pooling

Ferron manages FastCGI backend connections using a connection pool. This reduces the overhead of establishing new connections for each request.

### `fcgi_concurrent_conns`

| Directive               | Arguments                       | Description                                                                                                                       | Default |
| ----------------------- | ------------------------------- | --------------------------------------------------------------------------------------------------------------------------------- | ------- |
| `fcgi_concurrent_conns` | `<number: positive>` or `false` | This directive sets the global maximum number of concurrent FastCGI connections across all backends. Set to `false` for no limit. | `16384` |

**Configuration example:**

```ferron
fcgi_concurrent_conns 8192
```

**Configuration example with no limit:**

```ferron
fcgi_concurrent_conns false
```

> [!note]
>
> - This is a global setting that applies to all FastCGI backends.
> - Individual backends can also have their own per-upstream limits via the `limit` nested directive inside `fcgi`.
> - When the pool runs out of connections, new requests wait for a connection to become available.
> - Setting to `false` disables the global limit (unlimited concurrent connections).

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

| Variable          | Description                                                    |
| ----------------- | -------------------------------------------------------------- |
| `SERVER_SOFTWARE` | Always `Ferron`.                                               |
| `SERVER_NAME`     | Server hostname.                                               |
| `SERVER_ADDR`     | Local server address.                                          |
| `SERVER_PORT`     | Server port.                                                   |
| `REQUEST_METHOD`  | HTTP method.                                                   |
| `REQUEST_URI`     | Original request URI.                                          |
| `QUERY_STRING`    | Query string (empty string if none).                           |
| `PATH_INFO`       | Path info extracted from the request.                          |
| `SCRIPT_NAME`     | The script path relative to the document root.                 |
| `AUTH_TYPE`       | Authentication type from the `Authorization` header.           |
| `REMOTE_USER`     | Authenticated username, if available.                          |
| `SERVER_ADMIN`    | Server administrator email (from `admin_email` configuration). |
| `HTTPS`           | Set to `on` for encrypted connections.                         |

Additional variables that `environment` directives define override any variables with the same name.

> [!tip]
> FastCGI applications receive `REMOTE_USER` and `AUTH_TYPE` only when used alongside `http-basicauth`. For related configuration, see [Static file serving](/docs/v3/configuration/content/static-files), [URL rewriting](/docs/v3/configuration/routing/rewrite), and [HTTP headers and CORS](/docs/v3/configuration/content/headers).

## Authentication

When used alongside an authentication module (for example, `http-basicauth`), Ferron automatically populates the `AUTH_TYPE` and `REMOTE_USER` environment variables in the FastCGI request. Ferron extracts the authentication type from the `Authorization` header (for example, `Basic` or `Bearer`).

## Trace context injection

When a trace context exists for the request, Ferron automatically injects W3C Trace Context headers (`traceparent`, `tracestate`, and `baggage`) into the FastCGI request. Ferron maps these headers to standard CGI environment variables:

| Header        | FastCGI environment variable |
| ------------- | ---------------------------- |
| `traceparent` | `HTTP_TRACEPARENT`           |
| `tracestate`  | `HTTP_TRACESTATE`            |
| `baggage`     | `HTTP_BAGGAGE`               |

This works in both `pass true` and `pass false` modes. The FastCGI backend application can read the trace context headers. This enables end-to-end distributed tracing. For example, a PHP application can use the official OpenTelemetry SDK for PHP to read these headers. The application creates child spans automatically.

> [!info]
> You do not need per-module configuration. The system controls trace context injection globally based on whether a trace context exists. See [Tracing configuration](/docs/v3/configuration/observability/tracing) for details on trace generation and sampling.

## Observability

### Logs

- **`ERROR`**: logged when a connection to the FastCGI backend fails. The message includes the connection error details.
- **`WARN`**: logged when a FastCGI backend produces output on stderr. The message includes the trimmed stderr content.

### Structured logs

| Description (summary)       | Level | Attributes                                                               |
| --------------------------- | ----- | ------------------------------------------------------------------------ |
| FastCGI service unavailable | ERROR | `upstream.address` (string): backend server URL                          |
| FastCGI errors on stderr    | WARN  | `error.message` (string): trimmed stderr output from the FastCGI process |

### Metrics

| Metric                          | Type      | Attributes                                                        | Description                                                                   |
| ------------------------------- | --------- | ----------------------------------------------------------------- | ----------------------------------------------------------------------------- |
| `ferron.fcgi.requests`          | Counter   | None                                                              | Number of FastCGI requests processed                                          |
| `ferron.fcgi.failures`          | Counter   | `error.type` (`"service_unavailable"`), `ferron.fcgi.backend_url` | Number of FastCGI requests that failed before the backend returned a response |
| `ferron.fcgi.upstream.duration` | Histogram | `ferron.fcgi.backend_url`                                         | FastCGI upstream request processing time                                      |
| `ferron.fcgi.stderr_errors`     | Counter   | None                                                              | Number of FastCGI requests that produced non-empty stderr output              |

### Access log fields

The FastCGI module contributes the following fields to the HTTP access log line:

| Field                         | Type   | Description                  |
| ----------------------------- | ------ | ---------------------------- |
| `ferron.fcgi.backend_url`     | string | FastCGI backend URL.         |
| `ferron.fcgi.script_filename` | string | Script filename (file mode). |

### Trace spans

The FastCGI stage sets the following attributes on its `ferron.stage.fcgi_pass` span:

| Attribute                     | Type   | Description                                                                                     |
| ----------------------------- | ------ | ----------------------------------------------------------------------------------------------- |
| `http.response.status_code`   | int    | HTTP status code returned by the FastCGI backend.                                               |
| `ferron.fcgi.backend_url`     | string | URL of the FastCGI backend.                                                                     |
| `ferron.fcgi.script_filename` | string | Absolute path to the script on the backend filesystem, when available.                          |
| `error.type`                  | string | Error type on failure (for example, `service_unavailable`). This enables trace UI highlighting. |

## Examples

### PHP with PHP-FPM over a Unix socket

```ferron
example.com {
    root /var/www/html
    fcgi_php "unix:///run/php/php8.4-fpm.sock"
}
```

### PHP with PHP-FPM over TCP

```ferron
example.com {
    root /var/www/html
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
    root /var/www/html

    # Only .php files are processed by the FastCGI backend
    fcgi {
        backend tcp://127.0.0.1:9000
        pass false
        extension ".php"
    }

    # Other files are served statically
}
```
