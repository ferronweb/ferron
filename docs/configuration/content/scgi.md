---
title: "Configuration: SCGI support"
description: "Forward requests to SCGI backends over TCP or Unix sockets."
---

`scgi` forwards requests to an external application server with the SCGI protocol. Ferron does not start local processes for SCGI.

A named host does not inherit `scgi` from the wildcard `*` host. It still inherits from global defaults, and a `location` block inherits `scgi` from its host.

```ferron
example.com {
    scgi {
        backend tcp://127.0.0.1:4000
        environment APP_ENV "production"
    }
}
```

| Form                         | Effect                                        |
| ---------------------------- | --------------------------------------------- |
| `scgi <url: string>`         | Enables SCGI and sets the backend URL.        |
| `scgi <url: string> { ... }` | Enables SCGI, sets the URL, and adds options. |
| `scgi { ... }`               | Enables SCGI and configures nested directives. |
| `scgi false`                 | Disables SCGI for the current scope.          |

### `backend`

| Nested directive | Arguments       | Description                                                        | Default |
| ---------------- | --------------- | ------------------------------------------------------------------ | ------- |
| `backend`        | `<url: string>` | Backend URL: `tcp://host:port` or `unix:///path/to/socket`.        | none    |

```ferron
example.com {
    scgi {
        backend unix:///var/run/app.sock
    }
}
```

TCP URLs must include a host and a port. Unix socket paths must be absolute.

When the connection fails, for example with connection refused or an unreachable host, Ferron logs an error and returns `503 Service Unavailable`.

### `environment`

| Nested directive | Arguments                        | Description                                                    | Default |
| ---------------- | -------------------------------- | -------------------------------------------------------------- | ------- |
| `environment`    | `<name: string> <value: string>` | Set an environment variable for the backend. Repeatable.        | none    |

```ferron
example.com {
    scgi {
        backend tcp://127.0.0.1:4000
        environment APP_ENV "production"
        environment APP_SECRET "{{env.APP_SECRET}}"
        environment RUBY_VERSION "3.3"
    }
}
```

Values use the same interpolation syntax as other directives. Variables set here override existing variables with the same name.

Ferron removes the `Proxy` request header to prevent the [httpoxy](https://httpoxy.org/) vulnerability. It sets `SERVER_SOFTWARE`, `SERVER_NAME`, `SERVER_ADDR`, `SERVER_PORT`, `REQUEST_URI`, `QUERY_STRING`, `PATH_INFO`, `SCRIPT_NAME`, `AUTH_TYPE`, `REMOTE_USER`, and `SERVER_ADMIN` automatically.

## Environment variables

Ferron sets these SCGI environment variables automatically.

| Variable          | Description                                                        |
| ----------------- | ------------------------------------------------------------------ |
| `SERVER_SOFTWARE` | Always `Ferron`.                                                   |
| `SERVER_NAME`     | Server hostname.                                                   |
| `SERVER_ADDR`     | Local server address.                                              |
| `SERVER_PORT`     | Server port.                                                       |
| `REQUEST_METHOD`  | HTTP method.                                                       |
| `REQUEST_URI`     | Original request URI.                                              |
| `QUERY_STRING`    | Query string, empty when there is none.                            |
| `PATH_INFO`       | Path info extracted from the request.                              |
| `SCRIPT_NAME`     | Script path relative to the document root.                         |
| `AUTH_TYPE`       | Authentication type from the `Authorization` header, such as `Basic` or `Bearer`. |
| `REMOTE_USER`     | Authenticated username, when available.                            |
| `SERVER_ADMIN`    | Administrator email from `admin_email`.                            |
| `HTTPS`           | `on` when the connection is encrypted.                             |

An `environment` directive overrides an automatically set variable with the same name.

SCGI applications receive `REMOTE_USER` and `AUTH_TYPE` only with a module that sets the authenticated user, such as `http-basicauth`.

## Authentication

With an authentication module such as `http-basicauth`, Ferron fills `AUTH_TYPE` and `REMOTE_USER` in the SCGI request. It takes the authentication type from the `Authorization` header, for example `Basic` or `Bearer`.

```ferron
example.com {
    root /var/www/html

    basic_auth {
        users {
            admin "$argon2id$v=19$m=19456,t=2,p=1$..."
        }
    }

    scgi tcp://127.0.0.1:4000
}
```

## Trace context injection

When a trace context exists, Ferron injects the W3C Trace Context headers into the SCGI request and maps them to CGI-style variables.

| Header        | SCGI environment variable |
| ------------- | ------------------------- |
| `traceparent` | `HTTP_TRACEPARENT`        |
| `tracestate`  | `HTTP_TRACESTATE`         |
| `baggage`     | `HTTP_BAGGAGE`            |

This gives end-to-end distributed tracing for SCGI backend applications. No per-module configuration is required. Trace context injection depends on whether a trace context exists. See [Tracing configuration](/docs/configuration/observability/tracing) for trace generation and sampling.

## Observability

### Logs

`ERROR`: a connection to the SCGI backend fails. The message includes the connection error.

| Event                    | Level | Attributes                                      |
| ------------------------ | ----- | ----------------------------------------------- |
| SCGI service unavailable | ERROR | `upstream.address` (string): backend server URL. |

### Metrics

| Metric                          | Type      | Attributes                                                        | Description                                       |
| ------------------------------- | --------- | ----------------------------------------------------------------- | ------------------------------------------------- |
| `ferron.scgi.requests`          | Counter   | none                                                              | SCGI requests processed.                          |
| `ferron.scgi.failures`          | Counter   | `error.type` (`service_unavailable`), `ferron.scgi.backend_url`   | Requests that failed before the backend responded. |
| `ferron.scgi.upstream.duration` | Histogram | `ferron.scgi.backend_url`                                         | Backend request processing time.                  |

### Access log fields

| Field                     | Type   | Description       |
| ------------------------- | ------ | ----------------- |
| `ferron.scgi.backend_url` | string | SCGI backend URL. |

### Trace spans

The SCGI stage sets these attributes on its `ferron.stage.scgi` span.

| Attribute                   | Type   | Description                                            |
| --------------------------- | ------ | ------------------------------------------------------ |
| `http.response.status_code` | int    | Status code returned by the SCGI backend.              |
| `ferron.scgi.backend_url`   | string | SCGI backend URL.                                      |
| `error.type`                | string | Error type on failure. Highlights the span in the trace UI. |
