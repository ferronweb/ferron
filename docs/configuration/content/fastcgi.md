---
title: "Configuration: FastCGI support"
description: "Forward requests to FastCGI backends over TCP or Unix sockets, with pooling and keepalive."
---

`fcgi` forwards requests to an external FastCGI application server. Ferron supports TCP and Unix socket backends, connection pooling, and keepalive.

A named host does not inherit `fcgi` or `fcgi_php` from the wildcard `*` host. Both still inherit from global defaults, and a `location` block inherits them from its host.

```ferron
example.com {
    fcgi {
        backend tcp://127.0.0.1:4000
        environment APP_ENV production
    }
}
```

You can repeat `fcgi` and `fcgi_php` directives to configure several backends.

| Form                         | Effect                                                          |
| ---------------------------- | --------------------------------------------------------------- |
| `fcgi`                       | Enables FastCGI with defaults. Set the backend with `backend`.  |
| `fcgi true`                  | Explicitly enables FastCGI.                                     |
| `fcgi false`                 | Disables FastCGI for the current scope.                         |
| `fcgi <url: string>`         | Enables FastCGI and sets the backend URL.                       |
| `fcgi <url: string> { ... }` | Enables FastCGI, sets the URL, and configures nested directives. |
| `fcgi { ... }`               | Enables FastCGI and configures nested directives.               |

## `backend`

| Nested directive | Arguments       | Description                                                        | Default |
| ---------------- | --------------- | ------------------------------------------------------------------ | ------- |
| `backend`        | `<url: string>` | Backend URL: `tcp://host:port` or `unix:///path/to/socket`.        | none    |

```ferron
example.com {
    fcgi {
        backend unix:///run/php/php8.4-fpm.sock
    }
}
```

The URL supports interpolation. TCP URLs must include a host and a port. Unix socket paths must be absolute.

If the connection fails, for example with connection refused or an unreachable host, Ferron logs an error and returns `503 Service Unavailable`. If the backend returns a non-zero status, Ferron logs a `WARN` message with the trimmed stderr output and returns `500 Internal Server Error`.

### `extension`

| Nested directive | Arguments  | Description                                                              | Default |
| ---------------- | ---------- | ------------------------------------------------------------------------ | ------- |
| `extension`      | `<string>` | Register a file extension for the backend. Repeatable, accepts several.   | none    |

```ferron
example.com {
    fcgi {
        backend tcp://127.0.0.1:9000
        extension ".php"
        extension ".php5" ".php7"
    }
}
```

Extensions match case-insensitively. The backend processes matching files anywhere in the document root, but only when `pass` is `false`. `fcgi_php` registers `.php` for you.

### `environment`

| Nested directive | Arguments                        | Description                                                    | Default |
| ---------------- | -------------------------------- | -------------------------------------------------------------- | ------- |
| `environment`    | `<name: string> <value: string>` | Set a FastCGI environment variable for the backend. Repeatable. | none    |

```ferron
example.com {
    fcgi {
        backend tcp://127.0.0.1:9000
        environment APP_ENV "production"
        environment APP_SECRET "{{env.APP_SECRET}}"
        environment RUBY_VERSION "3.3"
    }
}
```

Values use the same interpolation syntax as other directives. Variables set here override existing variables with the same name.

Ferron removes the `Proxy` request header to prevent the [httpoxy](https://httpoxy.org/) vulnerability. It sets `SERVER_SOFTWARE`, `SERVER_NAME`, `SERVER_ADDR`, `SERVER_PORT`, `REQUEST_URI`, `QUERY_STRING`, `PATH_INFO`, `SCRIPT_NAME`, `AUTH_TYPE`, `REMOTE_USER`, and `SERVER_ADMIN` automatically. The working directory is the directory that contains the script file.

### `pass`

| Nested directive | Arguments             | Description                                                          | Default |
| ---------------- | --------------------- | -------------------------------------------------------------------- | ------- |
| `pass`           | `<boolean: optional>` | Send all requests to the backend, or only files that match `extension`. | `true`  |

```ferron
example.com {
    fcgi {
        backend tcp://127.0.0.1:9000
        pass false
        extension ".php"
    }
}
```

With `pass true`, Ferron forwards every request to the backend. With `pass false`, requests go through the file pipeline, and only files with a registered extension reach the backend. This is how you send one file type to FastCGI and serve the rest statically.

### `keepalive`

| Nested directive | Arguments             | Description                                              | Default |
| ---------------- | --------------------- | -------------------------------------------------------- | ------- |
| `keepalive`      | `<boolean: optional>` | Reuse backend connections across requests.               | `false` |

```ferron
example.com {
    fcgi {
        backend tcp://127.0.0.1:9000
        keepalive
    }
}
```

Ferron keeps idle connections in a pool. Combine keepalive with `limit` to cap the pool for each upstream. Keepalive helps high-traffic sites where connection setup is expensive.

## `fcgi_php`

`fcgi_php` is a shortcut for PHP FastCGI backends. It enables FastCGI and registers the `.php` extension. Use it for PHP-FPM applications.

```ferron
example.com {
    root /var/www/html
    fcgi_php unix:///run/php/php8.4-fpm.sock
}
```

| Form                     | Effect                                                                                |
| ------------------------ | ------------------------------------------------------------------------------------- |
| `fcgi_php <url: string>` | Enables PHP FastCGI with the given backend URL.                                       |
| `fcgi_php false`         | Disables PHP FastCGI for the current scope.                                           |
| `fcgi_php { ... }`       | Configures PHP FastCGI without `extension` and `pass`, which `fcgi_php` sets itself.  |

This is roughly equivalent to the following configuration:

```ferron
example.com {
    root /var/www/html
    fcgi {
        backend tcp://php:9000/ # <-- Backend URL
        pass false
        extension ".php"
        # Other subdirectives...
    }
    index index.php index.html index.htm
}
```

```ferron
example.com {
    root /var/www/html
    fcgi_php tcp://127.0.0.1:9000
}
```

For a Unix socket, check that the Ferron process can access the socket. Check the owner, group, and mode in the PHP-FPM pool configuration.

## Connection pooling

Ferron reuses backend connections through a pool.

### `fcgi_concurrent_conns`

| Directive               | Arguments                       | Description                                              | Default |
| ----------------------- | ------------------------------- | -------------------------------------------------------- | ------- |
| `fcgi_concurrent_conns` | `<number: positive>` or `false` | Global concurrent connection limit. `false` means no limit. | `16384` |

```ferron
{
    fcgi_concurrent_conns 8192
}
```

This is a global setting for all FastCGI backends. Setting it to `false` removes the global limit. When the pool is full, new requests wait for a connection.

### Per-backend limits

Set `limit` inside an `fcgi` block to cap concurrent connections for that backend:

```ferron
example.com {
    fcgi {
        backend tcp://127.0.0.1:9000
        limit 64
    }
}
```

## Environment variables

Ferron sets these FastCGI environment variables automatically.

| Variable          | Description                                                     |
| ----------------- | --------------------------------------------------------------- |
| `SERVER_SOFTWARE` | Always `Ferron`.                                                |
| `SERVER_NAME`     | Server hostname.                                                |
| `SERVER_ADDR`     | Local server address.                                           |
| `SERVER_PORT`     | Server port.                                                    |
| `REQUEST_METHOD`  | HTTP method.                                                    |
| `REQUEST_URI`     | Original request URI.                                           |
| `QUERY_STRING`    | Query string, empty when there is none.                         |
| `PATH_INFO`       | Path info extracted from the request.                           |
| `SCRIPT_NAME`     | Script path relative to the document root.                      |
| `AUTH_TYPE`       | Authentication type from the `Authorization` header.            |
| `REMOTE_USER`     | Authenticated username, when available.                         |
| `SERVER_ADMIN`    | Administrator email from `admin_email`.                         |
| `HTTPS`           | `on` for encrypted connections.                                 |

An `environment` directive overrides an automatically set variable with the same name.

FastCGI applications receive `REMOTE_USER` and `AUTH_TYPE` only when used with `http-basicauth`. For related configuration, see [Static file serving](/docs/configuration/content/static-files), [URL rewriting](/docs/configuration/routing/rewrite), and [HTTP headers and CORS](/docs/configuration/content/headers).

## Authentication

With an authentication module such as `http-basicauth`, Ferron fills `AUTH_TYPE` and `REMOTE_USER` in the FastCGI request. It takes the authentication type from the `Authorization` header, for example `Basic` or `Bearer`.

## Trace context injection

When a trace context exists, Ferron injects the W3C Trace Context headers into the FastCGI request and maps them to CGI-style variables.

| Header        | FastCGI environment variable |
| ------------- | ---------------------------- |
| `traceparent` | `HTTP_TRACEPARENT`           |
| `tracestate`  | `HTTP_TRACESTATE`            |
| `baggage`     | `HTTP_BAGGAGE`               |

This works with `pass true` and `pass false`. The backend application can read these headers and create child spans. For example, a PHP application with the official OpenTelemetry SDK for PHP gets end-to-end tracing.

No per-module configuration is required. Trace context injection depends on whether a trace context exists. See [Tracing configuration](/docs/configuration/observability/tracing) for trace generation and sampling.

## Observability

### Logs

- `ERROR`: a connection to the backend fails. The message includes the connection error.
- `WARN`: the backend writes to stderr. The message includes the trimmed stderr output.

| Event                       | Level | Attributes                                                               |
| --------------------------- | ----- | ------------------------------------------------------------------------ |
| FastCGI service unavailable | ERROR | `upstream.address` (string): backend server URL.                         |
| FastCGI errors on stderr    | WARN  | `error.message` (string): trimmed stderr output from the FastCGI process. |

### Metrics

| Metric                          | Type      | Attributes                                                        | Description                                                    |
| ------------------------------- | --------- | ----------------------------------------------------------------- | -------------------------------------------------------------- |
| `ferron.fcgi.requests`          | Counter   | none                                                              | FastCGI requests processed.                                    |
| `ferron.fcgi.failures`          | Counter   | `error.type` (`service_unavailable`), `ferron.fcgi.backend_url`   | Requests that failed before the backend responded.             |
| `ferron.fcgi.upstream.duration` | Histogram | `ferron.fcgi.backend_url`                                         | Backend request processing time.                               |
| `ferron.fcgi.stderr_errors`     | Counter   | none                                                              | Requests with non-empty stderr output.                         |

### Access log fields

| Field                         | Type   | Description                  |
| ----------------------------- | ------ | ---------------------------- |
| `ferron.fcgi.backend_url`     | string | FastCGI backend URL.         |
| `ferron.fcgi.script_filename` | string | Script filename in file mode. |

### Trace spans

The FastCGI stage sets these attributes on its `ferron.stage.fcgi_pass` span.

| Attribute                     | Type   | Description                                                                    |
| ----------------------------- | ------ | ------------------------------------------------------------------------------ |
| `http.response.status_code`   | int    | Status code returned by the backend.                                           |
| `ferron.fcgi.backend_url`     | string | Backend URL.                                                                   |
| `ferron.fcgi.script_filename` | string | Absolute script path on the backend filesystem, when available.                |
| `error.type`                  | string | Error type on failure, such as `service_unavailable`. Highlights the span in the trace UI. |
