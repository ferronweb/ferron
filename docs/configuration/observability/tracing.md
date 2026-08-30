---
title: "Configuration: tracing"
description: "W3C Trace Context propagation, trace spans, trace sampling, and trace ID response headers."
---

This page describes the tracing system of Ferron. The system covers W3C Trace Context propagation, internal trace spans, trace sampling, and trace ID response headers.

> [!info]
>
> - For OTLP export configuration, see [OTLP observability](/docs/v3/configuration/observability/otlp).
> - For Prometheus metrics export, see [Prometheus metrics](/docs/v3/configuration/observability/prometheus).

## W3C Trace Context

Ferron 3 supports W3C Trace Context (`traceparent` and `tracestate`) and W3C Baggage (`baggage`) propagation and generation. This enables end-to-end observability by carrying trace identifiers and application-defined context across service boundaries.

### Incoming trace context

By default, Ferron discards incoming `traceparent`, `tracestate`, and `baggage` headers. It generates a new trace ID for each request. Each request in the Ferron boundary then starts with a fresh server-generated trace identity.

When the `trace` block enables `trust_request`, Ferron parses the incoming `traceparent` and `tracestate` headers. It uses them as the parent for the internal `ferron.request` span. Ferron creates a local request span with the same trace ID and a new span ID. It reuses that span context for upstream propagation, access logs, and request-scoped OTLP logs. If the request has no trace context, Ferron can still generate a new one when `generate` is active.

With `trust_request` enabled, Ferron also parses the incoming `baggage` header and attaches it to the local request span context. Ferron then propagates baggage to upstream services and includes it in OTLP span exports. This lets application-defined key-value pairs flow through the entire request path.

### Trace configuration

These directives go inside the `http` block.

| Directive       | Arguments | Description                                                                                                                                                                                                              | Default |
| --------------- | --------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------- |
| `trace`         | none      | Opens a block for trace-related configuration.                                                                                                                                                                           | none    |
| `generate`      | boolean   | Chooses whether to generate a new trace context when none exists, either from trust or from generation.                                                                                                                  | `true`  |
| `trust_request` | boolean   | When enabled, Ferron uses the incoming `traceparent`, `tracestate`, and `baggage` headers as the parent trace context. When disabled (the default), Ferron discards incoming trace headers and generates a new trace ID. | `false` |

### W3C Baggage

Ferron 3 propagates the W3C Baggage header (`baggage`) alongside trace context headers. Baggage carries application-defined key-value pairs (for example, tenant ID, user segment, request flags) across service boundaries with no explicit configuration.

#### How baggage propagation works

1. By default, Ferron discards incoming `baggage` headers. With `trust_request`, Ferron reads the incoming `baggage` header from the request instead.
2. Ferron stores the baggage string (when available) in the request trace context.
3. When Ferron forwards the request to an upstream service, it includes the `baggage` header alongside `traceparent` and `tracestate`. It does this only when the trace context carries non-empty baggage values.
4. When Ferron exports via OTLP, it parses baggage and attaches it to the OpenTelemetry span context as OpenTelemetry baggage.

#### Baggage header format

The `baggage` header follows the [W3C Baggage specification](https://www.w3.org/TR/baggage/). Multiple items are comma-separated:

```text
baggage: userId=alice,serverNode=5;props;otherKey=otherValue
```

Each item is a `key=value` pair with optional semicolon-separated properties. Values use URL encoding.

#### Baggage promotion to telemetry attributes

You can also promote specific baggage keys into OpenTelemetry attributes on telemetry signals (logs, metrics, traces). The `baggage` sub-directive inside each observability backend block configures this promotion:

```ferron
{
    observability {
        provider otlp

        traces "https://collector:4317/v1/traces" {
            protocol "grpc"
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
> See [OTLP observability](/docs/v3/configuration/observability/otlp#baggage-promotion) and [Prometheus metrics](/docs/v3/configuration/observability/prometheus#baggage-promotion) for complete documentation about the `baggage` directive.

#### Examples

**With default settings (`trust_request false`):**

A client sends trace headers, but Ferron discards them and generates a new trace ID:

```http
GET /api/data HTTP/1.1
Host: example.com
traceparent: 00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01
baggage: userId=alice,tenantId=acme
```

Ferron ignores the incoming `traceparent` and `baggage`. It generates a fresh trace ID and a new span ID. No baggage goes upstream unless a module adds it explicitly.

**With `trust_request true`:**

```ferron
http {
    trace {
        generate
        trust_request
    }
}
```

With `trust_request` enabled, Ferron reads the incoming `traceparent`, `tracestate`, and `baggage` headers. It stores baggage in the request trace context and passes the trace headers to upstream services. With the OTLP provider, Ferron attaches the baggage to the span context, and the observability backend sees it.

> [!tip]
> The reverse proxy, CGI, FastCGI, and SCGI modules inject trace context headers into outgoing requests when a trace context exists. The headers are `traceparent`, `tracestate`, and `baggage`, and they need no per-module configuration. The `trace` block with `generate` and `trust_request` controls this injection globally, and a configured trace sink also matters.
>
> For CGI, FastCGI, and SCGI backends, the modules map these headers to standard CGI environment variables (`HTTP_TRACEPARENT`, `HTTP_TRACESTATE`, `HTTP_BAGGAGE`). Application code can then read the variables without special header parsing.

> [!note]
>
> - Trace header generation and propagation carry unique identifiers, so confirm this complies with your privacy requirements.
> - By default, Ferron discards incoming baggage values, but with `trust_request` enabled it propagates them as-is without validation or modification.
> - With the OTLP provider, Ferron attaches baggage items to OpenTelemetry spans. High-cardinality baggage keys may increase span storage costs.

### Trace ID response header

Ferron can inject the trace ID of the current request into HTTP response headers. Clients can then correlate their requests with the server-side traces and logs.

#### `trace_id_header`

The `trace_id_header` directive configures whether and how Ferron injects the trace ID into response headers.

```ferron
example.com {
    trace_id_header {
        header_name "X-Trace-Id"
    }
}
```

| Nested directive  | Arguments  | Description                                                                              | Default             |
| ----------------- | ---------- | ---------------------------------------------------------------------------------------- | ------------------- |
| `header_name`     | `<string>` | Name of the response header to inject the trace ID into.                                 | `X-Ferron-Trace-Id` |
| `reflect_request` | `[bool]`   | Only inject the trace ID when the incoming request contains `X-Ferron-Trace-Reflect: 1`. | `false`             |

**Configuration example (default behavior):**

```ferron
example.com {
    trace_id_header
}
```

Ferron injects the trace ID into the `X-Ferron-Trace-Id` response header for every response, including error responses.

**Configuration example (custom header name):**

```ferron
example.com {
    trace_id_header {
        header_name "X-Request-Trace-Id"
    }
}
```

Ferron injects the trace ID into the custom `X-Request-Trace-Id` header.

**Configuration example (conditional injection):**

```ferron
example.com {
    trace_id_header {
        reflect_request
    }
}
```

Ferron injects the trace ID only when the incoming request includes `X-Ferron-Trace-Reflect: 1`. This is useful for development or debugging when you want trace IDs on demand.

**Configuration example (disabled):**

```ferron
example.com {
    trace_id_header false
}
```

Ferron explicitly disables trace ID injection.

#### Behavior

- Ferron takes the trace ID from the trace context of the current request. This is the W3C `traceparent` context if present, or the generated trace ID.
- Ferron injects the header into custom responses (for example, from reverse proxy, static file serving). It also injects it into built-in error responses, such as 404 and 500.
- With `reflect_request` enabled, Ferron injects the trace ID only when the request carries the `X-Ferron-Trace-Reflect: 1` header.

> [!note]
> If no trace context exists for the request, Ferron does not inject the header. This can happen when the config sets `trace { generate false }` and the request has no `traceparent` header.

## Trace spans

Each HTTP request generates a root trace span and multiple nested spans for pipeline execution.

### Root request span

- **`StartSpan("ferron.request")`** emits when the request enters the handler.
  - Attributes: `http.request.method`, `url.full`, `url.scheme`, `server.address`, `server.port`, `client.address`
  - For HTTPS connections: `tls.protocol.version` (for example, `"TLSv1.3"`), `tls.cipher_suite` (for example, `"TLS_AES_256_GCM_SHA384"`)
- **`EndSpan("ferron.request", error)`** emits when the request completes.
  - Attributes: `http.response.status_code`, `http.route` (if applicable), `error.type` (if status >= 400)

### Pipeline execution span

- **`ferron.pipeline.execute`** wraps the entire pipeline execution, including all forward and inverse stages. The span is a child of `ferron.request`.

### File resolution span

- **`ferron.pipeline.file_resolve`** wraps static file path resolution with a `root` directive. The span is a child of `ferron.pipeline.execute`.
  - Attributes: `ferron.file_resolve.request_path`, `ferron.file_resolve.root_path`, `ferron.file_resolve.outcome`
  - On success: `ferron.file_resolve.resolved_path`
  - On error: `ferron.file_resolve.last_candidate_path`

### Per-stage spans

Each pipeline stage and file-serving stage generates a forward span (`ferron.stage.<stage_name>`) and an inverse span (`ferron.stage.<stage_name>.inverse`). These are children of `ferron.pipeline.execute`, which enables flame graph analysis. Every per-stage span carries a `ferron.stage.name` attribute.

### Error pipeline span

- **`ferron.pipeline.execute_error`** wraps error pipeline execution when Ferron generates error responses.
  - Attributes: `http.response.status_code`

Observability backends that support tracing (for example, OTLP) consume the trace events. All spans from one request share the same `trace_id`. Access logs carry the matching request span context when available.

## Trace sampling

The `trace_sampling` directive (in the `http` block) controls which traces Ferron samples and exports. Sampling reduces the volume of trace data that Ferron sends to the collector while keeping representative coverage.

| Mode                       | Description                                                                                               |
| -------------------------- | --------------------------------------------------------------------------------------------------------- |
| `always_on`                | Sample every trace. Useful for development.                                                               |
| `always_off`               | Sample no traces. This disables trace export effectively.                                                 |
| `parentbased_always_on`    | Follow the parent sampling decision. Always sample root spans, which have no parent. This is the default. |
| `traceidratio`             | Sample a fixed ratio of traces based on the trace ID.                                                     |
| `parentbased_traceidratio` | Sample root spans by ratio, and follow the parent decision for child spans. Recommended for production.   |
| `attribute_based`          | Sample based on span attributes visible when Ferron creates the span.                                     |

**Configuration example:**

```ferron
example.com {
    http {
        trace {
            generate
        }

        # Sample 10% of root spans, respect parent for child spans
        trace_sampling "parentbased_traceidratio" {
            ratio 0.1
        }
    }
}
```

> [!note]
> The default trace sampling mode (`parentbased_always_on`) samples all traces. In production, use `parentbased_traceidratio`.

### Ratio-based sampling

The `traceidratio` and `parentbased_traceidratio` modes accept a `ratio` sub-directive (a float between `0.0` and `1.0`):

```ferron
example.com {
    http {
        trace_sampling "parentbased_traceidratio" {
            ratio 0.05   # 5% of root spans
        }
    }
}
```

Use `parentbased_traceidratio` instead of bare `traceidratio` in distributed systems. It makes sampling decisions consistent across service boundaries. With `traceidratio`, a child span may get sampled even when the parent is not, which produces partial traces.

### Attribute-based sampling

The `attribute_based` mode samples spans from the attributes that exist when Ferron creates the span. Configure rules inside a `rules` block:

```ferron
example.com {
    http {
        trace_sampling "attribute_based" {
            # What to do with spans that don't match any rule
            default_action "sample"

            rules {
                # Always sample spans with http.request.method == "POST"
                rule "exact" "http.request.method" "POST"

                # Sample spans where url.path starts with "/api/"
                rule "prefix" "url.path" "/api/"

                # Sample spans that have an "error.type" attribute (any value)
                rule "exists" "error.type"
            }
        }
    }
}
```

Each `rule` takes 2 or 3 arguments:

| Argument       | Description                                                                   |
| -------------- | ----------------------------------------------------------------------------- |
| `<match_type>` | One of `exact`, `prefix`, or `exists`.                                        |
| `<attribute>`  | The span attribute key to match.                                              |
| `<value>`      | The value to match (required for `exact` and `prefix`, omitted for `exists`). |

**Any** matching rule samples the span. When no rule matches, the `default_action` directive controls the outcome:

| Value    | Behavior                                                    |
| -------- | ----------------------------------------------------------- |
| `drop`   | Ferron drops spans that match no rule. This is the default. |
| `sample` | Ferron samples spans even when they match no rule.          |

> [!warning]
> Setting `attribute_based` without an explicit `default_action` drops all non-matching spans silently. This is usually not intended. For example, adding rules to sample `/api/` routes also drops health checks, static assets, and everything else. Always set `default_action "sample"` unless you deliberately want to drop non-matching spans.

> [!note]
> In Ferron, HTTP request attributes (`http.request.method`, `url.path`, `url.scheme`, `server.address`, `server.port`, `client.address`) appear during this stage. They drive the sampling decisions for attribute-based sampling.

## See also

- [W3C Trace Context](#w3c-trace-context): incoming trace context parsing, trace configuration, Baggage propagation, and trace ID response headers
- [Trace sampling](#trace-sampling): trace sampling modes and configuration
- [OTLP observability](/docs/v3/configuration/observability/otlp): export traces via OpenTelemetry Protocol
- [Prometheus metrics](/docs/v3/configuration/observability/prometheus): native Prometheus metrics export
- [Access logging](/docs/v3/configuration/observability/logging): access log configuration
