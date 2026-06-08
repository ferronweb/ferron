---
title: Trace context
description: "Propagation and generation of W3C Trace Context (traceparent, tracestate) and W3C Baggage."
---

Ferron 3 supports W3C Trace Context (`traceparent` and `tracestate`) and W3C Baggage (`baggage`) propagation and generation. This enables end-to-end observability by carrying trace identifiers and application-defined context across service boundaries.

Incoming `traceparent` and `tracestate` headers are parsed and used as the parent for Ferron's internal `ferron.request` span. Ferron creates a local request span with the same trace ID and a new span ID, then reuses that local request span context for upstream propagation, access logs, and request-scoped OTLP logs. If the request arrives without trace context, Ferron can generate a new one (default behavior).

The incoming `baggage` header is parsed and attached to the local request span context. Baggage is then propagated to upstream services and included in OTLP span exports, allowing application-defined key-value pairs to flow through the entire request path.

## Trace configuration

These directives are configured within the `http` block.

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `trace` | none | Opens a block for trace-related configuration. | - |
| `generate` | boolean | Specifies whether a new trace context should be generated if the incoming request lacks one. | `true` |
| `sampled` | boolean | Sampling flag set in the `traceparent` header propagated to upstream services. Does not affect Ferron's own OTLP trace export. | `false` |

**Configuration example:**

```ferron
example.com {
    http {
        trace {
            generate true
            sampled true
        }
    }
}
```

> [!note]
> The `sampled` flag controls only the `traceparent` header propagated to upstream services — it does not influence the OTLP sampling mode. To export traces to an external system, configure an observability sink such as `observability-otlp`.

## W3C Baggage

Ferron 3 propagates the W3C Baggage header (`baggage`) alongside trace context headers. Baggage carries application-defined key-value pairs (e.g. tenant ID, user segment, request flags) across service boundaries without requiring explicit configuration.

### How baggage propagation works

1. Ferron reads the incoming `baggage` header from the request.
2. The baggage string is stored in the request's trace context.
3. When forwarding the request to an upstream service, the `baggage` header is included alongside `traceparent` and `tracestate`.
4. When exporting via OTLP, baggage is parsed and attached to the OpenTelemetry span context as OpenTelemetry baggage.

### Baggage header format

The `baggage` header follows the [W3C Baggage specification](https://www.w3.org/TR/baggage/). Multiple items are comma-separated:

```text
baggage: userId=alice,serverNode=5;props;otherKey=otherValue
```

Each item is a `key=value` pair with optional semicolon-separated properties. Values are URL-encoded.

### Baggage promotion to telemetry attributes

In addition to propagating baggage to upstream services, you can promote specific baggage keys into OpenTelemetry attributes on your telemetry signals (logs, metrics, traces). This is configured via the `baggage` sub-directive within each observability backend block:

```ferron
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
```

> [!info]
> See [OTLP observability](/docs/v3/configuration/observability/otlp#baggage-promotion) and [Prometheus metrics](/docs/v3/configuration/observability/prometheus#baggage-promotion) for full documentation of the `baggage` directive.

### Example

A client sends:

```http
GET /api/data HTTP/1.1
Host: example.com
traceparent: 00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01
baggage: userId=alice,tenantId=acme
```

Ferron stores the baggage in the request trace context and propagates both `traceparent` and `baggage` to upstream services. When using the OTLP provider, the baggage is attached to the span context and visible in your observability backend.

> [!note]
> The `http-proxy` and `http-fproxy` modules automatically propagate the current trace context and baggage to upstream services. Ferron 3 preserves the incoming `tracestate` header and propagates it as-is.

> [!note]
>
> - Generating and propagating trace headers carries unique identifiers — ensure this complies with your privacy requirements.
> - Baggage values are propagated as-is; Ferron does not validate or modify them by default.
> - Baggage items are attached to OpenTelemetry spans when using the OTLP provider — high-cardinality baggage keys may increase span storage costs.

## Trace ID response header

Ferron can inject the current request's trace ID into HTTP response headers, making it easy for clients to correlate their requests with server-side traces and logs.

### `trace_id_header`

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

### Behavior

- The trace ID is taken from the current request's trace context (W3C `traceparent` if present, or the generated trace ID).
- The header is injected into both custom responses (e.g., from reverse proxy, static file serving) and built-in error responses (e.g., 404, 500).
- When `reflect_request` is enabled, the trace ID is only injected if the request carries the `X-Ferron-Trace-Reflect: 1` header.

> [!note]
> If no trace context exists for the request, the header is not injected. This can happen when `trace { generate false }` is configured and the incoming request lacks a `traceparent` header.

## See also

- [OTLP observability](/docs/v3/configuration/observability/otlp) for exporting traces and baggage to OpenTelemetry collectors
- [Prometheus metrics](/docs/v3/configuration/observability/prometheus) for native Prometheus metrics export
- [Observability and logging](/docs/v3/configuration/observability/logging) for general observability configuration
