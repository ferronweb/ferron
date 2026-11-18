---
title: "Configuration: JSON error responses"
description: "Return structured JSON bodies for 4xx and 5xx errors instead of HTML pages."
---

`json_errors` makes Ferron return JSON error bodies. This suits APIs where clients expect machine-readable errors.

## `json_errors`

`json_errors <bool>` (`http-jsonerror`) returns JSON for HTTP 4xx and 5xx responses instead of an HTML page. Default: `false`.

| Block option | Arguments                 | Description                                                              | Default         |
| ------------ | ------------------------- | ------------------------------------------------------------------------ | --------------- |
| `format`     | `"problem"` \| `"simple"` | `problem` uses RFC 9457 Problem Details. `simple` uses plain JSON.       | `"problem"`     |
| `type_uri`   | `<string>`                | URI for the RFC 9457 `type` field. `{status}` becomes the status code.   | `"about:blank"` |
| `trace_id`   | `<bool>`                  | Include the request trace ID in the response, when available.            | `true`          |

```ferron
example.com {
    json_errors {
        type_uri "https://api.example.com/errors/{status}"
    }
}
```

### Problem Details format

With `format "problem"`, the default, responses use `Content-Type: application/problem+json` and follow RFC 9457. A 404 response looks like this:

```json
{
  "type": "https://api.example.com/errors/404",
  "title": "Not Found",
  "status": 404,
  "detail": "The requested resource wasn't found.",
  "trace_id": "4bf92f3577b34da6a3ce929d0e0e4736"
}
```

With the default `type_uri`, the `type` field is `about:blank`.

### Simple format

```ferron
example.com {
    json_errors {
        format simple
    }
}
```

With `format "simple"`, responses use `Content-Type: application/json` and a smaller structure:

```json
{
  "error": "Not Found",
  "status": 404,
  "detail": "The requested resource wasn't found.",
  "trace_id": "4bf92f3577b34da6a3ce929d0e0e4736"
}
```

### Scope to API paths

Use `location` blocks to enable JSON errors only for API paths:

```ferron
example.com {
    location /api {
        json_errors {
            type_uri "https://api.example.com/errors/{status}"
        }
    }

    # Other paths get standard HTML error pages
}
```

## Interaction with error pages

The JSON error stage runs before the `error_page` stage, which serves custom HTML error pages. JSON errors therefore take precedence over file-based `error_page` pages. With JSON errors enabled, Ferron does not reach the built-in HTML fallback.

To use HTML and JSON errors on different paths, separate them with `location` blocks.

## Observability

### Trace spans

The stage sets these attributes on its `ferron.stage.json_error` span.

| Attribute                       | Type   | Description                               |
| ------------------------------- | ------ | ----------------------------------------- |
| `ferron.json_error.format`      | string | Output format (`problem` or `simple`).    |
| `ferron.json_error.status_code` | i64    | HTTP status code of the error response.   |
