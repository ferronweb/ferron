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
| `sampling` | `<string>` | Trace sampling mode. Only applicable to `traces` blocks. See [Trace sampling](#trace-sampling). | `parentbased_always_on` |

### Global options

| Directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `service_name` | `<string>` | OTLP resource service name. | `"ferron"` |
| `no_verification` | `<bool>` | Disable TLS certificate verification. Use with caution. | `false` |
| `log_style` | `<string>` | Log style for log records. `legacy` (default) preserves the existing human-readable `message` body. `modern` publishes a short `summary` plus typed per-event attributes and remaps access-log fields to OTEL semantic conventions. | `"legacy"` |

### Baggage promotion

The `baggage` sub-directive promotes specific W3C Baggage keys into telemetry attributes for logs, metrics, and traces. This is useful for adding request-scoped context (such as tenant IDs or user roles) to your telemetry signals without custom instrumentation.

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

Each `key` entry configures one baggage key to promote:

| Nested directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `key` | `<string>` | The W3C Baggage key to extract. Required. | - |
| `attribute` | `<string>` | The OpenTelemetry attribute name to use. | same as the baggage key |
| `signals` | `<string>...` | Which signals to emit the attribute on. Values: `traces`, `logs`, `metrics`. | all signals |
| `max_distinct` | `<number>` | Maximum distinct values for metrics before hashing. Prevents high-cardinality label explosion. | no cap |

### Trace sampling

The `sampling` sub-directive inside a `traces` block controls which traces are sampled and exported. Sampling reduces the volume of trace data sent to your collector while maintaining representative coverage.

| Mode | Description |
| --- | --- |
| `always_on` | Sample every trace. Useful for development. |
| `always_off` | Sample no traces. Effectively disables trace export. |
| `parentbased_always_on` | Respect the parent span's sampling decision. Always sample root spans (no parent). **This is the default.** |
| `traceidratio` | Sample a fixed ratio of traces based on trace ID. |
| `parentbased_traceidratio` | Parent-based sampling with ratio-based sampling for root spans. Recommended for production. |
| `attribute_based` | Sample based on span attributes set at creation time. |

```ferron
{
    observability {
        provider otlp

        traces "https://collector:4317/v1/traces" {
            protocol "grpc"

            # Sample 10% of root spans, respect parent for child spans
            sampling "parentbased_traceidratio" {
                ratio 0.1
            }
        }
    }
}
```

#### Ratio-based sampling

The `traceidratio` and `parentbased_traceidratio` modes accept a `ratio` sub-directive (a float between `0.0` and `1.0`):

```ferron
{
    observability {
        provider otlp

        traces "https://collector:4317/v1/traces" {
            sampling "parentbased_traceidratio" {
                ratio 0.05   # 5% of root spans
            }
        }
    }
}
```

Use `parentbased_traceidratio` (not bare `traceidratio`) in distributed systems to ensure consistent sampling decisions across service boundaries. With `traceidratio`, child spans may be sampled even if the parent was not, leading to partial traces.

#### Attribute-based sampling

The `attribute_based` mode samples spans based on attributes visible at span creation time. Configure rules inside a `rules` block:

```ferron
{
    observability {
        provider otlp

        traces "https://collector:4317/v1/traces" {
            sampling "attribute_based" {
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
}
```

Each `rule` takes 2 or 3 arguments:

| Argument | Description |
| --- | --- |
| `<match_type>` | One of `exact`, `prefix`, or `exists`. |
| `<attribute>` | The span attribute key to match. |
| `<value>` | The value to match (required for `exact` and `prefix`, omitted for `exists`). |

A span is sampled if **any** rule matches. If no rules match, the span is dropped.

:::note
Attribute-based sampling inspects attributes set on the `SpanBuilder` before the span is built. In Ferron, HTTP request attributes (`http.request.method`, `url.path`, `url.scheme`, `server.address`, `server.port`, `client.address`) are set at this stage and are available for sampling decisions.
:::

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

### Production trace sampling

```ferron
example.com {
    observability {
        provider otlp
        service_name "ferron-production"

        traces "https://collector:4317/v1/traces" {
            protocol "grpc"

            # Sample 10% of root spans, respect parent for child spans
            sampling "parentbased_traceidratio" {
                ratio 0.1
            }
        }
    }
}
```

### Attribute-based trace sampling

```ferron
example.com {
    observability {
        provider otlp
        service_name "ferron-production"

        traces "https://collector:4317/v1/traces" {
            protocol "grpc"

            # Always sample POST requests and /api/ routes
            sampling "attribute_based" {
                rules {
                    rule "exact" "http.request.method" "POST"
                    rule "prefix" "url.path" "/api/"
                }
            }
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
- **Trace sampling** - the default sampling mode (`parentbased_always_on`) samples all traces. In production, use `parentbased_traceidratio` with an appropriate ratio to control trace volume. For attribute-based sampling, ensure the attributes you match on are set in the `traces` block's builder attributes (HTTP request method, URL path, scheme, server address/port, client address are available).
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
