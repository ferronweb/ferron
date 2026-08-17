---
title: "Configuration: forwarded authentication"
description: "External authentication backend integration with connection pooling, header copying, and configurable backends."
---

This page documents the `auth_to` directive for configuring forwarded authentication. Forwarded authentication sends every incoming request to an external backend server for verification before the server processes the request. If the backend returns a success status (2xx), the request continues through the pipeline. If it returns a failure status (4xx/5xx), the backend sends its response directly to the client.

This pattern works with authentication proxies like [Authelia](https://www.authelia.com/), [Keycloak](https://www.keycloak.org/), or custom services.

## Directives

### `auth_to`

```ferron
example.com {
    auth_to http://localhost:9091 {
        limit 50
        idle_timeout "30s"
        no_verification false

        copy X-Auth-User X-Auth-Roles
    }
}
```

| Nested directive  | Arguments     | Description                                                                                               | Default                 |
| ----------------- | ------------- | --------------------------------------------------------------------------------------------------------- | ----------------------- |
| `url`             | `<string>`    | Backend server URL (http:// or https://). Required if you do not provide it as an argument.               | none                    |
| `unix`            | `<path>`      | Connect to the backend via Unix domain socket instead of TCP.                                             | TCP                     |
| `limit`           | `<number>`    | Maximum concurrent connections to this backend.                                                           | No limit (per upstream) |
| `idle_timeout`    | `<duration>`  | Keep-alive idle timeout for connections. Connections idle longer than this duration expire from the pool. | `60s`                   |
| `no_verification` | `[bool]`      | Skip TLS certificate verification for HTTPS backends.                                                     | `false`                 |
| `copy`            | `<string>...` | Headers to copy from the auth response back to the original request. Supports multiple headers.           | none                    |
| `last`            | `[bool]`      | Whether this is the last backend in the chain (no further verification).                                  | `false`                 |

> [!note]
>
> - When you enable `client_ip_from_header`, Ferron **appends** `X-Forwarded-For` to the existing chain rather than replacing it. Ferron removes Upgrade and Connection headers from auth requests.
> - The forwarded authentication module supports chaining multiple backends together. To terminate the chain, set `last` to `true`.

#### Backend URL

The `auth_to` directive requires a backend URL, specified either as a direct argument or via a nested `url` directive:

```ferron
example.com {
    # Direct argument form
    auth_to http://auth.example.com/auth

    # Nested form
    auth_to {
        url http://auth.example.com/auth
    }
}
```

> [!note]
> The forwarded auth request uses the **same path and query string** as the original request. If the backend is unreachable or returns a non-2xx status, Ferron blocks the request. Ferron returns the backend's response to the client.

#### Unix socket connections

To connect to a backend via Unix domain socket, use the `unix` nested directive:

```ferron
example.com {
    auth_to http://localhost/auth {
        unix /var/run/authelia/authelia.sock
    }
}
```

When you use `unix`, Ferron ignores the URL host for the actual connection. The host must still be present for the HTTP scheme.

#### Connection limits

Each backend can have its own connection limit via the `limit` directive:

```ferron
example.com {
    auth_to http://auth1.example.com {
        limit 100
        idle_timeout "60s"
    }
}

second.example.com {
    auth_to http://auth2.example.com {
        limit 50
    }
}
```

You can define multiple `auth_to` blocks for different backends. Ferron uses the first matching configuration.

#### Header copying

When authentication succeeds, Ferron can copy headers from the backend response to the original request. This is useful for passing user identity, roles, or other metadata downstream:

```ferron
example.com {
    auth_to http://auth.example.com/auth {
        copy X-Auth-User X-Auth-Roles X-Auth-Email
    }

    # The copied headers are available in the request
    proxy http://backend:8080
}
```

Ferron copies headers by name. If the auth response contains the specified header, Ferron adds it to the original request. Ferron preserves multiple values.

### Global connection limit

The global `auth_to_concurrent_conns` directive controls the maximum number of concurrent connections across all forwarded authentication backends:

```ferron
{
    auth_to_concurrent_conns 16384
}

example.com {
    auth_to http://auth.example.com
}
```

| Argument   | Description                                        |
| ---------- | -------------------------------------------------- |
| `<number>` | Maximum concurrent connections (positive integer). |
| `false`    | Disable the limit (unbounded).                     |

Default: `auth_to_concurrent_conns 16384`

## Authentication flow

1. The stage receives the incoming request and parses the `auth_to` configuration.
2. The stage constructs a new HTTP request using the original request's method, path, query string, and headers.
3. The stage adds standard forwarding headers (`X-Forwarded-For`, `X-Forwarded-Proto`, `X-Forwarded-Uri`, `X-Forwarded-Method`, `Forwarded`).
4. The stage sends the request to the authentication backend via the connection pool.
5. **On success (2xx)**: The stage copies configured headers from the response to the original request. The pipeline continues.
6. **On failure (4xx/5xx)**: The stage returns the backend's response directly to the client. The pipeline stops.

## Configuration examples

### Basic forwarded authentication

```ferron
example.com {
    auth_to http://auth.example.com/auth

    proxy http://backend:8080
}
```

### Authentication with user headers

```ferron
api.example.com {
    auth_to http://auth.example.com/validate {
        copy X-Auth-User X-Auth-Roles X-Auth-Email
    }

    proxy http://backend:8080 {
        request_header +X-User "{{request.header.x_auth_user}}"
        request_header +X-Roles "{{request.header.x_auth_roles}}"
    }
}
```

### Unix socket backend

```ferron
secure.example.com {
    auth_to http://localhost/auth {
        unix /var/run/authelia/authelia.sock
        limit 100
        idle_timeout "120s"
    }

    proxy http://backend:8080
}
```

### Self-signed certificate backend

```ferron
internal.example.com {
    auth_to https://auth.internal:8443/auth {
        no_verification
    }

    proxy https://backend:8443 {
        no_verification
    }
}
```

> [!tip]
> For authentication backends behind TLS, make sure the backend's certificate is valid or use `no_verification true` for development/testing.

### Disabling the global connection limit

```ferron
{
    auth_to_concurrent_conns false
}

example.com {
    auth_to http://auth.example.com
}
```

## Best practices

`ferron doctor` reports the following best-practice checks for directives on this page.

- **`auth_to_concurrent_conns false`**. Disabling the global forwarded-auth connection limit removes backpressure on authentication backends. Keep a bounded limit.
- **`auth_to { no_verification }`**. Disable TLS verification for the authentication backend only in tightly controlled internal test environments.

## Observability

### Access log fields

The forwarded authentication module contributes the following fields to the HTTP access log line:

| Field                      | Type   | Description                                     |
| -------------------------- | ------ | ----------------------------------------------- |
| `ferron.fauth.result`      | string | Forwarded auth outcome: `success` or `failure`. |
| `ferron.fauth.backend_url` | string | Auth backend URL contacted.                     |

### Trace spans

The forwarded authentication stage sets the following attributes on its `ferron.stage.forwarded_auth` span:

| Attribute                   | Type   | Description                                                      |
| --------------------------- | ------ | ---------------------------------------------------------------- |
| `ferron.fauth.result`       | string | Authentication result: `success` or `failure`.                   |
| `ferron.fauth.backend_url`  | string | URL of the authentication backend.                               |
| `http.response.status_code` | int    | HTTP status code returned on authentication failure.             |
| `error.type`                | string | Set to `auth_failed` on failure, enabling trace UI highlighting. |
