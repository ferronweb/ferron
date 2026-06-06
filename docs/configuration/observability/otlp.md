---
title: "Configuration: OTLP observability"
description: "OpenTelemetry Protocol (OTLP) export configuration for logs, metrics, and traces."
---

This page documents the OTLP (OpenTelemetry Protocol) observability configuration for Ferron. The `observability-otlp` module exports logs, metrics, and traces to OpenTelemetry collectors, allowing integration with modern observability platforms such as Jaeger, Loki, Prometheus, and commercial APM solutions.

## Directives

OTLP export is configured via `observability` blocks with `provider otlp`:

```ferron
example.com {
    observability {
        provider otlp

        logs "https://collector:4318/v1/logs" {
            protocol "http/protobuf"
        }

        metrics "https://collector:4318/v1/metrics" {
            protocol "http/protobuf"
        }

        traces "https://collector:4317/v1/traces" {
            protocol "grpc"
        }

        service_name "my-service"
    }
}
```

### Signal sub-blocks

Each signal type (`logs`, `metrics`, `traces`) is configured independently. Omitting a signal disables it for that host.

| Directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `logs` | `<endpoint>` | OTLP logs endpoint. | disabled |
| `metrics` | `<endpoint>` | OTLP metrics endpoint. | disabled |
| `traces` | `<endpoint>` | OTLP traces endpoint. | disabled |

Each signal sub-block supports these nested directives:

| Directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `protocol` | `<string>` | Transport protocol. One of `grpc`, `http/protobuf`, `http/json`. | `grpc` |
| `authorization` | `<string>` | HTTP `Authorization` header (HTTP) or gRPC metadata (gRPC). | none |

### Global options

| Directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `service_name` | `<string>` | OTLP resource service name. | `"ferron"` |
| `no_verification` | `<bool>` | Disable TLS certificate verification. Use with caution. | `false` |

## Configuration examples

### Basic OTLP configuration

```ferron
example.com {
    observability {
        provider otlp
        service_name "my-ferron-instance"

        traces "https://otlp-collector:4317/v1/traces" {
            protocol "grpc"
        }
    }
    root /var/www/html
}
```

### Complete observability setup

```ferron
example.com {
    observability {
        provider otlp
        service_name "ferron-production"

        logs "https://logs-collector:4318/v1/logs" {
            protocol "http/protobuf"
            authorization "Bearer my-secret-token"
        }

        metrics "https://metrics-collector:4318/v1/metrics" {
            protocol "http/json"
        }

        traces "https://traces-collector:4317/v1/traces" {
            protocol "grpc"
        }
    }
    root /var/www/html
}
```

### Multiple protocols

```ferron
# Different protocols for different signals
example.com {
    observability {
        provider otlp
        service_name "ferron-mixed"

        logs "http://localhost:4318/v1/logs" {
            protocol "http/json"
        }

        metrics "http://localhost:4318/v1/metrics" {
            protocol "http/protobuf"
        }

        traces "http://localhost:4317/v1/traces" {
            protocol "grpc"
        }
    }
}
```

### Disabling TLS verification (development only)

```ferron
# Only for development/testing
example.com {
    observability {
        provider otlp
        service_name "ferron-dev"
        no_verification

        traces "https://localhost:4317/v1/traces" {
            protocol "grpc"
        }
    }
}
```

## Protocol options

Ferron supports three OTLP protocols for exporting signals:

- `grpc` - gRPC protocol for efficient binary communication, recommended for production environments
- `http/protobuf` - HTTP with Protocol Buffers encoding, recommended for compatibility with HTTP-based collectors
- `http/json` - HTTP with JSON encoding, recommended for debugging and development

## Signal correlation

Request traces, request-scoped logs, and access logs from the same HTTP request share the same request span context when Ferron has a request trace. Baggage from the incoming `baggage` header is attached to the span context, making it available for correlated queries like "show me all logs for trace `abc123`" or "filter by baggage key `userId`" in your observability backend.

### Trace context propagation

Ferron automatically:

1. **Generates trace IDs** for incoming requests without trace context
2. **Propagates trace context** via W3C Trace Context headers (`traceparent`, `tracestate`)
3. **Propagates baggage** via the W3C Baggage header (`baggage`)
4. **Creates one local request span** per request and nests pipeline, stage, and error-pipeline spans under it
5. **Adds request span context** to OTLP logs and access logs for correlation

When a request carries a `baggage` header, Ferron parses it and attaches the key-value pairs to the OpenTelemetry span context. This makes baggage available to downstream spans and visible in your observability backend as span baggage attributes.

Metrics exported through OTLP do not carry per-request trace or span IDs. Correlate metrics using their semantic attributes, resource attributes, and timestamps instead of expecting a metric data point to join directly to a single trace.

## Integration with observability platforms

Ferron supports integration with various observability platforms via OTLP. Below are some example configurations for popular platforms.

### Jaeger

Configure Jaeger to receive OTLP traces:

```yaml
# jaeger-config.yaml
receivers:
  otlp:
    protocols:
      grpc:
        endpoint: 0.0.0.0:4317
      http:
        endpoint: 0.0.0.0:4318

exporters:
  otlphttp:
    endpoint: "http://jaeger:4318"
    tls:
      insecure: true

services:
  pipelines:
    traces:
      receivers: [otlp]
      exporters: [otlphttp]
```

### Prometheus (via OTLP)

While Ferron has native Prometheus support, you can also use OTLP for metrics:

```yaml
# prometheus-config.yaml
receivers:
  otlp:
    protocols:
      grpc:
        endpoint: 0.0.0.0:4317
      http:
        endpoint: 0.0.0.0:4318

exporters:
  prometheus:
    endpoint: "0.0.0.0:8889"

service:
  pipelines:
    metrics:
      receivers: [otlp]
      exporters: [prometheus]
```

### Commercial APM solutions

Most commercial APM solutions support OTLP:

- **Datadog** - use OTLP endpoint with API key
- **New Relic** - configure OTLP exporter with license key
- **Dynatrace** - OTLP ingestion endpoint
- **Honeycomb** - OTLP-compatible endpoint
- **Grafana Cloud** - OTLP-compatible endpoint

## Notes and troubleshooting

- **TLS certificate verification** - disabling with `no_verification` should only be used for development or testing with self-signed certificates.
- **Protocol compatibility** - not all collectors support all protocols. Check your collector's documentation.
- **Authorization format** - some collectors expect `Bearer token`, others expect just the token. Check your collector's requirements.
- **Signal correlation** - all signals from the same request share the same trace context, enabling correlated analysis in your observability backend.
- **Baggage** - the `baggage` header is parsed and attached to OpenTelemetry spans automatically. Baggage values are not validated; ensure they comply with the W3C Baggage specification and your privacy requirements. High-cardinality baggage keys may increase span storage costs.
- **Metric exemplars** - Ferron does not currently support OTLP metric exemplars, so high-cardinality metrics may be less effective for correlation.
- **Troubleshooting connection issues** - if you're having connection issues, verify collector endpoints are reachable: `curl -v https://collector:4317` and check your firewall rules.

## See also

- [Observability and logging](/docs/v3/configuration/observability/logging) for general observability configuration
- [Trace context](/docs/v3/configuration/observability/trace) for W3C Trace Context and Baggage propagation details
- [Prometheus metrics](/docs/v3/configuration/observability/prometheus) for native Prometheus metrics export
- [Core directives](/docs/v3/configuration/server/core-directives#observability) for global observability settings
