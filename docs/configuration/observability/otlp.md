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
| `log_style` | `<string>` | Log style for log records. `legacy` (default) preserves the existing human-readable `message` body. `modern` publishes a short `summary` plus typed per-event attributes and remaps access-log fields to OTEL semantic conventions. | `"legacy"` |

### Baggage promotion

The `baggage` sub-directive promotes specific W3C Baggage keys into telemetry attributes for logs, metrics, and traces. This is useful for adding request-scoped context (such as tenant IDs or user roles) to your telemetry signals without custom instrumentation.

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

Each `key` entry configures one baggage key to promote:

| Nested directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `key` | `<string>` | The W3C Baggage key to extract. Required. | - |
| `attribute` | `<string>` | The OpenTelemetry attribute name to use. | same as the baggage key |
| `signals` | `<string>...` | Which signals to emit the attribute on. Values: `traces`, `logs`, `metrics`. | all signals |
| `max_distinct` | `<number>` | Maximum distinct values for metrics before hashing. Prevents high-cardinality label explosion. | no cap |

### Log style

The `log_style` directive selects how log records are emitted over OTLP:

- `legacy` (default) - each log record's body is the human-readable `message` text. The `format` directive (when set) continues to apply to log records. This is the existing behavior.
- `modern` - each log record's body is a short OTEL-friendly `summary` (e.g. `"Upstream circuit opened"`) and per-event attributes are published as typed OpenTelemetry attributes (string, boolean, integer, float). The `format` directive is ignored for log records in this mode. Access logs in modern mode use a body of `"Access log (<protocol>)"`, set the record timestamp from the access event, and remap access-log fields onto OTEL semantic-convention attribute names.

The most common access-log field remappings in modern mode are:

| Legacy field | OTEL semantic-convention attribute |
| --- | --- |
| `path` | `url.path` |
| `path_and_query` | `url.full` |
| `method` | `http.request.method` |
| `version` | `network.protocol.version` |
| `scheme` | `url.scheme` |
| `client_ip` | `client.address` |
| `client_port` | `client.port` |
| `server_ip` | `server.address` |
| `server_port` | `server.port` |
| `auth_user` | `user.name` |
| `status` | `http.response.status_code` |
| `content_length` | `http.response.body.size` |
| `duration_secs` | `http.server.request.duration` |
| `header_<name>` | `http.request.header.<name>` |
| `timestamp`, `trace_id`, `span_id`, `*_canonical` | dropped (use the record timestamp and standard attributes instead) |
| other fields | `ferron.legacy_field.<field_name>` |

Example:

```ferron
example.com {
    observability {
        provider otlp
        log_style modern
        service_name "my-service"

        logs "https://collector:4318/v1/logs" {
            protocol "http/protobuf"
        }
    }
}
```

Setting `log_style modern` together with a `format` directive is allowed but the format is ignored for log records in modern mode; the validator errors out when both are set.

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

### Baggage promotion with cardinality control

```ferron
example.com {
    observability {
        provider otlp
        service_name "my-service"

        traces "https://collector:4317/v1/traces" {
            protocol "grpc"
        }

        baggage {
            # Promote tenant ID to traces and logs
            key "tenant.id" {
                attribute "tenant.id"
                signals traces logs
            }

            # Promote user role to all signals with cardinality cap
            key "user.role" {
                attribute "ferron.user_role"
                max_distinct 100
            }
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
- **Baggage propagation** - the `baggage` header is parsed and attached to OpenTelemetry spans automatically. Baggage values are not validated; ensure they comply with the W3C Baggage specification and your privacy requirements. High-cardinality baggage keys may increase span storage costs.
- **Baggage promotion** - use the `baggage` sub-directive to promote specific baggage keys into telemetry attributes. For metrics, always set `max_distinct` on keys with unbounded values to prevent high-cardinality label explosion. Values exceeding the distinct cap are automatically hashed.
- **Log style** - the `log_style modern` directive changes the body and attribute shape of OTLP log records, including how access logs are mapped onto OTEL semantic conventions. Existing file and console log output is unchanged. The `format` directive is ignored for log records in modern mode.
- **Metric exemplars** - Ferron does not currently support OTLP metric exemplars, so high-cardinality metrics may be less effective for correlation.
- **Troubleshooting connection issues** - if you're having connection issues, verify collector endpoints are reachable: `curl -v https://collector:4317` and check your firewall rules.

## Best practices

The following best-practice checks are reported by `ferron doctor` for directives on this page.

### `max_distinct` high cardinality prevention

- **No `max_distinct` inside Baggage configuration** - high-cardinality attributes should not be set in baggage, as they can lead to excessive memory usage and performance issues.

## See also

- [Observability and logging](/docs/v3/configuration/observability/logging) for general observability configuration
- [Trace context](/docs/v3/configuration/observability/trace) for W3C Trace Context and Baggage propagation details
- [Prometheus metrics](/docs/v3/configuration/observability/prometheus) for native Prometheus metrics export
- [Core directives](/docs/v3/configuration/server/core-directives#observability) for global observability settings
