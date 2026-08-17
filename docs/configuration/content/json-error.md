---
title: "Configuration: JSON error responses"
description: "The `json_errors` directive for structured JSON error responses."
---

This page documents the `json_errors` directive for generating structured JSON error responses instead of HTML. This is useful for RESTful API endpoints where clients expect machine-readable error bodies.

## Directives

### JSON errors

- `json_errors <enabled: bool>` (`http-jsonerror`)
  - When `true`, Ferron returns HTTP error responses (4xx, 5xx) with a JSON body instead of an HTML error page. Default: `false`

#### Block options

| Option     | Arguments                 | Description                                                                                                                             | Default         |
| ---------- | ------------------------- | --------------------------------------------------------------------------------------------------------------------------------------- | --------------- |
| `format`   | `"problem"` \| `"simple"` | Output format. `"problem"` uses RFC 9457 Problem Details (`application/problem+json`). `"simple"` uses plain JSON (`application/json`). | `"problem"`     |
| `type_uri` | `<string>`                | URI for the `type` field in RFC 9457 format. Ferron replaces the `{status}` placeholder with the HTTP status code.                      | `"about:blank"` |
| `trace_id` | `<bool>`                  | Include the trace ID of the request in the response when available.                                                                     | `true`          |

**Configuration example:**

```ferron
example.com {
    json_errors
}
```

#### RFC 9457 Problem Details format

With `format "problem"` (default), responses use `Content-Type: application/problem+json` and follow the RFC 9457 structure:

```ferron
example.com {
    json_errors {
        type_uri "https://api.example.com/errors/{status}"
    }
}
```

Example response for a 404 error:

```json
{
  "type": "https://api.example.com/errors/404",
  "title": "Not Found",
  "status": 404,
  "detail": "The requested resource wasn't found.",
  "trace_id": "4bf92f3577b34da6a3ce929d0e0e4736"
}
```

#### Simple JSON format

With `format "simple"`, responses use `Content-Type: application/json` with a minimal structure:

```ferron
example.com {
    json_errors {
        format "simple"
    }
}
```

Example response:

```json
{
  "error": "Not Found",
  "status": 404,
  "detail": "The requested resource wasn't found.",
  "trace_id": "4bf92f3577b34da6a3ce929d0e0e4736"
}
```

#### Scope to API paths

Use `location` blocks to enable JSON errors only for API endpoints:

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

When you enable `json_errors`, the JSON error stage runs **before** the `error_page` stage, which serves custom HTML error pages. This means:

- JSON errors take precedence over `error_page` file-based error pages
- With JSON errors enabled, Ferron never reaches the built-in HTML error page fallback
- To use both HTML and JSON error pages for different paths, use `location` blocks

## Observability

### Trace spans

The stage sets the following attributes on its `ferron.stage.json_error` span:

| Attribute                       | Type   | Description                               |
| ------------------------------- | ------ | ----------------------------------------- |
| `ferron.json_error.format`      | string | Output format (`"problem"` or `"simple"`) |
| `ferron.json_error.status_code` | i64    | HTTP status code of the error response    |
