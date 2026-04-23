---
title: "Configuration: OTLP observability"
description: "OpenTelemetry Protocol (OTLP) export configuration for logs, metrics, and traces."
---

This page documents the OTLP (OpenTelemetry Protocol) observability configuration for Ferron. The `observability-otlp` module exports logs, metrics, and traces to OpenTelemetry collectors, enabling integration with modern observability platforms like Jaeger, Zipkin, Prometheus, and commercial APM solutions.

## Directives

OTLP export is configured via `observability` blocks with `provider "otlp"`:

```ferron
example.com {
    observability {
        provider "otlp"

        logs "https://collector:4318/v1/Logs" {
            protocol "http/protobuf"
        }

        metrics "https://collector:4318/v1/Metrics" {
            protocol "http/protobuf"
        }

        traces "https://collector:4317" {
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
| `no_verify` | `<bool>` | Disable TLS certificate verification. Use with caution. | `false` |

## Configuration examples

### Basic OTLP configuration

```ferron
example.com {
    observability {
        provider "otlp"
        service_name "my-ferron-instance"

        traces "https://otlp-collector:4317" {
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
        provider "otlp"
        service_name "ferron-production"

        logs "https://logs-collector:4318/v1/logs" {
            protocol "http/protobuf"
            authorization "Bearer my-secret-token"
        }

        metrics "https://metrics-collector:4318/v1/metrics" {
            protocol "http/json"
        }

        traces "https://traces-collector:4317" {
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
        provider "otlp"
        service_name "ferron-mixed"

        logs "http://localhost:4318/v1/logs" {
            protocol "http/json"
        }

        metrics "http://localhost:4318/v1/metrics" {
            protocol "http/protobuf"
        }

        traces "http://localhost:4317" {
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
        provider "otlp"
        service_name "ferron-dev"
        no_verify true

        traces "https://localhost:4317" {
            protocol "grpc"
        }
    }
}
```

## Protocol options

### gRPC protocol

The `grpc` protocol uses gRPC for efficient binary communication:

- **Endpoint format** - typically `host:port` (no path)
- **Example** - `"https://collector:4317"`
- **Best for** - high-volume production environments
- **Authorization** - passed as gRPC metadata

### HTTP/protobuf protocol

The `http/protobuf` protocol uses HTTP with Protocol Buffers encoding:

- **Endpoint format** - full URL with path
- **Example** - `"https://collector:4318/v1/metrics"`
- **Best for** - compatibility with HTTP-based collectors
- **Authorization** - passed as HTTP `Authorization` header

### HTTP/json protocol

The `http/json` protocol uses HTTP with JSON encoding:

- **Endpoint format** - full URL with path
- **Example** - `"https://collector:4318/v1/metrics"`
- **Best for** - debugging and development
- **Authorization** - passed as HTTP `Authorization` header

## Signal correlation

All three signals from the same HTTP request share the same `trace_id`. This enables correlated queries like "show me all logs and metrics for trace `abc123`" in your observability backend.

### Trace context propagation

Ferron automatically:

1. **Generates trace IDs** for incoming requests without trace context
2. **Propagates trace context** via W3C Trace Context headers (`traceparent`, `tracestate`)
3. **Links all signals** (logs, metrics, traces) with the same trace ID
4. **Adds span context** to logs for correlation

### Example trace flow

```text
Request → Ferron (generates trace_id) → OTLP Collector
    ↓
Logs (trace_id) → OTLP Collector
    ↓
Metrics (trace_id) → OTLP Collector
    ↓
Traces (trace_id) → OTLP Collector
```

## Integration with observability platforms

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

## Performance considerations

### Protocol choice

- **gRPC** - best performance, lowest overhead
- **HTTP/protobuf** - good balance of performance and compatibility
- **HTTP/json** - highest overhead, best for debugging

### Batch size and intervals

OTLP batching is handled by the collector. For high-volume sites:

- Use gRPC protocol
- Configure collector batch processor appropriately
- Monitor export latency

### Network considerations

- **Local collectors** - low latency, high reliability
- **Remote collectors** - consider connection pooling and retries
- **TLS overhead** - use `no_verify` cautiously in development only

## Notes and troubleshooting

### Troubleshooting

### Connection issues

- Verify collector endpoints are reachable: `curl -v https://collector:4317`
- Check firewall rules allow outbound connections
- Test with `no_verify true` temporarily to rule out TLS issues

### Notes

- **TLS certificate verification** - disabling with `no_verify true` should only be used for development or testing with self-signed certificates.
- **Protocol compatibility** - not all collectors support all protocols. Check your collector's documentation.
- **Endpoint paths** - HTTP endpoints require full paths (e.g., `/v1/metrics`), while gRPC typically uses just the port.
- **Authorization format** - some collectors expect `Bearer token`, others expect just the token. Check your collector's requirements.
- **Signal correlation** - all signals from the same request share the same trace context, enabling correlated analysis in your observability backend.

### Authentication problems

- Verify authorization tokens/secrets are correct
- Check if the collector expects `Bearer` prefix in authorization
- Test with simple endpoints first

### Performance problems

- Monitor export queue length in metrics
- Consider reducing log volume or sampling traces
- Check collector resource usage

### Missing data

- Verify signal types are properly configured
- Check that endpoints are correct (ports, paths)
- Ensure service_name matches expected values

### See also

- [Observability and logging](/docs/v3/configuration/observability-logging) for general observability configuration
- [Prometheus metrics](/docs/v3/configuration/observability-prometheus) for native Prometheus metrics export
- [Core directives](/docs/v3/configuration/core-directives#observability) for global observability settings
