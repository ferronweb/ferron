---
title: "Configuration: HTTP buffering"
description: "Buffer request and response bodies to protect backends and bound memory use."
---

`buffer_request` and `buffer_response` collect a body up to a size limit before Ferron passes it on. Request buffering protects backends from Slowloris-style slow uploads. Response buffering bounds memory use and makes delivery to clients more even.

```ferron
example.com {
    buffer_request 8192
    buffer_response 32768
}
```

Both directives take a buffer size in bytes. You can set them at global or HTTP host scope. Ferron buffers up to the limit and then streams the rest, so bodies larger than the limit still work. Both directives are disabled by default.

| Directive         | Arguments | Description                                  | Default  |
| ----------------- | --------- | -------------------------------------------- | -------- |
| `buffer_request`  | `<int>`   | Buffer size in bytes for request bodies.     | disabled |
| `buffer_response` | `<int>`   | Buffer size in bytes for response bodies.    | disabled |

Typical values range from 4 KB to 64 KB. A small buffer provides little benefit. A large buffer uses more memory per active connection. Monitor memory under high concurrency.

To disable inherited buffering at one scope, set the directive to `0`. Remove the directive when no parent scope configures it.

## Request buffering

- Ferron collects request body frames up to the limit.
- If the body is smaller than the limit, Ferron collects the whole body before later stages run, including reverse proxy and authentication.
- If the body is larger, downstream stages get a chained stream: the buffered frames, then the remaining body.
- If the request has an `Incremental: ?1` header, Ferron returns `501 Not Implemented`.
- Ferron keeps non-data frames such as trailing headers and stops further collection.

> [!important]
> Request buffering is one layer of Slowloris defense. Also set the `timeout` directive to enforce connection timeouts. See [Core directives](../server/core-directives.md).

## Response buffering

- Ferron collects response body frames up to the limit.
- The body splits into buffered frames and a remaining stream. The remaining stream exists only when the body exceeds the limit.
- Ferron reassembles the buffered frames and sends them as a chained stream.
- If the response has an `Incremental: ?1` or `X-Accel-Buffering: no` header, Ferron skips buffering.
- Only custom responses from the pipeline are buffered. Built-in error responses are not.

## Pipeline position

The buffer stage runs early. It runs after URL rewriting and before rate limiting, authentication, caching, reverse proxy, and static file serving. Request bodies are therefore buffered before they reach a backend handler.

## Observability

### Trace spans

The request buffer stage sets these attributes on its `ferron.stage.http_buffer` span.

| Attribute                | Type | Description                                 |
| ------------------------ | ---- | ------------------------------------------- |
| `ferron.buffer.capacity` | int  | Configured buffer capacity in bytes.        |
| `ferron.buffer.size`     | int  | Actual buffered request body size in bytes. |
