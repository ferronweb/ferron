---
title: "Configuration: forwarded authentication"
description: "Send requests to an external authentication backend, with connection pooling, header copying, and multiple backends."
---

`auth_to` sends every request to an external backend before Ferron processes it. A `2xx` response lets the request continue through the pipeline. A `4xx` or `5xx` response goes to the client and stops the pipeline. This works with authentication proxies such as [Authelia](https://www.authelia.com/), [Keycloak](https://www.keycloak.org/), or a custom service.

## auth_to

```ferron
example.com {
    auth_to http://localhost:9091 {
        limit 50
        idle_timeout "30s"
        no_verification false

        request_header +X-Internal-Auth "shared-secret"

        copy X-Auth-User X-Auth-Roles
    }
}
```

| Nested directive   | Arguments     | Description                                                                                     | Default                 |
| ------------------ | ------------- | ----------------------------------------------------------------------------------------------- | ----------------------- |
| `url`              | `<string>`    | Backend server URL (`http://` or `https://`). Required if you do not provide it as an argument. | none                    |
| `unix`             | `<path>`      | Connect to the backend through a Unix domain socket instead of TCP.                             | TCP                     |
| `limit`            | `<number>`    | Maximum concurrent connections to this backend.                                                 | No limit (per upstream) |
| `idle_timeout`     | `<duration>`  | Keep-alive idle timeout. Connections idle longer than this expire from the pool.                | `60s`                   |
| `no_verification`  | `[bool]`      | Skip TLS certificate verification for HTTPS backends.                                           | `false`                 |
| `request_header`   | see below     | Add, replace, or remove a request header sent to the auth backend. Repeat for more headers.     | none                    |
| `copy`             | `<string>...` | Headers to copy from the auth response to the original request. Supports multiple headers.      | none                    |
| `last`             | `[bool]`      | Whether this is the last backend in the chain (no further verification).                        | `false`                 |
| `intercept_errors` | `[bool]`      | Whether to intercept upstream error responses and replace them with built-in error pages.       | `false`                 |
| `append_uri`       | `[bool]`      | Whether to append the URI to the backend URL.                                                   | `false`                 |
| `method`           | `<string>`    | HTTP method to use when forwarding requests to the auth backend.                                | GET                     |

When you enable `client_ip_from_header`, Ferron appends `X-Forwarded-For` to the existing chain instead of replacing it. Ferron removes the `Content-*` (by default), `Upgrade` and `Connection` headers from auth requests.

You can chain multiple backends. Set `last` to `true` on the final backend to terminate the chain.

### Backend URL

Pass the backend URL as an argument or in a nested `url` directive:

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

The auth request uses the same path and query string as the original request. If the backend is unreachable, Ferron blocks the request.

### Unix socket

```ferron
example.com {
    auth_to http://localhost/auth {
        unix /var/run/authelia/authelia.sock
    }
}
```

With `unix`, Ferron ignores the URL host for the connection. The host must still be present for the HTTP scheme.

### Connection limit

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

Each `auth_to` block has its own connection limit. You can define multiple blocks for different backends. Ferron uses the first matching configuration.

### Copy response headers

When authentication succeeds, Ferron can copy headers from the backend response to the original request. Use this to pass user identity, roles, or other metadata downstream:

```ferron
example.com {
    auth_to http://auth.example.com/auth {
        copy X-Auth-User X-Auth-Roles X-Auth-Email
    }

    # The copied headers are available in the request
    proxy http://backend:8080
}
```

Ferron matches headers by name. If the auth response contains a specified header, Ferron adds it to the original request and preserves multiple values.

### Transform request headers

`request_header` changes the request that Ferron sends to the auth backend. Ferron applies it before it injects its own forwarding headers.

```ferron
example.com {
    auth_to http://auth.example.com/auth {
        request_header +X-Internal-Auth "shared-secret"
        request_header -Authorization
        request_header X-Forwarded-Host "example.com"
    }
}
```

| Form                           | Behavior                                                           |
| ------------------------------ | ------------------------------------------------------------------ |
| `request_header +Name "value"` | Add the header (appends, allows duplicates).                       |
| `request_header -Name`         | Remove all instances of the header.                                |
| `request_header Name "value"`  | Replace the header (removes existing instances, sets a new value). |

The value supports `{{variable}}` interpolation, so you can forward request or host data:

```ferron
example.com {
    auth_to http://auth.example.com/auth {
        request_header +X-Original-Host "{{request.host}}"
    }
}
```

`request_header` affects only the request to the auth backend, not the original client request. To pass data from the auth response back to the original request, use `copy`.

## auth_to_concurrent_conns

`auth_to_concurrent_conns` is global-only. It sets the maximum concurrent connections across all forwarded authentication backends. The default is `16384`.

| Argument   | Description                                        |
| ---------- | -------------------------------------------------- |
| `<number>` | Maximum concurrent connections (positive integer). |
| `false`    | No limit (unbounded).                              |

```ferron
{
    auth_to_concurrent_conns 16384
}

example.com {
    auth_to http://auth.example.com
}
```

## Authentication flow

1. The stage parses the `auth_to` configuration for the request.
2. It builds an HTTP request with the original method, path, query string, and headers.
3. It applies the configured `request_header` transformations.
4. It adds the forwarding headers: `X-Forwarded-For`, `X-Forwarded-Proto`, `X-Forwarded-Uri`, `X-Forwarded-Method`, `X-Real-IP`, and `Forwarded`.
5. It sends the request to the backend through the connection pool.
6. On a `2xx` response, it copies the configured headers to the original request and continues the pipeline.
7. On a `4xx` or `5xx` response, it returns the backend response to the client. With `intercept_errors`, it returns a custom error response instead. The pipeline stops.

## Examples

### Pass user headers to the backend

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

### Backend with a self-signed certificate

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

> [!warning]
> `no_verification` disables certificate checks. Use it only for internal test environments. For production, install a certificate the backend can validate.

## Best practices

`ferron doctor` reports the following best-practice checks for directives on this page.

- `auth_to_concurrent_conns false`: Disabling the global forwarded-auth connection limit removes backpressure on authentication backends. Keep a bounded limit.
- `auth_to { no_verification }`: Disable TLS verification for the authentication backend only in tightly controlled internal test environments.

## Observability

### Access log fields

| Field                      | Type   | Description                                     |
| -------------------------- | ------ | ----------------------------------------------- |
| `ferron.fauth.result`      | string | Forwarded auth outcome: `success` or `failure`. |
| `ferron.fauth.backend_url` | string | Auth backend URL contacted.                     |

### Trace spans

The stage sets these attributes on its `ferron.stage.forwarded_auth` span:

| Attribute                   | Type   | Description                                                      |
| --------------------------- | ------ | ---------------------------------------------------------------- |
| `ferron.fauth.result`       | string | Authentication result: `success` or `failure`.                   |
| `ferron.fauth.backend_url`  | string | URL of the authentication backend.                               |
| `http.response.status_code` | int    | HTTP status code returned on authentication failure.             |
| `error.type`                | string | Set to `auth_failed` on failure, enabling trace UI highlighting. |
