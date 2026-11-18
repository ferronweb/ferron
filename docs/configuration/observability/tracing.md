---
title: "Configuration: tracing"
description: "W3C Trace Context propagation, trace spans, trace sampling, and trace ID response headers."
---

Ferron generates and propagates W3C Trace Context and W3C Baggage, creates request spans, samples traces, and can return the trace ID in a response header.

## W3C Trace Context

Ferron supports `traceparent` and `tracestate` propagation and generation, plus W3C Baggage in the `baggage` header.

### Incoming trace context

By default, Ferron discards the incoming `traceparent`, `tracestate`, and `baggage` headers. It generates a new trace ID for every request, so each request that enters Ferron starts with a new server-generated trace identity.

With `trace { trust_request }`, Ferron parses the incoming `traceparent` and `tracestate` headers and uses them as the parent of the internal `ferron.request` span. Ferron creates a local request span with the same trace ID and a new span ID. It reuses that span context for upstream propagation, access logs, and request-scoped OTLP logs.

If the request has no trace context, Ferron can still generate one when `generate` is active.

With `trust_request`, Ferron also parses the incoming `baggage` header and attaches it to the local request span context. Ferron propagates the baggage to upstream services and includes it in OTLP span exports. Application-defined key-value pairs then flow through the request path.

### Sampling probability in `tracestate`

When Ferron starts a trace, it owns the sampling decision. It records the sampling probability in the W3C `ot` `tracestate` entry:

```ferron
http {
    trace_sampling traceidratio { ratio 0.1 }
}
```

Generated traces carry `tracestate: ot=0.1` in the headers sent to upstreams and on the exported span. Backends that extrapolate sampled traces read `ot` to scale trace-derived counts back up. Without `ot`, a 10% sampler looks like a 10x drop in traffic.

Ferron omits `ot` when the probability carries no information. This covers `always_on`, `always_off`, `parentbased_always_on`, any ratio of `1.0` or `0.0`, and `attribute_based`. `attribute_based` decides per trace, not by ratio.

With `trust_request` and an incoming `tracestate`, Ferron passes the value through unchanged. The caller owns the sampling decision for its own trace, so Ferron does not overwrite its `ot` entry.

### Trace configuration

These directives go inside the `http` block.

| Directive       | Arguments | Description                                                                                       | Default |
| --------------- | --------- | ------------------------------------------------------------------------------------------------- | ------- |
| `trace`         | none      | Opens a block for trace-related configuration.                                                    | none    |
| `generate`      | boolean   | Generate a new trace context when none exists, either from trust or from generation.              | `true`  |
| `trust_request` | boolean   | Use the incoming `traceparent`, `tracestate`, and `baggage` headers as the parent trace context. When disabled, Ferron discards incoming trace headers and generates a new trace ID. | `false` |

Ferron reads these directives before it routes the request, so you can set them per host. A host block inherits values from the global `http` block.

> [!note]
> The directives apply to the matched host block only. They do not apply to `location`, `if`, or `if_not` blocks. Put them in the host block or in the global `http` block.

### W3C Baggage

Baggage carries application-defined key-value pairs, such as tenant ID, user segment, or request flags, across service boundaries. Ferron propagates it with the trace context headers.

#### Propagation

1. By default, Ferron discards incoming `baggage` headers. With `trust_request`, it reads the `baggage` header from the request.
2. Ferron stores the baggage string in the request trace context, when available.
3. When Ferron forwards a request to an upstream service and the trace context has non-empty baggage, it sends the `baggage` header with `traceparent` and `tracestate`.
4. With OTLP export, Ferron parses baggage and attaches it to the OpenTelemetry span context.

#### Header format

The `baggage` header follows the [W3C Baggage specification](https://www.w3.org/TR/baggage/). Multiple items are comma-separated:

```text
baggage: userId=alice,serverNode=5;props;otherKey=otherValue
```

Each item is a `key=value` pair with optional semicolon-separated properties. Values use URL encoding.

#### Baggage promotion

You can promote specific baggage keys into OpenTelemetry attributes on logs, metrics, and traces. The `baggage` sub-directive inside an observability backend block configures the promotion:

```ferron
{
    observability {
        provider otlp

        traces https://collector:4317/v1/traces {
            protocol grpc
        }

        baggage {
            key "tenant.id" {
                attribute "tenant.id"
                signals traces logs
                max_distinct 1000
            }
        }
    }
}
```

> [!info]
> See [OTLP observability](/docs/configuration/observability/otlp#baggage-promotion) and [Prometheus metrics](/docs/configuration/observability/prometheus#baggage-promotion) for the full `baggage` directive reference.

#### Examples

Without `trust_request`, Ferron discards client trace headers. A client sends:

```http
GET /api/data HTTP/1.1
Host: example.com
traceparent: 00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01
baggage: userId=alice,tenantId=acme
```

Ferron ignores the incoming `traceparent` and `baggage`. It generates a fresh trace ID and span ID. No baggage goes upstream unless a module adds it.

With `trust_request`, Ferron reads the headers:

```ferron
example.com {
    http {
        trace {
            generate
            trust_request
        }
    }
}
```

Ferron reads `traceparent`, `tracestate`, and `baggage`. It stores baggage in the request trace context and passes the trace headers to upstream services. With the OTLP provider, it also attaches the baggage to the span context.

> [!tip]
> The reverse proxy, CGI, FastCGI, and SCGI modules inject `traceparent`, `tracestate`, and `baggage` into outgoing requests when a trace context exists. No per-module configuration is needed.
>
> The `trace` block `generate` and `trust_request` directives control this injection, and a configured trace sink also matters. Ferron reads both before it routes the request, so a host block sets them for its own requests.
>
> For CGI, FastCGI, and SCGI backends, the modules map the headers to `HTTP_TRACEPARENT`, `HTTP_TRACESTATE`, and `HTTP_BAGGAGE`. Application code can read these variables without special header parsing.

> [!note]
>
> - Trace headers carry unique identifiers. Confirm that this complies with your privacy requirements.
> - With `trust_request`, Ferron propagates incoming baggage as-is without validation or modification.
> - With OTLP, Ferron attaches baggage items to spans. High-cardinality keys can increase span storage costs.

### Trace ID response header

Ferron can inject the trace ID of the current request into a response header. Clients can then correlate requests with server-side traces and logs.

> [!note]
> A named host does not inherit `trace_id_header` from the wildcard `*` host. It still inherits from global defaults. A `location` block inherits the directive from its host.

#### `trace_id_header`

`trace_id_header` configures whether and how Ferron injects the trace ID into response headers.

```ferron
example.com {
    trace_id_header {
        header_name X-Request-Trace-Id
        reflect_request
    }
}
```

| Nested directive  | Arguments  | Description                                                                              | Default             |
| ----------------- | ---------- | ---------------------------------------------------------------------------------------- | ------------------- |
| `header_name`     | `<string>` | Response header that receives the trace ID.                                              | `X-Ferron-Trace-Id` |
| `reflect_request` | `[bool]`   | Inject the trace ID only when the request contains `X-Ferron-Trace-Reflect: 1`.           | `false`             |

In this example, Ferron writes the trace ID to `X-Request-Trace-Id`, and only when the request asks for reflection.

Use bare `trace_id_header` to inject `X-Ferron-Trace-Id` into every response, including error responses. Use `trace_id_header false` to disable injection.

With `reflect_request`, Ferron injects the header only when the request includes `X-Ferron-Trace-Reflect: 1`. This is useful for development and debugging when you want trace IDs on demand.

#### Behavior

- Ferron takes the trace ID from the trace context of the current request. This is the W3C `traceparent` context when present, or the generated trace ID.
- Ferron injects the header into custom responses, for example from reverse proxy or static file serving. It also injects the header into built-in error responses, such as 404 and 500.
- With `reflect_request`, Ferron injects the trace ID only when the request carries `X-Ferron-Trace-Reflect: 1`.

> [!note]
> If no trace context exists for the request, Ferron does not inject the header. This can happen when the configuration sets `trace { generate false }` and the request has no `traceparent` header.

## Trace spans

Each HTTP request creates a root span and nested spans for pipeline execution.

### Root request span

`ferron.request` starts when the request enters the handler. It has these attributes:

- `http.request.method`, `url.full`, `url.scheme`, `server.address`, `server.port`, `client.address`, `ferron.host`
- For HTTPS connections: `tls.protocol.version` (for example `"TLSv1.3"`) and `tls.cipher_suite` (for example `"TLS_AES_256_GCM_SHA384"`)

`ferron.request` ends when the request completes. It adds `http.response.status_code`, `http.route` when applicable, and `error.type` when the status is 400 or higher.

`ferron.host` is the requested virtual host from the `Host` header or the SNI hostname. Ferron omits it when the request has neither. Group by this attribute for multi-tenant filtering. It matches the `ferron.host` attribute on TLS metrics, so traces and metrics join on the same key.

`server.address` is the bound socket address, an IP address or the Unix socket path. It is not the virtual host.

`http.route` is the matched route template, built from the `location` block names from root to leaf. Given this configuration:

```ferron
api.example.com {
    location /api {
        location /users/:id { proxy http://localhost:8080 }
    }
}
```

A request to `/api/users/42` reports `http.route` as `/api/users/:id`. The concrete request path is not used, so the attribute stays low-cardinality. When no `location` block matches, Ferron omits `http.route`.

### Pipeline execution span

`ferron.pipeline.execute` wraps the entire pipeline execution, including all forward and inverse stages. The span is a child of `ferron.request`.

### File resolution span

`ferron.pipeline.file_resolve` wraps static file path resolution with a `root` directive. The span is a child of `ferron.pipeline.execute`.

- Attributes: `ferron.file_resolve.request_path`, `ferron.file_resolve.root_path`, `ferron.file_resolve.outcome`
- On success: `ferron.file_resolve.resolved_path`
- On error: `ferron.file_resolve.last_candidate_path`

### Per-stage spans

Each pipeline stage and file-serving stage generates a forward span (`ferron.stage.<stage_name>`) and an inverse span (`ferron.stage.<stage_name>.inverse`). These spans are children of `ferron.pipeline.execute`, which enables flame graph analysis. Every per-stage span carries a `ferron.stage.name` attribute.

### Error pipeline span

`ferron.pipeline.execute_error` wraps error pipeline execution when Ferron generates error responses. It carries `http.response.status_code`.

Tracing backends such as OTLP consume these spans. All spans from one request share the same `trace_id`. Access logs carry the matching request span context when available.

## Trace sampling

`trace_sampling` in the `http` block controls which traces Ferron samples and exports. Sampling reduces trace volume while keeping representative coverage.

Set `trace_sampling` per host. Ferron reads it before it routes the request, so a host block sets the sampling mode for its own requests. A host block that does not set it inherits the global value.

| Mode                       | Description                                                                                     |
| -------------------------- | ----------------------------------------------------------------------------------------------- |
| `always_on`                | Sample every trace. Use it for development.                                                     |
| `always_off`               | Sample no traces. This disables trace export.                                                   |
| `parentbased_always_on`    | Follow the parent sampling decision. Always sample root spans, which have no parent. This is the default. |
| `traceidratio`             | Sample a fixed ratio of traces by trace ID.                                                     |
| `parentbased_traceidratio` | Sample root spans by ratio and follow the parent decision for child spans. Recommended for production. |
| `attribute_based`          | Sample by span attributes visible when Ferron creates the span.                                 |

```ferron
example.com {
    http {
        trace {
            generate
        }

        # Sample 10% of root spans, respect parent for child spans
        trace_sampling parentbased_traceidratio {
            ratio 0.1
        }
    }
}
```

> [!note]
> The default mode, `parentbased_always_on`, samples all traces. Use `parentbased_traceidratio` in production.

### Ratio-based sampling

`traceidratio` and `parentbased_traceidratio` accept a `ratio` sub-directive, a float from `0.0` to `1.0`:

```ferron
example.com {
    http {
        trace_sampling parentbased_traceidratio {
            ratio 0.05   # 5% of root spans
        }
    }
}
```

Use `parentbased_traceidratio` instead of bare `traceidratio` in distributed systems. It keeps sampling decisions consistent across service boundaries. With `traceidratio`, a child span can be sampled when the parent is not, which produces partial traces.

### Attribute-based sampling

`attribute_based` samples spans by the attributes that exist when Ferron creates the span. Configure rules in a `rules` block:

```ferron
example.com {
    http {
        trace_sampling attribute_based {
            # What to do with spans that don't match any rule
            default_action sample

            rules {
                # Always sample spans with http.request.method == POST
                rule exact http.request.method POST

                # Sample spans where url.path starts with "/api/"
                rule prefix url.path /api/
            }
        }
    }
}
```

Each `rule` takes 2 or 3 arguments:

| Argument       | Description                                                                   |
| -------------- | ----------------------------------------------------------------------------- |
| `<match_type>` | One of `exact`, `prefix`, or `exists`.                                        |
| `<attribute>`  | Span attribute key to match.                                                  |
| `<value>`      | Value to match. Required for `exact` and `prefix`, omitted for `exists`.      |

Any matching rule samples the span. When no rule matches, `default_action` decides:

| Value    | Behavior                                                    |
| -------- | ----------------------------------------------------------- |
| `drop`   | Drop spans that match no rule. This is the default.         |
| `sample` | Sample spans even when they match no rule.                  |

> [!warning]
> `attribute_based` without an explicit `default_action` drops all non-matching spans silently. For example, rules that sample `/api/` routes also drop health checks, static assets, and everything else. Set `default_action sample` unless you deliberately want to drop non-matching spans.

> [!note]
> These HTTP request attributes exist during the sampling decision: `http.request.method`, `url.path`, `url.scheme`, `server.address`, `server.port`, `client.address`, `ferron.host`, `tls.protocol.version`, and `tls.cipher_suite`.

> [!important]
> `attribute_based` supports head sampling only. For tail sampling, configure your collector instead.

`http.route` is not in that list. It describes the routing outcome, which happens after the sampling decision, so an `attribute_based` rule can never match it. Use `ferron.host` to sample one virtual host, or `url.path` with a `prefix` matcher to sample a path prefix.

## See also

- [W3C Trace Context](#w3c-trace-context): incoming trace context parsing, trace configuration, baggage propagation, and trace ID response headers
- [Trace sampling](#trace-sampling): sampling modes and configuration
- [OTLP observability](/docs/configuration/observability/otlp): export traces over OpenTelemetry Protocol
- [Prometheus metrics](/docs/configuration/observability/prometheus): native Prometheus metrics export
- [Access logging](/docs/configuration/observability/logging): access log configuration
