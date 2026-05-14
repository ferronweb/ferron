---
title: Trace context
description: Propagation and generation of W3C Trace Context (traceparent / tracestate).
---

Ferron 3 supports W3C Trace Context (`traceparent` and `tracestate`) propagation and generation. This enables end-to-end observability by carrying trace identifiers across service boundaries.

Incoming `traceparent` and `tracestate` headers are parsed and used as the parent for Ferron's internal `ferron.request` span. Ferron creates a local request span with the same trace ID and a new span ID, then reuses that local request span context for upstream propagation, access logs, and request-scoped OTLP logs. If the request arrives without trace context, Ferron can generate a new one (default behavior).

## Trace configuration

These directives are configured within the `http` block.

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `trace` | none | Opens a block for trace-related configuration. | - |
| `generate` | boolean | Specifies whether a new trace context should be generated if the incoming request lacks one. | `true` |
| `sampled` | boolean | Specifies the default sampling flag for generated trace contexts. | `false` |

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

## Notes and troubleshooting

- The `http-proxy` and `http-fproxy` modules automatically propagate the current trace context to upstream services.
- Generating and propagating trace headers carries unique identifiers. Ensure this complies with your privacy requirements.
- Ferron 3 preserves the incoming `tracestate` header and propagates it as-is.
- To export these traces to an external system, configure an observability sink such as `observability-otlp`.
