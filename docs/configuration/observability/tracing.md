---
title: "Configuration: tracing"
description: "W3C Trace Context propagation, trace spans, trace sampling, and trace ID response headers."
---

This page documents Ferron's tracing system, including W3C Trace Context propagation, internal trace spans, trace sampling, and trace ID response headers.

> [!info]
>
> - For OTLP export configuration, see [OTLP observability](/docs/v3/configuration/observability/otlp).
> - For Prometheus metrics export, see [Prometheus metrics](/docs/v3/configuration/observability/prometheus).

## W3C Trace Context

Ferron 3 supports W3C Trace Context (`traceparent` and `tracestate`) and W3C Baggage (`baggage`) propagation and generation. This enables end-to-end observability by carrying trace identifiers and application-defined context across service boundaries.

### Incoming trace context

By default, Ferron discards any incoming `traceparent`, `tracestate`, and `baggage` headers and generates a new trace ID for each request. This ensures that each request within Ferron's boundary starts with a fresh, server-generated trace identity.

When the `trust_request` directive is enabled in the `trace` block, incoming `traceparent` and `tracestate` headers are parsed and used as the parent for Ferron's internal `ferron.request` span. In this mode, Ferron creates a local request span with the same trace ID and a new span ID, then reuses that local request span context for upstream propagation, access logs, and request-scoped OTLP logs. If the request arrives without trace context, Ferron can still generate a new one when `generate` is enabled.

When `trust_request` is enabled, the incoming `baggage` header is also parsed and attached to the local request span context. Baggage is then propagated to upstream services and included in OTLP span exports, allowing application-defined key-value pairs to flow through the entire request path.

### Trace configuration

These directives are configured within the `http` block.

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `trace` | none | Opens a block for trace-related configuration. | - |
| `generate` | boolean | Specifies whether a new trace context should be generated if no context exists (either from trust or generation). | `true` |
| `trust_request` | boolean | When enabled, incoming `traceparent`, `tracestate`, and `baggage` headers are parsed and used as the parent trace context. When disabled (default), incoming trace headers are discarded and a new trace ID is generated. | `false` |

### W3C Baggage

Ferron 3 propagates the W3C Baggage header (`baggage`) alongside trace context headers. Baggage carries application-defined key-value pairs (e.g. tenant ID, user segment, request flags) across service boundaries without requiring explicit configuration.

#### How baggage propagation works

1. By default, incoming `baggage` headers are discarded (unless `trust_request` is enabled). When `trust_request` is enabled, Ferron reads the incoming `baggage` header from the request.
2. The baggage string (when available) is stored in the request's trace context.
3. When forwarding the request to an upstream service, the `baggage` header is included alongside `traceparent` and `tracestate` only if the trace context carries non-empty baggage values.
4. When exporting via OTLP, baggage is parsed and attached to the OpenTelemetry span context as OpenTelemetry baggage.

#### Baggage header format

The `baggage` header follows the [W3C Baggage specification](https://www.w3.org/TR/baggage/). Multiple items are comma-separated:

```text
baggage: userId=alice,serverNode=5;props;otherKey=otherValue
```

Each item is a `key=value` pair with optional semicolon-separated properties. Values are URL-encoded.

#### Baggage promotion to telemetry attributes

In addition to propagating baggage to upstream services, you can promote specific baggage keys into OpenTelemetry attributes on your telemetry signals (logs, metrics, traces). This is configured via the `baggage` sub-directive within each observability backend block:

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
> See [OTLP observability](/docs/v3/configuration/observability/otlp#baggage-promotion) and [Prometheus metrics](/docs/v3/configuration/observability/prometheus#baggage-promotion) for full documentation of the `baggage` directive.

#### Examples

**With default settings (`trust_request false`):**

A client sends trace headers, but Ferron discards them and generates a new trace ID:

```http
GET /api/data HTTP/1.1
Host: example.com
traceparent: 00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01
baggage: userId=alice,tenantId=acme
```

The incoming `traceparent` and `baggage` are ignored. Ferron generates a fresh trace ID and new span ID. No baggage is propagated upstream unless a module explicitly adds it.

**With `trust_request true`:**

```ferron
http {
    trace {
        generate true
        trust_request true
    }
}
```

When `trust_request` is enabled, Ferron reads the incoming `traceparent`, `tracestate`, and `baggage` headers, stores the baggage in the request trace context, and propagates them to upstream services. When using the OTLP provider, the baggage is attached to the span context and visible in your observability backend.

> [!note]
>
> - The reverse proxy, CGI, FastCGI, and SCGI modules automatically inject trace context headers (`traceparent`, `tracestate`, and `baggage`) into outgoing requests to backend services when a trace context exists. No per-module configuration is needed — trace context injection is controlled globally via the `trace` block (directives `generate` and `trust_request`) and whether a trace sink is configured.
> - For CGI, FastCGI, and SCGI backends, trace context headers are mapped to standard CGI environment variables (`HTTP_TRACEPARENT`, `HTTP_TRACESTATE`, `HTTP_BAGGAGE`), making them accessible to application code without any special header parsing.

> [!note]
>
> - Generating and propagating trace headers carries unique identifiers — ensure this complies with your privacy requirements.
> - By default, incoming baggage values are discarded. When `trust_request` is enabled, baggage values are propagated as-is and Ferron does not validate or modify them.
> - Baggage items are attached to OpenTelemetry spans when using the OTLP provider — high-cardinality baggage keys may increase span storage costs.

### Trace ID response header

Ferron can inject the current request's trace ID into HTTP response headers, making it easy for clients to correlate their requests with server-side traces and logs.

#### `trace_id_header`

The `trace_id_header` directive configures whether and how the trace ID is injected into response headers.

```ferron
example.com {
    trace_id_header {
        header_name "X-Trace-Id"
    }
}
```

| Nested directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `header_name` | `<string>` | Name of the response header to inject the trace ID into. | `X-Ferron-Trace-Id` |
| `reflect_request` | `[bool]` | Only inject the trace ID when the incoming request contains `X-Ferron-Trace-Reflect: 1`. | `false` |

**Configuration example — default behavior:**

```ferron
example.com {
    trace_id_header
}
```

Injects the current request's trace ID into the `X-Ferron-Trace-Id` response header for every response (including error responses).

**Configuration example — custom header name:**

```ferron
example.com {
    trace_id_header {
        header_name "X-Request-Trace-Id"
    }
}
```

Injects the trace ID into a custom `X-Request-Trace-Id` header.

**Configuration example — conditional injection:**

```ferron
example.com {
    trace_id_header {
        reflect_request
    }
}
```

Only injects the trace ID when the incoming request includes `X-Ferron-Trace-Reflect: 1`. This is useful for development or debugging scenarios where you only want trace IDs on demand.

**Configuration example — disable:**

```ferron
example.com {
    trace_id_header false
}
```

Explicitly disables trace ID injection.

#### Behavior

- The trace ID is taken from the current request's trace context (W3C `traceparent` if present, or the generated trace ID).
- The header is injected into both custom responses (e.g., from reverse proxy, static file serving) and built-in error responses (e.g., 404, 500).
- When `reflect_request` is enabled, the trace ID is only injected if the request carries the `X-Ferron-Trace-Reflect: 1` header.

> [!note]
> If no trace context exists for the request, the header is not injected. This can happen when `trace { generate false }` is configured and the incoming request lacks a `traceparent` header.

## Trace spans

Each HTTP request generates a root trace span and multiple nested spans for pipeline execution.

### Root request span

- **`StartSpan("ferron.request")`** — emitted when the request enters the handler.
  - Attributes: `http.request.method`, `url.full`, `url.scheme`, `server.address`, `server.port`, `client.address`
  - For HTTPS connections: `tls.protocol.version` (e.g. `"TLSv1.3"`), `tls.cipher_suite` (e.g. `"TLS_AES_256_GCM_SHA384"`)
- **`EndSpan("ferron.request", error)`** — emitted when the request completes.
  - Attributes: `http.response.status_code`, `http.route` (if applicable), `error.type` (if status >= 400)

### Pipeline execution span

- **`ferron.pipeline.execute`** — wraps the entire pipeline execution, including all forward and inverse stages. This span is a child of `ferron.request`.

### Per-stage spans

Each pipeline and file-serving stage generates its own forward (`ferron.stage.<stage_name>`) and inverse (`ferron.stage.<stage_name>.inverse`) span as a child of `ferron.pipeline.execute`, enabling flame graph analysis. Every per-stage span carries a `stage.name` attribute.

### Error pipeline span

- **`ferron.pipeline.execute_error`** — wraps error pipeline execution when generating error responses.
  - Attributes: `http.response.status_code`

Trace events are consumed by observability backends that support tracing (e.g. OTLP). All spans from the same request share the same `trace_id`, and access logs carry the matching request span context when available.

## Trace sampling

The `trace_sampling` directive (in `http` block) controls which traces are sampled and exported. Sampling reduces the volume of trace data sent to your collector while maintaining representative coverage.

| Mode | Description |
| --- | --- |
| `always_on` | Sample every trace. Useful for development. |
| `always_off` | Sample no traces. Effectively disables trace export. |
| `parentbased_always_on` | Respect the parent span's sampling decision. Always sample root spans (no parent). **This is the default.** |
| `traceidratio` | Sample a fixed ratio of traces based on trace ID. |
| `parentbased_traceidratio` | Parent-based sampling with ratio-based sampling for root spans. Recommended for production. |
| `attribute_based` | Sample based on span attributes visible at span creation time. |

**Configuration example:**

```ferron
example.com {
    http {
        trace {
            generate true
        }

        # Sample 10% of root spans, respect parent for child spans
        trace_sampling "parentbased_traceidratio" {
            ratio 0.1
        }
    }
}
```

> [!note]
> The default trace sampling mode (`parentbased_always_on`) samples all traces; in production use `parentbased_traceidratio`.

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

Use `parentbased_traceidratio` (not bare `traceidratio`) in distributed systems to ensure consistent sampling decisions across service boundaries. With `traceidratio`, child spans may be sampled even if the parent was not, leading to partial traces.

### Attribute-based sampling

The `attribute_based` mode samples spans based on attributes visible at span creation time. Configure rules inside a `rules` block:

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

| Argument | Description |
| --- | --- |
| `<match_type>` | One of `exact`, `prefix`, or `exists`. |
| `<attribute>` | The span attribute key to match. |
| `<value>` | The value to match (required for `exact` and `prefix`, omitted for `exists`). |

A span is sampled if **any** rule matches. When no rules match, the `default_action` directive controls the outcome:

| Value | Behavior |
| --- | --- |
| `drop` | Spans not matching any rule are dropped. **This is the default.** |
| `sample` | Spans not matching any rule are still sampled. |

> [!warning]
> Setting `attribute_based` without an explicit `default_action` drops all non-matching spans silently. This is usually unintended — for example, adding rules to sample `/api/` routes will also drop health checks, static assets, and everything else. Always set `default_action "sample"` unless you deliberately want to drop non-matching spans.

> [!note]
> In Ferron, HTTP request attributes (`http.request.method`, `url.path`, `url.scheme`, `server.address`, `server.port`, `client.address`) are set at this stage and are available for sampling decisions for attribute-based sampling.

## See also

- [W3C Trace Context](#w3c-trace-context) — incoming trace context parsing, trace configuration, Baggage propagation, and trace ID response headers
- [Trace sampling](#trace-sampling) — trace sampling modes and configuration
- [OTLP observability](/docs/v3/configuration/observability/otlp) — exporting traces via OpenTelemetry Protocol
- [Prometheus metrics](/docs/v3/configuration/observability/prometheus) — native Prometheus metrics export
- [Access logging](/docs/v3/configuration/observability/logging) — access log configuration
