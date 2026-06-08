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

See [OTLP observability](/docs/v3/configuration/observability/otlp#baggage-promotion) and [Prometheus metrics](/docs/v3/configuration/observability/prometheus#baggage-promotion) for full documentation of the `baggage` directive.

### Example

A client sends:

```http
GET /api/data HTTP/1.1
Host: example.com
traceparent: 00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01
baggage: userId=alice,tenantId=acme
```

Ferron stores the baggage in the request trace context and propagates both `traceparent` and `baggage` to upstream services. When using the OTLP provider, the baggage is attached to the span context and visible in your observability backend.

## Notes and troubleshooting

- The `http-proxy` and `http-fproxy` modules automatically propagate the current trace context and baggage to upstream services.
- Generating and propagating trace headers carries unique identifiers. Ensure this complies with your privacy requirements.
- Ferron 3 preserves the incoming `tracestate` header and propagates it as-is.
- Baggage values are propagated as-is; Ferron does not validate or modify them. Ensure baggage content complies with your privacy and security requirements.
- Baggage items are attached to OpenTelemetry spans when using the OTLP provider. High-cardinality baggage keys may increase span storage costs in your observability backend.
- The `sampled` flag controls only the `traceparent` header propagated to upstream services. It does not influence the OTLP `sampling` mode or whether Ferron exports its own spans. See [OTLP trace sampling](/docs/v3/configuration/observability/otlp#trace-sampling) for how OTLP export sampling works.
- To export these traces to an external system, configure an observability sink such as `observability-otlp`.

## See also

- [OTLP observability](/docs/v3/configuration/observability/otlp) for exporting traces and baggage to OpenTelemetry collectors
- [Prometheus metrics](/docs/v3/configuration/observability/prometheus) for native Prometheus metrics export
- [Observability and logging](/docs/v3/configuration/observability/logging) for general observability configuration
